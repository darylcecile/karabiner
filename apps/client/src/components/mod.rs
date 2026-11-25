//! Shared building blocks for feature views, sized to `.groundwork/guides/visual-contract.md`.
//! Text: `typography::Typography` roles. Surfaces and clip-safe edges: `surfaces`.
//! Controls and Kit size adapters: `controls`. Rotated text: `vertical_label`. Run trace: `trace`.

mod controls;
pub mod factory_canvas;
mod form;
mod menu;
mod run_step;
mod surfaces;
mod trace;
pub mod typography;
mod vertical_label;

pub use controls::*;
pub use form::*;
pub use menu::{glyph_item, menu_popover};
pub use run_step::{Step, StepAnswer, StepState, run_step, step_glyph};
pub use surfaces::*;
pub use trace::TraceView;
pub use typography::Typography;
pub use vertical_label::vertical_label;

use gpui_kit::component::{
    Sizable as _, WindowExt as _,
    button::Button,
    notification::Notification,
    spinner::Spinner,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{api::ApiError, theme};

/// Semantic state colours used for dots, chips and progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Running,
    Attention,
    Success,
    Danger,
    Validating,
}

impl Tone {
    pub fn color(self, cx: &App) -> Hsla {
        let t = theme::tokens(cx);
        match self {
            Self::Neutral => t.text_tertiary,
            Self::Running => t.running,
            Self::Attention => t.attention,
            Self::Success => t.success,
            Self::Danger => t.danger,
            Self::Validating => t.validating,
        }
    }

    /// Tone colour at `alpha` for fills: 0.10 resting, 0.15 hover/selected.
    pub fn tint(self, cx: &App, alpha: f32) -> Hsla {
        self.color(cx).opacity(alpha)
    }
}

/// 8pt status dot.
pub fn status_dot(tone: Tone, cx: &App) -> Div {
    div().size(px(8.)).flex_none().rounded_full().bg(tone.color(cx))
}

/// Centered title + explanation, for empty and unavailable states.
pub fn empty_state(title: impl Into<SharedString>, detail: impl Into<SharedString>) -> impl IntoElement {
    EmptyState { title: title.into(), detail: detail.into(), retry: None }
}

/// Error state with a Retry button that calls `on_retry`.
pub fn error_state(
    error: impl Into<SharedString>,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    EmptyState {
        title: "Couldn't load".into(),
        detail: error.into(),
        retry: Some(Box::new(on_retry)),
    }
}

/// Spinner with a short label, centered in the available space.
pub fn loading_state(label: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .t_body()
        .text_color(theme::tokens(cx).text_secondary)
        .child(Spinner::new().small())
        .child(label.into())
}

type RetryHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
struct EmptyState {
    title: SharedString,
    detail: SharedString,
    retry: Option<RetryHandler>,
}

impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::tokens(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .p_6()
            .child(div().t_body().font_weight(FontWeight::MEDIUM).text_color(t.text).child(self.title))
            .child(
                div()
                    .max_w(px(420.))
                    .t_body()
                    .text_center()
                    .text_color(t.text_secondary)
                    .child(self.detail),
            )
            .when_some(self.retry, |this, retry| {
                this.child(div().pt_2().child(button_sm(Button::new("retry").outline().label("Retry")).on_click(retry)))
            })
    }
}

pub fn toast_warning(message: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
    window.push_notification(Notification::warning(message), cx);
}

/// Shows the error as a toast and returns its text for inline display.
pub fn toast_error(error: &ApiError, window: &mut Window, cx: &mut App) -> SharedString {
    let message: SharedString = error.to_string().into();
    let title = match error {
        ApiError::Unavailable(_) => "Service unavailable",
        _ if error.is_conflict() => "Changed elsewhere",
        _ if error.is_not_implemented() => "Not implemented by the service",
        _ => "Request failed",
    };
    window.push_notification(Notification::error(message.clone()).title(title), cx);
    message
}

/// Full-colour Kubai feature icon (`apps/client/assets/kubai/svg-soft/<name>.svg`, MIT —
/// see that folder's LICENSE), served from the embedded asset source. Use for larger
/// feature glyphs; keep 16px chrome on monochrome Lucide icons. Files include shadow
/// margin, so ~28px matches a 24px glyph.
pub fn feature_icon(name: &str, size: Pixels) -> Img {
    img(SharedString::from(format!("kubai/svg-soft/{name}.svg"))).size(size).flex_none()
}
