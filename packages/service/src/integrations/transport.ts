import { HttpError } from "../errors.ts";
import { runProcess } from "../runtime/process.ts";

export interface IntegrationTransports {
	fetch?: typeof globalThis.fetch;
	run?: typeof runProcess;
	env?: NodeJS.ProcessEnv;
}

export class IntegrationError extends HttpError {
	constructor(
		code: string,
		message: string,
		readonly uncertain = false,
		status = 502,
	) {
		super(status, code, message);
	}
}

export function record(value: unknown): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value))
		throw new IntegrationError("invalid_response", "Provider returned an invalid object");

	return value as Record<string, unknown>;
}
export function string(value: unknown): string {
	if (typeof value !== "string")
		throw new IntegrationError("invalid_response", "Provider returned an invalid string");

	return value;
}
export function nodes(value: unknown): Record<string, unknown>[] {
	const list = record(value).nodes;

	if (!Array.isArray(list))
		throw new IntegrationError("invalid_response", "Provider returned an invalid collection");

	return list.map(record);
}
export function nextCursor(value: unknown): string | undefined {
	const page = record(record(value).pageInfo);
	if (page.hasNextPage === false) return undefined;

	if (page.hasNextPage !== true || typeof page.endCursor !== "string" || !page.endCursor)
		throw new IntegrationError(
			"invalid_response",
			"Provider returned invalid pagination information",
		);

	return page.endCursor;
}
export function timestamp(value: unknown): string {
	const date = string(value);

	if (!Number.isFinite(Date.parse(date)))
		throw new IntegrationError("invalid_response", "Provider returned an invalid timestamp");

	return new Date(date).toISOString();
}
function decode(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		throw new IntegrationError("invalid_response", "Provider returned invalid JSON");
	}
}
export function graphData(value: unknown): Record<string, unknown> {
	const response = record(value);

	if (Array.isArray(response.errors) && response.errors.length) {
		const limited = response.errors.some(
			(e) => record(record(e).extensions ?? {}).code === "RATELIMITED",
		);

		throw new IntegrationError(
			limited ? "rate_limited" : "graphql_error",
			limited
				? "Provider rate limit reached; retry after its reset window"
				: "Provider rejected part of the GraphQL request; no partial snapshot was applied",
		);
	}

	return record(response.data);
}

export class GithubTransport {
	constructor(
		readonly hostname: string,
		readonly options: IntegrationTransports,
		readonly signal: AbortSignal,
	) {}
	async request(endpoint: string, method = "GET", body?: unknown): Promise<unknown> {
		const env = {
			...(this.options.env ?? process.env),
			GH_PROMPT_DISABLED: "1",
			GH_PAGER: "cat",
		};

		const args = [
			"api",
			"--hostname",
			this.hostname,
			"--method",
			method,
			"-H",
			"Accept: application/vnd.github+json",
			"-H",
			"X-GitHub-Api-Version: 2026-03-10",
			endpoint,
		];

		if (body !== undefined) args.push("--input", "-");
		let result;

		try {
			result = await (this.options.run ?? runProcess)({
				command: "gh",
				args,
				cwd: process.cwd(),
				env,
				input: body === undefined ? undefined : JSON.stringify(body),
				signal: this.signal,
				timeoutMs: 30_000,
				maxOutputBytes: 8 * 1024 * 1024,
			});
		} catch {
			throw new IntegrationError(
				"transport_error",
				"GitHub CLI request failed or timed out",
				method !== "GET",
			);
		}

		if (result.code !== 0) {
			const status = Number(/HTTP (\d{3})/.exec(result.stderr)?.[1]);

			const code =
				status === 401
					? "authentication_required"
					: status === 403
						? "access_denied"
						: status === 404
							? "remote_not_found"
							: status === 429
								? "rate_limited"
								: "github_error";

			throw new IntegrationError(
				code,
				`GitHub request failed${status ? ` (HTTP ${status})` : "; check gh authentication and project access"}`,
				method !== "GET" && (!status || status >= 500),
			);
		}

		return decode(result.stdout);
	}
	async graphql(
		query: string,
		variables: Record<string, unknown> = {},
	): Promise<Record<string, unknown>> {
		return graphData(await this.request("graphql", "POST", { query, variables }));
	}
	async list(path: string): Promise<Record<string, unknown>[]> {
		const result: Record<string, unknown>[] = [];

		for (let page = 1; page <= 1_000; page++) {
			const values = await this.request(
				`${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`,
			);

			if (!Array.isArray(values))
				throw new IntegrationError(
					"invalid_response",
					"GitHub returned an invalid issue list",
				);

			result.push(...values.map(record));
			if (values.length < 100) return result;
		}

		throw new IntegrationError(
			"pagination_limit",
			"GitHub collection exceeded 100,000 records",
		);
	}
}

export class LinearTransport {
	constructor(
		readonly apiKeyEnv: string,
		readonly options: IntegrationTransports,
		readonly signal: AbortSignal,
	) {}
	async graphql(
		query: string,
		variables: Record<string, unknown> = {},
	): Promise<Record<string, unknown>> {
		const token = (this.options.env ?? process.env)[this.apiKeyEnv];

		if (!token)
			throw new IntegrationError(
				"credential_env_missing",
				`Set the service environment variable ${this.apiKeyEnv} to a Linear API key`,
				false,
				409,
			);

		let response: Response;

		try {
			response = await (this.options.fetch ?? globalThis.fetch)(
				"https://api.linear.app/graphql",
				{
					method: "POST",
					headers: { "Content-Type": "application/json", Authorization: token },
					body: JSON.stringify({ query, variables }),
					redirect: "error",
					signal: AbortSignal.any([this.signal, AbortSignal.timeout(30_000)]),
				},
			);
		} catch {
			throw new IntegrationError(
				"transport_error",
				"Linear request failed or timed out",
				query.trimStart().startsWith("mutation"),
			);
		}

		if (!response.ok) {
			await response.body?.cancel();

			const code =
				response.status === 401
					? "authentication_required"
					: response.status === 403
						? "access_denied"
						: response.status === 429
							? "rate_limited"
							: "linear_error";

			throw new IntegrationError(
				code,
				`Linear request failed (HTTP ${response.status})`,
				response.status >= 500 && query.trimStart().startsWith("mutation"),
			);
		}

		let body: string;

		try {
			body = await boundedBody(response);
		} catch {
			throw new IntegrationError(
				"invalid_response",
				"Linear response was incomplete or exceeded its size limit",
				query.trimStart().startsWith("mutation"),
			);
		}

		return graphData(decode(body));
	}
}

async function boundedBody(response: Response): Promise<string> {
	const reader = response.body?.getReader();
	if (!reader) throw new Error("Missing response body");
	const chunks: Uint8Array[] = [];
	let bytes = 0;

	try {
		while (true) {
			const { value, done } = await reader.read();
			if (done) return Buffer.concat(chunks).toString("utf8");
			bytes += value.byteLength;
			if (bytes > 8 * 1024 * 1024) throw new Error("Response limit");
			chunks.push(value);
		}
	} finally {
		await reader.cancel().catch(() => undefined);
	}
}
