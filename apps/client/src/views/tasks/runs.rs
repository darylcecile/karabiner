//! Run inspection and control (ref B457FF32 step tracker): stage rail with done / running /
//! needs-you / pending glyphs, collapsible steps, pause / resume / cancel / retry, and the
//! run's pending questions and approvals. Runs come from `GET /v1/runs?taskId=` and refresh on
//! service SSE invalidations (via `ServiceState`), plus a bounded poll while a run is active. Nothing here fabricates
//! activity; config edits only affect future runs (each run keeps its snapshot).

use std::collections::HashSet;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use protocol::{AddRunInstruction, CreateRun, QuestionState, Run, RunControl, RunQuestion, RunState, RunStep, RunStepStatus};

mod questions;

use questions::Reply;
use super::ui::{self, Load};

use crate::{api::query_value, state::{AppState, TabTarget}};
use crate::components::{self, Step, StepAnswer, StepState, Typography as _, step_glyph};
use crate::theme::{self as theme_tokens, metrics};

/// One configured workflow stage with its assigned agents `(name, harness)`.
#[derive(Clone, PartialEq)]
pub struct StageRow {
    pub name: String,
    pub agents: Vec<(String, String)>,
}

pub struct RunsPanel {
    state: AppState,
    project_id: String,
    task_id: String,
    stages: Vec<StageRow>,
    runs: Load<Vec<Run>>,
    expanded: HashSet<String>,
    start_note: Option<String>,
    starting: bool,
    seen_version: u64,
    /// One trace component per run, kept so its expand/select state survives refreshes.
    traces: std::collections::HashMap<String, Entity<components::TraceView>>,
    /// Agent id → name, for the agent a running step actually uses.
    agents: Vec<(String, String)>,
    /// Answer drafts per open question (by question id), so parallel questions keep
    /// independent drafts and refreshes never touch what was typed.
    drafts: std::collections::HashMap<String, questions::QuestionDraft>,
    /// Run whose "Add instruction" field is open, and that field.
    instructing: Option<String>,
    instruction_input: Entity<InputState>,
    polling: Option<gpui_kit::Task<()>>,
    _load: Option<gpui_kit::Task<()>>,
    /// Start-run and approve/decline writes; independent of refreshes.
    _actions: Vec<gpui_kit::Task<()>>,
    _subscription: Subscription,
}

impl RunsPanel {
    pub fn new(state: AppState, project_id: String, task_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state.service, |this: &mut Self, service, cx| {
            let version = service.read(cx).data_version();
            if version != this.seen_version {
                this.seen_version = version;
                this.reload(cx);
            }
        });
        let instruction_input = cx.new(|cx| InputState::new(window, cx).placeholder("Guidance for the next agent step, then Enter"));
        cx.subscribe(&instruction_input, |this: &mut Self, input, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::PressEnter { .. }) { return }
            let text = input.read(cx).value().trim().to_string();
            if let (false, Some(run)) = (text.is_empty(), this.instructing.clone()) { this.instruct(&run, text, cx) }
        }).detach();
        let mut this = Self {
            seen_version: state.service.read(cx).data_version(),
            state,
            project_id,
            task_id,
            stages: Vec::new(),
            runs: Load::Loading,
            expanded: HashSet::new(),
            start_note: None,
            starting: false,
            traces: Default::default(),
            agents: Vec::new(),
            drafts: Default::default(),
            instructing: None,
            instruction_input,
            polling: None,
            _load: None,
            _actions: Vec::new(),
            _subscription: subscription,
        };
        this.reload(cx);
        this
    }

    pub fn set_stages(&mut self, stages: Vec<StageRow>, agents: Vec<(String, String)>) {
        self.stages = stages;
        self.agents = agents;
    }

    fn agent_label(&self, id: &str) -> String {
        self.agents.iter().find(|(agent, _)| agent == id).map_or_else(|| id.to_string(), |(_, name)| name.clone())
    }

    /// Re-fetches every few seconds while a run is active and this panel is alive. The SSE
    /// stream normally refreshes first; this bounded fallback covers services without
    /// `events` and the gap while the stream reconnects (events aren't replayed).
    fn sync_polling(&mut self, cx: &mut Context<Self>) {
        let active = self.runs.ready().is_some_and(|runs| runs.iter().any(Run::is_active));
        if !active { self.polling = None; return }
        if self.polling.is_some() { return }
        self.polling = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
            if this.update(cx, |this, cx| this.reload(cx)).is_err() { break }
        }));
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/runs?taskId={}", query_value(&self.task_id));
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = api.get::<Vec<Run>>(&path).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(runs) => this.runs = Load::Ready(runs),
                    // Keep the last runs on a failed background refresh.
                    Err(_) if this.runs.ready().is_some() => {}
                    Err(error) => this.runs = Load::Failed(ui::error_text(&error)),
                }
                this.sync_traces(cx);
                this.sync_polling(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        self.starting = true;
        cx.notify();
        let api = self.state.api.clone();
        let path = format!("/v1/tasks/{}/run", query_value(&self.task_id));
        self._actions.push(cx.spawn(async move |this, cx| {
            let result = api.post::<_, Run>(&path, &CreateRun::default()).await;
            this.update(cx, |this, cx| {
                this.starting = false;
                this.start_note = Some(match result {
                    Ok(_) => {
                        this.state.data_changed(cx);
                        "Run started.".into()
                    }
                    Err(error) if error.is_not_implemented() => {
                        "Execution isn't available yet: the service stores the task but cannot run agents.".into()
                    }
                    Err(error) => ui::error_text(&error),
                });
                cx.notify();
            })
            .ok();
        }));
    }

    /// Rail rows for a run: recorded stages (with their steps as detail), else flat
    /// recorded steps, else the configured stages marked as not recorded.
    /// Each row carries the pending approval question it can answer, if any.
    fn steps(&self, run: &Run) -> Vec<(Step, Option<String>)> {
        if let Some(stages) = run.stages.as_ref().filter(|stages| !stages.is_empty()) {
            return stages.iter().map(|stage| {
                let stage_id = stage.stage_id.as_deref().unwrap_or(&stage.id);
                let question = pending(run).find(|question| question.kind == "approval" && question.stage_id == stage_id);
                // The agent actually executing now, from the run, not the task's assignee.
                let agent = stage.steps.iter().find(|step| step.status == RunStepStatus::Running)
                    .and_then(|step| step.agent_id.as_deref()).map(|id| self.agent_label(id));
                let step = Step {
                    id: stage.id.clone(),
                    title: stage.title.clone(),
                    agent,
                    state: step_state(&stage.status),
                    verb: status_word(&stage.status).to_string(),
                    elapsed: elapsed(stage.started_at.as_deref(), stage.finished_at.as_deref()),
                    detail: stage.steps.iter().map(|step| (self.step_line(step), status_word(&step.status).to_string())).collect(),
                    // Only a manual approval gate gets rail buttons; permission requests and forms
                    // are answered once, on their own card.
                    request: question.map(|question| question.prompt.clone()),
                };
                (step, question.map(|question| question.id.clone()))
            }).collect();
        }
        if let Some(steps) = run.steps.as_ref().filter(|steps| !steps.is_empty()) {
            return steps.iter().map(|step| (Step {
                id: step.id.clone(),
                title: step.title.clone(),
                agent: step.agent_id.as_deref().map(|id| self.agent_label(id)),
                state: step_state(&step.status),
                verb: status_word(&step.status).to_string(),
                elapsed: elapsed(step.started_at.as_deref(), step.finished_at.as_deref()),
                detail: step.output.iter().map(|output| (output.clone(), String::new())).collect(),
                request: None,
            }, None)).collect();
        }
        self.stages.iter().enumerate().map(|(index, stage)| (Step {
            id: format!("configured-{index}"),
            title: stage.name.clone(),
            agent: stage.agents.first().map(|(name, harness)| format!("{name} · {harness}")),
            state: StepState::NotRecorded,
            verb: "Not recorded".into(),
            elapsed: None,
            detail: stage.agents.iter().map(|(name, harness)| (name.clone(), harness.clone())).collect(),
            request: None,
        }, None)).collect()
    }

    fn step_line(&self, step: &RunStep) -> String {
        let mut line = step_line(step);
        if let Some(attempt) = step.attempts.iter().map(|attempt| attempt.number).max().filter(|number| *number > 1) {
            line.push_str(&format!(" · attempt {attempt}"));
        }
        if step.retry_at.is_some() { line.push_str(" · retry scheduled") }
        match (&step.agent_id, &step.error) {
            (_, Some(error)) => format!("{line} · {}", error.message),
            (Some(agent), None) => format!("{line} · {}", self.agent_label(agent)),
            _ => line,
        }
    }

    /// `POST /v1/runs/:id/{pause|resume|cancel|retry}` with the run's revision.
    fn control(&mut self, run: &Run, action: &'static str, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/runs/{}/{action}", query_value(&run.id));
        // A running run bumps its revision with live output, so pause/cancel there send none
        // and the service checks the state itself; stable runs keep the revision guard.
        let live = matches!(run.run_state(), RunState::Queued | RunState::Running) && matches!(action, "pause" | "cancel");
        let body = RunControl { revision: if live { None } else { run.revision } };
        self._actions.push(cx.spawn(async move |this, cx| {
            let result = api.post::<_, Run>(&path, &body).await;
            this.update(cx, |this, cx| {
                this.start_note = match result {
                    Ok(_) => { this.state.data_changed(cx); None }
                    Err(error) => Some(ui::service_message(&error)),
                };
                this.reload(cx);
                cx.notify();
            }).ok();
        }));
    }

    /// Queues guidance; the service adds it to the run's next agent steps.
    fn instruct(&mut self, run_id: &str, message: String, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/runs/{}/instructions", query_value(run_id));
        // Append-only, so no revision: live output would make any loaded revision stale.
        let body = AddRunInstruction { message, revision: None };
        self.instructing = None;
        self._actions.push(cx.spawn(async move |this, cx| {
            let result = api.post::<_, Run>(&path, &body).await;
            this.update(cx, |this, cx| {
                this.start_note = match result {
                    Ok(_) => { this.state.data_changed(cx); None }
                    Err(error) => Some(ui::service_message(&error)),
                };
                this.reload(cx);
                cx.notify();
            }).ok();
        }));
    }

    /// Queued and delivered guidance, plus the input while open. Pending guidance on a
    /// finished run stays listed as not applied.
    fn render_instructions(&self, run: &Run, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme_tokens::tokens(cx);
        let (active, open) = (run.is_active(), self.instructing.as_deref() == Some(run.id.as_str()));
        div().flex().flex_col().gap_1()
            .children(run.instructions.iter().map(|instruction| {
                // How it reached the agent, from the service's delivery receipt.
                let mode = instruction.deliveries.last().map(|delivery| match delivery.mode.as_str() {
                    "native-turn" => "Delivered live to the running agent",
                    "native-tool-boundary" => "Delivered at the agent's next tool call",
                    _ => "Delivered to the next agent step",
                });
                let status = match (&instruction.delivered_at, active) {
                    (Some(_), _) => mode.unwrap_or("Delivered"),
                    (None, true) => "Queued for the next agent step",
                    (None, false) => "Not applied: the run ended first",
                };
                div().flex().items_start().gap_2().t_caption()
                    .child(div().flex_1().min_w_0().text_color(t.text).child(instruction.message.clone()))
                    .child(div().flex_none().text_color(t.text_secondary).child(status))
            }))
            .when(open, |this| this.child(div().flex().flex_col().gap_1()
                .child(components::input_md(Input::new(&self.instruction_input)))
                .child(div().t_caption().text_color(t.text_secondary).child("Reaches the running agent when its harness accepts live guidance; otherwise the next agent step."))))
    }

    /// Outputs the run recorded: documents and code tours open in a tab; files show their path.
    fn render_artifacts(&self, run: &Run, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme_tokens::tokens(cx);
        let project = run.project_id.clone().unwrap_or_else(|| self.project_id.clone());
        div().flex().flex_col().gap_0p5()
            .when(!run.artifacts.is_empty(), |this| this.child(div().t_caption().text_color(t.text_secondary).child("Outputs")))
            .children(run.artifacts.iter().map(|artifact| {
                let target = artifact.document_id.clone().map(|id| match artifact.kind.as_str() {
                    "code_tour" => TabTarget::review(project.clone(), id),
                    _ => TabTarget::document(project.clone(), id),
                });
                let state = self.state.clone();
                div().flex().items_center().gap_2().t_label()
                    .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(artifact.name.clone()))
                    .children(artifact.path.clone().filter(|_| target.is_none()).map(|path| div().min_w_0().truncate().t_caption().text_color(t.text_secondary).child(path)))
                    .children(target.map(|target| Button::new(SharedString::from(format!("artifact-{}", artifact.id))).small().t_label().ghost().label("Open")
                        .on_click(move |_, window, cx| state.open(target.clone(), window, cx))))
            }))
    }

    /// Answers a run question. Approvals take `approve` / `reject`; the service fails the gate
    /// on anything but approval and follows the stage's failure route.
    /// Keeps one trace component per loaded run (expand/select state survives refreshes).
    fn sync_traces(&mut self, cx: &mut Context<Self>) {
        let runs = self.runs.ready().cloned().unwrap_or_default();
        self.traces.retain(|id, _| runs.iter().any(|run| &run.id == id));
        for run in runs {
            match self.traces.get(&run.id) {
                Some(view) => view.update(cx, |view, cx| { view.set_run(run); cx.notify() }),
                None => { self.traces.insert(run.id.clone(), cx.new(|_| components::TraceView::new(run))); }
            }
        }
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        ui::card(cx).p_4().flex().flex_col().gap_1()
            .child(div().t_label().font_weight(FontWeight::MEDIUM).child("No runs yet"))
            .child(div().t_body().text_color(cx.theme().muted_foreground).child(
                "Runs appear here with a step for each workflow stage, live status and approval requests.",
            ))
            .when(!self.stages.is_empty(), |this| {
                this.child(div().flex().flex_col().gap_1().children(self.stages.iter().map(|stage| {
                    div().flex().items_center().gap_2().t_caption().text_color(cx.theme().muted_foreground)
                        .child(step_glyph(StepState::NotRecorded, cx))
                        .child(stage.name.clone())
                        .children(stage.agents.first().map(|(name, _)| div().ml_auto().child(name.clone())))
                })))
            })
            .children(self.start_note.clone().map(|note| div().w_full().min_w_0().whitespace_normal()
                .t_caption().text_color(cx.theme().muted_foreground).child(note)))
            .child(div().flex().justify_end()
                .child(Button::new("start-run").small().t_label().icon(IconName::Play).label("Start run").loading(self.starting)
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx)))))
    }

    fn render_controls(&self, run: &Run, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let state = run.run_state();
        // The service refuses to resume a run interrupted mid-step (its effects are unknown);
        // retry starts a new linked run and cancels this one.
        let interrupted = run.errors.iter().any(|error| error.code == "interrupted");
        let actions: &[(&'static str, &'static str)] = match state {
            RunState::Paused if interrupted => &[("retry", "Retry"), ("cancel", "Cancel")],
            RunState::Paused => &[("resume", "Resume"), ("cancel", "Cancel")],
            RunState::Queued | RunState::Running | RunState::Waiting if run.pause_requested => &[("cancel", "Cancel")],
            RunState::Queued | RunState::Running | RunState::Waiting => &[("pause", "Pause"), ("cancel", "Cancel")],
            RunState::Failed | RunState::Cancelled => &[("retry", "Retry")],
            _ => &[],
        };
        let instruct = run.is_active().then(|| {
            let id = run.id.clone();
            Button::new(SharedString::from(format!("run-instruct-{}", run.id))).small().t_label().ghost().label("Add instruction")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.instructing = Some(id.clone());
                    this.instruction_input.update(cx, |input, cx| { input.set_value("", window, cx); input.focus(window, cx) });
                    cx.notify();
                }))
        });
        div().flex().items_center().gap_1()
            .when(interrupted && state == RunState::Paused, |this| this.child(div().t_caption().text_color(cx.theme().muted_foreground)
                .child("Interrupted mid-step; check its effects, then retry or cancel.")))
            .children(instruct)
            .children(actions.iter().map(|(action, label)| {
            let run = run.clone();
            Button::new(SharedString::from(format!("run-{action}-{}", run.id))).small().t_label().ghost().label(*label)
                .on_click(cx.listener(move |this, _, _, cx| this.control(&run, action, cx)))
        }))
    }

    fn render_run(&self, run: &Run, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let steps = self.steps(run);
        // Processed visits, including handled failures and skips, so a finished run reads n/n.
        let count = steps.len();
        let done = run.stages.as_ref().filter(|stages| !stages.is_empty()).map_or_else(
            || steps.iter().filter(|(step, _)| step.state == StepState::Done).count(),
            |stages| stages.iter().filter(|stage| matches!(stage.status,
                RunStepStatus::Completed | RunStepStatus::Failed | RunStepStatus::Skipped | RunStepStatus::Cancelled)).count(),
        );
        let on_rail: Vec<String> = steps.iter().filter_map(|(_, question)| question.clone()).collect();
        // A failed run leads with its errors. Otherwise errors were handled (e.g. a failed
        // check rerouted to repair): step errors already show on their step lines, and
        // run-level ones stay as muted history.
        let failed = run.run_state() == RunState::Failed;
        let errors: Vec<String> = run.errors.iter().filter(|error| failed || error.step_id.is_none()).map(|error| error.message.clone()).collect();
        let trigger = run.trigger.as_deref().filter(|trigger| *trigger != "manual").map(|trigger| ui::chip(trigger.to_string(), cx));
        div().flex().flex_col().rounded(px(metrics::R_CARD)).border_1().border_color(cx.theme().border).bg(theme_tokens::tokens(cx).surface).p_4().gap_1()
            .child(div().flex().items_center().gap_2().mb_1()
                .child(div().t_label().font_weight(FontWeight::MEDIUM).child(format!("Run {}", ui::short_date(&run.created_at))))
                .child(ui::chip(if run.pause_requested && run.is_active() { "pausing".to_string() } else { run.state.clone() }, cx))
                .children(trigger)
                .child(div().ml_auto().t_caption().text_color(cx.theme().muted_foreground).child(format!("{done}/{count}")))
                .child(self.render_controls(run, cx)))
            .child(self.render_questions(run, &on_rail, cx))
            .child(self.render_instructions(run, cx))
            .child(self.render_artifacts(run, cx))
            .children(errors.into_iter().map(|error| div().t_label()
                .text_color(if failed { cx.theme().danger } else { cx.theme().muted_foreground })
                .child(if failed { error } else { format!("Handled earlier: {error}") })))
            .when(steps.is_empty(), |this| this.child(div().t_label().text_color(cx.theme().muted_foreground).child("No step data recorded for this run.")))
            .children(steps.into_iter().enumerate().map(|(index, (step, question))| self.render_step(&run.id, question, index, index + 1 == count, step, window, cx)))
            .children(self.traces.get(&run.id).cloned().map(|view| div().mt_2().child(view)))
    }

    fn render_step(&self, run_id: &str, question: Option<String>, index: usize, last: bool, step: Step, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let key = format!("{run_id}:{index}:{}", step.id);
        let expanded = self.expanded.contains(&key);
        let (weak, toggle_key) = (cx.weak_entity(), key.clone());
        let on_toggle = move |_: &mut Window, cx: &mut App| {
            weak.update(cx, |this, cx| {
                if !this.expanded.remove(&toggle_key) { this.expanded.insert(toggle_key.clone()); }
                cx.notify();
            }).ok();
        };
        let (weak, run_id) = (cx.weak_entity(), run_id.to_string());
        let on_answer = move |answer: StepAnswer, _: String, window: &mut Window, cx: &mut App| {
            let Some(question) = question.clone() else {
                crate::components::toast_warning("This approval has no open question on the service.", window, cx);
                return;
            };
            let answer = match answer { StepAnswer::Approve => "approve", StepAnswer::Decline => "reject" };
            weak.update(cx, |this, cx| this.answer(&run_id, &question, Reply::Text(answer.into()), cx)).ok();
        };
        components::run_step(&key, step, expanded, last, on_toggle, on_answer, window, cx)
    }
}



impl Render for RunsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare_drafts(window, cx);
        match &self.runs {
            Load::Loading => ui::skeleton_rows(1, cx).into_any_element(),
            Load::Failed(error) => {
                let weak = cx.weak_entity();
                div().flex().items_center().justify_between().t_label().text_color(cx.theme().danger)
                    .child(error.clone())
                    .child(Button::new("retry-runs").small().t_label().label("Retry").on_click(move |_, _, cx| { weak.update(cx, |this, cx| this.reload(cx)).ok(); }))
                    .into_any_element()
            }
            Load::Ready(runs) if runs.is_empty() => self.render_empty(cx).into_any_element(),
            Load::Ready(runs) => {
                let runs = runs.clone();
                div().flex().flex_col().gap_2().children(runs.iter().map(|run| self.render_run(run, window, cx))).into_any_element()
            }
        }
    }
}

pub(super) fn pending(run: &Run) -> impl Iterator<Item = &RunQuestion> {
    run.questions.iter().filter(|question| question.state == QuestionState::Pending)
}

fn step_state(status: &RunStepStatus) -> StepState {
    match status {
        RunStepStatus::Completed => StepState::Done,
        RunStepStatus::Running => StepState::Running,
        RunStepStatus::NeedsApproval => StepState::NeedsYou,
        RunStepStatus::Failed => StepState::Failed,
        _ => StepState::Pending,
    }
}

fn status_word(status: &RunStepStatus) -> &str {
    match status {
        RunStepStatus::Completed => "Done",
        RunStepStatus::Running => "Running",
        RunStepStatus::NeedsApproval => "Needs approval",
        RunStepStatus::Failed => "Failed",
        RunStepStatus::Pending => "Pending",
        RunStepStatus::Cancelled => "Cancelled",
        RunStepStatus::Skipped => "Skipped",
        RunStepStatus::Unknown(raw) => raw,
    }
}

fn step_line(step: &RunStep) -> String {
    match &step.activity {
        Some(activity) => format!("{} · {activity}", step.title),
        None => step.title.clone(),
    }
}

fn format_ms(ms: f64) -> String {
    if ms < 1000. { format!("{ms:.0}ms") } else if ms < 60_000. { format!("{:.0}s", ms / 1000.) } else { format!("{:.0}m {:.0}s", (ms / 60_000.).floor(), (ms % 60_000.) / 1000.) }
}

/// Elapsed between two ISO timestamps (dates included).
fn elapsed(start: Option<&str>, end: Option<&str>) -> Option<String> {
    let (start, end) = (ui::epoch_seconds(start?)?, ui::epoch_seconds(end?)?);
    (end >= start).then(|| format_ms((end - start) * 1000.))
}

