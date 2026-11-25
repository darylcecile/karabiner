# Native permission requests and active-session guidance

Status: accepted and implemented, 2026-11-25. Controlled SDK/protocol, native GUI and one actual Copilot permission/tool/guidance path are accepted with the precise scopes in the [capability matrix](../guides/factory-capabilities.md). Other providers' live-account exercise remains untested, not missing bridge implementation.

Supersedes the native-permission limitation and queued-only guidance boundary in [factory execution boundary](factory-execution-boundary.md). Those constraints remain the history of the earlier implementation, not the current contract.

## Decision

Expose supported local harness sessions through small adapter interaction/control seams: correlated question/permission requests, capability-declared steering mode, session identity, abort and delivery acknowledgement. The service persists their run/branch/attempt context; native UI and CLI answer the same addressed interaction. A live response settles the existing invocation rather than dispatching another one.

Guidance reaches the active session at its supported native turn/tool boundary. The returned delivery mode must be truthful; a next-step fallback remains visibly deferred rather than being claimed as live steering. Preserve the configured local account and permission policy. Do not solve a missing bridge with blanket allow flags, synthetic approval or credential copying.

Confirmed work and cleanup are separate facts. `AdapterCleanupError` is nonretryable and carries the actual completed outcome, usage/session and phase-labelled cleanup failures. Runtime attempt history preserves those facts instead of automatically replaying already successful side effects because session teardown failed.

## Copilot release compatibility

Use the official released `@microsoft/copilot-sdk` **2.0.3** for the installed Copilot CLI **1.0.116**. The published package declares that CLI version and SDK protocol **4**, minimum **4**. Its client setup uses `options.runtime` with `RuntimeConnection.forSpawnedStdio` and `github.useLoggedInUser`. This replaces the older `@github/copilot-sdk` 1.0.34 dependency, whose maximum protocol is 3 and cannot negotiate the installed protocol-4 CLI.

This is an implementation compatibility correction, not a missing-account/manual-setup condition. Use the matching official release rather than a dependency patch, protocol override, installed-CLI change or older-runtime fallback. The earlier failed live attempt stopped at negotiation before inference and remains preserved. The matched-release path has scoped targeted/readiness/proxy proof and one successful actual owned-workspace permission/tool/guidance run; other SDK/core/native proofs retain their original scope. Compiled-service readiness also exercises a controlled protocol-3 peer: it reports `protocol_mismatch` with authentication unknown and creates no session/model, rather than misclassifying compatibility as a login failure.

## Reason and consequences

The existing structured outcomes and approval gates cannot answer a provider's native tool permission request. Likewise, storing guidance for a later invocation does not complete an active-session steering workflow. Explicit adapter capabilities and durable request IDs connect those real channels to the user without making the GUI a second harness.

The seam lives in `packages/service/src/runtime/adapter-types.ts` and public interaction/answer/delivery DTOs. Public responses accept choice, text, string/multiselect answers and provider JSON value. Runtime validation normalizes typed fields before persistence/resolution; invalid replies leave the request pending, while a denial discards unsubmitted form values. Native requests, controls and receipts retain attempt identity. Old custom JSONL v1 EOF behavior remains compatible; explicit `jsonrpc-v2` supplies the negotiated duplex permission/question/instruction/abort channel.

Native acceptance requires a real supported request displayed in the app, an allow/deny response affecting the actual fixture tool outcome, and active guidance reaching the same provider session with delivery provenance. Installed/authenticated live tools may be exercised only under the coordinator's approved non-mutating scope; missing accounts/binaries remain human setup blockers after the code paths are complete. No user login, live database or existing service is replaced by verification.
