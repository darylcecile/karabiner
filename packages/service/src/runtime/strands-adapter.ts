import { createHarness } from "@strands-agents/harness";
import {
	SessionManager,
	tool,
	BeforeToolCallEvent,
	AfterModelCallEvent,
	AfterToolCallEvent,
	type AgentResult,
	type Model,
} from "@strands-agents/sdk";
import {
	PosixShellSandbox,
	type ExecuteOptions,
	type ExecutionResult,
	type StreamChunk,
} from "@strands-agents/sdk/sandbox";
import { Skill } from "@strands-agents/sdk/vended-plugins/skills";
import { ContextInjector } from "@strands-agents/sdk/vended-plugins/context-injector";
import {
	outcomeSchema,
	parseOutcome,
	AdapterCleanupError,
	cleanupFailure,
	type AdapterRequest,
	type AgentOutcome,
} from "./adapter-types.ts";
import { runPrepared } from "./process.ts";
import { DeliveryReceipts, interact, jsonValue, toolDetails } from "./bridge-common.ts";
import { resolveStrandsModel } from "./strands-model.ts";

class ProjectSandbox extends PosixShellSandbox {
	constructor(private readonly request: AdapterRequest) {
		super();
	}
	async *executeStreaming(
		command: string,
		options: ExecuteOptions = {},
	): AsyncGenerator<StreamChunk | ExecutionResult> {
		const launch = await this.request.launch(
			"/bin/sh",
			["-c", command],
			options.cwd,
			options.env,
		);

		const result = await runPrepared({
			...launch,
			signal: options.signal
				? AbortSignal.any([options.signal, this.request.signal])
				: this.request.signal,
			timeoutMs: Math.min(this.request.timeoutMs, (options.timeout ?? 120) * 1_000),
			onLine: (line, stream) =>
				this.request.emit({ kind: "activity", text: `${stream}: ${line}` }),
		});

		yield {
			type: "executionResult",
			exitCode: result.code,
			stdout: result.stdout,
			stderr: result.stderr,
			outputFiles: [],
		};
	}
}

function usage(result: AgentResult): AgentOutcome["usage"] {
	const value = result.metrics?.accumulatedUsage;
	if (!value) return;

	return {
		inputTokens: value.inputTokens ?? 0,
		outputTokens: value.outputTokens ?? 0,
		totalTokens: value.totalTokens ?? (value.inputTokens ?? 0) + (value.outputTokens ?? 0),
	};
}

/** Strands is opt-in; its model credential path is separate from installed CLI authentication. */
export async function executeStrands(
	request: AdapterRequest,
	options: { model?: Model } = {},
): Promise<AgentOutcome> {
	if (!request.agent.model)
		throw new Error("Strands requires an explicitly configured provider/model");

	if (!request.storage) throw new Error("Strands requires durable session storage");
	let completed: AgentOutcome | undefined;
	const receipts = new DeliveryReceipts();
	const pendingGuidance = new Map<string, string>();
	let sentGuidance: string[] = [];

	const injector = new ContextInjector({
		name: "karabiner:operator-guidance",
		trigger: "everyTurn",
		renderContent: async () => {
			if (!pendingGuidance.size) return undefined;
			sentGuidance = [...pendingGuidance.keys()];
			return `Operator instructions:\n${[...pendingGuidance.values()].join("\n\n")}`;
		},
	});

	const finish = tool({
		name: "finish_step",
		description:
			"Finish this factory step with its factual outcome. Call only after completing the work or identifying a blocker or permitted return.",
		inputSchema: JSON.parse(JSON.stringify(outcomeSchema)),
		callback: async (input: unknown, context) => {
			const outcome = parseOutcome(input);

			if (!outcome || outcome.outcome === "question")
				throw new Error("Invalid step result; use ask_user for questions");

			completed = outcome;
			context?.agent.cancel({ message: outcome.summary, afterCurrentTools: true });
			return outcome.summary;
		},
	});

	const ask = tool({
		name: "ask_user",
		description:
			"Ask the user for a decision and pause until they answer. Never supply an answer yourself.",
		inputSchema: {
			type: "object",
			properties: { question: { type: "string" } },
			required: ["question"],
			additionalProperties: false,
		},
		callback: async (input: unknown, context) => {
			if (request.agent.mode !== "interactive")
				throw new Error("Only interactive factory agents may ask questions");

			const question = (input as { question: string }).question;

			if (!question?.trim() || !context)
				throw new Error("A nonempty question and tool context are required");

			if (!request.interact)
				return context.interrupt({ name: "karabiner-question", reason: { question } });

			const answer = await interact(
				request,
				{
					id: context.toolUse.toolUseId,
					provider: "strands",
					kind: "question",
					prompt: question,
				},
				context.cancelSignal,
			);

			return answer.answer ?? answer.choice ?? answer.value ?? "";
		},
	});

	const sessionId = request.sessionId ?? `run-${request.runId}-step-${request.stepId}`;

	const agent = await createHarness({
		id: request.stepId,
		name: request.agent.name,
		model: options.model ?? (await resolveStrandsModel(request.agent)),
		instructions: `${request.factoryInstructions ?? request.instructions}\n\nCall finish_step with a factual result to end this step. Use only the supplied permitted return stage IDs. Never publish, merge or change external task status unless explicitly instructed by this step.`,
		skills: request.skills.map(
			(skill) =>
				new Skill({
					name: skill.name.toLowerCase().replace(/[^a-z0-9-]/g, "-"),
					description: skill.description || skill.name,
					instructions: skill.content,
				}),
		),
		builtinTools: ["shell", "read", "write", "edit"],
		tools: request.agent.mode === "interactive" ? [finish, ask] : [finish],
		toolExecutor: "sequential",
		sandbox: new ProjectSandbox(request),
		storage: request.storage,
		sessionManager: new SessionManager({
			sessionId,
			storage: request.storage,
			saveLatestOn: "message",
		}),
		memory: false,
		printer: false,
		plugins: [injector],
	});

	agent.addHook(AfterModelCallEvent, (event) => {
		if (event.error) return;

		for (const id of sentGuidance) {
			receipts.ack(id, "native-tool-boundary");
			pendingGuidance.delete(id);
		}

		sentGuidance = [];
	});

	agent.addHook(AfterToolCallEvent, (event) => {
		if (event.error instanceof AdapterCleanupError) throw event.error;
	});

	if (request.interact)
		agent.addHook(BeforeToolCallEvent, async (event) => {
			if (!["shell", "write", "edit"].includes(event.toolUse.name)) return;

			const response = await interact(
				request,
				{
					id: event.toolUse.toolUseId,
					provider: "strands",
					kind: "permission",
					prompt: `Allow ${event.toolUse.name}?`,
					choices: [
						{ id: "allow", label: "Allow once", effect: "approve" },
						{ id: "deny", label: "Deny", effect: "deny" },
					],
					tool: toolDetails(event.toolUse.name, { input: event.toolUse.input }),
					payload: jsonValue(event.toolUse.input),
				},
				event.agent.cancelSignal,
			);

			if (response.choice !== "allow" && response.choice !== "deny")
				throw new Error("Invalid Strands tool permission decision");

			if (response.choice === "deny") event.cancel = "User denied this tool call";
		});

	await request.onSession?.(sessionId);

	const unregister = request.registerControl?.({
		capabilities: {
			permissions: Boolean(request.interact),
			questions: Boolean(request.interact),
			steering: "native-tool-boundary",
		},
		abort: async () => agent.cancel(),
		steer: ({ id, message }) => {
			pendingGuidance.set(id, message);
			return receipts.wait(id);
		},
	});

	let finalResult: AgentOutcome | undefined;
	let failure: unknown;

	try {
		// SDK 1.26 requires interrupt-response-only input on resume. Its response may be JSON,
		// so ask_user returns the exact answer and the captured guidance as separate fields.
		const guidance = request.operatorInstructions ?? [];

		const response = guidance.length
			? { answer: request.answer?.response ?? null, operatorInstructions: guidance }
			: (request.answer?.response ?? null);

		const prompt = guidance.length
			? `${request.prompt}\n\n## Operator instructions\n\n${guidance.join("\n\n")}`
			: request.prompt;

		const input = request.answer
			? [{ interruptResponse: { interruptId: request.answer.requestId, response } }]
			: prompt;

		const stream = agent.stream(input, {
			cancelSignal: AbortSignal.any([request.signal, AbortSignal.timeout(request.timeoutMs)]),
			limits: { turns: 100 },
		});

		let result: AgentResult | undefined;

		for await (const event of stream) {
			if (event.type === "agentResultEvent") result = event.result;

			if (event.type === "beforeToolCallEvent")
				request.emit({
					kind: "tool",
					text: event.toolUse.name,
					name: event.toolUse.name,
					input: event.toolUse.input,
					toolId: event.toolUse.toolUseId,
					status: "running",
				});

			if (event.type === "toolResultEvent")
				request.emit({
					kind: "tool",
					text: "Tool finished",
					toolId: event.result.toolUseId,
					status: event.result.status === "error" ? "failed" : "completed",
					output: {
						content: event.result.content
							.filter(
								(block) => block.type === "textBlock" || block.type === "jsonBlock",
							)
							.map((block) => block.toJSON()),
						error: event.result.error?.message,
					},
				});

			if (
				event.type === "modelStreamUpdateEvent" &&
				event.event.type === "modelContentBlockDeltaEvent" &&
				event.event.delta.type === "textDelta"
			)
				request.emit({ kind: "text", text: event.event.delta.text });

			if (
				event.type === "modelStreamUpdateEvent" &&
				event.event.type === "modelContentBlockDeltaEvent" &&
				event.event.delta.type === "reasoningContentDelta" &&
				event.event.delta.text
			)
				request.emit({ kind: "reasoning", text: event.event.delta.text });
		}

		request.signal.throwIfAborted();
		if (!result) throw new Error("Strands ended without a result event");

		if (result.stopReason === "interrupt") {
			const interrupt = result.interrupts?.[0];
			if (!interrupt) throw new Error("Strands interrupted without a pending request");

			const question = String(
				(interrupt.reason as { question?: string })?.question ??
					interrupt.reason ??
					"Human input required",
			);

			finalResult = {
				outcome: "question",
				summary: question,
				question,
				requestId: interrupt.id,
				sessionId,
				usage: usage(result),
			};
		} else {
			if (!completed)
				throw new Error(`Strands stopped without finish_step (${result.stopReason})`);

			finalResult = { ...completed, sessionId, usage: usage(result) };
		}
	} catch (error) {
		failure = error;
	} finally {
		unregister?.();
		receipts.close();

		try {
			await agent.shutdown();
		} catch (error) {
			failure = new AdapterCleanupError(
				"Strands",
				[cleanupFailure("agent.shutdown", error)],
				finalResult,
				failure,
			);
		}
	}

	if (failure) throw failure;
	return finalResult!;
}
