/** Retains an actual post-capture review; does not replay UI or manufacture observations. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";

const destination = process.env.GROUNDWORK_EVIDENCE_DIR;
const selected = process.env.KARABINER_NATIVE_REVIEW;
if (!destination || !selected) {
  console.error("Set KARABINER_NATIVE_REVIEW to the Opus review JSON beside its recorded capture files; Groundwork supplies the destination.");
  process.exit(2);
}
const source = resolve(selected), origin = dirname(source);
const review = JSON.parse(await readFile(source, "utf8")) as {
  observations: { id?: string; kind: string; status: "passed" | "failed" | "inconclusive"; observed: string; covers: string[]; evidence: string[] }[];
};
assert.ok(review.observations?.length, "The reviewer must supply actual observations");
const files = new Map<string, string>();
for (const observation of review.observations) {
  assert.equal(observation.kind, "review", "This check only records an actual review");
  assert.ok(observation.observed?.trim() && observation.covers?.length);
  assert.ok(["passed", "failed", "inconclusive"].includes(observation.status));
  if (observation.status !== "inconclusive") assert.ok(observation.evidence.length, "A review conclusion needs inspected evidence");
  for (const name of observation.evidence) {
    const path = resolve(origin, name), local = relative(origin, path);
    assert.ok(local && !isAbsolute(name) && local !== ".." && !local.startsWith("../") && name !== "result.json" && name !== "review-source.json", "Evidence must be a concrete capture/review file beside the review manifest");
    const bytes = await readFile(path);
    assert.ok(bytes.length, `Empty evidence ${name}`);
    const output = join(destination, name);
    await mkdir(dirname(output), { recursive: true });
    await writeFile(output, bytes);
    const digest = createHash("sha256").update(bytes).digest("hex");
    assert.equal(createHash("sha256").update(await readFile(output)).digest("hex"), digest);
    files.set(name, digest);
  }
}
await writeFile(join(destination, "review-source.json"), JSON.stringify({ reviewFile: source, capturedEvidence: Object.fromEntries(files), operation: "Post-capture review of retained native/React Flow images; no fresh UI execution" }, null, 2));
await writeFile(join(destination, "result.json"), JSON.stringify({ observations: review.observations.map((observation) => ({ ...observation, observed: `Review of retained captures (${source}): ${observation.observed}`, evidence: [...observation.evidence, "review-source.json"] })), context: { observationMode: "retained-capture-review", freshUiExecution: false } }, null, 2));
process.exitCode = review.observations.some((observation) => observation.status === "failed") ? 1 : review.observations.some((observation) => observation.status === "inconclusive") ? 2 : 0;
