import type { RunStage, RunToken, StoredRun } from "@karabiner/contracts";
import { invocation, invocationStep } from "./run-state.ts";

const TRUNCATED = "[truncated]\n";

/** These budgets apply only to prior execution text, never task or instruction content. */
class TextBudget {
	truncated = false;
	constructor(private remaining: number) {}
	take(value: string | undefined, maximum: number): string | undefined {
		if (value === undefined) return undefined;
		const length = Math.min(maximum, this.remaining);

		if (value.length <= length) {
			this.remaining -= value.length;
			return value;
		}

		this.truncated = true;
		if (length < TRUNCATED.length) return undefined;
		this.remaining -= length;
		const contentLength = length - TRUNCATED.length;
		return TRUNCATED + (contentLength ? value.slice(-contentLength) : "");
	}
}

function completedSteps(stage: RunStage) {
	return stage.steps.filter((step) => step.status === "completed" || step.status === "failed");
}

function branchStages(run: StoredRun, branch: RunToken): RunStage[] {
	const tokens = new Map(run.execution!.tokens.map((token) => [token.id, token]));
	const scopes = new Map(run.execution!.scopes.map((scope) => [scope.id, scope]));

	return branch.history
		.map((id) => invocation(run, id))
		.filter((stage) => {
			let token = stage.tokenId ? tokens.get(stage.tokenId) : undefined;

			while (token) {
				if (token.id === branch.id) return true;
				const parent = token.scopeId ? scopes.get(token.scopeId)?.parentTokenId : undefined;
				token = parent ? tokens.get(parent) : undefined;
			}

			return false;
		});
}

function joinedResults(run: StoredRun, invocationId: string, budget: TextBudget) {
	const scope = run.execution!.scopes.find((scope) => scope.joinInvocationId === invocationId);
	if (!scope) return [];
	const maximum = Math.min(1_000, Math.floor(16_000 / scope.branchTokenIds.length));

	return scope.branchTokenIds.map((id) => {
		const branch = run.execution!.tokens.find((token) => token.id === id)!;
		const stages = branchStages(run, branch);
		const last = stages.at(-1);

		const output = stages
			.flatMap(completedSteps)
			.findLast((step) => step.output || step.error?.message);

		return {
			branchId: branch.branchId,
			tokenId: branch.id,
			scopeId: scope.id,
			stageId: last?.stageId,
			stageInvocationId: last?.id,
			outcome: last?.outcome ?? last?.status ?? branch.status,
			summary: budget.take(
				output?.error?.message ??
					output?.output ??
					`Branch ${branch.branchId} ${branch.status}`,
				maximum,
			),
		};
	});
}

/** Join summaries cannot be displaced by a longer sibling's recent history. */
export function historicalContext(
	run: StoredRun,
	tokenId: string,
	invocationId: string,
	stepId: string,
) {
	const branch = run.execution!.tokens.find((token) => token.id === tokenId)!;

	const stages = [
		...branch.history.map((id) => invocation(run, id)),
		invocation(run, invocationId),
	].filter((stage) => completedSteps(stage).length);

	const recent = stages.slice(-8);
	const history = new TextBudget(36_000);
	const joins = new TextBudget(16_000);
	const retries = new TextBudget(8_000);
	const handoff = new TextBudget(4_000);
	const branchResults = joinedResults(run, invocationId, joins);
	let omittedSteps = 0;

	const priorOutputs = [...recent]
		.reverse()
		.map((stage) => {
			const allSteps = completedSteps(stage);
			omittedSteps += Math.max(0, allSteps.length - 8);

			const steps = allSteps
				.slice(-8)
				.reverse()
				.map((step) => ({
					agentId: step.agentId,
					title: history.take(step.title, 256),
					output: history.take(step.output, 4_000),
					error: step.error
						? {
								code: history.take(step.error.code, 128),
								message: history.take(step.error.message, 2_000),
							}
						: undefined,
				}))
				.reverse();

			return {
				stageId: stage.stageId,
				stageInvocationId: stage.id,
				tokenId: stage.tokenId,
				scopeId: stage.scopeId,
				branchId: stage.branchId,
				outcome: stage.outcome,
				steps,
			};
		})
		.reverse();

	const attempts =
		invocationStep(run, invocationId, stepId).attempts?.filter(
			(attempt) => attempt.status === "failed",
		) ?? [];

	const previousAttempts = attempts.slice(-5).map((attempt) => ({
		number: attempt.number,
		error: attempt.error
			? {
					code: retries.take(attempt.error.code, 128),
					message: retries.take(attempt.error.message, 400),
				}
			: undefined,
		output: retries.take(attempt.output, 1_000),
	}));

	return {
		branchResults,
		priorOutputs,
		previousAttempts,
		handoff:
			handoff.take(
				stages.flatMap(completedSteps).findLast((step) => step.output)?.output,
				4_000,
			) ?? "",
		historyTruncated:
			stages.length > recent.length ||
			omittedSteps > 0 ||
			attempts.length > 5 ||
			[history, joins, retries, handoff].some((budget) => budget.truncated),
		omittedInvocations: Math.max(0, stages.length - recent.length),
		omittedSteps,
		historyTextBudget: 64_000,
	};
}
