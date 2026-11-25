import { createHash } from "node:crypto";
import type {
	IntegrationCatalog,
	IntegrationInfo,
	Project,
	ProjectConfig,
	Task,
} from "@karabiner/contracts";
import { checkRevision } from "../database.ts";
import { conflict, invalid } from "../errors.ts";
import type { Repository } from "../repository.ts";
import { GithubIntegration } from "./github.ts";
import { githubCatalog, linearCatalog } from "./catalog.ts";
import { LinearIntegration } from "./linear.ts";
import { IntegrationStore } from "./store.ts";
import {
	GithubTransport,
	IntegrationError,
	LinearTransport,
	type IntegrationTransports,
} from "./transport.ts";
import { integrationConfig, pushInput } from "./validation.ts";
import {
	SYNC_FIELDS,
	type ExternalLink,
	type IntegrationConfig,
	type IntegrationPushInput,
	type IntegrationPushResult,
	type IntegrationSyncResult,
	type ProviderStatus,
	type PushOperation,
	type RemoteTask,
	type SyncField,
	type SyncIssue,
	type TaskProjection,
} from "./types.ts";

export { validateIntegrationConfig } from "./validation.ts";
export type {
	IntegrationConfig,
	IntegrationPushInput,
	IntegrationPushResult,
	IntegrationSyncResult,
} from "./types.ts";

type ProviderClient = GithubIntegration | LinearIntegration;

interface Context {
	project: Project;
	config: IntegrationConfig;
	source: string;
	client: ProviderClient;
}

interface RefreshRecord {
	key: string;
	nextAt: number;
	running: boolean;
	lastAttemptAt?: string;
	lastSyncedAt?: string;
	error?: string;
}

function hash(value: unknown): string {
	return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}

function equal(left: unknown, right: unknown): boolean {
	return JSON.stringify(left) === JSON.stringify(right);
}

function projection(task: Task): TaskProjection {
	return {
		title: task.title,
		description: task.description,
		status: task.status,
		priority: task.priority,
		labels: [...task.labels].sort(),
		dueAt: task.dueAt,
	};
}

function publishFields(input: IntegrationPushInput, config: IntegrationConfig): SyncField[] {
	const fields: SyncField[] = input.fields ?? config.pushFields ?? ["title", "description"];

	if (!fields.length)
		throw new IntegrationError(
			"publish_fields_empty",
			"Select publish fields in the project settings or supply explicit fields for this push",
			false,
			409,
		);

	return [...new Set(fields)];
}

function issue(error: unknown, id?: string): SyncIssue {
	return error instanceof IntegrationError
		? { id, code: error.code, message: error.message }
		: {
				id,
				code: "sync_error",
				message: error instanceof Error ? error.message : "Integration operation failed",
			};
}

/** Pulls may be explicitly scheduled; every remote write still requires an explicit push. */
export class IntegrationService {
	readonly links: IntegrationStore;
	private readonly controller = new AbortController();
	private readonly active = new Set<Promise<unknown>>();
	private readonly busy = new Set<string>();
	private readonly observations = new Map<string, ProviderStatus>();
	private readonly refreshes = new Map<string, RefreshRecord>();
	private refreshTimer?: ReturnType<typeof setInterval>;
	private refreshing?: Promise<void>;
	private refreshError?: string;
	constructor(
		readonly repo: Repository,
		readonly options: IntegrationTransports & { changed?: (path: string) => void } = {},
	) {
		this.links = new IntegrationStore(repo.store.db);
	}
	private context(projectId: string, signal = this.controller.signal): Context {
		const project = this.repo.store.get("projects", projectId);

		const stored = this.repo.store.config(projectId) as ProjectConfig & {
			integration?: unknown;
		};

		const config = integrationConfig(project, stored.integration);

		const source = hash({
			provider: config.provider,
			scope: config.scope,
			locator: config.scope === "repository" ? project.repository : project.externalRef,
			hostname: config.provider === "github" ? (config.hostname ?? "github.com") : undefined,
		});

		const client =
			config.provider === "github"
				? new GithubIntegration(
						project,
						config,
						new GithubTransport(config.hostname ?? "github.com", this.options, signal),
					)
				: new LinearIntegration(
						project,
						config,
						new LinearTransport(config.apiKeyEnv, this.options, signal),
					);

		return { project, config, source, client };
	}
	private operationSignal(): AbortSignal {
		return AbortSignal.any([this.controller.signal, AbortSignal.timeout(120_000)]);
	}
	async catalog(projectId: string): Promise<IntegrationCatalog> {
		return this.track(async () => {
			const context = this.context(projectId, this.operationSignal());

			const result =
				context.client instanceof GithubIntegration
					? await githubCatalog(context.client)
					: await linearCatalog(context.client);

			this.assertContext(context);
			return result;
		});
	}
	async status(projectId: string, check = false): Promise<ProviderStatus> {
		const context = this.context(
			projectId,
			check ? this.operationSignal() : this.controller.signal,
		);

		const key = hash({ source: context.source, config: context.config });

		const configured = Boolean(
			context.config.scope === "repository"
				? context.project.repository
				: context.project.externalRef,
		);

		const available =
			context.config.provider === "github"
				? Boolean(this.options.run || Bun.which("gh"))
				: true;

		const hasCredential =
			context.config.provider === "github" ||
			Boolean((this.options.env ?? process.env)[context.config.apiKeyEnv]);

		const base: ProviderStatus = {
			id: context.config.provider,
			configured,
			available,
			authenticated: null,
			checkedAt: new Date().toISOString(),
			status: "unknown",
			message: "Authentication and remote access have not been checked",
			refresh: this.refreshStatus(context),
		};

		if (!configured || !available || !hasCredential)
			return {
				...base,
				status: "unavailable",
				code: !configured
					? "source_not_configured"
					: !available
						? "executable_missing"
						: "credential_env_missing",
				message: !configured
					? "Configure the project's external source"
					: !available
						? "GitHub CLI is not on the service PATH"
						: `The service environment variable ${context.config.provider === "linear" ? context.config.apiKeyEnv : ""} is not set`,
			};

		if (!check) return { ...(this.observations.get(key) ?? base), refresh: base.refresh };

		return this.track(async () => {
			let result: ProviderStatus;

			try {
				await context.client.probe();

				result = {
					...base,
					authenticated: true,
					status: "ready",
					message: "Authentication and source read access confirmed",
				};
			} catch (error) {
				const failure = issue(error);

				result = {
					...base,
					authenticated: failure.code === "authentication_required" ? false : null,
					status: "error",
					code: failure.code,
					message: failure.message,
				};
			}

			result.checkedAt = new Date().toISOString();
			this.observations.set(key, result);
			return result;
		});
	}
	async sync(
		projectId: string,
		input: { conflict?: "preserve-local" | "overwrite-local" } = {},
	): Promise<IntegrationSyncResult> {
		if (
			input.conflict !== undefined &&
			input.conflict !== "preserve-local" &&
			input.conflict !== "overwrite-local"
		)
			invalid("Unknown sync conflict policy");

		const context = this.context(projectId, this.operationSignal());

		return this.exclusive(projectId, async () => {
			const result: IntegrationSyncResult = {
				provider: context.config.provider,
				projectId,
				imported: 0,
				updated: 0,
				skipped: 0,
				conflicts: 0,
				errors: [],
				syncedAt: new Date().toISOString(),
			};

			try {
				const collection = await context.client.list();
				this.assertContext(context);
				result.errors.push(...collection.errors);
				const links = this.links.links(projectId, context.config.provider);
				const seen = new Set<string>();

				for (const remote of collection.tasks) {
					if (seen.has(remote.external.id)) {
						result.skipped++;
						continue;
					}

					seen.add(remote.external.id);

					try {
						this.pullTask(
							context,
							remote,
							links,
							input.conflict === "overwrite-local",
							result,
						);
					} catch (error) {
						result.errors.push(issue(error, remote.external.id));
					}
				}
			} catch (error) {
				result.errors.push(issue(error));
			}

			result.syncedAt = new Date().toISOString();

			if (result.imported || result.updated)
				this.options.changed?.(`/v1/tasks?projectId=${encodeURIComponent(projectId)}`);

			return result;
		});
	}
	private pullTask(
		context: Context,
		remote: RemoteTask,
		links: ExternalLink[],
		overwrite: boolean,
		result: IntegrationSyncResult,
	): void {
		const previous = links.find(
			(l) =>
				l.externalId === remote.external.id ||
				(remote.external.projectItemId &&
					l.remote.external.projectItemId === remote.external.projectItemId),
		);

		if (previous?.source && previous.source !== context.source) {
			result.skipped++;

			result.errors.push({
				id: remote.external.id,
				code: "source_changed",
				message: "This task is linked to the project's previous external source",
			});

			return;
		}

		if (previous?.taskId === null || remote.archived) {
			result.skipped++;
			return;
		}

		if (
			!previous &&
			remote.values.status === undefined &&
			["canceled", "duplicate", "not_planned"].includes(remote.state ?? "")
		) {
			result.skipped++;

			result.errors.push({
				id: remote.external.id,
				code: "status_mapping_required",
				message: "Map this terminal provider status before importing it as a task",
			});

			return;
		}

		this.repo.store.atomic(() => {
			if (!previous) {
				const task = this.repo.createTask({
					projectId: context.project.id,
					...remote.values,
				});

				this.repo.store.put("tasks", { ...task, external: remote.external } as Task);

				const link: ExternalLink = {
					projectId: task.projectId,
					provider: context.config.provider,
					externalId: remote.external.id,
					taskId: task.id,
					source: context.source,
					baseline: remote.values,
					remote,
				};

				this.links.put(link);
				links.push(link);
				result.imported++;
				return;
			}

			const task = this.repo.store.get("tasks", previous.taskId!);
			const local = projection(task);
			const patch: Partial<TaskProjection> = {};
			const baseline = { ...previous.baseline };

			for (const field of SYNC_FIELDS) {
				const incoming = remote.values[field];
				if (incoming === undefined) continue;

				const diverged =
					!equal(local[field], previous.baseline[field]) &&
					!equal(local[field], incoming);

				if (diverged && !overwrite) {
					if (!equal(incoming, previous.baseline[field])) {
						result.conflicts++;

						result.errors.push({
							id: task.id,
							code: "local_conflict",
							message: `Kept the local ${field}; the provider also changed it`,
						});
					}

					continue;
				}

				Object.assign(baseline, { [field]: incoming });
				if (!equal(local[field], incoming)) Object.assign(patch, { [field]: incoming });
			}

			const externalChanged = !equal(
				(task as Task & { external?: unknown }).external,
				remote.external,
			);

			if (Object.keys(patch).length || externalChanged) {
				const updated = this.repo.updateTask(task.id, {
					revision: task.revision,
					...patch,
				});

				this.repo.store.put("tasks", { ...updated, external: remote.external } as Task);
				result.updated++;
			} else result.skipped++;

			this.links.put(
				{ ...previous, externalId: remote.external.id, baseline, remote },
				previous.externalId,
			);
		});
	}
	async push(taskId: string, value: IntegrationPushInput): Promise<IntegrationPushResult> {
		const input = pushInput(value);
		const task = this.repo.store.get("tasks", taskId);
		const context = this.context(task.projectId, this.operationSignal());

		if (!context.config.pushEnabled)
			conflict("Enable push in this project's integration configuration before pushing");

		return this.exclusive(task.projectId, () => this.pushTask(context, taskId, input));
	}
	private async pushTask(
		context: Context,
		taskId: string,
		input: IntegrationPushInput,
	): Promise<IntegrationPushResult> {
		const fingerprint = hash({ source: context.source, taskId, ...input });
		const existing = this.links.operation(input.requestId);

		if (existing && (existing.taskId !== taskId || existing.fingerprint !== fingerprint))
			conflict("requestId was already used for a different push");

		if (existing?.state === "finished") return existing.result!;
		const task = this.repo.store.get("tasks", taskId);
		if (!existing) checkRevision(task, input.revision);
		let link = this.links.forTask(taskId);

		if (link && (link.source !== context.source || link.provider !== context.config.provider))
			conflict("Task is linked to a different external source");

		if (!link && !input.createMissing)
			conflict("Task has no external link; set createMissing to explicitly create an issue");

		const pending = this.links.pendingForTask(taskId);

		if (pending && pending.requestId !== input.requestId)
			conflict("A previous push is unconfirmed; reconcile its original requestId first");

		const operation: PushOperation = existing ?? {
			taskId,
			requestId: input.requestId,
			fingerprint,
			state: "pending",
			remoteId:
				context.config.provider === "linear" ? crypto.randomUUID() : hash(input.requestId),
			values: projection(task),
			// Evaluated only for a new operation; retries retain the original selection.
			fields: publishFields(input, context.config),
		};

		const result: IntegrationPushResult = {
			taskId,
			requestId: input.requestId,
			status: "failed",
			errors: [],
		};

		let wrote = Boolean(existing);
		let observed: RemoteTask | undefined;

		try {
			let remote = link
				? await context.client.get(link.remote)
				: existing
					? await this.recoverCreation(context, operation)
					: undefined;

			observed = remote;

			if (!link && existing && !remote)
				throw new IntegrationError(
					"creation_uncertain",
					"Issue creation remains unconfirmed; retry this requestId after checking the provider",
					true,
				);

			if (remote && !link) link = this.attach(context, taskId, remote, remote.values);
			const desired = operation.values!;
			const fields = operation.fields!;
			if (remote && link) this.pushConflicts(remote, link.baseline, desired, fields);
			this.assertContext(context);
			if (!existing) checkRevision(this.repo.store.get("tasks", taskId), input.revision);
			this.links.putOperation(operation);

			if (!remote) {
				wrote = true;
				remote = await context.client.create(desired, operation.remoteId!);
				observed = remote;
				result.external = remote.external;
				link = this.attach(context, taskId, remote, remote.values);
			}

			const changed = fields.filter((f) => !equal(desired[f], remote!.values[f]));

			if (changed.length) {
				wrote = true;
				remote = await context.client.update(remote, desired, changed);
				observed = remote;
			}

			const unapplied = fields.filter((f) => !equal(desired[f], remote!.values[f]));

			if (unapplied.length)
				throw new IntegrationError(
					"push_unconfirmed",
					`Provider did not confirm fields: ${unapplied.join(", ")}`,
					true,
				);

			this.attach(context, taskId, remote, {
				...link?.baseline,
				...Object.fromEntries(fields.map((f) => [f, remote!.values[f]])),
			} as TaskProjection);

			result.status = wrote ? "pushed" : "unchanged";
			result.external = remote.external;
		} catch (error) {
			// These failures precede a field mutation, or stop after observing a
			// conflicting remote snapshot. They are resolved failures, not lost replies.
			const resolvedFailure =
				error instanceof IntegrationError &&
				[
					"unsupported_field",
					"status_mapping_required",
					"label_mapping_required",
					"team_required",
					"remote_conflict",
				].includes(error.code);

			result.status =
				!resolvedFailure &&
				(wrote || (error instanceof IntegrationError && error.uncertain))
					? "uncertain"
					: "failed";

			result.errors.push(issue(error));
		}

		if (!this.repo.store.db.query("SELECT id FROM tasks WHERE id=?").get(taskId)) {
			if (
				observed &&
				this.repo.store.db
					.query("SELECT id FROM projects WHERE id=?")
					.get(context.project.id)
			)
				this.links.put(
					{
						projectId: context.project.id,
						provider: context.config.provider,
						externalId: observed.external.id,
						taskId: null,
						source: context.source,
						baseline: observed.values,
						remote: observed,
					},
					link?.externalId,
				);

			result.status = wrote ? "uncertain" : "failed";

			result.errors.push({
				code: "task_deleted",
				message:
					"The local task was deleted during push; its remote outcome is retained as a tombstone when possible",
			});

			return result;
		}

		operation.result = result;
		operation.state = result.status === "uncertain" ? "pending" : "finished";
		this.links.putOperation(operation);

		if (result.status === "pushed")
			this.repo.store.activity(
				"integration.pushed",
				`Pushed task to ${context.config.provider}`,
				context.project.id,
				taskId,
				taskId,
			);

		this.options.changed?.(`/v1/tasks/${encodeURIComponent(taskId)}`);
		return result;
	}
	private pushConflicts(
		remote: RemoteTask,
		baseline: TaskProjection,
		desired: TaskProjection,
		fields: SyncField[],
	): void {
		for (const field of fields)
			if (
				!equal(remote.values[field], baseline[field]) &&
				!equal(remote.values[field], desired[field])
			)
				throw new IntegrationError(
					"remote_conflict",
					`Provider ${field} changed since the last synchronization; pull before pushing`,
					false,
					409,
				);
	}
	private async recoverCreation(
		context: Context,
		operation: PushOperation,
	): Promise<RemoteTask | undefined> {
		if (context.client instanceof LinearIntegration)
			return context.client.getId(operation.remoteId!);

		return context.client.recoverCreate(operation.remoteId!);
	}
	private attach(
		context: Context,
		taskId: string,
		remote: RemoteTask,
		baseline: TaskProjection,
	): ExternalLink {
		return this.repo.store.atomic(() => {
			const task = this.repo.store.get("tasks", taskId);
			const previous = this.links.forTask(taskId);

			const link: ExternalLink = {
				projectId: context.project.id,
				provider: context.config.provider,
				externalId: remote.external.id,
				taskId,
				source: context.source,
				remote,
				baseline,
			};

			this.links.put(link, previous?.externalId);
			this.repo.store.put("tasks", { ...task, external: remote.external } as Task);
			return link;
		});
	}
	private assertContext(context: Context): void {
		this.controller.signal.throwIfAborted();
		const current = this.context(context.project.id);

		if (current.source !== context.source || !equal(current.config, context.config))
			conflict("Project integration changed during synchronization; try again");
	}
	private async exclusive<T>(projectId: string, work: () => Promise<T>): Promise<T> {
		if (this.busy.has(projectId))
			conflict("An integration operation is already active for this project");

		this.busy.add(projectId);

		try {
			return await this.track(work);
		} finally {
			this.busy.delete(projectId);
		}
	}
	private async track<T>(work: () => Promise<T>): Promise<T> {
		this.controller.signal.throwIfAborted();
		const promise = work();
		this.active.add(promise);

		try {
			return await promise;
		} finally {
			this.active.delete(promise);
		}
	}
	/** The server calls this only after its listener has started successfully. */
	start(): void {
		this.controller.signal.throwIfAborted();
		if (this.refreshTimer) return;

		const tick = () => {
			void this.refreshDue().catch((error) => {
				this.refreshError = issue(error).message;
			});
		};

		this.refreshTimer = setInterval(tick, 5_000);
		this.refreshTimer.unref();
		tick();
	}
	/** One automatic source read at a time; also a deterministic fixture entry point. */
	refreshDue(now = Date.now()): Promise<void> {
		if (this.controller.signal.aborted) return Promise.resolve();
		if (this.refreshing) return this.refreshing;
		const work = this.refreshNext(now);
		this.refreshing = work;

		void work
			.finally(() => {
				if (this.refreshing === work) this.refreshing = undefined;
			})
			.catch(() => undefined);

		return work;
	}
	private async refreshNext(now: number): Promise<void> {
		const due: Array<{ projectId: string; interval: number; state: RefreshRecord }> = [];
		const existing = new Set<string>();

		for (const project of this.repo.store.list("projects")) {
			if (project.kind === "standalone") continue;
			let context: Context;

			try {
				context = this.context(project.id);
			} catch (error) {
				this.refreshes.set(project.id, {
					key: "invalid",
					nextAt: now + 300_000,
					running: false,
					error: issue(error).message,
				});

				existing.add(project.id);
				continue;
			}

			if (!context.config.refresh?.enabled) {
				this.refreshes.delete(project.id);
				continue;
			}

			existing.add(project.id);
			const interval = context.config.refresh.intervalMs ?? 300_000;
			const key = hash({ source: context.source, config: context.config });
			let state = this.refreshes.get(project.id);

			if (!state || state.key !== key) {
				state = { key, nextAt: now, running: false };
				this.refreshes.set(project.id, state);
			}

			if (state.nextAt <= now && !this.busy.has(project.id))
				due.push({ projectId: project.id, interval, state });
		}

		for (const id of this.refreshes.keys()) if (!existing.has(id)) this.refreshes.delete(id);
		const next = due.sort((a, b) => a.state.nextAt - b.state.nextAt)[0];
		if (!next) return;
		const { projectId, interval, state } = next;
		state.running = true;
		state.lastAttemptAt = new Date(now).toISOString();
		state.nextAt = now + interval;

		try {
			const result = await this.sync(projectId);
			state.error = result.errors[0]?.message;
			if (!result.errors.length) state.lastSyncedAt = result.syncedAt;
			this.refreshError = undefined;
		} catch (error) {
			state.error = issue(error).message;
		} finally {
			state.running = false;
			this.options.changed?.(`/v1/projects/${encodeURIComponent(projectId)}/integration`);
		}
	}
	private refreshStatus(context: Context): NonNullable<IntegrationInfo["refresh"]> {
		const policy = context.config.refresh;
		const state = this.refreshes.get(context.project.id);

		return {
			enabled: policy?.enabled ?? false,
			intervalMs: policy?.intervalMs ?? 300_000,
			running: state?.running ?? false,
			lastAttemptAt: state?.lastAttemptAt,
			lastSyncedAt: state?.lastSyncedAt,
			nextSyncAt: policy?.enabled && state ? new Date(state.nextAt).toISOString() : undefined,
			error: state?.error ?? this.refreshError,
		};
	}
	async close(): Promise<void> {
		if (this.refreshTimer) clearInterval(this.refreshTimer);
		this.refreshTimer = undefined;
		this.controller.abort(new Error("Integration service stopped"));
		await Promise.allSettled(this.active);
		await this.refreshing?.catch(() => undefined);
	}
}
