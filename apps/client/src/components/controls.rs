//! Shared controls: chips, pills, counts, rows, segmented, meter, file rows, and the
//! contract heights for Kit buttons and inputs (Kit's own sizes are rem-based).

use gpui_kit::component::{Sizable as _, button::Button, input::{Input, Textarea, TextareaState}};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Tone, status_dot, typography::{Typography as _, tabular}};
use crate::theme::{self, metrics};

/// Page/toolbar action button (`769AF488` md): 28 high, R8, 13pt.
pub fn button_md(button: Button) -> Button {
    button.small().h(px(metrics::CONTROL_MD)).px_3().rounded(px(metrics::R_INNER)).t_label()
}

/// Settings-row / banner action ("Open →", "Upgrade" in `92D96623`): 28 high with the
/// rounder radius 10 that reference uses. Caller picks the variant (outline/primary).
pub fn button_row_action(button: Button) -> Button {
    button_md(button).rounded(px(metrics::R_POPOVER))
}

/// Footer call-to-action pill ("Use template" in `85F1DC37`): 32 high, fully rounded.
pub fn button_pill(button: Button) -> Button {
    button.small().h(px(32.)).px_4().rounded_full().t_label().font_weight(FontWeight::MEDIUM)
}

/// Compact button: 24 high, 13pt.
pub fn button_sm(button: Button) -> Button {
    button.small().t_label()
}

/// Inline button ("Edit"): 20 high, 12pt.
pub fn button_xs(button: Button) -> Button {
    button.xsmall().t_caption()
}

/// Square icon-only button with a `size` hitbox (28 chrome, 24 toolbar, 20 inline).
/// Glyph and hitbox are set separately: Kit's `with_size(px)` would also scale the glyph
/// to 0.75× the hitbox (`button.rs:659`). Glyph follows the icon roles: 16 for 28+,
/// 14 for 24, 12 below.
pub fn icon_button(button: Button, size: f32) -> Button {
    let glyph = if size >= metrics::CONTROL_MD {
        gpui_kit::component::Size::Medium
    } else if size >= metrics::CONTROL_SM {
        gpui_kit::component::Size::Small
    } else {
        gpui_kit::component::Size::XSmall
    };
    button.with_size(glyph).w(px(size)).h(px(size)).p_0()
}

/// Single-line Kit input at `height` (22/24/28/32). Kit's inherent `Input::h` only sizes
/// multi-line editors; single-line fields size from `Size`, so this refines the box's own
/// (Styled) height and keeps Small's 2pt vertical inset.
pub fn input_h(input: Input, height: f32) -> Input {
    <Input as Styled>::h(input.small(), px(height))
}

/// Form input/select: 28 high, 13pt.
pub fn input_md(input: Input) -> Input {
    input_h(input, metrics::CONTROL_MD).t_label()
}

/// Page search field: 32 high, 13pt.
pub fn input_lg(input: Input) -> Input {
    input_h(input, 32.).t_label()
}

/// Right-aligned count (sidebar, column headers). Renders nothing for `None`.
pub fn count_badge(count: Option<usize>, cx: &App) -> Option<Div> {
    let count = count?;
    Some(
        div()
            .flex_none()
            .min_w(px(16.))
            .t_caption()
            .font_features(tabular())
            .text_right()
            .text_color(theme::tokens(cx).text_secondary)
            .child(count.to_string()),
    )
}

/// Tag with a tone dot ("Finance", "Working"): 20 high, radius 6, 12pt.
pub fn chip(label: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .h(px(metrics::CHIP_H))
        .px_1p5()
        .rounded(px(metrics::R_CONTROL))
        .bg(t.tray)
        .t_caption()
        .text_color(t.text)
        .child(status_dot(tone, cx).size(px(6.)))
        .child(label.into())
}

/// Solid status pill ("Active"): tone tint fill, tone text, fully rounded.
pub fn status_pill(label: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(metrics::CHIP_H))
        .px_2()
        .rounded_full()
        .bg(tone.tint(cx, 0.15))
        .t_caption()
        .font_weight(FontWeight::MEDIUM)
        .text_color(tone.color(cx))
        .child(label.into())
}

/// List row: 36 high, radius 6, hover/selected fills, 13pt.
pub fn list_row(id: impl Into<ElementId>, selected: bool, cx: &App) -> Stateful<Div> {
    let t = theme::tokens(cx);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(px(metrics::ROW_LIST))
        .px_2()
        .rounded(px(metrics::R_CONTROL))
        .t_label()
        .cursor_pointer()
        .when(selected, |row| row.bg(t.selected))
        .when(!selected, |row| row.hover(move |style| style.bg(t.hover)))
}

/// One segmented-control item: label and optional count.
#[derive(Debug, Clone)]
pub struct Segment {
    pub label: SharedString,
    pub count: Option<usize>,
}

impl Segment {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), count: None }
    }

    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }
}

/// Segmented control: 28 track (padding 2, radius 8), 24 items (radius 6), active item is
/// a raised surface. `on_select` receives the clicked index.
pub fn segmented(
    id: impl Into<SharedString>,
    items: Vec<Segment>,
    selected: usize,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let t = theme::tokens(cx);
    let id: SharedString = id.into();
    let on_select = std::rc::Rc::new(on_select);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_0p5()
        .h(px(metrics::CONTROL_MD))
        .p_0p5()
        .rounded(px(metrics::R_INNER))
        .bg(t.tray)
        .children(items.into_iter().enumerate().map(|(index, item)| {
            let active = index == selected;
            let on_select = on_select.clone();
            div()
                .id(ElementId::Name(format!("{id}:{index}").into()))
                .flex()
                .items_center()
                .gap_1p5()
                .h(px(metrics::CONTROL_SM))
                .px_2p5()
                .rounded(px(metrics::R_CONTROL))
                .t_label()
                .cursor_pointer()
                .when(active, |this| this.bg(t.surface).text_color(t.text).shadow_xs())
                .when(!active, |this| this.text_color(t.text_secondary).hover(move |s| s.text_color(t.text)))
                .child(item.label)
                .when_some(item.count, |this, count| {
                    this.child(div().t_caption().font_features(tabular()).text_color(t.text_secondary).child(count.to_string()))
                })
                .on_click(move |_, window, cx| on_select(index, window, cx))
        }))
}

/// Segment meter (risk, effort): `segments` pills of 10×4, gap 2, the first `filled` toned.
pub fn meter(segments: usize, filled: usize, tone: Tone, cx: &App) -> Div {
    let t = theme::tokens(cx);
    let on = tone.color(cx);
    div().flex().flex_none().items_center().gap_0p5().children((0..segments).map(move |index| {
        div().w(px(10.)).h(px(4.)).rounded_full().bg(if index < filled { on } else { t.border })
    }))
}

/// Path label: directory secondary (truncates first), basename primary, optional suffix
/// such as a line range (":12-30") in secondary. `emphasized` makes the basename medium.
pub fn file_path_label(path: &str, suffix: Option<SharedString>, emphasized: bool, cx: &App) -> Div {
    let t = theme::tokens(cx);
    let (dir, name) = match path.rfind('/') {
        Some(split) => (path[..=split].to_string(), path[split + 1..].to_string()),
        None => (String::new(), path.to_string()),
    };
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .t_label()
        .child(div().flex_shrink(1.).min_w_0().truncate().text_color(t.text_secondary).child(dir))
        .child(div().flex_none().text_color(t.text).when(emphasized, |el| el.font_weight(FontWeight::MEDIUM)).child(name))
        .when_some(suffix, |this, suffix| this.child(div().flex_none().text_color(t.text_secondary).child(suffix)))
}

/// `+adds −dels` in mono caption, success/danger.
pub fn change_counts(adds: usize, dels: usize, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_none()
        .gap_1p5()
        .t_code_caption(cx)
        .child(div().text_color(t.success).child(format!("+{adds}")))
        .child(div().text_color(t.danger).child(format!("−{dels}")))
}

/// Changed-file row: icon, [`file_path_label`], [`change_counts`]. 28 high, R6 hover.
/// For other heights or trailing parts, compose the two sub-helpers in a local row.
pub fn file_row(
    id: impl Into<ElementId>,
    path: &str,
    adds: usize,
    dels: usize,
    selected: bool,
    cx: &App,
) -> Stateful<Div> {
    let t = theme::tokens(cx);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(px(metrics::ROW_PROP))
        .px_2()
        .rounded(px(metrics::R_CONTROL))
        .cursor_pointer()
        .when(selected, |row| row.bg(t.selected))
        .when(!selected, |row| row.hover(move |s| s.bg(t.hover)))
        .child(gpui_kit::component::Icon::new(gpui_kit::assets::IconName::FileText).size_3p5().text_color(t.text_secondary))
        .child(file_path_label(path, None, true, cx))
        .child(change_counts(adds, dels, cx))
}

/// Bare prose field (task description): Base's unstyled `Textarea` on the same
/// `TextareaState` engine (editing, undo, IME, history, focus), with no editor padding so
/// its text aligns flush with surrounding text. The styled `Textarea` resets padding to its
/// `Size` inset every render (`input.rs:650`), so it can't do this. Theme ink, selection,
/// caret and the Cut/Copy/Paste/Select All context menu are applied here, matching Kit's
/// Input. Text style is inherited: wrap it in the caller's typography role. Boxed fields
/// stay [`textarea_box`].
pub fn plain_textarea(state: &Entity<TextareaState>, cx: &mut App) -> Div {
    use gpui_kit::component::{
        ActiveTheme as _,
        input::{Copy, Cut, Paste, SelectAll},
        native_menu::NativeMenu,
    };
    let theme = cx.theme();
    let style = gpui_kit::base::input::InputEditorStyle {
        foreground: theme.foreground,
        muted_foreground: theme.muted_foreground,
        background: transparent_black(),
        border: transparent_black(),
        selection: theme.selection,
        caret: theme.caret,
        ..Default::default()
    };
    state.update(cx, |state, _| {
        state.set_editor_paddings(Edges::default());
        state.set_editor_style(style);
        state.on_context_menu(std::rc::Rc::new(|_, capabilities, position, window, cx| {
            let editable = !capabilities.is_disabled() && !capabilities.is_readonly();
            NativeMenu::new()
                .menu_with_disabled("Cut", !(editable && capabilities.is_copyable()), Box::new(Cut))
                .menu_with_disabled("Copy", !capabilities.is_copyable(), Box::new(Copy))
                .menu_with_disabled("Paste", !editable, Box::new(Paste))
                .separator()
                .menu("Select All", Box::new(SelectAll))
                .show(position, window, cx);
        }));
    });
    div().w_full().child(gpui_kit::base::input::Textarea::new(state))
}

/// Bordered multi-line field: frame (radius 6, border, surface, 13pt) around a Kit
/// `Textarea` with its own appearance off. Kit pads multi-line editors itself (10/8 at
/// Medium, `Input::set_editor_paddings`), matching a single-line input's text inset, so
/// the frame adds none.
pub fn textarea_box(state: &Entity<TextareaState>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .rounded(px(metrics::R_CONTROL))
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .t_label()
        .child(Textarea::new(state).appearance(false))
}
