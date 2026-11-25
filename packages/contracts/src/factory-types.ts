import type { AgentDefinition, ExecutionTargetInfo, HarnessInfo, Id, InstructionDefinition, Json, Project, ProjectConfig, Run, SkillDefinition, StageDefinition, Task, TaskStatus, Timestamp } from "./index.ts";

/** Success/failure select one successor; return selects its explicitly reported target. */
export type StageOutcome = "success" | "failure" | "return";
export interface WorkflowEdge {
  id: Id; sourceStageId: Id; targetStageId: Id; on: StageOutcome;
}
export interface WorkflowFork { id: Id; stageId: Id; branches: { id: Id; entryStageId: Id }[]; joinStageId: Id }
export interface WorkflowGraph { entryStageId: Id; edges: WorkflowEdge[]; maxTransitions?: number; forks?: WorkflowFork[] }
export interface WorkflowLayout { positions: Record<Id, { x: number; y: number }> }
export interface CommandDefinition {
  id: Id; name?: string; executable: string; args: string[];
  cwd?: string; timeoutMs?: number; env?: Record<string, string>;
  retries?: number; retryDelayMs?: number;
}
export type GateDefinition =
  | { id: Id; kind: "command"; name?: string; command: CommandDefinition }
  | { id: Id; kind: "approval"; name?: string; prompt: string };
export interface StageExecution {
  before?: CommandDefinition[]; commands?: CommandDefinition[];
  gates?: GateDefinition[]; after?: CommandDefinition[];
  timeoutMs?: number;
  /** Extra attempts per step, excluding denial, cancellation and shutdown. */
  retries?: number; retryDelayMs?: number;
  cwd?: string; executionTarget?: ProjectConfig["executionTarget"]; targetOptions?: TargetOptions;
}
export interface SchedulerConfig {
  enabled: boolean; workflowId?: Id; statuses?: TaskStatus[];
  maxConcurrentRuns?: number; pollIntervalMs?: number;
}
export interface TargetOptions {
  image?: string; network?: boolean; writablePaths?: string[];
  executable?: string; profilePath?: string; readonlyPaths?: string[]; keychainAccess?: boolean;
  guestExecutable?: string; guestWorkdir?: string; envNames?: string[]; cpus?: number;
  memory?: string; guestHarnessConfigured?: boolean;
}
export interface ResolvedAgent extends AgentDefinition { skills: SkillDefinition[]; resolvedInstructions: string }
export interface StrandsModelConfig {
  provider: "bedrock" | "anthropic" | "openai" | "google" | "ollama" | "litellm";
  credentialEnv?: string; baseUrl?: string; region?: string; api?: "chat" | "responses";
}
export interface StrandsReadiness { available: boolean; authenticated: null; code: string; credentialEnv?: string; message: string }
export interface HarnessReadiness extends HarnessInfo {
  code: string; credentialEnv?: string;
  supported: { permissions: boolean; questions: boolean; steering: "native-turn" | "native-tool-boundary" | "next-step" };
  checks: { code: string; message: string; status: "passed" | "unknown" | "failed" }[];
}
export interface HarnessCheckInput { agent: AgentDefinition; check?: boolean; projectRoot?: string }
export interface CompiledStage extends StageDefinition {
  agents: ResolvedAgent[]; resolvedInstructions: string; maxVisits: number; execution: StageExecution;
}
export interface CompiledWorkflow {
  id: Id; name: string; description: string; instructions: string;
  entryStageId: Id; stages: CompiledStage[]; edges: WorkflowEdge[]; maxTransitions: number;
  forks?: WorkflowFork[];
  stageScopes?: Record<Id, string>;
}
/** Pure data, detached from mutable config, with no layout or callbacks. */
export interface FactoryDefinition {
  schemaVersion: 1; id: string; configRevision: number;
  instructions: InstructionDefinition[]; skills: SkillDefinition[]; agents: ResolvedAgent[];
  workflows: CompiledWorkflow[]; executionTarget: ProjectConfig["executionTarget"];
  targetOptions?: TargetOptions;
}
export interface FactoryIssue { code: string; path: string; message: string }
export interface FactoryValidation { valid: boolean; errors: FactoryIssue[] }
export interface FactoryFile { path: string; content: string }
export interface FactoryBundle {
  format: "karabiner-factory"; version: 1; config: ProjectConfig;
  definition: FactoryDefinition; files: FactoryFile[];
}
export const RUN_STATES = ["queued", "running", "waiting", "paused", "completed", "failed", "cancelled"] as const;
export type RunState = typeof RUN_STATES[number];
export interface RunSnapshot {
  project: Project; task: Task; config: ProjectConfig; factory: FactoryDefinition;
}
export interface RunError { code: string; message: string; stageId?: Id; stepId?: Id; stageInvocationId?: Id; attemptId?: Id; details?: Json }
export interface RunArtifact { id: Id; kind: "file" | "document" | "code_tour"; name: string; path?: string; documentId?: Id; mediaType?: string; stageInvocationId?: Id; stepId?: Id; attemptId?: Id; tokenId?: Id; scopeId?: Id | null; branchId?: Id | null }
export interface RunQuestion {
  id: Id; stageId: Id; stepId: Id | null; kind: "question" | "approval" | "permission"; prompt: string;
  state: "pending" | "answered" | "cancelled"; createdAt: Timestamp;
  answer: string | null; answeredAt: Timestamp | null;
  stageInvocationId?: Id; tokenId?: Id; attemptId?: Id; requestId?: string; provider?: string;
  transport?: "live" | "checkpoint"; choices?: InteractionChoice[];
  payload?: Json; response?: InteractionResponse;
  tool?: InteractionTool; fields?: InteractionField[];
}
export interface InteractionChoice { id: string; label: string; effect?: "approve" | "deny" | "cancel" }
export interface InteractionTool { name: string; kind: "shell" | "read" | "write" | "network" | "mcp" | "other"; command?: string; cwd?: string; paths?: string[]; diff?: string; urls?: string[] }
export interface InteractionField {
  id: string; label: string; type: "text" | "select" | "multiselect" | "boolean" | "number" | "integer";
  options?: { id: string; label: string }[]; required?: boolean; allowOther?: boolean;
  minLength?: number; maxLength?: number; minItems?: number; maxItems?: number; minimum?: number; maximum?: number;
}
export interface InteractionResponse { choice?: string; answer?: string; answers?: Record<string, string | string[]>; value?: Json }
export interface RunInstruction {
  id: Id; message: string; createdAt: Timestamp; deliveredAt: Timestamp | null; stepId: Id | null;
  deliveries?: { stepId: Id; stageInvocationId: Id; attemptId?: Id; deliveredAt: Timestamp; mode: "native-turn" | "native-tool-boundary" | "next-step" }[];
}
export interface RunStepAttempt {
  id: Id; number: number; status: "running" | "waiting" | "completed" | "failed" | "cancelled";
  startedAt: Timestamp; finishedAt: Timestamp | null; output?: string;
  error?: RunError | null; tokens?: number | null; cost?: number | null;
  sessionId?: string;
  denied?: boolean;
  artifacts?: RunArtifact[];
}
export interface RunToken {
  id: Id; scopeId: Id | null; branchId: Id | null; stageId: Id;
  stageInvocationId: Id | null; history: Id[];
  status: "ready" | "running" | "waiting" | "paused" | "joining" | "completed" | "failed" | "cancelled";
}
export interface RunScope {
  id: Id; forkId: Id; parentTokenId: Id; joinStageId: Id; branchTokenIds: Id[];
  state: "running" | "joined" | "failed" | "cancelled"; joinInvocationId: Id | null;
}
export interface RunExecutionState { version: 1; tokens: RunToken[]; scopes: RunScope[] }
export interface StoredRun extends Run {
  projectId: Id; revision: number; updatedAt: Timestamp; snapshot: RunSnapshot;
  retryOf: Id | null; currentStageId: Id | null; questions: RunQuestion[];
  errors: RunError[]; artifacts: RunArtifact[]; transitionCount: number;
  trigger: "manual" | "scheduler" | "retry";
}
export interface CreateRun { workflowId?: Id; revision?: number }
export interface RunControl { revision?: number; message?: string }
export interface AnswerRunQuestion extends RunControl, InteractionResponse {}
export interface AddRunInstruction { message: string; revision?: number }
export interface RunFilter { projectId?: Id; taskId?: Id; state?: RunState }
export interface IntegrationInfo {
  id: "github" | "linear"; configured: boolean; available: boolean;
  authenticated: boolean | null; message: string; checkedAt: Timestamp;
  code?: string;
  status?: "unknown" | "ready" | "unavailable" | "error";
  refresh?: { enabled: boolean; intervalMs: number; running: boolean; lastAttemptAt?: Timestamp; lastSyncedAt?: Timestamp; nextSyncAt?: Timestamp; error?: string };
}
export interface IntegrationRefresh { enabled: boolean; intervalMs?: number }
export type SyncField = "title" | "description" | "status" | "priority" | "labels" | "dueAt";
export type IntegrationConfig =
  | { provider: "github"; scope: "repository" | "project"; hostname?: string; pushEnabled?: boolean; pushFields?: SyncField[]; statusFieldId?: string; pullStatusMap?: Record<string, TaskStatus>; pushStatusMap?: Partial<Record<TaskStatus, string>>; refresh?: IntegrationRefresh }
  | { provider: "linear"; scope: "team" | "project"; apiKeyEnv: string; pushEnabled?: boolean; pushFields?: SyncField[]; teamId?: string; pullStatusMap?: Record<string, TaskStatus>; pushStatusMap?: Record<string, Partial<Record<TaskStatus, string>>>; refresh?: IntegrationRefresh };
export interface IntegrationCatalog {
  provider: "github" | "linear"; projectId: Id; source: { id: string; name: string; url?: string };
  teams: { id: string; name: string; key?: string; states: { id: string; name: string; type?: string; color?: string; suggestedStatus?: TaskStatus }[] }[];
  statusFields: { id: string; name: string; options: { id: string; name: string; color?: string; suggestedStatus?: TaskStatus }[] }[];
  writableFields: SyncField[]; checkedAt: Timestamp;
}
export interface TargetReadiness extends ExecutionTargetInfo {
  installed: boolean; configured: boolean;
  checks: { code: string; path: string; message: string; status: "passed" | "unknown" | "failed" }[];
}
export interface ExternalTaskReference {
  provider: "github" | "linear"; id: string; identifier: string; url: string; updatedAt: Timestamp;
  projectItemId?: string; kind?: "issue" | "draft" | "pull_request";
}
export interface IntegrationError { id?: string; code: string; message: string }
export interface IntegrationSyncResult {
  provider: "github" | "linear"; projectId: Id; imported: number; updated: number; skipped: number;
  conflicts: number; errors: IntegrationError[]; syncedAt: Timestamp; message?: string;
}
export interface IntegrationPushInput {
  revision: number; requestId: string;
  fields?: SyncField[]; createMissing?: boolean;
}
export interface IntegrationPushResult {
  taskId: Id; requestId: string; status: "pushed" | "unchanged" | "uncertain" | "failed";
  external?: ExternalTaskReference; errors: IntegrationError[];
}
