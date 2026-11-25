# Service, contracts and CLI

The implemented boundary is `packages/contracts/src/index.ts`: API major 1, camelCase JSON, `{data: ...}` success and `{error:{code,message}}` failure. The default listener is `http://127.0.0.1:43173`. Client/service/CLI product versions are independent; clients check `/v1/meta` and capabilities. The service owns SQLite; clients never access it directly.

## Storage and entry points

`packages/service/src/main.ts` owns explicit foreground startup and graceful signals. `server.ts` routes HTTP, `repository.ts` owns CRUD/relationships/activity, `database.ts` owns SQLite tables/foreign keys/atomic writes, and `validation.ts` validates payloads. Tables use stable IDs and relational columns with JSON for revisioned structured content. Schema 3 adds execution/session and integration-link/receipt tables through service-owned transactional upgrades from versions 1/2; unknown schemas fail clearly.

- Projects, tasks, documents, project configuration and namespaced settings persist across process restarts. IDs are UUID strings.
- PATCH updates only supplied fields. Optional `revision` detects stale edits. Config/settings/workspace PUT requires the revision last read.
- Project deletion cascades project-owned records. Task deletion detaches authored documents/interventions rather than destroying them. Missing saved tab targets remain client-visible unavailable context.
- CRUD alone does not start a run. Workflow/stage/agent references are checked against project configuration; tasks retain due dates, ordering and typed subtasks. `task.agentId` is assignment/context only. Actual ordered execution comes from each stage's `agentIds`; `RunStep.agentId` identifies the executing agent. An optional `task.stageId` selects the captured start stage, otherwise graph entry. `POST /v1/tasks/:id/comments` records operator activity.
- Runs project actual execution state/current stage onto their task. Statistics combine stored counts with observed run outcomes, durations, stage/agent/tool spans and supported token usage. Missing metrics remain null; CLI subscription access does not establish API cost.
- Runtime questions/approvals have run/question identity and pending/answered/cancelled state. Legacy free-standing intervention records still retain intent with `executionStatus: not_implemented`; recording one is not runtime delivery. Use the addressed run controls/question endpoints for their implemented effects.

## Factories and durable execution

`packages/contracts/src/factory-types.ts`, `factory.ts` and `factory-validation.ts` define graph/config/run DTOs, pure compilation and validation. `factory-service.ts` resolves configured instruction/skill files before compilation or snapshot creation. `GET /v1/projects/:id/factory` compiles the saved config; POST `/factory/validate` or `/factory/compile` may supply `{config}`. Export returns a `karabiner-factory` version 1 bundle: editable `config.json` retains layout, `factory.json` is the normalized execution snapshot, and workflows/stages/agents/instructions/skills are JSON data modules. Import accepts `{revision,bundle}` and preserves revision conflicts. No executable TypeScript is evaluated.

Success/failure select one successor; explicit structured forks run disjoint branches and join once within durable nested scopes. Multiple return targets are disambiguated by `returnToStageId` within their permitted scope and visit/transition bounds. Zero/one-branch drafts can be stored, but executable validation requires complete forks. Existing serial workflows remain valid. Layout and config revision are independent of executable identity. Stage execution is before commands → ordered agents → stage commands → gates → after commands. Bounded stage/command retries retain attempts and delay policy; denial, cancellation and shutdown are not automatically retried. Manual linked run retry remains distinct.

`execution.ts` exposes `createRuntime`; `runtime/scheduler.ts`, `runtime/pipeline.ts` and `run-store.ts` own dispatch, execution and durable state. `POST /v1/tasks/:id/run` captures project/task/config/factory before queuing. List/detail use `/v1/runs?projectId=&taskId=&state=` and `/v1/runs/:id`. Explicit scheduler configuration selects statuses/workflow and per-project scan/concurrency limits; its pickup ledger prevents repeated execution until an intentional status reset/retry. Active work reserves its project workspace, including paused/waiting checkpoints.

POST `/v1/runs/:id/pause|resume|cancel|retry` accepts optional `{revision}`. Pause settles at a step boundary; cancellation terminates and settles the owned process tree. Retry creates a new run with `retryOf` and the prior captured factory. Waiting questions survive service restart as safe checkpoints. A service interruption during active execution records an interrupted paused run requiring explicit inspection/retry/cancel rather than replaying uncertain effects.

POST `/v1/runs/:id/instructions` accepts `{message,revision?}`; CLI uses `run instruct RUN TEXT`. Active adapters receive guidance at their supported native-turn or tool boundary; delivery records name the actual attempt and mode. Submission stays nonblocking during an outstanding permission request. A next-step fallback is explicitly deferred, not claimed as live steering. Guidance remains cumulative for later invocations and retries with separate delivery provenance.

POST `/v1/runs/:id/questions/:questionId/answer` accepts `{choice?,answer?,answers?,value?,revision?}`. Choice IDs come from the actual request; fields are normalized/validated before persistence or resolution, including integers, numbers, booleans and multiselects. Invalid replies return 400 and remain pending. Live answers settle the current invocation, not a duplicate dispatch; checkpoint answers remain attempt-scoped. Approval gates retain approve/approved/yes/y semantics. Pending requests remain visible while other branches continue running.

## Local harnesses and optional targets

Claude and Copilot use their published SDK/native session channels, and Codex uses its app-server protocol, retaining installed executables and existing accounts. Copilot uses official `@microsoft/copilot-sdk` 2.0.3 for CLI 1.0.116/protocol 4 (`options.runtime` / `RuntimeConnection.forSpawnedStdio`, `github.useLoggedInUser`). Native tool permissions, questions, steering and cancellation are modeled explicitly; there is no terminal-prompt scraping. PATH discovery is not authentication/model-access proof. Existing user permission policy remains authoritative.

POST `/v1/harnesses/check` accepts `{agent,check?,projectRoot?}`. Default checks observe configuration/local availability; explicit checks use bounded authentication status APIs without creating a model session. Custom executables are never run by readiness. CLI uses `harnesses check --file agent.json|- [--project-root PATH] [--check]` or saved `--project PROJECT --agent AGENT`; relative executables resolve against that actual root. Unknown-only diagnostics do not imply failure or authentication. Target draft checks use `/v1/projects/:id/execution-target/check` with optional config/root and non-mutating runtime/image/policy probes.

Copilot protocol incompatibility is reported as `protocol_mismatch` / failed protocol compatibility with `authenticated:null`, not as a missing login. Readiness does not open an inference session to determine that result.

The opt-in Strands adapter uses installed harness 0.5.0 / SDK 1.26.0 `createHarness`, real file/shell/skill tools, cancellation/shutdown and service-owned SQLite session snapshots. Its provider/model identifiers and credential path are distinct from local CLI model IDs; there is no Strands Copilot alias. The service owns the factory graph scheduler.

Harness and target selection are independent. Host execution inherits the local environment with explicit overlays. MXC is macOS Seatbelt process isolation; its config travels through inherited fd 3 while stdin remains available for the harness. Apple Container uses an explicitly prepared Linux image, a fixed project-root mount with separate guest working directory and uniquely owned cleanup. Host macOS binaries/keychain do not supply guest harness authentication. Missing target prerequisites produce errors rather than falling back to host execution.

## Custom local harness protocol

For compatible `protocol: "jsonl-v1"` (the default), configure `harness: "custom"`, `executable` and `args`. The service sends one JSONL record and closes stdin:

```json
{"protocol":"karabiner-harness-v1","runId":"RUN","stepId":"STEP","agent":{"id":"builder"},"instructions":"resolved project/stage/agent instructions and skills","prompt":"serialized task/project/stage context","responseSchema":{}}
```

`agent` is the full configured definition; `prompt` and `responseSchema` contain the actual service context/schema. The `instructions` string includes resolved factory/agent instructions, skills and captured cumulative operator guidance for this invocation. Optional fields are `sessionId` and, when resuming human input, `answer: {"requestId":"QUESTION","response":"the human answer"}`. The service starts the same step again with that answer and its recorded session ID.

Emit newline-delimited JSON on stdout, then exit 0 on a valid step result:

```json
{"type":"text","text":"Progress from actual work\n"}
{"type":"result","session_id":"YOUR_SESSION","result":{"outcome":"complete","summary":"What actually changed"}}
```

Other results use `outcome: "return"` with `returnToStageId`, `outcome: "blocked"` with a concrete summary, or `outcome: "question"` with `question`. An interactive agent may alternatively emit `{"type":"question","question":"What should happen next?"}` and exit 0. `{"type":"error","message":"reason"}` fails the invocation. Optional observed usage on events is `{ "usage": { "input_tokens": 7, "output_tokens": 3 } }`; never fabricate it. Native permission/control-input events unsupported by this protocol fail explicitly. Plain process success without a valid outcome is not task completion.

For live controls, explicitly select `protocol: "jsonrpc-v2"`. The service negotiates `initialize` with protocol `karabiner-harness-v2`, run/step/attempt identity and optional session ID; the peer returns that protocol, session ID and capabilities. A duplex `run` may issue correlated `permission`/`question` requests with choices, normalized tool details, typed fields and payload. Permission choices declare offered IDs/labels and approve/deny/cancel effects. `instruction` replies acknowledge actual delivery; `abort` and stream closure settle owned work. This channel keeps stdin open and is distinct from v1 EOF/checkpoint behavior. See [contract details](factory-contracts.md) for exact consumer DTOs.

## Project source synchronization

`integrations/service.ts` supplies named paginated catalogs, explicit pull/push and optional pull-only refresh through GitHub CLI/Linear GraphQL. Construction/cached status do not perform remote I/O. Refresh defaults off, preserves local edits and respects manual exclusion; deletion tombstones remain. Native catalog selections store IDs behind human labels. Publication resolves explicit request fields, then project pushFields, then title/description, freezing the selection in its durable request. Empty policy refuses new publication. Original-key retry reconciles uncertainty without restoring stale local edits; another key cannot bypass a pending push. Configuration retains references, not credential values.

`packages/cli/src/main.ts` and resource modules parse friendly commands and JSON input. Factory compile/export/import, run start/list/show/control, and question list/answer use the same API. Flag validation rejects irrelevant or immutable options; `--data` remains additive. Statistics support project scope only. `client.ts` checks API compatibility, submits each mutation once and preserves structured service replies/errors. CLI defaults to the same loopback URL and supports `--url`/`KARABINER_SERVICE_URL`. `demo seed` explicitly creates labelled samples; ordinary startup creates none. See root README and `bun run cli --help`.

## Workspace and change notifications

`GET/PUT /v1/workspace` persists ordered project/task/document/review tabs, active tab ID, selected project and up to 50 recently closed tabs. Keyboard behavior and in-tab navigation belong to the native client; storage verifies the target structure and active-tab membership.

`GET /v1/events` is a small SSE invalidation stream. A connection receives `event: ready` with `{"type":"ready"}`; successful mutations emit `event: change` with `{"type":"changed","path":"/v1/..."}`. Refetch snapshots on ready/reconnect, relevant changes, and focus. There is no durable event history or replay cursor. Slow subscribers are disconnected and can reconnect/refetch. An SSE subscriber's active network reader must run off the UI thread. A client without SSE must explicitly refresh snapshots on focus (and after its own changes) to see CLI edits.

The native `ServiceState` now subscribes when metadata advertises events, using a background network reader and a 400ms coalesced native data-version bump. It reconnects with bounded backoff and retains manual/focus refresh. Feature views continue to protect dirty drafts during invalidation/refetch.

## Documents and code tours

Markdown and typed code tours retain authored content, source references, chapters, diffs, commit groups and review progress. Additive fields survive updates. Runtime artifact generation writes a run summary and, for a Git working tree with observed changes, a source-provenanced code tour relative to the run's captured baseline, including pre-existing dirty content in that baseline. Reading/reviewing artifacts does not apply diffs or infer a merge. Onboarding templates remain editable declarative configuration/document starters.

## Verification

`service-roundtrip` preserves the compiled storage/CLI journey. `factory-runtime` adds real local process and scripted Strands execution; `factory-integrations` exercises injected connectors and fake target executables. They own only temporary services/databases/repositories and retain evidence in the Groundwork run directory. See [factory-runtime journey](factory-runtime.md) for exact assertions and live-provider/containment/native gaps. Configured checks describe intended proof; only inspected run evidence establishes a result.
