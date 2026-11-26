//! Window-level status banner above the content (tab persistence, service health).

use gpui_kit::component::button::Button;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{Tone, Typography as _, button_sm, status_dot};
use crate::theme::{self, metrics};

/// The outlined Retry control; the caller attaches its click handler.
pub fn retry_button(id: &'static str) -> Button {
    button_sm(Button::new(id)).outline().label("Retry")
}

/// Status dot, optional medium title, message (wraps, so the action in it stays readable),
/// then `retry`. The 24pt Retry
/// sits 6pt from the top, bottom and trailing edge; text keeps the 12pt leading inset.
pub fn banner(message: SharedString, title: Option<&'static str>, tone: Tone, retry: Button, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .items_center()
        .gap_3()
        .mx_2()
        .mt_2()
        .pl_3()
        .pr_1p5()
        .py_1p5()
        .rounded(px(metrics::R_INNER))
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .t_label()
        .child(status_dot(tone, cx))
        .when_some(title, |row, title| row.child(div().font_weight(FontWeight::MEDIUM).child(title)))
        .child(div().flex_1().min_w_0().text_color(t.text_secondary).child(message))
        .child(retry)
}
