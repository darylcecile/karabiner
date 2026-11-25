# Service, contracts and CLI

The implemented boundary is `packages/contracts/src/index.ts`: API major 1, camelCase JSON, `{data: ...}` success and `{error:{code,message}}` failure. The default listener is `http://127.0.0.1:43173`. Client/service/CLI product versions are independent; clients check `/v1/meta` and capabilities. The service owns SQLite; clients never access it directly.

## Storage and entry points

`packages/service/src/main.ts` owns explicit foreground startup and graceful signals. `server.ts` routes HTTP, `repository.ts` owns CRUD/validation relationships and activity, `database.ts` owns SQLite tables/foreign keys/atomic writes, and `validation.ts` validates payload shapes. Tables have stable IDs and relational project/task columns, with JSON for revisioned entity/config/content fields. Initial schema version is 1; unknown schemas fail clearly. No migration framework is installed.

- Projects, tasks, documents, project configuration and namespaced settings persist across process restarts. IDs are UUID strings.
- PATCH updates only supplied fields. Optional `revision` detects stale edits. Config/settings/workspace PUT requires the revision last read.
- Project deletion cascades project-owned records. Task deletion detaches authored documents/interventions rather than destroying them. Missing saved tab targets remain client-visible unavailable context.
- Stored task statuses are explicit user/CLI edits. They do not represent an agent run. Workflow/stage/agent assignments are checked against the project's configuration. Tasks retain due dates, numeric sort order and typed subtasks. Stages can declare exit rules, mapped status and color. Custom executable/argument agent configuration is stored without invoking it. `POST /v1/tasks/:id/comments` records operator activity separately from intervention intent.
- Interventions record real intent/questions/answers but explicitly report `executionStatus: not_implemented`. Scheduler, harness, integration and VM capabilities are false. Statistics derive manual task status counts; runtime metrics are null.

`execution.ts` exposes the small user-owned `RunReader` boundary. The default returns no records; `/v1/runs/:id` returns a structured 404. Contracts include optional stage/step trackers, outputs, timings, tokens/cost and trace spans so a future provider can supply observations. No scheduler, run mutation system or synthetic active run is installed.

`packages/cli/src/main.ts` parses friendly commands and JSON file input. Resource/action flag lists reject irrelevant or immutable options; `--data` JSON remains additive. Statistics support project scope only: CLI `--task` is a usage error and API `taskId` is a 400. `client.ts` checks API compatibility, issues requests once, and preserves structured service errors. CLI defaults to the same loopback URL and supports `--url`/`KARABINER_SERVICE_URL`. `demo seed` explicitly creates labelled sample content; ordinary startup creates no sample records. See root README and `bun run cli --help`.

## Workspace and change notifications

`GET/PUT /v1/workspace` persists ordered project/task/document/review tabs, active tab ID, selected project and up to 50 recently closed tabs. Keyboard behavior and in-tab navigation belong to the native client; storage verifies the target structure and active-tab membership.

`GET /v1/events` is a small SSE invalidation stream. A connection receives `event: ready` with `{"type":"ready"}`; successful mutations emit `event: change` with `{"type":"changed","path":"/v1/..."}`. Refetch snapshots on ready/reconnect, relevant changes, and focus. There is no durable event history or replay cursor. Slow subscribers are disconnected and can reconnect/refetch. An SSE subscriber's active network reader must run off the UI thread. A client without SSE must explicitly refresh snapshots on focus (and after its own changes) to see CLI edits.

## Documents and code tours

Markdown documents retain rich Markdown text for the native editor. Code-tour documents store typed ordered chapters and Markdown/diagram/code/diff blocks, source revision/line references, commit records/file changes, commit groups and review progress. Config and tour updates preserve additive fields in matched ID-bearing objects. They do not invoke Git, generate commits, apply diffs or infer approval/merge results. Onboarding templates are static declarative config plus authored document starters; choosing one stores those records.

## Verification

`groundwork verify service-types service-roundtrip` type-checks and runs one isolated scripted journey. The driver compiles separate service/CLI binaries, starts only its own ephemeral-loopback service/temp database, uses real CLI commands for CRUD/settings/config/tours/interventions, observes SSE, restarts the service, verifies persistence and deletion relationships, and checks offline/API-major errors. It removes its own processes/data and retains `roundtrip.json` plus the structured result in the run evidence directory. This is backend/CLI proof, not native keyboard or rendering proof. No provider, VM, external integration or user database is exercised.
