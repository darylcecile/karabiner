import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { AgentDefinition, FactoryValidation, Project, ProjectConfig, RunQuestion, StageDefinition, StoredRun, Task, WorkflowDefinition } from "../../packages/contracts/src/index.ts";
import { startService } from "../../packages/service/src/server.ts";
import { boundedJoinContext } from "./fixtures/factory-context.ts";

const root = resolve(import.meta.dir, "../..");
const evidence = process.env.GROUNDWORK_EVIDENCE_DIR ?? join(root, ".groundwork/tmp/factory-parallel");
await mkdir(evidence, { recursive: true });
const temporary = await mkdtemp(join(tmpdir(), "karabiner-parallel-"));
const dbPath = join(temporary, "state.sqlite");
const duplex = join(import.meta.dir, "fixtures/factory-duplex.ts");
const checkpoint = join(import.meta.dir, "fixtures/factory-checkpoint.ts");
const roots: string[] = [];
const pids = new Set<number>();
const observations: { id: string; observed: string; details: unknown }[] = [];
const requests: { method: string; path: string; status: number; body?: unknown }[] = [];
const scenario = process.env.FACTORY_PARALLEL_SCENARIO ?? "all";
let service: ReturnType<typeof startService> | undefined;
let label = "start owned service", failure: string | undefined;

function boot() { service = startService({ dbPath, port: 0 }); }
async function shutdown() { const current = service; service = undefined; if (current) await current.stop(); }
async function api<T>(path: string, method = "GET", body?: unknown, expected = 200): Promise<T> {
  const response = await fetch(service!.url + path, { method, headers: body === undefined ? undefined : { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(20_000) });
  const reply = await response.json() as { data: T; error?: unknown };
  requests.push({ method, path, status: response.status, body });
  assert.equal(response.status, expected, `${method} ${path}: ${JSON.stringify(reply).slice(0, 4_000)}`);
  return reply.data;
}
async function until<T>(name: string, read: () => Promise<T>, accept: (value: T) => boolean, timeout = 20_000): Promise<T> {
  const deadline = Date.now() + timeout;
  let value: T;
  do { value = await read(); if (accept(value)) return value; await Bun.sleep(40); } while (Date.now() < deadline);
  throw new Error(`${name} did not settle: ${JSON.stringify(value!).slice(-5_000)}`);
}
const getRun = (id: string) => api<StoredRun>(`/v1/runs/${id}`);
function waitRun(id: string, name: string, accept: (run: StoredRun) => boolean) {
  return until(name, async () => { const run = await getRun(id); if (!accept(run) && ["completed", "failed", "cancelled"].includes(run.state)) throw new Error(`${name}: unexpected ${run.state}: ${JSON.stringify(run.errors)}`); return run; }, accept);
}
function alive(pid: number): boolean {
  try { process.kill(pid, 0); return true; }
  catch (error) { if ((error as NodeJS.ErrnoException).code === "ESRCH") return false; throw error; }
}
async function journal(directory: string): Promise<any[]> {
  const file = Bun.file(join(directory, ".fixture/events.jsonl"));
  const entries = await file.exists() ? (await file.text()).trim().split("\n").filter(Boolean).map((line) => JSON.parse(line)) : [];
  for (const entry of entries) if (entry.event === "start") pids.add(entry.pid);
  return entries;
}
async function started(directory: string, runId: string, roles: string[]) {
  return until("owned child barriers", async () => (await journal(directory)).filter((entry) => entry.runId === runId && entry.event === "start"), (entries) => roles.every((role) => entries.some((entry) => entry.role === role)));
}
async function release(directory: string, runId: string, role: string) { await Bun.write(join(directory, ".fixture", runId, `release-${role}`), "release\n"); }
async function stopped(entries: { pid: number }[]) {
  await until("owned children exited", async () => entries.every((entry) => !alive(entry.pid)), Boolean);
  for (const entry of entries) pids.delete(entry.pid);
}
function observed(id: string, message: string, details: unknown) { observations.push({ id, observed: message, details }); }
async function git(directory: string, args: string[]) {
  const child = Bun.spawn(["git", ...args], { cwd: directory, stdout: "pipe", stderr: "pipe" });
  const [exit, stderr] = await Promise.all([child.exited, new Response(child.stderr).text(), new Response(child.stdout).text()]);
  assert.equal(exit, 0, `Fixture git failed: ${stderr}`);
}
async function folder(name: string) {
  const directory = join(temporary, name); roots.push(directory);
  await mkdir(directory, { recursive: true });
  await Bun.write(join(directory, ".gitignore"), ".fixture/\n");
  await Bun.write(join(directory, "baseline.txt"), "Owned fixture baseline\n");
  await git(directory, ["init", "--quiet"]); await git(directory, ["add", "."]);
  await git(directory, ["-c", "user.name=Karabiner fixture", "-c", "user.email=fixture@localhost", "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "Fixture baseline"]);
  return directory;
}
function agent(id: string, role = id, legacy = false): AgentDefinition {
  return { id, name: id, role, harness: "custom", model: "fixture", mode: "interactive", instructions: "Only operate on this owned fixture.", skillIds: [], executable: process.execPath, args: [legacy ? checkpoint : duplex], protocol: legacy ? "jsonl-v1" : "jsonrpc-v2" };
}
const stage = (id: string, agents: string[] = []): StageDefinition => ({ id, name: id, instructions: "", agentIds: agents, returnToStageIds: [] });
async function project(name: string, workflow: WorkflowDefinition, agents: AgentDefinition[], kind: Project["kind"] = "standalone") {
  const directory = await folder(name);
  const value = await api<Project>("/v1/projects", "POST", { name, rootPath: directory, kind, ...(kind === "github" ? { repository: "owner/fixture" } : {}) }, 201);
  const current = await api<ProjectConfig>(`/v1/projects/${value.id}/config`);
  const config: ProjectConfig = { revision: current.revision, instructions: [], skills: [], agents, workflows: [workflow], executionTarget: "host" };
  const saved = await api<ProjectConfig>(`/v1/projects/${value.id}/config`, "PUT", config);
  return { project: value, directory, config: saved };
}
async function start(projectId: string, workflowId: string, title: string) {
  const task = await api<Task>("/v1/tasks", "POST", { projectId, workflowId, title }, 201);
  return api<StoredRun>(`/v1/tasks/${task.id}/run`, "POST", {}, 202);
}
async function question(runId: string, afterId?: string): Promise<{ run: StoredRun; question: RunQuestion }> {
  const run = await waitRun(runId, "pending addressed question", (run) => run.questions.some((question) => question.state === "pending" && question.id !== afterId));
  return { run, question: run.questions.find((question) => question.state === "pending" && question.id !== afterId)! };
}
const answer = (runId: string, questionId: string, body: unknown, status = 200) => api<StoredRun>(`/v1/runs/${runId}/questions/${questionId}/answer`, "POST", body, status);
const control = (id: string, action: string) => api<StoredRun>(`/v1/runs/${id}/${action}`, "POST", {});

async function configReplacementJourney() {
  label = "known-field config replacement and incomplete fork drafts";
  const workflow: WorkflowDefinition = { id: "editing", name: "Editing", description: "Known field replacement", instructions: "", stages: [{ ...stage("one", ["configured"]), execution: { cwd: "subdir", commands: [{ id: "command", executable: process.execPath, args: [], env: { KEEP: "yes", REMOVE: "old" } }] } }, stage("two")], graph: { entryStageId: "one", edges: [{ id: "next", sourceStageId: "one", targetStageId: "two", on: "success" }] }, layout: { positions: { one: { x: 0, y: 0 }, two: { x: 200, y: 0 } } } };
  const fixture = await project("editing", workflow, [{ ...agent("configured"), harness: "strands", strands: { provider: "openai", credentialEnv: "FIXTURE_KEY_REFERENCE" } }], "github");
  const path = `/v1/projects/${fixture.project.id}/config`;
  const configured = { ...fixture.config, targetOptions: { image: "fixture-image" }, integration: { provider: "github" as const, scope: "repository" as const, pushFields: ["title", "status"], pullStatusMap: { Todo: "todo" as const, Review: "in_review" as const }, pushStatusMap: { todo: "Todo", in_review: "Review" } }, futureRoot: { keep: true } };
  Object.assign(configured.agents[0]!, { futureAgent: { keep: true } });
  let saved = await api<ProjectConfig & { futureRoot?: unknown }>(path, "PUT", configured);
  const changed = structuredClone(saved);
  delete changed.integration!.pullStatusMap!.Review;
  delete changed.integration!.pushStatusMap!.in_review;
  changed.integration!.pushFields = ["title"];
  delete changed.workflows[0]!.stages[0]!.execution!.commands![0]!.env!.REMOVE;
  delete changed.workflows[0]!.stages[0]!.execution!.cwd;
  delete changed.targetOptions!.image;
  delete changed.agents[0]!.strands!.credentialEnv;
  delete changed.futureRoot;
  delete (changed.agents[0] as AgentDefinition & { futureAgent?: unknown }).futureAgent;
  saved = await api(path, "PUT", changed);
  assert.deepEqual(saved.integration!.pullStatusMap, { Todo: "todo" });
  assert.deepEqual(saved.integration!.pushStatusMap, { todo: "Todo" });
  assert.deepEqual(saved.integration!.pushFields, ["title"]);
  assert.deepEqual(saved.workflows[0]!.stages[0]!.execution!.commands![0]!.env, { KEEP: "yes" });
  assert.equal(saved.workflows[0]!.stages[0]!.execution!.cwd, undefined);
  assert.equal(saved.targetOptions!.image, undefined); assert.equal(saved.agents[0]!.strands!.credentialEnv, undefined);
  assert.deepEqual(saved.futureRoot, { keep: true }); assert.deepEqual((saved.agents[0] as any).futureAgent, { keep: true });
  const empty = structuredClone(saved);
  empty.integration!.pullStatusMap = {}; empty.integration!.pushStatusMap = {};
  empty.integration!.pushFields = [];
  empty.workflows[0]!.stages[0]!.execution!.commands![0]!.env = {};
  empty.workflows[0]!.stages.pop(); empty.workflows[0]!.graph!.edges = [];
  delete empty.workflows[0]!.layout!.positions.two;
  saved = await api(path, "PUT", empty);
  assert.deepEqual(saved.integration!.pullStatusMap, {}); assert.deepEqual(saved.integration!.pushStatusMap, {});
  assert.deepEqual(saved.integration!.pushFields, []);
  assert.deepEqual(saved.workflows[0]!.stages[0]!.execution!.commands![0]!.env, {});
  assert.deepEqual(Object.keys(saved.workflows[0]!.layout!.positions), ["one"]);

  const draft = structuredClone(saved);
  delete draft.integration!.pushFields;
  draft.workflows[0] = { id: "editing", name: "Draft fork", description: "Draft", instructions: "", stages: [{ ...stage("fork"), kind: "fork" }, stage("a"), stage("b"), { ...stage("join"), kind: "join" }], graph: { entryStageId: "fork", edges: [{ id: "a-join", sourceStageId: "a", targetStageId: "join", on: "success" }, { id: "b-join", sourceStageId: "b", targetStageId: "join", on: "success" }], forks: [{ id: "parallel", stageId: "fork", branches: [], joinStageId: "join" }] } };
  saved = await api(path, "PUT", draft);
  assert.equal(saved.integration!.pushFields, undefined);
  let validation = await api<FactoryValidation>(`/v1/projects/${fixture.project.id}/factory/validate`, "POST", {});
  assert.equal(validation.valid, false); assert.ok(validation.errors.some((error) => error.code === "insufficient_branches"));
  saved.workflows[0]!.graph!.forks![0]!.branches.push({ id: "a", entryStageId: "a" });
  saved = await api(path, "PUT", saved);
  validation = await api<FactoryValidation>(`/v1/projects/${fixture.project.id}/factory/validate`, "POST", {});
  assert.equal(validation.valid, false); assert.ok(validation.errors.some((error) => error.code === "insufficient_branches"));
  saved.workflows[0]!.graph!.forks![0]!.branches.push({ id: "b", entryStageId: "b" });
  await api(path, "PUT", saved);
  validation = await api<FactoryValidation>(`/v1/projects/${fixture.project.id}/factory/validate`, "POST", {});
  assert.equal(validation.valid, true);
  observed("configuration-replacement", "Actual config PUT removes known optional values/map entries and positioned stages while retaining unknown extensions; zero/one-branch drafts save but compile only after both branches are configured.", { config: await api(path), validation });
}

async function nestedJourney() {
  label = "nested fork overlap, live permission, pause and restart";
  const workflow: WorkflowDefinition = {
    id: "nested", name: "Nested", description: "Concurrent owned barriers", instructions: "",
    stages: [{ ...stage("start"), kind: "fork" }, stage("permission", ["permission"]), { ...stage("nested"), kind: "fork" }, stage("left", ["left"]), { ...stage("right", ["right"]), returnToStageIds: ["right"], maxVisits: 2 }, { ...stage("inner-join", ["inner"]), kind: "join" }, { ...stage("outer-join", ["outer"]), kind: "join" }, stage("finish", ["finish"])],
    graph: { entryStageId: "start", maxTransitions: 30, forks: [{ id: "outer", stageId: "start", branches: [{ id: "short", entryStageId: "permission" }, { id: "long", entryStageId: "nested" }], joinStageId: "outer-join" }, { id: "inner", stageId: "nested", branches: [{ id: "left", entryStageId: "left" }, { id: "right", entryStageId: "right" }], joinStageId: "inner-join" }], edges: [{ id: "p-j", sourceStageId: "permission", targetStageId: "outer-join", on: "success" }, { id: "l-j", sourceStageId: "left", targetStageId: "inner-join", on: "success" }, { id: "r-j", sourceStageId: "right", targetStageId: "inner-join", on: "success" }, { id: "r-r", sourceStageId: "right", targetStageId: "right", on: "return" }, { id: "inner-outer", sourceStageId: "inner-join", targetStageId: "outer-join", on: "success" }, { id: "outer-finish", sourceStageId: "outer-join", targetStageId: "finish", on: "success" }] },
  };
  const fixture = await project("nested", workflow, [agent("permission"), agent("left", "hold-left"), agent("right", "hold-return"), agent("inner", "join-inner"), agent("outer", "join-outer"), agent("finish")]);
  const run = await start(fixture.project.id, "nested", "Nested execution");
  const barriers = await started(fixture.directory, run.id, ["permission", "hold-left", "hold-return"]);
  assert.ok(barriers.every((entry) => alive(entry.pid)), "All three real child processes overlap before any barrier is released");
  const pending = await question(run.id);
  assert.equal(pending.run.state, "running", "Other branches continue during a live permission wait");
  assert.equal(pending.question.tool?.kind, "write");
  await waitRun(run.id, "sibling streaming changes revision", (value) => value.revision > pending.run.revision);
  await answer(run.id, pending.question.id, { choice: "allow_once" });
  await until("actual permitted file effect", () => Bun.file(join(fixture.directory, ".fixture", run.id, "effect-permission-1")).exists(), Boolean);
  await control(run.id, "pause");
  await Promise.all([release(fixture.directory, run.id, "hold-left"), release(fixture.directory, run.id, "hold-return")]);
  const paused = await waitRun(run.id, "paused branch cursor", (value) => value.state === "paused");
  assert.equal(paused.execution!.scopes.length, 2);
  assert.ok(paused.execution!.tokens.some((token) => token.stageId === "right" && token.status === "paused"));
  await shutdown(); await boot();
  const reopened = await getRun(run.id);
  assert.equal(reopened.state, "paused"); assert.deepEqual(reopened.execution, paused.execution);
  assert.equal(reopened.snapshot.factory.id, paused.snapshot.factory.id);
  await control(run.id, "resume");
  const completed = await waitRun(run.id, "joined nested completion", (value) => value.state === "completed");
  for (const id of ["inner-join", "outer-join"]) assert.equal(completed.stages!.filter((stage) => stage.stageId === id).length, 1);
  assert.equal(completed.stages!.filter((stage) => stage.stageId === "right").length, 2);
  assert.ok(completed.execution!.scopes.every((scope) => scope.state === "joined" && scope.joinInvocationId));
  assert.equal(new Set(completed.stages!.map((stage) => stage.id)).size, completed.stages!.length);
  const attempts = completed.steps!.flatMap((step) => step.attempts ?? []);
  assert.equal(new Set(attempts.map((attempt) => attempt.id)).size, attempts.length);
  assert.ok(completed.execution!.scopes.every((scope) => completed.stages!.find((stage) => stage.id === scope.joinInvocationId)?.tokenId === scope.parentTokenId));
  const inner = await Bun.file(join(fixture.directory, ".fixture", run.id, "join-inner.json")).json();
  const outer = await Bun.file(join(fixture.directory, ".fixture", run.id, "join-outer.json")).json();
  assert.deepEqual(inner.map((entry: any) => entry.branchId), ["left", "right"]);
  assert.deepEqual(outer.map((entry: any) => entry.branchId), ["short", "long"]);
  assert.match(inner[0].summary, /hold-left complete/); assert.match(inner[1].summary, /hold-return complete/);
  const rightContext = await Bun.file(join(fixture.directory, ".fixture", run.id, "prompt-hold-return-2.json")).json();
  assert.deepEqual(rightContext.priorOutputs.map((entry: any) => entry.branchId), ["right"]);
  const entries = (await journal(fixture.directory)).filter((entry) => entry.runId === run.id && entry.event === "start");
  for (const role of ["join-inner", "join-outer"]) assert.equal(entries.filter((entry) => entry.role === role).length, 1, `${role} has one actual process invocation`);
  await stopped(entries);
  observed("nested-parallel", "Three real child processes overlap; a permission is answered without run CAS during sibling streaming, scoped return/pause survives restart, and nested joins each execute once with ordered branch outputs and local handoff.", { completed, paused, reopened, barriers, inner, outer, boundedContext: boundedJoinContext(completed) });
}

async function failureJourney() {
  label = "branch failure, cancellation and interrupted scope history";
  const workflow: WorkflowDefinition = { id: "failure", name: "Failure", description: "Sibling cleanup", instructions: "", stages: [{ ...stage("fork"), kind: "fork" }, stage("fail", ["fail"]), { ...stage("hold", ["hold"]), execution: { retries: 1 } }, { ...stage("join"), kind: "join" }], graph: { entryStageId: "fork", forks: [{ id: "fork", stageId: "fork", branches: [{ id: "fail", entryStageId: "fail" }, { id: "hold", entryStageId: "hold" }], joinStageId: "join" }], edges: [{ id: "f-j", sourceStageId: "fail", targetStageId: "join", on: "success" }, { id: "h-j", sourceStageId: "hold", targetStageId: "join", on: "success" }] } };
  const fixture = await project("failure", workflow, [agent("fail"), agent("hold", "hold-sibling")]);
  const run = await start(fixture.project.id, "failure", "Fail one branch");
  const children = await started(fixture.directory, run.id, ["fail", "hold-sibling"]);
  assert.ok(children.every((entry) => alive(entry.pid)));
  await release(fixture.directory, run.id, "fail");
  const failed = await waitRun(run.id, "unhandled branch failure", (value) => value.state === "failed");
  await stopped(children);
  assert.equal(failed.execution!.scopes[0]!.state, "failed");
  assert.equal(failed.execution!.tokens.find((token) => token.branchId === "hold")!.status, "cancelled");
  assert.equal(failed.stages!.find((stage) => stage.stageId === "hold")!.steps[0]!.attempts!.length, 1);
  assert.equal(failed.stages!.filter((stage) => stage.stageId === "join").length, 0);
  const cancelRun = await start(fixture.project.id, "failure", "Cancel both branches");
  const cancelledChildren = await started(fixture.directory, cancelRun.id, ["fail", "hold-sibling"]);
  const cancelled = await control(cancelRun.id, "cancel");
  assert.equal(cancelled.state, "cancelled"); await stopped(cancelledChildren);
  const interrupted = await start(fixture.project.id, "failure", "Interrupt active branches");
  const interruptedChildren = await started(fixture.directory, interrupted.id, ["fail", "hold-sibling"]);
  const before = await getRun(interrupted.id);
  await shutdown(); await boot();
  const recovered = await getRun(interrupted.id);
  assert.equal(recovered.state, "paused"); assert.ok(recovered.errors.some((error) => error.code === "interrupted"));
  assert.deepEqual(recovered.execution!.scopes.map((scope) => scope.id), before.execution!.scopes.map((scope) => scope.id));
  assert.deepEqual(recovered.execution!.tokens.map((token) => ({ id: token.id, history: token.history })), before.execution!.tokens.map((token) => ({ id: token.id, history: token.history })));
  await stopped(interruptedChildren);
  await api(`/v1/runs/${interrupted.id}/resume`, "POST", {}, 409);
  await control(interrupted.id, "cancel");
  observed("parallel-cleanup", "Unhandled branch failure cancels the owned sibling, run cancellation settles both branches, and interrupted scope identities remain inspectable without automatic replay.", { failed, cancelled, before, recovered });
}

async function retryJourney() {
  label = "permission request reuse, step retry and nonblocking guidance";
  const workflow: WorkflowDefinition = { id: "retry", name: "Retry", description: "Bounded step retry", instructions: "", stages: [{ ...stage("work", ["worker"]), execution: { retries: 1, retryDelayMs: 50 } }] };
  const fixture = await project("retry", workflow, [agent("worker", "permission-retry")]);
  const run = await start(fixture.project.id, "retry", "Retry after an allowed attempt fails");
  const first = await question(run.id);
  const instructed = await api<StoredRun>(`/v1/runs/${run.id}/instructions`, "POST", { message: "Apply this at the next native boundary" });
  assert.equal(instructed.instructions!.at(-1)!.deliveredAt, null, "Instruction POST returns while the native permission is still pending");
  await answer(run.id, first.question.id, { choice: "allow_once" });
  const second = await question(run.id, first.question.id);
  assert.equal(second.question.requestId, first.question.requestId);
  assert.equal(second.question.stepId, first.question.stepId);
  assert.notEqual(second.question.attemptId, first.question.attemptId);
  assert.equal(second.question.state, "pending");
  await answer(run.id, second.question.id, { choice: "allow_once" });
  const completed = await waitRun(run.id, "retried step completion", (value) => value.state === "completed");
  assert.deepEqual(completed.steps![0]!.attempts!.map((attempt) => attempt.status), ["failed", "completed"]);
  assert.ok(completed.steps![0]!.attempts![0]!.error);
  assert.ok(completed.steps![0]!.attempts!.every((attempt) => attempt.startedAt && attempt.finishedAt));
  assert.ok(completed.instructions![0]!.deliveries!.some((delivery) => delivery.mode === "native-turn"));
  const liveCalls = (await journal(fixture.directory)).filter((entry) => entry.runId === run.id);
  assert.equal(liveCalls.filter((entry) => entry.event === "permission-answer").length, 2);

  label = "denial bypasses invalid form and never retries";
  const config = await api<ProjectConfig>(`/v1/projects/${fixture.project.id}/config`);
  config.agents[0]!.role = "permission-form";
  await api(`/v1/projects/${fixture.project.id}/config`, "PUT", config);
  const deniedRun = await start(fixture.project.id, "retry", "Deny without submitting invalid fields");
  const deniedQuestion = await question(deniedRun.id);
  await answer(deniedRun.id, deniedQuestion.question.id, { choice: "allow_once", value: { count: "abc" } }, 400);
  assert.equal((await getRun(deniedRun.id)).questions[0]!.state, "pending");
  await answer(deniedRun.id, deniedQuestion.question.id, { choice: "deny", value: { count: "abc", unsubmitted: "discard this" } });
  const denied = await waitRun(deniedRun.id, "denied step failed once", (value) => value.state === "failed");
  assert.equal(denied.steps![0]!.attempts!.length, 1);
  assert.equal(denied.steps![0]!.attempts![0]!.denied, true);
  assert.deepEqual(denied.questions[0]!.response, { choice: "deny" });
  assert.equal((await journal(fixture.directory)).filter((entry) => entry.runId === denied.id && entry.event === "start").length, 1);
  observed("retry-permission", "Configured retry retains failed/successful attempt history and asks a fresh allow-once question for a reused provider request ID; guidance submission stays nonblocking and receives a native receipt. Denial bypasses unsent malformed fields, discards their payload and never retries.", { completed, denied, liveCalls });
}

async function checkpointJourney() {
  label = "checkpoint answer scope on automatic retry";
  const workflow: WorkflowDefinition = { id: "checkpoint", name: "Checkpoint", description: "Legacy checkpoint", instructions: "", stages: [{ ...stage("work", ["worker"]), execution: { retries: 1 } }] };
  const fixture = await project("checkpoint", workflow, [agent("worker", "checkpoint", true)]);
  const run = await start(fixture.project.id, "checkpoint", "Fresh checkpoint after retry");
  const first = await question(run.id);
  await answer(run.id, first.question.id, { answer: "first answer" });
  const second = await question(run.id, first.question.id);
  assert.equal(second.question.stepId, first.question.stepId); assert.notEqual(second.question.attemptId, first.question.attemptId);
  await answer(run.id, second.question.id, { answer: "second answer" });
  const completed = await waitRun(run.id, "checkpoint retry completion", (value) => value.state === "completed");
  const calls = (await Bun.file(join(fixture.directory, ".fixture", run.id, "checkpoint-calls.jsonl")).text()).trim().split("\n").map((line) => JSON.parse(line));
  assert.deepEqual(calls.map((call) => call.answer), [null, "first answer", null, "second answer"]);
  assert.deepEqual(completed.steps![0]!.attempts!.map((attempt) => attempt.status), ["failed", "completed"]);
  assert.equal(await Bun.file(join(fixture.directory, ".fixture", run.id, "checkpoint-result.txt")).text(), "second answer");
  observed("checkpoint-attempt", "A checkpoint resumes its own attempt, but a fresh automatic attempt asks again and receives only its new answer.", { completed, calls });
}

async function formJourney() {
  label = "authoritative typed form validation";
  const workflow: WorkflowDefinition = { id: "form", name: "Form", description: "Primitive form", instructions: "", stages: [stage("form", ["form"])] };
  const fixture = await project("form", workflow, [agent("form")]);
  const run = await start(fixture.project.id, "form", "Typed live form");
  const pending = await question(run.id);
  const values = { count: "3", ratio: "0", enabled: "false", tags: ["a", "b"] };
  for (const invalid of [{ ...values, count: "abc" }, { ...values, count: "3.5" }, { ...values, tags: ["unknown"] }, { ...values, unknown: "not declared" }]) {
    await answer(run.id, pending.question.id, { answers: invalid }, 400);
    assert.equal((await getRun(run.id)).questions.find((question) => question.id === pending.question.id)!.state, "pending");
  }
  await answer(run.id, pending.question.id, { answers: values });
  const completed = await waitRun(run.id, "typed form consumed", (value) => value.state === "completed");
  const received = await Bun.file(join(fixture.directory, ".fixture", run.id, "typed-form.json")).json();
  assert.deepEqual(received, { count: 3, ratio: 0, enabled: false, tags: ["a", "b"] });
  assert.deepEqual(completed.questions[0]!.response!.value, received);
  observed("typed-live-form", "Invalid numeric/integer/option/unknown-field replies return 400 without consuming the request. The real live child receives canonical integer/number/false/multiselect values after correction.", { completed, received });
}

try {
  assert.ok(scenario === "all" || scenario === "retry-permission", "Unknown FACTORY_PARALLEL_SCENARIO");
  boot();
  if (scenario === "retry-permission") await retryJourney();
  else {
    await configReplacementJourney();
    await nestedJourney();
    await failureJourney();
    await retryJourney();
    await checkpointJourney();
    await formJourney();
  }
} catch (error) {
  failure = `${label}: ${error instanceof Error ? error.stack ?? error.message : String(error)}`;
  process.exitCode = 1;
} finally {
  try { await shutdown(); } catch (error) { failure = [failure, `Shutdown: ${String(error)}`].filter(Boolean).join("\n"); process.exitCode = 1; }
  const journals = await Promise.all(roots.map(async (directory) => ({ directory, events: await journal(directory) })));
  for (const pid of pids) if (alive(pid)) {
    failure = [failure, `Owned fixture child ${pid} survived service shutdown`].filter(Boolean).join("\n"); process.exitCode = 1;
    try { process.kill(-pid, "SIGKILL"); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ESRCH") failure = [failure, `Child cleanup: ${String(error)}`].filter(Boolean).join("\n"); }
  }
  await Bun.write(join(evidence, "factory-parallel.json"), JSON.stringify({ context: { fixture: "owned startService/API, real custom duplex and JSONL children", scenario, externalWrites: false }, observations, requests, journals, failure: failure ?? null }, null, 2));
  await Bun.write(join(evidence, "result.json"), JSON.stringify({ observations: [...observations.map((item) => ({ id: item.id, status: "passed", observed: item.observed, evidence: ["factory-parallel.json"] })), ...(failure ? [{ id: "failure", status: "failed", observed: failure, evidence: ["factory-parallel.json"] }] : [])], context: { fixture: "owned-public-api-parallel", scenario, providers: "local-fixtures-only", externalWrites: false } }, null, 2));
  await rm(temporary, { recursive: true, force: true });
}
if (failure) console.error(failure); else console.log(`Factory parallel passed: ${observations.length} focused scenarios; evidence ${join(evidence, "factory-parallel.json")}`);
