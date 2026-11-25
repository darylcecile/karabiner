//! Document status banner (deletion, conflict, save failure): a tinted band with a dot and
//! message. Callers append their own 24 action buttons.

use gpui_kit::*;

use crate::{components::Typography as _, theme::{self, metrics}};

pub fn banner(message: String, color: Hsla, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        // Actions are 24 buttons: the right inset matches their 8 vertical inset.
        .pl(px(metrics::GUTTER_X))
        .pr_2()
        .py_2()
        .border_b_1()
        .border_color(t.border)
        .bg(color.opacity(0.10))
        .t_label()
        .text_color(t.text)
        .child(div().size(px(6.)).flex_none().rounded_full().bg(color))
        .child(div().flex_1().child(message))
}
