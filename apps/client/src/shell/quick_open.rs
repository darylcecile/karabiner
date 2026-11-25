//! Quick Open (⌘T / `+`): search projects, tasks, documents and reviews and open one as a tab.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Icon, WindowExt as _,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{Document, DocumentKind, Project, TabKind, Task as TaskRecord};

use crate::{
    components::Typography as _,
    api::ApiError,
    components::{empty_state, loading_state},
    shell::titlebar::tab_icon,
    state::{AppState, TabTarget},
    theme::{self, metrics},
};

const MAX_RESULTS: usize = 50;
/// Reference metrics (`0945F5E8`): header and footer heights, results height range.
const HEADER_H: f32 = 64.;
const FOOTER_H: f32 = 48.;
const RESULTS_MAX: f32 = 440.;
const RESULTS_MIN: f32 = 120.;
const MARGIN_TOP: f32 = metrics::TITLEBAR_H + 80.;
/// Space kept below the dialog (Kit's dialog edge margin is similar).
const MARGIN_BOTTOM: f32 = 24.;

/// Results height that keeps the whole palette inside a short window. Kit's dialog clamps
/// its width to the viewport and snaps an overflowing dialog up to its edge margin, but
/// doesn't shrink a fixed-height child, so the list owns this clamp. Text never shrinks.
fn results_max_height(viewport_h: f32) -> f32 {
    let room = viewport_h - MARGIN_TOP - MARGIN_BOTTOM - HEADER_H - FOOTER_H - 2.;
    room.clamp(RESULTS_MIN, RESULTS_MAX)
}

#[derive(Clone)]
struct Entry {
    target: TabTarget,
    title: SharedString,
    context: SharedString,
}

pub struct QuickOpen {
    state: AppState,
    query: Entity<InputState>,
    entries: Option<Result<Vec<Entry>, ApiError>>,
    selected: usize,
    /// Tracks the results list; its children are section labels and rows.
    results: ScrollHandle,
    /// Last pointer position seen over a row. Scrolling the list under a still pointer
    /// can deliver a move at the same position; only real pointer motion selects.
    pointer: Option<Point<Pixels>>,
    _load: Task<()>,
    _subscription: Subscription,
}

pub fn open(state: AppState, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| QuickOpen::new(state, window, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let confirm_view = view.clone();
        // Enter inside a dialog dispatches the dialog's Confirm; route it to the selection.
        dialog
            // 0945F5E8: 740 wide, upper centre ~80 below the content top, R16, faint edge,
            // large soft shadow, no title. Kit's dialog keeps focus, Esc and overlay close.
            .w(px(740.))
            .p_0()
            .rounded(px(16.))
            .border_color(theme::tokens(cx).border.opacity(0.6))
            .shadow_xl()
            .margin_top(px(MARGIN_TOP))
            .close_button(false)
            .on_ok(move |_, window, cx| {
                confirm_view.update(cx, |view, cx| view.confirm(window, cx));
                false
            })
            .child(view.clone())
    });
}

impl QuickOpen {
    fn new(state: AppState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Open project, task, document or review…"));
        // The dialog takes focus when it opens; focus the field once it is mounted.
        let focus_query = query.clone();
        window.defer(cx, move |window, cx| focus_query.update(cx, |input, cx| input.focus(window, cx)));
        let _subscription = cx.subscribe_in(&query, window, |this, _, event: &InputEvent, _, cx| match event {
            InputEvent::Change => {
                this.selected = 0;
                this.results.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }
            _ => {}
        });
        let api = state.api.clone();
        let _load = cx.spawn(async move |this, cx| {
            let result = load_entries(&api).await;
            this.update(cx, |this, cx| {
                this.entries = Some(result);
                cx.notify();
            })
            .ok();
        });
        Self { state, query, entries: None, selected: 0, results: ScrollHandle::new(), pointer: None, _load, _subscription }
    }

    fn matches(&self, cx: &App) -> Vec<Entry> {
        let Some(Ok(entries)) = &self.entries else {
            return Vec::new();
        };
        let query = self.query.read(cx).value().to_lowercase();
        let mut matches: Vec<Entry> = entries
            .iter()
            .filter(|entry| query.is_empty() || entry.title.to_lowercase().contains(&query))
            .take(MAX_RESULTS)
            .cloned()
            .collect();
        // Stable: keeps load order within each section, so labels never repeat.
        matches.sort_by_key(|entry| section(&entry.target.kind).0);
        matches
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.matches(cx).get(self.selected).cloned() {
            self.choose(entry, window, cx);
        }
    }

    fn choose(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        self.state.open(entry.target.clone(), window, cx);
        self.state.set_title(&entry.target, entry.title, cx);
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let matches = self.matches(cx);
        let len = matches.len() as isize;
        if len > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
            self.results.scroll_to_item(reveal_child(&matches, self.selected));
            cx.notify();
        }
    }
}

async fn load_entries(api: &crate::api::ApiClient) -> Result<Vec<Entry>, ApiError> {
    let projects = api.get::<Vec<Project>>("/v1/projects").await?;
    let tasks = api.get::<Vec<TaskRecord>>("/v1/tasks").await?;
    let documents = api.get::<Vec<Document>>("/v1/documents").await?;
    let project_name = |id: &str| -> SharedString {
        projects.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default().into()
    };
    let mut entries: Vec<Entry> = projects
        .iter()
        .map(|p| Entry { target: TabTarget::project(&p.id), title: p.name.clone().into(), context: SharedString::default() })
        .collect();
    entries.extend(tasks.iter().map(|task| Entry {
        target: TabTarget::task(&task.project_id, &task.id),
        title: task.title.clone().into(),
        context: project_name(&task.project_id),
    }));
    entries.extend(documents.iter().map(|doc| Entry {
        target: match doc.kind {
            DocumentKind::CodeTour => TabTarget::review(&doc.project_id, &doc.id),
            _ => TabTarget::document(&doc.project_id, &doc.id),
        },
        title: doc.title.clone().into(),
        context: project_name(&doc.project_id),
    }));
    Ok(entries)
}

/// Section order and label for each kind; matches are grouped in this order.
fn section(kind: &TabKind) -> (u8, &'static str) {
    match kind {
        TabKind::Project => (0, "Projects"),
        TabKind::Task => (1, "Tasks"),
        TabKind::Document => (2, "Documents"),
        TabKind::Review => (3, "Reviews"),
        TabKind::Unknown(_) => (4, "Other"),
    }
}

/// Child index of row `selected` in the results list, where each section adds a label
/// child before its rows. `scroll_to_item` reveals exactly this row; its heading shows
/// whenever it also fits.
fn reveal_child(matches: &[Entry], selected: usize) -> usize {
    let rank = |ix: usize| section(&matches[ix].target.kind).0;
    selected + (0..=selected).filter(|&ix| ix == 0 || rank(ix) != rank(ix - 1)).count()
}

/// Outlined key hint (`0945F5E8` "Esc", `3158C0B8`): 22 high, R5, 1px border, 11pt.
fn keycap(label: &'static str, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(22.))
        .min_w(px(22.))
        .px_1p5()
        .rounded(px(5.))
        .border_1()
        .border_color(t.border)
        .text_size(px(11.))
        .text_color(t.text_secondary)
        .child(label)
}

/// Section label: 13 medium secondary, with the section's count in tertiary (`3158C0B8`).
fn group_label(label: &'static str, count: usize, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .gap_1p5()
        .px(px(12.))
        .pt_4()
        .pb_2()
        .t_label()
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.text_secondary)
        .child(label)
        .child(div().text_color(t.text_tertiary).child(count.to_string()))
}

impl QuickOpen {
    /// Direct children of the results scroll list, so `ScrollHandle::scroll_to_item`
    /// indexes labels and rows (see [`reveal_child`]).
    fn render_results(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = theme::tokens(cx);
        let matches = match &self.entries {
            None => return vec![loading_state("Loading…", cx).into_any_element()],
            Some(Err(error)) => return vec![empty_state("Couldn't load items", error.to_string()).into_any_element()],
            Some(Ok(_)) => self.matches(cx),
        };
        if matches.is_empty() {
            return vec![empty_state("No matches", "Try a different name.").into_any_element()];
        }
        let count = |rank: u8| matches.iter().filter(|entry| section(&entry.target.kind).0 == rank).count();
        let mut list = Vec::new();
        let mut current = None;
        for (index, entry) in matches.iter().cloned().enumerate() {
            let (rank, label) = section(&entry.target.kind);
            if current != Some(rank) {
                current = Some(rank);
                list.push(group_label(label, count(rank), cx).flex_none().into_any_element());
            }
            let chosen = entry.clone();
            // One selection fill shared by pointer and keyboard (`0945F5E8`): hovering a
            // row selects it, so the two never show different rows.
            list.push(
                div()
                    .id(("quick-open", index))
                    // Fixed 45 rows: the list scrolls rather than squeezing them.
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_4()
                    .h(px(45.))
                    .px(px(14.))
                    .rounded(px(10.))
                    .text_size(px(15.))
                    .text_color(t.text)
                    .cursor_pointer()
                    .when(index == self.selected, |row| row.bg(t.hover))
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        if this.pointer.replace(event.position) == Some(event.position) {
                            return;
                        }
                        if this.selected != index {
                            this.selected = index;
                            cx.notify();
                        }
                    }))
                    .child(Icon::new(tab_icon(&entry.target.kind)).size(px(18.)).text_color(t.text_secondary))
                    .child(div().flex_1().min_w_0().truncate().child(entry.title))
                    .child(div().flex_none().max_w(px(220.)).truncate().t_label().text_color(t.text_secondary).child(entry.context))
                    .on_click(cx.listener(move |this, _, window, cx| this.choose(chosen.clone(), window, cx)))
                    .into_any_element(),
            );
        }
        list
    }
}

impl Render for QuickOpen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let results_max = results_max_height(f32::from(window.viewport_size().height));
        let hint = |label: &'static str| div().pl_1p5().child(label);
        // Anatomy and metrics from `0945F5E8` (matrix "Command palette").
        div()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| match event.keystroke.key.as_str() {
                "down" => this.move_selection(1, cx),
                "up" => this.move_selection(-1, cx),
                _ => {}
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(px(HEADER_H))
                    .pl(px(14.))
                    .pr_4()
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div().flex_1().min_w_0().child(
                            // On the Input itself: Kit sets the field's text size on its own
                            // root (`input_text_size`), so an inherited size would be ignored.
                            Input::new(&self.query)
                                .appearance(false)
                                .text_size(px(16.))
                                .line_height(px(24.))
                                .prefix(Icon::new(IconName::Search).size(px(18.)).text_color(t.text_secondary).mr_2()),
                        ),
                    )
                    .child(keycap("Esc", cx)),
            )
            .child(
                div()
                    .id("quick-open-results")
                    .track_scroll(&self.results)
                    .flex()
                    .flex_col()
                    .max_h(px(results_max))
                    .min_h(px(RESULTS_MIN))
                    .overflow_y_scroll()
                    .px(px(10.))
                    .pb_2()
                    .children(self.render_results(cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(FOOTER_H))
                    .px_5()
                    .border_t_1()
                    .border_color(t.border)
                    .t_label()
                    .text_color(t.text_secondary)
                    .child(keycap("↑", cx))
                    .child(keycap("↓", cx))
                    .child(hint("Move").mr_3())
                    .child(keycap("↵", cx))
                    .child(hint("Open")),
            )
    }
}
