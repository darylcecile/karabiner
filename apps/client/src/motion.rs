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
