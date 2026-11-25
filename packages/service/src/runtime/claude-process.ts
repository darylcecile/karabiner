import { spawn, type ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import { PassThrough, type Writable } from "node:stream";
import type { SpawnedProcess, SpawnOptions } from "@anthropic-ai/claude-agent-sdk/core";
import type { AdapterRequest } from "./adapter-types.ts";

/** The SDK's synchronous spawn hook is backed by the asynchronous target preparation seam. */
export class ClaudeProcess extends EventEmitter implements SpawnedProcess {
	readonly stdin = new PassThrough();
	readonly stdout = new PassThrough();
	private child?: ChildProcess;
	private requestedSignal?: NodeJS.Signals;
	private ended = false;
	private settle!: () => void;
	private cleanup?: () => Promise<void>;
	readonly closed = new Promise<void>((resolve) => {
		this.settle = resolve;
	});
	get killed() {
		return this.requestedSignal !== undefined;
	}
	get exitCode() {
		return this.child?.exitCode ?? (this.ended ? 1 : null);
	}
	get signalCode() {
		return this.child?.signalCode ?? null;
	}
	constructor(options: SpawnOptions, request: AdapterRequest) {
		super();
		this.on("error", () => {});

		void this.start(options, request).catch((error) => {
			this.emit("error", error);
			this.ended = true;
			this.stdout.end();
			this.emit("exit", 1, null);
			this.settle();
		});
	}
	private async start(options: SpawnOptions, request: AdapterRequest) {
		const env = Object.fromEntries(
			Object.entries(options.env).filter(
				(entry): entry is [string, string] => entry[1] !== undefined,
			),
		);

		let args = options.args;

		if (
			request.executionTarget === "apple-container" &&
			/\.(?:mjs|cjs|js)$/.test(args[0] ?? "")
		) {
			if (args[0] !== request.agent.executable)
				throw new Error(
					"Claude guest launch cannot translate an unknown host SDK script; configure a native Claude executable",
				);

			args = args.slice(1);
		}

		const launch = await request.launch(
			options.command,
			[...args, ...(request.agent.args ?? [])],
			options.cwd,
			env,
		);

		this.cleanup = launch.cleanup;
		options.signal.throwIfAborted();

		const child = spawn(launch.command, launch.args, {
			cwd: launch.cwd,
			env: launch.env,
			stdio:
				launch.extraInput === undefined
					? ["pipe", "pipe", "pipe"]
					: ["pipe", "pipe", "pipe", "pipe"],
			detached: process.platform !== "win32",
		});

		this.child = child;
		let bytes = 0;
		this.stdin.pipe(child.stdin!);

		child.stdout!.on("data", (data: Buffer) => {
			bytes += data.length;

			if (bytes > 16 * 1024 * 1024) {
				this.emit("error", new Error("Claude protocol output limit exceeded"));
				this.kill("SIGTERM");
			}
		});

		child.stdout!.pipe(this.stdout);

		child
			.stderr!.setEncoding("utf8")
			.on("data", (text: string) =>
				request.emit({ kind: "activity", text: text.slice(-4_000) }),
			);

		child.stdin!.on("error", (error) => {
			if ((error as NodeJS.ErrnoException).code !== "EPIPE") this.emit("error", error);
		});

		if (launch.extraInput !== undefined) {
			const extra = child.stdio[3] as Writable;

			extra.on("error", (error) => {
				if ((error as NodeJS.ErrnoException).code !== "EPIPE") this.emit("error", error);
			});

			extra.end(launch.extraInput);
		}

		const abort = () => this.kill("SIGTERM");
		options.signal.addEventListener("abort", abort, { once: true });
		child.on("error", (error) => this.emit("error", error));

		child.once("close", (code, signal) => {
			options.signal.removeEventListener("abort", abort);
			this.ended = true;
			this.emit("exit", code, signal);
			this.settle();
		});

		if (this.requestedSignal || options.signal.aborted)
			this.kill(this.requestedSignal ?? "SIGTERM");
	}
	kill(signal: NodeJS.Signals): boolean {
		this.requestedSignal = signal;

		try {
			if (this.child?.pid && process.platform !== "win32")
				process.kill(-this.child.pid, signal);
			else this.child?.kill(signal);

			return true;
		} catch (error) {
			if ((error as NodeJS.ErrnoException).code !== "ESRCH") this.emit("error", error);
			return false;
		}
	}
	async close() {
		this.stdin.end();
		const timer = setTimeout(() => this.kill("SIGKILL"), 2_000);

		try {
			await this.closed;
		} finally {
			clearTimeout(timer);
			if (this.killed) this.kill("SIGKILL");
			await this.cleanup?.();
		}
	}
}
