# Named connected-project setup and explicit background pull

Status: accepted and implemented, 2026-11-25. Native setup and controlled connector/target acceptance are closed; actual GitHub READ is separately observed. Live Linear, provider writes and physical containment remain untested prerequisites, with exact scope in the [feature matrix](../guides/factory-capabilities.md).

Related: [factory execution boundary](factory-execution-boundary.md), [feature matrix](../guides/factory-capabilities.md), [contract handoff](../guides/factory-contracts.md#connected-catalogs-refresh-and-target-readiness).

## Decision

Expose explicit paginated integration catalogs with human-readable sources/teams/states/single-select fields. Native setup selects names and stores existing provider IDs/maps, rather than asking users to discover opaque IDs or edit JSON. Keep credential values outside project configuration.

Allow opt-in scheduled **pull**, default off, with a bounded configurable interval and preserve-local conflict behavior. One automatic operation runs at a time, manual per-project operations keep precedence, and close stops timers/aborts/drains requests. In-memory refresh observations are not durable event history; no schema migration or automatic push/publication is introduced. Task scheduling remains a separate configured service concern.

Provide structured target readiness for the actual configuration: local validation by default, explicit supported dry-run/service/image inspection when requested. Readiness never starts the workload or a VM and never equates host account availability with guest authentication. Existing launch context and uniquely owned cleanup semantics remain.

Explicit publication resolves `input.fields ?? config.pushFields ?? [title, description]` once per new operation. Empty policy is a valid configuration but refuses publication before transport; explicit fields override it. Pending retries retain their original body/key/values/fields across local edits or later policy changes, and reconciliation attaches only external metadata to the current task. New keys are refused while another push is uncertain. No automatic publication is introduced.

## Reason and acceptance

The earlier transport/config fields alone were insufficient for the requested primary GUI setup workflow. Named catalogs and factual checks make those capabilities operable from the client without changing the source-of-truth boundary. Shared `IntegrationCatalog` and `TargetReadiness` DTOs, draft/saved readiness and native named selections have scoped backend/GUI acceptance. Native forms now expose the actual configuration rather than requiring raw JSON.

Acceptance combines injected catalog/refresh/readiness behavior with actual native form/save/status flows and approved non-mutating live reads where credentials exist. Missing installed tools/accounts remain human setup blockers after those code paths are complete.
