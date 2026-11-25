import { Store, type Filter, type Table } from "./database.ts";
import { Repository } from "./repository.ts";
import { HttpError, invalid, missing } from "./errors.ts";
import { executionTargets, harnesses, metadata } from "./capabilities.ts";
import { templates } from "./templates.ts";
import { Changes } from "./events.ts";
import { createRuntime } from "./execution.ts";
import { RunStore } from "./run-store.ts";
import {
	FactoryCompileError,
	RUN_STATES,
	validateFactory,
	type RunFilter,
} from "@karabiner/contracts";
import {
	answerInput,
	configInput,
	controlInput,
	instructionInput,
	strandsReadinessInput,
	harnessCheckInput,
	object,
	pushInput,
	runInput,
	syncInput,
} from "./validation.ts";
import {
	compileProjectFactory,
	exportProjectFactory,
	importFactoryConfig,
} from "./factory-service.ts";
import { IntegrationService } from "./integrations/service.ts";
import { targetReadiness } from "./runtime/targets.ts";
import { strandsReadiness } from "./runtime/strands-model.ts";
import { harnessReadiness } from "./runtime/harness-readiness.ts";

type Handler = (body: unknown) => unknown;
export type IntegrationOptions = Omit<
	NonNullable<ConstructorParameters<typeof IntegrationService>[1]>,
	"changed"
>;
export interface ServiceOptions {
	dbPath: string;
	port: number;
	integrationOptions?: IntegrationOptions;
}

interface Resource {
	table: Table;
	create: Handler;
	update: (id: string, body: unknown) => unknown;
}

function reply(data: unknown, status = 200): Response {
	return Response.json({ data }, { status, headers: { "Cache-Control": "no-store" } });
}

function filter(url: URL): Filter {
	return {
		projectId: url.searchParams.get("projectId") || undefined,
		taskId: url.searchParams.get("taskId") || undefined,
	};
}

function limit(url: URL): number {
	const value = Number(url.searchParams.get("limit") ?? 100);

	if (!Number.isSafeInteger(value) || value < 1 || value > 500)
		invalid("limit must be an integer from 1 to 500");

	return value;
}

async function json(request: Request): Promise<unknown> {
	if (!request.headers.get("content-type")?.toLowerCase().startsWith("application/json"))
		throw new HttpError(415, "unsupported_media_type", "Send application/json");

	try {
		return await request.json();
	} catch {
		invalid("Request body is not valid JSON");
	}
}

function optionalJson(request: Request): Promise<unknown> {
	return request.body === null ? Promise.resolve({}) : json(request);
}

function runFilter(url: URL): RunFilter {
	const state = url.searchParams.get("state");

	if (state && !RUN_STATES.includes(state as RunFilter["state"] & string))
		invalid("Unknown run state");

	return { ...filter(url), ...(state ? { state: state as RunFilter["state"] } : {}) };
}

function validateConfig(value: unknown) {
	const result = validateFactory(value);
	if (!result.valid) return result;

	try {
		configInput(value);
		return result;
	} catch (error) {
		if (error instanceof HttpError)
			return {
				valid: false,
				errors: [{ code: error.code, path: "config", message: error.message }],
			};

		throw error;
	}
}

function methodNotAllowed(): never {
	throw new HttpError(405, "method_not_allowed", "This method is not supported for the resource");
}

export function startService(options: ServiceOptions) {
	const store = new Store(options.dbPath);
	const repo = new Repository(store);
	const events = new Changes();
	const runStore = new RunStore(store, (path) => events.changed(path));
	const runtime = createRuntime({ repository: repo, runs: runStore, changes: events });

	const integrations = new IntegrationService(repo, {
		...options.integrationOptions,
		changed: (path) => events.changed(path),
	});

	const resources: Record<string, Resource> = {
		projects: {
			table: "projects",
			create: (body) => repo.createProject(body),
			update: (id, body) => repo.updateProject(id, body),
		},
		tasks: {
			table: "tasks",
			create: (body) => repo.createTask(body),
			update: (id, body) => repo.updateTask(id, body),
		},
		documents: {
			table: "documents",
			create: (body) => repo.createDocument(body),
			update: (id, body) => repo.updateDocument(id, body),
		},
		interventions: {
			table: "interventions",
			create: (body) => repo.createIntervention(body),
			update: (id, body) => repo.updateIntervention(id, body),
		},
	};

	const readRoutes: Record<string, (url: URL) => unknown> = {
		"/health": () => ({ status: "ok" }),
		"/v1/meta": () => metadata,
		"/v1/templates": () => templates,
		"/v1/harnesses": () => harnesses(),
		"/v1/execution-targets": (url) =>
			executionTargets(
				url.searchParams.get("projectId")
					? store.config(url.searchParams.get("projectId")!)
					: undefined,
				url.searchParams.get("check") === "true",
			),
		"/v1/settings": () => repo.settings(),
		"/v1/workspace": () => repo.workspace(),
		"/v1/stats": (url) => {
			if (url.searchParams.has("taskId"))
				invalid("Statistics support projectId only; taskId is not supported");

			return repo.statistics({ projectId: url.searchParams.get("projectId") || undefined });
		},
		"/v1/activity": (url) => store.activities(filter(url), limit(url)),
		"/v1/runs": (url) => runtime.list(runFilter(url)),
	};

	const putRoutes: Record<string, Handler> = {
		"/v1/settings": (body) => repo.saveSettings(body),
		"/v1/workspace": (body) => repo.saveWorkspace(body),
	};

	async function route(request: Request, url: URL): Promise<Response> {
		const path = url.pathname;
		const method = request.method;

		if (method === "GET" && readRoutes[path]) return reply(await readRoutes[path](url));
		if (method === "PUT" && putRoutes[path]) return reply(putRoutes[path](await json(request)));

		if (path === "/v1/harnesses/strands/check") {
			if (method !== "POST") methodNotAllowed();
			return reply(strandsReadiness(strandsReadinessInput(await json(request))));
		}

		if (path === "/v1/harnesses/check") {
			if (method !== "POST") methodNotAllowed();
			const input = harnessCheckInput(await json(request));

			return reply(
				await harnessReadiness(input.agent, {
					check: input.check,
					root: input.projectRoot,
				}),
			);
		}

		const control = path.match(/^\/v1\/runs\/([^/]+)\/(pause|resume|cancel|retry)$/);

		if (control) {
			if (method !== "POST") methodNotAllowed();

			return reply(
				await runtime.control(
					decodeURIComponent(control[1]!),
					control[2] as "pause" | "resume" | "cancel" | "retry",
					controlInput(await optionalJson(request)),
				),
				control[2] === "retry" ? 202 : 200,
			);
		}

		const answer = path.match(/^\/v1\/runs\/([^/]+)\/questions\/([^/]+)\/answer$/);

		if (answer) {
			if (method !== "POST") methodNotAllowed();

			return reply(
				await runtime.answer(
					decodeURIComponent(answer[1]!),
					decodeURIComponent(answer[2]!),
					answerInput(await json(request)),
				),
			);
		}

		const instruction = path.match(/^\/v1\/runs\/([^/]+)\/instructions$/);

		if (instruction) {
			if (method !== "POST") methodNotAllowed();

			return reply(
				await runtime.instruct(
					decodeURIComponent(instruction[1]!),
					instructionInput(await json(request)),
				),
			);
		}

		const runDetail = path.match(/^\/v1\/runs\/([^/]+)$/);

		if (runDetail) {
			if (method !== "GET") methodNotAllowed();
			const record = await runtime.get(decodeURIComponent(runDetail[1]!));
			if (!record) missing("Run");
			return reply(record);
		}

		const comment = path.match(/^\/v1\/tasks\/([^/]+)\/comments$/);

		if (comment) {
			if (method !== "POST") methodNotAllowed();

			return reply(
				repo.addComment(decodeURIComponent(comment[1]!), await json(request)),
				201,
			);
		}

		const config = path.match(/^\/v1\/projects\/([^/]+)\/config$/);

		if (config) {
			const id = decodeURIComponent(config[1]!);
			if (method === "GET") return reply(store.config(id));
			if (method === "PUT") return reply(repo.saveConfig(id, await json(request)));
			methodNotAllowed();
		}

		const factory = path.match(
			/^\/v1\/projects\/([^/]+)\/factory(?:\/(validate|compile|export|import))?$/,
		);

		if (factory) {
			const id = decodeURIComponent(factory[1]!);
			const project = store.get("projects", id);
			const action = factory[2];

			if (method === "GET" && !action)
				return reply(await compileProjectFactory(project, store.config(id)));

			if (method === "GET" && action === "export")
				return reply(await exportProjectFactory(project, store.config(id)));

			if (method !== "POST") methodNotAllowed();
			const body = object(await optionalJson(request));
			if (action === "import") return reply(repo.saveConfig(id, importFactoryConfig(body)));
			const value = body.config ?? store.config(id);
			if (action === "validate") return reply(validateConfig(value));
			if (action === "compile") return reply(await compileProjectFactory(project, value));
			methodNotAllowed();
		}

		const catalog = path.match(/^\/v1\/projects\/([^/]+)\/integration\/catalog$/);

		if (catalog) {
			if (method !== "GET") methodNotAllowed();
			return reply(await integrations.catalog(decodeURIComponent(catalog[1]!)));
		}

		const targetCheck = path.match(/^\/v1\/projects\/([^/]+)\/execution-target\/check$/);

		if (targetCheck) {
			if (method !== "POST") methodNotAllowed();
			const id = decodeURIComponent(targetCheck[1]!);
			const project = store.get("projects", id);
			const body = object(await optionalJson(request));

			if (body.check !== undefined && typeof body.check !== "boolean")
				invalid("check must be boolean");

			if (
				body.purpose !== undefined &&
				body.purpose !== "command" &&
				body.purpose !== "harness"
			)
				invalid("purpose must be command or harness");

			if (
				body.executable !== undefined &&
				(typeof body.executable !== "string" ||
					!body.executable.trim() ||
					body.executable.length > 4096)
			)
				invalid("executable must be a nonempty string of at most 4096 characters");

			if (
				body.projectRoot !== undefined &&
				(typeof body.projectRoot !== "string" ||
					!body.projectRoot.trim() ||
					body.projectRoot.length > 4096)
			)
				invalid("projectRoot must be a nonempty string of at most 4096 characters");

			const config = body.config === undefined ? store.config(id) : configInput(body.config);

			return reply(
				await targetReadiness({
					target: config.executionTarget,
					projectRoot:
						(body.projectRoot as string | undefined) ?? project.rootPath ?? undefined,
					targetOptions: config.targetOptions,
					purpose: body.purpose as "command" | "harness" | undefined,
					executable: body.executable as string | undefined,
					check: body.check as boolean | undefined,
					signal: request.signal,
				}),
			);
		}

		const integration = path.match(/^\/v1\/projects\/([^/]+)\/(integration|sync)$/);

		if (integration) {
			const id = decodeURIComponent(integration[1]!);

			if (integration[2] === "integration" && method === "GET")
				return reply(
					await integrations.status(id, url.searchParams.get("check") === "true"),
				);

			if (integration[2] === "sync" && method === "POST")
				return reply(await integrations.sync(id, syncInput(await optionalJson(request))));

			methodNotAllowed();
		}

		const push = path.match(/^\/v1\/tasks\/([^/]+)\/push$/);

		if (push) {
			if (method !== "POST") methodNotAllowed();

			return reply(
				await integrations.push(
					decodeURIComponent(push[1]!),
					pushInput(await json(request)),
				),
			);
		}

		const run = path.match(/^\/v1\/tasks\/([^/]+)\/run$/);

		if (run) {
			if (method !== "POST") methodNotAllowed();

			return reply(
				await runtime.start(
					decodeURIComponent(run[1]!),
					runInput(await optionalJson(request)),
				),
				202,
			);
		}

		const match = path.match(/^\/v1\/(projects|tasks|documents|interventions)(?:\/([^/]+))?$/);

		if (!match) {
			if (readRoutes[path] || putRoutes[path]) methodNotAllowed();
			missing("Endpoint");
		}

		const resource = resources[match[1]!]!;
		const id = match[2] ? decodeURIComponent(match[2]) : null;

		if (method === "GET")
			return reply(
				id ? store.get(resource.table, id) : store.list(resource.table, filter(url)),
			);

		if (method === "POST" && !id) return reply(resource.create(await json(request)), 201);
		if (method === "PATCH" && id) return reply(resource.update(id, await json(request)));
		if (method === "DELETE" && id) return reply(repo.remove(resource.table, id));
		methodNotAllowed();
	}

	let server: ReturnType<typeof Bun.serve>;

	try {
		server = Bun.serve({
			hostname: "127.0.0.1",
			port: options.port,
			maxRequestBodySize: 2 * 1024 * 1024,
			async fetch(request, server) {
				try {
					const url = new URL(request.url);
					const origin = request.headers.get("origin");

					if (origin && origin !== url.origin)
						throw new HttpError(
							403,
							"origin_not_allowed",
							"Cross-origin service requests are not allowed",
						);

					if (url.pathname === "/v1/events" && request.method === "GET") {
						server.timeout(request, 0);
						return events.response();
					}

					const response = await route(request, url);
					if (request.method !== "GET") events.changed(url.pathname);
					return response;
				} catch (error) {
					if (error instanceof FactoryCompileError)
						return Response.json(
							{
								error: {
									code: "invalid_factory",
									message: error.message,
									details: error.issues,
								},
							},
							{ status: 400 },
						);

					if (error instanceof URIError)
						return Response.json(
							{
								error: {
									code: "invalid_request",
									message: "Malformed URL encoding",
								},
							},
							{ status: 400 },
						);

					if (error instanceof HttpError)
						return Response.json(
							{ error: { code: error.code, message: error.message } },
							{ status: error.status },
						);

					console.error(
						"Service request failed:",
						error instanceof Error ? error.message : "Unknown error",
					);

					return Response.json(
						{
							error: {
								code: "internal_error",
								message: "The service could not complete the request",
							},
						},
						{ status: 500 },
					);
				}
			},
		});
	} catch (error) {
		store.close();
		throw error;
	}

	runtime.startScheduler();
	integrations.start();

	let stopping: Promise<void> | undefined;

	return {
		url: `http://127.0.0.1:${server.port}`,
		stop(): Promise<void> {
			return (stopping ??= (async () => {
				events.close();

				try {
					const results = await Promise.allSettled([
						server.stop(),
						integrations.close(),
						runtime.shutdown(),
					]);

					const failures = results.filter(
						(result): result is PromiseRejectedResult => result.status === "rejected",
					);

					if (failures.length)
						throw new AggregateError(
							failures.map((result) => result.reason),
							"Service shutdown failed",
						);
				} finally {
					store.close();
				}
			})());
		},
	};
}
