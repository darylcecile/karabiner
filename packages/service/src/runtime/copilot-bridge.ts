import {
	CopilotClient,
	RuntimeConnection,
	createAttributedResult,
	type CopilotSession,
	type PermissionHandler,
	type CreateSessionOptions,
} from "@microsoft/copilot-sdk";
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
	interact,
	jsonValue,
	object,
	waitWithSignal,
	toolDetails,
	formFields,
	nativeCwd,
	fieldAnswer,
} from "./bridge-common.ts";
import { copilotLaunch } from "./copilot-launch.ts";

const humanDecision = {
	source: "human_response",
	responseCapability: "interactive",
	surface: "sdk",
	outcome: "prompted_user",
} as const;

export function copilotPermissionHandler(request: AdapterRequest): PermissionHandler {
	return async (permission) => {
		const details = object(permission);

		const response = await interact(request, {
			id: `${details.toolCallId ?? crypto.randomUUID()}:${permission.kind}`,
			provider: "copilot",
			kind: "permission",
			prompt: String(
				details.intention ??
					details.fullCommandText ??
					details.url ??
					`Allow ${permission.kind}?`,
			),
			choices: [
				{ id: "approve-once", label: "Allow once", effect: "approve" },
				{ id: "reject", label: "Deny", effect: "deny" },
			],
			tool: toolDetails(String(details.toolName ?? permission.kind), details),
			payload: jsonValue(permission),
		});

		if (response.choice === "approve-once")
			return createAttributedResult(
				{ kind: "approve-once", approvedInteractively: true },
				humanDecision,
			);

		if (response.choice !== "reject")
			throw new Error("Choose allow once or deny for this Copilot permission");

		return createAttributedResult(
			{ kind: "reject", feedback: response.answer ?? "User denied this request" },
			humanDecision,
		);
	};
}

export async function executeCopilot(request: AdapterRequest): Promise<AgentOutcome> {
	const launch = await copilotLaunch(request);

	const client = new CopilotClient({
		runtime: RuntimeConnection.forSpawnedStdio({
			path: launch.path,
			args: launch.args,
			workingDirectory: launch.cwd,
			env: launch.env,
		}),
		mode: "coding-agent",
		github: { useLoggedInUser: true },
		observability: { logLevel: "none" },
	});

	const deadline = new AbortController();
	const signal = AbortSignal.any([request.signal, deadline.signal]);

	const timer = setTimeout(
		() => deadline.abort(new Error("Copilot invocation timed out")),
		request.timeoutMs,
	);

	const scoped = { ...request, signal };
	const receipts = new DeliveryReceipts();
	let session: CopilotSession | undefined;
	let unregister: (() => void) | undefined;
	let unsubscribe: (() => void) | undefined;
	let usage: AgentOutcome["usage"];
	let error: unknown;
	let result: AgentOutcome | undefined;

	try {
		await waitWithSignal(client.start(), signal);

		const config: CreateSessionOptions = {
			model: request.agent.model || undefined,
			workingDirectory: nativeCwd(request),
			events: { streaming: true },
			systemMessage: {
				mode: "append",
				content: request.factoryInstructions ?? request.instructions,
			},
			permissions: { onRequest: copilotPermissionHandler(scoped) },
			interaction: {
				onUserInputRequest: async (input) => {
					const response = await interact(scoped, {
						id: input.requestId ?? crypto.randomUUID(),
						provider: "copilot",
						kind: "question",
						prompt: input.question,
						choices: input.choices?.map((id) => ({ id, label: id })),
						fields: [
							{
								id: "answer",
								label: input.question,
								type:
									input.choices?.length && input.allowFreeform === false
										? "select"
										: "text",
								options: input.choices?.map((id) => ({ id, label: id })),
								required: true,
								allowOther: input.allowFreeform !== false,
							},
						],
						payload: jsonValue({ allowFreeform: input.allowFreeform }),
					});

					const answer = fieldAnswer(response, "answer");

					if (typeof answer !== "string")
						throw new Error("Copilot requires a single text answer");

					return createAttributedResult(
						{ answer, wasFreeform: !input.choices?.includes(answer) },
						humanDecision,
					);
				},
				onElicitationRequest: async (input) => {
					const response = await interact(scoped, {
						id: input.requestId ?? crypto.randomUUID(),
						provider: "copilot",
						kind: "question",
						prompt: input.message,
						choices: ["accept", "decline", "cancel"].map((id) => ({ id, label: id })),
						fields: formFields(input.requestedSchema),
						payload: jsonValue({
							requestedSchema: input.requestedSchema,
							mode: input.mode,
							url: input.url,
						}),
					});

					const action = response.choice;

					if (action !== "accept" && action !== "decline" && action !== "cancel")
						throw new Error("Invalid Copilot elicitation response");

					return createAttributedResult(
						{
							action,
							...(action === "accept"
								? {
										content: object(
											response.value ?? response.answers,
										) as Record<string, string>,
									}
								: {}),
						},
						humanDecision,
					);
				},
			},
		};

		session = await waitWithSignal(
			request.sessionId
				? client.resumeSession(request.sessionId, config)
				: client.createSession(config),
			signal,
		);

		await request.onSession?.(session.sessionId);
		const active = session;

		unsubscribe = active.on((event) => {
			if (event.type === "user.message" && event.data.clientTurnId)
				receipts.ack(event.data.clientTurnId, "native-turn");

			if (event.type === "assistant.message_delta")
				request.emit({ kind: "text", text: event.data.deltaContent });

			if (event.type === "tool.execution_start")
				request.emit({
					kind: "tool",
					text: event.data.toolName,
					toolId: event.data.toolCallId,
					status: "running",
				});

			if (event.type === "tool.execution_complete")
				request.emit({
					kind: "tool",
					text: "Tool completed",
					toolId: event.data.toolCallId,
					status: event.data.success ? "completed" : "failed",
				});

			if (event.type === "assistant.usage") {
				const data = object(event.data);

				if (typeof data.inputTokens === "number" && typeof data.outputTokens === "number")
					usage = {
						inputTokens: (usage?.inputTokens ?? 0) + data.inputTokens,
						outputTokens: (usage?.outputTokens ?? 0) + data.outputTokens,
						totalTokens:
							(usage?.totalTokens ?? 0) + data.inputTokens + data.outputTokens,
					};
			}
		});

		unregister = request.registerControl?.({
			capabilities: { permissions: true, questions: true, steering: "native-turn" },
			abort: () => active.abort(),
			steer: async ({ id, message }) => {
				const receipt = receipts.wait(id);
				await active.send({ prompt: message, mode: "immediate", clientTurnId: id });
				return receipt;
			},
		});

		const message = await waitWithSignal(
			active.sendAndWait(
				{
					prompt: bridgePrompt(request),
					responseSchema: JSON.parse(JSON.stringify(outcomeSchema)),
				},
				request.timeoutMs,
			),
			signal,
		);

		signal.throwIfAborted();
		result = { ...finalOutcome(message?.data.content), sessionId: active.sessionId, usage };
	} catch (caught) {
		error = caught;
	} finally {
		unregister?.();
		receipts.close();
		unsubscribe?.();
		clearTimeout(timer);
		const cleanupErrors: AdapterCleanupFailure[] = [];

		const cleanup = async (phase: string, work: () => Promise<unknown>) => {
			try {
				await waitWithSignal(work(), AbortSignal.timeout(10_000));
			} catch (caught) {
				cleanupErrors.push(cleanupFailure(phase, caught));
			}
		};

		if (session && (signal.aborted || error))
			await cleanup("session.abort", () => session!.abort());

		if (session) await cleanup("session.disconnect", () => session!.disconnect());

		await cleanup("client.stop", async () => {
			for (const failure of await client.stop())
				cleanupErrors.push(cleanupFailure("client.stop", failure));
		});

		if (cleanupErrors.length) await cleanup("client.forceStop", () => client.forceStop());

		await cleanup("target.cleanup", () => launch.cleanup());

		if (cleanupErrors.length)
			error = new AdapterCleanupError("Copilot", cleanupErrors, result, error);
	}

	if (error) throw error;
	return result!;
}
