//! Toolbar, inspector and layout of the factory pane.

use gpui_kit::component::{
    Disableable as _, IconName, Selectable as _,
    button::{Button, ButtonVariants as _},
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{ProjectConfig, SchedulerConfig, StageOutcome, WorkflowDefinition};

use super::{FactoryPane, Selected, Validation, agent_name, fields::Key, fork, graph, library::LIBRARY_W};
use crate::views::responsive::{narrower_than, width_probe};
use crate::components::{self, Typography as _};
use crate::theme::{self, metrics};
use crate::views::{tasks::ui::{self, Load}, workflow::{agents::environment_dropdown, editor::toggle_row}};

const INSPECTOR_W: f32 = 320.;
/// Narrowest useful canvas; panels stop docking when they would leave less.
const CANVAS_MIN_W: f32 = 480.;
const DOCK_BOTH_W: f32 = LIBRARY_W + INSPECTOR_W + CANVAS_MIN_W;
const DOCK_INSPECTOR_W: f32 = INSPECTOR_W + CANVAS_MIN_W;
const OUTCOMES: [(StageOutcome, &str); 2] = [(StageOutcome::Success, "On success"), (StageOutcome::Failure, "On failure")];

fn outcome_label(on: &StageOutcome) -> &'static str {
    match on { StageOutcome::Failure => "Failure", StageOutcome::Return => "Return", _ => "Success" }
}

fn section(title: &'static str, cx: &App) -> Div {
    div().pt_4().pb_1().t_caption().font_weight(FontWeight::MEDIUM).text_color(theme::tokens(cx).text_secondary).child(title)
}

fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div().t_caption().text_color(theme::tokens(cx).text_secondary).child(text.into())
}

impl FactoryPane {
    fn input(&self, key: Key) -> Input {
        components::input_md(Input::new(&self.inputs[&key]))
    }

    fn area(&self, key: Key, cx: &App) -> Div {
        components::textarea_box(&self.areas[&key], cx)
    }

    // ---- Toolbar ---------------------------------------------------------------------

    fn render_toolbar(&self, config: &ProjectConfig, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let name = self.current(config).map_or_else(|| "No workflow".to_string(), |workflow| if workflow.name.is_empty() { "Untitled".into() } else { workflow.name.clone() });
        let names: Vec<String> = config.workflows.iter().map(|workflow| workflow.name.clone()).collect();
        let has_workflow = self.current(config).is_some();
        let validation = match &self.validation {
            Validation::Idle => None,
            Validation::Running => Some(ui::chip("Validating…", cx)),
            Validation::Done(result) if result.valid => Some(components::status_pill("Valid", components::Tone::Success, cx)),
            Validation::Done(result) => Some(components::status_pill(format!("{} issue{}", result.errors.len(), if result.errors.len() == 1 { "" } else { "s" }), components::Tone::Danger, cx)),
            Validation::Failed(_) => Some(ui::amber_chip("Couldn't validate", cx)),
        };
        let (float_library, float_inspector) = (narrower_than(&self.width, DOCK_BOTH_W), narrower_than(&self.width, DOCK_INSPECTOR_W));
        components::header_bar(cx)
            .when(float_library, |this| this.child(components::icon_button(Button::new("factory-library-toggle").ghost()
                .icon(gpui_kit::assets::IconName::PanelLeft).tooltip("Nodes and agents").selected(self.library_open), metrics::CONTROL_MD)
                .on_click(cx.listener(|this, _, _, cx| { this.library_open = !this.library_open; cx.notify() }))))
            .child(components::button_sm(Button::new("factory-workflow").ghost().icon(gpui_kit::assets::IconName::Workflow).label(name).dropdown_caret(true)).dropdown_menu({
                let weak = weak.clone();
                move |mut menu, _, _| {
                    for (index, name) in names.iter().enumerate() {
                        let weak = weak.clone();
                        menu = menu.item(PopupMenuItem::new(if name.is_empty() { "Untitled".to_string() } else { name.clone() })
                            .on_click(move |_, window, cx| { weak.update(cx, |this, cx| this.select_workflow(index, window, cx)).ok(); }));
                    }
                    let add = weak.clone();
                    menu.separator().item(PopupMenuItem::new("New workflow").on_click(move |_, window, cx| { add.update(cx, |this, cx| this.add_workflow(window, cx)).ok(); }))
                }
            }))
            .child(div().flex_1())
            .child(components::icon_button(Button::new("factory-undo").ghost().icon(IconName::Undo2).tooltip("Undo (⌘Z)"), metrics::CONTROL_MD)
                .disabled(!self.config.read(cx).can_undo())
                .on_click(cx.listener(|this, _, window, cx| this.undo(window, cx))))
            .child(components::icon_button(Button::new("factory-redo").ghost().icon(IconName::Redo2).tooltip("Redo (⇧⌘Z)"), metrics::CONTROL_MD)
                .disabled(!self.config.read(cx).can_redo())
                .on_click(cx.listener(|this, _, window, cx| this.redo(window, cx))))
            .children(validation)
            .when(has_workflow, |this| this
                .child(components::button_sm(Button::new("factory-add-stage").ghost().icon(IconName::Plus).label("Stage"))
                    .on_click(cx.listener(|this, _, window, cx| this.add_stage(None, window, cx))))
                .child(components::button_sm(Button::new("factory-validate").outline().label("Validate"))
                    .disabled(matches!(self.validation, Validation::Running))
                    .on_click(cx.listener(|this, _, _, cx| this.validate(cx)))))
            .when(float_inspector, |this| this.child(components::icon_button(Button::new("factory-inspector-toggle").ghost()
                .icon(gpui_kit::assets::IconName::PanelRight).tooltip("Inspector").selected(self.inspector_open), metrics::CONTROL_MD)
                .on_click(cx.listener(|this, _, _, cx| { this.inspector_open = !this.inspector_open; cx.notify() }))))
            .child(components::icon_button(Button::new("factory-more").ghost().icon(IconName::Ellipsis).tooltip("More"), metrics::CONTROL_MD).dropdown_menu({
                move |menu, _, _| {
                    let (export, import, delete, arrange) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
                    menu.when(has_workflow, |menu| menu.item(PopupMenuItem::new("Arrange left to right").on_click(move |_, _, cx| {
                            // One undo step; positions you placed by hand come back with Undo.
                            arrange.update(cx, |this, cx| {
                                let placed = this.config_value(cx).and_then(|config| this.current(&config).and_then(|w| w.layout.as_ref().map(|l| !l.positions.is_empty()))).unwrap_or(false);
                                if placed {
                                    this.fit_after_change = true;
                                    this.edit_workflow(cx, graph::arrange);
                                } else {
                                    this.canvas.update(cx, |canvas, cx| canvas.fit_view(cx));
                                }
                            }).ok();
                        })).separator())
                        .item(PopupMenuItem::new("Export factory…").on_click(move |_, window, cx| { export.update(cx, |this, cx| this.export(window, cx)).ok(); }))
                        .item(PopupMenuItem::new("Import factory…").on_click(move |_, window, cx| { import.update(cx, |this, cx| this.import(window, cx)).ok(); }))
                        .when(has_workflow, |menu| menu.separator().item(PopupMenuItem::new("Delete workflow").on_click(move |_, window, cx| { delete.update(cx, |this, cx| this.delete_workflow(window, cx)).ok(); })))
                }
            }))
    }

    // ---- Inspector -------------------------------------------------------------------

    fn render_inspector(&self, config: &ProjectConfig, workflow: &WorkflowDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let body = match self.selected.clone() {
            Selected::Workflow => self.render_workflow_form(config, workflow, cx).into_any_element(),
            Selected::Stage(id) => self.render_stage_form(config, workflow, &id, cx).into_any_element(),
            Selected::Edge(id) => self.render_edge_form(workflow, &id, cx).into_any_element(),
        };
        div().id("factory-inspector").w(px(INSPECTOR_W)).flex_none().h_full().overflow_y_scroll().border_l_1().border_color(t.border).bg(t.canvas)
            .child(div().p_4().flex().flex_col().gap_2().child(body).children(self.render_issues(cx)))
    }

    fn render_issues(&self, cx: &App) -> Option<Div> {
        let issues = match &self.validation {
            Validation::Done(result) if !result.errors.is_empty() => result.errors.iter().map(|issue| (issue.message.clone(), Some(issue.path.clone()))).collect::<Vec<_>>(),
            Validation::Failed(error) => vec![(error.clone(), None)],
            _ => return None,
        };
        let t = theme::tokens(cx);
        Some(div().flex().flex_col().gap_1().child(section("Validation", cx))
            .children(issues.into_iter().map(move |(message, path)| div().flex().flex_col().p_2().rounded(px(metrics::R_CONTROL)).bg(t.tray)
                .child(div().t_label().text_color(t.text).child(message))
                .children(path.map(|path| div().t_caption().text_color(t.text_secondary).child(path))))))
    }

    fn render_workflow_form(&self, config: &ProjectConfig, workflow: &WorkflowDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let graph = graph::normalized(workflow);
        let entry = workflow.stages.iter().find(|stage| stage.id == graph.entry_stage_id).map_or("None".to_string(), |stage| stage.name.clone());
        let stages: Vec<(String, String)> = workflow.stages.iter().map(|stage| (stage.id.clone(), stage.name.clone())).collect();
        let weak = cx.weak_entity();
        let shared = workflow.instruction_ids.clone().unwrap_or_default();
        let scheduler_summary = if config.scheduler.as_ref().is_some_and(|scheduler| scheduler.enabled) { "On" } else { "Off" };
        div().flex().flex_col().gap_3()
            .child(div().t_headline().child("Workflow"))
            .child(self.group("wf-basics", "Basics", None, cx))
            .when(self.is_open("wf-basics"), |this| this
            .child(ui::field("Name", self.input(Key::WorkflowName), cx))
            .child(ui::field("Instructions", self.area(Key::WorkflowInstructions, cx), cx))
            .child(ui::field("Entry stage", div().flex().child(components::button_sm(Button::new("factory-entry").outline().label(entry).dropdown_caret(true))
                .disabled(stages.is_empty())
                .dropdown_menu(move |mut menu, _, _| {
                    for (id, name) in stages.iter() {
                        let (weak, id) = (weak.clone(), id.clone());
                        menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| { weak.update(cx, |this, cx| this.edit_workflow(cx, |w| graph::set_entry(w, &id))).ok(); }));
                    }
                    menu
                })), cx))
            .child(ui::field("Max transitions", self.input(Key::MaxTransitions), cx))
            .child(self.instruction_toggles("Shared instructions", config, &shared, cx, |this, id, cx| {
                this.edit_workflow(cx, |workflow| toggle(workflow.instruction_ids.get_or_insert_with(Vec::new), id));
            })))
            .child(self.group("wf-environment", "Environment", None, cx))
            .when(self.is_open("wf-environment"), |this| this
                .child(environment_dropdown(&self.config, &self.targets, cx))
                .child(hint("Options and readiness: project Settings → Environment.", cx)))
            .child(self.group("wf-scheduler", "Scheduler", Some(scheduler_summary.into()), cx))
            .when(self.is_open("wf-scheduler"), |this| this.child(self.render_scheduler(config, workflow, cx)))
    }

    fn render_scheduler(&self, config: &ProjectConfig, workflow: &WorkflowDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let scheduler = config.scheduler.clone().unwrap_or_default();
        let this_workflow = scheduler.workflow_id.as_deref() == Some(workflow.id.as_str());
        let workflow_id = workflow.id.clone();
        let statuses = scheduler.statuses.clone().unwrap_or_default();
        let config_entity = self.config.downgrade();
        let edit = move |cx: &mut App, f: Box<dyn FnOnce(&mut SchedulerConfig)>| {
            config_entity.update(cx, |config, cx| config.edit(cx, |config| f(config.scheduler.get_or_insert_with(SchedulerConfig::default)))).ok();
        };
        let (edit_enabled, edit_target) = (edit.clone(), edit.clone());
        div().flex().flex_col().gap_2()
            .child(section("Scheduler", cx))
            .child(div().flex().items_center().gap_2()
                .child(div().flex_1().t_label().child("Start runs automatically"))
                .child(Switch::new("factory-scheduler").checked(scheduler.enabled)
                    .on_click(move |checked: &bool, _, cx| { let checked = *checked; edit_enabled(cx, Box::new(move |s| s.enabled = checked)) })))
            .when(scheduler.enabled, |this| this
                .child(toggle_row("factory-scheduler-workflow".into(), this_workflow, "Use this workflow".into(), None, cx)
                    .on_click(move |_, _, cx| {
                        let id = workflow_id.clone();
                        edit_target(cx, Box::new(move |s| s.workflow_id = if this_workflow { None } else { Some(id) }))
                    }))
                .child(hint("Pick up tasks in these statuses:", cx))
                .children(ui::BOARD_STATUSES.iter().map(|status| {
                    let (on, status, edit) = (statuses.contains(status), status.clone(), edit.clone());
                    toggle_row(SharedString::from(format!("factory-scheduler-{}", status.as_str())), on, ui::status_label(&status).to_string(), None, cx)
                        .on_click(move |_, _, cx| {
                            let status = status.clone();
                            edit(cx, Box::new(move |s| toggle(s.statuses.get_or_insert_with(Vec::new), &status)))
                        })
                }))
                .child(ui::field("Max concurrent runs", self.input(Key::MaxConcurrentRuns), cx))
                .child(ui::field("Poll every (seconds)", self.input(Key::PollSeconds), cx)))
    }

    fn instruction_toggles(&self, title: &'static str, config: &ProjectConfig, selected: &[String], cx: &mut Context<Self>,
        on_toggle: fn(&mut Self, &String, &mut Context<Self>)) -> impl IntoElement + use<> {
        div().flex().flex_col().gap_0p5()
            .child(section(title, cx))
            .when(config.instructions.is_empty(), |this| this.child(hint("No instruction files yet. Add them in Settings → Instructions.", cx)))
            .children(config.instructions.iter().map(|instruction| {
                let id = instruction.id.clone();
                toggle_row(SharedString::from(format!("factory-instr-{title}-{}", instruction.id)), selected.contains(&instruction.id), instruction.name.clone(), None, cx)
                    .on_click(cx.listener(move |this, _, _, cx| on_toggle(this, &id, cx)))
            }))
    }

    fn render_stage_form(&self, config: &ProjectConfig, workflow: &WorkflowDefinition, stage_id: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(stage) = workflow.stages.iter().find(|stage| stage.id == stage_id).cloned() else { return div().into_any_element() };
        let graph = graph::normalized(workflow);
        let entry = graph.entry_stage_id == stage.id;
        let instructions = stage.instruction_ids.clone().unwrap_or_default();
        let execution = stage.execution.clone().unwrap_or_default();
        let hooks = [&execution.before, &execution.commands, &execution.after].iter().map(|list| list.as_ref().map_or(0, Vec::len)).sum::<usize>();
        let gates = execution.gates.as_ref().map_or(0, Vec::len);
        let execution_summary = super::super::join_parts(&[
            if stage.instructions.is_empty() && instructions.is_empty() { "" } else { "instructions" },
            &if hooks > 0 { format!("{hooks} command{}", if hooks == 1 { "" } else { "s" }) } else { String::new() },
            &if gates > 0 { format!("{gates} gate{}", if gates == 1 { "" } else { "s" }) } else { String::new() },
            if execution.cwd.is_some() || execution.execution_target.is_some() { "custom environment" } else { "" },
        ]);
        let limits_summary = super::super::join_parts(&[
            &stage.max_visits.map_or(String::new(), |n| format!("{n} visits")),
            &execution.timeout_ms.map_or(String::new(), |ms| format!("{}s timeout", ms / 1000)),
            &execution.retries.filter(|n| *n > 0).map_or(String::new(), |n| format!("{n} retries")),
        ]);
        div().flex().flex_col().gap_3()
            .child(div().flex().items_center().gap_2()
                .child(div().flex_1().t_headline().child("Stage"))
                .when(entry, |this| this.child(ui::chip("Entry", cx)))
                .child(components::icon_button(Button::new("factory-stage-delete").ghost().icon(gpui_kit::assets::IconName::Trash).tooltip("Delete stage"), metrics::CONTROL_SM)
                    .on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx)))))
            .child(self.group("basics", "Basics", None, cx))
            .when(self.is_open("basics"), |this| this
                .child(ui::field("Name", self.input(Key::StageName), cx))
                .child(ui::field("Exit rule", self.input(Key::ExitRule), cx))
                .child(self.render_agents(config, &stage, cx)))
            .child(self.group("routes", "Routes", None, cx))
            .when(self.is_open("routes"), |this| this
                .child(self.render_routes(workflow, &stage.id, cx))
                .child(self.render_parallel(workflow, &stage.id, cx)))
            .child(self.group("execution", "Execution", (!execution_summary.is_empty()).then(|| execution_summary.into()), cx))
            .when(self.is_open("execution"), |this| this
                .child(ui::field("Instructions", self.area(Key::StageInstructions, cx), cx))
                .child(self.instruction_toggles("Instruction files", config, &instructions, cx, |this, id, cx| {
                    let Some(stage) = this.selected_stage_id().map(str::to_string) else { return };
                    this.edit_stage(&stage, cx, |stage| toggle(stage.instruction_ids.get_or_insert_with(Vec::new), id));
                }))
                .child(ui::field("Before", self.area(Key::Before, cx), cx))
                .child(ui::field("Commands", self.area(Key::Commands, cx), cx))
                .child(ui::field("Gates", self.area(Key::Gates, cx), cx))
                .child(hint("One per line: approval: <question> or command: <executable args>.", cx))
                .child(ui::field("After", self.area(Key::After, cx), cx))
                .child(ui::field("Working directory", self.input(Key::Cwd), cx))
                .child(self.render_stage_target(&stage, cx)))
            .child(self.group("limits", "Limits", (!limits_summary.is_empty()).then(|| limits_summary.into()), cx))
            .when(self.is_open("limits"), |this| this
                .child(div().flex().gap_2()
                    .child(div().flex_1().min_w_0().child(ui::field("Timeout (s)", self.input(Key::TimeoutSeconds), cx)))
                    .child(div().flex_1().min_w_0().child(ui::field("Max visits", self.input(Key::MaxVisits), cx))))
                .map(|this| if self.state.service.read(cx).capabilities().step_retries {
                    this.child(div().flex().gap_2()
                        .child(div().flex_1().min_w_0().child(ui::field("Retries per step", self.input(Key::Retries), cx)))
                        .child(div().flex_1().min_w_0().child(ui::field("Retry delay (s)", self.input(Key::RetryDelaySeconds), cx))))
                    .child(hint("Failed steps retry automatically; denials and cancellations don't.", cx))
                } else {
                    this.child(ui::update_service_note("retry failed steps automatically", cx))
                }))
            .into_any_element()
    }

    /// Assigned agents in run order with move/remove, plus an add menu.
    fn render_agents(&self, config: &ProjectConfig, stage: &protocol::StageDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let count = stage.agent_ids.len();
        let unassigned: Vec<(String, String)> = config.agents.iter().filter(|agent| !stage.agent_ids.contains(&agent.id)).map(|agent| (agent.id.clone(), agent.name.clone())).collect();
        let stage_id = stage.id.clone();
        let weak = cx.weak_entity();
        div().flex().flex_col().gap_0p5()
            .child(div().flex().items_center()
                .child(div().flex_1().child(ui::caption("Agents", cx)))
                .child(components::button_xs(Button::new("factory-agent-add").ghost().icon(IconName::Plus).label("Add")).disabled(unassigned.is_empty()).dropdown_menu({
                    let stage_id = stage_id.clone();
                    move |mut menu, _, _| {
                        for (id, name) in unassigned.iter() {
                            let (weak, id, stage) = (weak.clone(), id.clone(), stage_id.clone());
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, cx| { weak.update(cx, |this, cx| this.assign_agent(&stage, &id, window, cx)).ok(); }));
                        }
                        menu
                    }
                })))
            .when(count == 0, |this| this.child(hint("Drag an agent from the library, or use Add.", cx)))
            .children(stage.agent_ids.iter().enumerate().map(|(index, id)| {
                let stage_id = stage_id.clone();
                let mover = move |delta: isize| {
                    let stage_id = stage_id.clone();
                    move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| this.edit_stage(&stage_id, cx, |stage| {
                        let to = index as isize + delta;
                        if to < 0 || to as usize >= stage.agent_ids.len() { return }
                        if delta == 0 { stage.agent_ids.remove(index); } else { stage.agent_ids.swap(index, to as usize) }
                    })
                };
                div().flex().items_center().gap_1().h(px(metrics::ROW_PROP)).px_2().rounded(px(metrics::R_CONTROL)).bg(t.tray)
                    .child(div().w(px(18.)).t_caption().font_features(components::typography::tabular()).text_color(t.text_secondary).child(format!("{}", index + 1)))
                    .child(div().flex_1().min_w_0().truncate().t_label().child(agent_name(config, id)))
                    .child(components::icon_button(Button::new(("agent-up", index)).ghost().icon(IconName::ChevronUp).tooltip("Run earlier"), metrics::CONTROL_XS)
                        .disabled(index == 0).on_click(cx.listener(mover(-1))))
                    .child(components::icon_button(Button::new(("agent-down", index)).ghost().icon(IconName::ChevronDown).tooltip("Run later"), metrics::CONTROL_XS)
                        .disabled(index + 1 >= count).on_click(cx.listener(mover(1))))
                    .child(components::icon_button(Button::new(("agent-remove", index)).ghost().icon(IconName::Close).tooltip("Remove from stage"), metrics::CONTROL_XS)
                        .on_click(cx.listener(mover(0))))
            }))
    }

    /// Keyboard-reachable routing: pick each outcome's successor without dragging a handle.
    fn render_routes(&self, workflow: &WorkflowDefinition, stage_id: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let graph = graph::normalized(workflow);
        let stages: Vec<(String, String)> = workflow.stages.iter().map(|stage| (stage.id.clone(), stage.name.clone())).collect();
        let is_fork = fork::fork_at(workflow, stage_id).is_some();
        div().flex().flex_col().gap_1().child(ui::caption("Routes", cx)).children(OUTCOMES.into_iter().filter(move |(on, _)| !(is_fork && *on == StageOutcome::Success)).map(|(on, label)| {
            let edge = graph.edges.iter().find(|edge| edge.source_stage_id == stage_id && edge.on == on).cloned();
            let current = edge.as_ref().and_then(|edge| stages.iter().find(|(id, _)| id == &edge.target_stage_id)).map_or("End".to_string(), |(_, name)| name.clone());
            let (weak, stages, source) = (cx.weak_entity(), stages.clone(), stage_id.to_string());
            div().flex().items_center().gap_2()
                .child(div().w(px(80.)).flex_none().t_label().text_color(theme::tokens(cx).text_secondary).child(label))
                .child(div().flex_1().min_w_0().child(components::button_sm(Button::new(SharedString::from(format!("route-{}", on.as_str()))).outline().label(current).dropdown_caret(true))
                    .dropdown_menu(move |mut menu, _, _| {
                        if let Some(edge) = edge.clone() {
                            let weak = weak.clone();
                            menu = menu.item(PopupMenuItem::new("End").on_click(move |_, _, cx| { weak.update(cx, |this, cx| this.edit_workflow(cx, |w| graph::remove_edges(w, &[edge.id.clone()]))).ok(); }));
                        }
                        for (id, name) in stages.iter().filter(|(id, _)| id != &source) {
                            let (weak, id, source, on) = (weak.clone(), id.clone(), source.clone(), on.clone());
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, window, cx| {
                                weak.update(cx, |this, cx| { this.connect(&source, &id, on.clone(), window, cx); this.select(Selected::Stage(source.clone()), window, cx) }).ok();
                            }));
                        }
                        menu
                    })))
        })).child(self.render_returns(cx))
    }

    /// Return routes: any number of distinct targets, toggled individually.
    fn render_edge_form(&self, workflow: &WorkflowDefinition, edge_id: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        if let Some(edge) = fork::branch_edges(workflow).into_iter().find(|edge| edge.id == edge_id) {
            let name = |id: &str| workflow.stages.iter().find(|stage| stage.id == id).map_or(id.to_string(), |stage| stage.name.clone());
            return div().flex().flex_col().gap_3()
                .child(div().flex().items_center().gap_2()
                    .child(div().flex_1().t_headline().child("Parallel branch"))
                    .child(components::icon_button(Button::new("factory-branch-delete").ghost().icon(gpui_kit::assets::IconName::Trash).tooltip("Remove branch"), metrics::CONTROL_SM)
                        .on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx)))))
                .child(hint(format!("{} → {}", name(&edge.source_stage_id), name(&edge.target_stage_id)), cx))
                .child(hint("Runs alongside the fork's other branches; each branch continues on success until the join.", cx))
                .into_any_element();
        }
        let Some(edge) = graph::normalized(workflow).edges.into_iter().find(|edge| edge.id == edge_id) else { return div().into_any_element() };
        let name = |id: &str| workflow.stages.iter().find(|stage| stage.id == id).map_or(id.to_string(), |stage| stage.name.clone());
        let weak = cx.weak_entity();
        let id = edge.id.clone();
        div().flex().flex_col().gap_3()
            .child(div().flex().items_center().gap_2()
                .child(div().flex_1().t_headline().child("Route"))
                .child(components::icon_button(Button::new("factory-edge-delete").ghost().icon(gpui_kit::assets::IconName::Trash).tooltip("Delete route"), metrics::CONTROL_SM)
                    .on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx)))))
            .child(hint(format!("{} → {}", name(&edge.source_stage_id), name(&edge.target_stage_id)), cx))
            .child(ui::field("Outcome", ui::segmented("factory-edge-outcome",
                &[(StageOutcome::Success, outcome_label(&StageOutcome::Success)), (StageOutcome::Failure, outcome_label(&StageOutcome::Failure)), (StageOutcome::Return, outcome_label(&StageOutcome::Return))],
                edge.on.clone(), move |on, window, cx| {
                    let id = id.clone();
                    weak.update(cx, |this, cx| {
                        let Some(mut candidate) = this.config_value(cx).and_then(|config| this.current(&config).cloned()) else { return };
                        match graph::set_outcome(&mut candidate, &id, on) {
                            Ok(changed) => { this.edit_workflow(cx, move |w| *w = candidate); this.select(Selected::Edge(changed), window, cx) }
                            Err(reason) => this.reject(reason, cx),
                        }
                    }).ok();
                }, cx), cx))
            .child(hint("Success and failure each have one successor. A stage can return to several stages for rework; returns count toward max visits.", cx))
            .into_any_element()
    }
}

impl FactoryPane {
    /// Refused-link reason, pinned over the top of the canvas (not a toast that can be missed).
    fn render_rejection(&self, cx: &mut Context<Self>) -> Option<Div> {
        let reason = self.rejection.clone()?;
        let t = theme::tokens(cx);
        Some(div().absolute().top_3().left_0().right_0().flex().justify_center().px_3().child(
            div().id("factory-rejection").occlude().max_w(px(480.)).flex().items_center().gap_2().pl_3().pr_1().py_1()
                .rounded(px(metrics::R_INNER)).border_1().border_color(t.danger.opacity(0.4)).bg(t.surface).shadow_md()
                .child(gpui_kit::component::Icon::new(gpui_kit::assets::IconName::TriangleAlert).size(px(14.)).text_color(t.danger))
                .child(div().flex_1().min_w_0().t_label().text_color(t.text).child(reason))
                .child(components::icon_button(Button::new("factory-rejection-dismiss").ghost().icon(IconName::Close).tooltip("Dismiss"), metrics::CONTROL_XS)
                    .on_click(cx.listener(|this, _, _, cx| { this.rejection = None; cx.notify() }))),
        ))
    }
}

impl FactoryPane {
    /// Fork: join stage and branch list; join: which fork it waits for; else "Run in parallel".
    fn render_parallel(&self, workflow: &WorkflowDefinition, stage_id: &str, cx: &mut Context<Self>) -> AnyElement {
        let name = |id: &str| workflow.stages.iter().find(|stage| stage.id == id).map_or("Missing stage".to_string(), |stage| stage.name.clone());
        if let Some(fork) = fork::joined_by(workflow, stage_id) {
            return div().flex().flex_col().gap_1().child(ui::caption("Parallel", cx))
                .child(hint(format!("Waits for every branch of {} before continuing.", name(&fork.stage_id)), cx)).into_any_element();
        }
        let others: Vec<(String, String)> = workflow.stages.iter().filter(|stage| stage.id != stage_id).map(|stage| (stage.id.clone(), stage.name.clone())).collect();
        let source = stage_id.to_string();
        let Some(fork) = fork::fork_at(workflow, stage_id) else {
            if !self.state.service.read(cx).capabilities().parallel_execution {
                return div().flex().flex_col().gap_1().child(ui::caption("Parallel", cx)).child(ui::update_service_note("run stages in parallel", cx)).into_any_element();
            }
            let weak = cx.weak_entity();
            return div().flex().flex_col().gap_1().child(ui::caption("Parallel", cx))
                .child(div().flex().child(components::button_sm(Button::new("factory-make-fork").outline().icon(gpui_kit::assets::IconName::GitFork).label("Run branches in parallel…").dropdown_caret(true))
                    .disabled(others.is_empty())
                    .dropdown_menu(move |mut menu, _, _| {
                        menu = menu.label("Join at");
                        for (id, label) in others.iter() {
                            let (weak, id, source) = (weak.clone(), id.clone(), source.clone());
                            menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, _, cx| {
                                weak.update(cx, |this, cx| this.edit_workflow(cx, |w| fork::make_fork(w, &source, &id))).ok();
                            }));
                        }
                        menu
                    })))
                .child(hint("Turns this stage into a fork: its success starts every branch at once.", cx)).into_any_element();
        };
        let weak = cx.weak_entity();
        let (join_menu, branch_menu) = (others.clone(), others.iter().filter(|(id, _)| *id != fork.join_stage_id && !fork.branches.iter().any(|b| &b.entry_stage_id == id)).cloned().collect::<Vec<_>>());
        let (join_source, branch_source) = (source.clone(), source.clone());
        let (join_weak, branch_weak, remove_weak) = (weak.clone(), weak.clone(), weak);
        div().flex().flex_col().gap_1()
            .child(div().flex().items_center().child(div().flex_1().child(ui::caption("Parallel", cx)))
                .child(components::button_xs(Button::new("factory-unfork").ghost().label("Stop parallel"))
                    .on_click(cx.listener(move |this, _, _, cx| { let source = source.clone(); this.edit_workflow(cx, |w| fork::remove_fork(w, &source)) }))))
            .child(div().flex().items_center().gap_2()
                .child(div().w(px(80.)).flex_none().t_label().text_color(theme::tokens(cx).text_secondary).child("Join at"))
                .child(components::button_sm(Button::new("factory-join").outline().label(name(&fork.join_stage_id)).dropdown_caret(true))
                    .dropdown_menu(move |mut menu, _, _| {
                        for (id, label) in join_menu.iter() {
                            let (weak, id, source) = (join_weak.clone(), id.clone(), join_source.clone());
                            menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, _, cx| {
                                weak.update(cx, |this, cx| this.edit_workflow(cx, |w| fork::set_join(w, &source, &id))).ok();
                            }));
                        }
                        menu
                    })))
            .children(fork.branches.iter().map(|branch| {
                let edge = fork::branch_edge_id(&fork.id, &branch.id);
                let weak = remove_weak.clone();
                div().flex().items_center().gap_1().h(px(metrics::ROW_PROP)).px_2().rounded(px(metrics::R_CONTROL)).bg(theme::tokens(cx).tray)
                    .child(gpui_kit::component::Icon::new(gpui_kit::assets::IconName::GitFork).size(px(12.)).text_color(theme::tokens(cx).text_secondary))
                    .child(div().flex_1().min_w_0().truncate().t_label().child(name(&branch.entry_stage_id)))
                    .child(components::icon_button(Button::new(SharedString::from(format!("branch-remove-{}", branch.id))).ghost().icon(IconName::Close).tooltip("Remove branch"), metrics::CONTROL_XS)
                        .on_click(move |_, _, cx| { let edge = edge.clone(); weak.update(cx, |this, cx| this.edit_workflow(cx, |w| graph::remove_edges(w, &[edge]))).ok(); }))
            }))
            .child(div().flex().child(components::button_xs(Button::new("factory-branch-add").ghost().icon(IconName::Plus).label("Add branch")).disabled(branch_menu.is_empty())
                .dropdown_menu(move |mut menu, _, _| {
                    for (id, label) in branch_menu.iter() {
                        let (weak, id, source) = (branch_weak.clone(), id.clone(), branch_source.clone());
                        menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, window, cx| {
                            weak.update(cx, |this, cx| this.connect(&source, &id, StageOutcome::Success, window, cx)).ok();
                        }));
                    }
                    menu
                })))
            .child(hint("Each branch runs on its own and continues on success until it reaches the join.", cx))
            .into_any_element()
    }
}

impl FactoryPane {
    /// Per-stage execution target override; options for it come from the project's.
    fn render_stage_target(&self, stage: &protocol::StageDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = stage.execution.as_ref().and_then(|execution| execution.execution_target.clone());
        let label = current.as_ref().map_or("Project default".to_string(), |target| ui::target_label(target).to_string());
        let (weak, id) = (cx.weak_entity(), stage.id.clone());
        ui::field("Environment", div().flex().child(components::button_sm(Button::new("stage-target").outline().label(label).dropdown_caret(true))
            .dropdown_menu(move |mut menu, _, _| {
                let choices = std::iter::once(None).chain(protocol::ExecutionTarget::ALL.iter().cloned().map(Some));
                for choice in choices {
                    let (weak, id) = (weak.clone(), id.clone());
                    let name = choice.as_ref().map_or("Project default".to_string(), |target| ui::target_label(target).to_string());
                    menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                        let choice = choice.clone();
                        weak.update(cx, |this, cx| this.edit_stage(&id, cx, |stage| {
                            let execution = stage.execution.get_or_insert_with(Default::default);
                            execution.execution_target = choice;
                            if *execution == protocol::StageExecution::default() { stage.execution = None }
                        })).ok();
                    }));
                }
                menu
            })), cx)
    }
}

impl FactoryPane {
    /// Inspector group header; open state is per group name and survives selection changes.
    fn group(&self, key: &'static str, title: &'static str, summary: Option<SharedString>, cx: &mut Context<Self>) -> Stateful<Div> {
        components::disclosure(SharedString::from(format!("group-{key}")), title, summary, self.is_open(key), cx)
            .on_click(cx.listener(move |this, _, _, cx| {
                let open = this.is_open(key);
                this.groups.insert(key, !open);
                cx.notify();
            }))
    }

    /// Basics, Routes and Environment start open; Execution, Limits and Scheduler start folded
    /// and show a summary of what they hold.
    pub(super) fn is_open(&self, key: &'static str) -> bool {
        self.groups.get(key).copied().unwrap_or(matches!(key, "basics" | "routes" | "wf-basics" | "wf-environment"))
    }
}

fn toggle<T: PartialEq + Clone>(list: &mut Vec<T>, id: &T) {
    match list.iter().position(|existing| existing == id) {
        Some(index) => { list.remove(index); }
        None => list.push(id.clone()),
    }
}

impl Render for FactoryPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let draft = self.config.read(cx);
        let config = match draft.status() {
            Load::Loading => return ui::skeleton_rows(3, cx).into_any_element(),
            Load::Failed(error) => {
                let config = self.config.downgrade();
                return ui::error_panel("Couldn't load workflow", error, move |_, _, cx| { config.update(cx, |config, cx| config.reload(cx)).ok(); }, cx).into_any_element();
            }
            Load::Ready(_) => draft.config().cloned(),
        };
        let Some(config) = config else { return div().into_any_element() };
        if let (Selected::Stage(stage), Some(workflow)) = (self.selected.clone(), self.current(&config).cloned()) {
            self.prepare_returns(&workflow, &stage, window, cx);
        }
        let toolbar = self.render_toolbar(&config, cx).into_any_element();
        let Some(workflow) = self.current(&config).cloned() else {
            return div().size_full().flex().flex_col().child(toolbar).child(div().flex_1().p_6().child(ui::empty_panel(
                "No workflow yet",
                "A workflow routes a task through stages of agents, commands and approval gates.",
                Some(components::button_md(Button::new("first-workflow").primary().icon(IconName::Plus).label("Create workflow"))
                    .on_click(cx.listener(|this, _, window, cx| this.add_workflow(window, cx))).into_any_element()),
                cx,
            ))).into_any_element();
        };
        // Docked when the canvas keeps CANVAS_MIN_W; otherwise each panel floats over the
        // canvas from its edge (toolbar toggle, close button) and the canvas keeps full width.
        let (float_library, float_inspector) = (narrower_than(&self.width, DOCK_BOTH_W), narrower_than(&self.width, DOCK_INSPECTOR_W));
        let t = theme::tokens(cx);
        let floating = |id: &'static str, left: bool, panel: AnyElement, cx: &mut Context<Self>| {
            div().id(id).occlude().absolute().top_0().bottom_0().map(|this| if left { this.left_0() } else { this.right_0() })
                .shadow_lg().bg(t.canvas).flex().flex_col()
                // Own close row so it never sits on the panel's header controls.
                .child(div().flex_none().h(px(metrics::CONTROL_MD + 8.)).px_1().flex().items_center().justify_end().border_b_1().border_color(t.border)
                    .child(components::icon_button(Button::new(SharedString::from(format!("{id}-close"))).ghost()
                        .icon(IconName::Close).tooltip("Close"), metrics::CONTROL_SM)
                        .on_click(cx.listener(move |this, _, _, cx| { if left { this.library_open = false } else { this.inspector_open = false }; cx.notify() }))))
                .child(div().flex_1().min_h_0().flex().child(panel))
        };
        let library = self.render_library(&config, cx).into_any_element();
        let inspector = self.render_inspector(&config, &workflow, cx).into_any_element();
        let (library, floating_library) = if float_library { (None, self.library_open.then(|| floating("factory-library-float", true, library, cx))) } else { (Some(library), None) };
        let (inspector, floating_inspector) = if float_inspector { (None, self.inspector_open.then(|| floating("factory-inspector-float", false, inspector, cx))) } else { (Some(inspector), None) };
        div().size_full().flex().flex_col()
            .child(toolbar)
            .child(div().relative().flex_1().min_h_0().flex()
                .child(width_probe(&self.width))
                .children(library)
                .child(div().relative().flex_1().min_w_0().h_full().child(self.canvas.clone())
                    .children(self.render_rejection(cx)))
                .children(inspector)
                .children(floating_library)
                .children(floating_inspector))
            .into_any_element()
    }
}
