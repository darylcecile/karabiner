import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { AgentDefinition, HarnessReadiness, RunQuestion, StoredRun } from "../../packages/contracts/src/index.ts";
import { alive, FixtureApi, folder, waitFor, wireLog } from "./fixtures/native-harness-api.ts";
import { verifyNativeStrands } from "./fixtures/native-harness-strands.ts";
import { verifyLiveCopilot } from "./fixtures/native-harness-live.ts";
import type { AdapterInteraction, AdapterRequest, AgentOutcome } from "../../packages/service/src/runtime/adapter-types.ts";
import { interact } from "../../packages/service/src/runtime/bridge-common.ts";
import { copilotPermissionHandler } from "../../packages/service/src/runtime/copilot-bridge.ts";

type Provider = "claude" | "codex" | "copilot" | "custom";
type CaseMode = "approve" | "deny" | "cancel" | "cleanup-failure";
const providers = ["claude", "codex", "copilot", "custom"] as const;
const selectedCases = new Set<string>();
const knownCases = new Set([...providers.flatMap((provider) => ["approve", "deny", "cancel", "autonomous"].map((mode) => `${provider}-${mode}`)), "copilot-cleanup-failure", "copilot-readiness", "permission-policy", "strands-autonomous"]);
for (let index = 2; index < process.argv.length; index++) if (process.argv[index] === "--case") {
  const name = process.argv[++index];
  if (!name || !knownCases.has(name)) throw new Error(`Unknown or missing --case: ${name ?? ""}`);
  selectedCases.add(name);
}

const root = resolve(import.meta.dir, "../..");
const live = process.argv.includes("--live-copilot");
if (live && selectedCases.size) throw new Error("--case selects deterministic native scenarios only");
if (live && process.env.KARABINER_LIVE_COPILOT !== "1") throw new Error("Live Copilot requires explicit KARABINER_LIVE_COPILOT=1 authorization");
const evidence = process.env.GROUNDWORK_EVIDENCE_DIR ?? join(root, ".groundwork/tmp/factory-harnesses");
await mkdir(evidence, { recursive: true });
const temporary = await mkdtemp(join(tmpdir(), "karabiner-native-harnesses-"));
const binary = join(temporary, "service");
const api = new FixtureApi(binary, join(temporary, "state.sqlite"), join(temporary, "home"), live);
const observations: { id: string; status?: "passed" | "failed"; observed: string; details: unknown }[] = [];
const roots: string[] = [];
const caseRuns = new Map<string, string>();
const caseFailures: string[] = [];
let failure: string | undefined;

async function isolated(id: string, work: () => Promise<void>, directory?: string) {
  let error: unknown;
  try { await work(); } catch (caught) { error = caught; }
  let run: StoredRun | undefined;
  let log: unknown[] = [];
  try {
    const runId = caseRuns.get(id);
    if (runId) {
      run = await api.run(runId);
      if (!["completed", "failed", "cancelled"].includes(run.state)) {
        await api.call(`/v1/runs/${runId}/cancel`, "POST", {});
        run = await api.run(runId);
      }
    }
    if (directory) log = await waitFor("isolated provider cleanup", () => wireLog(directory), (rows) => rows.filter((row) => row.event === "started").every((row) => !alive(row.pid)), 6_000);
  } catch (caught) {
    error = error ? new AggregateError([error, caught], "Scenario and cleanup failed") : caught;
    if (directory) {
      const rows = await wireLog(directory);
      const remaining = [...new Set(rows.filter((row) => row.event === "started").map((row) => row.pid as number))].filter(alive);
      for (const pid of remaining) { try { process.kill(pid, "SIGKILL"); } catch (stopError) { if ((stopError as NodeJS.ErrnoException).code !== "ESRCH") error = new AggregateError([error, stopError], "Fixture process cleanup failed"); } }
      try { await waitFor("failed case process cleanup", async () => remaining.filter(alive), (pids) => !pids.length, 3_000); }
      catch (stopError) { error = new AggregateError([error, stopError], "Fixture process cleanup did not settle"); }
    }
  }
  if (error) {
    const message = error instanceof Error ? error.stack ?? error.message : String(error);
    caseFailures.push(`${id}: ${message}`);
    const previous = observations.find((item) => item.id === id);
    const failed = { id, status: "failed" as const, observed: message, details: { completedAssertions: previous?.details, run, log: directory ? await wireLog(directory) : log } };
    if (previous) Object.assign(previous, failed); else observations.push(failed);
    process.exitCode = 1;
  }
}

async function executable(name: string) {
  const path = join(temporary, `native-${name}`);
  await writeFile(path, `#!${process.execPath}\nawait import(${JSON.stringify(join(root, ".groundwork/scripts/fixtures/native-harness-peer.ts"))});\n`, { mode: 0o700 });
  await chmod(path, 0o700);
  return path;
}

async function pending(id: string) {
  return waitFor("pending native interaction", () => api.run(id), (run) => {
    if (["failed", "cancelled", "completed"].includes(run.state)) throw new Error(`Expected interaction: ${JSON.stringify(run)}`);
    return run.questions.some((question) => question.state === "pending");
  });
}

async function answer(run: StoredRun, question: RunQuestion, mode: string, invalid = false) {
  const path = `/v1/runs/${run.id}/questions/${question.id}/answer`;
  assert.ok(question.attemptId, "Every native request is bound to an attempt");
  assert.equal(question.transport, "live");
  if (question.kind === "permission") {
    assert.ok(question.tool?.name, "Native permission contains a normalized tool");
    const choice = question.choices?.find((choice) => choice.effect === (mode === "deny" ? "deny" : "approve"));
    assert.ok(choice);
    return api.call(path, "POST", { choice: choice.id });
  }
  if (question.fields?.some((field) => field.id === "count")) {
    assert.equal(question.fields.find((field) => field.id === "count")?.type, "integer");
    assert.equal(question.fields.find((field) => field.id === "count")?.minimum, 1);
    if (invalid) for (const count of ["abc", 3.5]) {
      const result = await api.raw(path, "POST", { choice: "accept", value: { count, enabled: false, tags: ["a", "b"] } });
      assert.equal(result.status, 400, JSON.stringify(result));
      assert.equal((await api.run(run.id)).questions.find((item) => item.id === question.id)!.state, "pending");
    }
    return api.call(path, "POST", { choice: "accept", value: { count: 3, enabled: false, tags: ["a", "b"] } });
  }
  const values = Object.fromEntries((question.fields ?? []).map((field) => [field.id, field.type === "multiselect" ? ["Alpha", "Beta"] : field.type === "select" ? "Beta" : "Answer exactly"]));
  return api.call(path, "POST", Object.keys(values).length ? { value: values } : { answer: "Answer exactly" });
}

async function permissionPolicy() {
  const events: unknown[] = [];
  const request: AdapterRequest = {
    agent: { id: "policy", name: "Policy", harness: "custom", role: "fixture", mode: "autonomous", model: "fixture", instructions: "", skillIds: [] },
    prompt: "", instructions: "", skills: [], rootPath: temporary, runId: "policy", stepId: "policy", timeoutMs: 1_000,
    signal: new AbortController().signal,
    launch: async () => { throw new Error("Policy fixture does not launch a process"); },
    emit: (event) => events.push(event),
  };
  const permission: AdapterInteraction = { id: "offered-tool", provider: "custom", kind: "permission", prompt: "Allow owned tool?", choices: [{ id: "stop", label: "Deny", effect: "deny" }, { id: "provider-specific-allow", label: "Allow", effect: "approve" }] };
  for (const executionTarget of ["host", "mxc", "apple-container"] as const) {
    assert.deepEqual(await interact({ ...request, executionTarget }, permission), { choice: "provider-specific-allow" });
  }
  assert.equal(events.length, 3);
  await assert.rejects(interact(request, { ...permission, kind: "question" }), /questions require interactive mode/);
  await assert.rejects(interact(request, { ...permission, fields: [{ id: "value", label: "Required input", type: "text", required: true }] }), /form input require interactive mode/);
  await assert.rejects(interact(request, { ...permission, choices: [permission.choices![0]!] }), /did not offer a permission approval/);
  const cancelled = AbortSignal.abort(new Error("Owned invocation cancelled"));
  await assert.rejects(interact({ ...request, signal: cancelled }, permission), /Owned invocation cancelled/);
  await assert.rejects(interact(request, permission, cancelled), /Owned invocation cancelled/);
  await assert.rejects(async () => copilotPermissionHandler(request)({ kind: "read", path: join(temporary, "README.md"), intention: "Managed permission", managedApprovalRequired: true }, { sessionId: "policy" }), /Copilot requires human approval/);
  observations.push({ id: "permission-policy", observed: "Autonomous mode selects an offered approval on every target; cancelled requests and human input fail explicitly before entering a waiter", details: { events } });
}

async function autonomousJourney(provider: Provider, path: string) {
  const id = `${provider}-autonomous`;
  const directory = await folder(join(temporary, id)); roots.push(directory);
  const agent: AgentDefinition = { id: provider, name: id, role: "developer", harness: provider, model: "fixture-local", executable: path, args: ["--fixture-case", "autonomous"], protocol: provider === "custom" ? "jsonrpc-v2" : undefined, mode: "autonomous", skillIds: [], instructions: "Use the controlled native protocol only." };
  const { task } = await api.project(directory, agent);
  const started = await api.call<StoredRun>(`/v1/tasks/${task.id}/run`, "POST", {});
  caseRuns.set(id, started.id);
  const current = await waitFor("autonomous native completion", () => api.run(started.id), (run) => {
    assert.equal(run.questions.length, 0, "Autonomous permissions never create a human waiter");
    return ["completed", "failed", "cancelled"].includes(run.state);
  });
  assert.equal(current.state, "completed", JSON.stringify(current));
  assert.equal(current.stages![0]!.steps[0]!.attempts!.length, 1);
  for (const kind of ["write", "shell", "network"]) assert.equal(await readFile(join(directory, `effect-${kind}.txt`), "utf8"), "native effect\n");
  const log = await waitFor("autonomous provider cleanup", () => wireLog(directory), (rows) => rows.filter((row) => row.event === "started").every((row) => !alive(row.pid)));
  if (provider === "copilot") {
    const receipts = log.filter((row) => row.event === "received" && row.data.method === "session.permissions.handlePendingPermissionRequest");
    assert.equal(receipts.length, 4);
    for (const receipt of receipts) {
      assert.deepEqual(receipt.data.params.result, { kind: "approve-once", approvedInteractively: false });
      assert.deepEqual(receipt.data.params.decisionContext, { source: "host_policy", responseCapability: "headless", surface: "sdk", outcome: "auto_approved" });
    }
    assert.ok(log.some((row) => row.event === "read-effect" && row.data.content === "Owned native harness fixture\n"), "Copilot reads README through its automatic permission receipt");
  }
  observations.push({ id, observed: `${provider} autonomous mode completes actual owned effects through its native permission channel with no human answers`, details: { run: current, log } });
}

async function providerJourney(provider: Provider, mode: CaseMode, path: string, mxc: string) {
  const effectsComplete = mode === "approve" || mode === "cleanup-failure";
  const directory = await folder(join(temporary, `${provider}-${mode}`)); roots.push(directory);
  const agent: AgentDefinition = { id: provider, name: `${provider}-${mode}`, role: "developer", harness: provider, model: "fixture-local", executable: path, args: ["--fixture-case", mode, "--fixture-quoted", "one ' quoted argument", ...(mode === "cleanup-failure" ? ["--fixture-cleanup-failure"] : [])], protocol: provider === "custom" ? "jsonrpc-v2" : undefined, mode: "interactive", skillIds: [], instructions: "Use the controlled native protocol only." };
  // This exercises the released SDK -> compiled Bun stdio proxy -> fd3 target wrapper path.
  const target = provider === "copilot" && mode === "approve" ? { executable: mxc, network: false } : undefined;
  const { task } = await api.project(directory, agent, target);
  const started = await api.call<StoredRun>(`/v1/tasks/${task.id}/run`, "POST", {});
  caseRuns.set(`${provider}-${mode}`, started.id);
  let current = await pending(started.id);
  const first = current.questions.find((question) => question.state === "pending")!;
  let cancelDurationMs: number | undefined;
  if (mode === "cancel") {
    const beforeCancel = performance.now();
    await api.call(`/v1/runs/${started.id}/cancel`, "POST", {});
    cancelDurationMs = performance.now() - beforeCancel;
    current = await waitFor("native cancellation", () => api.run(started.id), (run) => run.state === "cancelled");
    assert.ok(cancelDurationMs < 10_000, `Cancellation settled in ${cancelDurationMs}ms`);
    assert.equal(current.questions.find((question) => question.id === first.id)!.state, "cancelled");
    assert.ok(current.questions.every((question) => question.state !== "pending"), "Cancellation settles every pending native request");
  } else {
    if (effectsComplete) {
      await waitFor("native control registration", () => api.run(started.id), (run) => Boolean(run.stages?.flatMap((stage) => stage.steps).some((step) => step.control)));
      await api.call(`/v1/runs/${started.id}/instructions`, "POST", { message: `guidance-${provider}-received` });
    }
    await answer(current, first, mode);
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
      current = await api.run(started.id);
      if (["completed", "failed", "cancelled"].includes(current.state)) break;
      const question = current.questions.find((question) => question.state === "pending");
      if (question) await answer(current, question, mode, provider === "copilot");
      else await Bun.sleep(35);
    }
    assert.equal(current.state, mode === "approve" ? "completed" : "failed", JSON.stringify(current));
  }
  const log = await waitFor("native process cleanup", () => wireLog(directory), (rows) => rows.filter((row) => row.event === "started").every((row) => !alive(row.pid)));
  assert.equal(current.stages![0]!.steps[0]!.attempts!.length, 1, "Completed effects and operator decisions never trigger automatic replay");
  if (effectsComplete) {
    for (const kind of ["write", "shell", "network"]) assert.equal(await readFile(join(directory, `effect-${kind}.txt`), "utf8"), "native effect\n");
    assert.ok(current.instructions?.some((instruction) => instruction.deliveries?.some((delivery) => delivery.mode === "native-turn" && delivery.attemptId === first.attemptId)));
    assert.ok(log.some((row) => row.event === "form-response" && row.data.count === 3 && row.data.enabled === false && Array.isArray(row.data.tags)));
    if (target) assert.ok(log.some((row) => row.provider === "mxc" && row.event === "fd3"));
    if (provider === "custom") {
      const initialize = log.find((row) => row.event === "received" && row.data.method === "initialize").data.params;
      const invoked = log.find((row) => row.event === "received" && row.data.method === "run").data.params;
      assert.equal(initialize.attemptId, first.attemptId); assert.equal(invoked.attemptId, first.attemptId);
      assert.ok(current.questions.some((question) => question.tool?.diff === "+native effect"));
    }
    if (provider === "copilot") {
      assert.equal(log.filter((row) => row.event === "received" && row.data.method === "session.create").length, 1);
      assert.equal(log.filter((row) => row.event === "effect").length, 3);
      assert.equal(log.filter((row) => row.event === "finished").length, 1);
      assert.equal(current.tokens, 7, "Provider-reported usage survives cleanup handling");
      for (const receipt of log.filter((row) => row.event === "received" && row.data.method === "session.permissions.handlePendingPermissionRequest")) {
        assert.equal(receipt.data.params.result.approvedInteractively, true);
        assert.deepEqual(receipt.data.params.decisionContext, { source: "human_response", responseCapability: "interactive", surface: "sdk", outcome: "prompted_user" });
      }
    }
    if (mode === "cleanup-failure") {
      const step = current.stages![0]!.steps[0]!;
      const attempt = step.attempts![0]!;
      assert.equal(attempt.error?.code, "adapter_cleanup_failed");
      assert.match(attempt.error!.message, /session\.disconnect:.*fixture-detach-failure-after-completion/);
      const details = attempt.error!.details as unknown as { provider: string; failures: { phase: string; message: string }[]; completed: AgentOutcome };
      assert.equal(details.provider, "Copilot");
      assert.ok(details.failures.some((failure) => failure.phase === "session.disconnect" && failure.message.includes("fixture-detach-failure-after-completion")));
      assert.equal(details.completed.outcome, "complete");
      assert.equal(details.completed.usage?.totalTokens, 7);
      assert.equal(attempt.sessionId, details.completed.sessionId);
      assert.equal(attempt.output, details.completed.summary);
      assert.equal(step.output, details.completed.summary);
      assert.equal(current.output, details.completed.summary);
      assert.equal(attempt.tokens, 7);
    }
  } else {
    assert.equal(log.filter((row) => row.event === "effect").length, 0);
    assert.equal(await Bun.file(join(directory, "effect-write.txt")).exists(), false);
    if (mode === "cancel") {
      // Claude SDK abortController closes stdin and then applies its bounded process
      // termination grace. Query.interrupt() is a separate per-turn control; an
      // interrupt frame is not promised when the whole SDK query is cancelled.
      const stopEvents = provider === "claude" ? ["native-abort", "eof", "terminated"] : ["native-abort"];
      assert.ok(log.some((row) => stopEvents.includes(row.event)), "The native cancellation path stopped the owned provider process");
    }
  }
  observations.push({ id: `${provider}-${mode}`, observed: `${provider} released protocol ${mode}: real SDK/transport, correlated human response, owned effect and cleanup`, details: { run: current, log, cancelDurationMs } });
}

try {
  const build = await Bun.build({ entrypoints: [join(root, "packages/service/src/main.ts")], compile: { outfile: binary } });
  assert.equal(build.success, true, build.logs.join("\n"));
  await api.start();
  if (live) {
    observations.push({ id: "live-copilot", observed: "Installed authenticated Copilot executes an owned temporary task through the public run and human-control APIs", details: await verifyLiveCopilot(api, await folder(join(temporary, "live-copilot"))) });
  } else {
    const paths = Object.fromEntries(await Promise.all(["claude", "codex", "copilot", "custom", "mxc"].map(async (name) => [name, await executable(name)])));
    if (!selectedCases.size || selectedCases.has("permission-policy")) await isolated("permission-policy", permissionPolicy);
    if (!selectedCases.size || selectedCases.has("copilot-readiness")) {
    const readinessRoot = await folder(join(temporary, "readiness")); roots.push(readinessRoot);
    const readinessProviders = selectedCases.size ? ["copilot"] as const : providers;
    for (const provider of readinessProviders) await isolated(`readiness-${provider}`, async () => {
      const agent: AgentDefinition = { id: provider, name: provider, harness: provider, executable: paths[provider], protocol: provider === "custom" ? "jsonrpc-v2" : undefined, args: [], role: "fixture", mode: "interactive", model: "fixture", instructions: "", skillIds: [] };
      const result = await api.call<HarnessReadiness>("/v1/harnesses/check", "POST", { agent, check: true, projectRoot: readinessRoot });
      assert.equal(result.installed, true); assert.equal(result.authenticated, provider === "custom" ? null : true);
    });
    await isolated("copilot-protocol-readiness", async () => {
      const agent: AgentDefinition = { id: "legacy-protocol", name: "Legacy protocol fixture", harness: "copilot", executable: await executable("copilot-protocol3"), args: [], role: "fixture", mode: "interactive", model: "fixture", instructions: "", skillIds: [] };
      const result = await api.call<HarnessReadiness>("/v1/harnesses/check", "POST", { agent, check: true, projectRoot: readinessRoot });
      assert.equal(result.code, "protocol_mismatch");
      assert.equal(result.installed, true); assert.equal(result.authenticated, null);
      assert.equal(result.message, "Installed Copilot reports protocol 3; this application SDK supports 4-4. Update the application SDK or choose a compatible installed CLI.");
      assert.ok(result.checks.some((check) => check.code === "protocol_compatibility" && check.status === "failed"));
      observations.push({ id: "copilot-protocol-readiness", observed: "Compiled service distinguishes protocol incompatibility from authentication without creating a session", details: result });
    });
    if (!selectedCases.size) await isolated("relative-custom-readiness", async () => {
    await writeFile(join(readinessRoot, "native-custom"), await readFile(paths.custom!), { mode: 0o700 });
    const relativeAgent: AgentDefinition = { id: "relative-custom", name: "Relative custom", harness: "custom", executable: "./native-custom", protocol: "jsonrpc-v2", args: [], role: "fixture", mode: "interactive", model: "fixture", instructions: "", skillIds: [] };
    const relativeReadiness = await api.call<HarnessReadiness>("/v1/harnesses/check", "POST", { agent: relativeAgent, check: true, projectRoot: readinessRoot });
    assert.equal(relativeReadiness.installed, true);
    assert.equal(relativeReadiness.authenticated, null);
    });
    await isolated("readiness-invariants", async () => {
      const readiness = await wireLog(readinessRoot);
      assert.equal(readiness.filter((row) => row.event === "received" && ["session.create", "thread/start", "turn/start", "session.sendAndWait", "run"].includes(row.data.method)).length, 0);
      assert.equal(readiness.some((row) => row.provider === "custom"), false);
      observations.push({ id: selectedCases.size ? "copilot-readiness" : "readiness", observed: "Known-provider checks use status APIs without sessions/inference; custom executable is not executed", details: readiness });
    });
    }
    for (const provider of providers) {
      const modes: CaseMode[] = provider === "copilot" ? ["approve", "deny", "cancel", "cleanup-failure"] : ["approve", "deny", "cancel"];
      for (const mode of modes) if (!selectedCases.size || selectedCases.has(`${provider}-${mode}`)) await isolated(`${provider}-${mode}`, () => providerJourney(provider, mode, paths[provider]!, paths.mxc!), join(temporary, `${provider}-${mode}`));
      if (!selectedCases.size || selectedCases.has(`${provider}-autonomous`)) await isolated(`${provider}-autonomous`, () => autonomousJourney(provider, paths[provider]!), join(temporary, `${provider}-autonomous`));
    }
    await isolated("provider-service-stop", () => api.stop());
    if (!selectedCases.size || selectedCases.has("strands-autonomous")) {
    const strandsRoot = await folder(join(temporary, "strands")); roots.push(strandsRoot);
    const strands = await verifyNativeStrands(strandsRoot, join(temporary, "strands.sqlite"), selectedCases.size ? ["autonomous"] : undefined);
    for (const item of strands) {
      if (item.status === "failed") { caseFailures.push(`strands-${item.mode}: ${item.error}`); process.exitCode = 1; }
      observations.push({ id: `strands-${item.mode}`, status: item.status, observed: item.status === "passed" ? "Real createHarness model-boundary guidance and native controls observed" : item.error!, details: item });
    }
    }
  }
} catch (error) { failure = error instanceof Error ? error.stack ?? error.message : String(error); process.exitCode = 1; }
finally {
  try { await api.stop(); } catch (error) { failure = [failure, String(error)].filter(Boolean).join("\n"); process.exitCode = 1; }
  if (caseFailures.length) failure = [failure, ...caseFailures].filter(Boolean).join("\n");
  const logs = await Promise.all(roots.map(async (directory) => ({ directory, wire: await wireLog(directory) })));
  const remaining = [...new Set(logs.flatMap((item) => item.wire.filter((row) => row.event === "started").map((row) => row.pid as number)))].filter(alive);
  if (remaining.length) {
    failure = [failure, `Fixture processes remained after service shutdown: ${remaining.join(", ")}`].filter(Boolean).join("\n"); process.exitCode = 1;
    for (const pid of remaining) { try { process.kill(pid, "SIGTERM"); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error; } }
    await waitFor("owned fixture process cleanup", async () => remaining.filter(alive), (pids) => !pids.length, 3_000).catch(() => { for (const pid of remaining.filter(alive)) process.kill(pid, "SIGKILL"); });
  }
  await Bun.write(join(evidence, "factory-harnesses.json"), JSON.stringify({ live, selectedCases: [...selectedCases], observations, logs, service: api.logs, failure: failure ?? null }, null, 2));
  await Bun.write(join(evidence, "result.json"), JSON.stringify({ observations: observations.map((item) => ({ id: item.id, status: item.status ?? "passed", observed: item.observed, evidence: ["factory-harnesses.json"] })).concat(failure ? [{ id: "failure", status: "failed", observed: failure, evidence: ["factory-harnesses.json"] }] : []), context: { fixture: live ? "live-copilot-owned-workspace" : "released-native-sdk-controlled-peers", externalWrites: false } }, null, 2));
  await rm(temporary, { recursive: true, force: true });
}
if (failure) console.error(failure); else console.log(`Native harness checks passed: ${observations.length}; evidence ${join(evidence, "factory-harnesses.json")}`);
