import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { AgentDefinition, Document, FactoryBundle, FactoryDefinition, FactoryValidation, HarnessReadiness, Project, ProjectConfig, RunQuestion, Statistics, StoredRun, TargetReadiness, Task, WorkflowDefinition } from "../../packages/contracts/src/index.ts";

const root = resolve(import.meta.dir, "../..");
const evidence = process.env.GROUNDWORK_EVIDENCE_DIR ?? join(root, ".groundwork/tmp/factory-runtime");
await mkdir(evidence, { recursive: true });
const temporary = await mkdtemp(join(tmpdir(), "karabiner-factory-"));
const serviceBin = join(temporary, "karabiner-service"), cliBin = join(temporary, "karabiner"), db = join(temporary, "state.sqlite");
const harness = join(root, ".groundwork/scripts/fixtures/factory-harness.ts");
const observations: { id: string; observed: string; details: unknown }[] = [];
const roots: string[] = [], ownedPids = new Set<number>();
const serviceLogs: { pid: number; stdout: string; stderr: string }[] = [];
const receipts = new Map<string, { count: number; request?: unknown; response: unknown }>();
let service: Bun.Subprocess<"ignore", "pipe", "pipe"> | undefined;
let readers: Promise<void>[] = [];
let serviceUrl = "", failedStep = "compile", failure: string | undefined, cliCalls = 0;
let lostReplyPath: string | undefined;

async function waitFor<T>(label: string, read: () => Promise<T>, accept: (value: T) => boolean, timeout = 15_000): Promise<T> {
  const deadline = Date.now() + timeout;
  let value: T;
  do {
    value = await read();
    if (accept(value)) return value;
    await Bun.sleep(50);
  } while (Date.now() < deadline);
  throw new Error(`${label} did not settle: ${JSON.stringify(value!).slice(-5_000)}`);
}
async function startService(): Promise<void> {
  const child = Bun.spawn([serviceBin, "--port", "0", "--db", db], { stdin: "ignore", stdout: "pipe", stderr: "pipe" });
  service = child;
  serviceUrl = "";
  const log = { pid: child.pid, stdout: "", stderr: "" };
  serviceLogs.push(log);
  readers = [
    (async () => {
      for await (const chunk of child.stdout) {
        log.stdout += new TextDecoder().decode(chunk);
        for (const line of log.stdout.split("\n")) {
          try {
            const ready = JSON.parse(line);
            if (ready.event === "ready" && ready.pid === child.pid) serviceUrl = ready.url;
          } catch { /* A partial startup line is completed by the next chunk. */ }
        }
      }
    })(),
    (async () => { log.stderr = await new Response(child.stderr).text(); })(),
  ];
  await waitFor("owned service ready", async () => {
    if (child.exitCode !== null) throw new Error(`Service exited ${child.exitCode}: ${log.stderr}`);
    return serviceUrl;
  }, Boolean, 10_000);
}
async function stopService(): Promise<void> {
  if (!service) return;
  const child = service;
  service = undefined;
  child.kill("SIGTERM");
  const timeout = setTimeout(() => child.kill("SIGKILL"), 5_000);
  try { assert.equal(await child.exited, 0, "Owned service shutdown"); }
  finally { clearTimeout(timeout); await Promise.all(readers); }
}

// Only this fixture's CLI goes through the proxy. It records exact replies and an
// accepted-but-lost response, establishing request-once behavior without live APIs.
const proxy = Bun.serve({
  hostname: "127.0.0.1", port: 0,
  async fetch(request) {
    const url = new URL(request.url), path = url.pathname + url.search;
    const body = ["GET", "HEAD"].includes(request.method) ? undefined : await request.arrayBuffer();
    const submitted = body?.byteLength ? JSON.parse(new TextDecoder().decode(body)) : undefined;
    const response = await fetch(serviceUrl + path, { method: request.method, headers: request.headers, body });
    const text = await response.text();
    const key = `${request.method} ${url.pathname}`;
    receipts.set(key, { count: (receipts.get(key)?.count ?? 0) + 1, request: submitted, response: JSON.parse(text) });
    if (request.method === "POST" && url.pathname === lostReplyPath) {
      lostReplyPath = undefined;
      return Response.json({ error: { code: "fixture_reply_lost", message: "The fixture dropped the reply after the service accepted the mutation" } }, { status: 502 });
    }
    return new Response(text, { status: response.status, headers: { "content-type": "application/json" } });
  },
});
const cliUrl = `http://127.0.0.1:${proxy.port}`;

async function cliRaw(args: string[], input?: string): Promise<{ exit: number; result: { data?: unknown; error?: { code: string; message: string } }; stderr: string }> {
  cliCalls++;
  const child = Bun.spawn([cliBin, "--url", cliUrl, "--json", ...args], { stdin: input === undefined ? "ignore" : new TextEncoder().encode(input), stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr, exit] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
  return { exit, result: JSON.parse(stdout), stderr };
}
async function cli<T>(args: string[], expectedError?: string, input?: string): Promise<T> {
  const reply = await cliRaw(args, input);
  if (expectedError) {
    assert.equal(reply.exit, 1, JSON.stringify(reply));
    assert.equal(reply.result.error?.code, expectedError, JSON.stringify(reply));
  } else {
    assert.equal(reply.exit, 0, `${args.join(" ")}: ${JSON.stringify(reply)}`);
    assert.ok("data" in reply.result, "CLI preserves the data envelope");
  }
  return reply.result.data as T;
}
async function request<T>(path: string, method = "GET", body?: unknown): Promise<T> {
  const response = await fetch(serviceUrl + path, { method, headers: body === undefined ? undefined : { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(10_000) });
  const reply = await response.json() as { data: T; error?: unknown };
  assert.ok(response.ok, `${method} ${path}: ${JSON.stringify(reply)}`);
  return reply.data;
}
async function body(name: string, value: unknown): Promise<string> {
  const path = join(temporary, `${name}.json`);
  await Bun.write(path, JSON.stringify(value));
  return path;
}
function observed(id: string, message: string, details: unknown): void { observations.push({ id, observed: message, details }); }
async function git(directory: string, args: string[]): Promise<void> {
  const child = Bun.spawn(["git", ...args], { cwd: directory, stdout: "pipe", stderr: "pipe" });
  const [stderr, exit] = await Promise.all([new Response(child.stderr).text(), child.exited, new Response(child.stdout).text()]);
  assert.equal(exit, 0, `Fixture git ${args.join(" ")}: ${stderr}`);
}
async function projectFolder(name: string): Promise<string> {
  const directory = join(temporary, name);
  roots.push(directory);
  await mkdir(join(directory, "skills/fixture"), { recursive: true });
  await Bun.write(join(directory, "instructions.md"), "project-context-marker: preserve the fixture's real effects\n");
  await Bun.write(join(directory, "skills/fixture/SKILL.md"), "fixture-skill-marker: report only observed results\n");
  await Bun.write(join(directory, "result.txt"), "Before execution\n");
  await Bun.write(join(directory, ".gitignore"), ".fixture/\n");
  await git(directory, ["init", "--quiet"]);
  await git(directory, ["add", "."]);
  await git(directory, ["-c", "user.name=Karabiner fixture", "-c", "user.email=fixture@localhost", "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "Fixture baseline"]);
  return directory;
}
function agent(id: string, role = id): AgentDefinition {
  return { id, name: id, role, harness: "custom", model: "fixture-local", instructions: "Use only the supplied fixture context.", instructionIds: ["project"], skillIds: ["fixture"], mode: role === "question" ? "interactive" : "autonomous", executable: process.execPath, args: [harness] };
}
function command(id: string) { return { id, name: id, executable: process.execPath, args: [harness, "command", id], timeoutMs: 10_000 }; }
function config(workflow: WorkflowDefinition, agents: AgentDefinition[], revision: number): ProjectConfig {
  return { revision, instructions: [{ id: "project", name: "Project fixture", content: "resolve this file at capture", path: "instructions.md" }], skills: [{ id: "fixture", name: "Fixture", description: "Deterministic fixture context", content: "resolve this skill at capture", path: "skills/fixture/SKILL.md" }], agents, workflows: [workflow], executionTarget: "host", scheduler: { enabled: false, workflowId: workflow.id, statuses: ["todo", "done"], pollIntervalMs: 1_000, maxConcurrentRuns: 1 } };
}
const stage = (id: string, agentIds: string[] = []) => ({ id, name: id, instructions: "", agentIds, returnToStageIds: [] as string[] });
async function createProject(name: string, directory: string, workflow: WorkflowDefinition, agents: AgentDefinition[]) {
  const project = await cli<Project>(["project", "create", name, "--path", directory]);
  const current = await cli<ProjectConfig>(["config", "get", project.id]);
  const saved = await cli<ProjectConfig>(["config", "set", project.id, "--file", await body(`${name}-config`, config(workflow, agents, current.revision))]);
  return { project, saved };
}
async function createTask(project: Project, workflowId: string, title: string, agentId?: string): Promise<Task> {
  return cli(["task", "create", title, "--project", project.id, "--workflow", workflowId, ...(agentId ? ["--agent", agentId] : [])]);
}
const run = (id: string) => request<StoredRun>(`/v1/runs/${id}`);
const waitRun = (id: string, label: string, predicate: (value: StoredRun) => boolean) => waitFor(label, () => run(id), predicate);
async function calls(directory: string): Promise<{ runId: string; stepId: string; agentId: string; role: string; answered: boolean }[]> {
  const file = Bun.file(join(directory, ".fixture/calls.jsonl"));
  return await file.exists() ? (await file.text()).trim().split("\n").filter(Boolean).map((line) => JSON.parse(line)) : [];
}
async function ready(directory: string, id: string): Promise<number> {
  const path = join(directory, ".fixture", id, "ready.json");
  await waitFor("owned fixture process ready", () => Bun.file(path).exists(), Boolean);
  const info = await Bun.file(path).json() as { pid: number; runId: string };
  assert.equal(info.runId, id);
  ownedPids.add(info.pid);
  return info.pid;
}
function alive(pid: number): boolean {
  try { process.kill(pid, 0); return true; }
  catch (error) { if ((error as NodeJS.ErrnoException).code === "ESRCH") return false; throw error; }
}
async function release(directory: string, id: string): Promise<void> { await Bun.write(join(directory, ".fixture", id, "release"), "release\n"); }
function repliedOnce(path: string, value?: unknown): void {
  const receipt = receipts.get(`POST ${path}`);
  assert.equal(receipt?.count, 1, `${path} was submitted once`);
  if (value !== undefined) assert.deepEqual((receipt!.response as { data: unknown }).data, value, "CLI preserves the complete service response");
}

async function harnessReadinessJourney(): Promise<void> {
  failedStep = "harness readiness and exact CLI request bodies";
  const directory = await projectFolder("readiness");
  const marker = join(directory, "unexpected-probe-start"), executable = join(directory, "bin/agent");
  await mkdir(join(directory, "bin"), { recursive: true });
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
  await Bun.write(executable, `#!/bin/sh\nprintf started > ${quote(marker)}\nexit 0\n`);
  await chmod(executable, 0o755);
  const configured: AgentDefinition = { ...agent("readiness"), executable, args: [], instructionIds: [], skillIds: [], protocol: "jsonrpc-v2" };
  const workflow: WorkflowDefinition = { id: "readiness", name: "Readiness", description: "No execution requested", instructions: "", stages: [stage("later", [configured.id])] };
  const { project, saved } = await createProject("Readiness fixture", directory, workflow, [configured]);
  const key = "POST /v1/harnesses/check";
  const check = async (args: string[], expected: unknown, exit = 0, stdin?: string): Promise<HarnessReadiness> => {
    const before = receipts.get(key)?.count ?? 0;
    const reply = await cliRaw(["harnesses", "check", ...args], stdin);
    assert.equal(reply.exit, exit, JSON.stringify(reply));
    assert.ok(reply.result.data, "Readiness preserves the full data/diagnostics even on unavailable status");
    const receipt = receipts.get(key)!;
    assert.equal(receipt.count, before + 1);
    assert.deepEqual(receipt.request, expected);
    assert.deepEqual((receipt.response as { data: unknown }).data, reply.result.data);
    return reply.result.data as HarnessReadiness;
  };
  const file = await body("readiness-agent", configured);
  const unknown = await check(["--file", file], { agent: configured, check: false });
  assert.equal(unknown.authenticated, null);
  assert.equal(unknown.executionAvailable, true);
  assert.equal(unknown.supported.permissions, true);
  const explicitCustom = await check(["--file", "-", "--check"], { agent: configured, check: true }, 0, JSON.stringify(configured));
  assert.equal(explicitCustom.code, "custom_auth_unknown");
  const relativeAgent = { ...configured, executable: "./bin/agent" };
  const rooted = await check(["--file", "-", "--project-root", directory], { agent: relativeAgent, check: false, projectRoot: directory }, 0, JSON.stringify(relativeAgent));
  assert.equal(rooted.installed, true);
  assert.equal(rooted.executionAvailable, true);
  await check(["--project", project.id, "--agent", configured.id, "--check"], { agent: saved.agents[0], check: true, projectRoot: directory });
  assert.equal(await Bun.file(marker).exists(), false, "Custom readiness never launches the configured executable, even with --check");
  const missingAgent = { ...configured, executable: join(temporary, "missing-harness") };
  const missing = await check(["--file", await body("missing-readiness-agent", missingAgent)], { agent: missingAgent, check: false }, 1);
  assert.equal(missing.code, "executable_missing");
  assert.ok(missing.checks.some((item) => item.status === "failed"));
  const targetKey = `POST /v1/projects/${project.id}/execution-target/check`, targetBefore = receipts.get(targetKey)?.count ?? 0;
  const target = await cli<TargetReadiness>(["targets", "--project", project.id, "--check", "--purpose", "command", "--executable", process.execPath, "--file", "-"], undefined, JSON.stringify(saved));
  assert.equal(target.available, true, JSON.stringify(target));
  const targetReceipt = receipts.get(targetKey)!;
  assert.equal(targetReceipt.count, targetBefore + 1);
  assert.deepEqual(targetReceipt.request, { check: true, purpose: "command", executable: process.execPath, config: saved });
  assert.deepEqual((targetReceipt.response as { data: unknown }).data, target);
  observed("readiness-cli", "Compiled CLI preserves exact file/stdin/saved-agent harness checks, explicit roots for relative executables, unknown/unavailable diagnostics and one configured-target draft readiness POST; custom readiness never executes its configured harness.", { unknown, explicitCustom, rooted, missing, target });
}

async function nativeCliJourney(): Promise<void> {
  failedStep = "CLI offered permission, typed stdin answer and live delivery";
  const directory = await projectFolder("native-cli"), duplex = join(root, ".groundwork/scripts/fixtures/factory-duplex.ts");
  const make = (id: string, role: string): AgentDefinition => ({ ...agent(id, role), mode: "interactive", protocol: "jsonrpc-v2", args: [duplex] });
  const workflow: WorkflowDefinition = { id: "native-cli", name: "Native CLI", description: "Reuse the core-owned live protocol fixture", instructions: "", stages: [stage("permission", ["cli-permission"]), stage("form", ["cli-form"])] };
  const { project } = await createProject("Native CLI fixture", directory, workflow, [make("cli-permission", "permission"), make("cli-form", "form")]);
  const task = await createTask(project, workflow.id, "Answer the actual live requests through the CLI");
  const started = await cli<StoredRun>(["run", "start", task.id]);
  const waiting = await waitRun(started.id, "offered native permission", (value) => value.questions.some((question) => question.kind === "permission" && question.state === "pending"));
  const permission = waiting.questions.find((question) => question.kind === "permission" && question.state === "pending")!;
  const shown = await cli<RunQuestion & { runId: string }>(["question", "show", permission.id, "--run", started.id]);
  assert.equal(shown.runId, started.id);
  assert.equal(shown.attemptId, permission.attemptId);
  assert.deepEqual(shown.choices, permission.choices);
  assert.deepEqual(shown.tool, permission.tool);
  const choice = permission.choices!.find((item) => item.effect === "approve")!;
  const human = Bun.spawn([cliBin, "--url", cliUrl, "question", "list", "--run", started.id], { stdout: "pipe", stderr: "pipe" });
  const [display, humanError, humanExit] = await Promise.all([new Response(human.stdout).text(), new Response(human.stderr).text(), human.exited]);
  assert.equal(humanExit, 0, humanError);
  for (const value of [permission.prompt, choice.id, permission.provider!, permission.transport!, permission.attemptId!]) assert.ok(display.includes(value), `Pending request display includes ${value}`);
  await cli(["run", "instruct", started.id, "CLI live guidance reaches this invocation"]);
  const permissionReply = await cli<StoredRun>(["question", "answer", permission.id, "--run", started.id, "--choice", choice.id]);
  const permissionPath = `/v1/runs/${started.id}/questions/${permission.id}/answer`;
  repliedOnce(permissionPath, permissionReply);
  assert.deepEqual(receipts.get(`POST ${permissionPath}`)!.request, { choice: choice.id });
  const formRun = await waitRun(started.id, "typed native form", (value) => value.questions.some((question) => question.kind === "question" && question.state === "pending"));
  const form = formRun.questions.find((question) => question.kind === "question" && question.state === "pending")!;
  const formShown = await cli<RunQuestion>(["question", "show", form.id, "--run", started.id]);
  assert.deepEqual(formShown.fields, form.fields);
  const response = { answers: { count: "3", ratio: "0", enabled: "false", tags: ["a", "b"] } };
  const answered = await cli<StoredRun>(["question", "answer", form.id, "--run", started.id, "--file", "-"], undefined, JSON.stringify(response));
  const formPath = `/v1/runs/${started.id}/questions/${form.id}/answer`;
  repliedOnce(formPath, answered);
  assert.deepEqual(receipts.get(`POST ${formPath}`)!.request, response, "CLI preserves the full submitted JSON before authoritative normalization");
  const completed = await waitRun(started.id, "native CLI fixture completion", (value) => value.state === "completed" && value.artifacts.length > 0);
  const actual = await Bun.file(join(directory, ".fixture", started.id, "typed-form.json")).json();
  assert.deepEqual(actual, { count: 3, ratio: 0, enabled: false, tags: ["a", "b"] });
  assert.ok(completed.instructions?.some((instruction) => instruction.deliveries?.some((delivery) => delivery.mode === "native-turn")));
  const full = await cli<StoredRun>(["run", "show", started.id, "--project", project.id, "--task", task.id]);
  assert.deepEqual(full, await run(started.id), "Run show preserves raw execution, attempts and delivery metadata");
  assert.ok(full.execution?.tokens.length && full.steps?.every((step) => step.attempts?.length));
  const peer = (await Bun.file(join(directory, ".fixture/events.jsonl")).text()).trim().split("\n").map((line) => JSON.parse(line));
  observed("native-cli", "Compiled CLI displays full offered requests, answers the actual permission ID once, preserves multiselect/stdin JSON and live instruction data, and exposes raw token/attempt histories; the reused core-owned live child receives canonical numeric/boolean/array values.", { shown, display, formShown, submitted: response, actual, run: full, peer });
}

async function deliveryJourney(): Promise<{ id: string; documentId: string }> {
  failedStep = "factory compile, export and import";
  const directory = await projectFolder("delivery");
  const workflow: WorkflowDefinition = {
    id: "delivery", name: "Delivery", description: "Branch, review return and human decision", instructions: "",
    stages: [
      { ...stage("build", ["builder"]), maxVisits: 2, execution: { before: [command("build.before")], gates: [{ id: "build-gate", kind: "command", command: command("build.gate") }], after: [command("build.after")] } },
      { ...stage("check"), maxVisits: 2, execution: { commands: [command("check")] } },
      { ...stage("repair", ["repair"]), maxVisits: 1 },
      { ...stage("review", ["review-first", "review-second"]), returnToStageIds: ["build", "check"], maxVisits: 2 },
      { ...stage("decision", ["question"]), execution: { gates: [{ id: "ship", kind: "approval", prompt: "Approve this observed fixture output?" }], after: [command("delivery.after")] } },
    ],
    graph: { entryStageId: "build", maxTransitions: 12, edges: [
      { id: "build-check", sourceStageId: "build", targetStageId: "check", on: "success" },
      { id: "check-repair", sourceStageId: "check", targetStageId: "repair", on: "failure" },
      { id: "check-review", sourceStageId: "check", targetStageId: "review", on: "success" },
      { id: "repair-review", sourceStageId: "repair", targetStageId: "review", on: "success" },
      { id: "review-return", sourceStageId: "review", targetStageId: "build", on: "return" },
      { id: "review-return-check", sourceStageId: "review", targetStageId: "check", on: "return" },
      { id: "review-decision", sourceStageId: "review", targetStageId: "decision", on: "success" },
    ] },
    layout: { positions: { build: { x: 0, y: 0 }, check: { x: 260, y: 0 }, repair: { x: 260, y: 180 }, review: { x: 520, y: 0 }, decision: { x: 780, y: 0 } } },
  };
  const { project, saved } = await createProject("Delivery fixture", directory, workflow, [agent("builder", "build"), agent("repair"), agent("review-first"), agent("review-second"), agent("question")]);
  const compiled = await cli<FactoryDefinition>(["factory", "compile", project.id]);
  const legacy = structuredClone(saved);
  delete legacy.workflows[0]!.graph;
  const legacyCompiled = await cli<FactoryDefinition>(["factory", "compile", project.id, "--file", await body("legacy-returns", legacy)]);
  assert.deepEqual(legacyCompiled.workflows[0]!.edges.filter((edge) => edge.sourceStageId === "review" && edge.on === "return").map((edge) => edge.targetStageId).sort(), ["build", "check"]);
  const moved = structuredClone(saved);
  moved.workflows[0]!.layout!.positions.build = { x: 90, y: 70 };
  let latest = await cli<ProjectConfig>(["config", "set", project.id, "--file", await body("moved", moved)]);
  assert.equal((await cli<FactoryDefinition>(["factory", "compile", project.id])).id, compiled.id);
  const exported = await cli<FactoryBundle>(["factory", "export", project.id]);
  assert.equal(exported.definition.id, compiled.id);
  assert.deepEqual(exported.config.workflows[0]!.layout, moved.workflows[0]!.layout);
  assert.equal(JSON.parse(exported.files.find((file) => file.path === "factory.json")!.content).id, compiled.id);
  assert.deepEqual(JSON.parse(exported.files.find((file) => file.path === "config.json")!.content).workflows[0].layout, moved.workflows[0]!.layout);
  const exportDir = join(temporary, "exported-factory");
  await cli(["factory", "export", project.id, "--directory", exportDir]);
  latest = await cli<ProjectConfig>(["factory", "import", project.id, "--file", join(exportDir, "bundle.json"), "--revision", String(latest.revision)]);
  assert.equal((await cli<FactoryDefinition>(["factory", "show", project.id])).id, compiled.id);
  assert.deepEqual(latest.workflows[0]!.layout, moved.workflows[0]!.layout);
  const invalid = structuredClone(latest);
  invalid.workflows[0]!.graph!.edges[0]!.targetStageId = "missing-stage";
  const validation = await cliRaw(["factory", "validate", project.id, "--file", await body("invalid", invalid)]);
  assert.equal(validation.exit, 1);
  assert.equal((validation.result.data as FactoryValidation).valid, false);
  assert.ok((validation.result.data as FactoryValidation).errors.some((issue) => issue.code === "dangling_reference" && issue.path.includes("targetStageId")));
  const unavailableSource = structuredClone(latest);
  unavailableSource.instructions[0]!.path = "missing-instructions.md";
  await cli(["factory", "compile", project.id, "--file", await body("unavailable-source", unavailableSource)], "source_unavailable");
  await cli(["config", "set", project.id, "--file", await body("stale", saved)], "conflict");
  observed("factory-identity", "CLI/native-equivalent config, layout-only edits and exported JSON bundle retain executable identity; invalid edges and stale writes are rejected.", { factoryId: compiled.id, layout: latest.workflows[0]!.layout, files: exported.files.map((file) => file.path), validation: validation.result });

  failedStep = "delivery branch, return and durable question";
  const task = await createTask(project, "delivery", "Deliver the fixture", "builder");
  const started = await cli<StoredRun>(["run", "start", task.id, "--revision", String(task.revision)]);
  repliedOnce(`/v1/tasks/${task.id}/run`, started);
  assert.equal(started.snapshot.factory.id, compiled.id);
  assert.match(started.snapshot.config.instructions[0]!.content, /project-context-marker/);
  const waiting = await waitRun(started.id, "human question", (value) => value.state === "waiting" && value.questions.some((question) => question.kind === "question" && question.state === "pending"));
  const question = waiting.questions.find((item) => item.state === "pending")!;
  const beforeRestart = await calls(directory);
  assert.deepEqual(waiting.stages!.map((item) => item.stageId), ["build", "check", "repair", "review", "build", "check", "review", "decision"]);
  latest.agents[0]!.name = "Edited after snapshot";
  await cli(["config", "set", project.id, "--file", await body("during-run", latest)]);
  await stopService();
  await startService();
  const restored = await run(started.id);
  assert.equal(restored.state, "waiting");
  assert.deepEqual(restored.snapshot, started.snapshot);
  assert.equal(restored.questions.find((item) => item.state === "pending")!.id, question.id);
  assert.deepEqual(await calls(directory), beforeRestart);
  const listed = await cli<(RunQuestion & { runId: string })[]>(["question", "list", "--run", started.id]);
  assert.equal(listed[0]!.id, question.id);
  const answered = await cli<StoredRun>(["question", "answer", question.id, "Ship the observed change", "--run", started.id, "--revision", String(restored.revision)]);
  repliedOnce(`/v1/runs/${started.id}/questions/${question.id}/answer`, answered);
  const approval = await waitRun(started.id, "approval gate", (value) => value.state === "waiting" && value.questions.some((item) => item.kind === "approval" && item.state === "pending"));
  const gate = approval.questions.find((item) => item.state === "pending")!;
  await cli(["question", "answer", gate.id, "approve", "--run", started.id, "--revision", String(approval.revision)]);
  const completed = await waitRun(started.id, "completed delivery artifacts", (value) => value.state === "completed" && value.artifacts.some((item) => item.kind === "code_tour"));
  assert.deepEqual(completed.snapshot, started.snapshot);
  const recorded = await calls(directory);
  assert.deepEqual(recorded.map((item) => item.role), ["build", "repair", "review-first", "build", "review-first", "review-second", "question", "question"]);
  assert.equal(recorded.at(-1)!.answered, true);
  assert.equal(completed.tokens, recorded.length * 10);
  assert.equal(completed.cost, null);
  assert.ok(completed.durationMs! >= 0);
  assert.ok(completed.stages!.every((item) => item.visit! <= 2));
  assert.ok(completed.traceSpans!.some((span) => span.kind === "stage" && span.durationMs! >= 0));
  assert.ok(completed.traceSpans!.some((span) => span.kind === "agent" && span.parentId !== null && span.durationMs! >= 0));
  const events = (await Bun.file(join(directory, ".fixture/events.jsonl")).text()).trim().split("\n").map((line) => JSON.parse(line).event);
  assert.deepEqual(events.slice(0, 4), ["build.before", "build", "build.gate", "build.after"]);
  assert.equal(events.at(-1), "delivery.after");
  assert.equal(await Bun.file(join(directory, "result.txt")).text(), "Implemented revision 2\n");
  assert.equal(await Bun.file(join(directory, "human-answer.txt")).text(), "Ship the observed change\n");
  const documentId = completed.artifacts.find((item) => item.kind === "code_tour")!.documentId!;
  const tour = await cli<Document>(["tour", "show", documentId]);
  const diff = tour.tour!.chapters.flatMap((chapter) => chapter.blocks).find((block) => block.kind === "diff" && block.path === "result.txt");
  assert.ok(diff?.kind === "diff");
  assert.match(diff.patch, /\+Implemented revision 2/);
  assert.match(tour.tour!.overview!, new RegExp(started.id));
  const summary = await cli<Document>(["doc", "show", completed.artifacts.find((item) => item.kind === "document")!.documentId!]);
  assert.match(summary.markdown, new RegExp(started.id));
  const stats = await cli<Statistics>(["stats", "--project", project.id]);
  assert.equal(stats.runCount, 1);
  assert.equal(stats.completedRunCount, 1);
  assert.equal(stats.tokens, completed.tokens);
  assert.equal(stats.cost, null);
  observed("delivery", "A real custom process follows a failure branch and bounded review return, preserves distinct reviewers despite task assignment, survives a waiting checkpoint restart, receives a CLI answer, passes approval and persists actual file/tour/usage results.", { run: completed, calls: recorded, events, diff: diff.patch, statistics: stats });

  failedStep = "captured start stage and rejected approval";
  const rejectedTask = await cli<Task>(["task", "create", "Inspect a decision-stage start", "--project", project.id, "--workflow", "delivery", "--stage", "decision", "--agent", "builder"]);
  const rejectedStart = await cli<StoredRun>(["run", "start", rejectedTask.id]);
  const rejectedQuestion = await waitRun(rejectedStart.id, "decision-stage question", (value) => value.state === "waiting" && value.questions.some((item) => item.state === "pending"));
  await cli(["question", "answer", rejectedQuestion.questions.find((item) => item.state === "pending")!.id, "Do not ship this attempt", "--run", rejectedStart.id]);
  const rejectedGate = await waitRun(rejectedStart.id, "decision-stage approval", (value) => value.state === "waiting" && value.questions.some((item) => item.kind === "approval" && item.state === "pending"));
  await cli(["question", "answer", rejectedGate.questions.find((item) => item.state === "pending")!.id, "reject", "--run", rejectedStart.id]);
  const rejected = await waitRun(rejectedStart.id, "rejected gate outcome", (value) => value.state === "failed" && value.artifacts.length > 0);
  assert.deepEqual(rejected.stages!.map((item) => item.stageId), ["decision"]);
  assert.equal(rejected.stages![0]!.outcome, "failure");
  assert.ok(rejected.errors.some((error) => error.message.includes("declined")));
  assert.equal(rejected.stages![0]!.steps.find((step) => step.title === "delivery.after")!.status, "skipped");

  failedStep = "rejected approval follows its configured failure edge";
  const recoveryConfig = await cli<ProjectConfig>(["config", "get", project.id]);
  const recoveryWorkflow = recoveryConfig.workflows.find((workflow) => workflow.id === "delivery")!;
  recoveryWorkflow.stages.find((stage) => stage.id === "decision")!.execution!.retries = 2;
  recoveryWorkflow.stages.push({ ...stage("recovery"), execution: { commands: [command("finish")] } });
  recoveryWorkflow.graph!.edges.push({ id: "decision-recovery", sourceStageId: "decision", targetStageId: "recovery", on: "failure" });
  const savedRecovery = await cli<ProjectConfig>(["config", "set", project.id, "--file", await body("approval-recovery", recoveryConfig)]);
  const recoveryTask = await cli<Task>(["task", "create", "Route a declined gate", "--project", project.id, "--workflow", "delivery", "--stage", "decision"]);
  const recoveryStart = await cli<StoredRun>(["run", "start", recoveryTask.id]);
  assert.equal(recoveryStart.snapshot.config.revision, savedRecovery.revision, "Manual start captures the newly saved config");
  const recoveryQuestion = await waitRun(recoveryStart.id, "recovery-stage question", (run) => run.questions.some((question) => question.state === "pending"));
  await cli(["question", "answer", recoveryQuestion.questions.find((question) => question.state === "pending")!.id, "Inspect rejection recovery", "--run", recoveryStart.id]);
  const recoveryGate = await waitRun(recoveryStart.id, "recovery-stage approval", (run) => run.questions.some((question) => question.kind === "approval" && question.state === "pending"));
  await cli(["question", "answer", recoveryGate.questions.find((question) => question.kind === "approval" && question.state === "pending")!.id, "reject", "--run", recoveryStart.id]);
  const recovered = await waitRun(recoveryStart.id, "configured failure edge completed", (run) => run.state === "completed");
  assert.deepEqual(recovered.stages!.map((stage) => [stage.stageId, stage.outcome]), [["decision", "failure"], ["recovery", "success"]]);
  assert.equal(recovered.stages![0]!.steps.find((step) => step.kind === "gate")!.attempts!.length, 1, "Declining a gate never consumes configured automatic retries");
  assert.equal(await Bun.file(join(directory, "finished.txt")).text(), "The resumed pipeline reached its final command.\n");
  observed("rejected-approval", "Declining approval finalizes the failed invocation and skips remaining steps. Without a recovery edge the run fails; with a configured failure edge it executes the real recovery command once, without retrying the denial.", { run: rejected, recovered });
  return { id: completed.id, documentId };
}

async function controlJourney(): Promise<void> {
  failedStep = "cooperative pause and resume";
  const directory = await projectFolder("controls");
  const workflow: WorkflowDefinition = { id: "control", name: "Control", description: "Owned process boundary", instructions: "", stages: [stage("hold", ["hold"]), { ...stage("finish", ["guidance"]), execution: { commands: [command("finish")] } }] };
  const { project } = await createProject("Control fixture", directory, workflow, [agent("hold"), agent("guidance")]);
  const task = await createTask(project, "control", "Pause a real active step");
  const started = await cli<StoredRun>(["run", "start", task.id]);
  await ready(directory, started.id);
  await cli(["run", "pause", started.id]);
  await release(directory, started.id);
  const paused = await waitRun(started.id, "pause between steps", (value) => value.state === "paused");
  assert.equal(paused.steps!.filter((step) => step.status === "completed").length, 1);
  const instructed = await cli<StoredRun>(["run", "instruct", started.id, "operator-fixture-guidance: inspect before completing", "--revision", String(paused.revision)]);
  assert.equal(instructed.instructions!.at(-1)!.deliveredAt, null);
  const resumed = await cli<StoredRun>(["run", "resume", started.id, "--revision", String(instructed.revision)]);
  assert.equal(resumed.id, started.id);
  const completed = await waitRun(started.id, "resumed process completion", (value) => value.state === "completed" && value.artifacts.length > 0);
  assert.equal(await Bun.file(join(directory, "finished.txt")).text(), "The resumed pipeline reached its final command.\n");
  assert.match(await Bun.file(join(directory, "guidance.txt")).text(), /operator-fixture-guidance: inspect before completing/);
  assert.ok(completed.instructions!.at(-1)!.deliveredAt);
  assert.equal(completed.steps!.find((step) => step.id === completed.instructions!.at(-1)!.stepId)!.agentId, "guidance");

  failedStep = "uncertain CLI reply, cancellation and retry";
  const cancelTask = await createTask(project, "control", "Cancel a real active process");
  lostReplyPath = `/v1/tasks/${cancelTask.id}/run`;
  await cli(["run", "start", cancelTask.id], "fixture_reply_lost");
  repliedOnce(`/v1/tasks/${cancelTask.id}/run`);
  const accepted = await cli<StoredRun[]>(["run", "list", "--task", cancelTask.id]);
  assert.equal(accepted.length, 1, "Accepted mutation is not automatically repeated after its reply is lost");
  const cancelledId = accepted[0]!.id, pid = await ready(directory, cancelledId);
  const cancelled = await cli<StoredRun>(["run", "cancel", cancelledId]);
  assert.equal(cancelled.state, "cancelled");
  await waitFor("cancelled process exited", async () => alive(pid), (value) => !value);
  const retried = await cli<StoredRun>(["run", "retry", cancelledId, "--revision", String(cancelled.revision)]);
  assert.notEqual(retried.id, cancelledId);
  assert.equal(retried.retryOf, cancelledId);
  assert.equal(retried.snapshot.factory.id, cancelled.snapshot.factory.id);
  await ready(directory, retried.id);
  await release(directory, retried.id);
  await waitRun(retried.id, "explicit retry completed", (value) => value.state === "completed" && value.artifacts.length > 0);
  assert.equal((await run(cancelledId)).state, "cancelled");

  failedStep = "interrupted running process recovery";
  const interruptedTask = await createTask(project, "control", "Inspect interrupted effects");
  const interrupted = await cli<StoredRun>(["run", "start", interruptedTask.id]);
  const interruptedPid = await ready(directory, interrupted.id);
  await stopService();
  await startService();
  const recovered = await run(interrupted.id);
  assert.equal(recovered.state, "paused");
  assert.ok(recovered.errors.some((error) => error.code === "interrupted"));
  assert.equal(alive(interruptedPid), false);
  await cli(["run", "resume", interrupted.id], "conflict");
  const effectRows = (await Bun.file(join(directory, ".fixture/effects.jsonl")).text()).trim().split("\n").map((line) => JSON.parse(line));
  assert.equal(effectRows.filter((entry) => entry.runId === interrupted.id).length, 1);
  await cli(["run", "cancel", interrupted.id]);
  observed("controls", "Pause waits for the active process boundary, resume finishes it, cancellation settles the owned process, one lost start reply is not retried, explicit retry retains history, and interrupted running work requires operator review.", { completed, cancelled, retryId: retried.id, recovered, effects: effectRows });
}

async function schedulerJourney(): Promise<void> {
  failedStep = "scheduler opt-in and one-time pickup";
  const directory = await projectFolder("scheduler");
  const workflow: WorkflowDefinition = { id: "scheduled", name: "Scheduled", description: "Explicit pickup", instructions: "", stages: [stage("once", ["once"])] };
  const { project, saved } = await createProject("Scheduler fixture", directory, workflow, [agent("once")]);
  const task = await createTask(project, "scheduled", "Pick up exactly once");
  await Bun.sleep(1_100); // Observe a full configured scheduler interval while disabled.
  assert.deepEqual(await cli(["run", "list", "--task", task.id]), []);
  saved.scheduler!.enabled = true;
  await cli(["config", "set", project.id, "--file", await body("enabled", saved)]);
  const scheduled = await waitFor("scheduler completion", () => request<StoredRun[]>(`/v1/runs?taskId=${task.id}`), (values) => values.length === 1 && values[0]!.state === "completed" && values[0]!.artifacts.length > 0);
  await Bun.sleep(2_100); // Two more eligible intervals: done is deliberately in the filter.
  const retained = await cli<StoredRun[]>(["run", "list", "--task", task.id]);
  assert.equal(retained.length, 1);
  assert.equal(retained[0]!.trigger, "scheduler");
  assert.equal(await Bun.file(join(directory, "scheduled.txt")).text(), scheduled[0]!.id + "\n");
  observed("scheduler", "An eligible task waits for explicit scheduler enablement and is picked up once, including two later polls where completed tasks remain eligible by status.", { taskId: task.id, run: retained[0] });
}

try {
  const builds = await Promise.all([Bun.build({ entrypoints: [join(root, "packages/service/src/main.ts")], compile: { outfile: serviceBin } }), Bun.build({ entrypoints: [join(root, "packages/cli/src/main.ts")], compile: { outfile: cliBin } })]);
  for (const build of builds) assert.equal(build.success, true, build.logs.join("\n"));
  await startService();
  await harnessReadinessJourney();
  await nativeCliJourney();
  const delivery = await deliveryJourney();
  await controlJourney();
  assert.equal((await run(delivery.id)).state, "completed");
  assert.equal((await cli<Document>(["tour", "show", delivery.documentId])).kind, "code_tour");
  await schedulerJourney();
  await stopService();
} catch (error) {
  failure = `${failedStep}: ${error instanceof Error ? error.stack ?? error.message : String(error)}`;
  process.exitCode = 1;
} finally {
  try { await stopService(); } catch (error) { failure = [failure, `Cleanup: ${String(error)}`].filter(Boolean).join("\n"); process.exitCode = 1; }
  await proxy.stop(true);
  for (const pid of ownedPids) {
    if (!alive(pid)) continue;
    try { process.kill(-pid, "SIGTERM"); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ESRCH") { failure = [failure, `Fixture process cleanup: ${String(error)}`].filter(Boolean).join("\n"); process.exitCode = 1; } }
  }
  const fixtures = await Promise.all(roots.map(async (directory) => ({ directory, calls: await calls(directory), result: await Bun.file(join(directory, "result.txt")).exists() ? await Bun.file(join(directory, "result.txt")).text() : null })));
  await Bun.write(join(evidence, "factory-runtime.json"), JSON.stringify({ fixture: "isolated compiled service/CLI, five serial business scenarios and custom CLI/protocol cases", cliCalls, observations, receipts: Object.fromEntries(receipts), fixtures, serviceLogs, failure: failure ?? null }, null, 2));
  await Bun.write(join(evidence, "result.json"), JSON.stringify({ observations: observations.map((item) => ({ id: item.id, status: "passed", observed: item.observed, evidence: ["factory-runtime.json"] })).concat(failure ? [{ id: "failure", status: "failed", observed: failure, evidence: ["factory-runtime.json"] }] : []), context: { fixture: "isolated-local-factory", providers: "scripted-only", externalWrites: false } }, null, 2));
  await rm(temporary, { recursive: true, force: true });
}
if (failure) console.error(failure); else console.log(`Factory runtime passed: ${observations.length} scenarios, ${cliCalls} CLI calls; evidence ${join(evidence, "factory-runtime.json")}`);
