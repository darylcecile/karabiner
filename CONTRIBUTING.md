# Contributing to Karabiner

Start with a focused branch from `main`. Keep the change small enough to review, and describe what changed, why it matters and how you checked it in your pull request.

## Set up a development workspace

Use **Bun 1.3.14** from [`package.json`](package.json) and **Rust 1.100.0** from [`rust-toolchain.toml`](rust-toolchain.toml). Rustup selects the repository's pinned toolchain. Native client work requires **macOS and Xcode with the macOS SDK and Metal tools** (`metal` and `metallib`); CI uses Xcode 16.4 on Apple silicon.

Clone your fork or this repository, then run from the repository root:

```sh
bun install --frozen-lockfile
```

Keep development data separate from the installed app. In **terminal 1**, start a service on a development port with a database under the ignored `.groundwork/tmp/` directory:

```sh
mkdir -p .groundwork/tmp/dev
bun run service --port 43174 --db "$PWD/.groundwork/tmp/dev/karabiner.sqlite"
```

Wait for its `ready` message. In **terminal 2**, also at the repository root:

```sh
KARABINER_SERVICE_URL=http://127.0.0.1:43174 \
  cargo run -p karabiner-client --bin karabiner --locked
```

Both commands stay in the foreground. The explicit `KARABINER_SERVICE_URL` makes this an externally managed development service, so the client does not download or start another one. Ordinary source builds without an embedded release URL also use manual service setup.

For the CLI, use a **third terminal** and the same development endpoint:

```sh
KARABINER_SERVICE_URL=http://127.0.0.1:43174 bun run cli status --json
```

If the port is occupied, choose another and update both client commands, or reuse your existing development service. Stop your dev service with **Ctrl-C** in its terminal. Preserve any running installed app/service, its data and its sign-in state.

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
