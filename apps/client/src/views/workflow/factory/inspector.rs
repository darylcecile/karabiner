//! Toolbar, inspector and layout of the factory pane.

use gpui_kit::component::{
    Disableable as _, Icon, IconName, Selectable as _,
    button::{Button, ButtonVariants as _},
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{ProjectConfig, SchedulerConfig, StageOutcome, TaskStatus, WorkflowDefinition};

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
    div().pb_1().t_caption().font_weight(FontWeight::MEDIUM).text_color(theme::tokens(cx).text_secondary).child(title)
}

fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div().t_caption().text_color(theme::tokens(cx).text_secondary).child(text.into())
}

/// Inspector group: spacing separates groups (no rules); fields inside sit 12 apart.
fn block() -> Div {
    div().flex().flex_col().gap_3().min_w_0()
}

/// A group that is always open: title and an optional one-line scope note.
fn titled(title: &'static str, note: Option<&'static str>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    block().child(div().flex().flex_col().gap_0p5()
        .child(div().t_label().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title))
        .children(note.map(|note| hint(note, cx))))
}

/// Full-width picker: label at the leading edge, caret at the trailing one.
fn picker(button: Button) -> Button {
    components::button_sm(button.outline().dropdown_caret(true)).w_full()
}

/// Numbers are short; a full-width box for "3" reads as a text field.
fn short(input: impl IntoElement) -> Div {
    div().w(px(120.)).child(input)
}

/// Statuses the scheduler picks up when none are saved (`scheduler.ts`).
const DEFAULT_STATUSES: [TaskStatus; 1] = [TaskStatus::Todo];

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
        let (float_library, float_inspector) = (narrower_than(&self.width, DOCK_BOTH_W), narrower_than(&self.width, DOCK_INSPECTOR_W));
        let status = !matches!(self.validation, Validation::Idle);
        let budget = toolbar_budget(self.width.get().map(f32::from), float_library, float_inspector, status, has_workflow);
        let (compact, name_w) = (budget.compact, budget.name_w);
        let validation = self.render_validation(budget.short_status, cx);
        let (can_undo, can_redo) = (self.config.read(cx).can_undo(), self.config.read(cx).can_redo());
        let (undo_item, redo_item) = (weak.clone(), weak.clone());
        components::header_bar(cx)
            .when(float_library, |this| this.child(components::icon_button(Button::new("factory-library-toggle").ghost()
                .icon(gpui_kit::assets::IconName::PanelLeft).tooltip("Nodes and agents").selected(self.library_open), metrics::CONTROL_MD)
                .on_click(cx.listener(|this, _, _, cx| { this.library_open = !this.library_open; cx.notify() }))))
            // Long names ellipsize within what the controls leave; the tooltip has the full name.
            .child(components::button_sm(Button::new("factory-workflow").ghost().icon(gpui_kit::assets::IconName::Workflow).label(name.clone()).dropdown_caret(true).tooltip(name))
                .when_some(name_w, |this, w| this.max_w(px(w))).dropdown_menu({
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
            // Narrow panes move Undo / Redo into More; ⌘Z and ⇧⌘Z still work.
            .when(!compact, |this| this
                .child(components::icon_button(Button::new("factory-undo").ghost().icon(IconName::Undo2).tooltip("Undo (⌘Z)"), metrics::CONTROL_MD)
                    .disabled(!can_undo)
                    .on_click(cx.listener(|this, _, window, cx| this.undo(window, cx))))
                .child(components::icon_button(Button::new("factory-redo").ghost().icon(IconName::Redo2).tooltip("Redo (⇧⌘Z)"), metrics::CONTROL_MD)
                    .disabled(!can_redo)
                    .on_click(cx.listener(|this, _, window, cx| this.redo(window, cx)))))
            .children(validation)
            .when(has_workflow, |this| this
                // The tightest panes keep both actions as icons; tooltips carry the labels.
                .child(if budget.icon_actions {
                    components::icon_button(Button::new("factory-add-stage").ghost().icon(IconName::Plus).tooltip("Add stage"), metrics::CONTROL_MD)
                } else {
                    components::button_sm(Button::new("factory-add-stage").ghost().icon(IconName::Plus).label("Stage"))
                }
                    .on_click(cx.listener(|this, _, window, cx| this.add_stage(None, window, cx))))
                .child(if budget.icon_actions {
                    components::icon_button(Button::new("factory-validate").outline().icon(IconName::Check).tooltip("Validate"), metrics::CONTROL_MD)
                } else {
                    components::button_sm(Button::new("factory-validate").outline().label("Validate"))
                }
                    .disabled(matches!(self.validation, Validation::Running))
                    .on_click(cx.listener(|this, _, _, cx| this.validate(cx)))))
            .when(float_inspector, |this| this.child(components::icon_button(Button::new("factory-inspector-toggle").ghost()
                .icon(gpui_kit::assets::IconName::PanelRight).tooltip("Inspector · or double-click a stage to edit it").selected(self.inspector_open), metrics::CONTROL_MD)
                .on_click(cx.listener(|this, _, window, cx| if this.inspector_open { this.close_inspector(window, cx) } else { this.open_inspector(window, cx) }))))
            .child(components::icon_button(Button::new("factory-more").ghost().icon(IconName::Ellipsis).tooltip("More"), metrics::CONTROL_MD).dropdown_menu({
                move |menu, _, _| {
                    let (export, import, delete, arrange) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
                    let (undo, redo) = (undo_item.clone(), redo_item.clone());
                    menu.when(compact, |menu| menu
                            .item(PopupMenuItem::new("Undo").disabled(!can_undo).on_click(move |_, window, cx| { undo.update(cx, |this, cx| this.undo(window, cx)).ok(); }))
                            .item(PopupMenuItem::new("Redo").disabled(!can_redo).on_click(move |_, window, cx| { redo.update(cx, |this, cx| this.redo(window, cx)).ok(); }))
                            .separator())
                        .when(has_workflow, |menu| menu.item(PopupMenuItem::new("Arrange left to right").on_click(move |_, _, cx| {
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

    /// The validation status; narrow toolbars show a short label with the full one as tooltip.
    fn render_validation(&self, short: bool, cx: &App) -> Option<AnyElement> {
        let (full, brief, pill): (SharedString, SharedString, fn(SharedString, &App) -> Div) = match &self.validation {
            Validation::Idle => return None,
            Validation::Running => ("Validating…".into(), "…".into(), |label, cx| ui::chip(label, cx)),
            Validation::Done(result) if result.valid => ("Valid".into(), "Valid".into(), |label, cx| components::status_pill(label, components::Tone::Success, cx)),
            Validation::Done(result) => {
                let n = result.errors.len();
                (format!("{n} issue{}", if n == 1 { "" } else { "s" }).into(), if n > 99 { "99+".into() } else { n.to_string().into() }, |label, cx| components::status_pill(label, components::Tone::Danger, cx))
            }
            Validation::Failed(_) => ("Couldn't validate".into(), "!".into(), |label, cx| ui::amber_chip(label, cx)),
        };
        if !short { return Some(pill(full, cx).into_any_element()) }
        Some(pill(brief, cx).id("factory-validation").tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(full.clone()).build(window, cx)).into_any_element())
    }

    // ---- Inspector -------------------------------------------------------------------

    /// Close for the floating inspector, placed in each form's own header row.
    fn inspector_close(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        narrower_than(&self.width, DOCK_INSPECTOR_W).then(|| components::icon_button(Button::new("factory-inspector-float-close").ghost()
            .icon(IconName::Close).tooltip("Close (Esc)"), metrics::CONTROL_SM)
            .on_click(cx.listener(|this, _, window, cx| this.close_inspector(window, cx))).into_any_element())
    }

    /// Opens the inspector; floating, it takes focus so Esc reaches its handler.
    pub(super) fn open_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inspector_open = true;
        if narrower_than(&self.width, DOCK_INSPECTOR_W) { window.focus(&self.inspector_focus, cx) }
        cx.notify();
    }

    /// Closes the floating inspector and hands focus back to the canvas.
    fn close_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inspector_open = false;
        window.focus(&self.canvas.focus_handle(cx), cx);
        cx.notify();
    }

    fn render_inspector(&self, config: &ProjectConfig, workflow: &WorkflowDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let body = match self.selected.clone() {
            Selected::Workflow => self.render_workflow_form(config, workflow, cx).into_any_element(),
            Selected::Stage(id) => self.render_stage_form(config, workflow, &id, cx).into_any_element(),
            Selected::Edge(id) => self.render_edge_form(workflow, &id, cx).into_any_element(),
        };
        div().id("factory-inspector").w(px(INSPECTOR_W)).flex_none().h_full().overflow_y_scroll().border_l_1().border_color(t.border).bg(t.canvas)
            .child(div().p_4().flex().flex_col().gap_6().child(body).children(self.render_issues(cx)))
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
        let shared = workflow_instruction_ids(config, workflow);
        let scheduler_summary = scheduler_summary(config, workflow);
        let transitions = workflow.graph.as_ref().and_then(|graph| graph.max_transitions);
        div().flex().flex_col().gap_6()
            .child(block()
            .child(div().flex().items_center().gap_2().child(div().flex_1().t_headline().child("Workflow")).children(self.inspector_close(cx)))
            .child(ui::field("Name", self.input(Key::WorkflowName), cx))
            .child(ui::field("Instructions", self.area(Key::WorkflowInstructions, cx), cx))
            .child(ui::field("Starts at", picker(Button::new("factory-entry").label(entry))
                .disabled(stages.is_empty())
                .dropdown_menu(move |mut menu, _, _| {
                    for (id, name) in stages.iter() {
                        let (weak, id) = (weak.clone(), id.clone());
                        menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| { weak.update(cx, |this, cx| this.edit_workflow(cx, |w| graph::set_entry(w, &id))).ok(); }));
                    }
                    menu
                }), cx))
            .child(self.instruction_toggles("Instruction files", config, &shared, cx, |this, id, cx| {
                let Some(inherited) = this.config_value(cx).map(|config| config.instructions.iter().map(|item| item.id.clone()).collect::<Vec<_>>()) else { return };
                this.edit_workflow(cx, move |workflow| toggle(workflow.instruction_ids.get_or_insert(inherited), id));
            })))
            .child(self.group("wf-limits", "Run limits", Some(transitions.map_or("Default".into(), |n| format!("{n} moves").into())), cx)
                .when(self.is_open("wf-limits"), |this| this
                    .child(ui::field("Max stage moves", short(self.input(Key::MaxTransitions)), cx))
                    .child(hint("A run stops after this many moves between stages. Leave blank for 100.", cx))))
            .child(titled("Project settings", Some("Shared by every workflow in this project."), cx)
                .child(ui::field("Environment", environment_dropdown(&self.config, &self.targets, cx), cx))
                .child(hint("More options in Settings → Environment.", cx))
                .child(self.group("wf-scheduler", "Scheduler", Some(scheduler_summary), cx).pt_3()
                    .when(self.is_open("wf-scheduler"), |this| this.child(self.render_scheduler(config, workflow, cx)))))
    }

    fn render_scheduler(&self, config: &ProjectConfig, workflow: &WorkflowDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let scheduler = config.scheduler.clone().unwrap_or_default();
        let this_workflow = scheduler.workflow_id.as_deref() == Some(workflow.id.as_str());
        let workflow_id = workflow.id.clone();
        let statuses = scheduler.statuses.clone().unwrap_or_else(|| DEFAULT_STATUSES.to_vec());
        let config_entity = self.config.downgrade();
        let edit = move |cx: &mut App, f: Box<dyn FnOnce(&mut SchedulerConfig)>| {
            config_entity.update(cx, |config, cx| config.edit(cx, |config| f(config.scheduler.get_or_insert_with(SchedulerConfig::default)))).ok();
        };
        let (edit_enabled, edit_target) = (edit.clone(), edit.clone());
        div().flex().flex_col().gap_3()
            .child(div().flex().items_center().gap_2()
                .child(div().flex_1().t_label().child("Start runs automatically"))
                .child(Switch::new("factory-scheduler").checked(scheduler.enabled)
                    .on_click(move |checked: &bool, _, cx| { let checked = *checked; edit_enabled(cx, Box::new(move |s| s.enabled = checked)) })))
            .when(!scheduler.enabled, |this| this.child(hint("Picks up tasks from the board and runs them. One scheduler per project.", cx)))
            .when(scheduler.enabled, |this| this
                .child(toggle_row("factory-scheduler-workflow".into(), this_workflow, "Run tasks with this workflow".into(), None, cx)
                    .on_click(move |_, _, cx| {
                        let id = workflow_id.clone();
                        edit_target(cx, Box::new(move |s| s.workflow_id = if this_workflow { None } else { Some(id) }))
                    }))
                .child(div().flex().flex_col().gap_0p5()
                    .child(ui::caption("Pick up tasks in", cx))
                    .children(ui::BOARD_STATUSES.iter().map(|status| {
                        let (on, status, edit) = (statuses.contains(status), status.clone(), edit.clone());
                        toggle_row(SharedString::from(format!("factory-scheduler-{}", status.as_str())), on, ui::status_label(&status).to_string(), None, cx)
                            .on_click(move |_, _, cx| {
                                let status = status.clone();
                                edit(cx, Box::new(move |s| toggle(s.statuses.get_or_insert_with(|| DEFAULT_STATUSES.to_vec()), &status)))
                            })
                    })))
                .child(div().flex().gap_2()
                    .child(div().flex_1().min_w_0().child(ui::field("Runs at once", self.input(Key::MaxConcurrentRuns), cx)))
                    .child(div().flex_1().min_w_0().child(ui::field("Check every (s)", self.input(Key::PollSeconds), cx)))))
    }

    fn instruction_toggles(&self, title: &'static str, config: &ProjectConfig, selected: &[String], cx: &mut Context<Self>,
        on_toggle: fn(&mut Self, &String, &mut Context<Self>)) -> impl IntoElement + use<> {
        div().flex().flex_col().gap_0p5()
            .child(ui::caption(title, cx))
            .when(config.instructions.is_empty(), |this| this.child(hint("None yet. Add files in Settings → Instructions.", cx)))
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
        let commands_summary = super::super::join_parts(&[
            &if hooks > 0 { format!("{hooks} command{}", if hooks == 1 { "" } else { "s" }) } else { String::new() },
            &if gates > 0 { format!("{gates} gate{}", if gates == 1 { "" } else { "s" }) } else { String::new() },
        ]);
        let instructions_summary = match (stage.instructions.trim().is_empty(), instructions.len()) {
            (true, 0) => "Workflow's instructions".to_string(),
            (false, 0) => "Written".to_string(),
            (true, n) => format!("{n} file{}", if n == 1 { "" } else { "s" }),
            (false, n) => format!("Written, {n} file{}", if n == 1 { "" } else { "s" }),
        };
        let environment_summary = super::super::join_parts(&[
            &execution.execution_target.as_ref().map_or("Project default".to_string(), |target| ui::target_label(target).to_string()),
            &execution.cwd.clone().unwrap_or_default(),
        ]);
        let limits_summary = super::super::join_parts(&[
            &stage.max_visits.map_or(String::new(), |n| format!("{n} visits")),
            &execution.timeout_ms.map_or(String::new(), |ms| format!("{} timeout", seconds(ms))),
            &execution.retries.filter(|n| *n > 0).map_or(String::new(), |n| format!("{n} retries")),
        ]);
        let or_default = |summary: String, default: &str| -> SharedString { if summary.is_empty() { default.to_string().into() } else { summary.into() } };
        div().flex().flex_col().gap_6()
            .child(block()
                .child(div().flex().items_center().gap_2()
                    .child(div().flex_1().t_headline().child("Stage"))
                    .when(entry, |this| this.child(ui::chip("Entry", cx)))
                    .child(components::icon_button(Button::new("factory-stage-delete").ghost().icon(gpui_kit::assets::IconName::Trash).tooltip("Delete stage"), metrics::CONTROL_SM)
                        .on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx))))
                    .children(self.inspector_close(cx)))
                .child(ui::field("Name", self.input(Key::StageName), cx))
                .child(ui::field("Done when", self.input(Key::ExitRule), cx))
                .child(self.render_agents(config, &stage, cx)))
            .child(titled("Routes", None, cx)
                .child(self.render_routes(workflow, &stage.id, cx))
                .child(self.render_parallel(workflow, &stage.id, cx)))
            .child(self.group("instructions", "Instructions", Some(instructions_summary.into()), cx)
                .when(self.is_open("instructions"), |this| this
                    .child(ui::field("For this stage", self.area(Key::StageInstructions, cx), cx))
                    .child(self.instruction_toggles("Instruction files", config, &instructions, cx, |this, id, cx| {
                        let Some(stage) = this.selected_stage_id().map(str::to_string) else { return };
                        this.edit_stage(&stage, cx, |stage| toggle(stage.instruction_ids.get_or_insert_with(Vec::new), id));
                    }))))
            .child(self.group("commands", "Commands and gates", Some(or_default(commands_summary, "None")), cx)
                .when(self.is_open("commands"), |this| this
                    .child(hint("Shell commands run around the agents. One per line.", cx))
                    .child(ui::field("Before agents", self.area(Key::Before, cx), cx))
                    .child(ui::field("Commands", self.area(Key::Commands, cx), cx))
                    .child(div().flex().flex_col().gap_1()
                        .child(ui::field("Gates", self.area(Key::Gates, cx), cx))
                        .child(hint("approval: <question> or command: <executable args>", cx)))
                    .child(ui::field("After agents", self.area(Key::After, cx), cx))))
            .child(self.group("environment", "Environment", Some(environment_summary.into()), cx)
                .when(self.is_open("environment"), |this| this
                    .child(self.render_stage_target(&stage, cx))
                    .child(ui::field("Working directory", self.input(Key::Cwd), cx))))
            .child(self.group("limits", "Limits", Some(or_default(limits_summary, "Default")), cx)
                .when(self.is_open("limits"), |this| this
                    .child(div().flex().gap_2()
                        .child(div().flex_1().min_w_0().child(ui::field("Max visits", self.input(Key::MaxVisits), cx)))
                        .child(div().flex_1().min_w_0().child(ui::field("Timeout (s)", self.input(Key::TimeoutSeconds), cx))))
                    .child(hint("Blank: 3 visits per run; 10 min for agents and 2 min per command.", cx))
                    .map(|this| if self.state.service.read(cx).capabilities().step_retries {
                        this.child(div().flex().gap_2()
                            .child(div().flex_1().min_w_0().child(ui::field("Retries", self.input(Key::Retries), cx)))
                            .child(div().flex_1().min_w_0().child(ui::field("Retry delay (s)", self.input(Key::RetryDelaySeconds), cx))))
                        .child(hint("Retries failed steps, not denials or cancellations.", cx))
                    } else {
                        this.child(ui::update_service_note("retry failed steps automatically", cx))
                    })))
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
        div().flex().flex_col().gap_1().children(OUTCOMES.into_iter().filter(move |(on, _)| !(is_fork && *on == StageOutcome::Success)).map(|(on, label)| {
            let edge = graph.edges.iter().find(|edge| edge.source_stage_id == stage_id && edge.on == on).cloned();
            let current = edge.as_ref().and_then(|edge| stages.iter().find(|(id, _)| id == &edge.target_stage_id)).map_or("End".to_string(), |(_, name)| name.clone());
            let (weak, stages, source) = (cx.weak_entity(), stages.clone(), stage_id.to_string());
            div().flex().items_center().gap_2()
                .child(div().w(px(80.)).flex_none().t_label().text_color(theme::tokens(cx).text_secondary).child(label))
                .child(div().flex_1().min_w_0().child(picker(Button::new(SharedString::from(format!("route-{}", on.as_str()))).label(current))
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
                        .on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx))))
                    .children(self.inspector_close(cx)))
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
                    .on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx))))
                .children(self.inspector_close(cx)))
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
                .child(div().flex_1().min_w_0().child(picker(Button::new("factory-join").label(name(&fork.join_stage_id)))
                    .dropdown_menu(move |mut menu, _, _| {
                        for (id, label) in join_menu.iter() {
                            let (weak, id, source) = (join_weak.clone(), id.clone(), join_source.clone());
                            menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, _, cx| {
                                weak.update(cx, |this, cx| this.edit_workflow(cx, |w| fork::set_join(w, &source, &id))).ok();
                            }));
                        }
                        menu
                    }))))
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
        ui::field("Runs on", picker(Button::new("stage-target").label(label))
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
            }), cx)
    }
}

impl FactoryPane {
    /// Collapsible group: title, then the summary while folded, chevron at the trailing edge,
    /// so titles share the fields' left edge. Open state is per key and survives selection.
    fn group(&self, key: &'static str, title: &'static str, summary: Option<SharedString>, cx: &mut Context<Self>) -> Div {
        let (t, open) = (theme::tokens(cx), self.is_open(key));
        // A Kit button: Tab reaches it, Enter/Space toggles it, and it shows the focus ring.
        let header = components::button_sm(Button::new(SharedString::from(format!("group-{key}"))).ghost()).w_full().px_0()
            .child(div().w_full().min_w_0().flex().items_center().gap_2()
                .child(div().flex_none().t_label().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title))
                .child(div().flex_1().min_w_0().truncate().t_caption().text_color(t.text_secondary).children(summary.filter(|_| !open)))
                .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(14.)).text_color(t.text_secondary)))
            .on_click(cx.listener(move |this, _, _, cx| {
                let open = this.is_open(key);
                this.groups.insert(key, !open);
                cx.notify();
            }));
        block().child(header)
    }

    /// Instructions starts open; technical groups start folded and summarise what they hold.
    pub(super) fn is_open(&self, key: &'static str) -> bool {
        self.groups.get(key).copied().unwrap_or(key == "instructions")
    }
}

fn toggle<T: PartialEq + Clone>(list: &mut Vec<T>, id: &T) {
    match list.iter().position(|existing| existing == id) {
        Some(index) => { list.remove(index); }
        None => list.push(id.clone()),
    }
}

/// Unset means every project instruction file (`factory.ts` compile); `Some([])` means none.
fn workflow_instruction_ids(config: &ProjectConfig, workflow: &WorkflowDefinition) -> Vec<String> {
    workflow.instruction_ids.clone().unwrap_or_else(|| config.instructions.iter().map(|item| item.id.clone()).collect())
}

/// The project scheduler from this workflow's point of view.
fn scheduler_summary(config: &ProjectConfig, workflow: &WorkflowDefinition) -> SharedString {
    let Some(scheduler) = config.scheduler.as_ref().filter(|scheduler| scheduler.enabled) else { return "Off".into() };
    match scheduler.workflow_id.as_deref() {
        Some(id) if id == workflow.id => "Runs this workflow".into(),
        Some(id) => format!("Runs {}", config.workflows.iter().find(|w| w.id == id).map_or("another workflow", |w| w.name.as_str())).into(),
        None => "On, each task's workflow".into(),
    }
}

/// Toolbar space for the pane's actual allocated width. Widths are the controls' rendered sizes
/// plus the bar's 8pt gaps. Space is reclaimed in order — Undo / Redo move into More, the
/// validation status shortens, then Stage and Validate become icons — until the workflow
/// picker keeps at least a short name. Every control stays on the bar.
struct ToolbarBudget { compact: bool, short_status: bool, icon_actions: bool, name_w: Option<f32> }

const NAME_MIN_W: f32 = 88.;
const STATUS_W: f32 = 128.;
const SHORT_STATUS_W: f32 = 56.;
const ACTIONS_W: f32 = 160.; // Stage + Validate with labels

fn toolbar_budget(width: Option<f32>, library: bool, inspector: bool, status: bool, workflow: bool) -> ToolbarBudget {
    let Some(width) = width else { return ToolbarBudget { compact: false, short_status: false, icon_actions: false, name_w: None } };
    let icon = metrics::CONTROL_MD + 8.;
    let fixed = 16. + 8. + icon // insets, picker gap, More
        + if library { icon } else { 0. }
        + if inspector { icon } else { 0. };
    let actions = if workflow { ACTIONS_W } else { 0. };
    let status_w = if status { STATUS_W } else { 0. };
    let compact = width - fixed - actions - status_w - icon * 2. < 160.;
    let undo = if compact { 0. } else { icon * 2. };
    let mut left = width - fixed - actions - status_w - undo;
    let short_status = status && left < NAME_MIN_W;
    if short_status { left += STATUS_W - SHORT_STATUS_W }
    let icon_actions = workflow && left < NAME_MIN_W;
    if icon_actions { left += ACTIONS_W - icon * 2. }
    // Only below every reclaim (under ~350) does the floor win and the bar clip.
    ToolbarBudget { compact, short_status, icon_actions, name_w: Some(left.max(NAME_MIN_W)) }
}

fn seconds(ms: u64) -> String {
    if ms % 60_000 == 0 { format!("{} min", ms / 60_000) } else { format!("{} s", ms / 1000) }
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
        let inspector_focus = self.inspector_focus.clone();
        let floating = |id: &'static str, left: bool, panel: AnyElement, cx: &mut Context<Self>| {
            div().id(id).occlude().absolute().top_0().bottom_0().map(|this| if left { this.left_0() } else { this.right_0() })
                .shadow_lg().bg(t.canvas).flex().flex_col()
                // The library has no header of its own, so it gets a close row; the inspector's
                // close sits in each form's header (`inspector_close`) and Esc closes it too.
                .when(left, |this| this.child(div().flex_none().h(px(metrics::CONTROL_MD + 8.)).px_1().flex().items_center().justify_end().border_b_1().border_color(t.border)
                    .child(components::icon_button(Button::new(SharedString::from(format!("{id}-close"))).ghost()
                        .icon(IconName::Close).tooltip("Close"), metrics::CONTROL_SM)
                        .on_click(cx.listener(|this, _, _, cx| { this.library_open = false; cx.notify() })))))
                .when(!left, |this| this.track_focus(&inspector_focus).on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" { cx.stop_propagation(); this.close_inspector(window, cx) }
                })))
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
