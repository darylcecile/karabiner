import { API_VERSION, DEFAULT_SERVICE_URL, type ApiError, type Reply, type ServiceMeta } from "@karabiner/contracts";

export class ClientError extends Error {
  constructor(readonly code: string, message: string, readonly status?: number) { super(message); }
}
export class Client {
  readonly url: string;
  constructor(url = process.env.KARABINER_SERVICE_URL ?? DEFAULT_SERVICE_URL) {
    this.url = new URL(url).href.replace(/\/$/, "");
  }
  async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    let response: Response;
    try {
      response = await fetch(`${this.url}${path}`, { method, headers: body === undefined ? {} : { "Content-Type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(10_000) });
    } catch (error) {
      throw new ClientError("service_unavailable", `Cannot contact Karabiner at ${this.url}. Start karabiner-service or set --url. ${error instanceof Error ? error.message : "Connection failed"}`);
    }
    let result: Reply<T> | ApiError;
    try { result = await response.json(); } catch { throw new ClientError("invalid_response", "The service returned invalid JSON", response.status); }
    if ("error" in result) throw new ClientError(result.error.code, result.error.message, response.status);
    if (!response.ok || !("data" in result)) throw new ClientError("invalid_response", `Unexpected service response (${response.status})`, response.status);
    return result.data;
  }
  async connect(): Promise<ServiceMeta> {
    const meta = await this.request<ServiceMeta>("GET", "/v1/meta");
    if (meta.apiVersion !== API_VERSION) throw new ClientError("incompatible_version", `Service API ${meta.apiVersion} is incompatible with this CLI (API ${API_VERSION}). Update the CLI or service.`);
    return meta;
  }
}
