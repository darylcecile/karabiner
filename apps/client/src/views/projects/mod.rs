//! Projects: the Projects section (all projects) and the Project tab
//! (Board / List / Workflow / Settings).

mod documents_menu;
mod settings;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{Document, DocumentKind, Project, Task, TaskStatus};

use crate::{
    api::query_value,
    components::{self, Typography as _},
    theme::{self, metrics},
    state::{AppState, Draft, Section, TabTarget},
    views::{
        docs::{DocTemplate, TEMPLATES, create_document},
        tasks::{Grouping, Layout, TaskCollection, ui::{self, Load}},
        workflow::{config::ConfigDraft, editor::WorkflowEditor},
    },
};
use settings::ProjectSettings;
use documents_menu::{DocumentEntry, DocumentsMenuActions, documents_menu};
use std::rc::Rc;

/// `create_document` needs `'static` templates.
static DOC_TEMPLATES: [DocTemplate; TEMPLATES.len()] = TEMPLATES;

pub fn section(state: AppState, _window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| ProjectsIndex::new(state, cx)).into()
}

pub fn tab(target: &TabTarget, state: AppState, window: &mut Window, cx: &mut App) -> AnyView {
    let id = target.id.clone();
    let view = cx.new(|cx| ProjectView::new(state.clone(), id, window, cx));
    state.track_draft(target, &view, cx);
    view.into()
}

/// Done / total and percentage for a task set.
fn progress(tasks: &[Task]) -> (usize, usize, f32) {
    let done = tasks.iter().filter(|task| task.status == TaskStatus::Done).count();
    let ratio = if tasks.is_empty() { 0. } else { done as f32 / tasks.len() as f32 };
    (done, tasks.len(), ratio)
}

/// Thin progress bar (ref 630BC713 header).
fn progress_bar(ratio: f32, cx: &App) -> impl IntoElement + use<> {
    div().w(px(120.)).h(px(4.)).rounded_full().bg(crate::theme::tokens(cx).border)
        .child(div().h_full().w(relative(ratio)).rounded_full().bg(crate::theme::tokens(cx).success))
}

/// Source-kind tile (ref 630BC713 header icon): 28 square, R6, tray fill.
fn kind_tile(kind: &protocol::ProjectKind, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div().size(px(28.)).flex_none().rounded(px(metrics::R_CONTROL)).bg(t.tray)
        .flex().items_center().justify_center().child(Icon::new(ui::kind_icon(kind)).size(px(16.)).text_color(t.text_secondary))
}

// ---- Projects section ----------------------------------------------------------------

pub struct ProjectsIndex {
    state: AppState,
    projects: Load<Vec<(Project, Vec<Task>)>>,
    _load: Option<gpui_kit::Task<()>>,
    _subscription: Subscription,
}

impl ProjectsIndex {
    fn new(state: AppState, cx: &mut Context<Self>) -> Self {
        let mut seen = state.service.read(cx).data_version();
        let subscription = cx.observe(&state.service, move |this: &mut Self, service, cx| {
            let version = service.read(cx).data_version();
            if version != seen { seen = version; this.reload(cx) }
        });
        let mut this = Self { state, projects: Load::Loading, _load: None, _subscription: subscription };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let mut rows = Vec::new();
                for project in api.get::<Vec<Project>>("/v1/projects").await? {
                    let tasks = api.get::<Vec<Task>>(&format!("/v1/tasks?projectId={}", query_value(&project.id))).await?;
                    rows.push((project, tasks));
                }
                rows.sort_by(|a, b| b.0.updated_at.cmp(&a.0.updated_at));
                Ok(rows)
            }.await;
            this.update(cx, |this, cx| { this.projects = Load::from_result(result); cx.notify() }).ok();
        }));
    }

    fn render_row(&self, project: &Project, tasks: &[Task], cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (done, total, ratio) = progress(tasks);
        let active = tasks.iter().filter(|task| matches!(task.status, TaskStatus::InProgress | TaskStatus::InReview)).count();
        let blocked = tasks.iter().filter(|task| task.status == TaskStatus::Blocked).count();
        let target = TabTarget::project(project.id.clone());
        let state = self.state.clone();
        let source = match &project.kind {
            protocol::ProjectKind::Github => project.repository.clone(),
            protocol::ProjectKind::Linear => project.external_ref.clone(),
            _ => project.root_path.clone(),
        };
        let t = theme::tokens(cx);
        // Project row card (refs 630BC713 header tile, EB27363B rows): 64 high, R12.
        div().id(SharedString::from(format!("project-{}", project.id)))
            .flex().items_center().gap_3().px_4().h(px(64.)).rounded(px(metrics::R_CARD)).border_1().border_color(t.border)
            .bg(t.surface).cursor_pointer().hover(move |style| style.bg(t.hover))
            .on_click(move |_, window, cx| state.open(target.clone(), window, cx))
            .child(kind_tile(&project.kind, cx))
            .child(div().flex_1().min_w_0().flex().flex_col().gap_0p5()
                .child(div().flex().items_center().gap_2()
                    .child(div().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).truncate().child(project.name.clone()))
                    .child(ui::chip(ui::kind_label(&project.kind), cx)))
                .child(div().t_caption().truncate().text_color(cx.theme().muted_foreground)
                    .child(source.unwrap_or_else(|| if project.description.is_empty() { "No source connected".into() } else { project.description.clone() }))))
            .when(blocked > 0, |this| this.child(ui::amber_chip(format!("{blocked} blocked"), cx)))
            .child(div().w(px(80.)).flex_none().t_caption().text_color(cx.theme().muted_foreground).child(format!("{active} active")))
            .child(div().flex().flex_none().flex_col().items_end().gap_1p5()
                .child(div().t_caption().font_features(components::typography::tabular()).text_color(cx.theme().muted_foreground).child(format!("{done} of {total} done")))
                .child(progress_bar(ratio, cx)))
    }
}

impl Render for ProjectsIndex {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.clone();
        let new_button = components::button_md(Button::new("new-project").primary().icon(IconName::Plus).label("New project"))
            .on_click(move |_, _, cx| state.show_section(Section::Templates, cx));
        let body = match &self.projects {
            Load::Loading => ui::skeleton_rows(4, cx).into_any_element(),
            Load::Failed(error) => {
                let weak = cx.weak_entity();
                ui::error_panel("Couldn't load projects", error, move |_, _, cx| { weak.update(cx, |this, cx| this.reload(cx)).ok(); }, cx).into_any_element()
            }
            Load::Ready(rows) if rows.is_empty() => {
                let state = self.state.clone();
                ui::empty_panel("No projects yet", "Start from a template or an empty project. Standalone, GitHub and Linear sources are supported.",
                    Some(components::button_md(Button::new("first-project").primary().icon(IconName::Plus).label("Choose a template"))
                        .on_click(move |_, _, cx| state.show_section(Section::Templates, cx)).into_any_element()), cx).into_any_element()
            }
            Load::Ready(rows) => {
                let rows: Vec<_> = rows.iter().map(|(project, tasks)| (project.clone(), tasks.clone())).collect();
                div().flex().flex_col().gap_2().children(rows.iter().map(|(project, tasks)| self.render_row(project, tasks, cx))).into_any_element()
            }
        };
        div().id("projects").size_full().overflow_y_scroll().child(
            components::page_body(false).max_w(px(880. + 2. * metrics::GUTTER_X)).mx_auto()
                .child(div().flex().items_start().justify_between().gap_4()
                    .child(components::page_header("Projects", Some("Boards, workflows and agents for each source.".into()), cx))
                    .child(new_button))
                .child(body),
        )
    }
}

// ---- Project tab ---------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum ProjectPane {
    Board,
    List,
    Workflow,
    Settings,
}

pub struct ProjectView {
    state: AppState,
    project_id: String,
    project: Load<Project>,
    documents: Vec<Document>,
    documents_error: Option<String>,
    pane: ProjectPane,
    tasks: Entity<TaskCollection>,
    config: Entity<ConfigDraft>,
    workflow: Entity<WorkflowEditor>,
    settings: Entity<ProjectSettings>,
    _load: Option<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ProjectView {
    fn new(state: AppState, project_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tasks = cx.new(|cx| TaskCollection::new(state.clone(), project_id.clone(), window, cx));
        let config = cx.new(|cx| ConfigDraft::new(state.clone(), project_id.clone(), cx));
        let workflow = cx.new(|cx| WorkflowEditor::new(config.clone(), window, cx));
        let settings = cx.new(|cx| ProjectSettings::new(state.clone(), project_id.clone(), config.clone(), window, cx));
        let mut seen = state.service.read(cx).data_version();
        let subscriptions = vec![
            cx.observe(&state.service, move |this: &mut Self, service, cx| {
                let version = service.read(cx).data_version();
                if version != seen { seen = version; this.reload(cx) }
            }),
            // Board/list follow the saved configuration (agents, stages), not unsaved drafts.
            cx.observe(&config, |this, config, cx| {
                let saved = config.read(cx).status().ready().cloned();
                this.tasks.update(cx, |tasks, cx| tasks.set_config(saved, cx));
            }),
            cx.observe(&tasks, |_, _, cx| cx.notify()),
            cx.observe(&settings, |_, _, cx| cx.notify()),
        ];
        let mut this = Self { state, project_id, project: Load::Loading, documents: Vec::new(), documents_error: None, pane: ProjectPane::Board, tasks, config, workflow, settings, _load: None, _subscriptions: subscriptions };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/projects/{}", query_value(&self.project_id));
        let documents_path = format!("/v1/documents?projectId={}", query_value(&self.project_id));
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = api.get::<Project>(&path).await;
            let documents = api.get::<Vec<Document>>(&documents_path).await;
            this.update(cx, |this, cx| {
                // Keep the last list on failure; the menu says it couldn't refresh.
                match documents {
                    Ok(documents) => { this.documents = documents; this.documents_error = None }
                    Err(error) => this.documents_error = Some(ui::error_text(&error)),
                }
                if let Ok(project) = &result {
                    this.state.set_title(&TabTarget::project(this.project_id.clone()), project.name.clone(), cx);
                    let project = project.clone();
                    this.settings.update(cx, |settings, cx| settings.set_project(project, cx));
                }
                this.project = Load::from_result(result);
                cx.notify();
            }).ok();
        }));
    }

    fn set_pane(&mut self, pane: ProjectPane, cx: &mut Context<Self>) {
        self.pane = pane;
        match pane {
            ProjectPane::Board => self.tasks.update(cx, |tasks, cx| tasks.set_layout(Layout::Board, cx)),
            ProjectPane::List => self.tasks.update(cx, |tasks, cx| tasks.set_layout(Layout::List, cx)),
            _ => {}
        }
        cx.notify();
    }

    fn render_header(&self, project: &Project, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tasks = self.tasks.read(cx).tasks().cloned().unwrap_or_default();
        let (done, total, ratio) = progress(&tasks);
        let state = self.state.clone();
        let blocked = tasks.iter().filter(|task| task.status == TaskStatus::Blocked).count();
        let t = theme::tokens(cx);
        // Project header (ref 630BC713): breadcrumb, 28 tile + title, subtitle, progress.
        div().flex_none().px(px(metrics::GUTTER_X)).pt(px(metrics::GUTTER_TOP)).pb_3().flex().flex_col()
            .child(div().flex().items_center().gap_1p5().pb_3().t_label().text_color(t.text_secondary)
                .child(div().id("crumb-projects").cursor_pointer().hover(|style| style.text_color(cx.theme().foreground)).child("Projects")
                    .on_click(move |_, _, cx| state.show_section(Section::Projects, cx)))
                .child(Icon::new(IconName::ChevronRight).size(px(14.)).text_color(t.text_tertiary))
                .child(div().text_color(t.text).child(project.name.clone())))
            .child(div().flex().items_center().gap_3()
                .child(kind_tile(&project.kind, cx))
                .child(div().min_w_0().truncate().t_title().text_color(t.text).child(project.name.clone()))
                .child(ui::chip(ui::kind_label(&project.kind), cx))
                .when(blocked > 0, |this| this.child(ui::amber_chip(format!("{blocked} need you"), cx))))
            .when(!project.description.is_empty(), |this| this.child(div().pt_1().t_body().text_color(t.text_secondary).child(project.description.clone())))
            .child(div().pt_3().flex().items_center().gap_3()
                .child(progress_bar(ratio, cx))
                .child(div().t_caption().font_features(components::typography::tabular()).text_color(t.text_secondary)
                    .child(format!("{:.0}% · {done} of {total} done", ratio * 100.))))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let tasks = self.tasks.clone();
        let grouping = tasks.read(cx).grouping();
        let workflows = tasks.read(cx).workflows();
        let has_stages = !workflows.is_empty();
        let board_workflow = tasks.read(cx).board_workflow().map_or_else(String::new, |workflow| workflow.name.clone());
        let tasks_for_menu = tasks.clone();
        // One 36 row: view tabs left, grouping/workflow/documents right (visual contract).
        // Tab labels (8 inner padding) on the title's 24 edge; trailing 24 controls 6 from the
        // right like their 6 vertical inset.
        // When tabs + actions can't share 36, the action group wraps to a second 36 line
        // (6 inset kept): Documents keeps its full label and count, every tab stays visible.
        div().flex_none().min_h(px(36.)).pl(px(metrics::GUTTER_X - 8.)).pr_1p5().flex().flex_wrap().items_center().gap_x_2().border_b_1().border_color(cx.theme().border)
            .child(div().flex().h(px(36.)).flex_none().gap_1().children([
                (ProjectPane::Board, "Board", gpui_kit::assets::IconName::Kanban),
                (ProjectPane::List, "List", gpui_kit::assets::IconName::List),
                (ProjectPane::Workflow, "Workflow", gpui_kit::assets::IconName::Workflow),
                (ProjectPane::Settings, "Settings", gpui_kit::assets::IconName::Settings),
            ].into_iter().map(|(pane, label, icon)| {
                let active = self.pane == pane;
                let weak = weak.clone();
                div().id(label).flex().items_center().gap_1p5().px_2().h_full().cursor_pointer().t_label()
                    .when(active, |this| this.font_weight(FontWeight::MEDIUM))
                    .border_b_2().border_color(if active { cx.theme().foreground } else { transparent_black() })
                    .text_color(if active { cx.theme().foreground } else { cx.theme().muted_foreground })
                    .hover(|style| style.text_color(cx.theme().foreground))
                    .child(Icon::new(icon).size(px(14.))).child(label)
                    .on_click(move |_, _, cx| { weak.update(cx, |this, cx| this.set_pane(pane, cx)).ok(); })
            })))
            .child(div().ml_auto().h(px(36.)).flex_none().flex().items_center().gap_2()
                .when(self.pane == ProjectPane::Board && grouping == Grouping::Stage && workflows.len() > 1, |this| this.child(
                    components::button_sm(Button::new("board-workflow").ghost().label(board_workflow.clone()).dropdown_caret(true)).dropdown_menu({
                        let tasks = tasks_for_menu.clone();
                        move |mut menu, _, _| {
                            for (id, name) in workflows.iter() {
                                let (tasks, id) = (tasks.clone(), id.clone());
                                menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                                    tasks.update(cx, |tasks, cx| tasks.set_board_workflow(id.clone(), cx));
                                }));
                            }
                            menu
                        }
                    })))
                .when(self.pane == ProjectPane::Board && has_stages, |this| this.child(
                    ui::segmented("board-grouping", &[(Grouping::Status, "Status"), (Grouping::Stage, "Stages")], grouping,
                        move |grouping, _, cx| tasks.update(cx, |tasks, cx| tasks.set_grouping(grouping, cx)), cx)))
                .child(self.render_documents_menu()))
    }
}

impl ProjectView {
    /// Project documents (open as tabs) plus "New" from the docs templates (PRD, ADR, …).
    fn render_documents_menu(&self) -> impl IntoElement + use<> {
        let documents: Vec<(TabTarget, String, bool)> = self.documents.iter().map(|document| {
            let tour = document.kind == DocumentKind::CodeTour;
            let target = if tour {
                TabTarget::review(document.project_id.clone(), document.id.clone())
            } else {
                TabTarget::document(document.project_id.clone(), document.id.clone())
            };
            (target, document.title.clone(), tour)
        }).collect();
        let (state, project_id) = (self.state.clone(), self.project_id.clone());
        let entries = documents.iter().map(|(_, title, tour)| DocumentEntry { title: title.clone(), tour: *tour }).collect();
        let targets: Vec<TabTarget> = documents.into_iter().map(|(target, _, _)| target).collect();
        let actions = DocumentsMenuActions {
            on_retry: { let state = state.clone(); Rc::new(move |_: &mut Window, cx: &mut App| state.data_changed(cx)) },
            on_open: { let state = state.clone(); Rc::new(move |index: usize, window: &mut Window, cx: &mut App| state.open(targets[index].clone(), window, cx)) },
            on_new: Rc::new(move |index: usize, window: &mut Window, cx: &mut App| {
                let template = &DOC_TEMPLATES[index];
                let state_for_error = state.clone();
                let task = create_document(state.clone(), project_id.clone(), template, window, cx);
                let window_handle = window.window_handle();
                cx.spawn(async move |cx| {
                    if let Err(error) = task.await {
                        window_handle.update(cx, |_, window, cx| { state_for_error.report_error(&error, window, cx); }).ok();
                    }
                }).detach();
            }),
        };
        documents_menu(entries, self.documents_error.clone(), DOC_TEMPLATES.iter().map(|template| template.name).collect(), actions)
    }
}

impl Render for ProjectView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let project = match &self.project {
            Load::Loading => return ui::skeleton_rows(4, cx).into_any_element(),
            Load::Failed(error) => {
                let weak = cx.weak_entity();
                return ui::error_panel("Couldn't load project", error, move |_, _, cx| { weak.update(cx, |this, cx| this.reload(cx)).ok(); }, cx).into_any_element();
            }
            Load::Ready(project) => project.clone(),
        };
        let content = match self.pane {
            ProjectPane::Board | ProjectPane::List => self.tasks.clone().into_any_element(),
            ProjectPane::Workflow => self.workflow.clone().into_any_element(),
            ProjectPane::Settings => self.settings.clone().into_any_element(),
        };
        let draft = self.config.read(cx);
        let config_bar = (draft.dirty || draft.conflict) && matches!(self.pane, ProjectPane::Workflow | ProjectPane::Settings);
        let (conflict, saving, error) = (draft.conflict, draft.saving, draft.error.clone());
        // The task drawer overlays the whole tab content (header and toolbar included).
        let overlay = if matches!(self.pane, ProjectPane::Board | ProjectPane::List) {
            self.tasks.update(cx, |tasks, cx| tasks.render_overlay(window, cx))
        } else {
            Vec::new()
        };
        div().relative().size_full().flex().flex_col()
            .child(self.render_header(&project, cx))
            .child(self.render_toolbar(cx))
            .child(div().flex_1().min_h_0().child(content))
            .when(config_bar, |this| {
                let (discard, save) = (self.config.downgrade(), self.config.downgrade());
                this.child(ui::save_bar("config-save", conflict, saving, error.as_deref(),
                    move |_, _, cx| { discard.update(cx, |config, cx| config.discard(cx)).ok(); },
                    move |_, _, cx| { save.update(cx, |config, cx| if config.conflict { config.keep_mine(cx) } else { config.save(cx) }).ok(); },
                    cx))
            })
            .children(overlay)
            .into_any_element()
    }
}

/// One close guard for the whole project tab: workflow/agent config, general settings and
/// the task drawer.
impl Draft for ProjectView {
    fn is_dirty(&self, cx: &App) -> bool {
        self.config.read(cx).dirty || self.settings.read(cx).is_dirty() || self.tasks.read(cx).drawer_dirty(cx)
    }

    fn save(&mut self, _: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Task<anyhow::Result<()>> {
        let config = self.config.update(cx, |config, cx| config.start_save(cx));
        let settings = self.settings.update(cx, |settings, cx| settings.start_save(cx));
        let drawer = self.tasks.update(cx, |tasks, cx| tasks.save_drawer(cx));
        cx.background_spawn(async move {
            let (config, settings, drawer) = futures::join!(config, settings, drawer);
            config.and(settings).and(drawer)
        })
    }
}
