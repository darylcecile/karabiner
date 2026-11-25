//! Left rail: the stage palette and the project's agents. Both drag onto the canvas (a stage
//! where it lands, an agent onto a stage); click adds/assigns for keyboard and trackpad users.

use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::ProjectConfig;

use super::FactoryPane;
use crate::components::factory_canvas::{LibraryDrag, LibraryItem};
use crate::components::{self, Typography as _};
use crate::theme::{self, metrics};
use crate::views::tasks::ui;

pub const LIBRARY_W: f32 = 220.;

/// Ghost shown under the pointer while dragging a library item.
pub struct DragGhost(SharedString);

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        div().px_2().h(px(metrics::CONTROL_MD)).flex().items_center().rounded(px(metrics::R_CONTROL)).border_1().border_color(t.border)
            .bg(t.surface).shadow_md().t_label().text_color(t.text).child(self.0.clone())
    }
}

fn item(id: SharedString, icon: impl Into<Icon>, title: String, meta: String, payload: LibraryItem, cx: &App) -> Stateful<Div> {
    let t = theme::tokens(cx);
    let drag = LibraryDrag { item: payload, label: title.clone().into() };
    div().id(id).flex().items_center().gap_2().h(px(metrics::ROW_LIST)).px_2().rounded(px(metrics::R_CONTROL)).cursor_grab()
        .hover(move |style| style.bg(t.hover))
        .on_drag(drag, |drag, _, _, cx| { let label = drag.label.clone(); cx.new(|_| DragGhost(label)) })
        .child(Icon::new(icon).size(px(16.)).text_color(t.text_secondary))
        .child(div().flex_1().min_w_0().flex().flex_col()
            .child(div().t_label().truncate().text_color(t.text).child(title))
            .when(!meta.is_empty(), |this| this.child(div().t_caption().truncate().text_color(t.text_secondary).child(meta))))
}

impl FactoryPane {
    pub(super) fn render_library(&self, config: &ProjectConfig, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let stage = self.selected_stage_id().map(str::to_string);
        let parallel = self.state.service.read(cx).capabilities().parallel_execution;
        div().id("factory-library").w(px(LIBRARY_W)).flex_none().h_full().overflow_y_scroll().border_r_1().border_color(t.border).bg(t.canvas)
            .p_2().flex().flex_col().gap_0p5()
            .child(components::group_label("Nodes", cx))
            .child(item("palette-stage".into(), IconName::Plus, "Stage".into(), "Agents, hooks and gates".into(), LibraryItem::Stage, cx)
                .on_click(cx.listener(|this, _, window, cx| this.add_stage(None, window, cx))))
            .when(!parallel, |this| this.child(div().px_2().child(ui::update_service_note("run stages in parallel", cx))))
            .when(parallel, |this| this.child(item("palette-parallel".into(), gpui_kit::assets::IconName::GitFork, "Parallel".into(), "Fork into branches, then join".into(), LibraryItem::Fork, cx)
                .on_click(cx.listener(|this, _, window, cx| this.add_parallel(None, window, cx))))
            .child(item("palette-join".into(), gpui_kit::assets::IconName::GitMerge, "Join".into(), "New join for the selected fork".into(), LibraryItem::Join, cx)
                .on_click(cx.listener(|this, _, window, cx| {
                    let at = super::layout(this.canvas.read(cx).view_center());
                    this.add_join(at, window, cx)
                }))))
            .child(div().pt_3().child(components::group_label("Agents", cx)))
            .when(config.agents.is_empty(), |this| this.child(div().px_2().t_caption().text_color(t.text_secondary)
                .child("No agents yet. Add them in the project's Settings → Agents.")))
            .children(config.agents.iter().map(|agent| {
                let agent_id = agent.id.clone();
                let stage = stage.clone();
                item(SharedString::from(format!("library-agent-{}", agent.id)), IconName::Bot, agent.name.clone(),
                    crate::views::workflow::join_parts(&[&agent.role, ui::harness_label(&agent.harness)]), LibraryItem::Agent(agent.id.clone().into()), cx)
                    .on_click(cx.listener(move |this, _, window, cx| match &stage {
                        Some(stage) => this.assign_agent(stage, &agent_id, window, cx),
                        None => crate::components::toast_warning("Select a stage, or drag the agent onto one.", window, cx),
                    }))
            }))
            .child(div().pt_3().px_2().t_caption().text_color(t.text_secondary)
                .child(if stage.is_some() { "Click an agent to add it to the selected stage." } else { "Drag onto the canvas, or select a stage and click an agent." }))
    }
}
