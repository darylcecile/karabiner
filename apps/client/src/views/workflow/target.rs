//! Options for the project's execution target (`ProjectConfig.targetOptions`), shown for the
//! selected target only, plus a readiness check of the draft. Host needs no options; MXC
//! takes its executable, profile, path access, network and keychain; Apple container takes
//! the image, guest command and resources. Lists are one entry per line. Environment
//! variables are passed by name only.

use std::collections::HashMap;

use gpui_kit::component::{
    Disableable as _,
    button::Button,
    input::{InputEvent, TextareaState, InputState, Input},
    switch::Switch,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{CheckTarget, ExecutionTarget, ProjectConfig, TargetOptions, TargetReadiness};

use super::config::ConfigDraft;
use crate::{
    api::query_value,
    components::{self, Tone, Typography as _},
    state::AppState,
    theme,
    views::tasks::ui,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Field {
    Executable,
    ProfilePath,
    ReadonlyPaths,
    WritablePaths,
    Image,
    GuestExecutable,
    GuestWorkdir,
    EnvNames,
    Cpus,
    Memory,
}

impl Field {
    const LINES: [Field; 3] = [Field::ReadonlyPaths, Field::WritablePaths, Field::EnvNames];
    const SINGLE: [Field; 7] = [Field::Executable, Field::ProfilePath, Field::Image, Field::GuestExecutable, Field::GuestWorkdir, Field::Cpus, Field::Memory];

    fn placeholder(self) -> &'static str {
        match self {
            Field::Executable => "mxc (from PATH)",
            Field::ProfilePath => "Optional custom profile",
            Field::ReadonlyPaths => "One path per line",
            Field::WritablePaths => "One path per line (the checkout is writable)",
            Field::Image => "e.g. ghcr.io/org/agent-image:latest",
            Field::GuestExecutable => "Harness command inside the container",
            Field::GuestWorkdir => "/workspace",
            Field::EnvNames => "One variable name per line, e.g. ANTHROPIC_API_KEY",
            Field::Cpus => "2",
            Field::Memory => "4G",
        }
    }

    fn value(self, options: &TargetOptions) -> String {
        let lines = |items: &Option<Vec<String>>| items.as_ref().map_or_else(String::new, |items| items.join("\n"));
        match self {
            Field::Executable => options.executable.clone().unwrap_or_default(),
            Field::ProfilePath => options.profile_path.clone().unwrap_or_default(),
            Field::ReadonlyPaths => lines(&options.readonly_paths),
            Field::WritablePaths => lines(&options.writable_paths),
            Field::Image => options.image.clone().unwrap_or_default(),
            Field::GuestExecutable => options.guest_executable.clone().unwrap_or_default(),
            Field::GuestWorkdir => options.guest_workdir.clone().unwrap_or_default(),
            Field::EnvNames => lines(&options.env_names),
            Field::Cpus => options.cpus.map_or_else(String::new, |cpus| cpus.to_string()),
            Field::Memory => options.memory.clone().unwrap_or_default(),
        }
    }

    fn apply(self, text: &str, options: &mut TargetOptions) {
        let text = text.trim();
        let single = (!text.is_empty()).then(|| text.to_string());
        let lines = || {
            let items: Vec<String> = text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string).collect();
            (!items.is_empty()).then_some(items)
        };
        match self {
            Field::Executable => options.executable = single,
            Field::ProfilePath => options.profile_path = single,
            Field::ReadonlyPaths => options.readonly_paths = lines(),
            Field::WritablePaths => options.writable_paths = lines(),
            Field::Image => options.image = single,
            Field::GuestExecutable => options.guest_executable = single,
            Field::GuestWorkdir => options.guest_workdir = single,
            Field::EnvNames => options.env_names = lines(),
            Field::Cpus => options.cpus = if text.is_empty() { None } else { text.parse().ok().or(options.cpus) },
            Field::Memory => options.memory = single,
        }
    }
}

/// General's Local checkout relative to the saved project.
#[derive(Clone, PartialEq)]
pub enum RootDraft {
    Saved,
    Edited(String),
    Cleared,
}

enum Check {
    Idle,
    Busy,
    Done(TargetReadiness),
    Failed(String),
}

pub struct TargetEditor {
    state: AppState,
    project_id: String,
    config: Entity<ConfigDraft>,
    inputs: HashMap<Field, Entity<InputState>>,
    areas: HashMap<Field, Entity<TextareaState>>,
    syncing: bool,
    check: Check,
    /// The project's checkout as currently edited in General (may be unsaved).
    project_root: RootDraft,
    /// Narrow settings pane: controls under their labels.
    stacked: bool,
    /// Passed readiness checks are listed only on request.
    show_passed: bool,
    _task: Option<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn options(config: Option<&ProjectConfig>) -> TargetOptions {
    config.and_then(|config| config.target_options.clone()).unwrap_or_default()
}

impl TargetEditor {
    pub fn new(state: AppState, project_id: String, config: Entity<ConfigDraft>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![cx.observe_in(&config, window, |this, _, window, cx| { this.sync(window, cx); cx.notify() })];
        let mut inputs = HashMap::new();
        for field in Field::SINGLE {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(field.placeholder()));
            subscriptions.push(cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) { let text = input.read(cx).value().to_string(); this.apply(field, text, cx) }
            }));
            inputs.insert(field, input);
        }
        let mut areas = HashMap::new();
        for field in Field::LINES {
            let area = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 6).placeholder(field.placeholder()));
            subscriptions.push(cx.subscribe_in(&area, window, move |this, area, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) { let text = area.read(cx).value().to_string(); this.apply(field, text, cx) }
            }));
            areas.insert(field, area);
        }
        let mut this = Self { state, project_id, config, inputs, areas, syncing: false, check: Check::Idle, project_root: RootDraft::Saved, stacked: false, show_passed: false, _task: None, _subscriptions: subscriptions };
        this.sync(window, cx);
        this
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = options(self.config.read(cx).config());
        self.syncing = true;
        for (field, input) in &self.inputs {
            let value = field.value(&options);
            if input.read(cx).value().trim() != value { input.update(cx, |input, cx| input.set_value(value, window, cx)) }
        }
        for (field, area) in &self.areas {
            let value = field.value(&options);
            if area.read(cx).value().trim() != value { area.update(cx, |area, cx| area.set_value(value, window, cx)) }
        }
        self.syncing = false;
    }

    fn apply(&mut self, field: Field, text: String, cx: &mut Context<Self>) {
        if self.syncing { return }
        self.edit(Some(format!("target:{field:?}")), cx, move |options| field.apply(&text, options));
    }

    fn edit(&mut self, group: Option<String>, cx: &mut Context<Self>, f: impl FnOnce(&mut TargetOptions)) {
        let update = move |config: &mut ProjectConfig| {
            let mut options = config.target_options.clone().unwrap_or_default();
            f(&mut options);
            config.target_options = (options != TargetOptions::default()).then_some(options);
        };
        self.config.update(cx, |config, cx| match group {
            Some(group) => config.edit_grouped(group, cx, update),
            None => config.edit(cx, update),
        });
        self.check = Check::Idle;
    }

    pub fn set_stacked(&mut self, stacked: bool) {
        self.stacked = stacked;
    }

    pub fn set_project_root(&mut self, root: RootDraft) {
        self.project_root = root;
    }

    /// Checks the draft as it is now (nothing is launched; the service only inspects).
    fn run_check(&mut self, cx: &mut Context<Self>) {
        let config = self.config.read(cx).config().cloned();
        // A cleared checkout is reported, never silently replaced by the saved one.
        let project_root = match &self.project_root {
            RootDraft::Saved => None,
            RootDraft::Edited(root) => Some(root.clone()),
            RootDraft::Cleared => {
                self.check = Check::Failed("The local checkout in General is empty. Set it, or discard that change, to check.".into());
                cx.notify();
                return;
            }
        };
        self.check = Check::Busy;
        cx.notify();
        let api = self.state.api.clone();
        let path = format!("/v1/projects/{}/execution-target/check", query_value(&self.project_id));
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = api.post::<_, TargetReadiness>(&path, &CheckTarget { check: true, config: config.as_ref(), project_root }).await;
            this.update(cx, |this, cx| {
                this.check = match result { Ok(readiness) => Check::Done(readiness), Err(error) => Check::Failed(ui::service_message(&error)) };
                cx.notify();
            }).ok();
        }));
    }

    /// Readiness: a summary row with Check, then failed and unknown checks as rows. Passed
    /// checks stay folded behind a count.
    fn render_check(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let supported = self.state.service.read(cx).capabilities().target_readiness;
        let section = components::form_section("Readiness", Some("Checks the settings above, including unsaved changes. Nothing is launched or installed.".into()), cx);
        if !supported {
            return div().child(section).child(components::form_rows(cx).child(components::form_block(ui::update_service_note("check this environment before running", cx), cx)));
        }
        let (chip, detail, checks) = match &self.check {
            Check::Idle => (ui::chip("Not checked", cx).into_any_element(), None, Vec::new()),
            Check::Busy => (ui::chip("Checking…", cx).into_any_element(), None, Vec::new()),
            Check::Failed(error) => (ui::amber_chip("Couldn't check", cx).into_any_element(), Some(error.clone()), Vec::new()),
            Check::Done(readiness) => (
                if readiness.available { components::status_pill("Ready", Tone::Success, cx).into_any_element() } else { ui::amber_chip("Not ready", cx).into_any_element() },
                (!readiness.message.is_empty()).then(|| readiness.message.clone()),
                readiness.checks.clone(),
            ),
        };
        let (passed, open): (Vec<_>, Vec<_>) = checks.into_iter().partition(|check| check.status == "passed");
        let passed_count = passed.len();
        let show_passed = self.show_passed;
        let check_row = move |check: protocol::ReadinessCheck, cx: &App| {
            let tone = match check.status.as_str() { "passed" => Tone::Success, "failed" => Tone::Danger, _ => Tone::Neutral };
            components::form_block(div().flex().items_start().gap_2().t_label()
                .child(div().pt(px(5.)).flex_none().child(components::status_dot(tone, cx)))
                .child(div().flex_1().min_w_0().text_color(theme::tokens(cx).text).child(check.message)), cx)
        };
        div().child(section).child(components::form_rows(cx)
            .child(components::form_block(div().flex().items_center().gap_2()
                .child(div().flex_1().min_w_0().t_body().text_color(t.text_secondary).child(detail.unwrap_or_else(|| "Run a check to see what this environment needs.".into())))
                .child(chip)
                .child(components::button_sm(Button::new("target-check").outline().label("Check"))
                    .disabled(matches!(self.check, Check::Busy))
                    .on_click(cx.listener(|this, _, _, cx| this.run_check(cx)))), cx))
            .children(open.into_iter().map(|check| check_row(check, cx)))
            .when(passed_count > 0, |this| this
                .child(components::disclosure("target-passed", format!("{passed_count} passed"), None, show_passed, cx)
                    .on_click(cx.listener(|this, _, _, cx| { this.show_passed = !this.show_passed; cx.notify() })))
                .when(show_passed, |this| this.children(passed.into_iter().map(|check| check_row(check, cx))))))
    }
}

impl Render for TargetEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.config.read(cx).config().cloned();
        let target = config.as_ref().map_or(ExecutionTarget::Host, |config| config.execution_target.clone());
        let options = options(config.as_ref());
        let stacked = self.stacked;
        let input = |field: Field| div().w_full().child(components::input_md(Input::new(&self.inputs[&field])));
        // Lists are always full width under their label.
        let lines = |label: &'static str, help: &'static str, field: Field, cx: &App| components::form_row(label, Some(help.into()), components::textarea_box(&self.areas[&field], cx), true, cx);
        let row = |label: &'static str, help: Option<&'static str>, control: AnyElement, cx: &App| components::form_row(label, help.map(Into::into), control, stacked, cx);
        let switch = |id: &'static str, on: bool, set: fn(&mut TargetOptions, bool), cx: &mut Context<Self>| {
            Switch::new(id).checked(on).on_click(cx.listener(move |this, checked: &bool, _, cx| { let checked = *checked; this.edit(None, cx, |options| set(options, checked)) })).into_any_element()
        };
        let (title, rows) = match target {
            ExecutionTarget::Mxc => ("MXC sandbox options", components::form_rows(cx)
                .child(row("Executable", Some("Leave empty to use mxc from PATH."), input(Field::Executable).into_any_element(), cx))
                .child(row("Profile", Some("Optional custom sandbox profile."), input(Field::ProfilePath).into_any_element(), cx))
                .child(lines("Read-only paths", "One per line.", Field::ReadonlyPaths, cx))
                .child(lines("Writable paths", "One per line. The checkout is always writable.", Field::WritablePaths, cx))
                .child(row("Network", Some("Allow outbound network access."), switch("target-network", options.network.unwrap_or(false), |o, v| o.network = Some(v), cx), cx))
                .child(row("Keychain", Some("Allow keychain access (for harness sign-in)."), switch("target-keychain", options.keychain_access.unwrap_or(false), |o, v| o.keychain_access = Some(v), cx), cx))
                .into_any_element()),
            ExecutionTarget::AppleContainer => ("Apple container options", components::form_rows(cx)
                .child(row("Image", None, input(Field::Image).into_any_element(), cx))
                .child(row("Harness command", Some("Inside the container."), input(Field::GuestExecutable).into_any_element(), cx))
                .child(row("Working directory", Some("Inside the container."), input(Field::GuestWorkdir).into_any_element(), cx))
                .child(lines("Environment variables", "Names only, one per line; values come from the service's environment.", Field::EnvNames, cx))
                .child(row("CPUs", None, input(Field::Cpus).into_any_element(), cx))
                .child(row("Memory", Some("e.g. 4G (at least 200M)."), input(Field::Memory).into_any_element(), cx))
                .child(row("Network", Some("Allow outbound network access."), switch("target-network", options.network.unwrap_or(false), |o, v| o.network = Some(v), cx), cx))
                .child(row("Harness set up in the image", Some("The image already has the harness installed and signed in."),
                    switch("target-guest-harness", options.guest_harness_configured.unwrap_or(false), |o, v| o.guest_harness_configured = Some(v), cx), cx))
                .into_any_element()),
            _ => ("Local host options", components::form_rows(cx)
                .child(components::form_block(div().t_body().text_color(theme::tokens(cx).text_secondary)
                    .child("Runs directly in the project's checkout with your user's permissions. No options."), cx))
                .into_any_element()),
        };
        div().flex().flex_col()
            .child(components::form_section(title, None, cx))
            .child(rows)
            .child(self.render_check(cx))
    }
}
