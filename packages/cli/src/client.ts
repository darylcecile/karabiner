import { API_VERSION, DEFAULT_SERVICE_URL, type ApiError, type Json, type Reply, type ServiceMeta } from "@karabiner/contracts";

export class ClientError extends Error {
  constructor(readonly code: string, message: string, readonly status?: number, readonly details?: Json) { super(message); }
}
export class Client {
  readonly url: string;
  constructor(url = process.env.KARABINER_SERVICE_URL ?? DEFAULT_SERVICE_URL) {
    this.url = new URL(url).href.replace(/\/$/, "");
  }
  async request<T>(method: string, path: string, body?: unknown, timeoutMs = 10_000): Promise<T> {
    let response: Response;
    try {
      response = await fetch(`${this.url}${path}`, { method, headers: body === undefined ? {} : { "Content-Type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(timeoutMs) });
    } catch (error) {
      const guidance = method === "GET" ? "Start karabiner-service or set --url." : "The mutation outcome is unknown; inspect current state before repeating it.";
      throw new ClientError("service_unavailable", `Cannot contact Karabiner at ${this.url}. ${guidance} ${error instanceof Error ? error.message : "Connection failed"}`);
    }
    let result: Reply<T> | ApiError;
    try { result = await response.json(); } catch { throw new ClientError("invalid_response", "The service returned invalid JSON", response.status); }
    if ("error" in result) throw new ClientError(result.error.code, result.error.message, response.status, result.error.details);
    if (!response.ok || !("data" in result)) throw new ClientError("invalid_response", `Unexpected service response (${response.status})`, response.status);
    return result.data;
  }
  async connect(): Promise<ServiceMeta> {
    const meta = await this.request<ServiceMeta>("GET", "/v1/meta");
    if (meta.apiVersion !== API_VERSION) throw new ClientError("incompatible_version", `Service API ${meta.apiVersion} is incompatible with this CLI (API ${API_VERSION}). Update the CLI or service.`);
    return meta;
  }
}
