import type { IntegrationCatalog, TaskStatus } from "@karabiner/contracts";
import { GithubIntegration } from "./github.ts";
import { LinearIntegration } from "./linear.ts";
import { IntegrationError, nextCursor, nodes, record, string } from "./transport.ts";

type StatusOption = IntegrationCatalog["statusFields"][number]["options"][number];
type Team = IntegrationCatalog["teams"][number];

function suggestedStatus(name: string, type?: string): TaskStatus | undefined {
	const key = name.toLowerCase().replace(/[ -]+/g, "_");

	const names: Record<string, TaskStatus> = {
		backlog: "backlog",
		todo: "todo",
		to_do: "todo",
		in_progress: "in_progress",
		in_review: "in_review",
		blocked: "blocked",
		done: "done",
	};

	const types: Record<string, TaskStatus> = {
		triage: "backlog",
		backlog: "backlog",
		unstarted: "todo",
		started: "in_progress",
		completed: "done",
	};

	return names[key] ?? (type ? types[type] : undefined);
}

function option(raw: Record<string, unknown>): StatusOption {
	const name = string(raw.name);

	return {
		id: string(raw.id),
		name,
		...(typeof raw.color === "string" ? { color: raw.color } : {}),
		suggestedStatus: suggestedStatus(name, typeof raw.type === "string" ? raw.type : undefined),
	};
}

/** Reads the source's named choices; IDs are returned for the GUI to bind, not entered by users. */
export async function githubCatalog(integration: GithubIntegration): Promise<IntegrationCatalog> {
	const base: IntegrationCatalog = {
		provider: "github",
		projectId: integration.project.id,
		source: { id: "", name: "" },
		teams: [],
		statusFields: [],
		writableFields: ["title", "description", "status", "labels"],
		checkedAt: new Date().toISOString(),
	};

	if (integration.config.scope === "repository") {
		const repo = record(await integration.client.request(`repos/${integration.repository()}`));

		return {
			...base,
			statusFields: [
				{
					id: "issue-state",
					name: "Issue state",
					options: [
						{ id: "open", name: "Open", suggestedStatus: "todo" },
						{ id: "completed", name: "Completed", suggestedStatus: "done" },
						{ id: "not_planned", name: "Not planned" },
					],
				},
			],
			source: {
				id: string(repo.node_id),
				name: string(repo.full_name),
				url: string(repo.html_url),
			},
		};
	}

	const source = await integration.projectIdentity();
	let after: string | undefined;
	const seen = new Set<string>();

	do {
		const data = await integration.client.graphql(
			`query($id:ID!,$after:String){node(id:$id){... on ProjectV2 {id title url fields(first:100,after:$after){nodes{... on ProjectV2SingleSelectField{id name options{id name}}} pageInfo{hasNextPage endCursor}}}}}`,
			{ id: source.id, after },
		);

		const project = record(data.node);

		base.source = {
			id: string(project.id),
			name: string(project.title),
			url: string(project.url),
		};

		for (const field of nodes(project.fields)) {
			if (!Array.isArray(field.options)) continue;

			base.statusFields.push({
				id: string(field.id),
				name: string(field.name),
				options: field.options.map((value) => option(record(value))),
			});
		}

		after = nextCursor(project.fields);

		if (after && seen.has(after))
			throw new IntegrationError(
				"invalid_response",
				"GitHub repeated a catalog pagination cursor",
			);

		if (after) seen.add(after);
	} while (after);

	base.checkedAt = new Date().toISOString();
	return base;
}

export async function linearCatalog(integration: LinearIntegration): Promise<IntegrationCatalog> {
	const identity = await integration.identity();
	const teams: Team[] = [];
	for (const id of identity.teamIds) teams.push(await linearTeam(integration, id));

	let source: IntegrationCatalog["source"] = {
		id: identity.id,
		name: teams[0]?.name ?? integration.project.name,
	};

	if (integration.config.scope === "project") {
		const data = await integration.client.graphql(
			`query($id:String!){project(id:$id){id name url}}`,
			{ id: identity.id },
		);

		const project = record(data.project);
		source = { id: string(project.id), name: string(project.name), url: string(project.url) };
	}

	return {
		provider: "linear",
		projectId: integration.project.id,
		source,
		teams,
		statusFields: [],
		writableFields: ["title", "description", "status", "priority", "labels", "dueAt"],
		checkedAt: new Date().toISOString(),
	};
}

async function linearTeam(integration: LinearIntegration, id: string): Promise<Team> {
	const result: Team = { id, name: "", states: [] };
	let after: string | undefined;
	const seen = new Set<string>();

	do {
		const data = await integration.client.graphql(
			`query($id:String!,$after:String){team(id:$id){id name key states(first:100,after:$after){nodes{id name type color} pageInfo{hasNextPage endCursor}}}}`,
			{ id, after },
		);

		const team = record(data.team);
		result.name = string(team.name);
		result.key = string(team.key);

		for (const state of nodes(team.states))
			result.states.push({ ...option(state), type: string(state.type) });

		after = nextCursor(team.states);

		if (after && seen.has(after))
			throw new IntegrationError(
				"invalid_response",
				"Linear repeated a catalog pagination cursor",
			);

		if (after) seen.add(after);
	} while (after);

	return result;
}
