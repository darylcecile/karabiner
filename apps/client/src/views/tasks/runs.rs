//! Run inspection (ref B457FF32 step tracker): stage rail with done / running / needs-you /
//! pending glyphs, collapsible steps and an inline approval panel. Runs come from
//! `GET /v1/runs?taskId=`; the service records none until execution exists, so the no-runs
//! state is the normal state today. Nothing here fabricates activity.

use std::collections::HashSet;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{CreateIntervention, Intervention, InterventionKind, Run, RunStep, RunStepStatus};

use super::ui::{self, Load};

use crate::{api::query_value, state::AppState};
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
    _load: Option<gpui_kit::Task<()>>,
    /// Start-run and approve/decline writes; independent of refreshes.
    _actions: Vec<gpui_kit::Task<()>>,
    _subscription: Subscription,
}

impl RunsPanel {
    pub fn new(state: AppState, project_id: String, task_id: String, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state.service, |this: &mut Self, service, cx| {
            let version = service.read(cx).data_version();
            if version != this.seen_version {
                this.seen_version = version;
                this.reload(cx);
            }
        });
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
            _load: None,
            _actions: Vec::new(),
            _subscription: subscription,
        };
        this.reload(cx);
        this
    }

    pub fn set_stages(&mut self, stages: Vec<StageRow>) {
        self.stages = stages;
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/runs?taskId={}", query_value(&self.task_id));
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = api.get::<Vec<Run>>(&path).await;
            this.update(cx, |this, cx| {
                this.runs = Load::from_result(result);
                this.sync_traces(cx);
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
            let result = api.post::<_, Run>(&path, &serde_json::json!({})).await;
            this.update(cx, |this, cx| {
                this.starting = false;
                this.start_note = Some(match result {
                    Ok(_) => {
                        this.state.data_changed(cx);
                        "Run requested.".into()
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
    fn steps(&self, run: &Run) -> Vec<Step> {
        if let Some(stages) = run.stages.as_ref().filter(|stages| !stages.is_empty()) {
            return stages.iter().map(|stage| Step {
                id: stage.id.clone(),
                title: stage.title.clone(),
                agent: None,
                state: step_state(&stage.status),
                verb: status_word(&stage.status).to_string(),
                elapsed: elapsed(stage.started_at.as_deref(), stage.finished_at.as_deref()),
                detail: stage.steps.iter().map(|step| (step_line(step), status_word(&step.status).to_string())).collect(),
                request: stage.steps.iter().find(|step| step.status == RunStepStatus::NeedsApproval)
                    .map(|step| step.activity.clone().unwrap_or_else(|| step.title.clone())),
            }).collect();
        }
        if let Some(steps) = run.steps.as_ref().filter(|steps| !steps.is_empty()) {
            return steps.iter().map(|step| Step {
                id: step.id.clone(),
                title: step.title.clone(),
                agent: step.agent_id.clone(),
                state: step_state(&step.status),
                verb: status_word(&step.status).to_string(),
                elapsed: elapsed(step.started_at.as_deref(), step.finished_at.as_deref()),
                detail: step.output.iter().map(|output| (output.clone(), String::new())).collect(),
                request: (step.status == RunStepStatus::NeedsApproval).then(|| step.activity.clone().unwrap_or_else(|| step.title.clone())),
            }).collect();
        }
        self.stages.iter().enumerate().map(|(index, stage)| Step {
            id: format!("configured-{index}"),
            title: stage.name.clone(),
            agent: stage.agents.first().map(|(name, harness)| format!("{name} · {harness}")),
            state: StepState::NotRecorded,
            verb: "Not recorded".into(),
            elapsed: None,
            detail: stage.agents.iter().map(|(name, harness)| (name.clone(), harness.clone())).collect(),
            request: None,
        }).collect()
    }

    /// Approve / decline become stored intervention intents for the run's task.
    fn answer(&mut self, kind: InterventionKind, message: String, cx: &mut Context<Self>) {
        let body = CreateIntervention { project_id: self.project_id.clone(), task_id: Some(self.task_id.clone()), kind, message };
        let api = self.state.api.clone();
        self._actions.push(cx.spawn(async move |this, cx| {
            let result = api.post::<_, Intervention>("/v1/interventions", &body).await;
            this.update(cx, |this, cx| {
                this.start_note = Some(match result {
                    Ok(_) => { this.state.data_changed(cx); "Decision recorded. It is delivered once execution is available.".into() }
                    Err(error) => ui::error_text(&error),
                });
                cx.notify();
            }).ok();
        }));
    }

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
            .child(div().flex().items_center().justify_between().gap_2()
                .child(div().t_caption().text_color(cx.theme().muted_foreground)
                    .children(self.start_note.clone()))
                .child(Button::new("start-run").small().t_label().icon(IconName::Play).label("Start run").loading(self.starting)
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx)))))
    }

    fn render_run(&self, run: &Run, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let steps = self.steps(run);
        let done = steps.iter().filter(|step| step.state == StepState::Done).count();
        let count = steps.len();
        div().flex().flex_col().rounded(px(metrics::R_CARD)).border_1().border_color(cx.theme().border).bg(theme_tokens::tokens(cx).surface).p_4().gap_1()
            .child(div().flex().items_center().gap_2().mb_1()
                .child(div().t_label().font_weight(FontWeight::MEDIUM).child(format!("Run {}", ui::short_date(&run.created_at))))
                .child(ui::chip(run.state.clone(), cx))
                .child(div().ml_auto().t_caption().text_color(cx.theme().muted_foreground).child(format!("{done}/{}", steps.len()))))
            .when(steps.is_empty(), |this| this.child(div().t_label().text_color(cx.theme().muted_foreground).child("No step data recorded for this run.")))
            .children(steps.into_iter().enumerate().map(|(index, step)| self.render_step(&run.id, index, index + 1 == count, step, window, cx)))
            .children(self.traces.get(&run.id).cloned().map(|view| div().mt_2().child(view)))
    }

    fn render_step(&self, run_id: &str, index: usize, last: bool, step: Step, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let key = format!("{run_id}:{index}:{}", step.id);
        let expanded = self.expanded.contains(&key);
        let (weak, toggle_key) = (cx.weak_entity(), key.clone());
        let on_toggle = move |_: &mut Window, cx: &mut App| {
            weak.update(cx, |this, cx| {
                if !this.expanded.remove(&toggle_key) { this.expanded.insert(toggle_key.clone()); }
                cx.notify();
            }).ok();
        };
        let weak = cx.weak_entity();
        let on_answer = move |answer: StepAnswer, request: String, _: &mut Window, cx: &mut App| {
            let (kind, message) = match answer {
                StepAnswer::Approve => (InterventionKind::Approve, format!("Approved: {request}")),
                StepAnswer::Decline => (InterventionKind::Reject, format!("Declined: {request}")),
            };
            weak.update(cx, |this, cx| this.answer(kind, message, cx)).ok();
        };
        components::run_step(&key, step, expanded, last, on_toggle, on_answer, window, cx)
    }
}



impl Render for RunsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

