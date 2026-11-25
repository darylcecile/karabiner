//! Guide: chapter navigation, narrative blocks, file-range rows that focus their diff,
//! and per-chapter / per-file review marks.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{
    Disableable as _, Icon,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    diff::{Diff, DiffChangeIndicator, DiffHoverHighlight},
    text::TextView,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{FileChangeStatus, TourBlock, TourBlockContent, TourChapter};

use super::{
    ReviewView,
    files::{block_key, count_changes, normalize_patch, short_sha},
    overview::diagram_body,
    panel,
};
use crate::{
    components::{Edge, Tone, Typography as _, button_sm, button_xs, change_counts, chip, edge_band, file_path_label},
    theme::{self, metrics},
};

/// A file mentioned by a chapter, with the block that shows it (if any).
struct ChapterFile {
    path: String,
    lines: Option<(u32, Option<u32>)>,
    block_id: Option<String>,
    changes: Option<(usize, usize)>,
}

fn chapter_files(chapter: &TourChapter) -> Vec<ChapterFile> {
    let mut files: Vec<ChapterFile> = Vec::new();
    for block in &chapter.blocks {
        let file = match block.content() {
            Some(TourBlockContent::Diff { path, patch, .. }) => {
                ChapterFile { path, lines: None, block_id: Some(block.id.clone()), changes: Some(count_changes(&patch)) }
            }
            Some(TourBlockContent::Code { source: Some(source), .. }) => ChapterFile {
                lines: source.start_line.map(|start| (start, source.end_line)),
                path: source.path,
                block_id: Some(block.id.clone()),
                changes: None,
            },
            _ => continue,
        };
        files.push(file);
    }
    for path in chapter.file_paths.iter().flatten() {
        if files.iter().all(|file| &file.path != path) {
            files.push(ChapterFile { path: path.clone(), lines: None, block_id: None, changes: None });
        }
    }
    files
}

/// Kit's diff row: code font 12.5 × 1.6 line height (`diff/mod.rs:582`, theme
/// `mono_font_size`).
const DIFF_ROW_H: f32 = 20.;

/// Kit's hunk separator row (`diff/mod.rs:1133`, `h_6` at the 16pt rem).
const DIFF_HUNK_H: f32 = 24.;
/// Tallest a diff grows before it scrolls inside its card.
const DIFF_MAX_H: f32 = 28. * DIFF_ROW_H;

/// Height the diff viewer needs, capped so long patches scroll inside their card. Counts
/// rendered rows, not raw patch lines. Each hunk renders a separator. Inside a hunk, in
/// split mode a run of removed (`-`) then added (`+`) lines shares rows side by side, so
/// the run takes the longer side (unified: one row each); context lines take one row
/// each, and `\ No newline` markers aren't rows. `+++x`/`---x` inside a hunk are
/// content lines; file headers before the first hunk aren't rows.
pub(super) fn diff_height(patch: &str, header: bool, split: bool) -> Pixels {
    let (mut code, mut hunks, mut in_hunk) = (0usize, 0usize, false);
    let (mut dels, mut adds) = (0usize, 0usize);
    let flush = |code: &mut usize, dels: &mut usize, adds: &mut usize| {
        *code += if split { (*dels).max(*adds) } else { *dels + *adds };
        (*dels, *adds) = (0, 0);
    };
    for line in patch.lines() {
        if line.starts_with("@@") {
            flush(&mut code, &mut dels, &mut adds);
            hunks += 1;
            in_hunk = true;
        } else if line.starts_with("diff ") {
            flush(&mut code, &mut dels, &mut adds);
            in_hunk = false;
        } else if !in_hunk {
            continue;
        } else if line.starts_with('-') {
            if adds > 0 {
                flush(&mut code, &mut dels, &mut adds);
            }
            dels += 1;
        } else if line.starts_with('+') {
            adds += 1;
        } else if line.starts_with(' ') {
            flush(&mut code, &mut dels, &mut adds);
            code += 1;
        }
    }
    flush(&mut code, &mut dels, &mut adds);
    let body = (code.max(1) as f32 * DIFF_ROW_H + hunks as f32 * DIFF_HUNK_H).min(DIFF_MAX_H);
    px(body + if header { 44. } else { 0. })
}

impl ReviewView {
    pub(super) fn render_guide(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(tour) = self.tour().cloned() else { return div() };
        if tour.chapters.is_empty() {
            return div().size_full().child(crate::components::empty_state("No chapters", "This tour has no chapters yet."));
        }
        let index = self.chapter.min(tour.chapters.len() - 1);
        let chapter = tour.chapters[index].clone();
        let t = theme::tokens(cx);
        let mut prose: Vec<AnyElement> = Vec::new();
        let mut visuals: Vec<AnyElement> = Vec::new();
        for block in &chapter.blocks {
            match block.content() {
                Some(TourBlockContent::Markdown { text }) => prose.push(
                    div()
                        .t_read()
                        .text_color(t.text)
                        .child(TextView::markdown(SharedString::from(format!("block-{}", block.id)), text).selectable(true))
                        .into_any_element(),
                ),
                _ => visuals.push(self.render_block(&chapter.id, block, cx)),
            }
        }
        // 5:7 chapter row (`F046D5DF`, `AEF224FF`): narrative and file rows on the left, the
        // chapter's diffs/code/diagrams on the right. Right-column children are direct
        // children of the tracked scroll so `focus_block` can scroll to them by index.
        // Below 900 of allocated review width the 5:7 split crushes both sides: the story
        // stacks above the blocks in one tracked scroll (`focus_block` offsets by one).
        let stacked = self.guide_stacked();
        let left = div()
            .id("guide-prose")
            .when(!stacked, |this| this.w(relative(5. / 12.)).flex_none().h_full().overflow_y_scroll())
            .when(stacked, |this| this.w_full())
            .flex()
            .flex_col()
            .gap_5()
            .pl(px(metrics::GUTTER_X))
            .pr(px(if stacked { metrics::GUTTER_X } else { 16. }))
            .pt(px(metrics::GUTTER_TOP))
            .pb_6()
            .child(self.render_chapter_header(index, tour.chapters.len(), &chapter, cx))
            .child(div().flex().flex_col().gap_3().children(prose))
            .child(self.render_chapter_files(&chapter, cx))
            .child(self.render_chapter_footer(index, tour.chapters.len(), cx));
        if stacked {
            return div().size_full().child(div()
                .id("guide-visuals")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.guide_scroll)
                .flex()
                .flex_col()
                .gap_4()
                .child(left)
                .children(visuals.into_iter().map(|block| div().px(px(metrics::GUTTER_X)).min_w_0().child(block)))
                .child(div().h(px(24.))));
        }
        let right = div()
            .id("guide-visuals")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .track_scroll(&self.guide_scroll)
            .flex()
            .flex_col()
            .gap_4()
            .pl_4()
            .pr(px(metrics::GUTTER_X))
            .pt(px(metrics::GUTTER_TOP))
            .pb_6()
            .children(visuals);
        div()
            .size_full()
            .flex()
            .child(left)
            .child(right)
    }

    fn guide_stacked(&self) -> bool {
        crate::views::responsive::narrower_than(&self.body_width, 900.)
    }

    fn render_chapter_header(&self, index: usize, total: usize, chapter: &TourChapter, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let id = chapter.id.clone();
        let reviewed = self.is_chapter_reviewed(&chapter.id);
        let commits: Vec<AnyElement> = chapter
            .commit_shas
            .iter()
            .flatten()
            .enumerate()
            .map(|(ix, sha)| {
                let sha = sha.clone();
                button_xs(Button::new(("chapter-commit", ix)).ghost().icon(Lucide::GitCommitHorizontal).label(short_sha(&sha).to_owned()))
                    .tooltip("Show in timeline")
                    .on_click(cx.listener(move |this, _, _, cx| this.show_commit(&sha, cx)))
                    .into_any_element()
            })
            .collect();
        // AEF224FF: the "05 / 06" counter sits above the chapter title.
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_x_3()
                    .gap_y_1()
                    .child(div().t_code_caption(cx).text_color(t.text_secondary).child(format!("{:02} / {:02}", index + 1, total)))
                    .child(
                        div().flex_none().t_label().text_color(t.text_secondary).child(
                            // Distinct from the per-file "Reviewed" in each diff header
                            // (set_chapter_reviewed vs set_file_reviewed), so it says what it marks.
                            Checkbox::new(("chapter-reviewed", index))
                                .label("Chapter reviewed")
                                .checked(reviewed)
                                .accessibility_label("Mark chapter reviewed")
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| this.set_chapter_reviewed(&id, *checked, cx))),
                        ),
                    ),
            )
            .child(div().t_read_heading().text_color(t.text).child(chapter.title.clone()))
            .when(!chapter.summary.trim().is_empty(), |el| {
                el.child(
                    div().pt_2().t_read().text_color(t.text).child(
                        TextView::markdown(SharedString::from(format!("chapter-summary-{}", chapter.id)), chapter.summary.clone()).selectable(true),
                    ),
                )
            })
            .when(!commits.is_empty(), |el| el.child(div().flex().flex_wrap().gap_1().children(commits)))
    }

    /// File rows under the narrative (`AEF224FF`): 36 high, R8, tray fill when its block is
    /// the selected one in the right column.
    fn render_chapter_files(&self, chapter: &TourChapter, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let rows: Vec<AnyElement> = chapter_files(chapter)
            .into_iter()
            .enumerate()
            .map(|(ix, file)| {
                let range = file.lines.map(|(start, end)| match end {
                    Some(end) if end != start => format!(":{start}-{end}"),
                    _ => format!(":{start}"),
                });
                let focused = file.block_id.is_some() && file.block_id == self.focused_block;
                let target_block = file.block_id.clone();
                let path = file.path.clone();
                div()
                    .id(("chapter-file", ix))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(metrics::ROW_LIST))
                    .px_2p5()
                    .rounded(px(metrics::R_INNER))
                    .cursor_pointer()
                    .when(focused, |row| row.bg(t.tray))
                    .when(!focused, |row| row.hover(move |style| style.bg(t.hover)))
                    .child(Icon::new(Lucide::FileCode).size_3p5().flex_none().text_color(t.text_secondary))
                    .child(div().flex_1().min_w_0().overflow_hidden().flex().child(file_path_label(&file.path, range.map(SharedString::from), focused, cx)))
                    .children(file.changes.map(|(add, del)| change_counts(add, del, cx)))
                    .when(self.is_file_reviewed(&file.path), |row| row.child(Icon::new(Lucide::CircleCheck).size_3p5().flex_none().text_color(t.success)))
                    .on_click(cx.listener(move |this, _, _, cx| match &target_block {
                        Some(block) => this.focus_block(block, cx),
                        None => this.focus_file(&path, cx),
                    }))
                    .into_any_element()
            })
            .collect();
        div().flex().flex_col().gap_0p5().children(rows)
    }

    fn render_chapter_footer(&self, index: usize, total: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .justify_between()
            .pt_4()
            .border_t_1()
            .border_color(t.border)
            .child(
                button_sm(Button::new("prev-chapter").ghost().icon(Lucide::ChevronLeft).label("Previous chapter"))
                    .disabled(index == 0)
                    .on_click(cx.listener(move |this, _, _, cx| this.show_chapter(index.saturating_sub(1), cx))),
            )
            .child(
                button_sm(Button::new("next-chapter").ghost().label("Next chapter").icon(Lucide::ChevronRight))
                    .disabled(index + 1 >= total)
                    .on_click(cx.listener(move |this, _, _, cx| this.show_chapter(index + 1, cx))),
            )
    }

    /// Scrolls the right column to a block; its children are the chapter's non-Markdown
    /// blocks in order.
    fn focus_block(&mut self, block_id: &str, cx: &mut Context<Self>) {
        let Some(tour) = self.tour() else { return };
        let Some(chapter) = tour.chapters.get(self.chapter) else { return };
        let mut visual = chapter.blocks.iter().filter(|block| !matches!(block.content(), Some(TourBlockContent::Markdown { .. })));
        if let Some(position) = visual.position(|block| block.id == block_id) {
            self.focused_block = Some(block_id.to_owned());
            // Stacked layout: the story is item 0 of the same scroll.
            self.guide_scroll.scroll_to_top_of_item(position + usize::from(self.guide_stacked()));
            cx.notify();
        }
    }

    fn render_block(&mut self, chapter_id: &str, block: &TourBlock, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let focused = self.focused_block.as_deref() == Some(block.id.as_str());
        let body = match block.content() {
            Some(TourBlockContent::Markdown { text }) => div()
                .t_read()
                .child(TextView::markdown(SharedString::from(format!("block-{}", block.id)), text).selectable(true))
                .into_any_element(),
            Some(TourBlockContent::Code { title, code, source, .. }) => self.render_code(&title, &code, source.as_ref(), cx),
            Some(TourBlockContent::Diagram { title, format, source }) => panel(cx)
                .bg(t.tray)
                .p_4()
                .min_w_0()
                .child(diagram_body(&format!("diagram-{}", block.id), &title, &format, &source, None, None, cx))
                .into_any_element(),
            Some(TourBlockContent::Diff { title, path, patch, .. }) => self.render_diff_block(&block_key(chapter_id, &block.id), &title, &path, &patch, cx),
            None => unknown_block(block, cx),
        };
        // Focus ring sits 1pt outside the card, so its radius is the card's plus one.
        div()
            .rounded(px(metrics::R_CARD + 1.))
            .border_1()
            .border_color(if focused { cx_ring(cx) } else { gpui::transparent_black() })
            .child(body)
            .into_any_element()
    }

    fn render_code(&self, title: &str, code: &str, source: Option<&protocol::SourceReference>, cx: &App) -> AnyElement {
        let t = theme::tokens(cx);
        let start = source.and_then(|s| s.start_line).unwrap_or(1);
        let location = source.map(|s| {
            let lines = s.start_line.map(|a| format!(":{a}{}", s.end_line.map(|b| format!("-{b}")).unwrap_or_default())).unwrap_or_default();
            format!("{}{lines}", s.path)
        });
        panel(cx)
            .child(
                edge_band(Edge::Top, metrics::R_CARD)
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(40.))
                    .px_4()
                    .border_b_1()
                    .border_color(t.border)
                    .t_label()
                    .child(div().flex_none().font_weight(FontWeight::MEDIUM).text_color(t.text).child(title.to_owned()))
                    .children(location.map(|loc| div().min_w_0().truncate().t_code_caption(cx).text_color(t.text_secondary).child(loc))),
            )
            .child(
                div()
                    .id(SharedString::from(format!("code-{title}-{start}")))
                    .overflow_x_scroll()
                    .pt_2()
                    .pb(px(metrics::R_CARD))
                    .t_code(cx)
                    .text_color(t.text)
                    .children(code.lines().enumerate().map(|(ix, line)| {
                        div()
                            .flex()
                            .child(div().w(px(48.)).pr_4().flex_none().text_right().text_color(t.text_tertiary).child((start as usize + ix).to_string()))
                            .child(div().whitespace_nowrap().child(line.to_owned()))
                    })),
            )
            .into_any_element()
    }

    fn render_diff_block(&mut self, key: &str, title: &str, path: &str, patch: &str, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let patch = &normalize_patch(path, &FileChangeStatus::Modified, patch);
        let height = diff_height(patch, true, false);
        let state = match self.diff_state(key, patch, cx) {
            Ok(state) => state,
            Err(message) => return raw_patch(title, path, patch, &message, cx),
        };
        let reviewed = self.is_file_reviewed(path);
        let view = cx.entity().downgrade();
        let path_owned = path.to_owned();
        let header_check = move |_: &gpui_kit::component::diff::DiffFile, _: &mut Window, _: &mut App| {
            let view = view.clone();
            let path = path_owned.clone();
            Checkbox::new(SharedString::from(format!("reviewed-{path}")))
                .label("Reviewed")
                .checked(reviewed)
                .on_click(move |checked, _, cx| {
                    let path = path.clone();
                    view.update(cx, |this, cx| this.set_file_reviewed(&path, *checked, cx)).ok();
                })
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children((!title.is_empty()).then(|| div().px_1().t_caption().text_color(t.text_secondary).child(title.to_owned())))
            .child(
                div().w_full().h(height + px(metrics::R_CARD)).child(
                    diff_card(Diff::new(&state), cx)
                        .size_full()
                        .soft_wrap(true)
                        .hover_highlight(DiffHoverHighlight::Line)
                        .change_indicator(DiffChangeIndicator::Bars)
                        .render_header_suffix(header_check),
                ),
            )
            .into_any_element()
    }
}

/// Kit's diff viewer as the review diff card (`AEF224FF`): its own border becomes the card
/// border at R12 on a surface fill. Its rows sit in a rectangular scroll clip, so the body
/// ends `R_CARD` above the bottom edge and never paints into the lower arcs; the header
/// band at the top has no fill.
pub(super) fn diff_card(diff: Diff, cx: &App) -> Diff {
    let t = theme::tokens(cx);
    diff.rounded(px(metrics::R_CARD)).border_color(t.border).bg(t.surface).pb(px(metrics::R_CARD)).t_label()
}

fn cx_ring(cx: &App) -> Hsla {
    theme::tokens(cx).running.opacity(0.6)
}

/// Fallback for patches the diff viewer can't parse: show the stored text.
pub(super) fn raw_patch(title: &str, path: &str, patch: &str, message: &str, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    panel(cx)
        .child(
            edge_band(Edge::Top, metrics::R_CARD)
                .flex()
                .items_center()
                .gap_2()
                .h(px(40.))
                .pl_4()
                // Trailing 20 chip: right inset equals its vertical inset.
                .pr(px((40. - metrics::CHIP_H) / 2.))
                .border_b_1()
                .border_color(t.border)
                .child(div().min_w_0().truncate().t_code_caption(cx).text_color(t.text).child(path.to_owned()))
                .child(div().min_w_0().truncate().t_caption().text_color(t.text_secondary).child(title.to_owned()))
                .child(div().flex_1())
                .child(chip(message.to_owned(), Tone::Attention, cx)),
        )
        .child(
            div()
                .id(SharedString::from(format!("raw-patch-{path}")))
                .max_h(px(480.))
                .overflow_y_scroll()
                .px_4()
                .pt_3()
                .pb(px(metrics::R_CARD))
                .t_code(cx)
                .children(patch.lines().map(|line| {
                    let color = if line.starts_with('+') { t.success } else if line.starts_with('-') { t.danger } else { t.text };
                    div().whitespace_nowrap().text_color(color).child(line.to_owned())
                })),
        )
        .into_any_element()
}

/// Blocks written by a newer client: kept unchanged in saves, summarised here.
fn unknown_block(block: &TourBlock, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    let fields = block.fields.keys().cloned().collect::<Vec<_>>().join(", ");
    panel(cx)
        .p_4()
        .gap_1()
        .bg(t.tray)
        .child(div().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child(format!("Unsupported block “{}”", block.kind)))
        .child(
            div().t_body().text_color(t.text_secondary).child(format!(
                "This version can't display it. It's kept unchanged when the review is saved.{}",
                if fields.is_empty() { String::new() } else { format!(" Fields: {fields}.") }
            )),
        )
        .into_any_element()
}
