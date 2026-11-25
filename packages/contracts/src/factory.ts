import type { ProjectConfig } from "./index.ts";
import type { FactoryBundle, FactoryDefinition, ResolvedAgent } from "./factory-types.ts";
import { FactoryCompileError, graphIssues, normalizedGraph, parseProjectConfig } from "./factory-validation.ts";
import { inspectScopes } from "./factory-graph.ts";
export { FactoryCompileError, parseProjectConfig, parseAgentDefinition, validateFactory, normalizedGraph, validateStrandsModelConfig } from "./factory-validation.ts";

/** Stable serialization is also used by the export; property insertion order is irrelevant. */
export function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") return `{${Object.entries(value).filter(([, value]) => value !== undefined).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([key, value]) => `${JSON.stringify(key)}:${canonicalJson(value)}`).join(",")}}`;
  return JSON.stringify(value) ?? "null";
}
function fingerprint(value: unknown): string {
  // A stable identity, not a cryptographic signature or an authorization boundary.
  let hash = 0xcbf29ce484222325n;
  for (const byte of new TextEncoder().encode(canonicalJson(value))) hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 0x100000001b3n);
  return `factory-${hash.toString(16).padStart(16, "0")}`;
}
function freeze<T>(value: T): T {
  if (value && typeof value === "object") {
    Object.values(value).forEach(freeze);
    Object.freeze(value);
  }
  return value;
}
function instructions(config: ProjectConfig, ids: string[] = [], inline = ""): string {
  return [...ids.map((id) => config.instructions.find((item) => item.id === id)!.content), inline].filter(Boolean).join("\n\n");
}
function inlineSources(config: ProjectConfig): void {
  for (const source of [...config.instructions, ...config.skills]) {
    if (source.path) source.sourcePath = source.path;
    source.path = null;
  }
}

/** No clock, filesystem, PATH, executable import, or mutation is consulted here. */
export function compileFactory(value: unknown): FactoryDefinition {
  const config = structuredClone(parseProjectConfig(value));
  const errors = graphIssues(config);
  if (errors.length) throw new FactoryCompileError(errors);
  inlineSources(config);
  const agents: ResolvedAgent[] = config.agents.map((agent) => ({
    ...agent,
    skills: agent.skillIds.map((id) => config.skills.find((item) => item.id === id)!),
    resolvedInstructions: instructions(config, agent.instructionIds, agent.instructions),
  }));
  const definition: Omit<FactoryDefinition, "id"> = {
    schemaVersion: 1,
    configRevision: config.revision,
    executionTarget: config.executionTarget,
    ...(config.targetOptions ? { targetOptions: config.targetOptions } : {}),
    instructions: config.instructions,
    skills: config.skills,
    agents,
    workflows: config.workflows.map((workflow) => {
      const graph = normalizedGraph(workflow);
      const context = instructions(config, workflow.instructionIds ?? config.instructions.map((item) => item.id), workflow.instructions);
      return {
        id: workflow.id, name: workflow.name, description: workflow.description, instructions: context,
        entryStageId: graph.entryStageId, edges: graph.edges, maxTransitions: graph.maxTransitions ?? 100,
        ...(graph.forks?.length ? { forks: graph.forks, stageScopes: inspectScopes(graph, "graph").scopes } : {}),
        stages: workflow.stages.map((stage) => ({
          ...stage,
          agents: stage.agentIds.map((id) => agents.find((agent) => agent.id === id)!),
          resolvedInstructions: [context, instructions(config, stage.instructionIds, stage.instructions)].filter(Boolean).join("\n\n"),
          execution: stage.execution ?? {}, maxVisits: stage.maxVisits ?? 3,
        })),
      };
    }),
  };
  const { configRevision: _revision, ...identity } = definition;
  return freeze({ ...definition, id: fingerprint(identity) });
}

/** Portable data modules are useful to code-based tooling without executing user code. */
export function exportFactory(value: unknown): FactoryBundle {
  const config = structuredClone(parseProjectConfig(value));
  const definition = compileFactory(config);
  inlineSources(config);
  const file = (path: string, data: unknown) => ({ path, content: JSON.stringify(data, null, 2) + "\n" });
  const files = [
    file("factory.json", definition), file("config.json", config),
    file("workflows.json", definition.workflows.map(({ stages: _stages, ...workflow }) => workflow)),
    file("stages.json", definition.workflows.map((workflow) => ({ workflowId: workflow.id, stages: workflow.stages }))),
    file("agents.json", definition.agents), file("instructions.json", definition.instructions), file("skills.json", definition.skills),
    { path: "README.md", content: "# Karabiner factory\n\n`factory.json` is the normalized executable definition used for run snapshots. `config.json` is the revisioned, editable source (including layout). Workflows route one stage at a time by success, failure or bounded return; commands are executable/argument data, not TypeScript callbacks. Import this bundle through Karabiner's factory import API to edit or run it.\n" },
  ];
  return { format: "karabiner-factory", version: 1, config, definition, files };
}
