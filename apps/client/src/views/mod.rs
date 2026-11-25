//! Feature view registry (lead-owned). Feature modules implement the frozen signatures
//! below; this file only routes to them.
//!
//! ```ignore
//! pub fn section(state: AppState, window: &mut Window, cx: &mut App) -> AnyView;          // sidebar destination
//! pub fn tab(target: &TabTarget, state: AppState, window: &mut Window, cx: &mut App) -> AnyView; // work-item tab
//! ```

pub mod docs;
pub mod onboarding;
pub mod projects;
pub mod responsive;
pub mod review;
pub mod stats;
pub mod tasks;
pub mod workflow;

use gpui_kit::*;
use protocol::TabKind;

use crate::{
    components::empty_state,
    state::{AppState, Section, TabTarget},
};

pub fn build_section(section: Section, state: AppState, window: &mut Window, cx: &mut App) -> AnyView {
    match section {
        Section::Home => stats::section(state, window, cx),
        Section::Inbox => workflow::inbox_section(state, window, cx),
        Section::Projects => projects::section(state, window, cx),
        Section::Templates => onboarding::section(state, window, cx),
        Section::Settings => workflow::settings_section(state, window, cx),
    }
}

pub fn build_tab(target: &TabTarget, state: AppState, window: &mut Window, cx: &mut App) -> AnyView {
    match target.kind {
        TabKind::Project => projects::tab(target, state, window, cx),
        TabKind::Task => tasks::tab(target, state, window, cx),
        TabKind::Document => docs::tab(target, state, window, cx),
        TabKind::Review => review::tab(target, state, window, cx),
        TabKind::Unknown(_) => not_implemented("Unsupported item", "This tab was saved by a newer version of the app.", cx),
    }
}

/// Honest placeholder for a destination whose feature module has not landed yet.
pub fn not_implemented(title: &'static str, detail: &'static str, cx: &mut App) -> AnyView {
    cx.new(|_| NotImplemented { title, detail }).into()
}

struct NotImplemented {
    title: &'static str,
    detail: &'static str,
}

impl Render for NotImplemented {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        empty_state(self.title, self.detail)
    }
}
