import { realpath, stat } from "node:fs/promises";
import {
	compileFactory,
	type AnswerRunQuestion,
	type CreateRun,
	type RunControl,
	type RunFilter,
	type StoredRun,
	type AddRunInstruction,
} from "@karabiner/contracts";
import { checkRevision } from "../database.ts";
import { conflict, invalid, missing } from "../errors.ts";
import type { Repository } from "../repository.ts";
import type { RunStore } from "../run-store.ts";
import type { AdapterRequest, AgentOutcome } from "./adapter-types.ts";
import { runPipeline } from "./pipeline.ts";
import { asError } from "./process.ts";
import { resolveConfigSources } from "../factory-service.ts";
import { RunInteractions } from "./interactions.ts";
import { finishedToken, mutateRun } from "./run-state.ts";
import { normalizeInteractionResponse } from "./interaction-response.ts";

export interface RuntimeOptions {
	repository: Repository;
	runs: RunStore;
	changes: { changed(path: string): void };
	executeAgent?: (request: AdapterRequest) => Promise<AgentOutcome>;
}

export class FactoryRuntime {
	private readonly active = new Map<
		string,
		{ controller: AbortController; work: Promise<void> }
	>();
	private readonly scheduleErrors = new Map<string, string>();
	private timer?: ReturnType<typeof setInterval>;
	private tickWork?: Promise<void>;
	private readonly lastScan = new Map<string, number>();
	private stopped = false;
	private readonly interactions: RunInteractions;
	constructor(private readonly options: RuntimeOptions) {
		this.interactions = new RunInteractions(options.runs);
	}

	async list(filter: RunFilter = {}): Promise<StoredRun[]> {
		return this.options.runs.list(filter);
	}
	async get(id: string): Promise<StoredRun | null> {
		return this.options.runs.get(id);
	}
	private required(id: string): StoredRun {
		const run = this.options.runs.get(id);
		if (!run) missing("Run");
		return run;
	}
	async start(taskId: string, input: CreateRun = {}): Promise<StoredRun> {
		return this.queue(taskId, input, "manual");
	}
	async instruct(id: string, input: AddRunInstruction): Promise<StoredRun> {
		const run = this.required(id);
		checkRevision(run, input.revision);

		if (["completed", "failed", "cancelled"].includes(run.state))
			conflict("Instructions require a nonterminal run");

		if (
			typeof input.message !== "string" ||
			!input.message.trim() ||
			input.message.length > 65_536
		)
			invalid("Instruction message must contain between 1 and 65536 characters");

		const updated = this.options.runs.update({
			...run,
			instructions: [
				...(run.instructions ?? []),
				{
					id: crypto.randomUUID(),
					message: input.message,
					createdAt: new Date().toISOString(),
					deliveredAt: null,
					stepId: null,
				},
			],
		});

		void this.interactions.steer(id, updated.instructions!.at(-1)!).catch((error) => {
			console.error("Run guidance delivery failed:", asError(error).message);
		});

		return updated;
	}
	private async queue(taskId: string, input: CreateRun, trigger: "manual" | "scheduler") {
		if (this.stopped) conflict("Runtime is shutting down");
		const { store } = this.options.repository;
		const task = store.get("tasks", taskId);
		checkRevision(task, input.revision);
		const project = store.get("projects", task.projectId);
		if (!project.rootPath) invalid("Select a local project folder before starting a factory");
		let rootPath: string;

		try {
			rootPath = await realpath(project.rootPath);

			if (!(await stat(rootPath)).isDirectory())
				invalid("Project rootPath must be a directory");
		} catch (error) {
			invalid(
				`Project folder is unavailable (${project.rootPath}): ${asError(error).message}`,
			);
		}

		const config = await resolveConfigSources(
			{ ...project, rootPath },
			store.config(project.id),
		);

		const factory = compileFactory(config);
		if (this.stopped) conflict("Runtime is shutting down");

		const workflowId =
			input.workflowId ??
			task.workflowId ??
			config.scheduler?.workflowId ??
			(factory.workflows.length === 1 ? factory.workflows[0]!.id : undefined);

		if (!workflowId) invalid("Select a workflow before starting this task");
		const workflow = factory.workflows.find((item) => item.id === workflowId);
		if (!workflow) invalid("Selected workflow does not exist in the captured factory");

		if (task.stageId && !workflow.stages.some((stage) => stage.id === task.stageId))
			invalid("Selected stage does not belong to this workflow");

		if (
			task.stageId &&
			workflow.stageScopes?.[task.stageId] &&
			workflow.stageScopes[task.stageId] !== "root"
		)
			invalid("Start at the fork stage rather than inside one of its branches");

		const run = this.options.runs.create(
			{ task, project: { ...project, rootPath }, config, factory },
			{ workflowId, trigger },
		);

		this.kick();
		return run;
	}
	async control(
		id: string,
		kind: "pause" | "resume" | "cancel" | "retry",
		input: RunControl = {},
	): Promise<StoredRun> {
		const run = this.required(id);
		checkRevision(run, input.revision);
		if (kind === "retry") return this.retry(run);

		if (["completed", "failed", "cancelled"].includes(run.state))
			conflict(`A ${run.state} run cannot ${kind}`);

		if (kind === "cancel") {
			const active = this.active.get(id);

			if (active) {
				active.controller.abort(new Error("Run cancelled by user"));
				await this.interactions.abort(id, new Error("Run cancelled by user"));
				await active.work;
			} else this.cancelInactive(run);

			this.kick();
			return this.required(id);
		}

		if (kind === "pause") {
			const updated = this.options.runs.update({
				...run,
				...(this.active.has(id)
					? {
							pauseRequested: true,
							activity: "Pause requested; the active step will finish first",
						}
					: { state: "paused", pauseRequested: false, activity: "Paused" }),
			});

			this.interactions.wake(id);
			return updated;
		}

		if (run.state !== "paused" && run.state !== "waiting")
			conflict("Only a paused or waiting run can resume");

		if (run.errors.some((error) => error.code === "interrupted"))
			conflict(
				"This run was interrupted during execution; inspect its effects and retry or cancel explicitly",
			);

		mutateRun(this.options.runs, id, (current) => {
			current.pauseRequested = false;

			for (const token of current.execution?.tokens ?? [])
				if (token.status === "paused")
					token.status = current.questions.some(
						(question) => question.tokenId === token.id && question.state === "pending",
					)
						? "waiting"
						: "ready";

			current.state = current.questions.some((question) => question.state === "pending")
				? "waiting"
				: "running";
		});

		this.interactions.resume(id);
		if (!this.active.has(id)) this.queueReady(id);
		this.kick();
		return this.required(id);
	}
	private cancelInactive(run: StoredRun): StoredRun {
		const finishedAt = new Date().toISOString();

		for (const token of run.execution?.tokens ?? [])
			if (!finishedToken(token)) token.status = "cancelled";

		for (const scope of run.execution?.scopes ?? [])
			if (scope.state === "running") scope.state = "cancelled";

		for (const stage of run.stages ?? [])
			if (stage.status === "running") {
				stage.status = "cancelled";
				stage.finishedAt = finishedAt;

				for (const step of stage.steps)
					if (["pending", "running", "needs_approval"].includes(step.status)) {
						step.status = "cancelled";
						step.finishedAt = finishedAt;
						delete step.control;
						const attempt = step.attempts?.at(-1);

						if (attempt && ["running", "waiting"].includes(attempt.status)) {
							attempt.status = "cancelled";
							attempt.finishedAt = finishedAt;
						}
					}
			}

		run.steps = run.stages?.flatMap((stage) => stage.steps);

		return this.options.runs.update({
			...run,
			state: "cancelled",
			finishedAt,
			pauseRequested: false,
			durationMs: run.startedAt ? Date.parse(finishedAt) - Date.parse(run.startedAt) : null,
			questions: run.questions.map((question) =>
				question.state === "pending" ? { ...question, state: "cancelled" } : question,
			),
		});
	}
	private async retry(run: StoredRun): Promise<StoredRun> {
		if (this.active.has(run.id) || run.state === "queued")
			conflict("Pause or cancel active execution before retrying");

		if (run.state === "paused" || run.state === "waiting") this.cancelInactive(run);

		let retried = this.options.runs.create(run.snapshot, {
			workflowId: run.workflowId,
			trigger: "retry",
			retryOf: run.id,
		});

		if (run.instructions?.length)
			retried = this.options.runs.update({
				...retried,
				instructions: run.instructions.map((instruction) => ({
					...instruction,
					id: crypto.randomUUID(),
					deliveredAt: null,
					stepId: null,
					deliveries: [],
				})),
			});

		this.kick();
		return retried;
	}
	async answer(id: string, questionId: string, input: AnswerRunQuestion): Promise<StoredRun> {
		const run = this.required(id);
		checkRevision(run, input.revision);

		if (["completed", "failed", "cancelled"].includes(run.state))
			conflict("This run is no longer awaiting input");

		if (run.errors.some((error) => error.code === "interrupted"))
			conflict(
				"The invocation was interrupted; inspect its effects and retry or cancel the run",
			);

		const question = run.questions.find((question) => question.id === questionId);
		if (!question) missing("Question");
		if (question.state !== "pending") conflict("This question has already been resolved");

		if (question.transport === "live" && !this.interactions.isLive(question.id))
			conflict(
				"The native interaction is no longer connected; retry or cancel the interrupted run",
			);

		const { revision: _revision, message: _message, ...response } = input;

		if (
			response.answer === undefined &&
			response.choice === undefined &&
			response.answers === undefined &&
			!Object.hasOwn(response, "value")
		)
			invalid("An interaction response is required");

		if (question.kind === "permission" && question.choices?.length)
			response.choice ??= response.answer;

		if (
			response.choice !== undefined &&
			question.choices?.length &&
			!question.choices.some((choice) => choice.id === response.choice)
		)
			invalid("Choose one of the request's supplied choice IDs");

		if (question.kind === "permission" && question.choices?.length && !response.choice)
			invalid("This permission request requires a supplied choice ID");

		const normalized = normalizeInteractionResponse(question, response);

		question.state = "answered";

		question.answer =
			normalized.answer ??
			normalized.choice ??
			JSON.stringify(normalized.value ?? normalized.answers);

		question.response = normalized;
		question.answeredAt = new Date().toISOString();
		const effect = question.choices?.find((choice) => choice.id === response.choice)?.effect;

		if ((effect === "deny" || effect === "cancel") && question.stepId) {
			const step = run.stages
				?.flatMap((stage) => stage.steps)
				.find((step) => step.id === question.stepId);

			const attempt = step?.attempts?.find((attempt) => attempt.id === question.attemptId);
			if (attempt) attempt.denied = true;
		}

		this.options.runs.update(run);
		if (this.interactions.answer(id, questionId)) return this.required(id);

		mutateRun(this.options.runs, id, (current) => {
			const token = current.execution?.tokens.find((token) => token.id === question.tokenId);

			if (token)
				token.status =
					current.pauseRequested || current.state === "paused" ? "paused" : "ready";
		});

		if (!this.interactions.wake(id)) this.queueReady(id);
		this.kick();
		return this.required(id);
	}
	private queueReady(id: string): void {
		const run = this.required(id);
		if (run.pauseRequested || run.state === "paused") return;

		const ready = run.execution
			? run.execution.tokens.some((token) => token.status === "ready")
			: !run.questions.some((question) => question.state === "pending");

		if (ready) this.options.runs.update({ ...run, state: "queued" });
	}
	startScheduler(): void {
		if (this.timer || this.stopped) return;
		this.options.runs.recover();
		this.timer = setInterval(() => this.kick(), 250);
		this.timer.unref();
		this.kick();
	}
	private kick(): void {
		if (this.tickWork || this.stopped) return;

		this.tickWork = this.tick()
			.catch((error) => {
				this.options.repository.store.activity(
					"scheduler.error",
					asError(error).message,
					null,
					null,
					null,
				);

				this.options.changes.changed("/v1/activity");
			})
			.finally(() => {
				this.tickWork = undefined;
			});
	}
	private async tick() {
		const { store } = this.options.repository;
		const limits = new Map<string, number>();

		for (const project of store.list("projects")) {
			const config = store.config(project.id);
			limits.set(project.id, config.scheduler?.maxConcurrentRuns ?? 1);
			if (!config.scheduler?.enabled) continue;

			if (
				Date.now() - (this.lastScan.get(project.id) ?? 0) <
				(config.scheduler.pollIntervalMs ?? 1_000)
			)
				continue;

			this.lastScan.set(project.id, Date.now());

			for (const task of store.list("tasks", { projectId: project.id })) {
				if (
					!(config.scheduler.statuses ?? ["todo"]).includes(task.status) ||
					this.options.runs.hasScheduled(task.id)
				)
					continue;

				try {
					await this.queue(
						task.id,
						{ workflowId: config.scheduler.workflowId },
						"scheduler",
					);

					this.scheduleErrors.delete(task.id);
				} catch (error) {
					const message = asError(error).message;
					if (this.scheduleErrors.get(task.id) === message) continue;
					this.scheduleErrors.set(task.id, message);
					store.activity("scheduler.blocked", message, project.id, task.id, task.id);
					this.options.changes.changed("/v1/activity");
				}
			}
		}

		while (!this.stopped && this.active.size < 32) {
			const counts = new Map<string, number>();

			for (const id of this.active.keys()) {
				const project = this.required(id).projectId;
				counts.set(project, (counts.get(project) ?? 0) + 1);
			}

			const excluded = [...counts]
				.filter(([project, count]) => count >= (limits.get(project) ?? 1))
				.map(([project]) => project);

			const run = this.options.runs.claimNext(excluded, [...this.active.keys()]);
			if (!run) break;
			const controller = new AbortController();

			const work = runPipeline(
				run.id,
				{
					...this.options,
					changed: (path) => this.options.changes.changed(path),
					stopping: () => this.stopped,
					interactions: this.interactions,
				},
				controller.signal,
			)
				.catch((error) => {
					const current = this.required(run.id);

					if (current.state === "running")
						this.options.runs.update({
							...current,
							state: "failed",
							errors: [
								...current.errors,
								{ code: "runtime_failed", message: asError(error).message },
							],
						});
				})
				.finally(() => {
					this.active.delete(run.id);
					this.kick();
				});

			this.active.set(run.id, { controller, work });
		}
	}
	async shutdown(): Promise<void> {
		this.stopped = true;
		if (this.timer) clearInterval(this.timer);
		await this.tickWork;

		for (const active of this.active.values())
			active.controller.abort(new Error("Service shutdown interrupted the active step"));

		await Promise.allSettled(
			[...this.active.keys()].map((id) =>
				this.interactions.abort(
					id,
					new Error("Service shutdown interrupted the active step"),
				),
			),
		);

		await Promise.allSettled([...this.active.values()].map((active) => active.work));
		await this.interactions.close();
	}
}
