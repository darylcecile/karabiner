/** Karabiner wire API v1. JSON over loopback HTTP; all success replies are { data }. */
import type { RunState, StageExecution, WorkflowGraph, WorkflowLayout, SchedulerConfig, TargetOptions, RunSnapshot, RunQuestion, RunError, RunArtifact, RunInstruction, IntegrationConfig, ExternalTaskReference, RunExecutionState, RunStepAttempt, StrandsModelConfig } from "./factory-types.ts";
export * from "./factory-types.ts";
export * from "./factory.ts";
export const API_VERSION = 1;
export const DEFAULT_SERVICE_URL = "http://127.0.0.1:43173";
export type Id = string;
export type Timestamp = string;
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export interface Reply<T> { data: T }
export interface ApiError { error: { code: string; message: string; details?: Json } }
export interface Versioned { revision: number; createdAt: Timestamp; updatedAt: Timestamp }

export interface ServiceMeta {
  apiVersion: number;
  serviceVersion: string;
  capabilities: {
    storage: boolean; configuration: boolean; documents: boolean; codeTours: boolean;
    interventionIntents: boolean; activity: boolean; statistics: boolean;
    scheduler: boolean; harnessExecution: boolean; integrations: boolean; virtualMachines: boolean; events?: boolean;
    factoryCompilation?: boolean; runControls?: boolean;
    /** Implemented wire semantics; installation/authentication readiness is reported separately. */
    parallelExecution?: boolean; stepRetries?: boolean; interactiveControl?: boolean;
    integrationCatalog?: boolean; integrationRefresh?: boolean; targetReadiness?: boolean;
    strandsConfiguration?: boolean; strandsReadiness?: boolean;
    harnessReadiness?: boolean;
  };
}
export type ProjectKind = "standalone" | "github" | "linear";
export interface Project extends Versioned {
  id: Id; name: string; description: string; kind: ProjectKind;
  rootPath: string | null; repository: string | null; externalRef: string | null; color: string;
}
export interface CreateProject {
  name: string; description?: string; kind?: ProjectKind; rootPath?: string | null;
  repository?: string | null; externalRef?: string | null; color?: string; templateId?: string;
}
export type UpdateProject = Partial<Omit<CreateProject, "templateId">> & { revision?: number };
export const TASK_STATUSES = ["backlog", "todo", "in_progress", "in_review", "blocked", "done"] as const;
export type TaskStatus = typeof TASK_STATUSES[number];
export type Priority = "low" | "normal" | "high" | "urgent";
export interface Subtask { id: Id; title: string; completed: boolean }
export interface Task extends Versioned {
  id: Id; projectId: Id; title: string; description: string; status: TaskStatus;
  priority: Priority; workflowId: Id | null; agentId: Id | null; labels: string[];
  stageId: Id | null; dueAt: Timestamp | null; sortOrder: number; subtasks: Subtask[];
  /** Projected from the latest actual run; CRUD alone does not start execution. */
  executionState: "not_started" | RunState;
  latestRunId?: Id | null;
  external?: ExternalTaskReference;
}
export interface CreateTask {
  projectId: Id; title: string; description?: string; status?: TaskStatus;
  priority?: Priority; workflowId?: Id | null; agentId?: Id | null; labels?: string[];
  stageId?: Id | null; dueAt?: Timestamp | null; sortOrder?: number; subtasks?: Subtask[];
}
export type UpdateTask = Partial<Omit<CreateTask, "projectId">> & { revision?: number };
export type HarnessId = "claude" | "codex" | "copilot" | "custom" | "strands";
export interface AgentDefinition {
  id: Id; name: string; role: string; harness: HarnessId; model: string;
  instructions: string; skillIds: Id[]; mode: "interactive" | "autonomous";
  executable?: string | null; args?: string[];
  instructionIds?: Id[];
  protocol?: "jsonl-v1" | "jsonrpc-v2";
  strands?: StrandsModelConfig;
}
export interface SkillDefinition { id: Id; name: string; description: string; content: string; path: string | null; sourcePath?: string }
export interface InstructionDefinition { id: Id; name: string; content: string; path: string | null; sourcePath?: string }
export interface StageDefinition {
  id: Id; name: string; instructions: string; agentIds: Id[]; returnToStageIds: Id[];
  exitRule?: string; status?: TaskStatus; color?: string;
  instructionIds?: Id[]; execution?: StageExecution; maxVisits?: number;
  kind?: "stage" | "fork" | "join";
}
export interface WorkflowDefinition {
  id: Id; name: string; description: string; instructions: string; stages: StageDefinition[];
  instructionIds?: Id[]; graph?: WorkflowGraph; layout?: WorkflowLayout;
}
export interface ProjectConfig {
  revision: number; instructions: InstructionDefinition[]; skills: SkillDefinition[];
  agents: AgentDefinition[]; workflows: WorkflowDefinition[];
  executionTarget: "host" | "mxc" | "apple-container";
  targetOptions?: TargetOptions; scheduler?: SchedulerConfig;
  integration?: IntegrationConfig;
}
export interface Template {
  id: string; name: string; description: string; config: Omit<ProjectConfig, "revision">;
  documents?: { title: string; kind: DocumentKind; markdown: string; tour: CodeTour | null }[];
}

export interface SourceReference { path: string; revision: string | null; startLine: number | null; endLine: number | null }
export type TourBlock =
  | { id: Id; kind: "markdown"; text: string }
  | { id: Id; kind: "diagram"; title: string; format: "mermaid" | "text"; source: string }
  | { id: Id; kind: "code"; title: string; language: string; code: string; source: SourceReference | null }
  | { id: Id; kind: "diff"; title: string; path: string; baseRevision: string | null; headRevision: string | null; patch: string };
export interface TourChapter { id: Id; title: string; summary: string; blocks: TourBlock[]; commitShas?: string[]; filePaths?: string[] }
export interface TourFileChange {
  path: string; previousPath?: string | null; status: "added" | "modified" | "deleted" | "renamed";
  additions: number; deletions: number; patch: string;
}
export interface TourCommit { sha: string; message: string; author: string; createdAt: Timestamp; files: TourFileChange[] }
export interface TourCommitGroup { id: Id; title: string; commitShas: string[] }
export interface TourReview {
  reviewedChapterIds: Id[]; reviewedFilePaths: string[];
  decision: "pending" | "approved" | "changes_requested"; summary: string;
}
export interface CodeTour {
  chapters: TourChapter[]; commits?: TourCommit[]; commitGroups?: TourCommitGroup[];
  review?: TourReview; overview?: string;
  risk?: "low" | "medium" | "high"; impact?: "low" | "medium" | "high";
}
export type DocumentKind = "markdown" | "code_tour";
export interface Document extends Versioned {
  id: Id; projectId: Id; taskId: Id | null; title: string; kind: DocumentKind;
  markdown: string; tour: CodeTour | null;
}
export interface CreateDocument {
  projectId: Id; taskId?: Id | null; title: string; kind?: DocumentKind;
  markdown?: string; tour?: CodeTour | null;
}
export type UpdateDocument = Partial<Omit<CreateDocument, "projectId" | "kind">> & { revision?: number };

export interface AppSettings {
  revision: number; theme: "system" | "light" | "dark"; notifications: boolean;
  defaultHarness: HarnessId; onboardingComplete: boolean;
  motion?: "system" | "reduce" | "full";
}
export type TabKind = "project" | "task" | "document" | "review";
export interface WorkspaceTab { id: Id; kind: TabKind; resourceId: Id; projectId: Id; title: string }
export interface WorkspaceState {
  revision: number; tabs: WorkspaceTab[]; activeTabId: Id | null;
  recentlyClosed: WorkspaceTab[]; selectedProjectId: Id | null;
}
export interface HarnessInfo {
  id: HarnessId; name: string; executable: string;
  installed: boolean | null; authenticated: boolean | null;
  status: "not_checked" | "available" | "unavailable" | "error"; executionAvailable: boolean; message: string;
  checkedAt?: Timestamp; version?: string | null;
}
export interface ExecutionTargetInfo {
  id: "host" | "mxc" | "apple-container"; name: string; kind: "host" | "process_sandbox" | "virtual_machine";
  available: boolean; requirements: string; message: string; checkedAt?: Timestamp;
}
export type InterventionKind = "instruction" | "question" | "pause" | "resume" | "cancel" | "retry" | "approve" | "reject";
export interface Intervention extends Versioned {
  id: Id; projectId: Id; taskId: Id | null; kind: InterventionKind; message: string;
  status: "recorded" | "answered" | "dismissed"; response: string | null;
  executionStatus: "not_implemented" | "pending" | "delivered" | "resolved";
  runId?: Id | null; questionId?: Id | null;
}
export interface CreateIntervention { projectId: Id; taskId?: Id | null; kind: InterventionKind; message: string }
export interface UpdateIntervention { revision?: number; status?: "answered" | "dismissed"; response?: string | null }
export interface Activity {
  id: Id; projectId: Id | null; taskId: Id | null; kind: string; message: string;
  resourceId: Id | null; createdAt: Timestamp;
}
export interface Statistics {
  projectCount: number; taskCount: number; documentCount: number; tourCount: number;
  interventionCount: number; tasksByStatus: Record<TaskStatus, number>;
  completedTaskCount: number; completionRate: number;
  runCount: number | null; tokens: number | null; cost: number | null; executionAvailable: boolean;
  runsByState?: Partial<Record<RunState, number>>; completedRunCount?: number; failedRunCount?: number;
  averageDurationMs?: number | null; costCurrency?: string;
}
export type RunStepStatus = "pending" | "running" | "completed" | "failed" | "needs_approval" | "cancelled" | "skipped";
export interface RunStep {
  id: Id; title: string; agentId: Id | null; status: RunStepStatus;
  startedAt: Timestamp | null; finishedAt: Timestamp | null;
  activity?: string; output?: string;
  kind?: "agent" | "command" | "gate"; error?: RunError | null; artifacts?: RunArtifact[];
  tokens?: number | null; cost?: number | null;
  attempts?: RunStepAttempt[]; retryAt?: Timestamp | null;
  sessionId?: string; control?: { permissions: boolean; questions: boolean; steering: "native-turn" | "native-tool-boundary" | "next-step" };
}
export interface RunStage {
  id: Id; title: string; status: RunStepStatus; steps: RunStep[];
  startedAt: Timestamp | null; finishedAt: Timestamp | null;
  stageId?: Id; visit?: number; outcome?: string; error?: RunError | null;
  tokenId?: Id; scopeId?: Id | null; branchId?: Id | null;
}
export interface TraceSpan {
  id: Id; parentId: Id | null; name: string; kind: "stage" | "agent" | "tool" | "gate" | "other";
  status: RunStepStatus; startOffsetMs: number; durationMs: number | null;
}
/** Durable observed execution. Optional additions retain compatibility with v1 readers. */
export interface Run {
  id: Id; taskId: Id; workflowId: Id; state: RunState; createdAt: Timestamp;
  stages?: RunStage[]; steps?: RunStep[]; startedAt?: Timestamp | null; finishedAt?: Timestamp | null;
  activity?: string; output?: string; durationMs?: number | null; tokens?: number | null;
  cost?: number | null; costCurrency?: string; traceSpans?: TraceSpan[];
  projectId?: Id; revision?: number; updatedAt?: Timestamp;
  snapshot?: RunSnapshot; retryOf?: Id | null; currentStageId?: Id | null;
  questions?: RunQuestion[]; errors?: RunError[]; artifacts?: RunArtifact[];
  instructions?: RunInstruction[];
  execution?: RunExecutionState; activeStageIds?: Id[];
  pauseRequested?: boolean; transitionCount?: number; trigger?: "manual" | "scheduler" | "retry";
}

/** Methods return Reply<T>; deletes return Reply<{deleted:true}>. Lists return arrays.
 * PATCH accepts only changed fields. revision is optional optimistic concurrency:
 * if supplied and stale, return 409 conflict. PUT config/settings/workspace requires
 * revision from the last read (initial value 0). Unknown JSON fields are preserved
 * in config/tour documents to support additive v1 updates.
 * Config PUT replaces known values: omit optional known fields to clear them;
 * supplied dynamic maps replace their entries. Unknown fields survive on retained objects.
 * All IDs are strings, all dates ISO UTC. No default seed data is installed.
 */
export const ENDPOINTS = {
  meta: "GET /v1/meta -> ServiceMeta",
  projects: "GET /v1/projects -> Project[]; POST CreateProject -> Project",
  project: "GET|PATCH UpdateProject|DELETE /v1/projects/:id -> Project",
  tasks: "GET /v1/tasks?projectId= -> Task[]; POST CreateTask -> Task",
  task: "GET|PATCH UpdateTask|DELETE /v1/tasks/:id -> Task",
  taskComments: "POST /v1/tasks/:id/comments {message:string} -> Activity (kind task.comment)",
  documents: "GET /v1/documents?projectId=&taskId= -> Document[]; POST CreateDocument -> Document",
  document: "GET|PATCH UpdateDocument|DELETE /v1/documents/:id -> Document",
  config: "GET|PUT ProjectConfig /v1/projects/:id/config -> ProjectConfig",
  templates: "GET /v1/templates -> Template[]",
  settings: "GET|PUT AppSettings /v1/settings -> AppSettings",
  workspace: "GET|PUT WorkspaceState /v1/workspace -> WorkspaceState",
  harnesses: "GET /v1/harnesses -> HarnessInfo[]",
  strandsCheck: "POST /v1/harnesses/strands/check {agent:AgentDefinition} -> StrandsReadiness (local configuration/environment references only)",
  harnessCheck: "POST /v1/harnesses/check HarnessCheckInput -> HarnessReadiness (check:true uses supported nonmutating local authentication probes)",
  executionTargets: "GET /v1/execution-targets?projectId=&check=true -> ExecutionTargetInfo[]",
  interventions: "GET /v1/interventions?projectId=&taskId= -> Intervention[]; POST CreateIntervention -> Intervention",
  intervention: "GET|PATCH UpdateIntervention|DELETE /v1/interventions/:id -> Intervention",
  activity: "GET /v1/activity?projectId=&taskId=&limit=100 -> Activity[] (newest first; max 500)",
  stats: "GET /v1/stats?projectId= -> Statistics",
  runs: "GET /v1/runs?projectId=&taskId=&state= -> Run[]",
  runDetail: "GET /v1/runs/:id -> Run",
  run: "POST /v1/tasks/:id/run CreateRun -> Run (202)",
  runControl: "POST /v1/runs/:id/pause|resume|cancel|retry RunControl -> Run",
  answer: "POST /v1/runs/:id/questions/:questionId/answer AnswerRunQuestion -> Run",
  instruct: "POST /v1/runs/:id/instructions AddRunInstruction -> Run (guidance for the next agent invocation)",
  factory: "GET /v1/projects/:id/factory -> FactoryDefinition; POST /v1/projects/:id/factory/validate|compile {config?:ProjectConfig}",
  factoryExport: "GET /v1/projects/:id/factory/export -> FactoryBundle; POST /v1/projects/:id/factory/import {revision,bundle} -> ProjectConfig",
  integration: "GET /v1/projects/:id/integration?check=true -> IntegrationInfo; POST /v1/projects/:id/sync -> IntegrationSyncResult",
  integrationCatalog: "GET /v1/projects/:id/integration/catalog -> IntegrationCatalog",
  targetCheck: "POST /v1/projects/:id/execution-target/check {check?,purpose?,executable?,config?,projectRoot?} -> TargetReadiness",
  push: "POST /v1/tasks/:id/push IntegrationPushInput -> IntegrationPushResult",
  events: "GET /v1/events -> SSE ready/change events; invalidate and refetch, no durable replay",
} as const;
