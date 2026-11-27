import { query, type CanUseTool, type SDKUserMessage } from "@anthropic-ai/claude-agent-sdk/core";
import type { AdapterRequest, AgentOutcome } from "./adapter-types.ts";
import {
	outcomeSchema,
	AdapterCleanupError,
	cleanupFailure,
	type AdapterCleanupFailure,
} from "./adapter-types.ts";
import {
	bridgePrompt,
	DeliveryReceipts,
	finalOutcome,
	InputQueue,
	interact,
	jsonValue,
	object,
	toolDetails,
	formFields,
	fieldAnswer,
} from "./bridge-common.ts";
import { ClaudeProcess } from "./claude-process.ts";

export function claudePermissionHandler(request: AdapterRequest): CanUseTool {
	return async (toolName, input, native) => {
		const signal = AbortSignal.any([request.signal, native.signal]);
		const id = native.toolUseID;

		if (toolName === "AskUserQuestion") {
			const answers: Record<string, string | string[]> = {};

			for (const value of Array.isArray(input.questions) ? input.questions : []) {
				const question = object(value);

				const options = (Array.isArray(question.options) ? question.options : []).map(
					object,
				);

				const response = await interact(
					request,
					{
						id: `${id}:${question.question}`,
						provider: "claude",
						kind: "question",
						prompt: String(question.question),
						choices: options.map((option) => ({
							id: String(option.label),
							label: String(option.label),
						})),
						fields: [
							{
								id: String(question.question),
								label: String(question.question),
								type: question.multiSelect
									? "multiselect"
									: options.length
										? "select"
										: "text",
								options: options.map((option) => ({
									id: String(option.label),
									label: String(option.label),
								})),
								required: true,
								allowOther: true,
							},
						],
						payload: jsonValue(question),
					},
					signal,
				);

				const answer = fieldAnswer(response, String(question.question));
				answers[String(question.question)] = answer;
			}

			return { behavior: "allow", updatedInput: { ...input, answers }, toolUseID: id };
		}

		const response = await interact(
			request,
			{
				id,
				provider: "claude",
				kind: "permission",
				prompt: native.title ?? native.decisionReason ?? `Allow ${toolName}?`,
				choices: [
					{ id: "allow", label: "Allow once", effect: "approve" },
					{ id: "deny", label: "Deny", effect: "deny" },
				],
				tool: toolDetails(toolName, { input, path: native.blockedPath }),
				payload: jsonValue({
					toolName,
					input,
					blockedPath: native.blockedPath,
					description: native.description,
				}),
			},
			signal,
		);

		if (response.choice === "allow")
			return { behavior: "allow", updatedInput: input, toolUseID: id };

		if (response.choice !== "deny")
			throw new Error("Choose allow or deny for the Claude tool request");

		return {
			behavior: "deny",
			message: response.answer ?? "User denied this tool call",
			toolUseID: id,
		};
	};
}

export async function executeClaude(request: AdapterRequest): Promise<AgentOutcome> {
	const executable = request.agent.executable || "claude";
	const abortController = new AbortController();

	const abort = () =>
		abortController.abort(request.signal.reason ?? new Error("Claude cancelled"));

	request.signal.addEventListener("abort", abort, { once: true });

	const timer = setTimeout(
		() => abortController.abort(new Error("Claude invocation timed out")),
		request.timeoutMs,
	);

	const scoped = {
		...request,
		signal: AbortSignal.any([request.signal, abortController.signal]),
	};

	const inputs = new InputQueue<SDKUserMessage>();
	const receipts = new DeliveryReceipts();
	const sessionId = request.sessionId ?? crypto.randomUUID();
	const children: ClaudeProcess[] = [];
	let ended = false;
	let unregister: (() => void) | undefined;
	let result: AgentOutcome | undefined;
	let failure: unknown;
	const streamingMessages = new Map<string | null, string>();
	const streamedBlocks = new Set<string>();

	const message = (text: string, uuid: NonNullable<SDKUserMessage["uuid"]>): SDKUserMessage => ({
		type: "user",
		message: { role: "user", content: text },
		parent_tool_use_id: null,
		session_id: sessionId,
		uuid,
	});

	inputs.push(message(bridgePrompt(request), crypto.randomUUID()));

	const stream = query({
		prompt: inputs,
		options: {
			pathToClaudeCodeExecutable: executable,
			cwd: request.rootPath,
			...(request.sessionId ? { resume: sessionId } : { sessionId }),
			model: request.agent.model || undefined,
			abortController,
			permissionPrompts: "host",
			canUseTool: claudePermissionHandler(scoped),
			onElicitation: async (input, native) => {
				const fields = formFields(input.requestedSchema);

				const answer = await interact(
					scoped,
					{
						id: input.elicitationId ?? crypto.randomUUID(),
						provider: "claude",
						kind:
							input.title && input.mode !== "url" && !fields.length
								? "permission"
								: "question",
						prompt: input.title ?? input.message,
						choices: [
							{ id: "accept", label: "Accept", effect: "approve" },
							{ id: "decline", label: "Decline", effect: "deny" },
							{ id: "cancel", label: "Cancel", effect: "cancel" },
						],
						fields,
						payload: jsonValue({
							mode: input.mode,
							url: input.url,
							serverName: input.serverName,
						}),
					},
					AbortSignal.any([scoped.signal, native.signal]),
				);

				const action = answer.choice;

				if (action !== "accept" && action !== "decline" && action !== "cancel")
					throw new Error("Invalid Claude elicitation response");

				return {
					action,
					...(action === "accept"
						? {
								content: object(answer.value ?? answer.answers) as Record<
									string,
									string | number | boolean | string[]
								>,
							}
						: {}),
				};
			},
			systemPrompt: { type: "preset", preset: "claude_code" },
			outputFormat: {
				type: "json_schema",
				schema: JSON.parse(JSON.stringify(outcomeSchema)),
			},
			includePartialMessages: true,
			spawnClaudeCodeProcess: (options) => {
				const child = new ClaudeProcess(options, request);
				children.push(child);
				return child;
			},
		},
	});

	try {
		if (request.signal.aborted) abort();

		for await (const frame of stream) {
			if (frame.type === "system" && frame.subtype === "init") {
				await request.onSession?.(frame.session_id);

				unregister = request.registerControl?.({
					capabilities: { permissions: true, questions: true, steering: "native-turn" },
					abort: async () => {
						await stream.interrupt();
					},
					steer: ({ message: text }) => {
						if (ended) return Promise.resolve({ mode: "next-step", delivered: false });
						const id = crypto.randomUUID();
						const receipt = receipts.wait(id);
						inputs.push(message(text, id));
						return receipt;
					},
				});
			}

			const data = object(frame);

			for (const id of Array.isArray(data.user_message_uuids)
				? data.user_message_uuids
				: data.user_message_uuid
					? [data.user_message_uuid]
					: [])
				receipts.ack(String(id), "native-turn");

			if (frame.type === "stream_event") {
				const event = frame.event;

				if (event.type === "message_start")
					streamingMessages.set(frame.parent_tool_use_id, event.message.id);

				if (event.type === "message_stop")
					streamingMessages.delete(frame.parent_tool_use_id);

				const streamingMessage = streamingMessages.get(frame.parent_tool_use_id) ?? "";

				if (event.type === "content_block_delta") {
					const delta = event.delta;

					if (delta.type === "text_delta" || delta.type === "thinking_delta") {
						const kind = delta.type === "text_delta" ? "text" : "reasoning";
						streamedBlocks.add(`${streamingMessage}:${kind}`);

						request.emit({
							kind,
							text: delta.type === "text_delta" ? delta.text : delta.thinking,
							messageId: `${streamingMessage}:${event.index}`,
						});
					}
				}
			}

			if (frame.type === "assistant")
				for (const block of frame.message.content) {
					if (block.type === "text" && !streamedBlocks.delete(`${frame.message.id}:text`))
						request.emit({
							kind: frame.error ? "error" : "text",
							text: block.text,
							delta: false,
						});

					if (
						block.type === "thinking" &&
						!streamedBlocks.delete(`${frame.message.id}:reasoning`)
					)
						request.emit({ kind: "reasoning", text: block.thinking, delta: false });

					if (block.type === "tool_use")
						request.emit({
							kind: "tool",
							text: block.name,
							name: block.name,
							input: block.input,
							toolId: block.id,
							status: "running",
						});
				}

			if (frame.type === "user" && Array.isArray(frame.message.content))
				for (const block of frame.message.content)
					if (block.type === "tool_result")
						request.emit({
							kind: "tool",
							text: "Tool completed",
							toolId: block.tool_use_id,
							status: block.is_error ? "failed" : "completed",
							output:
								typeof block.content === "string"
									? block.content
									: block.content
											?.filter((part) => part.type === "text")
											.map((part) => part.text)
											.join("\n"),
						});

			if (frame.type === "result") {
				if (frame.subtype !== "success")
					throw new Error(
						`Claude ended ${frame.subtype}: ${"errors" in frame ? frame.errors.join("; ") : "No structured result"}`,
					);

				result = {
					...finalOutcome(frame.structured_output ?? frame.result),
					sessionId: frame.session_id,
					usage: {
						inputTokens: frame.usage.input_tokens,
						outputTokens: frame.usage.output_tokens,
						totalTokens: frame.usage.input_tokens + frame.usage.output_tokens,
					},
				};

				if (!receipts.size) {
					ended = true;
					inputs.close();
					break;
				}
			}
		}

		abortController.signal.throwIfAborted();
		if (!result) throw new Error("Claude ended without a factory result");
	} catch (error) {
		failure = error;
	} finally {
		ended = true;
		unregister?.();
		receipts.close();
		inputs.close();
		clearTimeout(timer);
		request.signal.removeEventListener("abort", abort);
		const errors: AdapterCleanupFailure[] = [];

		try {
			await stream.return();
		} catch (error) {
			errors.push(cleanupFailure("query.return", error));
		}

		const cleanup = await Promise.allSettled(children.map((child) => child.close()));

		for (const [index, outcome] of cleanup.entries())
			if (outcome.status === "rejected")
				errors.push(cleanupFailure(`target.close[${index}]`, outcome.reason));

		if (errors.length) failure = new AdapterCleanupError("Claude", errors, result, failure);
	}

	if (failure) throw failure;
	return result!;
}
