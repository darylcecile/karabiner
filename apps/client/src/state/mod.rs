//! Shared client state handed to every feature view.

pub mod service;
pub mod tabs;

use gpui_kit::*;

use crate::api::{ApiClient, ApiError};
pub use service::{ServiceState, ServiceStatus};
pub use tabs::{Draft, DraftHandle, Section, TabTarget, Workspace, WorkspaceEvent};

/// Cloned into each feature view constructor. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub api: ApiClient,
    pub service: Entity<ServiceState>,
    pub workspace: WeakEntity<Workspace>,
}

impl AppState {
    /// Call after a successful mutation so every view refetches.
    pub fn data_changed(&self, cx: &mut App) {
        self.service.update(cx, |service, cx| service.data_changed(cx));
    }

    /// Open (or focus) a work-item tab.
    pub fn open(&self, target: TabTarget, window: &mut Window, cx: &mut App) {
        self.defer_in(window, cx, move |workspace, window, cx| workspace.open(target, window, cx));
    }

    /// Navigate the active tab to `target` in place. A tab with unsaved edits is kept and
    /// `target` opens in a new tab instead.
    pub fn replace_active(&self, target: TabTarget, window: &mut Window, cx: &mut App) {
        self.defer_in(window, cx, move |workspace, window, cx| workspace.replace_active(target, window, cx));
    }

    pub fn show_section(&self, section: Section, cx: &mut App) {
        self.defer(cx, move |workspace, cx| workspace.show_section(section, cx));
    }

    /// Update the tab title once the view has loaded its record.
    pub fn set_title(&self, target: &TabTarget, title: impl Into<SharedString>, cx: &mut App) {
        let (target, title) = (target.clone(), title.into());
        self.defer(cx, move |workspace, cx| workspace.set_title(&target, title, cx));
    }

    /// Register a tab view's unsaved-edit state; see [`Draft`]. Call from the view constructor.
    pub fn track_draft<V: Draft>(&self, target: &TabTarget, view: &Entity<V>, cx: &mut App) {
        let (target, draft) = (target.clone(), DraftHandle::new(view));
        self.defer(cx, move |workspace, _| workspace.track_draft(&target, draft));
    }

    /// Close a tab after its record was deleted (no draft prompt).
    pub fn close(&self, target: &TabTarget, cx: &mut App) {
        let target = target.clone();
        self.defer(cx, move |workspace, cx| workspace.close_target(&target, cx));
    }

    /// Show a failed request as a toast; returns the error text for inline display.
    pub fn report_error(&self, error: &ApiError, window: &mut Window, cx: &mut App) -> SharedString {
        crate::components::toast_error(error, window, cx)
    }

    // Workspace updates are deferred: views call these while the workspace may already be
    // mid-update (e.g. while it constructs the view).
    fn defer(&self, cx: &mut App, f: impl FnOnce(&mut Workspace, &mut Context<Workspace>) + 'static) {
        let workspace = self.workspace.clone();
        cx.defer(move |cx| {
            workspace.update(cx, f).ok();
        });
    }

    fn defer_in(
        &self,
        window: &mut Window,
        cx: &mut App,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace.update(cx, |workspace, cx| f(workspace, window, cx)).ok();
        });
    }
}
