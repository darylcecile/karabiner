# Contributing to Karabiner

Start with a focused branch from `main`. Keep the change small enough to review, and describe what changed, why it matters and how you checked it in your pull request.

## Set up a development workspace

Use **Bun 1.3.14** from [`package.json`](package.json) and **Rust 1.100.0** from [`rust-toolchain.toml`](rust-toolchain.toml). Rustup selects the repository's pinned toolchain. Native client work requires **macOS and Xcode with the macOS SDK and Metal tools** (`metal` and `metallib`); CI uses Xcode 16.4 on Apple silicon.

Clone your fork or this repository, then run from the repository root:

```sh
bun run dev
```

The launcher installs locked Bun dependencies, incrementally builds the native client with the pinned Rust toolchain and `--locked`, waits for a compatible service, then opens the app. You can also run `./scripts/dev.sh` directly; it reports missing prerequisites, including Bun. The first native build can take several minutes. Cargo's configured target directory (including `CARGO_TARGET_DIR`) is respected.

Development defaults are `http://127.0.0.1:43174` and the persistent, ignored `.groundwork/tmp/dev/karabiner.sqlite`, separate from the installed app. Service/client logs append to `.groundwork/tmp/dev/service.log` and `client.log`; build diagnostics appear in the terminal.

Quit the app or press **Ctrl-C** in the launcher terminal to stop the app and the service it started. After source changes, stop and rerun `bun run dev` to rebuild; there is no hot reload. Rerunning preserves your database and configuration.

Optional overrides:

```sh
KARABINER_PORT=43175 KARABINER_DB="$PWD/.groundwork/tmp/other.sqlite" bun run dev
# Reuse a service you manage yourself:
KARABINER_SERVICE_URL=http://127.0.0.1:43174 bun run dev
```

An already-compatible service on the selected port is reused and left running when the launcher exits. Its existing database/logs remain in use; `KARABINER_DB` applies only when the launcher starts a service. An explicit `KARABINER_SERVICE_URL` takes precedence over port/database settings and must already be available with API major 1. An incompatible occupied port is reported rather than replaced. The client always receives an explicit URL, so it does not download or start another service.

For the CLI, use another terminal and the same development endpoint:

```sh
KARABINER_SERVICE_URL=http://127.0.0.1:43174 bun run cli status --json
```

The launcher does not sign in to providers or seed sample data. Configure the app normally using your existing tools/accounts. Preserve any running installed app/service, its data and its sign-in state.

See the [development guide](.groundwork/guides/project.md#development) for detailed setup, configuration, CLI usage and standalone build commands.

## Find the code

| Location | Responsibility |
| --- | --- |
| `apps/client/` | Native GPUI interface, client state and managed-service setup |
| `crates/protocol/` | Rust mirror of the shared wire contracts |
| `packages/contracts/` | TypeScript API/domain types and factory definitions |
| `packages/service/` | Domain behavior, execution, integrations and SQLite persistence |
| `packages/cli/` | Command-line access through the service API |
| `.groundwork/` | Architecture decisions, guides, verification catalogue and drivers |
| `.github/` | CI, release workflow and macOS packaging helpers |

## Preserve the boundaries

- Keep the client native **GPUI**. The service owns domain rules, execution and SQLite; clients use its API rather than opening the database or implementing a second runtime.
- Change the TypeScript contracts and their Rust mirrors together when changing the wire format. Preserve versioned API compatibility, unknown fields and revision-conflict behavior deliberately; component versions remain independent.
- A released client pins its service asset to the **same release tag**, with checksum verification. Keep that pin explicit rather than downloading a floating latest service.

Read the relevant [client/service](.groundwork/decisions/client-service-boundary.md), [factory execution](.groundwork/decisions/factory-execution-boundary.md) and [managed-service](.groundwork/decisions/managed-local-service.md) decisions before changing those boundaries. Update an existing ADR when an architectural decision changes; routine edits do not need a new decision document.

Keep functions focused, handle errors explicitly and follow the surrounding component/module conventions. Add a dependency when it solves a concrete need, and explain that need in the PR.

## Check your change

Choose checks for the area you changed; these are useful starting points, not a command to run every suite for every edit:

| Area | Existing commands |
| --- | --- |
| TypeScript, contracts or CLI | `bun run typecheck` |
| Service lint and formatting | `bun run --cwd packages/service lint` and `bun run --cwd packages/service fmt:check` |
| Native client | `cargo test -p karabiner-client --locked` |
| Rust protocol | `cargo test -p karabiner-protocol --locked` |
| Documentation | Follow changed links and run `git diff --check` |

Use the [verification guide](.groundwork/VERIFY.md) to select affected named workflow/fixture checks. Standard fixtures use owned temporary services and data; live model/provider checks are separate explicit opt-ins. Routine verification should not write to live providers or use the installed app's database.

Add tests for meaningful behavior or regressions, preferably in the existing tests/drivers. For small styling or reversible layout changes, inspect the actual UI instead of adding brittle tests that repeat layout constants. Native UI changes need a visual review of the affected interaction and screenshots of relevant states, including narrow layouts when affected. Report any behavior you could not exercise.

## Send a pull request

Keep unrelated changes out of the branch. Include:

- **What and why:** the user-visible result or problem addressed.
- **How verified:** the commands and scenarios actually exercised, with relevant results or screenshots.
- **Compatibility and limits:** any changed API/storage/release assumptions or remaining unverified behavior.

Update user-facing documentation when usage changes, and make error/loading/recovery states part of the change when relevant.

## Releases

The tag-triggered workflow builds the client and service independently, then publishes the complete app ZIP, raw service and checksum only after both builds succeed. Manual workflow dispatch is build-only and does not publish. See the [release guide](.groundwork/guides/project.md#single-client-releases) for the exact asset names, embedded service URL and packaging commands.
