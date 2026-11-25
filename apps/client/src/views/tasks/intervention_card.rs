//! Open intervention card: the kind glyph, label and message in the attention frame. The
//! detail adds its actions; the visual-states example renders it per kind.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Icon;
use gpui_kit::*;
use protocol::InterventionKind;

use crate::{components::{self, Typography as _}, theme::{self, metrics}};

/// Frame, header and message; callers append their own action rows.
pub fn card(kind: &InterventionKind, message: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div().rounded(px(metrics::R_INNER)).border_1().border_color(t.attention.opacity(0.4)).bg(components::Tone::Attention.tint(cx, 0.10)).p_3().flex().flex_col().gap_1()
        .child(div().flex().items_center().gap_1p5().t_caption().font_weight(FontWeight::MEDIUM).text_color(t.attention)
            .child(Icon::new(icon(kind)).size(px(14.)))
            .child(format!("{} · recorded, not delivered", label(kind))))
        .child(div().t_label().text_color(t.text).child(message.into()))
}

pub fn icon(kind: &InterventionKind) -> Lucide {
    match kind {
        InterventionKind::Instruction => Lucide::FileText,
        InterventionKind::Question => Lucide::MessageCircleQuestionMark,
        InterventionKind::Pause => Lucide::Pause,
        InterventionKind::Resume => Lucide::Play,
        InterventionKind::Cancel => Lucide::CircleStop,
        InterventionKind::Retry => Lucide::RotateCw,
        InterventionKind::Approve => Lucide::ThumbsUp,
        InterventionKind::Reject => Lucide::ThumbsDown,
        InterventionKind::Unknown(_) => Lucide::CircleQuestionMark,
    }
}

pub fn label(kind: &InterventionKind) -> &'static str {
    match kind {
        InterventionKind::Instruction => "Instruction",
        InterventionKind::Question => "Question",
        InterventionKind::Pause => "Pause",
        InterventionKind::Resume => "Resume",
        InterventionKind::Cancel => "Cancel",
        InterventionKind::Retry => "Retry",
        InterventionKind::Approve => "Approve",
        InterventionKind::Reject => "Reject",
        InterventionKind::Unknown(_) => "Other",
    }
}
