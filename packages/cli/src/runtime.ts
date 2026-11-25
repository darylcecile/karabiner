import { RUN_STATES, type AddRunInstruction, type AnswerRunQuestion, type CreateRun, type Run, type RunControl, type RunQuestion, type Task } from "@karabiner/contracts";
import { argument, flag, jsonFile, maxArgs, query, required, revision, validateFlags, type Flags } from "./arguments.ts";
import { Client, ClientError } from "./client.ts";

const runActions = ["list", "show", "get", "start", "pause", "resume", "cancel", "retry", "instruct"];
const runQuery = [["project", "projectId"], ["task", "taskId"], ["state", "state"]] as const;

function checkScope(value: { projectId?: string; taskId?: string }, flags: Flags): void {
  for (const key of ["project", "task"] as const) {
    const scope = flag(flags, key);
    if (scope !== undefined && value[`${key}Id`] !== scope) throw new ClientError("not_found", `Record does not belong to --${key} ${scope}`);
  }
}

async function start(client: Client, args: string[], flags: Flags): Promise<Run> {
  validateFlags("run start", flags, ["task", "project", "workflow", "revision"]);
  const taskId = argument(args, 1, flags, "task", "TASK");
  const input: CreateRun = { workflowId: flag(flags, "workflow"), revision: revision(flags) };
  await client.connect();
  if (flag(flags, "project") !== undefined) {
    const task = await client.request<Task>("GET", `/v1/tasks/${encodeURIComponent(taskId)}`);
    checkScope({ projectId: task.projectId, taskId: task.id }, flags);
  }
  return client.request<Run>("POST", `/v1/tasks/${encodeURIComponent(taskId)}/run`, input);
}

export async function runCommand(client: Client, supplied: string[], flags: Flags): Promise<Run | Run[]> {
  // Retain the original `run TASK` shorthand for starting a task.
  const args = supplied.length && !runActions.includes(supplied[0]!) ? ["start", ...supplied] : supplied;
  const [action = "list", id] = args;
  maxArgs(args, action === "list" ? 1 : action === "instruct" ? 3 : 2, `run ${action}`);
  if (action === "start") return start(client, args, flags);
  if (action === "list") {
    validateFlags("run list", flags, ["project", "task", "state"]);
    const state = flag(flags, "state");
    if (state !== undefined && !RUN_STATES.includes(state as typeof RUN_STATES[number])) throw new ClientError("usage", `--state must be one of ${RUN_STATES.join(", ")}`);
    await client.connect();
    return client.request<Run[]>("GET", "/v1/runs" + query(flags, runQuery));
  }
  const path = `/v1/runs/${encodeURIComponent(required(id, "RUN"))}`;
  if (action === "instruct") {
    validateFlags("run instruct", flags, ["message", "revision"]);
    const input: AddRunInstruction = { message: argument(args, 2, flags, "message", "TEXT"), revision: revision(flags) };
    await client.connect();
    return client.request<Run>("POST", `${path}/instructions`, input);
  }
  if (action === "show" || action === "get") {
    validateFlags(`run ${action}`, flags, ["project", "task"]);
    await client.connect();
    const run = await client.request<Run>("GET", path);
    checkScope(run, flags);
    return run;
  }
  validateFlags(`run ${action}`, flags, ["revision"]);
  const input: RunControl = { revision: revision(flags) };
  await client.connect();
  return client.request<Run>("POST", `${path}/${action}`, input);
}

export type AddressedQuestion = RunQuestion & { runId: string; taskId: string; projectId?: string };

async function questionResponse(args: string[], flags: Flags): Promise<AnswerRunQuestion> {
  const path = flag(flags, "file"), choice = flag(flags, "choice"), inputRevision = revision(flags);
  const text = args[2] !== undefined || flag(flags, "message") !== undefined;
  if ([path !== undefined, choice !== undefined, text].filter(Boolean).length > 1) throw new ClientError("usage", "Choose one response: TEXT/--message, --choice, or --file");
  if (path !== undefined) {
    const response = await jsonFile(required(path, "--file response.json"));
    if (inputRevision !== undefined && response.revision !== undefined && inputRevision !== response.revision) throw new ClientError("usage", "--revision conflicts with the response file revision");
    return { ...response, ...(inputRevision === undefined ? {} : { revision: inputRevision }) };
  }
  if (choice !== undefined) return { choice: required(choice, "--choice CHOICE"), revision: inputRevision };
  return { answer: argument(args, 2, flags, "message", "TEXT"), revision: inputRevision };
}

function addressed(question: RunQuestion, run: Run): AddressedQuestion {
  return { ...question, runId: run.id, taskId: run.taskId, projectId: run.projectId };
}

export async function questionCommand(client: Client, args: string[], flags: Flags): Promise<Run | AddressedQuestion | AddressedQuestion[]> {
  const [action = "list", id] = args;
  if (action === "answer") {
    maxArgs(args, 3, "question answer");
    validateFlags("question answer", flags, ["run", "message", "revision", "choice", "file"]);
    const runId = required(flag(flags, "run"), "--run RUN"), questionId = required(id, "QUESTION");
    const input = await questionResponse(args, flags);
    await client.connect();
    return client.request<Run>("POST", `/v1/runs/${encodeURIComponent(runId)}/questions/${encodeURIComponent(questionId)}/answer`, input);
  }
  if (action === "show" || action === "get") {
    maxArgs(args, 2, `question ${action}`);
    validateFlags(`question ${action}`, flags, ["run"]);
    const runId = required(flag(flags, "run"), "--run RUN"), questionId = required(id, "QUESTION");
    await client.connect();
    const run = await client.request<Run>("GET", `/v1/runs/${encodeURIComponent(runId)}`);
    const question = run.questions?.find(value => value.id === questionId);
    if (!question) throw new ClientError("not_found", "Question was not found in this run");
    return addressed(question, run);
  }
  if (action !== "list") throw new ClientError("usage", `Unknown question action: ${action}`);
  maxArgs(args, 1, "question list");
  validateFlags("question list", flags, ["run", "project", "task"]);
  await client.connect();
  const runId = flag(flags, "run");
  const runs = runId
    ? [await client.request<Run>("GET", `/v1/runs/${encodeURIComponent(runId)}`)]
    : await client.request<Run[]>("GET", "/v1/runs" + query(flags, runQuery));
  if (runId) checkScope(runs[0]!, flags);
  return runs.flatMap(run => (run.questions ?? []).filter(question => question.state === "pending").map(question => addressed(question, run)));
}
