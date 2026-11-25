//! Titlebar with workspace tabs (refs `5EEF77CF`, `EB27363B`, `789CB4EF`).
//!
//! The active tab is a folder tab joined to the content card below it: canvas fill, border
//! on top/left/right, and concave 8pt flares into the card's top edge. Inactive tabs show a
//! hover pill and 1pt dividers. When the strip overflows, ‹/› buttons and 24pt fades mark
//! the hidden side, and activating a tab scrolls it (flares included) clear of them.
//! Behaviour: close on hover/active, dirty dot, middle-click close, drag reorder, overflow
//! menu, `+` → Quick Open; opening a tab expands it in, closing collapses a ghost out.

use gpui_kit::base::motion::Sequence;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Side, TitleBar,
    tooltip::Tooltip,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::TabKind;

use crate::{
    components::{Typography as _, glyph_item, icon_button, menu_popover},
    motion,
    state::{TabTarget, Workspace},
    theme::{self, metrics},
};

const TAB_H: f32 = metrics::TAB_H;
const TAB_MAX_WIDTH: f32 = 200.;
/// Narrowest the active tab shrinks to in a cramped strip (icon, a few letters, close).
const MIN_ACTIVE_TAB: f32 = 96.;
const TAB_RADIUS: f32 = metrics::R_INNER;
/// Concave join between the active tab and the content card.
const FLARE: f32 = 8.;
/// Interactive tab face, centred in the bar like the chrome; the foot below it is paint only.
const TAB_FACE: f32 = TAB_H - FLARE;
/// Opaque run after the overflow arrow, so no glyph fragments sit beside it.
const EDGE_SOLID: f32 = 8.;
/// Gradient run from opaque to clear.
const EDGE_FADE: f32 = 40.;
/// Fraction of the fade kept opaque, so a hidden tab's icon or first letters never show
/// as a fragment beside the arrow; only the last part of the run fades.
const EDGE_FADE_OPAQUE: f32 = 0.5;
/// Arrow + opaque run + fade: tabs inside this zone are never treated as visible.
const EDGE: f32 = metrics::CONTROL_MD + EDGE_SOLID + EDGE_FADE;
/// Space kept between a revealed tab's flare and the strip edge controls.
const REVEAL_MARGIN: f32 = 8.;
/// Strip inset so the first/last tab's flares land on the card's flat top edge.
const STRIP_PAD: f32 = metrics::R_CARD + FLARE;
/// Width Kit reserves for the traffic lights.
const LIGHTS: f32 = 80.;
/// Leading controls (sidebar toggle, back, forward) after the traffic lights.
const LEADING_MIN: f32 = 100.;

pub fn tab_icon(kind: &TabKind) -> IconName {
    match kind {
        TabKind::Project => IconName::Folder,
        TabKind::Task => IconName::SquareCheckBig,
        TabKind::Document => IconName::FileText,
        TabKind::Review => IconName::GitPullRequest,
        TabKind::Unknown(_) => IconName::TriangleAlert,
    }
}

/// A tab that was just closed and is collapsing out.
pub struct ClosingTab {
    pub generation: u64,
    pub index: usize,
    pub target: TabTarget,
    pub title: SharedString,
}

#[derive(Clone)]
pub struct DraggedTab {
    index: usize,
    title: SharedString,
    kind: TabKind,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        tab_pill(cx)
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .opacity(0.95)
            .shadow_md()
            .child(Icon::new(tab_icon(&self.kind)).size_3p5().text_color(t.text_secondary))
            .child(div().truncate().max_w(px(160.)).child(self.title.clone()))
    }
}

/// Label row shared by every tab state so widths never shift between states.
fn tab_pill(cx: &App) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .h(px(TAB_FACE))
        .px_2()
        .rounded(px(metrics::R_CONTROL))
        .t_label()
        .text_color(theme::tokens(cx).text)
}

type Handler = Box<dyn Fn(&mut Window, &mut App)>;

pub struct TabStripActions {
    pub on_toggle_sidebar: Handler,
    pub on_new_tab: Handler,
    /// `None` disables the button (no history in the active tab).
    pub on_back: Option<Handler>,
    pub on_forward: Option<Handler>,
    pub inspector_open: bool,
    pub on_toggle_inspector: Handler,
    /// Window x where the content card starts; the strip aligns to it.
    pub card_left: f32,
}

fn chrome_button(id: &'static str, icon: IconName) -> Button {
    icon_button(Button::new(id).ghost().icon(Icon::new(icon)), metrics::CONTROL_MD)
}

fn nav_button(id: &'static str, icon: IconName, tooltip: &'static str, handler: Option<Handler>) -> Button {
    let button = chrome_button(id, icon).tooltip(tooltip);
    match handler {
        Some(handler) => button.on_click(move |_, window, cx| handler(window, cx)),
        None => button.disabled(true),
    }
}

/// Strip state the shell keeps between frames.
#[derive(Default)]
pub struct StripMemory {
    /// Active tab id once it has been revealed at full width with a valid layout.
    revealed: Option<SharedString>,
    /// Scroll extent and viewport width that reveal was judged against. If either changes
    /// (tabs finished animating, window resized), the reveal is pending again.
    extent: (f32, f32),
}

/// Which strip edges hide tabs, from the last layout.
#[derive(Clone, Copy)]
struct Overflow {
    before: bool,
    after: bool,
}

fn overflow(scroll: &ScrollHandle) -> Overflow {
    let max = f32::from(scroll.max_offset().x);
    let offset = -f32::from(scroll.offset().x);
    Overflow { before: max > 0.5 && offset > 0.5, after: max > 0.5 && offset < max - 0.5 }
}

/// Outcome of one reveal attempt against the last measured layout.
#[derive(Clone, Copy, PartialEq)]
enum Reveal {
    /// No usable bounds yet; retry next frame.
    Unmeasured,
    /// Offset changed; the next frame re-measures arrows and the connected shape.
    Moved,
    /// Already as visible as the scroll range allows.
    Done,
}

/// Scrolls so child `ix` (with its flares and a margin) sits clear of whichever edge
/// controls will be showing at the new offset. At either end of the range that side has
/// no arrow or fade, so the strip padding alone provides the clearance.
fn reveal(scroll: &ScrollHandle, ix: usize) -> Reveal {
    let (Some(item), viewport) = (scroll.bounds_for_item(ix), scroll.bounds()) else { return Reveal::Unmeasured };
    if f32::from(viewport.size.width) <= 0. || f32::from(item.size.width) <= 0. {
        return Reveal::Unmeasured;
    }
    let pad = FLARE + REVEAL_MARGIN;
    let current = f32::from(scroll.offset().x);
    let plan = plan_reveal(
        (f32::from(item.left()) - pad, f32::from(item.right()) + pad),
        (f32::from(viewport.left()), f32::from(viewport.right())),
        current,
        f32::from(scroll.max_offset().x).max(0.),
    );
    match plan {
        Some(next) => {
            scroll.set_offset(point(px(next), px(0.)));
            Reveal::Moved
        }
        None => Reveal::Done,
    }
}

/// Offset that reveals `span` (item plus flares and margin) inside `view`, or `None` when
/// the current offset is already the answer. Edge controls (`EDGE`) count only on sides
/// that hide content at that offset. A span too wide to fit between both edges always
/// aligns to the leading edge. That target doesn't depend on `current`, so it settles
/// after one move instead of alternating between left and right alignment.
fn plan_reveal(span: (f32, f32), view: (f32, f32), current: f32, max: f32) -> Option<f32> {
    let (left, right) = span;
    let (view_left, view_right) = view;
    let fits = |offset: f32| {
        let before = offset < -0.5;
        let after = -offset < max - 0.5;
        left + offset >= view_left + if before { EDGE } else { 0. } - 0.5
            && right + offset <= view_right - if after { EDGE } else { 0. } + 0.5
    };
    if fits(current) {
        return None;
    }
    let too_wide = right - left > view_right - view_left - 2. * EDGE;
    let next = if too_wide || left + current < view_left + EDGE {
        view_left + EDGE - left
    } else {
        view_right - EDGE - right
    };
    let next = next.clamp(-max, 0.);
    ((next - current).abs() > 0.5).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::plan_reveal;

    #[test]
    fn reveal_settles_when_the_tab_cannot_fit_between_both_edges() {
        // 300pt strip, 200pt tab at 300..500 (plus flares/margin): both 56pt edges can't fit.
        let span = (300. - 16., 500. + 16.);
        let (view, max) = ((0., 300.), 1000.);
        let first = plan_reveal(span, view, 0., max).expect("moves once");
        assert_eq!(plan_reveal(span, view, first, max), None);
    }
}

/// Widest the active tab may be: its normal maximum, or narrower when the strip can't
/// fit it between both edge controls (with flares and margins), so its label and close
/// stay usable at narrow widths. Depends only on the strip's width, never the tabs, so it
/// can't oscillate with the reveal.
fn active_tab_max(scroll: &ScrollHandle) -> f32 {
    let viewport = f32::from(scroll.bounds().size.width);
    if viewport <= 0. {
        return TAB_MAX_WIDTH;
    }
    (viewport - 2. * (EDGE + FLARE + REVEAL_MARGIN)).clamp(MIN_ACTIVE_TAB, TAB_MAX_WIDTH)
}

/// Whether child `ix` is fully visible (flares included) inside the strip.
fn fully_visible(scroll: &ScrollHandle, ix: usize) -> bool {
    let Some(item) = scroll.bounds_for_item(ix) else { return false };
    let viewport = scroll.bounds();
    let offset = f32::from(scroll.offset().x);
    let state = overflow(scroll);
    let view_left = f32::from(viewport.left()) + if state.before { EDGE } else { 0. };
    let view_right = f32::from(viewport.right()) - if state.after { EDGE } else { 0. };
    f32::from(item.left()) + offset - FLARE >= view_left - 0.5
        && f32::from(item.right()) + offset + FLARE <= view_right + 0.5
}

fn scroll_by(scroll: &ScrollHandle, direction: f32) {
    let max = f32::from(scroll.max_offset().x);
    let step = (f32::from(scroll.bounds().size.width) * 0.6).max(120.);
    let next = (f32::from(scroll.offset().x) - direction * step).clamp(-max, 0.);
    scroll.set_offset(point(px(next), px(0.)));
}

/// `scroll`: the strip's scroll handle. A newly active tab stays pending in `memory` until
/// it has been revealed at full width against a measured layout (on launch the first
/// frame has no bounds yet), so it is never left clipped under the arrows.
pub fn render(
    workspace: &Entity<Workspace>,
    closing: &[ClosingTab],
    scroll: &ScrollHandle,
    memory: &mut StripMemory,
    actions: TabStripActions,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let bar = cx.theme().title_bar;
    let (items, active) = {
        let ws = workspace.read(cx);
        let active = ws.active_index();
        let items: Vec<_> = ws
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| (index, tab.target.clone(), tab.title.clone(), tab.is_dirty(cx)))
            .collect();
        (items, active)
    };
    // Child index of each tab, counting collapsing ghosts inserted before it.
    let child_index = |index: usize| index + closing.iter().filter(|ghost| ghost.index.min(items.len()) <= index).count();
    let connected = active.is_some_and(|ix| fully_visible(scroll, child_index(ix)));
    let active_max = active_tab_max(scroll);

    let mut tabs: Vec<(Option<f32>, AnyElement)> = items
        .into_iter()
        .map(|(index, target, title, dirty)| {
            let state = TabState {
                max_width: if active == Some(index) { active_max } else { TAB_MAX_WIDTH },
                is_active: active == Some(index),
                after_active: index > 0 && active == Some(index - 1),
                first: index == 0,
                connected,
                dirty,
            };
            let (tab, progress) = render_tab(workspace, index, target, title, state, window, cx);
            (state.is_active.then_some(progress), tab.into_any_element())
        })
        .collect();
    for ghost in closing.iter().rev() {
        let position = ghost.index.min(tabs.len());
        tabs.insert(position, (None, render_ghost(ghost, window, cx).into_any_element()));
    }
    let active_id: Option<SharedString> =
        active.and_then(|ix| workspace.read(cx).tabs().get(ix).map(|tab| tab.target.tab_id().into()));
    let extent = (f32::from(scroll.max_offset().x), f32::from(scroll.bounds().size.width));
    if (extent.0 - memory.extent.0).abs() > 0.5 || (extent.1 - memory.extent.1).abs() > 0.5 {
        memory.extent = extent;
        memory.revealed = None;
    }
    // Frames are requested only while a reveal is in progress; idle and manually scrolled
    // strips stay quiet.
    // A layout change (resize, tabs settling) is first seen with last frame's bounds, so
    // judge the reveal again next frame rather than settling on stale geometry.
    let mut needs_frame = memory.revealed.is_none() && active.is_some();
    match tabs.iter().enumerate().find_map(|(ix, (active, _))| active.map(|p| (ix, p))) {
        Some((child, progress)) if memory.revealed != active_id || progress < 1.0 => {
            match reveal(scroll, child) {
                Reveal::Done if progress >= 1.0 => memory.revealed = active_id.clone(),
                _ => needs_frame = true,
            }
        }
        Some(_) => {}
        None => memory.revealed = None,
    }
    let tabs = tabs.into_iter().map(|(_, tab)| tab);
    let edges = overflow(scroll);
    if needs_frame {
        window.request_animation_frame();
    }

    let toggle = actions.on_toggle_sidebar;
    let new_tab = actions.on_new_tab;
    let toggle_inspector = actions.on_toggle_inspector;
    let leading_width = (actions.card_left - LIGHTS).max(LEADING_MIN);
    TitleBar::new()
        .h(px(metrics::TITLEBAR_H))
        .bg(bar)
        .border_b_0()
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .items_end()
                .pr_2()
                // Controls centre on the full bar (as the traffic lights do); only the tab
                // strip sits in the bottom tab row.
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_0p5()
                        .h_full()
                        .w(px(leading_width))
                        .child(
                            chrome_button("toggle-sidebar", IconName::PanelLeft)
                                .tooltip("Toggle sidebar (⌘\\)")
                                .on_click(move |_, window, cx| toggle(window, cx)),
                        )
                        .child(nav_button("go-back", IconName::ArrowLeft, "Back (⌘[)", actions.on_back))
                        .child(nav_button("go-forward", IconName::ArrowRight, "Forward (⌘])", actions.on_forward)),
                )
                // Kit's TitleBar wraps children in a `flex_shrink_0` row whose minimum width
                // is its content's min-content width, so an in-flow strip would widen the
                // whole bar past the window instead of scrolling. The strip is absolutely
                // positioned inside a flexible slot so it never contributes intrinsic width.
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(
                            div()
                                .id("tab-strip")
                                .track_scroll(scroll)
                                .absolute()
                                .top(px(metrics::TITLEBAR_H - TAB_H))
                                .left_0()
                                .right_0()
                                // GPUI's x-scroll mask also clips y to the element bounds
                                // (`Style::overflow_mask`), so the strip reaches 1pt into the
                                // card row where the active tab's join hangs.
                                .bottom(px(-1.))
                                .pb(px(1.))
                                .flex()
                                .items_end()
                                .pl(px(STRIP_PAD))
                                .overflow_x_scroll()
                                .children(tabs)
                                // Trailing space as a child (not padding) so it is part of the
                                // scroll range and the last tab's flare can clear the end.
                                .child(div().flex_none().w(px(STRIP_PAD)).h(px(TAB_H))),
                        )
                        .when(edges.before, |slot| slot.child(edge_control(EdgeSide::Before, scroll, bar)))
                        .when(edges.after, |slot| slot.child(edge_control(EdgeSide::After, scroll, bar))),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_0p5()
                        .h_full()
                        .pl_0p5()
                        .child(
                            chrome_button("new-tab", IconName::Plus)
                                .tooltip("Open… (⌘T)")
                                .on_click(move |_, window, cx| new_tab(window, cx)),
                        )
                        .child(overflow_menu(workspace, cx))
                        .child(
                            chrome_button("toggle-inspector", IconName::PanelRight)
                                .selected(actions.inspector_open)
                                .tooltip("Inspector (⌥⌘I)")
                                .on_click(move |_, window, cx| toggle_inspector(window, cx)),
                        ),
                ),
        )
        .text_color(t.text)
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq)]
enum EdgeSide {
    Before,
    After,
}

/// Overflow arrow on the bar colour with a fade over the tabs next to it.
fn edge_control(side: EdgeSide, scroll: &ScrollHandle, bar: Hsla) -> impl IntoElement {
    let before = side == EdgeSide::Before;
    let (id, icon, direction) = if before {
        ("tab-scroll-before", IconName::ChevronLeft, -1.)
    } else {
        ("tab-scroll-after", IconName::ChevronRight, 1.)
    };
    let handle = scroll.clone();
    // Same 28 hitbox and 16 glyph as the neighbouring chrome buttons, so the gap to them
    // matches theirs (the trailing cluster adds the same 2pt gap).
    let button = chrome_button(id, icon)
        .on_click(move |_, window, _| {
            scroll_by(&handle, direction);
            window.refresh();
        });
    let solid = div().flex().flex_none().items_center().h_full().pr(px(EDGE_SOLID)).bg(bar).child(button);
    let solid = if before { solid } else { solid.pr_0().pl(px(EDGE_SOLID)) };
    // 90° runs left → right (0° is up); the fade is opaque next to the solid run.
    let fade = div().w(px(EDGE_FADE)).h_full().bg(if before {
        linear_gradient(90., linear_color_stop(bar, EDGE_FADE_OPAQUE), linear_color_stop(bar.opacity(0.), 1.))
    } else {
        linear_gradient(90., linear_color_stop(bar.opacity(0.), 0.), linear_color_stop(bar, 1. - EDGE_FADE_OPAQUE))
    });
    // Occludes: the solid run and fade sit over tabs, which must not receive hover,
    // tooltips or clicks through them.
    let row = div().absolute().top_0().bottom_0().flex().occlude();
    if before { row.left_0().child(solid).child(fade) } else { row.right_0().child(fade).child(solid) }
}

#[derive(Clone, Copy)]
struct TabState {
    max_width: f32,
    is_active: bool,
    after_active: bool,
    first: bool,
    /// The active tab is fully visible, so it can join the card.
    connected: bool,
    dirty: bool,
}

fn render_tab(
    workspace: &Entity<Workspace>,
    index: usize,
    target: TabTarget,
    title: SharedString,
    state: TabState,
    window: &mut Window,
    cx: &mut App,
) -> (impl IntoElement + use<>, f32) {
    let t = theme::tokens(cx);
    let bar = cx.theme().title_bar;
    let key: SharedString = target.tab_id().into();
    let progress = *Sequence::new(ElementId::Name(format!("tab-open:{key}").into()), 0.0f32)
        .with_step(1.0, motion::tab())
        .sample(window, cx)
        .value();
    let group: SharedString = format!("tab-group:{key}").into();
    let (activate_ws, close_ws, middle_ws, drop_ws) =
        (workspace.downgrade(), workspace.downgrade(), workspace.downgrade(), workspace.downgrade());
    let drag = DraggedTab { index, title: title.clone(), kind: target.kind.clone() };
    let is_active = state.is_active;

    let close_button = icon_button(
        Button::new(ElementId::Name(format!("close:{key}").into())).ghost().icon(Icon::new(IconName::X)),
        metrics::CONTROL_XS,
    )
    // Concentric with the R6 tab pill it sits 2pt inside.
    .rounded(px(metrics::R_CONTROL - 2.))
        .tooltip("Close (⌘W)")
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            close_ws.update(cx, |ws, cx| ws.request_close(index, window, cx)).ok();
        });

    let trailing = div().flex_none().size(px(metrics::CONTROL_XS)).flex().items_center().justify_center();
    let trailing = if is_active {
        trailing.child(close_button)
    } else if state.dirty {
        trailing
            .child(div().group_hover(group.clone(), |s| s.invisible()).child(dirty_dot(cx)))
            .child(div().absolute().invisible().group_hover(group.clone(), |s| s.visible()).child(close_button))
            .relative()
    } else {
        trailing.child(div().invisible().group_hover(group.clone(), |s| s.visible()).child(close_button))
    };

    // The close slot sits 2pt from the pill's trailing edge (as from its top and bottom),
    // so its hover square is evenly inset rather than flush with the pill.
    let pill = tab_pill(cx)
        .pr(px(2.))
        .flex_1()
        .min_w_0()
        .when(is_active, |pill| pill.font_weight(FontWeight::MEDIUM))
        .when(!is_active, |pill| pill.text_color(t.text_secondary).group_hover(group.clone(), |s| s.bg(t.hover).text_color(t.text)))
        .child(Icon::new(tab_icon(&target.kind)).size_3p5().text_color(t.text_secondary))
        .child(div().flex_1().min_w_0().truncate().child(title.clone()))
        .when(state.dirty && is_active, |pill| pill.child(dirty_dot(cx)))
        .child(trailing);

    let show_divider = !is_active && !state.first && !state.after_active;
    let tab = div()
        .id(ElementId::Name(key.clone()))
        .group(group.clone())
        .relative()
        .flex()
        .flex_none()
        .items_start()
        .h(px(TAB_H))
        .px(px(3.))
        .max_w(px(state.max_width * progress))
        .opacity(progress)
        .when(is_active && state.connected, |tab| {
            // GPUI paints a div's border after its children, so the tab's own side borders
            // would run over the flares. A child owns the stroke and stops where they begin.
            tab.bg(t.canvas)
                .rounded_t(px(TAB_RADIUS))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom(px(FLARE - 1.))
                        .rounded_t(px(TAB_RADIUS))
                        .border_t_1()
                        .border_l_1()
                        .border_r_1()
                        .border_color(t.border),
                )
                .child(join_to_card(t, bar))
        })
        .when(is_active && !state.connected, |tab| {
            tab.child(div().absolute().top_0().h(px(TAB_FACE)).left(px(3.)).right(px(3.)).rounded(px(metrics::R_CONTROL)).bg(t.surface).border_1().border_color(t.border))
        })
        .when(!is_active, |tab| tab.overflow_hidden())
        .when(show_divider, |tab| {
            tab.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px((TAB_FACE - 14.) / 2.))
                    .h(px(14.))
                    .w_px()
                    .bg(t.border)
                    .group_hover(group.clone(), |s| s.invisible()),
            )
        })
        .child(pill)
        .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
        .on_click(move |_, _, cx| {
            activate_ws.update(cx, |ws, cx| ws.activate(index, cx)).ok();
        })
        .on_mouse_down(MouseButton::Middle, move |_, window, cx| {
            middle_ws.update(cx, |ws, cx| ws.request_close(index, window, cx)).ok();
        })
        .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
        .drag_over::<DraggedTab>(move |style, _, _, _| style.bg(t.selected))
        .on_drop(move |dragged: &DraggedTab, _, cx| {
            drop_ws.update(cx, |ws, cx| ws.move_tab(dragged.index, index, cx)).ok();
        });
    (tab, progress)
}

/// Flares and the 1pt cover over the card's top border, hanging 1pt below the tab. The
/// shell paints the titlebar after the body (`flex_col_reverse`), so they sit above the
/// card's border. Only rendered while the active tab is fully visible.
fn join_to_card(t: theme::Tokens, bar: Hsla) -> impl IntoElement {
    // A canvas square under a bar-coloured piece with one rounded corner reads as a
    // concave curve from the tab side into the card's top edge.
    // GPUI clamps a radius to half its quad, so the curve is a 2×FLARE box whose rounded
    // corner is the only part the FLARE-sized piece shows. The piece overlaps the tab's
    // outer 1pt column, so the curve's stroke continues the side stroke above it.
    let flare = |left: bool| {
        let curve = div().absolute().bottom_0().size(px(2. * FLARE)).bg(bar).border_b_1().border_color(t.border);
        let curve = if left {
            curve.right_0().rounded_br(px(FLARE)).border_r_1()
        } else {
            curve.left_0().rounded_bl(px(FLARE)).border_l_1()
        };
        div()
            .absolute()
            .bottom(px(-1.))
            .size(px(FLARE))
            .overflow_hidden()
            .bg(t.canvas)
            .map(|piece| if left { piece.left(px(1. - FLARE)) } else { piece.right(px(1. - FLARE)) })
            .child(curve)
    };
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h_0()
        .child(div().absolute().left_0().right_0().bottom(px(-1.)).h_px().bg(t.canvas))
        .child(flare(true))
        .child(flare(false))
}

fn render_ghost(ghost: &ClosingTab, window: &mut Window, cx: &mut App) -> impl IntoElement {
    let t = theme::tokens(cx);
    let progress = *Sequence::new(ElementId::Name(format!("tab-close:{}", ghost.generation).into()), 1.0f32)
        .with_step(0.0, motion::tab())
        .sample(window, cx)
        .value();
    div()
        .flex()
        .flex_none()
        .items_start()
        .h(px(TAB_H))
        .px(px(3.))
        .max_w(px(TAB_MAX_WIDTH * progress))
        .opacity(progress)
        .overflow_hidden()
        .child(
            tab_pill(cx)
                .text_color(t.text_secondary)
                .child(Icon::new(tab_icon(&ghost.target.kind)).size_3p5())
                .child(div().truncate().child(ghost.title.clone())),
        )
}

fn dirty_dot(cx: &App) -> Div {
    div().size(px(6.)).rounded_full().bg(theme::tokens(cx).text_secondary)
}

fn overflow_menu(workspace: &Entity<Workspace>, cx: &App) -> impl IntoElement {
    let ws = workspace.clone();
    let has_tabs = !workspace.read(cx).tabs().is_empty();
    let trigger = chrome_button("tab-overflow", IconName::ChevronDown).tooltip("All tabs").disabled(!has_tabs);
    // Right-aligned under the trigger, a shell inset below it so it clears the card's top.
    menu_popover("tab-overflow-menu", trigger, Anchor::TopRight, px(metrics::SHELL_INSET), move |mut menu, _, cx| {
        let tabs: Vec<_> = ws.read(cx).tabs().iter().map(|tab| (tab.title.clone(), tab.target.kind.clone())).collect();
        let active = ws.read(cx).active_index();
        // Check on the right so the current tab keeps its kind icon.
        menu = menu.check_side(Side::Right);
        for (index, (title, kind)) in tabs.into_iter().enumerate() {
            let ws = ws.downgrade();
            menu = menu.item(glyph_item(tab_icon(&kind), title).checked(active == Some(index)).on_click(move |_, _, cx| {
                ws.update(cx, |ws, cx| ws.activate(index, cx)).ok();
            }));
        }
        menu
    })
}
