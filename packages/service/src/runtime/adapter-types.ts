import type {
	AgentDefinition,
	Json,
	SkillDefinition,
	ProjectConfig,
	TargetOptions,
} from "@karabiner/contracts";
import type { ProcessRequest } from "./process.ts";

export interface AgentOutcome {
	outcome: "complete" | "return" | "blocked" | "question";
	summary: string;
	returnToStageId?: string;
	question?: string;
	requestId?: string;
	sessionId?: string;
	usage?: { inputTokens: number; outputTokens: number; totalTokens: number };
	artifacts?: { title: string; path: string }[];
}

export interface AdapterCleanupFailure {
	phase: string;
	message: string;
}

export function cleanupFailure(phase: string, error: unknown): AdapterCleanupFailure {
	const message =
		error instanceof AggregateError
			? `${error.message}: ${error.errors.map((cause) => (cause instanceof Error ? cause.message : String(cause))).join("; ")}`
			: error instanceof Error
				? error.message
				: String(error);

	return { phase, message: message.slice(0, 4_000) };
}

/** A resource-release failure never authorizes replaying completed or uncertain effects. */
export class AdapterCleanupError extends Error {
	readonly code = "adapter_cleanup_failed";
	readonly retryable = false;
	readonly details: Json;
	readonly completed?: AgentOutcome;
	constructor(
		provider: string,
		failures: AdapterCleanupFailure[],
		completed?: AgentOutcome,
		executionError?: unknown,
	) {
		super(
			`${provider} cleanup failed: ${failures.map((failure) => `${failure.phase}: ${failure.message}`).join("; ")}`,
			{ cause: executionError },
		);

		this.name = "AdapterCleanupError";
		this.completed = completed ? structuredClone(completed) : undefined;

		this.details = JSON.parse(
			JSON.stringify({
				provider,
				failures,
				completed: this.completed,
				...(executionError
					? {
							executionError:
								executionError instanceof Error
									? executionError.message
									: String(executionError),
						}
					: {}),
			}),
		) as Json;
	}
}

export interface AdapterEvent {
	kind: "text" | "reasoning" | "tool" | "activity" | "error";
	text: string;
	toolId?: string;
	name?: string;
	input?: unknown;
	output?: unknown;
	outputDelta?: string;
	/** Provider block identity for coalescing interleaved streams; not a tool identity. */
	messageId?: string;
	/** Stream deltas append; complete messages establish a new message boundary. */
	delta?: boolean;
	status?: "running" | "completed" | "failed";
}

export interface AdapterCapabilities {
	permissions: boolean;
	questions: boolean;
	steering: "native-turn" | "native-tool-boundary" | "next-step";
}

export interface AdapterInteraction {
	id: string;
	kind: "question" | "permission";
	provider: string;
	prompt: string;
	choices?: { id: string; label: string; effect?: "approve" | "deny" | "cancel" }[];
	payload?: Json;
	tool?: AdapterTool;
	fields?: AdapterField[];
}

export interface AdapterTool {
	name: string;
	kind: "shell" | "read" | "write" | "network" | "mcp" | "other";
	command?: string;
	cwd?: string;
	paths?: string[];
	diff?: string;
	urls?: string[];
}

export interface AdapterField {
	id: string;
	label: string;
	type: "text" | "select" | "multiselect" | "boolean" | "number" | "integer";
	options?: { id: string; label: string }[];
	required?: boolean;
	allowOther?: boolean;
	minLength?: number;
	maxLength?: number;
	minItems?: number;
	maxItems?: number;
	minimum?: number;
	maximum?: number;
}

export interface AdapterAnswer {
	choice?: string;
	answer?: string;
	answers?: Record<string, string | string[]>;
	value?: Json;
}

export interface AdapterDelivery {
	mode: AdapterCapabilities["steering"];
	delivered: boolean;
}

export interface AdapterControl {
	capabilities: AdapterCapabilities;
	steer(instruction: { id: string; message: string }): Promise<AdapterDelivery>;
	abort(): Promise<void>;
}

/** Target code owns launch translation; the process runner owns cancellation and I/O. */
export type Launch = (
	command: string,
	args: string[],
	cwd?: string,
	env?: Record<string, string>,
) => Promise<
	Pick<ProcessRequest, "command" | "args" | "cwd" | "env" | "extraInput"> & {
		cleanup?: () => Promise<void>;
	}
>;

export interface AdapterRequest {
	agent: AgentDefinition;
	prompt: string;
	instructions: string;
	/** Immutable factory layer, before cumulative operator guidance is appended. */
	factoryInstructions?: string;
	operatorInstructions?: string[];
	skills: SkillDefinition[];
	rootPath: string;
	executionTarget?: ProjectConfig["executionTarget"];
	targetOptions?: TargetOptions;
	runId: string;
	stepId: string;
	attemptId?: string;
	sessionId?: string;
	answer?: { requestId: string; response: Json };
	signal: AbortSignal;
	timeoutMs: number;
	launch: Launch;
	emit: (event: AdapterEvent) => void;
	interact?: (request: AdapterInteraction, signal?: AbortSignal) => Promise<AdapterAnswer>;
	onSession?: (sessionId: string) => void | Promise<void>;
	registerControl?: (control: AdapterControl) => () => void;
	storage?: {
		write(key: string, value: Uint8Array): Promise<void>;
		read(key: string): Promise<Uint8Array | null>;
		delete(key: string): Promise<void>;
		list(prefix: string): Promise<string[]>;
	};
}

export const outcomeSchema = {
	type: "object",
	properties: {
		outcome: { type: "string", enum: ["complete", "return", "blocked", "question"] },
		summary: { type: "string" },
		returnToStageId: { type: ["string", "null"] },
		question: { type: ["string", "null"] },
	},
	required: ["outcome", "summary", "returnToStageId", "question"],
	additionalProperties: false,
} as const;

export function parseOutcome(value: unknown): AgentOutcome | undefined {
	if (!value || typeof value !== "object") return;
	const object = value as Record<string, unknown>;

	if (
		!["complete", "return", "blocked", "question"].includes(String(object.outcome)) ||
		typeof object.summary !== "string"
	)
		return;

	if (object.outcome === "return" && typeof object.returnToStageId !== "string") return;
	if (object.outcome === "question" && typeof object.question !== "string") return;

	return {
		outcome: object.outcome as AgentOutcome["outcome"],
		summary: object.summary,
		...(typeof object.returnToStageId === "string" && {
			returnToStageId: object.returnToStageId,
		}),
		...(typeof object.question === "string" && { question: object.question }),
	};
}

export function resultInstructions(interactive = false): string {
	return `Return a JSON object describing this step: ${JSON.stringify(outcomeSchema)}. Use complete only when the requested work is finished, blocked for a concrete blocker, return only for a supplied permitted stage ID, or question when human input is required. Do not invent approval or command results. ${interactive ? "Native permission and user-input requests are handled by the runtime; await the actual response." : "This legacy JSONL protocol requires a result followed by process exit; return a question result instead of waiting for terminal input."}`;
}
