import type { AdapterEvent, AdapterRequest, AgentOutcome } from "./adapter-types.ts";
import { outcomeSchema, AdapterCleanupError, cleanupFailure } from "./adapter-types.ts";
import {
	bridgePrompt,
	finalOutcome,
	interact,
	jsonValue,
	object,
	waitWithSignal,
	toolDetails,
	formFields,
	nativeCwd,
	fieldAnswer,
} from "./bridge-common.ts";
import { RpcProcess } from "./rpc-process.ts";

function toolActivity(item: Record<string, unknown>, started: boolean): AdapterEvent | undefined {
	const status = started
		? "running"
		: ["failed", "declined"].includes(String(item.status)) || item.success === false
			? "failed"
			: "completed";

	const base: AdapterEvent = {
		kind: "tool",
		text: String(item.tool ?? item.type),
		name: String(item.tool ?? item.type),
		toolId: String(item.id),
		status,
	};

	switch (item.type) {
		case "commandExecution":
			return {
				...base,
				name: "command",
				text: String(item.command ?? "command"),
				input: { command: item.command, cwd: item.cwd },
				output: item.aggregatedOutput,
			};
		case "fileChange":
			return { ...base, input: item.changes };
		case "mcpToolCall":
			return {
				...base,
				input: item.arguments,
				output: item.error
					? { error: item.error, result: item.result }
					: (item.result ?? undefined),
			};
		case "dynamicToolCall":
			return { ...base, input: item.arguments, output: item.contentItems };
		case "webSearch":
			return { ...base, input: { query: item.query, action: item.action } };
		default:
			return undefined;
	}
}

async function approval(
	request: AdapterRequest,
	method: string,
	params: Record<string, unknown>,
	id: string,
	signal: AbortSignal,
): Promise<unknown> {
	if (method === "item/tool/requestUserInput" || method === "tool/requestUserInput") {
		const answers: Record<string, { answers: string[] }> = {};

		for (const value of Array.isArray(params.questions) ? params.questions : []) {
			const question = object(value);
			const options = (Array.isArray(question.options) ? question.options : []).map(object);

			const response = await interact(
				request,
				{
					id: `${id}:${question.id}`,
					provider: "codex",
					kind: "question",
					prompt: String(question.question),
					choices: options.map((option) => ({
						id: String(option.label),
						label: String(option.label),
					})),
					fields: [
						{
							id: String(question.id),
							label: String(question.question),
							type: options.length ? "select" : "text",
							options: options.map((option) => ({
								id: String(option.label),
								label: String(option.label),
							})),
							required: true,
							allowOther: question.isOther === true,
						},
					],
					payload: jsonValue(question),
				},
				signal,
			);

			const answer = fieldAnswer(response, String(question.id));
			answers[String(question.id)] = { answers: Array.isArray(answer) ? answer : [answer] };
		}

		return { answers };
	}

	if (method === "mcpServer/elicitation/request") {
		const response = await interact(
			request,
			{
				id,
				provider: "codex",
				kind: "question",
				prompt: String(params.message ?? "MCP input required"),
				choices: ["accept", "decline", "cancel"].map((id) => ({ id, label: id })),
				fields: formFields(params.requestedSchema),
				payload: jsonValue(params),
			},
			signal,
		);

		const action = response.choice ?? "cancel";

		if (!["accept", "decline", "cancel"].includes(action))
			throw new Error("Invalid elicitation response");

		return {
			action,
			content: action === "accept" ? (response.value ?? response.answers ?? null) : null,
		};
	}

	if (method === "item/permissions/requestApproval") {
		const response = await interact(
			request,
			{
				id,
				provider: "codex",
				kind: "permission",
				prompt: String(
					params.reason ?? "Allow the requested filesystem/network access for this turn?",
				),
				choices: [
					{
						id: "accept",
						label: "Allow requested access for this turn",
						effect: "approve",
					},
					{ id: "decline", label: "Deny", effect: "deny" },
				],
				tool: toolDetails("permissions", params),
				payload: jsonValue(params),
			},
			signal,
		);

		if (response.choice !== "accept" && response.choice !== "decline")
			throw new Error("Choose allow or deny for the native permission request");

		return {
			permissions: response.choice === "accept" ? (params.permissions ?? {}) : {},
			scope: "turn",
		};
	}

	if (
		method !== "item/commandExecution/requestApproval" &&
		method !== "item/fileChange/requestApproval"
	)
		throw new Error(`Unsupported Codex server request: ${method}`);

	const available = Array.isArray(params.availableDecisions)
		? params.availableDecisions
		: ["accept", "decline", "cancel"];

	// Offer only one-shot decisions; persistent policy amendments are separate configuration.
	const choices = available.filter(
		(value): value is string =>
			typeof value === "string" && ["accept", "decline", "cancel"].includes(value),
	);

	const network = object(params.networkApprovalContext);

	const prompt = network.host
		? `Allow ${String(network.protocol ?? "network")} access to ${String(network.host)}?`
		: String(params.reason ?? params.command ?? "Allow the proposed file change?");

	const response = await interact(
		request,
		{
			id,
			provider: "codex",
			kind: "permission",
			prompt,
			choices: choices.map((id) => ({
				id,
				label: id === "accept" ? "Allow once" : id,
				effect: id === "accept" ? "approve" : id === "cancel" ? "cancel" : "deny",
			})),
			tool: toolDetails(method.includes("fileChange") ? "fileChange" : "command", params),
			payload: jsonValue(params),
		},
		signal,
	);

	if (!response.choice || !choices.includes(response.choice))
		throw new Error("Invalid native Codex permission decision");

	return { decision: response.choice };
}

export async function executeCodex(request: AdapterRequest): Promise<AgentOutcome> {
	const launch = await request.launch(request.agent.executable || "codex", [
		...(request.agent.args ?? []),
		"app-server",
		"--listen",
		"stdio://",
	]);

	const transport = new AbortController();
	const rpc = new RpcProcess(launch, transport.signal);
	const signal = AbortSignal.any([request.signal, AbortSignal.timeout(request.timeoutMs)]);
	let threadId = request.sessionId ?? "";
	let turnId = "";
	let finalText = "";
	let usage: AgentOutcome["usage"];
	const items = new Map<string, Record<string, unknown>>();
	const streamedMessages = new Set<string>();
	const streamedReasoning = new Set<string>();
	let finish!: (value: AgentOutcome) => void;
	let fail!: (error: Error) => void;

	const done = new Promise<AgentOutcome>((resolve, reject) => {
		finish = resolve;
		fail = reject;
	});

	void done.catch(() => {});
	void rpc.closed.then(() => fail(new Error("Codex transport closed before the turn finished")));

	rpc.onRequest = (method, params, id, nativeSignal) => {
		if (params.threadId && params.threadId !== threadId)
			return Promise.reject(new Error("Permission belongs to another Codex thread"));

		return approval(
			request,
			method,
			{ ...items.get(String(params.itemId)), ...params },
			id,
			AbortSignal.any([signal, nativeSignal]),
		);
	};

	rpc.onNotification = (method, params) => {
		if (params.threadId && params.threadId !== threadId) return;
		const turn = object(params.turn);
		if (method === "turn/started") turnId = String(turn.id);

		if (method === "item/agentMessage/delta") {
			streamedMessages.add(String(params.itemId));

			request.emit({
				kind: "text",
				text: String(params.delta ?? ""),
				messageId: String(params.itemId),
			});
		}

		if (method === "item/reasoning/summaryTextDelta" || method === "item/reasoning/textDelta") {
			streamedReasoning.add(String(params.itemId));

			request.emit({
				kind: "reasoning",
				text: String(params.delta ?? ""),
				messageId: `${params.itemId}:${method}:${params.summaryIndex ?? params.contentIndex ?? 0}`,
			});
		}

		if (method === "item/commandExecution/outputDelta")
			request.emit({
				kind: "tool",
				text: "command",
				toolId: String(params.itemId),
				outputDelta: String(params.delta ?? ""),
			});

		if (method === "error" && typeof object(params.error).message === "string")
			request.emit({
				kind: "error",
				text: String(object(params.error).message),
				delta: false,
			});

		const item = object(params.item);
		if (typeof item.id === "string") items.set(item.id, item);

		if (method === "item/completed" && item.type === "agentMessage") {
			finalText = String(item.text ?? "");

			if (!streamedMessages.delete(String(item.id)))
				request.emit({ kind: "text", text: finalText, delta: false });
		}

		if (
			method === "item/completed" &&
			item.type === "reasoning" &&
			!streamedReasoning.delete(String(item.id))
		) {
			const parts = [
				...(Array.isArray(item.summary) ? item.summary : []),
				...(Array.isArray(item.content) ? item.content : []),
			];

			request.emit({
				kind: "reasoning",
				text: parts.filter((part): part is string => typeof part === "string").join("\n\n"),
				delta: false,
			});
		}

		if (
			(method === "item/started" || method === "item/completed") &&
			typeof item.id === "string"
		) {
			const event = toolActivity(item, method === "item/started");
			if (event) request.emit(event);
		}

		if (method === "thread/tokenUsage/updated") {
			const total = object(object(params.tokenUsage).last);

			if (typeof total.inputTokens === "number" && typeof total.outputTokens === "number")
				usage = {
					inputTokens: total.inputTokens,
					outputTokens: total.outputTokens,
					totalTokens: Number(
						total.totalTokens ?? total.inputTokens + total.outputTokens,
					),
				};
		}

		if (method === "turn/completed") {
			if (turnId && turn.id !== turnId) return;

			try {
				if (turn.status !== "completed")
					throw new Error(
						String(object(turn.error).message ?? `Codex turn ${turn.status}`),
					);

				finish({ ...finalOutcome(finalText), sessionId: threadId, usage });
			} catch (error) {
				fail(error as Error);
			}
		}
	};

	const abort = async () => {
		if (threadId && turnId) await rpc.call("turn/interrupt", { threadId, turnId });
	};

	let unregister: (() => void) | undefined;
	let completed: AgentOutcome | undefined;
	let failure: unknown;

	try {
		await waitWithSignal(
			rpc.call("initialize", {
				clientInfo: { name: "karabiner", title: "Karabiner", version: "0.1.0" },
				capabilities: { experimentalApi: true },
			}),
			signal,
		);

		rpc.notify("initialized");

		const response = await waitWithSignal(
			rpc.call(request.sessionId ? "thread/resume" : "thread/start", {
				...(request.sessionId ? { threadId: request.sessionId, excludeTurns: true } : {}),
				cwd: nativeCwd(request),
				...(request.agent.model ? { model: request.agent.model } : {}),
			}),
			signal,
		);

		threadId = String(object(response.thread).id ?? "");
		if (!threadId) throw new Error("Codex did not return a thread identity");
		await request.onSession?.(threadId);

		unregister = request.registerControl?.({
			capabilities: { permissions: true, questions: true, steering: "native-turn" },
			abort,
			steer: async ({ message }) => {
				if (!turnId) return { mode: "next-step", delivered: false };

				const result = await rpc.call("turn/steer", {
					threadId,
					expectedTurnId: turnId,
					input: [{ type: "text", text: message }],
				});

				return { mode: "native-turn", delivered: result.turnId === turnId };
			},
		});

		const started = await waitWithSignal(
			rpc.call("turn/start", {
				threadId,
				input: [{ type: "text", text: bridgePrompt(request) }],
				outputSchema: outcomeSchema,
			}),
			signal,
		);

		turnId = String(object(started.turn).id ?? turnId);
		completed = await waitWithSignal(done, signal);
	} catch (error) {
		failure = error;
	} finally {
		unregister?.();

		if (signal.aborted)
			await Promise.race([
				abort().catch(() => {}),
				new Promise((resolve) => setTimeout(resolve, 1_000)),
			]);

		try {
			await rpc.close();
		} catch (error) {
			failure = new AdapterCleanupError(
				"Codex",
				[cleanupFailure("transport.close", error)],
				completed,
				failure,
			);
		}
	}

	if (failure) throw failure;
	return completed!;
}
