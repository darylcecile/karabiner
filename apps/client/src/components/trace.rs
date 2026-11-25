//! Run trace (ref `F263F6D3`, CCCF2D3A): header with status, six stat cells, and a span
//! tree on a time grid with bars, a SLOWEST tag and an anchored detail popover.
//!
//! Pure renderer over stored `Run` data: no app state or network. Values the record doesn't
//! carry render "—". The trace has no LLM-call kind, so the cell counts agent spans instead.

use std::collections::HashSet;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{Icon, button::{Button, ButtonVariants as _}, tooltip::Tooltip};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{Run, RunStepStatus, SpanKind, TraceSpan};

use super::{Tone, icon_button, status_pill, typography::{Typography as _, tabular}};
use crate::theme::{self, metrics};

// Measured on boards/K15-trace.png (F263F6D3 t16 at ~2.37 px/pt, 38pt rows): the span
// column is ~320pt with 16pt names, 15pt kinds and ~20pt stat values. The column takes
// 40% of the card up to that width, so a 720pt trace still keeps ~430pt of timeline.
const SPAN_COL_MAX_W: f32 = 320.;
const SPAN_COL_FRACTION: f32 = 0.4;
const HEADER_H: f32 = 52.;
const RULER_H: f32 = 36.;
const ROW_H: f32 = 38.;
const BAR_H: f32 = 16.;
const INDENT: f32 = 16.;
const TICKS: usize = 4;
const POPOVER_W: f32 = 340.;
/// Three stat cells; below twice this the six-cell strip wraps to two rows.
const STATS_HALF_W: f32 = 360.;

/// Trace panel for one run. Holds only view state (collapsed parents, selected span).
pub struct TraceView {
    run: Run,
    collapsed: HashSet<String>,
    selected: Option<String>,
}

impl TraceView {
    pub fn new(run: Run) -> Self {
        Self { run, collapsed: HashSet::new(), selected: None }
    }

    /// Replaces the record (refresh); drops view state for spans that no longer exist.
    pub fn set_run(&mut self, run: Run) {
        let spans = run.trace_spans.clone().unwrap_or_default();
        let exists = |id: &String| spans.iter().any(|span| &span.id == id);
        self.collapsed.retain(|id| exists(id));
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
        cx.notify();
    }

    fn render_header(&self, cx: &App) -> Div {
        let t = theme::tokens(cx);
        div().flex().items_center().gap_2().h(px(HEADER_H)).pl_4().pr(px((HEADER_H - metrics::CHIP_H) / 2.))
            .border_b_1().border_color(t.border)
            .child(Icon::new(Lucide::Sparkles).size_4().flex_none().text_color(t.running))
            .child(div().flex_none().t_headline().text_color(t.text).child("Trace"))
            .child(div().flex_1().min_w_0().truncate().t_body().text_color(t.text_secondary)
                .child(format!("Run {}", self.run.id.chars().take(8).collect::<String>())))
            .child(status_pill(state_label(&self.run.state), state_tone(&self.run.state), cx))
    }

    fn render_stats(&self, spans: &[TraceSpan], cx: &App) -> Div {
        let t = theme::tokens(cx);
        let traced = self.run.trace_spans.is_some();
        let count = |kind: SpanKind| traced.then(|| spans.iter().filter(|span| span.kind == kind).count().to_string());
        let errors = traced.then(|| spans.iter().filter(|span| span.status == RunStepStatus::Failed).count());
        let cells = [
            ("Duration", self.run.duration_ms.or_else(|| traced.then(|| trace_total(spans))).map(format_ms), false),
            ("Cost", self.run.cost.map(|cost| format_cost(cost, self.run.cost_currency.as_deref())), false),
            ("Tokens", self.run.tokens.map(format_count), false),
            ("Agent spans", count(SpanKind::Agent), false),
            ("Tool calls", count(SpanKind::Tool), false),
            ("Errors", errors.map(|n| n.to_string()), errors.is_some_and(|n| n > 0)),
        ];
        // Two halves of three cells: one strip when wide, two rows when narrow. Every cell
        // draws its left divider; at the card edge it sits under the card's border overlay.
        let [a, b, c, d, e, f] = cells.map(|cell| stat_cell(cell, cx));
        let half = |cells: [Div; 3]| div().flex_grow(1.).flex_basis(px(STATS_HALF_W)).flex().border_b_1().border_color(t.border).children(cells);
        div().flex().flex_wrap().child(half([a, b, c])).child(half([d, e, f]))
    }

    fn render_ruler(&self, total: f64, cx: &App) -> Div {
        let t = theme::tokens(cx);
        div().flex().h(px(RULER_H)).border_b_1().border_color(t.border).t_body().text_color(t.text_secondary)
            .child(span_col(div()).px_4().flex().items_center().border_r_1().border_color(t.border)
                .font_weight(FontWeight::MEDIUM).child("SPAN"))
            .child(div().flex_1().flex().children((0..TICKS).map(|index| {
                div().flex_1().h_full().px_2().flex().items_center().font_features(tabular())
                    .when(index > 0, |this| this.border_l_1().border_color(t.border))
                    .child(format_tick(total * index as f64 / TICKS as f64))
            })))
    }

    /// `extent` is the axis length; `end` is where the recorded trace ends (running spans
    /// draw to it). The popover, when this row is selected, anchors to the row's bottom.
    fn render_row(&self, row: &Row, extent: f64, end: f64, slowest: bool, last: bool, popover: Option<AnyElement>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let span = row.span;
        let selected = self.selected.as_ref() == Some(&span.id);
        let collapsed = self.collapsed.contains(&span.id);
        let left = (span.start_offset_ms / extent).clamp(0., 1.) as f32;
        let width = (span.duration_ms.unwrap_or(end - span.start_offset_ms) / extent).clamp(0.005, 1.) as f32;
        let palette = bar_palette(span, row.has_children, cx);
        let select_id = span.id.clone();
        let toggle_id = span.id.clone();
        let name = SharedString::from(span.name.clone());
        let chevron = div().size(px(16.)).flex_none().flex().items_center().justify_center()
            .when(row.has_children, |this| this.child(
                div().id(SharedString::from(format!("trace-toggle-{}", span.id))).size(px(16.)).flex().items_center().justify_center()
                    .rounded(px(4.)).cursor_pointer().hover(move |style| style.bg(t.hover))
                    .child(Icon::new(if collapsed { Lucide::ChevronRight } else { Lucide::ChevronDown }).size_3p5().text_color(t.text_secondary))
                    .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.toggle(&toggle_id, cx) }))));
        div().id(SharedString::from(format!("trace-row-{}", span.id))).relative().flex().h(px(ROW_H)).cursor_pointer()
            // The last row meets the card's bottom edge: its fill takes the card's own
            // radius, and the card's border overlay paints the single edge on top.
            .when(last, |this| this.rounded_b(px(metrics::R_CARD)))
            .when(selected, |this| this.bg(Tone::Running.tint(cx, 0.08)))
            .when(!selected, |this| this.hover(move |style| style.bg(t.hover)))
            .on_click(cx.listener(move |this, _, _, cx| this.select(Some(select_id.clone()), cx)))
            .child(span_col(div()).relative().h_full().pr_3().pl(px(8. + row.depth as f32 * INDENT)).flex().items_center().gap_2()
                .border_r_1().border_color(t.border)
                .children(tree_guides(row, !collapsed, cx))
                .child(chevron)
                .child(kind_tile(&span.kind, cx))
                .child(div().id(SharedString::from(format!("trace-name-{}", span.id))).min_w_0().truncate().t_trace_label().text_color(t.text)
                    .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
                    .child(span.name.clone()))
                .child(div().flex_none().t_body().text_color(t.text_secondary).child(kind_label(&span.kind)))
                .when(span.status == RunStepStatus::Failed, |this| this.child(
                    Icon::new(Lucide::TriangleAlert).size_3p5().flex_none().text_color(t.danger))))
            // Bars share the grid's coordinates (no padding). The axis keeps at least a
            // quarter of headroom past the trace end, and labels truncate inside the cell.
            .child(div().flex_1().min_w_0().h_full().relative().overflow_hidden()
                .child(div().absolute().top_0().bottom_0().left(relative(left)).right_0().pr_2().flex().items_center().gap_2()
                    .child(div().w(relative(width / (1. - left).max(0.005))).min_w(px(6.)).h(px(BAR_H)).flex_none()
                        .rounded(px(4.)).border_1().border_color(palette.border).bg(palette.fill))
                    .child(div().flex_1().min_w_0().flex().items_center().gap_2()
                        .child(div().min_w_0().truncate().t_body().font_features(tabular()).text_color(t.text_secondary)
                            .child(span.duration_ms.map_or_else(|| "running".into(), format_ms)))
                        .when(slowest, |this| this.child(
                            div().flex_none().h(px(metrics::CHIP_H)).px_1p5().flex().items_center().rounded(px(metrics::R_CONTROL))
                                .bg(Tone::Attention.tint(cx, 0.15)).t_caption().font_weight(FontWeight::MEDIUM)
                                .text_color(Tone::Attention.color(cx)).child("SLOWEST"))))))
            // Zero-size anchor at the row's bottom-left of the timeline; the popup snaps
            // inside the window with an 8pt margin, as Kit's menus do.
            .children(popover.map(|popover| div().absolute().top(px(ROW_H + 4.)).left_0().right_0().flex()
                .child(span_col(div())).child(div().ml_4().size_0().child(popover))))
    }

    fn render_popover(&self, span: &TraceSpan, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let fields = [
            ("Duration", span.duration_ms.map_or_else(|| "Running".into(), format_ms)),
            ("Started", format!("+{}", format_ms(span.start_offset_ms))),
            ("Kind", kind_label(&span.kind)),
        ];
        deferred(anchored().anchor(Anchor::TopLeft).snap_to_window_with_margin(px(8.)).child(
            // Occlusion blocks hitboxes beneath; stopping here keeps clicks inside the card
            // from bubbling to the row it hangs from.
            div().id("trace-popover").w(px(POPOVER_W)).occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(|_, _, cx| cx.stop_propagation())
                .flex().flex_col().gap_3().p_4().rounded(px(metrics::R_POPOVER)).border_1().border_color(t.border).bg(t.surface).shadow_lg()
                .child(div().flex().items_start().gap_3()
                    .child(kind_tile(&span.kind, cx).size(px(32.)))
                    .child(div().flex_1().min_w_0().flex().flex_col()
                        .child(div().truncate().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child(span.name.clone()))
                        .child(div().t_caption().text_color(t.text_secondary).child(kind_label(&span.kind))))
                    .child(status_pill(span_status_label(&span.status), span_tone(&span.status), cx))
                    .child(icon_button(Button::new("trace-popover-close").ghost().icon(Lucide::X).tooltip("Close"), metrics::CONTROL_SM)
                        .on_click(cx.listener(|this, _, _, cx| { cx.stop_propagation(); this.select(None, cx) }))))
                .child(div().flex().gap_4().p_3().rounded(px(metrics::R_INNER)).bg(t.section_fill).children(fields.into_iter().map(|(label, value)| {
                    div().flex_1().min_w_0().flex().flex_col().gap_1()
                        .child(div().t_caption().font_weight(FontWeight::MEDIUM).text_color(t.text_secondary).child(label.to_uppercase()))
                        .child(div().truncate().t_label().font_features(tabular()).text_color(t.text).child(value))
                }))),
        ))
        .with_priority(gpui_kit::base::POPUP_PRIORITY)
        .into_any_element()
    }
}

impl Render for TraceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let spans = self.run.trace_spans.clone().unwrap_or_default();
        let end = trace_total(&spans).max(1.);
        let extent = axis_step(end) * TICKS as f64;
        // SLOWEST is the longest finished leaf call; parents only aggregate their children.
        let slowest = spans.iter()
            .filter(|span| span.duration_ms.is_some() && !spans.iter().any(|child| child.parent_id.as_ref() == Some(&span.id)))
            .max_by(|a, b| a.duration_ms.partial_cmp(&b.duration_ms).unwrap_or(std::cmp::Ordering::Equal))
            .map(|span| span.id.clone());
        let rows = visible_rows(&spans, &self.collapsed);
        let grid = div().absolute().inset_0().flex().child(span_col(div()))
            .children((0..TICKS).map(|index| div().flex_1().h_full().when(index > 0, |this| this.border_l_1().border_color(t.border.opacity(0.6)))));
        let body: AnyElement = if rows.is_empty() {
            div().px_4().py_4().t_label().text_color(t.text_secondary).child("No trace spans recorded for this run.").into_any_element()
        } else {
            div().relative().flex().flex_col()
                .child(self.render_ruler(extent, cx))
                .child(div().relative().flex().flex_col()
                    .child(grid)
                    .children(rows.iter().enumerate().map(|(index, row)| {
                        let last = index + 1 == rows.len();
                        let popover = (self.selected.as_ref() == Some(&row.span.id)).then(|| self.render_popover(row.span, cx));
                        self.render_row(row, extent, end, slowest.as_ref() == Some(&row.span.id), last, popover, cx)
                    })))
                .into_any_element()
        };
        // The border is a top layer with the card's shape, so edge fills (the last row's
        // selection) share one arc instead of drawing a second, inset one.
        div().relative().flex().flex_col().rounded(px(metrics::R_CARD)).bg(t.surface)
            .child(self.render_header(cx))
            .child(self.render_stats(&spans, cx))
            .child(body)
            .child(div().absolute().inset_0().rounded(px(metrics::R_CARD)).border_1().border_color(t.border))
    }
}

fn stat_cell((label, value, alert): (&str, Option<String>, bool), cx: &App) -> Div {
    let t = theme::tokens(cx);
    div().flex_1().min_w_0().flex().flex_col().gap_1().px_3().py_3().border_l_1().border_color(t.border)
        .child(div().truncate().t_caption().font_weight(FontWeight::MEDIUM).text_color(t.text_secondary).child(label.to_uppercase()))
        .child(div().truncate().t_stat_sm()
            .text_color(if alert { t.danger } else if value.is_some() { t.text } else { t.text_tertiary })
            .child(value.unwrap_or_else(|| "—".into())))
}

/// The span column: 40% of the card, capped at the measured reference width.
fn span_col(el: Div) -> Div {
    el.flex_none().w(relative(SPAN_COL_FRACTION)).max_w(px(SPAN_COL_MAX_W))
}

/// 1pt hierarchy guides (ref F263F6D3) under each ancestor's chevron centre, from the
/// chevron down on an expanded parent through its descendants. An ancestor's guide stops
/// at the centre of its last visible descendant, so it never reaches the card edge; roots
/// and collapsed parents draw none, so unrelated roots are never joined.
fn tree_guides(row: &Row, expanded: bool, cx: &App) -> Vec<Div> {
    let color = theme::tokens(cx).border;
    let x = |depth: usize| px(8. + depth as f32 * INDENT + 8. - 0.5);
    let mut guides: Vec<Div> = (0..row.depth).map(|depth| {
        let guide = div().absolute().top_0().left(x(depth)).w(px(1.)).bg(color);
        if row.next_depth > depth { guide.bottom_0() } else { guide.h(px(ROW_H / 2.)) }
    }).collect();
    if row.has_children && expanded {
        guides.push(div().absolute().top(px(ROW_H / 2. + 8.)).bottom_0().left(x(row.depth)).w(px(1.)).bg(color));
    }
    guides
}

struct Row<'a> {
    span: &'a TraceSpan,
    depth: usize,
    has_children: bool,
    /// Depth of the next visible row (0 after the last), to end guides.
    next_depth: usize,
}

/// Spans in tree order, skipping descendants of collapsed parents. A span whose parent
/// isn't in the trace is a root, so nothing is dropped.
fn visible_rows<'a>(spans: &'a [TraceSpan], collapsed: &HashSet<String>) -> Vec<Row<'a>> {
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
        let has_children = children_of(spans, &span.id).next().is_some();
        out.push(Row { span, depth, has_children, next_depth: 0 });
        if !collapsed.contains(&span.id) {
            let mut kids: Vec<_> = children_of(spans, &span.id).collect();
            kids.reverse();
            stack.extend(kids.into_iter().map(|child| (child, depth + 1)));
        }
    }
    let depths: Vec<usize> = out.iter().map(|row| row.depth).collect();
    for (row, next) in out.iter_mut().zip(depths.iter().skip(1)) {
        row.next_depth = *next;
    }
    out
}

fn children_of<'a>(spans: &'a [TraceSpan], id: &'a str) -> impl Iterator<Item = &'a TraceSpan> {
    spans.iter().filter(move |child| child.parent_id.as_deref() == Some(id))
}

/// Nice tick step so `TICKS − 1` steps cover the trace; the axis is `TICKS` steps long,
/// which leaves at least one step of room for trailing labels.
fn axis_step(end: f64) -> f64 {
    const STEPS_MS: [f64; 16] = [
        100., 200., 500., 1_000., 2_000., 5_000., 10_000., 15_000., 30_000.,
        60_000., 120_000., 300_000., 600_000., 900_000., 1_800_000., 3_600_000.,
    ];
    let wanted = end / (TICKS - 1) as f64;
    STEPS_MS.iter().copied().find(|step| *step >= wanted)
        .unwrap_or_else(|| (wanted / 3_600_000.).ceil() * 3_600_000.)
}

fn trace_total(spans: &[TraceSpan]) -> f64 {
    spans.iter().map(|span| span.start_offset_ms + span.duration_ms.unwrap_or(0.)).fold(0., f64::max)
}

/// 24 tile with a 16 glyph per span kind.
fn kind_tile(kind: &SpanKind, cx: &App) -> Div {
    let t = theme::tokens(cx);
    let icon = match kind {
        SpanKind::Stage => Lucide::Layers,
        SpanKind::Agent => Lucide::Bot,
        SpanKind::Tool => Lucide::Wrench,
        SpanKind::Gate => Lucide::ShieldCheck,
        _ => Lucide::Circle,
    };
    div().size(px(24.)).flex_none().flex().items_center().justify_center().rounded(px(metrics::R_CONTROL))
        .border_1().border_color(t.border).bg(t.surface)
        .child(Icon::new(icon).size_4().text_color(t.text_secondary))
}

struct BarPalette {
    fill: Hsla,
    border: Hsla,
}

/// Parents are neutral (their children carry the colour); agents violet, tools teal, gates
/// amber, failures red, unfinished work blue (ref F263F6D3 bar families).
fn bar_palette(span: &TraceSpan, has_children: bool, cx: &App) -> BarPalette {
    let t = theme::tokens(cx);
    let color: Hsla = match (&span.status, &span.kind) {
        (RunStepStatus::Failed, _) => t.danger,
        (RunStepStatus::NeedsApproval, _) | (_, SpanKind::Gate) => t.attention,
        (RunStepStatus::Running | RunStepStatus::Pending, _) => t.running,
        _ if has_children => return BarPalette { fill: t.tray, border: t.border },
        (_, SpanKind::Agent) => rgb(0x8B5CF6).into(),
        (_, SpanKind::Tool) => rgb(0x14B8A6).into(),
        _ => t.text_tertiary,
    };
    BarPalette { fill: color.opacity(0.16), border: color.opacity(0.55) }
}

fn kind_label(kind: &SpanKind) -> String {
    match kind {
        SpanKind::Stage => "Stage".into(),
        SpanKind::Agent => "Agent".into(),
        SpanKind::Tool => "Tool".into(),
        SpanKind::Gate => "Gate".into(),
        SpanKind::Other => "Other".into(),
        SpanKind::Unknown(raw) => raw.clone(),
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

/// Axis origin reads "0s" rather than "0ms"; other ticks use the duration format.
fn format_tick(ms: f64) -> String {
    if ms == 0. { "0s".into() } else { format_ms(ms) }
}

fn format_count(count: u64) -> String {
    if count >= 1000 { format!("{:.1}k", count as f64 / 1000.) } else { count.to_string() }
}

fn format_cost(cost: f64, currency: Option<&str>) -> String {
    match currency {
        None | Some("USD") => format!("${cost:.2}"),
        Some(code) => format!("{cost:.2} {code}"),
    }
}
