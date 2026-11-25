//! Element tree and painting for [`FactoryCanvas`]. Nodes, handles, labels and controls are
//! elements sized by the current zoom (GPUI cannot transform text), so this module sets text
//! sizes directly from React Flow's metrics instead of the app's fixed typography roles.

use gpui_kit::{
    assets::IconName,
    component::{Icon, tooltip::Tooltip},
    prelude::FluentBuilder as _,
    *,
};

use super::{
    CanvasEdge, CanvasNode, FactoryCanvas, Gesture, LibraryDrag, NodeKind, Outcome, handle_point, node_rect, node_size,
    style::{self, FlowTheme},
    xyflow::{self, Bezier, Rect, Side, Viewport, Xy, xy},
};

fn pt(p: Xy) -> Point<Pixels> {
    point(px(p.x), px(p.y))
}

type ControlAction = fn(&mut FactoryCanvas, &mut Context<FactoryCanvas>);

struct EdgePaint {
    curve: Bezier,
    color: Hsla,
    /// Dash phase for animated edges.
    dash: Option<f32>,
}

struct Scene {
    theme: FlowTheme,
    viewport: Viewport,
    edges: Vec<EdgePaint>,
    connection: Option<Bezier>,
    marquee: Option<Rect>,
    stroke: f32,
}

impl Render for FactoryCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.auto_fit(cx);
        if self.auto_pan_step() {
            window.request_animation_frame();
        }
        let theme = FlowTheme::current(cx);
        let grabbing = matches!(self.gesture, Gesture::Pan { .. } | Gesture::Move { moved: true, .. });
        let phase = self.dash_phase(cx);
        if phase > 0. && self.graph.edges.iter().any(|e| e.active) {
            window.request_animation_frame();
        }
        let scene = self.scene(theme, crate::components::Tone::Danger.color(cx), phase);
        let entity = cx.entity().downgrade();
        let resized = entity.clone();
        let bounds_cell = self.bounds.clone();
        let tracking = !matches!(self.gesture, Gesture::Idle);

        let surface = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let previous = bounds_cell.replace(Some(bounds));
                // Bounds are only known here, after this frame's nodes, fit and minimap were
                // built. When they first appear or resize, re-render the view once next frame
                // (a cached view ignores a bare window refresh) so `auto_fit` runs.
                if previous.map(|b| b.size) != Some(bounds.size) {
                    let resized = resized.clone();
                    window.on_next_frame(move |_, cx| {
                        resized.update(cx, |_, cx| cx.notify()).ok();
                    });
                }
                paint_scene(&scene, bounds, window);
                listen_pointer(entity, tracking, window);
            },
        )
        .absolute()
        .size_full();

        div()
            .id("factory-canvas")
            .key_context(super::CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .cursor(if grabbing { CursorStyle::ClosedHand } else { CursorStyle::OpenHand })
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|this, _: &super::FocusNextNode, window, cx| {
                this.interacted = true;
                this.cycle(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &super::FocusPreviousNode, window, cx| {
                this.interacted = true;
                this.cycle(-1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &super::CopySelection, _, cx| this.run(super::commands::Command::Copy, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::pane_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::pane_right_down))
            .on_scroll(cx.listener(|this, event, _, cx| this.scrolled(event, cx)))
            .on_pinch(cx.listener(|this, event, _, cx| this.pinched(event, cx)))
            .on_drop(cx.listener(|this, drag: &LibraryDrag, window, cx| this.dropped(drag, None, window, cx)))
            .child(surface)
            .children(self.label_positions().into_iter().map(|(edge, center)| self.edge_label(edge, center, theme, cx)))
            .children(self.graph.nodes.iter().map(|node| self.node(node, theme, cx)))
            .children(self.reconnectors(cx))
            .child(self.controls(theme, cx))
            .child(minimap(self, theme, cx))
            .children(self.render_menu(cx))
    }
}

impl FactoryCanvas {
    fn scene(&self, theme: FlowTheme, danger: Hsla, phase: f32) -> Scene {
        let updating = match &self.gesture {
            Gesture::Connect { reconnect: Some(edge), .. } | Gesture::ReconnectSource { edge, .. } => Some(edge.clone()),
            _ => None,
        };
        let edges = self
            .graph
            .edges
            .iter()
            .filter_map(|edge| {
                let emphasised = self.selection.edges.contains(&edge.id) || self.hovered_edge.as_ref() == Some(&edge.id);
                let color = match () {
                    // `.react-flow__edge.updating` strokes #777 while an end is dragged.
                    _ if updating.as_ref() == Some(&edge.id) => rgb(0x777777).into(),
                    _ if emphasised => theme.edge_hover,
                    _ if edge.issue.is_some() => danger,
                    _ => theme.edge,
                };
                Some(EdgePaint { curve: self.edge_curve(edge)?, color, dash: edge.active.then_some(phase) })
            })
            .collect();
        let connection = match &self.gesture {
            Gesture::Connect { source, outcome, cursor, .. } => self.connection_line(source, *outcome, *cursor),
            Gesture::ReconnectSource { outcome, target, cursor, .. } => self.source_line(*outcome, target, *cursor),
            _ => None,
        };
        let marquee = match self.gesture {
            Gesture::Marquee { start, cursor } => Some(Rect {
                x: start.x.min(cursor.x),
                y: start.y.min(cursor.y),
                width: (start.x - cursor.x).abs(),
                height: (start.y - cursor.y).abs(),
            }),
            _ => None,
        };
        Scene { theme, viewport: self.viewport, edges, connection, marquee, stroke: self.viewport.zoom.max(0.5) }
    }

    /// React Flow's connection line: a default Bézier from the source handle to the pointer,
    /// or to the snapped target handle.
    fn connection_line(&self, source: &SharedString, outcome: Outcome, cursor: Xy) -> Option<Bezier> {
        let node = self.graph.nodes.iter().find(|n| &n.id == source)?;
        let from = self.viewport.to_screen(handle_point(node, Some(outcome)));
        let snapped = self.snapped_target(cursor);
        let (to, side) = match snapped {
            Some(target) => (self.viewport.to_screen(handle_point(target, None)), Side::Left),
            None => (cursor, opposite(outcome.side())),
        };
        Some(xyflow::bezier(from, outcome.side(), to, side))
    }

    /// Connection line while dragging an edge's source end: pointer (or snapped source
    /// handle) to the fixed target handle.
    fn source_line(&self, outcome: Outcome, target: &SharedString, cursor: Xy) -> Option<Bezier> {
        let node = self.graph.nodes.iter().find(|n| &n.id == target)?;
        let to = self.viewport.to_screen(handle_point(node, None));
        let snapped = self.source_near(cursor, outcome).and_then(|(id, outcome)| {
            let source = self.graph.nodes.iter().find(|n| n.id == id)?;
            Some((self.viewport.to_screen(handle_point(source, Some(outcome))), outcome.side()))
        });
        let (from, side) = snapped.unwrap_or((cursor, Side::Right));
        Some(xyflow::bezier(from, side, to, Side::Left))
    }

    fn snapped_target(&self, cursor: Xy) -> Option<&CanvasNode> {
        let handles = self.graph.nodes.iter().map(|n| (n, self.viewport.to_screen(handle_point(n, None))));
        xyflow::closest(cursor, xyflow::CONNECTION_RADIUS, handles)
    }

    fn connecting_to(&self) -> Option<SharedString> {
        match &self.gesture {
            Gesture::Connect { cursor, .. } => self.target_near(*cursor),
            _ => None,
        }
    }

    fn node(&self, node: &CanvasNode, theme: FlowTheme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let z = self.viewport.zoom;
        let origin = self.viewport.to_screen(node.position);
        let (width, height) = node_size(node);
        let selected = self.selection.nodes.contains(&node.id);
        let focused = self.focused.as_ref() == Some(&node.id);
        let border = match () {
            _ if selected => theme.selected,
            _ if focused => theme.focus_border,
            _ if node.issue.is_some() => crate::components::Tone::Danger.color(cx),
            _ => theme.node_border,
        };
        let id = node.id.clone();
        let menu_id = node.id.clone();
        let drop_id = node.id.clone();
        let status = node.status.map(|s| s.tone().color(cx));
        let label = div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6. * z))
            .h(px(style::NODE_LINE * z))
            .when_some(status, |el, color| el.child(div().flex_none().size(px(6. * z)).rounded_full().bg(color)))
            .when_some(kind_icon(node.kind), |el, icon| el.child(Icon::new(icon).size(px(12. * z))))
            .child(one_line().min_w_0().child(node.label.clone()));
        let body = div()
            .id(SharedString::from(format!("node-{}", node.id)))
            .absolute()
            .left(px(origin.x))
            .top(px(origin.y))
            .w(px(width * z))
            .h(px(height * z))
            .px(px(style::NODE_PAD * z))
            .py(px(style::NODE_PAD * z))
            .flex()
            .flex_col()
            .items_center()
            .rounded(px(style::NODE_RADIUS * z))
            .border_1()
            .border_color(border)
            .bg(theme.node_bg)
            .text_color(theme.node_text)
            .text_size(px(style::NODE_TEXT * z))
            .line_height(px(style::NODE_LINE * z))
            .when(selected, |el| el.shadow(node_shadow()))
            .when(focused && !selected, |el| el.shadow(focus_ring(theme)))
            .when(!selected, |el| el.hover(move |s| s.border_color(theme.node_hover)))
            .drag_over::<LibraryDrag>(move |s, _, _, _| s.border_color(theme.selected))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.node_down(id.clone(), event, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.node_right_down(menu_id.clone(), event, window, cx);
                }),
            )
            .on_drop(cx.listener(move |this, drag: &LibraryDrag, window, cx| {
                cx.stop_propagation();
                this.dropped(drag, Some(drop_id.clone()), window, cx);
            }))
            .child(label)
            .when_some(node.detail.clone(), |el, detail| {
                el.child(
                    one_line()
                        .max_w_full()
                        .text_size(px(style::DETAIL_TEXT * z))
                        .line_height(px(style::DETAIL_LINE * z))
                        .text_color(theme.node_detail)
                        .child(detail),
                )
            })
            .when(!node.badges.is_empty(), |el| el.child(badges(node, z, cx)));
        let target_active = self.connecting_to().as_ref() == Some(&node.id);
        let handles = std::iter::once(self.handle(node, None, target_active, theme, cx))
            .chain(node.shown_outcomes().into_iter().map(|outcome| self.handle(node, Some(outcome), false, theme, cx)));
        div().child(body).children(handles)
    }

    /// 6px React Flow handle with a 14px hit area. Source handles start connections.
    fn handle(&self, node: &CanvasNode, outcome: Option<Outcome>, active: bool, theme: FlowTheme, cx: &mut Context<Self>) -> AnyElement {
        let z = self.viewport.zoom;
        let at = self.viewport.to_screen(handle_point(node, outcome));
        let dot = (style::HANDLE * z).max(5.);
        let hit = dot.max(style::HANDLE_HIT);
        let border = if active { theme.edge_hover } else { theme.handle_border };
        let key = format!("handle-{}-{}", node.id, outcome.map_or("target", Outcome::label));
        // Per-handle group: hovering one handle must not light up another.
        let group = SharedString::from(key.clone());
        let source = node.id.clone();
        div()
            .id(SharedString::from(key))
            .absolute()
            .left(px(at.x - hit / 2.))
            .top(px(at.y - hit / 2.))
            .size(px(hit))
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::Crosshair)
            .when_some(outcome, |el, outcome| el.tooltip(move |window, cx| Tooltip::new(handle_hint(outcome)).build(window, cx)))
            .group(group.clone())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(outcome) = outcome {
                        this.handle_down(source.clone(), outcome, None, event, window, cx);
                    }
                }),
            )
            .child(
                div()
                    .size(px(dot))
                    .rounded_full()
                    .bg(theme.handle_bg)
                    .border_1()
                    .border_color(border)
                    .group_hover(group, move |s| s.border_color(theme.edge_hover)),
            )
            .into_any_element()
    }

    /// React Flow edge text: 10px, background chip in the canvas colour, padding 2×4, radius 2.
    /// Success edges stay unlabelled like a default edge; failure and return say so.
    /// Label centres at each edge's midpoint. When labels would overlap (e.g. a failure and a
    /// return edge between the same stages), later ones step down by one label height.
    fn label_positions(&self) -> Vec<(&CanvasEdge, Xy)> {
        let (w, h) = label_size(self.viewport.zoom);
        let mut placed: Vec<(&CanvasEdge, Xy)> = Vec::new();
        for edge in self.graph.edges.iter().filter(|e| e.outcome != Outcome::Success) {
            let Some(curve) = self.edge_curve(edge) else { continue };
            let mut center = curve.center();
            while placed.iter().any(|(_, p)| (p.x - center.x).abs() < w && (p.y - center.y).abs() < h) {
                center.y += h + 2.;
            }
            placed.push((edge, center));
        }
        placed
    }

    fn edge_label(&self, edge: &CanvasEdge, center: Xy, theme: FlowTheme, cx: &mut Context<Self>) -> AnyElement {
        let z = self.viewport.zoom;
        let (w, h) = label_size(z);
        let id = edge.id.clone();
        div()
                .id(SharedString::from(format!("edge-label-{}", edge.id)))
                .absolute()
                .left(px(center.x - w / 2.))
                .top(px(center.y - h / 2.))
                .w(px(w))
                .h(px(h))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .px(px(4. * z))
                        .rounded(px(2. * z))
                        .bg(theme.background)
                        .text_size(px(style::EDGE_TEXT * z))
                        .line_height(px(h))
                        .text_color(theme.edge_label)
                        .child(edge.outcome.label()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        window.focus(&this.focus, cx);
                        this.menu = None;
                        this.click_edge(id.clone(), event.modifiers.platform, cx);
                    }),
                )
                .into_any_element()
    }

    /// React Flow's edge updater: a `reconnectRadius` circle at each end of a selected edge.
    fn reconnectors(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.locked {
            return Vec::new();
        }
        let r = xyflow::RECONNECT_RADIUS;
        self.graph
            .edges
            .iter()
            .filter(|edge| self.selection.edges.contains(&edge.id))
            .filter_map(|edge| Some((edge, self.edge_curve(edge)?)))
            .flat_map(|(edge, curve)| {
                let (source, outcome, id, source_id) = (edge.source.clone(), edge.outcome, edge.id.clone(), edge.id.clone());
                let target_end = updater(format!("reconnect-target-{}", edge.id), curve.to, r).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.handle_down(source.clone(), outcome, Some(id.clone()), event, window, cx);
                    }),
                );
                let source_end = updater(format!("reconnect-source-{}", edge.id), curve.from, r).on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.source_end_down(source_id.clone(), event, window, cx);
                    }),
                );
                [source_end.into_any_element(), target_end.into_any_element()]
            })
            .collect()
    }

    /// `<Controls />`: zoom in, zoom out, fit view, toggle interactivity (26px buttons, 12px
    /// glyphs, hairline separators, soft shadow), bottom-left at the 15px panel margin.
    fn controls(&self, theme: FlowTheme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let lock = if self.locked { IconName::Lock } else { IconName::LockOpen };
        let buttons: [(&'static str, IconName, ControlAction); 4] = [
            ("zoom-in", IconName::Plus, |this, cx| this.zoom_by(1.2, cx)),
            ("zoom-out", IconName::Minus, |this, cx| this.zoom_by(1. / 1.2, cx)),
            ("fit-view", IconName::Maximize, |this, cx| this.fit_view(cx)),
            ("interactive", lock, |this, cx| {
                this.locked = !this.locked;
                cx.notify();
            }),
        ];
        let last = buttons.len() - 1;
        div()
            .absolute()
            .left(px(style::PANEL_MARGIN))
            .bottom(px(style::PANEL_MARGIN))
            .flex()
            .flex_col()
            .occlude()
            .shadow(vec![BoxShadow {
                color: theme.controls_shadow,
                offset: point(px(0.), px(0.)),
                blur_radius: px(2.),
                spread_radius: px(1.),
                inset: false,
            }])
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(buttons.into_iter().enumerate().map(|(ix, (id, icon, action))| {
                div()
                    .id(id)
                    .size(px(style::CONTROL_BUTTON))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.controls_bg)
                    .text_color(theme.controls_text)
                    .cursor_pointer()
                    .when(ix < last, |el| el.border_b_1().border_color(theme.controls_border))
                    .hover(move |s| s.bg(theme.controls_hover))
                    .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
                    .child(Icon::new(icon).size(px(style::CONTROL_ICON)))
            }))
    }
}

fn kind_icon(kind: NodeKind) -> Option<IconName> {
    match kind {
        NodeKind::Stage => None,
        NodeKind::Fork => Some(IconName::GitFork),
        NodeKind::Join => Some(IconName::GitMerge),
    }
}

/// Run chips (branch state, attempt) in the edge-label type size, tinted by tone.
fn badges(node: &CanvasNode, z: f32, cx: &App) -> Div {
    div()
        .mt(px(4. * z))
        .flex()
        .gap(px(4. * z))
        .max_w_full()
        .overflow_x_hidden()
        .children(node.badges.iter().map(|badge| {
            div()
                .flex_none()
                .h(px(style::BADGE_H * z))
                .px(px(4. * z))
                .rounded(px(3. * z))
                .bg(badge.tone.tint(cx, 0.12))
                .text_color(badge.tone.color(cx))
                .text_size(px(style::EDGE_TEXT * z))
                .line_height(px(style::BADGE_H * z))
                .whitespace_nowrap()
                .child(badge.label.clone())
        }))
}

/// Single line that ellipsizes horizontally without clipping descenders (GPUI's `truncate`
/// also hides vertical overflow).
fn one_line() -> Div {
    div().overflow_x_hidden().whitespace_nowrap().text_ellipsis()
}

/// Handles are unlabelled dots like React Flow's; the hover hint names each outcome. The left
/// input has none, as in React Flow.
fn handle_hint(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Success => "Success · drag to the next stage",
        Outcome::Failure => "Failure · drag to the stage that handles it",
        Outcome::Return => "Return · drag back to an earlier stage",
    }
}

/// `.react-flow__node.selectable:focus`: 4px ring in `--xy-theme-color-focus`.
fn focus_ring(theme: FlowTheme) -> Vec<BoxShadow> {
    vec![BoxShadow { color: theme.focus_ring, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(4.), inset: false }]
}

/// Edge label box: 10px text with React Flow's 2×4 label background padding.
fn label_size(zoom: f32) -> (f32, f32) {
    (48. * zoom, 14. * zoom)
}

fn opposite(side: Side) -> Side {
    match side {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
    }
}

/// Invisible `reconnectRadius` hit circle (`.react-flow__edgeupdater`, cursor: move).
fn updater(id: String, at: Xy, r: f32) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .absolute()
        .left(px(at.x - r))
        .top(px(at.y - r))
        .size(px(r * 2.))
        .rounded_full()
        .cursor(CursorStyle::ClosedHand)
}

/// `--xy-node-boxshadow-default` from xy-theme.css.
fn node_shadow() -> Vec<BoxShadow> {
    let layer = |alpha: u32, y: f32, blur: f32| BoxShadow {
        color: rgba(alpha).into(),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    };
    vec![layer(0x00000005, 3.54, 4.55), layer(0x0000000D, 3.54, 4.55), layer(0x0000001A, 0.51, 1.01)]
}

/// Moves and releases are tracked window-wide while a gesture is active, so drags that leave
/// the canvas still finish. Idle moves only update edge hover.
fn listen_pointer(entity: WeakEntity<FactoryCanvas>, tracking: bool, window: &mut Window) {
    let moves = entity.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            moves.update(cx, |this, cx| this.pointer_moved(event.position, cx)).ok();
        }
    });
    if tracking {
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                entity.update(cx, |this, cx| this.pointer_released(cx)).ok();
            }
        });
    }
}

fn paint_scene(scene: &Scene, bounds: Bounds<Pixels>, window: &mut Window) {
    let origin = xy(f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let offset = |p: Xy| xy(p.x + origin.x, p.y + origin.y);
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        paint_dots(scene, bounds, window);
        for edge in &scene.edges {
            let curve = Bezier { from: offset(edge.curve.from), c1: offset(edge.curve.c1), c2: offset(edge.curve.c2), to: offset(edge.curve.to) };
            paint_curve(curve, edge.color, scene.stroke, edge.dash, window);
            paint_arrow(curve, edge.color, scene.stroke, window);
        }
        if let Some(line) = scene.connection {
            let curve = Bezier { from: offset(line.from), c1: offset(line.c1), c2: offset(line.c2), to: offset(line.to) };
            paint_curve(curve, scene.theme.edge, scene.stroke, None, window);
        }
        if let Some(rect) = scene.marquee {
            let area = Bounds::new(pt(offset(xy(rect.x, rect.y))), size(px(rect.width), px(rect.height)));
            window.paint_quad(quad(area, px(0.), scene.theme.selection_bg, px(1.), scene.theme.selection_border, BorderStyle::Dashed));
        }
    });
}

/// `<Background variant="dots" gap={20} size={1} />`, scrolling and scaling with the viewport.
/// All dots go into one filled path: one scene primitive per frame instead of one quad per
/// dot (thousands at low zoom, each inserted into GPUI's bounds tree on every redraw). Dots
/// are at most 2px across, so squares render the same as React Flow's circles.
fn paint_dots(scene: &Scene, bounds: Bounds<Pixels>, window: &mut Window) {
    let gap = style::DOT_GAP * scene.viewport.zoom;
    let r = (style::DOT_RADIUS * scene.viewport.zoom).max(0.5);
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (start_x, start_y) = (scene.viewport.x.rem_euclid(gap), scene.viewport.y.rem_euclid(gap));
    let origin = xy(f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let mut path = PathBuilder::fill();
    let mut y = start_y;
    while y < h {
        let mut x = start_x;
        while x < w {
            let (left, top) = (origin.x + x - r, origin.y + y - r);
            path.move_to(point(px(left), px(top)));
            path.line_to(point(px(left + r * 2.), px(top)));
            path.line_to(point(px(left + r * 2.), px(top + r * 2.)));
            path.line_to(point(px(left), px(top + r * 2.)));
            path.close();
            x += gap;
        }
        y += gap;
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, scene.theme.dots);
    }
}

fn paint_curve(curve: Bezier, color: Hsla, width: f32, dash: Option<f32>, window: &mut Window) {
    let mut path = PathBuilder::stroke(px(width));
    if let Some(phase) = dash {
        let pattern: Vec<Pixels> = dash_pattern(phase).iter().map(|d| px(d * width)).collect();
        path = path.dash_array(&pattern);
    }
    path.move_to(pt(curve.from));
    path.cubic_bezier_to(pt(curve.to), pt(curve.c1), pt(curve.c2));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// `stroke-dasharray: 5` shifted by `phase` (0..10). GPUI has no dash offset, so the first
/// period is split so the repeating array reproduces the offset pattern exactly.
fn dash_pattern(phase: f32) -> [f32; 4] {
    if phase < 5. { [5. - phase, 5., phase, 0.] } else { [0., 10. - phase, 5., phase - 5.] }
}

fn paint_arrow(curve: Bezier, color: Hsla, scale: f32, window: &mut Window) {
    let [a, tip, b] = xyflow::arrow_closed(curve.to, curve.end_direction(), scale);
    let mut head = PathBuilder::fill();
    head.move_to(pt(a));
    head.line_to(pt(tip));
    head.line_to(pt(b));
    head.close();
    if let Ok(head) = head.build() {
        window.paint_path(head, color);
    }
}

/// `<MiniMap />`: 200×150 at the bottom-right panel margin. Node rects (radius 5) and a mask
/// outside the visible area. Click or drag pans (handled by the pane's pointer input).
fn minimap(view: &FactoryCanvas, theme: FlowTheme, cx: &mut Context<FactoryCanvas>) -> impl IntoElement + use<> {
    let nodes: Vec<Rect> = view.graph.nodes.iter().map(node_rect).collect();
    let frame = view.minimap_frame();
    let mini = canvas(
        |_, _, _| {},
        move |area, _, window, _| {
            let Some((visible, top_left, unit)) = frame else { return };
            let map = |r: Rect| {
                Bounds::new(
                    area.origin + point(px((r.x - top_left.x) / unit), px((r.y - top_left.y) / unit)),
                    size(px(r.width / unit), px(r.height / unit)),
                )
            };
            for node in &nodes {
                window.paint_quad(quad(map(*node), px(5. / unit), theme.minimap_node, px(0.), transparent_black(), BorderStyle::Solid));
            }
            for band in mask_bands(area, map(visible)) {
                window.paint_quad(fill(band, theme.minimap_mask));
            }
        },
    )
    .size_full();
    div()
        .absolute()
        .right(px(style::PANEL_MARGIN))
        .bottom(px(style::PANEL_MARGIN))
        .w(px(style::MINIMAP_W))
        .h(px(style::MINIMAP_H))
        .overflow_hidden()
        .bg(theme.minimap_bg)
        .cursor(CursorStyle::PointingHand)
        .occlude()
        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            this.pane_down(event, window, cx);
        }))
        .child(mini)
}

/// Four rects covering `area` outside `hole` (the SVG mask's even-odd path).
fn mask_bands(area: Bounds<Pixels>, hole: Bounds<Pixels>) -> [Bounds<Pixels>; 4] {
    let hole = hole.intersect(&area);
    let (l, t, r, b) = (area.left(), area.top(), area.right(), area.bottom());
    [
        Bounds::from_corners(point(l, t), point(r, hole.top())),
        Bounds::from_corners(point(l, hole.bottom()), point(r, b)),
        Bounds::from_corners(point(l, hole.top()), point(hole.left(), hole.bottom())),
        Bounds::from_corners(point(hole.right(), hole.top()), point(r, hole.bottom())),
    ]
}
