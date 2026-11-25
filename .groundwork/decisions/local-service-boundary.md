# Local service and update boundary

Status: accepted, 2026-11-24, from the user's service-backed-storage choice and parent implementation direction. The initial execution deferral is superseded by [factory execution boundary](factory-execution-boundary.md), 2026-11-25; the storage/transport/update boundary remains accepted.

Bun owns SQLite and exposes loopback HTTP JSON `/v1` to the independently built Rust/GPUI client and Bun CLI. Product versions and database schema versions are separate from API major compatibility. API contracts live in `packages/contracts/src/index.ts`; Rust models mirror that boundary.

This keeps user-editable backend behavior in TypeScript while supporting native UI and agent-friendly CLI updates independently. The service offers small SSE invalidations (`/v1/events`); the native client now subscribes with a background reader/coalesced refetch and retains its own-write, ⌘R and focus refresh paths. There is no event-sourcing framework. Revisioned writes prevent lost document/config/tab edits. Project/task relationships use SQLite foreign keys, while structured configuration and authored artifact content use validated JSON.

Initial research considered Unix sockets. The implementation uses the explicitly approved loopback HTTP boundary for the native adapter. Review `/v1/meta` and capabilities before optional operations; never infer execution support from a stored task or intent. See [service guide](../guides/service.md).

## Initial execution scope (historical)

The 2026-11-24 implementation deliberately made storage real while leaving scheduler, harness, integration and VM capabilities unavailable; interventions recorded intent only. The user subsequently requested the complete factory service. [Factory execution boundary](factory-execution-boundary.md) supersedes that deferral with implemented scheduling, adapters, synchronization and addressed operator control. This record's SQLite/HTTP/independent-update decisions remain valid. Current operations and remaining environment limits are described in the service guide.
