//! Diff: stored patches grouped by scope. "Tour" holds every chapter diff block, and each
//! commit is its own scope, so no patch is merged or dropped. A file list focuses entries.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    Icon, Selectable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    diff::{Diff, DiffChangeIndicator, DiffHoverHighlight, DiffMode},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::CodeTour;

use super::{
    ReviewView,
    files::{FileEntry, chapter_entries, commit_entries, file_name, short_sha},
    guide::{diff_height, raw_patch},
    panel,
};
use crate::{
    components::{Edge, Tone, Typography as _, button_sm, change_counts, edge_band, file_path_label, icon_button},
    theme::{self, metrics},
};

#[derive(Clone, PartialEq, Eq)]
pub enum DiffScope {
    Tour,
    Commit(String),
}

fn scope_entries(tour: &CodeTour, scope: &DiffScope) -> Vec<FileEntry> {
    match scope {
        DiffScope::Tour => chapter_entries(tour),
        DiffScope::Commit(sha) => tour
            .commits
            .iter()
            .flatten()
            .find(|commit| &commit.sha == sha)
            .map(commit_entries)
            .unwrap_or_default(),
    }
}

/// Tour patches when there are any, otherwise the newest commit.
fn default_scope(tour: &CodeTour) -> DiffScope {
    if !chapter_entries(tour).is_empty() {
        return DiffScope::Tour;
    }
    match tour.commits.iter().flatten().last() {
        Some(commit) => DiffScope::Commit(commit.sha.clone()),
        None => DiffScope::Tour,
    }
}

fn scope_label(tour: &CodeTour, scope: &DiffScope) -> String {
    match scope {
        DiffScope::Tour => format!("Tour patches ({})", chapter_entries(tour).len()),
        DiffScope::Commit(sha) => {
            let subject = tour
                .commits
                .iter()
                .flatten()
                .find(|commit| &commit.sha == sha)
                .map(|commit| commit.message.lines().next().unwrap_or_default().to_owned())
                .unwrap_or_default();
            format!("{} {subject}", short_sha(sha))
        }
    }
}

impl ReviewView {
    /// Opens the Diff tab on the scope holding `path` (chapter patch first, else the newest
    /// commit that touched it) and scrolls to it. The scroll request is kept by the handle
    /// until the list is laid out, so it works on first open.
    pub(super) fn focus_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(tour) = self.tour().cloned() else { return };
        let mut scopes = vec![DiffScope::Tour];
        scopes.extend(tour.commits.iter().flatten().rev().map(|commit| DiffScope::Commit(commit.sha.clone())));
        let found = scopes.into_iter().find_map(|scope| {
            let index = scope_entries(&tour, &scope).iter().position(|entry| entry.path == path)?;
            Some((scope, index))
        });
        self.tab = super::ReviewTab::Diff;
        if let Some((scope, index)) = found {
            self.diff_scope = Some(scope);
            self.diff_scroll.scroll_to_top_of_item(index);
        }
        cx.notify();
    }

    pub(super) fn render_diff_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let Some(tour) = self.tour().cloned() else { return div().into_any_element() };
        let scope = self.diff_scope.clone().unwrap_or_else(|| default_scope(&tour));
        let entries = scope_entries(&tour, &scope);
        let reviewed = entries.iter().filter(|entry| self.is_file_reviewed(&entry.path)).count();
        let rows: Vec<AnyElement> = entries.iter().enumerate().map(|(ix, entry)| self.render_file_row(ix, entry, cx)).collect();
        let viewers: Vec<AnyElement> = entries.iter().map(|entry| self.render_entry(entry, cx)).collect();
        let body: AnyElement = if entries.is_empty() {
            crate::components::empty_state("No patches in this scope", "Pick another scope, or add diff blocks to the tour.").into_any_element()
        } else {
            div()
                .id("diff-entries")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.diff_scroll)
                .flex()
                .flex_col()
                .gap_4()
                .px(px(metrics::GUTTER_X))
                .pt(px(metrics::GUTTER_TOP))
                .pb_6()
                .children(viewers)
                .into_any_element()
        };
        // Below 760 of allocated width the 300 file list would leave the diff ~400: stack the
        // list above the diffs (capped, scrolling) so both stay usable.
        let stacked = crate::views::responsive::narrower_than(&self.body_width, 760.);
        div()
            .size_full()
            .flex()
            .when(stacked, |this| this.flex_col())
            .child(
                div()
                    .when(!stacked, |this| this.w(px(300.)).h_full().border_r_1())
                    .when(stacked, |this| this.w_full().max_h(px(220.)).border_b_1())
                    .flex_none()
                    .flex()
                    .flex_col()
                    .border_color(t.border)
                    .child(self.render_scope_bar(&tour, &scope, cx))
                    .child(
                        div()
                            .px(px(metrics::GUTTER_X))
                            .pb_2()
                            .t_caption()
                            .text_color(t.text_secondary)
                            .child(format!("{reviewed} of {} files reviewed", entries.len())),
                    )
                    .child(div().id("diff-files").flex_1().overflow_y_scroll().px(px(metrics::GUTTER_X - 8.)).pb_3().flex().flex_col().gap_0p5().children(rows)),
            )
            .child(div().flex_1().min_w_0().min_h_0().child(body))
            .into_any_element()
    }

    fn render_scope_bar(&self, tour: &CodeTour, scope: &DiffScope, cx: &mut Context<Self>) -> impl IntoElement {
        let mut scopes = vec![(DiffScope::Tour, scope_label(tour, &DiffScope::Tour))];
        scopes.extend(tour.commits.iter().flatten().map(|commit| {
            let scope = DiffScope::Commit(commit.sha.clone());
            let label = scope_label(tour, &scope);
            (scope, label)
        }));
        let current = scope.clone();
        let view = cx.entity().downgrade();
        let picker = button_sm(Button::new("diff-scope").ghost().label(scope_label(tour, scope)))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                scopes.iter().fold(menu, |menu, (scope, label)| {
                    let (view, scope) = (view.clone(), scope.clone());
                    menu.item(PopupMenuItem::new(label.clone()).checked(scope == current).on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.diff_scope = Some(scope.clone());
                            this.diff_scroll.scroll_to_item(0);
                            cx.notify();
                        })
                        .ok();
                    }))
                })
            });
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_1()
            // List text sits on the page header's 24 edge (controls' 8pt inner pad included).
            .px(px(metrics::GUTTER_X - 8.))
            // Controls sit 16 from the top like the list's 16 side inset.
            .pt_4()
            .pb_2()
            .child(div().flex_1().min_w_0().overflow_hidden().child(picker))
            .child(
                icon_button(Button::new("diff-unified").ghost().icon(Lucide::Rows2), metrics::CONTROL_SM).tooltip("Unified").selected(!self.split_diff)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.split_diff = false;
                        cx.notify();
                    })),
            )
            .child(
                icon_button(Button::new("diff-split").ghost().icon(Lucide::Columns2), metrics::CONTROL_SM).tooltip("Split").selected(self.split_diff)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.split_diff = true;
                        cx.notify();
                    })),
            )
    }

    fn render_entry(&mut self, entry: &FileEntry, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let state = match self.diff_state(&entry.key, &entry.patch, cx) {
            Ok(state) => state,
            Err(message) => return raw_patch(&entry.origin, &entry.path, &entry.patch, &message, cx),
        };
        let mode = if self.split_diff { DiffMode::Split } else { DiffMode::Unified };
        if state.read(cx).mode() != mode {
            state.update(cx, |state, cx| state.set_mode(mode, cx));
        }
        let path = entry.path.clone();
        // Header: the filename group (basis 200) and the meta group (chip, ±, Reviewed) share
        // one 40 line when the card allows; otherwise meta wraps to its own line under the
        // full filename. The card itself is `min_w_0`, so it follows the pane, not its content.
        panel(cx)
            .min_w_0()
            .child(
                edge_band(Edge::Top, metrics::R_CARD)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x_3()
                    .gap_y_1()
                    .min_h(px(40.))
                    .py_2()
                    .px_4()
                    .border_b_1()
                    .border_color(t.border)
                    .child(div().flex_1().flex_basis(px(200.)).min_w_0().flex().items_center().gap_2()
                        .child(Icon::new(Lucide::FileCode).size_3p5().flex_none().text_color(t.text_secondary))
                        .child(div().flex_1().min_w_0().overflow_hidden().flex().child(file_path_label(&entry.path, None, true, cx))))
                    .child(div().ml_auto().max_w_full().min_w_0().flex().items_center().gap_2()
                    .child(origin_chip(&entry.key, entry.origin.clone(), cx))
                    .child(change_counts(entry.additions, entry.deletions, cx))
                    .child(
                        div().flex_none().t_label().text_color(t.text_secondary).child(
                            Checkbox::new(SharedString::from(format!("entry-reviewed-{}", entry.key)))
                                .label("Reviewed")
                                .checked(self.is_file_reviewed(&entry.path))
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| this.set_file_reviewed(&path, *checked, cx))),
                        ),
                    )),
            )
            // The viewer's rows clip to a rectangle: it ends R_CARD above the card's bottom
            // edge and has no border of its own (the card draws it).
            .child(
                // Explicit full width: the viewer sizes its row fills from its viewport.
                div().w_full().h(diff_height(&entry.patch, false, self.split_diff)).mb(px(metrics::R_CARD)).child(
                    Diff::new(&state)
                        .size_full()
                        .header_visible(false)
                        .border_0()
                        .bg(t.surface)
                        .soft_wrap(!self.split_diff)
                        .hover_highlight(DiffHoverHighlight::Line)
                        .change_indicator(DiffChangeIndicator::Bars),
                ),
            )
            .into_any_element()
    }

    fn render_file_row(&self, ix: usize, entry: &FileEntry, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let (dir, name) = file_name(&entry.path);
        let reviewed = self.is_file_reviewed(&entry.path);
        let check_path = entry.path.clone();
        crate::components::list_row(("diff-file", ix), false, cx)
            .h(px(44.))
            .child(
                Checkbox::new(("diff-file-reviewed", ix))
                    .checked(reviewed)
                    .accessibility_label(format!("Mark {} reviewed", entry.path))
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        cx.stop_propagation();
                        this.set_file_reviewed(&check_path, *checked, cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child(name.to_owned()))
                    .child(div().truncate().t_caption().text_color(t.text_secondary).child(format!("{} · {}", dir.trim_end_matches('/'), entry.origin))),
            )
            .child(change_counts(entry.additions, entry.deletions, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.diff_scroll.scroll_to_top_of_item(ix);
                cx.notify();
            }))
            .into_any_element()
    }
}

/// Chapter/commit origin chip for the diff header: `chip` geometry, but it can shrink (max
/// 140) and its label ellipsizes, with the full text as a tooltip.
fn origin_chip(key: &str, label: String, cx: &App) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    let tooltip = SharedString::from(label.clone());
    div()
        .id(SharedString::from(format!("entry-origin-{key}")))
        .flex()
        .flex_shrink(1.)
        .min_w_0()
        .max_w(px(140.))
        .items_center()
        .gap_1p5()
        .h(px(metrics::CHIP_H))
        .px_1p5()
        .rounded(px(metrics::R_CONTROL))
        .bg(t.tray)
        .t_caption()
        .text_color(t.text)
        .child(crate::components::status_dot(Tone::Neutral, cx).size(px(6.)).flex_none())
        .child(div().min_w_0().truncate().child(label))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx))
}
