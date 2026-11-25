import { Store, type Filter, type Table } from "./database.ts";
import { Repository } from "./repository.ts";
import { HttpError, invalid, missing } from "./errors.ts";
import { executionTargets, harnesses, metadata } from "./capabilities.ts";
import { templates } from "./templates.ts";
import { Changes } from "./events.ts";
import { unimplementedRuns, type RunReader } from "./execution.ts";

type Handler = (body: unknown) => unknown;
interface Resource { table: Table; create: Handler; update: (id: string, body: unknown) => unknown }
function reply(data: unknown, status = 200): Response { return Response.json({ data }, { status, headers: { "Cache-Control": "no-store" } }); }
function filter(url: URL): Filter {
  return { projectId: url.searchParams.get("projectId") || undefined, taskId: url.searchParams.get("taskId") || undefined };
}
function limit(url: URL): number {
  const value = Number(url.searchParams.get("limit") ?? 100);
  if (!Number.isSafeInteger(value) || value < 1 || value > 500) invalid("limit must be an integer from 1 to 500");
  return value;
}
async function json(request: Request): Promise<unknown> {
  if (!request.headers.get("content-type")?.toLowerCase().startsWith("application/json")) throw new HttpError(415, "unsupported_media_type", "Send application/json");
  try { return await request.json(); } catch { invalid("Request body is not valid JSON"); }
}
function methodNotAllowed(): never { throw new HttpError(405, "method_not_allowed", "This method is not supported for the resource"); }

export function startService(options: { dbPath: string; port: number; runs?: RunReader }) {
  const store = new Store(options.dbPath), repo = new Repository(store), events = new Changes();
  const runs = options.runs ?? unimplementedRuns;
  const resources: Record<string, Resource> = {
    projects: { table: "projects", create: body => repo.createProject(body), update: (id, body) => repo.updateProject(id, body) },
    tasks: { table: "tasks", create: body => repo.createTask(body), update: (id, body) => repo.updateTask(id, body) },
    documents: { table: "documents", create: body => repo.createDocument(body), update: (id, body) => repo.updateDocument(id, body) },
    interventions: { table: "interventions", create: body => repo.createIntervention(body), update: (id, body) => repo.updateIntervention(id, body) },
  };
  const readRoutes: Record<string, (url: URL) => unknown> = {
    "/health": () => ({ status: "ok" }), "/v1/meta": () => metadata,
    "/v1/templates": () => templates, "/v1/harnesses": () => harnesses,
    "/v1/execution-targets": () => executionTargets, "/v1/settings": () => repo.settings(),
    "/v1/workspace": () => repo.workspace(),
    "/v1/stats": url => {
      if (url.searchParams.has("taskId")) invalid("Statistics support projectId only; taskId is not supported");
      return repo.statistics({ projectId: url.searchParams.get("projectId") || undefined });
    },
    "/v1/activity": url => store.activities(filter(url), limit(url)), "/v1/runs": url => runs.list({ taskId: url.searchParams.get("taskId") || undefined }),
  };
  const putRoutes: Record<string, Handler> = { "/v1/settings": body => repo.saveSettings(body), "/v1/workspace": body => repo.saveWorkspace(body) };
  async function route(request: Request, url: URL): Promise<Response> {
    const path = url.pathname, method = request.method;
    if (method === "GET" && readRoutes[path]) return reply(await readRoutes[path](url));
    if (method === "PUT" && putRoutes[path]) return reply(putRoutes[path](await json(request)));
    const runDetail = path.match(/^\/v1\/runs\/([^/]+)$/);
    if (runDetail) {
      if (method !== "GET") methodNotAllowed();
      const record = await runs.get(decodeURIComponent(runDetail[1]!));
      if (!record) missing("Run");
      return reply(record);
    }
    const comment = path.match(/^\/v1\/tasks\/([^/]+)\/comments$/);
    if (comment) {
      if (method !== "POST") methodNotAllowed();
      return reply(repo.addComment(decodeURIComponent(comment[1]!), await json(request)), 201);
    }
    const config = path.match(/^\/v1\/projects\/([^/]+)\/config$/);
    if (config) {
      const id = decodeURIComponent(config[1]!);
      if (method === "GET") return reply(store.config(id));
      if (method === "PUT") return reply(repo.saveConfig(id, await json(request)));
      methodNotAllowed();
    }
    const run = path.match(/^\/v1\/tasks\/([^/]+)\/run$/);
    if (run) {
      if (method !== "POST") methodNotAllowed();
      store.get("tasks", decodeURIComponent(run[1]!));
      throw new HttpError(501, "not_implemented", "Task execution is not implemented. Your task is saved; no run has started.");
    }
    const match = path.match(/^\/v1\/(projects|tasks|documents|interventions)(?:\/([^/]+))?$/);
    if (!match) { if (readRoutes[path] || putRoutes[path]) methodNotAllowed(); missing("Endpoint"); }
    const resource = resources[match[1]!]!, id = match[2] ? decodeURIComponent(match[2]) : null;
    if (method === "GET") return reply(id ? store.get(resource.table, id) : store.list(resource.table, filter(url)));
    if (method === "POST" && !id) return reply(resource.create(await json(request)), 201);
    if (method === "PATCH" && id) return reply(resource.update(id, await json(request)));
    if (method === "DELETE" && id) return reply(repo.remove(resource.table, id));
    methodNotAllowed();
  }
  let server: ReturnType<typeof Bun.serve>;
  try {
    server = Bun.serve({
      hostname: "127.0.0.1", port: options.port, maxRequestBodySize: 2 * 1024 * 1024,
      async fetch(request, server) {
        try {
          const url = new URL(request.url);
          if (url.pathname === "/v1/events" && request.method === "GET") { server.timeout(request, 0); return events.response(); }
          const response = await route(request, url);
          if (request.method !== "GET") events.changed(url.pathname);
          return response;
        } catch (error) {
          if (error instanceof URIError) return Response.json({ error: { code: "invalid_request", message: "Malformed URL encoding" } }, { status: 400 });
          if (error instanceof HttpError) return Response.json({ error: { code: error.code, message: error.message } }, { status: error.status });
          console.error("Service request failed:", error instanceof Error ? error.message : "Unknown error");
          return Response.json({ error: { code: "internal_error", message: "The service could not complete the request" } }, { status: 500 });
        }
      },
    });
  } catch (error) { store.close(); throw error; }
  let stopping: Promise<void> | undefined;
  return {
    url: `http://127.0.0.1:${server.port}`,
    stop(): Promise<void> {
      return stopping ??= (async () => { events.close(); try { await server.stop(); } finally { store.close(); } })();
    },
  };
}
