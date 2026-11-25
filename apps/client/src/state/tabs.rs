//! Workspace: sidebar section + open work-item tabs. Open tabs stay mounted, so view state
//! (scroll, drafts, selection) survives switching. Tab layout persists to `/v1/workspace`.

use std::{collections::HashMap, rc::Rc};

use gpui_kit::*;
use protocol::{Id, TabKind, WorkspaceState, WorkspaceTab};

use super::{AppState, ServiceState};
use crate::{api::ApiClient, views};

const WORKSPACE_PATH: &str = "/v1/workspace";
const MAX_CLOSED: usize = 20;

/// Global destinations in the sidebar. Not tabs; one view each, kept alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Home,
    Inbox,
    Projects,
    Templates,
    Settings,
}

impl Section {
    pub const ALL: [Section; 5] = [Self::Home, Self::Inbox, Self::Projects, Self::Templates, Self::Settings];

    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Inbox => "Inbox",
            Self::Projects => "Projects",
            Self::Templates => "Templates",
            Self::Settings => "Settings",
        }
    }
}

/// A work item shown in a tab. Mirrors the contract's `WorkspaceTab` target.
/// `Review` targets a `code_tour` document id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TabTarget {
    pub kind: TabKind,
    pub id: Id,
    pub project_id: Id,
}

impl TabTarget {
    pub fn project(id: impl Into<Id>) -> Self {
        let id = id.into();
        Self { kind: TabKind::Project, project_id: id.clone(), id }
    }

    pub fn task(project_id: impl Into<Id>, id: impl Into<Id>) -> Self {
        Self { kind: TabKind::Task, id: id.into(), project_id: project_id.into() }
    }

    pub fn document(project_id: impl Into<Id>, id: impl Into<Id>) -> Self {
        Self { kind: TabKind::Document, id: id.into(), project_id: project_id.into() }
    }

    pub fn review(project_id: impl Into<Id>, document_id: impl Into<Id>) -> Self {
        Self { kind: TabKind::Review, id: document_id.into(), project_id: project_id.into() }
    }

    pub fn default_title(&self) -> SharedString {
        match self.kind {
            TabKind::Project => "Project",
            TabKind::Task => "Task",
            TabKind::Document => "Document",
            TabKind::Review => "Review",
            TabKind::Unknown(_) => "Unsupported item",
        }
        .into()
    }

    /// Stable key: `kind:id`. Used for persistence and element ids.
    pub fn tab_id(&self) -> Id {
        format!("{}:{}", self.kind.as_str(), self.id)
    }

    /// A fresh wire record for a newly opened tab.
    fn to_wire(&self, title: &SharedString) -> WorkspaceTab {
        WorkspaceTab {
            id: self.tab_id(),
            kind: self.kind.clone(),
            resource_id: self.id.clone(),
            project_id: self.project_id.clone(),
            title: title.to_string(),
            extra: Default::default(),
        }
    }

    fn from_wire(tab: &WorkspaceTab) -> Self {
        Self { kind: tab.kind.clone(), id: tab.resource_id.clone(), project_id: tab.project_id.clone() }
    }
}

/// Implemented by tab views that hold unsaved edits; register with
/// [`AppState::track_draft`](super::AppState::track_draft). Closing a dirty tab asks to
/// save, discard or cancel instead of silently dropping the draft.
pub trait Draft: Sized + 'static {
    fn is_dirty(&self, cx: &App) -> bool;
    /// Persist the draft. Resolve `Ok` only after the service accepted it.
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<anyhow::Result<()>>;
}

#[derive(Clone)]
pub struct DraftHandle {
    is_dirty: Rc<dyn Fn(&App) -> bool>,
    save: Rc<dyn Fn(&mut Window, &mut App) -> Task<anyhow::Result<()>>>,
}

impl DraftHandle {
    pub fn new<V: Draft>(entity: &Entity<V>) -> Self {
        let weak = entity.downgrade();
        let save_weak = weak.clone();
        Self {
            is_dirty: Rc::new(move |cx| weak.upgrade().is_some_and(|view| view.read(cx).is_dirty(cx))),
            save: Rc::new(move |window, cx| match save_weak.upgrade() {
                Some(view) => view.update(cx, |view, cx| view.save(window, cx)),
                None => Task::ready(Ok(())),
            }),
        }
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        (self.is_dirty)(cx)
    }

    pub fn save(&self, window: &mut Window, cx: &mut App) -> Task<anyhow::Result<()>> {
        (self.save)(window, cx)
    }
}

pub struct OpenTab {
    pub target: TabTarget,
    pub title: SharedString,
    pub view: AnyView,
    pub draft: Option<DraftHandle>,
    /// The stored record: its id and unknown fields are written back unchanged.
    wire: WorkspaceTab,
    /// In-tab navigation (`replace_active`), most recent last. Not persisted.
    back: Vec<(TabTarget, SharedString)>,
    forward: Vec<(TabTarget, SharedString)>,
}

impl OpenTab {
    pub fn is_dirty(&self, cx: &App) -> bool {
        self.draft.as_ref().is_some_and(|draft| draft.is_dirty(cx))
    }

    fn wire(&self) -> WorkspaceTab {
        current_wire(&self.wire, &self.title)
    }
}

/// The stored record with the current title; id, kind, target and unknown fields kept.
fn current_wire(wire: &WorkspaceTab, title: &SharedString) -> WorkspaceTab {
    WorkspaceTab { title: title.to_string(), ..wire.clone() }
}

/// Index of the stored active tab among open tabs, matched by stored id.
fn active_position(tabs: &[WorkspaceTab], active_id: Option<&str>) -> Option<usize> {
    active_id.and_then(|id| tabs.iter().position(|tab| tab.id == id))
}

#[derive(Default)]
struct Persistence {
    loaded: bool,
    revision: u64,
    saving: bool,
    pending: bool,
    extra: protocol::Extra,
    selected_project_id: Option<Id>,
    /// Last load/save failure, shown by the shell with a Retry.
    error: Option<SharedString>,
    task: Option<Task<()>>,
}

pub enum WorkspaceEvent {
    /// A tab left the strip; the shell animates its exit from this position.
    TabClosed { index: usize, target: TabTarget, title: SharedString },
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

pub struct Workspace {
    api: ApiClient,
    service: Entity<ServiceState>,
    section: Section,
    sections: HashMap<Section, AnyView>,
    tabs: Vec<OpenTab>,
    active: Option<usize>,
    closed: Vec<WorkspaceTab>,
    persistence: Persistence,
    _service_observer: Subscription,
}

impl Workspace {
    pub fn new(api: ApiClient, service: Entity<ServiceState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _service_observer = cx.observe_in(&service, window, |this, service, window, cx| {
            if !this.persistence.loaded && this.persistence.task.is_none() && service.read(cx).status().is_ready() {
                this.load(window, cx);
            }
        });
        Self {
            api,
            service,
            section: Section::Home,
            sections: HashMap::new(),
            tabs: Vec::new(),
            active: None,
            closed: Vec::new(),
            persistence: Persistence::default(),
            _service_observer,
        }
    }

    pub fn app_state(&self, cx: &Context<Self>) -> AppState {
        AppState { api: self.api.clone(), service: self.service.clone(), workspace: cx.weak_entity() }
    }

    pub fn api(&self) -> &ApiClient {
        &self.api
    }

    pub fn app_state_for(&self, entity: &Entity<Self>) -> AppState {
        AppState { api: self.api.clone(), service: self.service.clone(), workspace: entity.downgrade() }
    }

    pub fn section(&self) -> Section {
        self.section
    }

    pub fn tabs(&self) -> &[OpenTab] {
        &self.tabs
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn active(&self) -> Option<&OpenTab> {
        self.tabs.get(self.active?)
    }

    /// The view to show: the active tab, otherwise the current section (built lazily).
    pub fn content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyView {
        if let Some(tab) = self.active() {
            return tab.view.clone();
        }
        let section = self.section;
        if let Some(view) = self.sections.get(&section) {
            return view.clone();
        }
        let view = views::build_section(section, self.app_state(cx), window, cx);
        self.sections.insert(section, view.clone());
        view
    }

    pub fn show_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        self.active = None;
        cx.notify();
        self.save(cx);
    }

    /// Activates an existing tab for `target` or opens a new one after the active tab.
    pub fn open(&mut self, target: TabTarget, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.position(&target) {
            return self.activate(index, cx);
        }
        let tab = self.new_tab(target, None, window, cx);
        let index = self.active.map_or(self.tabs.len(), |active| active + 1);
        self.tabs.insert(index, tab);
        self.active = Some(index);
        cx.notify();
        self.save(cx);
    }

    /// Navigates the active tab to `target` in place (opens a tab if none is active).
    pub fn replace_active(&mut self, target: TabTarget, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return self.open(target, window, cx);
        };
        if let Some(existing) = self.position(&target) {
            return self.activate(existing, cx);
        }
        if self.tabs[active].is_dirty(cx) {
            // Never drop a draft by navigating in place; open alongside instead.
            return self.open(target, window, cx);
        }
        let replacement = self.new_tab(target, None, window, cx);
        let previous = std::mem::replace(&mut self.tabs[active], replacement);
        let tab = &mut self.tabs[active];
        tab.back = previous.back;
        tab.back.push((previous.target, previous.title));
        cx.notify();
        self.save(cx);
    }

    pub fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() && self.active != Some(index) {
            self.active = Some(index);
            cx.notify();
            self.save(cx);
        }
    }

    pub fn activate_offset(&mut self, offset: isize, cx: &mut Context<Self>) {
        let len = self.tabs.len() as isize;
        if len == 0 {
            return;
        }
        let current = self.active.map_or(-1, |active| active as isize);
        self.activate((current + offset).rem_euclid(len) as usize, cx);
    }

    /// Closes the tab, first asking to save/discard/cancel when it has unsaved edits.
    pub fn request_close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        match tab.draft.clone().filter(|draft| draft.is_dirty(cx)) {
            Some(draft) => {
                let target = tab.target.clone();
                let title = tab.title.clone();
                crate::shell::close_prompt::open(cx.weak_entity(), target, title, draft, window, cx);
            }
            None => self.close(index, cx),
        }
    }

    pub fn close_target(&mut self, target: &TabTarget, cx: &mut Context<Self>) {
        if let Some(index) = self.position(target) {
            self.close(index, cx);
        }
    }

    /// Closes without checking drafts. Use [`Self::request_close`] from user actions.
    pub fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        self.closed.push(tab.wire());
        cx.emit(WorkspaceEvent::TabClosed { index, target: tab.target.clone(), title: tab.title.clone() });
        if self.closed.len() > MAX_CLOSED {
            self.closed.remove(0);
        }
        self.active = match self.active {
            Some(_) if self.tabs.is_empty() => None,
            Some(active) if active > index || active == self.tabs.len() => Some(active - 1),
            other => other,
        };
        cx.notify();
        self.save(cx);
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.active {
            self.request_close(index, window, cx);
        }
    }

    /// Moves a tab (drag reorder); keeps the same tab active.
    pub fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return;
        }
        let active = self.active().map(|tab| tab.target.clone());
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = active.and_then(|target| self.position(&target));
        cx.notify();
        self.save(cx);
    }

    pub fn track_draft(&mut self, target: &TabTarget, draft: DraftHandle) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| &tab.target == target) {
            tab.draft = Some(draft);
        }
    }

    /// Any open tab with unsaved edits.
    pub fn has_dirty_tabs(&self, cx: &App) -> bool {
        self.tabs.iter().any(|tab| tab.is_dirty(cx))
    }

    pub fn reopen_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wire) = self.closed.pop() else { return };
        let target = TabTarget::from_wire(&wire);
        if let Some(index) = self.position(&target) {
            return self.activate(index, cx);
        }
        let tab = self.tab_from_wire(wire, window, cx);
        let index = self.active.map_or(self.tabs.len(), |active| active + 1);
        self.tabs.insert(index, tab);
        self.active = Some(index);
        cx.notify();
        self.save(cx);
    }

    pub fn set_title(&mut self, target: &TabTarget, title: SharedString, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| &tab.target == target) else {
            return;
        };
        if tab.title != title {
            tab.title = title;
            cx.notify();
            self.save(cx);
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.active().is_some_and(|tab| !tab.back.is_empty())
    }

    pub fn can_go_forward(&self) -> bool {
        self.active().is_some_and(|tab| !tab.forward.is_empty())
    }

    /// Steps the active tab through its in-place navigation history.
    pub fn navigate_history(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active else { return };
        if self.tabs[active].is_dirty(cx) {
            crate::components::toast_warning("Save or discard your changes before navigating away.", window, cx);
            return;
        }
        let tab = &self.tabs[active];
        let next = if forward { tab.forward.last() } else { tab.back.last() };
        let Some((target, title)) = next.cloned() else { return };
        if let Some(index) = self.position(&target) {
            // Already open in another tab: switch to it rather than duplicating.
            return self.activate(index, cx);
        }
        let mut replacement = self.new_tab(target, Some(title), window, cx);
        let tab = &mut self.tabs[active];
        let (mut back, mut forward_stack) = (std::mem::take(&mut tab.back), std::mem::take(&mut tab.forward));
        let current = (tab.target.clone(), tab.title.clone());
        if forward {
            forward_stack.pop();
            back.push(current);
        } else {
            back.pop();
            forward_stack.push(current);
        }
        replacement.back = back;
        replacement.forward = forward_stack;
        self.tabs[active] = replacement;
        cx.notify();
        self.save(cx);
    }

    pub fn persist_error(&self) -> Option<SharedString> {
        self.persistence.error.clone()
    }

    /// Retries the failed workspace load or save.
    pub fn retry_persistence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persistence.error = None;
        cx.notify();
        if self.persistence.loaded {
            self.save(cx);
        } else if self.persistence.task.is_none() {
            self.load(window, cx);
        }
    }

    fn position(&self, target: &TabTarget) -> Option<usize> {
        self.tabs.iter().position(|tab| &tab.target == target)
    }

    fn new_tab(&self, target: TabTarget, title: Option<SharedString>, window: &mut Window, cx: &mut Context<Self>) -> OpenTab {
        let title = title.unwrap_or_else(|| target.default_title());
        let wire = target.to_wire(&title);
        self.tab_from_wire(wire, window, cx)
    }

    fn tab_from_wire(&self, wire: WorkspaceTab, window: &mut Window, cx: &mut Context<Self>) -> OpenTab {
        let target = TabTarget::from_wire(&wire);
        let view = views::build_tab(&target, self.app_state(cx), window, cx);
        let title = wire.title.clone().into();
        OpenTab { target, title, view, draft: None, wire, back: Vec::new(), forward: Vec::new() }
    }

    // ---- Persistence ------------------------------------------------------------------

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let api = self.api.clone();
        let workspace = cx.weak_entity();
        self.persistence.task = Some(window.spawn(cx, async move |cx| {
            let result = api.get::<WorkspaceState>(WORKSPACE_PATH).await;
            workspace
                .update_in(cx, |this, window, cx| {
                    this.persistence.task = None;
                    match result {
                        Ok(state) => this.restore(state, window, cx),
                        Err(error) => {
                            this.persistence.error = Some(format!("Open tabs couldn't be restored: {error}").into());
                            cx.notify();
                        }
                    }
                })
                .ok();
        }));
    }

    /// Restored tabs come first; tabs opened before the load finished are kept after them.
    fn restore(&mut self, state: WorkspaceState, window: &mut Window, cx: &mut Context<Self>) {
        let opened_early = std::mem::take(&mut self.tabs);
        for wire in &state.tabs {
            if self.position(&TabTarget::from_wire(wire)).is_none() {
                let tab = self.tab_from_wire(wire.clone(), window, cx);
                self.tabs.push(tab);
            }
        }
        let active_early = self.active.and_then(|index| opened_early.get(index)).map(|tab| tab.target.clone());
        for tab in opened_early {
            if self.position(&tab.target).is_none() {
                self.tabs.push(tab);
            }
        }
        let wires: Vec<WorkspaceTab> = self.tabs.iter().map(OpenTab::wire).collect();
        let restored_active = active_position(&wires, state.active_tab_id.as_deref());
        self.active = active_early.and_then(|target| self.position(&target)).or(restored_active);
        self.closed = state.recently_closed;
        self.persistence.revision = state.revision;
        self.persistence.extra = state.extra;
        self.persistence.selected_project_id = state.selected_project_id;
        self.persistence.loaded = true;
        cx.notify();
    }

    fn snapshot(&self) -> WorkspaceState {
        WorkspaceState {
            revision: self.persistence.revision,
            tabs: self.tabs.iter().map(OpenTab::wire).collect(),
            active_tab_id: self.active().map(|tab| tab.wire.id.clone()),
            recently_closed: self.closed.clone(),
            selected_project_id: self.persistence.selected_project_id.clone(),
            extra: self.persistence.extra.clone(),
        }
    }

    /// Saves are serialized; changes made during a save trigger one follow-up save.
    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.persistence.loaded {
            return;
        }
        if self.persistence.saving {
            self.persistence.pending = true;
            return;
        }
        self.persistence.saving = true;
        let api = self.api.clone();
        let snapshot = self.snapshot();
        self.persistence.task = Some(cx.spawn(async move |this, cx| {
            let result = put_workspace(&api, snapshot).await;
            this.update(cx, |this, cx| {
                this.persistence.saving = false;
                match result {
                    Ok(revision) => {
                        this.persistence.revision = revision;
                        this.persistence.error = None;
                    }
                    Err(error) => this.persistence.error = Some(format!("Open tabs weren't saved: {error}").into()),
                }
                cx.notify();
                if std::mem::take(&mut this.persistence.pending) {
                    this.save(cx);
                }
            })
            .ok();
        }));
    }
}

/// PUT with the last-read revision; on 409 (e.g. CLI edited it) re-read the revision and
/// write once more. The open tabs in this window are the user's current intent.
async fn put_workspace(api: &ApiClient, mut snapshot: WorkspaceState) -> crate::api::ApiResult<u64> {
    match api.put::<_, WorkspaceState>(WORKSPACE_PATH, &snapshot).await {
        Err(error) if error.is_conflict() => {
            let current = api.get::<WorkspaceState>(WORKSPACE_PATH).await?;
            snapshot.revision = current.revision;
            api.put::<_, WorkspaceState>(WORKSPACE_PATH, &snapshot).await.map(|state| state.revision)
        }
        result => result.map(|state| state.revision),
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceState, WorkspaceTab, active_position, current_wire};

    /// Tabs written by another client (CLI) keep their id, unknown fields and active state.
    #[test]
    fn stored_tab_identity_survives_restore_and_save() {
        let json = r#"{"revision":4,"activeTabId":"tab-1","recentlyClosed":[],"selectedProjectId":null,
            "tabs":[{"id":"tab-0","kind":"project","resourceId":"p","projectId":"p","title":"P"},
                    {"id":"tab-1","kind":"task","resourceId":"t","projectId":"p","title":"T","pinned":true}]}"#;
        let state: WorkspaceState = serde_json::from_str(json).unwrap();
        let active = active_position(&state.tabs, state.active_tab_id.as_deref());
        assert_eq!(active, Some(1));

        let saved: Vec<WorkspaceTab> = state.tabs.iter().map(|tab| current_wire(tab, &tab.title.clone().into())).collect();
        assert_eq!(saved, state.tabs);
        assert_eq!(saved[1].extra.get("pinned"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(saved[active.unwrap()].id, "tab-1");
    }
}
