//! Shared checkable row for assignment lists (stage agents, instruction files, skills).
//! The graphical workflow editor it used to host is now `factory`.

use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{self, Typography as _};
use crate::theme::{self as theme_tokens, metrics};

/// Checkable row used for agent/skill assignment (ref 630BC713 subtask rows): 28 high,
/// 14 checkbox, hover fill only.
pub fn toggle_row(id: SharedString, on: bool, label: String, meta: Option<String>, cx: &App) -> Stateful<Div> {
    let t = theme_tokens::tokens(cx);
    div().id(id).flex().items_center().gap_2().h(px(metrics::ROW_PROP)).px_2().rounded(px(metrics::R_CONTROL)).cursor_pointer()
        .hover(move |style| style.bg(t.hover))
        .child(div().size(px(14.)).flex_none().rounded(px(4.)).border_1().border_color(if on { t.accent_fill } else { t.text_tertiary })
            .when(on, |this| this.bg(t.accent_fill)).flex().items_center().justify_center()
            .when(on, |this| this.child(Icon::new(IconName::Check).size(px(10.)).text_color(t.accent_text))))
        .child(div().flex_1().min_w_0().t_label().text_color(t.text).truncate().child(label))
        .children(meta.map(|meta| div().t_caption().font_features(components::typography::tabular()).text_color(t.text_secondary).child(meta)))
}
