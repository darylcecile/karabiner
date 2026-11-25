import { spawn } from "node:child_process";
import type { Writable } from "node:stream";
import type { Launch } from "./adapter-types.ts";
import { object } from "./bridge-common.ts";

type Frame = Record<string, unknown>;

/** Codex/custom JSONL duplex transport. The caller owns the native protocol. */
export class RpcProcess {
	private pending = new Map<
		number,
		{ resolve(value: Frame): void; reject(error: Error): void }
	>();
	private requestControllers = new Map<string, AbortController>();
	private sequence = 0;
	private tail = "";
	private stderr = "";
	private bytes = 0;
	private error?: Error;
	private killTimer?: ReturnType<typeof setTimeout>;
	private readonly child;
	readonly closed: Promise<void>;
	private readonly abort: () => void;
	onRequest?: (
		method: string,
		params: Frame,
		id: string,
		signal: AbortSignal,
	) => Promise<unknown>;
	onNotification?: (method: string, params: Frame) => void;
	constructor(
		private readonly launch: Awaited<ReturnType<Launch>>,
		private readonly signal: AbortSignal,
	) {
		this.child = spawn(launch.command, launch.args, {
			cwd: launch.cwd,
			env: launch.env ?? process.env,
			stdio:
				launch.extraInput === undefined
					? ["pipe", "pipe", "pipe"]
					: ["pipe", "pipe", "pipe", "pipe"],
			detached: process.platform !== "win32",
		});

		this.abort = () => this.terminate(new Error(String(signal.reason ?? "Harness cancelled")));
		signal.addEventListener("abort", this.abort, { once: true });
		this.child.stdin!.on("error", (error) => this.terminate(error));
		this.child.stdout!.setEncoding("utf8").on("data", (text: string) => this.receive(text));

		this.child.stderr!.setEncoding("utf8").on("data", (text: string) => {
			this.stderr = (this.stderr + text).slice(-8_000);
		});

		if (launch.extraInput !== undefined) {
			const extra = this.child.stdio[3] as Writable;
			extra.on("error", (error) => this.terminate(error));
			extra.end(launch.extraInput);
		}

		this.closed = new Promise((resolve) => {
			this.child.on("error", (error) => this.terminate(error));

			this.child.once("close", (code) => {
				if (this.killTimer) clearTimeout(this.killTimer);

				this.fail(
					this.error ??
						new Error(
							`Harness transport closed (${code ?? "signal"})${this.stderr ? `: ${this.stderr}` : ""}`,
						),
				);

				signal.removeEventListener("abort", this.abort);
				resolve();
			});
		});

		if (signal.aborted) this.abort();
	}
	private fail(error: Error) {
		for (const waiter of this.pending.values()) waiter.reject(error);
		this.pending.clear();
		for (const controller of this.requestControllers.values()) controller.abort(error);
		this.requestControllers.clear();
	}
	private kill(signal: NodeJS.Signals) {
		try {
			if (this.child.pid && process.platform !== "win32")
				process.kill(-this.child.pid, signal);
			else this.child.kill(signal);
		} catch (error) {
			if ((error as NodeJS.ErrnoException).code !== "ESRCH") this.error ??= error as Error;
		}
	}
	private terminate(error: Error) {
		this.error ??= error;
		this.fail(error);
		this.kill("SIGTERM");
		this.killTimer ??= setTimeout(() => this.kill("SIGKILL"), 1_000);
	}
	private send(frame: Frame) {
		this.signal.throwIfAborted();

		if (this.error || this.child.exitCode !== null)
			throw this.error ?? new Error("Harness transport is closed");

		this.child.stdin!.write(`${JSON.stringify(frame)}\n`);
	}
	notify(method: string, params: Frame = {}) {
		this.send({ method, params });
	}
	call(method: string, params: Frame = {}): Promise<Frame> {
		const id = ++this.sequence;

		return new Promise((resolve, reject) => {
			this.pending.set(id, { resolve, reject });

			try {
				this.send({ id, method, params });
			} catch (error) {
				this.pending.delete(id);
				reject(error);
			}
		});
	}
	private receive(text: string) {
		this.bytes += Buffer.byteLength(text);

		if (this.bytes > 16 * 1024 * 1024) {
			this.terminate(new Error("Harness protocol output limit exceeded"));
			return;
		}

		this.tail += text;
		const lines = this.tail.split("\n");
		this.tail = lines.pop() ?? "";

		for (const line of lines) {
			if (!line.trim()) continue;

			try {
				this.frame(object(JSON.parse(line)));
			} catch (error) {
				this.terminate(error as Error);
			}
		}
	}
	private frame(frame: Frame) {
		if (typeof frame.method === "string") {
			const params = object(frame.params);

			if (frame.id === undefined) {
				if (frame.method === "serverRequest/resolved")
					this.requestControllers
						.get(String(params.requestId))
						?.abort(new Error("Native request resolved"));

				this.onNotification?.(frame.method, params);
				return;
			}

			const id = String(frame.id);
			const controller = new AbortController();
			this.requestControllers.set(id, controller);

			const reply =
				this.onRequest?.(frame.method, params, id, controller.signal) ??
				Promise.reject(new Error(`Unsupported harness request ${frame.method}`));

			void reply
				.then(
					(result) => {
						if (!controller.signal.aborted) this.send({ id: frame.id, result });
					},
					(error) => {
						if (!controller.signal.aborted)
							this.send({
								id: frame.id,
								error: { code: -32603, message: String(error) },
							});
					},
				)
				.catch((error) => this.terminate(error))
				.finally(() => this.requestControllers.delete(id));

			return;
		}

		const waiter = this.pending.get(Number(frame.id));
		if (!waiter) return;
		this.pending.delete(Number(frame.id));

		if (frame.error)
			waiter.reject(new Error(String(object(frame.error).message ?? "Harness RPC failed")));
		else waiter.resolve(object(frame.result));
	}
	async close() {
		this.child.stdin!.end();
		const timer = setTimeout(() => this.kill("SIGKILL"), 2_000);

		try {
			await this.closed;
		} finally {
			clearTimeout(timer);
			this.kill("SIGKILL");
			await this.launch.cleanup?.();
		}
	}
}
