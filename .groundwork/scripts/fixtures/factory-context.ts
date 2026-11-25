import assert from "node:assert/strict";
import type { RunStage, RunToken, StoredRun } from "../../../packages/contracts/src/index.ts";
import { historicalContext } from "../../../packages/service/src/runtime/prompt-context.ts";

/** One short branch must remain visible when its sibling exceeds the recent-history window. */
export function boundedJoinContext(base: StoredRun) {
  const time = base.startedAt ?? base.createdAt;
  const stage = (id: string, tokenId: string, output: string): RunStage => ({
    id, stageId: id, tokenId, scopeId: tokenId === "root" ? null : "scope", branchId: tokenId === "root" ? null : tokenId,
    title: id, status: "completed", outcome: "success", startedAt: time, finishedAt: time,
    steps: [{ id: `${id}-step`, title: id, agentId: "fixture", kind: "agent", status: "completed", startedAt: time, finishedAt: time, output }],
  });
  const short = stage("short-result", "short", "The short branch independently completed its review.");
  const long = Array.from({ length: 12 }, (_, index) => stage(`long-${index}`, "long", "x".repeat(12_000) + (index === 11 ? "Long branch final outcome." : `Long branch step ${index}.`)));
  const join = { ...stage("join", "root", ""), status: "running" as const, steps: [{ id: "join-step", title: "Join reviewer", agentId: "fixture", kind: "agent" as const, status: "running" as const, startedAt: time, finishedAt: null, attempts: [] }] };
  const branch = (id: string, history: string[]): RunToken => ({ id, scopeId: id === "root" ? null : "scope", branchId: id === "root" ? null : id, stageId: "join", stageInvocationId: id === "root" ? "join" : null, history, status: id === "root" ? "running" : "completed" });
  const run: StoredRun = {
    ...base, stages: [short, ...long, join],
    execution: { version: 1, tokens: [branch("root", [short.id, ...long.map((stage) => stage.id)]), branch("short", [short.id]), branch("long", long.map((stage) => stage.id))], scopes: [{ id: "scope", forkId: "review-fork", parentTokenId: "root", joinStageId: "join", branchTokenIds: ["short", "long"], state: "joined", joinInvocationId: "join" }] },
  };
  const context = historicalContext(run, "root", "join", "join-step");
  assert.deepEqual(context.branchResults.map((branch) => branch.branchId), ["short", "long"]);
  assert.match(context.branchResults[0]!.summary!, /short branch independently completed/);
  assert.match(context.branchResults[1]!.summary!, /Long branch final outcome/);
  assert.ok(context.priorOutputs.length <= 8);
  assert.ok(JSON.stringify(context).length < 64_000, "The fixture's historical prompt remains bounded");
  assert.equal(context.historyTruncated, true);
  return { branchResults: context.branchResults, historicalCharacters: JSON.stringify(context).length, recentInvocations: context.priorOutputs.length, truncated: context.historyTruncated };
}
