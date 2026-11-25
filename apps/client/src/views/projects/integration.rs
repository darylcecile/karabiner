//! Project → Settings → Sync: the GitHub / Linear connection stored in
//! `ProjectConfig.integration` (shared config draft, saved with the project's Save bar), plus
//! the service actions: check the connection, load names for status ids (catalog), and sync
//! now. Credentials are never entered here: GitHub uses `gh` auth on the service host and
//! Linear names an environment variable the service reads.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{
    IntegrationCatalog, IntegrationConfig, IntegrationInfo, IntegrationRefresh, IntegrationSyncResult, ProjectKind, TaskStatus,
};
use serde_json::{Map, Value};

use crate::{
    api::query_value,
    components::{self, Typography as _},
    state::AppState,
    theme,
    views::{tasks::ui, workflow::config::ConfigDraft},
};

const DEFAULT_INTERVAL_MIN: u64 = 5;

/// What the service last told us for one action.
enum Outcome<T> {
    Idle,
    Busy,
    Done(T),
    Failed(String),
}

pub struct IntegrationPanel {
    state: AppState,
    project_id: String,
    config: Entity<ConfigDraft>,
    kind: ProjectKind,
    hostname: Entity<InputState>,
    api_key_env: Entity<InputState>,
    interval: Entity<InputState>,
    syncing_inputs: bool,
    info: Outcome<IntegrationInfo>,
    catalog: Outcome<IntegrationCatalog>,
    sync: Outcome<IntegrationSyncResult>,
    /// General → Source (kind, repository, project/team) has unsaved changes; the service
    /// would act on the old source, so actions wait for Save.
    source_dirty: bool,
    /// Narrow settings pane: controls under their labels.
    stacked: bool,
    _tasks: Vec<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn default_config(kind: &ProjectKind) -> Option<IntegrationConfig> {
    let (provider, scope, api_key_env) = match kind {
        ProjectKind::Github => ("github", "repository", None),
        ProjectKind::Linear => ("linear", "team", Some("LINEAR_API_KEY".to_string())),
        _ => return None,
    };
    Some(IntegrationConfig {
        provider: provider.into(), scope: scope.into(), hostname: None, api_key_env, team_id: None, push_enabled: None,
        status_field_id: None, pull_status_map: None, push_status_map: None, push_fields: None, refresh: None, extra: Default::default(),
    })
}

impl IntegrationPanel {
    pub fn new(state: AppState, project_id: String, config: Entity<ConfigDraft>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let hostname = input("github.com", window, cx);
        let api_key_env = input("LINEAR_API_KEY", window, cx);
        let interval = input("5", window, cx);
        let mut subscriptions = vec![cx.observe_in(&config, window, |this, _, window, cx| { this.sync_inputs(window, cx); cx.notify() })];
        for (input, field) in [(&hostname, Field::Hostname), (&api_key_env, Field::ApiKeyEnv), (&interval, Field::Interval)] {
            subscriptions.push(cx.subscribe_in(input, window, move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && !this.syncing_inputs {
                    let text = input.read(cx).value().trim().to_string();
                    this.apply(field, text, cx);
                }
            }));
        }
        let mut this = Self {
            state, project_id, config, kind: ProjectKind::Standalone, hostname, api_key_env, interval, syncing_inputs: false,
            info: Outcome::Idle, catalog: Outcome::Idle, sync: Outcome::Idle, source_dirty: false, stacked: false, _tasks: Vec::new(), _subscriptions: subscriptions,
        };
        this.sync_inputs(window, cx);
        this
    }

    pub fn set_stacked(&mut self, stacked: bool) {
        self.stacked = stacked;
    }

    pub fn set_source_dirty(&mut self, dirty: bool) {
        self.source_dirty = dirty;
    }

    /// Check, Load statuses and Sync use the saved connection and source.
    fn unsaved(&self, cx: &App) -> bool {
        self.source_dirty || self.config.read(cx).dirty
    }

    pub fn set_kind(&mut self, kind: ProjectKind, cx: &mut Context<Self>) {
        if self.kind != kind { self.kind = kind; self.info = Outcome::Idle; self.catalog = Outcome::Idle; cx.notify() }
    }

    /// Saved/draft settings, or this provider's defaults when none are stored (or they belong to
    /// another provider after the source changed).
    fn current(&self, cx: &App) -> Option<IntegrationConfig> {
        let default = default_config(&self.kind)?;
        let stored = self.config.read(cx).config().and_then(|config| config.integration.clone());
        Some(stored.filter(|stored| stored.provider == default.provider).unwrap_or(default))
    }

    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut IntegrationConfig)) {
        let Some(mut integration) = self.current(cx) else { return };
        f(&mut integration);
        self.config.update(cx, |config, cx| config.edit(cx, |config| config.integration = Some(integration)));
    }

    fn apply(&mut self, field: Field, text: String, cx: &mut Context<Self>) {
        let value = (!text.is_empty()).then_some(text);
        let group = format!("integration:{field:?}");
        let Some(mut integration) = self.current(cx) else { return };
        match field {
            Field::Hostname => integration.hostname = value,
            Field::ApiKeyEnv => integration.api_key_env = Some(value.unwrap_or_default()),
            Field::Interval => {
                let minutes = value.and_then(|text| text.parse::<u64>().ok());
                let refresh = integration.refresh.get_or_insert_with(IntegrationRefresh::default);
                refresh.interval_ms = minutes.map(|minutes| minutes.clamp(1, 24 * 60) * 60_000);
            }
        }
        self.config.update(cx, |config, cx| config.edit_grouped(group, cx, |config| config.integration = Some(integration)));
    }

    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(integration) = self.current(cx) else { return };
        let minutes = integration.refresh.as_ref().and_then(|refresh| refresh.interval_ms).map(|ms| (ms / 60_000).to_string()).unwrap_or_default();
        self.syncing_inputs = true;
        for (input, value) in [
            (&self.hostname, integration.hostname.unwrap_or_default()),
            (&self.api_key_env, integration.api_key_env.unwrap_or_default()),
            (&self.interval, minutes),
        ] {
            if input.read(cx).value().trim() != value { input.update(cx, |input, cx| input.set_value(value, window, cx)) }
        }
        self.syncing_inputs = false;
    }

    // ---- Service actions -------------------------------------------------------------

    fn path(&self, tail: &str) -> String {
        format!("/v1/projects/{}/{tail}", query_value(&self.project_id))
    }

    fn check(&mut self, cx: &mut Context<Self>) {
        self.info = Outcome::Busy;
        cx.notify();
        let (api, path) = (self.state.api.clone(), self.path("integration?check=true"));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = api.get::<IntegrationInfo>(&path).await;
            this.update(cx, |this, cx| {
                this.info = match result { Ok(info) => Outcome::Done(info), Err(error) => Outcome::Failed(ui::service_message(&error)) };
                cx.notify();
            }).ok();
        }));
    }

    fn load_catalog(&mut self, cx: &mut Context<Self>) {
        self.catalog = Outcome::Busy;
        cx.notify();
        let (api, path) = (self.state.api.clone(), self.path("integration/catalog"));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = api.get::<IntegrationCatalog>(&path).await;
            this.update(cx, |this, cx| {
                this.catalog = match result { Ok(catalog) => Outcome::Done(catalog), Err(error) => Outcome::Failed(ui::service_message(&error)) };
                cx.notify();
            }).ok();
        }));
    }

    fn sync_now(&mut self, cx: &mut Context<Self>) {
        self.sync = Outcome::Busy;
        cx.notify();
        let (api, path) = (self.state.api.clone(), self.path("sync"));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = api.post::<_, IntegrationSyncResult>(&path, &serde_json::json!({})).await;
            this.update(cx, |this, cx| {
                this.sync = match result {
                    Ok(result) => { this.state.data_changed(cx); Outcome::Done(result) }
                    Err(error) => Outcome::Failed(ui::service_message(&error)),
                };
                cx.notify();
            }).ok();
        }));
    }
}

#[derive(Debug, Clone, Copy)]
enum Field {
    Hostname,
    ApiKeyEnv,
    Interval,
}

// ---- Status maps -------------------------------------------------------------------------

/// Push targets for the configured provider: GitHub status-field options, or the chosen
/// Linear team's workflow states. `(id, name)`.
/// Repository scope maps the issue state (a fixed catalog entry); project scope maps the
/// chosen Projects status field, which is the only use of `statusFieldId`.
const ISSUE_STATE: &str = "issue-state";

fn github_field<'a>(integration: &IntegrationConfig, catalog: &'a IntegrationCatalog) -> Option<&'a protocol::CatalogField> {
    let id = if integration.scope == "repository" { Some(ISSUE_STATE) } else { integration.status_field_id.as_deref() };
    catalog.status_fields.iter().find(|field| Some(field.id.as_str()) == id)
}

fn push_options(integration: &IntegrationConfig, catalog: &IntegrationCatalog) -> Vec<(String, String)> {
    let options = if integration.provider == "github" {
        github_field(integration, catalog).map(|field| &field.options)
    } else {
        linear_team(integration, catalog).map(|team| &team.states)
    };
    options.map_or_else(Vec::new, |options| options.iter().map(|option| (option.id.clone(), option.name.clone())).collect())
}

fn linear_team<'a>(integration: &IntegrationConfig, catalog: &'a IntegrationCatalog) -> Option<&'a protocol::CatalogTeam> {
    catalog.teams.iter().find(|team| Some(&team.id) == integration.team_id.as_ref()).or_else(|| catalog.teams.first())
}

/// Current push option id for a task status.
fn push_value(integration: &IntegrationConfig, team: Option<&str>, status: &TaskStatus) -> Option<String> {
    let map = integration.push_status_map.as_ref()?;
    let map = if integration.provider == "github" { map } else { map.get(team?)? };
    map.get(status.as_str())?.as_str().map(str::to_string)
}

fn set_push_value(integration: &mut IntegrationConfig, team: Option<&str>, status: &TaskStatus, option: Option<String>) {
    let root = integration.push_status_map.get_or_insert_with(|| Value::Object(Map::new()));
    let Some(root) = root.as_object_mut() else { return };
    let map = if integration.provider == "github" {
        root
    } else {
        let Some(team) = team else { return };
        let Some(map) = root.entry(team.to_string()).or_insert_with(|| Value::Object(Map::new())).as_object_mut() else { return };
        map
    };
    match option {
        Some(option) => { map.insert(status.as_str().into(), Value::String(option)); }
        None => { map.remove(status.as_str()); }
    }
}

// ---- Rendering ---------------------------------------------------------------------------

impl IntegrationPanel {
    fn render_status(&self, integration: &IntegrationConfig, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let github = integration.provider == "github";
        let (headline, detail): (AnyElement, Option<String>) = match &self.info {
            Outcome::Idle => (ui::chip("Not checked", cx).into_any_element(), None),
            Outcome::Busy => (ui::chip("Checking…", cx).into_any_element(), None),
            Outcome::Failed(error) => (ui::amber_chip("Couldn't check", cx).into_any_element(), Some(error.clone())),
            Outcome::Done(info) => {
                let chip = match (info.available, info.authenticated) {
                    (true, Some(true)) | (true, None) => components::status_pill("Connected", components::Tone::Success, cx).into_any_element(),
                    (_, Some(false)) => ui::amber_chip("Not signed in", cx).into_any_element(),
                    _ => ui::amber_chip("Unavailable", cx).into_any_element(),
                };
                // Actionable next step; the service message names the variable, never a value.
                let fix = match (info.code.as_deref(), info.authenticated, github) {
                    (Some("credential_env_missing" | "credential_missing"), _, _) => " Set it in the environment the service starts with, restart the service, then check again.",
                    (Some("source_not_configured"), _, true) => " Set the repository or GitHub Project URL under General → Source.",
                    (Some("source_not_configured"), _, false) => " Set the Linear team key or project under General → Source.",
                    (Some("executable_missing"), _, _) => " Install the GitHub CLI (`gh`), run `gh auth login`, then check again.",
                    (_, Some(false), true) => " Run `gh auth login` on this Mac, then check again.",
                    (_, Some(false), false) => " Set the API key variable in the service's environment and restart the service.",
                    _ => "",
                };
                (chip, Some(format!("{}{fix}", info.message)))
            }
        };
        let refresh = match &self.info { Outcome::Done(info) => info.refresh.clone(), _ => None };
        div().flex().flex_col().gap_2()
            .child(div().flex().items_center().gap_2()
                .child(div().flex_1().t_body().font_weight(FontWeight::MEDIUM).text_color(t.text).child(if github { "GitHub" } else { "Linear" }))
                .child(headline)
                .child(components::button_sm(Button::new("integration-check").outline().label("Check connection"))
                    .disabled(matches!(self.info, Outcome::Busy) || self.unsaved(cx))
                    .on_click(cx.listener(|this, _, _, cx| this.check(cx)))))
            .children(detail.map(|detail| div().t_body().text_color(t.text_secondary).child(detail)))
            .children(refresh.filter(|refresh| refresh.enabled).map(|refresh| div().t_caption().text_color(t.text_secondary).child(
                match (refresh.running, refresh.error, refresh.next_sync_at) {
                    (true, _, _) => "Automatic refresh running now.".to_string(),
                    (_, Some(error), _) => format!("Last automatic refresh failed: {error}"),
                    (_, _, Some(next)) => format!("Next automatic refresh {}.", ui::short_date(&next)),
                    _ => "Automatic refresh on.".to_string(),
                })))
    }

    fn render_settings(&self, integration: &IntegrationConfig, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let github = integration.provider == "github";
        let scopes: &[(&str, &str)] = if github { &[("repository", "Repository issues"), ("project", "GitHub Project")] } else { &[("team", "Team"), ("project", "Project")] };
        let scopes: Vec<(String, &str)> = scopes.iter().map(|(value, label)| (value.to_string(), *label)).collect();
        let push = integration.push_enabled.unwrap_or(false);
        let refresh = integration.refresh.as_ref().is_some_and(|refresh| refresh.enabled);
        // An older service can't refresh on a timer; an already saved setting stays visible.
        let refresh_supported = self.state.service.read(cx).capabilities().integration_refresh || refresh;
        let t = theme::tokens(cx);
        let stacked = self.stacked;
        let row = |title: &'static str, detail: &'static str, control: AnyElement, cx: &App| components::form_row(title, Some(detail.into()), control, stacked, cx);
        components::form_rows(cx)
            .child(row("Sync", if github { "Issues from the repository, or items from a GitHub Project (set its URL under General)." } else { "Issues from a Linear team or project (set its key under General)." },
                ui::segmented("integration-scope", &scopes.iter().map(|(value, label)| (value.clone(), *label)).collect::<Vec<_>>(), integration.scope.clone(),
                    move |scope, _, cx| { weak.update(cx, |this, cx| this.edit(cx, |i| i.scope = scope)).ok(); }, cx).into_any_element(), cx))
            .child(if github {
                row("GitHub host", "Leave empty for github.com. Uses the `gh` sign-in on this Mac.", div().w_full().child(components::input_md(Input::new(&self.hostname))).into_any_element(), cx)
            } else {
                row("API key variable", "Name of the environment variable the service reads. The key itself is never stored.", div().w_full().child(components::input_md(Input::new(&self.api_key_env))).into_any_element(), cx)
            })
            .child(row("Allow push", "Lets you push a task's changes back with its Push button. Nothing is pushed automatically.",
                Switch::new("integration-push").checked(push).on_click(cx.listener(|this, checked: &bool, _, cx| { let checked = *checked; this.edit(cx, |i| i.push_enabled = Some(checked)) })).into_any_element(), cx))
            .when(push, |this| this.child(components::form_block(self.render_push_fields(integration, cx), cx)))
            .when(!refresh_supported, |this| this.child(components::form_block(ui::update_service_note("refresh from the source automatically", cx), cx)))
            .when(refresh_supported, |this| this.child(row("Refresh automatically", "Pulls changes on a timer; local edits are kept.",
                Switch::new("integration-refresh").checked(refresh).on_click(cx.listener(|this, checked: &bool, _, cx| {
                    let checked = *checked;
                    this.edit(cx, |i| i.refresh.get_or_insert_with(IntegrationRefresh::default).enabled = checked)
                })).into_any_element(), cx)))
            .when(refresh, |this| this
                .child(row("Every (minutes)", "Between 1 and 1440.", div().w_full().child(components::input_md(Input::new(&self.interval))).into_any_element(), cx))
                .child(div().py_2().t_caption().text_color(t.text_secondary).child(format!("Default {DEFAULT_INTERVAL_MIN} minutes."))))
    }

    /// Which task fields Push sends. Only fields the source accepts are offered once its
    /// catalog is loaded (e.g. pull requests in a Project take status only).
    fn render_push_fields(&self, integration: &IntegrationConfig, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        use crate::views::tasks::source::{DEFAULT_PUSH_FIELDS, field_label};
        const ALL: [&str; 6] = ["title", "description", "status", "priority", "labels", "dueAt"];
        let chosen: Vec<String> = integration.push_fields.clone().unwrap_or_else(|| DEFAULT_PUSH_FIELDS.iter().map(|field| field.to_string()).collect());
        let writable = match &self.catalog { Outcome::Done(catalog) => Some(catalog.writable_fields.clone()), _ => None };
        let offered: Vec<&str> = ALL.iter().copied().filter(|field| writable.as_ref().is_none_or(|writable| writable.iter().any(|w| w == field))).collect();
        let t = theme::tokens(cx);
        div().py_3().flex().flex_col().gap_1()
            .child(div().t_body().font_weight(FontWeight::MEDIUM).text_color(t.text).child("Fields to push"))
            .child(div().t_body().text_color(t.text_secondary).child(match &writable {
                None => "Load statuses to see exactly which fields this source accepts.".to_string(),
                Some(writable) if writable.is_empty() => "This source accepts no pushed fields.".to_string(),
                Some(_) => "Push sends only these fields. Status uses the mapping below.".to_string(),
            }))
            .children(offered.into_iter().map(|field| {
                let on = chosen.iter().any(|item| item == field);
                let value = field.to_string();
                crate::views::workflow::editor::toggle_row(SharedString::from(format!("push-field-{field}")), on, ui::capitalize(field_label(field)), None, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let value = value.clone();
                        this.edit(cx, |i| {
                            let fields = i.push_fields.get_or_insert_with(|| DEFAULT_PUSH_FIELDS.iter().map(|field| field.to_string()).collect());
                            match fields.iter().position(|item| *item == value) { Some(index) => { fields.remove(index); } None => fields.push(value) }
                        })
                    }))
            }))
            .when(chosen.is_empty(), |this| this.child(div().t_caption().text_color(t.danger).child("Choose at least one field, or Push stays disabled.")))
    }

    fn render_mapping(&self, integration: &IntegrationConfig, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let supported = self.state.service.read(cx).capabilities().integration_catalog;
        let body = match &self.catalog {
            Outcome::Idle if !supported => ui::update_service_note("load the source's statuses for mapping", cx).into_any_element(),
            Outcome::Idle => div().t_body().text_color(t.text_secondary).child("Load the source's statuses to map them to task statuses.").into_any_element(),
            Outcome::Busy => ui::skeleton_rows(2, cx).into_any_element(),
            Outcome::Failed(error) => div().t_body().text_color(cx.theme().danger).child(error.clone()).into_any_element(),
            Outcome::Done(catalog) => self.render_maps(integration, catalog, cx).into_any_element(),
        };
        div().flex().flex_col().gap_2()
            .child(div().flex().items_center()
                .child(div().flex_1().min_w_0().t_body().text_color(t.text_secondary).child("Statuses come from the saved source."))
                .child(components::button_sm(Button::new("integration-catalog").ghost().icon(IconName::Redo2).label("Load statuses"))
                    .disabled(matches!(self.catalog, Outcome::Busy) || self.unsaved(cx) || !supported)
                    .on_click(cx.listener(|this, _, _, cx| this.load_catalog(cx)))))
            .child(body)
    }

    fn render_maps(&self, integration: &IntegrationConfig, catalog: &IntegrationCatalog, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let github = integration.provider == "github";
        let weak = cx.weak_entity();
        // Which list the maps use: a GitHub Project status field, or a Linear team.
        let project_fields: Vec<&protocol::CatalogField> = catalog.status_fields.iter().filter(|field| field.id != ISSUE_STATE).collect();
        let picker: Option<AnyElement> = if github {
            (integration.scope == "project" && !project_fields.is_empty()).then(|| {
                let current = catalog.status_fields.iter().find(|field| Some(&field.id) == integration.status_field_id.as_ref()).map_or("Choose field".to_string(), |field| field.name.clone());
                let fields: Vec<(String, String)> = project_fields.iter().map(|field| (field.id.clone(), field.name.clone())).collect();
                let weak = weak.clone();
                dropdown("integration-field", current, fields, move |id, cx| { weak.update(cx, |this, cx| this.edit(cx, |i| i.status_field_id = Some(id))).ok(); }).into_any_element()
            })
        } else {
            (!catalog.teams.is_empty()).then(|| {
                let current = linear_team(integration, catalog).map_or("Choose team".to_string(), |team| team.name.clone());
                let teams: Vec<(String, String)> = catalog.teams.iter().map(|team| (team.id.clone(), team.name.clone())).collect();
                let weak = weak.clone();
                dropdown("integration-team", current, teams, move |id, cx| { weak.update(cx, |this, cx| this.edit(cx, |i| i.team_id = Some(id))).ok(); }).into_any_element()
            })
        };
        let pulls: Vec<protocol::CatalogOption> = if github {
            github_field(integration, catalog).map_or_else(Vec::new, |field| field.options.clone())
        } else {
            linear_team(integration, catalog).map_or_else(Vec::new, |team| team.states.clone())
        };
        let team = (!github).then(|| linear_team(integration, catalog).map(|team| team.id.clone())).flatten();
        let options = push_options(integration, catalog);
        let push = integration.push_enabled.unwrap_or(false);
        div().flex().flex_col().gap_3()
            .children(picker.map(|picker| div().flex().items_center().gap_2()
                .child(div().w(px(160.)).t_label().text_color(t.text_secondary).child(if github { "Status field" } else { "Team" })).child(picker)))
            .when(github && pulls.is_empty(), |this| this.child(div().t_caption().text_color(t.text_secondary)
                .child(if integration.scope == "project" { "Choose the Project's status field to map its options." } else { "This source reported no issue states." })))
            .when(!pulls.is_empty(), |this| this.child(div().pt_2().t_caption().font_weight(FontWeight::SEMIBOLD).text_color(t.text_secondary).child("Incoming"))
                .children(pulls.iter().map(|option| {
                    let (weak, id) = (weak.clone(), option.id.clone());
                    // "Default" removes the entry, so the service's own mapping applies again.
                    let mut statuses: Vec<(String, String)> = vec![(String::new(), "Default".to_string())];
                    statuses.extend(ui::BOARD_STATUSES.iter().map(|status| (status.as_str().to_string(), ui::status_label(status).to_string())));
                    let mapped = integration.pull_status_map.as_ref().and_then(|map| map.get(&option.id)).cloned();
                    div().flex().items_center().gap_2()
                        .child(div().w(px(160.)).truncate().t_label().child(option.name.clone()))
                        .child(dropdown(SharedString::from(format!("pull-{}", option.id)),
                            mapped.as_ref().map_or("Default".to_string(), |status| ui::status_label(status).to_string()), statuses,
                            move |status, cx| {
                                let id = id.clone();
                                weak.update(cx, |this, cx| this.edit(cx, |i| {
                                    if status.is_empty() {
                                        if let Some(map) = i.pull_status_map.as_mut() { map.remove(&id); }
                                        if i.pull_status_map.as_ref().is_some_and(|map| map.is_empty()) { i.pull_status_map = None }
                                    } else {
                                        i.pull_status_map.get_or_insert_with(Default::default).insert(id, TaskStatus::from(status.as_str()));
                                    }
                                })).ok();
                            }))
                })))
            .when(push && !options.is_empty(), |this| this.child(div().pt_2().t_caption().font_weight(FontWeight::SEMIBOLD).text_color(t.text_secondary).child("When pushing"))
                .children(ui::BOARD_STATUSES.iter().map(|status| {
                    let current = push_value(integration, team.as_deref(), status)
                        .and_then(|id| options.iter().find(|(option, _)| *option == id).map(|(_, name)| name.clone()));
                    let (weak, status_value, team) = (weak.clone(), status.clone(), team.clone());
                    let mut choices = vec![(String::new(), "Don't change".to_string())];
                    choices.extend(options.iter().cloned());
                    div().flex().items_center().gap_2()
                        .child(div().w(px(160.)).t_label().child(ui::status_label(status)))
                        .child(dropdown(SharedString::from(format!("push-{}", status.as_str())), current.unwrap_or_else(|| "Don't change".into()), choices,
                            move |option, cx| {
                                let (status, team) = (status_value.clone(), team.clone());
                                weak.update(cx, |this, cx| this.edit(cx, |i| set_push_value(i, team.as_deref(), &status, (!option.is_empty()).then_some(option)))).ok();
                            }))
                })))
            .when(push && github && integration.scope == "repository", |this| this.child(div().t_caption().text_color(t.text_secondary)
                .child("Pushing a status opens or closes the issue (as completed or not planned).")))
    }

    fn render_sync(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let result = match &self.sync {
            Outcome::Idle => None,
            Outcome::Busy => Some("Syncing…".to_string()),
            Outcome::Failed(error) => Some(error.clone()),
            Outcome::Done(result) => Some(format!("{} imported · {} updated · {} unchanged{}{}", result.imported, result.updated, result.skipped,
                if result.conflicts > 0 { format!(" · {} kept local (changed on both sides)", result.conflicts) } else { String::new() },
                result.message.as_ref().map_or(String::new(), |message| format!(". {message}")))),
        };
        let errors = match &self.sync { Outcome::Done(result) => result.errors.iter().map(|error| error.message.clone()).collect(), _ => Vec::new() };
        let failed = matches!(self.sync, Outcome::Failed(_));
        div().flex().flex_col().gap_1()
            .child(div().flex().items_center().gap_2()
                .child(div().flex_1().t_body().text_color(if failed { cx.theme().danger } else { t.text_secondary })
                    .child(result.unwrap_or_else(|| "Sync uses the saved settings.".into())))
                .child(components::button_sm(Button::new("integration-sync").primary().label("Sync now"))
                    .disabled(matches!(self.sync, Outcome::Busy) || self.unsaved(cx))
                    .on_click(cx.listener(|this, _, _, cx| this.sync_now(cx)))))
            .children(errors.into_iter().map(|error: String| div().t_caption().text_color(cx.theme().danger).child(error)))
    }
}

fn dropdown(id: impl Into<SharedString>, label: String, items: Vec<(String, String)>, on_pick: impl Fn(String, &mut App) + 'static) -> impl IntoElement {
    let on_pick = std::rc::Rc::new(on_pick);
    components::button_sm(Button::new(id.into()).outline().label(label).dropdown_caret(true)).dropdown_menu(move |mut menu, _, _| {
        for (value, name) in items.iter() {
            let (on_pick, value) = (on_pick.clone(), value.clone());
            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| on_pick(value.clone(), cx)));
        }
        menu
    })
}

impl Render for IntegrationPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(integration) = self.current(cx) else {
            return div().t_body().text_color(theme::tokens(cx).text_secondary)
                .child("This is a local project. Choose GitHub or Linear under General → Source to sync tasks.").into_any_element();
        };
        let unsaved = self.unsaved(cx);
        // Plain sections with hairline rows; each control keeps its own (only) border.
        let section = |title: &'static str, cx: &App| components::form_section(title, None, cx);
        div().flex().flex_col()
            .when(unsaved, |this| this.child(div().pb_2().t_body().text_color(theme::tokens(cx).text_secondary)
                .child("Save your changes to check, load statuses or sync. These use the saved source and settings.")))
            .child(section("Connection", cx).pt_0())
            .child(components::form_rows(cx).child(components::form_block(self.render_status(&integration, cx), cx)))
            .child(section("Settings", cx))
            .child(self.render_settings(&integration, cx))
            .child(section("Status mapping", cx))
            .child(components::form_rows(cx).child(components::form_block(self.render_mapping(&integration, cx), cx)))
            .child(section("Sync", cx))
            .child(components::form_rows(cx).child(components::form_block(self.render_sync(cx), cx)))
            .into_any_element()
    }
}
