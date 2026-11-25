import type { Database } from "bun:sqlite";
import type { ExternalLink, Provider, PushOperation } from "./types.ts";

interface LinkRow {
	data: string;
	task_id: string | null;
}

/** Tables are created by the service's versioned schema migration, not this module. */
export class IntegrationStore {
	constructor(readonly db: Database) {}
	links(projectId: string, provider: Provider): ExternalLink[] {
		return this.db
			.query<LinkRow, [string, string]>(
				"SELECT data,task_id FROM external_task_links WHERE project_id=? AND provider=?",
			)
			.all(projectId, provider)
			.map((row) => ({ ...JSON.parse(row.data), taskId: row.task_id }));
	}
	forTask(taskId: string): ExternalLink | undefined {
		const row = this.db
			.query<LinkRow, [string]>(
				"SELECT data,task_id FROM external_task_links WHERE task_id=?",
			)
			.get(taskId);

		return row ? { ...JSON.parse(row.data), taskId: row.task_id } : undefined;
	}
	put(link: ExternalLink, previousExternalId?: string): void {
		if (previousExternalId && previousExternalId !== link.externalId)
			this.db
				.query(
					"DELETE FROM external_task_links WHERE project_id=? AND provider=? AND external_id=?",
				)
				.run(link.projectId, link.provider, previousExternalId);

		this.db
			.query(
				"INSERT INTO external_task_links(project_id,provider,external_id,task_id,data) VALUES (?,?,?,?,?) ON CONFLICT(project_id,provider,external_id) DO UPDATE SET task_id=excluded.task_id,data=excluded.data",
			)
			.run(link.projectId, link.provider, link.externalId, link.taskId, JSON.stringify(link));
	}
	operation(requestId: string): PushOperation | undefined {
		const row = this.db
			.query<{ data: string }, [string]>(
				"SELECT data FROM integration_operations WHERE request_id=?",
			)
			.get(requestId);

		return row ? JSON.parse(row.data) : undefined;
	}
	putOperation(operation: PushOperation): void {
		this.db
			.query(
				"INSERT INTO integration_operations(request_id,task_id,data) VALUES (?,?,?) ON CONFLICT(request_id) DO UPDATE SET data=excluded.data",
			)
			.run(operation.requestId, operation.taskId, JSON.stringify(operation));
	}
	pendingForTask(taskId: string): PushOperation | undefined {
		return this.db
			.query<{ data: string }, [string]>(
				"SELECT data FROM integration_operations WHERE task_id=?",
			)
			.all(taskId)
			.map((row) => JSON.parse(row.data) as PushOperation)
			.find((op) => op.state === "pending");
	}
}
