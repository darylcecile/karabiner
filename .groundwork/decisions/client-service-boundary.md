# Native client and service boundary

Status: accepted, 2026-11-24. Complements [local service boundary](local-service-boundary.md), which covers storage and the API.

**Decision.** The desktop app is a native Rust/GPUI client (`apps/client`, using `gpui-kit` 0.11.0 pinned with Rust 1.100). It never opens SQLite. It talks only to the separately built Bun service over versioned loopback HTTP (`/v1`, default `127.0.0.1:43173`). The Bun CLI uses the same API. `crates/protocol` hand-mirrors `packages/contracts`.

**Rationale.**
- A native GPUI UI gives desktop performance and motion.
- Backend behaviour stays in TypeScript, where the user will add scheduling, harnesses, integrations and VM execution.
- The client, the service and the CLI ship and update independently.
- One authority, the SQLite store owned by the service, avoids sync between clients.

**Consequences.**
- The client runs without the service and shows unavailable/incompatible states. Compatibility is decided only by `GET /v1/meta` reporting `apiVersion == 1`.
- The client refetches after its own writes, on ⌘R and on window focus. It does not subscribe to `/v1/events`, even though the service offers it.
- Whole-object PUTs preserve unknown fields and enum values, and revision conflicts are surfaced to the user rather than overwritten.
- Execution is an extension point. Run, trace, token and cost views render "not available" until a provider supplies records, and intervention UI records intent only.

Details: [client interface](../guides/client-interface.md), [GPUI foundation](../guides/gpui-foundation.md), [service](../guides/service.md), [domain research](../guides/domain-contracts.md).
