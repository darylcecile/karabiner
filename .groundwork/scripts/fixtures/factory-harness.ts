/** Deterministic local process for the factory journey; all effects stay in its fixture cwd. */
import assert from "node:assert/strict";
import { appendFile, mkdir, readFile } from "node:fs/promises";
import { join } from "node:path";

const state = join(process.cwd(), ".fixture");
await mkdir(state, { recursive: true });
async function journal(event: string): Promise<void> { await appendFile(join(state, "events.jsonl"), JSON.stringify({ event }) + "\n"); }

if (process.argv[2] === "command") {
  const action = process.argv[3]!;
  await journal(action);
  if (action === "check" && (await Bun.file("result.txt").text()).includes("revision 1")) {
    console.error("First revision needs repair");
    process.exit(7);
  }
  if (action === "finish") await Bun.write("finished.txt", "The resumed pipeline reached its final command.\n");
  console.log(`${action} complete`);
  process.exit(0);
}

interface Request {
  protocol: string; runId: string; stepId: string; sessionId?: string;
  agent: { id: string; role: string }; instructions: string; prompt: string;
  answer?: { requestId: string; response: unknown };
}
const request = JSON.parse(await Bun.stdin.text()) as Request;
assert.equal(request.protocol, "karabiner-harness-v1");
assert.match(request.instructions, /project-context-marker/);
assert.match(request.instructions, /fixture-skill-marker/);
const directory = join(state, request.runId);
await mkdir(directory, { recursive: true });
await appendFile(join(state, "calls.jsonl"), JSON.stringify({ runId: request.runId, stepId: request.stepId, agentId: request.agent.id, role: request.agent.role, answered: !!request.answer, sessionId: request.sessionId ?? null }) + "\n");
await journal(request.agent.role + (request.answer ? ":answered" : ""));
console.log(JSON.stringify({ type: "text", text: `Observed ${request.agent.role}\n` }));

async function count(name: string): Promise<number> {
  const path = join(directory, name);
  const next = await Bun.file(path).exists() ? Number(await readFile(path, "utf8")) + 1 : 1;
  await Bun.write(path, String(next));
  return next;
}
function result(outcome: "complete" | "return" | "question", summary: string, extra: Record<string, unknown> = {}): void {
  console.log(JSON.stringify({ type: "result", session_id: `fixture-${request.runId}`, usage: { input_tokens: 7, output_tokens: 3 }, result: { outcome, summary, returnToStageId: null, question: null, ...extra } }));
}

switch (request.agent.role) {
  case "build": {
    const visit = await count("build-count");
    await Bun.write("result.txt", `Implemented revision ${visit}\n`);
    result("complete", `Built revision ${visit}`);
    break;
  }
  case "repair":
    await Bun.write("result.txt", "Repaired first revision\n");
    result("complete", "Repaired the observed failed check");
    break;
  case "review-first": {
    const visit = await count("review-count");
    result(visit === 1 ? "return" : "complete", visit === 1 ? "One declared implementation return" : "First reviewer approved", visit === 1 ? { returnToStageId: "build" } : {});
    break;
  }
  case "review-second":
    result("complete", "Distinct second reviewer approved");
    break;
  case "question":
    if (!request.answer) result("question", "Choose the delivery note", { question: "Which delivery note should be written?" });
    else {
      assert.equal(request.sessionId, `fixture-${request.runId}`);
      assert.equal(typeof request.answer.response, "string");
      await Bun.write("human-answer.txt", String(request.answer.response) + "\n");
      result("complete", `Human supplied: ${request.answer.response}`);
    }
    break;
  case "hold": {
    await appendFile(join(state, "effects.jsonl"), JSON.stringify({ runId: request.runId, effect: "started" }) + "\n");
    await Bun.write(join(directory, "ready.json"), JSON.stringify({ pid: process.pid, runId: request.runId }));
    const deadline = Date.now() + 30_000;
    while (!await Bun.file(join(directory, "release")).exists()) {
      assert.ok(Date.now() < deadline, "Fixture hold was neither released nor cancelled");
      await Bun.sleep(50);
    }
    result("complete", "Released local process");
    break;
  }
  case "stream-hold": {
    // Like hold, but prints progress every ~150ms so the client sees live run updates.
    await Bun.write(join(directory, "ready.json"), JSON.stringify({ pid: process.pid, runId: request.runId }));
    const deadline = Date.now() + 30_000;
    for (let tick = 1; !await Bun.file(join(directory, "release")).exists(); tick++) {
      assert.ok(Date.now() < deadline, "Fixture stream-hold was neither released nor cancelled");
      console.log(JSON.stringify({ type: "text", text: `tick ${tick}\n` }));
      await Bun.sleep(150);
    }
    result("complete", "Released streaming process");
    break;
  }
  case "once":
    await appendFile("scheduled.txt", `${request.runId}\n`);
    result("complete", "Scheduled task executed once");
    break;
  case "guidance":
    await Bun.write("guidance.txt", request.instructions);
    result("complete", "Inspected the actual invocation instructions");
    break;
  default:
    throw new Error(`Unknown fixture role ${request.agent.role}`);
}
