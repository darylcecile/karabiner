# Recorded agent activity

Status: accepted, implementation 2026-11-27; runtime/native verification is tracked separately.

## Decision

Keep optional `RunStepAttempt.events` in existing `StoredRun` JSON, using the current throttled attempt writes and read API. Final `output` remains the outcome summary; it no longer serves as the only transcript. Older attempts need no backfill and retain their stored output fallback.

The activity contract records provider messages, exposed readable reasoning, tool input/results and explicit errors. Tool starts/completions update one event by raw provider call ID, with trace identity `<attemptId>:<toolId>`. Repeated tool names never establish identity, and retries keep separate journals. Adjacent deltas coalesce only within their message/kind and across no intervening tool boundary. Completed provider messages are fallbacks when their deltas were unavailable. No hidden/opaque reasoning or reasoning configuration is requested.

Use a rolling limit of 512 events and approximately 256,000 serialized characters per attempt; each text/input/output field is at most 32,000 characters. Truncation is visible. Keep at most 512 tool trace spans per attempt. Final summaries remain independent even after activity truncation. Closing the emitter prevents late callbacks from rewriting a finished attempt.

Only named activity fields are selected from provider responses. Tool JSON is pretty-printed, bounded, handles nonserializable values explicitly and redacts credential/environment-named fields. Launch configuration, authentication and session frames are not activity payloads. Native binary/media and signed/encrypted reasoning are not copied into activity.

## Evidence for the mapping

- Installed `@microsoft/copilot-sdk` 2.0.3: `dist/generated/session-events.d.ts` exposes `assistant.message[_delta]`, `assistant.reasoning[_delta]`, tool `arguments`, `result.content`, `result.detailedContent`, `result.structuredContent`, and `error.message`.
- Installed Claude Agent SDK 0.3.332: `sdk.d.ts` documents one assistant frame per completed content block, partial stream events, text/thinking/tool-use blocks and user tool-result blocks.
- Installed Strands SDK 1.26.0: `models/streaming.d.ts` exposes `reasoningContentDelta.text`; `types/messages.d.ts` and hook events expose tool input and text/JSON results. Binary blocks and reasoning signatures/redactions are omitted.
- Codex is unavailable on the worker PATH. The [official app-server documentation](https://developers.openai.com/codex/app-server#items) defines completed items, reasoning summaries/text deltas, command output, file changes and MCP/dynamic tool results. Its controlled JSON-RPC peer verifies mapping; this is not a live installed-Codex observation.

## Reason and consequences

The native inspector could select an agent or tool but the service had discarded its input/results and replaced its transcript with a final summary. The additive attempt journal fixes the data loss without a table, migration, endpoint or second event system. It extends [bounded attempt history](structured-factory-execution.md) and preserves [native control/cleanup semantics](live-harness-interactions.md).

Reasoning is available only when the provider emits readable text. Some tools (such as Codex file changes/web searches) expose input and completion status without a separate output body. Retention limits can remove earlier tool details; the UI must distinguish absent/truncated data from actual results. The original provider's textual output remains provider-authored data, not an inferred account of its thought process.
