//! Graphical workflow editor: stage nodes in order with transition arrows and return edges,
//! drag to reorder, and a side panel for the selected stage (agents, board status, exit rule,
//! instructions, return targets). Edits go to the shared `ConfigDraft`.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{ProjectConfig, StageDefinition, TaskStatus, WorkflowDefinition};

use super::config::ConfigDraft;
use crate::views::tasks::{new_id, ui::{self, Load}};
use crate::components::{self, Tone, Typography as _};
use crate::theme::{self as theme_tokens, metrics};

#[derive(Clone)]
struct StageDrag {
    index: usize,
    name: String,
}

impl Render for StageDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme_tokens::tokens(cx);
        div().w(px(NODE_W)).p_3().rounded(px(metrics::R_INNER)).border_1().border_color(t.border).bg(t.surface).shadow_md()
            .t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child(self.name.clone())
    }
}

pub struct WorkflowEditor {
    config: Entity<ConfigDraft>,
    workflow: usize,
    stage: Option<usize>,
    workflow_name: Entity<InputState>,
    workflow_description: Entity<TextareaState>,
    stage_name: Entity<InputState>,
    exit_rule: Entity<InputState>,
    stage_instructions: Entity<TextareaState>,
    syncing: bool,
    graph_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl WorkflowEditor {
    pub fn new(config: Entity<ConfigDraft>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workflow_name = cx.new(|cx| InputState::new(window, cx).placeholder("Workflow name"));
        let workflow_description = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 6).placeholder("Instructions that apply to every stage"));
        let stage_name = cx.new(|cx| InputState::new(window, cx).placeholder("Stage name"));
        let exit_rule = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. moves on when every check is green"));
        let stage_instructions = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 10).placeholder("What agents do in this stage"));
        let subscriptions = vec![
            cx.observe_in(&config, window, |this, _, window, cx| { this.sync_inputs(window, cx); cx.notify() }),
            cx.subscribe_in(&workflow_name, window, |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.edit_workflow(cx, |workflow| workflow.name = value);
                }
            }),
            cx.subscribe_in(&workflow_description, window, |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.edit_workflow(cx, |workflow| workflow.instructions = value);
                }
            }),
            cx.subscribe_in(&stage_name, window, |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.edit_stage(cx, |stage| stage.name = value);
                }
            }),
            cx.subscribe_in(&exit_rule, window, |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.edit_stage(cx, |stage| stage.exit_rule = Some(value));
                }
            }),
            cx.subscribe_in(&stage_instructions, window, |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.edit_stage(cx, |stage| stage.instructions = value);
                }
            }),
        ];
        let mut this = Self {
            config,
            workflow: 0,
            stage: None,
            workflow_name,
            workflow_description,
            stage_name,
            exit_rule,
            stage_instructions,
            syncing: false,
            graph_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.sync_inputs(window, cx);
        this
    }

    fn current<'a>(&self, config: &'a ProjectConfig) -> Option<&'a WorkflowDefinition> {
        config.workflows.get(self.workflow)
    }

    /// Push the selected workflow/stage values into the inputs without marking the draft dirty.
    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.config.read(cx).config().cloned();
        let workflow = config.as_ref().and_then(|config| self.current(config)).cloned();
        if workflow.is_none() { self.workflow = 0 }
        let stage = workflow.as_ref().and_then(|workflow| self.stage.and_then(|index| workflow.stages.get(index))).cloned();
        if stage.is_none() { self.stage = None }
        self.syncing = true;
        set_input(&self.workflow_name, workflow.as_ref().map_or("", |w| &w.name), window, cx);
        set_textarea(&self.workflow_description, workflow.as_ref().map_or("", |w| &w.instructions), window, cx);
        set_input(&self.stage_name, stage.as_ref().map_or("", |s| &s.name), window, cx);
        set_input(&self.exit_rule, stage.as_ref().and_then(|s| s.exit_rule.as_deref()).unwrap_or(""), window, cx);
        set_textarea(&self.stage_instructions, stage.as_ref().map_or("", |s| &s.instructions), window, cx);
        self.syncing = false;
    }

    fn edit_workflow(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut WorkflowDefinition)) {
        if self.syncing { return }
        let index = self.workflow;
        self.config.update(cx, |config, cx| config.edit(cx, |config| {
            if let Some(workflow) = config.workflows.get_mut(index) { f(workflow) }
        }));
    }

    fn edit_stage(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut StageDefinition)) {
        let Some(stage) = self.stage else { return };
        self.edit_workflow(cx, |workflow| if let Some(stage) = workflow.stages.get_mut(stage) { f(stage) });
    }

    fn select_stage(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.stage = index;
        self.sync_inputs(window, cx);
        cx.notify();
    }

    fn add_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut index = 0;
        self.config.update(cx, |config, cx| config.edit(cx, |config| {
            config.workflows.push(WorkflowDefinition {
                id: new_id("wf"),
                name: "New workflow".into(),
                description: String::new(),
                instructions: String::new(),
                stages: Vec::new(),
                extra: Default::default(),
            });
            index = config.workflows.len() - 1;
        }));
        self.workflow = index;
        self.select_stage(None, window, cx);
    }

    fn add_stage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut added = None;
        self.edit_workflow(cx, |workflow| {
            workflow.stages.push(StageDefinition {
                id: new_id("stage"),
                name: format!("Stage {}", workflow.stages.len() + 1),
                instructions: String::new(),
                agent_ids: Vec::new(),
                return_to_stage_ids: Vec::new(),
                exit_rule: None,
                status: None,
                color: None,
                extra: Default::default(),
            });
            added = Some(workflow.stages.len() - 1);
        });
        self.select_stage(added, window, cx);
    }

    fn move_stage(&mut self, from: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mut moved = None;
        self.edit_workflow(cx, |workflow| {
            if from < workflow.stages.len() && to < workflow.stages.len() && from != to {
                let stage = workflow.stages.remove(from);
                workflow.stages.insert(to, stage);
                moved = Some(to);
            }
        });
        if moved.is_some() { self.select_stage(moved, window, cx) }
    }

    fn delete_stage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.stage else { return };
        self.edit_workflow(cx, |workflow| {
            if index < workflow.stages.len() {
                let removed = workflow.stages.remove(index);
                for stage in workflow.stages.iter_mut() {
                    stage.return_to_stage_ids.retain(|id| id != &removed.id);
                }
            }
        });
        self.select_stage(None, window, cx);
    }

    fn delete_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.workflow;
        self.config.update(cx, |config, cx| config.edit(cx, |config| {
            if index < config.workflows.len() { config.workflows.remove(index); }
        }));
        self.workflow = 0;
        self.select_stage(None, window, cx);
    }
}

fn set_input(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value {
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

fn set_textarea(input: &Entity<TextareaState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value {
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

// ---- Rendering -----------------------------------------------------------------------

impl WorkflowEditor {
    /// Workflow picker: shared segmented control plus "+ Workflow" (ref 1721E892 filters).
    fn render_workflow_bar(&self, config: &ProjectConfig, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let items = config.workflows.iter()
            .map(|workflow| components::Segment::new(if workflow.name.is_empty() { "Untitled".to_string() } else { workflow.name.clone() }))
            .collect();
        let weak = cx.weak_entity();
        div().flex().items_center().gap_2().flex_wrap()
            .when(!config.workflows.is_empty(), |this| this.child(components::segmented("workflow-pick", items, self.workflow, move |index, window, cx| {
                weak.update(cx, |this, cx| { this.workflow = index; this.select_stage(None, window, cx) }).ok();
            }, cx)))
            .child(components::button_md(Button::new("add-workflow").ghost().icon(IconName::Plus).label("Workflow"))
                .on_click(cx.listener(|this, _, window, cx| this.add_workflow(window, cx))))
    }

    fn render_graph(&self, config: &ProjectConfig, workflow: &WorkflowDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme_tokens::tokens(cx);
        let count = workflow.stages.len();
        // Full-width scroll viewport (min_w_0, or the row grows to its content and the page
        // clips stage 03); fades mark stages past either edge.
        let mut row = div().id("stage-graph").w_full().min_w_0().flex().items_start().gap_0().overflow_x_scroll()
            .track_scroll(&self.graph_scroll).pb_2();
        for (index, stage) in workflow.stages.iter().enumerate() {
            row = row.child(self.render_node(config, workflow, index, stage, cx));
            if index + 1 < count {
                row = row.child(div().h(px(NODE_H)).flex().items_center().px_1()
                    .child(div().w(px(16.)).h(px(1.)).bg(t.border))
                    .child(Icon::new(IconName::ChevronRight).size(px(14.)).text_color(t.text_tertiary)));
            }
        }
        let row = row.child(
            div().id("add-stage-node").ml_3().w(px(128.)).h(px(NODE_H)).flex_none().rounded(px(metrics::R_INNER)).border_1().border_dashed()
                .border_color(t.border).flex().items_center().justify_center().gap_1p5().cursor_pointer()
                .t_label().text_color(t.text_secondary).hover(move |style| style.bg(t.hover).text_color(t.text))
                .child(Icon::new(IconName::Plus).size(px(14.))).child("Stage")
                .on_click(cx.listener(|this, _, window, cx| this.add_stage(window, cx))),
        );
        div().relative().w_full().min_w_0().child(row)
            .children(crate::views::responsive::x_scroll_fades(&self.graph_scroll, t.canvas))
    }

    /// Stage node (ref 851174F7 diagram nodes): R8 bordered box, mono index, name, meta.
    fn render_node(&self, config: &ProjectConfig, workflow: &WorkflowDefinition, index: usize, stage: &StageDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme_tokens::tokens(cx);
        let selected = self.stage == Some(index);
        let status = stage.status.clone().unwrap_or(TaskStatus::Todo);
        let agents: Vec<String> = stage.agent_ids.iter()
            .map(|id| config.agents.iter().find(|agent| &agent.id == id).map_or_else(|| "Missing agent".into(), |agent| agent.name.clone()))
            .collect();
        let returns: Vec<String> = stage.return_to_stage_ids.iter()
            .filter_map(|id| workflow.stages.iter().find(|stage| &stage.id == id).map(|stage| stage.name.clone()))
            .collect();
        let drag = StageDrag { index, name: stage.name.clone() };
        div().flex().flex_col().gap_1p5().flex_none()
            .child(
                div().id(("stage-node", index)).w(px(NODE_W)).h(px(NODE_H)).p_3().rounded(px(metrics::R_INNER)).border_1()
                    .border_color(if selected { t.running } else { t.border })
                    .bg(t.surface).when(selected, |this| this.shadow_xs()).cursor_pointer()
                    .when(!selected, |this| this.hover(move |style| style.border_color(t.text_tertiary)))
                    .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .drag_over::<StageDrag>(|style, _, _, cx| style.bg(cx.theme().accent))
                    .on_drop(cx.listener(move |this, drag: &StageDrag, window, cx| this.move_stage(drag.index, index, window, cx)))
                    .on_click(cx.listener(move |this, _, window, cx| this.select_stage(Some(index), window, cx)))
                    .flex().flex_col().gap_1()
                    .child(div().flex().items_center().gap_2()
                        .child(div().px_1().rounded(px(metrics::R_CONTROL)).bg(t.tray).t_code_caption(cx).text_color(t.text_secondary).child(format!("{:02}", index + 1)))
                        .child(components::status_dot(Tone::Neutral, cx).bg(ui::status_color(&status, cx)))
                        .child(div().flex_1().min_w_0().truncate().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child(stage.name.clone())))
                    .child(div().flex().items_center().gap_1p5().t_caption().text_color(t.text_secondary)
                        .child(Icon::new(IconName::Bot).size(px(14.)))
                        .child(div().min_w_0().truncate().child(match agents.len() { 0 => "No agents".to_string(), 1 => agents[0].clone(), n => format!("{} +{}", agents[0], n - 1) })))
                    .child(div().truncate().t_caption().text_color(t.text_secondary)
                        .child(stage.exit_rule.clone().filter(|rule| !rule.is_empty()).unwrap_or_else(|| "No exit rule".into()))),
            )
            .children(returns.into_iter().map(move |name| {
                div().w(px(NODE_W)).px_2().h(px(metrics::CHIP_H)).flex().items_center().gap_1p5().rounded(px(metrics::R_CONTROL))
                    .bg(Tone::Attention.tint(cx, 0.10)).t_caption().text_color(ui::needs_you(cx))
                    .child(Icon::new(IconName::Undo2).size(px(12.))).child(div().min_w_0().truncate().child(format!("returns to {name}")))
            }))
    }

    fn render_stage_panel(&self, config: &ProjectConfig, workflow: &WorkflowDefinition, index: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(stage) = workflow.stages.get(index) else { return div().into_any_element() };
        let weak = cx.weak_entity();
        let status_label = stage.status.as_ref().map_or("Keep task status", |status| ui::status_label(status)).to_string();
        let unknown_status = stage.status.as_ref().is_some_and(|status| !status.is_known());
        let count = workflow.stages.len();
        let t = theme_tokens::tokens(cx);
        // Stage inspector (refs B6BD5981 panel, 630BC713 property grid): header, then 100-column rows.
        components::section_card(cx).py_4().gap_1()
            .child(div().flex().items_center().justify_between().pb_2()
                .child(div().t_label().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(format!("Stage {:02}", index + 1)))
                .child(div().flex().gap_0p5()
                    .child(components::icon_button(Button::new("stage-left").ghost().icon(IconName::ChevronLeft).tooltip("Move earlier"), metrics::CONTROL_SM)
                        .when(index == 0, |this| this.disabled(true))
                        .on_click(cx.listener(move |this, _, window, cx| this.move_stage(index, index.saturating_sub(1), window, cx))))
                    .child(components::icon_button(Button::new("stage-right").ghost().icon(IconName::ChevronRight).tooltip("Move later"), metrics::CONTROL_SM)
                        .when(index + 1 >= count, |this| this.disabled(true))
                        .on_click(cx.listener(move |this, _, window, cx| this.move_stage(index, index + 1, window, cx))))
                    .child(components::icon_button(Button::new("stage-delete").ghost().icon(gpui_kit::assets::IconName::Trash).tooltip("Delete stage"), metrics::CONTROL_SM)
                        .on_click(cx.listener(|this, _, window, cx| this.delete_stage(window, cx))))))
            .child(prop_row("Name", components::input_md(Input::new(&self.stage_name)), cx))
            .child(prop_row("Exit rule", components::input_md(Input::new(&self.exit_rule)), cx))
            .child(prop_row("Entry status", if unknown_status {
                ui::chip(format!("{} (from a newer version)", stage.status.as_ref().map_or("", |s| s.as_str())), cx).into_any_element()
            } else {
                div().flex().child(components::button_sm(Button::new("stage-status").outline().label(status_label).dropdown_caret(true)).dropdown_menu({
                    let weak = weak.clone();
                    move |mut menu, _, _| {
                        let clear = weak.clone();
                        menu = menu.item(PopupMenuItem::new("Keep task status").on_click(move |_, _, cx| {
                            clear.update(cx, |this, cx| this.edit_stage(cx, |stage| stage.status = None)).ok();
                        }));
                        for status in ui::BOARD_STATUSES.iter() {
                            let (weak, status) = (weak.clone(), status.clone());
                            menu = menu.item(PopupMenuItem::new(ui::status_label(&status).to_string()).on_click(move |_, _, cx| {
                                weak.update(cx, |this, cx| this.edit_stage(cx, |stage| stage.status = Some(status.clone()))).ok();
                            }));
                        }
                        menu
                    }
                })).into_any_element()
            }, cx))
            .child(prop_row("Agents", div().flex().flex_col().gap_0p5()
                .when(config.agents.is_empty(), |this| this.child(div().t_caption().text_color(cx.theme().muted_foreground)
                    .child("No agents yet. Add them under Settings → Agents.")))
                .children(config.agents.iter().map(|agent| {
                    let position = stage.agent_ids.iter().position(|id| id == &agent.id);
                    let id = agent.id.clone();
                    toggle_row(SharedString::from(format!("stage-agent-{}", agent.id)), position.is_some(),
                        format!("{} · {}", agent.name, ui::harness_label(&agent.harness)),
                        position.map(|position| format!("#{}", position + 1)), cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.edit_stage(cx, |stage| {
                                if let Some(position) = stage.agent_ids.iter().position(|existing| existing == &id) {
                                    stage.agent_ids.remove(position);
                                } else {
                                    stage.agent_ids.push(id);
                                }
                            })
                        }))
                })), cx))
            .child(prop_row("Returns to", div().flex().flex_wrap().gap_1()
                .when(index == 0, |this| this.child(div().t_caption().text_color(cx.theme().muted_foreground).child("The first stage has no earlier stages.")))
                .children(workflow.stages.iter().take(index).map(|target| {
                    let on = stage.return_to_stage_ids.contains(&target.id);
                    let id = target.id.clone();
                    components::button_sm(Button::new(SharedString::from(format!("return-{}", target.id)))
                        .icon(if on { IconName::Check } else { IconName::Plus }).label(target.name.clone()))
                        .when(on, |this| this.outline()).when(!on, |this| this.ghost())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.edit_stage(cx, |stage| {
                                if let Some(position) = stage.return_to_stage_ids.iter().position(|existing| existing == &id) {
                                    stage.return_to_stage_ids.remove(position);
                                } else {
                                    stage.return_to_stage_ids.push(id);
                                }
                            })
                        }))
                })), cx))
            .child(prop_row("Instructions", components::textarea_box(&self.stage_instructions, cx), cx))
            .into_any_element()
    }
}

/// Checkable row used for agent/skill assignment (ref 630BC713 subtask rows): 28 high,
/// 14 checkbox, hover fill only.
pub fn toggle_row(id: SharedString, on: bool, label: String, meta: Option<String>, cx: &App) -> Stateful<Div> {
    let t = theme_tokens::tokens(cx);
    div().id(id).flex().items_center().gap_2().h(px(metrics::ROW_PROP)).px_2().rounded(px(metrics::R_CONTROL)).cursor_pointer()
        .hover(move |style| style.bg(t.hover))
        .child(div().size(px(14.)).flex_none().rounded(px(4.)).border_1().border_color(if on { t.accent_fill } else { t.text_tertiary })
            .when(on, |this| this.bg(t.accent_fill)).flex().items_center().justify_center()
            .when(on, |this| this.child(Icon::new(IconName::Check).size(px(10.)).text_color(t.accent_text))))
        .child(div().flex_1().min_w_0().t_label().text_color(t.text).truncate().child(label))
        .children(meta.map(|meta| div().t_caption().font_features(components::typography::tabular()).text_color(t.text_secondary).child(meta)))
}

/// Inspector property row: 100 label column, control fills (refs 630BC713, B6BD5981).
fn prop_row(label: &'static str, control: impl IntoElement, cx: &App) -> Div {
    div().flex().items_start().gap_3().py_1()
        .child(div().w(px(100.)).flex_none().h(px(metrics::CONTROL_MD)).flex().items_center().t_label()
            .text_color(theme_tokens::tokens(cx).text_secondary).child(label))
        .child(div().flex_1().min_w_0().min_h(px(metrics::CONTROL_MD)).flex().flex_col().justify_center().child(control))
}


const NODE_W: f32 = 200.;
const NODE_H: f32 = 84.;

impl Render for WorkflowEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let body = match self.current(&config).cloned() {
            None => ui::empty_panel(
                "No workflow yet",
                "A workflow is an ordered set of stages with agents. Tasks can follow it once execution is available.",
                Some(components::button_md(Button::new("first-workflow").primary().icon(IconName::Plus).label("Create workflow"))
                    .on_click(cx.listener(|this, _, window, cx| this.add_workflow(window, cx))).into_any_element()),
                cx,
            ).into_any_element(),
            // Delete lives on the section's label row so Name and Instructions share one
            // full-width field edge (b29, T4).
            Some(workflow) => div().flex().flex_col()
                .child(components::section_label("Workflow", cx).flex().items_center().justify_between()
                    .child(components::button_sm(Button::new("delete-workflow").ghost().icon(gpui_kit::assets::IconName::Trash).label("Delete workflow"))
                        .on_click(cx.listener(|this, _, window, cx| this.delete_workflow(window, cx)))))
                .child(components::section_card(cx).py_4().gap_1()
                    .child(prop_row("Name", components::input_md(Input::new(&self.workflow_name)), cx))
                    .child(prop_row("Instructions", components::textarea_box(&self.workflow_description, cx), cx)))
                .child(components::section_label("Stages · drag to reorder", cx))
                .child(self.render_graph(&config, &workflow, cx))
                .children(self.stage.map(|index| div().pt_6().child(self.render_stage_panel(&config, &workflow, index, cx))))
                .into_any_element(),
        };
        div().id("workflow-editor").size_full().overflow_y_scroll().child(
            components::page_body(false).max_w(px(980. + 2. * metrics::GUTTER_X))
                .child(self.render_workflow_bar(&config, cx))
                .child(body),
        ).into_any_element()
    }
}
