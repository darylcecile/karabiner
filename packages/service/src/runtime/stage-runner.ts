import { resolve } from "node:path";
import type {
	CommandDefinition,
	CompiledStage,
	GateDefinition,
	ResolvedAgent,
	RunStep,
	RunStepAttempt,
	StageOutcome,
	StoredRun,
	RunError,
} from "@karabiner/contracts";
import { executeCli } from "./cli-adapter.ts";
import { executeStrands } from "./strands-adapter.ts";
import {
	parseOutcome,
	AdapterCleanupError,
	type AdapterEvent,
	type AdapterRequest,
	type AgentOutcome,
	type Launch,
} from "./adapter-types.ts";
import { prepareLaunch } from "./targets.ts";
import { asError, runPrepared } from "./process.ts";
import {
	boundedOutput,
	invocation,
	invocationStep,
	mutateRun,
	recordDelivery,
	requiredRun,
	timestamp,
} from "./run-state.ts";
import type { InvocationAddress, RunInteractions } from "./interactions.ts";
import type { RunStore } from "../run-store.ts";
import { historicalContext } from "./prompt-context.ts";
import { ActivityRecorder } from "./activity.ts";

export type PlannedStep =
	| { kind: "agent"; agent: ResolvedAgent }
	| { kind: "command"; command: CommandDefinition }
	| { kind: "gate"; gate: GateDefinition };
export type StageResult =
	| { outcome: StageOutcome; returnTo?: string }
	| { suspended: "waiting" | "paused" | "retry" };
export interface StageRunnerOptions {
	runs: RunStore;
	interactions: RunInteractions;
	executeAgent?: (request: AdapterRequest) => Promise<AgentOutcome>;
}

class StepDenied extends Error {
	readonly retryable = false;
}

export function planSteps(stage: CompiledStage): PlannedStep[] {
	const command = (command: CommandDefinition): PlannedStep => ({ kind: "command", command });

	return [
		...(stage.execution.before ?? []).map(command),
		...stage.agents.map((agent): PlannedStep => ({ kind: "agent", agent })),
		...(stage.execution.commands ?? []).map(command),
		...(stage.execution.gates ?? []).map((gate): PlannedStep => ({ kind: "gate", gate })),
		...(stage.execution.after ?? []).map(command),
	];
}
export function plannedTitle(step: PlannedStep): string {
	if (step.kind === "agent") return step.agent.name;
	if (step.kind === "command") return step.command.name ?? step.command.executable;

	return (
		step.gate.name ??
		(step.gate.kind === "approval" ? "Approval" : (step.gate.command.name ?? "Verification"))
	);
}
function retryPolicy(stage: CompiledStage, step: PlannedStep): { retries: number; delay: number } {
	const command =
		step.kind === "command"
			? step.command
			: step.kind === "gate" && step.gate.kind === "command"
				? step.gate.command
				: undefined;

	return {
		retries: command?.retries ?? stage.execution.retries ?? 0,
		delay: command?.retryDelayMs ?? stage.execution.retryDelayMs ?? 0,
	};
}

function canRetry(error: unknown): boolean {
	const failure = error as { retryable?: boolean; code?: string; name?: string } | null;

	return (
		failure?.retryable !== false &&
		failure?.name !== "AbortError" &&
		!["permission_denied", "approval_denied", "cancelled", "interrupted"].includes(
			failure?.code ?? "",
		)
	);
}

export class StageRunner {
	constructor(
		private readonly address: Omit<InvocationAddress, "stepId">,
		private readonly stage: CompiledStage,
		private readonly options: StageRunnerOptions,
		private readonly signal: AbortSignal,
	) {}
	private get run(): StoredRun {
		return requiredRun(this.options.runs, this.address.runId);
	}
	private mutate(edit: (run: StoredRun) => void) {
		return mutateRun(this.options.runs, this.address.runId, edit);
	}
	private step(stepId: string): RunStep {
		return invocationStep(this.run, this.address.stageInvocationId, stepId);
	}
	private editStep(stepId: string, edit: (step: RunStep, run: StoredRun) => void): void {
		this.mutate((run) =>
			edit(invocationStep(run, this.address.stageInvocationId, stepId), run),
		);
	}
	private launch(purpose: "harness" | "command"): Launch {
		return async (executable, args, cwd, env) => {
			this.signal.throwIfAborted();
			const { factory, project } = this.run.snapshot;
			const target = this.stage.execution.executionTarget ?? factory.executionTarget;

			return prepareLaunch({
				target,
				projectRoot: project.rootPath!,
				cwd: cwd ?? this.stage.execution.cwd,
				executable,
				args,
				env,
				targetOptions:
					this.stage.execution.targetOptions ??
					(target === factory.executionTarget ? factory.targetOptions : undefined),
				purpose,
				signal: this.signal,
			});
		};
	}
	private emitter(stepId: string, attemptId: string) {
		const initial = this.step(stepId).attempts!.find((attempt) => attempt.id === attemptId)!;
		const activity = new ActivityRecorder(initial.events);
		let output = initial.output ?? "";
		let closed = false;

		let lastWrite = 0;

		const flush = () =>
			this.editStep(stepId, (step) => {
				step.output = output;
				const attempt = step.attempts!.find((attempt) => attempt.id === attemptId)!;
				attempt.output = output;
				attempt.events = activity.events;
			});

		const emit = (event: AdapterEvent) => {
			if (closed) return;
			activity.add(event);
			const progress = event.outputDelta !== undefined;

			if (!progress && event.kind !== "reasoning")
				output = boundedOutput(
					output + (event.kind === "text" ? event.text : `\n${event.text}\n`),
				);

			if (!progress && event.kind === "tool" && event.toolId)
				this.mutate((run) => {
					const spans = (run.traceSpans ??= []);
					const id = `${attemptId}:${event.toolId}`;
					const offset = Date.now() - Date.parse(run.startedAt!);
					const existing = spans.find((span) => span.id === id);

					if (existing) {
						existing.status = event.status ?? existing.status;

						if (existing.status !== "running")
							existing.durationMs = offset - existing.startOffsetMs;
					} else if (spans.filter((span) => span.parentId === attemptId).length < 512)
						spans.push({
							id,
							parentId: attemptId,
							name: event.name ?? event.text,
							kind: "tool",
							status: event.status ?? "running",
							startOffsetMs: offset,
							durationMs: null,
						});
				});

			if (Date.now() - lastWrite > 150 || (!progress && event.kind === "tool")) {
				flush();
				lastWrite = Date.now();
			}
		};

		return {
			emit,
			close: () => {
				closed = true;
				flush();
			},
			setOutput: (text: string) => {
				output = boundedOutput(text);
			},
		};
	}
	private storage(stepId: string, attemptId: string): NonNullable<AdapterRequest["storage"]> {
		const { runs } = this.options;
		const id = this.address.runId;
		const attempt = this.step(stepId).attempts!.find((attempt) => attempt.id === attemptId)!;
		const prefix = attempt.number > 1 ? `strands/retry-${attemptId}/` : "strands/";

		return {
			write: async (key, bytes) => {
				runs.putBlob(id, prefix + key, Buffer.from(bytes).toString("base64"));
			},
			read: async (key) => {
				const value = runs.getBlob(id, prefix + key);
				return value === null ? null : Buffer.from(value, "base64");
			},
			delete: async (key) => {
				runs.deleteBlob(id, prefix + key);
			},
			list: async (key) =>
				runs
					.listBlobs(id)
					.filter((value) => value.startsWith(prefix + key))
					.map((value) => value.slice(prefix.length)),
		};
	}
	private session(stepId: string): { sessionId?: string; requestId?: string } {
		const value = this.options.runs.getBlob(this.address.runId, `runtime/session/${stepId}`);
		return value ? JSON.parse(value) : {};
	}
	private saveSession(stepId: string, sessionId: string): void {
		this.options.runs.putBlob(
			this.address.runId,
			`runtime/session/${stepId}`,
			JSON.stringify({ ...this.session(stepId), sessionId }),
		);

		this.editStep(stepId, (step) => {
			step.sessionId = sessionId;
			step.attempts!.at(-1)!.sessionId = sessionId;
		});
	}
	private async command(
		stepId: string,
		attemptId: string,
		command: CommandDefinition,
	): Promise<AgentOutcome> {
		const events = this.emitter(stepId, attemptId);

		try {
			const launch = await this.launch("command")(
				command.executable,
				command.args,
				command.cwd,
				command.env,
			);

			const result = await runPrepared({
				...launch,
				signal: this.signal,
				timeoutMs: command.timeoutMs ?? this.stage.execution.timeoutMs ?? 120_000,
				onLine: (text) => events.emit({ kind: "activity", text }),
			});

			if (result.code !== 0)
				throw new Error(
					`${command.name ?? command.executable} exited ${result.code}: ${result.stderr.slice(-2_000)}`,
				);

			return {
				outcome: "complete",
				summary: boundedOutput(
					result.stdout || `${command.name ?? command.executable} passed (exit 0).`,
				),
			};
		} finally {
			events.close();
		}
	}
	private async agent(
		stepId: string,
		attemptId: string,
		agent: ResolvedAgent,
	): Promise<AgentOutcome> {
		const run = this.run;
		const session = this.session(stepId);

		const answered = run.questions.findLast(
			(question) =>
				question.stepId === stepId &&
				question.attemptId === attemptId &&
				question.transport !== "live" &&
				question.state === "answered",
		);

		const instructions = run.instructions ?? [];
		const captured = new Set(instructions.map((instruction) => instruction.id));

		const guidance = instructions.length
			? `## Operator instructions\n\n${instructions.map((instruction) => instruction.message).join("\n\n")}`
			: "";

		const factoryInstructions = [this.stage.resolvedInstructions, agent.resolvedInstructions]
			.filter(Boolean)
			.join("\n\n");

		const events = this.emitter(stepId, attemptId);
		const controls: (() => void)[] = [];
		const lifetime = new AbortController();
		const signal = AbortSignal.any([this.signal, lifetime.signal]);

		const request: AdapterRequest & { attemptId: string } = {
			attemptId,
			agent,
			rootPath: resolve(run.snapshot.project.rootPath!, this.stage.execution.cwd ?? "."),
			runId: run.id,
			stepId,
			sessionId: session.sessionId,
			instructions: [factoryInstructions, guidance].filter(Boolean).join("\n\n"),
			factoryInstructions,
			operatorInstructions: instructions.map((instruction) => instruction.message),
			skills: agent.skills,
			prompt: this.agentPrompt(run, stepId),
			answer: answered
				? {
						requestId: session.requestId ?? answered.requestId ?? answered.id,
						response: answered.response?.value ?? answered.answer,
					}
				: undefined,
			signal,
			timeoutMs: this.stage.execution.timeoutMs ?? 600_000,
			storage: this.storage(stepId, attemptId),
			emit: events.emit,
			launch: this.launch(agent.harness === "strands" ? "command" : "harness"),
			executionTarget:
				this.stage.execution.executionTarget ?? run.snapshot.factory.executionTarget,
			targetOptions:
				this.stage.execution.targetOptions ??
				(this.stage.execution.executionTarget === undefined ||
				this.stage.execution.executionTarget === run.snapshot.factory.executionTarget
					? run.snapshot.factory.targetOptions
					: undefined),
			onSession: (id) => {
				signal.throwIfAborted();
				this.saveSession(stepId, id);
			},
			interact: (input, requestSignal) =>
				this.options.interactions.interact(
					{ ...this.address, stepId, attemptId },
					input,
					requestSignal ? AbortSignal.any([requestSignal, signal]) : signal,
				),
			registerControl: (control) => {
				const dispose = this.options.interactions.register(
					{ ...this.address, stepId, attemptId },
					control,
					captured,
				);

				controls.push(dispose);
				return dispose;
			},
		};

		try {
			const execute =
				this.options.executeAgent ??
				(agent.harness === "strands" ? executeStrands : executeCli);

			let result: AgentOutcome;
			let cleanup: AdapterCleanupError | undefined;

			try {
				result = await execute(request);
			} catch (error) {
				if (!(error instanceof AdapterCleanupError) || !error.completed) throw error;
				cleanup = error;
				result = error.completed;
			}

			this.signal.throwIfAborted();

			if (!parseOutcome(result))
				throw cleanup ?? new Error("Adapter returned an invalid step result");

			this.mutate((current) => {
				for (const instruction of current.instructions ?? [])
					if (captured.has(instruction.id))
						recordDelivery(
							instruction,
							this.address.stageInvocationId,
							stepId,
							"next-step",
							attemptId,
						);
			});

			if (result.sessionId) this.saveSession(stepId, result.sessionId);

			this.options.runs.putBlob(
				run.id,
				`runtime/session/${stepId}`,
				JSON.stringify({
					sessionId: result.sessionId ?? this.session(stepId).sessionId,
					requestId: result.requestId,
				}),
			);

			if (result.artifacts?.length)
				this.editStep(stepId, (step, current) => {
					const branch = current.execution!.tokens.find(
						(token) => token.id === this.address.tokenId,
					)!;

					const artifacts = result.artifacts!.map((artifact) => ({
						id: crypto.randomUUID(),
						kind: "file" as const,
						name: artifact.title,
						path: artifact.path,
						stageInvocationId: this.address.stageInvocationId,
						stepId,
						attemptId,
						tokenId: branch.id,
						scopeId: branch.scopeId,
						branchId: branch.branchId,
					}));

					step.artifacts = [...(step.artifacts ?? []), ...artifacts];

					step.attempts!.find((attempt) => attempt.id === attemptId)!.artifacts =
						artifacts;

					current.artifacts.push(...artifacts);
				});

			if (result.usage)
				this.editStep(stepId, (step, current) => {
					const attempt = step.attempts!.find((attempt) => attempt.id === attemptId)!;
					attempt.tokens = (attempt.tokens ?? 0) + result.usage!.totalTokens;

					step.tokens = step.attempts!.every(
						(attempt) => typeof attempt.tokens === "number",
					)
						? step.attempts!.reduce((sum, attempt) => sum + attempt.tokens!, 0)
						: null;

					const agentSteps = current
						.stages!.flatMap((stage) => stage.steps)
						.filter((step) => step.kind === "agent" && step.startedAt);

					current.tokens = agentSteps.every((step) => typeof step.tokens === "number")
						? agentSteps.reduce((sum, step) => sum + step.tokens!, 0)
						: null;
				});

			if (cleanup) {
				events.setOutput(result.summary);
				throw cleanup;
			}

			return result;
		} finally {
			events.close();
			lifetime.abort(new Error("Agent invocation ended"));
			for (const dispose of controls) dispose();
		}
	}
	private agentPrompt(run: StoredRun, stepId: string): string {
		return JSON.stringify({
			task: run.snapshot.task,
			project: run.snapshot.project,
			stage: { id: this.stage.id, name: this.stage.name },
			permittedReturnStageIds: this.stage.returnToStageIds,
			...historicalContext(run, this.address.tokenId, this.address.stageInvocationId, stepId),
		});
	}
	private checkpoint(
		stepId: string,
		prompt: string,
		kind: "question" | "approval",
		requestId?: string,
	): void {
		this.editStep(stepId, (step, run) => {
			step.status = "needs_approval";
			step.attempts!.at(-1)!.status = "waiting";

			run.execution!.tokens.find((token) => token.id === this.address.tokenId)!.status =
				"waiting";

			run.questions.push({
				id: crypto.randomUUID(),
				stageId: this.stage.id,
				stageInvocationId: this.address.stageInvocationId,
				tokenId: this.address.tokenId,
				attemptId: step.attempts!.at(-1)!.id,
				stepId,
				requestId,
				kind,
				prompt,
				transport: "checkpoint",
				state: "pending",
				createdAt: timestamp(),
				answer: null,
				answeredAt: null,
			});

			run.activity = prompt;
		});
	}
	private async perform(
		stepId: string,
		attemptId: string,
		planned: PlannedStep,
	): Promise<AgentOutcome | undefined> {
		if (planned.kind === "agent") return this.agent(stepId, attemptId, planned.agent);
		if (planned.kind === "command") return this.command(stepId, attemptId, planned.command);

		if (planned.gate.kind === "command")
			return this.command(stepId, attemptId, planned.gate.command);

		const answer = this.run.questions.findLast(
			(question) =>
				question.stepId === stepId &&
				question.attemptId === attemptId &&
				question.state === "answered",
		);

		if (!answer) {
			this.checkpoint(stepId, planned.gate.prompt, "approval");
			return;
		}

		if (
			!["approve", "approved", "yes", "y"].includes(
				(answer.response?.choice ?? answer.answer ?? "").trim().toLowerCase(),
			)
		)
			throw new StepDenied("The stage approval was declined");

		return { outcome: "complete", summary: `Approved: ${answer.answer}` };
	}
	private startAttempt(stepId: string): RunStepAttempt {
		let result: RunStepAttempt;

		this.editStep(stepId, (step) => {
			const previous = step.attempts?.at(-1);

			if (previous?.status === "waiting") result = previous;
			else {
				result = {
					id: crypto.randomUUID(),
					number: (step.attempts?.length ?? 0) + 1,
					status: "running",
					startedAt: timestamp(),
					finishedAt: null,
					tokens: null,
					cost: null,
				};

				step.attempts = [...(step.attempts ?? []), result];

				if (result.number > 1) {
					this.options.runs.deleteBlob(this.address.runId, `runtime/session/${stepId}`);
					delete step.sessionId;
				}
			}

			result.status = "running";
			step.status = "running";
			step.startedAt ??= timestamp();
			step.retryAt = null;
			step.error = null;
		});

		return result!;
	}
	private async executeStep(
		stepId: string,
		planned: PlannedStep,
	): Promise<StageResult | undefined> {
		const attempt = this.startAttempt(stepId);

		try {
			const result = await this.perform(stepId, attempt.id, planned);
			this.signal.throwIfAborted();
			if (!result) return { suspended: "waiting" };

			if (result.outcome === "question") {
				this.checkpoint(
					stepId,
					result.question ?? result.summary,
					"question",
					result.requestId,
				);

				return { suspended: "waiting" };
			}

			if (result.outcome === "blocked") throw new StepDenied(result.summary);

			if (
				result.outcome === "return" &&
				(!result.returnToStageId ||
					!this.stage.returnToStageIds.includes(result.returnToStageId))
			)
				throw new StepDenied("Agent requested an undeclared return stage");

			this.editStep(stepId, (step, run) => {
				step.status = "completed";
				step.finishedAt = timestamp();
				step.output = boundedOutput(result.summary);

				Object.assign(step.attempts!.at(-1)!, {
					status: "completed",
					finishedAt: step.finishedAt,
					output: step.output,
				});

				run.output = step.output;
			});

			if (result.outcome === "return")
				return { outcome: "return", returnTo: result.returnToStageId };

			return;
		} catch (error) {
			this.signal.throwIfAborted();
			const policy = retryPolicy(this.stage, planned);

			const retry =
				canRetry(error) &&
				!this.step(stepId).attempts!.find((item) => item.id === attempt.id)!.denied &&
				attempt.number <= policy.retries;

			const detail: RunError = {
				code: error instanceof AdapterCleanupError ? error.code : "step_failed",
				message: asError(error).message,
				stageId: this.stage.id,
				stageInvocationId: this.address.stageInvocationId,
				stepId,
				attemptId: attempt.id,
				...(error instanceof AdapterCleanupError ? { details: error.details } : {}),
			};

			this.editStep(stepId, (step, run) => {
				const failed = step.attempts!.find((item) => item.id === attempt.id)!;
				const activity = new ActivityRecorder(failed.events);
				activity.add({ kind: "error", text: detail.message, delta: false });

				Object.assign(failed, {
					status: "failed",
					finishedAt: timestamp(),
					error: detail,
					events: activity.events,
				});

				step.status = retry ? "pending" : "failed";
				step.error = detail;
				step.finishedAt = retry ? null : timestamp();
				step.retryAt = retry ? new Date(Date.now() + policy.delay).toISOString() : null;

				if (
					error instanceof AdapterCleanupError &&
					error.completed &&
					parseOutcome(error.completed)
				)
					run.output = boundedOutput(error.completed.summary);

				run.errors.push(detail);
			});

			return retry ? { suspended: "retry" } : { outcome: "failure" };
		}
	}
	async execute(): Promise<StageResult> {
		const planned = planSteps(this.stage);

		for (let index = 0; index < planned.length; index++) {
			this.signal.throwIfAborted();
			const run = this.run;
			if (run.pauseRequested) return { suspended: "paused" };
			const step = invocation(run, this.address.stageInvocationId).steps[index]!;
			if (["completed", "skipped"].includes(step.status)) continue;

			if (
				run.questions.some(
					(question) => question.stepId === step.id && question.state === "pending",
				)
			)
				return { suspended: "waiting" };

			const result = await this.executeStep(step.id, planned[index]!);
			if (result) return result;
		}

		return { outcome: "success" };
	}
}
