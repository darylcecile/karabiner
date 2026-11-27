import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { AgentDefinition, Project, ProjectConfig, StoredRun, Task } from "../../../packages/contracts/src/index.ts";

export async function waitFor<T>(label: string, read: () => Promise<T>, accepts: (value: T) => boolean, timeoutMs = 20_000): Promise<T> {
  const end = Date.now() + timeoutMs;
  let value: T;
  do { value = await read(); if (accepts(value)) return value; await Bun.sleep(35); } while (Date.now() < end);
  throw new Error(`${label}: ${JSON.stringify(value!).slice(-6_000)}`);
}

export async function folder(path: string): Promise<string> {
  await mkdir(join(path, ".fixture"), { recursive: true });
  await writeFile(join(path, "README.md"), "Owned native harness fixture\n");
  await writeFile(join(path, ".gitignore"), ".fixture/\n");
  for (const args of [["init", "--quiet"], ["add", "."], ["-c", "user.name=Fixture", "-c", "user.email=fixture@localhost", "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "Fixture baseline"]]) {
    const child = Bun.spawn(["git", ...args], { cwd: path, stdout: "pipe", stderr: "pipe" });
    const [code, stderr] = await Promise.all([child.exited, new Response(child.stderr).text(), new Response(child.stdout).text()]);
    assert.equal(code, 0, stderr);
  }
  return path;
}

export async function wireLog(path: string): Promise<any[]> {
  try { return (await readFile(join(path, ".fixture/native-wire.jsonl"), "utf8")).trim().split("\n").filter(Boolean).map((line) => JSON.parse(line)); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return []; throw error; }
}

export function alive(pid: number): boolean {
  try { process.kill(pid, 0); return true; } catch (error) { if ((error as NodeJS.ErrnoException).code === "ESRCH") return false; throw error; }
}

export class FixtureApi {
  readonly logs = { stdout: "", stderr: "" };
  private child?: Bun.Subprocess<"ignore", "pipe", "pipe">;
  private readers: Promise<void>[] = [];
  url = "";
  constructor(readonly binary: string, readonly db: string, readonly home: string, readonly live = false) {}
  async start() {
    this.url = "";
    await mkdir(this.home, { recursive: true });
    const env = { ...process.env };
    if (!this.live) {
      env.HOME = this.home;
      for (const key of ["COPILOT_HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "GH_TOKEN", "GITHUB_TOKEN", "COPILOT_GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY", "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN", "AWS_BEARER_TOKEN_BEDROCK"]) delete env[key];
    }
    const child = Bun.spawn([this.binary, "--port", "0", "--db", this.db], { env, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
    this.child = child;
    this.readers = [(async () => { for await (const chunk of child.stdout) { this.logs.stdout += new TextDecoder().decode(chunk); for (const line of this.logs.stdout.split("\n")) { try { const value = JSON.parse(line); if (value.event === "ready" && value.pid === child.pid) this.url = value.url; } catch { /* partial line */ } } } })(), (async () => { this.logs.stderr = await new Response(child.stderr).text(); })()];
    await waitFor("compiled fixture service", async () => { if (child.exitCode !== null) throw new Error(`Service exited: ${this.logs.stderr}`); return this.url; }, Boolean);
  }
  async raw(path: string, method = "GET", body?: unknown) {
    const response = await fetch(this.url + path, { method, headers: body === undefined ? {} : { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(15_000) });
    return { status: response.status, body: await response.json() as { data: any; error?: unknown } };
  }
  async call<T>(path: string, method = "GET", body?: unknown): Promise<T> {
    const response = await this.raw(path, method, body);
    assert.ok(response.status >= 200 && response.status < 300, `${method} ${path}: ${JSON.stringify(response)}`);
    return response.body.data as T;
  }
  run(id: string) { return this.call<StoredRun>(`/v1/runs/${id}`); }
  async project(root: string, agent: AgentDefinition, targetOptions?: ProjectConfig["targetOptions"]) {
    const project = await this.call<Project>("/v1/projects", "POST", { name: agent.name, rootPath: root });
    const initial = await this.call<ProjectConfig>(`/v1/projects/${project.id}/config`);
    const config: ProjectConfig = { revision: initial.revision, executionTarget: targetOptions ? "mxc" : "host", targetOptions, agents: [agent], skills: [], instructions: [], workflows: [{ id: "native", name: "Native protocol", description: "Owned fixture", instructions: "Only operate on this owned fixture.", stages: [{ id: "step", name: "Native invocation", agentIds: [agent.id], instructions: "Return the observed result.", returnToStageIds: [], execution: { timeoutMs: this.live ? 120_000 : 30_000, retries: this.live ? 0 : 2 } }] }] };
    await this.call(`/v1/projects/${project.id}/config`, "PUT", config);
    const task = await this.call<Task>("/v1/tasks", "POST", { projectId: project.id, title: "Exercise native harness", workflowId: "native" });
    return { project, task };
  }
  async stop() {
    if (!this.child) return;
    const child = this.child; this.child = undefined;
    child.kill("SIGTERM");
    const timer = setTimeout(() => child.kill("SIGKILL"), 5_000);
    try { assert.equal(await child.exited, 0, this.logs.stderr); } finally { clearTimeout(timer); await Promise.all(this.readers); }
  }
}
