//! Task → linked GitHub/Linear item: the external link, and an explicit Push when the
//! project allows it. Push sends the saved task; the request id is reused when retrying an
//! uncertain push, so the service can de-duplicate it.

use gpui_kit::component::{
    Disableable as _,
    button::Button,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{IntegrationPushInput, IntegrationPushResult, Task};

use crate::{api::query_value, components::{self, Typography as _}, state::AppState, theme};
use super::{new_id, ui};

pub struct SourceLink {
    state: AppState,
    task: Option<Task>,
    /// `github` / `linear` when the project syncs, and whether Push is allowed.
    provider: Option<String>,
    push_enabled: bool,
    /// The project's `pushFields` (title + description when unset).
    fields: Vec<String>,
    dirty: bool,
    pushing: bool,
    result: Option<Result<IntegrationPushResult, String>>,
    /// The last push whose outcome isn't known (uncertain, or the reply was lost). Retry
    /// resends exactly this request, same id, revision and fields, until a definite answer.
    unresolved: Option<IntegrationPushInput>,
    _task: Option<gpui_kit::Task<()>>,
}

pub const DEFAULT_PUSH_FIELDS: [&str; 2] = ["title", "description"];

/// A reply that proves nothing about whether the remote write happened.
fn indefinite(result: &Result<IntegrationPushResult, crate::api::ApiError>) -> bool {
    match result {
        Ok(result) => result.status == "uncertain",
        Err(crate::api::ApiError::Unavailable(_)) | Err(crate::api::ApiError::Decode(_)) => true,
        Err(crate::api::ApiError::Status { status, .. }) => *status >= 500 || *status == 408,
    }
}

impl SourceLink {
    pub fn new(state: AppState) -> Self {
        Self {
            state, task: None, provider: None, push_enabled: false, fields: Vec::new(), dirty: false, pushing: false,
            result: None, unresolved: None, _task: None,
        }
    }

    pub fn set(&mut self, task: Option<Task>, provider: Option<String>, push_enabled: bool, fields: Option<Vec<String>>, dirty: bool) {
        self.task = task;
        self.provider = provider;
        self.push_enabled = push_enabled;
        self.fields = fields.unwrap_or_else(|| DEFAULT_PUSH_FIELDS.iter().map(|field| field.to_string()).collect());
        self.dirty = dirty;
    }

    pub fn is_shown(&self) -> bool {
        self.provider.is_some() && (self.push_enabled || self.unresolved.is_some() || self.task.as_ref().is_some_and(|task| task.external.is_some()))
    }

    fn push(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.task.clone() else { return };
        // An unresolved push is retried as the same request, never re-built from the task.
        let body = self.unresolved.clone().unwrap_or_else(|| IntegrationPushInput {
            revision: task.revision, request_id: new_id("push"), fields: Some(self.fields.clone()), create_missing: Some(task.external.is_none()),
        });
        self.unresolved = Some(body.clone());
        self.pushing = true;
        cx.notify();
        let api = self.state.api.clone();
        let path = format!("/v1/tasks/{}/push", query_value(&task.id));
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = api.post::<_, IntegrationPushResult>(&path, &body).await;
            this.update(cx, |this, cx| {
                this.pushing = false;
                if !indefinite(&result) { this.unresolved = None }
                if result.is_ok() { this.state.data_changed(cx) }
                this.result = Some(result.map_err(|error| ui::service_message(&error)));
                cx.notify();
            }).ok();
        }));
    }
}

impl Render for SourceLink {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::tokens(cx);
        let Some(provider) = self.provider.clone().filter(|_| self.is_shown()) else { return div() };
        let name = if provider == "github" { "GitHub" } else { "Linear" };
        let external = self.task.as_ref().and_then(|task| task.external.clone());
        let retry = self.unresolved.is_some();
        let note = match (&self.result, retry) {
            (_, true) => Some(("The last push wasn't confirmed. Retry resends that same push (same fields and version), so it can't be applied twice.".to_string(), true)),
            (None, _) => None,
            (Some(Err(error)), _) => Some((error.clone(), true)),
            (Some(Ok(result)), _) => Some(match result.status.as_str() {
                "pushed" => ("Pushed.".to_string(), false),
                "unchanged" => ("Already up to date.".to_string(), false),
                _ => (result.errors.first().map_or("Push failed.".to_string(), |error| error.message.clone()), true),
            }),
        };
        let no_fields = self.fields.is_empty() && !retry;
        let label = if retry { "Retry same push".to_string() } else { format!("Push to {name}") };
        div().flex().flex_col().gap_1p5()
            .children(external.map(|external| {
                let url = external.url.clone();
                div().id("task-external").flex().items_center().gap_1p5().t_label().text_color(t.text).cursor_pointer()
                    .hover(move |style| style.text_color(t.running))
                    .child(div().truncate().child(format!("{name} {}", external.identifier)))
                    .on_click(move |_, _, cx| cx.open_url(&url))
            }))
            .when(self.push_enabled || retry, |this| this.child(div().flex().items_center().gap_2()
                .child(components::button_sm(Button::new("task-push").outline().label(label))
                    .loading(self.pushing).disabled(self.pushing || (!retry && (self.dirty || no_fields)))
                    .on_click(cx.listener(|this, _, _, cx| this.push(cx))))
                .when(!retry && self.dirty, |this| this.child(div().t_caption().text_color(t.text_secondary).child("Save first")))
                .when(!retry && !self.dirty && no_fields, |this| this.child(div().t_caption().text_color(t.text_secondary).child("Choose fields to push in Settings → Sync")))))
            .when(!retry && self.push_enabled && !no_fields, |this| this.child(div().t_caption().text_color(t.text_secondary)
                .child(format!("Pushes {}.", self.fields.iter().map(|field| field_label(field)).collect::<Vec<_>>().join(", ")))))
            .children(note.map(|(text, problem)| div().t_caption().text_color(if problem { t.danger } else { t.text_secondary }).child(text)))
    }
}

pub fn field_label(field: &str) -> &str {
    match field {
        "title" => "title",
        "description" => "description",
        "status" => "status",
        "priority" => "priority",
        "labels" => "labels",
        "dueAt" => "due date",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use crate::api::ApiError;
    use protocol::IntegrationPushResult;

    fn ok(status: &str) -> Result<IntegrationPushResult, ApiError> {
        Ok(IntegrationPushResult { status: status.into(), external: None, errors: Vec::new() })
    }

    /// Lost replies and uncertain results keep the request for an identical retry.
    #[test]
    fn only_definite_outcomes_release_the_request() {
        assert!(super::indefinite(&ok("uncertain")));
        assert!(super::indefinite(&Err(ApiError::Unavailable("timeout".into()))));
        assert!(super::indefinite(&Err(ApiError::Status { status: 502, body: None })));
        assert!(!super::indefinite(&ok("pushed")));
        assert!(!super::indefinite(&Err(ApiError::Status { status: 409, body: None })));
    }
}
