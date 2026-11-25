import type { InteractionResponse, RunInstruction, RunQuestion } from "@karabiner/contracts";
import type { RunStore } from "../run-store.ts";
import type { AdapterAnswer, AdapterControl, AdapterInteraction } from "./adapter-types.ts";
import { invocationStep, mutateRun, recordDelivery, requiredRun, timestamp } from "./run-state.ts";

interface LiveWait {
	runId: string;
	stepId: string;
	promise: Promise<AdapterAnswer>;
	resolve: (answer: AdapterAnswer) => void;
	reject: (error: unknown) => void;
	dispose: () => void;
}

interface LiveControl {
	runId: string;
	stageInvocationId: string;
	stepId: string;
	attemptId?: string;
	control: AdapterControl;
}
export interface InvocationAddress {
	runId: string;
	tokenId: string;
	stageInvocationId: string;
	stepId: string;
	attemptId?: string;
}

/** Owns in-process continuations; SQLite owns their factual request/answer records. */
export class RunInteractions {
	private readonly waits = new Map<string, LiveWait>();
	private readonly controls = new Map<string, LiveControl>();
	private readonly engines = new Map<string, () => void>();
	private readonly deliveries = new Map<Promise<void>, string>();
	constructor(private readonly runs: RunStore) {}
	attach(runId: string, wake: () => void): () => void {
		this.engines.set(runId, wake);

		return () => {
			if (this.engines.get(runId) === wake) this.engines.delete(runId);
		};
	}
	wake(runId: string): boolean {
		const wake = this.engines.get(runId);
		wake?.();
		return !!wake;
	}
	isLive(questionId: string): boolean {
		return this.waits.has(questionId);
	}
	async interact(
		address: InvocationAddress,
		input: AdapterInteraction,
		signal: AbortSignal,
	): Promise<AdapterAnswer> {
		signal.throwIfAborted();
		const run = requiredRun(this.runs, address.runId);

		const existing = run.questions.find(
			(question) =>
				question.stepId === address.stepId &&
				question.requestId === input.id &&
				question.provider === input.provider &&
				question.attemptId === address.attemptId &&
				question.state !== "cancelled",
		);

		if (existing?.state === "answered")
			return existing.response ?? { answer: existing.answer ?? "" };

		const pending = existing && this.waits.get(existing.id);
		if (pending) return pending.promise;

		const question: RunQuestion = existing ?? {
			id: crypto.randomUUID(),
			stageId: run.execution!.tokens.find((token) => token.id === address.tokenId)!.stageId,
			stepId: address.stepId,
			stageInvocationId: address.stageInvocationId,
			tokenId: address.tokenId,
			attemptId: address.attemptId,
			kind: input.kind,
			prompt: input.prompt,
			provider: input.provider,
			requestId: input.id,
			transport: "live",
			choices: input.choices,
			payload: input.payload,
			tool: input.tool,
			fields: input.fields,
			state: "pending",
			createdAt: timestamp(),
			answer: null,
			answeredAt: null,
		};

		const deferred = Promise.withResolvers<AdapterAnswer>();

		const abort = () => {
			this.waits.delete(question.id);

			mutateRun(this.runs, address.runId, (current) => {
				const item = current.questions.find((item) => item.id === question.id);
				if (item?.state === "pending") item.state = "cancelled";

				const token = current.execution?.tokens.find(
					(token) => token.id === address.tokenId,
				);

				if (token?.status === "waiting") token.status = "running";
				const step = invocationStep(current, address.stageInvocationId, address.stepId);
				if (step.status === "needs_approval") step.status = "running";

				if (step.attempts?.at(-1)?.status === "waiting")
					step.attempts.at(-1)!.status = "running";
			});

			deferred.reject(signal.reason ?? new Error("Interaction cancelled"));
		};

		this.waits.set(question.id, {
			...deferred,
			runId: address.runId,
			stepId: address.stepId,
			dispose: () => signal.removeEventListener("abort", abort),
		});

		signal.addEventListener("abort", abort, { once: true });

		mutateRun(this.runs, address.runId, (current) => {
			if (!existing) current.questions.push(question);
			const token = current.execution!.tokens.find((token) => token.id === address.tokenId)!;
			token.status = "waiting";
			const step = invocationStep(current, address.stageInvocationId, address.stepId);
			step.status = "needs_approval";
			if (step.attempts?.at(-1)) step.attempts.at(-1)!.status = "waiting";
			current.activity = input.prompt;
		});

		if (signal.aborted) abort();

		try {
			return await deferred.promise;
		} finally {
			signal.removeEventListener("abort", abort);
			this.waits.delete(question.id);
		}
	}
	answer(runId: string, questionId: string): boolean {
		const live = this.waits.get(questionId);
		if (!live || live.runId !== runId) return false;
		const run = requiredRun(this.runs, runId);
		if (run.pauseRequested || run.state === "paused") return true;
		const question = run.questions.find((item) => item.id === questionId)!;
		if (question.state !== "answered") return true;

		mutateRun(this.runs, runId, (current) => {
			const token = current.execution?.tokens.find((token) => token.id === question.tokenId);
			if (token) token.status = "running";

			if (question.stageInvocationId && question.stepId) {
				const step = invocationStep(current, question.stageInvocationId, question.stepId);
				step.status = "running";
				if (step.attempts?.at(-1)) step.attempts.at(-1)!.status = "running";
			}
		});

		live.resolve(
			(question.response ?? { answer: question.answer ?? "" }) as InteractionResponse,
		);

		return true;
	}
	resume(runId: string): void {
		for (const [id, live] of this.waits) if (live.runId === runId) this.answer(runId, id);
		this.wake(runId);
	}
	register(
		address: InvocationAddress,
		control: AdapterControl,
		captured: Set<string>,
	): () => void {
		const key = `${address.runId}/${address.stepId}`;
		const live = { ...address, control };
		this.controls.set(key, live);

		mutateRun(this.runs, address.runId, (run) => {
			invocationStep(run, address.stageInvocationId, address.stepId).control =
				control.capabilities;
		});

		const run = requiredRun(this.runs, address.runId);

		for (const instruction of run.instructions ?? [])
			if (!captured.has(instruction.id)) void this.scheduleDelivery(live, instruction);

		return () => {
			if (this.controls.get(key) !== live) return;
			this.controls.delete(key);

			mutateRun(this.runs, address.runId, (run) => {
				delete invocationStep(run, address.stageInvocationId, address.stepId).control;
			});
		};
	}
	async steer(runId: string, instruction: RunInstruction): Promise<void> {
		await Promise.all(
			[...this.controls.values()]
				.filter((live) => live.runId === runId)
				.map((live) => this.scheduleDelivery(live, instruction)),
		);
	}
	private scheduleDelivery(live: LiveControl, instruction: RunInstruction): Promise<void> {
		const work = this.deliver(live, instruction).finally(() => this.deliveries.delete(work));
		this.deliveries.set(work, live.runId);
		return work;
	}
	private async deliver(live: LiveControl, instruction: RunInstruction): Promise<void> {
		const run = requiredRun(this.runs, live.runId);
		if (run.pauseRequested || run.state === "paused") return;

		try {
			const result = await live.control.steer({
				id: instruction.id,
				message: instruction.message,
			});

			if (!result.delivered) return;

			mutateRun(this.runs, live.runId, (current) => {
				const item = current.instructions?.find((item) => item.id === instruction.id);

				if (item)
					recordDelivery(
						item,
						live.stageInvocationId,
						live.stepId,
						result.mode,
						live.attemptId,
					);
			});
		} catch (error) {
			mutateRun(this.runs, live.runId, (current) => {
				current.errors.push({
					code: "guidance_delivery_failed",
					stepId: live.stepId,
					stageInvocationId: live.stageInvocationId,
					message:
						error instanceof Error ? error.message : "Native guidance delivery failed",
				});
			});
		}
	}
	async abort(runId: string, reason: unknown): Promise<void> {
		for (const [id, live] of this.waits)
			if (live.runId === runId) {
				live.dispose();
				live.reject(reason);
				this.waits.delete(id);
			}

		const results = await Promise.allSettled(
			[...this.controls.values()]
				.filter((live) => live.runId === runId)
				.map((live) => live.control.abort()),
		);

		for (const result of results)
			if (result.status === "rejected")
				mutateRun(this.runs, runId, (run) => {
					run.errors.push({
						code: "control_abort_failed",
						message:
							result.reason instanceof Error
								? result.reason.message
								: "Native control abort failed",
					});
				});

		await this.settle(runId);
		this.wake(runId);
	}
	async settle(runId: string): Promise<void> {
		await Promise.allSettled(
			[...this.deliveries].filter(([, id]) => id === runId).map(([work]) => work),
		);
	}
	async close(): Promise<void> {
		const ids = new Set(
			[...this.controls.values()]
				.map((live) => live.runId)
				.concat([...this.deliveries.values()]),
		);

		await Promise.allSettled(
			[...ids].map((id) => this.abort(id, new Error("Runtime shut down"))),
		);
	}
}
