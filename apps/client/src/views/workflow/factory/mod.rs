//! Factory builder for one project: a native graph of the selected workflow's stages with a
//! library (stage palette, agents), and an inspector for the workflow, a stage or an edge.
//! The canvas is view-only; every edit goes through the project's shared `ConfigDraft`, so
//! Save / Discard, conflicts and the close prompt stay the project tab's.

mod clipboard;
mod edit;
mod fields;
mod fork;
mod graph;
mod hooks;
mod inspector;
mod library;
mod returns;

use std::collections::HashMap;

use gpui_kit::component::{WindowExt as _, input::{InputEvent, InputState, TextareaState}};
use gpui_kit::*;
use protocol::{
    ExecutionTargetInfo, FactoryValidation, LayoutPosition, ProjectConfig, Run, RunStepStatus, StageDefinition, StageOutcome,
    ValidateFactory, WorkflowDefinition,
};

use crate::{
    api::query_value,
    components::factory_canvas::{CanvasEdge, CanvasEvent, CanvasGraph, CanvasNode, EditCapabilities, FactoryCanvas, LibraryItem, NodeBadge, NodeKind, NodeStatus, Outcome, Selection, xyflow::{Xy, xy}},
    state::AppState,
    views::tasks::{new_id, ui::{self, Load}},
};
use super::config::ConfigDraft;
use fields::Key;

#[derive(Debug, Clone, PartialEq)]
pub enum Selected {
    Workflow,
    Stage(String),
    Edge(String),
}

/// Result of the last explicit validation, kept until the config changes.
pub enum Validation {
    Idle,
    Running,
    Done(FactoryValidation),
    Failed(String),
}

pub struct FactoryPane {
    state: AppState,
    project_id: String,
    config: Entity<ConfigDraft>,
    canvas: Entity<FactoryCanvas>,
    workflow: usize,
    selected: Selected,
    inputs: HashMap<Key, Entity<InputState>>,
    areas: HashMap<Key, Entity<TextareaState>>,
    syncing: bool,
    validation: Validation,
    /// Why the last connect / reconnect / outcome change was refused; shown over the canvas
    /// until the next graph edit or dismissal.
    pub(super) rejection: Option<SharedString>,
    /// Allocated width of the pane body; below the docking widths the panels float.
    pub(super) width: crate::views::responsive::WidthCell,
    pub(super) library_open: bool,
    /// "Can return to" picker for the selected stage.
    returns: Option<returns::ReturnPicker>,
    /// Inspector group open/closed overrides by group name.
    groups: HashMap<&'static str, bool>,
    /// Fit the view once the next config change reaches the canvas (after Arrange).
    pub(super) fit_after_change: bool,
    /// Per-workflow pan/zoom for this session (presentation only, never saved).
    viewports: HashMap<String, crate::components::factory_canvas::xyflow::Viewport>,
    pub(super) inspector_open: bool,
    targets: Load<Vec<ExecutionTargetInfo>>,
    /// Latest run of the shown workflow, for the canvas overlay and run strip.
    run: Option<Run>,
    seen_version: u64,
    /// The one runtime refresh in flight; a newer one replaces (and cancels) it, so service
    /// change bursts never pile up concurrent `/v1/runs` loads.
    _runtime: Option<gpui_kit::Task<()>>,
    _tasks: Vec<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl FactoryPane {
    pub fn new(state: AppState, project_id: String, config: Entity<ConfigDraft>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let canvas = cx.new(|cx| {
            let mut canvas = FactoryCanvas::new(cx);
            canvas.set_connection_check(std::rc::Rc::new(|source, outcome, target| {
                if source == target && outcome != Outcome::Return { Err("Only a return route can loop to the same stage.".into()) } else { Ok(()) }
            }));
            canvas
        });
        let mut subscriptions = vec![
            cx.observe_in(&config, window, |this, _, window, cx| this.config_changed(window, cx)),
            cx.subscribe_in(&canvas, window, |this, _, event: &CanvasEvent, window, cx| this.on_canvas(event, window, cx)),
            // The clipboard can change outside the canvas (another app, a text field), so
            // Paste availability is re-read when the window comes back and when the canvas
            // takes focus — before any key or menu can use it. No polling.
            cx.observe_window_activation(window, |this, window, cx| if window.is_window_active() { this.sync_capabilities(cx) }),
            cx.on_focus_in(&canvas.focus_handle(cx), window, |this, _, cx| this.sync_capabilities(cx)),
            cx.observe(&state.service, |this, service, cx| {
                let version = service.read(cx).data_version();
                if version != this.seen_version { this.seen_version = version; this.load_runtime(cx) }
            }),
        ];
        let mut inputs = HashMap::new();
        for key in Key::SINGLE {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(key.placeholder()));
            subscriptions.push(cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) { let text = input.read(cx).value().to_string(); this.apply_field(key, text, cx) }
            }));
            inputs.insert(key, input);
        }
        let mut areas = HashMap::new();
        for key in Key::MULTI {
            let area = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 10).placeholder(key.placeholder()));
            subscriptions.push(cx.subscribe_in(&area, window, move |this, area, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) { let text = area.read(cx).value().to_string(); this.apply_field(key, text, cx) }
            }));
            areas.insert(key, area);
        }
        let mut this = Self {
            seen_version: state.service.read(cx).data_version(),
            state, project_id, config, canvas, workflow: 0, selected: Selected::Workflow, inputs, areas, syncing: false,
            validation: Validation::Idle, _runtime: None, rejection: None, width: Default::default(), library_open: false, returns: None, groups: HashMap::new(), fit_after_change: false, viewports: HashMap::new(), inspector_open: false, targets: Load::Loading, run: None, _tasks: Vec::new(), _subscriptions: subscriptions,
        };
        this.config_changed(window, cx);
        this.load_runtime(cx);
        this
    }

    fn config_value(&self, cx: &App) -> Option<ProjectConfig> {
        self.config.read(cx).config().cloned()
    }

    fn current<'a>(&self, config: &'a ProjectConfig) -> Option<&'a WorkflowDefinition> {
        config.workflows.get(self.workflow)
    }

    /// Tells the canvas which of Undo / Redo / Paste would do something, for its menus and keys.
    pub(super) fn sync_capabilities(&mut self, cx: &mut Context<Self>) {
        let draft = self.config.read(cx);
        let can_paste = cx.read_from_clipboard().and_then(|item| item.text()).is_some_and(|text| clipboard::Clip::from_text(&text).is_some());
        let capabilities = EditCapabilities { can_undo: draft.can_undo(), can_redo: draft.can_redo(), can_paste };
        self.canvas.update(cx, |canvas, cx| canvas.set_edit_capabilities(capabilities, cx));
    }

    fn config_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_capabilities(cx);
        if !matches!(self.validation, Validation::Running) { self.validation = Validation::Idle }
        self.normalize_selection(cx);
        self.sync_fields(window, cx);
        self.push_graph(cx);
        if std::mem::take(&mut self.fit_after_change) { self.canvas.update(cx, |canvas, cx| canvas.fit_view(cx)) }
        cx.notify();
    }

    /// Drops a selection whose workflow, stage or edge no longer exists.
    fn normalize_selection(&mut self, cx: &App) {
        let Some(config) = self.config_value(cx) else { return };
        if self.workflow >= config.workflows.len() { self.workflow = 0 }
        let Some(workflow) = self.current(&config) else { self.selected = Selected::Workflow; return };
        let exists = match &self.selected {
            Selected::Workflow => true,
            Selected::Stage(id) => workflow.stages.iter().any(|stage| &stage.id == id),
            Selected::Edge(id) => graph::normalized(workflow).edges.iter().any(|edge| &edge.id == id),
        };
        if !exists { self.selected = Selected::Workflow }
    }

    fn selected_stage_id(&self) -> Option<&str> {
        match &self.selected { Selected::Stage(id) => Some(id), _ => None }
    }

    /// Mirrors the config into the inspector fields without marking the draft dirty.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.config_value(cx) else { return };
        let workflow = self.current(&config);
        let stage = workflow.and_then(|workflow| workflow.stages.iter().find(|stage| Some(stage.id.as_str()) == self.selected_stage_id()));
        self.syncing = true;
        for (key, input) in &self.inputs {
            let value = fields::value(*key, &config, workflow, stage);
            if input.read(cx).value().as_ref() != value { input.update(cx, |input, cx| input.set_value(value, window, cx)) }
        }
        for (key, area) in &self.areas {
            let value = fields::value(*key, &config, workflow, stage);
            // Keep the user's in-progress text (e.g. a trailing blank line) when it parses the same.
            if area.read(cx).value().trim() != value.trim() { area.update(cx, |area, cx| area.set_value(value, window, cx)) }
        }
        self.syncing = false;
    }

    fn apply_field(&mut self, key: Key, text: String, cx: &mut Context<Self>) {
        if self.syncing { return }
        // Input change events can arrive after a programmatic sync; text that already matches
        // the config is an echo, not an edit.
        let Some(config) = self.config_value(cx) else { return };
        let workflow = self.current(&config);
        let stage = workflow.and_then(|workflow| workflow.stages.iter().find(|stage| Some(stage.id.as_str()) == self.selected_stage_id()));
        if fields::value(key, &config, workflow, stage).trim() == text.trim() { return }
        let (workflow, stage) = (self.workflow, self.selected_stage_id().map(str::to_string));
        // Typing in one field is one undo step until another edit intervenes.
        let group = format!("field:{key:?}:{workflow}:{}", stage.as_deref().unwrap_or(""));
        self.config.update(cx, |config, cx| config.edit_grouped(group, cx, |config| fields::apply(key, &text, config, workflow, stage.as_deref())));
    }

    pub(super) fn reject(&mut self, reason: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.rejection = Some(reason.into());
        cx.notify();
    }

    pub(super) fn edit_workflow(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut WorkflowDefinition)) {
        self.rejection = None;
        let index = self.workflow;
        self.config.update(cx, |config, cx| config.edit(cx, |config| if let Some(workflow) = config.workflows.get_mut(index) { f(workflow) }));
    }

    pub(super) fn edit_stage(&mut self, stage_id: &str, cx: &mut Context<Self>, f: impl FnOnce(&mut StageDefinition)) {
        let id = stage_id.to_string();
        self.edit_workflow(cx, move |workflow| if let Some(stage) = workflow.stages.iter_mut().find(|stage| stage.id == id) { f(stage) });
    }

    pub(super) fn select(&mut self, selected: Selected, window: &mut Window, cx: &mut Context<Self>) {
        // A floating inspector opens for a stage or route, like a detail popover.
        if selected != Selected::Workflow { self.inspector_open = true }
        self.selected = selected;
        let selection = match &self.selected {
            Selected::Workflow => Selection::default(),
            Selected::Stage(id) => Selection { nodes: vec![id.clone().into()], edges: Vec::new() },
            Selected::Edge(id) => Selection { nodes: Vec::new(), edges: vec![id.clone().into()] },
        };
        self.canvas.update(cx, |canvas, cx| canvas.select(selection, cx));
        self.sync_fields(window, cx);
        cx.notify();
    }

    pub(super) fn select_workflow(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.workflow = index;
        self.validation = Validation::Idle;
        self.select(Selected::Workflow, window, cx);
        self.push_graph(cx);
        let viewport = self.workflow_id(cx).and_then(|id| self.viewports.get(&id).copied());
        self.canvas.update(cx, |canvas, cx| match viewport { Some(viewport) => canvas.set_viewport(viewport, cx), None => canvas.fit_view(cx) });
        self.load_runtime(cx);
    }

    fn workflow_id(&self, cx: &App) -> Option<String> {
        self.config_value(cx).and_then(|config| self.current(&config).map(|workflow| workflow.id.clone()))
    }

    fn remember_viewport(&mut self, viewport: crate::components::factory_canvas::xyflow::Viewport, cx: &mut Context<Self>) {
        if let Some(id) = self.workflow_id(cx) { self.viewports.insert(id, viewport); }
    }

    // ---- Graph edits -----------------------------------------------------------------

    pub(super) fn add_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut index = 0;
        self.config.update(cx, |config, cx| config.edit(cx, |config| {
            config.workflows.push(WorkflowDefinition {
                id: new_id("wf"), name: "New workflow".into(), description: String::new(), instructions: String::new(),
                stages: Vec::new(), instruction_ids: None, graph: None, layout: None, extra: Default::default(),
            });
            index = config.workflows.len() - 1;
        }));
        self.select_workflow(index, window, cx);
    }

    pub(super) fn delete_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.workflow;
        self.config.update(cx, |config, cx| config.edit(cx, |config| if index < config.workflows.len() { config.workflows.remove(index); }));
        self.select_workflow(0, window, cx);
    }

    /// Adds a stage at `at` (or right of the last stage) and selects it.
    pub(super) fn add_stage(&mut self, at: Option<LayoutPosition>, window: &mut Window, cx: &mut Context<Self>) {
        let id = new_id("stage");
        let added = id.clone();
        // Keyboard/toolbar adds land in the middle of the view, like a dropped palette item.
        let center = layout(self.canvas.read(cx).view_center());
        self.edit_workflow(cx, move |workflow| {
            let at = at.unwrap_or(center);
            let stage = StageDefinition {
                id, name: format!("Stage {}", workflow.stages.len() + 1), instructions: String::new(), agent_ids: Vec::new(),
                return_to_stage_ids: Vec::new(), exit_rule: None, status: None, color: None, instruction_ids: None,
                execution: None, max_visits: None, extra: Default::default(),
            };
            graph::add_stage(workflow, stage, at);
        });
        self.select(Selected::Stage(added), window, cx);
    }

    fn new_stage(name: String) -> StageDefinition {
        StageDefinition {
            id: new_id("stage"), name, instructions: String::new(), agent_ids: Vec::new(), return_to_stage_ids: Vec::new(),
            exit_rule: None, status: None, color: None, instruction_ids: None, execution: None, max_visits: None, extra: Default::default(),
        }
    }

    /// A fork stage with its join below; connect branches from the fork's success handle.
    pub(super) fn add_parallel(&mut self, at: Option<LayoutPosition>, window: &mut Window, cx: &mut Context<Self>) {
        let at = at.unwrap_or_else(|| layout(self.canvas.read(cx).view_center()));
        let (fork, join) = (Self::new_stage("Parallel".into()), Self::new_stage("Join".into()));
        let selected = fork.id.clone();
        self.edit_workflow(cx, move |workflow| { fork::add_parallel(workflow, fork, join, at); });
        self.select(Selected::Stage(selected), window, cx);
    }

    /// A join only makes sense for a fork: it becomes the selected fork's join.
    pub(super) fn add_join(&mut self, at: LayoutPosition, window: &mut Window, cx: &mut Context<Self>) {
        let fork_stage = self.selected_stage_id().map(str::to_string)
            .filter(|id| self.config_value(cx).and_then(|config| self.current(&config).and_then(|w| fork::fork_at(w, id))).is_some());
        let Some(fork_stage) = fork_stage else {
            self.reject("Select a parallel (fork) stage first, or drag Parallel to add a fork with its join.", cx);
            return;
        };
        let join = Self::new_stage("Join".into());
        let (join_id, target) = (join.id.clone(), join.id.clone());
        self.edit_workflow(cx, move |workflow| { graph::add_stage(workflow, join, at); fork::set_join(workflow, &fork_stage, &target) });
        self.select(Selected::Stage(join_id), window, cx);
    }

    pub(super) fn connect(&mut self, source: &str, target: &str, on: StageOutcome, window: &mut Window, cx: &mut Context<Self>) {
        // Checked on a copy so a refused link never touches the draft.
        let Some(mut candidate) = self.config_value(cx).and_then(|config| self.current(&config).cloned()) else { return };
        match graph::connect(&mut candidate, source, target, on) {
            Ok(id) => {
                self.edit_workflow(cx, move |workflow| *workflow = candidate);
                self.select(Selected::Edge(id), window, cx);
            }
            Err(reason) => self.reject(reason, cx),
        }
    }

    pub(super) fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected.clone() {
            Selected::Stage(id) => self.edit_workflow(cx, |workflow| graph::remove_stages(workflow, &[id])),
            Selected::Edge(id) => self.edit_workflow(cx, |workflow| graph::remove_edges(workflow, &[id])),
            Selected::Workflow => return,
        }
        self.select(Selected::Workflow, window, cx);
    }

    fn assign_agent(&mut self, stage_id: &str, agent_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let agent = agent_id.to_string();
        self.edit_stage(stage_id, cx, |stage| if !stage.agent_ids.contains(&agent) { stage.agent_ids.push(agent) });
        self.select(Selected::Stage(stage_id.into()), window, cx);
    }

    fn on_canvas(&mut self, event: &CanvasEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            CanvasEvent::SelectionChanged(selection) => {
                self.sync_capabilities(cx);
                // The inspector edits one item; a multi-selection shows the first.
                let selected = match (selection.nodes.first(), selection.edges.first()) {
                    (Some(id), _) => Selected::Stage(id.to_string()),
                    (None, Some(id)) => Selected::Edge(id.to_string()),
                    _ => Selected::Workflow,
                };
                if selected != self.selected {
                    if selected != Selected::Workflow { self.inspector_open = true }
                    self.selected = selected;
                    self.sync_fields(window, cx);
                    cx.notify()
                }
            }
            CanvasEvent::NodesMoved(moves) => {
                let moves: Vec<(String, LayoutPosition)> = moves.iter().map(|(id, at)| (id.to_string(), layout(*at))).collect();
                self.edit_workflow(cx, move |workflow| for (id, at) in moves { graph::set_position(workflow, &id, at) });
            }
            CanvasEvent::Connect { source, outcome: kind, target } => self.connect(source, target, outcome(*kind), window, cx),
            CanvasEvent::Reconnect { edge, source, outcome: kind, target } => {
                // Try on a copy so a refused link leaves the original edge in place.
                let workflow = self.config_value(cx).and_then(|config| self.current(&config).cloned());
                let Some(mut candidate) = workflow else { return };
                graph::remove_edges(&mut candidate, &[edge.to_string()]);
                match graph::connect(&mut candidate, source, target, outcome(*kind)) {
                    Ok(id) => {
                        self.edit_workflow(cx, move |workflow| *workflow = candidate);
                        self.select(Selected::Edge(id), window, cx);
                    }
                    Err(reason) => self.reject(reason, cx),
                }
            }
            CanvasEvent::Delete(selection) => {
                let nodes: Vec<String> = selection.nodes.iter().map(|id| id.to_string()).collect();
                let edges: Vec<String> = selection.edges.iter().map(|id| id.to_string()).collect();
                self.edit_workflow(cx, |workflow| { graph::remove_edges(workflow, &edges); graph::remove_stages(workflow, &nodes) });
                self.select(Selected::Workflow, window, cx);
            }
            CanvasEvent::Drop { item, position, node } => match (item, node) {
                (LibraryItem::Agent(agent), Some(node)) => self.assign_agent(node, agent, window, cx),
                (LibraryItem::Agent(_), None) => crate::components::toast_warning("Drop an agent on a stage to assign it.", window, cx),
                (LibraryItem::Stage, _) => self.add_stage(Some(layout(*position)), window, cx),
                (LibraryItem::Fork, _) => self.add_parallel(Some(layout(*position)), window, cx),
                (LibraryItem::Join, _) => self.add_join(layout(*position), window, cx),
            },
            CanvasEvent::Activate(id) => self.select(Selected::Stage(id.to_string()), window, cx),
            CanvasEvent::ConnectRejected(reason) => self.reject(reason.clone(), cx),
            CanvasEvent::Copy(selection) => self.copy(selection, cx),
            CanvasEvent::Cut(selection) => self.cut(selection, window, cx),
            CanvasEvent::Paste { position } => self.paste(layout(*position), window, cx),
            CanvasEvent::Duplicate(selection) => self.duplicate(selection, window, cx),
            CanvasEvent::Undo => self.undo(window, cx),
            CanvasEvent::Redo => self.redo(window, cx),
            CanvasEvent::ViewportChanged(viewport) => self.remember_viewport(*viewport, cx),
        }
    }

    /// Rebuilds the canvas graph from the config, validation and the latest run.
    fn push_graph(&mut self, cx: &mut Context<Self>) {
        let config = self.config_value(cx);
        let Some(workflow) = config.as_ref().and_then(|config| self.current(config)) else {
            self.canvas.update(cx, |canvas, cx| canvas.set_graph(CanvasGraph::default(), cx));
            return;
        };
        let config = config.as_ref().expect("workflow implies config");
        let graph = graph::normalized(workflow);
        let issues = self.issue_counts(workflow);
        let nodes = workflow.stages.iter().enumerate().map(|(index, stage)| {
            let at = graph::position(workflow, index);
            let entry = if graph.entry_stage_id == stage.id { "Entry · " } else { "" };
            let kind = if fork::fork_at(workflow, &stage.id).is_some() { NodeKind::Fork }
                else if fork::joined_by(workflow, &stage.id).is_some() { NodeKind::Join } else { NodeKind::Stage };
            CanvasNode {
                id: stage.id.clone().into(),
                label: if stage.name.is_empty() { "Untitled stage".into() } else { stage.name.clone().into() },
                detail: Some(format!("{entry}{}", stage_summary(config, stage)).into()),
                position: xy(at.x as f32, at.y as f32),
                status: self.run.as_ref().and_then(|run| node_status(run, &stage.id)),
                issue: issues.get(&stage.id).cloned().map(Into::into),
                kind,
                badges: self.run.as_ref().map_or_else(Vec::new, |run| node_badges(run, &stage.id)),
                ..Default::default()
            }
        }).collect();
        // Every running stage (parallel branches run several at once).
        let active: Vec<String> = self.run.as_ref().filter(|run| run.is_active())
            .map_or_else(Vec::new, |run| if run.active_stage_ids.is_empty() { run.current_stage_id.iter().cloned().collect() } else { run.active_stage_ids.clone() });
        let edges = graph.edges.iter().chain(fork::branch_edges(workflow).iter()).map(|edge| CanvasEdge {
            id: edge.id.clone().into(), source: edge.source_stage_id.clone().into(), target: edge.target_stage_id.clone().into(),
            outcome: edge_kind(&edge.on), active: active.contains(&edge.target_stage_id), issue: None,
        }).collect();
        self.canvas.update(cx, |canvas, cx| canvas.set_graph(CanvasGraph { nodes, edges }, cx));
    }

    /// First validation message per stage id, matched from issue paths (`…stages[3]…`) and messages.
    fn issue_counts(&self, workflow: &WorkflowDefinition) -> HashMap<String, String> {
        let Validation::Done(result) = &self.validation else { return HashMap::new() };
        let prefix = format!("workflows[{}]", self.workflow);
        let mut counts = HashMap::new();
        for issue in result.errors.iter().filter(|issue| issue.path.starts_with(&prefix)) {
            let stage = workflow.stages.iter().enumerate().find(|(index, stage)| {
                issue.path.contains(&format!("stages[{index}]")) || issue.message.contains(&stage.id)
            });
            if let Some((_, stage)) = stage { counts.entry(stage.id.clone()).or_insert_with(|| issue.message.clone()); }
        }
        counts
    }

    // ---- Service ---------------------------------------------------------------------

    /// Execution targets and the shown workflow's latest run; refreshed on every data-version
    /// bump, which includes the service's SSE invalidations.
    fn load_runtime(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let workflow_id = self.config_value(cx).and_then(|config| self.current(&config).map(|workflow| workflow.id.clone()));
        let runs_path = format!("/v1/runs?projectId={}", query_value(&self.project_id));
        // Targets are fetched until one load succeeds, not on every run update.
        let fetch_targets = self.targets.ready().is_none();
        self._tasks.retain(|task| !task.is_ready());
        self._runtime = Some(cx.spawn(async move |this, cx| {
            let targets = if fetch_targets { Some(api.get::<Vec<ExecutionTargetInfo>>("/v1/execution-targets").await) } else { None };
            let runs = api.get::<Vec<Run>>(&runs_path).await;
            this.update(cx, |this, cx| {
                if let Some(targets) = targets { this.targets = Load::from_result(targets) }
                // Runs may be unavailable on an older service; the overlay just stays off.
                this.run = runs.ok().and_then(|runs| latest_run(runs, workflow_id.as_deref()));
                this.push_graph(cx);
                cx.notify();
            }).ok();
        }));
    }

    pub(super) fn validate(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config_value(cx) else { return };
        self.validation = Validation::Running;
        cx.notify();
        let api = self.state.api.clone();
        let path = format!("/v1/projects/{}/factory/validate", query_value(&self.project_id));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = api.post::<_, FactoryValidation>(&path, &ValidateFactory { config: &config }).await;
            this.update(cx, |this, cx| {
                this.validation = match result {
                    Ok(result) => Validation::Done(result),
                    Err(error) => Validation::Failed(ui::service_message(&error)),
                };
                this.push_graph(cx);
                cx.notify();

            }).ok();
        }));
    }

    pub(super) fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.read(cx).dirty {
            crate::components::toast_warning("Save your changes first; export uses the saved configuration.", window, cx);
            return;
        }
        let api = self.state.api.clone();
        let path = format!("/v1/projects/{}/factory/export", query_value(&self.project_id));
        let directory = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
        let target = cx.prompt_for_new_path(&directory, Some("factory.json"));
        let window_handle = window.window_handle();
        self._tasks.push(cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(target))) = target.await else { return };
            let outcome = match api.get::<serde_json::Value>(&path).await {
                Ok(bundle) => serde_json::to_vec_pretty(&bundle).map_err(anyhow::Error::from)
                    .and_then(|bytes| std::fs::write(&target, bytes).map_err(anyhow::Error::from))
                    .map_err(|error| error.to_string()),
                Err(error) => Err(ui::service_message(&error)),
            };
            window_handle.update(cx, |_, window, cx| match outcome {
                Ok(()) => window.push_notification(gpui_kit::component::notification::Notification::success(format!("Exported to {}", target.display())), cx),
                Err(error) => crate::components::toast_warning(format!("Couldn't export: {error}"), window, cx),
            }).ok();
        }));
    }

    pub(super) fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.config.read(cx);
        let Some(revision) = draft.status().ready().map(|config| config.revision).filter(|_| !draft.dirty) else {
            crate::components::toast_warning("Save or discard your changes before importing.", window, cx);
            return;
        };
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Import".into()) });
        let (api, state) = (self.state.api.clone(), self.state.clone());
        let path = format!("/v1/projects/{}/factory/import", query_value(&self.project_id));
        let window_handle = window.window_handle();
        self._tasks.push(cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(files))) = paths.await else { return };
            let Some(file) = files.into_iter().next() else { return };
            let outcome = match std::fs::read(&file).map_err(anyhow::Error::from).and_then(|bytes| Ok(serde_json::from_slice::<serde_json::Value>(&bytes)?)) {
                Ok(bundle) => api.post::<_, ProjectConfig>(&path, &protocol::ImportFactory { revision, bundle }).await.map(|_| ()).map_err(|error| ui::service_message(&error)),
                Err(error) => Err(format!("{} isn't a factory bundle: {error}", file.display())),
            };
            window_handle.update(cx, |_, window, cx| match outcome {
                Ok(()) => { state.data_changed(cx); window.push_notification(gpui_kit::component::notification::Notification::success("Factory imported"), cx) }
                Err(error) => crate::components::toast_warning(format!("Couldn't import: {error}"), window, cx),
            }).ok();
        }));
    }
}

fn outcome(kind: Outcome) -> StageOutcome {
    match kind { Outcome::Success => StageOutcome::Success, Outcome::Failure => StageOutcome::Failure, Outcome::Return => StageOutcome::Return }
}

fn edge_kind(on: &StageOutcome) -> Outcome {
    match on { StageOutcome::Failure => Outcome::Failure, StageOutcome::Return => Outcome::Return, _ => Outcome::Success }
}

fn layout(at: Xy) -> LayoutPosition {
    LayoutPosition { x: f64::from(at.x), y: f64::from(at.y) }
}

pub(super) fn agent_name(config: &ProjectConfig, id: &str) -> String {
    config.agents.iter().find(|agent| agent.id == id).map_or_else(|| "Missing agent".into(), |agent| agent.name.clone())
}

/// One line under the stage title: agents in run order plus hooks and gates when present.
fn stage_summary(config: &ProjectConfig, stage: &StageDefinition) -> String {
    let count = |items: Option<&Vec<protocol::CommandDefinition>>| items.map_or(0, Vec::len);
    let execution = stage.execution.as_ref();
    let commands = execution.map_or(0, |e| count(e.before.as_ref()) + count(e.commands.as_ref()) + count(e.after.as_ref()));
    let gates = execution.and_then(|e| e.gates.as_ref()).map_or(0, Vec::len);
    let mut parts = vec![match stage.agent_ids.as_slice() {
        [] => "No agents".to_string(),
        [only] => agent_name(config, only),
        [first, rest @ ..] => format!("{} +{}", agent_name(config, first), rest.len()),
    }];
    if commands > 0 { parts.push(format!("{commands} command{}", if commands == 1 { "" } else { "s" })) }
    if gates > 0 { parts.push(format!("{gates} gate{}", if gates == 1 { "" } else { "s" })) }
    parts.join(" · ")
}

/// Active run of the workflow first, else its most recent run.
fn latest_run(mut runs: Vec<Run>, workflow_id: Option<&str>) -> Option<Run> {
    runs.retain(|run| Some(run.workflow_id.as_str()) == workflow_id);
    runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let active = runs.iter().position(Run::is_active);
    active.map(|index| runs.swap_remove(index)).or_else(|| runs.into_iter().next())
}

fn latest_visit<'a>(run: &'a Run, stage_id: &str) -> Option<&'a protocol::RunStage> {
    run.stages.as_ref()?.iter().rev().find(|stage| stage.stage_id.as_deref().unwrap_or(&stage.id) == stage_id)
}

/// Real attempt data only: "Attempt n" after a retry, "Retrying" while one is scheduled.
fn node_badges(run: &Run, stage_id: &str) -> Vec<NodeBadge> {
    let Some(visit) = latest_visit(run, stage_id) else { return Vec::new() };
    let mut badges = Vec::new();
    if let Some(attempt) = visit.steps.iter().filter_map(|step| step.attempts.iter().map(|attempt| attempt.number).max()).max().filter(|number| *number > 1) {
        badges.push(NodeBadge { label: format!("Attempt {attempt}").into(), tone: crate::components::Tone::Neutral });
    }
    if run.is_active() && visit.steps.iter().any(|step| step.retry_at.is_some()) {
        badges.push(NodeBadge { label: "Retrying".into(), tone: crate::components::Tone::Attention });
    }
    badges
}

/// State of the stage's latest visit in the run, if the run reached it.
fn node_status(run: &Run, stage_id: &str) -> Option<NodeStatus> {
    if run.is_active() && run.active_stage_ids.iter().any(|id| id == stage_id) && run.run_state() != protocol::RunState::Paused {
        let waiting = latest_visit(run, stage_id).is_some_and(|visit| visit.status == RunStepStatus::NeedsApproval);
        return Some(if waiting { NodeStatus::Waiting } else { NodeStatus::Running });
    }
    let visit = latest_visit(run, stage_id)?;
    match visit.status {
        RunStepStatus::Running if run.run_state() == protocol::RunState::Paused => Some(NodeStatus::Paused),
        RunStepStatus::Running => Some(NodeStatus::Running),
        RunStepStatus::NeedsApproval => Some(NodeStatus::Waiting),
        RunStepStatus::Completed => Some(NodeStatus::Completed),
        RunStepStatus::Failed => Some(NodeStatus::Failed),
        RunStepStatus::Cancelled => Some(NodeStatus::Cancelled),
        _ => None,
    }
}
