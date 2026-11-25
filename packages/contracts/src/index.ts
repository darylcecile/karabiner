/** Karabiner wire API v1. JSON over loopback HTTP; all success replies are { data }. */
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
  /** Stored work item state is independent from a future execution/run. */
  executionState: "not_started";
}
export interface CreateTask {
  projectId: Id; title: string; description?: string; status?: TaskStatus;
  priority?: Priority; workflowId?: Id | null; agentId?: Id | null; labels?: string[];
  stageId?: Id | null; dueAt?: Timestamp | null; sortOrder?: number; subtasks?: Subtask[];
}
export type UpdateTask = Partial<Omit<CreateTask, "projectId">> & { revision?: number };
export type HarnessId = "claude" | "codex" | "copilot" | "custom";
export interface AgentDefinition {
  id: Id; name: string; role: string; harness: HarnessId; model: string;
  instructions: string; skillIds: Id[]; mode: "interactive" | "autonomous";
  executable?: string | null; args?: string[];
}
export interface SkillDefinition { id: Id; name: string; description: string; content: string; path: string | null }
export interface InstructionDefinition { id: Id; name: string; content: string; path: string | null }
export interface StageDefinition {
  id: Id; name: string; instructions: string; agentIds: Id[]; returnToStageIds: Id[];
  exitRule?: string; status?: TaskStatus; color?: string;
}
export interface WorkflowDefinition { id: Id; name: string; description: string; instructions: string; stages: StageDefinition[] }
export interface ProjectConfig {
  revision: number; instructions: InstructionDefinition[]; skills: SkillDefinition[];
  agents: AgentDefinition[]; workflows: WorkflowDefinition[];
  executionTarget: "host" | "mxc" | "apple-container";
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
  status: "not_checked"; executionAvailable: false; message: string;
}
export interface ExecutionTargetInfo {
  id: "host" | "mxc" | "apple-container"; name: string; kind: "host" | "process_sandbox" | "virtual_machine";
  available: false; requirements: string; message: string;
}
export type InterventionKind = "instruction" | "question" | "pause" | "resume" | "cancel" | "retry" | "approve" | "reject";
export interface Intervention extends Versioned {
  id: Id; projectId: Id; taskId: Id | null; kind: InterventionKind; message: string;
  status: "recorded" | "answered" | "dismissed"; response: string | null;
  /** Recording intent is real; execution/agent delivery remains unimplemented. */
  executionStatus: "not_implemented";
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
  runCount: null; tokens: null; cost: null; executionAvailable: false;
}
export type RunStepStatus = "pending" | "running" | "completed" | "failed" | "needs_approval";
export interface RunStep {
  id: Id; title: string; agentId: Id | null; status: RunStepStatus;
  startedAt: Timestamp | null; finishedAt: Timestamp | null;
  activity?: string; output?: string;
}
export interface RunStage {
  id: Id; title: string; status: RunStepStatus; steps: RunStep[];
  startedAt: Timestamp | null; finishedAt: Timestamp | null;
}
export interface TraceSpan {
  id: Id; parentId: Id | null; name: string; kind: "stage" | "agent" | "tool" | "gate" | "other";
  status: RunStepStatus; startOffsetMs: number; durationMs: number | null;
}
/** Read model for a future execution provider. Absent metrics are unknown, not zero. */
export interface Run {
  id: Id; taskId: Id; workflowId: Id; state: string; createdAt: Timestamp;
  stages?: RunStage[]; steps?: RunStep[]; startedAt?: Timestamp | null; finishedAt?: Timestamp | null;
  activity?: string; output?: string; durationMs?: number | null; tokens?: number | null;
  cost?: number | null; costCurrency?: string; traceSpans?: TraceSpan[];
}

/** Methods return Reply<T>; deletes return Reply<{deleted:true}>. Lists return arrays.
 * PATCH accepts only changed fields. revision is optional optimistic concurrency:
 * if supplied and stale, return 409 conflict. PUT config/settings/workspace requires
 * revision from the last read (initial value 0). Unknown JSON fields are preserved
 * in config/tour documents to support additive v1 updates.
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
  executionTargets: "GET /v1/execution-targets -> ExecutionTargetInfo[]",
  interventions: "GET /v1/interventions?projectId=&taskId= -> Intervention[]; POST CreateIntervention -> Intervention",
  intervention: "GET|PATCH UpdateIntervention|DELETE /v1/interventions/:id -> Intervention",
  activity: "GET /v1/activity?projectId=&taskId=&limit=100 -> Activity[] (newest first; max 500)",
  stats: "GET /v1/stats?projectId= -> Statistics",
  runs: "GET /v1/runs?taskId= -> Run[] (empty until execution exists)",
  runDetail: "GET /v1/runs/:id -> Run (structured 404 until a provider has a record)",
  run: "POST /v1/tasks/:id/run -> 501 not_implemented",
  events: "GET /v1/events -> SSE ready/change events; invalidate and refetch, no durable replay",
} as const;
