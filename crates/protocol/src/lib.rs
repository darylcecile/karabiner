//! Rust mirror of `packages/contracts/src/index.ts` (API v1).
//! Rules: unknown response fields are ignored, except on whole-object PUT resources
//! (config/settings/workspace/tours) which keep them in `extra` so writes do not drop them.
//! Unknown enum values decode to `Unknown` and must render a fallback.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const API_VERSION: u32 = 1;
pub const DEFAULT_SERVICE_URL: &str = "http://127.0.0.1:43173";

pub type Id = String;
/// ISO-8601 UTC string; parse only where needed.
pub type Timestamp = String;
pub type Extra = Map<String, Value>;

#[derive(Debug, Clone, Deserialize)]
pub struct Reply<T> {
    pub data: T,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiErrorEnvelope {
    pub error: ApiErrorBody,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)] pub storage: bool,
    #[serde(default)] pub configuration: bool,
    #[serde(default)] pub documents: bool,
    #[serde(default)] pub code_tours: bool,
    #[serde(default)] pub intervention_intents: bool,
    #[serde(default)] pub activity: bool,
    #[serde(default)] pub statistics: bool,
    #[serde(default)] pub scheduler: bool,
    #[serde(default)] pub harness_execution: bool,
    #[serde(default)] pub integrations: bool,
    #[serde(default)] pub virtual_machines: bool,
    /// `GET /v1/events` SSE invalidation stream is available.
    #[serde(default)] pub events: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceMeta {
    pub api_version: u32,
    pub service_version: String,
    #[serde(default)]
    pub capabilities: Capabilities,
}

impl ServiceMeta {
    pub fn is_compatible(&self) -> bool {
        self.api_version == API_VERSION
    }
}

/// String enums that keep unrecognised values verbatim (`Unknown("…")`), so whole-object
/// PUTs never rewrite values written by a newer service or client.
macro_rules! wire_enum {
    ($name:ident { $($variant:ident = $wire:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)+
            Unknown(String),
        }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub fn as_str(&self) -> &str {
                match self { $(Self::$variant => $wire,)+ Self::Unknown(raw) => raw }
            }
            pub fn is_known(&self) -> bool {
                !matches!(self, Self::Unknown(_))
            }
        }
        impl From<&str> for $name {
            fn from(raw: &str) -> Self {
                match raw { $($wire => Self::$variant,)+ other => Self::Unknown(other.to_owned()) }
            }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer).map(|raw| Self::from(raw.as_str()))
            }
        }
    };
}

wire_enum!(ProjectKind { Standalone = "standalone", Github = "github", Linear = "linear" });
wire_enum!(TaskStatus {
    Backlog = "backlog", Todo = "todo", InProgress = "in_progress",
    InReview = "in_review", Blocked = "blocked", Done = "done",
});
wire_enum!(Priority { Low = "low", Normal = "normal", High = "high", Urgent = "urgent" });
wire_enum!(HarnessId { Claude = "claude", Codex = "codex", Copilot = "copilot", Custom = "custom" });
wire_enum!(AgentMode { Interactive = "interactive", Autonomous = "autonomous" });
wire_enum!(ExecutionTarget { Host = "host", Mxc = "mxc", AppleContainer = "apple-container" });
wire_enum!(DocumentKind { Markdown = "markdown", CodeTour = "code_tour" });
wire_enum!(Theme { System = "system", Light = "light", Dark = "dark" });
wire_enum!(TabKind { Project = "project", Task = "task", Document = "document", Review = "review" });
wire_enum!(InterventionKind {
    Instruction = "instruction", Question = "question", Pause = "pause", Resume = "resume",
    Cancel = "cancel", Retry = "retry", Approve = "approve", Reject = "reject",
});
wire_enum!(InterventionStatus { Recorded = "recorded", Answered = "answered", Dismissed = "dismissed" });
wire_enum!(Level { Low = "low", Medium = "medium", High = "high" });
wire_enum!(FileChangeStatus { Added = "added", Modified = "modified", Deleted = "deleted", Renamed = "renamed" });
wire_enum!(ReviewDecision { Pending = "pending", Approved = "approved", ChangesRequested = "changes_requested" });

// ---- Projects -------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub kind: ProjectKind,
    pub root_path: Option<String>,
    pub repository: Option<String>,
    pub external_ref: Option<String>,
    pub color: String,
    pub revision: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProject {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub kind: Option<ProjectKind>,
    #[serde(skip_serializing_if = "Option::is_none")] pub root_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub external_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub template_id: Option<String>,
}

/// PATCH: only `Some` fields are sent. `Some(None)` clears a nullable field.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProject {
    #[serde(skip_serializing_if = "Option::is_none")] pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub kind: Option<ProjectKind>,
    #[serde(skip_serializing_if = "Option::is_none")] pub root_path: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub repository: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub external_ref: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub revision: Option<u64>,
}

// ---- Tasks ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtask {
    pub id: Id,
    pub title: String,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateComment {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: Id,
    pub project_id: Id,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub workflow_id: Option<Id>,
    pub agent_id: Option<Id>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub stage_id: Option<Id>,
    #[serde(default)]
    pub due_at: Option<Timestamp>,
    #[serde(default)]
    pub sort_order: f64,
    #[serde(default)]
    pub subtasks: Vec<Subtask>,
    /// Always `not_started` until execution exists.
    pub execution_state: String,
    pub revision: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTask {
    pub project_id: Id,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")] pub priority: Option<Priority>,
    #[serde(skip_serializing_if = "Option::is_none")] pub workflow_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub agent_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub stage_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub due_at: Option<Option<Timestamp>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub sort_order: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub subtasks: Option<Vec<Subtask>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTask {
    #[serde(skip_serializing_if = "Option::is_none")] pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")] pub priority: Option<Priority>,
    #[serde(skip_serializing_if = "Option::is_none")] pub workflow_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub agent_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub stage_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub due_at: Option<Option<Timestamp>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub sort_order: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub subtasks: Option<Vec<Subtask>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub revision: Option<u64>,
}

// ---- Project configuration (whole-object PUT: keep unknown fields) --------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub id: Id,
    pub name: String,
    pub role: String,
    pub harness: HarnessId,
    pub model: String,
    pub instructions: String,
    pub skill_ids: Vec<Id>,
    pub mode: AgentMode,
    /// Used when `harness` is `custom`.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub executable: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub args: Option<Vec<String>>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub content: String,
    pub path: Option<String>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstructionDefinition {
    pub id: Id,
    pub name: String,
    pub content: String,
    pub path: Option<String>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDefinition {
    pub id: Id,
    pub name: String,
    pub instructions: String,
    pub agent_ids: Vec<Id>,
    pub return_to_stage_ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub exit_rule: Option<String>,
    /// Board status a task takes when it enters this stage.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub status: Option<TaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub color: Option<String>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub stages: Vec<StageDefinition>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    pub revision: u64,
    pub instructions: Vec<InstructionDefinition>,
    pub skills: Vec<SkillDefinition>,
    pub agents: Vec<AgentDefinition>,
    pub workflows: Vec<WorkflowDefinition>,
    pub execution_target: ExecutionTarget,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `ProjectConfig` without `revision`; decoded loosely for preview.
    pub config: Value,
    #[serde(default)]
    pub documents: Vec<TemplateDocument>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TemplateDocument {
    pub title: String,
    pub kind: DocumentKind,
    pub markdown: String,
    pub tour: Option<CodeTour>,
}

// ---- Documents and code tours ---------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReference {
    pub path: String,
    pub revision: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

/// Lossless tour block: `kind` + raw fields. Use [`TourBlock::content`] for typed access
/// and [`TourBlock::new`] to build one; unknown kinds survive round trips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TourBlock {
    pub id: Id,
    pub kind: String,
    #[serde(flatten)]
    pub fields: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TourBlockContent {
    Markdown { text: String },
    Diagram { title: String, format: String, source: String },
    Code { title: String, language: String, code: String, source: Option<SourceReference> },
    #[serde(rename_all = "camelCase")]
    Diff { title: String, path: String, base_revision: Option<String>, head_revision: Option<String>, patch: String },
}

impl TourBlock {
    pub fn new(id: Id, content: &TourBlockContent) -> Self {
        let mut fields = match serde_json::to_value(content) {
            Ok(Value::Object(map)) => map,
            _ => Extra::new(),
        };
        let kind = match fields.remove("kind") {
            Some(Value::String(kind)) => kind,
            _ => String::new(),
        };
        Self { id, kind, fields }
    }

    /// `None` for unknown or malformed kinds; render a fallback and keep the block.
    pub fn content(&self) -> Option<TourBlockContent> {
        let mut map = self.fields.clone();
        map.insert("kind".into(), Value::String(self.kind.clone()));
        serde_json::from_value(Value::Object(map)).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TourChapter {
    pub id: Id,
    pub title: String,
    pub summary: String,
    pub blocks: Vec<TourBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub commit_shas: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub file_paths: Option<Vec<String>>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TourFileChange {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub previous_path: Option<Option<String>>,
    pub status: FileChangeStatus,
    pub additions: u64,
    pub deletions: u64,
    pub patch: String,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TourCommit {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub created_at: Timestamp,
    pub files: Vec<TourFileChange>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TourCommitGroup {
    pub id: Id,
    pub title: String,
    pub commit_shas: Vec<String>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TourReview {
    pub reviewed_chapter_ids: Vec<Id>,
    pub reviewed_file_paths: Vec<String>,
    pub decision: ReviewDecision,
    pub summary: String,
    #[serde(flatten)] pub extra: Extra,
}

/// Optional fields are omitted when `None`, so absent stays absent on write.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeTour {
    pub chapters: Vec<TourChapter>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub commits: Option<Vec<TourCommit>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub commit_groups: Option<Vec<TourCommitGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub review: Option<TourReview>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub overview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub risk: Option<Level>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub impact: Option<Level>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: Id,
    pub project_id: Id,
    pub task_id: Option<Id>,
    pub title: String,
    pub kind: DocumentKind,
    pub markdown: String,
    pub tour: Option<CodeTour>,
    pub revision: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDocument {
    pub project_id: Id,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub task_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub kind: Option<DocumentKind>,
    #[serde(skip_serializing_if = "Option::is_none")] pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub tour: Option<Option<CodeTour>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDocument {
    #[serde(skip_serializing_if = "Option::is_none")] pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub task_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub tour: Option<Option<CodeTour>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub revision: Option<u64>,
}

// ---- Settings and workspace (whole-object PUT) ----------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub revision: u64,
    pub theme: Theme,
    pub notifications: bool,
    pub default_harness: HarnessId,
    pub onboarding_complete: bool,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTab {
    pub id: Id,
    pub kind: TabKind,
    pub resource_id: Id,
    pub project_id: Id,
    pub title: String,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub revision: u64,
    pub tabs: Vec<WorkspaceTab>,
    pub active_tab_id: Option<Id>,
    pub recently_closed: Vec<WorkspaceTab>,
    pub selected_project_id: Option<Id>,
    #[serde(flatten)] pub extra: Extra,
}

// ---- Runtime metadata, interventions, activity, stats ---------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessInfo {
    pub id: HarnessId,
    pub name: String,
    pub executable: String,
    pub installed: Option<bool>,
    pub authenticated: Option<bool>,
    pub status: String,
    pub execution_available: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ExecutionTargetInfo {
    pub id: ExecutionTarget,
    pub name: String,
    pub kind: String,
    pub available: bool,
    pub requirements: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Intervention {
    pub id: Id,
    pub project_id: Id,
    pub task_id: Option<Id>,
    pub kind: InterventionKind,
    pub message: String,
    pub status: InterventionStatus,
    pub response: Option<String>,
    /// Always `not_implemented`: the intent is stored, never delivered.
    pub execution_status: String,
    pub revision: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIntervention {
    pub project_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")] pub task_id: Option<Id>,
    pub kind: InterventionKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIntervention {
    #[serde(skip_serializing_if = "Option::is_none")] pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub status: Option<InterventionStatus>,
    #[serde(skip_serializing_if = "Option::is_none")] pub response: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: Id,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub kind: String,
    pub message: String,
    pub resource_id: Option<Id>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Statistics {
    pub project_count: u64,
    pub task_count: u64,
    pub document_count: u64,
    pub tour_count: u64,
    pub intervention_count: u64,
    pub tasks_by_status: BTreeMap<String, u64>,
    pub completed_task_count: u64,
    pub completion_rate: f64,
    /// `None` until execution exists; render "Not available", never 0.
    pub run_count: Option<u64>,
    pub tokens: Option<u64>,
    pub cost: Option<f64>,
    pub execution_available: bool,
}

wire_enum!(RunStepStatus {
    Pending = "pending", Running = "running", Completed = "completed",
    Failed = "failed", NeedsApproval = "needs_approval",
});
wire_enum!(SpanKind { Stage = "stage", Agent = "agent", Tool = "tool", Gate = "gate", Other = "other" });

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStep {
    pub id: Id,
    pub title: String,
    pub agent_id: Option<Id>,
    pub status: RunStepStatus,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    #[serde(default)] pub activity: Option<String>,
    #[serde(default)] pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStage {
    pub id: Id,
    pub title: String,
    pub status: RunStepStatus,
    pub steps: Vec<RunStep>,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceSpan {
    pub id: Id,
    pub parent_id: Option<Id>,
    pub name: String,
    pub kind: SpanKind,
    pub status: RunStepStatus,
    pub start_offset_ms: f64,
    pub duration_ms: Option<f64>,
}

/// Read model for a future execution provider. Absent metrics are unknown, not zero.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: Id,
    pub task_id: Id,
    pub workflow_id: Id,
    pub state: String,
    pub created_at: Timestamp,
    #[serde(default)] pub stages: Option<Vec<RunStage>>,
    #[serde(default)] pub steps: Option<Vec<RunStep>>,
    #[serde(default)] pub started_at: Option<Timestamp>,
    #[serde(default)] pub finished_at: Option<Timestamp>,
    #[serde(default)] pub activity: Option<String>,
    #[serde(default)] pub output: Option<String>,
    #[serde(default)] pub duration_ms: Option<f64>,
    #[serde(default)] pub tokens: Option<u64>,
    #[serde(default)] pub cost: Option<f64>,
    #[serde(default)] pub cost_currency: Option<String>,
    #[serde(default)] pub trace_spans: Option<Vec<TraceSpan>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Deleted {
    pub deleted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_tour_block_round_trips() {
        let json = r#"{"id":"b1","kind":"video","url":"x"}"#;
        let block: TourBlock = serde_json::from_str(json).unwrap();
        assert!(block.content().is_none());
        let back: Value = serde_json::to_value(&block).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(json).unwrap());
    }

    #[test]
    fn known_tour_block_parses() {
        let content = TourBlockContent::Markdown { text: "hi".into() };
        let block = TourBlock::new("b".into(), &content);
        assert_eq!(block.kind, "markdown");
        assert_eq!(block.content(), Some(content));
    }

    /// A whole-object PUT must write back values and fields this client does not know.
    #[test]
    fn config_round_trips_unknown_values() {
        let json = r#"{"revision":3,"instructions":[],"skills":[],"workflows":[],
            "executionTarget":"firecracker","futureField":{"a":1},
            "agents":[{"id":"a","name":"A","role":"r","harness":"gemini","model":"m",
              "instructions":"","skillIds":[],"mode":"swarm","executable":"x"}]}"#;
        let config: ProjectConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.agents[0].harness, HarnessId::Unknown("gemini".into()));
        let back = serde_json::to_value(&config).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(json).unwrap());
    }
}
