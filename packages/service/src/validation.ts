import {
	TASK_STATUSES,
	parseProjectConfig,
	validateFactory,
	FactoryCompileError,
	validateStrandsModelConfig,
	parseAgentDefinition,
	type AgentDefinition,
	type HarnessCheckInput,
	type CreateRun,
	type RunControl,
	type AnswerRunQuestion,
	type AddRunInstruction,
	type IntegrationPushInput,
	type AppSettings,
	type CodeTour,
	type CreateDocument,
	type CreateIntervention,
	type CreateProject,
	type CreateTask,
	type ProjectConfig,
	type UpdateDocument,
	type UpdateIntervention,
	type UpdateProject,
	type UpdateTask,
	type WorkspaceState,
} from "@karabiner/contracts";
import { invalid } from "./errors.ts";
import { validateIntegrationConfig } from "./integrations/validation.ts";

export type ObjectValue = Record<string, unknown>;
export function object(value: unknown, label = "Body"): ObjectValue {
	if (!value || typeof value !== "object" || Array.isArray(value))
		invalid(`${label} must be an object`);

	return value as ObjectValue;
}
export function text(value: unknown, label: string, maximum = 65_536, allowEmpty = true): string {
	if (typeof value !== "string" || value.length > maximum || (!allowEmpty && !value.trim()))
		invalid(
			`${label} must be ${allowEmpty ? "a" : "a nonempty"} string of at most ${maximum} characters`,
		);

	return value;
}
function nullableText(value: unknown, label: string): string | null {
	return value === null ? null : text(value, label, 4096);
}

function choice<const T extends readonly string[]>(
	value: unknown,
	values: T,
	label: string,
): T[number] {
	if (typeof value !== "string" || !values.includes(value))
		invalid(`${label} must be one of: ${values.join(", ")}`);

	return value;
}

function boolean(value: unknown, label: string): boolean {
	if (typeof value !== "boolean") invalid(`${label} must be a boolean`);
	return value;
}

function integer(value: unknown, label: string, minimum = 0): number {
	if (typeof value !== "number" || !Number.isSafeInteger(value) || value < minimum)
		invalid(`${label} must be an integer >= ${minimum}`);

	return value;
}

function finite(value: unknown, label: string): number {
	if (typeof value !== "number" || !Number.isFinite(value))
		invalid(`${label} must be a finite number`);

	return value;
}

function array(value: unknown, label: string): unknown[] {
	if (!Array.isArray(value)) invalid(`${label} must be an array`);
	return value;
}

function strings(value: unknown, label: string): string[] {
	return array(value, label).map((item) => text(item, label, 4096));
}

function id(value: unknown, label = "ID"): string {
	return text(value, label, 256, false);
}

function revision(body: ObjectValue): { revision?: number } {
	return body.revision === undefined ? {} : { revision: integer(body.revision, "revision") };
}

export function projectInput(value: unknown, creating: true): CreateProject;
export function projectInput(value: unknown, creating: false): UpdateProject;
export function projectInput(value: unknown, creating: boolean): CreateProject | UpdateProject {
	const b = object(value);
	const result: UpdateProject & { templateId?: string } = revision(b);
	if (creating || b.name !== undefined) result.name = text(b.name, "name", 256, false).trim();
	if (b.description !== undefined) result.description = text(b.description, "description");

	if (b.kind !== undefined)
		result.kind = choice(b.kind, ["standalone", "github", "linear"], "kind");

	for (const key of ["rootPath", "repository", "externalRef"] as const)
		if (b[key] !== undefined) result[key] = nullableText(b[key], key);

	if (b.color !== undefined) result.color = text(b.color, "color", 64);
	if (creating && b.templateId !== undefined) result.templateId = id(b.templateId, "templateId");
	return result;
}
export function taskInput(value: unknown, creating: true): CreateTask;
export function taskInput(value: unknown, creating: false): UpdateTask;
export function taskInput(value: unknown, creating: boolean): CreateTask | UpdateTask {
	const b = object(value);
	const result: UpdateTask & { projectId?: string } = revision(b);
	if (creating) result.projectId = id(b.projectId, "projectId");

	if (creating || b.title !== undefined)
		result.title = text(b.title, "title", 4096, false).trim();

	if (b.description !== undefined) result.description = text(b.description, "description");
	if (b.status !== undefined) result.status = choice(b.status, TASK_STATUSES, "status");

	if (b.priority !== undefined)
		result.priority = choice(b.priority, ["low", "normal", "high", "urgent"], "priority");

	for (const key of ["workflowId", "agentId", "stageId"] as const)
		if (b[key] !== undefined) result[key] = b[key] === null ? null : id(b[key], key);

	if (b.labels !== undefined) result.labels = strings(b.labels, "labels");

	if (b.dueAt !== undefined) {
		result.dueAt = b.dueAt === null ? null : text(b.dueAt, "dueAt", 128, false);

		if (result.dueAt !== null) {
			if (!Number.isFinite(Date.parse(result.dueAt)))
				invalid("dueAt must be an ISO timestamp or null");

			result.dueAt = new Date(result.dueAt).toISOString();
		}
	}

	if (b.sortOrder !== undefined) result.sortOrder = finite(b.sortOrder, "sortOrder");

	if (b.subtasks !== undefined) {
		const values = array(b.subtasks, "subtasks");
		uniqueIds(values, "subtasks");

		result.subtasks = values.map((value) => {
			const s = object(value);

			return {
				id: id(s.id),
				title: text(s.title, "subtask.title", 4096, false),
				completed: boolean(s.completed, "subtask.completed"),
			};
		});
	}

	return result;
}
function sourceReference(value: unknown): void {
	if (value === null) return;
	const b = object(value, "source");
	text(b.path, "source.path", 4096, false);
	nullableText(b.revision, "source.revision");

	for (const key of ["startLine", "endLine"] as const)
		if (b[key] !== null) integer(b[key], key, 1);

	if (typeof b.startLine === "number" && typeof b.endLine === "number" && b.endLine < b.startLine)
		invalid("endLine must not precede startLine");
}

function tourBlock(value: unknown): void {
	const b = object(value, "tour block");
	id(b.id);
	text(b.kind, "block.kind", 128, false);

	switch (b.kind) {
		case "markdown":
			text(b.text, "block.text", 1_000_000);
			return;
		case "diagram":
			text(b.title, "diagram.title");
			choice(b.format, ["mermaid", "text"], "diagram.format");
			text(b.source, "diagram.source", 1_000_000);
			return;
		case "code":
			text(b.title, "code.title");
			text(b.language, "code.language", 128);
			text(b.code, "code.code", 1_000_000);
			sourceReference(b.source);
			return;
		case "diff":
			text(b.title, "diff.title");
			text(b.path, "diff.path", 4096, false);
			nullableText(b.baseRevision, "baseRevision");
			nullableText(b.headRevision, "headRevision");
			text(b.patch, "diff.patch", 1_000_000);
			return;
		default:
			return; // Preserve additive block kinds for a newer client.
	}
}
export function tourInput(value: unknown): CodeTour | null {
	if (value === null) return null;
	const b = object(value, "tour");
	const chapters = array(b.chapters, "chapters");
	uniqueIds(chapters, "chapters");

	for (const value of chapters) {
		const c = object(value, "chapter");
		id(c.id);
		text(c.title, "chapter.title", 4096, false);
		text(c.summary, "chapter.summary");
		const blocks = array(c.blocks, "blocks");
		uniqueIds(blocks, "blocks");
		blocks.forEach(tourBlock);
		if (c.commitShas !== undefined) strings(c.commitShas, "commitShas");
		if (c.filePaths !== undefined) strings(c.filePaths, "filePaths");
	}

	if (b.commits !== undefined)
		for (const value of array(b.commits, "commits")) {
			const c = object(value, "commit");
			id(c.sha, "commit.sha");
			text(c.message, "commit.message");
			text(c.author, "commit.author");
			text(c.createdAt, "commit.createdAt");

			for (const value of array(c.files, "commit.files")) {
				const f = object(value, "file");
				text(f.path, "file.path", 4096, false);
				choice(f.status, ["added", "modified", "deleted", "renamed"], "file.status");
				integer(f.additions, "additions");
				integer(f.deletions, "deletions");
				text(f.patch, "file.patch", 1_000_000);
				if (f.previousPath !== undefined) nullableText(f.previousPath, "previousPath");
			}
		}

	if (b.commitGroups !== undefined) {
		const groups = array(b.commitGroups, "commitGroups");
		uniqueIds(groups, "commitGroups");

		for (const value of groups) {
			const g = object(value);
			id(g.id);
			text(g.title, "group.title");
			strings(g.commitShas, "group.commitShas");
		}
	}

	if (b.review !== undefined) {
		const r = object(b.review);
		strings(r.reviewedChapterIds, "reviewedChapterIds");
		strings(r.reviewedFilePaths, "reviewedFilePaths");
		choice(r.decision, ["pending", "approved", "changes_requested"], "review.decision");
		text(r.summary, "review.summary");
	}

	if (b.overview !== undefined) text(b.overview, "overview");

	for (const key of ["risk", "impact"] as const)
		if (b[key] !== undefined) choice(b[key], ["low", "medium", "high"], key);

	return b as unknown as CodeTour;
}
export function documentInput(value: unknown, creating: true): CreateDocument;
export function documentInput(value: unknown, creating: false): UpdateDocument;
export function documentInput(value: unknown, creating: boolean): CreateDocument | UpdateDocument {
	const b = object(value);

	const result: UpdateDocument & { projectId?: string; kind?: "markdown" | "code_tour" } =
		revision(b);

	if (creating) result.projectId = id(b.projectId, "projectId");

	if (creating || b.title !== undefined)
		result.title = text(b.title, "title", 4096, false).trim();

	if (b.taskId !== undefined) result.taskId = b.taskId === null ? null : id(b.taskId, "taskId");

	if (creating && b.kind !== undefined)
		result.kind = choice(b.kind, ["markdown", "code_tour"], "kind");

	if (b.markdown !== undefined) result.markdown = text(b.markdown, "markdown", 1_000_000);
	if (b.tour !== undefined) result.tour = tourInput(b.tour);
	return result;
}
function uniqueIds(values: unknown[], label: string): Set<string> {
	const ids = values.map((value) => id(object(value, label).id, `${label}.id`));
	if (new Set(ids).size !== ids.length) invalid(`${label} IDs must be unique`);
	return new Set(ids);
}

export function configInput(value: unknown): ProjectConfig {
	const result = validateFactory(value, { executable: false });
	if (!result.valid) throw new FactoryCompileError(result.errors);
	const config = parseProjectConfig(value);
	if (config.integration !== undefined) validateIntegrationConfig(config.integration);
	return config;
}
export function settingsInput(value: unknown): AppSettings {
	const b = object(value);
	integer(b.revision, "revision");
	choice(b.theme, ["system", "light", "dark"], "theme");
	if (b.motion !== undefined) choice(b.motion, ["system", "reduce", "full"], "motion");
	boolean(b.notifications, "notifications");
	boolean(b.onboardingComplete, "onboardingComplete");
	choice(b.defaultHarness, ["claude", "codex", "copilot", "custom", "strands"], "defaultHarness");
	return b as unknown as AppSettings;
}
export function workspaceInput(value: unknown): WorkspaceState {
	const b = object(value);
	integer(b.revision, "revision");

	const tabs = array(b.tabs, "tabs");
	const closed = array(b.recentlyClosed, "recentlyClosed");
	const ids = uniqueIds(tabs, "tabs");

	if (closed.length > 50) invalid("recentlyClosed must contain at most 50 tabs");

	for (const value of [...tabs, ...closed]) {
		const t = object(value);
		id(t.id);
		choice(t.kind, ["project", "task", "document", "review"], "tab.kind");
		id(t.resourceId, "resourceId");
		id(t.projectId, "projectId");
		text(t.title, "title", 4096);
	}

	if (b.activeTabId !== null && !ids.has(id(b.activeTabId, "activeTabId")))
		invalid("activeTabId must reference an open tab");

	if (b.selectedProjectId !== null) id(b.selectedProjectId, "selectedProjectId");
	return b as unknown as WorkspaceState;
}
export function interventionInput(value: unknown): CreateIntervention {
	const b = object(value);

	return {
		projectId: id(b.projectId, "projectId"),
		taskId: b.taskId == null ? null : id(b.taskId, "taskId"),
		kind: choice(
			b.kind,
			["instruction", "question", "pause", "resume", "cancel", "retry", "approve", "reject"],
			"kind",
		),
		message: text(b.message, "message", 65_536, false),
	};
}
export function interventionUpdate(value: unknown): UpdateIntervention {
	const b = object(value);
	const result: UpdateIntervention = revision(b);

	if (b.status !== undefined)
		result.status = choice(b.status, ["answered", "dismissed"], "status");

	if (b.response !== undefined)
		result.response = b.response === null ? null : text(b.response, "response", 65_536, false);

	return result;
}

export function runInput(value: unknown): CreateRun {
	const b = object(value);

	return {
		...revision(b),
		...(b.workflowId !== undefined ? { workflowId: id(b.workflowId, "workflowId") } : {}),
	};
}
export function controlInput(value: unknown): RunControl {
	const b = object(value);

	return {
		...revision(b),
		...(b.message !== undefined ? { message: text(b.message, "message") } : {}),
	};
}
export function answerInput(value: unknown): AnswerRunQuestion {
	const b = object(value);
	const result: AnswerRunQuestion = revision(b);
	if (b.answer !== undefined) result.answer = text(b.answer, "answer", 65_536, false);
	if (b.choice !== undefined) result.choice = text(b.choice, "choice", 4096, false);

	if (b.answers !== undefined)
		result.answers = Object.fromEntries(
			Object.entries(object(b.answers, "answers")).map(([key, value]) => [
				key,
				Array.isArray(value)
					? strings(value, `answers.${key}`)
					: text(value, `answers.${key}`),
			]),
		);

	if (Object.hasOwn(b, "value")) result.value = b.value as AnswerRunQuestion["value"];

	if (
		result.answer === undefined &&
		result.choice === undefined &&
		result.answers === undefined &&
		!Object.hasOwn(result, "value")
	)
		invalid("Provide answer, choice, answers or value for the pending request");

	return result;
}

export function instructionInput(value: unknown): AddRunInstruction {
	const b = object(value);
	return { ...revision(b), message: text(b.message, "message", 65_536, false) };
}

/** Readiness consumes only these provider fields; it never dispatches this agent. */
export function strandsReadinessInput(value: unknown): AgentDefinition {
	const agent = object(object(value).agent, "agent");
	choice(agent.harness, ["strands"], "agent.harness");
	text(agent.model, "agent.model", 256);
	if (agent.strands !== undefined) validateStrandsModelConfig(agent.strands, "agent.strands");
	return agent as unknown as AgentDefinition;
}

export function harnessCheckInput(value: unknown): HarnessCheckInput {
	const body = object(value);

	return {
		agent: parseAgentDefinition(body.agent),
		...(body.check === undefined ? {} : { check: boolean(body.check, "check") }),
		...(body.projectRoot === undefined
			? {}
			: { projectRoot: text(body.projectRoot, "projectRoot", 4096, false) }),
	};
}
export function syncInput(value: unknown): { conflict?: "preserve-local" | "overwrite-local" } {
	const b = object(value);

	return b.conflict === undefined
		? {}
		: { conflict: choice(b.conflict, ["preserve-local", "overwrite-local"], "conflict") };
}
export function pushInput(value: unknown): IntegrationPushInput {
	const b = object(value);

	return {
		revision: integer(b.revision, "revision"),
		requestId: id(b.requestId, "requestId"),
		...(b.fields !== undefined
			? {
					fields: array(b.fields, "fields").map((field) =>
						choice(
							field,
							["title", "description", "status", "priority", "labels", "dueAt"],
							"fields",
						),
					),
				}
			: {}),
		...(b.createMissing !== undefined
			? { createMissing: boolean(b.createMissing, "createMissing") }
			: {}),
	};
}
