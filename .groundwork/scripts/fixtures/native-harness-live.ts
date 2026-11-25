/** Explicit opt-in live observation; never imported as an automatically running check. */
import assert from "node:assert/strict";
import { mkdir, realpath } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { CopilotClient, RuntimeConnection } from "../../../packages/service/node_modules/@microsoft/copilot-sdk/dist/index.js";
import type { AgentDefinition, InteractionResponse, RunQuestion, StoredRun } from "../../../packages/contracts/src/index.ts";
import { FixtureApi } from "./native-harness-api.ts";

function permitted(root: string, question: RunQuestion): boolean {
  const tool = question.tool;
  if (!tool || tool.kind === "network" || tool.kind === "mcp" || tool.urls?.length) return false;
  const paths = tool.paths ?? [];
  if (paths.length) return paths.every((path) => { const location = resolve(root, path); const child = relative(root, location); return child !== ".." && !child.startsWith("../") && !isAbsolute(child); });
  if (tool.kind === "shell") return /^(pwd|ls(?:\s+-[A-Za-z]+)?)$/.test(tool.command?.trim() ?? "") && (!tool.cwd || resolve(tool.cwd) === resolve(root));
  return false;
}

export async function verifyLiveCopilot(api: FixtureApi, root: string) {
  assert.equal(process.env.KARABINER_LIVE_COPILOT, "1", "Live fixture must be explicitly authorized");
  root = await realpath(root);
  const executable = Bun.which("copilot"); assert.ok(executable, "An installed Copilot CLI is required");
  const deniedRoot = join(dirname(root), "live-denied-boundary"); await mkdir(deniedRoot, { recursive: true });
  const deniedPath = join(deniedRoot, "must-not-exist.txt");
  const output = join(root, "live-result.txt");
  const marker = `live-guidance-${crypto.randomUUID()}`;
  const agent: AgentDefinition = { id: "live", name: "Live Copilot verification", harness: "copilot", executable, args: [], model: process.env.KARABINER_LIVE_COPILOT_MODEL ?? "", role: "developer", mode: "interactive", skillIds: [], instructions: `This is an explicitly authorized local fixture, not repository work. No GitHub, MCP, network, installation, settings, git publication, or unrelated file operations. Use at most eight tool calls. First request the native file-write tool for ${deniedPath}; that owned test path is deliberately outside the permitted workspace and the operator will deny it. After denial never retry that operation or use another tool to bypass it. Ask the user one question before continuing. Then use the native file-write tool to write ${output}; its text must contain the operator's exact live guidance marker received while you were working. Finish with the requested structured factory result.` };
  const { task } = await api.project(root, agent);
  let run: StoredRun | undefined;
  const decisions: unknown[] = [];
  const ownedSessions = new Set<string>();
  let sent = false;
  try {
    run = await api.call<StoredRun>(`/v1/tasks/${task.id}/run`, "POST", {});
    const deadline = Date.now() + 120_000;
    while (Date.now() < deadline) {
      run = await api.run(run.id);
      for (const step of run.stages?.flatMap((stage) => stage.steps) ?? []) for (const attempt of step.attempts ?? []) if (attempt.sessionId) ownedSessions.add(attempt.sessionId);
      const tools = run.traceSpans?.filter((span) => span.kind === "tool") ?? [];
      if (tools.length > 12) { await api.call(`/v1/runs/${run.id}/cancel`, "POST", {}); throw new Error("Live fixture exceeded its twelve-tool observation bound"); }
      if (["completed", "failed", "cancelled"].includes(run.state)) break;
      for (const question of run.questions.filter((question) => question.state === "pending")) {
        if (!sent) { await api.call(`/v1/runs/${run.id}/instructions`, "POST", { message: marker }); sent = true; }
        let response: InteractionResponse;
        if (question.kind === "permission") {
          const allow = permitted(root, question);
          const choice = question.choices?.find((choice) => choice.effect === (allow ? "approve" : "deny")); assert.ok(choice, "Native permission must expose its decision effect");
          response = { choice: choice.id };
          decisions.push({ question, allow, response });
        } else {
          const text = `Proceed only inside the fixture. Exact marker: ${marker}`;
          const value = Object.fromEntries((question.fields ?? []).map((field) => {
            const selected = field.options?.find((option) => /yes|proceed|continue/i.test(option.label))?.id ?? field.options?.[0]?.id;
            return [field.id, field.type === "boolean" ? true : field.type === "integer" || field.type === "number" ? field.minimum ?? 1 : field.type === "multiselect" ? [selected ?? text] : field.type === "select" && !field.allowOther ? selected ?? text : text];
          }));
          response = Object.keys(value).length ? { value } : { answer: text };
        }
        await api.call(`/v1/runs/${run.id}/questions/${question.id}/answer`, "POST", response);
      }
      await Bun.sleep(100);
    }
    assert.equal(run.state, "completed", JSON.stringify(run));
    assert.ok(decisions.length, "Live Copilot must actually request a native permission");
    assert.ok(decisions.some((value) => !(value as { allow: boolean }).allow), "The outside-fixture request was denied");
    assert.equal(await Bun.file(deniedPath).exists(), false);
    assert.match(await Bun.file(output).text(), new RegExp(marker));
    assert.ok(run.instructions?.some((instruction) => instruction.message === marker && instruction.deliveries?.some((delivery) => delivery.mode === "native-turn")));
    return { run, marker, output: await Bun.file(output).text(), decisions, ownedSessions: [...ownedSessions] };
  } finally {
    if (run) {
      const latest = await api.run(run.id);
      for (const step of latest.stages?.flatMap((stage) => stage.steps) ?? []) for (const attempt of step.attempts ?? []) if (attempt.sessionId) ownedSessions.add(attempt.sessionId);
      if (!["completed", "failed", "cancelled"].includes(latest.state)) await api.call(`/v1/runs/${run.id}/cancel`, "POST", {});
    }
    if (ownedSessions.size) {
      const client = new CopilotClient({ runtime: RuntimeConnection.forSpawnedStdio({ path: executable }), mode: "coding-agent", github: { useLoggedInUser: true }, observability: { logLevel: "none" } });
      const timer = setTimeout(() => { void client.forceStop(); }, 15_000);
      try { await client.start(); for (const id of ownedSessions) await client.deleteSession(id); }
      finally { clearTimeout(timer); const errors = await client.stop(); if (errors.length) { await client.forceStop(); throw new AggregateError(errors, "Owned live-session cleanup failed"); } }
    }
  }
}
