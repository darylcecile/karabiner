//! Task due date picker, after Spatial `661192A5`: a typed-date header, a quick-choice rail,
//! two month panes (one when the window is narrow) and a tray footer with "Apply date".
//!
//! The base `CalendarState` owns the visible month, the month/year view and the pending
//! (draft) date. Picking a day, a quick choice or typing only changes that pending date;
//! "Apply date" (or Enter in the field) emits `CalendarEvent::Selected`, which the task
//! detail records as a draft edit for Save. Closing without applying discards it.

use gpui_kit::base::{CalendarEvent, CalendarState, CalendarView, Date};
use gpui_kit::component::{
    Icon, IconName,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    popover::Popover,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jiff::civil::{self, Weekday};

use crate::{components::{self, Typography as _}, theme::{self, metrics}};

// Geometry: the reference at @2x scaled by 0.75 so day numerals land on the 13pt label role.
const CELL_W: f32 = 33.;
const ROW_H: f32 = 26.;
const RING: f32 = 22.;
const PANE_PX: f32 = 14.;
const PANE_GAP: f32 = 20.;
/// Fits the longest preset ("This weekend", ~78 at 13pt) with the same 12 inset on both sides.
const RAIL_W: f32 = 112.;
const RAIL_ROW_H: f32 = 25.;
const HEADER_H: f32 = 37.;
const FOOTER_H: f32 = 42.;
const PILL_H: f32 = 24.;
const NAV_BTN: f32 = 24.;
const SHELL_INSET: f32 = 3.;
/// Two-month popup width: shell insets + card borders + rail + divider + two panes + gap
/// (6 + 2 + 112 + 1 + 28 + 462 + 20 = 631).
const TWO_MONTH_POPUP_W: f32 = SHELL_INSET * 2. + 2. + RAIL_W + 1. + PANE_PX * 2. + CELL_W * 14. + PANE_GAP;
/// Show the pair only with ≥ ~84 of air on each side (631 + 168 ≈ 800); the 720 minimum
/// window gets one month (popup ≈ 370).
const TWO_MONTH_MIN_W: f32 = TWO_MONTH_POPUP_W + 2. * 84.5;

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

#[derive(Clone, Copy)]
enum Preset {
    Today,
    Tomorrow,
    ThisWeekend,
    NextWeek,
    TwoWeeks,
    NextMonth,
    NoDate,
}

const PRESETS: [(Preset, &str); 7] = [
    (Preset::Today, "Today"),
    (Preset::Tomorrow, "Tomorrow"),
    (Preset::ThisWeekend, "This weekend"),
    (Preset::NextWeek, "Next week"),
    (Preset::TwoWeeks, "In 2 weeks"),
    (Preset::NextMonth, "Next month"),
    (Preset::NoDate, "No date"),
];

/// Pane whose month pill opened the month/year view (0 = left).
struct MonthPane(usize);

/// Trigger and popover. `committed` is the task's current draft due date: the picker is
/// reset to it whenever it opens or closes, so an unapplied choice is discarded.
pub fn popover(
    id: &'static str,
    trigger: Button,
    state: &Entity<CalendarState>,
    input: &Entity<InputState>,
    committed: Option<civil::Date>,
    open: bool,
    on_open_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let t = theme::tokens(cx);
    let focus = input.read(cx).focus_handle(cx);
    let (reset_state, reset_input) = (state.clone(), input.clone());
    let (state, input) = (state.clone(), input.clone());
    Popover::new(id)
        .anchor(Anchor::TopLeft)
        .trigger(trigger)
        .open(open)
        .on_open_change(move |open, window, cx| {
            reset(&reset_state, &reset_input, committed, window, cx);
            on_open_change(open, window, cx);
        })
        .track_focus(&focus)
        .p_0()
        .bg(t.tray)
        .rounded(px(metrics::R_DIALOG))
        .content(move |_, window, cx| {
            // Escape closes only this popover. Capture runs before the field and the
            // drawer's Escape handler (an ancestor key listener).
            let popover = cx.entity().downgrade();
            let months = if window.viewport_size().width >= px(TWO_MONTH_MIN_W) { 2 } else { 1 };
            let pane = window.use_keyed_state("due-month-pane", cx, |_, _| MonthPane(0));
            div()
                .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                        cx.stop_propagation();
                        popover.update(cx, |popover, cx| popover.dismiss(window, cx)).ok();
                    }
                })
                .child(picker(&state, &input, &pane, committed, months, cx))
        })
}

/// Clear action, shown only when a date is set.
pub fn clear_button(id: &'static str) -> Button {
    components::icon_button(Button::new(id).ghost().icon(IconName::Close).tooltip("Clear due date"), metrics::CONTROL_XS)
}

/// Emits the pending date as the chosen one; the detail records it and closes the popover.
pub fn apply(state: &Entity<CalendarState>, cx: &mut App) {
    state.update(cx, |state, cx| cx.emit(CalendarEvent::Selected(state.date())));
}

/// Shows `date` as the pending choice and brings its month into view.
pub fn show_pending(state: &Entity<CalendarState>, date: Option<civil::Date>, window: &mut Window, cx: &mut App) {
    state.update(cx, |state, cx| {
        state.set_view(CalendarView::Day);
        state.set_date(Date::Single(date), window, cx);
    });
}

fn reset(state: &Entity<CalendarState>, input: &Entity<InputState>, committed: Option<civil::Date>, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| input.set_value("", window, cx));
    show_pending(state, committed, window, cx);
    state.update(cx, |state, cx| {
        if committed.is_none() {
            let today = state.today();
            state.select_year(today.year().into());
            state.select_month(today.month() as u8);
        }
        cx.notify();
    });
}

fn picker(
    state: &Entity<CalendarState>,
    input: &Entity<InputState>,
    pane: &Entity<MonthPane>,
    committed: Option<civil::Date>,
    months: usize,
    cx: &App,
) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    let inner = metrics::inner_radius(metrics::R_DIALOG, SHELL_INSET);
    let body = if state.read(cx).view().is_day() {
        (0..months).map(|offset| month_pane(state, input, pane, offset, months, cx).into_any_element()).collect::<Vec<_>>()
    } else {
        vec![month_picker(state, pane, months, cx).into_any_element()]
    };
    div().flex().flex_col()
        .child(div().m(px(SHELL_INSET)).mb_0().flex().flex_col().bg(t.surface).rounded(px(inner)).border_1().border_color(t.border).overflow_hidden()
            .child(header(input, cx))
            .child(div().flex().border_t_1().border_color(t.border)
                .child(rail(state, input, cx))
                .child(div().flex().gap(px(PANE_GAP)).px(px(PANE_PX)).pb(px(10.)).children(body))))
        .child(footer(state, input, committed, cx))
}

fn header(input: &Entity<InputState>, cx: &App) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    div().h(px(HEADER_H)).px(px(16.)).flex().items_center().gap_2()
        .child(Icon::new(IconName::Calendar).size(px(16.)).flex_none().text_color(t.text_secondary))
        .child(div().flex_1().min_w_0().t_body().text_color(t.text)
            .child(components::input_h(Input::new(input).appearance(false), 24.).px_0().t_body()))
}

fn rail(state: &Entity<CalendarState>, input: &Entity<InputState>, cx: &App) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    let (today, pending) = { let s = state.read(cx); (s.today(), s.date().start()) };
    div().w(px(RAIL_W)).flex_none().pt(px(8.)).pb_2().px(px(6.)).flex().flex_col().border_r_1().border_color(t.border)
        .children(PRESETS.iter().map(|&(preset, label)| {
            let date = preset_date(preset, today);
            let chosen = date == pending && (date.is_some() || matches!(preset, Preset::NoDate));
            let (state, input) = (state.clone(), input.clone());
            div().id(label).h(px(RAIL_ROW_H)).px(px(6.)).flex().items_center().rounded(px(metrics::R_CONTROL))
                .t_label().text_color(t.text).cursor_pointer()
                .when(chosen, |this| this.bg(t.selected).font_weight(FontWeight::MEDIUM))
                .when(!chosen, |this| this.hover(move |style| style.bg(t.hover)))
                .child(label)
                .on_click(move |_, window, cx| choose(&state, &input, date, window, cx))
        }))
}

fn choose(state: &Entity<CalendarState>, input: &Entity<InputState>, date: Option<civil::Date>, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| input.set_value("", window, cx));
    show_pending(state, date, window, cx);
}

fn month_pane(
    state: &Entity<CalendarState>,
    input: &Entity<InputState>,
    pane: &Entity<MonthPane>,
    offset: usize,
    months: usize,
    cx: &App,
) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    let (year, month) = state.read(cx).offset_year_month(offset);
    div().w(px(CELL_W * 7.)).flex().flex_col()
        .child(div().mt(px(13.)).mx(px(-6.)).h(px(PILL_H)).flex().items_center()
            .child(nav_slot(state, offset == 0, false, cx))
            .child(div().flex_1().flex().justify_center().child(month_pill(state, pane, offset, year, month, cx)))
            .child(nav_slot(state, offset + 1 == months, true, cx)))
        .child(div().mt(px(8.)).h(px(20.)).flex().children(WEEKDAYS.iter().map(|day| {
            div().w(px(CELL_W)).flex().items_center().justify_center().t_micro().text_color(t.text_secondary).child(*day)
        })))
        .child(div().mt(px(2.)).flex().flex_col().children(weeks(year, month).into_iter().map(|week| {
            div().flex().children(week.into_iter().map(|day| day_cell(state, input, day, cx)))
        })))
}

fn nav_slot(state: &Entity<CalendarState>, shown: bool, next: bool, cx: &App) -> AnyElement {
    if !shown {
        return div().w(px(NAV_BTN)).flex_none().into_any_element();
    }
    let t = theme::tokens(cx);
    let state = state.clone();
    let (id, icon, label) = if next { ("due-next", IconName::ChevronRight, "Next month") } else { ("due-prev", IconName::ChevronLeft, "Previous month") };
    div().id(id).size(px(NAV_BTN)).flex_none().flex().items_center().justify_center().rounded_full()
        .text_color(t.text_secondary).cursor_pointer().hover(move |style| style.bg(t.hover).text_color(t.text))
        .aria_label(label)
        .child(Icon::new(icon).size(px(16.)))
        .on_click(move |_, _, cx| state.update(cx, |state, cx| {
            if next { state.next_month() } else { state.prev_month() }
            cx.notify();
        }))
        .into_any_element()
}

fn month_pill(state: &Entity<CalendarState>, pane: &Entity<MonthPane>, offset: usize, year: i32, month: u32, cx: &App) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    let (state, pane) = (state.clone(), pane.clone());
    let label = format!("{} {year}", MONTHS[(month - 1) as usize]);
    div().id(("due-month", offset)).h(px(PILL_H)).pl(px(10.)).pr(px(8.)).flex().items_center().gap_1()
        .rounded_full().border_1().border_color(t.border).bg(t.surface).cursor_pointer()
        .hover(move |style| style.bg(t.hover))
        .t_label().font_weight(FontWeight::MEDIUM).text_color(t.text)
        .child(label)
        .child(Icon::new(IconName::ChevronDown).size(px(14.)).text_color(t.text_secondary))
        .on_click(move |_, _, cx| {
            pane.update(cx, |pane, _| pane.0 = offset);
            state.update(cx, |state, cx| { state.set_view(CalendarView::Month); cx.notify() });
        })
}

/// Month/year chooser that replaces the panes; it sets the month of the pane that opened it.
fn month_picker(state: &Entity<CalendarState>, pane: &Entity<MonthPane>, months: usize, cx: &App) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    let offset = pane.read(cx).0.min(months - 1);
    let (year, current) = state.read(cx).offset_year_month(offset);
    let width = CELL_W * 7. * months as f32 + PANE_GAP * (months - 1) as f32;
    let step = |delta: i32, cx: &App| {
        let state = state.clone();
        let (id, icon) = if delta < 0 { ("due-year-prev", IconName::ChevronLeft) } else { ("due-year-next", IconName::ChevronRight) };
        div().id(id).size(px(NAV_BTN)).flex().items_center().justify_center().rounded_full().cursor_pointer()
            .text_color(theme::tokens(cx).text_secondary).hover(move |style| style.bg(t.hover))
            .child(Icon::new(icon).size(px(16.)))
            .on_click(move |_, _, cx| state.update(cx, |state, cx| {
                // Moving the base by a whole year moves every pane, including a right pane
                // across Dec → Jan, by exactly one displayed year.
                state.select_year(state.current_year() + delta);
                state.set_view(CalendarView::Month);
                cx.notify();
            }))
    };
    div().w(px(width)).flex().flex_col()
        .child(div().mt(px(13.)).mx(px(-6.)).h(px(PILL_H)).flex().items_center().justify_between()
            .child(step(-1, cx))
            .child(div().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).child(year.to_string()))
            .child(step(1, cx)))
        .child(div().mt(px(12.)).grid().grid_cols(4).gap_1().children((1..=12u32).map(|month| {
            let state = state.clone();
            let active = month == current;
            div().id(("due-pick-month", month as usize)).h(px(32.)).flex().items_center().justify_center().rounded_full()
                .t_label().text_color(t.text).cursor_pointer()
                .when(active, |this| this.bg(t.selected).font_weight(FontWeight::MEDIUM))
                .when(!active, |this| this.hover(move |style| style.bg(t.hover)))
                .child(&MONTHS[(month - 1) as usize][..3])
                .on_click(move |_, _, cx| state.update(cx, |state, cx| {
                    // Show `month` of the displayed `year` in this pane, then move the base
                    // back so the pane at `offset` lands on it (Dec → Jan safe).
                    state.select_year(year);
                    state.select_month(month as u8);
                    (0..offset).for_each(|_| state.prev_month());
                    state.set_view(CalendarView::Day);
                    cx.notify();
                }))
        })))
}

/// Monday-first weeks; days of the neighbouring months are left blank.
fn weeks(year: i32, month: u32) -> Vec<Vec<Option<civil::Date>>> {
    let Ok(first) = civil::Date::new(year as i16, month as i8, 1) else { return Vec::new() };
    let lead = first.weekday().to_monday_zero_offset() as i32;
    let days = first.days_in_month() as i32;
    let cells = (lead + days + 6) / 7 * 7;
    let all: Vec<_> = (1 - lead..=cells - lead)
        .map(|day| (1..=days).contains(&day).then(|| civil::Date::new(first.year(), first.month(), day as i8).ok()).flatten())
        .collect();
    all.chunks(7).map(<[_]>::to_vec).collect()
}

fn day_cell(state: &Entity<CalendarState>, input: &Entity<InputState>, day: Option<civil::Date>, cx: &App) -> AnyElement {
    let cell = div().w(px(CELL_W)).h(px(ROW_H)).flex().items_center().justify_center();
    let Some(date) = day else { return cell.into_any_element() };
    let t = theme::tokens(cx);
    let (today, pending) = { let s = state.read(cx); (s.today(), s.date().start()) };
    let (chosen, is_today, past) = (pending == Some(date), date == today, date < today);
    let (state, input) = (state.clone(), input.clone());
    cell.child(div().id(SharedString::from(format!("due-{date}"))).size(px(RING)).flex().items_center().justify_center().rounded_full()
        .t_label().font_features(components::typography::tabular()).cursor_pointer()
        .aria_label(SharedString::from(date.to_string()))
        .text_color(if past { t.text_tertiary } else { t.text })
        .when(is_today && !chosen, |this| this.border_1().border_color(t.selected))
        .when(chosen, |this| this.bg(t.accent_fill).text_color(t.accent_text).font_weight(FontWeight::MEDIUM))
        .when(!chosen, |this| this.hover(move |style| style.bg(t.hover)))
        .child(date.day().to_string())
        .on_click(move |_, window, cx| {
            input.update(cx, |input, cx| input.set_value("", window, cx));
            state.update(cx, |state, cx| {
                // `select_date` re-bases the view on the chosen day; a click on a visible
                // day keeps both panes where they are.
                let (year, month) = (state.current_year(), state.current_month());
                state.select_date(date);
                state.select_year(year);
                state.select_month(month);
                cx.notify()
            });
        }))
        .into_any_element()
}

fn footer(state: &Entity<CalendarState>, input: &Entity<InputState>, committed: Option<civil::Date>, cx: &App) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    let text = input.read(cx).value().trim().to_string();
    let typed_invalid = !text.is_empty() && parse(&text, state.read(cx).today()).is_none();
    let pending = state.read(cx).date().start();
    let hint: SharedString = match (typed_invalid, pending) {
        (true, _) => "Couldn't read that date.".into(),
        (false, Some(date)) => format!("Due {}", date.strftime("%a, %b %-d, %Y")).into(),
        (false, None) if committed.is_some() => "No due date.".into(),
        (false, None) => "Type or select a date.".into(),
    };
    let enabled = !typed_invalid && pending != committed;
    let state = state.clone();
    div().h(px(FOOTER_H)).pl(px(14.)).pr(px(10.)).flex().items_center().justify_between().gap_2()
        .child(div().min_w_0().truncate().t_label().text_color(if typed_invalid { t.danger } else { t.text_secondary }).child(hint))
        .child(div().id("due-apply").h(px(PILL_H)).px(px(12.)).flex().flex_none().items_center().rounded_full()
            .t_label().font_weight(FontWeight::MEDIUM)
            .map(|this| if enabled {
                this.bg(t.accent_fill).text_color(t.accent_text).cursor_pointer()
                    .hover(move |style| style.opacity(0.88))
                    .on_click(move |_, _, cx| apply(&state, cx))
            } else {
                this.bg(t.text_tertiary).text_color(t.surface).opacity(0.8)
            })
            .child("Apply date"))
}

fn preset_date(preset: Preset, today: civil::Date) -> Option<civil::Date> {
    let days = |n: i64| today.checked_add(jiff::Span::new().days(n)).ok();
    match preset {
        Preset::Today => Some(today),
        Preset::Tomorrow => days(1),
        Preset::ThisWeekend if matches!(today.weekday(), Weekday::Saturday | Weekday::Sunday) => Some(today),
        Preset::ThisWeekend => today.nth_weekday(1, Weekday::Saturday).ok(),
        Preset::NextWeek => today.nth_weekday(1, Weekday::Monday).ok(),
        Preset::TwoWeeks => days(14),
        Preset::NextMonth => today.checked_add(jiff::Span::new().months(1)).ok(),
        Preset::NoDate => None,
    }
}

/// Reads a typed due date: ISO `2026-11-27`, `today`/`tomorrow`/`yesterday`, a weekday
/// (`fri`, `next friday`: the next one after today), `in 3 days`/`in 2 weeks`/`in 1 month`,
/// and `nov 27`, `27 november`, `nov 27 2027` (without a year, the next such day).
pub fn parse(text: &str, today: civil::Date) -> Option<civil::Date> {
    let text = text.trim().to_lowercase();
    if let Ok(date) = text.parse::<civil::Date>() {
        return Some(date);
    }
    let words: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect();
    match words.as_slice() {
        ["today"] => Some(today),
        ["tomorrow" | "tmr" | "tmrw"] => today.tomorrow().ok(),
        ["yesterday"] => today.yesterday().ok(),
        ["in", n, unit] | [n, unit] if n.parse::<i64>().is_ok() => relative(today, n.parse().ok()?, unit),
        ["next" | "this", day] | [day] if weekday(day).is_some() => today.nth_weekday(1, weekday(day)?).ok(),
        _ => month_day(&words, today),
    }
}

fn relative(today: civil::Date, n: i64, unit: &str) -> Option<civil::Date> {
    let span = match unit.trim_end_matches('s') {
        "day" | "d" => jiff::Span::new().try_days(n).ok()?,
        "week" | "w" => jiff::Span::new().try_weeks(n).ok()?,
        "month" | "mo" => jiff::Span::new().try_months(n).ok()?,
        _ => return None,
    };
    today.checked_add(span).ok()
}

fn weekday(word: &str) -> Option<Weekday> {
    const DAYS: [(&str, Weekday); 7] = [
        ("monday", Weekday::Monday), ("tuesday", Weekday::Tuesday), ("wednesday", Weekday::Wednesday),
        ("thursday", Weekday::Thursday), ("friday", Weekday::Friday), ("saturday", Weekday::Saturday), ("sunday", Weekday::Sunday),
    ];
    (word.len() >= 3).then_some(())?;
    DAYS.iter().find(|(name, _)| name.starts_with(word)).map(|&(_, day)| day)
}

fn month_number(word: &str) -> Option<i8> {
    (word.len() >= 3).then_some(())?;
    MONTHS.iter().position(|name| name.to_lowercase().starts_with(word)).map(|i| i as i8 + 1)
}

fn month_day(words: &[&str], today: civil::Date) -> Option<civil::Date> {
    let (month, day, year) = match words {
        [m, d] | [m, d, _] if month_number(m).is_some() => (month_number(m)?, d.trim_end_matches(char::is_alphabetic), words.get(2)),
        [d, m] | [d, m, _] if month_number(m).is_some() => (month_number(m)?, d.trim_end_matches(char::is_alphabetic), words.get(2)),
        _ => return None,
    };
    let day: i8 = day.parse().ok()?;
    match year {
        Some(year) => civil::Date::new(year.parse().ok()?, month, day).ok(),
        None => {
            let this_year = civil::Date::new(today.year(), month, day).ok()?;
            if this_year >= today { Some(this_year) } else { civil::Date::new(today.year() + 1, month, day).ok() }
        }
    }
}
