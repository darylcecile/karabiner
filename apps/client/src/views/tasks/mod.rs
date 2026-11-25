//! Tasks: board and list for a project (embedded by the project tab), quick-look drawer,
//! and the full Task tab.

mod board;
mod date_field;
mod detail;
pub mod intervention_card;
pub mod runs;
pub mod ui;

use std::collections::HashMap;

use gpui_kit::base::motion::Presence;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{AgentDefinition, CreateTask, ProjectConfig, Intervention, InterventionStatus, Priority, Task, TaskStatus, UpdateTask};

pub use detail::{DetailEvent, TaskDetail, new_id};
use ui::{Load, UpdateInWindow as _};

use crate::{api::query_value, components::{self, Typography as _}, motion, state::{AppState, TabTarget}, theme::metrics};

pub fn tab(target: &TabTarget, state: AppState, window: &mut Window, cx: &mut App) -> AnyView {
    let (project_id, task_id) = (target.project_id.clone(), target.id.clone());
    let view = cx.new(|cx| TaskDetail::new(state.clone(), project_id, task_id, true, window, cx));
    state.track_draft(target, &view, cx);
    view.into()
}

#[derive(Clone, Copy, PartialEq)]
pub enum Layout {
    Board,
    List,
}

/// Board columns: task status, or the stages of the project's first workflow.
#[derive(Clone, Copy, PartialEq)]
pub enum Grouping {
    Status,
    Stage,
}

/// Where a dropped task lands. `None` fields are left unchanged.
#[derive(Clone, Default)]
pub struct Placement {
    pub status: Option<TaskStatus>,
    /// Always set together with `stage_id` when entering a stage, so the pair stays valid.
    pub workflow_id: Option<Option<String>>,
    pub stage_id: Option<Option<String>>,
    pub sort_order: Option<f64>,
}

/// Dragged card payload.
#[derive(Clone)]
pub struct TaskDrag {
    pub id: String,
    pub title: String,
}

impl Render for TaskDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::tokens(cx);
        div().w(px(256.)).p_3().rounded(px(metrics::R_INNER)).border_1().border_color(t.border)
            .bg(t.surface).shadow_md().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child(self.title.clone())
    }
}

pub struct TaskCollection {
    state: AppState,
    project_id: String,
    layout: Layout,
    grouping: Grouping,
    config: Option<ProjectConfig>,
    tasks: Load<Vec<Task>>,
    agents: Vec<AgentDefinition>,
    queued: HashMap<String, usize>,
    search: Entity<InputState>,
    priority_filter: Option<Priority>,
    label_filter: Option<String>,
    agent_filter: Option<String>,
    adding: Option<TaskStatus>,
    /// Workflow + stage a quick-added task is created in (stage board columns).
    adding_stage: Option<(String, String)>,
    board_workflow: Option<String>,
    collapsed_columns: std::collections::HashSet<String>,
    refresh_error: Option<String>,
    new_title: Entity<InputState>,
    drawer: Option<Entity<TaskDetail>>,
    /// Focus for the drawer, so Escape reaches it after a row click.
    drawer_focus: FocusHandle,
    /// Set while a dirty drawer waits for Save / Discard / Cancel. `Some(next)` opens
    /// `next` afterwards; `None` just closes.
    pending_close: Option<Option<String>>,
    _pending_save: Option<gpui_kit::Task<()>>,
    collapsed: Vec<TaskStatus>,
    banner: Option<String>,
    seen_version: u64,
    _load: Option<gpui_kit::Task<()>>,
    _mutations: Vec<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
    _drawer_subscription: Option<Subscription>,
}

impl TaskCollection {
    pub fn new(state: AppState, project_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter tasks"));
        let new_title = cx.new(|cx| InputState::new(window, cx).placeholder("Task title, then Enter"));
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) { cx.notify() }
            }),
            cx.subscribe_in(&new_title, window, |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.create_task(input.clone(), window, cx),
                InputEvent::Blur if input.read(cx).value().trim().is_empty() => {
                    this.adding = None;
                    cx.notify();
                }
                _ => {}
            }),
            cx.observe(&state.service, |this: &mut Self, service, cx| {
                let version = service.read(cx).data_version();
                if version != this.seen_version {
                    this.seen_version = version;
                    this.reload(cx);
                }
            }),
        ];
        let mut this = Self {
            seen_version: state.service.read(cx).data_version(),
            state,
            project_id,
            layout: Layout::Board,
            // Stages first: workflow stages are the board's primary lens; without a workflow
            // the board falls back to status columns.
            grouping: Grouping::Stage,
            config: None,
            tasks: Load::Loading,
            agents: Vec::new(),
            queued: HashMap::new(),
            search,
            priority_filter: None,
            label_filter: None,
            agent_filter: None,
            adding: None,
            adding_stage: None,
            board_workflow: None,
            collapsed_columns: Default::default(),
            refresh_error: None,
            new_title,
            drawer: None,
            drawer_focus: cx.focus_handle(),
            pending_close: None,
            _pending_save: None,
            collapsed: Vec::new(),
            banner: None,
            _load: None,
            _mutations: Vec::new(),
            _subscriptions: subscriptions,
            _drawer_subscription: None,
        };
        this.reload(cx);
        this
    }

    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        cx.notify();
    }

    pub fn set_config(&mut self, config: Option<ProjectConfig>, cx: &mut Context<Self>) {
        if self.config != config {
            self.agents = config.as_ref().map_or_else(Vec::new, |config| config.agents.clone());
            self.config = config;
            cx.notify();
        }
    }

    pub fn set_grouping(&mut self, grouping: Grouping, cx: &mut Context<Self>) {
        self.grouping = grouping;
        cx.notify();
    }

    pub fn grouping(&self) -> Grouping {
        self.grouping
    }

    /// Workflow whose stages the board projects (chosen, else the first).
    pub fn board_workflow(&self) -> Option<&protocol::WorkflowDefinition> {
        let workflows = &self.config.as_ref()?.workflows;
        self.board_workflow.as_ref().and_then(|id| workflows.iter().find(|workflow| &workflow.id == id)).or_else(|| workflows.first())
    }

    pub fn workflows(&self) -> Vec<(String, String)> {
        self.config.as_ref().map_or_else(Vec::new, |config| config.workflows.iter().map(|workflow| (workflow.id.clone(), workflow.name.clone())).collect())
    }

    pub fn set_board_workflow(&mut self, id: String, cx: &mut Context<Self>) {
        self.board_workflow = Some(id);
        cx.notify();
    }

    pub fn toggle_column(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed_columns.remove(key) {
            self.collapsed_columns.insert(key.to_string());
        }
        cx.notify();
    }

    pub fn tasks(&self) -> Option<&Vec<Task>> {
        self.tasks.ready()
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let project = query_value(&self.project_id);
        self._load = Some(cx.spawn(async move |this, cx| {
            let tasks = api.get::<Vec<Task>>(&format!("/v1/tasks?projectId={project}")).await;
            let interventions = api.get::<Vec<Intervention>>(&format!("/v1/interventions?projectId={project}")).await;
            this.update(cx, |this, cx| {
                // A failed refresh keeps the last known data and says so, rather than
                // showing "no tasks" or dropping needs-you markers.
                let mut errors = Vec::new();
                match interventions {
                    Ok(items) => {
                        this.queued.clear();
                        for item in items {
                            if let (Some(task_id), InterventionStatus::Recorded) = (item.task_id, item.status) {
                                *this.queued.entry(task_id).or_default() += 1;
                            }
                        }
                    }
                    Err(error) => errors.push(format!("interventions: {}", ui::error_text(&error))),
                }
                match tasks {
                    Ok(tasks) => this.tasks = Load::Ready(tasks),
                    Err(error) if this.tasks.ready().is_some() => errors.push(format!("tasks: {}", ui::error_text(&error))),
                    Err(error) => this.tasks = Load::Failed(ui::error_text(&error)),
                }
                this.refresh_error = (!errors.is_empty()).then(|| format!("Couldn't refresh {}", errors.join("; ")));
                cx.notify();
            })
            .ok();
        }));
    }

    fn create_task(&mut self, input: Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        let title = input.read(cx).value().trim().to_string();
        let Some(status) = self.adding.clone() else { return };
        let stage = self.adding_stage.clone();
        if title.is_empty() {
            return;
        }
        input.update(cx, |input, cx| input.set_value("", window, cx));
        let sort_order = self.tasks.ready().map_or(0., |tasks| tasks.iter().map(|task| task.sort_order).fold(0., f64::max) + 1.);
        let body = CreateTask {
            project_id: self.project_id.clone(),
            title,
            status: Some(status),
            workflow_id: stage.as_ref().map(|(workflow, _)| Some(workflow.clone())),
            stage_id: stage.map(|(_, stage)| Some(stage)),
            sort_order: Some(sort_order),
            ..Default::default()
        };
        let api = self.state.api.clone();
        self._mutations.push(cx.spawn(async move |this, cx| {
            let result = api.post::<_, Task>("/v1/tasks", &body).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(task) => {
                        if let Some(tasks) = this.tasks.ready_mut() { tasks.push(task) }
                        this.state.data_changed(cx);
                    }
                    Err(error) => this.banner = Some(format!("Couldn't create task. {}", ui::error_text(&error))),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Optimistic move (drag and drop or menu); rolls back if the service refuses.
    pub fn move_task(&mut self, id: &str, placement: Placement, cx: &mut Context<Self>) {
        let Some(task) = self.tasks.ready_mut().and_then(|tasks| tasks.iter_mut().find(|task| task.id == id)) else { return };
        let previous = task.clone();
        if let Some(status) = &placement.status { task.status = status.clone() }
        if let Some(workflow_id) = &placement.workflow_id { task.workflow_id = workflow_id.clone() }
        if let Some(stage_id) = &placement.stage_id { task.stage_id = stage_id.clone() }
        if let Some(sort_order) = placement.sort_order { task.sort_order = sort_order }
        if *task == previous {
            return;
        }
        let body = UpdateTask {
            status: placement.status,
            workflow_id: placement.workflow_id,
            stage_id: placement.stage_id,
            sort_order: placement.sort_order,
            revision: Some(task.revision),
            ..Default::default()
        };
        let (api, id) = (self.state.api.clone(), id.to_string());
        cx.notify();
        self._mutations.push(cx.spawn(async move |this, cx| {
            let result = api.patch::<_, Task>(&format!("/v1/tasks/{}", query_value(&id)), &body).await;
            this.update(cx, |this, cx| {
                let slot = this.tasks.ready_mut().and_then(|tasks| tasks.iter_mut().find(|task| task.id == id));
                match (result, slot) {
                    (Ok(updated), Some(slot)) => {
                        *slot = updated;
                        this.state.data_changed(cx);
                    }
                    (Err(error), Some(slot)) => {
                        *slot = previous;
                        this.banner = Some(format!("Move undone. {}", ui::service_message(&error)));
                    }
                    _ => {}
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn open_drawer(&mut self, task_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.drawer.as_ref().is_some_and(|drawer| drawer.read(cx).task_id() == task_id) {
            return;
        }
        if self.drawer_dirty(cx) {
            self.pending_close = Some(Some(task_id.to_string()));
            cx.notify();
            return;
        }
        self.show_drawer(task_id, window, cx);
    }

    fn show_drawer(&mut self, task_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (state, project_id) = (self.state.clone(), self.project_id.clone());
        let drawer = cx.new(|cx| TaskDetail::new(state, project_id, task_id.to_string(), false, window, cx));
        self._drawer_subscription = Some(cx.subscribe(&drawer, |this, _, event: &DetailEvent, cx| match event {
            DetailEvent::RequestClose => this.request_close(cx),
            DetailEvent::Deleted => this.close_drawer(cx),
        }));
        self.drawer = Some(drawer);
        self.pending_close = None;
        self.drawer_focus.focus(window, cx);
        cx.notify();
    }

    /// Unsaved edits in the quick-look drawer.
    pub fn drawer_dirty(&self, cx: &App) -> bool {
        self.drawer.as_ref().is_some_and(|drawer| drawer.read(cx).is_dirty())
    }

    /// Saves the drawer's draft (used by the project tab's close guard).
    pub fn save_drawer(&mut self, cx: &mut Context<Self>) -> gpui_kit::Task<anyhow::Result<()>> {
        match self.drawer.clone() {
            Some(drawer) if drawer.read(cx).is_dirty() => drawer.update(cx, |drawer, cx| drawer.start_save(cx)),
            _ => gpui_kit::Task::ready(Ok(())),
        }
    }

    fn request_close(&mut self, cx: &mut Context<Self>) {
        if self.drawer_dirty(cx) {
            self.pending_close = Some(None);
            cx.notify();
        } else {
            self.close_drawer(cx);
        }
    }

    fn close_drawer(&mut self, cx: &mut Context<Self>) {
        self.drawer = None;
        self.pending_close = None;
        cx.notify();
    }

    /// Continue the pending close/switch after the draft is resolved.
    fn finish_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pending_close.take() {
            Some(Some(next)) => {
                self.drawer = None;
                self.show_drawer(&next, window, cx);
            }
            Some(None) => self.close_drawer(cx),
            None => {}
        }
    }

    fn pending_save(&mut self, cx: &mut Context<Self>) {
        let save = self.save_drawer(cx);
        self._pending_save = Some(cx.spawn(async move |this, cx| {
            let saved = save.await.is_ok();
            this.update_in_window(cx, |this, window, cx| {
                // A failed save keeps the drawer open with its error/conflict bar; edits made
                // while saving keep it open too (the save bar shows them as unsaved).
                if saved && !this.drawer_dirty(cx) {
                    this.finish_pending(window, cx)
                } else {
                    this.pending_close = None;
                    if saved { this.banner = Some("Saved. Newer edits in the task are still unsaved.".into()) }
                    cx.notify()
                }
            });
        }));
    }

    fn pending_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(drawer) = self.drawer.clone() {
            drawer.update(cx, |drawer, cx| drawer.discard(cx));
        }
        self.finish_pending(window, cx);
    }

    fn render_pending(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.pending_close.as_ref()?;
        let title = self.drawer.as_ref().map(|drawer| drawer.read(cx).title()).unwrap_or_default();
        let amber = ui::needs_you(cx);
        let t = crate::theme::tokens(cx);
        Some(
            components::popover_card(cx).absolute().bottom_3().right_3().w(px(416.)).p_4().gap_1()
                .border_color(amber.opacity(0.5))
                .child(div().t_label().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child("Save changes before leaving?"))
                .child(div().t_body().text_color(t.text_secondary)
                    .child(format!("\"{title}\" has unsaved edits.")))
                .child(div().pt_3().flex().justify_end().gap_2()
                    .child(components::button_sm(Button::new("pending-cancel").ghost().label("Cancel"))
                        .on_click(cx.listener(|this, _, _, cx| { this.pending_close = None; cx.notify() })))
                    .child(components::button_sm(Button::new("pending-discard").outline().label("Discard"))
                        .on_click(cx.listener(|this, _, window, cx| this.pending_discard(window, cx))))
                    .child(components::button_sm(Button::new("pending-save").primary().label("Save"))
                        .on_click(cx.listener(|this, _, _, cx| this.pending_save(cx)))))
                .into_any_element(),
        )
    }

    pub fn start_adding(&mut self, status: TaskStatus, stage_id: Option<(String, String)>, window: &mut Window, cx: &mut Context<Self>) {
        self.adding = Some(status);
        self.adding_stage = stage_id;
        self.new_title.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn visible(&self, cx: &App) -> Vec<Task> {
        let query = self.search.read(cx).value().to_lowercase();
        let mut tasks: Vec<Task> = self.tasks.ready().map_or_else(Vec::new, |tasks| {
            tasks.iter().filter(|task| {
                (query.is_empty() || task.title.to_lowercase().contains(&query) || task.description.to_lowercase().contains(&query))
                    && self.priority_filter.as_ref().is_none_or(|priority| &task.priority == priority)
                    && self.label_filter.as_ref().is_none_or(|label| task.labels.contains(label))
                    && self.agent_filter.as_ref().is_none_or(|agent| task.agent_id.as_ref() == Some(agent))
            }).cloned().collect()
        });
        tasks.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order));
        tasks
    }

    fn agent_name(&self, id: &Option<String>) -> Option<String> {
        let id = id.as_ref()?;
        Some(self.agents.iter().find(|agent| &agent.id == id).map_or_else(|| "Missing agent".into(), |agent| agent.name.clone()))
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let labels: Vec<String> = {
            let mut labels: Vec<String> = self.tasks.ready().into_iter().flatten().flat_map(|task| task.labels.clone()).collect();
            labels.sort();
            labels.dedup();
            labels
        };
        let agents: Vec<(String, String)> = self.agents.iter().map(|agent| (agent.id.clone(), agent.name.clone())).collect();
        let any_filter = self.priority_filter.is_some() || self.label_filter.is_some() || self.agent_filter.is_some();
        div().flex().flex_none().items_center().gap_2().px(px(metrics::GUTTER_X)).py_3()
            .child(div().w(px(220.)).child(components::input_h(Input::new(&self.search), metrics::CONTROL_MD).t_label().prefix(gpui_kit::component::Icon::new(IconName::Search).size(px(14.)))))
            .child(filter_button("filter-priority", "Priority", self.priority_filter.as_ref().map(|p| ui::priority_label(p).to_string()))
                .dropdown_menu({
                    let weak = weak.clone();
                    move |mut menu, _, cx| {
                        let current = weak.upgrade().and_then(|this| this.read(cx).priority_filter.clone());
                        menu = ui::option_menu(menu);
                        for option in Priority::ALL.iter().rev() {
                            let (weak, value, glyph) = (weak.clone(), option.clone(), option.clone());
                            menu = menu.item(ui::option_item(move |cx| ui::priority_bars(&glyph, cx), ui::priority_label(option), None, current.as_ref() == Some(option),
                                move |cx| { weak.update(cx, |this, cx| { this.priority_filter = Some(value.clone()); cx.notify(); }).ok(); }));
                        }
                        menu
                    }
                }))
            .child(filter_button("filter-label", "Label", self.label_filter.clone())
                .dropdown_menu({
                    let weak = weak.clone();
                    move |mut menu, _, cx| {
                        let current = weak.upgrade().and_then(|this| this.read(cx).label_filter.clone());
                        menu = ui::option_menu(menu);
                        if labels.is_empty() { menu = menu.item(PopupMenuItem::new("No labels yet").disabled(true)) }
                        for label in labels.iter() {
                            let (weak, value) = (weak.clone(), label.clone());
                            menu = menu.item(ui::option_item(|cx| label_dot(cx), label.clone(), None, current.as_ref() == Some(label),
                                move |cx| { weak.update(cx, |this, cx| { this.label_filter = Some(value.clone()); cx.notify(); }).ok(); }));
                        }
                        menu
                    }
                }))
            .child(filter_button("filter-agent", "Agent", self.agent_name(&self.agent_filter))
                .dropdown_menu({
                    let weak = weak.clone();
                    move |mut menu, _, cx| {
                        let current = weak.upgrade().and_then(|this| this.read(cx).agent_filter.clone());
                        menu = ui::option_menu(menu);
                        if agents.is_empty() { menu = menu.item(PopupMenuItem::new("No agents configured").disabled(true)) }
                        for (id, name) in agents.iter() {
                            let (weak, value) = (weak.clone(), id.clone());
                            menu = menu.item(ui::option_item(|cx| ui::option_icon(IconName::Bot, cx), name.clone(), None, current.as_ref() == Some(id),
                                move |cx| { weak.update(cx, |this, cx| { this.agent_filter = Some(value.clone()); cx.notify(); }).ok(); }));
                        }
                        menu
                    }
                }))
            .when(any_filter, |this| {
                this.child(components::button_md(Button::new("clear-filters").ghost().label("Clear")).on_click(cx.listener(|this, _, _, cx| {
                    this.priority_filter = None;
                    this.label_filter = None;
                    this.agent_filter = None;
                    cx.notify();
                })))
            })
    }

    /// Drawer and unsaved-changes prompt. The project tab places these over its whole content
    /// area so the drawer spans the full height, header and toolbar included.
    pub fn render_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.render_drawer(window, cx).into_iter().chain(self.render_pending(cx)).collect()
    }

    fn render_drawer(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sample = Presence::new("task-drawer", self.drawer.is_some()).transition(motion::standard()).sample(window, cx);
        let drawer = self.drawer.clone()?;
        if !sample.should_render() {
            return None;
        }
        let progress = sample.progress;
        Some(
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(px(DRAWER_W))
                // Narrow content (small window, wide sidebar) caps it to the view.
                .max_w_full()
                .ml(px((1. - progress) * 24.))
                .opacity(progress)
                // `630BC713`: the drawer's left corners are inside the card, so it rounds
                // them itself (card radius, concentric with the card's own corners) and
                // paints the surface; its right edge sits under the card's frame.
                .rounded_l(px(metrics::R_CARD))
                .border_l_1()
                .border_t_1()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(crate::theme::tokens(cx).surface)
                .shadow_lg()
                // Blocks the project header/toolbar hitboxes it covers, blank areas included.
                .occlude()
                .track_focus(&self.drawer_focus)
                // Escape closes through the guarded path: a dirty draft asks first.
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                        cx.stop_propagation();
                        this.request_close(cx);
                    }
                }))
                .child(drawer)
                .into_any_element(),
        )
    }
}

/// Filter chip (ref 630BC713 "+ Assignee"): 28 high, outlined, leading + or check.
fn filter_button(id: &'static str, label: &'static str, value: Option<String>) -> Button {
    let active = value.is_some();
    components::button_md(Button::new(id).outline()
        .icon(if active { IconName::Check } else { IconName::Plus })
        .label(value.map_or_else(|| label.to_string(), |value| format!("{label}: {value}"))))
        .when(active, |this| this.selected(true))
}

/// Neutral dot for label options (labels carry no colour of their own).
fn label_dot(cx: &App) -> AnyElement {
    div().size(px(8.)).rounded_full().bg(crate::theme::tokens(cx).text_tertiary).into_any_element()
}

/// Quick-look drawer width (ref 630BC713 ≈ 460).
const DRAWER_W: f32 = 460.;

impl Render for TaskCollection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.tasks {
            Load::Loading => ui::skeleton_rows(5, cx).into_any_element(),
            Load::Failed(error) => {
                let weak = cx.weak_entity();
                ui::error_panel("Couldn't load tasks", error, move |_, _, cx| { weak.update(cx, |this, cx| this.reload(cx)).ok(); }, cx).into_any_element()
            }
            Load::Ready(_) => {
                let tasks = self.visible(cx);
                match self.layout {
                    Layout::Board => self.render_board(tasks, window, cx).into_any_element(),
                    Layout::List => self.render_list(tasks, cx).into_any_element(),
                }
            }
        };
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_filters(cx))
            .children(self.refresh_error.clone().map(|error| {
                div().mx(px(metrics::GUTTER_X)).mb_2().flex().items_center().gap_2().t_label().text_color(cx.theme().danger)
                    .child(error)
                    .child(components::button_sm(Button::new("retry-refresh").ghost().label("Retry")).on_click(cx.listener(|this, _, _, cx| this.reload(cx))))
            }))
            .children(self.banner.clone().map(|banner| {
                div().mx(px(metrics::GUTTER_X)).mb_2().pl_3().pr_1().py_1().rounded(px(metrics::R_INNER)).flex().items_center().justify_between().gap_2()
                    .bg(components::Tone::Danger.tint(cx, 0.10)).t_label().text_color(cx.theme().danger)
                    .child(banner)
                    .child(components::icon_button(Button::new("dismiss-banner").ghost().icon(IconName::Close), metrics::CONTROL_SM)
                        .on_click(cx.listener(|this, _, _, cx| { this.banner = None; cx.notify(); })))
            }))
            .child(div().flex_1().min_h_0().child(body))
    }
}
