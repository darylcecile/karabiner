//! Contribution-style heatmap of stored activity events (UTC days, last 26 weeks).

use std::collections::HashMap;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::Activity;

use super::common::{day_label, month_label, now_seconds, parse_timestamp, weekday};
use super::widget_header;
use crate::{components::Typography as _, theme::metrics};

const WEEKS: i64 = 26;
const CELL: f32 = metrics::HEATMAP_CELL;
const GAP: f32 = metrics::HEATMAP_GAP;
/// Month-label row above the cells.
const MONTH_ROW: f32 = 14.;

pub struct Heatmap {
    counts: HashMap<i64, usize>,
    truncated: bool,
}

impl Heatmap {
    pub fn new(activity: &[Activity], truncated: bool) -> Self {
        let mut counts = HashMap::new();
        for item in activity {
            if let Some(seconds) = parse_timestamp(&item.created_at) {
                *counts.entry(seconds.div_euclid(86_400)).or_insert(0) += 1;
            }
        }
        Self { counts, truncated }
    }

    fn level(count: usize, max: usize) -> usize {
        if count == 0 {
            return 0;
        }
        (count * 4).div_ceil(max.max(1)).clamp(1, 4)
    }

    /// `scroll` lets the caller keep the newest weeks in view when the card is narrow.
    pub fn render(&self, scroll: &ScrollHandle, cx: &App) -> impl IntoElement {
        let t = crate::theme::tokens(cx);
        let today = now_seconds().div_euclid(86_400);
        let first = today - weekday(today) as i64 - (WEEKS - 1) * 7;
        let max = self.counts.values().copied().max().unwrap_or(0);
        let total: usize = (first..=today).map(|day| self.counts.get(&day).copied().unwrap_or(0)).sum();
        let shades = shades(cx);
        let columns = (0..WEEKS).map(|week| {
            let start = first + week * 7;
            // Fixed-width columns: a month label overflows to the right instead of widening
            // its week, which made gaps uneven and clipped the last week (b10).
            div()
                .flex()
                .flex_col()
                .flex_none()
                .w(px(CELL))
                .gap(px(GAP))
                .child(
                    div()
                        .h(px(MONTH_ROW))
                        .w(px(CELL))
                        .whitespace_nowrap()
                        .t_micro()
                        .text_color(t.text_secondary)
                        .child(if civil_day(start) <= 7 { month_label(start) } else { "" }),
                )
                .children((0..7).map(|offset| {
                    let day = start + offset;
                    let count = self.counts.get(&day).copied().unwrap_or(0);
                    let future = day > today;
                    let date = day_label(day);
                    let shade = shades[Self::level(count, max)];
                    div()
                        .id(("heat", day as u64))
                        .size(px(CELL))
                        .rounded(px(metrics::R_HEATMAP))
                        .when(!future, |cell| {
                            cell.bg(shade).tooltip(move |window, cx| {
                                let date = date.clone();
                                Tooltip::element(move |_, cx| day_tooltip(&date, count, shade, cx)).build(window, cx)
                            })
                        })
                }))
        });
        let legend = div()
            .flex()
            .items_center()
            .gap_1()
            .t_caption()
            .text_color(t.text_secondary)
            .child(div().pr_1().child("Less"))
            .children(shades.into_iter().map(|shade| div().size(px(CELL)).rounded(px(metrics::R_HEATMAP)).bg(shade)))
            .child(div().pl_1().child("More"));
        let note = if self.truncated { "Latest 500 events" } else { "UTC days" };
        div()
            .flex()
            .flex_col()
            .child(widget_header("Activity", Some(format!("{total} events · 26 weeks").into()), cx))
            .child(
                div()
                    .id("heatmap-scroll")
                    // min_w_0: without it the row's content width wins and the card clips
                    // the newest weeks instead of scrolling.
                    .min_w_0()
                    .track_scroll(scroll)
                    .mx_2()
                    .rounded(px(metrics::R_INNER))
                    .bg(t.section_fill)
                    .overflow_x_scroll()
                    .p_3()
                    // Flex scroller + non-shrinking row: GPUI measures scroll extent from the
                    // immediate child, which must own the full 26-week width.
                    .flex()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap(px(GAP))
                            .child(
                                div().flex().flex_col().gap(px(GAP)).pr_1p5().pt(px(MONTH_ROW + GAP)).children(
                                    ["Mon", "", "Wed", "", "Fri", "", ""].map(|day| {
                                        div().h(px(CELL)).flex().items_center().t_micro().text_color(t.text_secondary).child(day)
                                    }),
                                ),
                            )
                            .children(columns),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .pt_2()
                    .pb_1()
                    .child(div().t_caption().text_color(t.text_secondary).child(note))
                    .child(legend),
            )
    }
}

/// DB2A4628 tooltip: date caption over a swatch row with the value right-aligned.
fn day_tooltip(date: &str, count: usize, shade: Hsla, cx: &App) -> impl IntoElement + use<> {
    let t = crate::theme::tokens(cx);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w(px(120.))
        .child(div().t_caption().text_color(t.text_secondary).child(date.to_owned()))
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .t_label()
                .child(div().size(px(8.)).rounded(px(2.)).bg(shade))
                .child(div().flex_1().child("Events"))
                .child(div().font_weight(FontWeight::MEDIUM).child(count.to_string())),
        )
}

fn civil_day(days: i64) -> i64 {
    super::common::civil_from_days(days).2
}

fn shades(cx: &App) -> [Hsla; 5] {
    let base = crate::theme::tokens(cx).running;
    [
        crate::theme::tokens(cx).tray,
        base.opacity(0.3),
        base.opacity(0.5),
        base.opacity(0.75),
        base,
    ]
}
