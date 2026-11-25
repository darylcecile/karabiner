//! Inspector: a read-only side panel describing the active tab's stored record —
//! context, environment, linked changes, runs and the activity log. Everything shown
//! comes from the service; absent values read "Not set" / "Not available".

use std::collections::HashSet;

use gpui_kit::component::{
    Icon,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{Activity, Document, DocumentKind, Project, ProjectConfig, Run, Task as TaskRecord, TabKind};

use crate::{
    api::{ApiClient, ApiError, query_value},
    components::{button_xs, card, chip, empty_state, error_state, loading_state, status_dot, Tone, Typography as _},
    shell::sidebar::status_tone,
    state::{AppState, TabTarget},
    theme::{self, metrics},
};

pub const INSPECTOR_WIDTH: f32 = 320.;

/// Field row pitch; label column so values keep 264 − 88 − 12 inside a 320 panel.
const ROW_H: f32 = 24.;
const LABEL_W: f32 = 88.;
/// Activity entries shown before "Show N more" (`B6BD5981` "Show 10 more checks").
const ACTIVITY_PREVIEW: usize = 5;

fn count(n: usize) -> Option<String> {
    (n > 0).then(|| n.to_string())
}

/// Facts gathered for one target.
struct Details {
    project: Project,
    config: Option<ProjectConfig>,
    task: Option<TaskRecord>,
    document: Option<Document>,
    linked: Vec<Document>,
    runs: Vec<Run>,
    activity: Vec<Activity>,
}

pub struct Inspector {
    state: AppState,
    target: Option<TabTarget>,
    details: Option<Result<Details, ApiError>>,
    seen_version: Option<u64>,
    /// Card ids the user collapsed; every card starts open.
    collapsed: HashSet<&'static str>,
    show_all_activity: bool,
    _load: Option<Task<()>>,
    _observer: Subscription,
}

impl Inspector {
    pub fn new(state: AppState, cx: &mut Context<Self>) -> Self {
        let _observer = cx.observe(&state.service, |this, service, cx| {
            let version = service.read(cx).data_version();
            if this.seen_version != Some(version) {
                this.seen_version = Some(version);
                this.reload(cx);
            }
        });
        Self { state, target: None, details: None, seen_version: None, collapsed: HashSet::new(), show_all_activity: false, _load: None, _observer }
    }

    pub fn set_target(&mut self, target: Option<TabTarget>, cx: &mut Context<Self>) {
        if self.target != target {
            self.target = target;
            self.show_all_activity = false;
            self.reload(cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            self.details = None;
            return cx.notify();
        };
        self.details = None;
        cx.notify();
        let api = self.state.api.clone();
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = load(&api, &target).await;
            this.update(cx, |this, cx| {
                this.details = Some(result);
                cx.notify();
            })
            .ok();
        }));
    }
}

async fn load(api: &ApiClient, target: &TabTarget) -> Result<Details, ApiError> {
    let project_id = query_value(&target.project_id);
    let project = api.get::<Project>(&format!("/v1/projects/{project_id}")).await?;
    let config = api.get::<ProjectConfig>(&format!("/v1/projects/{project_id}/config")).await.ok();
    let id = query_value(&target.id);
    let (task, document) = match target.kind {
        TabKind::Task => (Some(api.get::<TaskRecord>(&format!("/v1/tasks/{id}")).await?), None),
        TabKind::Document | TabKind::Review => (None, Some(api.get::<Document>(&format!("/v1/documents/{id}")).await?)),
        _ => (None, None),
    };
    let (linked, runs, activity) = match target.kind {
        TabKind::Task => (
            api.get::<Vec<Document>>(&format!("/v1/documents?taskId={id}")).await?,
            api.get::<Vec<Run>>(&format!("/v1/runs?taskId={id}")).await?,
            api.get::<Vec<Activity>>(&format!("/v1/activity?taskId={id}&limit=50")).await?,
        ),
        _ => {
            let activity = api.get::<Vec<Activity>>(&format!("/v1/activity?projectId={project_id}&limit=100")).await?;
            let activity = match target.kind {
                TabKind::Project => activity,
                _ => activity.into_iter().filter(|entry| entry.resource_id.as_deref() == Some(&target.id)).collect(),
            };
            (Vec::new(), Vec::new(), activity)
        }
    };
    Ok(Details { project, config, task, document, linked, runs, activity })
}

fn field(label: &'static str, value: Option<String>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    let (text, color) = match value.filter(|value| !value.is_empty()) {
        Some(value) => (value, t.text),
        None => ("Not set".to_string(), t.text_tertiary),
    };
    field_row(label, cx).child(div().flex_1().min_w_0().truncate().text_color(color).child(text))
}

/// 24pt label/value row; the value (text or chip) is centred on it.
fn field_row(label: &'static str, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .h(px(ROW_H))
        .t_label()
        .child(div().w(px(LABEL_W)).flex_none().text_color(theme::tokens(cx).text_secondary).child(label))
}

/// Secondary one-line note inside a card (empty lists).
fn note(text: &'static str, cx: &App) -> Div {
    div().flex().items_center().min_h(px(ROW_H)).t_label().text_color(theme::tokens(cx).text_secondary).child(text)
}

/// `in_progress` → "In progress".
fn humanize(value: &str) -> String {
    let text = value.replace(['_', '-'], " ");
    let mut chars = text.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn short_time(timestamp: &str) -> String {
    // ISO-8601 UTC → "2026-11-24 18:25".
    timestamp.get(..16).map(|value| value.replace('T', " ")).unwrap_or_else(|| timestamp.to_string())
}

impl Inspector {
    /// Bordered R12 card (`B6BD5981`): 16 padding, header = title + collapse chevron and
    /// right-aligned meta; clicking the header collapses or expands the body.
    fn section(&self, id: &'static str, title: &'static str, meta: Option<String>, body: Div, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        let open = !self.collapsed.contains(id);
        let header = div()
            .id(id)
            .flex()
            .items_center()
            .gap_1()
            .h(px(ROW_H))
            // Hover fill extends 4pt past the text on each side (R6), keeping text aligned.
            .mx(px(-4.))
            .px(px(4.))
            .rounded(px(metrics::R_CONTROL))
            .hover(move |style| style.bg(t.hover))
            .cursor_pointer()
            .t_panel_title()
            .text_color(t.text)
            .child(title)
            .child(
                div().size_4().flex().items_center().justify_center().child(
                    Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size_3().text_color(t.text_secondary),
                ),
            )
            .child(div().flex_1())
            .children(meta.map(|meta| div().t_caption().text_color(t.text_secondary).child(meta)))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.collapsed.remove(id) {
                    this.collapsed.insert(id);
                }
                cx.notify();
            }));
        card(cx).bg(t.section_fill).flex().flex_col().p_4().child(header).when(open, |card| card.child(body.flex().flex_col().pt_2()))
    }

    fn render_context(&self, details: &Details, cx: &mut Context<Self>) -> Div {
        let config = details.config.as_ref();
        let workflow = |id: &Option<String>| {
            config.and_then(|c| c.workflows.iter().find(|w| Some(&w.id) == id.as_ref())).map(|w| w.name.clone())
        };
        let stage = |id: &Option<String>| {
            config
                .and_then(|c| c.workflows.iter().flat_map(|w| &w.stages).find(|s| Some(&s.id) == id.as_ref()))
                .map(|s| s.name.clone())
        };
        let agent = |id: &Option<String>| {
            config.and_then(|c| c.agents.iter().find(|a| Some(&a.id) == id.as_ref())).map(|a| a.name.clone())
        };
        let mut section = div().child(field("Project", Some(details.project.name.clone()), cx));
        if let Some(task) = &details.task {
            section = section
                .child(field_row("Status", cx).child(chip(humanize(task.status.as_str()), status_tone(&task.status), cx)))
                .child(field("Priority", Some(humanize(task.priority.as_str())), cx))
                .child(field("Workflow", workflow(&task.workflow_id), cx))
                .child(field("Stage", stage(&task.stage_id), cx))
                .child(field("Agent", agent(&task.agent_id), cx))
                .child(field("Execution", Some(humanize(&task.execution_state)), cx))
                .child(field("Updated", Some(short_time(&task.updated_at)), cx));
        }
        if let Some(document) = &details.document {
            let kind = if document.kind == DocumentKind::CodeTour { "Code tour" } else { "Markdown" };
            section = section
                .child(field("Kind", Some(kind.to_string()), cx))
                .child(field("Revision", Some(document.revision.to_string()), cx))
                .child(field("Updated", Some(short_time(&document.updated_at)), cx));
        }
        self.section("context", "Context", None, section, cx)
    }

    fn render_environment(&self, details: &Details, cx: &mut Context<Self>) -> Div {
        let target = details.config.as_ref().map(|config| match config.execution_target.as_str() {
            "host" => "Local host".to_string(),
            "mxc" => "MXC sandbox".to_string(),
            "apple-container" => "Apple container".to_string(),
            other => other.to_string(),
        });
        let body = div()
            .child(field("Runs on", target, cx))
            .child(field("Checkout", details.project.root_path.clone(), cx))
            .child(field("Repository", details.project.repository.clone(), cx))
            .child(field("Source", Some(humanize(details.project.kind.as_str())), cx));
        self.section("environment", "Environment", None, body, cx)
    }

    fn render_changes(&self, details: &Details, cx: &mut Context<Self>) -> Option<[Div; 2]> {
        details.task.as_ref()?;
        let t = theme::tokens(cx);
        let mut section = div();
        if details.linked.is_empty() {
            section = section.child(note("None linked to this task", cx));
        }
        for document in &details.linked {
            let target = match document.kind {
                DocumentKind::CodeTour => TabTarget::review(&document.project_id, &document.id),
                _ => TabTarget::document(&document.project_id, &document.id),
            };
            let title: SharedString = document.title.clone().into();
            let icon = if document.kind == DocumentKind::CodeTour { IconName::GitPullRequest } else { IconName::FileText };
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(ROW_H))
                    .t_label()
                    .child(Icon::new(icon).size_3p5().text_color(t.text_secondary))
                    .child(div().flex_1().min_w_0().truncate().child(title.clone()))
                    .child(
                        button_xs(Button::new(ElementId::Name(format!("open-linked:{}", document.id).into())))
                            .ghost()
                            .label("Open in tab")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.state.open(target.clone(), window, cx);
                                this.state.set_title(&target, title.clone(), cx);
                            })),
                    ),
            );
        }
        let runs = if details.runs.is_empty() {
            note("No runs recorded", cx)
        } else {
            div().children(details.runs.iter().map(|run| {
                field("Run", Some(format!("{} · {}", run.state, short_time(&run.created_at))), cx)
            }))
        };
        let linked = self.section("linked", "Linked documents", count(details.linked.len()), section, cx);
        let runs = self.section("runs", "Runs", count(details.runs.len()), runs, cx);
        Some([linked, runs])
    }

    fn render_log(&self, details: &Details, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        let total = details.activity.len();
        let mut log = div().gap_1();
        if total == 0 {
            log = log.child(note("No activity recorded", cx));
        }
        let shown = if self.show_all_activity { total } else { total.min(ACTIVITY_PREVIEW) };
        for entry in &details.activity[..shown] {
            log = log.child(
                div()
                    .flex()
                    .gap_2()
                    .py_1()
                    .t_caption()
                    .child(div().pt(px(4.)).child(status_dot(Tone::Neutral, cx).size(px(6.))))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_color(t.text).child(entry.message.clone()))
                            // Time and kind wrap as whole units, with no separator to orphan.
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_x_2()
                                    .t_code_caption(cx)
                                    .text_color(t.text_secondary)
                                    .child(short_time(&entry.created_at))
                                    .child(entry.kind.clone()),
                            ),
                    ),
            );
        }
        if total > ACTIVITY_PREVIEW {
            let label = if self.show_all_activity { "Show less".to_string() } else { format!("Show {} more", total - shown) };
            log = log.child(
                div().child(
                    button_xs(Button::new("activity-more")).ghost().label(label).on_click(cx.listener(|this, _, _, cx| {
                        this.show_all_activity = !this.show_all_activity;
                        cx.notify();
                    })),
                ),
            );
        }
        self.section("activity", "Activity", count(total), log, cx)
    }
}

impl Render for Inspector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let body: AnyElement = match (&self.target, &self.details) {
            (None, _) => empty_state("Nothing to inspect", "Open a project, task, document or review.").into_any_element(),
            (Some(_), None) => loading_state("Loading…", cx).into_any_element(),
            (Some(_), Some(Err(error))) => {
                let entity = cx.entity().downgrade();
                error_state(error.to_string(), move |_, _, cx| {
                    entity.update(cx, |this, cx| this.reload(cx)).ok();
                })
                .into_any_element()
            }
            (Some(_), Some(Ok(details))) => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(self.render_context(details, cx))
                .child(self.render_environment(details, cx))
                .children(self.render_changes(details, cx).into_iter().flatten())
                .child(self.render_log(details, cx))
                .into_any_element(),
        };
        div()
            .id("inspector")
            .w(px(INSPECTOR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .px_3()
            .pb_3()
            .bg(t.canvas)
            .border_l_1()
            .border_color(t.border)
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(44.))
                    .px_2()
                    .t_panel_title()
                    .child("Inspector")
                    .child(div().t_caption().text_color(t.text_secondary).child("Read-only")),
            )
            .child(body)
    }
}
