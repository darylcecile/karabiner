# Karabiner

A native Rust/GPUI workspace for project work, with an independently versioned Bun service and CLI.

## Development

Requirements: Rust/Xcode for the native macOS client; Bun 1.3.14+ for the service and CLI.

```sh
bun install
bun run service
```

The service prints its ready URL and listens on `127.0.0.1:43173`. It stores data in `~/Library/Application Support/Karabiner/karabiner.sqlite`. Override with `--db PATH` / `KARABINER_DB` and `--port N` / `KARABINER_PORT`. Ctrl-C shuts down the service cleanly.

In another terminal:

```sh
cargo run -p karabiner-client
bun run cli status --json
bun run cli project create "My project" --template software-delivery --json
bun run cli task create "Review navigation" --project PROJECT_ID --json
bun run cli doc create "Project notes" --project PROJECT_ID --file notes.md
bun run cli --help
```

CLI endpoint: `--url URL` or `KARABINER_SERVICE_URL`. The native client reads its service endpoint independently; the default is the URL above. No service is silently installed or replaced. To explore populated views explicitly, run `bun run cli demo seed`; the resulting project, documents, tasks and sample code-tour commits are labelled demo content.

## Independent builds

```sh
bun run build:service       # dist/karabiner-service
bun run build:cli           # dist/karabiner (command-line client)
bun run build:client        # target/debug/karabiner (native app)
```

The Bun binaries include their runtime and can be distributed separately. The compiled CLI's `service serve` command starts `karabiner-service` from PATH, or `KARABINER_SERVICE_BIN`. Source development can also use `bun run cli service serve`. Product versions are separate from API major 1; updating one component does not require matching all product versions.

## What the backend does

- Real SQLite CRUD for projects, tasks (including stage, due date, ordering and subtasks), documents, typed code tours, workflow/agent/instruction/skill configuration, settings, persistent tabs and intervention intentions. Operator task comments are distinct from execution intentions.
- Activity from real storage operations; counts and completion rate derived from manually stored task statuses.
- JSON HTTP API under `/v1`, optimistic revision conflicts, and SSE invalidation at `/v1/events` so CLI changes can refresh the GUI. Clients refetch snapshots on connection/reconnection or focus.
- Declarative onboarding templates and explicit demo seed command.

Execution remains an extension point: scheduler, Claude/Codex/Copilot/custom-local adapters, GitHub/Linear synchronization and optional sandbox/VM execution advertise unavailable capabilities. Saving a task or recording pause/approval intent does not start, pause or approve an agent run. `RunReader` supplies a future typed stage/step/trace read model; it currently returns no runs. Run/token/cost statistics are unavailable, not fabricated. MXC on macOS is a process sandbox; Apple `container` supplies Linux VMs on Apple silicon/macOS 26+.

## Native client

`cargo run -p karabiner-client` (Rust 1.100 via `rust-toolchain.toml`) talks to the service over loopback HTTP; override with `KARABINER_SERVICE_URL`. It runs without the service and shows an unavailable/incompatible banner with Retry.

- Workspace: sidebar (Home, Inbox, Projects, Templates, Settings, project tree with tasks and documents), titlebar tabs persisted through `/v1/workspace`, read-only inspector (⌥⌘I).
- Keys: ⌘T open, ⌘W close, ⌘⇧T reopen, ⌘1–9, ⌃Tab, ⌘[ / ⌘] in-tab back/forward, ⌘R refresh, ⌘\\ sidebar. Closing a tab with unsaved edits asks Save / Discard / Cancel.
- Projects: board/list by status or stage, task drawer, workflow stages and agents, instructions/skills, execution environment, two-step template picker.
- Documents: block editor — the active block shows its Markdown, others render formatted; a source view is secondary. It is not a full WYSIWYG editor. Code tours: overview, guide chapters, commit timeline, diffs and review decisions.
- Refresh: after its own edits, on ⌘R and when the window regains focus. The client does not subscribe to `/v1/events`.
- Release builds embed icons and assets (`cargo build -p karabiner-client --release`).

## API and verification

[`packages/contracts/src/index.ts`](packages/contracts/src/index.ts) contains DTOs and the full endpoint/method table. Successes return `{data}`, errors return `{error:{code,message}}`; `/v1/meta` provides API/service versions and capability flags. Project deletion cascades project data; task deletion detaches documents/interventions. Native tabs are persisted through `/v1/workspace`.

```sh
bun run typecheck
groundwork verify service-types service-roundtrip client-native-smoke
```

The bounded smoke check compiles real CLI/service binaries and verifies isolated SQLite persistence across restart, CRUD, stale edits, workspace context, typed tours/review progress, SSE and version/offline errors. It does not touch the user's data or call models/integrations/VMs. `client-native-smoke` (macOS, needs Screen Recording and Accessibility permission) starts its own service on an ephemeral port, drives the real window and saves screenshots to the evidence directory; its CLI-refresh and animation captures need visual review. Client build, unit tests and protocol round-trip checks run with it.

Architecture research: [domain relationships](.groundwork/guides/domain-contracts.md). Implementation guide: [service](.groundwork/guides/service.md).
