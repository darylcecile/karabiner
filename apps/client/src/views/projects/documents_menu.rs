//! Project toolbar "Documents" menu: trigger and items, state-free. The project view passes
//! its documents, load error and actions; the visual-states example renders the same
//! function with sample data, so the error/Retry state can be inspected natively.

use std::rc::Rc;

use gpui_kit::component::{
    IconName,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::*;

use crate::components;

/// One project document row.
#[derive(Clone)]
pub struct DocumentEntry {
    pub title: String,
    /// Code tours open as reviews and use the map glyph.
    pub tour: bool,
}

/// Menu callbacks: `on_retry` re-requests the document list; `on_open(index)` and
/// `on_new(template index)` refer to the slices passed in.
#[derive(Clone)]
pub struct DocumentsMenuActions {
    pub on_retry: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_open: Rc<dyn Fn(usize, &mut Window, &mut App)>,
    pub on_new: Rc<dyn Fn(usize, &mut Window, &mut App)>,
}

/// "Documents N" trigger (with " !" after a failed load) and its menu: the load error as
/// a Retry item, or "No documents yet"; the documents; a separator; "New …" per template.
pub fn documents_menu(
    documents: Vec<DocumentEntry>,
    error: Option<String>,
    template_names: Vec<&'static str>,
    actions: DocumentsMenuActions,
) -> impl IntoElement {
    let label = format!(
        "Documents{}{}",
        if documents.is_empty() { String::new() } else { format!(" {}", documents.len()) },
        if error.is_some() { " !" } else { "" },
    );
    components::button_sm(Button::new("project-documents").ghost().icon(IconName::FileText).label(label))
        .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
            if let Some(error) = &error {
                let retry = actions.on_retry.clone();
                menu = menu.item(PopupMenuItem::new(format!("Couldn't load documents: {error}")).icon(IconName::RefreshCw)
                    .on_click(move |_, window, cx| retry(window, cx)));
            } else if documents.is_empty() {
                menu = menu.item(PopupMenuItem::new("No documents yet").disabled(true));
            }
            for (index, document) in documents.iter().enumerate() {
                let open = actions.on_open.clone();
                menu = menu.item(PopupMenuItem::new(document.title.clone()).icon(if document.tour { IconName::Map } else { IconName::FileText })
                    .on_click(move |_, window, cx| open(index, window, cx)));
            }
            menu = menu.separator();
            for (index, name) in template_names.iter().enumerate() {
                let new = actions.on_new.clone();
                menu = menu.item(PopupMenuItem::new(format!("New {}", name.to_lowercase())).icon(IconName::Plus)
                    .on_click(move |_, window, cx| new(index, window, cx)));
            }
            menu
        })
}
