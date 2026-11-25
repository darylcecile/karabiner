//! Workflow feature: global Inbox (interventions) and Settings (app preferences, harness
//! and execution-target reports), plus the project configuration editors used by the
//! project tab (`config`, `editor`, `agents`).

pub mod agents;
pub mod config;
pub mod editor;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    switch::Switch,
};
use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{
    AppSettings, ExecutionTarget, ExecutionTargetInfo, HarnessId, HarnessInfo, Intervention, InterventionKind,
    InterventionStatus, Project, Theme, UpdateIntervention,
};

use crate::{
    api::query_value,
    state::{AppState, TabTarget},
    views::tasks::ui::{self, Load},
};
use crate::components::{self, Typography as _};
use crate::theme::{self as theme_tokens, metrics};

pub fn inbox_section(state: AppState, window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Inbox::new(state, window, cx)).into()
}

pub fn settings_section(state: AppState, _window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Settings::new(state, cx)).into()
}

/// Re-runs `reload` whenever the shared data version moves.
fn watch<T: 'static>(state: &AppState, cx: &mut Context<T>, reload: fn(&mut T, &mut Context<T>)) -> Subscription {
    let mut seen = state.service.read(cx).data_version();
    cx.observe(&state.service, move |this, service, cx| {
        let version = service.read(cx).data_version();
        if version != seen {
            seen = version;
            reload(this, cx);
        }
    })
}

// ---- Inbox ---------------------------------------------------------------------------

struct InboxItem {
    intervention: Intervention,
    project: String,
}

pub struct Inbox {
    state: AppState,
    items: Load<Vec<InboxItem>>,
    show_resolved: bool,
    error: Option<String>,
    /// Intervention currently being answered, and its answer input.
    responding: Option<String>,
    response: Entity<InputState>,
    _load: Option<gpui_kit::Task<()>>,
    _mutations: Vec<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Inbox {
    fn new(state: AppState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let response = cx.new(|cx| InputState::new(window, cx).placeholder("Your answer, then Enter"));
        let subscriptions = vec![
            watch(&state, cx, Self::reload),
            cx.subscribe_in(&response, window, |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let text = input.read(cx).value().trim().to_string();
                    if let (false, Some(id)) = (text.is_empty(), this.responding.clone()) {
                        input.update(cx, |input, cx| input.set_value("", window, cx));
                        this.answer(&id, text, cx);
                    }
                }
            }),
        ];
        let mut this = Self {
            state, items: Load::Loading, show_resolved: false, error: None, responding: None, response,
            _load: None, _mutations: Vec::new(), _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    fn answer(&mut self, id: &str, response: String, cx: &mut Context<Self>) {
        let Some(item) = self.items.ready().and_then(|items| items.iter().find(|item| item.intervention.id == id)).map(|item| item.intervention.clone()) else { return };
        self.responding = None;
        self.update(&item, InterventionStatus::Answered, Some(response), cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let projects = api.get::<Vec<Project>>("/v1/projects").await?;
                let mut items = Vec::new();
                for project in projects {
                    let path = format!("/v1/interventions?projectId={}", query_value(&project.id));
                    for intervention in api.get::<Vec<Intervention>>(&path).await? {
                        items.push(InboxItem { intervention, project: project.name.clone() });
                    }
                }
                items.sort_by(|a, b| b.intervention.created_at.cmp(&a.intervention.created_at));
                Ok(items)
            }.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(items) => { this.items = Load::Ready(items); this.error = None }
                    Err(error) if this.items.ready().is_some() => this.error = Some(format!("Couldn't refresh: {}", ui::error_text(&error))),
                    Err(error) => this.items = Load::Failed(ui::error_text(&error)),
                }
                cx.notify()
            }).ok();
        }));
    }

    fn update(&mut self, item: &Intervention, status: InterventionStatus, response: Option<String>, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/interventions/{}", query_value(&item.id));
        let body = UpdateIntervention { revision: Some(item.revision), status: Some(status), response: response.map(Some) };
        self._mutations.push(cx.spawn(async move |this, cx| {
            let result = api.patch::<_, Intervention>(&path, &body).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.state.data_changed(cx),
                    Err(error) => this.error = Some(ui::service_message(&error)),
                }
                cx.notify();
            }).ok();
        }));
    }

    fn render_item(&self, item: &InboxItem, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let intervention = item.intervention.clone();
        let open = intervention.status == InterventionStatus::Recorded;
        let amber = ui::needs_you(cx);
        let (dismiss, answer_id) = (intervention.clone(), intervention.id.clone());
        let responding = self.responding.as_deref() == Some(intervention.id.as_str());
        let response_input = self.response.clone();
        let intervention_message = intervention.message.clone();
        let task = intervention.task_id.clone().map(|task_id| TabTarget::task(intervention.project_id.clone(), task_id));
        let project = TabTarget::project(intervention.project_id.clone());
        let state = self.state.clone();
        let row = div().id(SharedString::from(format!("inbox-{}", intervention.id)))
            .flex().items_start().gap_3().p_4().rounded(px(metrics::R_CARD)).border_1()
            .border_color(if open { amber.opacity(0.45) } else { theme_tokens::tokens(cx).border })
            .bg(theme_tokens::tokens(cx).surface).cursor_pointer().hover(|style| style.bg(theme_tokens::tokens(cx).hover))
            .on_click(move |_, window, cx| state.open(task.clone().unwrap_or_else(|| project.clone()), window, cx))
            .child(div().h(px(20.)).flex().items_center().child(Icon::new(if open { IconName::CircleAlert } else { IconName::CircleCheck }).size(px(14.))
                .text_color(if open { amber } else { cx.theme().muted_foreground })))
            .child(div().flex_1().min_w_0().flex().flex_col().gap_1p5()
                .child(div().flex().items_center().gap_2()
                    .child(if open { ui::amber_chip(kind_label(&intervention.kind), cx) } else { ui::chip(kind_label(&intervention.kind), cx) })
                    .child(div().t_caption().text_color(cx.theme().muted_foreground).child(item.project.clone()))
                    .child(div().ml_auto().t_caption().text_color(cx.theme().muted_foreground).child(ui::short_date(&intervention.created_at))))
                .child(div().t_body().text_color(theme_tokens::tokens(cx).text).child(intervention.message.clone()))
                .child(div().t_caption().text_color(cx.theme().muted_foreground).child(match intervention.status {
                    InterventionStatus::Recorded => "Recorded. Not delivered: execution isn't available yet.".to_string(),
                    InterventionStatus::Answered => format!("Answered: {}", intervention.response.clone().unwrap_or_default()),
                    InterventionStatus::Dismissed => "Dismissed".to_string(),
                    InterventionStatus::Unknown(ref raw) => raw.clone(),
                })))
            .when(open, |this| this.child(div().flex().gap_1()
                .child(components::button_sm(Button::new(SharedString::from(format!("inbox-dismiss-{}", intervention.id))).ghost().label("Dismiss"))
                    .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.update(&dismiss, InterventionStatus::Dismissed, None, cx) })))
                .child(components::button_sm(Button::new(SharedString::from(format!("inbox-answer-{}", intervention.id))).outline().label("Answer"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.responding = Some(answer_id.clone());
                        this.response.update(cx, |input, cx| input.focus(window, cx));
                        cx.notify();
                    })))));
        div().flex().flex_col().gap_2()
            .child(row)
            .when(responding, |this| this.child(
                div().ml(px(28.)).pl_3().border_l_2().border_color(ui::needs_you(cx).opacity(0.5)).flex().flex_col().gap_1()
                    .child(div().t_caption().text_color(cx.theme().muted_foreground).child(format!("Answer to: {}", intervention_message)))
                    .child(components::input_md(Input::new(&response_input))),
            ))
    }
}

/// Joins non-empty parts with " · " (no orphan separators).
pub fn join_parts(parts: &[&str]) -> String {
    parts.iter().map(|part| part.trim()).filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ")
}

fn kind_label(kind: &InterventionKind) -> String {
    match kind {
        InterventionKind::Unknown(raw) => raw.clone(),
        known => {
            let raw = known.as_str();
            raw[..1].to_uppercase() + &raw[1..]
        }
    }
}

impl Render for Inbox {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.items {
            Load::Loading => ui::skeleton_rows(4, cx).into_any_element(),
            Load::Failed(error) => {
                let weak = cx.weak_entity();
                ui::error_panel("Couldn't load inbox", error, move |_, _, cx| { weak.update(cx, |this, cx| this.reload(cx)).ok(); }, cx).into_any_element()
            }
            Load::Ready(items) => {
                let visible: Vec<&InboxItem> = items.iter()
                    .filter(|item| self.show_resolved || item.intervention.status == InterventionStatus::Recorded).collect();
                if visible.is_empty() {
                    ui::empty_panel(
                        if self.show_resolved { "Nothing here yet" } else { "You're all caught up" },
                        "Instructions, approvals and questions you record on tasks appear here until you resolve them.",
                        None, cx,
                    ).into_any_element()
                } else {
                    div().flex().flex_col().gap_2().children(visible.into_iter().map(|item| self.render_item(item, cx))).into_any_element()
                }
            }
        };
        let open = self.items.ready().map_or(0, |items| items.iter().filter(|item| item.intervention.status == InterventionStatus::Recorded).count());
        let weak = cx.weak_entity();
        div().id("inbox").size_full().overflow_y_scroll().child(
            components::page_body(true).mx_auto()
                .child(div().flex().items_start().gap_2()
                    .child(components::page_header(format!("Inbox · {open} open"), Some("Instructions, approvals and questions waiting on you.".into()), cx))
                    .child(div().ml_auto().child(ui::segmented("inbox-filter", &[(false, "Open"), (true, "All")], self.show_resolved,
                        move |value, _, cx| { weak.update(cx, |this, cx| { this.show_resolved = value; cx.notify() }).ok(); }, cx))))
                .children(self.error.clone().map(|error| div().pb_3().t_label().text_color(cx.theme().danger).child(error)))
                .child(body),
        )
    }
}

// ---- Settings ------------------------------------------------------------------------

pub struct Settings {
    state: AppState,
    server: Load<AppSettings>,
    draft: Option<AppSettings>,
    conflict: bool,
    saving: bool,
    error: Option<String>,
    harnesses: Load<Vec<HarnessInfo>>,
    targets: Load<Vec<ExecutionTargetInfo>>,
    generation: u64,
    /// An edit arrived while a save was in flight; save again once it lands.
    resave: bool,
    _load: Option<gpui_kit::Task<()>>,
    _save: Option<gpui_kit::Task<()>>,
    _subscription: Subscription,
}

impl Settings {
    fn new(state: AppState, cx: &mut Context<Self>) -> Self {
        let subscription = watch(&state, cx, Self::reload);
        let mut this = Self {
            state, server: Load::Loading, draft: None, conflict: false, saving: false, error: None,
            harnesses: Load::Loading, targets: Load::Loading, generation: 0, resave: false, _load: None, _save: None, _subscription: subscription,
        };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        self._load = Some(cx.spawn(async move |this, cx| {
            let settings = api.get::<AppSettings>("/v1/settings").await;
            let harnesses = api.get::<Vec<HarnessInfo>>("/v1/harnesses").await;
            let targets = api.get::<Vec<ExecutionTargetInfo>>("/v1/execution-targets").await;
            this.update(cx, |this, cx| {
                if let (Ok(settings), Some(draft)) = (&settings, &this.draft) {
                    this.conflict = settings.revision != draft.revision;
                }
                match settings {
                    Ok(settings) => this.server = Load::Ready(settings),
                    Err(error) if this.server.ready().is_some() => this.error = Some(format!("Couldn't refresh: {}", ui::error_text(&error))),
                    Err(error) => this.server = Load::Failed(ui::error_text(&error)),
                }
                this.harnesses = Load::from_result(harnesses);
                this.targets = Load::from_result(targets);
                cx.notify();
            }).ok();
        }));
    }

    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut AppSettings)) {
        if self.draft.is_none() { self.draft = self.server.ready().cloned() }
        if let Some(draft) = self.draft.as_mut() { f(draft) }
        self.generation += 1;
        // Preferences save as soon as they change, so leaving Settings never loses them.
        if self.saving { self.resave = true } else { self.save(cx) }
        cx.notify();
    }

    /// Explicit "Keep mine": rebase the draft on the latest fetched revision, then save.
    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        if let (Some(draft), Some(server)) = (self.draft.as_mut(), self.server.ready()) { draft.revision = server.revision }
        self.conflict = false;
        self.save(cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.clone() else { return };
        if self.conflict { return }
        let (submitted, base) = (self.generation, draft.revision);
        self.saving = true;
        cx.notify();
        let api = self.state.api.clone();
        self._save = Some(cx.spawn(async move |this, cx| {
            let result = api.put::<_, AppSettings>("/v1/settings", &draft).await;
            let latest = match &result {
                Err(error) if error.is_conflict() => api.get::<AppSettings>("/v1/settings").await.ok(),
                _ => None,
            };
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(settings) if this.generation == submitted => {
                        this.server = Load::Ready(settings);
                        this.draft = None;
                        this.error = None;
                        this.state.data_changed(cx);
                    }
                    Ok(settings) => {
                        if let Some(draft) = this.draft.as_mut() { draft.revision = settings.revision }
                        this.server = Load::Ready(settings);
                        this.state.data_changed(cx);
                        if std::mem::take(&mut this.resave) { this.save(cx) }
                    }
                    Err(error) if error.is_conflict() => match latest {
                        Some(settings) if settings.revision != base => { this.conflict = true; this.server = Load::Ready(settings) }
                        _ => this.error = Some(ui::service_message(&error)),
                    },
                    Err(error) => this.error = Some(ui::error_text(&error)),
                }
                cx.notify();
            }).ok();
        }));
    }

    fn render_preferences(&self, settings: &AppSettings, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let theme = if settings.theme.is_known() {
            theme_tiles(&settings.theme, weak.clone(), window, cx).into_any_element()
        } else {
            ui::chip(format!("{} (unsupported here)", settings.theme.as_str()), cx).into_any_element()
        };
        let harness = if settings.default_harness.is_known() {
            components::button_md(Button::new("default-harness").outline().label(ui::harness_label(&settings.default_harness)).dropdown_caret(true)).dropdown_menu({
                let weak = weak.clone();
                move |mut menu, _, _| {
                    for harness in HarnessId::ALL.iter() {
                        let (weak, harness) = (weak.clone(), harness.clone());
                        menu = menu.item(PopupMenuItem::new(ui::harness_label(&harness)).on_click(move |_, _, cx| {
                            weak.update(cx, |this, cx| this.edit(cx, |s| s.default_harness = harness.clone())).ok();
                        }));
                    }
                    menu
                }
            }).into_any_element()
        } else {
            ui::chip(format!("{} (unsupported here)", settings.default_harness.as_str()), cx).into_any_element()
        };
        let notifications = settings.notifications;
        let onboarding = settings.onboarding_complete;
        let t = theme_tokens::tokens(cx);
        // Appearance card (ref B97FF317): heading + description, then preview tiles.
        div().flex().flex_col()
            .child(components::section_card(cx).py_4().gap_0p5()
                .child(div().t_body().font_weight(FontWeight::MEDIUM).text_color(t.text).child("Theme"))
                .child(div().t_body().text_color(t.text_secondary).child("System follows your device. Light is the default."))
                .child(div().pt_3().child(theme)))
            .child(components::section_label("General", cx))
            .child(components::section_card(cx)
            .child(setting_row("Default harness", "Used for new agents.", harness, cx))
            .child(components::row_divider(cx))
            .child(setting_row("Notifications", "Notify when something needs you.", Switch::new("notifications").checked(notifications)
                .on_click(cx.listener(|this, checked: &bool, _, cx| { let checked = *checked; this.edit(cx, |s| s.notifications = checked) })), cx))
            .child(components::row_divider(cx))
            .child(setting_row("Onboarding", if onboarding { "Completed." } else { "Not completed yet." },
                components::button_row_action(Button::new("reset-onboarding").outline().label(if onboarding { "Show again" } else { "Mark complete" }))
                    .on_click(cx.listener(move |this, _, _, cx| this.edit(cx, |s| s.onboarding_complete = !onboarding))), cx)))
    }

    fn render_harnesses(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let body = match &self.harnesses {
            Load::Loading => ui::skeleton_rows(2, cx).into_any_element(),
            Load::Failed(error) => div().t_label().text_color(cx.theme().danger).child(error.clone()).into_any_element(),
            Load::Ready(list) => div().flex().flex_col().children(list.iter().enumerate().map(|(index, info)| {
                let installed = match info.installed { Some(true) => ui::chip("Installed", cx), Some(false) => ui::amber_chip("Not found", cx), None => ui::chip("Not checked", cx) };
                let auth = match info.authenticated { Some(true) => ui::chip("Signed in", cx), Some(false) => ui::amber_chip("Signed out", cx), None => ui::chip("Auth unknown", cx) };
                // Wide: name then chips on one line. Narrow: the chip group wraps below the
                // name (same chips, wrapping among themselves) instead of crushing it.
                div().flex().flex_wrap().items_center().gap_3().min_h(px(56.)).py_3()
                    .when(index > 0, |this| this.border_t_1().border_color(theme_tokens::tokens(cx).border))
                    .child(Icon::new(IconName::Bot).size(px(16.)).text_color(cx.theme().muted_foreground))
                    .child(div().flex_1().flex_basis(px(180.)).min_w_0().flex().flex_col().gap_0p5()
                        .child(div().t_body().font_weight(FontWeight::MEDIUM).child(info.name.clone()))
                        .child(div().t_caption().truncate().text_color(cx.theme().muted_foreground)
                            .child(join_parts(&[&info.executable, &info.message]))))
                    .child(div().flex().flex_wrap().gap_2().max_w_full()
                        .child(installed).child(auth)
                        .child(if info.execution_available { ui::chip("Execution ready", cx) } else { ui::chip("Execution not available", cx) }))
            })).into_any_element(),
        };
        components::section_card(cx).child(body)
    }

    fn render_targets(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let body = match &self.targets {
            Load::Loading => ui::skeleton_rows(2, cx).into_any_element(),
            Load::Failed(error) => div().t_label().text_color(cx.theme().danger).child(error.clone()).into_any_element(),
            Load::Ready(list) => div().flex().flex_wrap().gap_2().children(ExecutionTarget::ALL.iter().map(|target| {
                let info = list.iter().find(|info| &info.id == target);
                components::section_card(cx).flex_1().flex_basis(px(220.)).min_w(px(200.)).py_4().gap_1()
                    .child(div().flex().items_center().gap_2()
                        .child(div().t_label().font_weight(FontWeight::MEDIUM).child(ui::target_label(target)))
                        .child(div().ml_auto().child(match info {
                            Some(info) if info.available => ui::chip("Available", cx),
                            Some(_) => ui::amber_chip("Unavailable", cx),
                            None => ui::chip("Not reported", cx),
                        })))
                    .child(div().t_caption().text_color(cx.theme().muted_foreground).child(ui::target_detail(target)))
                    .children(info.map(|info| div().flex().flex_col().gap_0p5().t_caption().text_color(cx.theme().muted_foreground)
                        .when(!info.requirements.is_empty(), |this| this.child(format!("Requires: {}", info.requirements)))
                        .when(!info.message.is_empty(), |this| this.child(info.message.clone()))))
            })).into_any_element(),
        };
        div().flex().flex_col().gap_2()
            .child(body)
            .child(div().px_4().t_caption().text_color(cx.theme().muted_foreground).child("Each project picks its environment under Project → Settings."))
    }
}

/// Theme preview tiles (ref B97FF317): miniature window per choice, black 2pt outline on
/// the selected tile, caption below. Same three choices and autosave as before.
fn theme_tiles(selected: &Theme, weak: WeakEntity<Settings>, window: &mut Window, cx: &App) -> Div {
    let t = theme_tokens::tokens(cx);
    let scale = window.scale_factor();
    // Three tiles share the card row evenly at 156:98 (B97FF317), within 120–260 wide.
    // Row aligns to the top so no tile stretches; each column only shares width.
    // Wraps on narrow cards (min 120 each) so the row never exceeds the section.
    div().flex().flex_wrap().items_start().gap_3().children([(Theme::System, "System"), (Theme::Light, "Light"), (Theme::Dark, "Dark")].into_iter().map(|(choice, label)| {
        let active = &choice == selected;
        let weak = weak.clone();
        let image = Arc::new(Image::from_bytes(ImageFormat::Svg, theme_tile_svg(&choice, scale).into_bytes()));
        div().id(label).flex_1().min_w(px(TILE_MIN_W)).max_w(px(TILE_MAX_W)).flex().flex_col().items_stretch().gap_2().cursor_pointer()
            .on_click(move |_, _, cx| { let choice = choice.clone(); weak.update(cx, |this, cx| this.edit(cx, |s| s.theme = choice)).ok(); })
            // Ring hugs the preview: 2 border + 2 gap, outer radius = 12 + 2 + 2 (concentric).
            .child(div().flex_none().w_full().p(px(RING_GAP)).rounded(px(TILE_R + RING_GAP + RING_W)).border(px(RING_W))
                .border_color(if active { t.accent_fill } else { transparent_black() })
                // The well owns the 156:98 geometry; the image fills it absolutely so its
                // intrinsic (DPR-scaled) size never drives layout. GPUI rounds the image
                // itself, so the corners hold at any tile width (SVG-unit radii scale).
                .child(div().relative().flex_none().w_full().aspect_ratio(TILE_W / TILE_H).min_h_0().rounded(px(TILE_R))
                    .child(img(image).absolute().inset_0().size_full().rounded(px(TILE_R)))
                    .when(!active, |well| well.child(div().absolute().inset_0().rounded(px(TILE_R)).border_1().border_color(t.border)))))
            .child(div().px_1().t_body().when(active, |this| this.font_weight(FontWeight::MEDIUM)).text_color(if active { t.text } else { t.text_secondary }).child(label))
    }))
}

const TILE_W: f32 = 156.;
const TILE_H: f32 = 98.;
const TILE_R: f32 = 12.;
const TILE_MIN_W: f32 = 120.;
const RING_W: f32 = 2.;
const RING_GAP: f32 = 2.;
const TILE_MAX_W: f32 = 260.;

/// SVG for one preview tile: sidebar with three lines, a pill, two text lines and a composer
/// bar with an accent dot. System splits light and dark along the diagonal. Square: the
/// caller rounds the image and draws the outline.
fn theme_tile_svg(choice: &Theme, scale: f32) -> String {
    const LIGHT: [&str; 6] = ["#F8F9FB", "#EEEFF1", "#E1E2E5", "#E3ECFA", "#ECEDEF", "#3B82F6"];
    const DARK: [&str; 6] = ["#0C0D0F", "#17181B", "#2C2D31", "#1D2433", "#1B1C1F", "#60A5FA"];
    let scene = |[bg, side, line, pill, bar, dot]: [&str; 6]| format!(
        r#"<rect width="156" height="98" fill="{bg}"/><rect width="40" height="98" fill="{side}"/><rect x="7" y="13" width="20" height="4" rx="2" fill="{line}"/><rect x="7" y="21" width="13" height="4" rx="2" fill="{line}"/><rect x="7" y="29" width="17" height="4" rx="2" fill="{line}"/><rect x="98" y="37" width="50" height="11" rx="5.5" fill="{pill}"/><rect x="48" y="55" width="75" height="4" rx="2" fill="{line}"/><rect x="48" y="65" width="50" height="4" rx="2" fill="{line}"/><rect x="47" y="76" width="101" height="13" rx="6.5" fill="{bar}" stroke="{line}"/><circle cx="141" cy="82.5" r="3.5" fill="{dot}"/>"#);
    let body = match choice {
        Theme::Dark => scene(DARK),
        Theme::Light => scene(LIGHT),
        _ => format!(r#"{}<g clip-path="url(#half)">{}</g>"#, scene(LIGHT), scene(DARK)),
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 156 98"><defs><clipPath id="half"><polygon points="156,0 156,98 0,98"/></clipPath></defs>{body}</svg>"#,
        // Rasterised at the largest tile size so stretching never upsamples.
        w = TILE_MAX_W * scale,
        h = TILE_MAX_W * TILE_H / TILE_W * scale,
    )
}

fn setting_row(title: &'static str, detail: &'static str, control: impl IntoElement, cx: &App) -> impl IntoElement {
    components::settings_row(title, Some(detail.into()), control, cx)
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.draft.clone().or_else(|| self.server.ready().cloned());
        let preferences = match (&self.server, settings) {
            (Load::Loading, _) => ui::skeleton_rows(3, cx).into_any_element(),
            (Load::Failed(error), _) => {
                let weak = cx.weak_entity();
                ui::error_panel("Couldn't load settings", error, move |_, _, cx| { weak.update(cx, |this, cx| this.reload(cx)).ok(); }, cx).into_any_element()
            }
            (_, Some(settings)) => self.render_preferences(&settings, window, cx).into_any_element(),
            _ => div().into_any_element(),
        };
        let dirty = self.draft.is_some();
        div().size_full().flex().flex_col()
            .child(div().id("settings").flex_1().min_h_0().overflow_y_scroll().child(
                components::page_body(true).mx_auto()
                    .child(components::page_header("Settings", Some("Preferences, harnesses and execution environments.".into()), cx).pb_0())
                    .child(components::section_label("Appearance", cx)).child(preferences)
                    .child(components::section_label("Harnesses", cx)).child(self.render_harnesses(cx))
                    .child(components::section_label("Execution environments", cx)).child(self.render_targets(cx)),
            ))
            .when((dirty && (self.error.is_some() || !self.saving && self.resave)) || self.conflict, |this| {
                let (discard, save) = (cx.weak_entity(), cx.weak_entity());
                this.child(ui::save_bar("settings-save", self.conflict, self.saving, self.error.as_deref(),
                    move |_, _, cx| { discard.update(cx, |this, cx| { this.draft = None; this.conflict = false; this.reload(cx) }).ok(); },
                    move |_, _, cx| { save.update(cx, |this, cx| if this.conflict { this.keep_mine(cx) } else { this.save(cx) }).ok(); },
                    cx))
            })
    }
}
