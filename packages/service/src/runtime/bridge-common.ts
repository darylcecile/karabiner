import type { Json } from "@karabiner/contracts";
import type {
	AdapterAnswer,
	AdapterDelivery,
	AdapterInteraction,
	AdapterRequest,
	AgentOutcome,
} from "./adapter-types.ts";
import {
	parseOutcome,
	resultInstructions,
	type AdapterField,
	type AdapterTool,
} from "./adapter-types.ts";

export function object(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

export function jsonValue(value: unknown): Json {
	return JSON.parse(JSON.stringify(value ?? null)) as Json;
}

export function finalOutcome(value: unknown): AgentOutcome {
	let parsed = value;

	if (typeof value === "string") {
		try {
			parsed = JSON.parse(
				value
					.trim()
					.replace(/^```(?:json)?\s*/, "")
					.replace(/\s*```$/, ""),
			);
		} catch {
			throw new Error("Harness ended without a valid structured factory result");
		}
	}

	const result = parseOutcome(parsed);
	if (!result) throw new Error("Harness ended without a valid structured factory result");
	return result;
}

export function waitWithSignal<T>(work: Promise<T>, signal: AbortSignal): Promise<T> {
	if (signal.aborted) return Promise.reject(signal.reason);

	return new Promise((resolve, reject) => {
		const abort = () => reject(signal.reason);
		signal.addEventListener("abort", abort, { once: true });
		work.then(resolve, reject).finally(() => signal.removeEventListener("abort", abort));
	});
}

export async function interact(
	request: AdapterRequest,
	input: AdapterInteraction,
	signal = request.signal,
): Promise<AdapterAnswer> {
	const activeSignal = AbortSignal.any([request.signal, signal]);
	activeSignal.throwIfAborted();

	if (input.kind === "question" && request.agent.mode !== "interactive")
		throw new Error(
			"This agent is autonomous; agent-origin questions require interactive mode",
		);

	if (input.kind === "permission" && request.agent.mode === "autonomous") {
		if (input.fields?.length)
			throw new Error("Permission requests needing form input require interactive mode");

		const approval = input.choices?.find((choice) => choice.effect === "approve");
		if (!approval) throw new Error(`${input.provider} did not offer a permission approval`);

		request.emit({
			kind: "activity",
			text: `Autonomous mode approved ${input.provider} permission: ${input.prompt}`,
			toolId: input.id,
		});

		return { choice: approval.id };
	}

	if (!request.interact)
		throw new Error(`${input.provider} needs the runtime interaction bridge for ${input.kind}`);

	return waitWithSignal(request.interact(input, activeSignal), activeSignal);
}

/** Delivery resolves only on a provider acknowledgement; closing leaves unconsumed guidance pending. */
export class DeliveryReceipts {
	private pending = new Map<string, (receipt: AdapterDelivery) => void>();
	get size() {
		return this.pending.size;
	}
	wait(id: string): Promise<AdapterDelivery> {
		return new Promise((resolve) => this.pending.set(id, resolve));
	}
	ack(id: string, mode: AdapterDelivery["mode"]) {
		this.pending.get(id)?.({ mode, delivered: true });
		this.pending.delete(id);
	}
	close() {
		for (const resolve of this.pending.values())
			resolve({ mode: "next-step", delivered: false });

		this.pending.clear();
	}
}

export function bridgePrompt(request: AdapterRequest): string {
	return [
		request.instructions,
		...request.skills.map((skill) => `# Skill: ${skill.name}\n${skill.content}`),
		resultInstructions(true),
		request.agent.mode === "autonomous"
			? "You are running autonomously. Tool permission requests are approved by the configured agent mode. Do not ask the user questions or request form input. If essential information is unavailable, report a concrete blocked result instead of waiting for human input."
			: "",
		request.prompt,
	]
		.filter(Boolean)
		.join("\n\n");
}

export function nativeCwd(request: AdapterRequest): string {
	return request.executionTarget === "apple-container"
		? (request.targetOptions?.guestWorkdir ?? "/workspace")
		: request.rootPath;
}

export function formFields(schema: unknown): AdapterField[] {
	const value = object(schema);

	return Object.entries(object(value.properties)).map(([id, raw]) => {
		const field = object(raw);
		const source = field.type === "array" ? object(field.items) : field;
		const alternatives = source.oneOf ?? source.anyOf;
		const names = Array.isArray(field.enumNames) ? field.enumNames : [];

		const options = Array.isArray(source.enum)
			? source.enum.map((choice, index) => ({
					id: String(choice),
					label: String(names[index] ?? choice),
				}))
			: Array.isArray(alternatives)
				? alternatives
						.map(object)
						.filter((choice) => choice.const !== undefined)
						.map((choice) => ({
							id: String(choice.const),
							label: String(choice.title ?? choice.const),
						}))
				: undefined;

		const constraints = Object.fromEntries(
			["minLength", "maxLength", "minItems", "maxItems", "minimum", "maximum"]
				.filter((key) => typeof field[key] === "number" && Number.isFinite(field[key]))
				.map((key) => [key, field[key]]),
		);

		return {
			id,
			label: String(field.title ?? field.description ?? id),
			type:
				field.type === "boolean"
					? "boolean"
					: field.type === "number" || field.type === "integer"
						? field.type
						: field.type === "array"
							? "multiselect"
							: options
								? "select"
								: "text",
			options,
			required: Array.isArray(value.required) && value.required.includes(id),
			allowOther: field.allowOther === true,
			...constraints,
		};
	});
}

export function fieldAnswer(response: AdapterAnswer, id: string): string | string[] {
	const value =
		response.answers?.[id] ?? object(response.value)[id] ?? response.answer ?? response.choice;

	if (
		typeof value === "string" ||
		(Array.isArray(value) && value.every((item) => typeof item === "string"))
	)
		return value;

	throw new Error("A question answer is required");
}

export function toolDetails(name: string, raw: Record<string, unknown>): AdapterTool {
	const input = object(raw.input ?? raw.arguments ?? raw);
	const kind = String(raw.kind ?? name).toLowerCase();
	const command = raw.fullCommandText ?? raw.command ?? input.command;
	const path = raw.fileName ?? raw.path ?? raw.grantRoot ?? input.file_path ?? input.path;
	const url = raw.url ?? object(raw.networkApprovalContext).host ?? input.url;
	const changes = (Array.isArray(raw.changes) ? raw.changes : []).map(object);

	const paths =
		typeof path === "string"
			? [path]
			: changes.map((change) => String(change.path ?? "")).filter(Boolean);

	const diff =
		typeof raw.diff === "string"
			? raw.diff
			: changes
					.map((change) => change.diff)
					.filter((value): value is string => typeof value === "string")
					.join("\n");

	return {
		name,
		kind: raw.networkApprovalContext
			? "network"
			: /bash|shell|command/.test(kind)
				? "shell"
				: /write|edit|filechange/.test(kind)
					? "write"
					: /read/.test(kind)
						? "read"
						: /url|network|fetch/.test(kind)
							? "network"
							: /mcp/.test(kind)
								? "mcp"
								: "other",
		...(typeof command === "string" && { command }),
		...(typeof raw.cwd === "string" && { cwd: raw.cwd }),
		...(paths.length && { paths }),
		...(diff && { diff }),
		...(typeof url === "string" && { urls: [url] }),
	};
}

export class InputQueue<T> implements AsyncIterable<T> {
	private values: T[] = [];
	private wake?: () => void;
	private ended = false;
	push(value: T) {
		if (this.ended) throw new Error("Harness input is closed");
		this.values.push(value);
		this.wake?.();
	}
	close() {
		this.ended = true;
		this.wake?.();
	}
	async *[Symbol.asyncIterator]() {
		while (!this.ended || this.values.length) {
			if (this.values.length) yield this.values.shift()!;
			else
				await new Promise<void>((resolve) => {
					this.wake = resolve;
				});
		}
	}
}
