//! Card, panel and page surfaces, plus the rounded-clip helpers.
//!
//! GPUI clips to rectangles only (`ContentMask` ignores corner radii), so a rounded
//! container never clips its children to the curve. Children that paint at an edge use
//! [`edge_band`] (concentric radius); scroll viewports stay [`clip_inset`] away from arcs.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::typography::Typography as _;
use crate::theme::{self, metrics};

/// Bordered surface card, radius 12.
pub fn card(cx: &App) -> Div {
    let t = theme::tokens(cx);
    div().rounded(px(metrics::R_CARD)).border_1().border_color(t.border).bg(t.surface)
}

/// Grouped settings card: soft fill, no border, radius 16, 16 horizontal padding.
/// Put rows in it with [`settings_row`] and separate them with [`row_divider`].
pub fn section_card(cx: &App) -> Div {
    div().flex().flex_col().rounded(px(metrics::R_SECTION)).bg(theme::tokens(cx).section_fill).px_4()
}

/// Settings row: title + optional description on the left, control right-aligned.
pub fn settings_row(
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .items_center()
        .gap_4()
        .min_h(px(56.))
        .py_3()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().t_body().font_weight(FontWeight::MEDIUM).text_color(t.text).child(title.into()))
                .when_some(description, |this, text| this.child(div().t_body().text_color(t.text_secondary).child(text))),
        )
        .child(div().flex_none().child(control))
}

/// Hairline between rows inside a padded card (the card's padding provides the inset).
pub fn row_divider(cx: &App) -> Div {
    div().h_px().w_full().bg(theme::tokens(cx).border)
}

/// Label above a section card ("Startup"), indented to the card's inner padding.
pub fn section_label(label: impl Into<SharedString>, cx: &App) -> Div {
    div().px_4().pt_7().pb_2().t_label().text_color(theme::tokens(cx).text_secondary).child(label.into())
}

/// Sidebar group label ("Projects").
pub fn group_label(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .px_2()
        .pt_4()
        .pb_1()
        .t_caption()
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::tokens(cx).text_secondary)
        .child(label.into())
}

/// Page title + optional subtitle; 24 below to the first block.
pub fn page_header(title: impl Into<SharedString>, subtitle: Option<SharedString>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .pb_6()
        .child(div().t_title().text_color(t.text).child(title.into()))
        .when_some(subtitle, |this, text| this.child(div().t_body().text_color(t.text_secondary).child(text)))
}

/// Page content column with the standard gutters. `reading` caps the width at 720.
pub fn page_body(reading: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .px(px(metrics::GUTTER_X))
        .pt(px(metrics::GUTTER_TOP))
        .pb_6()
        .when(reading, |this| this.max_w(px(metrics::READ_W + 2. * metrics::GUTTER_X)))
}

/// Header bar height (full task, docs): holds 28 controls.
pub const BAR_H: f32 = 44.;
/// Bar edge padding: equals a 28 control's vertical inset, so the hover box of an edge
/// control sits the same distance from the bar's side as from its top and bottom.
pub const BAR_INSET: f32 = (BAR_H - metrics::CONTROL_MD) / 2.;

/// Hairline header bar, 44 high, [`BAR_INSET`] on both sides. A bar that starts with plain
/// text overrides its left padding to the text edge of the content below it.
pub fn header_bar(cx: &App) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .h(px(BAR_H))
        .px(px(BAR_INSET))
        .border_b_1()
        .border_color(theme::tokens(cx).border)
}

/// Which edge of a rounded container a band touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

/// Full-width band (header, footer, first/last row) flush with a bordered container of
/// radius `outer`: the touching corners get `outer − 1` so its fill stays inside the curve.
pub fn edge_band(edge: Edge, outer: f32) -> Div {
    let radius = px((outer - 1.).max(0.));
    match edge {
        Edge::Top => div().rounded_t(radius),
        Edge::Bottom => div().rounded_b(radius),
    }
}

/// Minimum inset that keeps a rectangular clip (scroll viewport) inside a corner of
/// `radius`: the clip corner must sit on or inside the arc on both axes.
pub fn clip_inset(radius: f32) -> Pixels {
    px((radius * 0.3).ceil())
}

/// Rounded frame painted over a container's content: corner caps in `outside` (the colour
/// behind the container) and the container's 1pt `border` ring. GPUI clips children to the
/// bounding rectangle only, so this is what keeps full-bleed children, scrolled content and
/// drawers inside the curve. Place it as the last child of a `relative()` box with the same
/// bounds and radius as the container. It has no hitbox, so input passes through.
pub fn rounded_frame(radius: f32, outside: Hsla, border: Hsla) -> Div {
    let cap = |top: bool, left: bool| {
        // A ring of width `radius` around a box `radius` larger on every side: its inner
        // edge is the container's arc (GPUI insets the inner radius by the border width,
        // same centre), its outer edge lies beyond the cap, and the cap clips the rest.
        // GPUI clamps radii to half the quad size, hence the 4×radius box.
        let mut ring = div()
            .absolute()
            .size(px(radius * 4.))
            .rounded(px(radius * 2.))
            .border_color(outside)
            .map(|ring| if top { ring.top(px(-radius)) } else { ring.bottom(px(-radius)) })
            .map(|ring| if left { ring.left(px(-radius)) } else { ring.right(px(-radius)) });
        let width: AbsoluteLength = px(radius).into();
        ring.style().border_widths = EdgesRefinement {
            top: Some(width),
            right: Some(width),
            bottom: Some(width),
            left: Some(width),
        };
        div()
            .absolute()
            .size(px(radius))
            .overflow_hidden()
            .map(|cap| if top { cap.top_0() } else { cap.bottom_0() })
            .map(|cap| if left { cap.left_0() } else { cap.right_0() })
            .child(ring)
    };
    div()
        .absolute()
        .inset_0()
        .child(cap(true, true))
        .child(cap(true, false))
        .child(cap(false, true))
        .child(cap(false, false))
        .child(div().absolute().inset_0().rounded(px(radius)).border_1().border_color(border))
}

/// Floating popover/menu surface: border, radius 10, large shadow, 6 padding.
pub fn popover_card(cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_col()
        .p_1p5()
        .rounded(px(metrics::R_POPOVER))
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .shadow_lg()
}

/// Popover header: semibold title with an optional right-aligned caption.
pub fn popover_header(title: impl Into<SharedString>, detail: Option<SharedString>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_1p5()
        .pt_1p5()
        .pb_2()
        .child(div().flex_1().min_w_0().truncate().t_panel_title().text_color(t.text).child(title.into()))
        .when_some(detail, |this, text| this.child(div().flex_none().t_caption().text_color(t.text_secondary).child(text)))
}

/// Stat tile (`DB2A4628`: 151×66, R6, gap 8): large tabular value over a caption label.
pub fn stat_tile(value: impl Into<SharedString>, label: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2p5()
        .rounded(px(metrics::R_STAT))
        .bg(t.section_fill)
        .child(div().t_stat().text_color(t.text).child(value.into()))
        .child(div().t_caption().text_color(t.text_secondary).child(label.into()))
}
