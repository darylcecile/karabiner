import type {
	Intervention,
	RunFilter,
	RunSnapshot,
	RunState,
	StoredRun,
	Task,
	TaskStatus,
} from "@karabiner/contracts";
import { Store, checkRevision } from "./database.ts";
import { conflict, invalid, missing } from "./errors.ts";

interface RunRow {
	data: string;
}

interface CreateOptions {
	workflowId: string;
	trigger?: StoredRun["trigger"];
	retryOf?: string | null;
}

const activeStates = new Set<RunState>(["queued", "running", "waiting", "paused"]);

const transitions: Record<RunState, ReadonlySet<RunState>> = {
	queued: new Set(["queued", "running", "paused", "cancelled", "failed"]),
	running: new Set([
		"running",
		"queued",
		"waiting",
		"paused",
		"completed",
		"failed",
		"cancelled",
	]),
	waiting: new Set(["waiting", "running", "queued", "paused", "failed", "cancelled"]),
	paused: new Set(["paused", "queued", "running", "waiting", "cancelled", "failed"]),
	completed: new Set(["completed"]),
	failed: new Set(["failed"]),
	cancelled: new Set(["cancelled"]),
};

function lostInvocation(run: StoredRun): boolean {
	if (["completed", "failed", "cancelled"].includes(run.state)) return false;
	if (run.state === "running") return true;

	return (run.stages ?? []).some((stage) =>
		stage.steps.some(
			(step) =>
				step.control ||
				(step.status === "needs_approval" &&
					run.questions.findLast((question) => question.stepId === step.id)?.transport ===
						"live"),
		),
	);
}

function taskStatus(run: StoredRun): TaskStatus {
	if (run.state === "completed") return "done";
	if (["waiting", "paused", "failed"].includes(run.state)) return "blocked";
	if (run.state === "cancelled" || run.state === "queued") return run.snapshot.task.status;

	return (
		run.snapshot.factory.workflows
			.find((workflow) => workflow.id === run.workflowId)
			?.stages.find((stage) => stage.id === run.currentStageId)?.status ?? "in_progress"
	);
}

/** All state writes are synchronous transactions; no network work runs inside them. */
export class RunStore {
	constructor(
		readonly store: Store,
		private readonly changed: (path: string) => void = () => {},
	) {}

	get(id: string): StoredRun | null {
		const row = this.store.db
			.query<RunRow, [string]>("SELECT data FROM runs WHERE id=?")
			.get(id);

		return row ? (JSON.parse(row.data) as StoredRun) : null;
	}
	list(filter: RunFilter = {}): StoredRun[] {
		const clauses: string[] = [];
		const values: string[] = [];

		for (const [column, value] of [
			["project_id", filter.projectId],
			["task_id", filter.taskId],
			["state", filter.state],
		])
			if (value) {
				clauses.push(`${column}=?`);
				values.push(value);
			}

		return this.store.db
			.query<RunRow, string[]>(
				`SELECT data FROM runs ${clauses.length ? `WHERE ${clauses.join(" AND ")}` : ""} ORDER BY rowid DESC`,
			)
			.all(...values)
			.map((row) => JSON.parse(row.data) as StoredRun);
	}
	hasScheduled(taskId: string): boolean {
		return !!this.store.db.query("SELECT 1 FROM scheduled_tasks WHERE task_id=?").get(taskId);
	}
	create(snapshot: RunSnapshot, options: CreateOptions): StoredRun {
		const result = this.store.atomic(() => {
			const task = this.store.get("tasks", snapshot.task.id);
			if (options.trigger !== "retry") checkRevision(task, snapshot.task.revision);

			if (this.list({ taskId: task.id }).some((run) => activeStates.has(run.state)))
				conflict("This task already has an active run");

			if (options.trigger === "scheduler" && this.hasScheduled(task.id))
				conflict(
					"This task has already been scheduled; retry its run or explicitly change its status",
				);

			if (!snapshot.factory.workflows.some((workflow) => workflow.id === options.workflowId))
				invalid("The workflow is not in the captured factory");

			const now = new Date().toISOString();

			const run: StoredRun = {
				id: crypto.randomUUID(),
				projectId: task.projectId,
				taskId: task.id,
				workflowId: options.workflowId,
				state: "queued",
				revision: 1,
				createdAt: now,
				updatedAt: now,
				snapshot: structuredClone(snapshot),
				retryOf: options.retryOf ?? null,
				currentStageId: null,
				questions: [],
				errors: [],
				artifacts: [],
				transitionCount: 0,
				trigger: options.trigger ?? "manual",
				stages: [],
				steps: [],
				traceSpans: [],
				startedAt: null,
				finishedAt: null,
				durationMs: null,
				tokens: null,
				cost: null,
				activity:
					"Queued. Runs sharing a project folder wait for earlier runs; resume or cancel a paused run to release its folder.",
			};

			const workspace = snapshot.project.rootPath ?? `project:${task.projectId}`;

			this.store.db
				.query(
					"INSERT INTO runs(id,project_id,task_id,state,workspace_key,data) VALUES(?,?,?,?,?,?)",
				)
				.run(run.id, run.projectId, run.taskId, run.state, workspace, JSON.stringify(run));

			this.store.db
				.query(
					"INSERT INTO scheduled_tasks(task_id,run_id) VALUES(?,?) ON CONFLICT(task_id) DO UPDATE SET run_id=excluded.run_id",
				)
				.run(task.id, run.id);

			this.projectTask(run);

			this.store.activity(
				"run.created",
				`Queued run for ${task.title}`,
				task.projectId,
				task.id,
				run.id,
			);

			return run;
		});

		this.notify(result);
		return result;
	}
	update(value: StoredRun): StoredRun {
		const result = this.store.atomic(() => {
			const current = this.get(value.id);
			if (!current) missing("Run");
			checkRevision(current, value.revision);

			if (!transitions[current.state].has(value.state))
				conflict(`Cannot move a ${current.state} run to ${value.state}`);

			const run: StoredRun = {
				...value,
				id: current.id,
				taskId: current.taskId,
				projectId: current.projectId,
				workflowId: current.workflowId,
				snapshot: current.snapshot,
				retryOf: current.retryOf,
				trigger: current.trigger,
				createdAt: current.createdAt,
				revision: current.revision + 1,
				updatedAt: new Date().toISOString(),
			};

			this.store.db
				.query("UPDATE runs SET state=?,data=? WHERE id=?")
				.run(run.state, JSON.stringify(run), run.id);

			this.projectTask(run);
			this.projectQuestions(run);

			if (run.state !== current.state || run.currentStageId !== current.currentStageId)
				this.store.activity(
					"run.updated",
					`Run ${run.state}${run.currentStageId ? ` · ${run.currentStageId}` : ""}`,
					run.projectId,
					run.taskId,
					run.id,
				);

			return run;
		});

		this.notify(result);
		return result;
	}
	claimNext(excludedProjectIds: string[] = [], excludedRunIds: string[] = []): StoredRun | null {
		return this.store.atomic(() => {
			const excluded = excludedProjectIds.length
				? `AND candidate.project_id NOT IN (${excludedProjectIds.map(() => "?").join(",")})`
				: "";

			const active = excludedRunIds.length
				? `AND candidate.id NOT IN (${excludedRunIds.map(() => "?").join(",")})`
				: "";

			const row = this.store.db
				.query<RunRow, string[]>(
					`SELECT candidate.data FROM runs candidate WHERE candidate.state='queued' ${excluded} ${active} AND NOT EXISTS (SELECT 1 FROM runs active WHERE active.workspace_key=candidate.workspace_key AND active.state IN ('running','waiting','paused')) ORDER BY candidate.rowid LIMIT 1`,
				)
				.get(...excludedProjectIds, ...excludedRunIds);

			if (!row) return null;
			const run = JSON.parse(row.data) as StoredRun;

			return this.update({
				...run,
				state: "running",
				activity: "Starting execution",
				startedAt: run.startedAt ?? new Date().toISOString(),
			});
		});
	}
	recover(): StoredRun[] {
		return this.list()
			.filter(lostInvocation)
			.map((run) =>
				this.update({
					...run,
					state: "paused",
					pauseRequested: false,
					questions: run.questions.map((question) =>
						question.state === "pending"
							? { ...question, state: "cancelled" }
							: question,
					),
					stages: run.stages?.map((stage) => ({
						...stage,
						steps: stage.steps.map(({ control: _control, ...step }) => step),
					})),
					steps: run.steps?.map(({ control: _control, ...step }) => step),
					execution: run.execution
						? {
								...run.execution,
								tokens: run.execution.tokens.map((token) =>
									token.status === "running"
										? { ...token, status: "paused" }
										: token,
								),
							}
						: undefined,
					errors: [
						...run.errors,
						{
							code: "interrupted",
							message:
								"Service stopped during execution. Effects may already have happened; inspect the run and explicitly retry or cancel.",
						},
					],
					activity: "Interrupted execution requires operator review",
				}),
			);
	}
	putBlob(runId: string, key: string, data: string): void {
		if (!this.get(runId)) missing("Run");

		this.store.db
			.query(
				"INSERT INTO run_blobs(run_id,key,data) VALUES(?,?,?) ON CONFLICT(run_id,key) DO UPDATE SET data=excluded.data",
			)
			.run(runId, key, data);
	}
	getBlob(runId: string, key: string): string | null {
		return (
			this.store.db
				.query<{ data: string }, [string, string]>(
					"SELECT data FROM run_blobs WHERE run_id=? AND key=?",
				)
				.get(runId, key)?.data ?? null
		);
	}
	listBlobs(runId: string): string[] {
		return this.store.db
			.query<{ key: string }, [string]>(
				"SELECT key FROM run_blobs WHERE run_id=? ORDER BY key",
			)
			.all(runId)
			.map((row) => row.key);
	}
	deleteBlob(runId: string, key: string): void {
		this.store.db.query("DELETE FROM run_blobs WHERE run_id=? AND key=?").run(runId, key);
	}
	private projectTask(run: StoredRun): void {
		const task = this.store.get("tasks", run.taskId);
		if (task.latestRunId && task.latestRunId !== run.id && run.revision !== 1) return;
		let stageId = run.currentStageId;

		if (
			stageId &&
			stageId !== task.stageId &&
			!this.store
				.config(task.projectId)
				.workflows.find((workflow) => workflow.id === run.workflowId)
				?.stages.some((stage) => stage.id === stageId)
		)
			stageId = null;

		const next: Task = {
			...task,
			latestRunId: run.id,
			executionState: run.state,
			status: taskStatus(run),
			workflowId: run.workflowId,
			stageId,
			updatedAt: run.updatedAt,
		};

		if (
			task.latestRunId === next.latestRunId &&
			task.executionState === next.executionState &&
			task.status === next.status &&
			task.stageId === next.stageId
		)
			return;

		this.store.put("tasks", { ...next, revision: task.revision + 1 });
	}
	private projectQuestions(run: StoredRun): void {
		for (const question of run.questions) {
			const value: Intervention = {
				id: question.id,
				revision: run.revision,
				createdAt: question.createdAt,
				updatedAt: run.updatedAt,
				projectId: run.projectId,
				taskId: run.taskId,
				runId: run.id,
				questionId: question.id,
				kind: question.kind === "approval" ? "approve" : "question",
				message: question.prompt,
				status:
					question.state === "pending"
						? "recorded"
						: question.state === "answered"
							? "answered"
							: "dismissed",
				response: question.answer,
				executionStatus: question.state === "pending" ? "pending" : "resolved",
			};

			this.store.put("interventions", value);
		}
	}
	private notify(run: StoredRun): void {
		this.changed(`/v1/runs/${run.id}`);
		this.changed(`/v1/tasks/${run.taskId}`);
		this.changed("/v1/activity");
		this.changed("/v1/stats");
		if (run.questions.length) this.changed("/v1/interventions");
	}
}
