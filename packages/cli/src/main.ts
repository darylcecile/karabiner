#!/usr/bin/env bun
import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";
import type { AppSettings, Document, Intervention } from "@karabiner/contracts";
import manifest from "../package.json";
import { Client, ClientError } from "./client.ts";
import { seedDemo } from "./demo.ts";

const HELP = `Karabiner — manage local project work through the background service

  status                              Service/API versions and capabilities
  project list | show ID | create NAME | update ID | delete ID
  task list | show ID | create TITLE | update ID | delete ID
  task comment ID MESSAGE             Record an operator comment
  doc list | show ID | create TITLE | update ID | delete ID
  tour list | show ID | create TITLE | update ID | delete ID
  config get PROJECT | set PROJECT --file config.json
  settings get | set --theme dark [--harness copilot]
  workspace get | set --file workspace.json
  intervention list | show ID | record MESSAGE | answer ID TEXT | dismiss ID | delete ID
  activity [--project ID] [--task ID] [--limit 100]
  stats [--project ID]                 Counts from real stored records
  templates [show ID]                  Declarative onboarding templates
  harnesses | targets                 Explicit adapter availability
  run TASK                            Reports execution not implemented
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
Delete project also deletes its tasks, documents, configuration and intervention records.
Example: karabiner task create "Review navigation" --project PROJECT_ID --json
`;
const stringOptions = ["url", "project", "task", "description", "title", "name", "kind", "status", "priority", "workflow", "agent", "stage", "due", "order", "labels", "template", "path", "repository", "external-ref", "color", "revision", "file", "data", "content", "message", "theme", "harness", "limit", "port", "db"] as const;
const options = Object.fromEntries(stringOptions.map(name => [name, { type: "string" as const }]));
type Flags = Record<string, string | boolean | undefined>;
function flag(flags: Flags, name: string): string | undefined { const value = flags[name]; return typeof value === "string" ? value : undefined; }
function required(value: string | undefined, label: string): string { if (!value) throw new ClientError("usage", `${label} is required. Run --help for usage.`); return value; }
async function file(path: string): Promise<string> { return path === "-" ? await Bun.stdin.text() : await Bun.file(path).text(); }
async function jsonFile(path: string): Promise<Record<string, unknown>> {
  const value: unknown = JSON.parse(await file(path));
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new ClientError("usage", "JSON input must be an object");
  return value as Record<string, unknown>;
}
function query(flags: Flags): string {
  const params = new URLSearchParams();
  for (const [option, key] of [["project", "projectId"], ["task", "taskId"], ["limit", "limit"]]) if (flag(flags, option!)) params.set(key!, flag(flags, option!)!);
  return params.size ? `?${params}` : "";
}
function print(value: unknown, json: boolean): void {
  if (json) { console.log(JSON.stringify({ data: value })); return; }
  if (Array.isArray(value)) {
    if (!value.length) { console.log("No records."); return; }
    for (const item of value) console.log([item.id ?? item.sha ?? "", item.name ?? item.title ?? item.message ?? "", item.status ?? item.kind ?? ""].join("  ").replace(/[\x00-\x1f\x7f]/g, " "));
  } else console.log(JSON.stringify(value, null, 2));
}
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
  for (const name of Object.keys(flags)) {
    if (name !== "url" && name !== "json" && !allowed.includes(name)) throw new ClientError("usage", `--${name} is not valid for ${group} ${action}`);
  }
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
  if (resourceNames[group]) validateResourceFlags(group, rest[0] ?? "list", flags);
  if (group === "stats" && flag(flags, "task") !== undefined) throw new ClientError("usage", "stats supports --project, not --task");
  const meta = await client.connect();
  if (group === "status") return meta;
  if (resourceNames[group]) return resourceCommand(client, group, rest, flags);
  if (["config", "settings", "workspace"].includes(group)) return settingCommand(client, group, rest, flags);
  if (group === "intervention" || group === "interventions") return interventionCommand(client, rest, flags);
  if (["activity", "stats", "harnesses", "templates", "targets"].includes(group)) {
    const value = await client.request<unknown>("GET", `/v1/${group === "targets" ? "execution-targets" : group}${query(flags)}`);
    if (group === "templates" && rest[0] === "show") {
      const selected = (value as { id: string }[]).find(item => item.id === required(rest[1], "ID"));
      if (!selected) throw new ClientError("not_found", "Template was not found"); return selected;
    }
    return value;
  }
  if (group === "run") return client.request("POST", `/v1/tasks/${encodeURIComponent(required(rest[0], "TASK"))}/run`, {});
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
  const parsed = parseArgs({ args: process.argv.slice(2), options: { ...options, json: { type: "boolean" }, help: { type: "boolean", short: "h" }, version: { type: "boolean" } }, allowPositionals: true, strict: true });
  const flags: Flags = parsed.values; json = flags.json === true;
  if (flags.help) console.log(HELP);
  else if (flags.version) print({ version: manifest.version }, json);
  else if (parsed.positionals[0] === "service" && parsed.positionals[1] === "serve") await serve(flags);
  else print(await command(new Client(flag(flags, "url")), parsed.positionals, flags), json);
} catch (error) {
  const code = error instanceof ClientError ? error.code : "error", message = error instanceof Error ? error.message : String(error);
  if (json) console.log(JSON.stringify({ error: { code, message } })); else console.error(`Karabiner: ${message}`);
  process.exitCode = 1;
}
