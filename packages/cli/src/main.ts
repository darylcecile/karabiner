#!/usr/bin/env bun
import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";
import type { AppSettings, Document, Intervention } from "@karabiner/contracts";
import manifest from "../package.json";
import { Client, ClientError } from "./client.ts";
import { seedDemo } from "./demo.ts";
import { file, flag, jsonFile, query, required, validateFlags, type Flags } from "./arguments.ts";
import { runCommand, questionCommand } from "./runtime.ts";
import { integrationCommand } from "./integration.ts";
import { factoryCommand } from "./factory.ts";
import { availabilityCommand } from "./availability.ts";
import { print } from "./output.ts";

const HELP = `Karabiner — manage local project work through the background service

  status                              Service/API versions and capabilities
  project list | show ID | create NAME | update ID | delete ID
  task list | show ID | create TITLE | update ID | delete ID
  task comment ID MESSAGE             Record an operator comment
  doc list | show ID | create TITLE | update ID | delete ID
  tour list | show ID | create TITLE | update ID | delete ID
  config get PROJECT | set PROJECT --file config.json
  factory show|workflows PROJECT       Compiled factory or available workflows
  factory validate|compile PROJECT [--file config.json]
  factory export PROJECT [--output bundle.json | --directory DIR]
  factory import PROJECT --file bundle.json --revision N
  settings get | set --theme dark [--harness copilot]
  workspace get | set --file workspace.json
  intervention list | show ID | record MESSAGE | answer ID TEXT | dismiss ID | delete ID
  activity [--project ID] [--task ID] [--limit 100]
  stats [--project ID]                 Counts from real stored records
  templates [show ID]                  Declarative onboarding templates
  harnesses [show ID]                  Harness discovery and reported capabilities
  harnesses check --file agent.json [--project-root PATH] [--check]
  harnesses check --project ID --agent ID [--check]
  targets [show ID] [--project ID]      Target discovery
  targets --project ID --check [--file config.json]
    [--purpose command|harness] [--executable NAME]
  run start TASK [--workflow ID]       Start execution; returns the actual run state
  run list [--project ID] [--task ID] [--state STATE]
  run show RUN [--project ID] [--task ID]
  run pause|resume|cancel|retry RUN [--revision N]
  run instruct RUN TEXT [--revision N] Submit guidance; inspect its delivery mode
  question list [--run RUN] [--project ID] [--task ID]
  question show QUESTION --run RUN     Request details, transport and offered choices
  question answer QUESTION TEXT --run RUN [--revision N]
  question answer QUESTION --run RUN --choice CHOICE [--revision N]
  question answer QUESTION --run RUN --file response.json [--revision N]
  integration status PROJECT [--check]
  integration sync PROJECT [--conflict preserve-local|overwrite-local]
  integration push TASK --revision N [--request-id ID] [--fields a,b] [--create-missing]
  demo seed                           Create clearly labelled sample records
  api METHOD /v1/PATH [--file body.json]
  service serve [--port N] [--db PATH]  Explicitly run the separate service

Global: --json, --url URL, --help, --version
Create/update: --project ID, --description TEXT, --title TEXT, --name TEXT,
  --kind VALUE, --status VALUE, --priority VALUE, --workflow ID, --agent ID,
  --labels a,b, --stage ID, --due ISO_TIMESTAMP, --order NUMBER,
  --template ID, --path PATH, --repository OWNER/REPO,
  --external-ref REF, --color COLOR, --task ID, --revision N
Documents: --file Markdown-file; tours: --file tour.json; --content TEXT
Any create/update: --data JSON-file (or - for stdin) for the full request body.
Config/workspace files include the revision returned by get. IDs are complete IDs.
Factory validation/compilation can read a JSON config draft without saving or running it.
Fork/join is configured in workflow.graph.forks; for defined build/test/audit/review stages:
  [{"id":"checks","stageId":"build","branches":[{"id":"tests","entryStageId":"test"},
    {"id":"audit","entryStageId":"audit"}],"joinStageId":"review"}]
Configure each branch's edges to its join. Validate before saving the complete config.
Automatic step retry example in stage.execution: {"retries":2,"retryDelayMs":250}.
Export prints the saved factory bundle unless an explicit destination is supplied.
Export destinations must be new; --directory includes bundle.json and definition files.
Import requires the target revision from config get PROJECT. --file - reads stdin.
Run start also accepts --task TASK and --project PROJECT; run TASK is a shorthand.
Run/factory show retain complete parallel branches, stage/retry attempts and configuration.
Run instruct also accepts --message TEXT. Its delivery records distinguish native-turn,
  native-tool-boundary and next-step; readiness and transport are reported by the service.
Question list includes pending questions, approvals and permissions with run/question IDs.
Use the exact offered choice ID. --file - reads a structured answer from stdin:
  {"answer":"text"}, {"choice":"offered-id"},
  {"answers":{"field-id":"value","multi-select-id":["first","second"]}},
  or {"value":{"provider-field":"value"}} for the request's supplied schema.
Harness and target status report availability and unknown authentication honestly.
Harness checks read a full agent definition from --file (or - for stdin), or a saved agent.
  For a file's relative executable, pass --project-root PATH explicitly. Saved agents use
  their project's root; the agent file's directory is not used as the project root.
  Default checks inspect local readiness; --check explicitly probes supported local login status.
  Unknown authentication/model access stays unknown; unavailable/failed checks exit nonzero.
Targets --project --check probes the configured target and reports each prerequisite.
  --file checks an unsaved JSON config; unavailable/failed checks exit nonzero.
Integration push fields: title, description, status, priority, labels, dueAt.
Push requires the revision from task show; --create-missing explicitly allows creation.
Integration mutations are sent once. Partial/conflicted/uncertain results exit nonzero.
Delete project also deletes its tasks, documents, configuration and intervention records.
Example: karabiner task create "Review navigation" --project PROJECT_ID --json
Example: karabiner question answer QUESTION_ID "approved" --run RUN_ID --json
Example: karabiner factory validate PROJECT_ID --file fork-join-config.json --json
Example: karabiner run retry RUN_ID --revision 12 --json
Example: karabiner question show QUESTION_ID --run RUN_ID
Example: karabiner question answer QUESTION_ID --run RUN_ID --choice OFFERED_CHOICE_ID
Example: karabiner question answer QUESTION_ID --run RUN_ID --file - --json
Example: karabiner targets --project PROJECT_ID --check --purpose harness
Example: karabiner harnesses check --project PROJECT_ID --agent AGENT_ID --check
Example: karabiner harnesses check --file agent.json --project-root /path/to/project
`;
const stringOptions = ["url", "project", "task", "description", "title", "name", "kind", "status", "priority", "workflow", "agent", "stage", "due", "order", "labels", "template", "path", "repository", "external-ref", "color", "revision", "file", "data", "content", "message", "theme", "harness", "limit", "port", "db", "run", "state", "conflict", "request-id", "fields", "output", "directory", "choice", "purpose", "executable", "project-root"] as const;
const options = Object.fromEntries(stringOptions.map(name => [name, { type: "string" as const }]));
const resourceNames: Record<string, string> = { project: "projects", projects: "projects", task: "tasks", tasks: "tasks", doc: "documents", docs: "documents", document: "documents", documents: "documents", tour: "documents", tours: "documents" };
const resourceFlags: Record<string, Record<string, readonly string[]>> = {
  projects: {
    create: ["data", "name", "description", "kind", "path", "repository", "external-ref", "color", "template"],
    update: ["data", "name", "description", "kind", "path", "repository", "external-ref", "color", "revision"],
  },
  tasks: {
    list: ["project"], comment: ["message"],
    create: ["data", "project", "title", "description", "status", "priority", "workflow", "agent", "stage", "due", "order", "labels"],
    update: ["data", "title", "description", "status", "priority", "workflow", "agent", "stage", "due", "order", "labels", "revision"],
  },
  documents: {
    list: ["project", "task"],
    create: ["data", "project", "task", "title", "kind", "file", "content"],
    update: ["data", "task", "title", "file", "content", "revision"],
  },
};
function validateResourceFlags(group: string, action: string, flags: Flags): void {
  const allowed = resourceFlags[resourceNames[group]!]?.[action] ?? [];
  validateFlags(`${group} ${action}`, flags, allowed);
}
async function resourceBody(group: string, action: string, positional: string | undefined, flags: Flags): Promise<Record<string, unknown>> {
  const body = flag(flags, "data") ? await jsonFile(flag(flags, "data")!) : {};
  const mappings: Record<string, string> = { project: "projectId", task: "taskId", description: "description", title: "title", name: "name", kind: "kind", status: "status", priority: "priority", workflow: "workflowId", agent: "agentId", stage: "stageId", due: "dueAt", template: "templateId", path: "rootPath", repository: "repository", "external-ref": "externalRef", color: "color", content: "markdown" };
  for (const [key, field] of Object.entries(mappings)) if (flag(flags, key) !== undefined) body[field] = flag(flags, key);
  if (flag(flags, "labels") !== undefined) body.labels = flag(flags, "labels")!.split(",").map(value => value.trim()).filter(Boolean);
  if (flag(flags, "revision") !== undefined) body.revision = Number(flag(flags, "revision"));
  if (flag(flags, "order") !== undefined) body.sortOrder = Number(flag(flags, "order"));
  if (action === "create" && positional) body[group.startsWith("project") ? "name" : "title"] = positional;
  const isTour = group === "tour" || group === "tours";
  if (isTour && action === "create") body.kind = "code_tour";
  if (flag(flags, "file")) {
    if (isTour || body.kind === "code_tour") body.tour = await jsonFile(flag(flags, "file")!);
    else if (resourceNames[group] === "documents") body.markdown = await file(flag(flags, "file")!);
    else throw new ClientError("usage", "Use --data for JSON request bodies; --file is for document/tour content");
  }
  return body;
}
async function resourceCommand(client: Client, group: string, args: string[], flags: Flags): Promise<unknown> {
  const [action = "list", arg] = args, path = `/v1/${resourceNames[group]}`;
  if (action === "comment" && resourceNames[group] === "tasks") return client.request("POST", `${path}/${encodeURIComponent(required(arg, "ID"))}/comments`, { message: required(args[2] ?? flag(flags, "message"), "MESSAGE") });
  if (action === "list") {
    const result = await client.request<unknown[]>("GET", path + query(flags));
    return group.startsWith("tour") ? (result as Document[]).filter(value => value.kind === "code_tour") : result;
  }
  if (action === "create") return client.request("POST", path, await resourceBody(group, action, arg, flags));
  const id = encodeURIComponent(required(arg, "ID"));
  if (action === "show" || action === "get") return client.request("GET", `${path}/${id}`);
  if (action === "delete") return client.request("DELETE", `${path}/${id}`);
  if (action === "update") return client.request("PATCH", `${path}/${id}`, await resourceBody(group, action, undefined, flags));
  throw new ClientError("usage", `Unknown ${group} action: ${action}`);
}
async function interventionCommand(client: Client, args: string[], flags: Flags): Promise<unknown> {
  const [action = "list", id, answer] = args;
  if (action === "list") return client.request("GET", "/v1/interventions" + query(flags));
  if (action === "record" || action === "create") {
    const body = flag(flags, "data") ? await jsonFile(flag(flags, "data")!) : { projectId: required(flag(flags, "project"), "--project"), taskId: flag(flags, "task") ?? null, kind: flag(flags, "kind") ?? "instruction", message: required(flag(flags, "message") ?? id, "MESSAGE") };
    return client.request("POST", "/v1/interventions", body);
  }
  const path = `/v1/interventions/${encodeURIComponent(required(id, "ID"))}`;
  if (action === "show" || action === "get") return client.request("GET", path);
  if (action === "delete") return client.request("DELETE", path);
  if (action === "answer" || action === "dismiss") {
    const current = await client.request<Intervention>("GET", path);
    return client.request("PATCH", path, { revision: current.revision, status: action === "answer" ? "answered" : "dismissed", ...(action === "answer" ? { response: required(answer ?? flag(flags, "message"), "TEXT") } : {}) });
  }
  throw new ClientError("usage", `Unknown intervention action: ${action}`);
}
async function settingCommand(client: Client, group: string, args: string[], flags: Flags): Promise<unknown> {
  const [action = "get", projectId] = args;
  const path = group === "config" ? `/v1/projects/${encodeURIComponent(required(projectId, "PROJECT"))}/config` : `/v1/${group}`;
  if (action === "get" || action === "show") return client.request("GET", path);
  if (action !== "set") throw new ClientError("usage", `Unknown ${group} action: ${action}`);
  const inputPath = flag(flags, "file") ?? flag(flags, "data");
  if (inputPath) return client.request("PUT", path, await jsonFile(inputPath));
  if (group !== "settings") throw new ClientError("usage", "--file JSON-file is required");
  const current = await client.request<AppSettings>("GET", path);
  return client.request("PUT", path, { ...current, ...(flag(flags, "theme") ? { theme: flag(flags, "theme") } : {}), ...(flag(flags, "harness") ? { defaultHarness: flag(flags, "harness") } : {}) });
}
async function serve(flags: Flags): Promise<void> {
  const supplied = process.env.KARABINER_SERVICE_BIN;
  const command = supplied ? [supplied] : import.meta.path.startsWith("/$bunfs/") ? ["karabiner-service"] : [process.execPath, fileURLToPath(new URL("../../service/src/main.ts", import.meta.url))];
  for (const key of ["port", "db"]) if (flag(flags, key)) command.push(`--${key}`, flag(flags, key)!);
  const child = Bun.spawn(command, { stdin: "inherit", stdout: "inherit", stderr: "inherit" });
  const stop = () => child.kill("SIGTERM"); process.once("SIGINT", stop); process.once("SIGTERM", stop);
  process.exitCode = await child.exited;
  process.removeListener("SIGINT", stop); process.removeListener("SIGTERM", stop);
}
async function command(client: Client, args: string[], flags: Flags): Promise<unknown> {
  const [group = "status", ...rest] = args;
  if (group === "factory") return factoryCommand(client, rest, flags);
  if (group === "run" || group === "runs") return runCommand(client, rest, flags);
  if (group === "question" || group === "questions") return questionCommand(client, rest, flags);
  if (group === "integration" || group === "integrations") return integrationCommand(client, rest, flags);
  if (group === "harnesses" || group === "targets") return availabilityCommand(client, group, rest, flags);
  if (resourceNames[group]) validateResourceFlags(group, rest[0] ?? "list", flags);
  if (group === "stats" && flag(flags, "task") !== undefined) throw new ClientError("usage", "stats supports --project, not --task");
  const meta = await client.connect();
  if (group === "status") return meta;
  if (resourceNames[group]) return resourceCommand(client, group, rest, flags);
  if (["config", "settings", "workspace"].includes(group)) return settingCommand(client, group, rest, flags);
  if (group === "intervention" || group === "interventions") return interventionCommand(client, rest, flags);
  if (["activity", "stats", "templates"].includes(group)) {
    const value = await client.request<unknown>("GET", `/v1/${group}${query(flags)}`);
    if (group === "templates" && rest[0] === "show") {
      const selected = (value as { id: string }[]).find(item => item.id === required(rest[1], "ID"));
      if (!selected) throw new ClientError("not_found", "Template was not found"); return selected;
    }
    return value;
  }
  if (group === "demo" && rest[0] === "seed") return seedDemo(client);
  if (group === "api") {
    const method = required(rest[0], "METHOD").toUpperCase(), path = required(rest[1], "PATH");
    if (!path.startsWith("/v1/")) throw new ClientError("usage", "API path must start with /v1/");
    const inputPath = flag(flags, "file") ?? flag(flags, "data");
    return client.request(method, path, inputPath ? await jsonFile(inputPath) : undefined);
  }
  throw new ClientError("usage", `Unknown command: ${group}. Run --help for usage.`);
}
let json = process.argv.includes("--json");
try {
  const parsed = parseArgs({ args: process.argv.slice(2), options: { ...options, json: { type: "boolean" }, check: { type: "boolean" }, "create-missing": { type: "boolean" }, help: { type: "boolean", short: "h" }, version: { type: "boolean" } }, allowPositionals: true, strict: true });
  const flags: Flags = parsed.values; json = flags.json === true;
  if (flags.help) console.log(HELP);
  else if (flags.version) print({ version: manifest.version }, json);
  else if (parsed.positionals[0] === "service" && parsed.positionals[1] === "serve") await serve(flags);
  else print(await command(new Client(flag(flags, "url")), parsed.positionals, flags), json);
} catch (error) {
  const code = error instanceof ClientError ? error.code : "error", message = error instanceof Error ? error.message : String(error);
  const details = error instanceof ClientError ? error.details : undefined;
  if (json) console.log(JSON.stringify({ error: { code, message, ...(details === undefined ? {} : { details }) } }));
  else { console.error(`Karabiner: ${message}`); if (details !== undefined) console.error(JSON.stringify(details, null, 2)); }
  process.exitCode = 1;
}
