import { randomUUID } from "node:crypto";
import type { IntegrationInfo, IntegrationPushInput, IntegrationPushResult, IntegrationSyncResult } from "@karabiner/contracts";
import { argument, flag, maxArgs, revision, validateFlags, type Flags } from "./arguments.ts";
import { Client, ClientError } from "./client.ts";

const pushFields = ["title", "description", "status", "priority", "labels", "dueAt"] as const;

function selectedFields(flags: Flags): IntegrationPushInput["fields"] {
  const value = flag(flags, "fields");
  if (value === undefined) return undefined;
  const fields = value.split(",").map(field => field.trim());
  if (fields.some(field => !pushFields.includes(field as typeof pushFields[number]))) throw new ClientError("usage", `--fields must be a comma-separated list of ${pushFields.join(", ")}`);
  return [...new Set(fields)] as IntegrationPushInput["fields"];
}

async function push(client: Client, args: string[], flags: Flags): Promise<IntegrationPushResult> {
  validateFlags("integration push", flags, ["task", "revision", "request-id", "fields", "create-missing"]);
  const taskId = argument(args, 1, flags, "task", "TASK"), taskRevision = revision(flags);
  if (taskRevision === undefined) throw new ClientError("usage", "--revision is required; use the revision from task show TASK");
  const input: IntegrationPushInput = {
    revision: taskRevision, requestId: flag(flags, "request-id") ?? randomUUID(),
    fields: selectedFields(flags), createMissing: flags["create-missing"] === true,
  };
  if (!input.requestId.trim()) throw new ClientError("usage", "--request-id must not be empty");
  await client.connect();
  let result: IntegrationPushResult;
  try {
    result = await client.request<IntegrationPushResult>("POST", `/v1/tasks/${encodeURIComponent(taskId)}/push`, input, 120_000);
  } catch (error) {
    if (!(error instanceof ClientError)) throw error;
    throw new ClientError(error.code, error.message, error.status, { requestId: input.requestId, taskId, ...(error.details === undefined ? {} : { service: error.details }) });
  }
  if (result.status === "uncertain" || result.status === "failed" || result.errors.length) process.exitCode = 1;
  return result;
}

export async function integrationCommand(client: Client, args: string[], flags: Flags): Promise<unknown> {
  const [action = "status"] = args;
  maxArgs(args, 2, `integration ${action}`);
  if (action === "push") return push(client, args, flags);
  if (action !== "status" && action !== "sync") throw new ClientError("usage", `Unknown integration action: ${action}`);
  validateFlags(`integration ${action}`, flags, action === "status" ? ["project", "check"] : ["project", "conflict"]);
  const projectId = argument(args, 1, flags, "project", "PROJECT"), path = `/v1/projects/${encodeURIComponent(projectId)}`;
  const conflict = flag(flags, "conflict") ?? "preserve-local";
  if (action === "sync" && conflict !== "preserve-local" && conflict !== "overwrite-local") throw new ClientError("usage", "--conflict must be preserve-local or overwrite-local");
  await client.connect();
  if (action === "status") return client.request<IntegrationInfo>("GET", `${path}/integration${flags.check ? "?check=true" : ""}`, undefined, 120_000);
  const result = await client.request<IntegrationSyncResult>("POST", `${path}/sync`, { conflict }, 120_000);
  if (result.errors.length || result.conflicts) process.exitCode = 1;
  return result;
}
