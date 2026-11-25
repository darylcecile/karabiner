import type {
	RunInstruction,
	RunStage,
	RunStep,
	RunToken,
	StoredRun,
	TraceSpan,
} from "@karabiner/contracts";
import type { RunStore } from "../run-store.ts";

export const timestamp = () => new Date().toISOString();
export const boundedOutput = (text: string) =>
	text.length > 256_000 ? `${text.slice(0, 256_000)}\n[Output truncated]` : text;
export const finishedToken = (token: RunToken) =>
	["completed", "failed", "cancelled"].includes(token.status);

export function requiredRun(runs: RunStore, id: string): StoredRun {
	const run = runs.get(id);
	if (!run) throw new Error("Run was removed during execution");
	return run;
}
export function invocation(run: StoredRun, id: string): RunStage {
	const stage = run.stages?.find((item) => item.id === id);
	if (!stage) throw new Error(`Stage invocation ${id} is missing`);
	return stage;
}
export function invocationStep(run: StoredRun, invocationId: string, stepId: string): RunStep {
	const step = invocation(run, invocationId).steps.find((item) => item.id === stepId);
	if (!step) throw new Error(`Step ${stepId} is missing`);
	return step;
}
export function aggregateRun(run: StoredRun): void {
	const tokens = run.execution?.tokens;
	if (!tokens || ["completed", "failed", "cancelled"].includes(run.state)) return;
	const active = tokens.filter((token) => !finishedToken(token) && token.status !== "joining");
	run.activeStageIds = [...new Set(active.map((token) => token.stageId))];
	run.currentStageId = run.activeStageIds.length === 1 ? run.activeStageIds[0]! : null;

	if (run.pauseRequested && !active.some((token) => token.status === "running"))
		run.state = "paused";
	else if (active.some((token) => token.status === "running" || token.status === "ready"))
		run.state = "running";
	else if (active.some((token) => token.status === "waiting")) run.state = "waiting";
	else if (active.some((token) => token.status === "paused")) run.state = "paused";
}
export function measuredSpans(run: StoredRun): TraceSpan[] {
	const origin = Date.parse(run.startedAt ?? run.createdAt);
	const spans = (run.traceSpans ?? []).filter((span) => span.kind === "tool");

	for (const stage of run.stages ?? []) {
		if (!stage.startedAt) continue;

		spans.push({
			id: stage.id,
			parentId: null,
			name: stage.title,
			kind: "stage",
			status: stage.status,
			startOffsetMs: Date.parse(stage.startedAt) - origin,
			durationMs: stage.finishedAt
				? Date.parse(stage.finishedAt) - Date.parse(stage.startedAt)
				: null,
		});

		for (const step of stage.steps) {
			if (!step.startedAt) continue;

			spans.push({
				id: step.id,
				parentId: stage.id,
				name: step.title,
				kind: step.kind === "agent" ? "agent" : step.kind === "gate" ? "gate" : "other",
				status: step.status,
				startOffsetMs: Date.parse(step.startedAt) - origin,
				durationMs: step.finishedAt
					? Date.parse(step.finishedAt) - Date.parse(step.startedAt)
					: null,
			});

			for (const attempt of step.attempts ?? [])
				spans.push({
					id: attempt.id,
					parentId: step.id,
					name: `Attempt ${attempt.number}`,
					kind: step.kind === "agent" ? "agent" : "other",
					status: attempt.status === "waiting" ? "needs_approval" : attempt.status,
					startOffsetMs: Date.parse(attempt.startedAt) - origin,
					durationMs: attempt.finishedAt
						? Date.parse(attempt.finishedAt) - Date.parse(attempt.startedAt)
						: null,
				});
		}
	}

	return spans;
}
export function mutateRun(
	runs: RunStore,
	id: string,
	edit: (run: StoredRun) => void,
	aggregate = true,
): StoredRun {
	const run = requiredRun(runs, id);
	edit(run);
	if (aggregate) aggregateRun(run);
	run.steps = (run.stages ?? []).flatMap((stage) => stage.steps);
	run.traceSpans = measuredSpans(run);
	return runs.update(run);
}
export function recordDelivery(
	instruction: RunInstruction,
	stageInvocationId: string,
	stepId: string,
	mode: "native-turn" | "native-tool-boundary" | "next-step",
	attemptId?: string,
): void {
	if (
		instruction.deliveries?.some(
			(delivery) => delivery.stepId === stepId && delivery.attemptId === attemptId,
		)
	)
		return;

	const deliveredAt = timestamp();

	instruction.deliveries = [
		...(instruction.deliveries ?? []),
		{ stepId, stageInvocationId, attemptId, deliveredAt, mode },
	];

	instruction.deliveredAt ??= deliveredAt;
	instruction.stepId ??= stepId;
}
