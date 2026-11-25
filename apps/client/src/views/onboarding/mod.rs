//! Templates and onboarding: browse templates, preview their workflow/agents/documents,
//! customise source details and harness, and create a project through the service.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName,
    Disableable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::{AppSettings, CreateProject, HarnessId, Project, ProjectConfig, ProjectKind, Template};

use crate::{
    api::query_value,
    state::{AppState, TabTarget},
    views::tasks::ui::{self, Load, UpdateInWindow as _},
    theme,
};
use crate::components::{self, Typography as _};
use crate::theme::metrics;

pub fn section(state: AppState, window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Templates::new(state, window, cx)).into()
}

pub struct Templates {
    state: AppState,
    templates: Load<Vec<Template>>,
    settings: Option<AppSettings>,
    selected: Option<String>,
    search: Entity<InputState>,
    name: Entity<InputState>,
    root_path: Entity<InputState>,
    repository: Entity<InputState>,
    team: Entity<InputState>,
    kind: ProjectKind,
    harness: Option<HarnessId>,
    creating: bool,
    step: Step,
    category: Option<Category>,
    error: Option<String>,
    _load: Option<gpui_kit::Task<()>>,
    /// Create/customise writes; never cancelled by refreshes.
    _mutations: Vec<gpui_kit::Task<()>>,
    /// Template column width: drives the 3/2/1 card grid.
    width: crate::views::responsive::WidthCell,
    _subscriptions: Vec<Subscription>,
}

impl Templates {
    fn new(state: AppState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let search = input("Search templates", window, cx);
        let name = input("Project name", window, cx);
        let root_path = input("/Users/you/code/project (optional)", window, cx);
        let repository = input("owner/repository", window, cx);
        let team = input("Linear team key, e.g. ENG", window, cx);
        let mut seen = state.service.read(cx).data_version();
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |_, _, event: &InputEvent, _, cx| if matches!(event, InputEvent::Change) { cx.notify() }),
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) { this.create(cx) }
            }),
            cx.observe(&state.service, move |this: &mut Self, service, cx| {
                let version = service.read(cx).data_version();
                if version != seen { seen = version; this.reload(cx) }
            }),
        ];
        let mut this = Self {
            state, templates: Load::Loading, settings: None, selected: None, search, name, root_path, repository, team,
            kind: ProjectKind::Standalone, harness: None, creating: false, step: Step::Pick, category: None, error: None, _load: None, _mutations: Vec::new(), width: Default::default(), _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let api = self.state.api.clone();
        self._load = Some(cx.spawn(async move |this, cx| {
            let templates = api.get::<Vec<Template>>("/v1/templates").await;
            let settings = api.get::<AppSettings>("/v1/settings").await.ok();
            this.update(cx, |this, cx| {
                if let (Ok(list), None) = (&templates, &this.selected) {
                    this.selected = list.first().map(|template| template.id.clone());
                }
                match templates {
                    Ok(list) => this.templates = Load::Ready(list),
                    Err(error) if this.templates.ready().is_some() => this.error = Some(format!("Couldn't refresh templates: {}", ui::error_text(&error))),
                    Err(error) => this.templates = Load::Failed(ui::error_text(&error)),
                }
                if settings.is_some() { this.settings = settings }
                cx.notify();
            }).ok();
        }));
    }

    fn template(&self) -> Option<&Template> {
        let id = self.selected.as_ref()?;
        self.templates.ready()?.iter().find(|template| &template.id == id)
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some("Give the project a name.".into());
            cx.notify();
            return;
        }
        let value = |input: &Entity<InputState>, cx: &App| {
            let value = input.read(cx).value().trim().to_string();
            (!value.is_empty()).then_some(value)
        };
        let repository = value(&self.repository, cx);
        let team = value(&self.team, cx);
        if self.kind == ProjectKind::Github && repository.is_none() {
            self.error = Some("Enter the GitHub repository as owner/name.".into());
            cx.notify();
            return;
        }
        if self.kind == ProjectKind::Linear && team.is_none() {
            self.error = Some("Enter the Linear team key.".into());
            cx.notify();
            return;
        }
        let body = CreateProject {
            name,
            description: self.template().map(|template| template.description.clone()),
            kind: Some(self.kind.clone()),
            root_path: value(&self.root_path, cx),
            repository,
            external_ref: if self.kind == ProjectKind::Linear { team } else { None },
            color: None,
            template_id: self.selected.clone(),
        };
        let harness = self.harness.clone();
        let settings = self.settings.clone().filter(|settings| !settings.onboarding_complete);
        self.creating = true;
        self.error = None;
        cx.notify();
        let api = self.state.api.clone();
        self._mutations.push(cx.spawn(async move |this, cx| {
            let result: Result<(Project, Option<String>), _> = async {
                let project = api.post::<_, Project>("/v1/projects", &body).await?;
                let mut note = None;
                if let Some(harness) = harness {
                    // Customisation step: retarget the template's agents to the chosen harness.
                    let path = format!("/v1/projects/{}/config", query_value(&project.id));
                    let applied = async {
                        let mut config = api.get::<ProjectConfig>(&path).await?;
                        for agent in config.agents.iter_mut() { agent.harness = harness.clone() }
                        api.put::<_, ProjectConfig>(&path, &config).await
                    }.await;
                    if let Err(error) = applied { note = Some(format!("Project created, but the harness change wasn't saved: {}", ui::error_text(&error))) }
                }
                if let Some(mut settings) = settings {
                    settings.onboarding_complete = true;
                    api.put::<_, AppSettings>("/v1/settings", &settings).await.ok();
                }
                Ok((project, note))
            }.await;
            this.update_in_window(cx, |this, window, cx| {
                this.creating = false;
                match result {
                    Ok((project, note)) => {
                        this.error = note;
                        this.name.update(cx, |input, cx| input.set_value("", window, cx));
                        this.step = Step::Pick;
                        this.state.data_changed(cx);
                        this.state.open(TabTarget::project(project.id), window, cx);
                    }
                    Err(error) => this.error = Some(ui::error_text(&error)),
                }
                cx.notify();
            });
        }));
    }

    fn render_card(&self, index: usize, template: &Template, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let summary = Summary::of(template);
        let selected = self.selected.as_ref() == Some(&template.id);
        let id = template.id.clone();
        let hue = WELL_HUES[index % WELL_HUES.len()];
        let accent = selection_orange();
        div().id(SharedString::from(format!("template-{}", template.id)))
            // Selected border is 1.5 (85F1DC37); trim padding by the extra 0.5 so content doesn't shift.
            .relative().p(px(if selected { 5.5 } else { 6. })).rounded(px(metrics::R_CARD)).cursor_pointer()
            .border(px(if selected { 1.5 } else { 1. }))
            .border_color(if selected { accent } else { cx.theme().border })
            .bg(theme::tokens(cx).surface)
            .when(!selected, |this| this.hover(|style| style.border_color(theme::tokens(cx).text_tertiary)))
            .on_click(cx.listener(move |this, _, _, cx| { this.selected = Some(id.clone()); this.error = None; cx.notify() }))
            .on_double_click_like(cx)
            .child(well(&summary, hue, cx))
            .child(div().px_1p5().pt_2p5().pb_1p5().flex().flex_col().gap_0p5()
                .child(div().t_label().font_weight(FontWeight::MEDIUM).text_color(theme::tokens(cx).text).child(template.name.clone()))
                .child(div().t_caption().text_color(cx.theme().muted_foreground).child(summary.caption())))
            .when(selected, |this| this.child(
                div().absolute().top(px(12.)).right(px(12.)).size(px(18.)).rounded_full().bg(accent)
                    .flex().items_center().justify_center()
                    .child(Icon::new(IconName::Check).size(px(11.)).text_color(gpui_kit::white())),
            ))
    }

    fn render_filters(&self, list: &[Template], cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let count = |category: Option<Category>| list.iter().filter(|t| category.is_none_or(|c| Summary::of(t).category == c)).count();
        let options = [(None, "All"), (Some(Category::Workflow), "Workflows"), (Some(Category::Documents), "Documents"), (Some(Category::Blank), "Blank")];
        let shown: Vec<(Option<Category>, &str)> = options.into_iter().filter(|(category, _)| category.is_none() || count(*category) > 0).collect();
        let selected = shown.iter().position(|(category, _)| *category == self.category).unwrap_or(usize::MAX);
        let items = shown.iter().map(|(category, label)| components::Segment::new(*label).count(count(*category))).collect();
        let categories: Vec<Option<Category>> = shown.iter().map(|(category, _)| *category).collect();
        let weak = cx.weak_entity();
        components::segmented("template-filter", items, selected, move |index, _, cx| {
            if let Some(category) = categories.get(index).copied() {
                weak.update(cx, |this, cx| { this.category = category; cx.notify() }).ok();
            }
        }, cx)
    }

    /// Same well colour as the template's card in step 1.
    fn hue_of(&self, id: &str) -> u32 {
        let index = self.templates.ready().and_then(|list| list.iter().position(|template| template.id == id)).unwrap_or(0);
        WELL_HUES[index % WELL_HUES.len()]
    }

    /// Step 2: what the template sets up, plus the project details.
    fn render_customize(&self, template: &Template, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let config = parse(template);
        let weak = cx.weak_entity();
        let harness_weak = cx.weak_entity();
        let workflow = config.as_ref().and_then(|config| config.workflows.first()).cloned();
        let agents = config.as_ref().map(|config| config.agents.clone()).unwrap_or_default();
        div().flex().gap_5().items_start()
            .child(div().w(px(320.)).flex_none().flex().flex_col().gap_4()
                .child(well(&Summary::of(template), self.hue_of(&template.id), cx))
                .child(div().t_body().text_color(cx.theme().muted_foreground).child(template.description.clone()))
                .when(config.is_none(), |this| this.child(ui::amber_chip("This template uses a newer format; it can still be applied.", cx)))
                .children(workflow.map(|workflow| div().flex().flex_col().gap_1()
                    .child(ui::caption(format!("Workflow · {}", workflow.name), cx))
                    .children(workflow.stages.iter().enumerate().map(|(index, stage)| {
                        let names: Vec<String> = stage.agent_ids.iter()
                            .filter_map(|id| agents.iter().find(|agent| &agent.id == id).map(|agent| agent.name.clone())).collect();
                        div().flex().items_center().gap_2().h(px(metrics::ROW_PROP))
                            .child(div().size(px(14.)).rounded_full().border(px(1.5)).border_color(cx.theme().border))
                            .child(div().t_code_caption(cx).text_color(cx.theme().muted_foreground).child(format!("{:02}", index + 1)))
                            .child(div().t_label().child(stage.name.clone()))
                            .child(div().ml_auto().t_caption().text_color(cx.theme().muted_foreground).child(names.join(", ")))
                    }))))
                .when(!template.documents.is_empty(), |this| this.child(div().flex().flex_col().gap_1()
                    .child(ui::caption("Documents", cx))
                    .children(template.documents.iter().map(|document| div().flex().items_center().gap_2().t_label()
                        .child(Icon::new(IconName::FileText).size(px(14.)).text_color(cx.theme().muted_foreground)).child(document.title.clone()))))))
            .child(div().flex_1().min_w_0().flex().flex_col().gap_4()
                .child(ui::field("Name", components::input_md(Input::new(&self.name)), cx))
                .child(ui::field("Source", ui::segmented("new-kind",
                    &[(ProjectKind::Standalone, "Standalone"), (ProjectKind::Github, "GitHub"), (ProjectKind::Linear, "Linear")],
                    self.kind.clone(), move |kind, _, cx| { weak.update(cx, |this, cx| { this.kind = kind; cx.notify() }).ok(); }, cx), cx))
                .child(ui::field("Local checkout", components::input_md(Input::new(&self.root_path)), cx))
                .when(self.kind == ProjectKind::Github, |this| this.child(ui::field("GitHub repository", components::input_md(Input::new(&self.repository)), cx)))
                .when(self.kind == ProjectKind::Linear, |this| this.child(ui::field("Linear team", components::input_md(Input::new(&self.team)), cx)))
                .when(!agents.is_empty(), |this| this.child(ui::field("Agent harness", {
                    let mut options: Vec<(Option<HarnessId>, &'static str)> = vec![(None, "As template")];
                    options.extend([HarnessId::Claude, HarnessId::Codex, HarnessId::Copilot].into_iter().map(|h| { let label = ui::harness_label(&h); (Some(h), label) }));
                    ui::segmented("new-harness", &options, self.harness.clone(),
                        move |harness, _, cx| { harness_weak.update(cx, |this, cx| { this.harness = harness; cx.notify() }).ok(); }, cx)
                }, cx)))
                .children(self.error.clone().map(|error| div().t_label().text_color(cx.theme().danger).child(error))))
    }

    /// Footer tray: selection summary on the left, actions on the right (never shrink).
    /// Reference 3 across when each card gets ~200 (column ≥ 680 incl. gutters); 2, then 1.
    fn grid_columns(&self) -> u16 {
        use crate::views::responsive::narrower_than;
        if !narrower_than(&self.width, 680.) { 3 } else if !narrower_than(&self.width, 460.) { 2 } else { 1 }
    }

    fn render_footer(&self, template: Option<&Template>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let summary = template.map(|template| format!("{} · {}", template.name, Summary::of(template).caption()));
        let picking = self.step == Step::Pick;
        // Full-width band row: the 32 pill sits 12 from the band's right edge, matching its
        // 12 vertical inset (a centred 920 row left it 70+ from the edge).
        div().w_full().flex().items_center().gap_3().pl(px(metrics::GUTTER_X)).pr_3().py_3()
            .child(div().flex_1().min_w_0().truncate().t_label().text_color(cx.theme().muted_foreground)
                .child(if picking {
                    summary.unwrap_or_else(|| "Select a template".into())
                } else {
                    "Creates stored configuration only. No agents start.".into()
                }))
            .child(div().flex_none().flex().gap_2()
                .when(!picking, |this| this.child(components::button_pill(Button::new("back-to-templates").outline().label("Back"))
                    .on_click(cx.listener(|this, _, _, cx| { this.step = Step::Pick; this.error = None; cx.notify() }))))
                .child(if picking {
                    components::button_pill(Button::new("continue-template").primary().icon(IconName::ArrowRight).label("Use template"))
                        .disabled(template.is_none())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.step = Step::Customize;
                            this.name.update(cx, |input, cx| input.focus(window, cx));
                            cx.notify();
                        }))
                } else {
                    components::button_pill(Button::new("create-project").primary().label("Create project")).loading(self.creating)
                        .on_click(cx.listener(|this, _, _, cx| this.create(cx)))
                }))
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Pick,
    Customize,
}

/// Derived from the template's own contents (no invented metadata).
#[derive(Clone, Copy, PartialEq)]
enum Category {
    Workflow,
    Documents,
    Blank,
}

struct Summary {
    category: Category,
    stages: Vec<String>,
    agents: usize,
    documents: Vec<String>,
}

impl Summary {
    fn of(template: &Template) -> Self {
        let config = parse(template);
        let stages: Vec<String> = config.as_ref().and_then(|c| c.workflows.first())
            .map_or_else(Vec::new, |workflow| workflow.stages.iter().map(|stage| stage.name.clone()).collect());
        let agents = config.as_ref().map_or(0, |c| c.agents.len());
        let documents: Vec<String> = template.documents.iter().map(|document| document.title.clone()).collect();
        let category = if !stages.is_empty() { Category::Workflow } else if !documents.is_empty() { Category::Documents } else { Category::Blank };
        Self { category, stages, agents, documents }
    }

    fn caption(&self) -> String {
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        match self.category {
            Category::Workflow => format!("Workflow · {} · {}", plural(self.stages.len(), "stage"), plural(self.agents, "agent")),
            Category::Documents => format!("Documents · {}", plural(self.documents.len(), "doc")),
            Category::Blank => "Blank · configure everything yourself".into(),
        }
    }
}

/// Pastel well hues; the well is a light tint of each in either theme.
const WELL_HUES: [u32; 6] = [0x3B82F6, 0xF0532D, 0xEC4899, 0x10B981, 0xEAB308, 0x737373];

fn selection_orange() -> Hsla {
    rgb(0xF0532D).into()
}

/// Thumbnail well with a miniature of what the template creates: a stage rail for
/// workflows, document lines for documents, an empty dashed sheet for blank.
fn well(summary: &Summary, hue: u32, cx: &App) -> impl IntoElement + use<> {
    let tint: Hsla = rgb(hue).into();
    let tokens = theme::tokens(cx);
    let bar = |width: f32, strong: bool| div().h(px(4.)).w(px(width)).rounded_full().bg(if strong { tokens.text_tertiary } else { tokens.border });
    let sheet = div().w(px(150.)).h(px(104.)).mt(px(14.)).px_2p5().pt_2p5().rounded_t(px(6.)).bg(tokens.surface)
        .border_1().border_b_0().border_color(tokens.border).flex().flex_col().gap(px(7.));
    let content = match summary.category {
        Category::Workflow => sheet.children(summary.stages.iter().take(5).enumerate().map(|(index, _)| {
            div().flex().items_center().gap_1p5()
                .child(div().size(px(8.)).rounded_full().border(px(1.5)).border_color(if index == 0 { tint } else { tokens.border })
                    .when(index == 0, |this| this.bg(tint.opacity(0.3))))
                .child(bar(56. + (index % 3) as f32 * 14., index == 0))
                .child(div().ml_auto().size(px(10.)).rounded(px(3.)).bg(tint.opacity(0.18)))
        })),
        Category::Documents => sheet
            .child(div().flex().items_center().gap_1().child(div().size(px(8.)).rounded(px(2.)).bg(tokens.text)).child(bar(40., true)))
            .child(div().h(px(18.)).w_full().rounded(px(3.)).bg(tint.opacity(0.18)))
            .children((0..4).map(|index| bar(120. - (index % 2) as f32 * 34., false))),
        Category::Blank => sheet.child(div().flex_1().mb_2p5().rounded(px(4.)).border_1().border_dashed().border_color(tokens.border)),
    };
    div().h(px(118.)).w_full().rounded(px(metrics::inner_radius(metrics::R_CARD, 6.))).bg(tint.opacity(0.1)).overflow_hidden().flex().justify_center().child(content)
}

trait DoubleClickLike {
    fn on_double_click_like(self, cx: &mut Context<Templates>) -> Self;
}

impl DoubleClickLike for Stateful<Div> {
    /// Double-click a card to go straight to customising it.
    fn on_double_click_like(self, cx: &mut Context<Templates>) -> Self {
        self.on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
            if event.click_count == 2 {
                this.step = Step::Customize;
                this.name.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }
        }))
    }
}

/// Template config decoded as a project configuration (it may lack or add fields).
fn parse(template: &Template) -> Option<ProjectConfig> {
    let mut value = template.config.clone();
    if let Some(object) = value.as_object_mut() {
        object.entry("revision").or_insert(0.into());
    }
    serde_json::from_value(value).ok()
}

impl Render for Templates {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first_run = self.settings.as_ref().is_some_and(|settings| !settings.onboarding_complete);
        let list = self.templates.ready().cloned();
        let body = match &self.templates {
            Load::Loading => ui::skeleton_rows(3, cx).into_any_element(),
            Load::Failed(error) => {
                let weak = cx.weak_entity();
                ui::error_panel("Couldn't load templates", error, move |_, _, cx| { weak.update(cx, |this, cx| this.reload(cx)).ok(); }, cx).into_any_element()
            }
            Load::Ready(list) if list.is_empty() => ui::empty_panel("No templates", "The service has no templates installed.", None, cx).into_any_element(),
            Load::Ready(_) if self.step == Step::Customize && self.template().is_some() => {
                let template = self.template().cloned().unwrap_or_else(|| unreachable!());
                self.render_customize(&template, cx).into_any_element()
            }
            Load::Ready(list) => {
                let query = self.search.read(cx).value().to_lowercase();
                let visible: Vec<(usize, Template)> = list.iter().enumerate()
                    .filter(|(_, template)| self.category.is_none_or(|category| Summary::of(template).category == category))
                    .filter(|(_, template)| query.is_empty() || template.name.to_lowercase().contains(&query) || template.description.to_lowercase().contains(&query))
                    .map(|(index, template)| (index, template.clone()))
                    .collect();
                if visible.is_empty() {
                    div().p_6().t_label().text_color(cx.theme().muted_foreground).child("No templates match.").into_any_element()
                } else {
                    div().grid().grid_cols(self.grid_columns()).gap_3().children(visible.iter().map(|(index, template)| self.render_card(*index, template, cx))).into_any_element()
                }
            }
        };
        let selected = self.template().cloned();
        let filters = list.as_ref().filter(|_| self.step == Step::Pick).map(|list| self.render_filters(list, cx));
        let title = match (self.step, &selected) {
            (Step::Customize, Some(template)) => format!("Set up {}", template.name),
            _ if first_run => "Welcome to Karabiner".into(),
            _ => "Start from a template".into(),
        };
        let subtitle = if self.step == Step::Customize {
            "Name the project and connect its source. Stages, agents and documents stay editable."
        } else if first_run {
            "Pick a template to set up your first project. Everything it creates stays editable."
        } else {
            "Pick a starting point for a new project. Everything it creates stays editable."
        };
        // In-page adaptation of 85F1DC37: the page gutter owns header, filters and grid (no
        // wrapper card); the footer is a full-width tray band at the content bottom, like
        // the app's save bar.
        div().size_full().flex().flex_col()
            .child(div().id("templates").flex_1().min_h_0().overflow_y_scroll().child(
                div().relative().max_w(px(920.)).mx_auto().px(px(metrics::GUTTER_X)).py_6().flex().flex_col()
                    .child(crate::views::responsive::width_probe(&self.width))
                    .child(div().pb_4().flex().items_start().gap_3()
                        .child(div().size(px(36.)).flex_none().rounded(px(metrics::R_INNER)).bg(selection_orange().opacity(0.12)).flex().items_center().justify_center()
                            .child(Icon::new(IconName::GalleryVerticalEnd).size(px(16.)).text_color(selection_orange())))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap_0p5()
                            .child(div().t_headline().text_color(theme::tokens(cx).text).child(title))
                            .child(div().t_body().text_color(cx.theme().muted_foreground).child(subtitle))))
                    .children(filters.map(|filters| div().pb_4().flex().items_center().justify_between().gap_3().flex_wrap()
                        .child(filters)
                        .child(div().w(px(240.)).child(components::input_md(Input::new(&self.search).prefix(Icon::new(IconName::Search).size(px(14.))))))))
                    .child(body),
            ))
            .child(div().flex_none().border_t_1().border_color(cx.theme().border).bg(theme::tokens(cx).tray)
                .child(self.render_footer(selected.as_ref(), cx)))
    }
}
