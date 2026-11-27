import type {
	AdapterCapabilities,
	AdapterInteraction,
	AdapterRequest,
	AgentOutcome,
	AdapterTool,
	AdapterField,
} from "./adapter-types.ts";
import {
	bridgePrompt,
	finalOutcome,
	interact,
	jsonValue,
	object,
	waitWithSignal,
} from "./bridge-common.ts";
import { RpcProcess } from "./rpc-process.ts";
import { AdapterCleanupError, cleanupFailure } from "./adapter-types.ts";

function interactionTool(value: unknown): AdapterTool | undefined {
	if (value === undefined) return;
	const tool = object(value);

	if (
		typeof tool.name !== "string" ||
		!["shell", "read", "write", "network", "mcp", "other"].includes(String(tool.kind))
	)
		throw new Error("Invalid custom interaction tool");

	for (const key of ["command", "cwd", "diff"] as const)
		if (tool[key] !== undefined && typeof tool[key] !== "string")
			throw new Error(`Invalid custom tool ${key}`);

	for (const key of ["paths", "urls"] as const)
		if (
			tool[key] !== undefined &&
			(!Array.isArray(tool[key]) || !tool[key].every((item) => typeof item === "string"))
		)
			throw new Error(`Invalid custom tool ${key}`);

	return tool as unknown as AdapterTool;
}

function interactionFields(value: unknown): AdapterField[] | undefined {
	if (value === undefined) return;
	if (!Array.isArray(value)) throw new Error("Custom interaction fields must be an array");

	for (const raw of value) {
		const field = object(raw);

		if (
			typeof field.id !== "string" ||
			typeof field.label !== "string" ||
			!["text", "select", "multiselect", "boolean", "number", "integer"].includes(
				String(field.type),
			)
		)
			throw new Error("Invalid custom interaction field");

		if (field.required !== undefined && typeof field.required !== "boolean")
			throw new Error("Invalid custom field requirement");

		if (
			field.options !== undefined &&
			(!Array.isArray(field.options) ||
				!field.options.every(
					(option) =>
						typeof object(option).id === "string" &&
						typeof object(option).label === "string",
				))
		)
			throw new Error("Invalid custom field options");
	}

	return value as AdapterField[];
}

/** Explicit karabiner-harness-v2 JSONL JSON-RPC protocol; legacy stdin EOF semantics stay intact. */
export async function executeCustomBridge(request: AdapterRequest): Promise<AgentOutcome> {
	if (!request.agent.executable) throw new Error("Custom harness requires an executable");
	const launch = await request.launch(request.agent.executable, request.agent.args ?? []);
	const transport = new AbortController();
	const rpc = new RpcProcess(launch, transport.signal);
	const signal = AbortSignal.any([request.signal, AbortSignal.timeout(request.timeoutMs)]);
	let sessionId = request.sessionId;
	let unregister: (() => void) | undefined;
	let completed: AgentOutcome | undefined;
	let failure: unknown;

	rpc.onNotification = (method, params) => {
		if (method === "text" && typeof params.text === "string")
			request.emit({ kind: "text", text: params.text });

		if ((method === "reasoning" || method === "error") && typeof params.text === "string")
			request.emit({ kind: method, text: params.text });

		if (method === "tool")
			request.emit({
				kind: "tool",
				text: String(params.name ?? "Tool"),
				name: typeof params.name === "string" ? params.name : undefined,
				toolId: typeof params.id === "string" ? params.id : undefined,
				input: params.input,
				output: params.output,
				status:
					params.status === "completed"
						? "completed"
						: params.status === "failed"
							? "failed"
							: "running",
			});
	};

	rpc.onRequest = async (method, params, id, nativeSignal) => {
		if (method !== "permission" && method !== "question")
			throw new Error(`Unknown custom harness request ${method}`);

		const choices = (Array.isArray(params.choices) ? params.choices : []).map((value) => {
			const choice = object(value);

			if (typeof choice.id !== "string" || typeof choice.label !== "string")
				throw new Error("Invalid custom interaction choice");

			if (
				method === "permission" &&
				!["approve", "deny", "cancel"].includes(String(choice.effect))
			)
				throw new Error(
					"Custom permission choices must declare approve/deny/cancel effects",
				);

			return {
				id: choice.id,
				label: choice.label,
				...(choice.effect
					? { effect: choice.effect as "approve" | "deny" | "cancel" }
					: {}),
			};
		});

		if (method === "permission" && !choices.length)
			throw new Error("Custom permissions require explicit decision choices");

		const input: AdapterInteraction = {
			id,
			provider: "custom",
			kind: method,
			prompt: String(params.prompt ?? "Input required"),
			choices,
			payload: jsonValue(params.payload),
			tool: interactionTool(params.tool),
			fields: interactionFields(params.fields),
		};

		return interact(request, input, AbortSignal.any([signal, nativeSignal]));
	};

	const abort = async () => {
		await rpc.call("abort", {
			sessionId,
			stepId: request.stepId,
			attemptId: request.attemptId,
		});
	};

	try {
		const hello = await waitWithSignal(
			rpc.call("initialize", {
				protocol: "karabiner-harness-v2",
				runId: request.runId,
				stepId: request.stepId,
				attemptId: request.attemptId,
				sessionId,
			}),
			signal,
		);

		if (hello.protocol !== "karabiner-harness-v2" || typeof hello.sessionId !== "string")
			throw new Error(
				"Custom harness did not negotiate karabiner-harness-v2 and a session identity",
			);

		sessionId = hello.sessionId;
		await request.onSession?.(sessionId);
		const raw = object(hello.capabilities);

		const steering =
			raw.steering === "native-turn" || raw.steering === "native-tool-boundary"
				? raw.steering
				: "next-step";

		const capabilities: AdapterCapabilities = {
			permissions: raw.permissions === true,
			questions: raw.questions === true,
			steering,
		};

		unregister = request.registerControl?.({
			capabilities,
			abort,
			steer: async ({ id, message }) => {
				if (steering === "next-step") return { mode: steering, delivered: false };

				const response = await rpc.call("instruction", {
					id,
					message,
					sessionId,
					attemptId: request.attemptId,
				});

				return { mode: steering, delivered: response.delivered === true };
			},
		});

		const result = await waitWithSignal(
			rpc.call("run", {
				sessionId,
				runId: request.runId,
				stepId: request.stepId,
				attemptId: request.attemptId,
				prompt: bridgePrompt(request),
				agent: request.agent,
				answer: request.answer,
			}),
			signal,
		);

		completed = { ...finalOutcome(result), sessionId };
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
				"Custom harness",
				[cleanupFailure("transport.close", error)],
				completed,
				failure,
			);
		}
	}

	if (failure) throw failure;
	return completed!;
}
