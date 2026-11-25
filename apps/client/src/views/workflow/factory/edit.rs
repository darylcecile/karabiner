//! Editor commands for the factory pane: undo/redo (the shared config history), copy, cut,
//! paste and duplicate of stages. The canvas emits them from its keyboard handling (with the
//! selection or paste position); the toolbar's Undo/Redo call them directly.

use gpui_kit::*;
use protocol::LayoutPosition;

use super::{FactoryPane, Selected, clipboard::{self, Clip}, graph};
use crate::components::factory_canvas::Selection;

/// Offset of a duplicate from its original, in canvas units.
const DUPLICATE_OFFSET: f64 = 40.;

impl FactoryPane {
    fn stage_ids(&self, selection: &Selection) -> Vec<String> {
        let mut ids: Vec<String> = selection.nodes.iter().map(|id| id.to_string()).collect();
        if ids.is_empty() { ids.extend(self.selected_stage_id().map(str::to_string)) }
        ids
    }

    fn clip(&self, ids: &[String], cx: &App) -> Option<Clip> {
        clipboard::copy(&self.config_value(cx)?, self.workflow, ids)
    }

    pub(super) fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.config.update(cx, |config, cx| config.undo(cx));
        self.sync_fields(window, cx);
    }

    pub(super) fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.config.update(cx, |config, cx| config.redo(cx));
        self.sync_fields(window, cx);
    }

    pub(super) fn copy(&mut self, selection: &Selection, cx: &mut Context<Self>) {
        if let Some(clip) = self.clip(&self.stage_ids(selection), cx) { cx.write_to_clipboard(ClipboardItem::new_string(clip.to_text())) }
        self.sync_capabilities(cx);
    }

    pub(super) fn cut(&mut self, selection: &Selection, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clip) = self.clip(&self.stage_ids(selection), cx) else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(clip.to_text()));
        self.sync_capabilities(cx);
        // Exactly what was copied (a fork or join brings its whole region), in one step.
        let ids = clip.stage_ids();
        self.edit_workflow(cx, |workflow| graph::remove_stages(workflow, &ids));
        self.select(Selected::Workflow, window, cx);
    }

    pub(super) fn paste(&mut self, anchor: LayoutPosition, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clip) = cx.read_from_clipboard().and_then(|item| item.text()).and_then(|text| Clip::from_text(&text)) else { return };
        self.insert(clip, anchor, window, cx);
    }

    pub(super) fn duplicate(&mut self, selection: &Selection, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clip) = self.clip(&self.stage_ids(selection), cx) else { return };
        // The copied region's own top-left, so every duplicated stage moves by the same offset.
        let Some(origin) = clip.origin() else { return };
        self.insert(clip, LayoutPosition { x: origin.x + DUPLICATE_OFFSET, y: origin.y + DUPLICATE_OFFSET }, window, cx);
    }

    /// One history step; the pasted stages become the selection.
    fn insert(&mut self, clip: Clip, anchor: LayoutPosition, window: &mut Window, cx: &mut Context<Self>) {
        let workflow = self.workflow;
        let mut added = Vec::new();
        self.config.update(cx, |config, cx| config.edit(cx, |config| added = clipboard::paste(config, workflow, clip, anchor)));
        let nodes = added.iter().map(|id| id.clone().into()).collect();
        self.canvas.update(cx, |canvas, cx| canvas.select(Selection { nodes, edges: Vec::new() }, cx));
        self.selected = added.first().cloned().map_or(Selected::Workflow, Selected::Stage);
        self.sync_fields(window, cx);
        cx.notify();
    }
}
