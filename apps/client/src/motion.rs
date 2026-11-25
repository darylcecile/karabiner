//! Shared motion values. Use these with `gpui_kit::base::motion` primitives so
//! timing stays consistent; Base honours the macOS Reduce Motion setting.
//! References: tray open, content swap and promotion in `ADFBCD30` are instant cuts (no
//! height animation); popovers and rail details in `F046D5DF` fade in 150–180 ms.

use std::time::Duration;

use gpui_kit::base::motion::{Easing, Transition};

pub const FAST: Duration = Duration::from_millis(120);
/// Workspace tab open/close (width + opacity).
pub const TAB: Duration = Duration::from_millis(160);
/// Popover / detail fade (opacity only, no scale).
pub const POPOVER: Duration = Duration::from_millis(160);
pub const STANDARD: Duration = Duration::from_millis(200);

/// Opacity/colour changes and small reveals.
pub fn fade() -> Transition {
    Transition::new(FAST).easing(Easing::EaseOut)
}

/// Workspace tab open/close.
pub fn tab() -> Transition {
    Transition::new(TAB).easing(Easing::EaseOut)
}

/// Popover and floating detail fade-in.
pub fn popover() -> Transition {
    Transition::new(POPOVER).easing(Easing::EaseOut)
}

/// Page and panel movement (sidebar, inspector, drawers).
pub fn standard() -> Transition {
    Transition::new(STANDARD).easing(Easing::EaseOut)
}

pub use gpui_kit::base::ReduceMotionPreference;

/// Applies the persisted app motion setting (Settings → Appearance) on top of macOS.
pub fn apply(motion: Option<&protocol::Motion>, cx: &mut gpui_kit::App) {
    let preference = match motion {
        Some(protocol::Motion::Reduce) => ReduceMotionPreference::Reduce,
        Some(protocol::Motion::Full) => ReduceMotionPreference::Allow,
        _ => ReduceMotionPreference::System,
    };
    if cx.try_global::<MotionOverride>().map(|value| value.0) == Some(preference) { return }
    cx.set_global(MotionOverride(preference));
    gpui_kit::base::set_reduce_motion_preference(preference, cx);
    cx.refresh_windows();
}

struct MotionOverride(ReduceMotionPreference);

impl gpui_kit::Global for MotionOverride {}
