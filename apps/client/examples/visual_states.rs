//! Static specimen of shared states the running app rarely reaches: the real
//! `components::{empty_state, error_state, loading_state, toast_warning, toast_error}` and
//! the window banners (`shell::banner`, Retry is a no-op here),
//! rendered with the app's own theme and assets. No service calls; every error is a
//! constructed `ApiError`, so captures show product rendering, not a live failure. The
//! Run trace view feeds the real `components::TraceView` an in-memory, labelled QA fixture.
//!
//! `cargo build -p karabiner-client --example visual_states --locked`, then run
//! `target/debug/examples/visual_states`. It opens its own window beside the app.
#![allow(dead_code)]

#[path = "../src/api/mod.rs"]
mod api;
#[path = "../src/assets.rs"]
mod assets;
#[path = "../src/shell/banner.rs"]
mod banner;
#[path = "../src/components/mod.rs"]
mod components;
#[path = "../src/views/docs/banner.rs"]
mod doc_banner;
#[path = "../src/views/projects/documents_menu.rs"]
mod documents_menu;
#[path = "../src/views/docs/templates.rs"]
mod doc_templates;
#[path = "../src/views/tasks/intervention_card.rs"]
mod intervention_card;
#[path = "../src/motion.rs"]
mod motion;
#[path = "../src/theme.rs"]
mod theme;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use std::collections::HashSet;

use gpui_kit::*;
use protocol::{ApiErrorBody, InterventionKind, Run, RunStepStatus, SpanKind, Theme as ThemePreference, TraceSpan};

use api::ApiError;
use components::{
    button_sm, empty_state, error_state, loading_state, segmented, toast_error, toast_warning, Segment, Tone,
    Step, StepAnswer, StepState, TraceView, Typography as _, run_step,
};

struct Specimen {
    dark: bool,
    /// 0 = Status states, 1 = Run trace, 2 = Run steps.
    view: usize,
    trace: Entity<TraceView>,
    /// Expanded step ids and the last sample answer (local only, nothing is sent).
    expanded: HashSet<String>,
    answer: Option<String>,
    /// Local Retry clicks on the sample Documents error menu (nothing is requested).
    retries: usize,
}

/// One step per `StepState`, as a QA fixture for the real `run_step` row.
fn step_fixture() -> Vec<Step> {
    let step = |id: &str, title: &str, agent: Option<&str>, state, verb: &str, elapsed: Option<&str>| Step {
        id: id.into(),
        title: title.into(),
        agent: agent.map(Into::into),
        state,
        verb: verb.into(),
        elapsed: elapsed.map(Into::into),
        detail: Vec::new(),
        request: None,
    };
    let mut done = step("plan", "Plan the launch", Some("Planner"), StepState::Done, "Done", Some("6.9s"));
    done.detail = vec![("Output".into(), "Launch plan with 4 milestones".into()), ("Tokens".into(), "4.1k".into())];
    let mut needs = step("review", "Review pricing copy", Some("Writer"), StepState::NeedsYou, "Needs you", Some("2m"));
    needs.request = Some("Approve the draft pricing copy before publishing?".into());
    vec![
        done,
        step("draft", "Draft plan copy", Some("Writer"), StepState::Running, "Running", Some("23s")),
        needs,
        step("tests", "Run checkout tests", Some("Coder"), StepState::Failed, "Failed", Some("8.4s")),
        step("publish", "Publish pricing page", None, StepState::Pending, "Pending", None),
        step("qa", "QA sign-off", None, StepState::NotRecorded, "Not recorded", None),
    ]
}

fn status(status: u16, code: &str, message: &str) -> ApiError {
    let body = ApiErrorBody { code: code.into(), message: message.into(), details: None };
    ApiError::Status { status, body: Some(body) }
}

fn sample_errors() -> [(&'static str, ApiError); 4] {
    [
        ("Unavailable", ApiError::Unavailable("connection refused (127.0.0.1:43173)".into())),
        ("Conflict 409", status(409, "conflict", "The task changed since you opened it.")),
        ("Not implemented 501", status(501, "not_implemented", "Runs are not available yet.")),
        ("Request 500", status(500, "internal", "Unexpected error while saving.")),
    ]
}

/// A fixed-size card so each state renders in a contained area, as inside a view.
fn card(label: &'static str, body: impl IntoElement, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().t_caption().text_color(t.text_secondary).child(label))
        .child(div().w(px(320.)).h(px(180.)).rounded(px(12.)).border_1().border_color(t.border).bg(t.canvas).child(body))
}

impl Specimen {
    fn status_states(&self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        let toasts = sample_errors().into_iter().enumerate().map(|(ix, (label, error))| {
            button_sm(Button::new(("toast", ix)).outline().label(label))
                .on_click(move |_, window, cx| { toast_error(&error, window, cx); })
        });
        let banners = div()
            .w(px(660.))
            .flex()
            .flex_col()
            .child(div().t_caption().text_color(t.text_secondary).child("shell::banner · persist / service / incompatible"))
            .child(banner::banner("Couldn't save the tab layout: disk is read-only.".into(), None, Tone::Attention, banner::retry_button("b-persist"), cx))
            .child(banner::banner("Service unavailable: connection refused (127.0.0.1:43173)".into(), Some("Service unavailable"), Tone::Danger, banner::retry_button("b-service"), cx))
            .child(banner::banner("Service speaks API v2; this client needs v1.".into(), Some("Incompatible service"), Tone::Danger, banner::retry_button("b-incompatible"), cx));
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(banners)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .child(card("loading_state", loading_state("Loading tasks…", cx), cx))
                    .child(card("error_state", error_state(ApiError::Unavailable("connection refused".into()).to_string(), |_, _, _| {}), cx))
                    .child(card("empty_state", empty_state("No tasks yet", "Create a task to start planning this project."), cx))
                    .child(card("empty_state · missing / unsupported", empty_state("This document no longer exists", "It may have been deleted, or it uses a format this version can't open."), cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(button_sm(Button::new("toast-warning").outline().label("Warning toast")).on_click(|_, window, cx| {
                        toast_warning("Newer edits are unsaved; the tab was kept open.", window, cx)
                    }))
                    .children(toasts),
            )
            .child(self.doc_banners(cx))
            .child(self.documents_unavailable(cx))
            .child(self.interventions(cx))
    }

    /// The production project-toolbar Documents menu (`documents_menu`) in its load-error
    /// state: sample error text, no documents, the real template list. Retry only counts
    /// clicks here; open/new are no-ops.
    fn documents_unavailable(&self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        let weak = cx.weak_entity();
        let actions = documents_menu::DocumentsMenuActions {
            on_retry: std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
                weak.update(cx, |this, cx| { this.retries += 1; cx.notify() }).ok();
            }),
            on_open: std::rc::Rc::new(|_: usize, _: &mut Window, _: &mut App| {}),
            on_new: std::rc::Rc::new(|_: usize, _: &mut Window, _: &mut App| {}),
        };
        let templates = doc_templates::TEMPLATES.iter().map(|template| template.name).collect();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().t_caption().text_color(t.text_secondary).child("Project toolbar · Documents unavailable (sample error, QA fixture)"))
            .child(div().flex().items_center().gap_3()
                .child(documents_menu::documents_menu(Vec::new(), Some("service returned 503".into()), templates, actions))
                .child(div().t_caption().text_color(t.text_secondary)
                    .child(format!("Retry clicked {} time{}", self.retries, if self.retries == 1 { "" } else { "s" }))))
    }

    /// One real `intervention_card` per kind (QA fixture; nothing is delivered), with
    /// no-op actions in the shared buttons so the card is seen with its action row.
    fn interventions(&self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        let kinds = [
            (InterventionKind::Question, "Which pricing tier should the launch page lead with?"),
            (InterventionKind::Approve, "Approve publishing the pricing page."),
            (InterventionKind::Pause, "Pause until the legal review lands."),
            (InterventionKind::Instruction, "Use the brand voice doc for all headings."),
            (InterventionKind::Reject, "Reject the draft; the tone is off."),
            (InterventionKind::Resume, "Resume after the review."),
            (InterventionKind::Retry, "Retry the checkout tests."),
            (InterventionKind::Cancel, "Cancel this run."),
            (InterventionKind::Unknown("escalate".into()), "Unrecognised kind from a newer service."),
        ];
        let cards = kinds.into_iter().enumerate().map(|(ix, (kind, message))| {
            // Same row as TaskDetail: right-aligned, Dismiss then Answer.
            let actions = div()
                .pt_1()
                .flex()
                .justify_end()
                .gap_1()
                .child(button_sm(Button::new(("iv-dismiss", ix)).ghost().label("Dismiss")))
                .child(button_sm(Button::new(("iv-answer", ix)).outline().label("Answer")));
            div().w(px(320.)).child(intervention_card::card(&kind, message, cx).child(actions))
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().t_caption().text_color(t.text_secondary).child("views::tasks::intervention_card · QA fixture, no live execution, actions are no-ops"))
            .child(div().flex().flex_wrap().gap_4().children(cards))
    }

    /// The real document banner in its three states, with the editor's buttons as no-ops.
    fn doc_banners(&self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        div()
            .w(px(660.))
            .flex()
            .flex_col()
            .gap_2()
            .child(div().t_caption().text_color(t.text_secondary).child("views::docs::banner · conflict / save failed / delete confirm · QA fixture, no document changes"))
            .child(
                doc_banner::banner("Changed elsewhere 2 min ago. Your edits are kept until you choose.".into(), t.attention, cx)
                    .child(button_sm(Button::new("doc-reload").outline().label("Reload")))
                    .child(button_sm(Button::new("doc-keep").primary().label("Keep mine"))),
            )
            .child(
                doc_banner::banner("Couldn't save: Service unavailable: connection refused".into(), t.danger, cx)
                    .child(button_sm(Button::new("doc-retry").outline().label("Retry"))),
            )
            .child(
                doc_banner::banner("Delete “Launch plan”? This can't be undone.".into(), t.danger, cx)
                    .child(button_sm(Button::new("doc-cancel").outline().label("Cancel")))
                    .child(button_sm(Button::new("doc-delete").danger().label("Delete"))),
            )
    }

    fn run_trace(&self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().t_caption().text_color(t.text_secondary).child("QA fixture · in-memory Run, read-only, not live execution"))
            // TraceView paints its own card; a bordered frame here would double its contour.
            .child(self.trace.clone())
    }
}

impl Specimen {
    fn run_steps(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx);
        let mut rail = div().flex().flex_col().w(px(560.)).p_4().rounded(px(12.)).border_1().border_color(t.border).bg(t.canvas);
        let steps = step_fixture();
        let count = steps.len();
        for (index, step) in steps.into_iter().enumerate() {
            let id = step.id.clone();
            let expanded = self.expanded.contains(&id);
            let (toggle, answer) = (cx.entity().downgrade(), cx.entity().downgrade());
            let key = id.clone();
            rail = rail.child(run_step(
                &id,
                step,
                expanded,
                index + 1 == count,
                move |_, cx| {
                    toggle.update(cx, |this, cx| {
                        if !this.expanded.remove(&key) {
                            this.expanded.insert(key.clone());
                        }
                        cx.notify();
                    }).ok();
                },
                move |choice, text, _, cx| {
                    answer.update(cx, |this, cx| {
                        let verb = if choice == StepAnswer::Approve { "Approve" } else { "Decline" };
                        this.answer = Some(format!("Sample only, nothing sent: {verb} · {text}"));
                        cx.notify();
                    }).ok();
                },
                window,
                cx,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().t_caption().text_color(t.text_secondary).child("QA fixture · one step per state, read-only, not live execution"))
            .child(rail)
            .children(self.answer.clone().map(|answer| div().t_caption().text_color(t.text_secondary).child(answer)))
    }
}

impl Render for Specimen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let view = cx.entity().downgrade();
        let tabs = segmented(
            "specimen-view",
            vec![Segment::new("Status states"), Segment::new("Run trace"), Segment::new("Run steps")],
            self.view,
            move |index, _, cx| {
                view.update(cx, |this, cx| {
                    this.view = index;
                    cx.notify();
                })
                .ok();
            },
            cx,
        );
        let theme_toggle = button_sm(Button::new("theme").ghost().label(if self.dark { "Light" } else { "Dark" })).on_click(
            cx.listener(|this, _, window, cx| {
                this.dark = !this.dark;
                let mode = if this.dark { ThemePreference::Dark } else { ThemePreference::Light };
                theme::apply(mode, window, cx);
                cx.notify();
            }),
        );
        let body = match self.view {
            0 => self.status_states(cx),
            1 => self.run_trace(cx),
            _ => self.run_steps(window, cx),
        };
        div()
            .id("specimen")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_6()
            .p_6()
            .bg(t.tray)
            .text_color(t.text)
            .child(div().flex().items_center().justify_between().child(tabs).child(theme_toggle))
            .child(body)
    }
}

/// Labelled QA fixture: an in-memory run for static visual checks only (example-only data). Never sent to or
/// stored by the service, and never shown as live execution.
fn trace_qa_fixture() -> Run {
    let span = |id: &str, parent: Option<&str>, name: &str, kind: SpanKind, status: RunStepStatus, start: f64, duration: Option<f64>| TraceSpan {
        id: id.into(),
        parent_id: parent.map(Into::into),
        name: name.into(),
        kind,
        status,
        start_offset_ms: start,
        duration_ms: duration,
    };
    use RunStepStatus::{Completed, Failed};
    Run {
        id: "qa-fixture-trace".into(),
        task_id: "qa-fixture".into(),
        workflow_id: "qa-fixture".into(),
        state: "completed".into(),
        created_at: "2026-11-25T00:00:00Z".into(),
        stages: None,
        steps: None,
        started_at: None,
        finished_at: None,
        activity: None,
        output: None,
        duration_ms: Some(82_000.),
        tokens: Some(99_700),
        cost: Some(0.49),
        cost_currency: Some("USD".into()),
        trace_spans: Some(vec![
            span("plan", None, "QA fixture · Plan", SpanKind::Stage, Completed, 0., Some(82_000.)),
            span("planner", Some("plan"), "Planner", SpanKind::Agent, Completed, 400., Some(6_900.)),
            span("writer", Some("plan"), "Writer", SpanKind::Agent, Completed, 8_000., Some(37_000.)),
            span("read", Some("writer"), "Read brand voice", SpanKind::Tool, Completed, 8_200., Some(3_600.)),
            span("draft", Some("writer"), "Draft plan copy", SpanKind::Agent, Completed, 12_000., Some(23_000.)),
            span("tests", Some("plan"), "Run checkout tests", SpanKind::Tool, Failed, 46_000., Some(9_000.)),
            span("gate", Some("plan"), "Review gate", SpanKind::Gate, Completed, 56_000., Some(4_000.)),
        ]),
        project_id: None,
        revision: None,
        updated_at: None,
        retry_of: None,
        current_stage_id: None,
        active_stage_ids: Vec::new(),
        questions: Vec::new(),
        instructions: Vec::new(),
        errors: Vec::new(),
        artifacts: Vec::new(),
        pause_requested: false,
        transition_count: None,
        trigger: None,
    }
}

fn main() {
    application().with_assets(assets::AppAssets).run(|cx| {
        init(cx);
        theme::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(None, size(px(1080.), px(760.)), cx)),
            ..Default::default()
        };
        if let Err(error) = open_window(options, cx, |window, cx| {
            theme::apply(ThemePreference::Light, window, cx);
            let trace = cx.new(|_| TraceView::new(trace_qa_fixture()));
            cx.new(|_| Specimen { dark: false, view: 0, trace, expanded: HashSet::from(["plan".into(), "review".into()]), answer: None, retries: 0 })
        }) {
            eprintln!("failed to open window: {error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
}
