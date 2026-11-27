//! Run activity: a one-line summary, then the recorded stage → agent → tool tree with
//! compact trailing durations. Clicking a row opens its recorded detail inline (transcript,
//! errors, session); timing bars stay behind the "Timeline" toggle.
//!
//! Pure renderer over stored `Run` data: no app state or network, nothing synthesised.
//! A step's lone attempt is folded into the step row; retried steps list every attempt.

mod detail;

use std::collections::HashSet;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{Icon, Selectable as _, Sizable as _, button::{Button, ButtonCustomVariant, ButtonVariants as _}};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{Run, RunStep, RunStepStatus, SpanKind, TraceSpan};

use super::{Tone, icon_button, status_pill, typography::{Typography as _, tabular}};
use crate::theme::{self, metrics};

const ROW_H: f32 = 36.;
const INDENT: f32 = 20.;
const BAR_H: f32 = 6.;
/// Timeline track width when the toggle is on; names keep the rest.
const TRACK_FRACTION: f32 = 0.38;

/// Activity panel for one run. Holds only view state.
pub struct TraceView {
    run: Run,
    collapsed: HashSet<String>,
    selected: Option<String>,
    timeline: bool,
    /// Spans already shown once, so only newly arriving ones get the default collapse.
    seen: HashSet<String>,
    /// Latest events rendered in the open detail; "Show earlier" raises it.
    history: usize,
}

impl TraceView {
    pub fn new(run: Run) -> Self {
        let mut this = Self { run: run.clone(), collapsed: HashSet::new(), selected: None, timeline: false, seen: HashSet::new(), history: detail::HISTORY_PAGE };
        this.set_run(run);
        this
    }

    /// Replaces the record (refresh); drops view state for spans that no longer exist.
    pub fn set_run(&mut self, run: Run) {
        let spans = run.trace_spans.clone().unwrap_or_default();
        let exists = |id: &String| spans.iter().any(|span| &span.id == id);
        self.collapsed.retain(|id| exists(id));
        // Stages start open; agents and attempts start collapsed (tools on demand). Spans
        // seen before keep whatever the user chose.
        for span in &spans {
            if self.seen.insert(span.id.clone()) && !matches!(span.kind, SpanKind::Stage | SpanKind::Tool) {
                self.collapsed.insert(span.id.clone());
            }
        }
        self.seen.retain(|id| exists(id));
        if self.selected.as_ref().is_some_and(|id| !exists(id)) {
            self.selected = None;
        }
        self.run = run;
    }

    fn toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(id) {
            self.collapsed.insert(id.to_owned());
        }
        cx.notify();
    }

    fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.selected = if self.selected == id { None } else { id };
        self.history = detail::HISTORY_PAGE;
        cx.notify();
    }

    /// Selects a span from inside a detail pane, expanding its step so the row is visible.
    fn reveal(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(parent) = self.span(&id).and_then(|span| span.parent_id.clone()) {
            self.collapsed.remove(&parent);
        }
        self.selected = Some(id);
        self.history = detail::HISTORY_PAGE;
        cx.notify();
    }

    fn span(&self, id: &str) -> Option<&TraceSpan> {
        self.run.trace_spans.as_deref().unwrap_or_default().iter().find(|span| span.id == id)
    }

    fn render_summary(&self, spans: &[TraceSpan], cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        let traced = self.run.trace_spans.is_some();
        let tools = spans.iter().filter(|span| span.kind == SpanKind::Tool).count();
        let errors = spans.iter().filter(|span| span.status == RunStepStatus::Failed).count();
        let parts = [
            self.run.duration_ms.or_else(|| traced.then(|| trace_total(spans))).map(format_ms),
            self.run.tokens.map(|n| format!("{} tokens", format_count(n))),
            self.run.cost.map(|cost| format_cost(cost, self.run.cost_currency.as_deref())),
            traced.then(|| format!("{tools} tool {}", if tools == 1 { "call" } else { "calls" })),
        ];
        let summary = parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
        div().flex().items_center().gap_2().pb_2()
            .child(div().flex_none().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child("Activity"))
            .child(div().flex_1().min_w_0().truncate().t_caption().font_features(tabular()).text_color(t.text_secondary).child(summary))
            .when(errors > 0, |this| this.child(div().flex_none().t_caption().text_color(t.danger)
                .child(format!("{errors} {}", if errors == 1 { "error" } else { "errors" }))))
            .when(traced && !spans.is_empty(), |this| this.child(
                Button::new("trace-timeline").small().ghost().t_label().icon(Lucide::ChartGantt).label("Timeline")
                    .selected(self.timeline)
                    .on_click(cx.listener(|this, _, _, cx| { this.timeline = !this.timeline; cx.notify() }))))
            .child(status_pill(state_label(&self.run.state), state_tone(&self.run.state), cx))
    }

    /// Rows and chevrons are Kit buttons, so Tab reaches them and Enter/Space activate.
    fn render_row(&self, row: &Row, extent: f64, cx: &mut Context<Self>) -> Button {
        let t = theme::tokens(cx);
        let span = row.span;
        let selected = self.selected.as_ref() == Some(&span.id);
        let collapsed = self.collapsed.contains(&span.id);
        let (select_id, toggle_id) = (span.id.clone(), span.id.clone());
        let chevron = div().size(px(metrics::CONTROL_XS)).flex_none().flex().items_center().justify_center()
            .when(row.has_children, |this| this.child(
                icon_button(Button::new(SharedString::from(format!("trace-toggle-{}", span.id))).ghost()
                    .icon(if collapsed { Lucide::ChevronRight } else { Lucide::ChevronDown })
                    .aria_expanded(!collapsed).tooltip(if collapsed { "Expand" } else { "Collapse" }), metrics::CONTROL_XS)
                    .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.toggle(&toggle_id, cx) }))));
        let duration = span.duration_ms.map_or_else(|| "running".into(), format_ms);
        let content = div().size_full().min_w_0().flex().items_center().gap_2().pl(px(2. + row.depth as f32 * INDENT)).pr_2()
            .child(chevron)
            .child(Icon::new(kind_icon(&span.kind)).size_4().flex_none().text_color(status_color(&span.status, cx)))
            .child(div().flex_1().min_w_0().truncate()
                .t_label().text_color(if span.kind == SpanKind::Tool { t.text_secondary } else { t.text })
                .when(span.kind == SpanKind::Stage, |this| this.font_weight(FontWeight::MEDIUM))
                .child(span.name.clone()))
            .when(span.status == RunStepStatus::Failed, |this| this.child(
                Icon::new(Lucide::TriangleAlert).size_3p5().flex_none().text_color(t.danger)))
            .when(self.timeline, |this| this.child(bar_track(span, extent, cx)))
            .child(div().flex_none().min_w(px(52.)).text_right().t_caption().font_features(tabular()).text_color(t.text_secondary).child(duration));
        // Kit paints a selected button with its variant's active fill; a light one keeps the
        // selected row quiet (ghost's is a heavy grey in light theme).
        let variant = ButtonCustomVariant::new(cx).color(transparent_black()).foreground(t.text).hover(t.hover).active(t.selected).shadow(false);
        Button::new(SharedString::from(format!("trace-row-{}", span.id))).custom(variant).selected(selected).aria_expanded(selected)
            .w_full().h(px(ROW_H)).p_0().rounded(px(6.))
            .on_click(cx.listener(move |this, _, _, cx| this.select(Some(select_id.clone()), cx)))
            .child(content)
    }
}

impl Render for TraceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let spans = self.run.trace_spans.clone().unwrap_or_default();
        let extent = trace_total(&spans).max(1.);
        let rows = visible_rows(&spans, &attempt_ids(&self.run), &self.collapsed);
        let summary = self.render_summary(&spans, cx);
        let mut body = div().flex().flex_col();
        if rows.is_empty() {
            body = body.child(div().py_2().t_label().text_color(t.text_secondary).child("No activity recorded for this run yet."));
        }
        for row in &rows {
            body = body.child(self.render_row(row, extent, cx));
            if self.selected.as_ref() == Some(&row.span.id) {
                body = body.child(div().pl(px(10. + row.depth as f32 * INDENT)).pt_1().pb_3()
                    .child(self.render_detail(row.span, cx)));
            }
        }
        div().flex().flex_col()
            .child(summary)
            .when(self.timeline, |this| this.child(div().flex().justify_end().pr(px(60.)).t_caption().font_features(tabular())
                .text_color(t.text_tertiary).child(format!("0s – {}", format_ms(extent)))))
            .child(body)
    }
}

/// Bar for the span on a track spanning the recorded trace (no grid; the axis is one caption).
fn bar_track(span: &TraceSpan, extent: f64, cx: &App) -> Div {
    let t = theme::tokens(cx);
    let left = (span.start_offset_ms / extent).clamp(0., 1.);
    // Clamp the length to what remains of the track; `min_w` keeps a sliver visible.
    let width = (span.duration_ms.unwrap_or(extent - span.start_offset_ms) / extent).max(0.).min(1. - left);
    let (left, width) = (left as f32, width as f32);
    div().flex_none().w(relative(TRACK_FRACTION)).h(px(BAR_H)).relative().overflow_hidden().rounded_full().bg(t.section_fill)
        .child(div().absolute().top_0().bottom_0().left(relative(left)).w(relative(width)).min_w(px(3.))
            .rounded_full().bg(status_color(&span.status, cx).opacity(0.7)))
}

struct Row<'a> {
    span: &'a TraceSpan,
    depth: usize,
    has_children: bool,
}

fn attempt_ids(run: &Run) -> HashSet<String> {
    all_steps(run).flat_map(|step| step.attempts.iter().map(|attempt| attempt.id.clone())).collect()
}

pub(super) fn all_steps(run: &Run) -> impl Iterator<Item = &RunStep> {
    let staged: Vec<&RunStep> = run.stages.iter().flatten().flat_map(|stage| stage.steps.iter()).collect();
    let use_flat = staged.is_empty();
    let flat = run.steps.iter().flatten().filter(move |_| use_flat);
    staged.into_iter().chain(flat)
}

/// Displayed children: a step's only attempt is folded away, its tools shown under the step.
fn shown_children<'a>(spans: &'a [TraceSpan], attempts: &HashSet<String>, id: &'a str) -> Vec<&'a TraceSpan> {
    let kids: Vec<_> = children_of(spans, id).collect();
    match kids.as_slice() {
        [only] if attempts.contains(&only.id) => children_of(spans, &only.id).collect(),
        _ => kids,
    }
}

/// Spans in tree order, skipping descendants of collapsed parents. A span whose parent
/// isn't in the trace is a root, so nothing is dropped.
fn visible_rows<'a>(spans: &'a [TraceSpan], attempts: &HashSet<String>, collapsed: &HashSet<String>) -> Vec<Row<'a>> {
    let known = |id: &String| spans.iter().any(|span| &span.id == id);
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    let mut stack: Vec<(&TraceSpan, usize)> = spans.iter().rev()
        .filter(|span| span.parent_id.as_ref().is_none_or(|id| !known(id)))
        .map(|span| (span, 0)).collect();
    while let Some((span, depth)) = stack.pop() {
        if !seen.insert(span.id.as_str()) {
            continue;
        }
        let kids = shown_children(spans, attempts, &span.id);
        out.push(Row { span, depth, has_children: !kids.is_empty() });
        if !collapsed.contains(&span.id) {
            stack.extend(kids.into_iter().rev().map(|child| (child, depth + 1)));
        }
    }
    out
}

fn children_of<'a>(spans: &'a [TraceSpan], id: &'a str) -> impl Iterator<Item = &'a TraceSpan> {
    spans.iter().filter(move |child| child.parent_id.as_deref() == Some(id))
}

fn trace_total(spans: &[TraceSpan]) -> f64 {
    spans.iter().map(|span| span.start_offset_ms + span.duration_ms.unwrap_or(0.)).fold(0., f64::max)
}

fn kind_icon(kind: &SpanKind) -> Lucide {
    match kind {
        SpanKind::Stage => Lucide::Layers,
        SpanKind::Agent => Lucide::Bot,
        SpanKind::Tool => Lucide::Wrench,
        SpanKind::Gate => Lucide::ShieldCheck,
        _ => Lucide::Circle,
    }
}

/// Icons stay neutral unless the span needs attention, so colour only carries state.
fn status_color(status: &RunStepStatus, cx: &App) -> Hsla {
    let t = theme::tokens(cx);
    match status {
        RunStepStatus::Failed => t.danger,
        RunStepStatus::NeedsApproval => t.attention,
        RunStepStatus::Running | RunStepStatus::Pending => t.running,
        _ => t.text_secondary,
    }
}

fn span_status_label(status: &RunStepStatus) -> String {
    match status {
        RunStepStatus::Pending => "Pending".into(),
        RunStepStatus::Running => "Running".into(),
        RunStepStatus::Completed => "Done".into(),
        RunStepStatus::Failed => "Failed".into(),
        RunStepStatus::NeedsApproval => "Needs approval".into(),
        RunStepStatus::Cancelled => "Cancelled".into(),
        RunStepStatus::Skipped => "Skipped".into(),
        RunStepStatus::Unknown(raw) => raw.clone(),
    }
}

fn span_tone(status: &RunStepStatus) -> Tone {
    match status {
        RunStepStatus::Completed => Tone::Success,
        RunStepStatus::Running => Tone::Running,
        RunStepStatus::Failed => Tone::Danger,
        RunStepStatus::NeedsApproval => Tone::Attention,
        _ => Tone::Neutral,
    }
}

/// Run state is a free string; map the common words and show anything else verbatim.
fn state_label(state: &str) -> String {
    match state {
        "completed" | "done" => "Done".into(),
        "running" => "Running".into(),
        "failed" => "Failed".into(),
        "pending" | "queued" => "Pending".into(),
        other => other.to_owned(),
    }
}

fn state_tone(state: &str) -> Tone {
    match state {
        "completed" | "done" => Tone::Success,
        "running" => Tone::Running,
        "failed" => Tone::Danger,
        "needs_approval" => Tone::Attention,
        _ => Tone::Neutral,
    }
}

fn format_ms(ms: f64) -> String {
    if ms < 1000. {
        format!("{ms:.0}ms")
    } else if ms < 60_000. {
        let seconds = ms / 1000.;
        if seconds < 10. { format!("{seconds:.1}s") } else { format!("{seconds:.0}s") }
    } else {
        let (minutes, seconds) = ((ms / 60_000.).floor(), ((ms % 60_000.) / 1000.).floor());
        if seconds == 0. { format!("{minutes:.0}m") } else { format!("{minutes:.0}m {seconds:.0}s") }
    }
}

fn format_count(count: u64) -> String {
    match count {
        1_000_000.. => format!("{:.1}M", count as f64 / 1_000_000.),
        1_000.. => format!("{:.1}k", count as f64 / 1000.),
        _ => count.to_string(),
    }
}

fn format_cost(cost: f64, currency: Option<&str>) -> String {
    match currency {
        None | Some("USD") => format!("${cost:.2}"),
        Some(code) => format!("{cost:.2} {code}"),
    }
}
