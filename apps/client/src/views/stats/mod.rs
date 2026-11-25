//! Home: task prompt, stored-activity heatmap, and widgets for tasks, projects, needs-you
//! and recent work. Every number comes from the service; execution metrics read "Not available".

pub mod common;
mod heatmap;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    Disableable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{
    Activity, CreateTask, Document, DocumentKind, Intervention, InterventionStatus, Project, Statistics,
    Task as WorkTask, TaskStatus,
};

use crate::{
    api::ApiError,
    components::{
        Segment, Tone, Typography as _, button_md, button_sm, button_xs, card, feature_icon, segmented, stat_tile, status_dot,
        typography::tabular,
    },
    state::{AppState, Section, TabTarget},
    theme::{self, metrics},
    views::docs::{TEMPLATES, create_document},
};
use common::{now_seconds, parse_timestamp, relative_time};

const ACTIVITY_LIMIT: usize = 500;

pub fn section(state: AppState, window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| HomeView::new(state, window, cx)).into()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Range {
    Today,
    Week,
    Month,
    Quarter,
    All,
}

impl Range {
    const ALL: [Range; 5] = [Range::Today, Range::Week, Range::Month, Range::Quarter, Range::All];

    fn label(self) -> &'static str {
        match self {
            Range::Today => "Today",
            Range::Week => "7 days",
            Range::Month => "30 days",
            Range::Quarter => "90 days",
            Range::All => "All",
        }
    }

    /// Inclusive lower bound in seconds, or `None` for all time.
    fn since(self) -> Option<i64> {
        let today_start = now_seconds().div_euclid(86_400) * 86_400;
        let days = match self {
            Range::Today => 0,
            Range::Week => 6,
            Range::Month => 29,
            Range::Quarter => 89,
            Range::All => return None,
        };
        Some(today_start - days * 86_400)
    }
}

type Loaded<T> = Option<Result<T, String>>;

#[derive(Default)]
struct HomeData {
    projects: Loaded<Vec<Project>>,
    tasks: Loaded<Vec<WorkTask>>,
    documents: Loaded<Vec<Document>>,
    interventions: Loaded<Vec<Intervention>>,
    activity: Loaded<Vec<Activity>>,
    stats: Loaded<Statistics>,
}

pub struct HomeView {
    state: AppState,
    data: HomeData,
    range: Range,
    project_ix: usize,
    prompt: Entity<InputState>,
    creating: bool,
    create_error: Option<String>,
    doc_error: Option<String>,
    refresh_error: Option<String>,
    refreshing: bool,
    seen_version: u64,
    load_task: Option<Task<()>>,
    /// Allocated content width: two widget columns only when both fit.
    width: crate::views::responsive::WidthCell,
    heatmap_scroll: ScrollHandle,
    /// Set once the heatmap has been scrolled to its newest week.
    heatmap_pinned: bool,
    _subscriptions: Vec<Subscription>,
}

/// Applies a refresh result. A failed refresh keeps earlier data on screen and reports the
/// error separately, so widgets never fall back to misleading zeros.
fn merge<T>(slot: &mut Loaded<T>, result: Result<T, ApiError>, errors: &mut Vec<String>) {
    match result {
        Ok(value) => *slot = Some(Ok(value)),
        Err(error) => {
            errors.push(error.to_string());
            if !matches!(slot, Some(Ok(_))) {
                *slot = Some(Err(error.to_string()));
            }
        }
    }
}

impl HomeView {
    fn new(state: AppState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prompt = cx.new(|cx| InputState::new(window, cx).placeholder("Describe the task…"));
        let seen_version = state.service.read(cx).data_version();
        let _subscriptions = vec![
            cx.subscribe_in(&prompt, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.create_task(window, cx);
                }
            }),
            cx.observe(&state.service, |this, service, cx| {
                let version = service.read(cx).data_version();
                if version != this.seen_version {
                    this.seen_version = version;
                    this.reload(cx);
                }
            }),
        ];
        let mut view = Self {
            state,
            data: HomeData::default(),
            range: Range::Week,
            project_ix: 0,
            prompt,
            creating: false,
            create_error: None,
            doc_error: None,
            refresh_error: None,
            refreshing: false,
            seen_version,
            load_task: None,
            width: Default::default(),
            heatmap_scroll: ScrollHandle::new(),
            heatmap_pinned: false,
            _subscriptions,
        };
        view.reload(cx);
        view
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let activity_path = format!("/v1/activity?limit={ACTIVITY_LIMIT}");
        let api = self.state.api.clone();
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let (projects, tasks, documents, interventions, activity, stats) = futures::join!(
                api.get::<Vec<Project>>("/v1/projects"),
                api.get::<Vec<WorkTask>>("/v1/tasks"),
                api.get::<Vec<Document>>("/v1/documents"),
                api.get::<Vec<Intervention>>("/v1/interventions"),
                api.get::<Vec<Activity>>(&activity_path),
                api.get::<Statistics>("/v1/stats"),
            );
            this.update(cx, |this, cx| {
                let mut errors = Vec::new();
                let data = &mut this.data;
                merge(&mut data.projects, projects, &mut errors);
                merge(&mut data.tasks, tasks, &mut errors);
                merge(&mut data.documents, documents, &mut errors);
                merge(&mut data.interventions, interventions, &mut errors);
                merge(&mut data.activity, activity, &mut errors);
                merge(&mut data.stats, stats, &mut errors);
                this.refresh_error = errors.into_iter().next();
                this.refreshing = false;
                cx.notify();
            })
            .ok();
        }));
    }

    fn projects(&self) -> &[Project] {
        match &self.data.projects {
            Some(Ok(projects)) => projects,
            _ => &[],
        }
    }

    fn selected_project(&self) -> Option<&Project> {
        let projects = self.projects();
        projects.get(self.project_ix.min(projects.len().saturating_sub(1)))
    }

    fn create_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.prompt.read(cx).value().trim().to_owned();
        let Some(project) = self.selected_project() else { return };
        if text.is_empty() || self.creating {
            return;
        }
        let (title, description) = match text.split_once('\n') {
            Some((title, rest)) => (title.trim().to_owned(), Some(rest.trim().to_owned())),
            None => (text, None),
        };
        let body = CreateTask {
            project_id: project.id.clone(),
            title,
            description,
            status: Some(TaskStatus::Todo),
            ..Default::default()
        };
        self.creating = true;
        self.create_error = None;
        let api = self.state.api.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = api.post::<_, WorkTask>("/v1/tasks", &body).await;
            this.update_in(cx, |this, window, cx| {
                this.creating = false;
                match result {
                    Ok(task) => {
                        this.prompt.update(cx, |input, cx| input.set_value("", window, cx));
                        this.state.data_changed(cx);
                        this.state.open(TabTarget::task(task.project_id, task.id), window, cx);
                    }
                    Err(error) => this.create_error = Some(format!("Couldn't create task: {error}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn activity_in_range(&self) -> Vec<&Activity> {
        let since = self.range.since();
        match &self.data.activity {
            Some(Ok(items)) => items
                .iter()
                .filter(|item| since.is_none_or(|since| parse_timestamp(&item.created_at).is_some_and(|ts| ts >= since)))
                .collect(),
            _ => Vec::new(),
        }
    }
}

// ---- Rendering --------------------------------------------------------------------------

/// Widget header row (`DFCD11C8`): semibold title, right-aligned caption, 44 high.
pub(super) fn widget_header(title: impl Into<SharedString>, trailing: Option<SharedString>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .h(px(44.))
        .px_4()
        .child(div().min_w_0().truncate().t_panel_title().text_color(t.text).child(title.into()))
        .children(trailing.map(|text| div().flex_none().t_caption().text_color(t.text_secondary).child(text)))
}

/// Header with the count beside the title ("Working now 1" in `DFCD11C8`).
fn widget_header_count(title: impl Into<SharedString>, count: usize, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .h(px(44.))
        .px_4()
        .child(div().min_w_0().truncate().t_panel_title().text_color(t.text).child(title.into()))
        .child(div().flex_none().t_caption().font_features(tabular()).text_color(t.text_secondary).child(count.to_string()))
}

/// Home widget surface (`DFCD11C8`): bordered card holding a header row and, inset 8,
/// a soft inner body ([`inset_body`]).
fn widget(cx: &App) -> Div {
    card(cx).flex().flex_col().pb_2()
}

/// The widget's inner layer: section fill, inset 8 from the card's sides, radius 8.
fn inset_body(cx: &App) -> Div {
    div().flex().flex_col().mx_2().py_1().rounded(px(metrics::R_INNER)).bg(theme::tokens(cx).section_fill)
}

/// Quiet one-line note inside a widget (empty, loading).
fn note(text: impl Into<SharedString>, cx: &App) -> Div {
    div().px_3().py_2().t_body().text_color(theme::tokens(cx).text_secondary).child(text.into())
}

fn load_state<T>(loaded: &Loaded<T>, cx: &App) -> Option<AnyElement> {
    match loaded {
        None => Some(note("Loading…", cx).into_any_element()),
        Some(Err(error)) => Some(
            div()
                .px_4()
                .pb_2()
                .t_body()
                .text_color(theme::tokens(cx).danger)
                .child(format!("Couldn't load: {error}"))
                .into_any_element(),
        ),
        Some(Ok(_)) => None,
    }
}

fn list_row(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    crate::components::list_row(id, false, cx).mx_1()
}

/// Row meta on the right of a widget row.
fn row_meta(text: impl Into<SharedString>, cx: &App) -> Div {
    div().flex_none().t_caption().text_color(theme::tokens(cx).text_secondary).child(text.into())
}

fn status_label(status: &TaskStatus) -> &str {
    match status {
        TaskStatus::Backlog => "Backlog",
        TaskStatus::Todo => "Todo",
        TaskStatus::InProgress => "In progress",
        TaskStatus::InReview => "In review",
        TaskStatus::Blocked => "Blocked",
        TaskStatus::Done => "Done",
        TaskStatus::Unknown(raw) => raw,
    }
}

fn status_tone(status: &TaskStatus) -> Tone {
    match status {
        TaskStatus::InProgress => Tone::Running,
        TaskStatus::InReview => Tone::Validating,
        TaskStatus::Blocked => Tone::Danger,
        TaskStatus::Done => Tone::Success,
        TaskStatus::Todo => Tone::Attention,
        _ => Tone::Neutral,
    }
}

impl HomeView {
    fn render_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_projects = !self.projects().is_empty();
        let header = div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .child(feature_icon("sparkles", px(40.)))
            .child(div().t_title().text_color(theme::tokens(cx).text).child("What should we work on?"));
        let body: AnyElement = match (&self.data.projects, has_projects) {
            (Some(Ok(_)), false) => self.render_no_projects(cx).into_any_element(),
            _ => self.render_prompt_card(cx).into_any_element(),
        };
        div().flex().flex_col().items_center().gap_5().pt_12().pb_8().child(header).child(body)
    }

    fn render_prompt_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<(usize, String)> = self.projects().iter().map(|p| p.name.clone()).enumerate().collect();
        let current = self.selected_project().map(|p| p.name.clone()).unwrap_or_else(|| "Project".into());
        let selected = self.project_ix;
        let view = cx.entity().downgrade();
        let picker = button_sm(Button::new("home-project").ghost().icon(Lucide::Folder).label(current))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                names.iter().fold(menu, |menu, (index, name)| {
                    let view = view.clone();
                    let index = *index;
                    menu.item(PopupMenuItem::new(name.clone()).checked(index == selected).on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.project_ix = index;
                            cx.notify();
                        })
                        .ok();
                    }))
                })
            });
        let can_create = self.selected_project().is_some() && !self.creating;
        card(cx)
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(640.))
            // 12 on every side: the picker and Create buttons sit as far from the bottom as the sides.
            .p_3()
            .gap_1()
            .shadow_sm()
            .child(Input::new(&self.prompt).appearance(false).t_body().h(px(40.)).px_1().aria_label("New task"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(picker)
                    .child(div().flex_1())
                    .children(self.create_error.clone().map(|e| div().t_caption().text_color(theme::tokens(cx).danger).child(e)))
                    .child(
                        button_md(Button::new("home-create").primary().label("Create task"))
                            .loading(self.creating)
                            .disabled(!can_create)
                            .tooltip("Adds a Todo task to the project (↵)")
                            .on_click(cx.listener(|this, _, window, cx| this.create_task(window, cx))),
                    ),
            )
    }

    fn render_no_projects(&self, cx: &mut Context<Self>) -> impl IntoElement {
        card(cx)
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(640.))
            .p_5()
            .gap_4()
            .items_center()
            .child(div().t_body().text_color(theme::tokens(cx).text_secondary).child("Tasks live in a project. Start from a template or create one."))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(button_md(Button::new("home-templates").primary().label("Start from a template")).on_click(
                        cx.listener(|this, _, _, cx| this.state.show_section(Section::Templates, cx)),
                    ))
                    .child(button_md(Button::new("home-projects").outline().label("Projects")).on_click(
                        cx.listener(|this, _, _, cx| this.state.show_section(Section::Projects, cx)),
                    )),
            )
    }

    fn render_range_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let today = common::day_label(now_seconds().div_euclid(86_400));
        let selected = Range::ALL.iter().position(|range| *range == self.range).unwrap_or(0);
        let view = cx.entity().downgrade();
        // Date and range share a line when they fit; otherwise the range (all five items at
        // full size) wraps onto its own line.
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_x_4()
            .gap_y_2()
            .child(div().t_label().text_color(theme::tokens(cx).text_secondary).child(today))
            .child(segmented(
                "home-range",
                Range::ALL.iter().map(|range| Segment::new(range.label())).collect(),
                selected,
                move |index, _, cx| {
                    view.update(cx, |this, cx| {
                        this.range = Range::ALL[index];
                        cx.notify();
                    })
                    .ok();
                },
                cx,
            )
            // Capped at the line width: if the five full-size items can't fit (≈302 at the
            // 720/360 minimum) they wrap inside the track (24 items, 2 gaps/padding kept) and
            // the second row centres; wide, it stays one 28 track.
            .max_w_full()
            .flex_wrap()
            .justify_center()
            .h_auto()
            .min_h(px(metrics::CONTROL_MD)))
    }

    fn render_needs_you(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(Ok(items)) = &self.data.interventions {
            for (ix, item) in items.iter().filter(|i| i.status == InterventionStatus::Recorded).take(5).enumerate() {
                rows.push(self.render_intervention(ix, item, cx).into_any_element());
            }
        }
        if let Some(Ok(tasks)) = &self.data.tasks {
            let flagged = tasks.iter().filter(|t| matches!(t.status, TaskStatus::Blocked | TaskStatus::InReview));
            for (ix, task) in flagged.take(8).enumerate() {
                rows.push(self.render_task_row(("needs-task", ix), task, cx).into_any_element());
            }
        }
        let loading = load_state(&self.data.tasks, cx).or_else(|| load_state(&self.data.interventions, cx));
        let empty = rows.is_empty() && loading.is_none();
        widget(cx)
            .child(widget_header_count("Needs you", rows.len(), cx))
            .child(
                inset_body(cx)
                    .children(loading)
                    .children(empty.then(|| note("Nothing is blocked, waiting for review or asking a question.", cx)))
                    .children(rows),
            )
    }

    fn render_intervention(&self, ix: usize, item: &Intervention, cx: &mut Context<Self>) -> AnyElement {
        let target = item.task_id.clone().map(|task| TabTarget::task(item.project_id.clone(), task));
        list_row(("needs-int", ix), cx)
            .child(gpui_kit::component::Icon::new(Lucide::MessageCircleQuestionMark).size_4().flex_none().text_color(theme::tokens(cx).attention))
            .child(div().flex_1().min_w_0().truncate().text_color(theme::tokens(cx).text).child(item.message.clone()))
            .child(row_meta(format!("{} · {}", item.kind.as_str(), relative_time(&item.created_at)), cx))
            .on_click(cx.listener(move |this, _, window, cx| match &target {
                Some(target) => this.state.open(target.clone(), window, cx),
                None => this.state.show_section(Section::Inbox, cx),
            }))
            .into_any_element()
    }

    fn render_task_row(&self, id: impl Into<ElementId>, task: &WorkTask, cx: &mut Context<Self>) -> AnyElement {
        let target = TabTarget::task(task.project_id.clone(), task.id.clone());
        list_row(id, cx)
            .child(status_dot(status_tone(&task.status), cx))
            .child(div().flex_1().min_w_0().truncate().text_color(theme::tokens(cx).text).child(task.title.clone()))
            .child(row_meta(format!("{} · {}", status_label(&task.status), relative_time(&task.updated_at)), cx))
            .on_click(cx.listener(move |this, _, window, cx| this.state.open(target.clone(), window, cx)))
            .into_any_element()
    }

    /// Overview tiles (`DB2A4628`): section-fill tiles at R6 on the widget surface, gap 8.
    /// Stat tiles: three across when the widget is wide (stacked column, or a full 1000 page);
    /// two in a half-width column so captions like "8 created · 1 done" aren't cut.
    fn tile_columns(&self) -> u16 {
        use crate::views::responsive::narrower_than;
        if narrower_than(&self.width, 880.) || !narrower_than(&self.width, 1000.) { 3 } else { 2 }
    }

    fn render_overview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let in_range = self.activity_in_range();
        let created = in_range.iter().filter(|a| a.kind == "task.created").count();
        let docs_edited = in_range.iter().filter(|a| a.kind.starts_with("document.")).count();
        let range = self.range.label();
        let tiles: Vec<(String, String, String)> = match &self.data.stats {
            Some(Ok(stats)) => vec![
                (stats.project_count.to_string(), "Projects".into(), "All time".into()),
                (stats.task_count.to_string(), "Tasks".into(), format!("{created} created · {range}")),
                (format!("{:.0}%", stats.completion_rate * 100.0), "Completed".into(), format!("{} done", stats.completed_task_count)),
                (stats.document_count.to_string(), "Documents".into(), format!("{docs_edited} edits · {range}")),
                (stats.tour_count.to_string(), "Code tours".into(), "All time".into()),
                (stats.intervention_count.to_string(), "Interventions".into(), "Recorded intents".into()),
            ],
            _ => Vec::new(),
        };
        let unavailable = match &self.data.stats {
            Some(Ok(stats)) => vec![
                ("Runs", stats.run_count.map(|n| n.to_string())),
                ("Tokens", stats.tokens.map(|n| n.to_string())),
                ("Cost", stats.cost.map(|n| format!("${n:.2}"))),
            ],
            _ => Vec::new(),
        };
        widget(cx)
            .child(widget_header("Overview", None, cx))
            .children(load_state(&self.data.stats, cx).map(|state| div().px_1().child(state)))
            .child(
                div().grid().grid_cols(self.tile_columns()).gap_2().px_2().children(tiles.into_iter().map(|(value, label, detail)| {
                    stat_tile(value, label, cx).child(div().t_caption().text_color(t.text_secondary).truncate().child(detail))
                })),
            )
            .when(!unavailable.is_empty(), |el| {
                el.child(
                    div()
                        .mx_4()
                        .mt_3()
                        .pt_3()
                        .pb_2()
                        .border_t_1()
                        .border_color(t.border)
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x_5()
                        .gap_y_1()
                        .t_caption()
                        .children(unavailable.into_iter().map(|(label, value)| {
                            div()
                                .flex()
                                .gap_1()
                                .child(div().text_color(t.text_secondary).child(label))
                                .child(div().text_color(t.text).child(value.unwrap_or_else(|| "Not available".into())))
                        }))
                        .child(div().flex_1())
                        .child(div().text_color(t.text_secondary).child("Execution isn't connected yet")),
                )
            })
    }

    /// Tasks by status: the Home widget frame (`DFCD11C8`) around the `527B3F33` usage body,
    /// a prominent total, one segmented distribution bar and a two-column legend.
    fn render_tasks(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let counts: Vec<(TaskStatus, u64)> = match &self.data.stats {
            Some(Ok(stats)) => TaskStatus::ALL
                .iter()
                .map(|status| (status.clone(), stats.tasks_by_status.get(status.as_str()).copied().unwrap_or(0)))
                .collect(),
            _ => Vec::new(),
        };
        let total: u64 = counts.iter().map(|(_, n)| *n).sum();
        let done = counts.iter().find(|(status, _)| *status == TaskStatus::Done).map_or(0, |(_, n)| *n);
        let share = |count: u64| if total == 0 { 0 } else { (count * 100 + total / 2) / total };
        let bar = div()
            .flex()
            .h(px(8.))
            .gap(px(2.))
            .w_full()
            .when(total == 0, |bar| bar.child(div().flex_1().rounded_full().bg(t.border)))
            .children(counts.iter().filter(|(_, n)| *n > 0).enumerate().map(|(ix, (status, count))| {
                let first = ix == 0;
                let last = ix + 1 == counts.iter().filter(|(_, n)| *n > 0).count();
                div()
                    .h_full()
                    .w(relative(*count as f32 / total.max(1) as f32))
                    .bg(status_tone(status).color(cx))
                    .when(first, |el| el.rounded_l_full())
                    .when(last, |el| el.rounded_r_full())
            }));
        let legend = counts.iter().map(|(status, count)| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(28.))
                .t_body()
                .child(status_dot(status_tone(status), cx))
                .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(status_label(status).to_owned()))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_none()
                        .font_features(tabular())
                        .text_color(t.text_secondary)
                        .child(count.to_string())
                        .child(div().w(px(36.)).text_right().child(format!("{}%", share(*count)))),
                )
        });
        widget(cx)
            .child(widget_header("Tasks by status", matches!(self.data.stats, Some(Ok(_))).then(|| format!("{done} done · {}%", share(done)).into()), cx))
            .child(
                inset_body(cx)
                    .px_3()
                    .py_3()
                    .gap_3()
                    .children(load_state(&self.data.stats, cx))
                    .when(matches!(self.data.stats, Some(Ok(_))), |body| {
                        body.child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap_2()
                                .child(div().t_display().text_color(t.text).child(total.to_string()))
                                .child(div().t_body().text_color(t.text_secondary).child(if total == 1 { "task" } else { "tasks" })),
                        )
                        .child(bar)
                        // Legend items wrap on a 180 basis: two per line in a wide widget,
                        // one per line when narrow, so state names are never elided.
                        .child(div().flex().flex_wrap().gap_x_6().children(legend.map(|item| div().flex_1().flex_basis(px(180.)).min_w_0().child(item))))
                    }),
            )
    }

    fn render_projects(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tasks: &[WorkTask] = match &self.data.tasks {
            Some(Ok(tasks)) => tasks,
            _ => &[],
        };
        let rows = self.projects().iter().take(6).enumerate().map(|(ix, project)| {
            let open = tasks.iter().filter(|t| t.project_id == project.id && t.status != TaskStatus::Done).count();
            let target = TabTarget::project(project.id.clone());
            list_row(("home-project-row", ix), cx)
                .child(feature_icon("folder", px(22.)))
                .child(div().flex_1().min_w_0().truncate().text_color(theme::tokens(cx).text).child(project.name.clone()))
                .child(row_meta(format!("{open} open"), cx))
                .on_click(cx.listener(move |this, _, window, cx| this.state.open(target.clone(), window, cx)))
        }).collect::<Vec<_>>();
        let empty = matches!(&self.data.projects, Some(Ok(p)) if p.is_empty());
        widget(cx)
            .child(widget_header_count("Projects", self.projects().len(), cx))
            .child(
                inset_body(cx)
                    .children(load_state(&self.data.projects, cx))
                    .children(empty.then(|| note("No projects yet.", cx)))
                    .children(rows),
            )
    }

    fn render_documents(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut documents: Vec<&Document> = match &self.data.documents {
            Some(Ok(docs)) => docs.iter().collect(),
            _ => Vec::new(),
        };
        documents.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let rows = documents.into_iter().take(6).enumerate().map(|(ix, doc)| {
            let is_tour = doc.kind == DocumentKind::CodeTour;
            let target = if is_tour {
                TabTarget::review(doc.project_id.clone(), doc.id.clone())
            } else {
                TabTarget::document(doc.project_id.clone(), doc.id.clone())
            };
            list_row(("home-doc-row", ix), cx)
                .child(feature_icon(if is_tour { "git-pull-request" } else { "file-text" }, px(22.)))
                .child(div().flex_1().min_w_0().truncate().text_color(theme::tokens(cx).text).child(doc.title.clone()))
                .child(row_meta(relative_time(&doc.updated_at), cx))
                .on_click(cx.listener(move |this, _, window, cx| this.state.open(target.clone(), window, cx)))
        }).collect::<Vec<_>>();
        let project_id = self.selected_project().map(|p| p.id.clone());
        let view = cx.entity().downgrade();
        let new_doc = button_xs(Button::new("home-new-doc").ghost().icon(Lucide::Plus).label("New doc"))
            .disabled(project_id.is_none())
            .tooltip("Create a document in the selected project")
            .dropdown_menu(move |menu, _, _| {
                TEMPLATES.iter().fold(menu, |menu, template| {
                    let (view, project_id) = (view.clone(), project_id.clone());
                    menu.item(PopupMenuItem::new(template.name).on_click(move |_, window, cx| {
                        let (Some(project_id), Some(view)) = (project_id.clone(), view.upgrade()) else { return };
                        let state = view.read(cx).state.clone();
                        let task = create_document(state, project_id, template, window, cx);
                        let view = view.downgrade();
                        cx.spawn(async move |cx| {
                            let error = task.await.err().map(|error| format!("Couldn't create document: {error}"));
                            view.update(cx, |this, cx| {
                                this.doc_error = error;
                                cx.notify();
                            })
                            .ok();
                        })
                        .detach();
                    }))
                })
            });
        let empty = matches!(&self.data.documents, Some(Ok(d)) if d.is_empty());
        widget(cx)
            .child(widget_header("Recent documents", None, cx).child(new_doc))
            .children(self.doc_error.clone().map(|error| div().px_4().pb_1().t_caption().text_color(theme::tokens(cx).danger).child(error)))
            .child(
                inset_body(cx)
                    .children(load_state(&self.data.documents, cx))
                    .children(empty.then(|| note("No documents yet.", cx)))
                    .children(rows),
            )
    }

    fn render_activity(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let items = self.activity_in_range();
        let count = items.len();
        let rows = items.into_iter().take(10).map(|item| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .h(px(metrics::ROW_PROP))
                .px_3()
                .t_label()
                .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(item.message.clone()))
                .child(row_meta(relative_time(&item.created_at), cx))
        });
        widget(cx)
            .child(widget_header("Recent activity", Some(format!("{count} events · {}", self.range.label()).into()), cx))
            .child(
                inset_body(cx)
                    .children(load_state(&self.data.activity, cx))
                    .children((count == 0 && matches!(self.data.activity, Some(Ok(_)))).then(|| note("No activity in this range.", cx)))
                    .children(rows),
            )
    }
}

impl HomeView {
    fn render_refresh_error(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let error = self.refresh_error.clone()?;
        let t = theme::tokens(cx);
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_2()
                .rounded(px(metrics::R_CARD))
                .bg(Tone::Danger.tint(cx, 0.10))
                .t_label()
                .text_color(t.text)
                .child(div().flex_1().child(format!("Couldn't refresh Home; showing the last loaded data. {error}")))
                .child(
                    button_sm(Button::new("home-retry").outline().label("Retry"))
                        .loading(self.refreshing)
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                ),
        )
    }
}

impl Render for HomeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let activity: &[Activity] = match &self.data.activity {
            Some(Ok(items)) => items,
            _ => &[],
        };
        let heatmap = heatmap::Heatmap::new(activity, activity.len() >= ACTIVITY_LIMIT);
        // Open on the newest weeks when the heatmap has to scroll (once; later scrolling is the user's).
        let max = self.heatmap_scroll.max_offset().x;
        if !self.heatmap_pinned && max > px(0.) {
            self.heatmap_scroll.set_offset(point(-max, px(0.)));
            self.heatmap_pinned = true;
        }
        // Two columns need ~430 each; below that the widgets stack in one full-width column.
        let stacked = crate::views::responsive::narrower_than(&self.width, 880.);
        let column = |children: Vec<AnyElement>| div().flex().flex_col().gap_4().flex_1().min_w_0().children(children);
        div()
            .id("home")
            .size_full()
            .bg(theme::tokens(cx).canvas)
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w_full()
                    .max_w(px(1000.))
                    .mx_auto()
                    .relative()
                    .child(crate::views::responsive::width_probe(&self.width))
                    .px(px(metrics::GUTTER_X))
                    .pb_10()
                    .child(self.render_prompt(cx))
                    .children(self.render_refresh_error(cx))
                    .child(self.render_range_bar(cx))
                    .child(
                        div()
                            .flex()
                            .gap_4()
                            .items_start()
                            .when(stacked, |this| this.flex_col().items_stretch())
                            .child(column(vec![
                                self.render_needs_you(cx).into_any_element(),
                                self.render_tasks(cx).into_any_element(),
                                self.render_projects(cx).into_any_element(),
                            ]))
                            .child(column(vec![
                                match &self.data.activity {
                                    Some(Ok(_)) => widget(cx).child(heatmap.render(&self.heatmap_scroll, cx)).into_any_element(),
                                    other => widget(cx)
                                        .child(widget_header("Activity", None, cx))
                                        .child(inset_body(cx).children(load_state(other, cx)))
                                        .into_any_element(),
                                },
                                self.render_overview(cx).into_any_element(),
                                self.render_documents(cx).into_any_element(),
                                self.render_activity(cx).into_any_element(),
                            ])),
                    ),
            )
    }
}
