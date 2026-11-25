/** Fake MXC/container executables; executes only a local, driver-owned workload. */
import assert from "node:assert/strict";
import { appendFile } from "node:fs/promises";
import { readFileSync } from "node:fs";

const args = process.argv.slice(2);
if (args[0] === "--workload") {
  console.log(JSON.stringify({ cwd: process.cwd(), args: args.slice(1), input: await Bun.stdin.text(), value: process.env.FIXTURE_VALUE, hasPath: !!process.env.PATH }));
  process.exit(0);
}
const log = process.env.KARABINER_FIXTURE_LOG!;
async function record(value: unknown): Promise<void> { await appendFile(log, JSON.stringify(value) + "\n"); }
const configIndex = args.indexOf("--config");
if (configIndex !== -1) {
  const configPath = args[configIndex + 1];
  assert.equal(configPath, "/dev/fd/3");
  // Reopen the supplied path as the native MXC loader does, rather than reading fd 3 directly.
  const config = JSON.parse(readFileSync(configPath!, "utf8")) as { process: { commandLine: string; cwd: string; env: string[] } };
  if (args.includes("--dry-run")) {
    await record({ kind: "mxc", mode: "dry-run", configPath, cwd: config.process.cwd, commandLine: config.process.commandLine });
    console.log(JSON.stringify({ status: "fixture-dry-run", configPath }));
    process.exit(0);
  }
  const input = await Bun.stdin.text();
  await record({ kind: "mxc", mode: "run", configPath, cwd: config.process.cwd, commandLine: config.process.commandLine, input });
  const env = Object.fromEntries(config.process.env.map((entry) => { const at = entry.indexOf("="); return [entry.slice(0, at), entry.slice(at + 1)]; }));
  const child = Bun.spawn(["/bin/sh", "-c", config.process.commandLine], { cwd: config.process.cwd, env, stdin: new TextEncoder().encode(input), stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr, code] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
  process.stdout.write(stdout);
  process.stderr.write(stderr);
  process.exit(code);
}
await record({ kind: "apple", args });
if (args[0] === "run" && args.includes("fixture-hold")) {
  await Bun.write(log + ".ready", "ready\n");
  await Bun.sleep(30_000);
}
if (args[0] === "run" && args.includes("fixture-fail")) process.exit(7);
console.log(JSON.stringify({ status: "fixture-ready" }));
