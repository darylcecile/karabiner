# Managed local service

Status: accepted, 2026-11-26. Supersedes the "start the service yourself" setup in
[client-service-boundary](client-service-boundary.md) for the default local service only; the
client/service boundary (separate process, loopback HTTP, versioned API) is unchanged.

Verification status: accepted, including the actual public packaged cold start. Release
`v0.1.0` is public at source commit `f6a5a9df91ff6d207637f90348472d674842e95c`.
The direct `fpublic1` observation ran the intact downloaded bundle with an isolated empty
cache/database, using its embedded asset URL. It downloaded and verified the published service,
started its owned process, reached native Ready/Home and returned API1/service0.1.0. Local
managed-service/recovery cases and real build/publication CI retain their separate accepted scope.

This cold state was on the existing Mac, not a brand-new machine or a machine without developer
tools. The release's setup transient was too fast to capture; earlier debug progress/error visual
proof remains separate. Canonical result, journey, metadata, service log, Info.plist, screenshots,
hashes and the downloaded ZIP are retained at
`manual-evidence/client-managed-service-fpublic1-2026-11-26/` in the repository's external
Groundwork evidence namespace. This is a direct-driver result, not a formal Groundwork run.

## Decision

End users download only the native client. When the default local service
(`http://127.0.0.1:43173`) doesn't answer, the client uses or fetches the service binary that
matches its own build and starts it:

1. **Reuse first.** Any service answering a compatible `/v1/meta` is used as is, never
   replaced or restarted. An incompatible one is reported, not touched.
2. **External stays external.** `KARABINER_SERVICE_URL` means the user runs the service; the
   client never downloads or starts one.
3. **Matching version, verified.** The build embeds `KARABINER_SERVICE_ASSET_URL` (the raw
   service asset of the same release). The client downloads `<url>.sha256` and the binary,
   checks the SHA-256, and only then saves it (atomic rename, mode 0700) under
   `~/Library/Application Support/Karabiner/service/<url key>/`. A cached copy is re-checked
   against its stored checksum before every launch. Failures leave existing files alone.
4. **Start and wait.** The service is started on the default port in its own process group,
   logging to `~/Library/Application Support/Karabiner/logs/service.log`, and the client waits
   up to 30 s of wall-clock time for `/v1/meta` (a request that never replies counts). If it
   doesn't answer, the client stops only the process it just started; the download is bounded
   at 5 minutes. A started service keeps running after the window closes (it owns runs and the
   database). No login item, launch agent or daemon is installed.
   Its PATH is the client's, in order, plus the standard install folders it lacks
   (`~/.local/bin`, `~/.bun/bin`, `/opt/homebrew/bin`, `/opt/homebrew/sbin`, `/usr/local/bin`),
   so a Finder-launched app still finds installed harness CLIs and `gh`. No shell profile is
   read and no credentials are copied; providers that need API keys still need them set.
5. **One attempt per Retry.** Progress and failures show in the existing service banner
   ("Setting up…", then a plain reason with Retry). Window refreshes don't cancel a setup.

Builds without `KARABINER_SERVICE_ASSET_URL` (local development) keep the manual
`bun run service` message.

Run-time settings (also used to test setup in isolation, never touching the user's service):
`KARABINER_PORT` moves the managed service and the client to `127.0.0.1:<port>`;
`KARABINER_DB` is inherited by the started service; `KARABINER_SUPPORT_DIR` replaces the
Application Support folder for the cache and log; `KARABINER_SERVICE_ASSET_URL` overrides the
built-in asset (HTTPS, or HTTP on this machine only); `KARABINER_SERVICE_START_TIMEOUT_SECS`
shortens the start wait.

## Release contract (CI)

Per tag, published as release assets:

| Asset | Content |
|---|---|
| `Karabiner-macos-arm64.zip` | `Karabiner.app` (client) |
| `karabiner-service-macos-arm64` | raw compiled service (`bun build --compile`) |
| `karabiner-service-macos-arm64.sha256` | `shasum -a 256` line for it |

The client build sets
`KARABINER_SERVICE_ASSET_URL=https://github.com/darylcecile/karabiner/releases/download/<tag>/karabiner-service-macos-arm64`
(the public source repository's releases, exact tag, never `latest`). Downloads are anonymous
HTTPS; the client embeds no token.

## Consequences

- No new runtime framework or updater; `sha2` (already in the lockfile) is the only added
  direct dependency.
- Upgrading the client points at a new asset URL, so a new cache folder; old folders can be
  deleted by hand. An already-running older service is reused if its API is compatible.
