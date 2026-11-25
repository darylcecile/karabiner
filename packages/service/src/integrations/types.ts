import type {
	ExternalTaskReference,
	IntegrationConfig,
	IntegrationError,
	IntegrationInfo,
	IntegrationPushResult,
	Priority,
	TaskStatus,
} from "@karabiner/contracts";
export type {
	ExternalTaskReference,
	IntegrationCatalog,
	IntegrationConfig,
	IntegrationPushInput,
	IntegrationPushResult,
	IntegrationSyncResult,
} from "@karabiner/contracts";

export type Provider = "github" | "linear";
export const SYNC_FIELDS = [
	"title",
	"description",
	"status",
	"priority",
	"labels",
	"dueAt",
] as const;
export type SyncField = (typeof SYNC_FIELDS)[number];
export type GithubIntegrationConfig = Extract<IntegrationConfig, { provider: "github" }>;
export type LinearIntegrationConfig = Extract<IntegrationConfig, { provider: "linear" }>;
export interface TaskProjection {
	title: string;
	description: string;
	status?: TaskStatus;
	priority?: Priority;
	labels?: string[];
	dueAt?: string | null;
}
export interface RemoteTask {
	external: ExternalTaskReference;
	values: TaskProjection;
	repository?: string;
	teamId?: string;
	stateId?: string;
	state?: string;
	archived?: boolean;
	projectId?: string;
	labelIds?: Record<string, string>;
	operationId?: string;
}
export type SyncIssue = IntegrationError;
export type ProviderStatus = IntegrationInfo & {
	status: "unknown" | "ready" | "unavailable" | "error";
	code?: string;
};
export interface RemoteCollection {
	tasks: RemoteTask[];
	errors: SyncIssue[];
}
export interface ExternalLink {
	projectId: string;
	provider: Provider;
	externalId: string;
	taskId: string | null;
	source: string;
	baseline: TaskProjection;
	remote: RemoteTask;
}
export interface PushOperation {
	requestId: string;
	taskId: string;
	fingerprint: string;
	state: "pending" | "finished";
	remoteId?: string;
	values?: TaskProjection;
	fields?: SyncField[];
	result?: IntegrationPushResult;
}
