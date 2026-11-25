//! Board (status columns with drag and drop) and grouped list renderers for `TaskCollection`.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    input::Input,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{Task, TaskStatus};

use super::{Grouping, Placement, TaskCollection, TaskDrag, ui};
use crate::components::{self, Tone, Typography as _};
use crate::theme::{self, metrics};

/// One board column: what it shows and where dropped/added tasks land.
struct Column {
    key: String,
    title: String,
    rule: String,
    color: Hsla,
    status: TaskStatus,
    placement: Placement,
    /// `(workflow, stage)` for stage columns; quick-add creates the task there.
    stage: Option<(String, String)>,
    /// Which tasks belong here.
    filter: ColumnFilter,
    /// Drops are refused (e.g. the read-only "Other workflows" column).
    accepts_drop: bool,
}

#[derive(Clone)]
enum ColumnFilter {
    Status(TaskStatus),
    Stage(String, String),
    /// In the board workflow (or none) without one of its stages.
    NotStarted(String),
    /// Assigned to a different workflow than the one projected.
    OtherWorkflow(String),
}

impl TaskCollection {
    pub(super) fn render_board(&self, tasks: Vec<Task>, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        if self.tasks.ready().is_some_and(|all| all.is_empty()) && self.adding.is_none() {
            return self.render_no_tasks(cx).into_any_element();
        }
        let columns = self.columns(cx);
        // Each column owns its trailing gap (12, or 4 after a collapsed rail; ref E62FEC84).
        div()
            .id("board-scroll")
            .size_full()
            .overflow_x_scroll()
            .flex()
            .px(px(metrics::GUTTER_X))
            .pb_6()
            .children(columns.into_iter().map(|column| {
                let members: Vec<&Task> = tasks.iter().filter(|task| self.in_column(task, &column)).collect();
                self.render_column(column, members, window, cx)
            }))
            .into_any_element()
    }

    fn columns(&self, cx: &App) -> Vec<Column> {
        if let (Grouping::Stage, Some(workflow)) = (self.grouping, self.board_workflow()) {
            let wf = workflow.id.clone();
            let mut columns = vec![Column {
                key: format!("stage-none-{wf}"),
                title: "Not started".into(),
                rule: "moves on when it enters the first stage".into(),
                color: theme_tertiary(cx),
                status: TaskStatus::Todo,
                placement: Placement { workflow_id: Some(Some(wf.clone())), stage_id: Some(None), ..Default::default() },
                stage: None,
                filter: ColumnFilter::NotStarted(wf.clone()),
                accepts_drop: true,
            }];
            columns.extend(workflow.stages.iter().map(|stage| {
                let status = ui::stage_status(stage);
                Column {
                    key: format!("stage-{}", stage.id),
                    title: stage.name.clone(),
                    rule: stage.exit_rule.clone().filter(|rule| !rule.is_empty()).unwrap_or_else(|| "no exit rule set".into()),
                    color: ui::status_color(&status, cx),
                    placement: Placement {
                        status: stage.status.clone(),
                        workflow_id: Some(Some(wf.clone())),
                        stage_id: Some(Some(stage.id.clone())),
                        sort_order: None,
                    },
                    status,
                    stage: Some((wf.clone(), stage.id.clone())),
                    filter: ColumnFilter::Stage(wf.clone(), stage.id.clone()),
                    accepts_drop: true,
                }
            }));
            let others = self.tasks.ready().is_some_and(|tasks| tasks.iter().any(|task| task.workflow_id.as_ref().is_some_and(|id| id != &wf)));
            if others {
                columns.push(Column {
                    key: "stage-other".into(),
                    title: "Other workflows".into(),
                    rule: "switch the board workflow to move these".into(),
                    color: theme_tertiary(cx),
                    status: TaskStatus::Todo,
                    placement: Placement::default(),
                    stage: None,
                    filter: ColumnFilter::OtherWorkflow(wf),
                    accepts_drop: false,
                });
            }
            return columns;
        }
        let all = self.tasks.ready().cloned().unwrap_or_default();
        ui::statuses_for(&all).into_iter().map(|status| Column {
            key: format!("status-{}", status.as_str()),
            title: ui::status_label(&status).to_string(),
            rule: ui::status_rule(&status).into(),
            color: ui::status_color(&status, cx),
            placement: Placement { status: Some(status.clone()), ..Default::default() },
            stage: None,
            filter: ColumnFilter::Status(status.clone()),
            accepts_drop: status.is_known(),
            status,
        }).collect()
    }

    fn in_column(&self, task: &Task, column: &Column) -> bool {
        let stage_known = |workflow: &str| {
            task.workflow_id.as_deref() == Some(workflow)
                && self.board_workflow().is_some_and(|wf| wf.stages.iter().any(|stage| Some(&stage.id) == task.stage_id.as_ref()))
        };
        match &column.filter {
            ColumnFilter::Status(status) => &task.status == status,
            ColumnFilter::Stage(workflow, stage) => task.workflow_id.as_deref() == Some(workflow) && task.stage_id.as_deref() == Some(stage),
            ColumnFilter::NotStarted(workflow) => {
                task.workflow_id.as_ref().is_none_or(|id| id == workflow) && !stage_known(workflow)
            }
            ColumnFilter::OtherWorkflow(workflow) => task.workflow_id.as_ref().is_some_and(|id| id != workflow),
        }
    }

    fn render_column(&self, column: Column, tasks: Vec<&Task>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let key = column.key.clone();
        if self.collapsed_columns.contains(&key) {
            return self.render_collapsed(&column, tasks.len(), window, cx);
        }
        let t = theme::tokens(cx);
        let drop_placement = column.placement.clone();
        let accepts_drop = column.accepts_drop;
        let (add_status, add_stage) = (column.status.clone(), column.stage.clone());
        let adding_here = self.adding.as_ref() == Some(&column.status) && self.adding_stage == column.stage
            && matches!(column.filter, ColumnFilter::Status(_) | ColumnFilter::Stage(..));
        let can_add = matches!(column.filter, ColumnFilter::Status(_) | ColumnFilter::Stage(..)) && accepts_drop;
        let toggle_key = key.clone();
        let group: SharedString = format!("column-group-{key}").into();
        div()
            .id(SharedString::from(format!("column-{key}")))
            .group(group.clone())
            .w(px(COLUMN_W))
            .mr_3()
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .rounded(px(metrics::R_CARD))
            .bg(t.board_column)
            .border_1()
            .border_color(transparent_black())
            .when(accepts_drop, |this| this
                .drag_over::<TaskDrag>(|style, _, _, cx| style.border_color(cx.theme().ring))
                .on_drop(cx.listener(move |this, drag: &TaskDrag, _, cx| {
                    let mut placement = drop_placement.clone();
                    placement.sort_order = Some(this.last_order() + 1.);
                    this.move_task(&drag.id, placement, cx)
                })))
            .child(
                div().flex_none().px_3().pt_3().pb_2().flex().flex_col().gap_0p5()
                    .child(div().h(px(20.)).flex().items_center().gap_2()
                        .child(column_mark(&column, cx))
                        .child(div().flex_1().min_w_0().truncate().t_label().font_weight(FontWeight::SEMIBOLD).text_color(t.text)
                            .child(column.title.clone()))
                        // Trailing actions appear on hover; the count stays right-aligned.
                        .child(div().flex().flex_none().items_center().gap_0p5().invisible().group_hover(group.clone(), |style| style.visible())
                            .when(can_add, |this| this.child(components::icon_button(Button::new(SharedString::from(format!("add-{key}"))).custom(tray_button(cx))
                                .icon(IconName::Plus).tooltip("Add task"), metrics::CONTROL_XS)
                                .on_click(cx.listener(move |this, _, window, cx| this.start_adding(add_status.clone(), add_stage.clone(), window, cx)))))
                            .child(components::icon_button(Button::new(SharedString::from(format!("collapse-{key}"))).custom(tray_button(cx))
                                .icon(IconName::ChevronLeft).tooltip("Collapse column"), metrics::CONTROL_XS)
                                .on_click(cx.listener(move |this, _, _, cx| this.toggle_column(&toggle_key, cx)))))
                        .child(ui::count_pill(tasks.len(), cx)))
                    .child(div().truncate().t_caption().text_color(t.text_secondary).child(column.rule.clone())),
            )
            .child(
                // Scroll viewport inset 8 from the tray edges so cards never reach the
                // rounded corners (rectangular clip; visual contract clip policy).
                div().id(SharedString::from(format!("column-body-{key}"))).flex_1().min_h_0().overflow_y_scroll()
                    .mx_2().mb_2().flex().flex_col().gap_2()
                    .when(adding_here, |this| this.child(
                        div().p_1().rounded(px(metrics::R_INNER)).border_1().border_color(t.running).bg(t.surface)
                            .child(components::input_md(Input::new(&self.new_title).appearance(false))),
                    ))
                    .children(tasks.into_iter().map(|task| {
                        let placement = accepts_drop.then(|| column.placement.clone());
                        self.render_card(task, placement, cx)
                    })),
            )
            .into_any_element()
    }

    /// Collapsed column rail (ref E62FEC84 Backlog rail): 36 wide, chevron on the title
    /// row, count on the rule row, then the title rotated to read bottom-to-top.
    fn render_collapsed(&self, column: &Column, count: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let key = column.key.clone();
        let drop_placement = column.placement.clone();
        let accepts_drop = column.accepts_drop;
        div()
            .id(SharedString::from(format!("column-{key}")))
            .w(px(36.)).mr_1().flex_none().h_full().pt_3().flex().flex_col().items_center()
            .rounded(px(metrics::R_CARD)).bg(t.board_column).border_1().border_color(transparent_black())
            .cursor_pointer().hover(move |style| style.bg(t.hover))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_column(&key, cx)))
            .when(accepts_drop, |this| this
                .drag_over::<TaskDrag>(|style, _, _, cx| style.border_color(cx.theme().ring))
                .on_drop(cx.listener(move |this, drag: &TaskDrag, _, cx| {
                    let mut placement = drop_placement.clone();
                    placement.sort_order = Some(this.last_order() + 1.);
                    this.move_task(&drag.id, placement, cx)
                })))
            .child(div().h(px(20.)).flex().items_center().child(Icon::new(IconName::ChevronRight).size(px(14.)).text_color(t.text_secondary)))
            .child(div().h(px(16.)).mt_0p5().flex().items_center().t_caption().font_features(components::typography::tabular())
                .text_color(t.text_secondary).child(count.to_string()))
            .child(div().mt_2().child(components::vertical_label(column.title.clone(), t.text, window)))
            .into_any_element()
    }

    fn last_order(&self) -> f64 {
        self.tasks.ready().map_or(0., |tasks| tasks.iter().map(|task| task.sort_order).fold(0., f64::max))
    }

    /// Order just before `target`: midpoint with the task preceding it.
    fn order_before(&self, target: &Task) -> f64 {
        let previous = self.tasks.ready().into_iter().flatten()
            .filter(|task| task.sort_order < target.sort_order)
            .map(|task| task.sort_order)
            .fold(f64::NEG_INFINITY, f64::max);
        if previous.is_finite() { (previous + target.sort_order) / 2. } else { target.sort_order - 1. }
    }

    /// Board card (refs AE10217A anatomy, E62FEC84 amber "needs you" line).
    fn render_card(&self, task: &Task, placement: Option<Placement>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let drop_target = task.clone();
        let queued = self.queued.get(&task.id).copied().unwrap_or(0);
        let needs_you = queued > 0 || task.status == TaskStatus::Blocked;
        let amber = ui::needs_you(cx);
        let drag = TaskDrag { id: task.id.clone(), title: task.title.clone() };
        let open_id = task.id.clone();
        let agent = self.agent_name(&task.agent_id);
        let selected = self.drawer.as_ref().is_some_and(|drawer| drawer.read(cx).task_id() == task.id);
        let border = if selected { t.running } else if needs_you { amber.opacity(0.6) } else { t.border };
        div()
            .id(SharedString::from(format!("card-{}", task.id)))
            .flex_none()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(metrics::R_INNER))
            .border_1()
            .border_color(border)
            .bg(t.board_card)
            .cursor_pointer()
            .when(!selected, |this| this.hover(move |style| style.border_color(t.text_tertiary)))
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .drag_over::<TaskDrag>(|style, _, _, cx| style.border_color(cx.theme().ring))
            .on_drop(cx.listener(move |this, drag: &TaskDrag, _, cx| {
                cx.stop_propagation();
                let Some(mut placement) = placement.clone() else { return };
                if drag.id == drop_target.id { return }
                placement.sort_order = Some(this.order_before(&drop_target));
                this.move_task(&drag.id, placement, cx)
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.open_drawer(&open_id, window, cx)))
            .child(div().flex().items_start().gap_2()
                .child(div().h(px(18.)).flex().items_center().child(ui::status_icon(&task.status, cx)))
                .child(div().flex_1().min_w_0().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).line_clamp(2).child(task.title.clone()))
                .child(div().h(px(18.)).flex().items_center().child(ui::priority_bars(&task.priority, cx))))
            .when(!task.labels.is_empty(), |this| this.child(
                div().flex().flex_wrap().gap_1().children(task.labels.iter().take(3).map(|label| ui::chip(label.clone(), cx))),
            ))
            .when(needs_you, |this| this.child(
                div().flex().items_center().gap_1p5().t_caption().font_weight(FontWeight::MEDIUM).text_color(amber)
                    .child(Icon::new(IconName::CircleAlert).size(px(14.)))
                    .child(if queued > 0 { format!("{queued} intervention{} queued for the agent", if queued == 1 { "" } else { "s" }) } else { "Blocked: needs you".into() }),
            ))
            .child(
                div().flex().items_center().gap_1p5().t_caption().text_color(t.text_secondary)
                    .child(Icon::new(IconName::Bot).size(px(14.)))
                    .child(div().min_w_0().truncate().child(agent.unwrap_or_else(|| "Unassigned".into())))
                    .child(div().ml_auto().flex_none().font_features(components::typography::tabular()).child(ui::short_date(&task.updated_at))),
            )
    }

    fn render_no_tasks(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        ui::empty_panel(
            "No tasks yet",
            "Tasks you add here are stored by the service. Creating a task never starts an agent.",
            Some(components::button_md(Button::new("first-task").primary().icon(IconName::Plus).label("Add task"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.layout = super::Layout::Board;
                    this.start_adding(TaskStatus::Todo, None, window, cx);
                })).into_any_element()),
            cx,
        )
    }

    pub(super) fn render_list(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        if self.tasks.ready().is_some_and(|all| all.is_empty()) && self.adding.is_none() {
            return self.render_no_tasks(cx).into_any_element();
        }
        div()
            .id("list-scroll")
            .size_full()
            .overflow_y_scroll()
            .px(px(metrics::GUTTER_X))
            .pb_6()
            .flex()
            .flex_col()
            .gap_2()
            .children(ui::statuses_for(&self.tasks.ready().cloned().unwrap_or_default()).iter().map(|status| {
                let group: Vec<&Task> = tasks.iter().filter(|task| &task.status == status).collect();
                self.render_group(status, group, cx)
            }))
            .into_any_element()
    }

    /// Status group (ref 630BC713): 36 header with chevron, ring, label and count.
    fn render_group(&self, status: &TaskStatus, tasks: Vec<&Task>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let collapsed = self.collapsed.contains(status);
        let (toggle, add, drop_status) = (status.clone(), status.clone(), status.clone());
        let adding_here = self.adding.as_ref() == Some(status) && self.adding_stage.is_none();
        let group: SharedString = format!("list-group-{}", status.as_str()).into();
        div()
            .id(SharedString::from(format!("group-{}", status.as_str())))
            .group(group.clone())
            .flex()
            .flex_col()
            .rounded(px(metrics::R_INNER))
            .drag_over::<TaskDrag>(|style, _, _, cx| style.bg(cx.theme().accent))
            .on_drop(cx.listener(move |this, drag: &TaskDrag, _, cx| if drop_status.is_known() { this.move_task(&drag.id, Placement { status: Some(drop_status.clone()), ..Default::default() }, cx) }))
            .child(
                div().id(SharedString::from(format!("group-head-{}", status.as_str()))).flex().items_center().gap_2()
                    .h(px(metrics::ROW_LIST)).pl_2().pr_1p5().rounded(px(metrics::R_CONTROL)).bg(t.tray)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(index) = this.collapsed.iter().position(|item| item == &toggle) {
                            this.collapsed.remove(index);
                        } else {
                            this.collapsed.push(toggle.clone());
                        }
                        cx.notify();
                    }))
                    .child(Icon::new(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown }).size(px(14.)).text_color(t.text_secondary))
                    .child(ui::status_icon(status, cx))
                    .child(div().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child(ui::status_label(status).to_string()))
                    .child(ui::count_pill(tasks.len(), cx))
                    .child(div().ml_auto().invisible().group_hover(group.clone(), |style| style.visible())
                        // Tray-visible hover (`tray_button`): the ghost hover was Δ1 on grey.
                        .child(components::icon_button(Button::new(SharedString::from(format!("list-add-{}", status.as_str())))
                            .custom(tray_button(cx))
                            .icon(IconName::Plus).tooltip("Add task"), metrics::CONTROL_SM)
                            .on_click(cx.listener(move |this, _, window, cx| { cx.stop_propagation(); this.start_adding(add.clone(), None, window, cx) })))),
            )
            .when(!collapsed, |this| {
                this.when(adding_here, |this| this.child(div().pl(px(52.)).py_1().child(components::input_md(Input::new(&self.new_title)))))
                    .children(tasks.into_iter().map(|task| self.render_row(task, cx)))
            })
    }

    /// Task row (ref 630BC713): 40 high. Priority bars sit in the header's chevron slot and the
/// ring under the header ring, so titles share the group label's x.
    fn render_row(&self, task: &Task, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let open_id = task.id.clone();
        let queued = self.queued.get(&task.id).copied().unwrap_or(0);
        let selected = self.drawer.as_ref().is_some_and(|drawer| drawer.read(cx).task_id() == task.id);
        let drag = TaskDrag { id: task.id.clone(), title: task.title.clone() };
        components::list_row(SharedString::from(format!("row-{}", task.id)), selected, cx)
            .h(px(40.))
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .on_click(cx.listener(move |this, _, window, cx| this.open_drawer(&open_id, window, cx)))
            .child(div().w(px(14.)).flex_none().flex().justify_center().child(ui::priority_bars(&task.priority, cx)))
            .child(ui::status_icon(&task.status, cx))
            .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(task.title.clone()))
            .when(queued > 0, |this| this.child(ui::amber_chip(format!("{queued} queued"), cx)))
            .children(task.labels.iter().take(2).map(|label| ui::chip(label.clone(), cx)))
            .child(div().w(px(120.)).flex_none().truncate().t_caption().text_color(t.text_secondary)
                .child(self.agent_name(&task.agent_id).unwrap_or_else(|| "Unassigned".into())))
            .child(div().w(px(52.)).flex_none().text_right().t_caption().font_features(components::typography::tabular())
                .text_color(t.text_secondary).child(ui::short_date(&task.updated_at)))
    }
}

/// Column status mark: stage columns use a filled stage dot (ref 8AEF34E3); status
/// columns use the status ring (ref AE10217A).
fn column_mark(column: &Column, cx: &App) -> AnyElement {
    match &column.filter {
        ColumnFilter::Status(status) => ui::status_icon(status, cx),
        _ => components::status_dot(Tone::Neutral, cx).bg(column.color).into_any_element(),
    }
}

/// Expanded column width (refs E62FEC84 ≈284, AE10217A ≈270).
const COLUMN_W: f32 = 280.;

fn theme_tertiary(cx: &App) -> Hsla {
    theme::tokens(cx).text_tertiary
}

/// Icon buttons on the grey column/group trays: the ghost hover is nearly the tray colour,
/// so hover uses the selected fill (visible on the tray in both themes; the surface colour
/// is darker than the dark tray) and pressed the text colour at low alpha.
fn tray_button(cx: &App) -> ButtonCustomVariant {
    let t = theme::tokens(cx);
    ButtonCustomVariant::new(cx).foreground(t.text_secondary).hover(t.selected).active(t.text.opacity(0.14))
}
