//! Viewport and edge geometry ported from React Flow's `@xyflow/system` 1.0.1 (the version
//! `@xyflow/react` 13.0.3 depends on), inspected from the published npm package source:
//! `src/utils/edges/bezier-edge.ts` (getBezierPath, getBezierEdgeCenter, control offsets),
//! `src/utils/general.ts` (getViewportForBounds), `src/xyhandle/utils.ts` (getClosestHandle),
//! `src/xypanzoom/utils.ts` + `eventhandler.ts` (wheelDelta, pinch zoom), and the default
//! `ArrowClosed` marker from `src/utils/marker.ts`, and `calcAutoPan` from
//! `src/utils/general.ts` (edge auto-pan during node and connection drags). Only the algorithms are adapted to Rust;
//! no React or DOM code runs here.
//!
//! MIT License
//!
//! Copyright (c) 2019-2025 webkid GmbH
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all
//! copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! SOFTWARE.

/// Point in flow (graph) or canvas-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Xy {
    pub x: f32,
    pub y: f32,
}

pub const fn xy(x: f32, y: f32) -> Xy {
    Xy { x, y }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Left,
    Top,
    Right,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn union(self, other: Rect) -> Rect {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Rect { x, y, width: right - x, height: bottom - y }
    }

    pub fn contains_rect(self, other: Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width <= self.x + self.width
            && other.y + other.height <= self.y + self.height
    }
}

/// Pan offset (canvas-local px) and zoom. `screen = flow * zoom + (x, y)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self { x: 0., y: 0., zoom: 1. }
    }
}

/// React Flow defaults (`maxZoom`, `connectionRadius`, edge `interactionWidth / 2`,
/// `reconnectRadius`). `minZoom` is the configurable prop lowered from its 0.5 default: a
/// left-to-right workflow is wide, and fit view clamps to `minZoom`, so at 0.5 it could not
/// contain a five-stage graph in a narrow canvas.
pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 2.;
pub const CONNECTION_RADIUS: f32 = 20.;
pub const EDGE_HIT_RADIUS: f32 = 10.;
pub const RECONNECT_RADIUS: f32 = 10.;

impl Viewport {
    pub fn to_screen(self, p: Xy) -> Xy {
        xy(p.x * self.zoom + self.x, p.y * self.zoom + self.y)
    }

    pub fn to_flow(self, p: Xy) -> Xy {
        xy((p.x - self.x) / self.zoom, (p.y - self.y) / self.zoom)
    }

    /// Zoom to `zoom` keeping the flow point under `anchor` (canvas-local) fixed, as d3-zoom's
    /// `scaleTo(selection, k, point)` does.
    pub fn zoom_at(self, zoom: f32, anchor: Xy) -> Viewport {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let flow = self.to_flow(anchor);
        Viewport { x: anchor.x - flow.x * zoom, y: anchor.y - flow.y * zoom, zoom }
    }
}

/// `wheelDelta`: pixel wheel deltas scale by 0.002, line deltas by 0.05; pinch gestures that
/// arrive as ctrl+wheel on macOS are 10× stronger. The new zoom is `zoom * 2^delta`.
pub fn wheel_zoom(zoom: f32, delta_y: f32, lines: bool, pinch: bool) -> f32 {
    let factor = if lines { 0.05 } else { 0.002 };
    let boost = if pinch { 10. } else { 1. };
    zoom * 2f32.powf(-delta_y * factor * boost)
}

fn auto_pan_velocity(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        (value - min).abs().clamp(1., min) / min
    } else if value > max {
        -(value - max).abs().clamp(1., min) / min
    } else {
        0.
    }
}

/// `calcAutoPan` with React Flow's defaults (speed 15 px/frame, 40px edge distance): how far
/// to pan this frame when the pointer (canvas-local) is near or past an edge.
pub fn auto_pan(pointer: Xy, width: f32, height: f32) -> Xy {
    const SPEED: f32 = 15.;
    const DISTANCE: f32 = 40.;
    xy(
        auto_pan_velocity(pointer.x, DISTANCE, width - DISTANCE) * SPEED,
        auto_pan_velocity(pointer.y, DISTANCE, height - DISTANCE) * SPEED,
    )
}

/// `getViewportForBounds` with equal pixel padding on all sides.
pub fn viewport_for_bounds(bounds: Rect, width: f32, height: f32, padding: f32, max_zoom: f32) -> Viewport {
    let x_zoom = (width - padding * 2.) / bounds.width.max(1.);
    let y_zoom = (height - padding * 2.) / bounds.height.max(1.);
    let zoom = x_zoom.min(y_zoom).clamp(MIN_ZOOM, max_zoom);
    let center = xy(bounds.x + bounds.width / 2., bounds.y + bounds.height / 2.);
    Viewport { x: width / 2. - center.x * zoom, y: height / 2. - center.y * zoom, zoom }
}

fn control_offset(distance: f32, curvature: f32) -> f32 {
    if distance >= 0. { 0.5 * distance } else { curvature * 25. * (-distance).sqrt() }
}

fn control_with_curvature(side: Side, from: Xy, to: Xy, curvature: f32) -> Xy {
    match side {
        Side::Left => xy(from.x - control_offset(from.x - to.x, curvature), from.y),
        Side::Right => xy(from.x + control_offset(to.x - from.x, curvature), from.y),
        Side::Top => xy(from.x, from.y - control_offset(from.y - to.y, curvature)),
        Side::Bottom => xy(from.x, from.y + control_offset(to.y - from.y, curvature)),
    }
}

/// Cubic Bézier of a default React Flow edge, plus its label centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bezier {
    pub from: Xy,
    pub c1: Xy,
    pub c2: Xy,
    pub to: Xy,
}

/// `getBezierPath` with React Flow's default curvature 0.25.
pub fn bezier(from: Xy, from_side: Side, to: Xy, to_side: Side, zoom: f32) -> Bezier {
    const CURVATURE: f32 = 0.25;
    if from_side == Side::Left && to_side == Side::Right && !(0. ..=RETURN_SPAN * zoom).contains(&(from.x - to.x)) {
        return return_arch(from, to, zoom);
    }
    Bezier {
        from,
        c1: control_with_curvature(from_side, from, to, CURVATURE),
        c2: control_with_curvature(to_side, to, from, CURVATURE),
        to,
    }
}

/// Widest flow-space gap a return (source left border → target right border) crosses directly.
/// Narrower than a node, so nothing can sit in between.
const RETURN_SPAN: f32 = 150.;

/// Karabiner addition (not React Flow): a return to the adjacent node is a plain short
/// Bézier, but a self-return or one that skips a node would cut through a node body. Each
/// control reaches outward past its own handle and above the row, so the edge arcs over
/// and enters the target's right-hand input from outside.
fn return_arch(from: Xy, to: Xy, zoom: f32) -> Bezier {
    // Flow-space sizes, scaled like the nodes; the peak (¾ of the lift) clears the node tops.
    let (reach, lift) = (40. * zoom, 64. * zoom);
    Bezier { from, c1: xy(from.x - reach, from.y - lift), c2: xy(to.x + reach, to.y - lift), to }
}

impl Bezier {
    /// `getBezierEdgeCenter`: the curve at t = 0.5.
    pub fn center(self) -> Xy {
        xy(
            self.from.x * 0.125 + self.c1.x * 0.375 + self.c2.x * 0.375 + self.to.x * 0.125,
            self.from.y * 0.125 + self.c1.y * 0.375 + self.c2.y * 0.375 + self.to.y * 0.125,
        )
    }

    pub fn at(self, t: f32) -> Xy {
        let u = 1. - t;
        let (a, b, c, d) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
        xy(
            a * self.from.x + b * self.c1.x + c * self.c2.x + d * self.to.x,
            a * self.from.y + b * self.c1.y + c * self.c2.y + d * self.to.y,
        )
    }

    /// Approximate distance from `p` to the curve (DOM hit-testing uses the stroked path).
    pub fn distance(self, p: Xy) -> f32 {
        (0..=32).map(|i| distance(self.at(i as f32 / 32.), p)).fold(f32::MAX, f32::min)
    }

    /// Unit direction the curve travels as it reaches `to` (orients the end marker).
    pub fn end_direction(self) -> Xy {
        let (dx, dy) = (self.to.x - self.c2.x, self.to.y - self.c2.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.001 { xy(0., 1.) } else { xy(dx / len, dy / len) }
    }
}

pub fn distance(a: Xy, b: Xy) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// `MarkerType.ArrowClosed` outline (`-5,-4 0,0 -5,4`) at `tip`, pointing along `direction`,
/// scaled by `scale` (the marker is drawn in stroke units).
pub fn arrow_closed(tip: Xy, direction: Xy, scale: f32) -> [Xy; 3] {
    let normal = xy(-direction.y, direction.x);
    let back = |along: f32, across: f32| {
        xy(
            tip.x + (direction.x * along + normal.x * across) * scale,
            tip.y + (direction.y * along + normal.y * across) * scale,
        )
    };
    [back(-5., -4.), tip, back(-5., 4.)]
}

/// `getClosestHandle`: the nearest candidate within `radius`, ties keep the first.
pub fn closest<T: Copy>(position: Xy, radius: f32, candidates: impl IntoIterator<Item = (T, Xy)>) -> Option<T> {
    candidates
        .into_iter()
        .map(|(item, at)| (item, distance(at, position)))
        .filter(|(_, d)| *d <= radius)
        .fold(None, |best: Option<(T, f32)>, next| match best {
            Some(best) if best.1 <= next.1 => Some(best),
            _ => Some(next),
        })
        .map(|(item, _)| item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let viewport = Viewport { x: 40., y: -10., zoom: 1. };
        let anchor = xy(300., 200.);
        let before = viewport.to_flow(anchor);
        let after = viewport.zoom_at(1.6, anchor).to_flow(anchor);
        assert!(distance(before, after) < 0.001);
    }

    #[test]
    fn bezier_matches_react_flow_vertical_edge() {
        // getBezierPath({sourceX:0,sourceY:0,Bottom,targetX:100,targetY:100,Top}) →
        // "M0,0 C0,50 100,50 100,100", label at (50, 50).
        let curve = bezier(xy(0., 0.), Side::Bottom, xy(100., 100.), Side::Top, 1.);
        assert_eq!((curve.c1, curve.c2), (xy(0., 50.), xy(100., 50.)));
        assert_eq!(curve.center(), xy(50., 50.));
    }
}
