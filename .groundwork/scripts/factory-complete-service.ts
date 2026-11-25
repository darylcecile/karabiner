/**
 * GUI fixture service: the real startService (routes, SQLite, runtime) with only the external
 * connector transports injected through ServiceOptions.integrationOptions. GitHub `gh` calls
 * and Linear GraphQL requests are answered from the same fixture payload shapes as
 * factory-integrations.ts and logged to FIXTURE_TRANSPORT_LOG; nothing contacts GitHub/Linear.
 * The client talks to a small relay in front of the service (factory-runtime.ts proxy pattern):
 * it forwards everything unchanged (SSE streamed, no idle timeout), logs task Push bodies, and
 * for the first Push consumes the real service response, then answers the client with
 * 502 fixture_reply_lost, so a native Retry can be compared with the original request.
 * Prints {"event":"ready","url":<relay>,"serviceUrl":<service>,"pid"}.
 */
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { startService } from "../../packages/service/src/server.ts";

const log = process.env.FIXTURE_TRANSPORT_LOG!;
const record = (entry: Record<string, unknown>) => appendFileSync(log, JSON.stringify({ at: Date.now(), ...entry }) + "\n");
const connection = (nodes: unknown[]) => ({ nodes, pageInfo: { hasNextPage: false, endCursor: null } });

let issue = { node_id: "I_1", number: 1, title: "Remote issue", body: "body", html_url: "https://github.com/owner/repo/issues/1", updated_at: "2026-11-25T00:00:00Z", state: "open", state_reason: null as string | null, labels: [] as never[] };

const run = async (request: { command: string; args?: string[]; input?: string }) => {
  const args = request.args ?? [], method = args[args.indexOf("--method") + 1] ?? "GET";
  const endpoint = args.find((arg) => arg.startsWith("repos/") || arg === "graphql") ?? "";
  const input = request.input ? JSON.parse(request.input) : undefined;
  record({ transport: "github", method, endpoint, input });
  if (method === "PATCH") {
    issue = { ...issue, ...input, updated_at: "2026-11-25T01:00:00Z" };
    return { code: 0, stdout: JSON.stringify(issue), stderr: "" };
  }
  if (method === "POST") return { code: 1, stdout: "", stderr: "HTTP 422: fixture does not create issues" };
  const data = endpoint === "repos/owner/repo" ? { node_id: "R_1", full_name: "owner/repo", html_url: "https://github.com/owner/repo" }
    : endpoint.includes("?") ? [issue] : endpoint.startsWith("repos/owner/repo/issues/") ? issue : undefined;
  if (data === undefined) return { code: 1, stdout: "", stderr: `HTTP 404: no fixture for ${method} ${endpoint}` };
  return { code: 0, stdout: JSON.stringify(data), stderr: "" };
};

const linearFetch = (async (_url: string, options?: RequestInit) => {
  const body = JSON.parse(String(options?.body)) as { query: string; variables: Record<string, unknown> };
  record({ transport: "linear", query: body.query.slice(0, 80), variables: body.variables });
  if (body.query.includes("teams(filter")) return Response.json({ data: { teams: { nodes: [{ id: "team-1" }] } } });
  if (body.query.includes("states(first")) return Response.json({ data: { team: { id: "team-1", name: "Engineering", key: "ENG", states: connection([
    { id: "state-todo", name: "Todo", type: "unstarted", color: "#555555" },
    { id: "state-review", name: "Review queue", type: "started", color: "#888888" },
    { id: "state-done", name: "Done", type: "completed", color: "#22aa22" }]) } } });
  if (body.query.includes("issues(filter")) return Response.json({ data: { issues: connection([]) } });
  return Response.json({ errors: [{ message: "no fixture for this Linear operation" }] });
}) as typeof globalThis.fetch;

const service = startService({
  dbPath: resolve(process.env.KARABINER_DB!),
  port: 0,
  integrationOptions: { run: run as never, fetch: linearFetch, env: { KARABINER_TEST_LINEAR: "synthetic-fixture-value" } },
});
const upstream = String(service.url).replace(/\/$/, "");
let replyLost = false, requestSeq = 0, inflight = 0;
const relay = Bun.serve({
  hostname: "127.0.0.1", port: 0, idleTimeout: 0,
  async fetch(request) {
    const url = new URL(request.url);
    const isPush = request.method === "POST" && /^\/v1\/tasks\/[^/]+\/push$/.test(url.pathname);
    const isAnswer = request.method === "POST" && /\/questions\/[^/]+\/answer$/.test(url.pathname);
    if (url.pathname.startsWith("/v1/") && url.pathname !== "/v1/events" && !isPush && !isAnswer) {
      const id = ++requestSeq, start = Date.now(); inflight++;
      record({ transport: "http", id, phase: "start", method: request.method, path: url.pathname + url.search, inflight });
      const reqBody = ["GET", "HEAD"].includes(request.method) ? undefined : await request.arrayBuffer();
      const response = await globalThis.fetch(upstream + url.pathname + url.search, { method: request.method, headers: request.headers, body: reqBody });
      const bytes = await response.arrayBuffer(); inflight--;
      record({ transport: "http", id, phase: "finish", method: request.method, path: url.pathname + url.search, status: response.status, bytes: bytes.byteLength, ms: Date.now() - start, inflight });
      return new Response(bytes, { status: response.status, headers: response.headers });
    }
    const body = ["GET", "HEAD"].includes(request.method) ? undefined : await request.arrayBuffer();
    const response = await globalThis.fetch(upstream + url.pathname + url.search, { method: request.method, headers: request.headers, body });
    if (request.method === "POST" && /^\/v1\/runs\/[^/]+\/questions\/[^/]+\/answer$/.test(url.pathname)) {
      const text = await response.text();
      record({ transport: "relay", path: url.pathname, request: JSON.parse(new TextDecoder().decode(body)), status: response.status, response: JSON.parse(text) });
      return new Response(text, { status: response.status, headers: { "content-type": "application/json" } });
    }
    if (request.method !== "POST" || !/^\/v1\/tasks\/[^/]+\/push$/.test(url.pathname)) return response;
    const text = await response.text();
    record({ transport: "relay", path: url.pathname, request: JSON.parse(new TextDecoder().decode(body)), status: response.status, response: JSON.parse(text) });
    if (!replyLost) {
      replyLost = true;
      return Response.json({ error: { code: "fixture_reply_lost", message: "The fixture dropped the reply after the service accepted the push" } }, { status: 502 });
    }
    return new Response(text, { status: response.status, headers: { "content-type": "application/json" } });
  },
});
console.log(JSON.stringify({ event: "ready", url: `http://127.0.0.1:${relay.port}`, serviceUrl: upstream, pid: process.pid }));
const stop = () => { relay.stop(true); void service.stop(); };
process.once("SIGINT", stop);
process.once("SIGTERM", stop);
