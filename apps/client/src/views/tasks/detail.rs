//! Task detail: used as the board/list quick-look drawer and as the full Task tab.
//! Edits are a local draft with a base revision; refetches never overwrite a dirty draft.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    checkbox::Checkbox,
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::{CalendarEvent, CalendarState, Date};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{
    Activity, AgentDefinition, CreateComment, CreateIntervention, Subtask, Intervention, InterventionKind, InterventionStatus,
    Priority, ProjectConfig, Task, TaskStatus, UpdateIntervention, UpdateTask, WorkflowDefinition,
};

use super::date_field;
use super::intervention_card;
use super::runs::{self, RunsPanel};
use super::ui::{self, Load, UpdateInWindow as _};
use crate::{api::query_value, components::{self, Typography as _}, state::{AppState, Draft, TabTarget}, theme::{self, metrics}};

pub enum DetailEvent {
    /// The user asked to close the drawer (may have unsaved edits).
    RequestClose,
    /// The task was deleted; close without prompting.
    Deleted,
}

impl EventEmitter<DetailEvent> for TaskDetail {}

#[derive(Clone, Copy, PartialEq)]
enum Pane {
    Activity,
    Runs,
}

pub struct TaskDetail {
    state: AppState,
    project_id: String,
    task_id: String,
    full: bool,
    task: Load<Task>,
    config: Option<ProjectConfig>,
    activity: Load<Vec<Activity>>,
    interventions: Load<Vec<Intervention>>,
    runs: Entity<RunsPanel>,
    pane: Pane,
    /// Full tab's measured width; below `RAIL_MIN_W` the properties stack above the content.
    body_width: std::rc::Rc<std::cell::Cell<Option<Pixels>>>,
    // Draft
    title: Entity<InputState>,
    description: Entity<TextareaState>,
    label_input: Entity<InputState>,
    status: TaskStatus,
    priority: Priority,
    agent_id: Option<String>,
    workflow_id: Option<String>,
    stage_id: Option<String>,
    labels: Vec<String>,
    subtasks: Vec<Subtask>,
    due: Entity<CalendarState>,
    due_open: bool,
    /// Typed-date field in the due picker header; parsed into the calendar's pending date.
    due_input: Entity<InputState>,
    /// Draft due date as `YYYY-MM-DD` (empty = none), mirrored from the calendar.
    due_value: String,
    subtask_input: Entity<InputState>,
    comment: Entity<InputState>,
    base_revision: u64,
    dirty: bool,
    conflict: bool,
    saving: bool,
    save_error: Option<String>,
    syncing_inputs: bool,
    // Intervention compose
    compose_kind: InterventionKind,
    compose: Entity<TextareaState>,
    compose_error: Option<String>,
    sending: bool,
    seen_version: u64,
    /// Bumped on every draft edit; a save only adopts the response if no edit happened meanwhile.
    generation: u64,
    /// The due date shown when the record was adopted; unchanged means "don't rewrite dueAt".
    due_loaded: String,
    responding: Option<String>,
    response: Entity<InputState>,
    read_error: Option<String>,
    _load: Option<gpui_kit::Task<()>>,
    /// Accepted writes (save, comment, intervention, delete); never cancelled by refreshes.
    _tasks: Vec<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TaskDetail {
    pub fn new(
        state: AppState,
        project_id: String,
        task_id: String,
        full: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Task title"));
        let description = cx.new(|cx| {
            TextareaState::new(window, cx).auto_grow(3, 14).placeholder("Add a description…")
        });
        let label_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add label"));
        let compose = cx.new(|cx| {
            TextareaState::new(window, cx).auto_grow(2, 6).placeholder("Write an instruction or answer for the agent…")
        });
        let runs = cx.new(|cx| RunsPanel::new(state.clone(), project_id.clone(), task_id.clone(), window, cx));
        let due = cx.new(|cx| CalendarState::new(window, cx));
        let due_input = cx.new(|cx| InputState::new(window, cx).placeholder("What date is this due?"));
        let subtask_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add subtask"));
        let comment = cx.new(|cx| InputState::new(window, cx).placeholder("Leave a comment"));
        let response = cx.new(|cx| InputState::new(window, cx).placeholder("Your answer, then Enter"));
        let mut subscriptions = vec![
            cx.subscribe_in(&title, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) { this.mark_dirty(cx) }
            }),
            cx.subscribe_in(&description, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) { this.mark_dirty(cx) }
            }),
            cx.subscribe_in(&label_input, window, |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let label = input.read(cx).value().trim().to_string();
                    if !label.is_empty() && !this.labels.contains(&label) {
                        this.labels.push(label);
                        this.mark_dirty(cx);
                    }
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                }
            }),
        ];
        // A picked day is a draft edit only; Save sends it. Display is ISO YYYY-MM-DD.
        subscriptions.push(cx.subscribe_in(&due, window, |this, _, event: &CalendarEvent, _, cx| {
            let CalendarEvent::Selected(Date::Single(date)) = event else { return };
            this.due_open = false;
            this.set_due(date.map_or_else(String::new, |date| date.to_string()), cx);
        }));
        // Typing only moves the picker's pending date; Enter applies it like "Apply date".
        subscriptions.push(cx.subscribe_in(&due_input, window, |this, input, event: &InputEvent, window, cx| {
            let text = input.read(cx).value().trim().to_string();
            let parsed = date_field::parse(&text, this.due.read(cx).today());
            match event {
                InputEvent::Change if parsed.is_some() => date_field::show_pending(&this.due, parsed, window, cx),
                InputEvent::PressEnter { .. } if parsed.is_some() => {
                    date_field::show_pending(&this.due, parsed, window, cx);
                    date_field::apply(&this.due, cx);
                }
                InputEvent::PressEnter { .. } if text.is_empty() => date_field::apply(&this.due, cx),
                _ => {}
            }
        }));
        subscriptions.push(cx.subscribe_in(&subtask_input, window, |this, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let title = input.read(cx).value().trim().to_string();
                if !title.is_empty() {
                    this.subtasks.push(Subtask { id: new_id("sub"), title, completed: false });
                    this.mark_dirty(cx);
                }
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        }));
        subscriptions.push(cx.subscribe_in(&comment, window, |this, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let message = input.read(cx).value().trim().to_string();
                if !message.is_empty() {
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                    this.post_comment(message, cx);
                }
            }
        }));
        subscriptions.push(cx.subscribe_in(&response, window, |this, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = input.read(cx).value().trim().to_string();
                if let (false, Some(id)) = (text.is_empty(), this.responding.clone()) {
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                    this.answer_intervention(&id, text, cx);
                }
            }
        }));
        subscriptions.push(cx.observe(&state.service, |this: &mut Self, service, cx| {
            let version = service.read(cx).data_version();
            if version != this.seen_version {
                this.seen_version = version;
                this.reload(cx);
            }
        }));
        let mut this = Self {
            seen_version: state.service.read(cx).data_version(),
            state,
            project_id,
            task_id,
            full,
            task: Load::Loading,
            config: None,
            activity: Load::Loading,
            interventions: Load::Loading,
            runs,
            pane: Pane::Activity,
            body_width: Default::default(),
            title,
            description,
            label_input,
            status: TaskStatus::Todo,
            priority: Priority::Normal,
            agent_id: None,
            workflow_id: None,
            stage_id: None,
            labels: Vec::new(),
            subtasks: Vec::new(),
            due,
            due_open: false,
            due_input,
            due_value: String::new(),
            subtask_input,
            comment,
            base_revision: 0,
            dirty: false,
            conflict: false,
            saving: false,
            save_error: None,
            syncing_inputs: false,
            compose_kind: InterventionKind::Instruction,
            compose,
            compose_error: None,
            sending: false,
            generation: 0,
            due_loaded: String::new(),
            responding: None,
            response,
            read_error: None,
            _load: None,
            _tasks: Vec::new(),
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        if !self.syncing_inputs {
            self.touch(cx);
        }
    }

    /// Records a draft edit.
    fn set_due(&mut self, value: String, cx: &mut Context<Self>) {
        if !self.syncing_inputs && value != self.due_value {
            self.due_value = value;
            self.touch(cx);
        }
    }

    fn clear_due(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.due.update(cx, |calendar, cx| calendar.set_date(Date::Single(None), window, cx));
        self.set_due(String::new(), cx);
    }

    fn touch(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.generation += 1;
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let (task_id, project_id) = (self.task_id.clone(), self.project_id.clone());
        let load = cx.spawn(async move |this, cx| {
            let task = api.get::<Task>(&format!("/v1/tasks/{}", query_value(&task_id))).await;
            let config = api.get::<ProjectConfig>(&format!("/v1/projects/{}/config", query_value(&project_id))).await.ok();
            let query = format!("projectId={}&taskId={}", query_value(&project_id), query_value(&task_id));
            let activity = api.get::<Vec<Activity>>(&format!("/v1/activity?{query}&limit=100")).await;
            let interventions = api.get::<Vec<Intervention>>(&format!("/v1/interventions?{query}")).await;
            this.update(cx, |this, cx| {
                if config.is_some() { this.config = config }
                // Keep previously loaded lists if a refresh fails; show a compact error instead.
                let mut errors = Vec::new();
                match activity {
                    Ok(items) => this.activity = Load::Ready(items),
                    Err(error) if this.activity.ready().is_some() => errors.push(ui::error_text(&error)),
                    Err(error) => this.activity = Load::Failed(ui::error_text(&error)),
                }
                match interventions {
                    Ok(items) => this.interventions = Load::Ready(items),
                    Err(error) if this.interventions.ready().is_some() => errors.push(ui::error_text(&error)),
                    Err(error) => this.interventions = Load::Failed(ui::error_text(&error)),
                }
                this.read_error = (!errors.is_empty()).then(|| format!("Couldn't refresh: {}", errors.join(" ")));
                this.apply_task(task, cx);
                cx.notify();
            })
            .ok();
        });
        self._load = Some(load);
    }

    fn apply_task(&mut self, result: Result<Task, crate::api::ApiError>, cx: &mut Context<Self>) {
        match result {
            Ok(task) => {
                if self.dirty {
                    self.conflict = task.revision != self.base_revision;
                } else {
                    self.adopt(&task, cx);
                }
                if self.full {
                    self.state.set_title(&TabTarget::task(self.project_id.clone(), self.task_id.clone()), task.title.clone(), cx);
                }
                self.task = Load::Ready(task);
            }
            Err(error) if self.task.ready().is_some() => self.read_error = Some(format!("Couldn't refresh: {}", ui::error_text(&error))),
            Err(error) => self.task = Load::Failed(ui::error_text(&error)),
        }
    }

    /// Replace the draft with the server record. Input change events are ignored meanwhile.
    fn adopt(&mut self, task: &Task, cx: &mut Context<Self>) {
        self.status = task.status.clone();
        self.priority = task.priority.clone();
        self.agent_id = task.agent_id.clone();
        self.workflow_id = task.workflow_id.clone();
        self.stage_id = task.stage_id.clone();
        self.labels = task.labels.clone();
        self.subtasks = task.subtasks.clone();
        self.base_revision = task.revision;
        self.dirty = false;
        self.conflict = false;
        self.save_error = None;
        let (title, description) = (task.title.clone(), task.description.clone());
        let due = task.due_at.as_deref().and_then(|due| due.get(..10)).unwrap_or("").to_string();
        self.due_loaded = due.clone();
        self.due_value = due.clone();
        self.syncing_inputs = true;
        if let Some(window) = cx.active_window() {
            let (title_input, description_input, calendar) = (self.title.clone(), self.description.clone(), self.due.clone());
            // An open picker keeps its pending (unapplied) date; it resets to the new
            // committed value when it next opens or closes.
            let picker_open = self.due_open;
            window
                .update(cx, |_, window, cx| {
                    title_input.update(cx, |input, cx| input.set_value(title.as_str(), window, cx));
                    description_input.update(cx, |input, cx| input.set_value(description.as_str(), window, cx));
                    if !picker_open {
                        calendar.update(cx, |calendar, cx| calendar.set_date(Date::Single(due.parse().ok()), window, cx));
                    }
                })
                .ok();
        }
        self.syncing_inputs = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn title(&self) -> String {
        self.task.ready().map_or_else(String::new, |task| task.title.clone())
    }

    pub fn discard(&mut self, cx: &mut Context<Self>) {
        self.dirty = false;
        self.conflict = false;
        self.reload(cx);
    }

    /// Explicit "Keep mine": rebase on the latest fetched revision, then save.
    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        if let Some(task) = self.task.ready() {
            self.base_revision = task.revision;
        }
        self.conflict = false;
        self.start_save(cx).detach();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.start_save(cx).detach();
    }

    /// Sends the draft; resolves `Ok` only once the service accepted it.
    pub fn start_save(&mut self, cx: &mut Context<Self>) -> gpui_kit::Task<anyhow::Result<()>> {
        let title = self.title.read(cx).value().trim().to_string();
        if title.is_empty() {
            self.save_error = Some("A task needs a title.".into());
            cx.notify();
            return gpui_kit::Task::ready(Err(anyhow::anyhow!("A task needs a title.")));
        }
        if self.conflict {
            return gpui_kit::Task::ready(Err(anyhow::anyhow!("This task changed elsewhere; choose Reload or Keep mine first.")));
        }
        let due = self.due_value.clone();
        let due_at = match due.as_str() {
            unchanged if unchanged == self.due_loaded => None,
            "" => Some(None),
            date if is_date(date) => Some(Some(format!("{date}T00:00:00.000Z"))),
            _ => {
                self.save_error = Some("Due date must look like 2026-12-01.".into());
                cx.notify();
                return gpui_kit::Task::ready(Err(anyhow::anyhow!("Invalid due date")));
            }
        };
        let body = UpdateTask {
            stage_id: Some(self.stage_id.clone()),
            due_at,
            subtasks: Some(self.subtasks.clone()),
            title: Some(title),
            description: Some(self.description.read(cx).value().to_string()),
            status: Some(self.status.clone()),
            priority: Some(self.priority.clone()),
            agent_id: Some(self.agent_id.clone()),
            workflow_id: Some(self.workflow_id.clone()),
            labels: Some(self.labels.clone()),
            revision: Some(self.base_revision),
            ..Default::default()
        };
        self.saving = true;
        self.save_error = None;
        cx.notify();
        let api = self.state.api.clone();
        let path = format!("/v1/tasks/{}", query_value(&self.task_id));
        let (submitted, base) = (self.generation, self.base_revision);
        cx.spawn(async move |this, cx| {
            let result = api.patch::<_, Task>(&path, &body).await;
            // On 409, fetch the latest record (draft kept) to tell a stale revision apart
            // from a dependency refusal, which leaves the revision unchanged.
            let latest = match &result {
                Err(error) if error.is_conflict() => api.get::<Task>(&path).await.ok(),
                _ => None,
            };
            let outcome = result.as_ref().map(|_| ()).map_err(|error| anyhow::anyhow!(ui::service_message(error)));
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(task) if this.generation == submitted => {
                        this.adopt(&task, cx);
                        this.task = Load::Ready(task);
                        this.state.data_changed(cx);
                    }
                    Ok(task) => {
                        // Edited while saving: keep the newer draft on top of the saved revision.
                        this.base_revision = task.revision;
                        // The accepted dueAt is the new baseline, so a later save sends any
                        // difference from it (e.g. the user changed the date back).
                        this.due_loaded = task.due_at.as_deref().and_then(|due| due.get(..10)).unwrap_or("").to_string();
                        this.task = Load::Ready(task);
                        this.state.data_changed(cx);
                    }
                    Err(error) if error.is_conflict() => match latest {
                        Some(task) if task.revision != base => {
                            this.conflict = true;
                            this.task = Load::Ready(task);
                        }
                        _ => this.save_error = Some(ui::service_message(&error)),
                    },
                    Err(error) => this.save_error = Some(ui::error_text(&error)),
                }
                cx.notify();
            })
            .ok();
            outcome
        })
    }

    fn post_comment(&mut self, message: String, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/tasks/{}/comments", query_value(&self.task_id));
        let post = cx.spawn(async move |this, cx| {
            let result = api.post::<_, Activity>(&path, &CreateComment { message }).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(activity) => {
                        if let Some(items) = this.activity.ready_mut() { items.insert(0, activity) }
                        this.state.data_changed(cx);
                    }
                    Err(error) => this.compose_error = Some(format!("Comment not saved. {}", ui::error_text(&error))),
                }
                cx.notify();
            })
            .ok();
        });
        self._tasks.push(post);
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/tasks/{}", query_value(&self.task_id));
        let delete = cx.spawn(async move |this, cx| {
            let result = api.delete(&path).await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.state.data_changed(cx);
                    if this.full {
                        this.state.close(&TabTarget::task(this.project_id.clone(), this.task_id.clone()), cx);
                    }
                    cx.emit(DetailEvent::Deleted);
                }
                Err(error) => {
                    this.save_error = Some(ui::error_text(&error));
                    cx.notify();
                }
            })
            .ok();
        });
        self._tasks.push(delete);
    }

    fn send_intervention(&mut self, cx: &mut Context<Self>) {
        let message = self.compose.read(cx).value().trim().to_string();
        if message.is_empty() {
            return;
        }
        let body = CreateIntervention {
            project_id: self.project_id.clone(),
            task_id: Some(self.task_id.clone()),
            kind: self.compose_kind.clone(),
            message,
        };
        self.sending = true;
        self.compose_error = None;
        cx.notify();
        let api = self.state.api.clone();
        let send = cx.spawn(async move |this, cx| {
            let result = api.post::<_, Intervention>("/v1/interventions", &body).await;
            this.update_in_window(cx, |this, window, cx| {
                this.sending = false;
                match result {
                    Ok(_) => {
                        this.compose.update(cx, |input, cx| input.set_value("", window, cx));
                        this.state.data_changed(cx);
                    }
                    Err(error) => this.compose_error = Some(ui::error_text(&error)),
                }
                cx.notify();
            });
        });
        self._tasks.push(send);
    }

    fn answer_intervention(&mut self, id: &str, response: String, cx: &mut Context<Self>) {
        let Some(intervention) = self.interventions.ready().and_then(|list| list.iter().find(|item| item.id == id)).cloned() else { return };
        self.responding = None;
        self.update_intervention(&intervention, InterventionStatus::Answered, Some(response), cx);
    }

    fn resolve_intervention(&mut self, intervention: &Intervention, status: InterventionStatus, cx: &mut Context<Self>) {
        self.update_intervention(intervention, status, None, cx);
    }

    fn update_intervention(&mut self, intervention: &Intervention, status: InterventionStatus, response: Option<String>, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/interventions/{}", query_value(&intervention.id));
        let body = UpdateIntervention { revision: Some(intervention.revision), status: Some(status), response: response.map(Some) };
        let update = cx.spawn(async move |this, cx| {
            let result = api.patch::<_, Intervention>(&path, &body).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.state.data_changed(cx),
                    Err(error) => this.compose_error = Some(ui::service_message(&error)),
                }
                cx.notify();
            })
            .ok();
        });
        self._tasks.push(update);
    }

    fn agents(&self) -> &[AgentDefinition] {
        self.config.as_ref().map_or(&[], |config| config.agents.as_slice())
    }

    fn selected_workflow(&self) -> Option<&WorkflowDefinition> {
        let id = self.workflow_id.as_ref()?;
        self.workflows().iter().find(|workflow| &workflow.id == id)
    }

    /// Changing workflow clears a stage that doesn't belong to the new workflow.
    fn set_workflow(&mut self, workflow_id: Option<String>, cx: &mut Context<Self>) {
        self.workflow_id = workflow_id;
        let valid = self.selected_workflow().is_some_and(|workflow| workflow.stages.iter().any(|stage| Some(&stage.id) == self.stage_id.as_ref()));
        if !valid { self.stage_id = None }
        self.touch(cx);
    }

    fn workflows(&self) -> &[WorkflowDefinition] {
        self.config.as_ref().map_or(&[], |config| config.workflows.as_slice())
    }
}

// ---- Rendering -----------------------------------------------------------------------

impl TaskDetail {
    fn render_header(&self, task: &Task, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (project_id, task_id) = (self.project_id.clone(), self.task_id.clone());
        let t = theme::tokens(cx);
        // Full tab: 28 controls in the 44 header bar; drawer: 24 controls in its 20 padding.
        let control = if self.full { metrics::CONTROL_MD } else { metrics::CONTROL_SM };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .h(px(control))
            .child(
                div().flex().items_center().gap_2().t_caption().text_color(t.text_secondary)
                    .child(div().t_code_caption(cx).child(short_id(&task.id)))
                    .child(format!("Updated {}", ui::short_date(&task.updated_at))),
            )
            .child(
                div().flex().items_center().gap_0p5()
                    .when(!self.full, |this| {
                        let state = self.state.clone();
                        this.child(
                            components::icon_button(Button::new("open-full").ghost().icon(IconName::Maximize).tooltip("Open as tab"), control)
                                .on_click(move |_, window, cx| state.open(TabTarget::task(project_id.clone(), task_id.clone()), window, cx)),
                        )
                    })
                    .child(
                        components::icon_button(Button::new("task-actions").ghost().icon(IconName::Ellipsis), control).dropdown_menu({
                            let this = cx.weak_entity();
                            move |menu, _, _| {
                                let this = this.clone();
                                menu.item(PopupMenuItem::new("Delete task").icon(gpui_kit::assets::IconName::Trash).on_click(move |_, _, cx| {
                                    this.update(cx, |this, cx| this.delete(cx)).ok();
                                }))
                            }
                        }),
                    )
                    .when(!self.full, |this| {
                        this.child(components::icon_button(Button::new("close-drawer").ghost().icon(IconName::Close).tooltip("Close"), control)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DetailEvent::RequestClose))))
                    }),
            )
    }

    /// Property grid row (ref 630BC713): label column, 28 pitch, 13pt label and value.
    fn property_row(label: &'static str, label_w: f32, value: AnyElement, cx: &App) -> impl IntoElement {
        div().flex().items_center().min_h(px(metrics::ROW_PROP)).t_label()
            .child(div().w(px(label_w)).flex_none().text_color(theme::tokens(cx).text_secondary).child(label))
            .child(div().flex_1().min_w_0().flex().items_center().justify_start().text_color(theme::tokens(cx).text).child(value))
    }

    /// Value controls in display order. Menus read the live draft when opened, so the check
    /// marks the current value.
    fn property_values(&self, cx: &mut Context<Self>) -> Properties {
        let weak = cx.weak_entity();
        let status = self.status.clone();
        let status = prop_button("status-menu", cx)
            .child(prop_value(ui::status_icon(&status, cx), ui::status_label(&status).to_string()))
            .dropdown_menu({
                let weak = weak.clone();
                move |mut menu, _, cx| {
                    let current = weak.upgrade().map(|this| this.read(cx).status.clone());
                    menu = ui::option_menu(menu);
                    for option in ui::BOARD_STATUSES.iter() {
                        let (weak, value, glyph) = (weak.clone(), option.clone(), option.clone());
                        menu = menu.item(ui::option_item(move |cx| ui::status_icon(&glyph, cx), ui::status_label(option).to_string(), None,
                            current.as_ref() == Some(option),
                            move |cx| { weak.update(cx, |this, cx| { this.status = value.clone(); this.touch(cx); }).ok(); }));
                    }
                    menu
                }
            })
            .into_any_element();
        let priority = self.priority.clone();
        let priority = prop_button("priority-menu", cx)
            .child(prop_value(ui::priority_bars(&priority, cx), ui::priority_label(&priority)))
            .dropdown_menu({
                let weak = weak.clone();
                move |mut menu, _, cx| {
                    let current = weak.upgrade().map(|this| this.read(cx).priority.clone());
                    menu = ui::option_menu(menu);
                    // Highest first, as in Linear's priority menu.
                    for option in Priority::ALL.iter().rev() {
                        let (weak, value, glyph) = (weak.clone(), option.clone(), option.clone());
                        menu = menu.item(ui::option_item(move |cx| ui::priority_bars(&glyph, cx), ui::priority_label(option), None,
                            current.as_ref() == Some(option),
                            move |cx| { weak.update(cx, |this, cx| { this.priority = value.clone(); this.touch(cx); }).ok(); }));
                    }
                    menu
                }
            })
            .into_any_element();
        let agent_name = self.agent_id.as_ref().map(|id| {
            self.agents().iter().find(|agent| &agent.id == id).map_or_else(|| "Missing agent".to_string(), |agent| agent.name.clone())
        });
        let agents: Vec<(String, String, &'static str)> = self.agents().iter()
            .map(|agent| (agent.id.clone(), agent.name.clone(), ui::harness_label(&agent.harness))).collect();
        let agent = prop_button("agent-menu", cx)
            .child(prop_value(ui::option_icon(IconName::Bot, cx), agent_name.unwrap_or_else(|| "Unassigned".into())))
            .dropdown_menu({
                let weak = weak.clone();
                move |mut menu, _, cx| {
                    let current = weak.upgrade().and_then(|this| this.read(cx).agent_id.clone());
                    menu = ui::option_menu(menu);
                    let clear = weak.clone();
                    menu = menu.item(ui::option_item(|cx| ui::option_icon(IconName::Dash, cx), "Unassigned", None, current.is_none(),
                        move |cx| { clear.update(cx, |this, cx| { this.agent_id = None; this.touch(cx); }).ok(); }));
                    menu = menu.separator().label("Agents");
                    if agents.is_empty() {
                        menu = menu.item(PopupMenuItem::new("No agents configured in this project").disabled(true));
                    }
                    for (id, name, harness) in agents.iter() {
                        let (weak, value) = (weak.clone(), id.clone());
                        menu = menu.item(ui::option_item(|cx| ui::option_icon(IconName::Bot, cx), name.clone(), Some((*harness).into()),
                            current.as_ref() == Some(id),
                            move |cx| { weak.update(cx, |this, cx| { this.agent_id = Some(value.clone()); this.touch(cx); }).ok(); }));
                    }
                    menu
                }
            })
            .into_any_element();
        let workflow_name = self.workflow_id.as_ref().map(|id| {
            self.workflows().iter().find(|workflow| &workflow.id == id).map_or_else(|| "Missing workflow".to_string(), |workflow| workflow.name.clone())
        });
        let workflows: Vec<(String, String)> = self.workflows().iter().map(|workflow| (workflow.id.clone(), workflow.name.clone())).collect();
        let workflow = prop_button("workflow-menu", cx)
            .child(prop_value(lucide_option_icon(Lucide::Workflow, cx), workflow_name.unwrap_or_else(|| "Project default".into())))
            .dropdown_menu({
                let weak = weak.clone();
                move |mut menu, _, cx| {
                    let current = weak.upgrade().and_then(|this| this.read(cx).workflow_id.clone());
                    menu = ui::option_menu(menu);
                    let clear = weak.clone();
                    menu = menu.item(ui::option_item(|cx| ui::option_icon(IconName::Dash, cx), "Project default", None, current.is_none(),
                        move |cx| { clear.update(cx, |this, cx| this.set_workflow(None, cx)).ok(); }));
                    if !workflows.is_empty() { menu = menu.separator().label("Workflows") }
                    for (id, name) in workflows.iter() {
                        let (weak, value) = (weak.clone(), id.clone());
                        menu = menu.item(ui::option_item(|cx| lucide_option_icon(Lucide::Workflow, cx), name.clone(), None,
                            current.as_ref() == Some(id),
                            move |cx| { weak.update(cx, |this, cx| this.set_workflow(Some(value.clone()), cx)).ok(); }));
                    }
                    menu
                }
            })
            .into_any_element();
        // Stages belong to the task's own workflow; without one there is nothing to pick.
        let stages: Vec<(String, String, TaskStatus)> = self.selected_workflow().map_or_else(Vec::new, |workflow| {
            workflow.stages.iter().map(|stage| (stage.id.clone(), stage.name.clone(), ui::stage_status(stage))).collect()
        });
        let stage_name = self.stage_id.as_ref().map(|id| {
            stages.iter().find(|(stage, _, _)| stage == id).map_or_else(|| "Missing stage".to_string(), |(_, name, _)| name.clone())
        });
        let has_workflow = self.workflow_id.is_some();
        // Same dot and colour as the board's stage columns (`ui::stage_dot`).
        let stage_status = self.stage_id.as_ref().and_then(|id| stages.iter().find(|(stage, _, _)| stage == id)).map(|(_, _, status)| status.clone());
        let stage = prop_button("stage-menu", cx)
            .child(prop_value(ui::stage_dot(stage_status.as_ref(), cx), stage_name.unwrap_or_else(|| "Not in a stage".into())))
            .dropdown_menu({
                let weak = weak.clone();
                move |mut menu, _, cx| {
                    let current = weak.upgrade().and_then(|this| this.read(cx).stage_id.clone());
                    menu = ui::option_menu(menu);
                    let clear = weak.clone();
                    menu = menu.item(ui::option_item(|cx| ui::stage_dot(None, cx), "Not in a stage", None, current.is_none(),
                        move |cx| { clear.update(cx, |this, cx| { this.stage_id = None; this.touch(cx); }).ok(); }));
                    if stages.is_empty() {
                        menu = menu.item(PopupMenuItem::new(if has_workflow { "This workflow has no stages" } else { "Choose a workflow first" }).disabled(true));
                    } else {
                        menu = menu.separator().label("Stages");
                    }
                    for (id, name, status) in stages.iter() {
                        let (weak, value) = (weak.clone(), id.clone());
                        menu = menu.item(ui::option_item({ let status = status.clone(); move |cx| ui::stage_dot(Some(&status), cx) }, name.clone(), None,
                            current.as_ref() == Some(id),
                            move |cx| { weak.update(cx, |this, cx| { this.stage_id = Some(value.clone()); this.touch(cx); }).ok(); }));
                    }
                    menu
                }
            })
            .into_any_element();
        // Same 16 glyph slot + 6 gap as the other values; clear only when a date is set. The
        // date and clear keep their size; the relative caption takes what's left and truncates.
        let due_label = due_display(&self.due_value);
        let has_due = !self.due_value.is_empty();
        let due_trigger = prop_button("due-menu", cx)
            .child(prop_value(ui::option_icon(IconName::Calendar, cx), due_label.unwrap_or_else(|| "Set due date".into()))
                .when(!has_due, |this| this.text_color(theme::tokens(cx).text_secondary)));
        let weak = cx.weak_entity();
        let due = div().w_full().min_w_0().flex().items_center().gap_1()
            .child(div().flex_none().child(date_field::popover("due-popover", due_trigger, &self.due, &self.due_input, self.due_value.parse().ok(), self.due_open,
                move |open, _, cx| { weak.update(cx, |this, cx| { this.due_open = *open; cx.notify() }).ok(); }, cx)))
            .when(has_due, |this| this.child(div().flex_none().child(date_field::clear_button("clear-due")
                .on_click(cx.listener(|this, _, window, cx| this.clear_due(window, cx))))))
            .children(relative_days(&self.due_value).map(|text| {
                let tip = text.clone();
                div().id("due-relative").flex_1().min_w_0().pl_1().truncate().t_label().text_color(theme::tokens(cx).text_secondary)
                    .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                    .child(text)
            }))
            .into_any_element();
        let labels = div().flex().flex_wrap().items_center().gap_1()
            .children(self.labels.iter().enumerate().map(|(index, label)| {
                ui::chip(label.clone(), cx).gap_1().child(
                    div().id(("remove-label", index)).cursor_pointer().child(Icon::new(IconName::Close).size(px(12.)))
                        .on_click(cx.listener(move |this, _, _, cx| { this.labels.remove(index); this.touch(cx); })),
                )
            }))
            // Dashed "add" chip (ref 769AF488 optional style): Enter adds the typed label.
            .child(div().h(px(24.)).pl_1p5().pr_1().rounded(px(metrics::R_CONTROL)).border_1().border_dashed().border_color(theme::tokens(cx).border)
                .flex().items_center().gap_1().text_color(theme::tokens(cx).text_secondary)
                .child(Icon::new(IconName::Plus).size(px(12.)))
                .child(div().w(px(88.)).child(components::input_h(Input::new(&self.label_input).appearance(false), 22.).px_0().t_label())))
            .into_any_element();
        Properties { status, priority, agent, workflow, stage, due, labels }
    }

    /// Drawer: one stacked property grid (ref 630BC713).
    fn render_property_grid(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.property_values(cx);
        div().flex().flex_col()
            .child(Self::property_row("Status", 100., p.status, cx))
            .child(Self::property_row("Priority", 100., p.priority, cx))
            .child(Self::property_row("Agent", 100., p.agent, cx))
            .child(Self::property_row("Workflow", 100., p.workflow, cx))
            .child(Self::property_row("Stage", 100., p.stage, cx))
            .child(Self::property_row("Due", 100., p.due, cx))
            .child(Self::property_row("Labels", 100., div().pl(px(VALUE_INSET)).child(p.labels).into_any_element(), cx))
    }

    /// Full tab: grouped property sidebar (Linear issue view: properties beside the content).
    fn render_property_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = self.property_values(cx);
        div().flex().flex_col().gap_5()
            .child(property_group("Properties", cx)
                .child(Self::property_row("Status", SIDE_LABEL_W, p.status, cx))
                .child(Self::property_row("Priority", SIDE_LABEL_W, p.priority, cx))
                .child(Self::property_row("Agent", SIDE_LABEL_W, p.agent, cx))
                .child(Self::property_row("Due", SIDE_LABEL_W, p.due, cx))
                // Workflow and stage join the property rows; a "Workflow" caption over a
                // "Workflow" row only repeated itself.
                .child(Self::property_row("Workflow", SIDE_LABEL_W, p.workflow, cx))
                .child(Self::property_row("Stage", SIDE_LABEL_W, p.stage, cx)))
            .child(property_group("Labels", cx).child(p.labels))
    }

    fn render_interventions(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let open: Vec<Intervention> = self.interventions.ready().map_or_else(Vec::new, |list| {
            list.iter().filter(|item| item.status == InterventionStatus::Recorded).cloned().collect()
        });
        let kind = self.compose_kind.clone();
        let kinds = [
            InterventionKind::Instruction, InterventionKind::Question, InterventionKind::Approve,
            InterventionKind::Reject, InterventionKind::Pause, InterventionKind::Resume,
            InterventionKind::Retry, InterventionKind::Cancel,
        ];
        let t = theme::tokens(cx);
        div().flex().flex_col().gap_2()
            .child(div().flex().items_center().justify_between()
                .child(section_heading("Intervene", cx))
                .when(!open.is_empty(), |this| this.child(ui::amber_chip(format!("{} queued", open.len()), cx))))
            .children(match &self.interventions {
                Load::Failed(error) => Some(div().t_label().text_color(cx.theme().danger).child(format!("Couldn't load interventions: {error}"))),
                _ => None,
            })
            .children(open.iter().map(|item| {
                let (dismiss, answer) = (item.clone(), item.clone());
                intervention_card::card(&item.kind, item.message.clone(), cx)
                    .child(div().pt_1().flex().justify_end().gap_1()
                        .child(components::button_sm(Button::new(SharedString::from(format!("dismiss-{}", item.id))).ghost().label("Dismiss"))
                            .on_click(cx.listener(move |this, _, _, cx| this.resolve_intervention(&dismiss, InterventionStatus::Dismissed, cx))))
                        .child(components::button_sm(Button::new(SharedString::from(format!("resolve-{}", item.id))).outline().label("Answer"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.responding = Some(answer.id.clone());
                                this.response.update(cx, |input, cx| input.focus(window, cx));
                                cx.notify();
                            }))))
                    .when(self.responding.as_deref() == Some(item.id.as_str()), |this| this.child(components::input_md(Input::new(&self.response))))
            }))
            .child(
                // Composer (ref AE10217A comment box): bordered R8 field, actions inside.
                div().rounded(px(metrics::R_INNER)).border_1().border_color(t.border).bg(t.surface).p_3().flex().flex_col().gap_2()
                    .child(div().t_label().child(Textarea::new(&self.compose).appearance(false)))
                    .child(div().flex().items_center().justify_between()
                        .child(components::button_sm(Button::new("compose-kind").ghost().compact().icon(intervention_icon(&kind)).label(intervention_label(&kind)).dropdown_caret(true)).dropdown_menu({
                            move |mut menu, _, _| {
                                menu = ui::option_menu(menu);
                                for option in kinds.iter() {
                                    let (weak, value, icon) = (weak.clone(), option.clone(), intervention_icon(option));
                                    menu = menu.item(ui::option_item(move |cx| lucide_option_icon(icon, cx), intervention_label(option), None, *option == kind,
                                        move |cx| { weak.update(cx, |this, cx| { this.compose_kind = value.clone(); cx.notify(); }).ok(); }));
                                }
                                menu
                            }
                        }))
                        .child(components::button_sm(Button::new("send-intervention").primary().label("Record")).loading(self.sending)
                            .on_click(cx.listener(|this, _, _, cx| this.send_intervention(cx))))),
            )
            .child(div().t_caption().text_color(t.text_secondary)
                .child("Interventions are stored for the agent. Delivery starts once execution is available."))
            .children(self.compose_error.clone().map(|error| div().t_label().text_color(cx.theme().danger).child(error)))
    }

    fn render_subtasks(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let done = self.subtasks.iter().filter(|subtask| subtask.completed).count();
        let total = self.subtasks.len();
        let progress = if total == 0 { 0. } else { done as f32 / total as f32 };
        let t = theme::tokens(cx);
        div().flex().flex_col()
            .child(div().flex().items_center().gap_2().pb_2()
                .child(section_heading("Subtasks", cx))
                .child(div().t_label().font_features(components::typography::tabular()).text_color(t.text_secondary).child(format!("{done} of {total}")))
                .child(div().ml_auto().w(px(88.)).h(px(4.)).rounded_full().bg(t.border)
                    .child(div().h_full().w(relative(progress)).rounded_full().bg(t.success))))
            .children(self.subtasks.iter().enumerate().map(|(index, subtask)| {
                div().flex().items_center().gap_2().h(px(32.)).px_1().border_b_1().border_color(t.border).t_label()
                    .group(SharedString::from(format!("subtask-{index}")))
                    .child(Checkbox::new(("subtask", index)).checked(subtask.completed)
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            if let Some(subtask) = this.subtasks.get_mut(index) { subtask.completed = *checked; }
                            this.touch(cx);
                        })))
                    .child(div().flex_1().min_w_0().truncate().text_color(t.text)
                        .when(subtask.completed, |this| this.line_through().text_color(t.text_secondary))
                        .child(subtask.title.clone()))
                    .child(div().invisible().group_hover(SharedString::from(format!("subtask-{index}")), |style| style.visible())
                        .child(components::icon_button(Button::new(("remove-subtask", index)).ghost().icon(IconName::Close), metrics::CONTROL_XS)
                        .on_click(cx.listener(move |this, _, _, cx| { this.subtasks.remove(index); this.touch(cx); }))))
            }))
            .child(div().flex().items_center().gap_2().h(px(32.)).px_1().t_label()
                .child(div().w(px(16.)).flex_none().flex().justify_center().child(Icon::new(IconName::Plus).size(px(14.)).text_color(t.text_secondary)))
                .child(div().flex_1().min_w_0().child(components::input_h(Input::new(&self.subtask_input).appearance(false), 24.).px_0().t_label())))
    }

    fn render_activity(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.activity {
            Load::Loading => ui::skeleton_rows(2, cx).into_any_element(),
            Load::Failed(error) => div().t_label().text_color(cx.theme().danger).child(error.clone()).into_any_element(),
            Load::Ready(items) if items.is_empty() => div().t_label().text_color(theme::tokens(cx).text_secondary).child("No activity yet.").into_any_element(),
            Load::Ready(items) => {
                let t = theme::tokens(cx);
                // Activity timeline (refs 630BC713, AE10217A): oldest first so the composer
                // below continues the thread; dot, message, right-aligned date.
                div().flex().flex_col().children(items.iter().rev().map(move |item| {
                    div().flex().gap_3().py_1p5().t_label()
                        .child(div().h(px(18.)).flex().items_center().child(div().size(px(6.)).rounded_full().bg(t.text_tertiary)))
                        .child(div().flex_1().min_w_0().text_color(t.text).child(item.message.clone()))
                        .child(div().flex_none().t_caption().h(px(18.)).flex().items_center().text_color(t.text_secondary).child(ui::short_date(&item.created_at)))
                })).into_any_element()
            }
        }
    }

    fn render_read_error(&self, cx: &mut Context<Self>) -> Option<Div> {
        self.read_error.clone().map(|error| {
            div().flex().items_center().justify_between().gap_2().t_label().text_color(cx.theme().danger)
                .child(error)
                .child(components::button_sm(Button::new("retry-read").ghost().label("Retry")).on_click(cx.listener(|this, _, _, cx| this.reload(cx))))
        })
    }

    /// Style the Input itself: Kit inputs set their own text size, so a wrapper's role would
    /// lose to it.
    fn render_title(&self, cx: &App) -> Div {
        div().text_color(theme::tokens(cx).text).child(Input::new(&self.title).appearance(false).h(px(30.)).px_0().t_headline())
    }

    /// Borderless, as in Linear and ref 630BC713; the placeholder invites the first edit.
    /// `plain_textarea` drops the editor inset so prose starts on the title's x.
    fn render_description(&self, heading: bool, cx: &mut Context<Self>) -> Div {
        let field = components::plain_textarea(&self.description, cx);
        div().flex().flex_col().gap_2()
            .when(heading, |this| this.child(section_heading("Description", cx)))
            .child(div().t_body().text_color(theme::tokens(cx).text).child(field))
    }

    /// Activity / Runs, with the comment composer closing the activity thread (ref AE10217A).
    fn render_feed(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pane = self.pane;
        let weak = cx.weak_entity();
        div().flex().flex_col().gap_3()
            .child(div().flex().child(ui::segmented("task-pane", &[(Pane::Activity, "Activity"), (Pane::Runs, "Runs")], pane,
                move |pane, _, cx| { weak.update(cx, |this, cx| { this.pane = pane; cx.notify(); }).ok(); }, cx)))
            .child(match pane {
                Pane::Activity => div().flex().flex_col().gap_3()
                    .child(self.render_activity(cx))
                    .child(components::input_md(Input::new(&self.comment))).into_any_element(),
                Pane::Runs => self.runs.clone().into_any_element(),
            })
    }

    /// Quick-look drawer (ref 630BC713): header (id row + title) over a full-width divider,
    /// then the stacked property grid and sections, 24 apart, padding 20.
    fn render_drawer_body(&mut self, task: &Task, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        div().size_full().flex().flex_col()
            .child(div().flex_none().border_b_1().border_color(t.border).p_5().flex().flex_col().gap_2()
                .child(self.render_header(task, cx))
                .children(self.render_read_error(cx))
                .child(self.render_title(cx)))
            .child(div().id("task-detail-scroll").flex_1().min_h_0().overflow_y_scroll()
                .child(div().px_5().py_5().flex().flex_col().gap_6()
                    .child(self.render_property_grid(cx))
                    .child(self.render_description(true, cx))
                    .child(self.render_subtasks(cx))
                    .child(self.render_interventions(cx))
                    .child(self.render_feed(cx))))
    }

    /// Full tab (Linear issue view): a header strip over a reading column (title,
    /// description, subtasks, intervene, activity) and a narrow property sidebar; each pane
    /// scrolls on its own and both fill the tab's height.
    fn render_full_body(&mut self, task: &Task, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let rail = self.body_width.get().is_none_or(|width| width >= px(RAIL_MIN_W));
        // Records the tab's allocated width after layout, so the rail follows the available
        // space (window, sidebar, splits). Entity updates and `notify` during prepaint can be
        // lost to the frame in progress, so the width goes through a cell and a change asks
        // for the next frame, which re-renders this view.
        let width_cell = self.body_width.clone();
        let probe = canvas(move |bounds, window, _| {
            let width = Some(bounds.size.width);
            if width_cell.get() != width {
                width_cell.set(width);
                window.request_animation_frame();
            }
        }, |_, _, _, _| {}).absolute().size_full();
        div().relative().size_full().flex().flex_col()
            .child(probe)
            // Text on the title's 24 edge; the trailing 28 controls sit BAR_INSET from top, bottom and side.
            .child(components::header_bar(cx).pl(px(metrics::GUTTER_X))
                .child(div().flex_1().child(self.render_header(task, cx))))
            .child(div().flex_1().min_h_0().flex()
                .child(div().id("task-main-scroll").flex_1().min_w_0().h_full().overflow_y_scroll()
                    .child(div().w_full().max_w(px(metrics::READ_W + 2. * metrics::GUTTER_X)).mx_auto().px(px(metrics::GUTTER_X)).pt_6().pb_8()
                        .flex().flex_col().gap_6()
                        .child(div().flex().flex_col().gap_3()
                            .children(self.render_read_error(cx))
                            .child(self.render_title(cx))
                            .child(self.render_description(false, cx)))
                        .when(!rail, |this| this.child(self.render_property_grid(cx)))
                        .child(self.render_subtasks(cx))
                        .child(self.render_interventions(cx))
                        .child(div().pt_6().border_t_1().border_color(t.border).child(self.render_feed(cx)))))
                .when(rail, |this| this.child(div().id("task-side-scroll").w(px(SIDEBAR_W)).flex_none().h_full().overflow_y_scroll().border_l_1().border_color(t.border)
                    .child(div().px_4().py_4().child(self.render_property_sidebar(cx))))))
    }

    fn workflow_stages(&self) -> Vec<runs::StageRow> {
        let Some(config) = &self.config else { return Vec::new() };
        let workflow = self.workflow_id.as_ref()
            .and_then(|id| config.workflows.iter().find(|workflow| &workflow.id == id))
            .or_else(|| config.workflows.first());
        workflow.map_or_else(Vec::new, |workflow| {
            workflow.stages.iter().map(|stage| runs::StageRow {
                name: stage.name.clone(),
                agents: stage.agent_ids.iter()
                    .filter_map(|id| config.agents.iter().find(|agent| &agent.id == id))
                    .map(|agent| (agent.name.clone(), ui::harness_label(&agent.harness).to_string()))
                    .collect(),
            }).collect()
        })
    }
}

impl Render for TaskDetail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stages = self.workflow_stages();
        self.runs.update(cx, |runs, _| runs.set_stages(stages));
        let content = match &self.task {
            Load::Loading => div().p_5().child(ui::skeleton_rows(4, cx)).into_any_element(),
            Load::Failed(error) => {
                let weak = cx.weak_entity();
                ui::error_panel("Couldn't load task", error, move |_, _, cx| { weak.update(cx, |this, cx| this.reload(cx)).ok(); }, cx).into_any_element()
            }
            Load::Ready(task) if self.full => {
                let task = task.clone();
                self.render_full_body(&task, cx).into_any_element()
            }
            Load::Ready(task) => {
                let task = task.clone();
                self.render_drawer_body(&task, cx).into_any_element()
            }
        };
        let show_bar = self.dirty || self.conflict;
        let conflict = self.conflict;
        div()
            .size_full()
            .flex()
            .flex_col()
            // In the drawer the host paints the rounded surface (an inner corner of the
            // card); a square fill here would cover its curve.
            .when(self.full, |this| this.bg(theme::tokens(cx).surface))
            .child(div().flex_1().min_h_0().child(content))
            .when(show_bar, |this| {
                let (discard, save) = (cx.weak_entity(), cx.weak_entity());
                this.child(ui::save_bar(
                    "task-save",
                    conflict,
                    self.saving,
                    self.save_error.as_deref(),
                    move |_, _, cx| { discard.update(cx, |this, cx| this.discard(cx)).ok(); },
                    move |_, _, cx| {
                        save.update(cx, |this, cx| if this.conflict { this.keep_mine(cx) } else { this.save(cx) }).ok();
                    },
                    cx,
                ))
            })
    }
}

/// `ui::option_icon` for glyphs outside the component icon set (14, secondary).
fn lucide_option_icon(icon: Lucide, cx: &App) -> AnyElement {
    Icon::new(icon).size(px(14.)).text_color(theme::tokens(cx).text_secondary).into_any_element()
}

fn intervention_icon(kind: &InterventionKind) -> Lucide {
    intervention_card::icon(kind)
}

fn intervention_label(kind: &InterventionKind) -> &'static str {
    intervention_card::label(kind)
}

/// Full-tab property sidebar width and its label column (Linear's issue sidebar is ~260).
const SIDEBAR_W: f32 = 296.;
const SIDE_LABEL_W: f32 = 72.;
/// Narrowest tab that keeps a readable ~480 column beside the rail.
const RAIL_MIN_W: f32 = 480. + SIDEBAR_W;

/// Property value controls, built once and laid out by the drawer grid or the full sidebar.
struct Properties {
    status: AnyElement,
    priority: AnyElement,
    agent: AnyElement,
    workflow: AnyElement,
    stage: AnyElement,
    due: AnyElement,
    labels: AnyElement,
}

/// Property value: 16 glyph slot, then the label, so every value's text shares one x.
fn prop_value(glyph: AnyElement, label: impl Into<SharedString>) -> Div {
    div().flex().items_center().gap_1p5()
        .child(div().w(px(16.)).flex_none().flex().items_center().justify_center().child(glyph))
        .child(label.into())
}

/// Sidebar group: small caption over its rows.
fn property_group(title: &'static str, cx: &App) -> Div {
    div().flex().flex_col().gap_1()
        .child(div().h(px(24.)).flex().items_center().t_caption().font_weight(FontWeight::MEDIUM).text_color(theme::tokens(cx).text_secondary).child(title))
}


/// Left inset of compact Small Kit buttons (`px_1p5`); borderless value inputs match it.
const VALUE_INSET: f32 = 6.;

/// Drawer value control: borderless, compact, 24 high, 13pt (ref 630BC713 values). Kit
/// paints a button's open (selected) state with its variant's active colour, so a custom
/// variant gives hover and open the light `t.hover` fill instead of Kit's heavy grey.
fn prop_button(id: &'static str, cx: &App) -> Button {
    let t = theme::tokens(cx);
    let variant = ButtonCustomVariant::new(cx)
        .color(transparent_black())
        .foreground(t.text)
        .hover(t.hover)
        .active(t.hover)
        .shadow(false);
    components::button_sm(Button::new(id).compact().custom(variant))
}

/// Section heading inside the drawer ("Description", "Subtasks", "Intervene").
fn section_heading(text: &'static str, cx: &App) -> Div {
    div().t_label().font_weight(FontWeight::SEMIBOLD).text_color(theme::tokens(cx).text).child(text)
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect::<String>().to_uppercase()
}


/// "today", "tomorrow", "in 15 days", "3 days ago" for a `YYYY-MM-DD` due date, in civil
/// days from the local date (the same "today" Kit's calendar marks).
fn relative_days(date: &str) -> Option<String> {
    let due: jiff::civil::Date = date.parse().ok()?;
    let today = jiff::ZonedDateTime::now().date();
    let days = today.until(due).ok()?.get_days();
    Some(match days {
        0 => "today".into(),
        1 => "tomorrow".into(),
        -1 => "yesterday".into(),
        days if days > 1 => format!("in {days} days"),
        days => format!("{} days ago", -days),
    })
}

/// Compact date as in ref 630BC713: "Nov 18" this year, "Nov 18, 2027" otherwise.
fn due_display(date: &str) -> Option<String> {
    let date: jiff::civil::Date = date.parse().ok()?;
    let this_year = date.year() == jiff::ZonedDateTime::now().date().year();
    Some(date.strftime(if this_year { "%b %-d" } else { "%b %-d, %Y" }).to_string())
}

fn is_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-'
        && value.split('-').all(|part| part.chars().all(|c| c.is_ascii_digit()))
}

/// Client-generated id for nested records the service stores verbatim (subtasks, config items).
pub fn new_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_nanos());
    format!("{prefix}_{nanos:x}")
}

impl Draft for TaskDetail {
    fn is_dirty(&self, _: &App) -> bool {
        self.dirty
    }

    fn save(&mut self, _: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Task<anyhow::Result<()>> {
        self.start_save(cx)
    }
}
