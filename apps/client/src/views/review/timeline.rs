//! Timeline: stored commits on a rail. Commit groups collapse into a pill and expand in
//! place; hovering a node shows its details; selecting one lists its files and diff.

use std::time::Duration;

use gpui_kit::base::motion::transition;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    Icon,
    button::{Button, ButtonVariants as _},
    diff::{Diff, DiffChangeIndicator},
    hover_card::HoverCard,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{CodeTour, FileChangeStatus, TourCommit, TourCommitGroup};

use super::{
    ReviewView,
    files::{commit_patch, short_sha},
    guide::{diff_card, raw_patch},
    panel,
};
use crate::{
    components::{Edge, Tone, Typography as _, button_xs, change_counts, chip, edge_band, status_pill, typography::tabular},
    motion,
    theme::{self, metrics},
};
use super::super::stats::common::relative_time;

const NODE_W: f32 = 76.;
const PILL_W: f32 = 132.;
/// Expanded group slot: fits the natural "Collapse ‹" pill (~76) plus 4 each side.
const PILL_EXPANDED_W: f32 = 84.;
const RAIL_END_W: f32 = 64.;
/// Rail centre line from the top of the rail row.
const RAIL_Y: f32 = 12.;

fn group_of<'a>(tour: &'a CodeTour, sha: &str) -> Option<&'a TourCommitGroup> {
    tour.commit_groups.iter().flatten().find(|group| group.commit_shas.iter().any(|member| member == sha))
}

fn status_letter(status: &FileChangeStatus) -> &str {
    match status {
        FileChangeStatus::Added => "A",
        FileChangeStatus::Modified => "M",
        FileChangeStatus::Deleted => "D",
        FileChangeStatus::Renamed => "R",
        FileChangeStatus::Unknown(raw) => raw,
    }
}

fn commit_totals(commit: &TourCommit) -> (u64, u64) {
    commit.files.iter().fold((0, 0), |(a, d), file| (a + file.additions, d + file.deletions))
}

/// Hover card body (`F046D5DF` rail popover): kind chip, sha and time, subject, body,
/// then author, file count and size under a hairline.
/// Group hover card (`F046D5DF`): title and count, then each stored member commit (short
/// sha, subject, age) and the group's +/- totals. Only stored fields; shas the tour doesn't
/// carry are counted but not listed.
fn group_card(group: &TourCommitGroup, members: &[TourCommit], cx: &App) -> Div {
    let t = theme::tokens(cx);
    let (additions, deletions) = members.iter().map(commit_totals).fold((0, 0), |(a, d), (ca, cd)| (a + ca, d + cd));
    let count = group.commit_shas.len();
    div().w(px(300.)).flex().flex_col().gap_2()
        .child(div().flex().items_center().gap_2()
            .child(status_pill(group.title.clone(), Tone::Running, cx))
            .child(div().flex_1())
            .child(div().t_caption().text_color(t.text_secondary).child(plural(count, "grouped commit"))))
        .children(members.iter().map(|commit| {
            let subject = commit.message.lines().next().unwrap_or_default().to_owned();
            div().flex().items_center().gap_2().t_caption()
                .child(div().flex_none().t_code_caption(cx).text_color(t.text_secondary).child(short_sha(&commit.sha).to_owned()))
                .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(subject))
                .child(div().flex_none().text_color(t.text_secondary).child(relative_time(&commit.created_at)))
        }))
        .child(div().flex().items_center().gap_2().pt_2().border_t_1().border_color(t.border).t_caption()
            .child(div().flex_1().text_color(t.text_secondary).child(plural(members.iter().map(|c| c.files.len()).sum(), "file")))
            .child(change_counts(additions as usize, deletions as usize, cx)))
}

fn commit_card(commit: &TourCommit, group: Option<&TourCommitGroup>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    let (additions, deletions) = commit_totals(commit);
    let (subject, body) = commit.message.split_once('\n').unwrap_or((&commit.message, ""));
    let kind = match group {
        Some(group) => status_pill(group.title.clone(), Tone::Running, cx),
        None => chip("Commit", Tone::Neutral, cx),
    };
    div()
        .w(px(300.))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(kind)
                .child(div().t_code_caption(cx).text_color(t.text_secondary).child(short_sha(&commit.sha).to_owned()))
                .child(div().flex_1())
                .child(div().t_caption().text_color(t.text_secondary).child(relative_time(&commit.created_at))),
        )
        .child(div().t_panel_title().text_color(t.text).child(subject.to_owned()))
        .when(!body.trim().is_empty(), |el| el.child(div().t_caption().text_color(t.text_secondary).child(body.trim().to_owned())))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .pt_2()
                .border_t_1()
                .border_color(t.border)
                .t_caption()
                .child(div().min_w_0().truncate().text_color(t.text_secondary).child(commit.author.clone()))
                .child(div().flex_1())
                .child(div().text_color(t.text_secondary).child(plural(commit.files.len(), "file")))
                .child(change_counts(additions as usize, deletions as usize, cx)),
        )
}

impl ReviewView {
    pub(super) fn render_timeline(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(tour) = self.tour().cloned() else { return div().into_any_element() };
        let commits = tour.commits.clone().unwrap_or_default();
        if commits.is_empty() {
            return div().size_full().child(crate::components::empty_state(
                "No commit history stored",
                "This tour was saved without commits, so there is no timeline to show.",
            )).into_any_element();
        }
        let t = theme::tokens(cx);
        let selected = self.selected_commit.clone().and_then(|sha| commits.iter().find(|c| c.sha == sha).cloned());
        let rail = self.render_rail(&tour, &commits, window, cx);
        let (additions, deletions) = commits.iter().map(commit_totals).fold((0, 0), |(a, d), (ca, cd)| (a + ca, d + cd));
        div()
            .id("review-timeline")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .max_w(px(1180.))
                    .mx_auto()
                    .px(px(metrics::GUTTER_X))
                    .pt(px(metrics::GUTTER_TOP))
                    .pb_8()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .pb_5()
                            .border_b_1()
                            .border_color(t.border)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .t_caption()
                                    .child(div().font_weight(FontWeight::MEDIUM).text_color(t.text).child("Commits"))
                                    .child(div().text_color(t.text_secondary).child(format!("{} stored ·", commits.len())))
                                    .child(change_counts(additions as usize, deletions as usize, cx)),
                            )
                            .child(rail),
                    )
                    .child(match selected {
                        Some(commit) => self.render_commit_detail(&commit, cx),
                        None => div()
                            .t_body()
                            .text_color(t.text_secondary)
                            .child("Hover a commit for details, or select one to see its files.")
                            .into_any_element(),
                    }),
            )
            .into_any_element()
    }

    fn render_rail(&self, tour: &CodeTour, commits: &[TourCommit], window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        let mut items: Vec<AnyElement> = Vec::new();
        let mut seen_groups: Vec<String> = Vec::new();
        for (ix, commit) in commits.iter().enumerate() {
            let group = group_of(tour, &commit.sha);
            if let Some(group) = group
                && !seen_groups.contains(&group.id)
            {
                seen_groups.push(group.id.clone());
                let members: Vec<TourCommit> = commits.iter().filter(|c| group.commit_shas.contains(&c.sha)).cloned().collect();
                items.push(self.render_group_pill(group, members, window, cx));
            }
            let visible = group.is_none_or(|group| self.expanded_groups.contains(&group.id));
            items.push(self.render_commit_node(ix, commit, group, visible, window, cx));
        }
        // Rail geometry (`F046D5DF`): 2pt running line through 16pt node slots.
        // "Earlier" is a faint hollow ring and Head a 12pt running ring.
        // The row is content-sized (flex item, no grow) so the line ends at Head (b34).
        // Bounded viewport (w_full, min_w_0) over the content-sized rail row, so narrow panes
        // scroll to Head; edge fades show which side has more commits.
        let row = div()
            .id("commit-rail")
            .w_full()
            .min_w_0()
            .flex()
            .overflow_x_scroll()
            .track_scroll(&self.rail_scroll)
            .pb_1()
            .child(
                div()
                    .relative()
                    .flex()
                    .items_start()
                    .flex_none()
                    .child(div().absolute().left(px(RAIL_END_W / 2.)).right(px(RAIL_END_W / 2.)).top(px(RAIL_Y - 1.)).h(px(2.)).bg(t.running))
                    .child(rail_end("Earlier", false, cx))
                    .children(items)
                    .child(rail_end("Head", true, cx)),
            );
        div().relative().w_full().min_w_0().child(row)
            .children(crate::views::responsive::x_scroll_fades(&self.rail_scroll, t.canvas))
    }

    fn render_group_pill(&self, group: &TourCommitGroup, members: Vec<TourCommit>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let expanded = self.expanded_groups.contains(&group.id);
        let width = transition(("pill-w", group.id.clone()), if expanded { PILL_EXPANDED_W } else { PILL_W }, motion::standard(), window, cx);
        let id = group.id.clone();
        let card_group = group.clone();
        let trigger = div()
            .id(SharedString::from(format!("group-{}", group.id)))
            .flex_none()
            .w(px(width))
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .child(
                div()
                    .mt(px(RAIL_Y - 11.))
                    .h(px(22.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(t.running)
                    .text_color(gpui::white())
                    .t_caption()
                    .font_weight(FontWeight::MEDIUM)
                    .font_features(tabular())
                    .child(group.commit_shas.len().to_string()),
            )
            .child(
                div()
                    .h(px(22.))
                    // Never wider than the (animating) slot, so it can't cover neighbour labels.
                    .max_w(px((width - 8.).max(0.)))
                    .px_1p5()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded(px(metrics::R_CONTROL))
                    .bg(Tone::Running.tint(cx, 0.10))
                    .t_caption()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.running)
                    .child(div().min_w_0().truncate().child(if expanded { "Collapse".to_owned() } else { group.title.clone() }))
                    .child(Icon::new(if expanded { Lucide::ChevronLeft } else { Lucide::ChevronRight }).size_3().flex_none()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.expanded_groups.remove(&id) {
                    this.expanded_groups.insert(id.clone());
                }
                cx.notify();
            }));
        // Same hover timing as commit nodes; the card lists the group's stored commits.
        div()
            .flex_none()
            .child(
                HoverCard::new(SharedString::from(format!("group-hover-{}", card_group.id)))
                    .open_delay(Duration::from_millis(100))
                    .close_delay(Duration::from_millis(80))
                    .trigger(trigger)
                    .content(move |_, _, cx| group_card(&card_group, &members, cx)),
            )
            .into_any_element()
    }

    fn render_commit_node(
        &self,
        ix: usize,
        commit: &TourCommit,
        group: Option<&TourCommitGroup>,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme::tokens(cx);
        let width = transition(("node-w", commit.sha.clone()), if visible { NODE_W } else { 0. }, motion::standard(), window, cx);
        let opacity = transition(("node-o", commit.sha.clone()), if visible { 1. } else { 0. }, motion::fade(), window, cx);
        if width < 0.5 {
            return div().into_any_element();
        }
        let selected = self.selected_commit.as_deref() == Some(commit.sha.as_str());
        let sha = commit.sha.clone();
        let dot = div()
            .mt(px(RAIL_Y - 8.))
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(Tone::Running.tint(cx, if selected { 0.25 } else { 0.15 }))
            .child(div().size(px(if selected { 10. } else { 8. })).rounded_full().bg(t.running));
        let trigger = div()
            .id(("commit-node", ix))
            .w(px(NODE_W))
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .child(dot)
            .child(
                div()
                    .t_code_caption(cx)
                    .text_color(if selected { t.text } else { t.text_secondary })
                    .when(selected, |el| el.font_weight(FontWeight::MEDIUM))
                    .child(short_sha(&commit.sha).to_owned()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_commit = if this.selected_commit.as_deref() == Some(sha.as_str()) { None } else { Some(sha.clone()) };
                cx.notify();
            }));
        let card_commit = commit.clone();
        let card_group = group.cloned();
        // The width tween clips horizontally only; the node's own content stays square.
        div()
            .flex_none()
            .w(px(width))
            .overflow_hidden()
            .opacity(opacity)
            .child(
                HoverCard::new(("commit-hover", ix))
                    .open_delay(Duration::from_millis(100))
                    .close_delay(Duration::from_millis(80))
                    .trigger(trigger)
                    .content(move |_, _, cx| commit_card(&card_commit, card_group.as_ref(), cx)),
            )
            .into_any_element()
    }

    fn render_commit_detail(&mut self, commit: &TourCommit, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let (subject, _) = commit.message.split_once('\n').unwrap_or((&commit.message, ""));
        let rows = commit.files.iter().map(|file| {
            let reviewed = self.is_file_reviewed(&file.path);
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(px(metrics::ROW_PROP))
                .px_4()
                .t_label()
                .child(div().w(px(14.)).flex_none().t_code_caption(cx).text_color(t.text_secondary).child(status_letter(&file.status).to_owned()))
                .child(div().flex_1().min_w_0().truncate().t_code_caption(cx).text_color(t.text).child(file.path.clone()))
                .when(reviewed, |row| row.child(div().t_caption().text_color(t.success).child("Reviewed")))
                .child(change_counts(file.additions as usize, file.deletions as usize, cx))
        }).collect::<Vec<_>>();
        let patch = commit_patch(commit);
        let diff = match self.diff_state(&format!("commit:{}", commit.sha), &patch, cx) {
            Ok(state) => div()
                .h(px(420.))
                .child(diff_card(Diff::new(&state), cx).soft_wrap(true).change_indicator(DiffChangeIndicator::Bars))
                .into_any_element(),
            Err(message) => raw_patch(subject, "", &patch, &message, cx),
        };
        let first_path = commit.files.first().map(|file| file.path.clone());
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                panel(cx)
                    .child(
                        edge_band(Edge::Top, metrics::R_CARD)
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(40.))
                            .pl_4()
                            // Trailing 20 button: right inset equals its vertical inset.
                            .pr(px((40. - metrics::CONTROL_XS) / 2.))
                            .border_b_1()
                            .border_color(t.border)
                            .child(div().flex_none().t_code_caption(cx).text_color(t.text_secondary).child(short_sha(&commit.sha).to_owned()))
                            .child(div().flex_1().min_w_0().truncate().t_panel_title().text_color(t.text).child(subject.to_owned()))
                            .child(div().flex_none().t_caption().text_color(t.text_secondary).child(format!("{} · {}", commit.author, relative_time(&commit.created_at))))
                            .child(
                                button_xs(Button::new("commit-in-diff").ghost().label("Show in Diff")).on_click(cx.listener(move |this, _, _, cx| {
                                    match &first_path {
                                        Some(path) => this.focus_file(path, cx),
                                        None => this.tab = super::ReviewTab::Diff,
                                    }
                                    cx.notify();
                                })),
                            ),
                    )
                    .child(div().flex().flex_col().py_2().children(rows)),
            )
            .child(diff)
            .into_any_element()
    }
}

fn rail_end(label: &'static str, head: bool, cx: &App) -> impl IntoElement {
    let t = theme::tokens(cx);
    let marker = if head {
        div().size(px(12.)).rounded_full().border_2().border_color(t.running).bg(t.canvas)
    } else {
        div().size(px(10.)).rounded_full().border_1().border_color(Tone::Running.tint(cx, 0.6)).bg(t.canvas)
    };
    div()
        .flex_none()
        .w(px(RAIL_END_W))
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .child(div().h(px(16.)).mt(px(RAIL_Y - 8.)).flex().items_center().child(marker))
        .child(div().t_caption().text_color(t.text_secondary).child(label))
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}
