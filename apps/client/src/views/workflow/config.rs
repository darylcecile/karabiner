//! Shared draft of one project's configuration (`GET|PUT /v1/projects/:id/config`).
//! The Workflow and Settings tabs edit the same draft; saving sends the whole object with
//! the base revision, and unknown fields round-trip through `extra`.

use gpui_kit::*;
use protocol::ProjectConfig;

use crate::{api::query_value, state::AppState, views::tasks::ui::{self, Load}};

pub struct ConfigDraft {
    state: AppState,
    project_id: String,
    server: Load<ProjectConfig>,
    draft: Option<ProjectConfig>,
    pub dirty: bool,
    pub conflict: bool,
    pub saving: bool,
    pub error: Option<String>,
    /// Bumped per edit; a save adopts the response only if nothing changed meanwhile.
    generation: u64,
    seen_version: u64,
    _task: Option<gpui_kit::Task<()>>,
    _subscription: Subscription,
}

impl ConfigDraft {
    pub fn new(state: AppState, project_id: String, cx: &mut Context<Self>) -> Self {
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
            server: Load::Loading,
            draft: None,
            dirty: false,
            conflict: false,
            saving: false,
            error: None,
            generation: 0,
            _task: None,
            _subscription: subscription,
        };
        this.reload(cx);
        this
    }

    pub fn status(&self) -> &Load<ProjectConfig> {
        &self.server
    }

    /// Draft when editing, else the server copy.
    pub fn config(&self) -> Option<&ProjectConfig> {
        self.draft.as_ref().or(self.server.ready())
    }

    /// Mutates the draft (created from the server copy on first edit).
    pub fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut ProjectConfig)) {
        if self.draft.is_none() {
            self.draft = self.server.ready().cloned();
        }
        if let Some(draft) = self.draft.as_mut() {
            f(draft);
            self.dirty = true;
            self.generation += 1;
            cx.notify();
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/projects/{}/config", query_value(&self.project_id));
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = api.get::<ProjectConfig>(&path).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(config) => {
                        let base = this.draft.as_ref().map(|draft| draft.revision);
                        if this.dirty {
                            this.conflict = base != Some(config.revision);
                        } else {
                            this.draft = None;
                        }
                        this.server = Load::Ready(config);
                    }
                    Err(error) if this.server.ready().is_some() => this.error = Some(format!("Couldn't refresh: {}", ui::error_text(&error))),
                    Err(error) => this.server = Load::Failed(ui::error_text(&error)),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn discard(&mut self, cx: &mut Context<Self>) {
        self.draft = None;
        self.dirty = false;
        self.conflict = false;
        self.error = None;
        self.reload(cx);
    }

    /// Conflict resolution "Keep mine": rebase the draft on the latest revision and save.
    pub fn keep_mine(&mut self, cx: &mut Context<Self>) {
        if let (Some(draft), Some(server)) = (self.draft.as_mut(), self.server.ready()) {
            draft.revision = server.revision;
        }
        self.conflict = false;
        self.start_save(cx).detach();
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        self.start_save(cx).detach();
    }

    /// Sends the draft; resolves `Ok` once the service accepted it (or nothing was dirty).
    pub fn start_save(&mut self, cx: &mut Context<Self>) -> gpui_kit::Task<anyhow::Result<()>> {
        let Some(draft) = self.draft.clone().filter(|_| self.dirty) else { return gpui_kit::Task::ready(Ok(())) };
        if self.conflict {
            // Never rebase silently (e.g. from the close prompt); the user must choose.
            return gpui_kit::Task::ready(Err(anyhow::anyhow!("Configuration changed elsewhere; choose Reload or Keep mine first.")));
        }
        let (submitted, base) = (self.generation, draft.revision);
        self.saving = true;
        self.error = None;
        cx.notify();
        let api = self.state.api.clone();
        let path = format!("/v1/projects/{}/config", query_value(&self.project_id));
        cx.spawn(async move |this, cx| {
            let result = api.put::<_, ProjectConfig>(&path, &draft).await;
            // 409 is either a stale revision or a dependency refusal (e.g. removing an agent a
            // task uses). Fetch the latest copy (draft kept) to tell them apart.
            let latest = match &result {
                Err(error) if error.is_conflict() => api.get::<ProjectConfig>(&path).await.ok(),
                _ => None,
            };
            let outcome = result.as_ref().map(|_| ()).map_err(|error| anyhow::anyhow!(ui::service_message(error)));
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(config) if this.generation == submitted => {
                        this.server = Load::Ready(config);
                        this.draft = None;
                        this.dirty = false;
                        this.state.data_changed(cx);
                    }
                    Ok(config) => {
                        // Edited while saving: keep the newer draft on the saved revision.
                        if let Some(draft) = this.draft.as_mut() { draft.revision = config.revision }
                        this.server = Load::Ready(config);
                        this.state.data_changed(cx);
                    }
                    Err(error) if error.is_conflict() => match latest {
                        Some(config) if config.revision != base => {
                            this.conflict = true;
                            this.server = Load::Ready(config);
                        }
                        _ => this.error = Some(ui::service_message(&error)),
                    },
                    Err(error) => this.error = Some(ui::error_text(&error)),
                }
                cx.notify();
            })
            .ok();
            outcome
        })
    }
}
