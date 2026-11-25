//! Settings-form layout (refs: Linear team settings, `form-research/v-*`): a plain section
//! title with an optional description, then rows separated by hairlines. Each row has its
//! label and help on the left and the control on the right; on narrow panels the control
//! goes under the label at full width. Section wrappers draw no border or fill, so the
//! control inside a row owns the only border.

use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::typography::Typography as _;
use crate::theme;

/// Width of the control column in a wide row.
pub const CONTROL_W: f32 = 280.;

/// Section heading: title, optional description, then put rows in [`form_rows`].
pub fn form_section(title: impl Into<SharedString>, description: Option<SharedString>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div().flex().flex_col().gap_1().pt_6().pb_2()
        .child(div().t_body().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title.into()))
        .when_some(description, |this, text| this.child(div().t_body().text_color(t.text_secondary).child(text)))
}

/// Rows container: a hairline above the first row; each row draws the one below it.
pub fn form_rows(cx: &App) -> Div {
    div().flex().flex_col().border_t_1().border_color(theme::tokens(cx).border)
}

/// One labelled row. `stacked` puts the control under the label at full width.
pub fn form_row(label: impl Into<SharedString>, help: Option<SharedString>, control: impl IntoElement, stacked: bool, cx: &App) -> Div {
    let t = theme::tokens(cx);
    let text = div().flex().flex_col().gap_0p5().min_w_0()
        .child(div().t_body().font_weight(FontWeight::MEDIUM).text_color(t.text).child(label.into()))
        .when_some(help, |this, help| this.child(div().t_caption().text_color(t.text_secondary).child(help)));
    let row = div().py_3().border_b_1().border_color(t.border);
    if stacked {
        row.flex().flex_col().gap_2().child(text).child(div().w_full().min_w_0().child(control))
    } else {
        row.flex().items_center().gap_4()
            .child(text.flex_1())
            .child(div().flex_none().w(px(CONTROL_W)).min_w_0().flex().justify_end().child(control))
    }
}

/// A full-width row with no label column (lists, notes, grouped choices).
pub fn form_block(content: impl IntoElement, cx: &App) -> Div {
    div().py_3().border_b_1().border_color(theme::tokens(cx).border).min_w_0().child(content)
}

/// Clickable group header for progressive disclosure: chevron, title and a one-line summary
/// of what's inside (shown while collapsed).
pub fn disclosure(id: impl Into<ElementId>, title: impl Into<SharedString>, summary: Option<SharedString>, open: bool, cx: &App) -> Stateful<Div> {
    let t = theme::tokens(cx);
    div().id(id).flex().items_center().gap_1p5().py_2().cursor_pointer().min_w_0()
        .border_b_1().border_color(t.border)
        .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(14.)).text_color(t.text_secondary))
        .child(div().flex_none().t_label().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title.into()))
        .when_some(summary.filter(|_| !open), |this, summary| this.child(div().flex_1().min_w_0().truncate().t_caption().text_color(t.text_secondary).child(summary)))
}
