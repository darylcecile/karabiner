import type { Run } from "@karabiner/contracts";
import type { AddressedQuestion } from "./runtime.ts";

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isQuestion(value: unknown): value is AddressedQuestion {
  return record(value) && typeof value.prompt === "string" && typeof value.runId === "string";
}

function questionText(question: AddressedQuestion): string {
  const { id, runId, kind, state, prompt, choices, ...details } = question;
  const lines = [`${kind} ${id} — ${state}`, `Run: ${runId}`, prompt];
  if (choices?.length) lines.push("Choices:", ...choices.map(choice => `  ${choice.id}: ${choice.label}`));
  lines.push("Details:", JSON.stringify(details, null, 2));
  return lines.join("\n").replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, " ");
}

function runColumns(run: Run): unknown[] {
  const active = run.activeStageIds ?? (run.currentStageId ? [run.currentStageId] : []);
  return [run.id, run.state, `task=${run.taskId}`, `workflow=${run.workflowId}`,
    ...(active.length ? [`active=${active.join(",")}`] : []),
    ...(run.questions ? [`pending=${run.questions.filter(question => question.state === "pending").length}`] : []),
    run.activity ?? ""];
}

function recordText(value: unknown): string {
  if (isQuestion(value)) return questionText(value);
  if (!record(value)) return JSON.stringify(value) ?? String(value);
  // Keep additive readiness and interaction capabilities visible, including unknown values.
  if ("executionAvailable" in value || ("requirements" in value && "available" in value)) return JSON.stringify(value, null, 2);
  const columns = "workflowId" in value && "state" in value
    ? runColumns(value as unknown as Run)
    : [value.id ?? value.sha ?? "", value.name ?? value.title ?? value.message ?? "", value.status ?? value.kind ?? ""];
  return columns.join("  ").replace(/[\x00-\x1f\x7f]/g, " ");
}

export function print(value: unknown, json: boolean): void {
  if (json) { console.log(JSON.stringify({ data: value })); return; }
  if (isQuestion(value)) { console.log(questionText(value)); return; }
  if (!Array.isArray(value)) { console.log(JSON.stringify(value, null, 2)); return; }
  if (!value.length) { console.log("No records."); return; }
  for (const item of value) console.log(recordText(item));
}
