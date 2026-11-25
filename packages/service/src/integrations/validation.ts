import { TASK_STATUSES, type Project } from "@karabiner/contracts";
import { invalid } from "../errors.ts";
import { object, text } from "../validation.ts";
import { SYNC_FIELDS, type IntegrationConfig, type IntegrationPushInput } from "./types.ts";

function statusMap(value: unknown, reverse: boolean): void {
	if (value === undefined) return;

	for (const [key, status] of Object.entries(object(value, "Status map"))) {
		text(key, "Status map key", 256, false);
		text(status, "Status map value", 256, false);

		if (!TASK_STATUSES.includes((reverse ? key : status) as (typeof TASK_STATUSES)[number]))
			invalid("Status maps must use known task statuses");
	}
}

export function validateIntegrationConfig(value: unknown): IntegrationConfig {
	const b = object(value, "integration");
	if (b.provider !== "github" && b.provider !== "linear") invalid("Unknown integration provider");

	const allowed = new Set([
		"provider",
		"scope",
		"pushEnabled",
		"pushFields",
		"pullStatusMap",
		"pushStatusMap",
		"refresh",
		...(b.provider === "github" ? ["hostname", "statusFieldId"] : ["apiKeyEnv", "teamId"]),
	]);

	for (const key of Object.keys(b))
		if (!allowed.has(key))
			invalid(
				`Unknown integration field ${key}; credentials must use the configured reference, never a token value`,
			);

	if (b.pushEnabled !== undefined && typeof b.pushEnabled !== "boolean")
		invalid("pushEnabled must be boolean");

	if (
		b.pushFields !== undefined &&
		(!Array.isArray(b.pushFields) || b.pushFields.some((field) => !SYNC_FIELDS.includes(field)))
	)
		invalid(
			"pushFields must be a list of supported sync fields; an empty list disables configured publication",
		);

	if (b.refresh !== undefined) {
		const refresh = object(b.refresh, "integration.refresh");

		if (typeof refresh.enabled !== "boolean")
			invalid("integration.refresh.enabled must be boolean");

		for (const key of Object.keys(refresh))
			if (key !== "enabled" && key !== "intervalMs")
				invalid(`Unknown integration.refresh field ${key}`);

		if (
			refresh.intervalMs !== undefined &&
			(!Number.isSafeInteger(refresh.intervalMs) ||
				Number(refresh.intervalMs) < 30_000 ||
				Number(refresh.intervalMs) > 86_400_000)
		)
			invalid("integration.refresh.intervalMs must be an integer from 30000 to 86400000");
	}

	statusMap(b.pullStatusMap, false);

	if (b.provider === "github") {
		if (b.scope !== "repository" && b.scope !== "project")
			invalid("GitHub scope must be repository or project");

		if (
			b.hostname !== undefined &&
			!/^[a-z\d](?:[a-z\d.-]*[a-z\d])?$/i.test(text(b.hostname, "hostname", 253, false))
		)
			invalid("GitHub hostname must be a hostname, without URL or credentials");

		if (b.statusFieldId !== undefined) text(b.statusFieldId, "statusFieldId", 256, false);
		statusMap(b.pushStatusMap, true);
	} else {
		if (b.scope !== "team" && b.scope !== "project")
			invalid("Linear scope must be team or project");

		if (!/^[A-Za-z_][A-Za-z\d_]*$/.test(text(b.apiKeyEnv, "apiKeyEnv", 256, false)))
			invalid("apiKeyEnv must name an environment variable");

		if (b.teamId !== undefined) text(b.teamId, "teamId", 256, false);

		if (b.pushStatusMap !== undefined)
			for (const map of Object.values(object(b.pushStatusMap))) statusMap(map, true);
	}

	return b as unknown as IntegrationConfig;
}

export function integrationConfig(project: Project, value?: unknown): IntegrationConfig {
	if (project.kind === "standalone") invalid("Standalone projects have no external integration");

	const config =
		value === undefined
			? project.kind === "github"
				? { provider: "github" as const, scope: "repository" as const }
				: {
						provider: "linear" as const,
						scope: "team" as const,
						apiKeyEnv: "LINEAR_API_KEY",
					}
			: validateIntegrationConfig(value);

	if (config.provider !== project.kind)
		invalid("Integration provider must match the project kind");

	return config;
}

export function pushInput(value: unknown): IntegrationPushInput {
	const b = object(value);

	if (!Number.isSafeInteger(b.revision) || (b.revision as number) < 1)
		invalid("revision is required for push");

	text(b.requestId, "requestId", 256, false);

	if (b.createMissing !== undefined && typeof b.createMissing !== "boolean")
		invalid("createMissing must be boolean");

	if (
		b.fields !== undefined &&
		(!Array.isArray(b.fields) ||
			b.fields.length === 0 ||
			b.fields.some((f) => !SYNC_FIELDS.includes(f)))
	)
		invalid("fields must be a nonempty list of supported sync fields");

	return b as unknown as IntegrationPushInput;
}
