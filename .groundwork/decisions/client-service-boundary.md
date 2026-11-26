# Native client and service boundary

Status: accepted, 2026-11-24. Complements [local service boundary](local-service-boundary.md), which covers storage and the API.

**Decision.** The desktop app is a native Rust/GPUI client (`apps/client`, using `gpui-kit` 0.11.0 pinned with Rust 1.100). It never opens SQLite. It talks only to the separately built Bun service over versioned loopback HTTP (`/v1`, default `127.0.0.1:43173`). The Bun CLI uses the same API. `crates/protocol` hand-mirrors `packages/contracts`.

**Rationale.**
- A native GPUI UI gives desktop performance and motion.
- Backend behavior stays editable in TypeScript while the independently built native client consumes the shared API.
- The client, the service and the CLI ship and update independently.
- One authority, the SQLite store owned by the service, avoids sync between clients.

**Consequences.**
- The client shows connection/setup state while the service is unavailable. The [managed local service](managed-local-service.md) decision supersedes manual startup for packaged clients at the default local endpoint: reuse a compatible service, otherwise obtain and start the pinned matching service. Explicit `KARABINER_SERVICE_URL` and source builds without an embedded asset URL retain external/manual setup. Compatibility is decided by `GET /v1/meta` reporting `apiVersion == 1`; incompatible running services are not replaced.
- The client refetches after its own writes, on ⌘R, on focus and through advertised `/v1/events` invalidations. The 2026-11-25 execution extension adds a background SSE reader and 400ms burst coalescing while preserving dirty drafts and older-service fallback.
- Whole-object PUTs preserve unknown fields and enum values, and revision conflicts are surfaced to the user rather than overwritten.
- Run/trace/token/cost views show recorded values and retain an unavailable state for unobserved metrics. Runtime controls use addressed service operations.

**Historical execution scope.** The initial 2026-11-24 implementation reserved execution as an extension point and recorded intervention intent only. [Factory execution boundary](factory-execution-boundary.md), accepted 2026-11-25 after the complete-service request, supersedes that execution deferral. [Native factory canvas](native-factory-canvas.md) extends the client with the requested React Flow-derived GPUI graph surface; its final visual evidence remains a separate acceptance obligation.

Details: [client interface](../guides/client-interface.md), [GPUI foundation](../guides/gpui-foundation.md), [service](../guides/service.md), [domain research](../guides/domain-contracts.md).
