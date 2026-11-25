import type { ExecutionTargetInfo, HarnessInfo, HarnessReadiness, Project, ProjectConfig, TargetReadiness } from "@karabiner/contracts";
import { flag, jsonFile, maxArgs, query, required, validateFlags, type Flags } from "./arguments.ts";
import { Client, ClientError } from "./client.ts";

async function checkHarness(client: Client, args: string[], flags: Flags): Promise<HarnessReadiness> {
  maxArgs(args, 1, "harnesses check");
  validateFlags("harnesses check", flags, ["file", "project", "agent", "check", "project-root"]);
  const inputPath = flag(flags, "file");
  let projectRoot = flag(flags, "project-root");
  if (projectRoot !== undefined) {
    if (inputPath === undefined) throw new ClientError("usage", "--project-root is only valid with --file; saved agents use their project's root");
    required(projectRoot, "--project-root PATH");
  }
  if (inputPath !== undefined && (flag(flags, "project") !== undefined || flag(flags, "agent") !== undefined)) throw new ClientError("usage", "Choose --file agent.json or --project PROJECT --agent AGENT");
  const projectId = inputPath === undefined ? required(flag(flags, "project"), "--file agent.json or --project PROJECT") : undefined;
  const agentId = projectId === undefined ? undefined : required(flag(flags, "agent"), "--agent AGENT");
  let agent: unknown = inputPath === undefined ? undefined : await jsonFile(required(inputPath, "--file agent.json"));
  await client.connect();
  if (projectId !== undefined) {
    const path = `/v1/projects/${encodeURIComponent(projectId)}`;
    const [project, config] = await Promise.all([client.request<Project>("GET", path), client.request<ProjectConfig>("GET", `${path}/config`)]);
    agent = config.agents.find(value => value.id === agentId);
    if (!agent) throw new ClientError("not_found", "Agent was not found in this project configuration");
    projectRoot = project.rootPath ?? undefined;
  }
  const result = await client.request<HarnessReadiness>("POST", "/v1/harnesses/check", { agent, check: flags.check === true, projectRoot }, 120_000);
  if (!result.executionAvailable || result.status === "unavailable" || result.status === "error" || result.checks.some(check => check.status === "failed")) process.exitCode = 1;
  return result;
}

async function checkTarget(client: Client, flags: Flags): Promise<TargetReadiness> {
  const projectId = required(flag(flags, "project"), "--project PROJECT"), purpose = flag(flags, "purpose");
  if (purpose !== undefined && purpose !== "command" && purpose !== "harness") throw new ClientError("usage", "--purpose must be command or harness");
  const inputPath = flag(flags, "file"), executable = flag(flags, "executable");
  if (executable !== undefined) required(executable.trim(), "--executable NAME");
  const body = { check: true, purpose, executable, config: inputPath === undefined ? undefined : await jsonFile(required(inputPath, "--file config.json")) };
  await client.connect();
  const result = await client.request<TargetReadiness>("POST", `/v1/projects/${encodeURIComponent(projectId)}/execution-target/check`, body, 120_000);
  if (!result.available || result.checks.some(check => check.status === "failed")) process.exitCode = 1;
  return result;
}

export async function availabilityCommand(client: Client, group: "harnesses" | "targets", args: string[], flags: Flags): Promise<unknown> {
  const [action = "list", id] = args;
  if (group === "harnesses" && action === "check") return checkHarness(client, args, flags);
  if (action !== "list" && action !== "show" && action !== "status") throw new ClientError("usage", `Unknown ${group} action: ${action}`);
  maxArgs(args, action === "show" ? 2 : 1, `${group} ${action}`);
  validateFlags(`${group} ${action}`, flags, group === "targets" ? ["project", "check", "file", "purpose", "executable"] : []);
  if (flags.check === true) {
    if (action === "show") throw new ClientError("usage", "--check reports the configured project target; use targets --project PROJECT --check");
    return checkTarget(client, flags);
  }
  if (["file", "purpose", "executable"].some(name => flag(flags, name) !== undefined)) throw new ClientError("usage", "Target --file, --purpose and --executable require --project PROJECT --check");
  const selected = action === "show" ? required(id, "ID") : undefined;
  const path = group === "targets" ? "/v1/execution-targets" : "/v1/harnesses";
  await client.connect();
  const records = await client.request<(HarnessInfo | ExecutionTargetInfo)[]>("GET", path + query(flags, [["project", "projectId"]]), undefined, 120_000);
  if (selected === undefined) return records;
  const record = records.find(value => value.id === selected);
  if (!record) throw new ClientError("not_found", `${group === "targets" ? "Execution target" : "Harness"} was not found: ${selected}`);
  return record;
}
