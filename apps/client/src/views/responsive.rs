//! Layout helpers for views that adapt to their *allocated* width (sidebar, inspector and
//! splits change it, so window size is the wrong input), plus a scroll-edge fade.

use std::{cell::Cell, rc::Rc};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Last laid-out width of a container, read on the next render.
pub type WidthCell = Rc<Cell<Option<Pixels>>>;

/// Records its parent's width after layout (parent must be `relative()`); a change requests
/// the next frame so the view re-renders with the new value (the `TaskDetail` pattern).
pub fn width_probe(cell: &WidthCell) -> impl IntoElement + use<> {
    let cell = cell.clone();
    canvas(
        move |bounds, window, _| {
            let width = Some(bounds.size.width);
            if cell.get() != width {
                cell.set(width);
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

/// True once the measured width is below `threshold` (unmeasured counts as wide).
pub fn narrower_than(cell: &WidthCell, threshold: f32) -> bool {
    cell.get().is_some_and(|width| width < px(threshold))
}

/// Overlays for a horizontal scroll area (place in a `relative()` wrapper): a 32pt fade in
/// `background` on each side that still has content past the edge.
pub fn x_scroll_fades(handle: &ScrollHandle, background: Hsla) -> Vec<Div> {
    let (offset, max) = (handle.offset().x, handle.max_offset().x);
    let fade = |left: bool| {
        let (from, to) = (background.opacity(0.), background);
        div().absolute().top_0().bottom_0().w(px(32.))
            .map(|this| if left { this.left_0() } else { this.right_0() })
            .bg(linear_gradient(if left { 270. } else { 90. }, linear_color_stop(from, 0.), linear_color_stop(to, 1.)))
    };
    let mut fades = Vec::new();
    if offset < px(0.) {
        fades.push(fade(true));
    }
    if max > px(0.) && -offset < max {
        fades.push(fade(false));
    }
    fades
}
