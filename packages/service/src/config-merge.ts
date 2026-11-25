import type {
	AgentDefinition,
	CommandDefinition,
	GateDefinition,
	ProjectConfig,
	StageDefinition,
	TargetOptions,
	WorkflowDefinition,
	WorkflowFork,
	WorkflowGraph,
} from "@karabiner/contracts";
import { mergeFields } from "./database.ts";

/** PUT replaces known values; fields this service does not understand retain v1 compatibility. */
function known<T extends object>(
	previous: T | undefined,
	incoming: T,
	fields: readonly string[],
): T {
	const before = (previous ?? {}) as Record<string, unknown>;
	const after = incoming as Record<string, unknown>;
	const result = { ...before, ...after };
	const names = new Set(fields);

	for (const key of Object.keys(result)) {
		if (names.has(key)) {
			if (!Object.hasOwn(after, key)) delete result[key];
		} else if (Object.hasOwn(after, key)) result[key] = mergeFields(before[key], after[key]);
	}

	return result as T;
}

function named<T extends { id: string }>(
	previous: T[] | undefined,
	incoming: T[],
	merge: (before: T | undefined, after: T) => T,
): T[] {
	const old = new Map((previous ?? []).map((item) => [item.id, item]));
	return incoming.map((item) => merge(old.get(item.id), item));
}

function target(previous: TargetOptions | undefined, incoming: TargetOptions): TargetOptions {
	return known(previous, incoming, [
		"image",
		"network",
		"writablePaths",
		"executable",
		"profilePath",
		"readonlyPaths",
		"keychainAccess",
		"guestExecutable",
		"guestWorkdir",
		"envNames",
		"cpus",
		"memory",
		"guestHarnessConfigured",
	]);
}

function command(
	previous: CommandDefinition | undefined,
	incoming: CommandDefinition,
): CommandDefinition {
	return known(previous, incoming, [
		"id",
		"name",
		"executable",
		"args",
		"cwd",
		"timeoutMs",
		"env",
		"retries",
		"retryDelayMs",
	]);
}

function gate(previous: GateDefinition | undefined, incoming: GateDefinition): GateDefinition {
	const result = known(previous, incoming, ["id", "name", "kind", "command", "prompt"]);

	if (result.kind === "command" && incoming.kind === "command")
		result.command = command(
			previous?.kind === "command" ? previous.command : undefined,
			incoming.command,
		);

	return result;
}

function stage(previous: StageDefinition | undefined, incoming: StageDefinition): StageDefinition {
	const result = known(previous, incoming, [
		"id",
		"name",
		"instructions",
		"agentIds",
		"returnToStageIds",
		"exitRule",
		"status",
		"color",
		"instructionIds",
		"execution",
		"maxVisits",
		"kind",
	]);

	if (incoming.execution) {
		const before = previous?.execution;
		const after = incoming.execution;

		result.execution = known(before, after, [
			"before",
			"commands",
			"gates",
			"after",
			"timeoutMs",
			"retries",
			"retryDelayMs",
			"cwd",
			"executionTarget",
			"targetOptions",
		]);

		for (const key of ["before", "commands", "after"] as const)
			if (after[key]) result.execution[key] = named(before?.[key], after[key], command);

		if (after.gates) result.execution.gates = named(before?.gates, after.gates, gate);

		if (after.targetOptions)
			result.execution.targetOptions = target(before?.targetOptions, after.targetOptions);
	}

	return result;
}

function fork(previous: WorkflowFork | undefined, incoming: WorkflowFork): WorkflowFork {
	const result = known(previous, incoming, ["id", "stageId", "branches", "joinStageId"]);

	result.branches = named(previous?.branches, incoming.branches, (before, after) =>
		known(before, after, ["id", "entryStageId"]),
	);

	return result;
}

function graph(previous: WorkflowGraph | undefined, incoming: WorkflowGraph): WorkflowGraph {
	const result = known(previous, incoming, ["entryStageId", "edges", "maxTransitions", "forks"]);

	result.edges = named(previous?.edges, incoming.edges, (before, after) =>
		known(before, after, ["id", "sourceStageId", "targetStageId", "on"]),
	);

	if (incoming.forks) result.forks = named(previous?.forks, incoming.forks, fork);
	return result;
}

function workflow(
	previous: WorkflowDefinition | undefined,
	incoming: WorkflowDefinition,
): WorkflowDefinition {
	const result = known(previous, incoming, [
		"id",
		"name",
		"description",
		"instructions",
		"stages",
		"instructionIds",
		"graph",
		"layout",
	]);

	result.stages = named(previous?.stages, incoming.stages, stage);
	if (incoming.graph) result.graph = graph(previous?.graph, incoming.graph);

	if (incoming.layout) {
		result.layout = known(previous?.layout, incoming.layout, ["positions"]);

		result.layout.positions = Object.fromEntries(
			Object.entries(incoming.layout.positions).map(([id, position]) => [
				id,
				known(previous?.layout?.positions[id], position, ["x", "y"]),
			]),
		);
	}

	return result;
}

function agent(previous: AgentDefinition | undefined, incoming: AgentDefinition): AgentDefinition {
	const result = known(previous, incoming, [
		"id",
		"name",
		"role",
		"harness",
		"model",
		"instructions",
		"skillIds",
		"mode",
		"executable",
		"args",
		"instructionIds",
		"protocol",
		"strands",
	]);

	if (incoming.strands)
		result.strands = known(previous?.strands, incoming.strands, [
			"provider",
			"credentialEnv",
			"baseUrl",
			"region",
			"api",
		]);

	return result;
}

export function mergeProjectConfig(
	previous: ProjectConfig,
	incoming: ProjectConfig,
): ProjectConfig {
	const result = known(previous, incoming, [
		"revision",
		"instructions",
		"skills",
		"agents",
		"workflows",
		"executionTarget",
		"targetOptions",
		"scheduler",
		"integration",
	]);

	result.instructions = named(previous.instructions, incoming.instructions, (before, after) =>
		known(before, after, ["id", "name", "content", "path", "sourcePath"]),
	);

	result.skills = named(previous.skills, incoming.skills, (before, after) =>
		known(before, after, ["id", "name", "description", "content", "path", "sourcePath"]),
	);

	result.agents = named(previous.agents, incoming.agents, agent);
	result.workflows = named(previous.workflows, incoming.workflows, workflow);

	if (incoming.targetOptions)
		result.targetOptions = target(previous.targetOptions, incoming.targetOptions);

	if (incoming.scheduler)
		result.scheduler = known(previous.scheduler, incoming.scheduler, [
			"enabled",
			"workflowId",
			"statuses",
			"maxConcurrentRuns",
			"pollIntervalMs",
		]);

	if (incoming.integration) {
		result.integration = known(previous.integration, incoming.integration, [
			"provider",
			"scope",
			"hostname",
			"pushEnabled",
			"pushFields",
			"statusFieldId",
			"pullStatusMap",
			"pushStatusMap",
			"apiKeyEnv",
			"teamId",
			"refresh",
		]);

		if (incoming.integration.refresh)
			result.integration.refresh = known(
				previous.integration?.refresh,
				incoming.integration.refresh,
				["enabled", "intervalMs"],
			);
	}

	return structuredClone(result);
}
