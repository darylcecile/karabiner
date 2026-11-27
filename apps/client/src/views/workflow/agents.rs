//! Project-level configuration editors: agents (harness, model, mode, executable),
//! instruction and skill libraries, and the execution environment choice. All edit the
//! shared `ConfigDraft`; nothing here starts a process or stores credentials (Strands names
//! an environment variable).

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{
    AgentDefinition, AgentMode, StrandsModelConfig, ExecutionTarget, ExecutionTargetInfo, HarnessId, HarnessInfo,
    InstructionDefinition, ProjectConfig, SkillDefinition,
};

use super::config::ConfigDraft;
use super::editor::toggle_row;
use crate::views::tasks::{new_id, ui::{self, Load}};
use crate::components::{self, Typography as _};
use crate::theme::{self as theme_tokens, metrics};

/// An input and the field it writes.
type Binding<'a, T> = (&'a Entity<InputState>, fn(&mut T, String));

// ---- Agents --------------------------------------------------------------------------

pub struct AgentsEditor {
    state: crate::state::AppState,
    config: Entity<ConfigDraft>,
    /// General's Local checkout as shown (saved or edited). The harness check is global, so
    /// the service can only resolve relative executables against a root sent with it.
    checkout: Option<String>,
    /// Last readiness check (agent id, result).
    readiness: Option<(String, Result<protocol::HarnessReadiness, String>)>,
    _check: Option<gpui_kit::Task<()>>,
    harnesses: Vec<HarnessInfo>,
    /// `AppSettings.defaultHarness`, used for new agents once loaded.
    default_harness: Option<HarnessId>,
    selected: Option<usize>,
    name: Entity<InputState>,
    role: Entity<InputState>,
    model: Entity<InputState>,
    executable: Entity<InputState>,
    credential_env: Entity<InputState>,
    base_url: Entity<InputState>,
    region: Entity<InputState>,
    /// One input per argument so `[]` and `[""]` stay distinct and spaces are exact.
    arg_inputs: Vec<Entity<InputState>>,
    _arg_subscriptions: Vec<Subscription>,
    instructions: Entity<TextareaState>,
    syncing: bool,
    _subscriptions: Vec<Subscription>,
}

impl AgentsEditor {
    pub fn new(state: crate::state::AppState, config: Entity<ConfigDraft>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let name = input("Agent name", window, cx);
        let role = input("Role, e.g. implementer or reviewer", window, cx);
        let model = input("Model id, e.g. claude-sonnet-4.5", window, cx);
        let executable = input("/usr/local/bin/my-agent", window, cx);
        let credential_env = input("e.g. ANTHROPIC_API_KEY", window, cx);
        let base_url = input("Optional, e.g. http://localhost:11434", window, cx);
        let region = input("e.g. us-east-1", window, cx);
        let instructions = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 10).placeholder("System instructions for this agent"));
        let mut subscriptions = vec![cx.observe_in(&config, window, |this, _, window, cx| { this.sync(window, cx); cx.notify() })];
        let bindings: [Binding<AgentDefinition>; 7] = [
            (&credential_env, |agent, value| if let Some(config) = agent.strands.as_mut() { config.credential_env = (!value.trim().is_empty()).then(|| value.trim().to_string()) }),
            (&base_url, |agent, value| if let Some(config) = agent.strands.as_mut() { config.base_url = (!value.trim().is_empty()).then(|| value.trim().to_string()) }),
            (&region, |agent, value| if let Some(config) = agent.strands.as_mut() { config.region = (!value.trim().is_empty()).then(|| value.trim().to_string()) }),
            (&name, |agent, value| agent.name = value),
            (&role, |agent, value| agent.role = value),
            (&model, |agent, value| agent.model = value),
            (&executable, |agent, value| agent.executable = Some((!value.trim().is_empty()).then_some(value))),
        ];
        for (field, (input, apply)) in bindings.into_iter().enumerate() {
            subscriptions.push(cx.subscribe_in(input, window, move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    // Typing in one field is one undo step.
                    this.edit_in(Some(field), cx, |agent| apply(agent, value));
                }
            }));
        }
        subscriptions.push(cx.subscribe_in(&instructions, window, |this, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let value = input.read(cx).value().to_string();
                this.edit(cx, |agent| agent.instructions = value);
            }
        }));
        let mut this = Self { state, config, checkout: None, readiness: None, _check: None, harnesses: Vec::new(), default_harness: None, selected: None, name, role, model, executable, credential_env, base_url, region, arg_inputs: Vec::new(), _arg_subscriptions: Vec::new(), instructions, syncing: false, _subscriptions: subscriptions };
        this.sync(window, cx);
        this
    }

    /// Harness availability as observed by the service (status only; never authenticates).
    pub fn set_harnesses(&mut self, harnesses: Vec<HarnessInfo>, default_harness: Option<HarnessId>, cx: &mut Context<Self>) {
        self.harnesses = harnesses;
        self.default_harness = default_harness;
        cx.notify();
    }

    fn agent(&self, cx: &App) -> Option<AgentDefinition> {
        self.config.read(cx).config()?.agents.get(self.selected?).cloned()
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let agent = self.agent(cx);
        if agent.is_none() { self.selected = None }
        self.syncing = true;
        let text = |f: fn(&AgentDefinition) -> String| agent.as_ref().map(f).unwrap_or_default();
        for (input, value) in [
            (&self.name, text(|a| a.name.clone())),
            (&self.role, text(|a| a.role.clone())),
            (&self.model, text(|a| a.model.clone())),
            (&self.executable, text(|a| a.executable.clone().flatten().unwrap_or_default())),
            (&self.credential_env, text(|a| a.strands.as_ref().and_then(|s| s.credential_env.clone()).unwrap_or_default())),
            (&self.base_url, text(|a| a.strands.as_ref().and_then(|s| s.base_url.clone()).unwrap_or_default())),
            (&self.region, text(|a| a.strands.as_ref().and_then(|s| s.region.clone()).unwrap_or_default())),
        ] {
            if input.read(cx).value().as_ref() != value {
                input.update(cx, |input, cx| input.set_value(value.as_str(), window, cx));
            }
        }
        self.sync_args(agent.as_ref().and_then(|a| a.args.clone()).unwrap_or_default(), window, cx);
        let instructions = text(|a| a.instructions.clone());
        if self.instructions.read(cx).value().as_ref() != instructions {
            self.instructions.update(cx, |input, cx| input.set_value(instructions.as_str(), window, cx));
        }
        self.syncing = false;
    }

    /// Rebuilds argument rows when the count changes; otherwise updates values in place.
    fn sync_args(&mut self, args: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.arg_inputs.len() != args.len() {
            self.arg_inputs.clear();
            self._arg_subscriptions.clear();
            for (index, value) in args.iter().enumerate() {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("argument").default_value(value.clone()));
                self._arg_subscriptions.push(cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        this.edit(cx, |agent| {
                            let args = agent.args.get_or_insert_with(Vec::new);
                            if let Some(slot) = args.get_mut(index) { *slot = value }
                        });
                    }
                }));
                self.arg_inputs.push(input);
            }
            return;
        }
        for (input, value) in self.arg_inputs.iter().zip(args.iter()) {
            if input.read(cx).value().as_ref() != value.as_str() {
                input.update(cx, |input, cx| input.set_value(value.as_str(), window, cx));
            }
        }
    }

    fn edit_args(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Vec<String>)) {
        self.edit(cx, |agent| f(agent.args.get_or_insert_with(Vec::new)));
    }

    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut AgentDefinition)) {
        self.edit_in(None, cx, f);
    }

    fn edit_in(&mut self, field: Option<usize>, cx: &mut Context<Self>, f: impl FnOnce(&mut AgentDefinition)) {
        let Some(index) = self.selected else { return };
        if self.syncing { return }
        let apply = move |config: &mut ProjectConfig| if let Some(agent) = config.agents.get_mut(index) { f(agent) };
        self.config.update(cx, |config, cx| match field {
            Some(field) => config.edit_grouped(format!("agent:{index}:{field}"), cx, apply),
            None => config.edit(cx, apply),
        });
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut index = None;
        let harness = self.default_harness.clone().filter(HarnessId::is_known).unwrap_or(HarnessId::Claude);
        self.config.update(cx, |config, cx| config.edit(cx, |config| {
            config.agents.push(AgentDefinition {
                id: new_id("agent"),
                name: format!("Agent {}", config.agents.len() + 1),
                role: String::new(),
                harness,
                model: String::new(),
                instructions: String::new(),
                skill_ids: Vec::new(),
                mode: AgentMode::Autonomous,
                executable: None,
                args: None,
                instruction_ids: None,
                protocol: None,
                strands: None,
                extra: Default::default(),
            });
            index = Some(config.agents.len() - 1);
        }));
        self.selected = index;
        self.sync(window, cx);
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.selected else { return };
        self.config.update(cx, |config, cx| config.edit(cx, |config| {
            if index < config.agents.len() {
                let removed = config.agents.remove(index);
                for workflow in config.workflows.iter_mut() {
                    for stage in workflow.stages.iter_mut() { stage.agent_ids.retain(|id| id != &removed.id) }
                }
            }
        }));
        self.selected = None;
        self.sync(window, cx);
    }

    fn harness_note(&self, harness: &HarnessId) -> Option<String> {
        let info = self.harnesses.iter().find(|info| &info.id == harness)?;
        let installed = match info.installed { Some(true) => "found on PATH", Some(false) => "not found", None => "not checked" };
        Some(super::join_parts(&[&info.executable, installed, &info.message]))
    }

    fn render_form(&self, agent: &AgentDefinition, config: &ProjectConfig, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let harness_control = if agent.harness.is_known() {
            components::button_md(Button::new("agent-harness").outline().label(ui::harness_label(&agent.harness)).dropdown_caret(true)).dropdown_menu({
                let weak = weak.clone();
                move |mut menu, _, _| {
                    for harness in HarnessId::ALL.iter() {
                        let (weak, harness) = (weak.clone(), harness.clone());
                        menu = menu.item(PopupMenuItem::new(ui::harness_label(&harness)).on_click(move |_, _, cx| {
                            weak.update(cx, |this, cx| this.edit(cx, |agent| agent.harness = harness.clone())).ok();
                        }));
                    }
                    menu
                }
            }).into_any_element()
        } else {
            ui::chip(format!("{} (unsupported here, kept as is)", agent.harness.as_str()), cx).into_any_element()
        };
        let mode_control = if agent.mode.is_known() {
            ui::segmented("agent-mode", &[(AgentMode::Interactive, "Interactive"), (AgentMode::Autonomous, "Autonomous")], agent.mode.clone(),
                move |mode, _, cx| { weak.update(cx, |this, cx| this.edit(cx, |agent| agent.mode = mode)).ok(); }, cx).into_any_element()
        } else {
            ui::chip(format!("{} (unsupported here)", agent.mode.as_str()), cx).into_any_element()
        };
        let custom = agent.harness == HarnessId::Custom;
        let strands = agent.harness == HarnessId::Strands;
        // Plain groups separated by section titles; each field keeps its own (only) border.
        div().flex().flex_col().gap_4()
            .child(div().flex().items_center().justify_between()
                .child(div().t_body().font_weight(FontWeight::SEMIBOLD).text_color(theme_tokens::tokens(cx).text).child("Agent"))
                .child(components::button_sm(Button::new("remove-agent").ghost().icon(gpui_kit::assets::IconName::Trash).label("Remove"))
                    .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx)))))
            .child(div().flex().flex_wrap().gap_3()
                .child(div().flex_1().min_w(px(160.)).child(ui::field("Name", components::input_md(Input::new(&self.name)), cx)))
                .child(div().flex_1().min_w(px(160.)).child(ui::field("Role", components::input_md(Input::new(&self.role)), cx))))
            .child(components::form_section("Harness", None, cx).pt_2().border_t_1().border_color(theme_tokens::tokens(cx).border))
            .child(div().flex().flex_wrap().gap_3()
                .child(div().flex_1().min_w(px(160.)).child(ui::field("Harness", harness_control, cx)))
                .child(div().flex_1().min_w(px(160.)).child(ui::field("Model", components::input_md(Input::new(&self.model)), cx))))
            .children(self.harness_note(&agent.harness).map(|note| div().t_caption().text_color(cx.theme().muted_foreground).child(note)))
            .child(div().t_caption().text_color(cx.theme().muted_foreground).child(auth_note(&agent.harness)))
            .when(strands, |this| this.child(if self.state.service.read(cx).capabilities().strands_configuration || agent.strands.is_some() {
                self.render_strands(agent, cx).into_any_element()
            } else {
                ui::update_service_note("choose a Strands model provider", cx).into_any_element()
            }))
            .child(self.render_check(agent, cx))
            .when(custom, |this| this.child(ui::field("Protocol", ui::segmented("agent-protocol",
                &[("jsonl-v1".to_string(), "Line-based (default)"), ("jsonrpc-v2".to_string(), "JSON-RPC v2")],
                agent.protocol.clone().unwrap_or_else(|| "jsonl-v1".into()), {
                    let weak = cx.weak_entity();
                    move |protocol, _, cx| { weak.update(cx, |this, cx| this.edit(cx, |agent| agent.protocol = (protocol != "jsonl-v1").then_some(protocol))).ok(); }
                }, cx), cx)))
            .when(custom, |this| this.child(div().flex().flex_wrap().gap_3()
                .child(div().flex_1().min_w(px(160.)).child(ui::field("Executable", components::input_md(Input::new(&self.executable)), cx)))
                .child(div().flex_1().min_w(px(160.)).child(ui::field("Arguments", div().flex().flex_col().gap_1()
                    .children(self.arg_inputs.iter().enumerate().map(|(index, input)| {
                        div().flex().items_center().gap_1()
                            .child(div().flex_1().child(components::input_md(Input::new(input))))
                            .child(components::icon_button(Button::new(("remove-arg", index)).ghost().icon(IconName::Close).tooltip("Remove argument"), metrics::CONTROL_MD)
                                .on_click(cx.listener(move |this, _, _, cx| this.edit_args(cx, |args| { if index < args.len() { args.remove(index); } }))))
                    }))
                    .child(div().flex().child(components::button_sm(Button::new("add-arg").ghost().icon(IconName::Plus).label("Add argument"))
                        .on_click(cx.listener(|this, _, _, cx| this.edit_args(cx, |args| args.push(String::new())))))), cx)))))
            .child(components::form_section("Behavior", None, cx).pt_2().border_t_1().border_color(theme_tokens::tokens(cx).border))
            .child(ui::field("Mode", mode_control, cx))
            .child(div().t_caption().text_color(cx.theme().muted_foreground).child(
                "Autonomous runs tools without asking each time, within the selected execution target. Interactive pauses for permissions and questions. Workflow approval steps still wait for you."))
            .child(ui::field("Skills", div().flex().flex_col().gap_1()
                .when(config.skills.is_empty(), |this| this.child(div().t_caption().text_color(cx.theme().muted_foreground).child("No skills in this project yet.")))
                .children(config.skills.iter().map(|skill| {
                    let on = agent.skill_ids.contains(&skill.id);
                    let id = skill.id.clone();
                    toggle_row(SharedString::from(format!("agent-skill-{}", skill.id)), on, skill.name.clone(), None, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.edit(cx, |agent| {
                                if let Some(position) = agent.skill_ids.iter().position(|existing| existing == &id) {
                                    agent.skill_ids.remove(position);
                                } else {
                                    agent.skill_ids.push(id);
                                }
                            })
                        }))
                })), cx))
            .child(ui::field("Instructions", components::textarea_box(&self.instructions, cx), cx))
    }

    /// Checks the agent as drafted: install, sign-in (known harnesses only; custom programs are
    /// never run) and, for Strands, whether the credential variable is set. No model calls.
    pub fn set_checkout(&mut self, root: Option<String>) {
        self.checkout = root;
    }

    fn check_agent(&mut self, agent: AgentDefinition, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let id = agent.id.clone();
        // Always the current checkout; with none, sign-in is still checked and the service reports
        // a relative executable it can't resolve.
        let project_root = self.checkout.clone();
        self.readiness = None;
        cx.notify();
        self._check = Some(cx.spawn(async move |this, cx| {
            let body = protocol::CheckHarness { agent: &agent, check: true, project_root };
            let result = api.post::<_, protocol::HarnessReadiness>("/v1/harnesses/check", &body).await.map_err(|error| {
                if error.is_not_found() { "This service can't check agents yet. Update Karabiner's service to check them here.".to_string() } else { ui::service_message(&error) }
            });
            this.update(cx, |this, cx| { this.readiness = Some((id, result)); cx.notify() }).ok();
        }));
    }

    fn render_check(&self, agent: &AgentDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let result = self.readiness.as_ref().filter(|(id, _)| *id == agent.id).map(|(_, result)| result.clone());
        let muted = cx.theme().muted_foreground;
        let yes_no = |value: Option<bool>, yes: &'static str, no: &'static str, unknown: &'static str| match value { Some(true) => yes, Some(false) => no, None => unknown };
        let interactive = agent.mode == AgentMode::Interactive;
        let checked = agent.clone();
        let body = result.map(|result| match result {
            Err(error) => div().t_caption().text_color(muted).child(error).into_any_element(),
            Ok(readiness) => {
                let chip = if readiness.execution_available { components::status_pill("Ready", components::Tone::Success, cx).into_any_element() } else { ui::amber_chip("Not ready", cx).into_any_element() };
                let fix = match readiness.code.as_deref() {
                    Some("credential_env_missing" | "credential_missing") => " Set it in the environment the service starts with, then restart the service.",
                    _ => "",
                };
                let support = readiness.supported.map(|support| super::join_parts(&[
                    match (&agent.mode, support.permissions) {
                        (AgentMode::Autonomous, _) => "tools run without permission prompts",
                        (AgentMode::Interactive, true) => "asks permission in the app",
                        (AgentMode::Interactive, false) => "no in-app permission prompts",
                        _ => "mode not supported by this app",
                    },
                    if interactive && support.questions { "can ask questions" } else { "" },
                    match support.steering.as_str() { "native-turn" => "takes live guidance", "native-tool-boundary" => "takes guidance at tool calls", _ => "takes guidance between steps" },
                ]));
                div().flex().flex_col().gap_1()
                    .child(div().flex().items_center().gap_2().child(chip)
                        .child(div().t_caption().text_color(muted).child(super::join_parts(&[
                            yes_no(readiness.installed, "installed", "not installed", "install not checked"),
                            yes_no(readiness.authenticated, "signed in", "signed out", "sign-in unknown"),
                        ]))))
                    .child(div().t_caption().text_color(muted).child(format!("{}{fix}", readiness.message)))
                    .children(support.map(|support| div().t_caption().text_color(muted).child(format!("While running: {support}."))))
                    .children(readiness.checks.into_iter().filter(|check| check.status != "passed").map(move |check| div().t_caption().text_color(muted).child(check.message)))
                    .into_any_element()
            }
        });
        if !self.state.service.read(cx).capabilities().harness_readiness {
            return div().flex().flex_col().gap_2().child(ui::update_service_note("check this agent's install and sign-in", cx));
        }
        div().flex().flex_col().gap_2()
            .child(div().flex().child(components::button_sm(Button::new("agent-check").outline().label("Check readiness"))
                .on_click(cx.listener(move |this, _, _, cx| this.check_agent(checked.clone(), cx)))))
            .children(body)
    }

    /// Strands model provider and how it finds credentials (a variable name, never a key).
    fn render_strands(&self, agent: &AgentDefinition, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        const PROVIDERS: [(&str, &str); 6] = [("bedrock", "Amazon Bedrock"), ("anthropic", "Anthropic"), ("openai", "OpenAI"), ("google", "Google"), ("ollama", "Ollama"), ("litellm", "LiteLLM")];
        let provider = agent.strands.as_ref().map(|config| config.provider.clone());
        let label = provider.as_ref().map_or("Choose provider".to_string(), |id| PROVIDERS.iter().find(|(value, _)| value == id).map_or(id.clone(), |(_, name)| name.to_string()));
        let weak = cx.weak_entity();
        let picker = components::button_md(Button::new("strands-provider").outline().label(label).dropdown_caret(true)).dropdown_menu(move |mut menu, _, _| {
            for (value, name) in PROVIDERS {
                let weak = weak.clone();
                menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                    weak.update(cx, |this, cx| this.edit(cx, |agent| match agent.strands.as_mut() {
                        Some(config) => config.provider = value.into(),
                        None => agent.strands = Some(StrandsModelConfig { provider: value.into(), credential_env: None, base_url: None, region: None, api: None, extra: Default::default() }),
                    })).ok();
                }));
            }
            menu
        });
        let provider = provider.unwrap_or_default();
        let weak = cx.weak_entity();
        div().flex().flex_col().gap_3()
            .child(ui::field("Model provider", div().flex().child(picker), cx))
            .when(!provider.is_empty(), |this| this
                .when(provider != "ollama", |this| this.child(ui::field("Credential variable", components::input_md(Input::new(&self.credential_env)), cx))
                    .child(div().t_caption().text_color(cx.theme().muted_foreground).child(if provider == "bedrock" {
                        "Optional. Leave empty to use the service's AWS profile or role."
                    } else {
                        "Name of the environment variable the service reads. The key itself is never stored."
                    })))
                .when(provider == "bedrock", |this| this.child(ui::field("Region", components::input_md(Input::new(&self.region)), cx)))
                .when(matches!(provider.as_str(), "openai" | "ollama" | "litellm"), |this| this.child(ui::field("Base URL", components::input_md(Input::new(&self.base_url)), cx)))
                .when(provider == "openai", |this| this.child(ui::field("API", ui::segmented("strands-api", &[("chat".to_string(), "Chat"), ("responses".to_string(), "Responses")],
                    agent.strands.as_ref().and_then(|config| config.api.clone()).unwrap_or_else(|| "chat".into()),
                    move |api, _, cx| { weak.update(cx, |this, cx| this.edit(cx, |agent| if let Some(config) = agent.strands.as_mut() { config.api = Some(api) })).ok(); }, cx), cx))))
    }
}

/// How each harness authenticates; Karabiner never stores harness credentials.
fn auth_note(harness: &HarnessId) -> &'static str {
    match harness {
        HarnessId::Claude => "Uses Claude Code's own sign-in on this Mac (run `claude` once to log in).",
        HarnessId::Codex => "Uses Codex's own sign-in on this Mac (`codex login`).",
        HarnessId::Copilot => "Uses the Copilot CLI's GitHub sign-in on this Mac.",
        HarnessId::Strands => "Runs in the service with the model provider below.",
        HarnessId::Custom => "Your executable handles its own credentials.",
        HarnessId::Unknown(_) => "",
    }
}

impl Render for AgentsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(config) = self.config.read(cx).config().cloned() else { return ui::skeleton_rows(2, cx).into_any_element() };
        let selected = self.selected.and_then(|index| config.agents.get(index)).cloned();
        // The form wraps under the list when the pane can't give it 360 (narrow content).
        div().flex().flex_wrap().gap_4()
            .child(div().w(px(240.)).flex_none().flex().flex_col().gap_1()
                .children(config.agents.iter().enumerate().map(|(index, agent)| {
                    list_item(("agent-item", index), self.selected == Some(index), &agent.name,
                        &format!("{} · {}", ui::harness_label(&agent.harness), if agent.model.is_empty() { "no model" } else { &agent.model }), cx)
                        .on_click(cx.listener(move |this, _, window, cx| { this.selected = Some(index); this.sync(window, cx); cx.notify() }))
                }))
                .child(div().flex().pt_1().child(components::button_sm(Button::new("add-agent").ghost().icon(IconName::Plus).label("Add agent"))
                    .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))))))
            // 360 is the wrapping basis (the form drops under the list below it); alone on its
            // line the form shrinks to the pane (e.g. ~302 with a 360 sidebar at 720).
            .child(div().flex_1().flex_basis(px(360.)).min_w_0().child(match selected {
                Some(agent) => self.render_form(&agent, &config, cx).into_any_element(),
                None => div().p_4().t_label().text_color(cx.theme().muted_foreground)
                    .child(if config.agents.is_empty() { "No agents yet. Add one to assign it to workflow stages." } else { "Select an agent to edit it." })
                    .into_any_element(),
            }))
            .into_any_element()
    }
}

// ---- Instructions and skills ---------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum LibraryKind {
    Instructions,
    Skills,
}

/// Name + description + content + optional file path editor for instructions or skills.
pub struct LibraryEditor {
    config: Entity<ConfigDraft>,
    kind: LibraryKind,
    selected: Option<usize>,
    name: Entity<InputState>,
    description: Entity<InputState>,
    path: Entity<InputState>,
    content: Entity<TextareaState>,
    syncing: bool,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Default)]
struct Entry {
    name: String,
    description: String,
    path: String,
    content: String,
}

impl LibraryEditor {
    pub fn new(config: Entity<ConfigDraft>, kind: LibraryKind, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let description = cx.new(|cx| InputState::new(window, cx).placeholder("When should an agent use this?"));
        let path = cx.new(|cx| InputState::new(window, cx).placeholder(match kind {
            LibraryKind::Instructions => "instructions/review.md (optional)",
            LibraryKind::Skills => "skills/release/SKILL.md (optional)",
        }));
        let content = cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 24).placeholder("Markdown content"));
        let mut subscriptions = vec![cx.observe_in(&config, window, |this, _, window, cx| { this.sync(window, cx); cx.notify() })];
        let fields: [Binding<Entry>; 3] = [
            (&name, |entry, value| entry.name = value),
            (&description, |entry, value| entry.description = value),
            (&path, |entry, value| entry.path = value),
        ];
        for (input, apply) in fields {
            subscriptions.push(cx.subscribe_in(input, window, move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.edit(cx, |entry| apply(entry, value));
                }
            }));
        }
        subscriptions.push(cx.subscribe_in(&content, window, |this, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let value = input.read(cx).value().to_string();
                this.edit(cx, |entry| entry.content = value);
            }
        }));
        let mut this = Self { config, kind, selected: None, name, description, path, content, syncing: false, _subscriptions: subscriptions };
        this.sync(window, cx);
        this
    }

    fn entries(&self, config: &ProjectConfig) -> Vec<Entry> {
        match self.kind {
            LibraryKind::Instructions => config.instructions.iter().map(|item| Entry {
                name: item.name.clone(), description: String::new(), path: item.path.clone().unwrap_or_default(), content: item.content.clone(),
            }).collect(),
            LibraryKind::Skills => config.skills.iter().map(|item| Entry {
                name: item.name.clone(), description: item.description.clone(), path: item.path.clone().unwrap_or_default(), content: item.content.clone(),
            }).collect(),
        }
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = self.config.read(cx).config().and_then(|config| self.entries(config).get(self.selected?).cloned());
        if entry.is_none() { self.selected = None }
        let entry = entry.unwrap_or_default();
        self.syncing = true;
        for (input, value) in [(&self.name, &entry.name), (&self.description, &entry.description), (&self.path, &entry.path)] {
            if input.read(cx).value().as_ref() != value.as_str() {
                input.update(cx, |input, cx| input.set_value(value.as_str(), window, cx));
            }
        }
        if self.content.read(cx).value().as_ref() != entry.content {
            self.content.update(cx, |input, cx| input.set_value(entry.content.as_str(), window, cx));
        }
        self.syncing = false;
    }

    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Entry)) {
        let Some(index) = self.selected else { return };
        if self.syncing { return }
        let kind = self.kind;
        self.config.update(cx, |config, cx| config.edit(cx, |config| match kind {
            LibraryKind::Instructions => if let Some(item) = config.instructions.get_mut(index) {
                let mut entry = Entry { name: item.name.clone(), description: String::new(), path: item.path.clone().unwrap_or_default(), content: item.content.clone() };
                f(&mut entry);
                item.name = entry.name;
                item.content = entry.content;
                item.path = (!entry.path.trim().is_empty()).then_some(entry.path);
            },
            LibraryKind::Skills => if let Some(item) = config.skills.get_mut(index) {
                let mut entry = Entry { name: item.name.clone(), description: item.description.clone(), path: item.path.clone().unwrap_or_default(), content: item.content.clone() };
                f(&mut entry);
                item.name = entry.name;
                item.description = entry.description;
                item.content = entry.content;
                item.path = (!entry.path.trim().is_empty()).then_some(entry.path);
            },
        }));
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.kind;
        let mut index = None;
        self.config.update(cx, |config, cx| config.edit(cx, |config| match kind {
            LibraryKind::Instructions => {
                config.instructions.push(InstructionDefinition { id: new_id("instr"), name: "New instructions".into(), content: String::new(), path: None, extra: Default::default() });
                index = Some(config.instructions.len() - 1);
            }
            LibraryKind::Skills => {
                config.skills.push(SkillDefinition { id: new_id("skill"), name: "New skill".into(), description: String::new(), content: String::new(), path: None, extra: Default::default() });
                index = Some(config.skills.len() - 1);
            }
        }));
        self.selected = index;
        self.sync(window, cx);
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(index), kind) = (self.selected, self.kind) else { return };
        self.config.update(cx, |config, cx| config.edit(cx, |config| match kind {
            LibraryKind::Instructions => if index < config.instructions.len() { config.instructions.remove(index); },
            LibraryKind::Skills => if index < config.skills.len() {
                let removed = config.skills.remove(index);
                for agent in config.agents.iter_mut() { agent.skill_ids.retain(|id| id != &removed.id) }
            },
        }));
        self.selected = None;
        self.sync(window, cx);
    }
}

impl Render for LibraryEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(config) = self.config.read(cx).config().cloned() else { return ui::skeleton_rows(2, cx).into_any_element() };
        let entries = self.entries(&config);
        let noun = match self.kind { LibraryKind::Instructions => "instructions", LibraryKind::Skills => "skill" };
        div().flex().flex_wrap().gap_4()
            .child(div().w(px(240.)).flex_none().flex().flex_col().gap_1()
                .children(entries.iter().enumerate().map(|(index, entry)| {
                    let meta = if entry.path.is_empty() { format!("{} chars", entry.content.chars().count()) } else { entry.path.clone() };
                    list_item((noun, index), self.selected == Some(index), &entry.name, &meta, cx)
                        .on_click(cx.listener(move |this, _, window, cx| { this.selected = Some(index); this.sync(window, cx); cx.notify() }))
                }))
                .child(div().flex().pt_1().child(components::button_sm(Button::new("add-library").ghost().icon(IconName::Plus).label(format!("Add {noun}")))
                    .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))))))
            // 360 is the wrapping basis (the form drops under the list below it); alone on its
            // line the form shrinks to the pane (e.g. ~302 with a 360 sidebar at 720).
            .child(div().flex_1().flex_basis(px(360.)).min_w_0().child(if self.selected.is_some() {
                div().flex().flex_col().gap_4()
                    .child(div().flex().justify_end().child(components::button_sm(Button::new("remove-library").ghost().icon(gpui_kit::assets::IconName::Trash).label("Remove"))
                        .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx)))))
                    .child(ui::field("Name", components::input_md(Input::new(&self.name)), cx))
                    .when(self.kind == LibraryKind::Skills, |this| this.child(ui::field("Description", components::input_md(Input::new(&self.description)), cx)))
                    .child(ui::field("File path", components::input_md(Input::new(&self.path)), cx))
                    .child(ui::field("Content", components::textarea_box(&self.content, cx), cx))
                    .into_any_element()
            } else {
                div().p_4().t_label().text_color(cx.theme().muted_foreground)
                    .child(if entries.is_empty() { format!("No {noun} yet.") } else { format!("Select {noun} to edit.") })
                    .into_any_element()
            }))
            .into_any_element()
    }
}

/// Compact two-line selectable list row.
pub fn list_item(id: impl Into<ElementId>, selected: bool, title: &str, meta: &str, cx: &App) -> Stateful<Div> {
    let t = theme_tokens::tokens(cx);
    div().id(id).px_2().py_1p5().flex().flex_col().gap_0p5().rounded(px(metrics::R_CONTROL)).cursor_pointer()
        .when(selected, |this| this.bg(t.selected))
        .when(!selected, |this| this.hover(move |style| style.bg(t.hover)))
        .child(div().t_label().font_weight(FontWeight::MEDIUM).text_color(t.text).truncate().child(if title.is_empty() { "Untitled".to_string() } else { title.to_string() }))
        .child(div().t_caption().truncate().text_color(cx.theme().muted_foreground).child(meta.to_string()))
}


// ---- Environment ---------------------------------------------------------------------

/// Execution environment choice as a radio list, one full-width row per target: title,
/// availability on its own line (wraps, never overflows), then what it is. The service's
/// availability report is shown as reported. Selection is configuration only.
pub fn environment_picker(
    config: &Entity<ConfigDraft>,
    targets: &Load<Vec<ExecutionTargetInfo>>,
    cx: &App,
) -> impl IntoElement + use<> {
    let t = theme_tokens::tokens(cx);
    let current = config.read(cx).config().map(|config| config.execution_target.clone());
    let unknown = current.as_ref().filter(|target| !target.is_known()).cloned();
    let report = |target: &ExecutionTarget| targets.ready().and_then(|list| list.iter().find(|info| &info.id == target)).cloned();
    components::form_rows(cx)
        .children(unknown.map(|target| components::form_block(ui::amber_chip(format!("Current target \"{}\" comes from a newer version and is kept unless you pick another.", target.as_str()), cx), cx)))
        .children(ExecutionTarget::ALL.iter().map(|target| {
            let selected = current.as_ref() == Some(target);
            let info = report(target);
            let config = config.downgrade();
            let value = target.clone();
            let (availability, tone) = match &info {
                Some(info) if info.available => ("Available".to_string(), t.success),
                Some(info) => (if info.message.is_empty() { format!("Not available here. Requires: {}", info.requirements) } else { format!("Not available here. {}", info.message) }, t.attention),
                None => ("Not checked".to_string(), t.text_secondary),
            };
            div().id(SharedString::from(format!("target-{}", target.as_str()))).flex().items_start().gap_3().py_3().px_2()
                .border_b_1().border_color(t.border).cursor_pointer().hover(move |style| style.bg(t.hover))
                .on_click(move |_, _, cx| {
                    let value = value.clone();
                    config.update(cx, |config, cx| config.edit(cx, |config| config.execution_target = value)).ok();
                })
                // Radio mark: ring, filled when selected.
                .child(div().mt(px(3.)).size(px(14.)).flex_none().rounded_full().border_1()
                    .border_color(if selected { t.accent_fill } else { t.text_tertiary })
                    .flex().items_center().justify_center()
                    .when(selected, |this| this.child(div().size(px(6.)).rounded_full().bg(t.accent_fill))))
                .child(Icon::new(match target { ExecutionTarget::Host => IconName::SquareTerminal, ExecutionTarget::Mxc => IconName::Cpu, _ => IconName::HardDrive })
                    .size(px(16.)).text_color(t.text_secondary).mt(px(1.)))
                .child(div().flex_1().min_w_0().flex().flex_col().gap_0p5()
                    .child(div().t_body().font_weight(FontWeight::MEDIUM).text_color(t.text).truncate().child(ui::target_label(target)))
                    .child(div().t_caption().text_color(t.text_secondary).child(ui::target_detail(target)))
                    .child(div().t_caption().text_color(tone).child(availability)))
        }))
}

/// Compact form for narrow panels (the workflow inspector): a dropdown of targets and the
/// chosen one's availability underneath.
pub fn environment_dropdown(
    config: &Entity<ConfigDraft>,
    targets: &Load<Vec<ExecutionTargetInfo>>,
    cx: &App,
) -> impl IntoElement + use<> {
    let t = theme_tokens::tokens(cx);
    let current = config.read(cx).config().map(|config| config.execution_target.clone());
    let label = current.as_ref().map_or("Choose".to_string(), |target| ui::target_label(target).to_string());
    let info = current.as_ref().and_then(|target| targets.ready().and_then(|list| list.iter().find(|info| &info.id == target)).cloned());
    let weak = config.downgrade();
    div().flex().flex_col().gap_1()
        .child(div().flex().child(components::button_sm(Button::new("env-target").outline().label(label).dropdown_caret(true)).dropdown_menu(move |mut menu, _, _| {
            for target in ExecutionTarget::ALL.iter() {
                let (weak, value) = (weak.clone(), target.clone());
                menu = menu.item(PopupMenuItem::new(ui::target_label(target)).on_click(move |_, _, cx| {
                    let value = value.clone();
                    weak.update(cx, |config, cx| config.edit(cx, |config| config.execution_target = value)).ok();
                }));
            }
            menu
        })))
        .children(info.map(|info| div().t_caption().text_color(if info.available { t.text_secondary } else { t.attention })
            .child(if info.available { "Available".to_string() } else if info.message.is_empty() { format!("Not available here. Requires: {}", info.requirements) } else { format!("Not available here. {}", info.message) })))
}
