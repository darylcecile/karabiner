# Local service and update boundary

Status: accepted, 2026-11-24, from the user's service-backed-storage choice and parent implementation direction.

Bun owns SQLite and exposes loopback HTTP JSON `/v1` to the independently built Rust/GPUI client and Bun CLI. Product versions and database schema versions are separate from API major compatibility. API contracts live in `packages/contracts/src/index.ts`; Rust models mirror that boundary. Successful storage is real; scheduler, harness, integration and VM execution capabilities remain explicitly unavailable. Interventions record intent without claiming execution.

This keeps user-editable backend behavior in TypeScript while supporting native UI and agent-friendly CLI updates independently. The service offers small SSE invalidations (`/v1/events`) for clients that want them; the native client currently refetches after its own writes, on ⌘R and on window focus. There is no event-sourcing framework. Revisioned writes prevent lost document/config/tab edits. Project/task relationships use SQLite foreign keys, while structured configuration and authored artifact content use validated JSON.

Initial research considered Unix sockets. The implementation uses the explicitly approved loopback HTTP boundary for the native adapter. Review `/v1/meta` and capabilities before optional operations; never infer execution support from a stored task or intent. See [service guide](../guides/service.md).
