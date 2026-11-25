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
    #[serde(default)] pub factory_compilation: bool,
    #[serde(default)] pub run_controls: bool,
    #[serde(default)] pub parallel_execution: bool,
    #[serde(default)] pub step_retries: bool,
    #[serde(default)] pub interactive_control: bool,
    #[serde(default)] pub integration_catalog: bool,
    #[serde(default)] pub integration_refresh: bool,
    #[serde(default)] pub target_readiness: bool,
    #[serde(default)] pub strands_configuration: bool,
    #[serde(default)] pub strands_readiness: bool,
    #[serde(default)] pub harness_readiness: bool,
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
wire_enum!(HarnessId { Claude = "claude", Codex = "codex", Copilot = "copilot", Strands = "strands", Custom = "custom" });
wire_enum!(AgentMode { Interactive = "interactive", Autonomous = "autonomous" });
wire_enum!(ExecutionTarget { Host = "host", Mxc = "mxc", AppleContainer = "apple-container" });
wire_enum!(DocumentKind { Markdown = "markdown", CodeTour = "code_tour" });
wire_enum!(Theme { System = "system", Light = "light", Dark = "dark" });
wire_enum!(Motion { System = "system", Reduce = "reduce", Full = "full" });
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
    /// `not_started`, or the state of the latest actual run (`RunState` wire values).
    pub execution_state: String,
    #[serde(default)]
    pub latest_run_id: Option<Id>,
    /// Linked GitHub/Linear item, once synced or pushed.
    #[serde(default)]
    pub external: Option<ExternalTaskReference>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")] pub instruction_ids: Option<Vec<Id>>,
    /// Custom harness wire protocol: `jsonl-v1` (default) or `jsonrpc-v2`.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub protocol: Option<String>,
    /// Strands model provider; credentials by environment variable name only.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub strands: Option<StrandsModelConfig>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrandsModelConfig {
    /// `bedrock`, `anthropic`, `openai`, `google`, `ollama` or `litellm`.
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub credential_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub region: Option<String>,
    /// OpenAI: `chat` or `responses`.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub api: Option<String>,
    #[serde(flatten)] pub extra: Extra,
}

/// `POST /v1/harnesses/strands/check {agent}`: local configuration and environment only.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrandsReadiness {
    pub available: bool,
    pub code: String,
    #[serde(default)] pub credential_env: Option<String>,
    pub message: String,
}

/// `POST /v1/harnesses/check`: readiness of one agent as drafted. `check: false` looks at
/// PATH and configuration only; `true` also asks known harnesses for their sign-in state.
#[derive(Debug, Clone, Serialize)]
pub struct CheckHarness<'a> {
    pub agent: &'a AgentDefinition,
    pub check: bool,
    #[serde(rename = "projectRoot", skip_serializing_if = "Option::is_none")] pub project_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessReadiness {
    pub executable: String,
    pub installed: Option<bool>,
    pub authenticated: Option<bool>,
    pub execution_available: bool,
    pub message: String,
    #[serde(default)] pub code: Option<String>,
    #[serde(default)] pub credential_env: Option<String>,
    #[serde(default)] pub supported: Option<HarnessSupport>,
    #[serde(default)] pub checks: Vec<ReadinessCheck>,
}

/// What the harness adapter can do while running.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HarnessSupport {
    pub permissions: bool,
    pub questions: bool,
    /// `native-turn`, `native-tool-boundary` or `next-step`.
    pub steering: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")] pub instruction_ids: Option<Vec<Id>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub execution: Option<StageExecution>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub max_visits: Option<u32>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDefinition {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub stages: Vec<StageDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub instruction_ids: Option<Vec<Id>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub graph: Option<WorkflowGraph>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub layout: Option<WorkflowLayout>,
    #[serde(flatten)] pub extra: Extra,
}

// ---- Factory graph and execution (mirror of `factory-types.ts`) ----------------------

wire_enum!(StageOutcome { Success = "success", Failure = "failure", Return = "return" });
wire_enum!(GateKind { Command = "command", Approval = "approval" });

/// Each outcome selects one successor; no implicit parallel fork/join.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEdge {
    pub id: Id,
    pub source_stage_id: Id,
    pub target_stage_id: Id,
    pub on: StageOutcome,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowGraph {
    pub entry_stage_id: Id,
    pub edges: Vec<WorkflowEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub max_transitions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub forks: Option<Vec<WorkflowFork>>,
    #[serde(flatten)] pub extra: Extra,
}

/// Parallel branches: `stage_id` fans out to each branch entry; all branches meet at
/// `join_stage_id`. A fork stage owns its success routing (no ordinary success edge).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowFork {
    pub id: Id,
    pub stage_id: Id,
    pub branches: Vec<ForkBranch>,
    pub join_stage_id: Id,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkBranch {
    pub id: Id,
    pub entry_stage_id: Id,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayoutPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowLayout {
    pub positions: BTreeMap<Id, LayoutPosition>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDefinition {
    pub id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub name: Option<String>,
    pub executable: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub retry_delay_ms: Option<u64>,
    #[serde(flatten)] pub extra: Extra,
}

/// `kind: command` carries `command`; `kind: approval` carries `prompt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateDefinition {
    pub id: Id,
    pub kind: GateKind,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub command: Option<CommandDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub prompt: Option<String>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageExecution {
    #[serde(default, skip_serializing_if = "Option::is_none")] pub before: Option<Vec<CommandDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub commands: Option<Vec<CommandDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub gates: Option<Vec<GateDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub after: Option<Vec<CommandDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub timeout_ms: Option<u64>,
    /// Extra attempts per step (not after denial, cancellation or shutdown).
    #[serde(default, skip_serializing_if = "Option::is_none")] pub retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub retry_delay_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub execution_target: Option<ExecutionTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub target_options: Option<TargetOptions>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerConfig {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub workflow_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub statuses: Option<Vec<TaskStatus>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub max_concurrent_runs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub poll_interval_ms: Option<u64>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")] pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub network: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub writable_paths: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub executable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub profile_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub readonly_paths: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub keychain_access: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub guest_executable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub guest_workdir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub env_names: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cpus: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub memory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub guest_harness_configured: Option<bool>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FactoryIssue {
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FactoryValidation {
    pub valid: bool,
    pub errors: Vec<FactoryIssue>,
}

/// `POST /v1/projects/:id/factory/validate` body.
#[derive(Debug, Clone, Serialize)]
pub struct ValidateFactory<'a> {
    pub config: &'a ProjectConfig,
}

/// `POST /v1/projects/:id/factory/import` body; `bundle` is a `FactoryBundle` kept opaque.
#[derive(Debug, Clone, Serialize)]
pub struct ImportFactory {
    pub revision: u64,
    pub bundle: Value,
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
    #[serde(default, skip_serializing_if = "Option::is_none")] pub target_options: Option<TargetOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub scheduler: Option<SchedulerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub integration: Option<IntegrationConfig>,
    #[serde(flatten)] pub extra: Extra,
}

// ---- Integrations (GitHub / Linear) --------------------------------------------------

/// `ProjectConfig.integration`. One shape for both providers; the service rejects fields the
/// provider doesn't take, so the editor only sets that provider's fields. Credentials are
/// references (GitHub: `gh` auth; Linear: `api_key_env` names an environment variable),
/// never secret values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationConfig {
    /// `github` or `linear`.
    pub provider: String,
    /// GitHub: `repository` | `project`; Linear: `team` | `project`.
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub push_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub status_field_id: Option<String>,
    /// External status name → task status.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub pull_status_map: Option<BTreeMap<String, TaskStatus>>,
    /// GitHub: task status → option id. Linear: team id → (task status → state id); kept raw.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub push_status_map: Option<Value>,
    /// Task fields an explicit Push sends (`title`, `description`, `status`, `priority`,
    /// `labels`, `dueAt`); the service defaults to title and description.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub push_fields: Option<Vec<String>>,
    /// Periodic automatic pull (never push). Off unless enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub refresh: Option<IntegrationRefresh>,
    #[serde(flatten)] pub extra: Extra,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationRefresh {
    pub enabled: bool,
    /// 30 s to 24 h; the service defaults to 5 min.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub interval_ms: Option<u64>,
    #[serde(flatten)] pub extra: Extra,
}

/// Observed refresh state (in memory on the service).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationRefreshStatus {
    pub enabled: bool,
    pub interval_ms: u64,
    pub running: bool,
    #[serde(default)] pub last_synced_at: Option<Timestamp>,
    #[serde(default)] pub next_sync_at: Option<Timestamp>,
    #[serde(default)] pub error: Option<String>,
}

/// `GET /v1/projects/:id/integration/catalog`: names for the ids the config stores.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationCatalog {
    pub provider: String,
    pub source: CatalogSource,
    #[serde(default)] pub teams: Vec<CatalogTeam>,
    #[serde(default)] pub status_fields: Vec<CatalogField>,
    /// Fields this source accepts on push.
    #[serde(default)] pub writable_fields: Vec<String>,
    pub checked_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CatalogSource {
    pub id: String,
    pub name: String,
    #[serde(default)] pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CatalogTeam {
    pub id: String,
    pub name: String,
    #[serde(default)] pub key: Option<String>,
    #[serde(default)] pub states: Vec<CatalogOption>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CatalogField {
    pub id: String,
    pub name: String,
    #[serde(default)] pub options: Vec<CatalogOption>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogOption {
    pub id: String,
    pub name: String,
    #[serde(default)] pub suggested_status: Option<TaskStatus>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationInfo {
    pub id: String,
    pub configured: bool,
    pub available: bool,
    pub authenticated: Option<bool>,
    pub message: String,
    pub checked_at: Timestamp,
    #[serde(default)] pub status: Option<String>,
    /// e.g. `credential_env_missing`, `source_not_configured`, `executable_missing`.
    #[serde(default)] pub code: Option<String>,
    #[serde(default)] pub refresh: Option<IntegrationRefreshStatus>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct IntegrationError {
    #[serde(default)] pub id: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationSyncResult {
    pub provider: String,
    pub imported: u64,
    pub updated: u64,
    pub skipped: u64,
    #[serde(default)] pub conflicts: u64,
    #[serde(default)] pub errors: Vec<IntegrationError>,
    pub synced_at: Timestamp,
    #[serde(default)] pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationPushInput {
    pub revision: u64,
    /// Idempotency key; reuse it when retrying the same push.
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub fields: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub create_missing: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ExternalTaskReference {
    pub provider: String,
    pub id: String,
    pub identifier: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationPushResult {
    /// `pushed` | `unchanged` | `uncertain` | `failed`.
    pub status: String,
    #[serde(default)] pub external: Option<ExternalTaskReference>,
    #[serde(default)] pub errors: Vec<IntegrationError>,
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
    /// App animations: follow macOS, reduce, or full. Never changes the macOS setting.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub motion: Option<Motion>,
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
    #[serde(default)] pub version: Option<String>,
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

/// `POST /v1/projects/:id/execution-target/check`: readiness of the configured target.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetReadiness {
    pub id: ExecutionTarget,
    pub available: bool,
    #[serde(default)] pub message: String,
    #[serde(default)] pub installed: bool,
    #[serde(default)] pub configured: bool,
    #[serde(default)] pub checks: Vec<ReadinessCheck>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReadinessCheck {
    pub code: String,
    #[serde(default)] pub path: String,
    pub message: String,
    /// `passed`, `unknown` or `failed`.
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckTarget<'a> {
    pub check: bool,
    /// The unsaved draft, so the form can be checked before saving.
    #[serde(skip_serializing_if = "Option::is_none")] pub config: Option<&'a ProjectConfig>,
    /// The unsaved project checkout (General → Local checkout), when edited.
    #[serde(rename = "projectRoot", skip_serializing_if = "Option::is_none")] pub project_root: Option<String>,
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
    /// `not_implemented`, `pending`, `delivered` or `resolved`.
    pub execution_status: String,
    #[serde(default)] pub run_id: Option<Id>,
    #[serde(default)] pub question_id: Option<Id>,
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
    #[serde(default)] pub runs_by_state: BTreeMap<String, u64>,
    #[serde(default)] pub completed_run_count: Option<u64>,
    #[serde(default)] pub failed_run_count: Option<u64>,
    #[serde(default)] pub average_duration_ms: Option<f64>,
}

wire_enum!(RunStepStatus {
    Pending = "pending", Running = "running", Completed = "completed",
    Failed = "failed", NeedsApproval = "needs_approval", Cancelled = "cancelled", Skipped = "skipped",
});
wire_enum!(RunState {
    Queued = "queued", Running = "running", Waiting = "waiting", Paused = "paused",
    Completed = "completed", Failed = "failed", Cancelled = "cancelled",
});
wire_enum!(QuestionState { Pending = "pending", Answered = "answered", Cancelled = "cancelled" });

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunError {
    pub code: String,
    pub message: String,
    #[serde(default)] pub stage_id: Option<Id>,
    #[serde(default)] pub step_id: Option<Id>,
    #[serde(default)] pub attempt_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunArtifact {
    pub id: Id,
    pub kind: String,
    pub name: String,
    #[serde(default)] pub path: Option<String>,
    #[serde(default)] pub document_id: Option<Id>,
}

/// A question or approval a run is waiting on; answered through `POST /v1/runs/:id/questions/:qid/answer`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunQuestion {
    pub id: Id,
    pub stage_id: Id,
    pub step_id: Option<Id>,
    /// `question`, `approval` or `permission`.
    pub kind: String,
    pub prompt: String,
    pub state: QuestionState,
    pub created_at: Timestamp,
    pub answer: Option<String>,
    pub answered_at: Option<Timestamp>,
    /// Harness that asked (permission requests), and whether it waits live or resumes later.
    #[serde(default)] pub provider: Option<String>,
    #[serde(default)] pub transport: Option<String>,
    /// Offered choices (e.g. allow once / always / deny); answer with `choice`.
    #[serde(default)] pub choices: Vec<QuestionChoice>,
    /// Raw request details as the harness reported them (shown when `tool` is absent).
    #[serde(default)] pub payload: Option<Value>,
    /// Normalised tool request (permission questions).
    #[serde(default)] pub tool: Option<QuestionTool>,
    /// Structured answer fields; answered with `answers`.
    #[serde(default)] pub fields: Vec<QuestionField>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct QuestionChoice {
    pub id: String,
    pub label: String,
    /// `approve`, `deny` or `cancel`.
    #[serde(default)] pub effect: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct QuestionTool {
    pub name: String,
    /// `shell`, `read`, `write`, `network`, `mcp` or `other`.
    pub kind: String,
    #[serde(default)] pub command: Option<String>,
    #[serde(default)] pub cwd: Option<String>,
    #[serde(default)] pub paths: Vec<String>,
    #[serde(default)] pub diff: Option<String>,
    #[serde(default)] pub urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionField {
    pub id: String,
    pub label: String,
    /// `text`, `select`, `multiselect`, `boolean`, `number` or `integer`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)] pub options: Vec<FieldOption>,
    #[serde(default)] pub required: bool,
    /// Selects accept a typed answer besides the options.
    #[serde(default)] pub allow_other: bool,
    #[serde(default)] pub min_length: Option<usize>,
    #[serde(default)] pub max_length: Option<usize>,
    #[serde(default)] pub min_items: Option<usize>,
    #[serde(default)] pub max_items: Option<usize>,
    #[serde(default)] pub minimum: Option<f64>,
    #[serde(default)] pub maximum: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FieldOption {
    pub id: String,
    pub label: String,
}

/// Guidance for the run's next agent steps; `delivered_at` is set once a step received it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInstruction {
    pub id: Id,
    pub message: String,
    pub created_at: Timestamp,
    #[serde(default)] pub delivered_at: Option<Timestamp>,
    #[serde(default)] pub step_id: Option<Id>,
    #[serde(default)] pub deliveries: Vec<InstructionDelivery>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionDelivery {
    pub step_id: Id,
    pub delivered_at: Timestamp,
    /// `native-turn`, `native-tool-boundary` or `next-step`.
    pub mode: String,
}

/// `POST /v1/runs/:id/instructions` body.
#[derive(Debug, Clone, Serialize)]
pub struct AddRunInstruction {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub revision: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRun {
    #[serde(skip_serializing_if = "Option::is_none")] pub workflow_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")] pub revision: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RunControl {
    #[serde(skip_serializing_if = "Option::is_none")] pub revision: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AnswerRunQuestion {
    #[serde(skip_serializing_if = "Option::is_none")] pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub answer: Option<String>,
    /// A choice id from `RunQuestion.choices`.
    #[serde(skip_serializing_if = "Option::is_none")] pub choice: Option<String>,
    /// Legacy string form answers (field id → text or list of option ids).
    #[serde(skip_serializing_if = "Option::is_none")] pub answers: Option<BTreeMap<String, Value>>,
    /// Typed form answers (field id → string, number, boolean or list), the canonical form.
    #[serde(skip_serializing_if = "Option::is_none")] pub value: Option<Value>,
}
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
    /// `agent`, `command` or `gate`.
    #[serde(default)] pub kind: Option<String>,
    #[serde(default)] pub error: Option<RunError>,
    #[serde(default)] pub tokens: Option<u64>,
    #[serde(default)] pub cost: Option<f64>,
    #[serde(default)] pub attempts: Vec<RunStepAttempt>,
    /// Set while an automatic retry is scheduled.
    #[serde(default)] pub retry_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStepAttempt {
    pub id: Id,
    pub number: u32,
    pub status: String,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    #[serde(default)] pub error: Option<RunError>,
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
    /// Workflow stage id (the run stage `id` is per visit).
    #[serde(default)] pub stage_id: Option<Id>,
    #[serde(default)] pub visit: Option<u32>,
    #[serde(default)] pub outcome: Option<String>,
    #[serde(default)] pub error: Option<RunError>,
    /// Parallel branch this visit ran in, if any.
    #[serde(default)] pub branch_id: Option<Id>,
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

/// Durable observed execution. Absent metrics are unknown, not zero.
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
    #[serde(default)] pub project_id: Option<Id>,
    #[serde(default)] pub revision: Option<u64>,
    #[serde(default)] pub updated_at: Option<Timestamp>,
    #[serde(default)] pub retry_of: Option<Id>,
    #[serde(default)] pub current_stage_id: Option<Id>,
    /// Every stage running now (several while parallel branches run).
    #[serde(default)] pub active_stage_ids: Vec<Id>,
    #[serde(default)] pub questions: Vec<RunQuestion>,
    #[serde(default)] pub instructions: Vec<RunInstruction>,
    #[serde(default)] pub errors: Vec<RunError>,
    #[serde(default)] pub artifacts: Vec<RunArtifact>,
    #[serde(default)] pub pause_requested: bool,
    #[serde(default)] pub transition_count: Option<u32>,
    #[serde(default)] pub trigger: Option<String>,
}

impl Run {
    pub fn run_state(&self) -> RunState {
        RunState::from(self.state.as_str())
    }

    pub fn is_active(&self) -> bool {
        matches!(self.run_state(), RunState::Queued | RunState::Running | RunState::Waiting | RunState::Paused)
    }
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
