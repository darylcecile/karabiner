import type { Project, TaskStatus } from "@karabiner/contracts";
import {
	IntegrationError,
	LinearTransport,
	nextCursor,
	nodes,
	record,
	string,
	timestamp,
} from "./transport.ts";
import type {
	LinearIntegrationConfig,
	RemoteCollection,
	RemoteTask,
	SyncField,
	TaskProjection,
} from "./types.ts";
import { mappedStatus } from "./status-map.ts";

const issueFields = `id identifier title description url updatedAt archivedAt priority dueDate team{id} project{id} state{id name type} labels(first:100){nodes{id name} pageInfo{hasNextPage endCursor}}`;
const priorities = { 1: "urgent", 2: "high", 3: "normal", 4: "low" } as const;

const stateTypes: Record<string, TaskStatus> = {
	triage: "backlog",
	backlog: "backlog",
	unstarted: "todo",
	started: "in_progress",
	completed: "done",
};

function projectReference(ref: string): string {
	if (!/^https?:\/\//i.test(ref)) return ref;
	let url: URL;

	try {
		url = new URL(ref);
	} catch {
		throw new IntegrationError(
			"invalid_configuration",
			"Linear project URL is invalid",
			false,
			400,
		);
	}

	const match = /\/project\/([^/]+)/.exec(url.pathname);

	if (
		url.protocol !== "https:" ||
		url.hostname !== "linear.app" ||
		url.username ||
		url.password ||
		!match
	)
		throw new IntegrationError(
			"invalid_configuration",
			"Use a Linear project URL, UUID or project identifier",
			false,
			400,
		);

	const slug = decodeURIComponent(match[1]!);
	// Project URLs append the stable slugId to the human-readable title.
	return /-([a-f\d]{12})$/i.exec(slug)?.[1] ?? slug;
}

export class LinearIntegration {
	constructor(
		readonly project: Project,
		readonly config: LinearIntegrationConfig,
		readonly client: LinearTransport,
	) {}
	async identity(): Promise<{ id: string; teamIds: string[] }> {
		const ref = this.project.externalRef;

		if (!ref)
			throw new IntegrationError(
				"invalid_configuration",
				"Set externalRef to a Linear team key or project UUID/identifier",
				false,
				400,
			);

		if (this.config.scope === "project") {
			const data = await this.client.graphql(
				`query($id:String!){project(id:$id){id teams(first:100){nodes{id} pageInfo{hasNextPage endCursor}}}}`,
				{ id: projectReference(ref) },
			);

			const project = record(data.project);
			const ids = nodes(project.teams).map((team) => string(team.id));
			let after = nextCursor(project.teams);
			const seen = new Set<string>();

			while (after) {
				if (seen.has(after))
					throw new IntegrationError(
						"invalid_response",
						"Linear repeated a team pagination cursor",
					);

				seen.add(after);

				const page = await this.client.graphql(
					`query($id:String!,$after:String){project(id:$id){teams(first:100,after:$after){nodes{id} pageInfo{hasNextPage endCursor}}}}`,
					{ id: project.id, after },
				);

				const teams = record(page.project).teams;
				ids.push(...nodes(teams).map((team) => string(team.id)));
				after = nextCursor(teams);
			}

			return { id: string(project.id), teamIds: ids };
		}

		if (/^[a-f\d-]{36}$/i.test(ref)) {
			const data = await this.client.graphql(`query($id:String!){team(id:$id){id}}`, {
				id: ref,
			});

			const id = string(record(data.team).id);
			return { id, teamIds: [id] };
		}

		const data = await this.client.graphql(
			`query($key:String!){teams(filter:{key:{eq:$key}},first:2){nodes{id}}}`,
			{ key: ref },
		);

		const teams = nodes(data.teams);

		if (teams.length !== 1)
			throw new IntegrationError(
				"team_not_found",
				"Linear team key did not resolve to one accessible team",
				false,
				409,
			);

		const id = string(teams[0]!.id);
		return { id, teamIds: [id] };
	}
	async probe(): Promise<void> {
		await this.client.graphql("query { viewer { id } }");
		await this.identity();
	}
	async list(): Promise<RemoteCollection> {
		const identity = await this.identity();
		const filter = { [this.config.scope]: { id: { eq: identity.id } } };
		const tasks: RemoteTask[] = [];
		let after: string | undefined;
		const seen = new Set<string>();

		do {
			const data = await this.client.graphql(
				`query($filter:IssueFilter!,$after:String){issues(filter:$filter,first:50,after:$after,includeArchived:true){nodes{${issueFields}} pageInfo{hasNextPage endCursor}}}`,
				{ filter, after },
			);

			for (const raw of nodes(data.issues)) tasks.push(await this.normalize(raw));
			after = nextCursor(data.issues);

			if (after && seen.has(after))
				throw new IntegrationError(
					"invalid_response",
					"Linear repeated an issue pagination cursor",
				);

			if (after) seen.add(after);
		} while (after);

		return { tasks, errors: [] };
	}
	private async normalize(raw: Record<string, unknown>): Promise<RemoteTask> {
		const state = record(raw.state);
		const labelIds: Record<string, string> = {};
		let labels = raw.labels;
		const seen = new Set<string>();

		while (true) {
			for (const label of nodes(labels)) labelIds[string(label.name)] = string(label.id);
			const after = nextCursor(labels);
			if (!after) break;

			if (seen.has(after))
				throw new IntegrationError(
					"invalid_response",
					"Linear repeated a label pagination cursor",
				);

			seen.add(after);

			const data = await this.client.graphql(
				`query($id:String!,$after:String){issue(id:$id){labels(first:100,after:$after){nodes{id name} pageInfo{hasNextPage endCursor}}}}`,
				{ id: raw.id, after },
			);

			labels = record(data.issue).labels;
		}

		const dueDate = raw.dueDate == null ? null : string(raw.dueDate);

		if (dueDate && !/^\d{4}-\d{2}-\d{2}$/.test(dueDate))
			throw new IntegrationError("invalid_response", "Linear returned an invalid due date");

		return {
			external: {
				provider: "linear",
				id: string(raw.id),
				identifier: string(raw.identifier),
				url: string(raw.url),
				updatedAt: timestamp(raw.updatedAt),
				kind: "issue",
			},
			values: {
				title: string(raw.title),
				description: raw.description == null ? "" : string(raw.description),
				status:
					mappedStatus(
						string(state.id),
						this.config.pullStatusMap,
						this.config.pushStatusMap?.[string(record(raw.team).id)],
					) ?? stateTypes[string(state.type)],
				priority: priorities[raw.priority as keyof typeof priorities],
				labels: Object.keys(labelIds).sort(),
				dueAt: dueDate ? timestamp(`${dueDate}T00:00:00.000Z`) : null,
			},
			teamId: string(record(raw.team).id),
			projectId: raw.project ? string(record(raw.project).id) : undefined,
			stateId: string(state.id),
			state: string(state.type),
			archived: raw.archivedAt != null,
			labelIds,
		};
	}
	async get(remote: RemoteTask): Promise<RemoteTask> {
		return this.getId(remote.external.id);
	}
	async getId(id: string): Promise<RemoteTask> {
		const data = await this.client.graphql(
			`query($id:String!){issue(id:$id){${issueFields}}}`,
			{ id },
		);

		return this.normalize(record(data.issue));
	}
	async create(values: TaskProjection, id: string): Promise<RemoteTask> {
		const identity = await this.identity();

		const teamId =
			this.config.teamId ?? (identity.teamIds.length === 1 ? identity.teamIds[0] : undefined);

		if (!teamId || !identity.teamIds.includes(teamId))
			throw new IntegrationError(
				"team_required",
				"Choose a creation team belonging to this Linear project",
				false,
				409,
			);

		const input = {
			id,
			teamId,
			title: values.title,
			description: values.description,
			...(this.config.scope === "project" ? { projectId: identity.id } : {}),
		};

		const data = await this.client.graphql(
			`mutation($input:IssueCreateInput!){issueCreate(input:$input){success issue{${issueFields}}}}`,
			{ input },
		);

		return this.mutationResult(data.issueCreate);
	}
	async update(
		remote: RemoteTask,
		values: TaskProjection,
		fields: SyncField[],
	): Promise<RemoteTask> {
		const input: Record<string, unknown> = {};
		if (fields.includes("title")) input.title = values.title;
		if (fields.includes("description")) input.description = values.description;

		if (fields.includes("priority"))
			input.priority = { urgent: 1, high: 2, normal: 3, low: 4 }[values.priority!];

		if (fields.includes("dueAt"))
			input.dueDate = values.dueAt ? values.dueAt.slice(0, 10) : null;

		if (fields.includes("status")) {
			const id =
				values.status && this.config.pushStatusMap?.[remote.teamId!]?.[values.status];

			if (!id)
				throw new IntegrationError(
					"status_mapping_required",
					"Configure a push state ID for this task status and Linear team",
					false,
					409,
				);

			input.stateId = id;
		}

		if (fields.includes("labels"))
			input.labelIds = await this.resolveLabels(remote.teamId!, values.labels ?? []);

		const data = await this.client.graphql(
			`mutation($id:String!,$input:IssueUpdateInput!){issueUpdate(id:$id,input:$input){success issue{${issueFields}}}}`,
			{ id: remote.external.id, input },
		);

		return this.mutationResult(data.issueUpdate);
	}
	private async mutationResult(value: unknown): Promise<RemoteTask> {
		const result = record(value);

		if (result.success !== true || !result.issue)
			throw new IntegrationError(
				"mutation_unconfirmed",
				"Linear did not confirm the issue mutation",
				true,
			);

		return this.normalize(record(result.issue));
	}
	private async resolveLabels(teamId: string, names: string[]): Promise<string[]> {
		if (!names.length) return [];
		const found = new Map<string, string[]>();
		let after: string | undefined;
		const seen = new Set<string>();

		do {
			const data = await this.client.graphql(
				`query($filter:IssueLabelFilter!,$after:String){issueLabels(filter:$filter,first:100,after:$after){nodes{id name} pageInfo{hasNextPage endCursor}}}`,
				{
					filter: { or: [{ team: { id: { eq: teamId } } }, { team: { null: true } }] },
					after,
				},
			);

			for (const label of nodes(data.issueLabels)) {
				const name = string(label.name);
				found.set(name, [...(found.get(name) ?? []), string(label.id)]);
			}

			after = nextCursor(data.issueLabels);

			if (after && seen.has(after))
				throw new IntegrationError(
					"invalid_response",
					"Linear repeated a label pagination cursor",
				);

			if (after) seen.add(after);
		} while (after);

		return names.map((name) => {
			const matches = found.get(name);

			if (matches?.length !== 1)
				throw new IntegrationError(
					"label_mapping_required",
					`Linear label ${name} is missing or ambiguous for this team`,
					false,
					409,
				);

			return matches[0]!;
		});
	}
}
