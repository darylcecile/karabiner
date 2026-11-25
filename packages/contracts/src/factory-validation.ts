import { TASK_STATUSES, type AgentDefinition, type ProjectConfig } from "./index.ts";
import type { FactoryIssue, FactoryValidation, WorkflowEdge, WorkflowGraph } from "./factory-types.ts";
import { inspectScopes } from "./factory-graph.ts";

export class FactoryCompileError extends Error {
  constructor(readonly issues: FactoryIssue[]) {
    super(issues.map((issue) => `${issue.path}: ${issue.message}`).join("; "));
    this.name = "FactoryCompileError";
  }
}

function fail(path: string, message: string, code = "invalid_config"): never {
  throw new FactoryCompileError([{ code, path, message }]);
}
function record(value: unknown, path: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail(path, "must be an object");
  return value as Record<string, unknown>;
}
function list(value: unknown, path: string): unknown[] {
  if (!Array.isArray(value)) fail(path, "must be an array");
  if (value.length > 1000) fail(path, "must contain at most 1000 entries");
  return value;
}
function text(value: unknown, path: string, nonempty = false): string {
  if (typeof value !== "string" || value.length > 65_536 || (nonempty && !value.trim())) fail(path, "must be a string" + (nonempty ? " with content" : ""));
  return value;
}
function integer(value: unknown, path: string, minimum: number, maximum: number): void {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < minimum || value > maximum) fail(path, `must be an integer from ${minimum} to ${maximum}`);
}
function choice(value: unknown, path: string, choices: readonly string[]): void {
  if (typeof value !== "string" || !choices.includes(value)) fail(path, `must be one of: ${choices.join(", ")}`);
}
function ids(values: unknown[], path: string): Set<string> {
  const result = new Set<string>();
  values.forEach((value, index) => {
    const id = text(record(value, `${path}[${index}]`).id, `${path}[${index}].id`, true);
    if (id.length > 256) fail(path, "IDs must be at most 256 characters");
    if (result.has(id)) fail(path, `duplicate ID ${id}`, "duplicate_id");
    result.add(id);
  });
  return result;
}
function references(value: unknown, allowed: Set<string>, path: string): void {
  const seen = new Set<string>();
  for (const item of list(value, path)) {
    const id = text(item, path, true);
    if (!allowed.has(id)) fail(path, `unknown reference ${id}`, "dangling_reference");
    if (seen.has(id)) fail(path, `duplicate reference ${id}`, "duplicate_reference");
    seen.add(id);
  }
}
function optionalReferences(value: unknown, allowed: Set<string>, path: string): void {
  if (value !== undefined) references(value, allowed, path);
}
function source(value: unknown, path: string): void {
  const item = record(value, path);
  text(item.name, `${path}.name`, true);
  text(item.content, `${path}.content`);
  if (item.path !== null) text(item.path, `${path}.path`, true);
  if (item.sourcePath !== undefined) text(item.sourcePath, `${path}.sourcePath`, true);
}
function command(value: unknown, path: string): void {
  const item = record(value, path);
  text(item.id, `${path}.id`, true);
  text(item.executable, `${path}.executable`, true);
  for (const arg of list(item.args, `${path}.args`)) text(arg, `${path}.args`);
  if (item.name !== undefined) text(item.name, `${path}.name`);
  if (item.cwd !== undefined) text(item.cwd, `${path}.cwd`, true);
  if (item.timeoutMs !== undefined) integer(item.timeoutMs, `${path}.timeoutMs`, 1, 86_400_000);
  retries(item, path);
  if (item.env !== undefined) for (const [key, value] of Object.entries(record(item.env, `${path}.env`))) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) fail(`${path}.env`, `invalid environment name ${key}`);
    text(value, `${path}.env.${key}`);
  }
}
function retries(item: Record<string, unknown>, path: string): void {
  if (item.retries !== undefined) integer(item.retries, `${path}.retries`, 0, 5);
  if (item.retryDelayMs !== undefined) integer(item.retryDelayMs, `${path}.retryDelayMs`, 0, 60_000);
}
function execution(value: unknown, path: string): void {
  const item = record(value, path);
  if (item.timeoutMs !== undefined) integer(item.timeoutMs, `${path}.timeoutMs`, 1, 86_400_000);
  retries(item, path);
  if (item.cwd !== undefined) text(item.cwd, `${path}.cwd`, true);
  if (item.executionTarget !== undefined) choice(item.executionTarget, `${path}.executionTarget`, ["host", "mxc", "apple-container"]);
  if (item.targetOptions !== undefined) targetOptions(item.targetOptions, `${path}.targetOptions`);
  for (const key of ["before", "commands", "after"] as const) if (item[key] !== undefined) {
    const commands = list(item[key], `${path}.${key}`);
    ids(commands, `${path}.${key}`);
    commands.forEach((value, index) => command(value, `${path}.${key}[${index}]`));
  }
  if (item.gates === undefined) return;
  const gates = list(item.gates, `${path}.gates`);
  ids(gates, `${path}.gates`);
  gates.forEach((value, index) => {
    const at = `${path}.gates[${index}]`;
    const gate = record(value, at);
    choice(gate.kind, `${at}.kind`, ["command", "approval"]);
    if (gate.kind === "command") command(gate.command, `${at}.command`);
    else text(gate.prompt, `${at}.prompt`, true);
    if (gate.name !== undefined) text(gate.name, `${at}.name`);
  });
}
function graph(value: unknown, stageIds: Set<string>, path: string): void {
  const item = record(value, path);
  references([item.entryStageId], stageIds, `${path}.entryStageId`);
  if (item.maxTransitions !== undefined) integer(item.maxTransitions, `${path}.maxTransitions`, 1, 10_000);
  const edges = list(item.edges, `${path}.edges`);
  ids(edges, `${path}.edges`);
  edges.forEach((value, index) => {
    const at = `${path}.edges[${index}]`;
    const edge = record(value, at);
    references([edge.sourceStageId], stageIds, `${at}.sourceStageId`);
    references([edge.targetStageId], stageIds, `${at}.targetStageId`);
    choice(edge.on, `${at}.on`, ["success", "failure", "return"]);
  });
  if (item.forks === undefined) return;
  const forks = list(item.forks, `${path}.forks`);
  ids(forks, `${path}.forks`);
  forks.forEach((value, index) => {
    const at = `${path}.forks[${index}]`;
    const fork = record(value, at);
    references([fork.stageId], stageIds, `${at}.stageId`);
    references([fork.joinStageId], stageIds, `${at}.joinStageId`);
    const branches = list(fork.branches, `${at}.branches`);
    if (branches.length > 16) fail(`${at}.branches`, "a fork supports at most 16 branches");
    ids(branches, `${at}.branches`);
    branches.forEach((value, index) => {
      const branch = record(value, `${at}.branches[${index}]`);
      references([branch.entryStageId], stageIds, `${at}.branches[${index}].entryStageId`);
      if (branch.entryStageId === fork.joinStageId) fail(`${at}.branches[${index}]`, "an empty branch cannot start at its join");
    });
  });
}
function layout(value: unknown, stageIds: Set<string>, path: string): void {
  const positions = record(record(value, path).positions, `${path}.positions`);
  for (const [id, value] of Object.entries(positions)) {
    if (!stageIds.has(id)) fail(path, `position references unknown stage ${id}`, "dangling_reference");
    const position = record(value, path);
    for (const axis of ["x", "y"]) if (typeof position[axis] !== "number" || !Number.isFinite(position[axis])) fail(path, `position ${axis} must be finite`);
  }
}
function workflow(value: unknown, path: string, agentIds: Set<string>, instructionIds: Set<string>): void {
  const item = record(value, path);
  text(item.name, `${path}.name`, true);
  text(item.description, `${path}.description`);
  text(item.instructions, `${path}.instructions`);
  optionalReferences(item.instructionIds, instructionIds, `${path}.instructionIds`);
  const stages = list(item.stages, `${path}.stages`);
  const stageIds = ids(stages, `${path}.stages`);
  stages.forEach((value, index) => {
    const at = `${path}.stages[${index}]`;
    const stage = record(value, at);
    text(stage.name, `${at}.name`, true);
    text(stage.instructions, `${at}.instructions`);
    references(stage.agentIds, agentIds, `${at}.agentIds`);
    references(stage.returnToStageIds, stageIds, `${at}.returnToStageIds`);
    optionalReferences(stage.instructionIds, instructionIds, `${at}.instructionIds`);
    if (stage.exitRule !== undefined) text(stage.exitRule, `${at}.exitRule`);
    if (stage.color !== undefined) text(stage.color, `${at}.color`);
    if (stage.status !== undefined) choice(stage.status, `${at}.status`, TASK_STATUSES);
    if (stage.maxVisits !== undefined) integer(stage.maxVisits, `${at}.maxVisits`, 1, 100);
    if (stage.kind !== undefined) choice(stage.kind, `${at}.kind`, ["stage", "fork", "join"]);
    if (stage.execution !== undefined) execution(stage.execution, `${at}.execution`);
  });
  if (item.graph !== undefined) graph(item.graph, stageIds, `${path}.graph`);
  if (item.layout !== undefined) layout(item.layout, stageIds, `${path}.layout`);
}
function targetOptions(value: unknown, path: string): void {
  const target = record(value, path);
  for (const key of ["image", "executable", "profilePath", "guestExecutable", "guestWorkdir", "memory"]) if (target[key] !== undefined) text(target[key], `${path}.${key}`, true);
  for (const key of ["network", "keychainAccess", "guestHarnessConfigured"]) if (target[key] !== undefined && typeof target[key] !== "boolean") fail(`${path}.${key}`, "must be a boolean");
  for (const key of ["writablePaths", "readonlyPaths", "envNames"]) if (target[key] !== undefined) list(target[key], `${path}.${key}`).forEach((item) => text(item, `${path}.${key}`, true));
  if (target.cpus !== undefined) integer(target.cpus, `${path}.cpus`, 1, 128);
}
function options(config: Record<string, unknown>, workflowIds: Set<string>): void {
  if (config.targetOptions !== undefined) targetOptions(config.targetOptions, "targetOptions");
  if (config.scheduler === undefined) return;
  const scheduler = record(config.scheduler, "scheduler");
  if (typeof scheduler.enabled !== "boolean") fail("scheduler.enabled", "must be a boolean");
  if (scheduler.workflowId !== undefined) references([scheduler.workflowId], workflowIds, "scheduler.workflowId");
  if (scheduler.statuses !== undefined) for (const status of list(scheduler.statuses, "scheduler.statuses")) choice(status, "scheduler.statuses", TASK_STATUSES);
  if (scheduler.maxConcurrentRuns !== undefined) integer(scheduler.maxConcurrentRuns, "scheduler.maxConcurrentRuns", 1, 32);
  if (scheduler.pollIntervalMs !== undefined) integer(scheduler.pollIntervalMs, "scheduler.pollIntervalMs", 250, 3_600_000);
}
export function validateStrandsModelConfig(value: unknown, path = "strands"): void {
  const config = record(value, path);
  choice(config.provider, `${path}.provider`, ["bedrock", "anthropic", "openai", "google", "ollama", "litellm"]);
  if (config.credentialEnv !== undefined && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(text(config.credentialEnv, `${path}.credentialEnv`, true))) fail(`${path}.credentialEnv`, "must name an environment variable");
  if (config.baseUrl !== undefined) {
    const raw = text(config.baseUrl, `${path}.baseUrl`, true);
    let url: URL;
    try { url = new URL(raw); } catch { fail(`${path}.baseUrl`, "must be an HTTP(S) endpoint"); }
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) fail(`${path}.baseUrl`, "must be an HTTP(S) endpoint without credentials, query or fragment");
  }
  if (config.region !== undefined) { if (config.provider !== "bedrock") fail(`${path}.region`, "region applies only to Bedrock"); text(config.region, `${path}.region`, true); }
  if (config.api !== undefined) { if (config.provider !== "openai") fail(`${path}.api`, "api applies only to OpenAI"); choice(config.api, `${path}.api`, ["chat", "responses"]); }
}

export function parseAgentDefinition(value: unknown, path = "agent"): AgentDefinition {
  const agent = record(value, path);
  if (text(agent.id, `${path}.id`, true).length > 256) fail(`${path}.id`, "must be at most 256 characters");
  for (const field of ["name", "role", "model", "instructions"]) text(agent[field], `${path}.${field}`, field === "name");
  choice(agent.harness, `${path}.harness`, ["claude", "codex", "copilot", "custom", "strands"]);
  choice(agent.mode, `${path}.mode`, ["interactive", "autonomous"]);
  list(agent.skillIds, `${path}.skillIds`).forEach((id) => text(id, `${path}.skillIds`, true));
  if (agent.instructionIds !== undefined) list(agent.instructionIds, `${path}.instructionIds`).forEach((id) => text(id, `${path}.instructionIds`, true));
  if (agent.executable !== undefined && agent.executable !== null) text(agent.executable, `${path}.executable`, true);
  if (agent.args !== undefined) list(agent.args, `${path}.args`).forEach((item) => text(item, `${path}.args`));
  if (agent.protocol !== undefined) choice(agent.protocol, `${path}.protocol`, ["jsonl-v1", "jsonrpc-v2"]);
  if (agent.strands !== undefined) validateStrandsModelConfig(agent.strands, `${path}.strands`);
  return value as AgentDefinition;
}

/** Shape validation preserves unknown fields. Semantic graph validation is shared below. */
export function parseProjectConfig(value: unknown): ProjectConfig {
  const config = record(value, "config");
  integer(config.revision, "revision", 0, Number.MAX_SAFE_INTEGER);
  choice(config.executionTarget, "executionTarget", ["host", "mxc", "apple-container"]);
  const instructions = list(config.instructions, "instructions");
  const skills = list(config.skills, "skills");
  const agents = list(config.agents, "agents");
  const workflows = list(config.workflows, "workflows");
  const instructionIds = ids(instructions, "instructions");
  const skillIds = ids(skills, "skills");
  const agentIds = ids(agents, "agents");
  const workflowIds = ids(workflows, "workflows");
  instructions.forEach((item, index) => source(item, `instructions[${index}]`));
  skills.forEach((item, index) => { source(item, `skills[${index}]`); text(record(item, "skill").description, `skills[${index}].description`); });
  agents.forEach((value, index) => {
    const path = `agents[${index}]`;
    const agent = parseAgentDefinition(value, path);
    references(agent.skillIds, skillIds, `${path}.skillIds`);
    optionalReferences(agent.instructionIds, instructionIds, `${path}.instructionIds`);
  });
  workflows.forEach((item, index) => workflow(item, `workflows[${index}]`, agentIds, instructionIds));
  options(config, workflowIds);
  return value as ProjectConfig;
}

export function normalizedGraph(workflow: ProjectConfig["workflows"][number]): WorkflowGraph {
  if (workflow.graph) return workflow.graph;
  const edges: WorkflowEdge[] = [];
  workflow.stages.forEach((stage, index) => {
    const next = workflow.stages[index + 1];
    if (next) edges.push({ id: `${stage.id}:success:${next.id}`, sourceStageId: stage.id, targetStageId: next.id, on: "success" });
    for (const target of stage.returnToStageIds) edges.push({ id: `${stage.id}:return:${target}`, sourceStageId: stage.id, targetStageId: target, on: "return" });
  });
  return { entryStageId: workflow.stages[0]?.id ?? "", edges, maxTransitions: 100 };
}
function reachable(start: string, edges: WorkflowEdge[]): Set<string> {
  const visited = new Set<string>();
  const pending = [start];
  while (pending.length) {
    const id = pending.pop()!;
    if (visited.has(id)) continue;
    visited.add(id);
    pending.push(...edges.filter((edge) => edge.sourceStageId === id).map((edge) => edge.targetStageId));
  }
  return visited;
}
function hasForwardCycle(stageIds: string[], edges: WorkflowEdge[]): boolean {
  const remaining = new Set(stageIds);
  const forward = edges.filter((edge) => edge.on !== "return");
  while (remaining.size) {
    const roots = [...remaining].filter((id) => !forward.some((edge) => edge.targetStageId === id && remaining.has(edge.sourceStageId)));
    if (!roots.length) return true;
    roots.forEach((id) => remaining.delete(id));
  }
  return false;
}
export function graphIssues(config: ProjectConfig, executable = true): FactoryIssue[] {
  const errors: FactoryIssue[] = [];
  const add = (code: string, path: string, message: string) => errors.push({ code, path, message });
  config.workflows.forEach((workflow, index) => {
    const path = `workflows[${index}]`;
    if (!workflow.stages.length) { if (executable) add("empty_workflow", path, "an executable workflow needs at least one stage"); return; }
    const graph = normalizedGraph(workflow);
    if (executable) graph.forks?.forEach((fork, forkIndex) => {
      if (fork.branches.length < 2) add("insufficient_branches", `${path}.graph.forks[${forkIndex}].branches`, `fork ${fork.id} needs at least two branches before execution`);
    });
    if (executable) workflow.stages.forEach((stage, stageIndex) => {
      if (stage.kind === "fork" && !graph.forks?.some((fork) => fork.stageId === stage.id)) add("unconfigured_fork", `${path}.stages[${stageIndex}].kind`, `fork stage ${stage.id} needs configured branches and a paired join`);
      if (stage.kind === "join" && !graph.forks?.some((fork) => fork.joinStageId === stage.id)) add("orphan_join", `${path}.stages[${stageIndex}].kind`, `join stage ${stage.id} must be paired with a fork`);
    });
    const forward = [...graph.edges, ...(graph.forks ?? []).flatMap((fork) => fork.branches.map((branch): WorkflowEdge => ({ id: `${fork.id}:${branch.id}`, sourceStageId: fork.stageId, targetStageId: branch.entryStageId, on: "success" })))];
    const seen = new Set<string>();
    for (const edge of graph.edges) {
      const key = JSON.stringify([edge.sourceStageId, edge.on, ...(edge.on === "return" ? [edge.targetStageId] : [])]);
      if (seen.has(key)) add("ambiguous_edge", `${path}.graph`, edge.on === "return" ? `${edge.sourceStageId} has duplicate return edges to ${edge.targetStageId}` : `${edge.sourceStageId} has more than one ${edge.on} successor`);
      seen.add(key);
      if (edge.on === "return" && !reachable(edge.targetStageId, forward.filter((item) => item.on !== "return")).has(edge.sourceStageId)) add("invalid_return", `${path}.graph`, `return ${edge.id} must target this stage or an earlier stage on a forward path`);
    }
    const reached = reachable(graph.entryStageId, forward);
    for (const stage of workflow.stages) {
      if (!reached.has(stage.id)) add("unreachable_stage", path, `stage ${stage.id} is unreachable from the entry stage`);
      if (workflow.graph) {
        const targets = graph.edges.filter((edge) => edge.sourceStageId === stage.id && edge.on === "return").map((edge) => edge.targetStageId).sort();
        if (JSON.stringify(targets) !== JSON.stringify([...stage.returnToStageIds].sort())) add("return_mismatch", path, `stage ${stage.id} returnToStageIds must match its graph return edges`);
      }
    }
    if (hasForwardCycle(workflow.stages.map((stage) => stage.id), forward)) add("unbounded_cycle", `${path}.graph`, "cycles must use an explicit return edge; success/failure edges must be acyclic");
    if (graph.forks?.length) errors.push(...inspectScopes(graph, `${path}.graph`).errors);
  });
  return errors;
}
export function validateFactory(value: unknown, options: { executable?: boolean } = {}): FactoryValidation {
  try {
    const config = parseProjectConfig(value);
    const errors = options.executable === false ? [] : graphIssues(config);
    return { valid: !errors.length, errors };
  } catch (error) {
    if (error instanceof FactoryCompileError) return { valid: false, errors: error.issues };
    throw error;
  }
}
