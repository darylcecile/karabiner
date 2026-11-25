import type {
	CompiledStage,
	CompiledWorkflow,
	RunScope,
	RunStage,
	RunToken,
	StoredRun,
	StageOutcome,
} from "@karabiner/contracts";
import type { Repository } from "../repository.ts";
import type { RunStore } from "../run-store.ts";
import type { AdapterRequest, AgentOutcome } from "./adapter-types.ts";
import { captureBaseline, createRunDocuments } from "./artifacts.ts";
import { asError } from "./process.ts";
import { finishedToken, invocation, mutateRun, requiredRun, timestamp } from "./run-state.ts";
import { RunInteractions } from "./interactions.ts";
import { planSteps, plannedTitle, StageRunner, type StageResult } from "./stage-runner.ts";

export interface PipelineOptions {
	runs: RunStore;
	repository: Repository;
	changed: (path: string) => void;
	stopping: () => boolean;
	executeAgent?: (request: AdapterRequest) => Promise<AgentOutcome>;
	interactions?: RunInteractions;
}
function token(
	stageId: string,
	scopeId: string | null = null,
	branchId: string | null = null,
	history: string[] = [],
): RunToken {
	return {
		id: crypto.randomUUID(),
		scopeId,
		branchId,
		stageId,
		stageInvocationId: null,
		history: [...history],
		status: "ready",
	};
}

class Pipeline {
	private readonly active = new Map<string, Promise<void>>();
	private readonly failure = new AbortController();
	private readonly interactions: RunInteractions;
	private readonly signal: AbortSignal;
	private wakeup = Promise.withResolvers<void>();
	private branchError: Error | undefined;
	constructor(
		private readonly id: string,
		private readonly options: PipelineOptions,
		private readonly externalSignal: AbortSignal,
	) {
		this.interactions = options.interactions ?? new RunInteractions(options.runs);
		this.signal = AbortSignal.any([externalSignal, this.failure.signal]);
	}
	private get run(): StoredRun {
		return requiredRun(this.options.runs, this.id);
	}
	private get workflow(): CompiledWorkflow {
		return this.run.snapshot.factory.workflows.find(
			(workflow) => workflow.id === this.run.workflowId,
		)!;
	}
	private mutate(edit: (run: StoredRun) => void, aggregate = true) {
		return mutateRun(this.options.runs, this.id, edit, aggregate);
	}
	private wake = () => {
		this.wakeup.resolve();
	};
	private initialize(): void {
		if (this.run.execution) return;

		this.mutate((run) => {
			const workflow = run.snapshot.factory.workflows.find(
				(item) => item.id === run.workflowId,
			)!;

			const root = token(
				run.currentStageId ?? run.snapshot.task.stageId ?? workflow.entryStageId,
			);

			const previous = run.stages?.at(-1);

			root.history = (run.stages ?? [])
				.filter((stage) => stage.status === "completed" || stage.status === "failed")
				.map((stage) => stage.id);

			if (previous && !["completed", "failed", "cancelled"].includes(previous.status)) {
				root.stageInvocationId = previous.id;
				previous.tokenId = root.id;

				for (const question of run.questions)
					if (question.state === "pending" && question.stageId === root.stageId) {
						question.tokenId = root.id;
						question.stageInvocationId = previous.id;
					}
			}

			root.status = run.questions.some((question) => question.state === "pending")
				? "waiting"
				: "ready";

			run.execution = { version: 1, tokens: [root], scopes: [] };
		});
	}
	private resolveJoins(run: StoredRun): void {
		for (const scope of run.execution!.scopes) {
			if (scope.state !== "running") continue;

			const children = scope.branchTokenIds.map((id) =>
				run.execution!.tokens.find((token) => token.id === id)!,
			);

			if (!children.every((token) => token.status === "completed")) continue;
			const parent = run.execution!.tokens.find((token) => token.id === scope.parentTokenId)!;

			parent.history = [
				...new Set([...parent.history, ...children.flatMap((token) => token.history)]),
			];

			parent.stageId = scope.joinStageId;
			parent.stageInvocationId = null;
			parent.status = "ready";
			scope.state = "joined";
		}
	}
	private transition(run: StoredRun, branch: RunToken, nextStage: string): void {
		run.transitionCount++;

		if (run.transitionCount > this.workflow.maxTransitions)
			throw new Error("Workflow transition limit exceeded");

		branch.stageId = nextStage;
		branch.stageInvocationId = null;
		branch.status = "ready";
		const scope = run.execution!.scopes.find((scope) => scope.id === branch.scopeId);
		if (scope && nextStage === scope.joinStageId) branch.status = "completed";
		this.resolveJoins(run);
	}
	private fork(run: StoredRun, branch: RunToken): boolean {
		const fork = this.workflow.forks?.find((fork) => fork.stageId === branch.stageId);
		if (!fork) return false;

		const scope: RunScope = {
			id: crypto.randomUUID(),
			forkId: fork.id,
			parentTokenId: branch.id,
			joinStageId: fork.joinStageId,
			branchTokenIds: [],
			state: "running",
			joinInvocationId: null,
		};

		const children = fork.branches.map((entry) =>
			token(entry.entryStageId, scope.id, entry.id, branch.history),
		);

		run.transitionCount += children.length;

		if (run.transitionCount > this.workflow.maxTransitions)
			throw new Error("Workflow transition limit exceeded");

		scope.branchTokenIds = children.map((token) => token.id);
		branch.status = "joining";
		branch.stageInvocationId = null;
		run.execution!.scopes.push(scope);
		run.execution!.tokens.push(...children);
		return true;
	}
	private startStage(tokenId: string): { stage: CompiledStage; invocationId: string } {
		let result: { stage: CompiledStage; invocationId: string };

		this.mutate((run) => {
			const branch = run.execution!.tokens.find((token) => token.id === tokenId)!;
			const stage = this.workflow.stages.find((stage) => stage.id === branch.stageId);
			if (!stage) throw new Error(`Unknown stage ${branch.stageId}`);
			branch.status = "running";

			if (!branch.stageInvocationId) {
				const visit =
					(run.stages ?? []).filter(
						(item) =>
							item.stageId === stage.id && (item.tokenId ?? tokenId) === tokenId,
					).length + 1;

				if (visit > stage.maxVisits)
					throw new Error(
						`Stage ${stage.name} exceeded ${stage.maxVisits} visits in its branch`,
					);

				const current: RunStage = {
					id: crypto.randomUUID(),
					stageId: stage.id,
					tokenId,
					scopeId: branch.scopeId,
					branchId: branch.branchId,
					title: stage.name,
					visit,
					status: "running",
					startedAt: timestamp(),
					finishedAt: null,
					steps: planSteps(stage).map((step) => ({
						id: crypto.randomUUID(),
						title: plannedTitle(step),
						agentId: step.kind === "agent" ? step.agent.id : null,
						kind: step.kind,
						status: "pending",
						startedAt: null,
						finishedAt: null,
						attempts: [],
					})),
				};

				branch.stageInvocationId = current.id;
				(run.stages ??= []).push(current);

				const joined = run.execution!.scopes.findLast(
					(scope) =>
						scope.parentTokenId === tokenId &&
						scope.joinStageId === stage.id &&
						scope.state === "joined" &&
						scope.joinInvocationId === null,
				);

				if (joined) joined.joinInvocationId = current.id;
			}

			result = { stage, invocationId: branch.stageInvocationId };
		});

		return result!;
	}
	private stageResult(tokenId: string, invocationId: string, result: StageResult): void {
		this.mutate((run) => {
			const branch = run.execution!.tokens.find((token) => token.id === tokenId)!;

			if ("suspended" in result) {
				branch.status = result.suspended === "retry" ? "ready" : result.suspended;
				return;
			}

			const stage = invocation(run, invocationId);
			stage.status = result.outcome === "failure" ? "failed" : "completed";
			stage.outcome = result.outcome;
			stage.finishedAt = timestamp();
			for (const step of stage.steps) if (step.status === "pending") step.status = "skipped";
			branch.history.push(stage.id);
		});

		if (!("suspended" in result)) this.routeStage(tokenId, result);
	}
	private routeStage(
		tokenId: string,
		result: { outcome: StageOutcome; returnTo?: string },
	): void {
		let failure: Error | undefined;

		this.mutate((run) => {
			const branch = run.execution!.tokens.find((token) => token.id === tokenId)!;
			const stage = invocation(run, branch.stageInvocationId!);
			if (result.outcome === "success" && this.fork(run, branch)) return;

			const edge = this.workflow.edges.find(
				(edge) =>
					edge.sourceStageId === branch.stageId &&
					edge.on === result.outcome &&
					(!result.returnTo || edge.targetStageId === result.returnTo),
			);

			if (edge) {
				this.transition(run, branch, edge.targetStageId);
				return;
			}

			if (result.outcome !== "success") {
				branch.status = "failed";

				failure = new Error(
					result.returnTo
						? "Return target has no configured edge"
						: `Stage ${stage.title} failed without a recovery edge`,
				);

				return;
			}

			if (branch.scopeId) {
				branch.status = "failed";

				failure = new Error(
					`Branch ended before its paired join at stage ${branch.stageId}`,
				);

				return;
			}

			branch.status = "completed";
		});

		if (failure) throw failure;
	}
	private dispatch(tokenId: string): void {
		const work = (async () => {
			const { stage, invocationId } = this.startStage(tokenId);

			const runner = new StageRunner(
				{ runId: this.id, tokenId, stageInvocationId: invocationId },
				stage,
				{
					runs: this.options.runs,
					interactions: this.interactions,
					executeAgent: this.options.executeAgent,
				},
				this.signal,
			);

			this.stageResult(tokenId, invocationId, await runner.execute());
		})()
			.catch((error) => {
				if (!this.signal.aborted) {
					this.mutate((run) => {
						run.execution!.tokens.find((token) => token.id === tokenId)!.status =
							"failed";
					});

					this.branchError = asError(error);
					this.failure.abort(this.branchError);
				}
			})
			.finally(() => {
				this.active.delete(tokenId);
				this.wake();
			});

		this.active.set(tokenId, work);
	}
	private retryTime(branch: RunToken): number {
		if (!branch.stageInvocationId) return 0;

		const step = invocation(this.run, branch.stageInvocationId).steps.find(
			(step) => !["completed", "skipped"].includes(step.status),
		);

		return step?.retryAt ? Date.parse(step.retryAt) : 0;
	}
	private async wait(nextRetry: number | undefined): Promise<void> {
		let timer: ReturnType<typeof setTimeout> | undefined;

		if (nextRetry !== undefined)
			timer = setTimeout(this.wake, Math.max(0, nextRetry - Date.now()));

		try {
			await this.wakeup.promise;
		} finally {
			if (timer) clearTimeout(timer);
			this.wakeup = Promise.withResolvers<void>();
		}
	}
	private async pump(): Promise<void> {
		while (!this.signal.aborted) {
			let run = this.run;

			if (run.pauseRequested) {
				if (
					run.execution!.tokens.some((token) => token.status === "ready") ||
					(run.state !== "paused" &&
						!run.execution!.tokens.some((token) => token.status === "running"))
				)
					this.mutate((run) => {
						for (const token of run.execution!.tokens)
							if (token.status === "ready") token.status = "paused";
					});
			} else
				for (const branch of run.execution!.tokens)
					if (
						branch.status === "ready" &&
						!this.active.has(branch.id) &&
						this.retryTime(branch) <= Date.now()
					)
						this.dispatch(branch.id);

			run = this.run;
			const ready = run.execution!.tokens.filter((token) => token.status === "ready");

			if (!this.active.size && !ready.length) {
				if (run.execution!.tokens.every(finishedToken)) await this.finish("completed");
				return;
			}

			const retryTimes = ready
				.filter((token) => !this.active.has(token.id))
				.map((token) => this.retryTime(token))
				.filter((time) => time > Date.now());

			await this.wait(retryTimes.length ? Math.min(...retryTimes) : undefined);
		}

		throw this.branchError ?? this.signal.reason ?? new Error("Run cancelled");
	}
	private async finish(state: "completed" | "failed" | "cancelled"): Promise<void> {
		await this.interactions.settle(this.id);
		const finishedAt = timestamp();

		const current = this.mutate((run) => {
			run.activity = "Finalizing run artifacts";
		}, false);

		const run = {
			...current,
			state,
			finishedAt,
			durationMs: Date.parse(finishedAt) - Date.parse(current.startedAt!),
		};

		try {
			const baseline = this.options.runs.getBlob(this.id, "runtime/baseline");

			await createRunDocuments(
				this.options.repository,
				run,
				baseline ? JSON.parse(baseline) : null,
				(artifact) =>
					this.mutate((run) => {
						run.artifacts.push(artifact);
					}, false),
			);

			this.options.changed(`/v1/documents?projectId=${run.projectId}`);
		} catch (error) {
			this.mutate((run) => {
				run.errors.push({ code: "artifact_failed", message: asError(error).message });
			}, false);
		}

		this.mutate((run) => {
			run.state = state;
			run.finishedAt = finishedAt;
			run.durationMs = Date.parse(finishedAt) - Date.parse(run.startedAt!);
			run.pauseRequested = false;
			run.activeStageIds = [];

			for (const question of run.questions)
				if (question.state === "pending") question.state = "cancelled";
		}, false);
	}
	private async interrupted(error: unknown): Promise<void> {
		await this.interactions.abort(this.id, error);
		await Promise.allSettled(this.active.values());
		const interrupted = this.options.stopping();
		const cancelled = this.externalSignal.aborted && !interrupted;

		this.mutate((run) => {
			for (const branch of run.execution?.tokens ?? [])
				if (!finishedToken(branch)) branch.status = interrupted ? "paused" : "cancelled";

			for (const scope of run.execution?.scopes ?? [])
				if (scope.state === "running") scope.state = cancelled ? "cancelled" : "failed";

			for (const stage of run.stages ?? [])
				if (stage.status === "running") {
					stage.status = cancelled || interrupted ? "cancelled" : "failed";
					stage.finishedAt = timestamp();

					for (const step of stage.steps)
						if (["running", "needs_approval", "pending"].includes(step.status)) {
							step.status = cancelled || interrupted ? "cancelled" : "failed";
							step.finishedAt = timestamp();
							const attempt = step.attempts?.at(-1);

							if (attempt && ["running", "waiting"].includes(attempt.status)) {
								attempt.status = "cancelled";
								attempt.finishedAt = timestamp();
							}
						}
				}

			for (const question of run.questions)
				if (question.state === "pending") question.state = "cancelled";

			run.errors.push({
				code: interrupted ? "interrupted" : cancelled ? "cancelled" : "execution_failed",
				message: asError(error).message,
			});

			if (interrupted) {
				run.state = "paused";
				run.pauseRequested = false;
				run.activity = "Interrupted invocation; inspect effects before retrying";
			}
		}, false);

		if (!interrupted) await this.finish(cancelled ? "cancelled" : "failed");
	}
	async execute(): Promise<void> {
		const detach = this.interactions.attach(this.id, this.wake);
		this.signal.addEventListener("abort", this.wake);

		try {
			if (this.options.runs.getBlob(this.id, "runtime/baseline") === null)
				this.options.runs.putBlob(
					this.id,
					"runtime/baseline",
					JSON.stringify(await captureBaseline(this.run.snapshot.project.rootPath!)),
				);

			this.initialize();
			await this.pump();
		} catch (error) {
			await this.interrupted(error);
		} finally {
			detach();
			this.signal.removeEventListener("abort", this.wake);
		}
	}
}

export async function runPipeline(
	id: string,
	options: PipelineOptions,
	signal: AbortSignal,
): Promise<void> {
	await new Pipeline(id, options, signal).execute();
}
