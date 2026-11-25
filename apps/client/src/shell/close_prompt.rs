//! Save / Discard / Cancel prompt for closing a tab with unsaved edits.

use gpui_kit::component::{
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    notification::Notification,
};
use gpui_kit::*;

use crate::{
    components::{Typography as _, button_md},
    state::{DraftHandle, TabTarget, Workspace},
    theme,
};

pub fn open(
    workspace: WeakEntity<Workspace>,
    target: TabTarget,
    title: SharedString,
    draft: DraftHandle,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let (save_ws, save_target, save_draft) = (workspace.clone(), target.clone(), draft.clone());
        let (discard_ws, discard_target) = (workspace.clone(), target.clone());
        dialog
            .w(px(420.))
            // Kit pins the 24 close button 8 from the top-right inside the title's 16 padding;
            // 24 more keeps a long title wrapping 8 clear of it.
            .title(div().pr(px(24.)).child(format!("Save changes to “{title}”?")))
            .child(
                div()
                    .t_body()
                    .text_color(theme::tokens(cx).text_secondary)
                    .child("Your edits will be lost if you close this tab without saving."),
            )
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(button_md(Button::new("close-cancel")).label("Cancel").on_click(|_, window, cx| {
                        window.close_dialog(cx);
                    }))
                    .child(button_md(Button::new("close-discard")).danger().label("Discard").on_click(
                        move |_, window, cx| {
                            window.close_dialog(cx);
                            let target = discard_target.clone();
                            discard_ws.update(cx, |workspace, cx| workspace.close_target(&target, cx)).ok();
                        },
                    ))
                    .child(button_md(Button::new("close-save")).primary().label("Save").on_click(
                        move |_, window, cx| {
                            window.close_dialog(cx);
                            save_and_close(save_ws.clone(), save_target.clone(), &save_draft, window, cx);
                        },
                    )),
            )
    });
}

fn save_and_close(
    workspace: WeakEntity<Workspace>,
    target: TabTarget,
    draft: &DraftHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let save = draft.save(window, cx);
    let draft = draft.clone();
    window
        .spawn(cx, async move |cx| {
            let result = save.await;
            cx.update(|window, cx| match result {
                // Edits made while saving are still unsaved: keep the tab.
                Ok(()) if draft.is_dirty(cx) => {
                    window.push_notification(
                        Notification::warning("Newer edits are unsaved; the tab was kept open."),
                        cx,
                    );
                }
                Ok(()) => {
                    workspace.update(cx, |workspace, cx| workspace.close_target(&target, cx)).ok();
                }
                Err(error) => {
                    // Keep the tab open so the draft is not lost.
                    window.push_notification(
                        Notification::error(error.to_string()).title("Couldn't save; tab kept open"),
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
}
