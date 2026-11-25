/**
 * Seeds the native factory journey against an owned service (KARABINER_SERVICE_URL):
 * a git fixture project whose agents/commands use the deterministic custom harness,
 * a coherent pipeline (build → check → review ↺ build → decision with question + approval)
 * one task, plus a one-stage "hold" workflow whose agent streams progress until released. Prints { projectId, taskId, workflowId, revision, factoryId } as JSON.
 */
import { mkdir } from "node:fs/promises";
import { join, resolve } from "node:path";
import type { AgentDefinition, FactoryDefinition, Project, ProjectConfig, Task, WorkflowDefinition } from "../../packages/contracts/src/index.ts";

const url = process.env.KARABINER_SERVICE_URL;
const directory = process.argv[2];
if (!url || !directory) throw new Error("usage: KARABINER_SERVICE_URL=… bun factory-client-fixture.ts <project-dir>");
const harness = resolve(import.meta.dir, "fixtures/factory-harness.ts");

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(url + path, { method, headers: body === undefined ? undefined : { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
  const reply = await response.json() as { data?: T; error?: unknown };
  if (!response.ok || reply.data === undefined) throw new Error(`${method} ${path}: ${response.status} ${JSON.stringify(reply.error)}`);
  return reply.data;
}
async function git(args: string[]): Promise<void> {
  const child = Bun.spawn(["git", "-c", "user.name=Karabiner fixture", "-c", "user.email=fixture@localhost", "-c", "commit.gpgsign=false", ...args], { cwd: directory, stdout: "ignore", stderr: "inherit" });
  if (await child.exited !== 0) throw new Error(`git ${args.join(" ")} failed`);
}

const agent = (id: string, role = id): AgentDefinition => ({ id, name: id, role, harness: "custom", model: "fixture-local", instructions: "Use only the supplied fixture context.", instructionIds: ["project"], skillIds: ["fixture"], mode: role === "question" ? "interactive" : "autonomous", executable: process.execPath, args: [harness] });
const command = (id: string) => ({ id, name: id, executable: process.execPath, args: [harness, "command", id], timeoutMs: 10_000 });
const stage = (id: string, agentIds: string[] = []) => ({ id, name: id, instructions: "", agentIds, returnToStageIds: [] as string[] });

const workflow: WorkflowDefinition = {
  id: "delivery", name: "Delivery", description: "Build, check, review and a human decision", instructions: "",
  stages: [
    { ...stage("build", ["builder"]), maxVisits: 2 },
    { ...stage("check"), maxVisits: 2, execution: { commands: [command("check")] } },
    { ...stage("repair", ["repair"]), maxVisits: 1 },
    { ...stage("review", ["review-first"]), returnToStageIds: ["build"], maxVisits: 2 },
    { ...stage("decision", ["question"]), execution: { gates: [{ id: "ship", kind: "approval", prompt: "Approve this observed fixture output?" }], after: [command("finish")] } },
  ],
  graph: { entryStageId: "build", maxTransitions: 12, edges: [
    { id: "build-check", sourceStageId: "build", targetStageId: "check", on: "success" },
    { id: "check-repair", sourceStageId: "check", targetStageId: "repair", on: "failure" },
    { id: "check-review", sourceStageId: "check", targetStageId: "review", on: "success" },
    { id: "repair-review", sourceStageId: "repair", targetStageId: "review", on: "success" },
    { id: "review-return", sourceStageId: "review", targetStageId: "build", on: "return" },
    { id: "review-decision", sourceStageId: "review", targetStageId: "decision", on: "success" },
  ] },
  layout: { positions: { build: { x: 0, y: 0 }, check: { x: 260, y: 0 }, repair: { x: 260, y: 180 }, review: { x: 520, y: 0 }, decision: { x: 780, y: 0 } } },
};

// Separate one-stage workflow whose agent stays running until released, so the client
// receives live run updates while a canvas gesture is held.
const hold: WorkflowDefinition = {
  id: "hold", name: "Hold", description: "Stays running until released", instructions: "",
  stages: [stage("hold", ["stream-hold"])],
  graph: { entryStageId: "hold", maxTransitions: 1, edges: [] },
  layout: { positions: { hold: { x: 0, y: 0 } } },
};

await mkdir(join(directory, "skills/fixture"), { recursive: true });
await Bun.write(join(directory, "instructions.md"), "project-context-marker: preserve the fixture's real effects\n");
await Bun.write(join(directory, "skills/fixture/SKILL.md"), "fixture-skill-marker: report only observed results\n");
await Bun.write(join(directory, "result.txt"), "Before execution\n");
await Bun.write(join(directory, ".gitignore"), ".fixture/\n");
await git(["init", "--quiet"]);
await git(["add", "."]);
await git(["commit", "--quiet", "-m", "Fixture baseline"]);

const project = await request<Project>("POST", "/v1/projects", { name: "Factory fixture", rootPath: directory });
const current = await request<ProjectConfig>("GET", `/v1/projects/${project.id}/config`);
const saved = await request<ProjectConfig>("PUT", `/v1/projects/${project.id}/config`, {
  revision: current.revision,
  instructions: [{ id: "project", name: "Project fixture", content: "resolve this file at capture", path: "instructions.md" }],
  skills: [{ id: "fixture", name: "Fixture", description: "Deterministic fixture context", content: "resolve this skill at capture", path: "skills/fixture/SKILL.md" }],
  agents: [agent("builder", "build"), agent("repair"), agent("review-first"), agent("question"), agent("stream-hold")],
  workflows: [workflow, hold], executionTarget: "host",
  scheduler: { enabled: false, workflowId: workflow.id, statuses: ["todo", "done"], pollIntervalMs: 1_000, maxConcurrentRuns: 1 },
});
const factory = await request<FactoryDefinition>("GET", `/v1/projects/${project.id}/factory`);
const task = await request<Task>("POST", "/v1/tasks", { projectId: project.id, title: "Deliver the fixture", workflowId: workflow.id, agentId: "builder" });
console.log(JSON.stringify({ projectId: project.id, taskId: task.id, workflowId: workflow.id, revision: saved.revision, factoryId: factory.id }));
