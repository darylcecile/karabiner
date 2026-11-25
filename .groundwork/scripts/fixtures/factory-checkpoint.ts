/** Legacy JSONL checkpoint whose first answered attempt fails, requiring a fresh retry question. */
import assert from "node:assert/strict";
import { appendFile, mkdir } from "node:fs/promises";
import { join } from "node:path";

const request = JSON.parse(await Bun.stdin.text());
assert.equal(request.protocol, "karabiner-harness-v1");
const directory = join(process.cwd(), ".fixture", request.runId);
await mkdir(directory, { recursive: true });
await appendFile(join(directory, "checkpoint-calls.jsonl"), JSON.stringify({ stepId: request.stepId, answer: request.answer?.response ?? null, pid: process.pid }) + "\n");
const failed = Bun.file(join(directory, "first-answer-failed"));
function result(outcome: string, summary: string, question: string | null = null) {
  console.log(JSON.stringify({ type: "result", session_id: `checkpoint-${request.stepId}`, result: { outcome, summary, question, returnToStageId: null } }));
}
if (!request.answer) result("question", "A fresh answer is required", "Answer this attempt's checkpoint");
else if (!await failed.exists()) {
  assert.equal(request.answer.response, "first answer");
  await Bun.write(failed, "First answered attempt failed\n");
  console.error("Deliberate failure after consuming the first checkpoint answer");
  process.exitCode = 7;
} else {
  assert.equal(request.answer.response, "second answer");
  await Bun.write(join(directory, "checkpoint-result.txt"), request.answer.response);
  result("complete", "A fresh answer completed the retry");
}
