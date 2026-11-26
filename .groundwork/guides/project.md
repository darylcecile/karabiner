# Project context

Karabiner is a native Rust/GPUI desktop client (`apps/client`, `crates/protocol`) plus an independently versioned Bun service, CLI and contracts (`packages/`). Service-owned SQLite is the source of truth for CRUD, configuration, compiled factory snapshots, runs, questions and reviewable outputs. The service implements explicit task dispatch/scheduling, local/Strands harness adapters, GitHub/Linear synchronization and optional host/MXC/Apple-container targets. Environment prerequisites and authentication are observed separately from implemented capabilities. The native factory canvas adapts React Flow's relevant graph algorithms/interactions with MIT attribution.

- End-user preview guide: root [README](../../README.md). Contributor setup, build and CLI commands are below.
- Boundaries: [client/service decision](../decisions/client-service-boundary.md), [local service boundary](../decisions/local-service-boundary.md), [factory execution](../decisions/factory-execution-boundary.md), [native canvas](../decisions/native-factory-canvas.md).
- Current accepted scope and evidence: [single capability matrix](factory-capabilities.md) and [implemented contracts](factory-contracts.md).
- Guides: [client interface](client-interface.md), [GPUI foundation](gpui-foundation.md), [service](service.md), [domain research](domain-contracts.md), visual design authority [visual contract](visual-contract.md) (measurements: [projects/tasks](visual-project-measurements.md), [docs/review/stats](visual-docs-measurements.md); [A](spatial-references-a.md) / [B](spatial-references-b.md) are research notes only).

## Development

### Prerequisites

- macOS for the native GPUI client; the verified native setup uses Apple silicon.
- Xcode with the macOS SDK and Metal tools (`metal` and `metallib`). Select the full Xcode installation as the active developer directory. The client CI pins Xcode 16.4.
- Rust through [rustup](https://rustup.rs/). [`rust-toolchain.toml`](../../rust-toolchain.toml) pins Rust 1.100.0.
- [Bun](https://bun.com/docs/installation) 1.3.14, pinned in [`package.json`](../../package.json).

The local Rust workspace dependencies are included in this checkout. Cargo downloads the pinned GPUI crates; a separate local GPUI checkout is not required.

```sh
git clone https://github.com/darylcecile/karabiner.git
cd karabiner
bun install --frozen-lockfile
```

### Run from source

From the repository root, start the foreground service in terminal 1:

```sh
bun run service
```

Wait for its JSON `ready` message. The default listener is `http://127.0.0.1:43173`. Reuse an existing service instance and its URL when one is already running.

In terminal 2, also at the repository root:

```sh
cargo run -p karabiner-client --bin karabiner --locked
```

Cargo compiles and launches the native app. The command remains in the foreground; use a third terminal for CLI commands. The app shows an unavailable/incompatible banner with Retry when it cannot use the service. Close the app window after resolving unsaved changes; Ctrl-C in terminal 1 shuts down the service cleanly. Cancel active work explicitly if you intend to stop it.

### Persistence and connection settings

Default service-owned database:

```text
~/Library/Application Support/Karabiner/karabiner.sqlite
```

Project code stays in the configured local checkout. Override storage with `bun run service --db /absolute/path/karabiner.sqlite` or `KARABINER_DB`; override the listener port with `--port N` or `KARABINER_PORT`.

Both native and CLI clients read `KARABINER_SERVICE_URL`. For example, if the service runs on port 43174, export `KARABINER_SERVICE_URL=http://127.0.0.1:43174` in each client terminal before launching. The CLI also supports `--url URL`.

Installed agent executables must be on the service process's `PATH` and use their existing signed-in accounts. Copilot targets CLI 1.0.116/protocol 4 with `@microsoft/copilot-sdk` 2.0.3. Native **Check readiness** reports installation, authentication and compatibility separately. The [service guide](service.md#local-harnesses-and-optional-targets) covers other providers, custom protocols, credential references and MXC/Apple-container prerequisites. GitHub/Linear configuration lives under project **Settings → Sync**; publication is explicit.

### CLI and API

While the service is running, use terminal 3 at the repository root. Replace uppercase placeholders with IDs returned by the list commands:

```sh
bun run cli status --json
bun run cli project list --json
bun run cli task list --project PROJECT_ID --json
bun run cli run list --project PROJECT_ID --json
bun run cli question list --run RUN_ID
bun run cli --help
```

The CLI also creates/updates projects and tasks, starts/controls runs, answers offered choices and validates/exports/imports factories. See the [CLI contract](factory-contracts.md#cli-interaction-contract). `bun run cli demo seed` explicitly creates labelled sample records.

[`packages/contracts/src/index.ts`](../../packages/contracts/src/index.ts) defines API DTOs and the endpoint table. Success responses use `{data}` and errors use `{error:{code,message}}`; the [service guide](service.md) documents `/v1`, persistence, runtime controls and custom harnesses. Client, service and CLI versions are independent of API major 1.

### Standalone builds

```sh
bun run build:service       # dist/karabiner-service
bun run build:cli           # dist/karabiner (CLI)
bun run build:client        # target/debug/karabiner (native app)
cargo build -p karabiner-client --bin karabiner --release --locked
                           # target/release/karabiner (native app, embedded assets)
```

Rust output paths assume `CARGO_TARGET_DIR` is unset. Compiled Bun binaries include their runtime, and the release native client embeds its assets; users of the packaged app do not need Rust, Xcode or Bun. The native app and CLI share an executable name, so use the correct `karabiner` binary for the intended purpose.

Run the service binary before the native binary; they are separate foreground processes. The compiled CLI's `service serve` starts `karabiner-service` from `PATH` or `KARABINER_SERVICE_BIN`. Source development can also use `bun run cli service serve`.

### Single-client releases

The [release workflow](../../.github/workflows/release.yml) builds the client and service independently on `macos-15` ARM64 with the existing Rust/Bun pins and Xcode16.4 SDK. GitHub's [current runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) identifies `macos-15` as ARM64, and each build checks the actual host architecture. It emits:

| Release asset | Consumer |
| --- | --- |
| `Karabiner-macos-arm64.zip` | End user: contains `Karabiner.app/Contents/MacOS/karabiner`, Info.plist and license notices. |
| `karabiner-service-macos-arm64` | The app's automatic setup: raw compiled service. |
| `karabiner-service-macos-arm64.sha256` | The app's integrity check: standard shasum line for the service. |

The client build embeds `KARABINER_SERVICE_ASSET_URL=https://github.com/darylcecile/karabiner/releases/download/<exact-tag>/karabiner-service-macos-arm64`. The app never follows a floating latest-service URL. The same URL is recorded in Info.plist so the packaged tag can be inspected. See the [managed-service ADR](../decisions/managed-local-service.md) for reuse, download verification, cache location and process lifetime.

Release builds reuse a compatible running local service, or verify/download and start their matching service when missing. Builds without the embedded URL retain the manual development flow above. Setting `KARABINER_SERVICE_URL` explicitly opts into an externally managed service; the app then does not install/start one.

The repository is public. Tag pushes publish to this repository using the publish job's `GITHUB_TOKEN` with `contents: write`; no extra token or credential is embedded or copied. The publish job waits for **both builds**. `gh release create` with attached assets creates a draft, uploads the complete asset set and then publishes it. A failed upload leaves a draft rather than a half-ready public release; an existing release is not overwritten automatically.

`workflow_dispatch` is **build-only**, requires an exact `release_tag` to embed and never runs the publish job. Use it for bounded CI verification before tagging. Dispatch artifacts only auto-download a service when the embedded release asset is actually available; artifact creation itself does not publish that asset. Publication remains a separate, authorized tag action. As of the initial setup on November26, no release has been published; the README links the Releases page without claiming an existing download.

No Apple signing/notarization credentials are configured. Packaging creates a valid app bundle but does not claim notarization or add a signing-credential setup. The README describes the preview's macOS launch limitation. Existing React Flow and Kubai notices are included; no new icon artwork is generated.

For an explicitly versioned local package after building the native client with the same asset URL:

```sh
export KARABINER_SERVICE_ASSET_URL="https://github.com/darylcecile/karabiner/releases/download/v0.1.0/karabiner-service-macos-arm64"
cargo build -p karabiner-client --bin karabiner --release --locked
bash .github/scripts/package-client.sh target/release/karabiner 0.1.0 dist
bun run build:service
bash .github/scripts/package-service.sh dist/karabiner-service dist
```

The package helper takes a built executable and version, preserves executable mode inside the ZIP and stamps the public service URL. It does not build, install, launch or publish the app. The service helper emits the raw file and checksum without an archive/extraction dependency in the client.

### Verification

For static development checks, use `bun run typecheck`. The `release-packaging` check exercises the actual package helpers with synthetic executable files: plist/ZIP layout, executable mode, exact service URL, license notices and raw service checksum. It launches no app or service; actual compiled first-run behavior is separate native-bootstrap evidence.

See the [verification entry point](../VERIFY.md) and [factory journey](factory-runtime.md) to select relevant checks. Existing factory examples include `groundwork verify service-types service-roundtrip factory-runtime factory-integrations`; native graphical checks use an already-built client, Screen Recording/Accessibility permission and the owned fixture driver.

Reuse any running app, service and data. Never stop or replace them as a side effect. Verification starts only its own temporary service and database. No provider calls, VM launches, credential collection or external data changes are required.
