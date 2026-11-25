import { CopilotClient, RuntimeConnection } from "@microsoft/copilot-sdk";
import { realpath, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, resolve } from "node:path";
import type { AgentDefinition, HarnessReadiness } from "@karabiner/contracts";
import type { AdapterCapabilities } from "./adapter-types.ts";
import { object, waitWithSignal } from "./bridge-common.ts";
import { runProcess } from "./process.ts";
import { RpcProcess } from "./rpc-process.ts";
import { strandsReadiness } from "./strands-model.ts";

function supported(agent: AgentDefinition): AdapterCapabilities {
	if (agent.harness === "custom")
		return {
			permissions: agent.protocol === "jsonrpc-v2",
			questions: agent.protocol === "jsonrpc-v2",
			steering: agent.protocol === "jsonrpc-v2" ? "native-tool-boundary" : "next-step",
		};

	return {
		permissions: true,
		questions: true,
		steering: agent.harness === "strands" ? "native-tool-boundary" : "native-turn",
	};
}

async function copilotAuth(executable: string, cwd: string, signal: AbortSignal): Promise<boolean> {
	const client = new CopilotClient({
		runtime: RuntimeConnection.forSpawnedStdio({ path: executable, workingDirectory: cwd }),
		mode: "coding-agent",
		github: { useLoggedInUser: true },
		observability: { logLevel: "none" },
	});

	try {
		await waitWithSignal(client.start(), signal);
		const status = await waitWithSignal(client.getAuthStatus(), signal);
		return status.isAuthenticated;
	} finally {
		try {
			const errors = await waitWithSignal(client.stop(), AbortSignal.timeout(10_000));
			if (errors.length) await waitWithSignal(client.forceStop(), AbortSignal.timeout(5_000));
		} catch {
			await waitWithSignal(client.forceStop(), AbortSignal.timeout(5_000));
		}
	}
}

async function codexAuth(
	executable: string,
	cwd: string,
	signal: AbortSignal,
): Promise<boolean | null> {
	const rpc = new RpcProcess(
		{ command: executable, args: ["app-server", "--listen", "stdio://"], cwd },
		signal,
	);

	try {
		await rpc.call("initialize", {
			clientInfo: { name: "karabiner_readiness", version: "0.1.0" },
		});

		rpc.notify("initialized");
		const result = await rpc.call("account/read", { refreshToken: false });
		return result.account ? true : result.requiresOpenaiAuth === true ? false : null;
	} finally {
		await rpc.close();
	}
}

async function claudeAuth(executable: string, cwd: string, signal: AbortSignal): Promise<boolean> {
	const result = await runProcess({
		command: executable,
		args: ["auth", "status"],
		cwd,
		signal,
		timeoutMs: 15_000,
		maxOutputBytes: 65_536,
	});

	const status = object(JSON.parse(result.stdout));
	if (typeof status.loggedIn === "boolean") return status.loggedIn;
	if (result.code === 0) return true;
	if (result.code === 1 && status.authMethod === "none") return false;
	throw new Error("Claude authentication status was not recognized");
}

/** Known-provider status operations only: no inference, sessions, sign-in or custom executable probing. */
export async function harnessReadiness(
	agent: AgentDefinition,
	options: { check?: boolean; root?: string } = {},
): Promise<HarnessReadiness> {
	const result: HarnessReadiness = {
		id: agent.harness,
		name: agent.name,
		executable:
			agent.executable ||
			(agent.harness === "strands" || agent.harness === "custom" ? "" : agent.harness),
		installed: null,
		authenticated: null,
		status: "not_checked",
		executionAvailable: false,
		message: "Authentication has not been checked",
		code: "auth_unchecked",
		checkedAt: new Date().toISOString(),
		supported: supported(agent),
		checks: [],
	};

	if (agent.harness === "strands") {
		const readiness = strandsReadiness(agent);

		return {
			...result,
			installed: true,
			executionAvailable: readiness.available,
			code: readiness.code,
			credentialEnv: readiness.credentialEnv,
			status: readiness.available ? "available" : "unavailable",
			message: readiness.message,
			checks: [
				{
					code: readiness.code,
					message: readiness.message,
					status: readiness.available ? "passed" : "failed",
				},
				{
					code: "provider_auth",
					status: "unknown",
					message: "Provider authentication and model access require an invocation",
				},
			],
		};
	}

	const configuredPath =
		options.root && !isAbsolute(result.executable) && /[/\\]/.test(result.executable)
			? resolve(options.root, result.executable)
			: result.executable;

	const executable = configuredPath ? Bun.which(configuredPath) : null;
	result.installed = Boolean(executable);

	result.checks.push({
		code: "executable",
		status: executable ? "passed" : "failed",
		message: executable
			? "Configured executable is available"
			: `Executable ${result.executable || "path"} is missing from the service environment`,
	});

	if (!executable)
		return {
			...result,
			code: "executable_missing",
			status: "unavailable",
			message: result.checks[0]!.message,
		};

	result.executionAvailable = true;

	if (agent.harness === "custom")
		return {
			...result,
			code: "custom_auth_unknown",
			message: `Custom ${agent.protocol ?? "jsonl-v1"} executable found; authentication and protocol capabilities have not been negotiated`,
			checks: [
				...result.checks,
				{
					code: "custom_auth",
					status: "unknown",
					message: "Custom executables are not run by readiness checks",
				},
			],
		};

	if (!options.check)
		return {
			...result,
			checks: [
				...result.checks,
				{ code: "authentication", status: "unknown", message: result.message },
			],
		};

	let cwd: string;

	try {
		cwd = await realpath(options.root || homedir());
		if (!(await stat(cwd)).isDirectory()) throw new Error("Not a directory");
	} catch {
		return {
			...result,
			code: "root_unavailable",
			status: "unavailable",
			executionAvailable: false,
			message: "The configured authentication-check folder is unavailable",
		};
	}

	try {
		const signal = AbortSignal.timeout(15_000);

		const authenticated =
			agent.harness === "copilot"
				? await copilotAuth(executable, cwd, signal)
				: agent.harness === "claude"
					? await claudeAuth(executable, cwd, signal)
					: await codexAuth(executable, cwd, signal);

		const message =
			authenticated === true
				? "Installed harness reports an authenticated account; model access remains unverified"
				: authenticated === false
					? "Sign in using the installed harness, then check again"
					: "This configured provider does not report a local account; authentication remains unverified";

		return {
			...result,
			authenticated,
			executionAvailable: authenticated !== false,
			status:
				authenticated === true
					? "available"
					: authenticated === false
						? "unavailable"
						: "not_checked",
			code:
				authenticated === true
					? "authenticated"
					: authenticated === false
						? "authentication_required"
						: "auth_unknown",
			message,
			checks: [
				...result.checks,
				{
					code: "authentication",
					status:
						authenticated === true
							? "passed"
							: authenticated === false
								? "failed"
								: "unknown",
					message,
				},
			],
		};
	} catch (error) {
		// Provider diagnostics can include account/credential details. Return only the status classification.
		const mismatch =
			agent.harness === "copilot" && error instanceof Error
				? /^SDK protocol version mismatch: SDK supports versions (\d+)-(\d+), but server reports version (\d+)\./.exec(
						error.message,
					)
				: null;

		if (mismatch) {
			const message = `Installed Copilot reports protocol ${mismatch[3]}; this application SDK supports ${mismatch[1]}-${mismatch[2]}. Update the application SDK or choose a compatible installed CLI.`;

			return {
				...result,
				code: "protocol_mismatch",
				status: "error",
				authenticated: null,
				executionAvailable: false,
				message,
				checks: [
					...result.checks,
					{ code: "protocol_compatibility", status: "failed", message },
				],
			};
		}

		return {
			...result,
			code: "auth_check_failed",
			status: "error",
			executionAvailable: false,
			message:
				"Authentication check failed or timed out; verify the installed harness supports its current status API",
			checks: [
				...result.checks,
				{
					code: "authentication",
					status: "unknown",
					message: "No reliable authentication result was returned",
				},
			],
		};
	}
}
