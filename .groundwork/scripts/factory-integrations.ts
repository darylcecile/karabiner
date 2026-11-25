import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, realpath, rm, symlink } from "node:fs/promises";
import { cpus, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { Store } from "../../packages/service/src/database.ts";
import { Repository } from "../../packages/service/src/repository.ts";
import { IntegrationService } from "../../packages/service/src/integrations/service.ts";
import { prepareLaunch, targetReadiness } from "../../packages/service/src/runtime/targets.ts";
import { runPrepared } from "../../packages/service/src/runtime/process.ts";

const root = resolve(import.meta.dir, "../..");
const evidence = process.env.GROUNDWORK_EVIDENCE_DIR ?? join(root, ".groundwork/tmp/factory-integrations");
const liveGithub = process.env.FACTORY_GITHUB_READONLY === "1";
const artifact = liveGithub ? "github-readonly.json" : "factory-integrations.json";
await mkdir(evidence, { recursive: true });
const temporary = await mkdtemp(join(tmpdir(), "karabiner-connectors-"));
const observations: { id: string; status?: "passed" | "inconclusive"; observed: string; details: unknown }[] = [];
let failure: string | undefined;

function connection(nodes: unknown[], cursor: string | null = null) {
  return { nodes, pageInfo: { hasNextPage: cursor !== null, endCursor: cursor } };
}

async function bounded<T>(promise: Promise<T>, label: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([promise, new Promise<never>((_resolve, reject) => { timer = setTimeout(() => reject(new Error(`${label} did not reach its fixture checkpoint`)), 5_000); })]);
  } finally { if (timer) clearTimeout(timer); }
}

async function targetFixture(name: string) {
  const directory = join(temporary, name), log = join(temporary, `${name}-calls.jsonl`), binary = join(temporary, `${name}-target`);
  const script = join(root, ".groundwork/scripts/fixtures/target-executable.ts");
  await mkdir(join(directory, "subdir"), { recursive: true });
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
  await Bun.write(binary, `#!/bin/sh\nexport KARABINER_FIXTURE_LOG=${quote(log)}\nexec ${quote(process.execPath)} ${quote(script)} "$@"\n`);
  await chmod(binary, 0o755);
  return { directory, log, binary, script };
}

async function events(log: string): Promise<{ kind: string; mode?: string; configPath?: string; args?: string[] }[]> {
  return await Bun.file(log).exists() ? (await Bun.file(log).text()).trim().split("\n").filter(Boolean).map((line) => JSON.parse(line)) : [];
}

async function linear(): Promise<void> {
  const store = new Store(":memory:"), repo = new Repository(store);
  const remote = [1, 2].map((id) => ({ id: `issue-${id}`, identifier: `ENG-${id}`, title: `Remote ${id}`, description: "body", url: `https://linear.app/x/issue/ENG-${id}`, updatedAt: "2026-11-25T00:00:00Z", archivedAt: null, priority: 3, dueDate: null, team: { id: "team-1" }, project: null, state: { id: "state-todo", name: "Todo", type: "unstarted" }, labels: { nodes: [], pageInfo: { hasNextPage: false, endCursor: null } } }));
  const requests: { operation: string; after?: string }[] = [];
  const service = new IntegrationService(repo, {
    env: { KARABINER_TEST_LINEAR: "synthetic-fixture-value" },
    fetch: (async (_url, options) => {
      const body = JSON.parse(String(options?.body)) as { query: string; variables: { after?: string } };
      if (body.query.includes("teams(filter")) {
        requests.push({ operation: "teams" });
        return Response.json({ data: { teams: { nodes: [{ id: "team-1" }] } } });
      }
      if (body.query.includes("states(first")) {
        requests.push({ operation: "states", after: body.variables.after });
        const second = body.variables.after === "states-2";
        return Response.json({ data: { team: { id: "team-1", name: "Engineering", key: "ENG", states: connection(second
          ? [{ id: "state-review", name: "Review queue", type: "started", color: "#888888" }]
          : [{ id: "state-todo", name: "Todo", type: "unstarted", color: "#555555" }], second ? null : "states-2") } } });
      }
      assert.ok(body.query.includes("issues(filter"), "Only the expected fixture GraphQL operations are used");
      requests.push({ operation: "issues", after: body.variables.after });
      const second = body.variables.after === "page-2";
      return Response.json({ data: { issues: { nodes: [remote[second ? 1 : 0]], pageInfo: { hasNextPage: !second, endCursor: second ? null : "page-2" } } } });
    }) as typeof fetch,
  });
  try {
    const project = repo.createProject({ name: "Linear fixture", kind: "linear", externalRef: "ENG" });
    repo.saveConfig(project.id, { ...store.config(project.id), integration: { provider: "linear", scope: "team", apiKeyEnv: "KARABINER_TEST_LINEAR" } });
    const status = await service.status(project.id);
    assert.equal(status.status, "unknown");
    assert.equal(status.authenticated, null);
    assert.equal(requests.length, 0);
    const missing = new IntegrationService(repo, { env: {} });
    const missingStatus = await missing.status(project.id);
    await missing.close();
    assert.equal(missingStatus.code, "credential_env_missing");
    assert.match(missingStatus.message, /KARABINER_TEST_LINEAR/);
    const catalog = await service.catalog(project.id);
    assert.deepEqual(catalog.source, { id: "team-1", name: "Engineering" });
    assert.deepEqual(catalog.teams[0]!.states.map((state) => [state.id, state.name]), [["state-todo", "Todo"], ["state-review", "Review queue"]]);
    assert.equal(catalog.teams[0]!.states[0]!.suggestedStatus, "todo");
    const catalogRequests = requests.splice(0);
    const imported = await service.sync(project.id);
    assert.equal(imported.imported, 2, JSON.stringify(imported));
    assert.deepEqual(requests, [{ operation: "teams" }, { operation: "issues", after: undefined }, { operation: "issues", after: "page-2" }]);
    const original = store.list("tasks", { projectId: project.id });
    await service.sync(project.id);
    assert.deepEqual(store.list("tasks", { projectId: project.id }), original);
    const task = original.find((item) => item.external?.id === "issue-1")!;
    repo.updateTask(task.id, { title: "Local edit", revision: task.revision });
    remote[0]!.title = "Remote edit";
    remote[0]!.updatedAt = "2026-11-25T01:00:00Z";
    const conflict = await service.sync(project.id);
    assert.ok(conflict.errors.some((item) => item.code === "local_conflict"));
    assert.equal(store.get("tasks", task.id).title, "Local edit");
    const overwritten = await service.sync(project.id, { conflict: "overwrite-local" });
    assert.equal(store.get("tasks", task.id).title, "Remote edit");
    repo.remove("tasks", task.id);
    const tombstone = await service.sync(project.id);
    assert.equal(store.list("tasks", { projectId: project.id }).length, 1);
    assert.ok(tombstone.skipped >= 1);
    const remaining = store.list("tasks", { projectId: project.id })[0]!;
    remote[1]!.state = { id: "state-review", name: "Review queue", type: "started" };
    const mapped = { provider: "linear", scope: "team", apiKeyEnv: "KARABINER_TEST_LINEAR", pushStatusMap: { "team-1": { in_review: "state-review" } } };
    repo.saveConfig(project.id, { ...store.config(project.id), integration: mapped });
    await service.sync(project.id);
    assert.equal(store.get("tasks", remaining.id).status, "in_review", "Unique explicit push mapping supports review-state readback");
    repo.saveConfig(project.id, { ...store.config(project.id), integration: { ...mapped, pullStatusMap: { "state-review": "blocked" } } });
    await service.sync(project.id);
    assert.equal(store.get("tasks", remaining.id).status, "blocked", "Explicit pull mapping takes precedence");
    repo.saveConfig(project.id, { ...store.config(project.id), integration: { ...mapped, pushStatusMap: {}, pullStatusMap: {} } });
    await service.sync(project.id);
    assert.equal(store.get("tasks", remaining.id).status, "in_progress", "Clearing mappings restores normal provider-state readback");
    observations.push({ id: "linear", observed: "Injected Linear transport proves named paginated catalogs, precise missing-credential status, stable import identity, local conflicts/tombstones and editable status-map readback.", details: { status, missingStatus, catalog, catalogRequests, imported, conflict, overwritten, tombstone, requests } });
  } finally { await service.close(); store.close(); }
}

async function github(): Promise<void> {
  const store = new Store(":memory:"), repo = new Repository(store);
  let remote: { node_id: string; number: number; title: string; body: string; html_url: string; updated_at: string; state: string; state_reason: string | null; labels: never[] } | undefined;
  const requests: { method: string; endpoint: string; input?: unknown }[] = [];
  let creates = 0, updates = 0;
  const service = new IntegrationService(repo, { env: {}, run: async (request) => {
    assert.equal(request.command, "gh");
    const args = request.args!, method = args[args.indexOf("--method") + 1]!, endpoint = args.find((arg) => arg.startsWith("repos/"))!;
    requests.push({ method, endpoint, input: request.input ? JSON.parse(request.input) : undefined });
    if (method === "POST") {
      creates++;
      const input = JSON.parse(request.input!) as { title: string; body: string };
      remote = { node_id: "I_1", number: 1, title: input.title, body: input.body, html_url: "https://github.com/owner/repo/issues/1", updated_at: "2026-11-25T00:00:00Z", state: "open", state_reason: null, labels: [] };
      throw new Error("Fixture reply lost after issue creation");
    }
    if (method === "PATCH") {
      updates++;
      const input = JSON.parse(request.input!) as { title?: string; body?: string; state?: string; state_reason?: string | null };
      assert.ok(remote, "Update targets the one existing fixture issue");
      Object.assign(remote, input, { updated_at: "2026-11-25T01:00:00Z" });
      if (input.state === "open" && input.state_reason === undefined) remote.state_reason = null;
      return { code: 0, stdout: JSON.stringify(remote), stderr: "" };
    }
    assert.equal(method, "GET");
    const data = endpoint === "repos/owner/repo" ? { node_id: "R_1", full_name: "owner/repo", html_url: "https://github.com/owner/repo" } : endpoint.includes("?") ? (remote ? [remote] : []) : remote;
    return { code: 0, stdout: JSON.stringify(data), stderr: "" };
  } });
  try {
    const project = repo.createProject({ name: "GitHub fixture", kind: "github", repository: "owner/repo" });
    repo.saveConfig(project.id, { ...store.config(project.id), integration: { provider: "github", scope: "repository", pushEnabled: true } });
    const catalog = await service.catalog(project.id);
    assert.equal(catalog.source.name, "owner/repo");
    assert.equal(catalog.statusFields[0]!.id, "issue-state");
    assert.deepEqual(catalog.statusFields[0]!.options.map((option) => [option.id, option.name]), [["open", "Open"], ["completed", "Completed"], ["not_planned", "Not planned"]]);
    const task = repo.createTask({ projectId: project.id, title: "Create once", description: "A real local record", status: "done" });
    const input = { revision: task.revision, requestId: "fixture-create-once", createMissing: true };
    const uncertain = await service.push(task.id, input);
    assert.equal(uncertain.status, "uncertain");
    const editedDuringLoss = repo.updateTask(task.id, { revision: task.revision, title: "New local edit after lost reply", status: "in_progress" });
    const beforeWrongKey = requests.length;
    await assert.rejects(service.push(task.id, { revision: editedDuringLoss.revision, requestId: "fixture-wrong-retry-key", createMissing: true }), { status: 409, message: /previous push.*requestId/i });
    assert.equal(requests.length, beforeWrongKey);
    const changedPolicy = store.config(project.id);
    repo.saveConfig(project.id, { ...changedPolicy, integration: { ...changedPolicy.integration!, pushFields: ["status"] } });
    const recovered = await service.push(task.id, input);
    assert.equal(recovered.status, "pushed", JSON.stringify(recovered));
    assert.equal(store.get("tasks", task.id).external?.id, "I_1");
    assert.equal(store.get("tasks", task.id).title, editedDuringLoss.title, "Reconciliation preserves edits made after the uncertain create");
    assert.equal(store.get("tasks", task.id).status, "in_progress");
    assert.equal(store.get("tasks", task.id).revision, editedDuringLoss.revision);
    assert.equal(remote!.title, "Create once", "Retry reconciles the original publication rather than silently publishing newer edits");
    assert.equal(remote!.state, "open", "Retry retains the original title/description selection even after project policy changes");
    const requestCount = requests.length;
    const receipt = await service.push(task.id, input);
    assert.deepEqual(receipt, recovered);
    assert.equal(requests.length, requestCount);
    assert.equal(creates, 1);
    const edited = repo.updateTask(task.id, { title: "Local intended title" });
    remote!.title = "Someone else's remote title";
    const conflict = await service.push(task.id, { revision: edited.revision, requestId: "fixture-conflict", fields: ["title"] });
    assert.equal(conflict.status, "failed");
    assert.ok(conflict.errors.some((item) => item.code === "remote_conflict"));
    assert.equal(creates, 1);
    assert.equal(updates, 0);
    remote!.state = "closed";
    remote!.state_reason = "not_planned";
    repo.saveConfig(project.id, { ...store.config(project.id), integration: { provider: "github", scope: "repository", pushEnabled: true, pullStatusMap: { not_planned: "blocked" } } });
    await service.sync(project.id, { conflict: "overwrite-local" });
    assert.equal(store.get("tasks", task.id).status, "blocked");
    const selectedStatus = repo.updateTask(task.id, { title: "Newer local title stays local", status: "done" });
    const current = store.config(project.id);
    repo.saveConfig(project.id, { ...current, integration: { ...current.integration!, pushFields: ["status"], pushStatusMap: { done: "completed" } } });
    const remoteTitle = remote!.title;
    const statusPush = await service.push(task.id, { revision: selectedStatus.revision, requestId: "fixture-selected-status" });
    assert.equal(statusPush.status, "pushed", JSON.stringify(statusPush));
    assert.equal(remote!.state, "closed");
    assert.equal(remote!.state_reason, "completed", "Selected Status publication applies the provider's mapped state/reason");
    assert.equal(remote!.title, remoteTitle, "Unselected fields are not published");
    assert.equal(store.get("tasks", task.id).title, selectedStatus.title);
    assert.equal(creates, 1);
    assert.equal(updates, 1);
    const policy = store.config(project.id);
    repo.saveConfig(project.id, { ...policy, integration: { ...policy.integration!, pushFields: [] } });
    const beforeEmptyPolicy = requests.length;
    await assert.rejects(service.push(task.id, { revision: selectedStatus.revision, requestId: "fixture-empty-policy" }), { status: 409, code: "publish_fields_empty" });
    assert.equal(requests.length, beforeEmptyPolicy, "Empty policy rejects before transport and does not create a pending publication");
    const reopened = repo.updateTask(task.id, { status: "todo" });
    const explicitOverride = await service.push(task.id, { revision: reopened.revision, requestId: "fixture-explicit-fields", fields: ["status"] });
    assert.equal(explicitOverride.status, "pushed", JSON.stringify(explicitOverride));
    assert.equal(remote!.state, "open");
    assert.equal(creates, 1);
    assert.equal(updates, 2);
    observations.push({ id: "github", observed: "Injected gh transport exposes named states, reconciles exactly one uncertain create with the original request/policy while preserving newer local edits, and applies configured Status publication, empty-policy refusal and explicit-field override without publishing unselected fields.", details: { catalog, uncertain, editedDuringLoss, recovered, receipt, conflict, statusPush, explicitOverride, creates, updates, requests } });
  } finally { await service.close(); store.close(); }
}

async function projectCatalog(): Promise<void> {
  const store = new Store(":memory:"), repo = new Repository(store);
  const source = { id: "PVT_fixture", title: "Delivery project", url: "https://github.com/orgs/fixture/projects/1" };
  const item = { id: "PVTI_1", isArchived: false, project: source,
    content: { __typename: "Issue", id: "I_project", number: 2, title: "Review this change", body: "body", url: "https://github.com/owner/repo/issues/2", updatedAt: "2026-11-25T00:00:00Z", state: "OPEN", stateReason: null, repository: { nameWithOwner: "owner/repo" }, labels: connection([]) },
    fieldValues: connection([{ optionId: "review-option", name: "Review queue", field: { id: "field-flow", name: "Workflow state" } }]),
  };
  const requests: { kind: string; after?: string }[] = [];
  const service = new IntegrationService(repo, { env: {}, run: async (request) => {
    assert.equal(request.command, "gh");
    assert.ok(request.args!.includes("graphql"));
    const body = JSON.parse(request.input!) as { query: string; variables: { after?: string } };
    assert.ok(body.query.startsWith("query"), "Catalog and pull use read-only GraphQL operations");
    let node: unknown = source;
    if (body.query.includes("fields(first")) {
      requests.push({ kind: "fields", after: body.variables.after });
      const second = body.variables.after === "fields-2";
      node = { ...source, fields: connection(second
        ? [{ id: "field-release", name: "Release", options: [{ id: "release-beta", name: "Beta" }] }]
        : [{ id: "field-flow", name: "Workflow state", options: [{ id: "todo-option", name: "Todo" }, { id: "review-option", name: "Review queue" }] }], second ? null : "fields-2") };
    } else if (body.query.includes("items(first")) {
      requests.push({ kind: "items" });
      node = { ...source, items: connection([item]) };
    } else requests.push({ kind: "identity" });
    return { code: 0, stdout: JSON.stringify({ data: { node } }), stderr: "" };
  } });
  try {
    const project = repo.createProject({ name: "GitHub Projects catalog", kind: "github", externalRef: source.id });
    const integration = { provider: "github", scope: "project", statusFieldId: "field-flow", pushStatusMap: { in_review: "review-option" } };
    repo.saveConfig(project.id, { ...store.config(project.id), integration });
    const catalog = await service.catalog(project.id);
    assert.deepEqual(catalog.source, { id: source.id, name: source.title, url: source.url });
    assert.deepEqual(catalog.statusFields.map((field) => [field.id, field.name]), [["field-flow", "Workflow state"], ["field-release", "Release"]]);
    assert.equal(catalog.statusFields[0]!.options[0]!.suggestedStatus, "todo");
    const first = await service.sync(project.id);
    assert.equal(first.imported, 1, JSON.stringify(first));
    const task = store.list("tasks", { projectId: project.id })[0]!;
    assert.equal(task.status, "in_review", "Project readback inverts a unique explicit push mapping");
    repo.saveConfig(project.id, { ...store.config(project.id), integration: { ...integration, pullStatusMap: { "review-option": "blocked" } } });
    await service.sync(project.id);
    assert.equal(store.get("tasks", task.id).status, "blocked");
    const ambiguous = repo.createProject({ name: "Ambiguous explicit mapping", kind: "github", externalRef: source.id });
    repo.saveConfig(ambiguous.id, { ...store.config(ambiguous.id), integration: { ...integration, pushStatusMap: { in_review: "review-option", blocked: "review-option" } } });
    await service.sync(ambiguous.id);
    assert.equal(store.list("tasks", { projectId: ambiguous.id })[0]!.status, "todo", "An ambiguous custom mapping falls back to observed open state rather than guessing a mapped status");
    observations.push({ id: "project-catalog", observed: "Paginated named ProjectV2 fields/options preserve IDs, unique explicit push maps read back correctly, explicit pull maps win and ambiguous custom maps are not guessed.", details: { catalog, first, requests } });
  } finally { await service.close(); store.close(); }
}

async function refresh(): Promise<void> {
  const store = new Store(":memory:"), repo = new Repository(store);
  let requests = 0, active = 0, maxActive = 0;
  let gate: { entered: ReturnType<typeof Promise.withResolvers<void>>; release: ReturnType<typeof Promise.withResolvers<void>> } | undefined;
  let observedSignal: AbortSignal | undefined;
  const titles: Record<string, string> = { "team-A": "Remote A", "team-B": "Remote B" };
  const arm = () => { gate = { entered: Promise.withResolvers<void>(), release: Promise.withResolvers<void>() }; return gate; };
  const service = new IntegrationService(repo, { env: { KARABINER_TEST_LINEAR: "synthetic-fixture-value" }, fetch: (async (_url, options) => {
    const body = JSON.parse(String(options?.body)) as { query: string; variables: { key?: string; filter?: { team: { id: { eq: string } } } } };
    requests++; active++; maxActive = Math.max(maxActive, active);
    try {
      const held = gate;
      if (held) {
        gate = undefined;
        observedSignal = options!.signal!;
        held.entered.resolve();
        let rejectAbort: (() => void) | undefined;
        try {
          await Promise.race([held.release.promise, new Promise<never>((_resolve, reject) => {
            rejectAbort = () => reject(options!.signal!.reason);
            options!.signal!.addEventListener("abort", rejectAbort, { once: true });
            if (options!.signal!.aborted) rejectAbort();
          })]);
        } finally { if (rejectAbort) options!.signal!.removeEventListener("abort", rejectAbort); }
      }
      if (body.query.includes("teams(filter")) return Response.json({ data: { teams: { nodes: [{ id: `team-${body.variables.key}` }] } } });
      assert.ok(body.query.includes("issues(filter"));
      const team = body.variables.filter!.team.id.eq;
      const issue = { id: `issue-${team}`, identifier: `${team}-1`, title: titles[team], description: "body", url: `https://linear.app/fixture/issue/${team}-1`, updatedAt: "2026-11-25T00:00:00Z", archivedAt: null, priority: 3, dueDate: null, team: { id: team }, project: null, state: { id: "todo", name: "Todo", type: "unstarted" }, labels: connection([]) };
      return Response.json({ data: { issues: connection([issue]) } });
    } finally { active--; }
  }) as typeof fetch });
  try {
    const make = (key: string, enabled: boolean) => {
      const project = repo.createProject({ name: `Refresh ${key}`, kind: "linear", externalRef: key });
      repo.saveConfig(project.id, { ...store.config(project.id), integration: { provider: "linear", scope: "team", apiKeyEnv: "KARABINER_TEST_LINEAR", refresh: { enabled, intervalMs: 30_000 } } });
      return project;
    };
    const project = make("A", false), now = Date.now();
    service.start(); service.start();
    await service.refreshDue(now);
    assert.equal(requests, 0, "Automatic pull is off by default/configuration");
    const config = store.config(project.id);
    repo.saveConfig(project.id, { ...config, integration: { ...config.integration!, refresh: { enabled: true, intervalMs: 30_000 } } });
    const manualGate = arm(), manual = service.sync(project.id);
    await bounded(manualGate.entered.promise, "manual pull");
    const duringManual = requests;
    await service.refreshDue(now);
    assert.equal(requests, duringManual, "Automatic refresh skips a manually busy project");
    manualGate.release.resolve(); await manual;
    await service.refreshDue(now);
    const first = await service.status(project.id), afterFirst = requests;
    assert.equal(first.refresh!.lastAttemptAt, new Date(now).toISOString());
    assert.equal(first.refresh!.nextSyncAt, new Date(now + 30_000).toISOString());
    assert.ok(first.refresh!.lastSyncedAt);
    const task = store.list("tasks", { projectId: project.id })[0]!;
    repo.updateTask(task.id, { title: "Local A retained" });
    titles["team-A"] = "Remote A changed";
    await service.refreshDue(now + 29_999);
    assert.equal(requests, afterFirst);
    await service.refreshDue(now + 30_000);
    assert.equal(store.get("tasks", task.id).title, "Local A retained");
    assert.match((await service.status(project.id)).refresh!.error!, /local title/);
    const other = make("B", true);
    maxActive = 0;
    const autoGate = arm(), automatic = service.refreshDue(now + 60_000);
    await bounded(autoGate.entered.promise, "automatic pull");
    const beforeConcurrent = requests, concurrent = service.refreshDue(now + 60_000);
    assert.equal(requests, beforeConcurrent, "A concurrent automatic tick does not start another provider operation");
    autoGate.release.resolve(); await Promise.all([automatic, concurrent]);
    await service.refreshDue(now + 60_000);
    assert.equal(maxActive, 1);
    assert.equal(store.list("tasks", { projectId: other.id }).length, 1);
    const closingGate = arm(), closing = service.refreshDue(now + 90_000);
    await bounded(closingGate.entered.promise, "closing pull");
    await service.close(); await closing;
    assert.equal(observedSignal!.aborted, true);
    assert.equal(active, 0);
    const stoppedAt = requests;
    await service.refreshDue(now + 120_000);
    assert.equal(requests, stoppedAt);
    observations.push({ id: "automatic-pull", observed: "Deterministic refreshDue proves opt-in/interval behavior, preserve-local pulls, manual exclusion, one automatic operation and abort/drain on close without publication or real-time interval waits.", details: { first, requests, maxActive, stoppedAt } });
  } finally { gate?.release.resolve(); await service.close(); store.close(); }
}

async function readiness(): Promise<void> {
  const { directory, log, binary } = await targetFixture("readiness");
  const mxc = await targetReadiness({ target: "mxc", projectRoot: directory, executable: process.execPath, purpose: "command", check: true, targetOptions: { executable: binary, network: false } });
  assert.equal(mxc.available, true, JSON.stringify(mxc));
  assert.ok(mxc.checks.some((check) => check.code === "policy_valid" && check.status === "passed"));
  const mxcEvents = await events(log);
  assert.deepEqual(mxcEvents.map((event) => [event.kind, event.mode, event.configPath]), [["mxc", "dry-run", "/dev/fd/3"]]);
  const options = { executable: binary, image: "fixture:local", memory: "1.5G", cpus: 1 };
  const apple = await targetReadiness({ target: "apple-container", projectRoot: directory, purpose: "command", check: true, targetOptions: options });
  assert.equal(apple.available, true, JSON.stringify(apple));
  const probes = (await events(log)).filter((event) => event.kind === "apple").map((event) => event.args!);
  assert.deepEqual(probes, [["system", "status", "--format", "json"], ["image", "inspect", "fixture:local"]]);
  const missingEnv = `KARABINER_FIXTURE_MISSING_${crypto.randomUUID().replaceAll("-", "_")}`;
  const invalid = await targetReadiness({ target: "apple-container", projectRoot: directory, targetOptions: { ...options, cpus: cpus().length + 1, memory: "199M", envNames: [missingEnv], readonlyPaths: [join(temporary, "absent-mount")] } });
  assert.equal(invalid.available, false);
  for (const path of ["targetOptions.cpus", "targetOptions.memory", "targetOptions.envNames", "targetOptions.readonlyPaths"]) assert.ok(invalid.checks.some((check) => check.path === path && check.status === "failed"), path);
  assert.ok(invalid.checks.some((check) => check.code === "credential_env_missing"));
  assert.ok(invalid.checks.some((check) => check.code === "mount_source_missing"));
  const alias = join(temporary, "readiness-alias");
  await symlink(directory, alias);
  const launch = await prepareLaunch({ target: "apple-container", projectRoot: alias, cwd: join(alias, "subdir"), executable: "/bin/sh", args: ["fixture-fail"], targetOptions: { ...options, writablePaths: [join(directory, "subdir"), join(alias, "subdir")], readonlyPaths: [join(alias, "subdir")] } });
  try {
    const volumes = launch.args.flatMap((arg, index) => arg === "--volume" ? [launch.args[index + 1]!] : []);
    assert.deepEqual(volumes, [`${await realpath(directory)}:/workspace`, `${await realpath(join(directory, "subdir"))}:/workspace/subdir:ro`]);
    assert.equal(launch.args[launch.args.indexOf("--workdir") + 1], "/workspace/subdir");
  } finally { await launch.cleanup?.(); }
  observations.push({ id: "target-readiness", observed: "Readiness consumes the actual MXC config path in dry-run mode, probes only Apple service/image state, accepts valid decimal memory, reports precise resource/env/path failures and preserves canonical symlink-root mount/cwd rules.", details: { mxc, apple, invalid, launch: launch.args, probes } });
}

async function targets(): Promise<void> {
  const { directory, log, binary, script } = await targetFixture("project");
  const args = [script, "--workload", "space argument", "quote'argument"];
  const host = await prepareLaunch({ target: "host", projectRoot: directory, cwd: "subdir", executable: process.execPath, args, env: { FIXTURE_VALUE: "synthetic" } });
  const hostResult = JSON.parse((await runPrepared({ ...host, input: "host input" })).stdout);
  assert.equal(hostResult.cwd, await realpath(join(directory, "subdir")));
  assert.equal(hostResult.hasPath, true);
  assert.equal(hostResult.value, "synthetic");
  const mxc = await prepareLaunch({ target: "mxc", projectRoot: directory, cwd: "subdir", executable: process.execPath, args, env: { FIXTURE_VALUE: "synthetic" }, targetOptions: { executable: binary } });
  assert.deepEqual(mxc.args, ["--config", "/dev/fd/3"]);
  const mxcResult = JSON.parse((await runPrepared({ ...mxc, input: "separate harness stdin" })).stdout);
  assert.deepEqual(mxcResult.args, ["space argument", "quote'argument"]);
  assert.equal(mxcResult.input, "separate harness stdin");
  assert.equal(mxcResult.cwd, hostResult.cwd);
  assert.equal(mxcResult.value, "synthetic");
  const appleInput = { target: "apple-container" as const, projectRoot: directory, cwd: "/workspace/subdir", executable: "/bin/sh", targetOptions: { executable: binary, image: "fixture:local", network: false } };
  const apple = await prepareLaunch({ ...appleInput, args: ["fixture-fail"] });
  const failed = await runPrepared({ ...apple });
  assert.equal(failed.code, 7);
  assert.equal(apple.args[apple.args.indexOf("--volume") + 1], `${await realpath(directory)}:/workspace`);
  assert.equal(apple.args[apple.args.indexOf("--workdir") + 1], "/workspace/subdir");
  assert.equal(apple.args[apple.args.indexOf("--network") + 1], "none");
  const controller = new AbortController();
  const holding = await prepareLaunch({ ...appleInput, args: ["fixture-hold"], signal: controller.signal });
  const work = runPrepared({ ...holding, signal: controller.signal });
  try {
    const deadline = Date.now() + 5_000;
    while (!await Bun.file(log + ".ready").exists() && Date.now() < deadline) await Bun.sleep(25);
    assert.ok(await Bun.file(log + ".ready").exists());
    controller.abort(new Error("Fixture cancellation"));
    await assert.rejects(work, /Fixture cancellation/);
  } finally {
    if (!controller.signal.aborted) controller.abort(new Error("Fixture cleanup"));
    await Promise.allSettled([work]);
  }
  const parallel = await Promise.all([0, 1].map(() => prepareLaunch({ ...appleInput, args: ["fixture-fail"] })));
  const parallelResults = await Promise.all(parallel.map((launch) => runPrepared(launch)));
  assert.ok(parallelResults.every((result) => result.code === 7));
  const targetEvents = await events(log);
  const appleEvents = targetEvents.filter((entry) => entry.kind === "apple").map((entry) => entry.args!);
  const names = new Set<string>();
  for (const launch of [apple, holding, ...parallel]) {
    const name = launch.args[launch.args.indexOf("--name") + 1]!;
    names.add(name);
    assert.match(name, /^karabiner-[a-f\d-]{36}$/);
    assert.ok(appleEvents.some((args) => args[0] === "stop" && args.at(-1) === name));
    assert.ok(appleEvents.some((args) => args[0] === "delete" && args.at(-1) === name));
  }
  assert.equal(names.size, 4, "Concurrent launches retain separate owned cleanup identities");
  await assert.rejects(prepareLaunch({ target: "host", projectRoot: directory, executable: join(temporary, "missing"), args: [] }), /prerequisite missing/);
  await assert.rejects(prepareLaunch({ ...appleInput, purpose: "harness", args: [] }), /guest authentication/);
  observations.push({ id: "targets", observed: "Real local processes exercise host context, MXC config-path reopening with separate stdin/quoted argv, and separate Apple cleanup identities across failure, cancellation and concurrent launches; actual containment remains unverified.", details: { host: hostResult, mxc: mxcResult, apple: apple.args, cancelled: holding.args, parallel: parallel.map((launch) => launch.args), events: targetEvents } });
}

async function githubReadonly(): Promise<void> {
  const repository = process.env.KARABINER_GITHUB_READONLY_REPOSITORY;
  if (!repository) {
    observations.push({ id: "github-readonly", status: "inconclusive", observed: "Set KARABINER_GITHUB_READONLY_REPOSITORY to the explicitly approved repository before the live read-only check.", details: { externalWrites: false } });
    return;
  }
  const store = new Store(":memory:"), repo = new Repository(store);
  const service = new IntegrationService(repo);
  try {
    const project = repo.createProject({ name: "Approved GitHub read-only check", kind: "github", repository });
    repo.saveConfig(project.id, { ...store.config(project.id), integration: { provider: "github", scope: "repository", pushEnabled: false } });
    const status = await service.status(project.id, true);
    if (status.status !== "ready") {
      assert.notEqual(status.code, "invalid_response", "The live provider returned a response the implementation could not interpret");
      observations.push({ id: "github-readonly", status: "inconclusive", observed: "Live GitHub read access was unavailable; this is not a passed authentication/catalog observation.", details: { repository, status, externalWrites: false } });
      return;
    }
    const catalog = await service.catalog(project.id);
    assert.equal(status.authenticated, true);
    assert.equal(catalog.source.name.toLowerCase(), repository.toLowerCase());
    assert.equal(catalog.statusFields[0]!.id, "issue-state");
    observations.push({ id: "github-readonly", observed: "The actual IntegrationService used existing gh authentication to read the approved repository and its named catalog; only user/repository metadata was requested, with no remote writes or task import.", details: { repository, status, catalog, externalWrites: false } });
  } finally { await service.close(); store.close(); }
}

try {
  const failures: string[] = [];
  const scenarios: [string, () => Promise<void>][] = liveGithub ? [["github-readonly", githubReadonly]] : [["linear", linear], ["github", github], ["project-catalog", projectCatalog], ["automatic-pull", refresh], ["target-readiness", readiness], ["targets", targets]];
  for (const [name, scenario] of scenarios) {
    try { await scenario(); }
    catch (error) { failures.push(`${name}: ${error instanceof Error ? error.stack ?? error.message : String(error)}`); }
  }
  if (failures.length) { failure = failures.join("\n\n"); process.exitCode = 1; }
  else if (observations.some((item) => item.status === "inconclusive")) process.exitCode = 2;
}
finally {
  const results: { id: string; status: "passed" | "failed" | "inconclusive"; observed: string; evidence: string[] }[] = observations.map((item) => ({ id: item.id, status: item.status ?? "passed", observed: item.observed, evidence: [artifact] }));
  if (failure) results.push({ id: "failure", status: "failed", observed: failure, evidence: [artifact] });
  const context = liveGithub ? { fixture: "isolated-live-github-readonly", externalWrites: false, liveNetwork: true } : { fixture: "injected-connectors-and-targets", externalWrites: false, actualContainment: false };
  await Bun.write(join(evidence, artifact), JSON.stringify({ fixture: context.fixture, observations, failure: failure ?? null }, null, 2));
  await Bun.write(join(evidence, "result.json"), JSON.stringify({ observations: results, context }, null, 2));
  await rm(temporary, { recursive: true, force: true });
}
if (failure) console.error(failure); else console.log(`${liveGithub ? "GitHub read-only" : "Factory integrations"} ${process.exitCode === 2 ? "inconclusive" : "passed"}: ${observations.length} scenarios; evidence ${join(evidence, artifact)}`);
