//! Text rotated to read bottom-to-top (collapsed board rail).
//!
//! GPUI paints glyph sprites untransformed, so rotated text goes through an SVG `<text>`
//! rasterised by usvg with the system font database. Font stack: "SF Pro Text" (installed
//! in /Library/Fonts with real weight faces; usvg reads the system SFNS only as a
//! single regular "System Font"), then "SF Pro", "Helvetica Neue", "Helvetica",
//! sans-serif. The SVG is authored at the window scale factor because GPUI rasterises SVG
//! images at 1×. There is deliberately no per-letter fallback.

use std::sync::Arc;

use gpui_kit::*;

const FONT_STACK: &str = "'SF Pro Text', 'SF Pro', 'Helvetica Neue', Helvetica, sans-serif";
const SIZE: f32 = 13.;
/// Column width that fits ascender + descender of 13pt text.
const THICKNESS: f32 = 18.;
/// Distance from the left edge to the baseline after rotation (≈ ascender).
const BASELINE: f32 = 13.;

/// 13pt semibold label rotated 90° counter-clockwise, in `color`.
pub fn vertical_label(text: impl Into<SharedString>, color: Hsla, window: &mut Window) -> impl IntoElement {
    let text: SharedString = text.into();
    let length = measure(&text, window);
    let scale = window.scale_factor();
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {vw} {vh}"><text transform="translate({BASELINE} {vh}) rotate(-90)" font-family="{FONT_STACK}" font-weight="600" font-size="{SIZE}" fill="{fill}">{body}</text></svg>"#,
        w = THICKNESS * scale,
        h = length * scale,
        vw = THICKNESS,
        vh = length,
        fill = hex(color),
        body = escape(&text),
    );
    let image = Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()));
    img(image).flex_none().w(px(THICKNESS)).h(px(length))
}

fn measure(text: &SharedString, window: &mut Window) -> f32 {
    let font = Font { weight: FontWeight::SEMIBOLD, ..font("SF Pro Text") };
    let run = TextRun {
        len: text.len(),
        font,
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(text.clone(), px(SIZE), &[run], None);
    f32::from(line.width).ceil() + 2.
}

fn hex(color: Hsla) -> String {
    let c = color.to_rgb();
    let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", channel(c.r), channel(c.g), channel(c.b))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
