//! Review tab for a `code_tour` document: Overview / Guide / Timeline / Diff.
//! Review progress (reviewed chapters/files, decision) persists in the tour's `review` field.

mod diagram;
mod diff_tab;
mod files;
mod guide;
mod overview;
mod timeline;

use std::collections::{HashMap, HashSet};

use futures::{FutureExt as _, future::Shared};

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    diff::{DiffFile, DiffParseOptions, DiffState},
    input::{InputEvent, TextareaState},
};
use gpui_kit::*;
use protocol::{CodeTour, Document, DocumentKind, ReviewDecision, TourReview, UpdateDocument};

use crate::{
    api::ApiError,
    components::{Edge, Segment, Typography as _, button_md, button_sm, edge_band, empty_state, error_state, loading_state, segmented},
    state::{AppState, Draft, TabTarget},
    theme::{self, metrics},
};
use super::stats::common::relative_time;

pub fn tab(target: &TabTarget, state: AppState, window: &mut Window, cx: &mut App) -> AnyView {
    let view = cx.new(|cx| ReviewView::new(target.clone(), state.clone(), window, cx));
    state.track_draft(target, &view, cx);
    view.into()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReviewTab {
    Overview,
    Guide,
    Timeline,
    Diff,
}

impl ReviewTab {
    const ALL: [ReviewTab; 4] = [ReviewTab::Overview, ReviewTab::Guide, ReviewTab::Timeline, ReviewTab::Diff];

    fn label(self) -> &'static str {
        match self {
            ReviewTab::Overview => "Overview",
            ReviewTab::Guide => "Guide",
            ReviewTab::Timeline => "Timeline",
            ReviewTab::Diff => "Diff",
        }
    }
}

enum Load {
    Loading,
    Ready,
    Missing,
    Failed(String),
}

type SaveRequest = Shared<Task<Result<(), String>>>;

const CONFLICT_MESSAGE: &str = "This review changed elsewhere. Choose Reload or Keep mine first.";

enum Flush {
    Done,
    Blocked(String),
    Wait(SaveRequest),
}

type DiffCache = HashMap<String, Result<Entity<DiffState>, String>>;

pub struct ReviewView {
    target: TabTarget,
    state: AppState,
    load: Load,
    document: Option<Document>,
    review: TourReview,
    base_revision: u64,
    dirty: bool,
    request: Option<SaveRequest>,
    save_error: Option<String>,
    conflict: Option<Document>,
    tab: ReviewTab,
    chapter: usize,
    focused_block: Option<String>,
    expanded_groups: HashSet<String>,
    selected_commit: Option<String>,
    split_diff: bool,
    diff_scope: Option<diff_tab::DiffScope>,
    diff_scroll: ScrollHandle,
    diffs: DiffCache,
    guide_scroll: ScrollHandle,
    summary: Entity<TextareaState>,
    summary_synced: bool,
    summary_debounce: Option<Task<()>>,
    seen_version: u64,
    load_task: Option<Task<()>>,
    /// Allocated body width (inspector/sidebar change it); drives stacked layouts.
    body_width: crate::views::responsive::WidthCell,
    diagram_scroll: ScrollHandle,
    rail_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

fn empty_review() -> TourReview {
    TourReview {
        reviewed_chapter_ids: Vec::new(),
        reviewed_file_paths: Vec::new(),
        decision: ReviewDecision::Pending,
        summary: String::new(),
        extra: Default::default(),
    }
}

/// Parses a stored patch for the Kit diff viewer; malformed patches become an error message.
fn parse_patch(patch: &str) -> Result<Vec<DiffFile>, String> {
    let files = DiffFile::parse_with(patch, DiffParseOptions::new().truncated_hunks(true))
        .map_err(|error| format!("This patch couldn't be parsed ({error})."))?;
    if files.is_empty() {
        return Err("This patch has no file changes.".into());
    }
    Ok(files)
}

impl ReviewView {
    fn new(target: TabTarget, state: AppState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let seen_version = state.service.read(cx).data_version();
        let summary = cx.new(|cx| {
            TextareaState::new(window, cx).auto_grow(2, 8).placeholder("Feedback for the author (saved with your decision)")
        });
        let _subscriptions = vec![
            cx.observe_in(&state.service, window, |this, service, _, cx| {
                let version = service.read(cx).data_version();
                if version != this.seen_version {
                    this.seen_version = version;
                    this.reload(cx);
                }
            }),
            cx.subscribe_in(&summary, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && this.summary_synced {
                    this.summary_edited(cx);
                }
            }),
        ];
        let mut view = Self {
            target,
            state,
            load: Load::Loading,
            document: None,
            review: empty_review(),
            base_revision: 0,
            dirty: false,
            request: None,
            save_error: None,
            conflict: None,
            tab: ReviewTab::Overview,
            chapter: 0,
            focused_block: None,
            expanded_groups: HashSet::new(),
            selected_commit: None,
            split_diff: false,
            diff_scope: None,
            diff_scroll: ScrollHandle::new(),
            diffs: HashMap::new(),
            guide_scroll: ScrollHandle::new(),
            summary,
            summary_synced: false,
            summary_debounce: None,
            seen_version,
            load_task: None,
            body_width: Default::default(),
            diagram_scroll: ScrollHandle::new(),
            rail_scroll: ScrollHandle::new(),
            _subscriptions,
        };
        view.reload(cx);
        view
    }

    fn tour(&self) -> Option<&CodeTour> {
        self.document.as_ref()?.tour.as_ref()
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/documents/{}", self.target.id);
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = api.get::<Document>(&path).await;
            this.update(cx, |this, cx| this.apply_load(result, cx)).ok();
        }));
    }

    fn apply_load(&mut self, result: Result<Document, ApiError>, cx: &mut Context<Self>) {
        match result {
            Ok(document) => self.adopt(document, cx),
            Err(error) if error.is_not_found() => self.load = Load::Missing,
            Err(error) if self.document.is_some() => log::warn!("review refresh failed: {error}"),
            Err(error) => self.load = Load::Failed(error.to_string()),
        }
        cx.notify();
    }

    /// Unsaved review flags win; a newer server revision becomes a conflict.
    fn adopt(&mut self, document: Document, cx: &mut Context<Self>) {
        self.load = Load::Ready;
        self.state.set_title(&self.target, document.title.clone(), cx);
        if self.dirty || self.request.is_some() {
            if document.revision != self.base_revision && self.request.is_none() {
                self.conflict = Some(document);
            }
            return;
        }
        if self.document.as_ref().is_none_or(|current| current.tour != document.tour) {
            self.diffs.clear();
        }
        self.review = document.tour.as_ref().and_then(|tour| tour.review.clone()).unwrap_or_else(empty_review);
        self.summary_synced = false;
        self.base_revision = document.revision;
        self.conflict = None;
        let chapters = document.tour.as_ref().map_or(0, |tour| tour.chapters.len());
        self.chapter = self.chapter.min(chapters.saturating_sub(1));
        self.document = Some(document);
    }

    // ---- Review state -----------------------------------------------------------------

    fn is_chapter_reviewed(&self, id: &str) -> bool {
        self.review.reviewed_chapter_ids.iter().any(|reviewed| reviewed == id)
    }

    fn is_file_reviewed(&self, path: &str) -> bool {
        self.review.reviewed_file_paths.iter().any(|reviewed| reviewed == path)
    }

    fn set_chapter_reviewed(&mut self, id: &str, reviewed: bool, cx: &mut Context<Self>) {
        toggle_member(&mut self.review.reviewed_chapter_ids, id, reviewed);
        self.review_changed(cx);
    }

    fn set_file_reviewed(&mut self, path: &str, reviewed: bool, cx: &mut Context<Self>) {
        toggle_member(&mut self.review.reviewed_file_paths, path, reviewed);
        self.review_changed(cx);
    }

    fn summary_edited(&mut self, cx: &mut Context<Self>) {
        self.review.summary = self.summary.read(cx).value().to_string();
        self.dirty = true;
        self.summary_debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(900)).await;
            this.update(cx, |this, cx| this.review_changed(cx)).ok();
        }));
        cx.notify();
    }

    /// Writes the adopted review summary into the editor (needs a window, so done at render).
    fn sync_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.summary_synced {
            return;
        }
        let summary = self.review.summary.clone();
        if self.summary.read(cx).value().as_ref() != summary {
            self.summary.update(cx, |input, cx| input.set_value(summary, window, cx));
        }
        self.summary_synced = true;
    }

    fn set_decision(&mut self, decision: ReviewDecision, cx: &mut Context<Self>) {
        self.review.decision = decision;
        self.review_changed(cx);
    }

    /// Marks the review dirty and saves; a save already running picks up newer marks when it ends.
    fn review_changed(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.save_error = None;
        if self.request.is_none() && self.conflict.is_none() {
            // The request runs on the executor; `self.request` keeps it alive.
            drop(self.start_save(cx));
        }
        cx.notify();
    }

    fn start_save(&mut self, cx: &mut Context<Self>) -> SaveRequest {
        let mut tour = self.tour().cloned().unwrap_or_default();
        tour.review = Some(self.review.clone());
        let sent = self.review.clone();
        let body = UpdateDocument { tour: Some(Some(tour)), revision: Some(self.base_revision), ..Default::default() };
        let api = self.state.api.clone();
        let path = format!("/v1/documents/{}", self.target.id);
        let request = cx
            .spawn(async move |this, cx| {
                let result = api.patch::<_, Document>(&path, &body).await;
                let failure = result.as_ref().err().map(|error| {
                    if error.is_conflict() { CONFLICT_MESSAGE.to_owned() } else { error.to_string() }
                });
                this.update(cx, |this, cx| this.apply_save(result, sent, cx)).map_err(|e| e.to_string())?;
                failure.map_or(Ok(()), Err)
            })
            .shared();
        self.request = Some(request.clone());
        request
    }

    fn apply_save(&mut self, result: Result<Document, ApiError>, sent: TourReview, cx: &mut Context<Self>) {
        self.request = None;
        match result {
            Ok(document) => {
                self.base_revision = document.revision;
                self.dirty = self.review != sent;
                self.document = Some(document);
                self.state.data_changed(cx);
                if self.dirty {
                    // The request runs on the executor; `self.request` keeps it alive.
                    drop(self.start_save(cx));
                }
            }
            Err(error) if error.is_conflict() => self.fetch_conflict(cx),
            Err(error) => self.save_error = Some(error.to_string()),
        }
        cx.notify();
    }

    /// One step of the close-time flush: wait for the running save, then save newer marks.
    fn flush_step(&mut self, cx: &mut Context<Self>) -> Flush {
        self.summary_debounce = None;
        if self.conflict.is_some() {
            return Flush::Blocked(CONFLICT_MESSAGE.into());
        }
        if let Some(request) = &self.request {
            return Flush::Wait(request.clone());
        }
        if !self.dirty {
            return Flush::Done;
        }
        Flush::Wait(self.start_save(cx))
    }

    fn fetch_conflict(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/documents/{}", self.target.id);
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = api.get::<Document>(&path).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(document) => this.conflict = Some(document),
                    Err(error) => this.save_error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn reload_theirs(&mut self, cx: &mut Context<Self>) {
        let Some(remote) = self.conflict.take() else { return };
        self.dirty = false;
        self.summary_debounce = None;
        self.adopt(remote, cx);
        cx.notify();
    }

    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        let Some(remote) = self.conflict.take() else { return };
        self.base_revision = remote.revision;
        self.document = Some(remote);
        self.diffs.clear();
        self.review_changed(cx);
    }

    // ---- Navigation ---------------------------------------------------------------------

    fn show_chapter(&mut self, index: usize, cx: &mut Context<Self>) {
        self.tab = ReviewTab::Guide;
        self.chapter = index;
        self.focused_block = None;
        self.guide_scroll.scroll_to_item(0);
        cx.notify();
    }

    fn show_commit(&mut self, sha: &str, cx: &mut Context<Self>) {
        self.tab = ReviewTab::Timeline;
        self.selected_commit = Some(sha.to_owned());
        if let Some(group) = self.tour().and_then(|tour| {
            tour.commit_groups.iter().flatten().find(|group| group.commit_shas.iter().any(|s| s == sha))
        }) {
            self.expanded_groups.insert(group.id.clone());
        }
        cx.notify();
    }

    /// Cached Kit diff state for a patch; parse failures are cached as messages.
    fn diff_state(&mut self, key: &str, patch: &str, cx: &mut Context<Self>) -> Result<Entity<DiffState>, String> {
        if let Some(entry) = self.diffs.get(key) {
            return entry.clone();
        }
        let entry = parse_patch(patch).map(|files| cx.new(|cx| DiffState::new(files, cx)));
        self.diffs.insert(key.to_owned(), entry.clone());
        entry
    }
}

fn toggle_member(list: &mut Vec<String>, value: &str, present: bool) {
    list.retain(|item| item != value);
    if present {
        list.push(value.to_owned());
    }
}

impl Draft for ReviewView {
    fn is_dirty(&self, _: &App) -> bool {
        self.dirty || self.request.is_some()
    }

    /// Resolves once the latest review marks (including ones made while saving) are stored.
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

// ---- Rendering --------------------------------------------------------------------------

impl ReviewView {
    fn render_header(&self, document: &Document, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let selected = ReviewTab::ALL.iter().position(|tab| *tab == self.tab).unwrap_or(0);
        let status = if self.request.is_some() {
            "Saving review…".to_owned()
        } else if self.dirty {
            "Unsaved review changes".to_owned()
        } else {
            format!("Updated {}", relative_time(&document.updated_at))
        };
        let view = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .gap_3()
            // The 28 icon tile and the segmented track sit on the 24 gutter, so the header
            // keeps 24 above and below them too.
            .px(px(metrics::GUTTER_X))
            .py_6()
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .child(crate::components::feature_icon("git-pull-request", px(28.)))
                    .child(div().flex_1().min_w_0().truncate().t_title().text_color(t.text).child(document.title.clone()))
                    .child(div().flex_none().t_caption().text_color(t.text_secondary).child(status)),
            )
            // The track keeps its content width; a flex column would stretch it.
            .child(div().flex().child(segmented(
                "review-tabs",
                ReviewTab::ALL.iter().map(|tab| Segment::new(tab.label())).collect(),
                selected,
                move |index, _, cx| {
                    view.update(cx, |this, cx| {
                        this.tab = ReviewTab::ALL[index];
                        cx.notify();
                    })
                    .ok();
                },
                cx,
            )))
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = theme::tokens(cx);
        let (message, color, actions): (String, Hsla, Vec<AnyElement>) = if let Some(remote) = &self.conflict {
            (
                format!("This tour changed elsewhere {}. Your review marks are kept until you choose.", relative_time(&remote.updated_at)),
                t.attention,
                vec![
                    button_sm(Button::new("review-reload").outline().label("Reload"))
                        .on_click(cx.listener(|this, _, _, cx| this.reload_theirs(cx))).into_any_element(),
                    button_sm(Button::new("review-keep").primary().label("Keep mine"))
                        .on_click(cx.listener(|this, _, _, cx| this.keep_mine(cx))).into_any_element(),
                ],
            )
        } else if let Some(error) = &self.save_error {
            (
                format!("Couldn't save review: {error}"),
                t.danger,
                vec![button_sm(Button::new("review-retry").outline().label("Retry"))
                    .on_click(cx.listener(|this, _, _, cx| this.review_changed(cx))).into_any_element()],
            )
        } else {
            return None;
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .pl(px(metrics::GUTTER_X))
                .pr_2()
                .py_2()
                .border_b_1()
                .border_color(t.border)
                .bg(color.opacity(0.10))
                .t_label()
                .text_color(t.text)
                .child(div().size(px(6.)).flex_none().rounded_full().bg(color))
                .child(div().flex_1().child(message))
                .children(actions)
                .into_any_element(),
        )
    }

    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.tab {
            ReviewTab::Overview => self.render_overview(cx).into_any_element(),
            ReviewTab::Guide => self.render_guide(cx).into_any_element(),
            ReviewTab::Timeline => self.render_timeline(window, cx).into_any_element(),
            ReviewTab::Diff => self.render_diff_tab(cx),
        }
    }
}

impl Render for ReviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_summary(window, cx);
        let t = theme::tokens(cx);
        let root = div().size_full().flex().flex_col().bg(t.canvas);
        match &self.load {
            Load::Loading => return root.child(loading_state("Loading review…", cx)),
            Load::Missing => {
                return root.child(empty_state("Code tour not found", "It may have been deleted. Close this tab to continue."));
            }
            Load::Failed(error) => {
                let error = error.clone();
                return root.child(error_state(error, cx.listener(|this, _, _, cx| {
                    this.load = Load::Loading;
                    this.reload(cx);
                })));
            }
            Load::Ready => {}
        }
        let Some(document) = self.document.clone() else {
            return root.child(loading_state("Loading review…", cx));
        };
        if document.kind != DocumentKind::CodeTour || document.tour.is_none() {
            let target = TabTarget::document(document.project_id.clone(), document.id.clone());
            return root.child(
                div().size_full().flex().flex_col().items_center().justify_center().gap_2()
                    .child(empty_state("Not a code tour", "This document has no tour content. Open it as a document instead."))
                    .child(button_md(Button::new("open-as-doc").outline().label("Open document")).on_click(
                        cx.listener(move |this, _, window, cx| this.state.replace_active(target.clone(), window, cx)),
                    )),
            );
        }
        root.child(self.render_header(&document, cx))
            .children(self.render_banner(cx))
            .child(div().relative().flex_1().min_h_0()
                .child(crate::views::responsive::width_probe(&self.body_width))
                .child(self.render_body(window, cx)))
    }
}

/// Shared by sub-views: a bordered card. GPUI clips rectangles only, so children that
/// paint at its top or bottom edge use `edge_band`, and scroll bodies stay clear of the arcs.
fn panel(cx: &App) -> Div {
    crate::components::card(cx).flex().flex_col()
}

/// Card header band: semibold title with optional right-aligned caption, hairline below.
fn panel_title(title: impl Into<SharedString>, trailing: Option<SharedString>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    edge_band(Edge::Top, metrics::R_CARD)
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .h(px(40.))
        .px_4()
        .border_b_1()
        .border_color(t.border)
        .child(div().min_w_0().truncate().t_panel_title().text_color(t.text).child(title.into()))
        .children(trailing.map(|text| div().flex_none().t_caption().text_color(t.text_secondary).child(text)))
}
