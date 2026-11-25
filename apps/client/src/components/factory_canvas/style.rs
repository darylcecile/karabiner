//! React Flow's default theme (`@xyflow/react` 13.0.3 `dist/xy-theme.css`, the styling shown
//! on reactflow.dev examples) as native colours and metrics. Values are CSS px at zoom 1.
//! Copyright (c) 2019-2025 webkid GmbH, MIT licensed; full notice in `xyflow.rs`.

use gpui_kit::*;

pub const NODE_W: f32 = 150.;
pub const NODE_PAD: f32 = 10.;
pub const NODE_RADIUS: f32 = 8.;
pub const NODE_TEXT: f32 = 12.;
pub const NODE_LINE: f32 = 16.;
pub const DETAIL_TEXT: f32 = 10.;
pub const DETAIL_LINE: f32 = 16.;
pub const HANDLE: f32 = 6.;
/// Hit area of a handle and the spacing between stacked outcome handles on the right edge.
pub const HANDLE_HIT: f32 = 14.;
pub const HANDLE_GAP: f32 = 16.;
/// Badge chips row under the node text: 14 chip + 4 gap above.
pub const BADGE_ROW: f32 = 18.;
pub const BADGE_H: f32 = 14.;
pub const EDGE_TEXT: f32 = 10.;
pub const DOT_GAP: f32 = 20.;
pub const DOT_RADIUS: f32 = 0.5;
pub const CONTROL_BUTTON: f32 = 26.;
pub const CONTROL_ICON: f32 = 12.;
pub const PANEL_MARGIN: f32 = 15.;
pub const MINIMAP_W: f32 = 200.;
pub const MINIMAP_H: f32 = 150.;

#[derive(Clone, Copy)]
pub struct FlowTheme {
    pub background: Hsla,
    pub dots: Hsla,
    pub node_bg: Hsla,
    pub node_border: Hsla,
    pub node_hover: Hsla,
    pub selected: Hsla,
    pub node_text: Hsla,
    pub node_detail: Hsla,
    pub focus_ring: Hsla,
    pub focus_border: Hsla,
    pub handle_bg: Hsla,
    pub handle_border: Hsla,
    pub edge: Hsla,
    pub edge_hover: Hsla,
    pub edge_label: Hsla,
    pub selection_bg: Hsla,
    pub selection_border: Hsla,
    pub controls_bg: Hsla,
    pub controls_hover: Hsla,
    pub controls_border: Hsla,
    pub controls_text: Hsla,
    pub controls_shadow: Hsla,
    pub minimap_bg: Hsla,
    pub minimap_node: Hsla,
    pub minimap_mask: Hsla,
}

impl FlowTheme {
    pub fn current(cx: &App) -> Self {
        if gpui_kit::component::Theme::global(cx).is_dark() { Self::dark() } else { Self::light() }
    }

    fn light() -> Self {
        Self {
            background: rgb(0xF7F9FB).into(),
            dots: rgb(0x91919A).into(),
            node_bg: rgb(0xFFFFFF).into(),
            node_border: rgb(0xEDEDED).into(),
            node_hover: rgb(0xC5C5C5).into(),
            selected: rgb(0xF57DBD).into(),
            node_text: rgb(0x000000).into(),
            node_detail: rgb(0x505050).into(),
            focus_ring: rgb(0xE8E8E8).into(),
            focus_border: rgb(0xD9D9D9).into(),
            handle_bg: rgb(0xFFFFFF).into(),
            handle_border: rgb(0xAAAAAA).into(),
            edge: rgb(0xB1B1B7).into(),
            edge_hover: rgb(0x222222).into(),
            edge_label: rgb(0x505050).into(),
            selection_bg: rgba(0x9696B41A).into(),
            selection_border: rgba(0x9B9B9BCC).into(),
            controls_bg: rgb(0xFEFEFE).into(),
            controls_hover: rgb(0xF4F4F4).into(),
            controls_border: rgb(0xEEEEEE).into(),
            controls_text: rgb(0x000000).into(),
            controls_shadow: rgba(0x00000014).into(),
            minimap_bg: rgb(0xFFFFFF).into(),
            minimap_node: rgb(0xE2E2E2).into(),
            minimap_mask: rgba(0xF0F0F099).into(),
        }
    }

    fn dark() -> Self {
        Self {
            background: rgb(0x101012).into(),
            dots: rgb(0x555555).into(),
            node_bg: rgb(0x1A1B1E).into(),
            node_border: rgb(0x303238).into(),
            node_hover: rgb(0x7A7D86).into(),
            selected: rgb(0xF57DBD).into(),
            node_text: rgb(0xF8F8F8).into(),
            node_detail: rgb(0xE6E7EA).into(),
            focus_ring: rgb(0x4B4E57).into(),
            focus_border: rgb(0x5F6470).into(),
            handle_bg: rgb(0x141519).into(),
            handle_border: rgb(0x5A5D66).into(),
            edge: rgb(0x3E3E3E).into(),
            edge_hover: rgb(0xF6F7F5).into(),
            edge_label: rgb(0xE6E7EA).into(),
            selection_bg: rgba(0xC8C8DC14).into(),
            selection_border: rgba(0xC8C8DCCC).into(),
            controls_bg: rgb(0x2B2B2B).into(),
            controls_hover: rgb(0x3E3E3E).into(),
            controls_border: rgb(0x5B5B5B).into(),
            controls_text: rgb(0xF8F8F8).into(),
            controls_shadow: rgba(0x00000014).into(),
            minimap_bg: rgb(0x141414).into(),
            minimap_node: rgb(0x2B2B2B).into(),
            minimap_mask: rgba(0x3C3C3C99).into(),
        }
    }
}
