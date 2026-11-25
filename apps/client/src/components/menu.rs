//! Dropdown menus on Kit's public `Popover` + `PopupMenu`, for when a menu needs geometry
//! `Button::dropdown_menu` doesn't expose (gap to the trigger) or 16pt item glyphs (Kit's
//! `PopupMenuItem::icon` is forced to xsmall, `popup_menu.rs:1275`).

use std::rc::Rc;

use gpui_kit::component::{
    button::Button,
    menu::{PopupMenu, PopupMenuItem},
    popover::Popover,
};
use gpui_kit::*;

use crate::theme;

type MenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// `trigger` opening a Kit popup menu at `anchor`, `gap` below it. The menu entity is
/// built once per opening and dropped on dismiss, so items reflect current state.
pub fn menu_popover(
    id: impl Into<SharedString>,
    trigger: Button,
    anchor: Anchor,
    gap: Pixels,
    builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl IntoElement {
    let id: SharedString = id.into();
    let builder: MenuBuilder = Rc::new(builder);
    Popover::new(id.clone())
        .appearance(false)
        .overlay_closable(false)
        .anchor(anchor)
        .offset(gap)
        .trigger(trigger)
        .content(move |_, window, cx| {
            let slot = window.use_keyed_state(SharedString::from(format!("{id}-menu")), cx, |_, _| None::<Entity<PopupMenu>>);
            if let Some(menu) = slot.read(cx).clone() {
                return menu;
            }
            let builder = builder.clone();
            let menu = PopupMenu::build(window, cx, move |menu, window, cx| builder(menu, window, cx));
            slot.update(cx, |slot, _| *slot = Some(menu.clone()));
            menu.focus_handle(cx).focus(window, cx);
            let popover = cx.entity().downgrade();
            let slot = slot.downgrade();
            window
                .subscribe(&menu, cx, move |_, _: &DismissEvent, window, cx| {
                    if let Some(popover) = popover.upgrade() {
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                    }
                    slot.update(cx, |slot, _| *slot = None).ok();
                })
                .detach();
            menu
        })
}

/// Menu item with a 16pt leading glyph slot and a truncating label; `checked` and
/// handlers stay Kit's.
pub fn glyph_item(icon: gpui_kit::assets::IconName, label: SharedString) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| {
        let t = theme::tokens(cx);
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .child(gpui_kit::component::Icon::new(icon.clone()).size_4().flex_none().text_color(t.text_secondary))
            .child(div().min_w_0().truncate().child(label.clone()))
    })
}
