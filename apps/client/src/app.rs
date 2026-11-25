//! Window shell: titlebar tabs, collapsible sidebar, service banner, active content,
//! workspace key bindings, and the persisted theme preference.

use std::time::Duration;

use std::rc::Rc;

use gpui_kit::base::motion::{Transition, transition};
use gpui_kit::base::{ResizeHandleContext, ResizeHandleState, resize_handle};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::{
    ActiveTheme as _,
    tooltip::Tooltip,
};
use gpui_kit::*;
use protocol::AppSettings;

use crate::{
    api::ApiClient,
    components::Tone,
    motion,
    shell::{
        banner,
        inspector::{INSPECTOR_WIDTH, Inspector},
        quick_open,
        sidebar::{SIDEBAR_WIDTH, Sidebar},
        titlebar::{self, ClosingTab, TabStripActions},
    },
    state::{ServiceState, ServiceStatus, Workspace, WorkspaceEvent},
    theme::{self, metrics},
};

const CONTEXT: &str = "Workspace";
/// Sidebar width bounds while resizing; dragging below `SIDEBAR_COLLAPSE_AT` closes it.
const SIDEBAR_MIN: f32 = 200.;
const SIDEBAR_MAX: f32 = 360.;
/// Narrowest primary view beside a docked inspector: the task drawer's 100 label column,
/// ~300 of values/controls and 2×20 padding. Below it the inspector floats instead.
const PRIMARY_MIN: f32 = 480.;
const SIDEBAR_COLLAPSE_AT: f32 = SIDEBAR_MIN / 2.;

/// Drag payload for the sidebar divider (also its invisible drag preview).
#[derive(Clone)]
struct SidebarResize;

impl Render for SidebarResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Settled width after releasing the divider at `live`: closed below the threshold,
/// otherwise within the usable bounds.
fn sidebar_release_width(live: f32) -> f32 {
    if live < SIDEBAR_COLLAPSE_AT { 0. } else { live.clamp(SIDEBAR_MIN, SIDEBAR_MAX) }
}

actions!(
    workspace,
    [
        NewTab, CloseTab, ReopenClosedTab, NextTab, PreviousTab, Refresh, ToggleSidebar,
        ToggleInspector, GoBack, GoForward,
        ActivateTab1, ActivateTab2, ActivateTab3, ActivateTab4, ActivateTab5, ActivateTab6,
        ActivateTab7, ActivateTab8, ActivateLastTab
    ]
);

pub fn bind_keys(cx: &mut App) {
    let context = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("cmd-t", NewTab, context),
        KeyBinding::new("cmd-w", CloseTab, context),
        KeyBinding::new("cmd-shift-t", ReopenClosedTab, context),
        KeyBinding::new("ctrl-tab", NextTab, context),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, context),
        KeyBinding::new("cmd-shift-]", NextTab, context),
        KeyBinding::new("cmd-shift-[", PreviousTab, context),
        KeyBinding::new("cmd-r", Refresh, context),
        KeyBinding::new("cmd-\\", ToggleSidebar, context),
        KeyBinding::new("cmd-alt-i", ToggleInspector, context),
        KeyBinding::new("cmd-[", GoBack, context),
        KeyBinding::new("cmd-]", GoForward, context),
        KeyBinding::new("cmd-1", ActivateTab1, context),
        KeyBinding::new("cmd-2", ActivateTab2, context),
        KeyBinding::new("cmd-3", ActivateTab3, context),
        KeyBinding::new("cmd-4", ActivateTab4, context),
        KeyBinding::new("cmd-5", ActivateTab5, context),
        KeyBinding::new("cmd-6", ActivateTab6, context),
        KeyBinding::new("cmd-7", ActivateTab7, context),
        KeyBinding::new("cmd-8", ActivateTab8, context),
        KeyBinding::new("cmd-9", ActivateLastTab, context),
    ]);
}

pub struct AppShell {
    workspace: Entity<Workspace>,
    service: Entity<ServiceState>,
    sidebar: Entity<Sidebar>,
    sidebar_open: bool,
    /// Last usable sidebar width; toggling open restores it.
    sidebar_width: f32,
    /// Width while the divider is being dragged (0 = dragged closed).
    sidebar_live: Option<f32>,
    /// Pointer x minus sidebar width at press, so the edge doesn't jump to the pointer.
    sidebar_grab: f32,
    inspector: Entity<Inspector>,
    inspector_open: bool,
    closing: Vec<ClosingTab>,
    next_generation: u64,
    tab_scroll: ScrollHandle,
    /// What the tab strip last revealed and measured.
    strip: titlebar::StripMemory,
    settings_version: Option<u64>,
    focus: FocusHandle,
    _settings: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AppShell {
    pub fn new(api: ApiClient, window: &mut Window, cx: &mut Context<Self>) -> Self {
        theme::apply(protocol::Theme::System, window, cx);
        let service = cx.new(|cx| ServiceState::new(api.clone(), cx));
        let workspace = cx.new(|cx| Workspace::new(api, service.clone(), window, cx));
        let state = workspace.read(cx).app_state_for(&workspace);
        let sidebar = cx.new(|cx| Sidebar::new(state.clone(), cx));
        let inspector = cx.new(|cx| Inspector::new(state, cx));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let refresh_service = service.clone();
        let _subscriptions = vec![
            cx.observe(&workspace, |this, workspace, cx| {
                let target = workspace.read(cx).active().map(|tab| tab.target.clone());
                this.inspector.update(cx, |inspector, cx| inspector.set_target(target, cx));
                cx.notify();
            }),
            cx.observe_in(&service, window, |this, _, window, cx| this.sync_settings(window, cx)),
            cx.subscribe(&workspace, |this, _, event: &WorkspaceEvent, cx| this.on_workspace_event(event, cx)),
            // Picks up changes made outside the app (CLI) when the window regains focus.
            cx.observe_window_activation(window, move |_, window, cx| {
                if window.is_window_active() {
                    refresh_service.update(cx, |service, cx| service.refresh(cx));
                }
            }),
            cx.observe_window_appearance(window, |_, window, cx| theme::appearance_changed(window, cx)),
        ];
        let guard = workspace.clone();
        window.on_window_should_close(cx, move |window, cx| {
            if !guard.read(cx).has_dirty_tabs(cx) {
                return true;
            }
            crate::components::toast_warning("Some tabs have unsaved changes. Save or close them first.", window, cx);
            false
        });
        Self {
            workspace,
            service,
            sidebar,
            sidebar_open: true,
            sidebar_width: SIDEBAR_WIDTH,
            sidebar_live: None,
            sidebar_grab: 0.,
            inspector,
            inspector_open: false,
            closing: Vec::new(),
            next_generation: 0,
            tab_scroll: ScrollHandle::new(),
            strip: titlebar::StripMemory::default(),
            settings_version: None,
            focus,
            _settings: None,
            _subscriptions,
        }
    }

    /// Applies the persisted theme whenever data is (re)loaded.
    fn sync_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        let service = self.service.read(cx);
        let version = service.data_version();
        if !service.status().is_ready() || self.settings_version == Some(version) {
            return;
        }
        self.settings_version = Some(version);
        let api = self.workspace.read(cx).api().clone();
        self._settings = Some(cx.spawn_in(window, async move |_, cx| {
            match api.get::<AppSettings>("/v1/settings").await {
                Ok(settings) => {
                    cx.update(|window, cx| {
                        theme::apply(settings.theme, window, cx);
                        motion::apply(settings.motion.as_ref(), cx);
                    }).ok();
                }
                Err(error) => log::warn!("settings not loaded: {error}"),
            }
        }));
    }

    fn on_workspace_event(&mut self, event: &WorkspaceEvent, cx: &mut Context<Self>) {
        let WorkspaceEvent::TabClosed { index, target, title } = event;
        let generation = self.next_generation;
        self.next_generation += 1;
        self.closing.push(ClosingTab { generation, index: *index, target: target.clone(), title: title.clone() });
        let duration = motion::FAST + Duration::from_millis(40);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            this.update(cx, |this, cx| {
                this.closing.retain(|ghost| ghost.generation != generation);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        self.workspace.update(cx, |workspace, cx| workspace.activate(index, cx));
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.service.update(cx, |service, cx| service.refresh(cx));
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }

    fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.inspector_open = !self.inspector_open;
        cx.notify();
    }

    fn navigate_history(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |workspace, cx| workspace.navigate_history(forward, window, cx));
    }

    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.workspace.read(cx).app_state_for(&self.workspace);
        quick_open::open(state, window, cx);
    }

    fn register_actions(&self, root: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        root.on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_tab(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.workspace.update(cx, |workspace, cx| workspace.close_active(window, cx));
            }))
            .on_action(cx.listener(|this, _: &ReopenClosedTab, window, cx| {
                this.workspace.update(cx, |workspace, cx| workspace.reopen_closed(window, cx));
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                this.workspace.update(cx, |workspace, cx| workspace.activate_offset(1, cx));
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| {
                this.workspace.update(cx, |workspace, cx| workspace.activate_offset(-1, cx));
            }))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ToggleInspector, _, cx| this.toggle_inspector(cx)))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.navigate_history(false, window, cx)))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| this.navigate_history(true, window, cx)))
            .on_action(cx.listener(|this, _: &ActivateTab1, _, cx| this.activate(0, cx)))
            .on_action(cx.listener(|this, _: &ActivateTab2, _, cx| this.activate(1, cx)))
            .on_action(cx.listener(|this, _: &ActivateTab3, _, cx| this.activate(2, cx)))
            .on_action(cx.listener(|this, _: &ActivateTab4, _, cx| this.activate(3, cx)))
            .on_action(cx.listener(|this, _: &ActivateTab5, _, cx| this.activate(4, cx)))
            .on_action(cx.listener(|this, _: &ActivateTab6, _, cx| this.activate(5, cx)))
            .on_action(cx.listener(|this, _: &ActivateTab7, _, cx| this.activate(6, cx)))
            .on_action(cx.listener(|this, _: &ActivateTab8, _, cx| this.activate(7, cx)))
            .on_action(cx.listener(|this, _: &ActivateLastTab, _, cx| {
                let last = this.workspace.read(cx).tabs().len().saturating_sub(1);
                this.activate(last, cx);
            }))
    }

    /// Sidebar width and the collapsed-state gutter that keeps the content card off the
    /// window edge. While dragging, the width follows the pointer 1:1 (zero-duration
    /// transition); toggles and the release settle take 120ms (`789CB4EF` ~100–150ms).
    fn sidebar_metrics(&self, window: &mut Window, cx: &mut Context<Self>) -> (f32, f32) {
        let (target, policy) = match self.sidebar_live {
            Some(live) => (live, Transition::new(Duration::ZERO)),
            None => (if self.sidebar_open { self.sidebar_width } else { 0. }, motion::fade()),
        };
        let width = transition("sidebar-width", target, policy, window, cx);
        let gutter = metrics::SHELL_INSET * (1. - width / SIDEBAR_MIN).clamp(0., 1.);
        (width, gutter)
    }

    fn render_sidebar(&self, width: f32) -> impl IntoElement {
        // Clips to the animated width. The inner sidebar keeps its settled width while
        // animating, so contents clip rather than reflow; a drag resizes it directly.
        let inner = self.sidebar_live.map_or(self.sidebar_width, |live| live.max(SIDEBAR_MIN));
        div()
            .flex_none()
            .h_full()
            .w(px(width))
            .overflow_hidden()
            .child(div().h_full().w(px(inner)).child(self.sidebar.clone()))
    }

    /// Divider between the sidebar and the card: Base's resize band (9pt hit area, column
    /// resize cursor, occludes). Its 1pt blue line shows only while hovered or dragged
    /// (`0B143E01`: a blue seam line on hover and drag).
    /// `edge`: the sidebar's on-screen width this frame (mid-animation included), so a
    /// press starts the drag exactly where the edge is drawn.
    /// `seam`: x of the card's left edge within the content area (the collapsed gutter).
    fn render_sidebar_handle(&self, seam: f32, edge: f32, cx: &mut Context<Self>) -> impl IntoElement {
        // `accent_fill` is near-black; the app's only blue is `running`, which matches the ref's seam.
        let line = theme::tokens(cx).running;
        let handle = resize_handle::<SidebarResize, SidebarResize>("sidebar-resize", Axis::Horizontal)
            .with_appearance(Rc::new(move |handle: &ResizeHandleContext, _, _| {
                let shown = handle.state() != ResizeHandleState::Idle;
                Some(div().w_px().h_full().when(shown, |el| el.bg(line)).into_any_element())
            }))
            // `0B143E01`: the seam names its gesture on hover, beside the pointer.
            .tooltip(|window, cx| Tooltip::new("Drag to resize sidebar").build(window, cx))
            .on_drag(SidebarResize, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, _| {
                    this.sidebar_grab = f32::from(event.position.x) - edge;
                }),
            )
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    this.toggle_sidebar(cx);
                }
            }));
        // Zero-width anchor on the card's left edge; the band straddles it.
        div().absolute().top_0().bottom_0().left(px(seam)).w_0().child(handle)
    }

    fn drag_sidebar(&mut self, event: &DragMoveEvent<SidebarResize>, window: &mut Window, cx: &mut Context<Self>) {
        // GPUI delivers drag moves for any mouse move while a drag is active (it checks only
        // the drag type), so a move with the button already up ends the drag at the last
        // held width instead of resizing to the free pointer.
        if event.event.pressed_button != Some(MouseButton::Left) {
            self.end_sidebar_drag(cx);
            cx.stop_active_drag(window);
            return;
        }
        let width = (f32::from(event.event.position.x) - self.sidebar_grab).clamp(0., SIDEBAR_MAX);
        if self.sidebar_live != Some(width) {
            self.sidebar_live = Some(width);
            cx.notify();
        }
    }

    fn end_sidebar_drag(&mut self, cx: &mut Context<Self>) {
        let Some(live) = self.sidebar_live.take() else { return };
        let width = sidebar_release_width(live);
        self.sidebar_open = width > 0.;
        if self.sidebar_open {
            self.sidebar_width = width;
        }
        cx.notify();
    }

    /// Docked beside the content, or, when docking would leave the primary view under
    /// `PRIMARY_MIN`, floated over the card's right side so the view keeps its width.
    fn render_inspector(&self, overlay: bool, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = if self.inspector_open { INSPECTOR_WIDTH } else { 0. };
        let width = transition("inspector-width", target, motion::standard(), window, cx);
        let panel = div().h_full().w(px(width)).overflow_hidden().child(self.inspector.clone());
        if overlay {
            panel.absolute().top_0().right_0().when(width > 0., |panel| panel.shadow_lg().occlude())
        } else {
            panel.flex_none()
        }
    }

    /// Tab layout couldn't be loaded/saved: say so and offer Retry.
    fn render_persist_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let message = self.workspace.read(cx).persist_error()?;
        let retry = banner::retry_button("retry-workspace").on_click(cx.listener(|this, _, window, cx| {
            this.workspace.update(cx, |workspace, cx| workspace.retry_persistence(window, cx));
        }));
        Some(banner::banner(message.into(), None, Tone::Attention, retry, cx))
    }

    fn render_service_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let status = self.service.read(cx).status().clone();
        let message = status.message()?;
        let title = match status {
            ServiceStatus::Incompatible(_) => "Incompatible service",
            _ => "Service unavailable",
        };
        let retry = banner::retry_button("retry-service").on_click(cx.listener(|this, _, _, cx| this.refresh(cx)));
        Some(banner::banner(message.into(), Some(title), Tone::Danger, retry, cx))
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A release over the divider never reaches the root's `on_drop` (the band
        // occludes), and leaving the window cancels the drag; either way GPUI clears the
        // active drag and re-renders, so settle here.
        if self.sidebar_live.is_some() && !cx.has_active_drag() {
            self.end_sidebar_drag(cx);
        }
        // Keep workspace shortcuts reachable after dialogs close or focused views unmount.
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
        let content = self.workspace.update(cx, |workspace, cx| workspace.content(window, cx));
        let t = theme::tokens(cx);
        let this = cx.entity().downgrade();
        let (this_new, this_back, this_forward, this_inspector) = (this.clone(), this.clone(), this.clone(), this.clone());
        let (can_back, can_forward) = {
            let workspace = self.workspace.read(cx);
            (workspace.can_go_back(), workspace.can_go_forward())
        };
        let (sidebar_width, sidebar_gutter) = self.sidebar_metrics(window, cx);
        let tabs = titlebar::render(
            &self.workspace,
            &self.closing,
            &self.tab_scroll,
            &mut self.strip,
            TabStripActions {
                on_toggle_sidebar: Box::new(move |_, cx| {
                    this.update(cx, |shell, cx| shell.toggle_sidebar(cx)).ok();
                }),
                on_new_tab: Box::new(move |window, cx| {
                    this_new.update(cx, |shell, cx| shell.new_tab(window, cx)).ok();
                }),
                on_back: can_back.then(|| -> Box<dyn Fn(&mut Window, &mut App)> {
                    Box::new(move |window, cx| {
                        this_back.update(cx, |shell, cx| shell.navigate_history(false, window, cx)).ok();
                    })
                }),
                on_forward: can_forward.then(|| -> Box<dyn Fn(&mut Window, &mut App)> {
                    Box::new(move |window, cx| {
                        this_forward.update(cx, |shell, cx| shell.navigate_history(true, window, cx)).ok();
                    })
                }),
                inspector_open: self.inspector_open,
                on_toggle_inspector: Box::new(move |_, cx| {
                    this_inspector.update(cx, |shell, cx| shell.toggle_inspector(cx)).ok();
                }),
                card_left: sidebar_width + sidebar_gutter,
            },
            window,
            cx,
        );
        let root = div()
            .id("app-shell")
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .size_full()
            .flex()
            // Body first, titlebar last: the titlebar paints above the content card so the
            // active tab's join can cover the card's top border.
            .flex_col_reverse()
            .bg(cx.theme().sidebar)
            .text_color(t.text)
            // UI default; components set explicit roles (`components::Typography`).
            .text_size(px(13.))
            .line_height(relative(1.4));
        let root = root
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<SidebarResize>, window, cx| this.drag_sidebar(event, window, cx)))
            .on_drop(cx.listener(|this, _: &SidebarResize, _, cx| this.end_sidebar_drag(cx)));
        self.register_actions(root, cx).child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(self.render_sidebar(sidebar_width))
                // Single owner of the card's frame insets: right and bottom always, left
                // only as the sidebar collapses. The card stretches to fill what's left.
                // The divider sits at the card's left edge (after the gutter), so when
                // collapsed it is inside the window, clear of macOS's resize edge.
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .relative()
                        .pl(px(sidebar_gutter))
                        .pr(px(metrics::SHELL_INSET))
                        .pb(px(metrics::SHELL_INSET))
                        .child(self.render_card(content, sidebar_width + sidebar_gutter, window, cx))
                        // Last, so its band hit-tests above the card's edge content.
                        .child(self.render_sidebar_handle(sidebar_gutter, sidebar_width, cx)),
                ),
        )
        .child(tabs)
    }
}

impl AppShell {
    /// `card_left`: x of the card's left edge (live sidebar width plus gutter).
    fn render_card(&self, content: AnyView, card_left: f32, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let card_width = f32::from(window.viewport_size().width) - card_left - metrics::SHELL_INSET;
        let overlay = card_width - INSPECTOR_WIDTH < PRIMARY_MIN;
        let body = div()
            .flex()
            .flex_col()
            .size_full()
            .rounded(px(metrics::R_CARD))
            // Transparent: the frame below paints the visible border above the content.
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .bg(t.canvas)
            .overflow_hidden()
            .children(self.render_service_banner(cx))
            .children(self.render_persist_banner(cx))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().h_full().child(content))
                    .child(self.render_inspector(overlay, window, cx)),
            );
        div()
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(body)
            .child(crate::components::rounded_frame(metrics::R_CARD, cx.theme().sidebar, t.border))
    }
}
