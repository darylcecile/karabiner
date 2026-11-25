# Project context

Karabiner is a native Rust/GPUI desktop client (`apps/client`, `crates/protocol`) plus an independently versioned Bun service, CLI and contracts (`packages/`). The service-owned Bun SQLite store is the source of truth. Stored CRUD, configuration, documents, code tours and intervention intent are implemented. Scheduler, harness, integration and VM execution are user-owned extension points.

- Setup, run and build commands: root [README](../../README.md).
- Boundaries: [client/service decision](../decisions/client-service-boundary.md), [local service boundary](../decisions/local-service-boundary.md).
- Guides: [client interface](client-interface.md), [GPUI foundation](gpui-foundation.md), [service](service.md), [domain research](domain-contracts.md), visual design authority [visual contract](visual-contract.md) (measurements: [projects/tasks](visual-project-measurements.md), [docs/review/stats](visual-docs-measurements.md); [A](spatial-references-a.md) / [B](spatial-references-b.md) are research notes only).
- Checks: `groundwork verify service-types service-roundtrip client-native-smoke` (macOS; native smoke needs Screen Recording and Accessibility permission).

Reuse any running app, service and data. Never stop or replace them as a side effect. Verification starts only its own temporary service and database. No provider calls, VM launches, credential collection or external data changes are required.
