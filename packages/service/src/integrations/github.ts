import type { Project, TaskStatus } from "@karabiner/contracts";
import {
	GithubTransport,
	IntegrationError,
	nextCursor,
	nodes,
	record,
	string,
	timestamp,
} from "./transport.ts";
import type {
	GithubIntegrationConfig,
	RemoteCollection,
	RemoteTask,
	SyncField,
	TaskProjection,
} from "./types.ts";
import { mappedStatus } from "./status-map.ts";

const contentFields = `__typename ... on Issue { id number title body url updatedAt state stateReason repository { nameWithOwner } labels(first:100) { nodes { name } pageInfo { hasNextPage endCursor } } } ... on DraftIssue { id title body updatedAt } ... on PullRequest { id number title body url updatedAt state repository { nameWithOwner } }`;
const statusFields = `nodes { ... on ProjectV2ItemFieldSingleSelectValue { optionId name field { ... on ProjectV2SingleSelectField { id name } } } } pageInfo { hasNextPage endCursor }`;
const itemFields = `id isArchived project { id url } content { ${contentFields} } fieldValues(first:100) { ${statusFields} }`;

export function githubRepository(value: string | null, hostname: string): string {
	let name = value ?? "";

	if (name.startsWith("https://")) {
		const url = new URL(name);

		if (url.hostname !== hostname || url.username || url.password || url.search || url.hash)
			throw new IntegrationError(
				"invalid_configuration",
				"Repository URL must use the configured GitHub host without credentials",
				false,
				400,
			);

		name = url.pathname.replace(/^\//, "").replace(/\/$/, "");
	}

	name = name.replace(/\.git$/, "");

	if (!/^[A-Za-z\d][A-Za-z\d-]*\/[A-Za-z\d_.-]+$/.test(name) || name.endsWith("/.."))
		throw new IntegrationError(
			"invalid_configuration",
			"Set the GitHub repository to owner/name",
			false,
			400,
		);

	return name;
}

function description(body: unknown): { description: string; operationId?: string } {
	const content = body == null ? "" : string(body);
	const marker = /\n\n<!-- karabiner-sync:([a-f\d]{64}) -->$/.exec(content);

	return {
		description: marker ? content.slice(0, marker.index) : content,
		operationId: marker?.[1],
	};
}

function stateStatus(state: unknown, reason: unknown): TaskStatus | undefined {
	if (state === "OPEN" || state === "open") return "todo";

	if (
		reason === "NOT_PLANNED" ||
		reason === "not_planned" ||
		reason === "DUPLICATE" ||
		reason === "duplicate"
	)
		return undefined;

	return state === "CLOSED" || state === "closed" || state === "MERGED" ? "done" : undefined;
}

function conventionalStatus(name: string): TaskStatus | undefined {
	const key = name.toLowerCase().replace(/[ -]+/g, "_");

	return (
		{
			backlog: "backlog",
			todo: "todo",
			in_progress: "in_progress",
			in_review: "in_review",
			blocked: "blocked",
			done: "done",
		} as Record<string, TaskStatus>
	)[key];
}

export class GithubIntegration {
	constructor(
		readonly project: Project,
		readonly config: GithubIntegrationConfig,
		readonly client: GithubTransport,
	) {}
	async probe(): Promise<void> {
		await this.client.request("user");

		if (this.config.scope === "project") await this.projectIdentity();
		else await this.client.request(`repos/${this.repository()}`);
	}
	repository(): string {
		return githubRepository(this.project.repository, this.client.hostname);
	}
	async projectIdentity(): Promise<{ id: string; url: string }> {
		const ref = this.project.externalRef ?? "";
		let query: string;
		let variables: Record<string, unknown>;

		if (/^PVT_[A-Za-z\d_-]+$/.test(ref)) {
			query = `query($id:ID!){ node(id:$id){ ... on ProjectV2 { id url } } }`;
			variables = { id: ref };
		} else {
			let url: URL;

			try {
				url = new URL(ref);
			} catch {
				throw new IntegrationError(
					"invalid_configuration",
					"Set externalRef to a GitHub Projects v2 URL or node ID",
					false,
					400,
				);
			}

			const match = /^\/(orgs|users)\/([^/]+)\/projects\/(\d+)(?:\/|$)/.exec(url.pathname);

			if (
				url.protocol !== "https:" ||
				url.hostname !== this.client.hostname ||
				url.username ||
				url.password ||
				!match
			)
				throw new IntegrationError(
					"invalid_configuration",
					"GitHub project URL must match the configured host",
					false,
					400,
				);

			query = `query($owner:String!,$number:Int!){ owner:${match[1] === "orgs" ? "organization" : "user"}(login:$owner){ projectV2(number:$number){ id url } } }`;
			variables = { owner: match[2], number: Number(match[3]) };
		}

		const data = await this.client.graphql(query, variables);
		const project = record(data.node ?? record(data.owner).projectV2);
		return { id: string(project.id), url: string(project.url) };
	}
	async list(): Promise<RemoteCollection> {
		if (this.config.scope === "repository") {
			const values = await this.client.list(
				`repos/${this.repository()}/issues?state=all&sort=created&direction=asc`,
			);

			return {
				tasks: values.filter((v) => !v.pull_request).map((v) => this.restIssue(v)),
				errors: [],
			};
		}

		const project = await this.projectIdentity();
		const tasks: RemoteTask[] = [];
		const errors: RemoteCollection["errors"] = [];
		let after: string | undefined;
		const seen = new Set<string>();

		do {
			const data = await this.client.graphql(
				`query($id:ID!,$after:String){node(id:$id){... on ProjectV2 {items(first:50,after:$after){nodes {${itemFields}} pageInfo {hasNextPage endCursor}}}}}`,
				{ id: project.id, after },
			);

			const connection = record(data.node).items;

			for (const item of nodes(connection)) {
				if (!item.content) {
					errors.push({
						id: string(item.id),
						code: "inaccessible_item",
						message: "Project item content is not accessible",
					});

					continue;
				}

				tasks.push(await this.projectItem(item));
			}

			after = nextCursor(connection);

			if (after && seen.has(after))
				throw new IntegrationError(
					"invalid_response",
					"GitHub repeated its pagination cursor",
				);

			if (after) seen.add(after);
		} while (after);

		return { tasks, errors };
	}
	private restIssue(raw: Record<string, unknown>): RemoteTask {
		const body = description(raw.body);
		const number = Number(raw.number);

		if (!Number.isSafeInteger(number) || number < 1)
			throw new IntegrationError(
				"invalid_response",
				"GitHub returned an invalid issue number",
			);

		const reason = typeof raw.state_reason === "string" ? raw.state_reason : "";
		const state = string(raw.state);

		const mapped =
			mappedStatus(reason, this.config.pullStatusMap, this.config.pushStatusMap) ??
			mappedStatus(state, this.config.pullStatusMap, this.config.pushStatusMap);

		return {
			external: {
				provider: "github",
				id: string(raw.node_id),
				identifier: `${this.repository()}#${number}`,
				url: string(raw.html_url),
				updatedAt: timestamp(raw.updated_at),
				kind: "issue",
			},
			values: {
				title: string(raw.title),
				description: body.description,
				status: mapped ?? stateStatus(raw.state, raw.state_reason),
				labels: (raw.labels as unknown[])
					.map((label) =>
						typeof label === "string" ? label : string(record(label).name),
					)
					.sort(),
			},
			repository: this.repository(),
			state: typeof raw.state_reason === "string" ? raw.state_reason : string(raw.state),
			operationId: body.operationId,
		};
	}
	private async projectItem(item: Record<string, unknown>): Promise<RemoteTask> {
		const content = record(item.content);
		const project = record(item.project);

		const kind =
			content.__typename === "Issue"
				? "issue"
				: content.__typename === "DraftIssue"
					? "draft"
					: content.__typename === "PullRequest"
						? "pull_request"
						: undefined;

		if (!kind)
			throw new IntegrationError(
				"unsupported_item",
				"GitHub returned an unsupported project item type",
			);

		const body = description(content.body);

		const repository =
			kind === "draft" ? undefined : string(record(content.repository).nameWithOwner);

		const fields = await this.itemStatusFields(item);

		const status = fields.find(
			(field) =>
				field.field &&
				(this.config.statusFieldId
					? record(field.field).id === this.config.statusFieldId
					: record(field.field).name === "Status"),
		);

		const optionId = status ? string(status.optionId) : undefined;

		const mapped = optionId
			? (mappedStatus(optionId, this.config.pullStatusMap, this.config.pushStatusMap) ??
				conventionalStatus(string(status!.name)))
			: undefined;

		const values: TaskProjection = {
			title: string(content.title),
			description: body.description,
			status: mapped ?? stateStatus(content.state, content.stateReason),
		};

		if (kind === "issue") {
			values.labels = nextCursor(content.labels)
				? (
						await this.client.list(
							`repos/${repository}/issues/${Number(content.number)}/labels`,
						)
					)
						.map((l) => string(l.name))
						.sort()
				: nodes(content.labels)
						.map((l) => string(l.name))
						.sort();
		}

		return {
			external: {
				provider: "github",
				id: string(content.id),
				identifier:
					kind === "draft"
						? `draft:${string(content.id)}`
						: `${repository}#${Number(content.number)}`,
				url:
					kind === "draft"
						? `${string(project.url)}?pane=issue&itemId=${encodeURIComponent(string(item.id))}`
						: string(content.url),
				updatedAt: timestamp(content.updatedAt),
				projectItemId: string(item.id),
				kind,
			},
			values,
			repository,
			stateId: optionId,
			state: typeof content.state === "string" ? content.state : undefined,
			projectId: string(project.id),
			archived: item.isArchived === true,
			operationId: body.operationId,
		};
	}
	private async itemStatusFields(
		item: Record<string, unknown>,
	): Promise<Record<string, unknown>[]> {
		const fields = nodes(item.fieldValues);
		let after = nextCursor(item.fieldValues);
		const seen = new Set<string>();

		while (after) {
			if (seen.has(after))
				throw new IntegrationError(
					"invalid_response",
					"GitHub repeated a field pagination cursor",
				);

			seen.add(after);

			const data = await this.client.graphql(
				`query($id:ID!,$after:String){node(id:$id){... on ProjectV2Item {fieldValues(first:100,after:$after){${statusFields}}}}}`,
				{ id: item.id, after },
			);

			const connection = record(data.node).fieldValues;
			fields.push(...nodes(connection));
			after = nextCursor(connection);
		}

		return fields;
	}
	async get(remote: RemoteTask): Promise<RemoteTask> {
		if (remote.external.projectItemId) {
			const data = await this.client.graphql(
				`query($id:ID!){node(id:$id){... on ProjectV2Item {${itemFields}}}}`,
				{ id: remote.external.projectItemId },
			);

			return this.projectItem(record(data.node));
		}

		const number = remote.external.identifier.split("#").at(-1)!;

		const current = this.restIssue(
			record(await this.client.request(`repos/${remote.repository}/issues/${number}`)),
		);

		if (current.external.id !== remote.external.id)
			throw new IntegrationError("remote_identity_changed", "GitHub issue identity changed");

		return current;
	}
	async create(values: TaskProjection, operationId: string): Promise<RemoteTask> {
		const body = `${values.description}\n\n<!-- karabiner-sync:${operationId} -->`;

		if (this.config.scope === "project" && !this.project.repository) {
			const project = await this.projectIdentity();

			const data = await this.client.graphql(
				`mutation($input:AddProjectV2DraftIssueInput!){addProjectV2DraftIssue(input:$input){projectItem {${itemFields}}}}`,
				{ input: { projectId: project.id, title: values.title, body } },
			);

			return this.projectItem(record(record(data.addProjectV2DraftIssue).projectItem));
		}

		const created = this.restIssue(
			record(
				await this.client.request(`repos/${this.repository()}/issues`, "POST", {
					title: values.title,
					body,
				}),
			),
		);

		if (this.config.scope !== "project") return created;
		return this.addToProject(created);
	}
	private async addToProject(created: RemoteTask): Promise<RemoteTask> {
		const project = await this.projectIdentity();

		const data = await this.client.graphql(
			`mutation($input:AddProjectV2ItemByIdInput!){addProjectV2ItemById(input:$input){item {${itemFields}}}}`,
			{ input: { projectId: project.id, contentId: created.external.id } },
		);

		return this.projectItem(record(record(data.addProjectV2ItemById).item));
	}
	async recoverCreate(operationId: string): Promise<RemoteTask | undefined> {
		const projectItems = await this.list();
		const found = projectItems.tasks.filter((task) => task.operationId === operationId);

		if (found.length > 1)
			throw new IntegrationError(
				"creation_ambiguous",
				"Several GitHub items match this creation request",
				true,
			);

		if (found[0] || this.config.scope !== "project" || !this.project.repository)
			return found[0];

		// An issue POST may succeed before adding it to the project fails. Reconcile
		// the repository marker, then use GitHub's idempotent add-item operation.
		const issues = (await this.client.list(`repos/${this.repository()}/issues?state=all`))
			.filter((v) => !v.pull_request)
			.map((v) => this.restIssue(v));

		const matches = issues.filter((task) => task.operationId === operationId);

		if (matches.length > 1)
			throw new IntegrationError(
				"creation_ambiguous",
				"Several GitHub issues match this creation request",
				true,
			);

		return matches[0] ? this.addToProject(matches[0]) : undefined;
	}
	async update(
		remote: RemoteTask,
		values: TaskProjection,
		fields: SyncField[],
	): Promise<RemoteTask> {
		if (fields.some((f) => f === "priority" || f === "dueAt"))
			throw new IntegrationError(
				"unsupported_field",
				"GitHub sync supports title, description, status and issue labels",
				false,
				400,
			);

		if (remote.external.kind === "draft" && fields.includes("labels"))
			throw new IntegrationError(
				"unsupported_field",
				"Draft project items do not have labels",
				false,
				400,
			);

		if (remote.external.kind === "pull_request" && fields.some((f) => f !== "status"))
			throw new IntegrationError(
				"unsupported_field",
				"Project pull requests support board-status push only",
				false,
				400,
			);

		if (fields.includes("status")) {
			if (this.config.scope === "repository") this.repositoryState(values.status);

			if (
				this.config.scope === "project" &&
				(!values.status ||
					!this.config.statusFieldId ||
					!this.config.pushStatusMap?.[values.status] ||
					!remote.external.projectItemId)
			)
				throw new IntegrationError(
					"status_mapping_required",
					"Configure GitHub statusFieldId and a push status option ID",
					false,
					409,
				);
		}

		const body: Record<string, unknown> = {};
		if (fields.includes("title")) body.title = values.title;

		if (fields.includes("description"))
			body.body =
				values.description +
				(remote.operationId ? `\n\n<!-- karabiner-sync:${remote.operationId} -->` : "");

		if (fields.includes("labels")) body.labels = values.labels;

		if (fields.includes("status") && this.config.scope === "repository") {
			Object.assign(body, this.repositoryState(values.status));
		}

		if (Object.keys(body).length) await this.updateContent(remote, body);

		if (fields.includes("status") && this.config.scope === "project")
			await this.updateStatus(remote, values.status);

		return this.get(remote);
	}
	private repositoryState(status: TaskStatus | undefined): {
		state: string;
		state_reason: string;
	} {
		const mapped =
			status &&
			(this.config.pushStatusMap?.[status] ??
				(status === "done" ? "completed" : status === "todo" ? "open" : undefined));

		if (mapped === "open" || mapped === "reopened")
			return { state: "open", state_reason: "reopened" };

		if (mapped === "closed" || mapped === "completed")
			return { state: "closed", state_reason: "completed" };

		if (mapped === "not_planned") return { state: "closed", state_reason: "not_planned" };

		throw new IntegrationError(
			"status_mapping_required",
			"Map the task status to Open, Completed or Not planned; use a GitHub Project for additional board states",
			false,
			409,
		);
	}
	private async updateContent(remote: RemoteTask, body: Record<string, unknown>): Promise<void> {
		if (remote.external.kind === "draft") {
			if (body.labels)
				throw new IntegrationError(
					"unsupported_field",
					"Draft project items do not have labels",
					false,
					400,
				);

			await this.client.graphql(
				`mutation($input:UpdateProjectV2DraftIssueInput!){updateProjectV2DraftIssue(input:$input){draftIssue{id}}}`,
				{ input: { draftIssueId: remote.external.id, ...body } },
			);

			return;
		}

		if (remote.external.kind === "pull_request")
			throw new IntegrationError(
				"unsupported_field",
				"Project pull requests support board-status push only",
				false,
				400,
			);

		const number = remote.external.identifier.split("#").at(-1)!;
		await this.client.request(`repos/${remote.repository}/issues/${number}`, "PATCH", body);
	}
	private async updateStatus(remote: RemoteTask, status: TaskStatus | undefined): Promise<void> {
		const option = status && this.config.pushStatusMap?.[status];

		if (!option || !this.config.statusFieldId || !remote.external.projectItemId)
			throw new IntegrationError(
				"status_mapping_required",
				"Configure GitHub statusFieldId and a push status option ID",
				false,
				409,
			);

		await this.client.graphql(
			`mutation($input:UpdateProjectV2ItemFieldValueInput!){updateProjectV2ItemFieldValue(input:$input){projectV2Item{id}}}`,
			{
				input: {
					projectId: remote.projectId,
					itemId: remote.external.projectItemId,
					fieldId: this.config.statusFieldId,
					value: { singleSelectOptionId: option },
				},
			},
		);
	}
}
