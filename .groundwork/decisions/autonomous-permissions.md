# Autonomous agent permission decisions

Status: accepted, 2026-11-27. Refines the mode semantics in [live harness interactions](live-harness-interactions.md); verification evidence remains separately scoped.

## Decision

The existing `AgentDefinition.mode: autonomous` authorizes automatic native tool permission decisions for that agent invocation. The shared adapter seam selects an actually offered approve-effect choice and emits an activity entry instead of creating a pending human interaction. Interactive agents retain the addressed live allow/deny/question channel. This applies equally to host, MXC and Apple-container targets: containment chooses where tools execute; agent mode chooses who answers tool permission requests.

Approval is per request through the provider's supported callback. Copilot SDK 2.0.3 receives `approve-once`, `approvedInteractively: false` and the supported `host_policy` / `auto_approved` / `headless` attribution. A human answer retains `human_response` / `prompted_user` / `interactive`. Copilot requests marked `managedApprovalRequired` fail explicitly in Autonomous mode; the SDK converts handler failures to `user-not-available`. The released SDK's `dist/types.d.ts`, `dist/generated/{rpc,session-events}.d.ts` and `dist/session.js` define and forward this contract; no SDK patch or protocol override is involved.

Autonomous mode does not answer agent questions, supply form values, complete browser authentication or manufacture an unavailable approval choice. Claude elicitation is permission-only only when it has the SDK's permission-display title, no fields and no URL mode. Active invocation and provider-request cancellation take precedence over approval. Mode-specific prompt guidance directs autonomous agents to report a concrete blocker when essential human information is unavailable.

## Reason and consequences

Previously Autonomous disabled agent questions but still waited indefinitely for human tool permissions, preventing unattended factories. Reusing the explicit mode expresses the existing product choice without another policy setting. New-agent editor defaults and templates choose Autonomous; saved Interactive choices are preserved.

Workflow approval gates remain deliberate stage controls outside this adapter seam. Target configuration, mounts, network and native provider enforcement are not altered; automatic SDK attribution does not claim human authority. A mandatory provider restriction can still reject a request. Existing interactive live-account proof does not establish autonomous live-provider behavior; controlled native fixtures and the existing workflow-gate journey cover this change without using the user's running environment.

See [factory runtime verification](../guides/factory-runtime.md) for the relevant cases and evidence boundaries.
