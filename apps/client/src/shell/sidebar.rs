//! Sidebar: global sections with counts and a project tree (project → documents/reviews).
//! Click replaces the active tab; ⌘-click opens a new tab.

use std::collections::HashSet;

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{Document, DocumentKind, Id, Intervention, InterventionStatus, Project, Task as TaskRecord, TaskStatus};

use crate::{
    api::{ApiError, ApiResult},
    components::{count_badge, group_label, status_dot, Tone, Typography as _},
    state::{AppState, Section, TabTarget},
    theme::{self, metrics},
};

/// Default sidebar width; the user can resize it between the shell's bounds.
pub const SIDEBAR_WIDTH: f32 = metrics::SIDEBAR_W;

fn section_icon(section: Section) -> IconName {
    match section {
        Section::Home => IconName::House,
        Section::Inbox => IconName::Inbox,
        Section::Projects => IconName::Folder,
        Section::Templates => IconName::LayoutTemplate,
        Section::Settings => IconName::Settings,
    }
}

#[derive(Default)]
struct Data {
    projects: Vec<Project>,
    tasks: Vec<TaskRecord>,
    documents: Vec<Document>,
    open_interventions: usize,
}

pub struct Sidebar {
    state: AppState,
    data: Option<ApiResult<Data>>,
    expanded: HashSet<Id>,
    seen_version: Option<u64>,
    _load: Option<Task<()>>,
    _observer: Subscription,
    _workspace_observer: Option<Subscription>,
}

impl Sidebar {
    pub fn new(state: AppState, cx: &mut Context<Self>) -> Self {
        let _observer = cx.observe(&state.service, |this, service, cx| {
            let service = service.read(cx);
            let version = service.data_version();
            if service.status().is_ready() && this.seen_version != Some(version) {
                this.seen_version = Some(version);
                this.reload(cx);
            }
        });
        let _workspace_observer = state.workspace.upgrade().map(|workspace| cx.observe(&workspace, |_, _, cx| cx.notify()));
        Self { state, data: None, expanded: HashSet::new(), seen_version: None, _load: None, _observer, _workspace_observer }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let projects = api.get::<Vec<Project>>("/v1/projects").await?;
                let tasks = api.get::<Vec<TaskRecord>>("/v1/tasks").await?;
                let documents = api.get::<Vec<Document>>("/v1/documents").await?;
                let interventions = api.get::<Vec<Intervention>>("/v1/interventions").await?;
                let open_interventions =
                    interventions.iter().filter(|item| item.status == InterventionStatus::Recorded).count();
                Ok::<_, ApiError>(Data { projects, tasks, documents, open_interventions })
            }
            .await;
            this.update(cx, |this, cx| {
                this.data = Some(result);
                cx.notify();
            })
            .ok();
        }));
    }

    fn navigate(&self, target: TabTarget, title: SharedString, new_tab: bool, window: &mut Window, cx: &mut App) {
        if new_tab {
            self.state.open(target.clone(), window, cx);
        } else {
            self.state.replace_active(target.clone(), window, cx);
        }
        // Known name until the view loads its record.
        self.state.set_title(&target, title, cx);
    }

    fn render_sections(&self, current: Option<Section>, cx: &mut Context<Self>) -> impl IntoElement {
        let inbox = match &self.data {
            Some(Ok(data)) if data.open_interventions > 0 => Some(data.open_interventions),
            _ => None,
        };
        div().flex().flex_col().gap_0p5().children(Section::ALL.into_iter().map(|section| {
            let selected = current == Some(section);
            let count = if section == Section::Inbox { inbox } else { None };
            nav_row(ElementId::Name(section.title().into()), selected, cx)
                .child(icon_box().child(Icon::new(section_icon(section)).size_4()))
                .child(div().flex_1().child(section.title()))
                .children(count_badge(count, cx))
                .on_click(cx.listener(move |this, _, _, cx| this.state.show_section(section, cx)))
        }))
    }

    fn render_projects(&self, active: Option<&TabTarget>, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let data = match &self.data {
            None => return div().into_any_element(),
            Some(Err(error)) => {
                return div()
                    .px_2()
                    .t_caption()
                    .text_color(t.text_secondary)
                    .child(format!("Projects unavailable: {error}"))
                    .into_any_element()
            }
            Some(Ok(data)) => data,
        };
        if data.projects.is_empty() {
            return div()
                .px_2()
                .t_caption()
                .text_color(t.text_secondary)
                .child("No projects yet")
                .into_any_element();
        }
        let mut rows = div().flex().flex_col().gap_0p5();
        for project in &data.projects {
            let tasks: Vec<&TaskRecord> = data.tasks.iter().filter(|task| task.project_id == project.id).collect();
            let open = tasks.iter().filter(|task| task.status != TaskStatus::Done).count();
            rows = rows.child(self.project_row(project, open, active, cx));
            if self.expanded.contains(&project.id) {
                for task in tasks {
                    rows = rows.child(self.task_row(task, active, cx));
                }
                for document in data.documents.iter().filter(|doc| doc.project_id == project.id) {
                    rows = rows.child(self.document_row(document, active, cx));
                }
            }
        }
        rows.into_any_element()
    }

    fn task_row(&self, task: &TaskRecord, active: Option<&TabTarget>, cx: &mut Context<Self>) -> impl IntoElement {
        let target = TabTarget::task(&task.project_id, &task.id);
        let title: SharedString = task.title.clone().into();
        let done = task.status == TaskStatus::Done;
        nav_row(ElementId::Name(format!("task:{}", task.id).into()), active == Some(&target), cx)
            .pl(px(CHILD_INDENT))
            .child(icon_box().child(status_dot(status_tone(&task.status), cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(done, |label| label.line_through().text_color(theme::tokens(cx).text_tertiary))
                    .child(title.clone()),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.navigate(target.clone(), title.clone(), event.modifiers().platform, window, cx);
            }))
    }

    fn project_row(&self, project: &Project, open_tasks: usize, active: Option<&TabTarget>, cx: &mut Context<Self>) -> impl IntoElement {
        let target = TabTarget::project(project.id.clone());
        let expanded = self.expanded.contains(&project.id);
        let (toggle_id, click_id) = (project.id.clone(), project.id.clone());
        let title: SharedString = project.name.clone().into();
        let chevron_hover = theme::tokens(cx).selected;
        nav_row(ElementId::Name(format!("project:{click_id}").into()), active == Some(&target), cx)
            .child(
                div()
                    .id(ElementId::Name(format!("expand:{toggle_id}").into()))
                    .flex_none()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .hover(move |s| s.bg(chevron_hover))
                    .child(Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }).size_3())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if !this.expanded.remove(&toggle_id) {
                            this.expanded.insert(toggle_id.clone());
                        }
                        cx.notify();
                    })),
            )
            .child(icon_box().child(status_dot(Tone::Neutral, cx).bg(project_color(project, cx))))
            .child(div().flex_1().min_w_0().truncate().child(title.clone()))
            .children(count_badge((open_tasks > 0).then_some(open_tasks), cx))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.navigate(target.clone(), title.clone(), event.modifiers().platform, window, cx);
            }))
    }

    fn document_row(&self, document: &Document, active: Option<&TabTarget>, cx: &mut Context<Self>) -> impl IntoElement {
        let (target, icon) = match document.kind {
            DocumentKind::CodeTour => (TabTarget::review(&document.project_id, &document.id), IconName::GitPullRequest),
            _ => (TabTarget::document(&document.project_id, &document.id), IconName::FileText),
        };
        let title: SharedString = document.title.clone().into();
        nav_row(ElementId::Name(format!("doc:{}", document.id).into()), active == Some(&target), cx)
            .pl(px(CHILD_INDENT))
            .child(icon_box().child(Icon::new(icon).size_3p5()))
            .child(div().flex_1().min_w_0().truncate().child(title.clone()))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.navigate(target.clone(), title.clone(), event.modifiers().platform, window, cx);
            }))
    }
}

/// Project colours are hex (`#3b82f6`) or a small set of names.
fn project_color(project: &Project, cx: &App) -> Hsla {
    let named = match project.color.as_str() {
        "blue" => Some(0x3B82F6),
        "green" => Some(0x22C55E),
        "amber" | "orange" | "yellow" => Some(0xF59E0B),
        "red" => Some(0xEF4444),
        "pink" => Some(0xEC4899),
        "purple" | "violet" => Some(0x8B5CF6),
        "teal" | "cyan" => Some(0x14B8A6),
        _ => None,
    };
    let hex = project.color.trim_start_matches('#');
    let parsed = (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten();
    match named.or(parsed) {
        Some(value) => rgb(value).into(),
        None => theme::tokens(cx).text_tertiary,
    }
}

/// Board status → sidebar dot colour (matches the reference status palette).
pub fn status_tone(status: &TaskStatus) -> Tone {
    match status {
        TaskStatus::InProgress => Tone::Running,
        TaskStatus::InReview => Tone::Validating,
        TaskStatus::Blocked => Tone::Danger,
        TaskStatus::Done => Tone::Success,
        _ => Tone::Neutral,
    }
}

/// Every sidebar row leads with 16pt boxes and 8pt gaps, so glyphs share centres and
/// labels share left edges. Child rows indent by one box + gap (8 + 16 + 8) so their
/// icon box sits under the project's colour dot and their text under the project name.
const CHILD_INDENT: f32 = 32.;

fn icon_box() -> Div {
    div().flex_none().size(px(16.)).flex().items_center().justify_center()
}

fn nav_row(id: ElementId, selected: bool, cx: &App) -> Stateful<Div> {
    let t = theme::tokens(cx);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(px(metrics::ROW_NAV))
        .px_2()
        .rounded(px(metrics::R_CONTROL))
        .t_label()
        .cursor_pointer()
        .text_color(if selected { t.text } else { t.text_secondary })
        .when(selected, |row| row.bg(t.selected))
        .when(!selected, move |row| row.hover(move |s| s.bg(t.hover).text_color(t.text)))
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (current, active) = match self.state.workspace.upgrade() {
            Some(workspace) => {
                let ws = workspace.read(cx);
                (ws.active().is_none().then(|| ws.section()), ws.active().map(|tab| tab.target.clone()))
            }
            None => (None, None),
        };
        div()
            .id("sidebar")
            // The shell owns the width (resizable, see `app.rs`).
            .size_full()
            .flex()
            .flex_col()
            .px_2()
            .pt_1()
            .pb_2()
            .bg(cx.theme().sidebar)
            .overflow_y_scroll()
            .child(self.render_sections(current, cx))
            .child(group_label("Projects", cx))
            .child(self.render_projects(active.as_ref(), cx))
    }
}
