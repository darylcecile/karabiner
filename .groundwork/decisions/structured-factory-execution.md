# Structured fork/join and bounded attempt history

Status: accepted and implemented, 2026-11-25. Scoped backend/CLI execution, native authoring and branch/attempt history acceptance are closed in the [capability matrix](../guides/factory-capabilities.md); retained evidence remains distinct from untested live-provider/containment prerequisites.

Supersedes the serial-only execution and retry-zero constraints in [factory execution boundary](factory-execution-boundary.md). That record retains the previously implemented bounded feature set as history.

## Decision

Extend the existing service scheduler/compiler with explicit structured forks, disjoint branch entries and a join-all boundary. Preserve nested execution scopes, durable branch tokens and unique stage-invocation/attempt IDs so a join executes once and each observed result retains provenance. Existing serial/conditional/return graphs remain valid. The shared `WorkflowFork`, `RunToken`, `RunScope` and `RunStepAttempt` types in `packages/contracts/src/factory-types.ts` define the implemented seam.

Retry is explicit, bounded configuration on stages/commands, with delays and actual attempt history. It is distinct from manual whole-run retry and does not erase failed output, timing or usage. Denial, cancellation and service shutdown are excluded from automatic retry. Questions, session/storage state and delivery receipts are attempt-scoped; a repeated provider request ID cannot replay an earlier attempt's answer. Interrupted active work retains inspectable scope/token identity and requires explicit recovery rather than inferred reattachment.

## Reason and consequences

The user needs configurable parallel project work and retry history, not multiple drawn edges that still execute serially. Keeping execution in the current service avoids a second graph scheduler. Structured branches/join boundaries make failure, cancellation, return scope and native rendering tractable without adding a generic workflow framework. The core owner consulted [structured parallel-state semantics](https://docs.aws.amazon.com/step-functions/latest/dg/state-parallel.html) as a reference, not as a hosted dependency.

Effects execute in the configured project context, with explicit per-stage context/target overrides. There is no implicit Git merge/workspace policy in this decision. Native forms/graph UI and CLI must expose the same declarative semantics. Concurrent waits and controls use invocation/token IDs, never just stage names. Statistics/artifacts must aggregate actual branch attempts once, preserving unknown measurements.

Incomplete zero/one-branch drafts may be saved, but executable validation requires the complete structured fork. Known optional values and dictionary entries can be removed without losing unknown extension fields. Prompt history is bounded while direct joined-branch summaries remain explicit. Acceptance uses real public-API/process concurrency fixtures, compiled serial regressions and the corresponding GUI configuration/history flow; source declarations alone do not establish completion.
