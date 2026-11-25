import { Database } from "bun:sqlite";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import type {
	Activity,
	Document,
	Intervention,
	Project,
	ProjectConfig,
	Task,
} from "@karabiner/contracts";
import { conflict, missing } from "./errors.ts";

interface Entities {
	projects: Project;
	tasks: Task;
	documents: Document;
	interventions: Intervention;
}
export type Table = keyof Entities;
interface Row {
	data: string;
	project_id?: string | null;
	task_id?: string | null;
}
export interface Filter {
	projectId?: string;
	taskId?: string;
}
function decode<T>(row: Row): T {
	const value = JSON.parse(row.data);
	if (row.project_id !== undefined) value.projectId = row.project_id;
	if (row.task_id !== undefined) value.taskId = row.task_id;
	return value as T;
}

const schema = `
  CREATE TABLE projects (id TEXT PRIMARY KEY, data TEXT NOT NULL);
  CREATE TABLE tasks (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, data TEXT NOT NULL);
  CREATE TABLE documents (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL, data TEXT NOT NULL);
  CREATE TABLE interventions (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL, data TEXT NOT NULL);
  CREATE TABLE project_configs (project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE, data TEXT NOT NULL);
  CREATE TABLE settings (key TEXT PRIMARY KEY, data TEXT NOT NULL);
  CREATE TABLE activity (id TEXT PRIMARY KEY, project_id TEXT REFERENCES projects(id) ON DELETE CASCADE, task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL, data TEXT NOT NULL);
  CREATE INDEX tasks_project ON tasks(project_id);
  CREATE INDEX documents_project ON documents(project_id);
  CREATE INDEX interventions_project ON interventions(project_id);
  CREATE INDEX activity_project ON activity(project_id);
  PRAGMA user_version = 1;
`;

const executionSchema = `
  CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    workspace_key TEXT NOT NULL,
    data TEXT NOT NULL
  );
  CREATE INDEX runs_project ON runs(project_id);
  CREATE INDEX runs_task ON runs(task_id);
  CREATE INDEX runs_state ON runs(state);
  CREATE UNIQUE INDEX runs_active_task ON runs(task_id) WHERE state IN ('queued','running','waiting','paused');
  CREATE TABLE scheduled_tasks (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE
  );
  CREATE TABLE run_blobs (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    key TEXT NOT NULL, data TEXT NOT NULL,
    PRIMARY KEY (run_id, key)
  );
  PRAGMA user_version = 2;
`;

const integrationSchema = `
  CREATE TABLE external_task_links (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider TEXT NOT NULL, external_id TEXT NOT NULL,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    data TEXT NOT NULL,
    PRIMARY KEY(project_id,provider,external_id), UNIQUE(task_id)
  );
  CREATE TABLE integration_operations (
    request_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    data TEXT NOT NULL
  );
  PRAGMA user_version = 3;
`;

export class Store {
	readonly db: Database;
	constructor(path: string) {
		if (path !== ":memory:") mkdirSync(dirname(path), { recursive: true });
		this.db = new Database(path, { strict: true });

		try {
			this.db.run(
				"PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL;",
			);

			const { user_version: version } = this.db
				.query<{ user_version: number }, []>("PRAGMA user_version")
				.get()!;

			if (version === 0) {
				const existing = this.db
					.query<{ count: number }, []>(
						"SELECT count(*) AS count FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
					)
					.get()!;

				if (existing.count)
					throw new Error(
						"Database has an unrecognized schema; choose an empty Karabiner database path",
					);

				this.db.transaction(() => this.db.run(schema))();
			} else if (![1, 2, 3].includes(version))
				throw new Error(
					`Unsupported database version ${version}; this service supports versions 1 to 3`,
				);

			if (version < 2) this.db.transaction(() => this.db.run(executionSchema))();
			if (version < 3) this.db.transaction(() => this.db.run(integrationSchema))();
		} catch (error) {
			this.db.close();
			throw error;
		}
	}
	close(): void {
		this.db.close();
	}
	atomic<T>(work: () => T): T {
		return this.db.transaction(work)();
	}
	get<K extends Table>(table: K, id: string): Entities[K] {
		const row = this.db.query<Row, [string]>(`SELECT * FROM ${table} WHERE id = ?`).get(id);
		if (!row) missing(table.slice(0, -1));
		return this.hydrate(table, row);
	}
	list<K extends Table>(table: K, filter: Filter = {}): Entities[K][] {
		const clauses: string[] = [];
		const params: string[] = [];

		if (filter.projectId && table !== "projects") {
			clauses.push("project_id = ?");
			params.push(filter.projectId);
		}

		if (filter.taskId && (table === "documents" || table === "interventions")) {
			clauses.push("task_id = ?");
			params.push(filter.taskId);
		}

		const sql = `SELECT * FROM ${table}${clauses.length ? ` WHERE ${clauses.join(" AND ")}` : ""} ORDER BY rowid`;

		return this.db
			.query<Row, string[]>(sql)
			.all(...params)
			.map((row) => this.hydrate(table, row));
	}
	private hydrate<K extends Table>(table: K, row: Row): Entities[K] {
		const value = decode<Entities[K]>(row);

		return table === "tasks"
			? { stageId: null, dueAt: null, sortOrder: 0, subtasks: [], ...value }
			: value;
	}
	put<K extends Table>(table: K, value: Entities[K]): Entities[K] {
		const columns = ["id", "data"];
		const params: (string | null)[] = [value.id, JSON.stringify(value)];

		if ("projectId" in value) {
			columns.push("project_id");
			params.push(value.projectId);
		}

		if ("taskId" in value) {
			columns.push("task_id");
			params.push(value.taskId);
		}

		const assignments = columns
			.slice(1)
			.map((column) => `${column}=excluded.${column}`)
			.join(", ");

		this.db
			.query(
				`INSERT INTO ${table} (${columns.join(", ")}) VALUES (${columns.map(() => "?").join(", ")}) ON CONFLICT(id) DO UPDATE SET ${assignments}`,
			)
			.run(...params);

		return value;
	}
	remove(table: Table, id: string): void {
		this.db.query(`DELETE FROM ${table} WHERE id=?`).run(id);
	}
	config(projectId: string): ProjectConfig {
		this.get("projects", projectId);

		const row = this.db
			.query<Row, [string]>("SELECT data FROM project_configs WHERE project_id=?")
			.get(projectId);

		if (!row) throw new Error(`Project configuration missing for ${projectId}`);
		return decode<ProjectConfig>(row);
	}
	putConfig(projectId: string, value: ProjectConfig): ProjectConfig {
		this.db
			.query(
				"INSERT INTO project_configs (project_id,data) VALUES (?,?) ON CONFLICT(project_id) DO UPDATE SET data=excluded.data",
			)
			.run(projectId, JSON.stringify(value));

		return value;
	}
	setting<T>(key: string, fallback: T): T {
		const row = this.db.query<Row, [string]>("SELECT data FROM settings WHERE key=?").get(key);
		return row ? decode<T>(row) : structuredClone(fallback);
	}
	putSetting<T>(key: string, value: T): T {
		this.db
			.query(
				"INSERT INTO settings (key,data) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET data=excluded.data",
			)
			.run(key, JSON.stringify(value));

		return value;
	}
	activity(
		kind: string,
		message: string,
		projectId: string | null,
		taskId: string | null,
		resourceId: string | null,
	): Activity {
		const value: Activity = {
			id: crypto.randomUUID(),
			kind,
			message,
			projectId,
			taskId,
			resourceId,
			createdAt: new Date().toISOString(),
		};

		this.db
			.query("INSERT INTO activity (id,project_id,task_id,data) VALUES (?,?,?,?)")
			.run(value.id, projectId, taskId, JSON.stringify(value));

		return value;
	}
	activities(filter: Filter, limit: number): Activity[] {
		const clauses: string[] = [];
		const params: (string | number)[] = [];

		if (filter.projectId) {
			clauses.push("project_id=?");
			params.push(filter.projectId);
		}

		if (filter.taskId) {
			clauses.push("task_id=?");
			params.push(filter.taskId);
		}

		params.push(limit);

		return this.db
			.query<Row, (string | number)[]>(
				`SELECT * FROM activity ${clauses.length ? `WHERE ${clauses.join(" AND ")}` : ""} ORDER BY rowid DESC LIMIT ?`,
			)
			.all(...params)
			.map((row) => decode<Activity>(row));
	}
}
export function checkRevision(current: { revision: number }, expected: number | undefined): void {
	if (expected !== undefined && expected !== current.revision)
		conflict(
			`This record changed (current revision ${current.revision}); refresh before saving`,
		);
}
/** Preserve additive object fields while replacing arrays, matching named items by ID. */
export function mergeFields(previous: unknown, incoming: unknown): unknown {
	if (Array.isArray(incoming)) {
		const old = Array.isArray(previous) ? previous : [];

		return incoming.map((item) =>
			mergeFields(
				old.find(
					(value) =>
						value &&
						item &&
						typeof value === "object" &&
						typeof item === "object" &&
						value.id !== undefined &&
						value.id === item.id,
				),
				item,
			),
		);
	}

	if (!incoming || typeof incoming !== "object") return incoming;

	const result: Record<string, unknown> =
		previous && typeof previous === "object" && !Array.isArray(previous) ? { ...previous } : {};

	for (const [key, value] of Object.entries(incoming))
		result[key] = mergeFields(result[key], value);

	return result;
}
