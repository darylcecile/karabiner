//! Shared draft of one project's configuration (`GET|PUT /v1/projects/:id/config`).
//! The Workflow and Settings tabs edit the same draft; saving sends the whole object with
//! the base revision, and unknown fields round-trip through `extra`.
//!
//! Undo/redo keeps whole-config snapshots. Each `edit` is one step; `edit_grouped` merges
//! consecutive edits with the same group (typing in one field). No-op edits record nothing.

use gpui_kit::*;
use protocol::ProjectConfig;

use crate::{api::query_value, state::AppState, views::tasks::ui::{self, Load}};

const HISTORY_LIMIT: usize = 100;

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
    undo: Vec<ProjectConfig>,
    redo: Vec<ProjectConfig>,
    /// Group of the last recorded step; a matching `edit_grouped` extends it.
    group: Option<String>,
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
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
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

    /// Mutates the draft (created from the server copy on first edit). A mutation that leaves
    /// the config as it was is not an edit: no dirty flag, generation bump or notify. This
    /// keeps late input echoes (inputs re-synced from a saved config) from re-dirtying it.
    pub fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut ProjectConfig)) {
        self.record(None, cx, f);
    }

    /// Like `edit`, but consecutive edits with the same `group` are one undo step.
    pub fn edit_grouped(&mut self, group: String, cx: &mut Context<Self>, f: impl FnOnce(&mut ProjectConfig)) {
        self.record(Some(group), cx, f);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let (Some(previous), Some(current)) = (self.undo.pop(), self.config().cloned()) else { return };
        self.redo.push(current);
        self.restore(previous, cx);
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        let (Some(next), Some(current)) = (self.redo.pop(), self.config().cloned()) else { return };
        self.undo.push(current);
        self.restore(next, cx);
    }

    fn record(&mut self, group: Option<String>, cx: &mut Context<Self>, f: impl FnOnce(&mut ProjectConfig)) {
        let Some(current) = self.config().cloned() else { return };
        let mut next = current.clone();
        f(&mut next);
        if next == current {
            return;
        }
        if group.is_none() || group != self.group {
            self.undo.push(current);
            if self.undo.len() > HISTORY_LIMIT { self.undo.remove(0); }
        }
        self.redo.clear();
        self.group = group;
        self.set_current(next, cx);
    }

    /// A history snapshot keeps its content but takes the current base revision, so undoing
    /// across a save doesn't look like a stale write.
    fn restore(&mut self, mut snapshot: ProjectConfig, cx: &mut Context<Self>) {
        if let Some(current) = self.config() { snapshot.revision = current.revision }
        self.group = None;
        self.set_current(snapshot, cx);
    }

    fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.group = None;
    }

    fn set_current(&mut self, next: ProjectConfig, cx: &mut Context<Self>) {
        self.generation += 1;
        // Reverting to the saved copy is clean again (not while a save is in flight, whose
        // result would otherwise replace what the user just restored).
        if !self.saving && self.server.ready() == Some(&next) {
            self.draft = None;
            self.dirty = false;
            self.conflict = false;
        } else {
            self.draft = Some(next);
            self.dirty = true;
        }
        cx.notify();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let path = format!("/v1/projects/{}/config", query_value(&self.project_id));
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = api.get::<ProjectConfig>(&path).await;
            this.update(cx, |this, cx| {
                match result {
                    // Unchanged on the service (most SSE invalidations are run updates): keep
                    // the draft, validation and canvas state untouched and don't notify.
                    Ok(config) if this.error.is_none() && this.server.ready() == Some(&config) => return,
                    Ok(config) => {
                        this.error = None;
                        let base = this.draft.as_ref().map(|draft| draft.revision);
                        if this.dirty {
                            this.conflict = base != Some(config.revision);
                        } else {
                            this.draft = None;
                            // A newer remote copy replaces the clean one; old steps no longer apply.
                            if this.server.ready().is_some_and(|server| server.revision != config.revision) { this.clear_history() }
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
        self.clear_history();
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
        self.group = None;
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
