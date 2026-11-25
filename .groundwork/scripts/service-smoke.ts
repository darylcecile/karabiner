import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { Activity, AppSettings, Document, Intervention, Project, ProjectConfig, ServiceMeta, Statistics, Task, Template, WorkspaceState } from "../../packages/contracts/src/index.ts";

const root = resolve(import.meta.dir, "../..");
const evidence = process.env.GROUNDWORK_EVIDENCE_DIR ?? join(root, ".groundwork/tmp/service-smoke");
await mkdir(evidence, { recursive: true });
const temporary = await mkdtemp(join(tmpdir(), "karabiner-smoke-"));
const serviceBin = join(temporary, "karabiner-service"), cliBin = join(temporary, "karabiner"), db = join(temporary, "state.sqlite");
const observations: { name: string; observed: unknown }[] = [];
let service: Bun.Subprocess<"ignore", "pipe", "pipe"> | undefined;
let eventReader: ReadableStreamDefaultReader<Uint8Array> | undefined;
let url = "";
let failure: string | undefined;
let cliCalls = 0;
async function start(): Promise<void> {
  service = Bun.spawn([serviceBin, "--port", "0", "--db", db], { stdin: "ignore", stdout: "pipe", stderr: "pipe" });
  const child = service, reader = child.stdout.getReader();
  const timeout = setTimeout(() => child.kill("SIGKILL"), 10_000);
  let output = "";
  try {
    while (!output.includes("\n")) {
      const chunk = await reader.read();
      if (chunk.done) throw new Error(`Service did not become ready: ${await new Response(child.stderr).text()}`);
      output += new TextDecoder().decode(chunk.value);
    }
    const ready = JSON.parse(output.split("\n")[0]!); assert.equal(ready.event, "ready"); url = ready.url;
  } finally { clearTimeout(timeout); reader.releaseLock(); }
}
async function stop(): Promise<void> {
  if (!service) return;
  const child = service; service = undefined;
  child.kill("SIGTERM"); const timeout = setTimeout(() => child.kill("SIGKILL"), 5_000);
  try { assert.equal(await child.exited, 0, "Service shuts down cleanly"); }
  finally { clearTimeout(timeout); }
}
async function cli<T>(args: string[], expectedError?: string): Promise<T> {
  cliCalls++;
  const child = Bun.spawn([cliBin, "--url", url, "--json", ...args], { stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr, exit] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
  const result = JSON.parse(stdout);
  if (expectedError) { assert.equal(exit, 1, stderr); assert.equal(result.error?.code, expectedError, stdout); return result as T; }
  assert.equal(exit, 0, `${args.join(" ")}: ${stdout}\n${stderr}`); assert.ok("data" in result);
  return result.data as T;
}
async function body(name: string, value: unknown): Promise<string> { const path = join(temporary, `${name}.json`); await Bun.write(path, JSON.stringify(value)); return path; }
function observed(name: string, value: unknown): void { observations.push({ name, observed: value }); }

try {
  const builds = await Promise.all([
    Bun.build({ entrypoints: [join(root, "packages/service/src/main.ts")], compile: { outfile: serviceBin } }),
    Bun.build({ entrypoints: [join(root, "packages/cli/src/main.ts")], compile: { outfile: cliBin } }),
  ]);
  for (const build of builds) assert.equal(build.success, true, build.logs.join("\n"));
  observed("independent executables compiled", ["karabiner-service", "karabiner"]);
  await start();
  const meta = await cli<ServiceMeta>(["status"]); assert.equal(meta.apiVersion, 1); assert.equal(meta.capabilities.storage, true); assert.equal(meta.capabilities.harnessExecution, true);
  assert.deepEqual(await cli(["project", "list"]), []);
  const project = await cli<Project>(["project", "create", "Roundtrip fixture", "--kind", "github", "--repository", "example/fixture"]);
  const task = await cli<Task>(["task", "create", "Inspect a stored task", "--project", project.id]);
  const document = await cli<Document>(["doc", "create", "Architecture notes", "--project", project.id, "--task", task.id, "--content", "# Context\n\n**Persistent** documentation."]);
  const template = await cli<Template>(["templates", "show", "software-delivery"]);
  template.config.agents[0]!.harness = "custom";
  template.config.agents[0]!.executable = "/example/local-harness";
  template.config.agents[0]!.args = ["--interactive"];
  const config = await cli<ProjectConfig>(["config", "get", project.id]);
  const savedConfig = await cli<ProjectConfig>(["config", "set", project.id, "--file", await body("config", { ...template.config, revision: config.revision })]);
  assert.equal(savedConfig.workflows[0]?.id, "delivery");

  const events = await fetch(url + "/v1/events"); eventReader = events.body!.getReader();
  assert.match(new TextDecoder().decode((await eventReader.read()).value), /event: ready/);
  const updated = await cli<Task>(["task", "update", task.id, "--status", "done", "--workflow", "delivery", "--stage", "review", "--agent", "builder", "--due", "2026-12-01T10:00:00Z", "--order", "2.5", "--data", await body("subtasks", { subtasks: [{ id: "check", title: "Read the implementation", completed: true }] })]);
  assert.equal(updated.executionState, "not_started"); assert.equal(updated.status, "done");
  assert.equal(updated.stageId, "review"); assert.equal(updated.sortOrder, 2.5); assert.equal(updated.subtasks[0]?.completed, true);
  const event = new TextDecoder().decode((await eventReader.read()).value); assert.match(event, /event: change/); assert.ok(event.includes(`/v1/tasks/${task.id}`));
  await eventReader.cancel(); eventReader = undefined;
  observed("CLI changes emit GUI invalidation", { status: updated.status, event });
  const comment = await cli<Activity>(["task", "comment", task.id, "Operator reviewed the stored task"]);
  assert.equal(comment.kind, "task.comment"); assert.equal(comment.message, "Operator reviewed the stored task");

  const revised = await cli<Document>(["doc", "update", document.id, "--revision", String(document.revision), "--content", "# Saved after editing\n\n- [x] Persistent across restart"]);
  await cli(["doc", "update", document.id, "--revision", String(document.revision), "--content", "Stale edit"], "conflict");
  await cli(["task", "update", task.id, "--status", "invented"], "invalid_request");
  const settings = await cli<AppSettings>(["settings", "set", "--theme", "dark"]); assert.equal(settings.theme, "dark");
  const workspace = await cli<WorkspaceState>(["workspace", "get"]);
  const savedWorkspace = await cli<WorkspaceState>(["workspace", "set", "--file", await body("workspace", { ...workspace, selectedProjectId: project.id,
    tabs: [{ id: "project-tab", kind: "project", resourceId: project.id, projectId: project.id, title: project.name }, { id: "document-tab", kind: "document", resourceId: document.id, projectId: project.id, title: document.title }],
    activeTabId: "document-tab", recentlyClosed: [{ id: "task-tab", kind: "task", resourceId: task.id, projectId: project.id, title: task.title }] })]);
  const intervention = await cli<Intervention>(["intervention", "record", "Which verification should we run?", "--project", project.id, "--task", task.id, "--kind", "question"]);
  const answered = await cli<Intervention>(["intervention", "answer", intervention.id, "Use the focused checks"]);
  assert.equal(answered.status, "answered"); assert.equal(answered.executionStatus, "not_implemented");
  assert.deepEqual(await cli(["api", "GET", "/v1/runs"]), []);
  await cli(["api", "GET", "/v1/runs/unrecorded-run"], "not_found");
  const stats = await cli<Statistics>(["stats", "--project", project.id]);
  assert.equal(stats.taskCount, 1); assert.equal(stats.completedTaskCount, 1); assert.equal(stats.completionRate, 1); assert.equal(stats.runCount, 0); assert.equal(stats.executionAvailable, true);
  observed("persisted CRUD, settings, intents and derived statistics", { project, task: updated, document: revised, configRevision: savedConfig.revision, settings, workspace: savedWorkspace, intervention: answered, stats });
  const invalidFlag = await cli(["task", "update", task.id, "--project", project.id], "usage");
  const invalidStatsFlag = await cli(["stats", "--task", task.id], "usage");
  const invalidStatsQuery = await cli(["api", "GET", `/v1/stats?taskId=${task.id}`], "invalid_request");
  observed("unsupported command flags and statistics filters", { invalidFlag, invalidStatsFlag, invalidStatsQuery });

  const demo = await cli<{ demo: boolean; project: Project; tourId: string }>(["demo", "seed"]); assert.equal(demo.demo, true); assert.match(demo.project.name, /Demo/);
  const tour = await cli<Document>(["tour", "show", demo.tourId]);
  assert.ok(tour.tour!.chapters[0]!.blocks.some(block => block.kind === "diagram")); assert.equal(tour.tour!.commits?.length, 2);
  const tourContent = { ...tour.tour!, review: { ...tour.tour!.review!, reviewedChapterIds: ["selection"], reviewedFilePaths: ["src/workspace.ts"] } };
  await cli(["tour", "update", demo.tourId, "--revision", String(tour.revision), "--file", await body("tour", tourContent)]);
  await stop(); await start();
  assert.equal((await cli<Document>(["doc", "show", document.id])).markdown, revised.markdown);
  assert.deepEqual(await cli(["workspace", "get"]), savedWorkspace);
  assert.equal((await cli<AppSettings>(["settings", "get"])).theme, "dark");
  assert.equal((await cli<ProjectConfig>(["config", "get", project.id])).revision, savedConfig.revision);
  assert.deepEqual(await cli<Task>(["task", "show", task.id]), updated);
  assert.equal((await cli<Intervention>(["intervention", "show", intervention.id])).response, "Use the focused checks");
  const restoredTour = await cli<Document>(["tour", "show", demo.tourId]); assert.deepEqual(restoredTour.tour, tourContent);
  observed("SQLite roundtrip across a fresh service process", { restoredDocumentId: document.id, workspace: savedWorkspace, restoredTour: restoredTour.tour });

  await cli(["task", "delete", task.id]);
  assert.equal((await cli<Document>(["doc", "show", document.id])).taskId, null);
  await cli(["project", "delete", project.id]);
  await cli(["doc", "show", document.id], "not_found");
  await cli(["project", "delete", demo.project.id]);
  assert.deepEqual(await cli(["project", "list"]), []);
  observed("relational deletion", "Deleting a task detaches authored documents; deleting a project cascades owned records.");
  await stop();
  await cli(["status"], "service_unavailable");
  const incompatible = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch: () => Response.json({ data: { apiVersion: 99, serviceVersion: "future" } }) });
  try { url = `http://127.0.0.1:${incompatible.port}`; await cli(["project", "list"], "incompatible_version"); }
  finally { await incompatible.stop(true); }
  observed("independent update and offline errors", ["incompatible_version", "service_unavailable"]);
} catch (error) {
  failure = error instanceof Error ? error.stack ?? error.message : String(error); process.exitCode = 1;
} finally {
  if (eventReader) await eventReader.cancel().catch(() => {});
  try { await stop(); } catch (error) { failure = [failure, `Cleanup: ${String(error)}`].filter(Boolean).join("\n"); process.exitCode = 1; }
  await Bun.write(join(evidence, "roundtrip.json"), JSON.stringify({ fixture: "isolated compiled CLI/service with SQLite", cliCalls, observations, failure: failure ?? null }, null, 2));
  await Bun.write(join(evidence, "result.json"), JSON.stringify({ observations: [{ status: failure ? "failed" : "passed", observed: failure ?? `${cliCalls} CLI calls verified persistence across restart, revision conflicts, typed tours, saved workspace state, SSE invalidation, and available execution with no runs created by CRUD.`, evidence: ["roundtrip.json"] }] }, null, 2));
  await rm(temporary, { recursive: true, force: true });
}
if (failure) console.error(failure); else console.log(`Backend smoke passed: ${cliCalls} CLI calls; evidence ${join(evidence, "roundtrip.json")}`);
