//! Small presentational helpers shared by the projects/tasks/workflow/onboarding views.
//! Styling delegates to `crate::components` (visual contract); these keep the view-level API.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName,
    Side,
    button::{Button, ButtonVariants as _},
    menu::{PopupMenu, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{ExecutionTarget, HarnessId, Priority, ProjectKind, TaskStatus};

use crate::components::{self, Tone, Typography as _};
use crate::{api::ApiError, theme::{self, metrics}};

/// Request lifecycle for one fetched resource.
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn ready_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn from_result(result: Result<T, ApiError>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(error) => Self::Failed(error_text(&error)),
        }
    }
}

/// User-facing message for a failed request.
pub fn error_text(error: &ApiError) -> String {
    if error.is_conflict() {
        "This item changed elsewhere since you opened it.".into()
    } else if error.is_not_implemented() {
        "The service does not implement this action yet.".into()
    } else if error.is_not_found() {
        "This item no longer exists.".into()
    } else {
        error.to_string()
    }
}

// ---- Labels -------------------------------------------------------------------------

/// Known statuses in board order, followed by any unknown raw statuses present in `tasks`
/// (written by a newer service) so no task is hidden.
pub fn statuses_for(tasks: &[protocol::Task]) -> Vec<TaskStatus> {
    let mut statuses = BOARD_STATUSES.to_vec();
    for task in tasks {
        if !statuses.contains(&task.status) {
            statuses.push(task.status.clone());
        }
    }
    statuses
}

pub const BOARD_STATUSES: [TaskStatus; 6] = [
    TaskStatus::Backlog,
    TaskStatus::Todo,
    TaskStatus::InProgress,
    TaskStatus::InReview,
    TaskStatus::Blocked,
    TaskStatus::Done,
];

pub fn status_label(status: &TaskStatus) -> &str {
    match status {
        TaskStatus::Backlog => "Backlog",
        TaskStatus::Todo => "Todo",
        TaskStatus::InProgress => "In progress",
        TaskStatus::InReview => "In review",
        TaskStatus::Blocked => "Blocked",
        TaskStatus::Done => "Done",
        TaskStatus::Unknown(raw) => raw,
    }
}

/// Exit rule shown under each column header (Lemonade board pattern).
pub fn status_rule(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Backlog => "moves on when it is prioritised",
        TaskStatus::Todo => "moves on when work starts",
        TaskStatus::InProgress => "moves on when the change is ready",
        TaskStatus::InReview => "gate: the owner approves",
        TaskStatus::Blocked => "needs you to unblock",
        TaskStatus::Done => "shipped",
        TaskStatus::Unknown(_) => "status from a newer service",
    }
}

pub fn priority_label(priority: &Priority) -> &'static str {
    match priority {
        Priority::Low => "Low",
        Priority::Normal => "Normal",
        Priority::High => "High",
        Priority::Urgent => "Urgent",
        Priority::Unknown(_) => "Unknown",
    }
}

pub fn kind_label(kind: &ProjectKind) -> &'static str {
    match kind {
        ProjectKind::Standalone => "Standalone",
        ProjectKind::Github => "GitHub",
        ProjectKind::Linear => "Linear",
        ProjectKind::Unknown(_) => "Other",
    }
}

/// No Linear logo ships with the icon set, so Linear uses the neutral issue-board glyph.
pub fn kind_icon(kind: &ProjectKind) -> gpui_kit::assets::IconName {
    use gpui_kit::assets::IconName as Lucide;
    match kind {
        ProjectKind::Github => Lucide::Github,
        ProjectKind::Linear => Lucide::SquareKanban,
        _ => Lucide::Folder,
    }
}

pub fn harness_label(harness: &HarnessId) -> &'static str {
    match harness {
        HarnessId::Claude => "Claude Code",
        HarnessId::Codex => "Codex",
        HarnessId::Copilot => "Copilot CLI",
        HarnessId::Strands => "Strands Agents",
        HarnessId::Custom => "Custom executable",
        HarnessId::Unknown(_) => "Unknown harness",
    }
}

pub fn target_label(target: &ExecutionTarget) -> &'static str {
    match target {
        ExecutionTarget::Host => "Local host",
        ExecutionTarget::Mxc => "MXC sandbox",
        ExecutionTarget::AppleContainer => "Apple container",
        ExecutionTarget::Unknown(_) => "Unknown target",
    }
}

pub fn target_detail(target: &ExecutionTarget) -> &'static str {
    match target {
        ExecutionTarget::Host => "Runs directly in your checkout. No isolation.",
        ExecutionTarget::Mxc => "Seatbelt process-tree isolation on macOS 15+.",
        ExecutionTarget::AppleContainer => "Linux OCI container in a lightweight VM, macOS 26.",
        ExecutionTarget::Unknown(_) => "Configured by a newer version of the service.",
    }
}

/// `2026-11-24T10:00:00Z` → `Nov 24`. Falls back to the raw date.
pub fn short_date(timestamp: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let date = timestamp.get(..10).unwrap_or(timestamp);
    let mut parts = date.split('-');
    let (_, Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return date.to_string();
    };
    match (month.parse::<usize>(), day.parse::<u32>()) {
        (Ok(month @ 1..=12), Ok(day)) => format!("{} {day}", MONTHS[month - 1]),
        _ => date.to_string(),
    }
}

// ---- Colours ------------------------------------------------------------------------

pub fn status_color(status: &TaskStatus, cx: &App) -> Hsla {
    let tokens = theme::tokens(cx);
    match status {
        TaskStatus::InProgress => tokens.running,
        TaskStatus::InReview => tokens.validating,
        TaskStatus::Blocked => tokens.attention,
        TaskStatus::Done => tokens.success,
        _ => tokens.text_tertiary,
    }
}

/// Amber used for anything waiting on a person.
pub fn needs_you(cx: &App) -> Hsla {
    theme::tokens(cx).attention
}

/// Inset tray behind cards (board columns, footers).
pub fn tray(cx: &App) -> Hsla {
    theme::tokens(cx).tray
}

// ---- Elements -----------------------------------------------------------------------
// Thin adapters over `crate::components` so these views share one visual language
// (`.groundwork/guides/visual-contract.md`); callers keep their data and callbacks.

/// Status glyph: dashed ring (backlog/todo), part-filled ring with dot (active),
/// check disc (done). Matches refs 97129A65 / C24C9437 / AE10217A.
pub fn status_icon(status: &TaskStatus, cx: &App) -> AnyElement {
    let color = status_color(status, cx);
    let ring = div().size(px(14.)).flex_none().rounded_full().flex().items_center().justify_center();
    match status {
        TaskStatus::Done => ring
            .bg(color)
            .child(Icon::new(IconName::Check).size(px(10.)).text_color(cx.theme().background))
            .into_any_element(),
        TaskStatus::InProgress | TaskStatus::InReview => ring
            .border(px(1.5))
            .border_dashed()
            .border_color(color)
            .child(div().size(px(6.)).rounded_full().bg(color))
            .into_any_element(),
        TaskStatus::Blocked => ring
            .border(px(1.5))
            .border_color(color)
            .child(div().w(px(6.)).h(px(1.5)).bg(color))
            .into_any_element(),
        _ => ring
            .border(px(1.5))
            .border_dashed()
            .border_color(theme::tokens(cx).text_tertiary)
            .into_any_element(),
    }
}

/// Three ascending bars; filled bars indicate priority (refs 630BC713, AE10217A).
pub fn priority_bars(priority: &Priority, cx: &App) -> AnyElement {
    let t = theme::tokens(cx);
    if *priority == Priority::Urgent {
        return div()
            .size(px(14.))
            .flex_none()
            .rounded(px(3.))
            .bg(needs_you(cx))
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.theme().background)
            .t_caption()
            .font_weight(FontWeight::BOLD)
            .child("!")
            .into_any_element();
    }
    let filled = match priority {
        Priority::Low => 1,
        Priority::Normal => 2,
        Priority::High => 3,
        _ => 0,
    };
    div()
        .flex()
        .flex_none()
        .items_end()
        .gap(px(1.5))
        .h(px(12.))
        .children((0..3).map(|index| {
            let on = index < filled;
            div()
                .w(px(3.))
                .h(px(4. + index as f32 * 3.))
                .rounded(px(1.))
                .bg(if on { t.text_secondary } else { t.border })
        }))
        .into_any_element()
}

/// Count beside a task group heading: tactile pill, 1px border, tabular
/// (refs 97129A65 / C24C9437 group counts).
pub fn count_pill(count: usize, cx: &App) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    div().flex_none().h(px(18.)).min_w(px(20.)).px_1p5().flex().items_center().justify_center()
        .rounded_full().border_1().border_color(t.border).bg(t.surface).shadow_xs()
        .t_caption().font_features(components::typography::tabular()).text_color(t.text_secondary)
        .child(count.to_string())
}

/// Neutral tag (labels, metadata): contract chip without a tone dot.
pub fn chip(text: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(metrics::CHIP_H))
        .px_1p5()
        .rounded(px(metrics::R_CONTROL))
        .bg(t.tray)
        .t_caption()
        .text_color(t.text_secondary)
        .child(text.into())
}

/// Amber chip for items that wait on a person.
pub fn amber_chip(text: impl Into<SharedString>, cx: &App) -> Div {
    components::status_pill(text, Tone::Attention, cx)
}

/// The connected service is older than this feature: say so instead of offering a control
/// that would fail. `what` completes "Update Karabiner's service to …".
pub fn update_service_note(what: &str, cx: &App) -> Div {
    div().t_caption().text_color(theme::tokens(cx).text_secondary).child(format!("Update Karabiner's service to {what}."))
}

pub fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// Field caption above a control ("Name", "Exit rule").
pub fn caption(text: impl Into<SharedString>, cx: &App) -> Div {
    div().t_label().font_weight(FontWeight::MEDIUM).text_color(theme::tokens(cx).text).child(text.into())
}

/// Form field: caption above a control, 6 between them.
pub fn field(label: &'static str, control: impl IntoElement, cx: &App) -> Div {
    field_owned(label, control, cx)
}

/// `field` with a runtime label.
pub fn field_owned(label: impl Into<SharedString>, control: impl IntoElement, cx: &App) -> Div {
    div().flex().flex_col().gap_1p5().child(caption(label, cx)).child(control)
}

/// Bordered surface card (radius 12).
pub fn card(cx: &App) -> Div {
    components::card(cx)
}

/// Skeleton rows matching list rows (ref 71B3729C).
pub fn skeleton_rows(count: usize, cx: &App) -> impl IntoElement + use<> {
    let bone = cx.theme().skeleton;
    div().flex().flex_col().gap_1().px_6().pt_4().children((0..count).map(move |index| {
        div().h(px(metrics::ROW_LIST)).px_2().flex().items_center().gap_3()
            .child(div().size(px(14.)).rounded_full().bg(bone))
            .child(div().h(px(10.)).w(px(160. + (index % 3) as f32 * 60.)).rounded_full().bg(bone))
            .child(div().flex_1())
            .child(div().h(px(10.)).w(px(56.)).rounded_full().bg(bone))
    }))
}

/// Error panel with an explicit retry.
pub fn error_panel(
    title: &'static str,
    message: &str,
    retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    _cx: &App,
) -> impl IntoElement {
    components::error_state(format!("{title}. {message}"), retry)
}

/// Centered empty state with an optional primary action.
pub fn empty_panel(title: &'static str, detail: &'static str, action: Option<AnyElement>, cx: &App) -> impl IntoElement + use<> {
    let t = theme::tokens(cx);
    div().size_full().flex().flex_col().items_center().justify_center().gap_1().p_6()
        .child(div().t_body().font_weight(FontWeight::MEDIUM).text_color(t.text).child(title))
        .child(div().t_body().text_color(t.text_secondary).max_w(px(420.)).text_center().child(detail))
        .children(action.map(|action| div().pt_3().child(action)))
}

/// Banner for unsaved/conflicting edits. `conflict` switches it to Reload / Keep mine.
/// Flush with the bottom of its container; `outer` is that container's radius (0 if square).
pub fn save_bar(
    id: &'static str,
    conflict: bool,
    saving: bool,
    error: Option<&str>,
    on_discard: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_save: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let t = theme::tokens(cx);
    let amber = needs_you(cx);
    let (message, discard, save): (SharedString, &str, &str) = if conflict {
        ("Changed elsewhere. Reload theirs or keep your edits.".into(), "Reload", "Keep mine")
    } else {
        (error.map_or("Unsaved changes".into(), |error| error.to_string().into()), "Discard", "Save")
    };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_3()
        // Trailing 24 buttons: right inset equals their 8 vertical inset.
        .pl_4()
        .pr_2()
        .py_2()
        .border_t_1()
        .border_color(if conflict { amber.opacity(0.5) } else { t.border })
        .bg(if conflict { Tone::Attention.tint(cx, 0.10) } else { t.tray })
        .t_label()
        .child(div().min_w_0().truncate().text_color(if conflict { amber } else { t.text_secondary }).child(message))
        .child(
            div().flex().flex_none().gap_1()
                .child(components::button_sm(Button::new((id, 0usize)).ghost().label(discard)).on_click(on_discard))
                .child(components::button_sm(Button::new((id, 1usize)).primary().label(save)).loading(saving).on_click(on_save)),
        )
}

/// Status a stage stands for (its configured status, else in progress); drives its colour.
pub fn stage_status(stage: &protocol::StageDefinition) -> TaskStatus {
    stage.status.clone().unwrap_or(TaskStatus::InProgress)
}

/// Stage mark shared by board columns, the stage property and its menu: an 8 dot in the
/// stage's status colour; tertiary when the task isn't in a stage ("Not started").
pub fn stage_dot(status: Option<&TaskStatus>, cx: &App) -> AnyElement {
    let color = status.map_or_else(|| theme::tokens(cx).text_tertiary, |status| status_color(status, cx));
    components::status_dot(Tone::Neutral, cx).bg(color).into_any_element()
}

/// Property picker menu (Linear issue-property menus): leading glyphs, check on the right.
pub fn option_menu(menu: PopupMenu) -> PopupMenu {
    menu.check_side(Side::Right).min_w(px(220.))
}

/// One picker option: 16-wide glyph slot, label, optional muted detail, check on the right
/// (with `option_menu`). The glyph slot keeps every label on one x.
pub fn option_item(
    glyph: impl Fn(&App) -> AnyElement + 'static,
    label: impl Into<SharedString>,
    detail: Option<SharedString>,
    checked: bool,
    on_pick: impl Fn(&mut App) + 'static,
) -> PopupMenuItem {
    let label = label.into();
    PopupMenuItem::element(move |_, cx| {
        let t = theme::tokens(cx);
        div().flex_1().min_w_0().flex().items_center().gap_1p5()
            .child(div().w(px(16.)).flex_none().flex().items_center().justify_center().child(glyph(&*cx)))
            .child(div().min_w_0().truncate().child(label.clone()))
            .children(detail.clone().map(|detail| div().ml_auto().pl_3().flex_none().t_caption().text_color(t.text_tertiary).child(detail)))
    })
    .checked(checked)
    .on_click(move |_, _, cx| on_pick(cx))
}

/// Glyph for an icon-backed option, sized to the option slot.
pub fn option_icon(icon: IconName, cx: &App) -> AnyElement {
    Icon::new(icon).size(px(14.)).text_color(theme::tokens(cx).text_secondary).into_any_element()
}

/// Segmented control over typed values, drawn with the shared `components::segmented`.
pub fn segmented<T: Clone + PartialEq + 'static, F: Fn(T, &mut Window, &mut App) + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    selected: T,
    on_pick: F,
    cx: &App,
) -> impl IntoElement + use<T, F> {
    let values: Vec<T> = options.iter().map(|(value, _)| value.clone()).collect();
    let index = values.iter().position(|value| *value == selected).unwrap_or(usize::MAX);
    let items = options.iter().map(|(_, label)| components::Segment::new(*label)).collect();
    components::segmented(id, items, index, move |index, window, cx| {
        if let Some(value) = values.get(index) { on_pick(value.clone(), window, cx) }
    }, cx)
}

/// Selectable option card (template and execution-target pickers, ref 85F1DC37).
pub fn choice_card(
    id: impl Into<ElementId>,
    selected: bool,
    disabled: bool,
    cx: &App,
) -> Stateful<Div> {
    let t = theme::tokens(cx);
    div()
        .id(id)
        .p_3()
        .rounded(px(metrics::R_CARD))
        .border_1()
        .border_color(if selected { t.text } else { t.border })
        .bg(t.surface)
        .when(selected, |this| this.shadow_xs())
        .when(disabled, |this| this.opacity(0.55))
        .when(!disabled && !selected, |this| this.cursor_pointer().hover(move |s| s.bg(t.hover)))
        .when(!disabled && selected, |this| this.cursor_pointer())
}

/// Async-task helper: update an entity with the active window (input `set_value` needs one).
pub trait UpdateInWindow<T: 'static> {
    fn update_in_window(&self, cx: &mut AsyncApp, f: impl FnOnce(&mut T, &mut Window, &mut Context<T>));
}

impl<T: 'static> UpdateInWindow<T> for WeakEntity<T> {
    /// Runs `f` with the active window when one exists (input `set_value` needs a window).
    fn update_in_window(&self, cx: &mut AsyncApp, f: impl FnOnce(&mut T, &mut Window, &mut Context<T>)) {
        let entity = self.clone();
        cx.update(|cx| {
            let Some(window) = cx.active_window() else { return };
            window.update(cx, |_, window, cx| { entity.update(cx, |this, cx| f(this, window, cx)).ok(); }).ok();
        });
    }
}


/// Message the service attached to a failed request (e.g. a dependency conflict).
pub fn service_message(error: &ApiError) -> String {
    match error {
        ApiError::Status { body: Some(body), .. } => body.message.clone(),
        other => other.to_string(),
    }
}

/// Seconds since the Unix epoch for `YYYY-MM-DDTHH:MM:SS[.fff]Z` (UTC). `None` if malformed.
pub fn epoch_seconds(stamp: &str) -> Option<f64> {
    let number = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute) = (number(11..13)?, number(14..16)?);
    let seconds: f64 = stamp.get(17..)?.trim_end_matches('Z').parse().ok()?;
    // Days from civil (Howard Hinnant).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + hour * 3_600 + minute * 60) as f64 + seconds)
}

#[cfg(test)]
mod tests {
    use super::epoch_seconds;

    #[test]
    fn epoch_seconds_spans_dates() {
        let start = epoch_seconds("2026-11-24T23:59:30.000Z").unwrap();
        let end = epoch_seconds("2026-11-25T00:00:45.500Z").unwrap();
        assert_eq!(end - start, 75.5);
        assert_eq!(epoch_seconds("1970-01-01T00:00:00Z"), Some(0.));
        assert!(epoch_seconds("not a date").is_none());
    }
}
