import type { FactoryIssue, WorkflowGraph } from "./factory-types.ts";

/** Assign static branch scopes; runtime scope IDs are created for each fork visit. */
export function inspectScopes(graph: WorkflowGraph, path: string): { errors: FactoryIssue[]; scopes: Record<string, string> } {
  const errors: FactoryIssue[] = [];
  const scopes: Record<string, string> = {};
  const forks = graph.forks ?? [];
  const sources = new Set<string>();
  const joins = new Set<string>();
  const add = (message: string) => errors.push({ code: "invalid_fork_scope", path, message });
  for (const fork of forks) {
    if (sources.has(fork.stageId)) add(`stage ${fork.stageId} has more than one fork`);
    if (joins.has(fork.joinStageId)) add(`join stage ${fork.joinStageId} belongs to more than one fork`);
    if (fork.stageId === fork.joinStageId) add(`fork ${fork.id} must have a distinct join stage`);
    if (graph.edges.some((edge) => edge.sourceStageId === fork.stageId && edge.on === "success")) add(`fork ${fork.id} owns its success routing; remove its ordinary success edge`);
    sources.add(fork.stageId);
    joins.add(fork.joinStageId);
  }

  const walk = (entry: string, stop: string | null, scope: string, enclosing: Set<string>): boolean => {
    const visited = new Set<string>();
    const pending = [entry];
    let joined = false;
    while (pending.length) {
      const id = pending.pop()!;
      if (id === stop) { joined = true; continue; }
      if (visited.has(id)) continue;
      visited.add(id);
      if (Object.hasOwn(scopes, id) && scopes[id] !== scope) { add(`stage ${id} is shared across branch scopes`); continue; }
      Object.defineProperty(scopes, id, { value: scope, enumerable: true, configurable: true });
      const fork = forks.find((item) => item.stageId === id);
      const edges = graph.edges.filter((edge) => edge.sourceStageId === id && edge.on !== "return");
      if (fork) {
        if (enclosing.has(fork.id)) { add(`fork ${fork.id} recursively enters its own scope`); continue; }
        for (const branch of fork.branches) {
          const childScope = `${scope}/${encodeURIComponent(fork.id)}/${encodeURIComponent(branch.id)}`;
          if (!walk(branch.entryStageId, fork.joinStageId, childScope, new Set([...enclosing, fork.id]))) add(`branch ${branch.id} never reaches join ${fork.joinStageId}`);
        }
        pending.push(fork.joinStageId);
      } else if (stop && !edges.some((edge) => edge.on === "success")) add(`stage ${id} has a success path that ends before join ${stop}`);
      pending.push(...edges.map((edge) => edge.targetStageId));
    }
    return joined;
  };
  walk(graph.entryStageId, null, "root", new Set());
  for (const edge of graph.edges) if (edge.on === "return" && scopes[edge.sourceStageId] !== scopes[edge.targetStageId]) add(`return ${edge.id} crosses a fork branch boundary`);
  return { errors, scopes };
}
