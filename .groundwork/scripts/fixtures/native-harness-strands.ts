import assert from "node:assert/strict";
import { join } from "node:path";
import { Model, type BaseModelConfig, type Message, type ModelStreamEvent, type StreamOptions } from "../../../packages/service/node_modules/@strands-agents/sdk/dist/src/index.js";
import { Store } from "../../../packages/service/src/database.ts";
import { Repository } from "../../../packages/service/src/repository.ts";
import { RunStore } from "../../../packages/service/src/run-store.ts";
import { createRuntime } from "../../../packages/service/src/execution.ts";
import { executeStrands } from "../../../packages/service/src/runtime/strands-adapter.ts";
import { folder, waitFor } from "./native-harness-api.ts";
import type { StoredRun } from "../../../packages/contracts/src/index.ts";

class BoundaryModel extends Model {
  private config: BaseModelConfig = { modelId: "fixture-native-boundary", maxTokens: 512 };
  readonly calls: { tool: string; messages: unknown }[] = [];
  constructor(private readonly root: string, private readonly mode: "approve" | "deny" | "cancel") { super(); }
  updateConfig(value: BaseModelConfig) { this.config = { ...this.config, ...value }; }
  getConfig() { return this.config; }
  async *stream(messages: Message[], options?: StreamOptions): AsyncIterable<ModelStreamEvent> {
    options?.cancelSignal?.throwIfAborted();
    const index = this.calls.length;
    const previous = messages.flatMap((message) => message.content).findLast((block) => block.type === "toolResultBlock");
    let name: string, input: unknown;
    if (index === 0) { name = "write"; input = { path: join(this.root, "strands-effect.txt"), content: "actual Strands file tool\n" }; }
    else if (this.mode === "deny") {
      assert.equal(index, 1); assert.equal(previous?.type === "toolResultBlock" && previous.status, "error");
      name = "finish_step"; input = { outcome: "blocked", summary: "Operator denied write", returnToStageId: null, question: null };
    } else if (index === 1) {
      assert.equal(previous?.type === "toolResultBlock" && previous.status, "success");
      assert.match(JSON.stringify(messages), /live-strands-guidance/);
      name = "shell"; input = { command: `printf '%s\\n' 'actual Strands shell tool' > ${JSON.stringify(join(this.root, "strands-shell.txt"))}` };
    } else if (index === 2) {
      assert.equal(previous?.type === "toolResultBlock" && previous.status, "success");
      name = "read"; input = { path: join(this.root, "strands-effect.txt") };
    } else if (index === 3) {
      assert.equal(previous?.type === "toolResultBlock" && previous.status, "success");
      assert.match(JSON.stringify(previous), /actual Strands file tool/);
      name = "ask_user"; input = { question: "Confirm the owned Strands effect" };
    } else {
      assert.equal(index, 4); assert.match(JSON.stringify(previous), /Answer exactly/);
      name = "finish_step"; input = { outcome: "complete", summary: "Real tool and in-flight guidance observed", returnToStageId: null, question: null };
    }
    assert.ok(options?.toolSpecs?.some((tool) => tool.name === name));
    this.calls.push({ tool: name, messages: messages.map((message) => message.toJSON()) });
    yield { type: "modelMessageStartEvent", role: "assistant" };
    yield { type: "modelContentBlockStartEvent", start: { type: "toolUseStart", toolUseId: `boundary-${index}`, name } };
    yield { type: "modelContentBlockDeltaEvent", delta: { type: "toolUseInputDelta", input: JSON.stringify(input) } };
    yield { type: "modelContentBlockStopEvent" };
    yield { type: "modelMessageStopEvent", stopReason: "toolUse" };
    yield { type: "modelMetadataEvent", usage: { inputTokens: 5, outputTokens: 2, totalTokens: 7 } };
  }
}

export async function verifyNativeStrands(root: string, database: string) {
  const store = new Store(database), repository = new Repository(store), runs = new RunStore(store);
  const observations: { mode: string; status: "passed" | "failed"; error?: string; run?: StoredRun; modelCalls: BoundaryModel["calls"]; sessionKeys: string[] }[] = [];
  try {
    for (const mode of ["approve", "deny", "cancel"] as const) {
      const directory = await folder(join(root, mode));
      const model = new BoundaryModel(directory, mode);
      const runtime = createRuntime({ repository, runs, changes: { changed() {} }, executeAgent: (request) => executeStrands(request, { model }) });
      let runId: string | undefined;
      let error: unknown;
      try {
        const project = repository.createProject({ name: `Strands ${mode}`, rootPath: directory });
        repository.saveConfig(project.id, { revision: store.config(project.id).revision, executionTarget: "host", skills: [], instructions: [], agents: [{ id: "model", name: "Model", role: "developer", harness: "strands", model: "fixture-native-boundary", mode: "interactive", instructions: "Use real file tools.", skillIds: [] }], workflows: [{ id: "native", name: "Native boundary", description: "", instructions: "", stages: [{ id: "step", name: "Step", instructions: "", agentIds: ["model"], returnToStageIds: [], execution: { retries: 2, timeoutMs: 15_000 } }] }] });
        const task = repository.createTask({ projectId: project.id, title: "Native Strands boundary", workflowId: "native" });
        runtime.startScheduler();
        const run = await runtime.start(task.id);
        runId = run.id;
        const read = async () => runs.get(run.id)!;
        let current = await waitFor("Strands live permission", read, (run) => run.questions.some((question) => question.state === "pending"));
        const first = current.questions.find((question) => question.state === "pending")!;
        assert.equal(first.kind, "permission"); assert.equal(first.transport, "live"); assert.ok(first.attemptId);
        if (mode === "cancel") await runtime.control(run.id, "cancel");
        else {
          if (mode === "approve") await runtime.instruct(run.id, { message: "live-strands-guidance" });
          await runtime.answer(run.id, first.id, { choice: mode === "approve" ? "allow" : "deny" });
          if (mode === "approve") {
            current = await waitFor("Strands shell permission", read, (run) => run.questions.some((question) => question.kind === "permission" && question.state === "pending"));
            const shell = current.questions.find((question) => question.kind === "permission" && question.state === "pending")!;
            assert.equal(shell.tool?.name, "shell");
            await runtime.answer(run.id, shell.id, { choice: shell.choices!.find((choice) => choice.effect === "approve")!.id });
            current = await waitFor("Strands live question", read, (run) => run.questions.some((question) => question.kind === "question" && question.state === "pending"));
            const question = current.questions.find((question) => question.kind === "question" && question.state === "pending")!;
            await runtime.answer(run.id, question.id, { answer: "Answer exactly" });
          }
        }
        current = await waitFor("Strands terminal state", read, (run) => ["completed", "failed", "cancelled"].includes(run.state));
        assert.equal(current.state, mode === "approve" ? "completed" : mode === "deny" ? "failed" : "cancelled", JSON.stringify(current));
        assert.equal(current.stages![0]!.steps[0]!.attempts!.length, 1);
        const sessionKeys = runs.listBlobs(run.id).filter((key) => key.endsWith("snapshot_latest.json"));
        if (mode === "approve") {
          assert.equal(await Bun.file(join(directory, "strands-effect.txt")).text(), "actual Strands file tool\n");
          assert.equal(await Bun.file(join(directory, "strands-shell.txt")).text(), "actual Strands shell tool\n");
          assert.ok(current.instructions?.some((instruction) => instruction.deliveries?.some((delivery) => delivery.mode === "native-tool-boundary" && delivery.attemptId === first.attemptId)));
          assert.deepEqual(model.calls.map((call) => call.tool), ["write", "shell", "read", "ask_user", "finish_step"]);
          assert.equal(current.stages![0]!.id, first.stageInvocationId);
          assert.equal(current.stages![0]!.steps[0]!.id, first.stepId);
          assert.equal(current.tokens, model.calls.length * 7);
          assert.equal(current.cost, null);
          assert.ok(sessionKeys.length, "Actual SDK session snapshots persist in service-owned SQLite");
          for (const name of ["write", "shell", "read"]) assert.ok(current.traceSpans?.some((span) => span.kind === "tool" && span.name === name), `${name} has an observed tool span`);
        } else assert.equal(await Bun.file(join(directory, "strands-effect.txt")).exists(), false);
      } catch (caught) { error = caught; }
      finally {
        try {
          const current = runId ? runs.get(runId) : null;
          if (current && !["completed", "failed", "cancelled"].includes(current.state)) await runtime.control(current.id, "cancel");
          await runtime.shutdown();
        } catch (caught) { error = error ? new AggregateError([error, caught], "Strands scenario and cleanup failed") : caught; }
      }
      observations.push({ mode, status: error ? "failed" : "passed", error: error ? error instanceof Error ? error.stack ?? error.message : String(error) : undefined, run: runId ? runs.get(runId) ?? undefined : undefined, modelCalls: model.calls, sessionKeys: runId ? runs.listBlobs(runId).filter((key) => key.endsWith("snapshot_latest.json")) : [] });
    }
    return observations;
  } finally { store.close(); }
}
