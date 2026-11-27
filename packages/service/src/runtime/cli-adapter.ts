import {
	outcomeSchema,
	parseOutcome,
	resultInstructions,
	AdapterCleanupError,
	cleanupFailure,
	type AdapterRequest,
	type AgentOutcome,
} from "./adapter-types.ts";
import { runProcess } from "./process.ts";
import { executeClaude } from "./claude-bridge.ts";
import { executeCodex } from "./codex-bridge.ts";
import { executeCopilot } from "./copilot-bridge.ts";
import { executeCustomBridge } from "./custom-bridge.ts";

function json(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return undefined;
	}
}

function record(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function outcomeText(text: string): AgentOutcome | undefined {
	const trimmed = text
		.trim()
		.replace(/^```(?:json)?\s*/, "")
		.replace(/\s*```$/, "");

	return parseOutcome(json(trimmed));
}

interface Capture {
	result?: AgentOutcome;
	sessionId?: string;
	finalText?: string;
	error?: string;
	usage?: AgentOutcome["usage"];
}

function captureUsage(value: unknown): AgentOutcome["usage"] {
	const usage = record(value);
	const input = usage.input_tokens ?? usage.inputTokens;
	const output = usage.output_tokens ?? usage.outputTokens;
	if (typeof input !== "number" || typeof output !== "number") return;
	return { inputTokens: input, outputTokens: output, totalTokens: input + output };
}

function consume(line: string, capture: Capture, request: AdapterRequest) {
	const event = record(json(line));
	const data = record(event.data);
	const type = String(event.type ?? "");

	if (
		[
			"control_request",
			"permission.request",
			"permission.requested",
			"user_input.requested",
		].includes(type)
	)
		throw new Error(
			"Harness requested interactive permission or terminal input. This adapter cannot answer native permission prompts; configure unattended permissions or select an interactive Strands agent.",
		);

	if (typeof event.session_id === "string") capture.sessionId = event.session_id;
	if (typeof event.thread_id === "string") capture.sessionId = event.thread_id;

	if (type === "session.start" && typeof data.sessionId === "string")
		capture.sessionId = data.sessionId;

	const usage = captureUsage(event.usage ?? data.usage);
	if (usage) capture.usage = usage;

	if (type === "error" || type === "turn.failed" || type === "session.error") {
		capture.error = String(
			event.message ??
				data.message ??
				record(event.error).message ??
				"Harness execution failed",
		);
	}

	if (type === "result") {
		if (event.is_error === true)
			capture.error = String(event.result ?? "Harness execution failed");

		capture.result = parseOutcome(event.structured_output) ?? parseOutcome(event.result);
		if (typeof event.result === "string") capture.finalText = event.result;
	}

	if (type === "question") {
		if (typeof event.question !== "string")
			throw new Error("Custom harness emitted an invalid question");

		capture.result = { outcome: "question", summary: event.question, question: event.question };
	}

	const item = record(event.item);

	if (item.type === "agent_message" && typeof item.text === "string")
		capture.finalText = item.text;

	if (type === "assistant.message" && typeof data.content === "string")
		capture.finalText = data.content;

	if (type === "assistant") {
		const content = record(event.message).content;

		if (Array.isArray(content)) {
			const text = content
				.map(record)
				.filter((block) => block.type === "text")
				.map((block) => block.text)
				.join("");

			if (text) capture.finalText = text;
		}
	}

	if (type === "text" && typeof event.text === "string")
		request.emit({ kind: "text", text: event.text });

	if (type === "reasoning" && typeof event.text === "string")
		request.emit({ kind: "reasoning", text: event.text });

	if (type === "tool")
		request.emit({
			kind: "tool",
			text: String(event.name ?? event.text ?? "Tool"),
			name: typeof event.name === "string" ? event.name : undefined,
			toolId:
				typeof (event.toolId ?? event.id) === "string"
					? String(event.toolId ?? event.id)
					: undefined,
			status:
				event.status === "failed"
					? "failed"
					: event.status === "completed"
						? "completed"
						: "running",
			input: event.input,
			output: event.output,
		});

	if (type === "assistant.message_delta" && typeof data.deltaContent === "string")
		request.emit({ kind: "text", text: data.deltaContent });

	if (
		item.type === "command_execution" ||
		item.type === "mcp_tool_call" ||
		type.startsWith("tool.execution_")
	) {
		request.emit({
			kind: "tool",
			text: String(item.command ?? data.toolName ?? item.tool ?? "Tool execution"),
			toolId:
				typeof (item.id ?? data.toolCallId) === "string"
					? String(item.id ?? data.toolCallId)
					: undefined,
			status:
				data.success === false || item.status === "failed"
					? "failed"
					: type.endsWith("completed") || type.endsWith("complete")
						? "completed"
						: "running",
			input: item.arguments ?? data.arguments ?? item.command,
			output: item.aggregated_output ?? data.result,
		});
	}
}

function command(request: AdapterRequest): { executable: string; args: string[]; input: string } {
	const { agent } = request;

	const instructions = [
		request.instructions,
		...request.skills.map((skill) => `# Skill: ${skill.name}\n${skill.content}`),
		resultInstructions(),
	].join("\n\n");

	const extra = agent.args ?? [];

	if (!agent.executable) throw new Error("Custom harness requires an executable path");

	return {
		executable: agent.executable,
		args: extra,
		input: `${JSON.stringify({ protocol: "karabiner-harness-v1", runId: request.runId, stepId: request.stepId, sessionId: request.sessionId, agent, instructions, prompt: request.prompt, answer: request.answer, responseSchema: outcomeSchema })}\n`,
	};
}

/** Uses the installed executable's own account. No tokens or provider credentials are extracted. */
export async function executeCli(request: AdapterRequest): Promise<AgentOutcome> {
	if (request.agent.harness === "claude") return executeClaude(request);
	if (request.agent.harness === "codex") return executeCodex(request);
	if (request.agent.harness === "copilot") return executeCopilot(request);
	if (request.agent.protocol === "jsonrpc-v2") return executeCustomBridge(request);
	const spec = command(request);
	const launch = await request.launch(spec.executable, spec.args);
	const capture: Capture = {};
	let completed: AgentOutcome | undefined;
	let failure: unknown;

	try {
		const result = await runProcess({
			...launch,
			input: spec.input,
			signal: request.signal,
			timeoutMs: request.timeoutMs,
			onLine: (line, stream) => {
				if (stream === "stdout") consume(line, capture, request);
			},
		});

		if (capture.error) throw new Error(capture.error);

		if (result.code !== 0)
			throw new Error(
				`Harness exited ${result.code}: ${result.stderr.slice(-4_000) || capture.finalText || "No diagnostic output"}`,
			);

		const outcome = capture.result ?? outcomeText(capture.finalText ?? result.stdout);

		if (!outcome)
			throw new Error(
				"Legacy custom harness ended without a valid result. Return one JSONL result and exit, or select the jsonrpc-v2 protocol for interactive requests.",
			);

		if (outcome.outcome === "question" && request.agent.mode !== "interactive")
			throw new Error(
				"An autonomous agent requested human input; select interactive mode for this agent",
			);

		completed = {
			...outcome,
			sessionId: capture.sessionId ?? request.sessionId,
			usage: capture.usage,
		};
	} catch (error) {
		failure = error;
	} finally {
		try {
			await launch.cleanup?.();
		} catch (error) {
			failure = new AdapterCleanupError(
				"Legacy custom harness",
				[cleanupFailure("target.cleanup", error)],
				completed,
				failure,
			);
		}
	}

	if (failure) throw failure;
	return completed!;
}
