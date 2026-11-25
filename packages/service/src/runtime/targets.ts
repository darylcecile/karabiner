import { realpath, readFile, stat } from "node:fs/promises";
import { dirname, isAbsolute, posix, relative, resolve } from "node:path";
import { cpus, release } from "node:os";
import type {
	ExecutionTargetInfo,
	ProjectConfig,
	TargetOptions,
	TargetReadiness,
} from "@karabiner/contracts";
import { runProcess } from "./process.ts";

export interface LaunchSpec {
	command: string;
	args: string[];
	cwd: string;
	env?: Record<string, string | undefined>;
	/** In-memory configuration on inherited fd 3; never written to disk or argv. */
	extraInput?: string;
}
export interface PreparedLaunch extends LaunchSpec {
	/** Always await after process settlement, including failed spawn/cancellation. */
	cleanup?: () => Promise<void>;
}
export interface PrepareLaunchInput {
	target: ProjectConfig["executionTarget"];
	projectRoot: string;
	cwd?: string;
	executable: string;
	args: readonly string[];
	env?: Record<string, string | undefined>;
	targetOptions?: TargetOptions;
	purpose?: "command" | "harness";
	timeoutMs?: number;
	signal?: AbortSignal;
}

export interface TargetReadinessInput {
	target: ProjectConfig["executionTarget"];
	projectRoot?: string;
	targetOptions?: TargetOptions;
	purpose?: "command" | "harness";
	executable?: string;
	check?: boolean;
	signal?: AbortSignal;
}

type Options = TargetOptions;

function text(value: string, label: string): string {
	if (typeof value !== "string" || !value || value.includes("\0"))
		throw new Error(`${label} must be a nonempty string without NUL`);

	return value;
}

function executable(
	value: string,
	env: NodeJS.ProcessEnv = process.env,
	cwd = process.cwd(),
): string {
	text(value, "Executable");
	const found = Bun.which(value, { PATH: env.PATH, cwd });

	if (!found)
		throw new Error(
			`Execution prerequisite missing: ${value} is not executable on the service PATH`,
		);

	return found;
}

function requirePlatform(target: "mxc" | "apple-container"): void {
	const darwin = Number(release().split(".")[0]);

	if (process.platform !== "darwin" || darwin < (target === "mxc" ? 24 : 25))
		throw new Error(
			target === "mxc"
				? "MXC requires macOS 15 or later"
				: "Apple Container requires macOS 26 or later",
		);

	if (target === "apple-container" && process.arch !== "arm64")
		throw new Error("Apple Container requires Apple silicon");
}

function quote(value: string): string {
	return `'${value.replaceAll("'", "'\\''")}'`;
}

function object(value: unknown, label: string): Record<string, unknown> {
	if (!value || typeof value !== "object" || Array.isArray(value))
		throw new Error(`${label} must be an object`);

	return value as Record<string, unknown>;
}

function paths(value: unknown, label: string): string[] {
	if (value === undefined) return [];

	if (
		!Array.isArray(value) ||
		value.some((path) => typeof path !== "string" || !isAbsolute(path) || path.includes("\0"))
	)
		throw new Error(`${label} must contain absolute paths`);

	return value;
}

export async function prepareLaunch(input: PrepareLaunchInput): Promise<PreparedLaunch> {
	input.signal?.throwIfAborted();
	text(input.projectRoot, "Project root");
	if (!isAbsolute(input.projectRoot)) throw new Error("Project root must be absolute");
	const root = await realpath(input.projectRoot);
	if (!(await stat(root)).isDirectory()) throw new Error("Project root must be a directory");
	text(input.executable, "Executable");

	for (const arg of input.args)
		if (typeof arg !== "string" || arg.includes("\0"))
			throw new Error("Command arguments must be strings without NUL");

	if (input.target === "host") {
		const env = { ...process.env, ...input.env };
		const cwd = await hostCwd(input, root);
		return { command: executable(input.executable, env, cwd), args: [...input.args], cwd, env };
	}

	if (input.target === "mxc") return mxcLaunch(input, root);
	if (input.target === "apple-container") return appleLaunch(input, root);
	throw new Error("Unknown execution target");
}

async function hostCwd(input: PrepareLaunchInput, root: string): Promise<string> {
	const cwd = await realpath(resolve(root, input.cwd ?? root));

	if (!(await stat(cwd)).isDirectory())
		throw new Error("Execution working directory must be a directory");

	return cwd;
}

async function mxcLaunch(input: PrepareLaunchInput, root: string): Promise<PreparedLaunch> {
	requirePlatform("mxc");
	const options: Options = input.targetOptions ?? {};
	const command = executable(options.executable ?? "mxc-exec-mac", process.env, root);
	const cwd = await hostCwd(input, root);

	const workload = await realpath(
		executable(input.executable, { ...process.env, ...input.env }, cwd),
	);

	const profile = await mxcProfile(options);

	const filesystem = object(profile.filesystem ?? {}, "MXC filesystem");
	const processOptions = object(profile.process ?? {}, "MXC process");
	const seatbelt = object(profile.seatbelt ?? {}, "MXC seatbelt");

	const env = {
		PATH: process.env.PATH ?? "/usr/bin:/bin:/usr/sbin:/sbin",
		HOME: process.env.HOME,
		TMPDIR: process.env.TMPDIR,
		TERM: "dumb",
		...profileEnv(processOptions.env),
		...referencedEnvironment(options.envNames, { ...process.env, ...input.env }),
		...input.env,
	};

	const writable = [
		...new Set([
			root,
			...paths(filesystem.readwritePaths, "MXC readwritePaths"),
			...paths(options.writablePaths, "writablePaths"),
		]),
	];

	const toolDirectory = dirname(workload);
	const toolRead = writable.some((path) => contains(path, toolDirectory)) ? [] : [toolDirectory];
	const timeout = input.timeoutMs ?? 600_000;

	if (!Number.isSafeInteger(timeout) || timeout <= 0 || timeout > 2_147_483_647)
		throw new Error("MXC timeout must be a positive 32-bit millisecond integer");

	const network =
		options.network === undefined
			? (profile.network ?? {
					egress: { default: "deny" },
					ingress: { default: "deny", hostLoopback: "deny" },
				})
			: {
					egress: { default: options.network ? "allow" : "deny" },
					ingress: { default: "deny", hostLoopback: "deny" },
				};

	const config = {
		...profile,
		version: "1.0.0",
		containment: "seatbelt",
		network,
		process: {
			...processOptions,
			commandLine: `exec ${[workload, ...input.args].map(quote).join(" ")}`,
			cwd,
			timeout,
			inheritDefaultEnv: false,
			env: Object.entries(env)
				.filter((entry): entry is [string, string] => entry[1] !== undefined)
				.map(([key, value]) => `${key}=${value}`),
		},
		filesystem: {
			...filesystem,
			readwritePaths: writable,
			readonlyPaths: [
				...new Set([
					...toolRead,
					...paths(filesystem.readonlyPaths, "MXC readonlyPaths"),
					...paths(options.readonlyPaths, "readonlyPaths"),
				]),
			],
		},
		seatbelt: {
			...seatbelt,
			...(options.keychainAccess === undefined
				? {}
				: { keychainAccess: options.keychainAccess }),
		},
	};

	input.signal?.throwIfAborted();

	return {
		command,
		args: ["--config", "/dev/fd/3"],
		cwd,
		env: process.env,
		extraInput: JSON.stringify(config),
	};
}

async function mxcProfile(options: Options): Promise<Record<string, unknown>> {
	let profile: Record<string, unknown> = {};

	if (options.profilePath) {
		text(options.profilePath, "MXC profilePath");
		if (!isAbsolute(options.profilePath)) throw new Error("MXC profilePath must be absolute");
		const info = await stat(options.profilePath);

		if (!info.isFile() || info.size > 1024 * 1024)
			throw new Error("MXC profile must be a JSON file of at most 1 MiB");

		try {
			profile = object(
				JSON.parse(await readFile(options.profilePath, "utf8")),
				"MXC profile",
			);
		} catch {
			throw new Error("MXC profile is not valid JSON");
		}
	}

	if (profile.version !== undefined && profile.version !== "1.0.0")
		throw new Error("MXC profile must use stable schema version 1.0.0");

	if (profile.containment !== undefined && profile.containment !== "seatbelt")
		throw new Error("MXC profile containment must be seatbelt on this target");

	const filesystem = object(profile.filesystem ?? {}, "MXC filesystem");
	paths(filesystem.readwritePaths, "MXC readwritePaths");
	paths(filesystem.readonlyPaths, "MXC readonlyPaths");
	paths(filesystem.deniedPaths, "MXC deniedPaths");
	profileEnv(object(profile.process ?? {}, "MXC process").env);
	object(profile.seatbelt ?? {}, "MXC seatbelt");
	return profile;
}

function referencedEnvironment(names: string[] = [], env = process.env): Record<string, string> {
	return Object.fromEntries(
		names.map((name) => {
			if (!/^[A-Za-z_][A-Za-z\d_]*$/.test(name))
				throw new Error("envNames must contain environment variable names");

			const value = env[name];

			if (value === undefined)
				throw new Error(`Environment reference ${name} is not set in the service`);

			return [name, value];
		}),
	);
}

function profileEnv(value: unknown): Record<string, string> {
	if (value === undefined) return {};

	if (!Array.isArray(value))
		throw new Error("MXC process.env must be an array of KEY=VALUE strings");

	return Object.fromEntries(
		value.map((entry) => {
			if (
				typeof entry !== "string" ||
				!/^[A-Za-z_][A-Za-z\d_]*=/.test(entry) ||
				entry.includes("\0")
			)
				throw new Error("MXC process.env contains an invalid entry");

			const separator = entry.indexOf("=");
			return [entry.slice(0, separator), entry.slice(separator + 1)];
		}),
	);
}

function contains(root: string, path: string): boolean {
	const child = relative(root, path);
	return child !== ".." && !child.startsWith("../") && !isAbsolute(child);
}

async function appleLaunch(input: PrepareLaunchInput, root: string): Promise<PreparedLaunch> {
	requirePlatform("apple-container");
	const options: Options = input.targetOptions ?? {};
	const command = executable(options.executable ?? "container", process.env, root);

	if (!options.image)
		throw new Error(
			"Apple Container requires an explicit prepared Linux image in targetOptions.image",
		);

	text(options.image, "Container image");

	if (options.image.startsWith("-"))
		throw new Error("Container image must not start with a dash");

	if (
		input.purpose === "harness" &&
		(!options.guestHarnessConfigured || !options.guestExecutable)
	)
		throw new Error(
			"Apple CLI harness execution requires an image with a Linux harness and guest authentication, guestExecutable and guestHarnessConfigured; host installation/login cannot be used in the guest",
		);

	const guestRoot = options.guestWorkdir ?? "/workspace";
	text(guestRoot, "guestWorkdir");

	if (!isAbsolute(guestRoot) || guestRoot.includes(":"))
		throw new Error("guestWorkdir must be an absolute guest path without a colon");

	if (root.includes(":")) throw new Error("Apple volume source cannot contain a colon");

	const status = await runProcess({
		command,
		args: ["system", "status", "--format", "json"],
		cwd: root,
		signal: input.signal,
		timeoutMs: 10_000,
	});

	if (status.code !== 0)
		throw new Error(
			"Apple Container service is unavailable; start/configure it separately before selecting this target",
		);

	const name = `karabiner-${crypto.randomUUID()}`;

	const guestExecutable =
		input.purpose === "harness" ? options.guestExecutable! : input.executable;

	text(guestExecutable, "Guest executable");
	const mounts = await appleMounts(root, guestRoot, options);

	const args = [
		"run",
		"--name",
		name,
		"--interactive",
		"--workdir",
		guestCwd(input.cwd, root, guestRoot, input.projectRoot),
		"--entrypoint",
		guestExecutable,
	];

	for (const mount of mounts) args.push("--volume", mount);

	if (options.network === false) args.push("--network", "none");

	if (options.cpus !== undefined) {
		cpuCount(options.cpus);
		args.push("--cpus", String(options.cpus));
	}

	if (options.memory !== undefined) {
		memory(options.memory);
		args.push("--memory", options.memory);
	}

	const env = { ...process.env, ...input.env };

	const forwarded = referencedEnvironment(
		[...new Set([...(options.envNames ?? []), ...Object.keys(input.env ?? {})])],
		env,
	);

	for (const key of Object.keys(forwarded)) args.push("--env", key);

	args.push(options.image, ...input.args);
	let cleanup: Promise<void> | undefined;

	return {
		command,
		args,
		cwd: root,
		env,
		cleanup: () => (cleanup ??= removeContainer(command, name, root)),
	};
}

function guestCwd(
	cwd: string | undefined,
	hostRoot: string,
	guestRoot: string,
	originalRoot = hostRoot,
): string {
	if (!cwd) return guestRoot;
	text(cwd, "Guest working directory");
	if (!isAbsolute(cwd)) return posix.resolve(guestRoot, cwd);
	if (contains(hostRoot, cwd)) return posix.join(guestRoot, relative(hostRoot, cwd));

	if (contains(resolve(originalRoot), cwd))
		return posix.join(guestRoot, relative(resolve(originalRoot), cwd));

	return cwd;
}

async function appleMounts(root: string, guestRoot: string, options: Options): Promise<string[]> {
	const mounts = new Map<string, boolean>([[root, false]]);

	for (const [values, readonly] of [
		[options.writablePaths, false],
		[options.readonlyPaths, true],
	] as const) {
		for (const path of paths(values, readonly ? "readonlyPaths" : "writablePaths")) {
			const source = await realpath(path);
			if (source.includes(":")) throw new Error("Apple volume paths cannot contain colons");
			mounts.set(source, readonly || mounts.get(source) === true);
		}
	}

	return [...mounts]
		.map(([source, readonly]) => {
			const destination = contains(root, source)
				? posix.join(guestRoot, relative(root, source))
				: source;

			return { destination, value: `${source}:${destination}${readonly ? ":ro" : ""}` };
		})
		.sort((left, right) => left.destination.length - right.destination.length)
		.map((mount) => mount.value);
}

function memory(value: string): void {
	text(value, "Container memory");
	const match = /^((?:\d+(?:\.\d*)?|\.\d+))[ \t]*(?:([bkmgtp])(?:i?b)?)?$/i.exec(value.trim());

	if (!match || /[\r\n]/.test(value))
		throw new Error("Container memory must be a quantity such as 512M, 1.5G or 2048MiB");

	const powers: Record<string, number> = { "": 0, b: 0, k: 1, m: 2, g: 3, t: 4, p: 5 };

	const mib = Math.trunc(
		(Number(match[1]) * 1024 ** powers[(match[2] ?? "").toLowerCase()]!) / 1_048_576,
	);

	if (!Number.isSafeInteger(mib) || mib < 200 || mib >= 2 ** 43 - 1)
		throw new Error(
			"Container memory must be at least 200 MiB and within the runtime's supported range",
		);
}

function cpuCount(value: number): void {
	const maximum = cpus().length;

	if (!Number.isSafeInteger(value) || value < 1 || value > maximum)
		throw new Error(
			`Container CPUs must be an integer from 1 to ${maximum}, the host's CPU count`,
		);
}

type ReadinessCheck = TargetReadiness["checks"][number];

function setting(checks: ReadinessCheck[], path: string, validate: () => void): void {
	try {
		validate();

		checks.push({
			code: "configuration_valid",
			path,
			status: "passed",
			message: `${path} is valid`,
		});
	} catch (error) {
		checks.push({
			code: "invalid_configuration",
			path,
			status: "failed",
			message: error instanceof Error ? error.message : `${path} is invalid`,
		});
	}
}

async function workspaceChecks(input: TargetReadinessInput): Promise<ReadinessCheck[]> {
	const checks: ReadinessCheck[] = [];

	if (!input.projectRoot)
		checks.push({
			code: "project_root_required",
			path: "projectRoot",
			status: "failed",
			message: "Choose a local project workspace directory",
		});
	else {
		try {
			text(input.projectRoot, "Project root");

			if (!isAbsolute(input.projectRoot) || !(await stat(input.projectRoot)).isDirectory())
				throw new Error("Project root must be an existing absolute directory");

			checks.push({
				code: "workspace_available",
				path: "projectRoot",
				status: "passed",
				message: "Local workspace directory is available",
			});
		} catch (error) {
			checks.push({
				code: "workspace_unavailable",
				path: "projectRoot",
				status: "failed",
				message:
					error instanceof Error
						? error.message
						: "Local workspace directory is unavailable",
			});
		}
	}

	return checks;
}

async function configurationChecks(input: TargetReadinessInput): Promise<ReadinessCheck[]> {
	const options = input.targetOptions ?? {};
	const checks = await workspaceChecks(input);

	if (input.purpose === "harness" && input.target !== "apple-container" && !input.executable)
		checks.push({
			code: "harness_executable_required",
			path: "executable",
			status: "failed",
			message: "Select a local harness executable to check this target",
		});

	if (input.target === "host") return checks;

	for (const key of ["network", "keychainAccess", "guestHarnessConfigured"] as const)
		if (options[key] !== undefined)
			setting(checks, `targetOptions.${key}`, () => {
				if (typeof options[key] !== "boolean") throw new Error(`${key} must be boolean`);
			});

	await mountChecks(input.target, options, checks);
	environmentChecks(options, checks);

	if (input.target === "mxc") await mxcConfigurationChecks(options, checks);
	else appleConfigurationChecks(input, options, checks);

	return checks;
}

async function mountChecks(
	target: ProjectConfig["executionTarget"],
	options: Options,
	checks: ReadinessCheck[],
): Promise<void> {
	for (const key of ["readonlyPaths", "writablePaths"] as const) {
		setting(checks, `targetOptions.${key}`, () => {
			paths(options[key], key);
		});

		if (target === "apple-container") {
			for (const path of options[key] ?? []) {
				try {
					await realpath(path);
				} catch {
					checks.push({
						code: "mount_source_missing",
						path: `targetOptions.${key}`,
						status: "failed",
						message: `Mount source is unavailable: ${path}`,
					});
				}
			}
		}
	}
}

function environmentChecks(options: Options, checks: ReadinessCheck[]): void {
	setting(checks, "targetOptions.envNames", () => {
		if (
			options.envNames !== undefined &&
			(!Array.isArray(options.envNames) ||
				options.envNames.some((name) => !/^[A-Za-z_][A-Za-z\d_]*$/.test(name)))
		)
			throw new Error("envNames must contain environment variable names");
	});

	for (const name of Array.isArray(options.envNames) ? options.envNames : [])
		if (process.env[name] === undefined)
			checks.push({
				code: "credential_env_missing",
				path: "targetOptions.envNames",
				status: "failed",
				message: `Environment reference ${name} is not set in the service`,
			});
}

async function mxcConfigurationChecks(options: Options, checks: ReadinessCheck[]): Promise<void> {
	try {
		await mxcProfile(options);

		checks.push({
			code: "profile_valid",
			path: "targetOptions.profilePath",
			status: "passed",
			message: options.profilePath
				? "MXC profile format is valid"
				: "MXC policy is generated from these settings; no profile file is required",
		});
	} catch (error) {
		checks.push({
			code: "profile_invalid",
			path: "targetOptions.profilePath",
			status: "failed",
			message: error instanceof Error ? error.message : "MXC profile is invalid",
		});
	}
}

function appleConfigurationChecks(
	input: TargetReadinessInput,
	options: Options,
	checks: ReadinessCheck[],
): void {
	setting(checks, "targetOptions.image", () => {
		text(options.image ?? "", "Container image");

		if (options.image!.startsWith("-"))
			throw new Error("Container image must not start with a dash");
	});

	setting(checks, "targetOptions.guestWorkdir", () => {
		const path = options.guestWorkdir ?? "/workspace";
		text(path, "Guest working directory");

		if (!isAbsolute(path) || path.includes(":"))
			throw new Error("Guest working directory must be an absolute path without colons");
	});

	if (options.cpus !== undefined)
		setting(checks, "targetOptions.cpus", () => {
			cpuCount(options.cpus!);
		});

	if (options.memory !== undefined)
		setting(checks, "targetOptions.memory", () => {
			memory(options.memory!);
		});

	if (input.purpose === "harness")
		setting(checks, "targetOptions.guestExecutable", () => {
			text(options.guestExecutable ?? "", "Guest harness executable");

			if (!options.guestHarnessConfigured)
				throw new Error(
					"Confirm that the selected image contains the Linux harness and its guest authentication setup; the Mac login is not available there",
				);
		});
}

/** Validates draft GUI settings and optionally probes existing runtimes without starting workloads. */
export async function targetReadiness(input: TargetReadinessInput): Promise<TargetReadiness> {
	input.signal?.throwIfAborted();
	const options = input.targetOptions ?? {};
	const base = await targetStatus(input.target, options);
	const checks = await configurationChecks(input);
	const configured = !checks.some((check) => check.status === "failed");
	let installed = input.target === "host";
	let runtime: string | undefined;

	try {
		if (input.target !== "host") {
			requirePlatform(input.target);

			runtime = executable(
				options.executable ?? (input.target === "mxc" ? "mxc-exec-mac" : "container"),
				process.env,
				input.projectRoot,
			);

			installed = true;
		}

		checks.push({
			code: "runtime_available",
			path: input.target === "host" ? "executable" : "targetOptions.executable",
			status: "passed",
			message:
				input.target === "host"
					? "Host process execution is available"
					: "Target runtime executable is available",
		});
	} catch (error) {
		installed = false;

		checks.push({
			code: "runtime_unavailable",
			path: "targetOptions.executable",
			status: "failed",
			message: error instanceof Error ? error.message : "Execution runtime is unavailable",
		});
	}

	if (input.target !== "apple-container" && input.executable) {
		try {
			executable(input.executable, process.env, input.projectRoot);

			checks.push({
				code: "executable_available",
				path: "executable",
				status: "passed",
				message: "Selected local executable is available",
			});
		} catch (error) {
			checks.push({
				code: "executable_missing",
				path: "executable",
				status: "failed",
				message:
					error instanceof Error ? error.message : "Selected executable is unavailable",
			});
		}
	}

	if (input.purpose === "harness")
		checks.push({
			code: "authentication_unverified",
			path:
				input.target === "apple-container"
					? "targetOptions.guestHarnessConfigured"
					: "executable",
			status: "unknown",
			message:
				input.target === "apple-container"
					? "Guest authentication is verified during execution; no Mac credentials are assumed"
					: "Uses the existing local account; authentication has not been inspected",
		});

	let available = configured && installed && !checks.some((check) => check.status === "failed");

	if (available && input.target === "apple-container")
		available = await appleReadiness(input, runtime!, checks);

	if (available && input.target === "mxc") available = await mxcReadiness(input, checks);

	const message =
		checks.find((check) => check.status === "failed")?.message ??
		checks.find((check) => check.status === "unknown")?.message ??
		"Execution target prerequisites are ready";

	return {
		...base,
		installed,
		configured,
		available,
		checks,
		message,
		checkedAt: new Date().toISOString(),
	};
}

async function appleReadiness(
	input: TargetReadinessInput,
	command: string,
	checks: ReadinessCheck[],
): Promise<boolean> {
	if (!input.check) {
		checks.push({
			code: "runtime_not_checked",
			path: "targetOptions.image",
			status: "unknown",
			message: "Check the existing Container service and local image to confirm readiness",
		});

		return false;
	}

	const cwd = input.projectRoot!;

	try {
		const service = await runProcess({
			command,
			args: ["system", "status", "--format", "json"],
			cwd,
			signal: input.signal,
			timeoutMs: 10_000,
		});

		if (service.code !== 0) {
			checks.push({
				code: "container_service_unavailable",
				path: "targetOptions.executable",
				status: "failed",
				message:
					"The Container service is not running; start it separately before execution",
			});

			return false;
		}

		checks.push({
			code: "container_service_ready",
			path: "targetOptions.executable",
			status: "passed",
			message: "The existing Container service is running",
		});

		const image = await runProcess({
			command,
			args: ["image", "inspect", input.targetOptions!.image!],
			cwd,
			signal: input.signal,
			timeoutMs: 10_000,
		});

		checks.push({
			code: image.code === 0 ? "image_available" : "image_not_available",
			path: "targetOptions.image",
			status: image.code === 0 ? "passed" : "failed",
			message:
				image.code === 0
					? "The selected Linux image is available locally"
					: "The selected image is not available locally; prepare it before using this target",
		});

		return image.code === 0;
	} catch {
		checks.push({
			code: "runtime_probe_failed",
			path: "targetOptions.executable",
			status: "failed",
			message: "Container readiness check failed or timed out",
		});

		return false;
	}
}

async function mxcReadiness(
	input: TargetReadinessInput,
	checks: ReadinessCheck[],
): Promise<boolean> {
	if (!input.check) {
		checks.push({
			code: "policy_not_checked",
			path: "targetOptions",
			status: "unknown",
			message: "MXC policy has not been checked by the runtime",
		});

		return true;
	}

	try {
		const launch = await prepareLaunch({
			...input,
			projectRoot: input.projectRoot!,
			executable: input.executable ?? "/usr/bin/true",
			args: [],
			timeoutMs: 10_000,
		});

		const result = await runProcess({
			...launch,
			args: ["--dry-run", ...launch.args],
			signal: input.signal,
			timeoutMs: 10_000,
		});

		checks.push({
			code: result.code === 0 ? "policy_valid" : "policy_rejected",
			path: "targetOptions",
			status: result.code === 0 ? "passed" : "failed",
			message:
				result.code === 0
					? "MXC validated the policy without executing a workload"
					: "MXC rejected the configured policy; check its filesystem and network grants",
		});

		return result.code === 0;
	} catch (error) {
		checks.push({
			code: "policy_probe_failed",
			path: "targetOptions",
			status: "failed",
			message: error instanceof Error ? error.message : "MXC policy check failed",
		});

		return false;
	}
}

async function removeContainer(command: string, name: string, cwd: string): Promise<void> {
	// The CLI process is not the guest process. Only touch the uniquely owned ID.
	let stopError: unknown;

	try {
		await runProcess({ command, args: ["stop", "--time", "5", name], cwd, timeoutMs: 15_000 });
	} catch (error) {
		stopError = error;
	}

	const removed = await runProcess({
		command,
		args: ["delete", "--force", name],
		cwd,
		timeoutMs: 15_000,
	});

	if (removed.code !== 0 && !/not found|does not exist|no such container/i.test(removed.stderr))
		throw new Error(
			`Failed to clean up owned container ${name}${stopError ? " after stop failed" : ""}`,
		);
}

/** Local prerequisites only unless check=true requests a non-mutating service probe. */
export async function targetStatus(
	target: ProjectConfig["executionTarget"],
	targetOptions: TargetOptions = {},
	check = false,
): Promise<ExecutionTargetInfo> {
	const options: Options = targetOptions;

	const base: ExecutionTargetInfo = {
		id: target,
		name:
			target === "host" ? "This Mac" : target === "mxc" ? "MXC Seatbelt" : "Apple Container",
		kind: target === "host" ? "host" : target === "mxc" ? "process_sandbox" : "virtual_machine",
		available: false,
		requirements:
			target === "host"
				? "An installed harness or configured Strands provider"
				: target === "mxc"
					? "macOS 15+; mxc-exec-mac; explicit filesystem/network policy"
					: "Apple silicon; macOS 26+; running container service; prepared Linux image",
		message: "",
		checkedAt: new Date().toISOString(),
	};

	try {
		if (target === "host")
			return {
				...base,
				available: true,
				message:
					"Host command execution is available; harness authentication is checked separately",
			};

		requirePlatform(target);

		const command = executable(
			options.executable ?? (target === "mxc" ? "mxc-exec-mac" : "container"),
		);

		if (target === "mxc")
			return {
				...base,
				available: true,
				message:
					"MXC executable found; configured policy and host-local authentication are validated at launch",
			};

		if (!options.image)
			return {
				...base,
				message:
					"Configure a prepared Linux image; host harness binaries and login are not available in its guest",
			};

		if (!check)
			return {
				...base,
				message:
					"Container CLI found; service readiness and guest harness/authentication have not been checked",
			};

		const result = await runProcess({
			command,
			args: ["system", "status", "--format", "json"],
			cwd: resolve("."),
			timeoutMs: 10_000,
		});

		return {
			...base,
			available: result.code === 0,
			message:
				result.code === 0
					? "Container service is running; guest harness/authentication remains image-specific"
					: "Container service is unavailable",
		};
	} catch (error) {
		return {
			...base,
			message: error instanceof Error ? error.message : "Execution prerequisite check failed",
		};
	}
}
