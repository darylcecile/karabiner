import {
	API_VERSION,
	type HarnessInfo,
	type ServiceMeta,
	type ProjectConfig,
} from "@karabiner/contracts";
import manifest from "../package.json";
import { targetStatus } from "./runtime/targets.ts";

/** Capability flags describe implemented operations; availability is observed per adapter. */
export const metadata: ServiceMeta = {
	apiVersion: API_VERSION,
	serviceVersion: manifest.version,
	capabilities: {
		storage: true,
		configuration: true,
		documents: true,
		codeTours: true,
		interventionIntents: true,
		activity: true,
		statistics: true,
		events: true,
		scheduler: true,
		harnessExecution: true,
		integrations: true,
		virtualMachines: true,
		factoryCompilation: true,
		runControls: true,
		parallelExecution: true,
		stepRetries: true,
		interactiveControl: true,
		integrationCatalog: true,
		integrationRefresh: true,
		targetReadiness: true,
		strandsConfiguration: true,
		strandsReadiness: true,
		harnessReadiness: true,
	},
};

export function harnesses(): HarnessInfo[] {
	const checkedAt = new Date().toISOString();

	const commands = [
		{ id: "claude" as const, name: "Claude Code", executable: "claude" },
		{ id: "codex" as const, name: "Codex", executable: "codex" },
		{ id: "copilot" as const, name: "GitHub Copilot", executable: "copilot" },
	];

	return [
		...commands.map((item): HarnessInfo => {
			const installed = Bun.which(item.executable) !== null;

			return {
				...item,
				installed,
				authenticated: null,
				checkedAt,
				executionAvailable: installed,
				status: installed ? "available" : "unavailable",
				message: installed
					? "Executable found on PATH. Uses its existing account; authentication and model access are checked when running."
					: "Executable not found on the service's PATH.",
			};
		}),
		{
			id: "custom",
			name: "Custom local harness",
			executable: "",
			installed: null,
			authenticated: null,
			checkedAt,
			status: "not_checked",
			executionAvailable: true,
			message:
				"Configure an explicit executable and arguments on the agent. Availability is checked on launch.",
		},
		{
			id: "strands",
			name: "Strands",
			executable: "",
			installed: true,
			authenticated: null,
			checkedAt,
			status: "available",
			executionAvailable: true,
			message:
				"Built-in Strands harness; provider credentials and model access are supplied by the service environment and checked on invocation.",
		},
	];
}

export function executionTargets(config?: ProjectConfig, check = false) {
	return Promise.all(
		(["host", "mxc", "apple-container"] as const).map((target) =>
			targetStatus(
				target,
				config?.executionTarget === target ? config.targetOptions : undefined,
				check,
			),
		),
	);
}
