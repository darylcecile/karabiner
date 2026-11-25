import { TASK_STATUSES, type AppSettings, type Document, type Intervention, type Project, type ProjectConfig, type Statistics, type Task, type WorkspaceState } from "@karabiner/contracts";
import { Store, checkRevision, mergeFields, type Filter, type Table } from "./database.ts";
import { conflict, invalid, missing } from "./errors.ts";
import { templates } from "./templates.ts";
import { configInput, documentInput, interventionInput, interventionUpdate, object, projectInput, settingsInput, taskInput, text, workspaceInput } from "./validation.ts";

export const initialSettings: AppSettings = { revision: 0, theme: "system", notifications: true, defaultHarness: "copilot", onboardingComplete: false };
export const initialWorkspace: WorkspaceState = { revision: 0, tabs: [], activeTabId: null, recentlyClosed: [], selectedProjectId: null };
function identity() { const now = new Date().toISOString(); return { id: crypto.randomUUID(), revision: 1, createdAt: now, updatedAt: now }; }
function next<T extends { revision: number; updatedAt: string }>(current: T, patch: Partial<T>): T { return { ...current, ...patch, revision: current.revision + 1, updatedAt: new Date().toISOString() }; }

export class Repository {
  constructor(readonly store: Store) {}
  createProject(value: unknown): Project {
    const input = projectInput(value, true);
    const template = templates.find(item => item.id === (input.templateId ?? "blank"));
    if (!template) invalid("Unknown templateId");
    return this.store.atomic(() => {
      const project: Project = { ...identity(), name: input.name, description: input.description ?? "", kind: input.kind ?? "standalone", rootPath: input.rootPath ?? null, repository: input.repository ?? null, externalRef: input.externalRef ?? null, color: input.color ?? "blue" };
      this.store.put("projects", project);
      this.store.putConfig(project.id, { ...structuredClone(template.config), revision: 1 });
      for (const document of template.documents ?? []) this.createDocument({ ...document, projectId: project.id });
      this.store.activity("project.created", `Created ${project.name}`, project.id, null, project.id);
      return project;
    });
  }
  updateProject(id: string, value: unknown): Project {
    const patch = projectInput(value, false);
    return this.store.atomic(() => {
      const previous = this.store.get("projects", id); checkRevision(previous, patch.revision);
      const updated = this.store.put("projects", next(previous, patch));
      this.store.activity("project.updated", `Updated ${updated.name}`, id, null, id);
      return updated;
    });
  }
  createTask(value: unknown): Task {
    const input = taskInput(value, true);
    return this.store.atomic(() => {
      const config = this.store.config(input.projectId); this.taskReferences(input, config);
      const task: Task = { ...identity(), projectId: input.projectId, title: input.title, description: input.description ?? "", status: input.status ?? "todo", priority: input.priority ?? "normal", workflowId: input.workflowId ?? null, agentId: input.agentId ?? null, labels: input.labels ?? [], stageId: input.stageId ?? null, dueAt: input.dueAt ?? null, sortOrder: input.sortOrder ?? 0, subtasks: input.subtasks ?? [], executionState: "not_started" };
      this.store.put("tasks", task); this.store.activity("task.created", `Created ${task.title}`, task.projectId, task.id, task.id);
      return task;
    });
  }
  updateTask(id: string, value: unknown): Task {
    const patch = taskInput(value, false);
    return this.store.atomic(() => {
      const previous = this.store.get("tasks", id); checkRevision(previous, patch.revision);
      const task = next(previous, patch); this.taskReferences(task, this.store.config(task.projectId));
      this.store.put("tasks", task);
      const message = task.status !== previous.status ? `${task.title}: ${previous.status} → ${task.status}` : `Updated ${task.title}`;
      this.store.activity("task.updated", message, task.projectId, task.id, task.id);
      return task;
    });
  }
  private taskReferences(task: { workflowId?: string | null; agentId?: string | null; stageId?: string | null }, config: ProjectConfig): void {
    if (task.workflowId && !config.workflows.some(value => value.id === task.workflowId)) invalid("workflowId is not in this project's configuration");
    if (task.agentId && !config.agents.some(value => value.id === task.agentId)) invalid("agentId is not in this project's configuration");
    if (task.stageId && !config.workflows.find(value => value.id === task.workflowId)?.stages.some(value => value.id === task.stageId)) invalid("stageId is not in the task's selected workflow");
  }
  addComment(taskId: string, value: unknown) {
    const message = text(object(value).message, "message", 65_536, false);
    return this.store.atomic(() => {
      const task = this.store.get("tasks", taskId);
      return this.store.activity("task.comment", message, task.projectId, task.id, task.id);
    });
  }
  private taskBelongsTo(taskId: string | null | undefined, projectId: string): void {
    this.store.get("projects", projectId);
    if (taskId && this.store.get("tasks", taskId).projectId !== projectId) invalid("taskId belongs to a different project");
  }
  createDocument(value: unknown): Document {
    const input = documentInput(value, true);
    return this.store.atomic(() => {
      this.taskBelongsTo(input.taskId, input.projectId);
      const kind = input.kind ?? "markdown";
      const document: Document = { ...identity(), projectId: input.projectId, taskId: input.taskId ?? null, title: input.title, kind, markdown: input.markdown ?? "", tour: kind === "code_tour" ? input.tour ?? { chapters: [] } : null };
      if (kind === "markdown" && input.tour) invalid("Markdown documents cannot contain a code tour");
      this.store.put("documents", document); this.store.activity("document.created", `Created ${document.title}`, document.projectId, document.taskId, document.id);
      return document;
    });
  }
  updateDocument(id: string, value: unknown): Document {
    const patch = documentInput(value, false);
    return this.store.atomic(() => {
      const previous = this.store.get("documents", id); checkRevision(previous, patch.revision);
      const document = next(previous, patch);
      if (patch.tour) document.tour = mergeFields(previous.tour, patch.tour) as Document["tour"];
      if (document.kind === "code_tour" && !document.tour) invalid("Code tour documents require tour content");
      if (document.kind === "markdown" && document.tour) invalid("Markdown documents cannot contain a code tour");
      this.taskBelongsTo(document.taskId, document.projectId);
      this.store.put("documents", document); this.store.activity("document.updated", `Updated ${document.title}`, document.projectId, document.taskId, document.id);
      return document;
    });
  }
  saveConfig(projectId: string, value: unknown): ProjectConfig {
    const input = configInput(value);
    return this.store.atomic(() => {
      const previous = this.store.config(projectId); checkRevision(previous, input.revision);
      for (const task of this.store.list("tasks", { projectId })) {
        if (task.workflowId && !input.workflows.some(item => item.id === task.workflowId)) conflict(`Workflow ${task.workflowId} is assigned to a task; unassign it before deleting`);
        if (task.agentId && !input.agents.some(item => item.id === task.agentId)) conflict(`Agent ${task.agentId} is assigned to a task; unassign it before deleting`);
        if (task.stageId && !input.workflows.find(item => item.id === task.workflowId)?.stages.some(item => item.id === task.stageId)) conflict(`Stage ${task.stageId} is assigned to a task; unassign it before deleting`);
      }
      const config = { ...mergeFields(previous, input) as ProjectConfig, revision: previous.revision + 1 };
      this.store.putConfig(projectId, config); this.store.activity("config.updated", "Updated project configuration", projectId, null, projectId);
      return config;
    });
  }
  settings(): AppSettings { return this.store.setting("app", initialSettings); }
  workspace(): WorkspaceState { return this.store.setting("workspace", initialWorkspace); }
  saveSettings(value: unknown): AppSettings { return this.saveSetting("app", settingsInput(value), this.settings()); }
  saveWorkspace(value: unknown): WorkspaceState { return this.saveSetting("workspace", workspaceInput(value), this.workspace()); }
  private saveSetting<T extends { revision: number }>(key: string, input: T, fallback: T): T {
    return this.store.atomic(() => {
      const previous = this.store.setting(key, fallback); checkRevision(previous, input.revision);
      return this.store.putSetting(key, { ...mergeFields(previous, input) as T, revision: previous.revision + 1 });
    });
  }
  createIntervention(value: unknown): Intervention {
    const input = interventionInput(value);
    return this.store.atomic(() => {
      this.taskBelongsTo(input.taskId, input.projectId);
      const intervention: Intervention = { ...identity(), ...input, taskId: input.taskId ?? null, status: "recorded", response: null, executionStatus: "not_implemented" };
      this.store.put("interventions", intervention);
      this.store.activity("intervention.recorded", `Recorded ${input.kind} intent; agent delivery is not connected`, input.projectId, input.taskId ?? null, intervention.id);
      return intervention;
    });
  }
  updateIntervention(id: string, value: unknown): Intervention {
    const patch = interventionUpdate(value);
    return this.store.atomic(() => {
      const previous = this.store.get("interventions", id); checkRevision(previous, patch.revision);
      const updated = next(previous, patch);
      if (updated.status === "answered" && !updated.response) invalid("Answered interventions require a response");
      this.store.put("interventions", updated);
      this.store.activity("intervention.updated", `Intervention ${updated.status}; agent delivery is not connected`, updated.projectId, updated.taskId, updated.id);
      return updated;
    });
  }
  remove(table: Table, id: string): { deleted: true } {
    return this.store.atomic(() => {
      const value = this.store.get(table, id); this.store.remove(table, id);
      this.store.activity(`${table.slice(0, -1)}.deleted`, `Deleted ${"name" in value ? value.name : "title" in value ? value.title : value.kind}`, "projectId" in value ? value.projectId : null, "taskId" in value ? value.taskId : null, id);
      return { deleted: true };
    });
  }
  statistics(filter: Filter): Statistics {
    if (filter.projectId) this.store.get("projects", filter.projectId);
    const tasks = this.store.list("tasks", filter), documents = this.store.list("documents", filter);
    const tasksByStatus = Object.fromEntries(TASK_STATUSES.map(status => [status, tasks.filter(task => task.status === status).length])) as Statistics["tasksByStatus"];
    return { projectCount: filter.projectId ? 1 : this.store.list("projects").length, taskCount: tasks.length, documentCount: documents.length, tourCount: documents.filter(document => document.kind === "code_tour").length,
      interventionCount: this.store.list("interventions", filter).length, tasksByStatus, completedTaskCount: tasksByStatus.done, completionRate: tasks.length ? tasksByStatus.done / tasks.length : 0,
      runCount: null, tokens: null, cost: null, executionAvailable: false };
  }
}
