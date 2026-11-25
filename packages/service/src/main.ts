#!/usr/bin/env bun
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { metadata } from "./capabilities.ts";
import { startService } from "./server.ts";

try {
	const { values } = parseArgs({
		args: process.argv.slice(2),
		options: {
			port: { type: "string" },
			db: { type: "string" },
			help: { type: "boolean", short: "h" },
			version: { type: "boolean" },
		},
	});

	if (values.help)
		console.log(
			"karabiner-service [--port 43173] [--db PATH]\nEnvironment: KARABINER_PORT, KARABINER_DB. Listens on 127.0.0.1. Stop with Ctrl-C.",
		);
	else if (values.version) console.log(metadata.serviceVersion);
	else {
		const port = Number(values.port ?? process.env.KARABINER_PORT ?? 43173);

		if (!Number.isSafeInteger(port) || port < 0 || port > 65535)
			throw new Error("port must be an integer from 0 to 65535");

		const selected =
			values.db ??
			process.env.KARABINER_DB ??
			join(homedir(), "Library", "Application Support", "Karabiner", "karabiner.sqlite");

		const service = startService({
			dbPath: selected === ":memory:" ? selected : resolve(selected),
			port,
		});

		console.log(
			JSON.stringify({
				event: "ready",
				url: service.url,
				apiVersion: metadata.apiVersion,
				serviceVersion: metadata.serviceVersion,
				pid: process.pid,
			}),
		);

		const stop = () => {
			void service.stop().catch((error) => {
				console.error(error instanceof Error ? error.message : String(error));
				process.exitCode = 1;
			});
		};

		process.once("SIGINT", stop);
		process.once("SIGTERM", stop);
	}
} catch (error) {
	console.error(error instanceof Error ? error.message : String(error));
	process.exitCode = 1;
}
