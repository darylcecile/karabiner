//! Native React Flow–style graph canvas for factory workflows. The look and interaction model
//! follow React Flow's default editor (see `xyflow.rs` for the ported algorithms and licence,
//! `style.rs` for the theme). The canvas is a controlled view: the owner passes a
//! [`CanvasGraph`] with stage ids, positions and outcome edges, and applies the
//! [`CanvasEvent`]s it emits (to `WorkflowLayout.positions` / `WorkflowGraph.edges`) before
//! calling [`FactoryCanvas::set_graph`] again. The canvas never edits configuration itself.

mod commands;
mod render;
mod style;
pub mod xyflow;

use std::{cell::Cell, rc::Rc, sync::Once, time::Instant};

use gpui_kit::*;

use crate::components::Tone;
use xyflow::{Rect, Side, Viewport, Xy, xy};

/// `WorkflowEdge.on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Success,
    Failure,
    Return,
}

impl Outcome {
    pub const ALL: [Outcome; 3] = [Outcome::Success, Outcome::Failure, Outcome::Return];

    /// Graphs read left to right: forward outcomes leave from the right edge, while return
    /// (back to an earlier node) leaves from the left edge, above the single input.
    pub fn side(self) -> Side {
        match self {
            Outcome::Return => Side::Left,
            Outcome::Success | Outcome::Failure => Side::Right,
        }
    }

    /// Vertical offset of this outcome's handle from the node's middle, in flow units:
    /// return above, success level with the input (straight sequential edges), failure below.
    pub fn slot(self) -> f32 {
        match self {
            Outcome::Return => -style::HANDLE_GAP,
            Outcome::Success => 0.,
            Outcome::Failure => style::HANDLE_GAP,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::Failure => "failure",
            Outcome::Return => "return",
        }
    }
}

/// Live run state shown on a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Running,
    Waiting,
    Completed,
    Failed,
    Paused,
    Cancelled,
}

impl NodeStatus {
    pub fn tone(self) -> Tone {
        match self {
            NodeStatus::Running => Tone::Running,
            NodeStatus::Waiting => Tone::Attention,
            NodeStatus::Completed => Tone::Success,
            NodeStatus::Failed => Tone::Danger,
            NodeStatus::Paused | NodeStatus::Cancelled => Tone::Neutral,
        }
    }
}

/// Node presentation. All kinds share React Flow's default node; fork and join add a glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeKind {
    #[default]
    Stage,
    /// Starts parallel branches.
    Fork,
    /// Waits for parallel branches.
    Join,
}

/// Small chip under the node text, e.g. a branch state or "Attempt 2". Only real run data.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeBadge {
    pub label: SharedString,
    pub tone: Tone,
}

/// One node. `position` is the flow coordinate of the node's top-left corner. Build with
/// `..Default::default()` so later additive fields don't break callers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CanvasNode {
    pub id: SharedString,
    pub label: SharedString,
    /// One short secondary line, e.g. "2 agents · 1 gate".
    pub detail: Option<SharedString>,
    pub position: Xy,
    pub status: Option<NodeStatus>,
    /// Compiler diagnostic for this stage; outlines the node.
    pub issue: Option<SharedString>,
    pub kind: NodeKind,
    /// Source handles shown, in slot order. Empty means all three outcomes.
    pub outcomes: Vec<Outcome>,
    pub badges: Vec<NodeBadge>,
}

impl CanvasNode {
    pub fn shown_outcomes(&self) -> Vec<Outcome> {
        if self.outcomes.is_empty() { Outcome::ALL.to_vec() } else { self.outcomes.clone() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasEdge {
    pub id: SharedString,
    pub source: SharedString,
    pub target: SharedString,
    pub outcome: Outcome,
    /// Draw as an animated-style dashed edge (e.g. the transition the run is taking).
    pub active: bool,
    pub issue: Option<SharedString>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasGraph {
    pub nodes: Vec<CanvasNode>,
    pub edges: Vec<CanvasEdge>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    pub nodes: Vec<SharedString>,
    pub edges: Vec<SharedString>,
}

/// What a library row carries when dragged onto the canvas.
#[derive(Debug, Clone, PartialEq)]
pub enum LibraryItem {
    Stage,
    Fork,
    Join,
    Agent(SharedString),
}

/// Drag payload for library rows: `.on_drag(LibraryDrag { .. }, |d, _, _, cx| cx.new(|_| d.clone()))`.
#[derive(Debug, Clone)]
pub struct LibraryDrag {
    pub item: LibraryItem,
    pub label: SharedString,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CanvasEvent {
    SelectionChanged(Selection),
    /// Drag or arrow-key move finished: new top-left flow positions (layout only).
    NodesMoved(Vec<(SharedString, Xy)>),
    Connect { source: SharedString, outcome: Outcome, target: SharedString },
    /// One end of `edge` was dragged to another handle. Carries the complete new link; only the
    /// dragged end differs (a source-end drag may also pick another outcome handle).
    Reconnect { edge: SharedString, source: SharedString, outcome: Outcome, target: SharedString },
    /// Delete/Backspace with a selection. The owner decides what to remove.
    Delete(Selection),
    /// A library item dropped at a flow position, onto `node` when over one.
    Drop { item: LibraryItem, position: Xy, node: Option<SharedString> },
    /// Double-click or Enter on a stage.
    Activate(SharedString),
    /// The connection check refused a link; show the reason.
    ConnectRejected(SharedString),
    /// ⌘C. The owner copies the selected stages/edges to its clipboard.
    Copy(Selection),
    /// ⌘X: copy, then remove.
    Cut(Selection),
    /// ⌘V at a flow position (under the pointer when it is over the canvas, else the view
    /// centre).
    Paste { position: Xy },
    /// ⌘D.
    Duplicate(Selection),
    /// ⌘Z / ⇧⌘Z (or ⌘Y). The owner keeps the history.
    Undo,
    Redo,
    /// Pan or zoom finished. Presentation state for the owner to keep per workflow and
    /// restore with `set_viewport`; it must not mark the factory dirty.
    ViewportChanged(Viewport),
}

pub(crate) const CONTEXT: &str = "FactoryCanvas";

actions!(factory_canvas, [FocusNextNode, FocusPreviousNode, CopySelection]);

/// Base's root binds `tab`, `shift-tab` and `cmd-c` to window focus traversal and copy, and
/// GPUI dispatches matched key bindings before `on_key_down` listeners. Binding the same keys
/// in the canvas's deeper key context makes them win only while the canvas has focus, so
/// Tab cycles stages there and leaves inputs and the rest of the app unchanged.
fn bind_keys(cx: &mut App) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let context = Some(CONTEXT);
        cx.bind_keys([
            KeyBinding::new("tab", FocusNextNode, context),
            KeyBinding::new("shift-tab", FocusPreviousNode, context),
            KeyBinding::new("cmd-c", CopySelection, context),
        ]);
    });
}

/// History and clipboard availability from the owner; disables Undo, Redo and Paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditCapabilities {
    pub can_undo: bool,
    pub can_redo: bool,
    pub can_paste: bool,
}

/// Validates a prospective link before it is emitted (e.g. the shared compiler's rules).
pub type ConnectionCheck = Rc<dyn Fn(&SharedString, Outcome, &SharedString) -> Result<(), SharedString>>;

#[derive(Debug, Clone)]
enum Gesture {
    Idle,
    Pan { last: Xy },
    Move { start: Xy, pointer: Xy, origins: Vec<(SharedString, Xy)>, moved: bool },
    Connect { source: SharedString, outcome: Outcome, reconnect: Option<SharedString>, start: Xy, cursor: Xy },
    /// Dragging the source end of `edge`; its target stays fixed.
    ReconnectSource { edge: SharedString, outcome: Outcome, target: SharedString, cursor: Xy },
    Minimap,
    Marquee { start: Xy, cursor: Xy },
}

pub struct FactoryCanvas {
    graph: CanvasGraph,
    viewport: Viewport,
    /// Window-space bounds from the last paint; canvas-local = window − origin.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    selection: Selection,
    gesture: Gesture,
    hovered_edge: Option<SharedString>,
    /// Keyboard focus (Tab), drawn as React Flow's focus ring; independent of selection.
    focused: Option<SharedString>,
    epoch: Instant,
    check: Option<ConnectionCheck>,
    locked: bool,
    reduced_motion: bool,
    capabilities: EditCapabilities,
    /// Last pointer position over the canvas (canvas-local), for paste.
    pointer: Option<Xy>,
    menu: Option<commands::Menu>,
    /// Canvas size the view was last fitted to. Fitting repeats while the slot settles, until
    /// the user moves the viewport; data refreshes never refit.
    fitted: Option<Xy>,
    interacted: bool,
    focus: FocusHandle,
}

impl EventEmitter<CanvasEvent> for FactoryCanvas {}

impl Focusable for FactoryCanvas {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

pub(crate) fn node_size(node: &CanvasNode) -> (f32, f32) {
    let badges = if node.badges.is_empty() { 0. } else { style::BADGE_ROW };
    let lines = style::NODE_LINE + node.detail.as_ref().map_or(0., |_| style::DETAIL_LINE) + badges;
    // Tall enough that stacked outcome handles keep their full hit areas apart.
    let handles = if node.shown_outcomes().len() > 1 { style::HANDLE_GAP * 2. + style::HANDLE_HIT } else { 0. };
    (style::NODE_W, (lines + style::NODE_PAD * 2.).max(handles))
}

pub(crate) fn node_rect(node: &CanvasNode) -> Rect {
    let (width, height) = node_size(node);
    Rect { x: node.position.x, y: node.position.y, width, height }
}

/// Flow position of a node's handle: the target (`None`) or an outcome's source handle.
pub(crate) fn handle_point(node: &CanvasNode, outcome: Option<Outcome>) -> Xy {
    let r = node_rect(node);
    let middle = r.y + r.height / 2.;
    match outcome {
        Some(outcome) if outcome.side() == Side::Left => xy(r.x, middle + outcome.slot()),
        Some(outcome) => xy(r.x + r.width, middle + outcome.slot()),
        None => xy(r.x, middle),
    }
}

fn to_xy(p: Point<Pixels>) -> Xy {
    xy(f32::from(p.x), f32::from(p.y))
}

impl FactoryCanvas {
    pub fn new(cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
        Self {
            graph: CanvasGraph::default(),
            viewport: Viewport::default(),
            bounds: Rc::new(Cell::new(None)),
            selection: Selection::default(),
            gesture: Gesture::Idle,
            hovered_edge: None,
            focused: None,
            epoch: Instant::now(),
            check: None,
            locked: false,
            reduced_motion: false,
            capabilities: EditCapabilities::default(),
            pointer: None,
            menu: None,
            fitted: None,
            interacted: false,
            // A normal tab stop, so window Tab traversal can enter the canvas.
            focus: cx.focus_handle().tab_stop(true),
        }
    }

    /// Replaces the graph, keeping the viewport and any selection that still exists. The
    /// first non-empty graph is fitted to the view.
    pub fn set_graph(&mut self, mut graph: CanvasGraph, cx: &mut Context<Self>) {
        // A live refresh during a node drag keeps the drag: dragged nodes keep their live
        // positions and everything else updates. The drag ends only if a dragged node is gone.
        if let Gesture::Move { origins, .. } = &self.gesture {
            let live: Vec<(SharedString, Xy)> = origins
                .iter()
                .filter_map(|(id, _)| self.graph.nodes.iter().find(|n| &n.id == id).map(|n| (id.clone(), n.position)))
                .collect();
            let all_present = live.len() == origins.len() && live.iter().all(|(id, _)| graph.nodes.iter().any(|n| &n.id == id));
            if all_present {
                for node in graph.nodes.iter_mut() {
                    if let Some((_, position)) = live.iter().find(|(id, _)| id == &node.id) {
                        node.position = *position;
                    }
                }
            } else {
                self.gesture = Gesture::Idle;
            }
        }
        self.selection.nodes.retain(|id| graph.nodes.iter().any(|n| &n.id == id));
        if self.focused.as_ref().is_some_and(|id| !graph.nodes.iter().any(|n| &n.id == id)) {
            self.focused = None;
        }
        self.selection.edges.retain(|id| graph.edges.iter().any(|e| &e.id == id));
        self.graph = graph;
        cx.notify();
    }

    pub fn graph(&self) -> &CanvasGraph {
        &self.graph
    }

    pub fn set_connection_check(&mut self, check: ConnectionCheck) {
        self.check = Some(check);
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Selects programmatically (e.g. after the owner adds a stage). Emits no event.
    pub fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selection = selection;
        cx.notify();
    }

    /// Canvas-local centre of the view in flow coordinates, for keyboard "Add stage".
    pub fn view_center(&self) -> Xy {
        let size = self.bounds.get().map_or(xy(800., 600.), |b| xy(f32::from(b.size.width), f32::from(b.size.height)));
        self.viewport.to_flow(xy(size.x / 2., size.y / 2.))
    }

    pub fn fit_view(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<SharedString> = self.graph.nodes.iter().map(|n| n.id.clone()).collect();
        self.fit_nodes(&ids, cx);
    }

    /// `fitView({ nodes, padding: 40px, maxZoom: 1 })`: small graphs keep the standard size.
    pub fn fit_nodes(&mut self, ids: &[SharedString], cx: &mut Context<Self>) {
        let Some(size) = self.canvas_size() else { return };
        let rects = self.graph.nodes.iter().filter(|n| ids.contains(&n.id)).map(node_rect);
        let Some(rect) = rects.reduce(Rect::union) else { return };
        self.fitted = Some(size);
        self.viewport = xyflow::viewport_for_bounds(rect, size.x, size.y, 40., 1.);
        self.viewport_changed(cx);
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Restores a stored viewport (e.g. when switching workflows). Disables the initial fit.
    pub fn set_viewport(&mut self, viewport: Viewport, cx: &mut Context<Self>) {
        self.viewport = viewport;
        self.interacted = true;
        cx.notify();
    }

    /// Clears the stored view so the next graph is fitted again (a workflow with no saved
    /// viewport).
    pub fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.interacted = false;
        self.fitted = None;
        cx.notify();
    }

    /// The app's Reduce Motion setting, in addition to the system one.
    pub fn set_reduced_motion(&mut self, reduced: bool, cx: &mut Context<Self>) {
        self.reduced_motion = reduced;
        cx.notify();
    }

    pub fn set_edit_capabilities(&mut self, capabilities: EditCapabilities, cx: &mut Context<Self>) {
        if self.capabilities != capabilities {
            self.capabilities = capabilities;
            cx.notify();
        }
    }

    pub fn set_locked(&mut self, locked: bool, cx: &mut Context<Self>) {
        self.locked = locked;
        cx.notify();
    }

    fn viewport_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(CanvasEvent::ViewportChanged(self.viewport));
        cx.notify();
    }

    /// Initial fit: once real slot bounds exist, and again if they change before the user
    /// touches the canvas (pointer or keyboard). Never during a gesture, so panels resizing
    /// (e.g. the save bar appearing) cannot reframe the view under a drag. Manual Fit is
    /// `fit_view` and always applies.
    pub(crate) fn auto_fit(&mut self, cx: &mut Context<Self>) {
        if !self.interacted && matches!(self.gesture, Gesture::Idle) && !self.graph.nodes.is_empty() && self.canvas_size().is_some() && self.fitted != self.canvas_size() {
            self.fit_view(cx);
        }
    }

    pub fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        self.interacted = true;
        let Some(bounds) = self.bounds.get() else { return };
        let center = xy(f32::from(bounds.size.width) / 2., f32::from(bounds.size.height) / 2.);
        self.viewport = self.viewport.zoom_at(self.viewport.zoom * factor, center);
        self.viewport_changed(cx);
    }

    fn local(&self, window_point: Point<Pixels>) -> Xy {
        let origin = self.bounds.get().map_or(xy(0., 0.), |b| to_xy(b.origin));
        let p = to_xy(window_point);
        xy(p.x - origin.x, p.y - origin.y)
    }

    fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if self.selection != selection {
            self.selection = selection.clone();
            cx.emit(CanvasEvent::SelectionChanged(selection));
        }
        cx.notify();
    }

    fn edge_curve(&self, edge: &CanvasEdge) -> Option<xyflow::Bezier> {
        let source = self.graph.nodes.iter().find(|n| n.id == edge.source)?;
        let target = self.graph.nodes.iter().find(|n| n.id == edge.target)?;
        let from = self.viewport.to_screen(handle_point(source, Some(edge.outcome)));
        let to = self.viewport.to_screen(handle_point(target, None));
        Some(xyflow::bezier(from, edge.outcome.side(), to, Side::Left, self.viewport.zoom))
    }

    fn edge_at(&self, local: Xy) -> Option<SharedString> {
        self.graph
            .edges
            .iter()
            .filter_map(|edge| Some((edge.id.clone(), self.edge_curve(edge)?.distance(local))))
            .filter(|(_, d)| *d <= xyflow::EDGE_HIT_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    fn node_at(&self, flow: Xy) -> Option<SharedString> {
        self.graph.nodes.iter().rev().find(|n| node_rect(n).contains_rect(Rect { x: flow.x, y: flow.y, width: 0., height: 0. })).map(|n| n.id.clone())
    }

    // Pointer input ------------------------------------------------------------------------

    pub(crate) fn pane_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.focused = None;
        self.menu = None;
        let local = self.local(event.position);
        if self.minimap_contains(local) {
            self.gesture = Gesture::Minimap;
            self.center_on_minimap(local);
        } else if event.modifiers.shift {
            self.gesture = Gesture::Marquee { start: local, cursor: local };
        } else if let Some(edge) = self.edge_at(local) {
            self.click_edge(edge, event.modifiers.platform, cx);
        } else {
            self.set_selection(Selection::default(), cx);
            self.gesture = Gesture::Pan { last: local };
        }
        cx.notify();
    }

    fn node_down(&mut self, id: SharedString, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.focused = None;
        self.menu = None;
        self.interacted = true;
        if event.click_count == 2 {
            cx.emit(CanvasEvent::Activate(id));
            return;
        }
        let toggle = event.modifiers.platform || event.modifiers.shift;
        let mut selection = self.selection.clone();
        if toggle {
            match selection.nodes.iter().position(|n| n == &id) {
                Some(ix) => drop(selection.nodes.remove(ix)),
                None => selection.nodes.push(id.clone()),
            }
        } else if !selection.nodes.contains(&id) {
            selection = Selection { nodes: vec![id.clone()], edges: vec![] };
        }
        self.set_selection(selection, cx);
        if self.locked || !self.selection.nodes.contains(&id) {
            return;
        }
        let origins = self
            .graph
            .nodes
            .iter()
            .filter(|n| self.selection.nodes.contains(&n.id))
            .map(|n| (n.id.clone(), n.position))
            .collect();
        let start = self.local(event.position);
        self.gesture = Gesture::Move { start, pointer: start, origins, moved: false };
    }

    fn handle_down(&mut self, source: SharedString, outcome: Outcome, reconnect: Option<SharedString>, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.interacted = true;
        if self.locked {
            return;
        }
        let start = self.local(event.position);
        self.gesture = Gesture::Connect { source, outcome, reconnect, start, cursor: start };
        cx.notify();
    }

    fn pointer_moved(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let local = self.local(position);
        match &mut self.gesture {
            Gesture::Idle => {
                let inside = self.bounds.get().is_some_and(|b| b.contains(&position));
                let over_node = self.node_at(self.viewport.to_flow(local)).is_some();
                let hovered = if inside && !over_node { self.edge_at(local) } else { None };
                self.pointer = inside.then_some(local);
                if hovered != self.hovered_edge {
                    self.hovered_edge = hovered;
                    cx.notify();
                }
                return;
            }
            Gesture::Pan { last } => {
                self.interacted = true;
                self.viewport.x += local.x - last.x;
                self.viewport.y += local.y - last.y;
                *last = local;
            }
            Gesture::Move { start, pointer, origins, moved } => {
                *pointer = local;
                let zoom = self.viewport.zoom;
                let (dx, dy) = ((local.x - start.x) / zoom, (local.y - start.y) / zoom);
                // React Flow's default nodeDragThreshold is 1px.
                *moved |= dx.abs() * zoom >= 1. || dy.abs() * zoom >= 1.;
                for (id, origin) in origins.iter() {
                    if let Some(node) = self.graph.nodes.iter_mut().find(|n| &n.id == id) {
                        node.position = xy(origin.x + dx, origin.y + dy);
                    }
                }
            }
            Gesture::Connect { cursor, .. } | Gesture::ReconnectSource { cursor, .. } | Gesture::Marquee { cursor, .. } => *cursor = local,
            Gesture::Minimap => {
                self.interacted = true;
                self.center_on_minimap(local);
            }
        }
        cx.notify();
    }

    fn pointer_released(&mut self, cx: &mut Context<Self>) {
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Pan { .. } | Gesture::Minimap => self.viewport_changed(cx),
            Gesture::Move { origins, moved: true, .. } => {
                let moved = origins
                    .iter()
                    .filter_map(|(id, _)| self.graph.nodes.iter().find(|n| &n.id == id))
                    .map(|n| (n.id.clone(), xy(n.position.x.round(), n.position.y.round())))
                    .collect();
                cx.emit(CanvasEvent::NodesMoved(moved));
            }
            // A click without a drag never connects (the return handle sits within snapping range of
            // its own input). Same 1px threshold as node drags.
            Gesture::Connect { source, outcome, reconnect, start, cursor } if (cursor.x - start.x).abs().max((cursor.y - start.y).abs()) >= 1. => self.finish_connect(source, outcome, reconnect, cursor, cx),
            Gesture::ReconnectSource { edge, outcome, target, cursor } => self.finish_source_reconnect(edge, outcome, target, cursor, cx),
            Gesture::Marquee { start, cursor } => self.finish_marquee(start, cursor, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Snaps to the closest target handle within `connectionRadius`, else the node under the
    /// pointer (React Flow's "easy connect").
    fn target_near(&self, cursor: Xy) -> Option<SharedString> {
        let handles = self.graph.nodes.iter().map(|n| (n, self.viewport.to_screen(handle_point(n, None))));
        xyflow::closest(cursor, xyflow::CONNECTION_RADIUS, handles)
            .map(|n| n.id.clone())
            .or_else(|| self.node_at(self.viewport.to_flow(cursor)))
    }

    fn finish_connect(&mut self, source: SharedString, outcome: Outcome, reconnect: Option<SharedString>, cursor: Xy, cx: &mut Context<Self>) {
        let Some(target) = self.target_near(cursor) else { return };
        if let Some(Err(reason)) = self.check.as_ref().map(|check| check(&source, outcome, &target)) {
            cx.emit(CanvasEvent::ConnectRejected(reason));
            return;
        }
        cx.emit(match reconnect {
            Some(edge) => CanvasEvent::Reconnect { edge, source, outcome, target },
            None => CanvasEvent::Connect { source, outcome, target },
        });
    }

    /// Snaps to the closest source handle of any outcome, else keeps the edge's outcome on the
    /// node under the pointer.
    pub(crate) fn source_near(&self, cursor: Xy, fallback: Outcome) -> Option<(SharedString, Outcome)> {
        let handles = self.graph.nodes.iter().flat_map(|n| {
            Outcome::ALL.map(|outcome| ((n, outcome), self.viewport.to_screen(handle_point(n, Some(outcome)))))
        });
        xyflow::closest(cursor, xyflow::CONNECTION_RADIUS, handles)
            .map(|(n, outcome)| (n.id.clone(), outcome))
            .or_else(|| self.node_at(self.viewport.to_flow(cursor)).map(|id| (id, fallback)))
    }

    fn finish_source_reconnect(&mut self, edge: SharedString, outcome: Outcome, target: SharedString, cursor: Xy, cx: &mut Context<Self>) {
        let Some((source, outcome)) = self.source_near(cursor, outcome) else { return };
        if let Some(Err(reason)) = self.check.as_ref().map(|check| check(&source, outcome, &target)) {
            cx.emit(CanvasEvent::ConnectRejected(reason));
            return;
        }
        cx.emit(CanvasEvent::Reconnect { edge, source, outcome, target });
    }

    fn source_end_down(&mut self, edge: SharedString, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.interacted = true;
        let Some(found) = self.graph.edges.iter().find(|e| e.id == edge) else { return };
        if self.locked {
            return;
        }
        let (outcome, target) = (found.outcome, found.target.clone());
        self.gesture = Gesture::ReconnectSource { edge, outcome, target, cursor: self.local(event.position) };
        cx.notify();
    }

    // Minimap ------------------------------------------------------------------------------

    pub(crate) fn canvas_size(&self) -> Option<Xy> {
        self.bounds.get().map(|b| xy(f32::from(b.size.width), f32::from(b.size.height)))
    }

    /// Canvas-local origin of the minimap panel (bottom-right, 15px margin).
    pub(crate) fn minimap_origin(&self) -> Option<Xy> {
        let size = self.canvas_size()?;
        Some(xy(size.x - style::PANEL_MARGIN - style::MINIMAP_W, size.y - style::PANEL_MARGIN - style::MINIMAP_H))
    }

    fn minimap_contains(&self, local: Xy) -> bool {
        self.minimap_origin().is_some_and(|o| {
            (o.x..=o.x + style::MINIMAP_W).contains(&local.x) && (o.y..=o.y + style::MINIMAP_H).contains(&local.y)
        })
    }

    /// React Flow MiniMap framing: the union of node bounds and the visible area, centred with
    /// a 5-unit offset. Returns the flow rect shown and flow units per minimap pixel.
    pub(crate) fn minimap_frame(&self) -> Option<(Rect, Xy, f32)> {
        let size = self.canvas_size()?;
        let a = self.viewport.to_flow(xy(0., 0.));
        let b = self.viewport.to_flow(size);
        let visible = Rect { x: a.x, y: a.y, width: b.x - a.x, height: b.y - a.y };
        let frame = self.graph.nodes.iter().map(node_rect).fold(visible, Rect::union);
        let scale = (frame.width / style::MINIMAP_W).max(frame.height / style::MINIMAP_H);
        let (view_w, view_h) = (style::MINIMAP_W * scale, style::MINIMAP_H * scale);
        let pad = 5. * scale;
        let top_left = xy(frame.x - (view_w - frame.width) / 2. - pad, frame.y - (view_h - frame.height) / 2. - pad);
        Some((visible, top_left, (view_w + pad * 2.) / style::MINIMAP_W))
    }

    /// Click or drag in the minimap centres the viewport on that flow point.
    fn center_on_minimap(&mut self, local: Xy) {
        let (Some(origin), Some((_, top_left, unit)), Some(size)) = (self.minimap_origin(), self.minimap_frame(), self.canvas_size()) else {
            return;
        };
        let flow = xy(top_left.x + (local.x - origin.x) * unit, top_left.y + (local.y - origin.y) * unit);
        self.viewport.x = size.x / 2. - flow.x * self.viewport.zoom;
        self.viewport.y = size.y / 2. - flow.y * self.viewport.zoom;
    }

    fn finish_marquee(&mut self, start: Xy, cursor: Xy, cx: &mut Context<Self>) {
        let (a, b) = (self.viewport.to_flow(start), self.viewport.to_flow(cursor));
        let area = Rect { x: a.x.min(b.x), y: a.y.min(b.y), width: (a.x - b.x).abs(), height: (a.y - b.y).abs() };
        let nodes: Vec<SharedString> = self.graph.nodes.iter().filter(|n| area.contains_rect(node_rect(n))).map(|n| n.id.clone()).collect();
        // React Flow also selects the edges between selected nodes.
        let edges = self
            .graph
            .edges
            .iter()
            .filter(|e| nodes.contains(&e.source) && nodes.contains(&e.target))
            .map(|e| e.id.clone())
            .collect();
        self.set_selection(Selection { nodes, edges }, cx);
    }

    fn scrolled(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        let (delta, lines) = match event.delta {
            ScrollDelta::Pixels(p) => (f32::from(p.y), false),
            ScrollDelta::Lines(l) => (l.y, true),
        };
        // GPUI reports wheel "up" as positive y; the DOM's deltaY has the opposite sign.
        self.interacted = true;
        let zoom = xyflow::wheel_zoom(self.viewport.zoom, -delta, lines, event.modifiers.control);
        self.viewport = self.viewport.zoom_at(zoom, self.local(event.position));
        self.viewport_changed(cx);
    }

    fn pinched(&mut self, event: &PinchEvent, cx: &mut Context<Self>) {
        self.interacted = true;
        let zoom = self.viewport.zoom * (1. + event.delta);
        self.viewport = self.viewport.zoom_at(zoom, self.local(event.position));
        self.viewport_changed(cx);
    }

    fn dropped(&mut self, drag: &LibraryDrag, node: Option<SharedString>, window: &Window, cx: &mut Context<Self>) {
        if self.locked {
            return;
        }
        let position = self.viewport.to_flow(self.local(window.mouse_position()));
        let node = node.or_else(|| self.node_at(position));
        cx.emit(CanvasEvent::Drop { item: drag.item.clone(), position, node });
    }

    // Keyboard --------------------------------------------------------------------------

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.interacted = true;
        let keys = &event.keystroke;
        let step = if keys.modifiers.shift { 20. } else { 5. };
        if keys.modifiers.platform && self.command_key(&keys.key, keys.modifiers.shift, cx) {
            cx.stop_propagation();
            return;
        }
        match keys.key.as_str() {
            "backspace" | "delete" => self.run(commands::Command::Delete, cx),
            "escape" if self.menu.is_some() => self.menu = None,
            "escape" if !matches!(self.gesture, Gesture::Idle) => self.cancel_gesture(),
            "escape" if self.selection != Selection::default() || self.focused.is_some() => {
                self.focused = None;
                self.set_selection(Selection::default(), cx);
            }
            "escape" => window.blur(cx),
            // React Flow: Enter/Space select the focused node; Enter on a selected node opens it.
            "enter" | "space" => self.press_focused(keys.key == "enter", cx),
            "left" => self.nudge(xy(-step, 0.), cx),
            "right" => self.nudge(xy(step, 0.), cx),
            "up" => self.nudge(xy(0., -step), cx),
            "down" => self.nudge(xy(0., step), cx),
            "=" | "+" => self.zoom_by(1.2, cx),
            "-" => self.zoom_by(1. / 1.2, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Tab moves keyboard focus between stages without selecting, like React Flow's
    /// `tabIndex=0` nodes. Entering with no focused stage starts at the first stage (last
    /// for Shift-Tab). Past either end, or with no stages, the focus ring clears and window
    /// focus traversal continues normally; selection and the draft are untouched.
    fn cycle(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.graph.nodes.len() as isize;
        let current = self.focused.as_ref().and_then(|id| self.graph.nodes.iter().position(|n| &n.id == id));
        let next = match current {
            Some(ix) => ix as isize + direction,
            None if direction > 0 => 0,
            None => count - 1,
        };
        if !(0..count).contains(&next) {
            self.focused = None;
            cx.notify();
            if direction > 0 { window.focus_next(cx) } else { window.focus_prev(cx) }
            return;
        }
        let id = self.graph.nodes[next as usize].id.clone();
        self.focused = Some(id.clone());
        self.reveal(&id, cx);
        cx.notify();
    }

    fn press_focused(&mut self, enter: bool, cx: &mut Context<Self>) {
        let target = self.focused.clone().or_else(|| match self.selection.nodes.as_slice() {
            [id] => Some(id.clone()),
            _ => None,
        });
        let Some(id) = target else { return };
        if enter && self.selection.nodes == [id.clone()] {
            cx.emit(CanvasEvent::Activate(id));
        } else {
            self.set_selection(Selection { nodes: vec![id], edges: vec![] }, cx);
        }
    }

    /// Phase (0..10 stroke units) of React Flow's `dashdraw` animation: `stroke-dashoffset`
    /// 10 → 0 over 0.5s, linear, infinite.
    pub(crate) fn dash_phase(&self, cx: &App) -> f32 {
        if self.reduced_motion || cx.reduce_motion() {
            return 0.;
        }
        let t = self.epoch.elapsed().as_secs_f32() / 0.5;
        10. * (1. - t.fract())
    }

    fn nudge(&mut self, delta: Xy, cx: &mut Context<Self>) {
        if self.locked || self.selection.nodes.is_empty() {
            return;
        }
        let moved = self
            .graph
            .nodes
            .iter_mut()
            .filter(|n| self.selection.nodes.contains(&n.id))
            .map(|n| {
                n.position = xy(n.position.x + delta.x, n.position.y + delta.y);
                (n.id.clone(), n.position)
            })
            .collect();
        cx.emit(CanvasEvent::NodesMoved(moved));
        cx.notify();
    }
}
