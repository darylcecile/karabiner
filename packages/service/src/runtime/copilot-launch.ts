import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import type { AdapterRequest } from "./adapter-types.ts";

// The SDK has no custom-spawn callback. This fixed source forwards its stdio to a
// prepared target; the per-run launch description stays in process memory only.
const proxySource = `import {spawn} from 'node:child_process';
const spec=JSON.parse(process.env.KARABINER_COPILOT_LAUNCH);
const env={...process.env,...spec.env}; delete env.KARABINER_COPILOT_LAUNCH; delete env.BUN_BE_BUN;
const child=spawn(spec.command,spec.args,{cwd:spec.cwd,env,stdio:spec.extraInput===undefined?['pipe','pipe','pipe']:['pipe','pipe','pipe','pipe'],detached:process.platform!=='win32'});
const kill=signal=>{try{if(process.platform!=='win32')process.kill(-child.pid,signal);else child.kill(signal)}catch(error){if(error.code!=='ESRCH')process.stderr.write(String(error))}};
let timer; const stop=()=>{kill('SIGTERM');timer=setTimeout(()=>kill('SIGKILL'),1000)};
process.on('SIGTERM',stop);process.on('SIGINT',stop);
process.stdin.pipe(child.stdin);child.stdout.pipe(process.stdout);child.stderr.pipe(process.stderr);
child.stdin.on('error',error=>{if(error.code!=='EPIPE')stop()});
if(spec.extraInput!==undefined){child.stdio[3].on('error',error=>{if(error.code!=='EPIPE')stop()});child.stdio[3].end(spec.extraInput)}
child.on('error',error=>{process.stderr.write(error.message);process.exitCode=1});
child.on('close',code=>{clearTimeout(timer);if(timer)kill('SIGKILL');process.exit(code??1)});
`;

export async function copilotLaunch(request: AdapterRequest) {
	const executable = request.agent.executable || "copilot";
	const args = request.agent.args ?? [];

	const launch = await request.launch(executable, [
		...args,
		"--headless",
		"--no-auto-update",
		"--log-level",
		"none",
		"--stdio",
	]);

	if (
		launch.command === Bun.which(executable) &&
		launch.extraInput === undefined &&
		!launch.cleanup
	)
		return {
			path: launch.command,
			args,
			cwd: launch.cwd,
			env: launch.env,
			cleanup: async () => {},
		};

	const directory = await mkdtemp(join(tmpdir(), "karabiner-copilot-transport-"));
	const path = join(directory, "transport");

	try {
		await writeFile(path, `#!${process.execPath}\n${proxySource}`, { mode: 0o700 });
		await chmod(path, 0o700);

		return {
			path,
			args: [],
			cwd: launch.cwd,
			env: {
				...process.env,
				BUN_BE_BUN: "1",
				KARABINER_COPILOT_LAUNCH: JSON.stringify({
					command: launch.command,
					args: launch.args,
					cwd: launch.cwd,
					env: launch.env,
					extraInput: launch.extraInput,
				}),
			},
			cleanup: async () => {
				try {
					await launch.cleanup?.();
				} finally {
					await rm(directory, { recursive: true, force: true });
				}
			},
		};
	} catch (error) {
		await rm(directory, { recursive: true, force: true });
		await launch.cleanup?.();
		throw error;
	}
}
