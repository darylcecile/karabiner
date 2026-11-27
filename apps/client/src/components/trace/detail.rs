//! Inline detail for a selected trace row, read straight from the stored run: the attempt's
//! structured events when recorded, else its transcript text, plus errors and session.
//! Anything the record lacks is stated as absent rather than inferred.

use gpui_kit::component::{Selectable as _, Sizable as _, button::{Button, ButtonVariants as _}};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{RunActivityEvent, RunError, RunStep, RunStepAttempt, SpanKind, TraceSpan};

use super::{TraceView, all_steps, format_cost, format_count, span_status_label, span_tone};
use crate::components::{icon_button, status_pill, typography::{Typography as _, tabular}};
use crate::theme::{self, metrics};
use gpui_kit::assets::IconName as Lucide;

const DETAIL_MAX_H: f32 = 420.;
const CODE_MAX_H: f32 = 200.;
/// Events rendered per page of an attempt's stream (newest first in, older on request).
pub(super) const HISTORY_PAGE: usize = 30;
/// Transcript tail kept on screen; the service already bounds the stored text.
const TAIL_CHARS: usize = 24_000;

enum Subject<'a> {
    Step(&'a RunStep),
    Attempt(&'a RunStep, &'a RunStepAttempt),
    Tool(Option<(&'a RunStep, &'a RunStepAttempt)>, Option<&'a RunActivityEvent>),
    Other,
}

impl TraceView {
    fn subject<'a>(&'a self, span: &'a TraceSpan) -> Subject<'a> {
        let find_attempt = |id: &str| all_steps(&self.run).find_map(|step|
            step.attempts.iter().find(|attempt| attempt.id == id).map(|attempt| (step, attempt)));
        if span.kind == SpanKind::Tool {
            let owner = span.parent_id.as_deref().and_then(find_attempt);
            let tool_id = span.parent_id.as_deref().and_then(|parent| span.id.strip_prefix(parent)?.strip_prefix(':'));
            let event = owner.zip(tool_id).and_then(|((_, attempt), tool)| attempt.events.iter()
                .filter(|event| event.kind == "tool").rfind(|event| event.tool_id.as_deref() == Some(tool)));
            return Subject::Tool(owner, event);
        }
        if let Some(step) = all_steps(&self.run).find(|step| step.id == span.id) {
            return step.attempts.last().map_or(Subject::Step(step), |latest| Subject::Attempt(step, latest));
        }
        find_attempt(&span.id).map_or(Subject::Other, |(step, attempt)| Subject::Attempt(step, attempt))
    }

    /// Header and attempt navigation stay outside the bounded scroller, so a long stream
    /// never hides Close or the other attempts.
    pub(super) fn render_detail(&self, span: &TraceSpan, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let subject = self.subject(span);
        let nav = match &subject {
            Subject::Attempt(step, attempt) if step.attempts.len() > 1 => Some(self.attempt_nav(step, Some(attempt.id.as_str()), cx)),
            _ => None,
        };
        let body: Vec<AnyElement> = match subject {
            Subject::Attempt(step, attempt) => self.attempt_body(step, attempt, cx),
            Subject::Step(step) => step_body(step, cx),
            Subject::Tool(owner, event) => self.tool_body(owner, event, cx),
            Subject::Other => vec![note("Select an agent or tool to see its recorded activity.", cx)],
        };
        div().flex().flex_col().gap_2().pl_3().border_l_1().border_color(t.border)
            .child(div().flex_none().flex().items_start().gap_2()
                .child(div().flex_1().min_w_0().whitespace_normal().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child(span.name.clone()))
                .child(status_pill(span_status_label(&span.status), span_tone(&span.status), cx))
                .child(icon_button(Button::new("trace-detail-close").ghost().icon(Lucide::X).tooltip("Close"), metrics::CONTROL_SM)
                    .on_click(cx.listener(|this, _, _, cx| { cx.stop_propagation(); this.select(None, cx) }))))
            .children(nav)
            .when(!body.is_empty(), |this| this.child(div().id(SharedString::from(format!("trace-detail-{}", span.id)))
                .max_h(px(DETAIL_MAX_H)).min_h_0().overflow_y_scroll()
                .child(div().flex_none().flex().flex_col().gap_3().pr_2().children(body))))
            .into_any_element()
    }

    /// Every attempt of a retried step as a compact chip row; the current one is marked.
    fn attempt_nav(&self, step: &RunStep, current: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        div().flex_none().flex().flex_wrap().items_center().gap_1()
            .children(step.attempts.iter().map(|attempt| {
                let id = attempt.id.clone();
                let label = match &attempt.error { Some(_) => format!("Attempt {} · failed", attempt.number), None => format!("Attempt {} · {}", attempt.number, attempt.status) };
                Button::new(SharedString::from(format!("trace-attempt-{id}"))).small().ghost().t_label()
                    .selected(current == Some(id.as_str())).label(label)
                    .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.reveal(id.clone(), cx) }))
            }))
            .into_any_element()
    }

    fn attempt_body(&self, step: &RunStep, attempt: &RunStepAttempt, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let facts = [
            attempt.tokens.map(|n| format!("{} tokens", format_count(n))),
            attempt.cost.map(|cost| format_cost(cost, None)),
            attempt.session_id.as_ref().or(step.session_id.as_ref().filter(|_| step.attempts.len() == 1)).map(|id| format!("Session {id}")),
        ];
        let facts = facts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let mut out: Vec<AnyElement> = Vec::new();
        // With events, `output` is the attempt's final summary, distinct from the stream; it leads.
        if !attempt.events.is_empty() && let Some(result) = attempt.output.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
            out.push(labelled("Result", prose(result, cx), cx));
        }
        if !facts.is_empty() { out.push(caption(facts, cx)) }
        out.extend(errors(attempt.error.as_ref(), cx));
        if step.attempts.len() == 1 { out.extend(step_status(step, cx)); }
        if attempt.events.is_empty() {
            out.push(transcript(&attempt.id, attempt.output.as_deref().or(step.output.as_deref().filter(|_| step.attempts.len() == 1)),
                "No activity recorded for this attempt.", cx));
            return out;
        }
        // Only the latest events render; earlier ones stay one click away, never dropped.
        let hidden = attempt.events.len().saturating_sub(self.history);
        if hidden > 0 {
            out.push(div().flex().items_center().gap_2()
                .child(Button::new("trace-earlier").small().ghost().t_label().label(format!("Show {} earlier", hidden.min(HISTORY_PAGE)))
                    .on_click(cx.listener(|this, _, _, cx| { cx.stop_propagation(); this.history += HISTORY_PAGE; cx.notify() })))
                .child(Button::new("trace-all").small().ghost().t_label().label(format!("Show all {}", attempt.events.len()))
                    .on_click(cx.listener(|this, _, _, cx| { cx.stop_propagation(); this.history = usize::MAX; cx.notify() })))
                .into_any_element());
        }
        out.extend(attempt.events.iter().enumerate().skip(hidden).flat_map(|(index, event)| event_rows(&format!("{}-{index}", attempt.id), event, cx)));
        out
    }

    fn tool_body(&self, owner: Option<(&RunStep, &RunStepAttempt)>, event: Option<&RunActivityEvent>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = match event {
            Some(event) => tool_event("tool", event, cx),
            None => vec![note("This run recorded the tool's name and timing only; its input and output weren't stored.", cx)],
        };
        if let Some((step, attempt)) = owner {
            // A folded single attempt is shown on its step's row.
            let target = if step.attempts.len() == 1 { step.id.clone() } else { attempt.id.clone() };
            out.push(div().child(Button::new("trace-open-agent").small().ghost().t_label().label("Open agent activity")
                .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.reveal(target.clone(), cx) })))
                .into_any_element());
        }
        out
    }
}

/// A step with no recorded attempts: its status, errors and any step-level output.
fn step_body(step: &RunStep, cx: &App) -> Vec<AnyElement> {
    let mut out = step_status(step, cx);
    if step.attempts.is_empty() {
        out.extend(errors(step.error.as_ref(), cx));
        out.push(transcript(&step.id, step.output.as_deref(), "No activity recorded for this step.", cx));
    }
    out
}
fn event_rows(key: &str, event: &RunActivityEvent, cx: &App) -> Vec<AnyElement> {
    let t = theme::tokens(cx);
    let text = event.text.clone().unwrap_or_default();
    match event.kind.as_str() {
        "tool" => tool_event(key, event, cx),
        "reasoning" => vec![labelled("Reasoning", div().t_body().italic().whitespace_normal().text_color(t.text_secondary).child(text), cx)],
        "error" => vec![div().t_body().whitespace_normal().text_color(t.danger).child(text).into_any_element()],
        _ => vec![prose(&text, cx).into_any_element()],
    }
}

fn tool_event(key: &str, event: &RunActivityEvent, cx: &App) -> Vec<AnyElement> {
    let t = theme::tokens(cx);
    let name = event.name.clone().or_else(|| event.text.clone()).unwrap_or_else(|| "Tool".into());
    let mut out = vec![div().flex().items_center().gap_2().t_label().text_color(t.text)
        .child(gpui_kit::component::Icon::new(Lucide::Wrench).size_3p5().text_color(t.text_secondary)).child(name).into_any_element()];
    for (label, value) in [("Input", &event.input), ("Output", &event.output)] {
        if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
            out.push(labelled(label, code_block(&format!("{key}-{label}"), value, cx), cx));
        }
    }
    out
}

/// The step's live activity line and a scheduled retry, which the rail used to show.
fn step_status(step: &RunStep, cx: &App) -> Vec<AnyElement> {
    let parts = [step.activity.clone(), step.retry_at.as_ref().map(|_| "Retry scheduled".to_string())];
    let text = parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
    (!text.is_empty()).then(|| caption(text, cx)).into_iter().collect()
}

fn errors(error: Option<&RunError>, cx: &App) -> Vec<AnyElement> {
    let t = theme::tokens(cx);
    error.map(|error| div().t_body().text_color(t.danger).child(error.message.clone()).into_any_element()).into_iter().collect()
}

fn transcript(key: &str, output: Option<&str>, empty: &str, cx: &App) -> AnyElement {
    match output.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => {
            let chars = text.chars().count();
            let tail: String = text.chars().skip(chars.saturating_sub(TAIL_CHARS)).collect();
            let title = if chars > TAIL_CHARS { "Transcript (latest part)" } else { "Transcript" };
            labelled(title, code_block(&format!("{key}-transcript"), &tail, cx), cx)
        }
        None => note(empty, cx),
    }
}

/// Exact recorded text in one element: lines keep their breaks and long ones scroll
/// sideways inside a bounded box instead of wrapping across the pane.
fn code_block(key: &str, text: &str, cx: &App) -> Stateful<Div> {
    let t = theme::tokens(cx);
    div().id(SharedString::from(format!("trace-code-{key}"))).max_h(px(CODE_MAX_H)).overflow_scroll().flex().items_start()
        .rounded(px(metrics::R_INNER)).bg(t.section_fill).px_2().py_1p5()
        .child(div().flex_none().whitespace_nowrap().t_code(cx).text_color(t.text).child(text.to_string()))
}

fn prose(text: &str, cx: &App) -> Div {
    div().t_body().whitespace_normal().text_color(theme::tokens(cx).text).child(text.to_string())
}

fn labelled(label: &str, content: impl IntoElement, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    div().flex().flex_col().gap_1()
        .child(div().t_caption().font_weight(FontWeight::MEDIUM).text_color(t.text_secondary).child(label.to_string()))
        .child(content).into_any_element()
}

fn caption(text: String, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    div().t_caption().font_features(tabular()).text_color(t.text_secondary).child(text).into_any_element()
}

fn note(text: &str, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    div().t_body().text_color(t.text_secondary).child(text.to_string()).into_any_element()
}
