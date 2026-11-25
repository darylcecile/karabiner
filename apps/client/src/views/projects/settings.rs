//! Project → Settings: general fields and source connection (PATCH with revision), agents,
//! instructions, skills and execution environment (shared config draft), and deletion.

use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, TextareaState},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{AppSettings, ExecutionTargetInfo, HarnessInfo, Project, ProjectKind, UpdateProject};

use crate::{
    api::query_value,
    components::{self, Typography as _},
    theme::{self, metrics},
    state::{AppState, Section},
    views::{
        tasks::ui::{self, Load},
        workflow::{agents::{AgentsEditor, LibraryEditor, LibraryKind, environment_picker}, config::ConfigDraft},
    },
};


#[derive(Clone, Copy, PartialEq)]
enum Page {
    General,
    Agents,
    Instructions,
    Skills,
    Environment,
    Sync,
}

pub struct ProjectSettings {
    state: AppState,
    project_id: String,
    project: Option<Project>,
    config: Entity<ConfigDraft>,
    page: Page,
    agents: Entity<AgentsEditor>,
    instructions: Entity<LibraryEditor>,
    skills: Entity<LibraryEditor>,
    integration: Entity<super::integration::IntegrationPanel>,
    target: Entity<crate::views::workflow::target::TargetEditor>,
    targets: Load<Vec<ExecutionTargetInfo>>,
    // General draft
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    root_path: Entity<InputState>,
    repository: Entity<InputState>,
    external_ref: Entity<InputState>,
    kind: ProjectKind,
    base_revision: u64,
    dirty: bool,
    conflict: bool,
    saving: bool,
    error: Option<String>,
    confirm_delete: bool,
    syncing: bool,
    generation: u64,
    _reports: Option<gpui_kit::Task<()>>,
    /// Saves and deletion; never replaced by refreshes.
    _tasks: Vec<gpui_kit::Task<()>>,
    /// Allocated width: below 760 the section nav becomes a wrapping tab row above the pane.
    width: crate::views::responsive::WidthCell,
    _subscriptions: Vec<Subscription>,
}

impl ProjectSettings {
    pub fn new(state: AppState, project_id: String, config: Entity<ConfigDraft>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let agents = cx.new(|cx| AgentsEditor::new(state.clone(), config.clone(), window, cx));
        let instructions = cx.new(|cx| LibraryEditor::new(config.clone(), LibraryKind::Instructions, window, cx));
        let skills = cx.new(|cx| LibraryEditor::new(config.clone(), LibraryKind::Skills, window, cx));
        let target = cx.new(|cx| crate::views::workflow::target::TargetEditor::new(state.clone(), project_id.clone(), config.clone(), window, cx));
        let integration = cx.new(|cx| super::integration::IntegrationPanel::new(state.clone(), project_id.clone(), config.clone(), window, cx));
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let name = input("Project name", window, cx);
        let root_path = input("/Users/you/code/project", window, cx);
        let repository = input("owner/repository", window, cx);
        let external_ref = input("Linear team key or project URL, or GitHub Project URL", window, cx);
        let description = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 6).placeholder("What is this project for?"));
        let mut subscriptions = vec![cx.observe(&config, |_, _, cx| cx.notify())];
        for input in [&name, &root_path, &repository, &external_ref] {
            subscriptions.push(cx.subscribe_in(input, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && !this.syncing { this.touch(cx) }
            }));
        }
        subscriptions.push(cx.subscribe_in(&description, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) && !this.syncing { this.touch(cx) }
        }));
        let mut this = Self {
            state, project_id, project: None, config, page: Page::General, agents, instructions, skills, integration, target, targets: Load::Loading,
            name, description, root_path, repository, external_ref, kind: ProjectKind::Standalone, base_revision: 0,
            dirty: false, conflict: false, saving: false, error: None, confirm_delete: false, syncing: false,
            generation: 0, _reports: None, _tasks: Vec::new(), width: Default::default(), _subscriptions: subscriptions,
        };
        this.load_reports(cx);
        this
    }

    fn touch(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.generation += 1;
        cx.notify();
    }

    /// Called by the project view whenever it fetches the project.
    pub fn set_project(&mut self, project: Project, cx: &mut Context<Self>) {
        if self.dirty {
            self.conflict = project.revision != self.base_revision;
        } else {
            self.adopt(&project, cx);
        }
        self.project = Some(project);
        self.load_reports(cx);
        cx.notify();
    }

    fn adopt(&mut self, project: &Project, cx: &mut Context<Self>) {
        self.kind = project.kind.clone();
        let kind = self.kind.clone();
        self.integration.update(cx, |panel, cx| panel.set_kind(kind, cx));
        self.base_revision = project.revision;
        self.dirty = false;
        self.conflict = false;
        let values = [
            (self.name.clone(), project.name.clone()),
            (self.root_path.clone(), project.root_path.clone().unwrap_or_default()),
            (self.repository.clone(), project.repository.clone().unwrap_or_default()),
            (self.external_ref.clone(), project.external_ref.clone().unwrap_or_default()),
        ];
        let (description, text) = (self.description.clone(), project.description.clone());
        self.syncing = true;
        if let Some(window) = cx.active_window() {
            window.update(cx, |_, window, cx| {
                for (input, value) in values.iter() {
                    input.update(cx, |input, cx| input.set_value(value.as_str(), window, cx));
                }
                description.update(cx, |input, cx| input.set_value(text.as_str(), window, cx));
            }).ok();
        }
        self.syncing = false;
    }

    fn load_reports(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        let agents = self.agents.clone();
        self._reports = Some(cx.spawn(async move |this, cx| {
            let harnesses = api.get::<Vec<HarnessInfo>>("/v1/harnesses").await.unwrap_or_default();
            let default_harness = api.get::<AppSettings>("/v1/settings").await.ok().map(|settings| settings.default_harness);
            let targets = api.get::<Vec<ExecutionTargetInfo>>("/v1/execution-targets").await;
            this.update(cx, |this, cx| {
                agents.update(cx, |agents, cx| agents.set_harnesses(harnesses, default_harness, cx));
                this.targets = Load::from_result(targets);
                cx.notify();
            }).ok();
        }));
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.start_save(cx).detach();
    }

    /// Sends the general fields; resolves `Ok` once accepted (or nothing was dirty).
    pub fn start_save(&mut self, cx: &mut Context<Self>) -> gpui_kit::Task<anyhow::Result<()>> {
        if !self.dirty {
            return gpui_kit::Task::ready(Ok(()));
        }
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some("A project needs a name.".into());
            cx.notify();
            return gpui_kit::Task::ready(Err(anyhow::anyhow!("A project needs a name.")));
        }
        if self.conflict {
            // Never rebase silently (e.g. from the close prompt); the user must choose.
            return gpui_kit::Task::ready(Err(anyhow::anyhow!("Project changed elsewhere; choose Reload or Keep mine first.")));
        }
        let (submitted, base) = (self.generation, self.base_revision);
        let optional = |input: &Entity<InputState>, cx: &App| {
            let value = input.read(cx).value().trim().to_string();
            Some((!value.is_empty()).then_some(value))
        };
        let body = UpdateProject {
            name: Some(name),
            description: Some(self.description.read(cx).value().to_string()),
            kind: self.kind.is_known().then(|| self.kind.clone()),
            root_path: optional(&self.root_path, cx),
            repository: optional(&self.repository, cx),
            external_ref: optional(&self.external_ref, cx),
            color: None,
            revision: Some(self.base_revision),
        };
        self.saving = true;
        cx.notify();
        let api = self.state.api.clone();
        let path = format!("/v1/projects/{}", query_value(&self.project_id));
        cx.spawn(async move |this, cx| {
            let result = api.patch::<_, Project>(&path, &body).await;
            let latest = match &result {
                Err(error) if error.is_conflict() => api.get::<Project>(&path).await.ok(),
                _ => None,
            };
            let outcome = result.as_ref().map(|_| ()).map_err(|error| anyhow::anyhow!(ui::service_message(error)));
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(project) if this.generation == submitted => {
                        this.adopt(&project, cx);
                        this.project = Some(project);
                        this.error = None;
                        this.state.data_changed(cx);
                    }
                    Ok(project) => {
                        // Edited while saving: keep the newer draft on the saved revision.
                        this.base_revision = project.revision;
                        this.project = Some(project);
                        this.state.data_changed(cx);
                    }
                    Err(error) if error.is_conflict() => match latest {
                        Some(project) if project.revision != base => {
                            this.conflict = true;
                            this.project = Some(project);
                        }
                        _ => this.error = Some(ui::service_message(&error)),
                    },
                    Err(error) => this.error = Some(ui::error_text(&error)),
                }
                cx.notify();
            }).ok();
            outcome
        })
    }

    /// Explicit "Keep mine": rebase on the latest fetched revision, then save.
    fn keep_mine(&mut self, cx: &mut Context<Self>) {
        if let Some(project) = &self.project { self.base_revision = project.revision }
        self.conflict = false;
        self.save(cx);
    }

    fn discard(&mut self, cx: &mut Context<Self>) {
        if let Some(project) = self.project.clone() { self.adopt(&project, cx) }
        self.error = None;
        self.state.data_changed(cx);
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if !self.confirm_delete {
            self.confirm_delete = true;
            cx.notify();
            return;
        }
        let api = self.state.api.clone();
        let path = format!("/v1/projects/{}", query_value(&self.project_id));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = api.delete(&path).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.state.data_changed(cx);
                        this.state.show_section(Section::Projects, cx);
                    }
                    Err(error) => this.error = Some(ui::error_text(&error)),
                }
                cx.notify();
            }).ok();
        }));
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let kind_control = if self.kind.is_known() {
            ui::segmented("project-kind", &[(ProjectKind::Standalone, "Standalone"), (ProjectKind::Github, "GitHub"), (ProjectKind::Linear, "Linear")],
                self.kind.clone(), move |kind, _, cx| { weak.update(cx, |this, cx| {
                    this.integration.update(cx, |panel, cx| panel.set_kind(kind.clone(), cx));
                    this.kind = kind;
                    this.touch(cx)
                }).ok(); }, cx).into_any_element()
        } else {
            ui::chip(format!("{} (unsupported here, kept as is)", self.kind.as_str()), cx).into_any_element()
        };
        let source_note = match self.kind {
            ProjectKind::Github => "Issues from this repository or GitHub Project sync as tasks. Set up sync under Sync.",
            ProjectKind::Linear => "Issues from this Linear team or project sync as tasks. Set up sync under Sync.",
            _ => "A local project. Tasks live only in Karabiner.",
        };
        // Narrow (same <760 rule as the nav): controls go under their labels at full width.
        let stacked = crate::views::responsive::narrower_than(&self.width, 760.);
        let input = |state: &Entity<InputState>| div().w_full().child(components::input_md(Input::new(state)));
        let row = |title: &'static str, help: &'static str, control: AnyElement, cx: &App| components::form_row(title, (!help.is_empty()).then(|| help.into()), control, stacked, cx);
        // Plain sections with hairline rows (ref form-research/v-linear-*): no cards around
        // fields, so each field's own border is the only one.
        div().flex().flex_col()
            .child(components::form_section("Project", None, cx).pt_0())
            .child(components::form_rows(cx)
                .child(row("Name", "", input(&self.name).into_any_element(), cx))
                .child(components::form_row("Description", None, components::textarea_box(&self.description, cx), true, cx)))
            .child(components::form_section("Source", Some(source_note.into()), cx))
            .child(components::form_rows(cx)
                .child(row("Source", "", kind_control, cx))
                .child(row("Local checkout", "Folder agents work in.", input(&self.root_path).into_any_element(), cx))
                .when(self.kind == ProjectKind::Github, |this| this
                    .child(row("GitHub repository", "owner/name", input(&self.repository).into_any_element(), cx))
                    .child(row("GitHub Project", "Optional. Its URL, to sync the Project's items instead of the repository's issues.", input(&self.external_ref).into_any_element(), cx)))
                .when(self.kind == ProjectKind::Linear, |this| this
                    .child(row("Linear source", "Team key (e.g. ENG), or a project URL or id.", input(&self.external_ref).into_any_element(), cx))
                    .child(row("Code repository", "Optional.", input(&self.repository).into_any_element(), cx))))
            .child(components::form_section("Danger zone", None, cx))
            .child(components::form_rows(cx)
                .child(row("Delete project", "Removes its tasks, documents and configuration from the service.",
                    components::button_row_action(Button::new("delete-project").danger().icon(gpui_kit::assets::IconName::Trash)
                        .label(if self.confirm_delete { "Click again to delete" } else { "Delete" }))
                        .on_click(cx.listener(|this, _, _, cx| this.delete(cx))).into_any_element(), cx)))
    }
}

impl Render for ProjectSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Children act on saved data; tell them what General has unsaved.
        let (dirty, root) = (self.dirty, self.root_path.read(cx).value().trim().to_string());
        let checkout = (!root.is_empty()).then(|| root.clone());
        self.agents.update(cx, |agents, _| agents.set_checkout(checkout));
        let saved_root = self.project.as_ref().and_then(|project| project.root_path.clone()).unwrap_or_default();
        self.integration.update(cx, |panel, _| panel.set_source_dirty(dirty));
        use crate::views::workflow::target::RootDraft;
        let draft = match (dirty && root != saved_root, root.is_empty()) {
            (false, _) => RootDraft::Saved,
            (true, true) => RootDraft::Cleared,
            (true, false) => RootDraft::Edited(root),
        };
        let stacked = crate::views::responsive::narrower_than(&self.width, 760.);
        self.target.update(cx, |target, _| { target.set_project_root(draft); target.set_stacked(stacked) });
        self.integration.update(cx, |panel, _| panel.set_stacked(stacked));
        let page = self.page;
        let content = match page {
            Page::General => self.render_general(cx).into_any_element(),
            Page::Agents => self.agents.clone().into_any_element(),
            Page::Instructions => self.instructions.clone().into_any_element(),
            Page::Skills => self.skills.clone().into_any_element(),
            Page::Sync => self.integration.clone().into_any_element(),
            Page::Environment => div().flex().flex_col()
                .child(components::form_section("Where runs execute", Some("Runs already started keep the environment they started with.".into()), cx).pt_0())
                .child(environment_picker(&self.config, &self.targets, cx))
                .child(self.target.clone()).into_any_element(),
        };
        let nav = [
            (Page::General, "General"), (Page::Agents, "Agents"), (Page::Instructions, "Instructions"),
            (Page::Skills, "Skills"), (Page::Environment, "Environment"), (Page::Sync, "Sync"),
        ];
        let stacked = crate::views::responsive::narrower_than(&self.width, 760.);
        // The nav stays put (outside the scroll pane) so every section is reachable however
        // far the content has scrolled; only the content pane scrolls.
        let nav = div().flex_none().flex().gap_0p5()
            .map(|this| if stacked {
                this.flex_wrap().px(px(metrics::GUTTER_X - 8.)).pt(px(metrics::GUTTER_TOP)).pb_2()
            } else {
                this.w(px(180. + metrics::GUTTER_X)).flex_col().pl(px(metrics::GUTTER_X)).pt(px(metrics::GUTTER_TOP))
            })
            .children(nav.into_iter().map(|(target, label)| {
                        let active = target == page;
                        let t = theme::tokens(cx);
                        div().id(label).px_2().h(px(metrics::ROW_NAV)).flex().items_center().rounded(px(metrics::R_CONTROL)).cursor_pointer().t_label()
                            .when(active, |this| this.bg(t.selected).text_color(t.text).font_weight(FontWeight::MEDIUM))
                            .when(!active, |this| this.text_color(t.text_secondary).hover(move |style| style.bg(t.hover).text_color(t.text)))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| { this.page = target; cx.notify() }))
                    }));
        let pane = div().id("project-settings").flex_1().min_w_0().min_h_0().overflow_y_scroll().child(
            // Wide: 32 from the nav (the old gap_8); stacked: the page gutter.
            div().pl(px(if stacked { metrics::GUTTER_X } else { 32. })).pr(px(metrics::GUTTER_X))
                .pt(px(if stacked { 8. } else { metrics::GUTTER_TOP })).pb_6()
                .child(div().max_w(px(metrics::READ_W)).child(content)));
        div().size_full().flex().flex_col()
            .child(div().relative().flex_1().min_h_0().flex().when(stacked, |this| this.flex_col())
                .child(crate::views::responsive::width_probe(&self.width))
                .child(nav)
                .child(pane))
            .when(page == Page::General && (self.dirty || self.conflict), |this| {
                let (discard, save) = (cx.weak_entity(), cx.weak_entity());
                this.child(ui::save_bar("project-save", self.conflict, self.saving, self.error.as_deref(),
                    move |_, _, cx| { discard.update(cx, |this, cx| this.discard(cx)).ok(); },
                    move |_, _, cx| { save.update(cx, |this, cx| if this.conflict { this.keep_mine(cx) } else { this.save(cx) }).ok(); },
                    cx))
            })
    }
}
