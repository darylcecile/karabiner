//! App palette layered on the Kit theme. Values come from the reference designs
//! (`.groundwork/guides/spatial-references-a.md`, "Palette"): light-neutral by default,
//! with an optional dark palette. The user's choice is persisted in `/v1/settings`.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use protocol::Theme as ThemePreference;

/// Shared layout metrics in points (see `.groundwork/guides/visual-contract.md`).
pub mod metrics {
    pub const TITLEBAR_H: f32 = 40.;
    /// Connected tab shape: a 24 face centred in the bar plus the 8 foot down to the card.
    pub const TAB_H: f32 = 32.;
    pub const SIDEBAR_W: f32 = 240.;
    pub const SHELL_INSET: f32 = 8.;
    pub const GUTTER_X: f32 = 24.;
    pub const GUTTER_TOP: f32 = 20.;
    pub const READ_W: f32 = 720.;
    pub const ROW_NAV: f32 = 30.;
    pub const ROW_LIST: f32 = 36.;
    pub const ROW_PROP: f32 = 28.;
    pub const CONTROL_MD: f32 = 28.;
    pub const CONTROL_SM: f32 = 24.;
    pub const CONTROL_XS: f32 = 20.;
    pub const CHIP_H: f32 = 20.;
    pub const R_CONTROL: f32 = 6.;
    pub const R_INNER: f32 = 8.;
    /// Review chapter number boxes (measured, `851174F7`).
    pub const R_NUMBER: f32 = 5.;
    pub const R_POPOVER: f32 = 10.;
    /// Stat tiles (measured `DB2A4628`).
    pub const R_STAT: f32 = 6.;
    /// Contribution heatmap cells (measured `DB2A4628`).
    pub const HEATMAP_CELL: f32 = 11.;
    pub const HEATMAP_GAP: f32 = 3.;
    pub const R_HEATMAP: f32 = 2.;
    pub const R_CARD: f32 = 12.;
    /// Grouped settings cards (Hexta refs `92D96623`, `B97FF317`).
    pub const R_SECTION: f32 = 16.;
    pub const R_DIALOG: f32 = 14.;

    /// Concentric radius for a child inset by `inset` inside a corner of radius `outer`.
    pub fn inner_radius(outer: f32, inset: f32) -> f32 {
        (outer - inset).max(4.)
    }
}

/// Colours the Kit theme has no role for. Read with [`tokens`].
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    /// Page/canvas behind cards.
    pub canvas: Hsla,
    /// Cards and raised panels.
    pub surface: Hsla,
    /// Inset trays and footers around cards.
    pub tray: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    /// Hover fill for rows and tabs.
    pub hover: Hsla,
    /// Selected row/tab fill.
    pub selected: Hsla,
    pub running: Hsla,
    pub attention: Hsla,
    pub success: Hsla,
    pub danger: Hsla,
    pub validating: Hsla,
    /// Primary (dark) button fill and its label colour.
    pub accent_fill: Hsla,
    pub accent_text: Hsla,
    /// Grouped settings-card fill (no border).
    pub section_fill: Hsla,
    /// Board column tray and the task cards on it (`E62FEC84` dark, `AE10217A` light).
    pub board_column: Hsla,
    pub board_card: Hsla,
}

fn light() -> Tokens {
    Tokens {
        canvas: c(0xFFFFFF),
        surface: c(0xFFFFFF),
        tray: c(0xF4F4F4),
        border: c(0xE6E6E6),
        text: c(0x171717),
        text_secondary: c(0x737373),
        text_tertiary: c(0xA3A3A3),
        hover: c(0xEBEBEB),
        selected: c(0xE2E2E2),
        running: c(0x3B82F6),
        attention: c(0xF59E0B),
        success: c(0x22C55E),
        danger: c(0xEF4444),
        validating: c(0xEC4899),
        accent_fill: c(0x171717),
        accent_text: c(0xFFFFFF),
        section_fill: c(0xFAFAFA),
        board_column: c(0xF4F4F4),
        board_card: c(0xFFFFFF),
    }
}

fn dark() -> Tokens {
    Tokens {
        canvas: c(0x121212),
        surface: c(0x1A1A1A),
        tray: c(0x161616),
        border: c(0x2E2E2E),
        text: c(0xEDEDED),
        text_secondary: c(0x8A8A8A),
        text_tertiary: c(0x5C5C5C),
        hover: c(0x222222),
        selected: c(0x2A2A2A),
        running: c(0x3B82F6),
        attention: c(0xF59E0B),
        success: c(0x22C55E),
        danger: c(0xEF4444),
        validating: c(0xEC4899),
        accent_fill: c(0xEDEDED),
        accent_text: c(0x171717),
        section_fill: c(0x1C1C1C),
        board_column: c(0x1D1D1D),
        board_card: c(0x2A2829),
    }
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn sidebar_color(is_dark: bool) -> Hsla {
    c(if is_dark { 0x0B0B0B } else { 0xEEEEEE })
}

/// Current palette for the active light/dark mode.
pub fn tokens(cx: &App) -> Tokens {
    if Theme::global(cx).is_dark() { dark() } else { light() }
}

struct CurrentPreference(ThemePreference);
impl Global for CurrentPreference {}

pub fn init(cx: &mut App) {
    cx.set_global(CurrentPreference(ThemePreference::System));
}

/// Applies a stored preference. `System` follows the window appearance.
pub fn apply(preference: ThemePreference, window: &mut Window, cx: &mut App) {
    let unchanged = cx.global::<CurrentPreference>().0 == preference;
    if unchanged && preference != ThemePreference::System {
        return;
    }
    let mode = match preference {
        ThemePreference::Dark => ThemeMode::Dark,
        ThemePreference::Light => ThemeMode::Light,
        _ => window.appearance().into(),
    };
    cx.set_global(CurrentPreference(preference));
    Theme::change(mode, Some(window), cx);
    let is_dark = mode.is_dark();
    let t = if is_dark { dark() } else { light() };
    let sidebar = sidebar_color(is_dark);
    Theme::update(cx, |theme| {
        // Kit sets the window rem to `font_size`; 16 keeps Kit controls and rem spacing at
        // their nominal sizes. Text size comes from `components::typography` roles instead.
        theme.font_size = px(16.);
        // Kit's code editor/diff reads this directly rather than inheriting `t_code`.
        theme.mono_font_size = px(12.5);
        theme.radius = px(metrics::R_CONTROL);
        theme.radius_lg = px(metrics::R_CARD);
        let c = &mut theme.colors;
        c.background = t.canvas;
        c.foreground = t.text;
        c.muted_foreground = t.text_secondary;
        c.border = t.border;
        c.input = t.border;
        c.popover = t.surface;
        c.list = t.surface;
        c.list_hover = t.hover;
        c.list_active = t.selected;
        c.table = t.surface;
        c.sidebar = sidebar;
        c.sidebar_border = t.border;
        c.sidebar_foreground = t.text;
        c.sidebar_accent = t.selected;
        c.sidebar_accent_foreground = t.text;
        c.title_bar = sidebar;
        c.title_bar_border = t.border;
        c.tab_bar = sidebar;
        c.tab = sidebar;
        c.tab_active = t.selected;
        c.tab_foreground = t.text_secondary;
        c.tab_active_foreground = t.text;
        c.danger = t.danger;
        c.success = t.success;
        c.warning = t.attention;
        c.info = t.running;
    });
}

/// Re-applies `System` after the OS appearance changes.
pub fn appearance_changed(window: &mut Window, cx: &mut App) {
    if cx.global::<CurrentPreference>().0 == ThemePreference::System {
        apply(ThemePreference::System, window, cx);
    }
}
