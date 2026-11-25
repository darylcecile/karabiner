# Karabiner

A local software factory for your repositories. Configure workflows and agents, run tasks against a checkout, answer questions and review the results in a native Rust/GPUI app. A separate Bun service owns the data and execution; the CLI uses the same API.

## Getting started

### 1. Install the prerequisites

- **macOS** for the native client (verified on Apple silicon).
- **Xcode with the macOS SDK and Metal tools** (`metal` and `metallib`). Select the full Xcode installation as your active developer directory; the client CI uses Xcode 16.4.
- **Rust via [rustup](https://rustup.rs/)**. Cargo uses the **1.100.0** toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml).
- **[Bun](https://bun.com/docs/installation) 1.3.14**, pinned in [`package.json`](package.json).
- To execute agent tasks, an **installed and signed-in agent CLI** such as Copilot, Claude or Codex, available on the service's `PATH`. You can configure a custom harness or Strands provider instead; see [harness setup](.groundwork/guides/service.md#local-harnesses-and-optional-targets).

Clone this repository and install its JavaScript dependencies:

```sh
git clone https://github.com/darylcecile/karabiner.git
cd karabiner
bun install --frozen-lockfile
```

The Rust workspace's local dependencies are included in this checkout. Cargo downloads the pinned GPUI crates when it builds the app.

### 2. Start the service — terminal 1

From the repository root:

```sh
bun run service
```

Wait for the `ready` message. The default URL is **`http://127.0.0.1:43173`**. Leave this terminal running. If you already have a service instance, use that instance and its URL.

### 3. Start the native app — terminal 2

Open another terminal in the repository root:

```sh
cargo run -p karabiner-client --bin karabiner --locked
```

The first launch compiles the app. This command stays in the foreground while the app runs; use a **third terminal** for the optional CLI commands below.

If the app shows an unavailable/incompatible service banner, check terminal 1 and the configured URL, then use **Retry** or **⌘R**.

### 4. Create your first project and run a task

1. Open **Projects → New project** (or **Templates** in the sidebar). Select **Software delivery**, then **Use template**. It provides Plan → Implement → Review stages and their agents.
2. Enter a name, choose **Standalone** for a local project, and set **Local checkout** to the absolute path of an existing code repository. Choose an **Agent harness**, then **Create project**. “As template” uses Copilot for this template. You can change the checkout later in **Settings → General**.
3. Open the project's **Settings → Agents**. Select each agent, configure its **Harness** and **Model** as needed, and click **Check readiness**. Install/sign in using the selected CLI's own setup instructions if needed, then check again. Save your changes. Start with **Settings → Environment → Local host**; optional containment needs separate setup.
4. Open **Workflow** to review the stages, assigned agents and instructions. After any changes, click **Save**, then **Validate**. The graph should show **Valid** before execution. Project **Instructions** and **Skills** are available under Settings.
5. In the project's **Board**, use **Add task** or a column's **+** button, enter a clear task title and press Enter. Open the task, select its **Workflow** if needed, then choose **Runs → Start run**. Creating a project or task alone does not start agents; automatic scheduling is an explicit opt-in.
6. Follow stage progress in **Runs**. Answer questions and permission requests there, or use **Add instruction** to guide active work. Open the run's outputs, traces and change reviews, and inspect the resulting changes in your checkout.

**Stopping:** use **Cancel** on a run if you want to stop its work. Close the native app window, resolving any unsaved changes prompt, then press **Ctrl-C** in the service terminal for a clean service shutdown. Project/task/run data persists between launches.

## Data and connection settings

The service stores SQLite data at:

```text
~/Library/Application Support/Karabiner/karabiner.sqlite
```

Your code remains in the project's **Local checkout**. Change the database path with `bun run service --db /absolute/path/karabiner.sqlite` or `KARABINER_DB`; change the service port with `--port N` or `KARABINER_PORT`.

Both the native app and CLI read **`KARABINER_SERVICE_URL`**. Set it in each client terminal before launching, for example `export KARABINER_SERVICE_URL=http://127.0.0.1:43174` when the service uses port 43174. The CLI also accepts `--url URL`.

GitHub/Linear projects add source synchronization under **Settings → Sync**. Publication is explicit. Copilot currently targets CLI 1.0.116/protocol 4 through `@microsoft/copilot-sdk` 2.0.3; readiness reports version/authentication problems. See the [service guide](.groundwork/guides/service.md) for provider, credential and MXC/Apple-container requirements.

## Optional CLI — terminal 3

Run these from the repository root while the service is running. Replace uppercase placeholders with IDs returned by the list commands:

```sh
bun run cli status --json
bun run cli project list --json
bun run cli task list --project PROJECT_ID --json
bun run cli run list --project PROJECT_ID --json
bun run cli question list --run RUN_ID
bun run cli --help
```

The CLI can also create/update projects and tasks, start/control runs, answer offered choices, and validate/export/import factories. See [commands and contracts](.groundwork/guides/factory-contracts.md#cli-interaction-contract). For labelled sample data to explore the interface, explicitly run `bun run cli demo seed`.

## Build standalone binaries

```sh
bun run build:service       # dist/karabiner-service
bun run build:cli           # dist/karabiner (CLI)
bun run build:client        # target/debug/karabiner (native app)
cargo build -p karabiner-client --bin karabiner --release --locked
                           # target/release/karabiner (native app, embedded assets)
```

Rust output paths above assume `CARGO_TARGET_DIR` is unset. The Bun binaries include their runtime. Run the service binary first, then the native binary; each remains a separate process. The compiled CLI's `service serve` command can also start `karabiner-service` from `PATH` or `KARABINER_SERVICE_BIN`.

The tag-triggered [Build client](.github/workflows/build-client.yml) and [Build service](.github/workflows/build-service.yml) workflows build on `macos-15`. A successful tagged run uploads `karabiner-client-macos-arm64.tar.gz` or `karabiner-service-macos-arm64.tar.gz` as an Actions artifact on that run's page. Extract the archive to preserve executable permissions.

## API and development reference

- [Service and API guide](.groundwork/guides/service.md): `/v1` endpoints, persistence, run controls, integrations and custom harness protocols. [`packages/contracts/src/index.ts`](packages/contracts/src/index.ts) defines DTOs and the endpoint table; responses use `{data}` or `{error:{code,message}}`.
- [Native interface guide](.groundwork/guides/client-interface.md): workspace shortcuts, drafts, refresh and canvas behavior.
- [Verification guide](.groundwork/VERIFY.md) and [accepted feature matrix](.groundwork/guides/factory-capabilities.md): scoped checks, retained evidence and untested live prerequisites. For static development checks, use `bun run typecheck`; select broader checks through the guide.
- [Architecture decisions](.groundwork/decisions/): native GPUI client, independently versioned service/CLI, service-owned SQLite and declarative factory execution.
