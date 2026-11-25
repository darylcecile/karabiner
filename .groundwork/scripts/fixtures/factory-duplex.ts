/** Owned JSON-RPC child used by factory-parallel; no provider or user workspace access. */
import assert from "node:assert/strict";
import { appendFile, mkdir } from "node:fs/promises";
import { createInterface } from "node:readline";
import { join } from "node:path";

type Frame = { id?: string | number; method?: string; params?: Record<string, any>; result?: any; error?: { message?: string } };
const directory = join(process.cwd(), ".fixture");
await mkdir(directory, { recursive: true });
const controller = new AbortController();
const pending = new Map<string, ReturnType<typeof Promise.withResolvers<Record<string, any>>>>();
const work = new Set<Promise<void>>();
let identity: Record<string, any> = {};
let interactionWaiting = 0;
const deferredInstructions: Frame[] = [];
const send = (frame: Frame) => process.stdout.write(JSON.stringify(frame) + "\n");
const journal = (event: string, details: Record<string, unknown> = {}) => appendFile(join(directory, "events.jsonl"), JSON.stringify({ event, runId: identity.runId, stepId: identity.stepId, attemptId: identity.attemptId, pid: process.pid, at: Date.now(), ...details }) + "\n");
async function count(key: string): Promise<number> {
  const file = Bun.file(join(directory, identity.runId, key));
  const number = await file.exists() ? Number(await file.text()) + 1 : 1;
  await Bun.write(file, String(number));
  return number;
}
async function ask(id: string, method: string, params: Record<string, unknown>) {
  const result = Promise.withResolvers<Record<string, any>>();
  pending.set(id, result);
  interactionWaiting++;
  const abort = () => result.reject(new Error("Fixture request aborted"));
  controller.signal.addEventListener("abort", abort, { once: true });
  send({ id, method, params });
  try { return await result.promise; }
  finally {
    interactionWaiting--;
    controller.signal.removeEventListener("abort", abort); pending.delete(id);
    for (const frame of deferredInstructions.splice(0)) await acknowledgeInstruction(frame);
  }
}
async function acknowledgeInstruction(frame: Frame) {
  await journal("instruction", { instruction: frame.params });
  send({ id: frame.id, result: { delivered: !controller.signal.aborted } });
}
async function released(role: string) {
  const path = join(directory, identity.runId, `release-${role}`);
  const deadline = Date.now() + 25_000;
  while (!await Bun.file(path).exists()) {
    controller.signal.throwIfAborted();
    assert.ok(Date.now() < deadline, `Fixture ${role} was not released or cancelled`);
    send({ method: "text", params: { text: `${role}: waiting at owned barrier\n` } });
    await Bun.sleep(50);
  }
}
const complete = (summary: string) => ({ outcome: "complete", summary, returnToStageId: null, question: null });
async function run(params: Record<string, any>) {
  const role = String(params.agent.role);
  assert.equal(params.attemptId, identity.attemptId);
  const context = JSON.parse(String(params.prompt).trim().split("\n").at(-1)!);
  const visit = await count(`role-${role}`);
  await journal("start", { role, visit });
  await Bun.write(join(directory, identity.runId, `prompt-${role}-${visit}.json`), JSON.stringify(context));
  if (role.startsWith("hold-") || role === "fail") {
    await released(role);
    if (role === "fail") throw new Error("Deliberate unhandled branch failure");
    if (role === "hold-return" && visit === 1) return { outcome: "return", summary: "Bounded return inside right branch", returnToStageId: "right", question: null };
    await Bun.write(join(directory, identity.runId, `effect-${role}`), `${role} complete\n`);
    return complete(`${role} complete`);
  }
  if (role === "permission" || role === "permission-retry" || role === "permission-form") {
    const answer = await ask("permission-1", "permission", {
      prompt: "Permit the fixture to write its owned result?",
      choices: [{ id: "allow_once", label: "Allow once", effect: "approve" }, { id: "deny", label: "Deny", effect: "deny" }],
      tool: { name: "fixture_write", kind: "write", paths: ["fixture-result.txt"] },
      ...(role === "permission-form" ? { fields: [{ id: "count", label: "Count", type: "integer", required: true }] } : {}),
    });
    await journal("permission-answer", { role, visit, answer });
    if (answer.choice !== "allow_once") { assert.deepEqual(answer, { choice: "deny" }); throw new Error("Fixture permission was denied"); }
    await Bun.write(join(directory, identity.runId, `effect-${role}-${visit}`), `${role} allowed ${visit}\n`);
    if (role === "permission-retry" && visit === 1) throw new Error("First permitted attempt deliberately fails");
    return complete(`${role} complete`);
  }
  if (role === "form") {
    const answer = await ask("form-1", "question", {
      prompt: "Supply typed fixture values",
      fields: [
        { id: "count", label: "Count", type: "integer", required: true, minimum: 0, maximum: 9 },
        { id: "ratio", label: "Ratio", type: "number", required: true, minimum: 0, maximum: 1 },
        { id: "enabled", label: "Enabled", type: "boolean", required: true },
        { id: "tags", label: "Tags", type: "multiselect", required: true, minItems: 1, maxItems: 2, options: [{ id: "a", label: "A" }, { id: "b", label: "B" }] },
      ],
    });
    assert.deepEqual(answer.value, { count: 3, ratio: 0, enabled: false, tags: ["a", "b"] });
    await Bun.write(join(directory, identity.runId, "typed-form.json"), JSON.stringify(answer.value));
    await journal("form-answer", { role, answer });
    return complete("Typed form received by the live fixture");
  }
  if (role === "join-inner" || role === "join-outer") {
    assert.deepEqual(context.branchResults.map((branch: any) => branch.branchId), role === "join-inner" ? ["left", "right"] : ["short", "long"]);
    assert.ok(context.branchResults.every((branch: any) => branch.outcome === "success" && branch.summary.length));
    await Bun.write(join(directory, identity.runId, `${role}.json`), JSON.stringify(context.branchResults));
  }
  return complete(`${role} complete`);
}
async function handle(frame: Frame) {
  if (!frame.method) {
    const item = pending.get(String(frame.id));
    if (frame.error) item?.reject(new Error(frame.error.message));
    else item?.resolve(frame.result ?? {});
    return;
  }
  if (frame.method === "initialize") {
    identity = frame.params ?? {};
    assert.equal(identity.protocol, "karabiner-harness-v2");
    assert.equal(typeof identity.attemptId, "string");
    await mkdir(join(directory, identity.runId), { recursive: true });
    send({ id: frame.id, result: { protocol: "karabiner-harness-v2", sessionId: `fixture-${identity.attemptId}`, capabilities: { permissions: true, questions: true, steering: "native-turn" } } });
    return;
  }
  if (frame.method === "abort") { send({ id: frame.id, result: { aborted: true } }); controller.abort(new Error("Fixture abort acknowledged")); return; }
  if (frame.method === "instruction") { if (interactionWaiting) deferredInstructions.push(frame); else await acknowledgeInstruction(frame); return; }
  if (frame.method !== "run") throw new Error(`Unknown fixture method ${frame.method}`);
  try { send({ id: frame.id, result: await run(frame.params ?? {}) }); }
  finally { await journal("end", { role: frame.params?.agent.role }); }
}
process.stdout.on("error", () => process.exit(0));
const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of input) {
  const frame = JSON.parse(line) as Frame;
  const task = handle(frame).catch((error) => { send({ id: frame.id, error: { message: error instanceof Error ? error.message : String(error) } }); }).finally(() => work.delete(task));
  work.add(task);
}
controller.abort(new Error("Fixture input closed"));
await Promise.allSettled([...work]);
