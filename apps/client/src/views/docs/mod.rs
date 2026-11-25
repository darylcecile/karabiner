//! Document tab: Markdown authoring with a formatting toolbar, debounced saves that are
//! aware of revisions, conflict handling, and project document navigation.

mod banner;
mod blocks;
mod format;
mod templates;

use std::{rc::Rc, sync::Once, time::Duration};

use futures::{FutureExt as _, future::Shared};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _,
    button::{Button, ButtonVariants as _},
    input::{Editor, EditorState, Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    popover::Popover,
    text::TextView,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{CreateDocument, Document, DocumentKind, UpdateDocument};

use crate::{
    api::{ApiError, query_value},
    components::{Segment, Typography as _, button_md, button_sm, empty_state, group_label, segmented},
    theme::metrics,
    state::{AppState, Draft, TabTarget},
};
use super::stats::common::relative_time;
use banner::banner;
use format::{Block, Edit};
pub use templates::{DocTemplate, TEMPLATES};

const AUTOSAVE_DELAY: Duration = Duration::from_millis(900);
const CONTEXT: &str = "DocEditor";

actions!(docs, [Save, ToggleBold, ToggleItalic, ToggleCode, InsertLink, TogglePreview]);

fn bind_keys(cx: &mut App) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let context = Some(CONTEXT);
        cx.bind_keys([
            KeyBinding::new("cmd-s", Save, context),
            KeyBinding::new("cmd-b", ToggleBold, context),
            KeyBinding::new("cmd-i", ToggleItalic, context),
            KeyBinding::new("cmd-e", ToggleCode, context),
            KeyBinding::new("cmd-k", InsertLink, context),
            KeyBinding::new("cmd-shift-p", TogglePreview, context),
        ]);
    });
}

pub fn tab(target: &TabTarget, state: AppState, window: &mut Window, cx: &mut App) -> AnyView {
    bind_keys(cx);
    let view = cx.new(|cx| DocView::new(target.clone(), state.clone(), window, cx));
    state.track_draft(target, &view, cx);
    view.into()
}

/// Creates a document from a template in `project_id` and opens it in a tab.
/// Shared entry point for Home, project views and the document rail.
pub fn create_document(
    state: AppState,
    project_id: String,
    template: &'static DocTemplate,
    window: &mut Window,
    cx: &mut App,
) -> Task<Result<(), ApiError>> {
    let body = CreateDocument {
        project_id,
        title: template.title.trim().to_owned(),
        kind: Some(DocumentKind::Markdown),
        markdown: Some(template.markdown.to_owned()),
        ..Default::default()
    };
    window.spawn(cx, async move |cx| {
        let document: Document = state.api.post("/v1/documents", &body).await?;
        cx.update(|window, cx| {
            state.data_changed(cx);
            state.open(TabTarget::document(document.project_id.clone(), document.id.clone()), window, cx);
        })
        .ok();
        Ok(())
    })
}

#[derive(Clone)]
enum Load {
    Loading,
    Ready,
    Missing,
    Failed(String),
}

type SaveRequest = Shared<Task<Result<(), String>>>;

const CONFLICT_MESSAGE: &str = "This document changed elsewhere. Choose Reload or Keep mine first.";

enum Flush {
    Done,
    Blocked(String),
    Wait(SaveRequest),
}

#[derive(Clone, PartialEq)]
enum DeleteState {
    Idle,
    Confirming,
    Deleting,
    Failed(String),
}

#[derive(Clone, PartialEq)]
enum SaveState {
    Saved,
    Unsaved,
    Saving,
    Failed(String),
}

pub struct DocView {
    target: TabTarget,
    state: AppState,
    load: Load,
    document: Option<Document>,
    siblings: Vec<Document>,
    title: Entity<InputState>,
    body: Entity<EditorState>,
    blocks: Vec<blocks::Block>,
    active: Option<usize>,
    block_editor: Entity<TextareaState>,
    base_revision: u64,
    dirty: bool,
    save: SaveState,
    conflict: Option<Document>,
    source_mode: bool,
    create_error: Option<String>,
    /// Template menu open state, shared by both halves of the New doc split.
    new_doc_open: bool,
    seen_version: u64,
    suppress_change: bool,
    load_task: Option<Task<()>>,
    list_task: Option<Task<()>>,
    debounce_task: Option<Task<()>>,
    request: Option<SaveRequest>,
    delete: DeleteState,
    delete_task: Option<Task<()>>,
    focus: FocusHandle,
    /// Allocated width: below 700 the rail moves above the editor (bounded height).
    width: crate::views::responsive::WidthCell,
    toolbar_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl DocView {
    fn new(target: TabTarget, state: AppState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Untitled"));
        let body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .line_number(false)
                .soft_wrap(true)
                .placeholder("Start writing. Format with the toolbar, ⌘B, ⌘I or ⌘K.")
        });
        let block_editor = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 400).placeholder("Write something…"));
        let seen_version = state.service.read(cx).data_version();
        let _subscriptions = vec![
            cx.subscribe_in(&block_editor, window, |this, editor, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && !this.suppress_change {
                    if let Some(block) = this.active.and_then(|ix| this.blocks.get_mut(ix)) {
                        block.source = editor.read(cx).value().to_string();
                    }
                    this.mark_dirty(cx);
                }
            }),
            cx.subscribe_in(&title, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.mark_dirty(cx);
                }
            }),
            cx.subscribe_in(&body, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && this.source_mode {
                    this.mark_dirty(cx);
                }
            }),
            cx.observe_in(&state.service, window, |this, service, window, cx| {
                let version = service.read(cx).data_version();
                if version != this.seen_version {
                    this.seen_version = version;
                    this.reload(window, cx);
                }
            }),
        ];
        let mut view = Self {
            target,
            state,
            load: Load::Loading,
            document: None,
            siblings: Vec::new(),
            title,
            body,
            blocks: Vec::new(),
            active: None,
            block_editor,
            base_revision: 0,
            dirty: false,
            save: SaveState::Saved,
            conflict: None,
            source_mode: false,
            create_error: None,
            new_doc_open: false,
            seen_version,
            suppress_change: false,
            load_task: None,
            list_task: None,
            debounce_task: None,
            request: None,
            delete: DeleteState::Idle,
            delete_task: None,
            focus: cx.focus_handle(),
            width: Default::default(),
            toolbar_scroll: ScrollHandle::new(),
            _subscriptions,
        };
        view.reload(window, cx);
        view
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/documents/{}", self.target.id);
        self.load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = api.get::<Document>(&path).await;
            this.update_in(cx, |this, window, cx| this.apply_load(result, window, cx)).ok();
        }));
        let api = self.state.api.clone();
        let list_path = format!("/v1/documents?projectId={}", query_value(&self.target.project_id));
        self.list_task = Some(cx.spawn(async move |this, cx| {
            if let Ok(mut documents) = api.get::<Vec<Document>>(&list_path).await {
                documents.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                this.update(cx, |this, cx| {
                    this.siblings = documents;
                    cx.notify();
                })
                .ok();
            }
        }));
    }

    fn apply_load(&mut self, result: Result<Document, ApiError>, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Ok(document) => self.adopt_remote(document, window, cx),
            Err(error) if error.is_not_found() => self.load = Load::Missing,
            Err(error) if self.document.is_some() => log::warn!("document refresh failed: {error}"),
            Err(error) => self.load = Load::Failed(error.to_string()),
        }
        cx.notify();
    }

    /// Dirty drafts win: a newer server revision becomes a conflict instead of replacing edits.
    fn adopt_remote(&mut self, document: Document, window: &mut Window, cx: &mut Context<Self>) {
        self.load = Load::Ready;
        self.state.set_title(&self.target, display_title(&document.title), cx);
        if self.dirty {
            if document.revision != self.base_revision {
                self.conflict = Some(document.clone());
            }
            self.document = Some(document);
            return;
        }
        self.write_editors(&document, window, cx);
        self.base_revision = document.revision;
        self.conflict = None;
        self.save = SaveState::Saved;
        self.document = Some(document);
    }

    fn write_editors(&mut self, document: &Document, window: &mut Window, cx: &mut Context<Self>) {
        self.suppress_change = true;
        if self.title.read(cx).value().as_ref() != document.title {
            let title = document.title.clone();
            self.title.update(cx, |input, cx| input.set_value(title, window, cx));
        }
        if self.body.read(cx).value().as_ref() != document.markdown {
            let markdown = document.markdown.clone();
            self.body.update(cx, |editor, cx| editor.set_value(markdown, window, cx));
        }
        if blocks::join(&self.blocks) != document.markdown || self.blocks.is_empty() {
            self.blocks = blocks::split(&document.markdown);
            self.active = None;
        }
        self.suppress_change = false;
    }

    fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        if self.suppress_change || !matches!(self.load, Load::Ready) {
            return;
        }
        self.dirty = true;
        if self.request.is_none() {
            self.save = SaveState::Unsaved;
        }
        if self.conflict.is_none() {
            self.schedule_save(cx);
        }
        cx.notify();
    }

    /// Debounce only: the timer is separate from any request already in flight.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.debounce_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            this.update(cx, |this, cx| this.save_now(cx)).ok();
        }));
    }

    /// Starts a save unless one is running; a running save re-saves the latest draft when it ends.
    fn save_now(&mut self, cx: &mut Context<Self>) {
        self.debounce_task = None;
        if self.dirty && self.conflict.is_none() && self.request.is_none() {
            // The request runs on the executor; `self.request` keeps it alive.
            drop(self.start_save(cx));
        }
    }

    /// The PATCH body for the current draft plus the values it captured.
    fn save_request(&self, cx: &App) -> (UpdateDocument, String, String) {
        let title = self.title.read(cx).value().to_string();
        let markdown = self.markdown(cx);
        let body = UpdateDocument {
            title: Some(display_title(&title)),
            markdown: Some(markdown.clone()),
            revision: Some(self.base_revision),
            ..Default::default()
        };
        (body, title, markdown)
    }

    fn start_save(&mut self, cx: &mut Context<Self>) -> SaveRequest {
        let (body, title, markdown) = self.save_request(cx);
        let api = self.state.api.clone();
        let path = format!("/v1/documents/{}", self.target.id);
        self.save = SaveState::Saving;
        cx.notify();
        let request = cx
            .spawn(async move |this, cx| {
                let result = api.patch::<_, Document>(&path, &body).await;
                let failure = result.as_ref().err().map(|error| {
                    if error.is_conflict() { CONFLICT_MESSAGE.to_owned() } else { error.to_string() }
                });
                this.update(cx, |this, cx| this.apply_save(result, title, markdown, cx)).map_err(|e| e.to_string())?;
                failure.map_or(Ok(()), Err)
            })
            .shared();
        self.request = Some(request.clone());
        request
    }

    fn apply_save(
        &mut self,
        result: Result<Document, ApiError>,
        sent_title: String,
        sent_markdown: String,
        cx: &mut Context<Self>,
    ) {
        self.request = None;
        match result {
            Ok(document) => {
                self.base_revision = document.revision;
                let unchanged = self.title.read(cx).value().as_ref() == sent_title
                    && self.markdown(cx) == sent_markdown;
                self.dirty = !unchanged;
                self.save = if unchanged { SaveState::Saved } else { SaveState::Unsaved };
                self.state.set_title(&self.target, display_title(&document.title), cx);
                self.document = Some(document);
                if !unchanged && self.debounce_task.is_none() {
                    // The request runs on the executor; `self.request` keeps it alive.
                    drop(self.start_save(cx));
                }
                self.state.data_changed(cx);
            }
            Err(error) if error.is_conflict() => {
                self.save = SaveState::Unsaved;
                self.fetch_conflict(cx);
            }
            Err(error) => self.save = SaveState::Failed(error.to_string()),
        }
        cx.notify();
    }

    /// One step of the close-time flush: wait for the running save, then save anything newer.
    fn flush_step(&mut self, cx: &mut Context<Self>) -> Flush {
        if self.conflict.is_some() {
            return Flush::Blocked(CONFLICT_MESSAGE.into());
        }
        if let Some(request) = &self.request {
            return Flush::Wait(request.clone());
        }
        if !self.dirty {
            return Flush::Done;
        }
        self.debounce_task = None;
        Flush::Wait(self.start_save(cx))
    }

    fn delete_document(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/documents/{}", self.target.id);
        self.debounce_task = None;
        self.delete = DeleteState::Deleting;
        cx.notify();
        self.delete_task = Some(cx.spawn(async move |this, cx| {
            let result = api.delete(&path).await;
            this.update(cx, |this, cx| match result {
                Ok(()) | Err(ApiError::Status { status: 404, .. }) => {
                    this.dirty = false;
                    this.state.data_changed(cx);
                    this.state.close(&this.target, cx);
                }
                Err(error) => {
                    this.delete = DeleteState::Failed(error.to_string());
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn fetch_conflict(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/documents/{}", self.target.id);
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = api.get::<Document>(&path).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(document) => this.conflict = Some(document),
                    Err(error) => this.save = SaveState::Failed(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn discard_mine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(remote) = self.conflict.take() else { return };
        self.dirty = false;
        self.debounce_task = None;
        self.save = SaveState::Saved;
        self.adopt_remote(remote, window, cx);
        cx.notify();
    }

    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        let Some(remote) = self.conflict.take() else { return };
        self.base_revision = remote.revision;
        self.document = Some(remote);
        self.dirty = true;
        self.save = SaveState::Unsaved;
        self.save_now(cx);
    }

    /// Applies a toolbar edit to the focused block (Write) or the whole source (Source).
    /// The editor emits a change, which updates the draft and schedules the save.
    fn apply_edit(&mut self, edit: impl FnOnce(&str, std::ops::Range<usize>) -> Edit, window: &mut Window, cx: &mut Context<Self>) {
        if self.source_mode {
            let (text, selection) = {
                let editor = self.body.read(cx);
                (editor.value().to_string(), editor.selected_range())
            };
            let Edit { text, selection } = edit(&text, clamp(selection, &text));
            self.body.update(cx, |editor, cx| {
                editor.replace_all(text, window, cx);
                editor.set_selected_range(selection, cx);
                editor.focus(window, cx);
            });
            return;
        }
        if self.active.is_none() {
            self.append_block(window, cx);
        }
        let (text, selection) = {
            let editor = self.block_editor.read(cx);
            (editor.value().to_string(), editor.selected_range())
        };
        let Edit { text, selection } = edit(&text, clamp(selection, &text));
        self.block_editor.update(cx, |editor, cx| {
            editor.replace_all(text, window, cx);
            editor.set_selected_range(selection, cx);
            editor.focus(window, cx);
        });
    }

    /// Full Markdown of the draft: source text in Source mode, joined blocks otherwise.
    fn markdown(&self, cx: &App) -> String {
        if self.source_mode { self.body.read(cx).value().to_string() } else { blocks::join(&self.blocks) }
    }

    /// Leaves block editing; the edited block is re-split so a blank line, `## ` or a fence
    /// typed inside it becomes real structure. Text is unchanged, so this never dirties.
    fn commit_active(&mut self) -> Option<(usize, usize)> {
        let index = self.active.take()?;
        Some((index, blocks::resplit(&mut self.blocks, index)))
    }

    fn activate_block(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let index = match self.commit_active() {
            Some((old, count)) if index > old => index + count - 1,
            _ => index,
        };
        let Some(source) = self.blocks.get(index).map(|block| block.source.clone()) else { return };
        self.active = Some(index);
        self.suppress_change = true;
        self.block_editor.update(cx, |editor, cx| {
            editor.set_value(source.clone(), window, cx);
            editor.set_selected_range(source.len()..source.len(), cx);
            editor.focus(window, cx);
        });
        self.suppress_change = false;
        cx.notify();
    }

    /// Adds an empty paragraph after the last block and starts editing it.
    fn append_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_active();
        if let Some(last) = self.blocks.last_mut().filter(|block| !block.source.is_empty())
            && !last.trailing.ends_with("\n\n")
        {
            last.trailing = "\n\n".into();
        }
        self.blocks.push(blocks::Block { source: String::new(), trailing: String::new() });
        let index = self.blocks.len() - 1;
        self.activate_block(index, window, cx);
    }

    fn set_source_mode(&mut self, source: bool, window: &mut Window, cx: &mut Context<Self>) {
        if source == self.source_mode {
            return;
        }
        self.suppress_change = true;
        if source {
            self.commit_active();
            let markdown = blocks::join(&self.blocks);
            self.body.update(cx, |editor, cx| editor.set_value(markdown, window, cx));
        } else {
            self.blocks = blocks::split(self.body.read(cx).value().as_ref());
            self.active = None;
        }
        self.suppress_change = false;
        self.source_mode = source;
        cx.notify();
    }

    fn set_block(&mut self, block: Block, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_edit(|text, range| format::set_block(text, range, block), window, cx);
    }

    fn toggle_inline(&mut self, marker: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_edit(|text, range| format::toggle_inline(text, range, marker), window, cx);
    }

    fn new_document(&mut self, template: &'static DocTemplate, window: &mut Window, cx: &mut Context<Self>) {
        let task = create_document(self.state.clone(), self.target.project_id.clone(), template, window, cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.create_error = result.err().map(|error| format!("Couldn't create document: {error}"));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn current_block(&self, cx: &App) -> Block {
        let (text, cursor) = if self.source_mode {
            let editor = self.body.read(cx);
            (editor.value(), editor.selected_range().start)
        } else {
            let editor = self.block_editor.read(cx);
            (editor.value(), editor.selected_range().start)
        };
        format::block_at(text.as_ref(), cursor)
    }
}

impl Draft for DocView {
    fn is_dirty(&self, _: &App) -> bool {
        self.dirty || self.request.is_some()
    }

    /// Resolves once the latest draft (including edits made while saving) is stored.
    fn save(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        cx.spawn(async move |this, cx| loop {
            match this.update(cx, |this, cx| this.flush_step(cx))? {
                Flush::Done => return Ok(()),
                Flush::Blocked(message) => return Err(anyhow::anyhow!(message)),
                Flush::Wait(request) => request.await.map_err(|message| anyhow::anyhow!(message))?,
            }
        })
    }
}

fn clamp(selection: std::ops::Range<usize>, text: &str) -> std::ops::Range<usize> {
    selection.start.min(text.len())..selection.end.min(text.len())
}

fn display_title(title: &str) -> String {
    if title.trim().is_empty() { "Untitled".into() } else { title.to_owned() }
}

type MenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// `Button::dropdown_menu` equivalent whose trigger container takes `trigger_style`, with a
/// controlled open state. Kit's `DropdownMenuPopover` wraps the trigger in a content-sized
/// container it doesn't let callers style, so a flexed/full-height trigger can't fill its slot.
/// Local stand-in until the shell/shared split-button helper lands.
fn stretched_menu(
    id: &'static str,
    trigger: Button,
    trigger_style: StyleRefinement,
    open: bool,
    on_open_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
    builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl IntoElement {
    let builder: MenuBuilder = Rc::new(builder);
    Popover::new(id)
        .appearance(false)
        .overlay_closable(false)
        .anchor(Anchor::TopLeft)
        .trigger_style(trigger_style)
        .trigger(trigger)
        .open(open)
        .on_open_change(on_open_change)
        .content(move |_, window, cx| {
            let slot = window.use_keyed_state(SharedString::from(format!("{id}-menu")), cx, |_, _| None::<Entity<PopupMenu>>);
            if let Some(menu) = slot.read(cx).clone() {
                return menu;
            }
            let builder = builder.clone();
            let menu = PopupMenu::build(window, cx, move |menu, window, cx| builder(menu, window, cx));
            slot.update(cx, |slot, _| *slot = Some(menu.clone()));
            menu.focus_handle(cx).focus(window, cx);
            let popover = cx.entity().downgrade();
            let slot = slot.downgrade();
            window
                .subscribe(&menu, cx, move |_, _: &DismissEvent, window, cx| {
                    if let Some(popover) = popover.upgrade() {
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                    }
                    slot.update(cx, |slot, _| *slot = None).ok();
                })
                .detach();
            menu
        })
}

// ---- Rendering --------------------------------------------------------------------------

impl DocView {
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::tokens(cx);
        let new_button = self.render_new_doc(cx);
        let documents: Vec<Document> = self.siblings.iter().filter(|doc| doc.kind == DocumentKind::Markdown).cloned().collect();
        let items: Vec<_> = documents.iter().enumerate().map(|(index, doc)| self.render_rail_item(index, doc, cx)).collect();
        let compact = crate::views::responsive::narrower_than(&self.width, 700.);
        div()
            .flex()
            .flex_col()
            .flex_none()
            // Wide: 240 side column. Compact: the same rail (New doc + list) as a bounded
            // block above the editor, so the editor gets the full width.
            .map(|this| if compact {
                this.w_full().max_h(px(200.)).border_b_1()
            } else {
                this.w(px(metrics::SIDEBAR_W)).h_full().border_r_1()
            })
            .px_2()
            // 8 on both axes: New doc's 28 box centres on the 44 header beside it.
            .pt_2()
            .border_color(t.border)
            .child(new_button)
            .children(self.create_error.clone().map(|error| {
                div().px_2().pt_2().t_caption().text_color(t.danger).child(error)
            }))
            .child(group_label("Documents", cx))
            .child(
                div()
                    .id("doc-rail-list")
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .pb_2()
                    .overflow_y_scroll()
                    .children(items),
            )
    }

    /// Split "New doc" control (`669B3D47`, matrix U10): R8 shell, both halves open the
    /// template menu. Hover fills are flush to their segment (R7 outer corners, square seam).
    fn render_new_doc(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = crate::theme::tokens(cx);
        let view = cx.entity().downgrade();
        let menu = move |menu: gpui_kit::component::menu::PopupMenu, _: &mut Window, _: &mut Context<gpui_kit::component::menu::PopupMenu>| {
            TEMPLATES.iter().fold(menu, |menu, template| {
                let view = view.clone();
                menu.item(PopupMenuItem::new(template.name).on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| this.new_document(template, window, cx)).ok();
                }))
            })
        };
        let inner = metrics::CONTROL_MD - 2.;
        let outer_radius = 8.;
        let radius = px(metrics::inner_radius(outer_radius, 1.));
        let label = Button::new("new-doc")
            .ghost()
            .w_full()
            .h_full()
            .px_2p5()
            .rounded_l(radius)
            .rounded_r(px(0.))
            // Kit centres a button's content row; a full-width child row keeps
            // the label at the left edge.
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .t_label()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(Icon::new(Lucide::Plus).size_3p5().text_color(t.text_secondary))
                    .child("New doc"),
            );
        let open = self.new_doc_open;
        let sync = cx.entity().downgrade();
        let on_open_change = move |open: &bool, _: &mut Window, cx: &mut App| {
            let open = *open;
            sync.update(cx, |this, cx| {
                this.new_doc_open = open;
                cx.notify();
            })
            .ok();
        };
        div()
            .id("new-doc-split")
            .group("new-doc-split")
            .relative()
            .flex()
            .items_center()
            .h(px(metrics::CONTROL_MD))
            .rounded(px(outer_radius))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            // One menu for both halves, anchored to the split's left edge (S8): the label
            // half is its trigger, so it starts at the control's left with Kit's 4pt gap.
            .child(stretched_menu("new-doc-menu", label, StyleRefinement::default().flex_1().min_w_0().h(px(inner)), open, on_open_change, menu))
            // Overlaid (not in flow) so the halves meet flush and pressed/hover fills have no
            // seam; hidden while a half is hovered or the menu is open.
            .child(div().absolute().right(px(inner)).top(px((inner - 16.) / 2.)).w(px(1.)).h(px(16.)).bg(t.border).when(open, |d| d.opacity(0.)).group_hover("new-doc-split", |s| s.opacity(0.)))
            .child(
                Button::new("new-doc-templates")
                    .ghost()
                    .w(px(inner))
                    .h(px(inner))
                    .p_0()
                    .rounded_r(radius)
                    .rounded_l(px(0.))
                    .selected(open)
                    .accessibility_label("New document from a template")
                    .child(Icon::new(Lucide::ChevronDown).size_3p5().text_color(t.text_secondary))
                    .on_click(cx.listener(|this, _, _, cx| {
                        // Outside-click dismissal runs first, so opening (not toggling) avoids a reopen race.
                        this.new_doc_open = true;
                        cx.notify();
                    })),
            )
    }

    fn render_rail_item(&self, index: usize, doc: &Document, cx: &mut Context<Self>) -> AnyElement {
        let target = TabTarget::document(doc.project_id.clone(), doc.id.clone());
        let selected = doc.id == self.target.id;
        let t = crate::theme::tokens(cx);
        crate::components::list_row(("doc-rail", index), selected, cx)
            .h(px(metrics::ROW_NAV))
            .text_color(if selected { t.text } else { t.text_secondary })
            .child(gpui_kit::component::Icon::new(Lucide::FileText).size_4().flex_none().text_color(t.text_secondary))
            .child(div().flex_1().min_w_0().truncate().when(selected, |el| el.font_weight(FontWeight::MEDIUM)).child(display_title(&doc.title)))
            .on_click(cx.listener(move |this, _, window, cx| {
                if !selected {
                    this.state.open(target.clone(), window, cx);
                }
            }))
            .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::tokens(cx);
        let title = display_title(self.title.read(cx).value().as_ref());
        let view = cx.entity().downgrade();
        crate::components::header_bar(cx)
            // Header bar: "Docs" on the toolbar's block-label edge (8 bar + 8 button padding),
            // trailing 28 controls BAR_INSET from every side.
            .pl(px(crate::components::BAR_INSET + 8.))
            // Auto height with 8 vertical padding is 44 on one line (28 controls); when the
            // pane is too narrow the controls wrap below instead of clipping.
            .h_auto()
            .py(px(crate::components::BAR_INSET))
            .flex_wrap()
            .t_label()
            .child(div().text_color(t.text_secondary).child("Docs"))
            .child(gpui_kit::component::Icon::new(Lucide::ChevronRight).size_3p5().text_color(t.text_tertiary))
            .child(div().min_w_0().flex_shrink(1.).font_weight(FontWeight::MEDIUM).text_color(t.text).truncate().max_w(px(360.)).child(title))
            .child(self.render_save_pill(cx))
            .child(div().flex_1())
            .child(
                toolbar_icon(Button::new("delete-doc"), Lucide::Trash, cx)
                    .tooltip("Delete document")
                    .accessibility_label("Delete document")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.delete = DeleteState::Confirming;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("doc-mode")
                    .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Formatted editing, or the Markdown source (⌘⇧P)").build(window, cx))
                    .child(segmented(
                        "doc-mode-items",
                        vec![Segment::new("Write"), Segment::new("Markdown")],
                        usize::from(self.source_mode),
                        move |index, window, cx| {
                            view.update(cx, |this, cx| this.set_source_mode(index == 1, window, cx)).ok();
                        },
                        cx,
                    )),
            )
    }

    fn render_save_pill(&self, cx: &App) -> impl IntoElement {
        // Same state the close prompt uses (`Draft::is_dirty`): never "Saved" while edits are pending.
        let t = crate::theme::tokens(cx);
        let (label, color) = if self.request.is_some() {
            ("Saving…", t.text_secondary)
        } else if matches!(self.save, SaveState::Failed(_)) {
            ("Save failed", t.danger)
        } else if self.dirty {
            ("Unsaved changes", t.attention)
        } else {
            ("Saved", t.text_secondary)
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .h(px(metrics::CHIP_H))
            .px_2()
            .rounded_full()
            .bg(t.tray)
            .t_caption()
            .text_color(color)
            .child(div().size(px(6.)).rounded_full().bg(color))
            .child(label)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let block = self.current_block(cx);
        let view = cx.entity().downgrade();
        let block_menu = button_sm(Button::new("block-type").ghost().label(block.label()))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                Block::MENU.iter().fold(menu, |menu, item| {
                    let view = view.clone();
                    let item = *item;
                    menu.item(PopupMenuItem::new(item.label()).checked(item == block).on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| this.set_block(item, window, cx)).ok();
                    }))
                })
            });
        let border = crate::theme::tokens(cx).border;
        let separator = move || div().w(px(1.)).h(px(16.)).mx_1p5().bg(border);
        let row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_0p5()
            .h(px(40.))
            // The 24 block menu sits 8 from the top, bottom and left edge; its 8pt inner
            // padding puts the label on the header's text edge.
            .px_2()
            .child(block_menu)
            .child(separator())
            .child(self.tool("bold", Lucide::Bold, "Bold (⌘B)", cx, |this, w, cx| this.toggle_inline("**", w, cx)))
            .child(self.tool("italic", Lucide::Italic, "Italic (⌘I)", cx, |this, w, cx| this.toggle_inline("_", w, cx)))
            .child(self.tool("strike", Lucide::Strikethrough, "Strikethrough", cx, |this, w, cx| this.toggle_inline("~~", w, cx)))
            .child(self.tool("code", Lucide::Code, "Inline code (⌘E)", cx, |this, w, cx| this.toggle_inline("`", w, cx)))
            .child(separator())
            .child(self.tool("bullets", Lucide::List, "Bulleted list", cx, |this, w, cx| this.set_block(Block::Bullet, w, cx)))
            .child(self.tool("numbers", Lucide::ListOrdered, "Numbered list", cx, |this, w, cx| this.set_block(Block::Numbered, w, cx)))
            .child(self.tool("checks", Lucide::ListChecks, "Checklist", cx, |this, w, cx| this.set_block(Block::Checklist, w, cx)))
            .child(self.tool("quote", Lucide::Quote, "Quote", cx, |this, w, cx| this.set_block(Block::Quote, w, cx)))
            .child(separator())
            .child(self.tool("link", Lucide::Link, "Link (⌘K)", cx, |this, w, cx| this.apply_edit(format::insert_link, w, cx)))
            .child(self.tool("table", Lucide::Table, "Table", cx, |this, w, cx| this.apply_edit(format::insert_table, w, cx)));
        // Narrow panes scroll the toolbar sideways (every tool stays reachable), with fades.
        let surface = crate::theme::tokens(cx).surface;
        div()
            .relative()
            .flex_none()
            .w_full()
            .min_w_0()
            .border_b_1()
            .border_color(border)
            .child(div().id("doc-toolbar").w_full().flex().overflow_x_scroll().track_scroll(&self.toolbar_scroll).child(row))
            .children(crate::views::responsive::x_scroll_fades(&self.toolbar_scroll, surface))
    }

    fn tool(
        &self,
        id: &'static str,
        icon: Lucide,
        tooltip: &'static str,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        toolbar_icon(Button::new(id), icon, cx)
            .tooltip(tooltip)
            .accessibility_label(tooltip)
            .on_click(cx.listener(move |this, _, window, cx| action(this, window, cx)))
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let t = crate::theme::tokens(cx);
        let delete_message = match &self.delete {
            DeleteState::Idle => None,
            DeleteState::Confirming => Some(format!("Delete “{}”? This can't be undone.", display_title(self.title.read(cx).value().as_ref()))),
            DeleteState::Deleting => Some("Deleting…".to_owned()),
            DeleteState::Failed(error) => Some(format!("Couldn't delete: {error}")),
        };
        if let Some(message) = delete_message {
            let busy = self.delete == DeleteState::Deleting;
            return Some(
                banner(message, t.danger, cx)
                    .child(button_sm(Button::new("delete-cancel").outline().label("Cancel")).disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.delete = DeleteState::Idle;
                            cx.notify();
                        })))
                    .child(button_sm(Button::new("delete-confirm").danger().label("Delete")).loading(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.delete_document(cx)))),
            );
        }
        if let Some(remote) = &self.conflict {
            let message = format!("Changed elsewhere {}. Your edits are kept until you choose.", relative_time(&remote.updated_at));
            return Some(
                banner(message, crate::theme::tokens(cx).attention, cx)
                    .child(button_sm(Button::new("conflict-reload").outline().label("Reload"))
                        .tooltip("Discard your edits and load the latest version")
                        .on_click(cx.listener(|this, _, window, cx| this.discard_mine(window, cx))))
                    .child(button_sm(Button::new("conflict-keep").primary().label("Keep mine"))
                        .tooltip("Save your version over the latest one")
                        .on_click(cx.listener(|this, _, _, cx| this.keep_mine(cx)))),
            );
        }
        let SaveState::Failed(error) = &self.save else { return None };
        Some(
            banner(format!("Couldn't save: {error}"), crate::theme::tokens(cx).danger, cx).child(
                button_sm(Button::new("save-retry").outline().label("Retry")).on_click(cx.listener(|this, _, _, cx| {
                    this.save = SaveState::Unsaved;
                    this.save_now(cx);
                })),
            ),
        )
    }

    fn render_meta(&self, cx: &App) -> impl IntoElement {
        let words = format::word_count(&self.markdown(cx));
        let minutes = words.div_ceil(220).max(1);
        let edited = self
            .document
            .as_ref()
            .map(|doc| format!(" · Edited {}", relative_time(&doc.updated_at)))
            .unwrap_or_default();
        div()
            .t_label()
            .text_color(crate::theme::tokens(cx).text_secondary)
            .child(format!("{words} words · {minutes} min read{edited}"))
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        // The role goes on the Kit Input itself: Input applies its size class's text size
        // and line height, and only its own refinement overrides them.
        let title = Input::new(&self.title)
            .appearance(false)
            .t_doc_title()
            .h(px(34.))
            .px_0()
            .aria_label("Document title");
        // Title, meta and every block share one left edge: the column's padding plus the
        // same 8pt block inset (blocks need it for their hover fill). The header is the same
        // element in both modes, so switching never moves it.
        let edge = metrics::GUTTER_X + BLOCK_INSET;
        // Kit's Markdown editor draws its text after a gutter (gap + fold controls). The
        // source column starts that much further left, so its text lands on the same edge.
        // The measurement comes from the editor's last layout (see `render`).
        let gutter = if self.source_mode {
            self.body.read(cx).text_origin_offset().map_or(0., f32::from).min(edge)
        } else {
            0.
        };
        let header = div()
            .flex()
            .flex_col()
            .pl(px(BLOCK_INSET + gutter))
            .pr(px(BLOCK_INSET))
            .pb_7()
            .child(div().w_full().t_doc_title().child(title))
            .child(div().pt_1p5().child(self.render_meta(cx)));
        let column = div()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(DOC_MEASURE + 2. * edge))
            .mx_auto()
            .pl(px(metrics::GUTTER_X - gutter))
            .pr(px(metrics::GUTTER_X))
            .pt_10()
            .child(header);
        if self.source_mode {
            return div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(
                    column.h_full().child(
                        div().flex_1().min_h_0().child(Editor::new(&self.body).appearance(false).h_full().aria_label("Markdown source")),
                    ),
                )
                .into_any_element();
        }
        div()
            .id("doc-write")
            .flex_1()
            .overflow_y_scroll()
            .child(
                column
                    .pb_16()
                    .child(div().flex().flex_col().gap_2p5().children(self.render_blocks(cx)))
                    .child(self.render_add_block(cx)),
            )
            .into_any_element()
    }

    /// Formatted blocks; the focused one is edited in place. Read and edit use the same role,
    /// inset and padding per block kind, so text keeps its size, baseline and left edge when
    /// a block is clicked. Headings are read through their inline Markdown at the heading
    /// role (the `#` marker is dropped for display only); Kit's heading node would apply its
    /// own line height. A leading `# Title` that repeats the document title is left to
    /// Markdown mode so the page shows one H1.
    fn render_blocks(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = crate::theme::tokens(cx);
        let title = self.title.read(cx).value().trim().to_owned();
        let mut elements = Vec::new();
        for (index, block) in self.blocks.iter().enumerate() {
            let editing = self.active == Some(index);
            let repeats_title = index == first_content(&self.blocks)
                && block.source.trim_start().strip_prefix("# ").is_some_and(|text| text.trim() == title);
            if !editing && (block.source.trim().is_empty() || repeats_title) {
                continue;
            }
            let kind = block.kind();
            let shell = block_role(
                div().px(px(BLOCK_INSET)).py_0p5().rounded(px(metrics::R_CONTROL)),
                &kind,
                cx,
            )
            .text_color(t.text)
            .when(matches!(kind, blocks::BlockKind::Heading(_)) && index != first_content(&self.blocks), |el| el.mt_3p5());
            let element = if editing {
                let editor = block_role(Textarea::new(&self.block_editor).appearance(false).px_0().py_0(), &kind, cx).aria_label("Block");
                shell.bg(t.tray).child(editor).into_any_element()
            } else {
                let source: SharedString = match kind {
                    blocks::BlockKind::Heading(level) => block.source.trim_start()[level as usize..].trim().to_owned().into(),
                    _ => read_source(&block.source).into(),
                };
                shell
                    .id(("doc-block", index))
                    .cursor_text()
                    .hover(move |style| style.bg(t.hover))
                    .child(TextView::markdown(("doc-block-text", index), source).style(crate::components::typography::doc_text_view_style()))
                    .on_click(cx.listener(move |this, _, window, cx| this.activate_block(index, window, cx)))
                    .into_any_element()
            };
            elements.push(element);
        }
        elements
    }

    fn render_add_block(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::tokens(cx);
        let empty = self.blocks.iter().all(|block| block.source.trim().is_empty()) && self.active.is_none();
        div()
            .id("doc-add-block")
            .min_h(px(if empty { 120. } else { 56. }))
            .px(px(BLOCK_INSET))
            .pt_2()
            .t_prose()
            .text_color(t.text_tertiary)
            .cursor_text()
            .child(if empty { "Start writing…" } else { "" })
            .on_click(cx.listener(|this, _, window, cx| this.append_block(window, cx)))
    }

    fn render_code_tour_redirect(&self, cx: &mut Context<Self>) -> AnyElement {
        let target = TabTarget::review(self.target.project_id.clone(), self.target.id.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(crate::components::feature_icon("git-pull-request", px(36.)))
            .child(div().t_body().text_color(crate::theme::tokens(cx).text).child("This document is a code tour."))
            .child(button_md(Button::new("open-review").primary().label("Open review")).on_click(
                cx.listener(move |this, _, window, cx| this.state.replace_active(target.clone(), window, cx)),
            ))
            .into_any_element()
    }
}

/// Index of the first block with content (a leading-blank-lines block is skipped).
fn first_content(blocks: &[blocks::Block]) -> usize {
    blocks.iter().position(|block| !block.source.trim().is_empty()).unwrap_or(0)
}

/// Reading measure of the document column (`669B3D47`).
const DOC_MEASURE: f32 = 640.;
/// Text inset inside each block, so its hover/edit fill has room around the text.
const BLOCK_INSET: f32 = 8.;

/// Toolbar icon button (`669B3D47`): 28 square, a 14pt secondary glyph. The icon is a child
/// rather than `Button::icon`, which would resize it to 0.75 × the button.
fn toolbar_icon(button: Button, icon: Lucide, cx: &App) -> Button {
    let t = crate::theme::tokens(cx);
    button
        .ghost()
        .w(px(metrics::CONTROL_MD))
        .h(px(metrics::CONTROL_MD))
        .p_0()
        .child(Icon::new(icon).size_3p5().text_color(t.text_secondary))
}

/// Read-view source for a block: an empty task item ("- [ ]") gets a zero-width space so
/// Markdown parses it as a checkbox instead of literal brackets. Display only.
fn read_source(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let body = line.trim_start().trim_end();
            let empty_task = ["- [ ]", "* [ ]", "+ [ ]", "- [x]", "* [x]", "+ [x]", "- [X]", "* [X]", "+ [X]"].contains(&body);
            if empty_task { format!("{} \u{200B}", line.trim_end()) } else { line.to_owned() }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text role for a block kind, identical for the read view and the block editor.
fn block_role<E: Styled>(el: E, kind: &blocks::BlockKind, cx: &App) -> E {
    match kind {
        blocks::BlockKind::Heading(1) => el.t_doc_title(),
        blocks::BlockKind::Heading(2) => el.t_doc_h2(),
        blocks::BlockKind::Heading(3) => el.t_doc_h3(),
        blocks::BlockKind::Heading(_) => el.t_prose().font_weight(FontWeight::SEMIBOLD),
        blocks::BlockKind::Code => el.t_code(cx),
        blocks::BlockKind::Other => el.t_prose(),
    }
}

impl Render for DocView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The source column aligns with Kit's measured text origin, which exists only after
        // the editor's first layout. Until then, ask for one more frame; once measured, each
        // render uses the latest value, so a metric change settles on the next render.
        let editor_shown = self.source_mode
            && matches!(self.load, Load::Ready)
            && !self.document.as_ref().is_some_and(|doc| doc.kind == DocumentKind::CodeTour);
        if editor_shown && self.body.read(cx).text_origin_offset().is_none() {
            window.request_animation_frame();
        }
        let content: AnyElement = match &self.load {
            Load::Loading => empty_state("Loading document…", "").into_any_element(),
            Load::Missing => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(empty_state("Document unavailable", "It was deleted or no longer exists."))
                .child(button_md(Button::new("close-missing").outline().label("Close tab")).on_click(
                    cx.listener(|this, _, _, cx| this.state.close(&this.target, cx)),
                ))
                .into_any_element(),
            Load::Failed(error) => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(empty_state("Couldn't load document", error.clone()))
                .child(button_md(Button::new("retry-load").outline().label("Retry")).on_click(cx.listener(|this, _, window, cx| {
                    this.load = Load::Loading;
                    this.reload(window, cx);
                })))
                .into_any_element(),
            Load::Ready if self.document.as_ref().is_some_and(|doc| doc.kind == DocumentKind::CodeTour) => {
                self.render_code_tour_redirect(cx)
            }
            Load::Ready => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(self.render_header(cx))
                .children(self.render_banner(cx))
                .child(self.render_toolbar(cx))
                .child(self.render_body(cx))
                .into_any_element(),
        };
        div()
            .id("doc-view")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .when(crate::views::responsive::narrower_than(&self.width, 700.), |this| this.flex_col())
            .on_action(cx.listener(|this, _: &Save, _, cx| this.save_now(cx)))
            .on_action(cx.listener(|this, _: &ToggleBold, window, cx| this.toggle_inline("**", window, cx)))
            .on_action(cx.listener(|this, _: &ToggleItalic, window, cx| this.toggle_inline("_", window, cx)))
            .on_action(cx.listener(|this, _: &ToggleCode, window, cx| this.toggle_inline("`", window, cx)))
            .on_action(cx.listener(|this, _: &InsertLink, window, cx| this.apply_edit(format::insert_link, window, cx)))
            .on_action(cx.listener(|this, _: &TogglePreview, window, cx| {
                let source = !this.source_mode;
                this.set_source_mode(source, window, cx);
            }))
            .child(crate::views::responsive::width_probe(&self.width))
            .child(self.render_rail(cx))
            // One owner for the remaining space in both orientations (rail beside or above).
            .child(div().flex_1().min_w_0().min_h_0().flex().child(content))
    }
}
