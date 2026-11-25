//! Overview: risk/impact meters, review progress and decision, narrative, chapter index,
//! and the tour's dependency diagram.

use std::rc::Rc;

use gpui_kit::component::{input::Textarea, text::TextView};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{CodeTour, Level, ReviewDecision, TourBlockContent};

use super::{ReviewView, diagram, files::changed_paths, panel, panel_title};
use crate::{
    components::{Segment, Tone, Typography as _, chip, meter, segmented, status_pill},
    theme::{self, metrics},
};

fn level_meter(label: &'static str, level: Option<&Level>, cx: &App) -> impl IntoElement {
    let t = theme::tokens(cx);
    // 851174F7 draws five segments: Low 1, Medium 3, High 5 (same three stored levels).
    let (text, filled, tone) = match level {
        Some(Level::Low) => ("Low".to_owned(), 1, Tone::Success),
        Some(Level::Medium) => ("Medium".to_owned(), 3, Tone::Attention),
        Some(Level::High) => ("High".to_owned(), 5, Tone::Danger),
        Some(Level::Unknown(raw)) => (raw.clone(), 0, Tone::Neutral),
        None => ("Not assessed".to_owned(), 0, Tone::Neutral),
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .t_label()
        .child(div().text_color(t.text_secondary).child(label))
        .child(div().font_weight(FontWeight::MEDIUM).text_color(if filled == 0 { t.text_secondary } else { tone.color(cx) }).child(text))
        .child(meter(5, filled, tone, cx))
}

/// The first Mermaid/text diagram in the tour, with the chapter it came from.
fn first_diagram(tour: &CodeTour) -> Option<(String, String, String)> {
    tour.chapters.iter().flat_map(|chapter| &chapter.blocks).find_map(|block| match block.content() {
        Some(TourBlockContent::Diagram { title, format, source }) => Some((title, format, source)),
        _ => None,
    })
}

/// Maps a diagram node to a chapter: a leading number ("04 · …") or an exact title match.
fn chapter_for_label(tour: &CodeTour, label: &str) -> Option<usize> {
    let digits: String = label.trim().chars().take_while(char::is_ascii_digit).collect();
    if let Ok(number) = digits.parse::<usize>()
        && (1..=tour.chapters.len()).contains(&number)
    {
        return Some(number - 1);
    }
    tour.chapters.iter().position(|chapter| chapter.title.eq_ignore_ascii_case(label.trim()))
}

impl ReviewView {
    pub(super) fn render_overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(tour) = self.tour().cloned() else { return div().into_any_element() };
        // `851174F7`: risk row, feedback, then a 5:7 row of narrative/index and diagram.
        div()
            .id("review-overview")
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
                    .child(self.render_summary_bar(&tour, cx))
                    .child(self.render_feedback(cx))
                    // Side by side needs the narrative plus a 360 diagram; below 900 of
                    // allocated width the diagram goes under the narrative at full width.
                    .child(
                        div()
                            .flex()
                            .gap_8()
                            .items_start()
                            .when(crate::views::responsive::narrower_than(&self.body_width, 900.), |this| this.flex_col().items_stretch())
                            .child(self.render_narrative(&tour, cx))
                            .child(self.render_diagram_panel(&tour, cx)),
                    ),
            )
            .into_any_element()
    }

    fn render_summary_bar(&self, tour: &CodeTour, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let files = changed_paths(tour);
        let reviewed_files = files.iter().filter(|path| self.is_file_reviewed(path)).count();
        let reviewed_chapters = tour.chapters.iter().filter(|chapter| self.is_chapter_reviewed(&chapter.id)).count();
        let decisions = [ReviewDecision::Pending, ReviewDecision::Approved, ReviewDecision::ChangesRequested];
        let selected = decisions.iter().position(|decision| *decision == self.review.decision).unwrap_or(0);
        let view = cx.entity().downgrade();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_6()
            .gap_y_2()
            .child(level_meter("Risk", tour.risk.as_ref(), cx))
            .child(level_meter("Impact", tour.impact.as_ref(), cx))
            .child(
                div()
                    .t_label()
                    .text_color(t.text_secondary)
                    .child(format!("Reviewed {reviewed_chapters}/{} chapters · {reviewed_files}/{} files", tour.chapters.len(), files.len())),
            )
            .child(chip("Checks not connected", Tone::Neutral, cx))
            .child(div().flex_1())
            .child(
                div().id("review-decision").tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new("Records your review decision on this tour").build(window, cx)
                }).child(segmented(
                    "review-decision-items",
                    vec![Segment::new("Pending"), Segment::new("Approve"), Segment::new("Request changes")],
                    selected,
                    move |index, _, cx| {
                        let decision = decisions[index].clone();
                        view.update(cx, |this, cx| this.set_decision(decision, cx)).ok();
                    },
                    cx,
                )),
            )
    }

    fn render_feedback(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let status = if self.request.is_some() {
            "Saving…"
        } else if self.dirty {
            "Unsaved"
        } else {
            "Saved with the tour"
        };
        // The textarea's focus ring needs room inside the card: 16 padding on all sides.
        panel(cx)
            .child(panel_title("Review feedback", Some(status.into()), cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .child(Textarea::new(&self.summary).t_body().aria_label("Review feedback"))
                    .when(self.review.summary.is_empty() && !self.dirty, |el| {
                        el.child(div().t_caption().text_color(t.text_secondary).child("No feedback written yet."))
                    }),
            )
    }

    fn render_narrative(&self, tour: &CodeTour, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let overview = tour.overview.clone().filter(|text| !text.trim().is_empty());
        let files = changed_paths(tour).len();
        let commits = tour.commits.as_ref().map_or(0, Vec::len);
        let chapters: Vec<AnyElement> = tour
            .chapters
            .iter()
            .enumerate()
            .map(|(ix, chapter)| {
                let reviewed = self.is_chapter_reviewed(&chapter.id);
                let group = SharedString::from(format!("overview-chapter-{ix}"));
                // Badges sit on the prose column's left edge (b32, 851174F7); hover tints the
                // badge rather than an inset row fill that would push it off that edge.
                div()
                    .id(("overview-chapter", ix))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(px(32.))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_none()
                            .w(px(28.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(metrics::R_NUMBER))
                            .border_1()
                            .border_color(t.border)
                            .bg(t.surface)
                            .group_hover(group.clone(), move |style| style.bg(t.hover).text_color(t.text))
                            .t_code_caption(cx)
                            .text_color(t.text_secondary)
                            .child(format!("{:02}", ix + 1)),
                    )
                    .child(div().flex_1().min_w_0().truncate().t_body().text_color(t.text).child(chapter.title.clone()))
                    .children(reviewed.then(|| status_pill("Reviewed", Tone::Success, cx)))
                    .on_click(cx.listener(move |this, _, _, cx| this.show_chapter(ix, cx)))
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .map(|this| if crate::views::responsive::narrower_than(&self.body_width, 900.) {
                this.w_full()
            } else {
                this.w(relative(5. / 12.)).flex_none().min_w(px(320.))
            })
            .child(div().t_read_heading().text_color(t.text).child("Overview"))
            .child(match overview {
                Some(text) => div().t_read().text_color(t.text).child(TextView::markdown("tour-overview", text).selectable(true)).into_any_element(),
                None => div().t_read().text_color(t.text_secondary).child("No overview was written for this tour.").into_any_element(),
            })
            .child(div().flex().flex_col().children(chapters))
            .child(
                div()
                    .t_caption()
                    .text_color(t.text_secondary)
                    .child(format!("This tour covers {files} changed files across {commits} stored commits. CI and execution results aren't connected, so no checks are shown.")),
            )
    }

    fn render_diagram_panel(&self, tour: &CodeTour, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let body: AnyElement = match first_diagram(tour) {
            None => div().t_body().text_color(t.text_secondary).child("This tour has no dependency diagram.").into_any_element(),
            Some((title, format, source)) => {
                let view = cx.entity().downgrade();
                let tour_for_click = tour.clone();
                let on_click: diagram::NodeClick = Rc::new(move |node, _, cx| {
                    if let Some(ix) = chapter_for_label(&tour_for_click, &node.label) {
                        view.update(cx, |this, cx| this.show_chapter(ix, cx)).ok();
                    }
                });
                diagram_body("overview-diagram", &title, &format, &source, Some(on_click), Some(&self.diagram_scroll), cx)
            }
        };
        // The scroll viewport is inset 16 from the panel's R12 corners (≥ clip_inset).
        div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_w_0()
            .w_full()
            .child(div().t_read_heading().text_color(t.text).child("How the changes fit together"))
            .child(
                // Wide graphs scroll inside the panel; fades mark the hidden side.
                panel(cx).bg(t.tray).p_4().min_w_0().child(body),
            )
    }
}

/// Diagram block body shared with the Guide: laid-out graph, or the source as text.
pub(super) fn diagram_body(
    id: &str,
    title: &str,
    format: &str,
    source: &str,
    on_click: Option<diagram::NodeClick>,
    scroll: Option<&ScrollHandle>,
    cx: &App,
) -> AnyElement {
    let t = theme::tokens(cx);
    let graph = (format == "mermaid").then(|| diagram::parse_mermaid(source)).flatten();
    let note = match (&graph, format) {
        (Some(_), _) => None,
        (None, "text") => None,
        (None, "mermaid") => Some("Shown as source: only Mermaid flowcharts are laid out."),
        (None, _) => Some("Shown as source: unknown diagram format."),
    };
    div()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .gap_3()
        .children((!title.is_empty()).then(|| div().t_caption().text_color(t.text_secondary).child(title.to_owned())))
        .child(match graph {
            // Only the graph scrolls: the caption above and note below stay at the card edge.
            // GPUI takes scroll extent from the immediate child, so that child is the
            // content-sized graph (at least the viewport wide). Fades need a tracked handle.
            Some(graph) => div().relative().w_full().min_w_0()
                .child(div().id(SharedString::from(format!("{id}-scroll"))).w_full().flex().overflow_x_scroll()
                    .when_some(scroll, |this, handle| this.track_scroll(handle))
                    .child(div().flex_none().min_w(relative(1.)).py_2().child(diagram::render(SharedString::from(id.to_owned()), &graph, on_click, cx))))
                .children(scroll.map(|handle| crate::views::responsive::x_scroll_fades(handle, t.tray)).unwrap_or_default())
                .into_any_element(),
            None => div()
                .p_3()
                .rounded(px(metrics::R_INNER))
                .bg(t.surface)
                .t_code(cx)
                .text_color(t.text)
                .children(source.lines().map(|line| div().child(line.to_owned())))
                .into_any_element(),
        })
        .children(note.map(|note| div().t_caption().text_color(t.text_secondary).child(note)))
        .into_any_element()
}
