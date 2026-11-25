//! Editing commands shared by keyboard shortcuts and the context menu, plus keyboard reveal,
//! gesture cancel and edge auto-pan. Commands only emit [`CanvasEvent`]s; the owner edits
//! the configuration and keeps undo history and the clipboard.

use gpui_kit::{assets::IconName, component::Icon, prelude::FluentBuilder as _, *};

use super::{CanvasEvent, FactoryCanvas, Gesture, Selection, node_rect, xyflow::{self, Xy, xy}};
use crate::{components::Typography as _, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    SelectAll,
    Undo,
    Redo,
    FitView,
    FitSelection,
}

impl Command {
    fn label(self) -> &'static str {
        match self {
            Command::Cut => "Cut",
            Command::Copy => "Copy",
            Command::Paste => "Paste",
            Command::Duplicate => "Duplicate",
            Command::Delete => "Delete",
            Command::SelectAll => "Select all",
            Command::Undo => "Undo",
            Command::Redo => "Redo",
            Command::FitView => "Fit view",
            Command::FitSelection => "Zoom to selection",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Command::Cut => IconName::Scissors,
            Command::Copy => IconName::Copy,
            Command::Paste => IconName::ClipboardPaste,
            Command::Duplicate => IconName::CopyPlus,
            Command::Delete => IconName::Trash,
            Command::SelectAll => IconName::SquareDashedMousePointer,
            Command::Undo => IconName::Undo2,
            Command::Redo => IconName::Redo2,
            Command::FitView => IconName::Maximize,
            Command::FitSelection => IconName::Focus,
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Command::Cut => "⌘X",
            Command::Copy => "⌘C",
            Command::Paste => "⌘V",
            Command::Duplicate => "⌘D",
            Command::Delete => "⌫",
            Command::SelectAll => "⌘A",
            Command::Undo => "⌘Z",
            Command::Redo => "⇧⌘Z",
            Command::FitView | Command::FitSelection => "",
        }
    }

    /// Commands that change the factory are unavailable while the canvas is locked.
    fn edits(self) -> bool {
        matches!(self, Command::Cut | Command::Paste | Command::Duplicate | Command::Delete | Command::Undo | Command::Redo)
    }
}

/// Context menu at a canvas-local position, opened over a selection or the empty pane.
#[derive(Debug, Clone)]
pub(crate) struct Menu {
    at: Xy,
    commands: Vec<Command>,
}

impl FactoryCanvas {
    /// ⌘-shortcuts. Returns whether the key was handled.
    pub(crate) fn command_key(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) -> bool {
        let command = match key {
            "c" => Command::Copy,
            "x" => Command::Cut,
            "v" => Command::Paste,
            "d" => Command::Duplicate,
            "a" => Command::SelectAll,
            "z" if shift => Command::Redo,
            "z" => Command::Undo,
            "y" => Command::Redo,
            _ => return false,
        };
        self.run(command, cx);
        true
    }

    /// Locked canvas or missing history/clipboard (from the owner's capabilities).
    fn unavailable(&self, command: Command) -> bool {
        let caps = self.capabilities;
        (self.locked && command.edits())
            || matches!((command, caps.can_undo, caps.can_redo, caps.can_paste), (Command::Undo, false, _, _) | (Command::Redo, _, false, _) | (Command::Paste, _, _, false))
    }

    pub(crate) fn run(&mut self, command: Command, cx: &mut Context<Self>) {
        self.menu = None;
        if self.unavailable(command) {
            cx.notify();
            return;
        }
        let selection = self.selection.clone();
        let has_selection = selection != Selection::default();
        match command {
            Command::Copy if has_selection => cx.emit(CanvasEvent::Copy(selection)),
            Command::Cut if has_selection => cx.emit(CanvasEvent::Cut(selection)),
            Command::Duplicate if !selection.nodes.is_empty() => cx.emit(CanvasEvent::Duplicate(selection)),
            Command::Delete if has_selection => cx.emit(CanvasEvent::Delete(selection)),
            Command::Paste => {
                let at = self.pointer.map_or_else(|| self.view_center(), |p| self.viewport.to_flow(p));
                cx.emit(CanvasEvent::Paste { position: at });
            }
            Command::SelectAll => {
                let nodes = self.graph.nodes.iter().map(|n| n.id.clone()).collect();
                let edges = self.graph.edges.iter().map(|e| e.id.clone()).collect();
                self.set_selection(Selection { nodes, edges }, cx);
            }
            Command::Undo => cx.emit(CanvasEvent::Undo),
            Command::Redo => cx.emit(CanvasEvent::Redo),
            Command::FitView => self.fit_view(cx),
            Command::FitSelection => {
                let ids = selection.nodes.clone();
                self.fit_nodes(&ids, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// Plain click selects only this edge; ⌘-click toggles it in the selection.
    pub(crate) fn click_edge(&mut self, edge: SharedString, toggle: bool, cx: &mut Context<Self>) {
        let mut selection = if toggle { self.selection.clone() } else { Selection::default() };
        match selection.edges.iter().position(|e| e == &edge) {
            Some(ix) if toggle => drop(selection.edges.remove(ix)),
            Some(_) => {}
            None => selection.edges.push(edge),
        }
        self.set_selection(selection, cx);
    }

    /// Esc during a drag: nodes return to where they started; connections and boxes end
    /// without an event.
    pub(crate) fn cancel_gesture(&mut self) {
        if let Gesture::Move { origins, .. } = std::mem::replace(&mut self.gesture, Gesture::Idle) {
            for (id, origin) in origins {
                if let Some(node) = self.graph.nodes.iter_mut().find(|n| n.id == id) {
                    node.position = origin;
                }
            }
        }
    }

    /// `autoPanOnNodeFocus`: centres a keyboard-focused node that is not fully visible.
    pub fn reveal(&mut self, id: &SharedString, cx: &mut Context<Self>) {
        let (Some(size), Some(node)) = (self.canvas_size(), self.graph.nodes.iter().find(|n| &n.id == id)) else { return };
        let r = node_rect(node);
        let (a, b) = (self.viewport.to_screen(xy(r.x, r.y)), self.viewport.to_screen(xy(r.x + r.width, r.y + r.height)));
        if a.x >= 0. && a.y >= 0. && b.x <= size.x && b.y <= size.y {
            return;
        }
        let center = xy(r.x + r.width / 2., r.y + r.height / 2.);
        self.viewport.x = size.x / 2. - center.x * self.viewport.zoom;
        self.viewport.y = size.y / 2. - center.y * self.viewport.zoom;
        self.interacted = true;
        self.viewport_changed(cx);
    }

    /// `autoPanOnNodeDrag` / `autoPanOnConnect`: while dragging near an edge, pan each frame
    /// and keep dragged nodes (and the box start) under the pointer. Returns whether it
    /// panned, so the caller requests another frame.
    pub(crate) fn auto_pan_step(&mut self) -> bool {
        let Some(size) = self.canvas_size() else { return false };
        let pointer = match &self.gesture {
            Gesture::Move { pointer, moved: true, .. } => *pointer,
            Gesture::Connect { cursor, .. } | Gesture::ReconnectSource { cursor, .. } | Gesture::Marquee { cursor, .. } => *cursor,
            _ => return false,
        };
        let pan = xyflow::auto_pan(pointer, size.x, size.y);
        if pan == xy(0., 0.) {
            return false;
        }
        self.viewport.x += pan.x;
        self.viewport.y += pan.y;
        self.interacted = true;
        let zoom = self.viewport.zoom;
        match &mut self.gesture {
            Gesture::Move { start, origins, .. } => {
                start.x += pan.x;
                start.y += pan.y;
                let delta = xy((pointer.x - start.x) / zoom, (pointer.y - start.y) / zoom);
                for (id, origin) in origins.iter() {
                    if let Some(node) = self.graph.nodes.iter_mut().find(|n| &n.id == id) {
                        node.position = xy(origin.x + delta.x, origin.y + delta.y);
                    }
                }
            }
            Gesture::Marquee { start, .. } => {
                start.x += pan.x;
                start.y += pan.y;
            }
            _ => {}
        }
        true
    }

    /// Right-click: over a node or edge it acts on the selection (selecting the item first if
    /// it was not selected); on the empty pane it offers paste, select all and view commands.
    pub(crate) fn open_menu(&mut self, local: Xy, on_item: bool, cx: &mut Context<Self>) {
        let commands = if on_item {
            vec![Command::Cut, Command::Copy, Command::Duplicate, Command::FitSelection, Command::Delete]
        } else {
            vec![Command::Paste, Command::SelectAll, Command::Undo, Command::Redo, Command::FitView]
        };
        self.menu = Some(Menu { at: local, commands });
        cx.notify();
    }

    pub(crate) fn pane_right_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let local = self.local(event.position);
        let edge = self.edge_at(local);
        if let Some(edge) = edge.clone().filter(|e| !self.selection.edges.contains(e)) {
            self.click_edge(edge, false, cx);
        }
        self.open_menu(local, edge.is_some(), cx);
    }

    pub(crate) fn node_right_down(&mut self, id: SharedString, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if !self.selection.nodes.contains(&id) {
            self.set_selection(Selection { nodes: vec![id], edges: vec![] }, cx);
        }
        let local = self.local(event.position);
        self.open_menu(local, true, cx);
    }

    /// Native popover (app chrome, not React Flow styling): rows 28, 16 glyph slot, shortcut
    /// captions, clamped inside the canvas.
    pub(crate) fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        const W: f32 = 220.;
        const ROW: f32 = 28.;
        let menu = self.menu.as_ref()?;
        let t = theme::tokens(cx);
        let size = self.canvas_size()?;
        let height = menu.commands.len() as f32 * ROW + 12.;
        let left = menu.at.x.min(size.x - W - 8.).max(8.);
        let top = menu.at.y.min(size.y - height - 8.).max(8.);
        let selection_empty = self.selection == Selection::default();
        let rows = menu.commands.iter().map(|&command| {
            let disabled = self.unavailable(command)
                || (selection_empty && matches!(command, Command::Cut | Command::Copy | Command::Duplicate | Command::Delete | Command::FitSelection));
            div()
                .id(command.label())
                .h(px(ROW))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(6.))
                .t_label()
                .text_color(if disabled { t.text_tertiary } else { t.text })
                .when(!disabled, |el| {
                    el.cursor_pointer()
                        .hover(move |s| s.bg(t.hover))
                        .on_click(cx.listener(move |this, _, _, cx| this.run(command, cx)))
                })
                .child(div().size(px(16.)).flex().items_center().justify_center().child(Icon::new(command.icon()).size(px(14.))))
                .child(div().flex_1().child(command.label()))
                .child(div().t_caption().text_color(t.text_secondary).child(command.shortcut()))
        });
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(W))
                .p(px(6.))
                .flex()
                .flex_col()
                .rounded(px(crate::theme::metrics::R_POPOVER))
                .border_1()
                .border_color(t.border)
                .bg(t.surface)
                .shadow_lg()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .children(rows)
                .into_any_element(),
        )
    }
}
