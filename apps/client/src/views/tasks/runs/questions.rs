//! Run questions in the Runs panel: free-text questions, approvals, and harness permission
//! requests. A permission card shows the normalised tool request (command, working folder,
//! paths, diff, URLs) and the harness's own choices; structured fields compose `answers`
//! (multi-select as a list). Answers name the immutable question id and send no run
//! revision. Each question keeps its own draft, untouched by refreshes.

use std::collections::{BTreeMap, HashMap};

use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    switch::Switch,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{AnswerRunQuestion, QuestionField, QuestionTool, Run, RunQuestion};
use serde_json::Value;

use super::{RunsPanel, pending};
use crate::{api::query_value, components::{self, Typography as _}, theme::{self as theme_tokens, metrics}, views::tasks::ui};

#[derive(Clone)]
pub enum Reply {
    Text(String),
    /// A provider choice id with its declared effect (`approve`, `deny`, `cancel`).
    Choice(String, Option<String>),
}

/// What has been entered for one question.
#[derive(Default)]
pub struct QuestionDraft {
    /// Free-text answer (questions without fields), once opened.
    text: Option<Entity<InputState>>,
    /// Text and number fields.
    inputs: HashMap<String, Entity<InputState>>,
    /// Select, multi-select and boolean fields.
    values: HashMap<String, Value>,
    /// Per-field problems from the last attempt to send (the draft is kept).
    errors: HashMap<String, String>,
    /// Why the service refused the last answer.
    send_error: Option<String>,
}

impl RunsPanel {
    /// No run revision: the service answers the immutable question id atomically while other
    /// branches keep bumping the run's revision.
    pub(super) fn answer(&mut self, run_id: &str, question_id: &str, reply: Reply, cx: &mut Context<Self>) {
        // Deny / cancel always go through, without the form (whatever was typed into it).
        let refusal = match &reply {
            Reply::Choice(choice, Some(effect)) if matches!(effect.as_str(), "deny" | "cancel") => Some(AnswerRunQuestion { choice: Some(choice.clone()), ..Default::default() }),
            Reply::Text(answer) if answer == "reject" => Some(AnswerRunQuestion { answer: Some(answer.clone()), ..Default::default() }),
            _ => None,
        };
        if let Some(body) = refusal {
            self.send(run_id, question_id, body, cx);
            return;
        }
        // Everything else answers the form, so required fields and types must hold.
        let Some(answers) = self.validated(question_id, true, cx) else { return };
        // Typed form answers travel as `value` (`answers` only takes strings).
        let value = answers.map(|answers| Value::Object(answers.into_iter().collect()));
        let body = match reply {
            Reply::Text(answer) => AnswerRunQuestion { answer: Some(answer), value, ..Default::default() },
            Reply::Choice(choice, _) => AnswerRunQuestion { choice: Some(choice), value, ..Default::default() },
        };
        self.send(run_id, question_id, body, cx);
    }

    fn submit_fields(&mut self, run_id: &str, question: &RunQuestion, cx: &mut Context<Self>) {
        let Some(answers) = self.validated(&question.id, true, cx) else { return };
        let value = Value::Object(answers.unwrap_or_default().into_iter().collect());
        self.send(run_id, &question.id, AnswerRunQuestion { value: Some(value), ..Default::default() }, cx);
    }

    fn question(&self, question_id: &str) -> Option<RunQuestion> {
        self.runs.ready()?.iter().flat_map(|run| run.questions.iter()).find(|question| question.id == question_id).cloned()
    }

    /// Typed field answers, or `None` (nothing sent) after recording per-field errors.
    /// `require` checks required fields.
    fn validated(&mut self, question_id: &str, require: bool, cx: &mut Context<Self>) -> Option<Option<BTreeMap<String, Value>>> {
        let fields = self.question(question_id).map_or_else(Vec::new, |question| question.fields);
        let (answers, mut errors) = self.compose(question_id, &fields, cx);
        if require {
            for field in fields.iter().filter(|field| field.required && !answers.contains_key(&field.id)) {
                errors.entry(field.id.clone()).or_insert_with(|| "Needs an answer.".into());
            }
        }
        let ok = errors.is_empty();
        self.drafts.entry(question_id.into()).or_default().errors = errors;
        cx.notify();
        ok.then(|| (!answers.is_empty()).then_some(answers))
    }

    fn send(&mut self, run_id: &str, question_id: &str, body: AnswerRunQuestion, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/runs/{}/questions/{}/answer", query_value(run_id), query_value(question_id));
        let question = question_id.to_string();
        self._actions.push(cx.spawn(async move |this, cx| {
            let result = api.post::<_, Run>(&path, &body).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => { this.drafts.remove(&question); this.state.data_changed(cx) }
                    // Shown on the question's own card; the draft stays so nothing is lost.
                    Err(error) => this.drafts.entry(question.clone()).or_default().send_error = Some(ui::service_message(&error)),
                }
                this.reload(cx);
                cx.notify();
            }).ok();
        }));
    }

    /// Field answers in their JSON types: text exactly as typed, numbers as JSON numbers,
    /// booleans (unset shows and sends `false`), option ids, and multi-select id lists.
    fn compose(&self, question_id: &str, fields: &[QuestionField], cx: &App) -> (BTreeMap<String, Value>, HashMap<String, String>) {
        let (mut answers, mut errors) = (BTreeMap::new(), HashMap::new());
        let draft = self.drafts.get(question_id);
        let read = |key: &str| draft.and_then(|draft| draft.inputs.get(key)).map(|input| input.read(cx).value().to_string());
        for field in fields {
            let value = draft.and_then(|draft| draft.values.get(&field.id)).cloned();
            let text = read(&field.id).filter(|text| !text.trim().is_empty());
            let other = read(&other_key(&field.id)).map(|text| text.trim().to_string()).filter(|text| !text.is_empty() && field.allow_other);
            let answer = match field.kind.as_str() {
                "boolean" => Ok(Some(value.unwrap_or(Value::Bool(false)))),
                "number" | "integer" => text.map_or(Ok(None), |text| numeric(field, text.trim()).map(Some)),
                // A typed "Other" answer replaces the chosen option.
                "select" => Ok(other.map(Value::String).or(value)),
                "multiselect" => {
                    let mut items = value.and_then(|value| value.as_array().cloned()).unwrap_or_default();
                    items.extend(other.map(Value::String));
                    items_in_range(field, items.len()).map(|()| (!items.is_empty()).then_some(Value::Array(items)))
                }
                _ => text.map_or(Ok(None), |text| length_in_range(field, text.chars().count()).map(|()| Some(Value::String(text)))),
            };
            match answer {
                Ok(Some(value)) => { answers.insert(field.id.clone(), value); }
                Ok(None) => {}
                Err(error) => { errors.insert(field.id.clone(), error); }
            }
        }
        (answers, errors)
    }

    /// Opens (or focuses) the free-text answer for a question.
    fn open_text(&mut self, run_id: String, question_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.drafts.get(&question_id).and_then(|draft| draft.text.clone());
        let input = existing.unwrap_or_else(|| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Your answer, then Enter"));
            let (run, question) = (run_id.clone(), question_id.clone());
            cx.subscribe(&input, move |this: &mut Self, input, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::PressEnter { .. }) { return }
                let text = input.read(cx).value().trim().to_string();
                if !text.is_empty() { this.answer(&run, &question, Reply::Text(text), cx) }
            }).detach();
            self.drafts.entry(question_id.clone()).or_default().text = Some(input.clone());
            input
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Creates the inputs a question's text/number fields need (once).
    fn ensure_inputs(&mut self, question: &RunQuestion, window: &mut Window, cx: &mut Context<Self>) {
        for field in &question.fields {
            let key = match field.kind.as_str() {
                "text" | "number" | "integer" => field.id.clone(),
                "select" | "multiselect" if field.allow_other => other_key(&field.id),
                _ => continue,
            };
            if self.drafts.get(&question.id).is_some_and(|draft| draft.inputs.contains_key(&key)) { continue }
            let placeholder = if key == field.id { field.label.clone() } else { "Other…".to_string() };
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            self.drafts.entry(question.id.clone()).or_default().inputs.insert(key, input);
        }
    }

    fn set_value(&mut self, question: &str, field: &str, value: Value, cx: &mut Context<Self>) {
        self.drafts.entry(question.into()).or_default().values.insert(field.into(), value);
        cx.notify();
    }

    fn toggle_option(&mut self, question: &str, field: &str, option: &str, cx: &mut Context<Self>) {
        let values = &mut self.drafts.entry(question.into()).or_default().values;
        let list = values.entry(field.into()).or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(items) = list {
            match items.iter().position(|item| item.as_str() == Some(option)) {
                Some(index) => { items.remove(index); }
                None => items.push(Value::String(option.into())),
            }
        }
        cx.notify();
    }

    /// Pending questions not shown on a stage row (free-text questions, permission requests,
    /// approvals for a stage the rail doesn't list), each with its own controls and draft.
    /// Creates field inputs for every pending question before rendering.
    pub(super) fn prepare_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let questions: Vec<RunQuestion> = self.runs.ready().map_or_else(Vec::new, |runs| runs.iter().flat_map(|run| pending(run).cloned()).collect());
        for question in &questions { self.ensure_inputs(question, window, cx) }
    }

    pub(super) fn render_questions(&self, run: &Run, on_rail: &[String], cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let questions: Vec<RunQuestion> = pending(run).filter(|question| !on_rail.contains(&question.id)).cloned().collect();
        div().flex().flex_col().gap_2().children(questions.into_iter().map(|question| self.render_question(&run.id, question, cx)))
    }

    fn render_question(&self, run_id: &str, question: RunQuestion, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme_tokens::tokens(cx);
        let id = question.id.clone();
        let caption = match question.kind.as_str() {
            "approval" => "Approval needed".to_string(),
            "permission" => question.provider.as_ref().map_or("Permission request".to_string(), |provider| format!("{provider} asks for permission")),
            _ => "Question".to_string(),
        };
        let live = question.transport.as_deref() == Some("live");
        let reply = |label: String, reply: Reply, primary: bool, danger: bool, key: String| {
            let (run, qid) = (run_id.to_string(), id.clone());
            Button::new(SharedString::from(format!("q-{key}-{qid}"))).small().t_label().label(label)
                .map(|button| if primary { button.primary() } else if danger { button.danger() } else { button.ghost() })
                .on_click(cx.listener(move |this, _, _, cx| this.answer(&run, &qid, reply.clone(), cx)))
        };
        let has_fields = !question.fields.is_empty();
        let buttons = if !question.choices.is_empty() {
            // The harness's own choices; allow-type is primary, deny-type is marked as such.
            div().flex().flex_wrap().justify_end().gap_1().children(question.choices.iter().map(|choice| {
                let effect = choice.effect.as_deref();
                reply(choice.label.clone(), Reply::Choice(choice.id.clone(), choice.effect.clone()), effect == Some("approve"), effect == Some("deny"), choice.id.clone())
            }))
        } else if question.kind == "approval" {
            div().flex().gap_1()
                .child(reply("Reject".into(), Reply::Text("reject".into()), false, false, "reject".into()))
                .child(reply("Approve".into(), Reply::Text("approve".into()), true, false, "approve".into()))
        } else if has_fields {
            let (run, item) = (run_id.to_string(), question.clone());
            div().child(Button::new(SharedString::from(format!("q-submit-{id}"))).small().t_label().primary().label("Send")
                .on_click(cx.listener(move |this, _, _, cx| this.submit_fields(&run, &item, cx))))
        } else {
            let (run, qid) = (run_id.to_string(), id.clone());
            div().child(Button::new(SharedString::from(format!("q-answer-{id}"))).small().t_label().outline().label("Answer")
                .on_click(cx.listener(move |this, _, window, cx| this.open_text(run.clone(), qid.clone(), window, cx))))
        };
        let details = match (&question.tool, &question.payload) {
            (Some(tool), _) => Some(tool_details(tool, cx).into_any_element()),
            (None, Some(payload)) => Some(payload_details(payload, cx).into_any_element()),
            _ => None,
        };
        let text = self.drafts.get(&id).and_then(|draft| draft.text.clone());
        let send_error = self.drafts.get(&id).and_then(|draft| draft.send_error.clone());
        div().flex().flex_col().gap_2().p_3().rounded(px(metrics::R_INNER)).border_1().border_color(ui::needs_you(cx).opacity(0.45))
            .child(div().flex().items_start().gap_2()
                .child(div().flex_1().min_w_0().flex().flex_col().gap_0p5()
                    .child(div().t_caption().text_color(t.text_secondary).child(if live { format!("{caption} · waiting live") } else { caption }))
                    .child(div().t_body().text_color(t.text).child(question.prompt.clone())))
                .child(buttons))
            .children(details)
            .when(has_fields, |this| this.child(self.render_fields(&question, cx)))
            .children(text.map(|text| components::input_md(Input::new(&text))))
            .children(send_error.map(|error| div().t_label().text_color(t.danger).child(format!("Couldn't send: {error}"))))
    }

    fn render_fields(&self, question: &RunQuestion, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let draft = self.drafts.get(&question.id);
        let danger = theme_tokens::tokens(cx).danger;
        div().flex().flex_col().gap_2().children(question.fields.iter().map(|field| {
            let control = self.render_field(&question.id, field, draft, cx);
            let error = draft.and_then(|draft| draft.errors.get(&field.id)).cloned();
            let other = draft.and_then(|draft| draft.inputs.get(&other_key(&field.id))).cloned();
            ui::field_owned(format!("{}{}", field.label, if field.required { "" } else { " (optional)" }), control, cx)
                .children(other.map(|other| components::input_md(Input::new(&other))))
                .children(constraint_hint(field).map(|hint| div().t_caption().text_color(theme_tokens::tokens(cx).text_secondary).child(hint)))
                .children(error.map(|error| div().t_caption().text_color(danger).child(error)))
        }))
    }

    fn render_field(&self, question: &str, field: &QuestionField, draft: Option<&QuestionDraft>, cx: &mut Context<Self>) -> AnyElement {
        let value = draft.and_then(|draft| draft.values.get(&field.id)).cloned();
        let (qid, fid) = (question.to_string(), field.id.clone());
        match field.kind.as_str() {
            "boolean" => {
                let on = value.and_then(|value| value.as_bool()).unwrap_or(false);
                Switch::new(SharedString::from(format!("qf-{qid}-{fid}"))).checked(on)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| this.set_value(&qid, &fid, Value::Bool(*checked), cx))).into_any_element()
            }
            "select" => {
                let current = value.and_then(|value| value.as_str().map(str::to_string));
                div().flex().flex_wrap().gap_1().children(field.options.iter().map(|option| {
                    let (qid, fid, oid) = (qid.clone(), fid.clone(), option.id.clone());
                    let on = current.as_deref() == Some(option.id.as_str());
                    components::button_sm(Button::new(SharedString::from(format!("qf-{qid}-{fid}-{oid}"))).label(option.label.clone()))
                        .map(|button| if on { button.outline() } else { button.ghost() })
                        .on_click(cx.listener(move |this, _, _, cx| this.set_value(&qid, &fid, Value::String(oid.clone()), cx)))
                })).into_any_element()
            }
            "multiselect" => {
                let chosen: Vec<String> = value.and_then(|value| value.as_array().cloned()).unwrap_or_default().iter().filter_map(|item| item.as_str().map(str::to_string)).collect();
                div().flex().flex_col().children(field.options.iter().map(|option| {
                    let (qid, fid, oid) = (qid.clone(), fid.clone(), option.id.clone());
                    crate::views::workflow::editor::toggle_row(SharedString::from(format!("qf-{qid}-{fid}-{oid}")), chosen.contains(&option.id), option.label.clone(), None, cx)
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_option(&qid, &fid, &oid, cx)))
                })).into_any_element()
            }
            _ => match draft.and_then(|draft| draft.inputs.get(&field.id)) {
                Some(input) => components::input_md(Input::new(input)).into_any_element(),
                None => div().into_any_element(),
            },
        }
    }
}

fn other_key(field: &str) -> String {
    format!("{field}#other")
}

/// Number or integer within the field's range.
fn numeric(field: &QuestionField, text: &str) -> Result<Value, String> {
    let integer = field.kind == "integer";
    let value = number(text).filter(|value| !integer || value.is_i64()).ok_or(if integer { "Enter a whole number." } else { "Enter a number." })?;
    let n = value.as_f64().unwrap_or_default();
    match (field.minimum, field.maximum) {
        (Some(min), _) if n < min => Err(format!("At least {min}.")),
        (_, Some(max)) if n > max => Err(format!("At most {max}.")),
        _ => Ok(value),
    }
}

fn length_in_range(field: &QuestionField, length: usize) -> Result<(), String> {
    match (field.min_length, field.max_length) {
        (Some(min), _) if length < min => Err(format!("At least {min} characters.")),
        (_, Some(max)) if length > max => Err(format!("At most {max} characters.")),
        _ => Ok(()),
    }
}

/// An empty multi-select is "unanswered" (required decides), not a range error.
fn items_in_range(field: &QuestionField, count: usize) -> Result<(), String> {
    match (field.min_items, field.max_items) {
        (Some(min), _) if count > 0 && count < min => Err(format!("Choose at least {min}.")),
        (_, Some(max)) if count > max => Err(format!("Choose at most {max}.")),
        _ => Ok(()),
    }
}

/// Short limits shown under a field, e.g. "1–5" or "Up to 3".
fn constraint_hint(field: &QuestionField) -> Option<String> {
    let range = |min: Option<String>, max: Option<String>, unit: &str| match (min, max) {
        (Some(min), Some(max)) => Some(format!("{min}–{max}{unit}")),
        (Some(min), None) => Some(format!("At least {min}{unit}")),
        (None, Some(max)) => Some(format!("Up to {max}{unit}")),
        _ => None,
    };
    match field.kind.as_str() {
        "number" | "integer" => range(field.minimum.map(|n| n.to_string()), field.maximum.map(|n| n.to_string()), ""),
        "multiselect" => range(field.min_items.map(|n| n.to_string()), field.max_items.map(|n| n.to_string()), " choices"),
        "text" => range(field.min_length.map(|n| n.to_string()), field.max_length.map(|n| n.to_string()), " characters"),
        _ => None,
    }
}

/// A finite JSON number; integers stay integers.
fn number(text: &str) -> Option<Value> {
    if let Ok(integer) = text.parse::<i64>() { return Some(Value::from(integer)) }
    text.parse::<f64>().ok().filter(|value| value.is_finite()).and_then(serde_json::Number::from_f64).map(Value::Number)
}

#[cfg(test)]
mod tests {
    #[test]
    fn numbers_are_typed_and_invalid_input_is_rejected() {
        assert_eq!(super::number("42"), Some(serde_json::json!(42)));
        assert_eq!(super::number("1.5"), Some(serde_json::json!(1.5)));
        assert_eq!(super::number("inf"), None);
        assert_eq!(super::number("12abc"), None);
    }

    #[test]
    fn integers_are_whole_and_in_range() {
        let field: protocol::QuestionField = serde_json::from_value(serde_json::json!({"id": "count", "label": "Count", "type": "integer", "minimum": 0, "maximum": 5})).unwrap();
        assert_eq!(super::numeric(&field, "0"), Ok(serde_json::json!(0)));
        assert!(super::numeric(&field, "2.5").is_err());
        assert!(super::numeric(&field, "6").is_err());
    }
}

/// The tool request as rows: what it is, then command / folder / paths / URLs / diff.
fn tool_details(tool: &QuestionTool, cx: &App) -> impl IntoElement + use<> {
    let t = theme_tokens::tokens(cx);
    let kind = match tool.kind.as_str() {
        "shell" => "Run a command",
        "read" => "Read files",
        "write" => "Change files",
        "network" => "Use the network",
        "mcp" => "Use an MCP tool",
        _ => "Use a tool",
    };
    let code = |text: String| div().p_2().rounded(px(metrics::R_CONTROL)).bg(t.tray).t_code_caption(cx).text_color(t.text).child(text.chars().take(4000).collect::<String>());
    let row = |label: &'static str, body: AnyElement| div().flex().flex_col().gap_0p5()
        .child(div().t_caption().text_color(t.text_secondary).child(label)).child(body);
    div().flex().flex_col().gap_1p5()
        .child(div().t_label().text_color(t.text).child(format!("{kind} · {}", tool.name)))
        .children(tool.command.clone().map(|command| row("Command", code(command).into_any_element())))
        .children(tool.cwd.clone().map(|cwd| row("In folder", div().t_label().child(cwd).into_any_element())))
        .when(!tool.paths.is_empty(), |this| this.child(row("Paths", code(tool.paths.join("\n")).into_any_element())))
        .when(!tool.urls.is_empty(), |this| this.child(row("URLs", code(tool.urls.join("\n")).into_any_element())))
        .children(tool.diff.clone().map(|diff| row("Changes", code(diff).into_any_element())))
}

/// Fallback when the harness gave no normalised tool: top-level fields as text.
fn payload_details(payload: &Value, cx: &App) -> impl IntoElement + use<> {
    let t = theme_tokens::tokens(cx);
    let text = |value: &Value| match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(|item| item.as_str().map_or_else(|| item.to_string(), str::to_string)).collect::<Vec<_>>().join("\n"),
        other => other.to_string(),
    };
    let entries: Vec<(String, String)> = match payload {
        Value::Object(map) => map.iter().filter(|(_, value)| !value.is_null()).take(8).map(|(key, value)| (key.clone(), text(value))).collect(),
        other => vec![(String::new(), text(other))],
    };
    div().flex().flex_col().gap_1().children(entries.into_iter().map(move |(key, value)| {
        let long = value.len() > 60 || value.contains('\n');
        let shown: String = value.chars().take(1200).collect();
        div().flex().flex_col().gap_0p5()
            .when(!key.is_empty(), |this| this.child(div().t_caption().text_color(t.text_secondary).child(key)))
            .child(if long {
                div().p_2().rounded(px(metrics::R_CONTROL)).bg(t.tray).t_code_caption(cx).text_color(t.text).child(shown).into_any_element()
            } else {
                div().t_label().text_color(t.text).child(shown).into_any_element()
            })
    }))
}
