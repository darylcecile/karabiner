import { spawn } from "node:child_process";
import type { Writable } from "node:stream";
import { AdapterCleanupError, cleanupFailure } from "./adapter-types.ts";

export interface ProcessRequest {
	command: string;
	args?: string[];
	cwd: string;
	env?: NodeJS.ProcessEnv;
	input?: string;
	extraInput?: string;
	signal?: AbortSignal;
	timeoutMs?: number;
	maxOutputBytes?: number;
	onLine?: (line: string, stream: "stdout" | "stderr") => void;
}

export interface ProcessResult {
	code: number;
	stdout: string;
	stderr: string;
}

/** Owns only its child process tree; cancellation settles after the child closes. */
export async function runProcess(request: ProcessRequest): Promise<ProcessResult> {
	request.signal?.throwIfAborted();

	const child = spawn(request.command, request.args ?? [], {
		cwd: request.cwd,
		env: request.env ?? process.env,
		stdio:
			request.extraInput === undefined
				? ["pipe", "pipe", "pipe"]
				: ["pipe", "pipe", "pipe", "pipe"],
		detached: process.platform !== "win32",
	});

	let failure: Error | undefined;
	let killTimer: ReturnType<typeof setTimeout> | undefined;

	const signalTree = (signal: NodeJS.Signals) => {
		try {
			if (child.pid && process.platform !== "win32") process.kill(-child.pid, signal);
			else child.kill(signal);
		} catch (error) {
			if ((error as NodeJS.ErrnoException).code !== "ESRCH") failure ??= asError(error);
		}
	};

	const stop = (error: Error) => {
		if (failure) return;
		failure = error;
		signalTree("SIGTERM");
		killTimer = setTimeout(() => signalTree("SIGKILL"), 1_000);
	};

	const abort = () => stop(asError(request.signal?.reason ?? new Error("Execution cancelled")));

	const timer = setTimeout(
		() => stop(new Error("Execution timed out")),
		request.timeoutMs ?? 600_000,
	);

	request.signal?.addEventListener("abort", abort, { once: true });
	if (request.signal?.aborted) abort();

	let bytes = 0;
	const output = { stdout: "", stderr: "" };
	const partial = { stdout: "", stderr: "" };

	const onData = (stream: "stdout" | "stderr", data: string) => {
		bytes += Buffer.byteLength(data);

		if (bytes > (request.maxOutputBytes ?? 4 * 1024 * 1024)) {
			stop(new Error("Execution output exceeded its configured limit"));
			return;
		}

		output[stream] += data;
		partial[stream] += data;
		const lines = partial[stream].split("\n");
		partial[stream] = lines.pop() ?? "";

		try {
			for (const line of lines) request.onLine?.(line, stream);
		} catch (error) {
			stop(asError(error));
		}
	};

	child.stdout!.setEncoding("utf8").on("data", (data: string) => onData("stdout", data));
	child.stderr!.setEncoding("utf8").on("data", (data: string) => onData("stderr", data));

	const inputError = (error: NodeJS.ErrnoException) => {
		if (error.code !== "EPIPE") stop(error);
	};

	child.stdin!.on("error", inputError);

	if (request.extraInput !== undefined) {
		const extra = child.stdio[3] as Writable;
		extra.on("error", inputError);
		extra.end(request.extraInput);
	}

	const closed = new Promise<number>((resolve) => {
		child.on("error", (error) => {
			failure ??= error;
		});

		child.once("close", (code) => resolve(code ?? 1));
	});

	child.stdin!.end(request.input ?? "");

	try {
		const code = await closed;
		if (failure) throw failure;

		for (const stream of ["stdout", "stderr"] as const) {
			if (partial[stream]) request.onLine?.(partial[stream], stream);
		}

		return { code, ...output };
	} finally {
		clearTimeout(timer);
		if (failure) signalTree("SIGKILL");
		if (killTimer) clearTimeout(killTimer);
		request.signal?.removeEventListener("abort", abort);
	}
}

export function asError(error: unknown): Error {
	return error instanceof Error ? error : new Error(String(error));
}

export async function runPrepared(
	request: ProcessRequest & { cleanup?: () => Promise<void> },
): Promise<ProcessResult> {
	let result: ProcessResult | undefined;
	let failure: unknown;

	try {
		result = await runProcess(request);
	} catch (error) {
		failure = error;
	}

	try {
		await request.cleanup?.();
	} catch (error) {
		failure = new AdapterCleanupError(
			"Execution target",
			[cleanupFailure("target.cleanup", error)],
			undefined,
			failure,
		);
	}

	if (failure) throw failure;
	return result!;
}
