//! Run step tracker row (ref B457FF32): rail glyph, title, chevron, status word and
//! elapsed time; a revealed detail rail; and an inline approval panel. Pure renderer: the
//! caller owns expand state and handles the toggle and approve/decline callbacks.

use std::rc::Rc;

use gpui_kit::base::motion::Presence;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    spinner::Spinner,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Tone, Typography as _};
use crate::{motion, theme::{self, metrics}};

/// State of one step on the rail.
#[derive(Clone, Copy, PartialEq)]
pub enum StepState {
    Done,
    Running,
    NeedsYou,
    Failed,
    Pending,
    /// A configured stage with no recorded step data.
    NotRecorded,
}

pub struct Step {
    pub id: String,
    pub title: String,
    pub agent: Option<String>,
    pub state: StepState,
    /// Status word shown on the right (raw literal for unknown statuses).
    pub verb: String,
    pub elapsed: Option<String>,
    pub detail: Vec<(String, String)>,
    pub request: Option<String>,
}

/// The answer chosen in a step's approval panel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StepAnswer {
    Approve,
    Decline,
}

/// Glyph column geometry: the row's `px_1` inset plus half the 14pt glyph.
const GLYPH: f32 = 14.;
const ROW_H: f32 = 32.;
const RAIL_X: f32 = 4. + GLYPH / 2. - 0.5;
/// Detail and approval content align with the title (inset + glyph + gap).
const CONTENT_X: f32 = 4. + GLYPH + 8.;

/// One step row. `key` must be unique among visible steps (element ids and the reveal
/// animation derive from it). `last` ends the rail at this step's glyph. `on_answer`
/// receives the choice and the request text.
pub fn run_step(
    key: &str,
    step: Step,
    expanded: bool,
    last: bool,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
    on_answer: impl Fn(StepAnswer, String, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let blue = theme::tokens(cx).running;
    let amber = theme::tokens(cx).attention;
    let muted = cx.theme().muted_foreground;
    let verb_color = match step.state {
        StepState::Running => blue,
        StepState::NeedsYou => amber,
        StepState::Failed => theme::tokens(cx).danger,
        _ => muted,
    };
    let reveal = Presence::new(format!("step-reveal-{key}"), expanded).transition(motion::standard()).sample(window, cx);
    let hover = cx.theme().secondary;
    let rail = cx.theme().border;
    div().relative().flex().flex_col()
        // Rail segment from this glyph's bottom edge to the next glyph's top edge.
        .when(!last, |this| this.child(div().absolute().left(px(RAIL_X)).w(px(1.)).bg(rail)
            .top(px((ROW_H + GLYPH) / 2.)).bottom(px(-(ROW_H - GLYPH) / 2.))))
        .child(
            div().id(SharedString::from(format!("step-{key}"))).flex().items_center().gap_2().h(px(ROW_H)).px_1().rounded(px(metrics::R_CONTROL))
                .cursor_pointer().hover(move |style| style.bg(hover))
                .on_click(move |_, window, cx| on_toggle(window, cx))
                .child(step_glyph(step.state, cx))
                .child(div().t_label().when(matches!(step.state, StepState::Pending | StepState::NotRecorded), |this| this.text_color(muted)).child(step.title.clone()))
                .child(Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.)).text_color(muted))
                .child(div().ml_auto().flex().gap_2().t_caption()
                    .child(div().text_color(verb_color).child(step.verb.clone()))
                    .children(step.elapsed.clone().map(|elapsed| div().text_color(muted).child(elapsed)))),
        )
        .when(reveal.should_render(), |this| {
            let progress = reveal.progress;
            let empty = step.detail.is_empty().then(|| empty_detail(step.agent.is_some()));
            this.child(div().ml(px(CONTENT_X)).opacity(progress).mt(px((progress - 1.) * 6.)).flex().flex_col().gap_0p5().py_1()
                .children(step.agent.clone().map(|agent| div().t_caption().text_color(muted).child(agent)))
                .children(empty.map(|text| div().t_caption().text_color(muted).child(text)))
                .children(step.detail.iter().map(|(label, meta)| {
                    div().flex().justify_between().t_caption()
                        .child(div().child(label.clone()))
                        .child(div().text_color(muted).child(meta.clone()))
                })))
        })
        .children(step.request.clone().map(|request| {
            let on_answer = Rc::new(on_answer);
            let (approve, decline) = (on_answer.clone(), on_answer);
            let (approve_text, decline_text) = (request.clone(), request.clone());
            div().ml(px(CONTENT_X)).mt_1().p_3().rounded(px(metrics::R_INNER)).border_1().border_color(amber.opacity(0.4)).bg(Tone::Attention.tint(cx, 0.10))
                .flex().flex_col().gap_2()
                .child(div().t_label().child(request))
                .child(div().flex().justify_end().gap_1()
                    .child(Button::new(SharedString::from(format!("decline-{key}"))).ghost().small().t_label().label("Decline")
                        .on_click(move |_, window, cx| decline(StepAnswer::Decline, decline_text.clone(), window, cx)))
                    .child(Button::new(SharedString::from(format!("approve-{key}"))).primary().small().t_label().label("Approve")
                        .on_click(move |_, window, cx| approve(StepAnswer::Approve, approve_text.clone(), window, cx))))
        }))
}

/// Empty-detail line that never contradicts a known agent.
fn empty_detail(has_agent: bool) -> &'static str {
    if has_agent { "No step details recorded." } else { "No agents assigned to this stage." }
}

/// Rail glyph: check disc, spinning arc (static under Reduce Motion), amber dashed ring, warning triangle, hollow rings.
pub fn step_glyph(state: StepState, cx: &App) -> impl IntoElement {
    let t = theme::tokens(cx);
    let base = div().size(px(14.)).flex_none().rounded_full().flex().items_center().justify_center();
    match state {
        StepState::Done => base.bg(t.running).child(Icon::new(IconName::Check).size(px(9.)).text_color(cx.theme().background)),
        StepState::Running => base.child(Spinner::new().icon(Icon::new(IconName::LoaderCircle)).with_size(px(GLYPH)).color(t.running)),
        StepState::NeedsYou => base.border(px(1.5)).border_dashed().border_color(t.attention),
        // A failure reads as a warning (ref F263F6D3 failed rows), not a cancel ×.
        StepState::Failed => base.child(Icon::new(gpui_kit::assets::IconName::TriangleAlert).size(px(14.)).text_color(t.danger)),
        StepState::Pending => base.border(px(1.5)).border_dashed().border_color(cx.theme().muted_foreground),
        // Tertiary, not `border`: the row hover fill would hide a border-coloured ring.
        StepState::NotRecorded => base.border(px(1.5)).border_color(t.text_tertiary),
    }
}
