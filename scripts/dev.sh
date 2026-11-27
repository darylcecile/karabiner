#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

log() { printf '[dev] %s\n' "$*"; }
fail() { printf '[dev] %s\n' "$*" >&2; exit 1; }
require() { command -v "$1" >/dev/null 2>&1 || fail "$2"; }

[[ $(uname -s) == Darwin ]] || fail 'The native client requires macOS with Xcode.'
require bun 'Install Bun 1.3.14 from https://bun.com/docs/installation, then rerun ./scripts/dev.sh.'
require rustup 'Install Rust via https://rustup.rs/, then rerun ./scripts/dev.sh.'
require xcrun 'Install Xcode and select it in Xcode Settings → Locations → Command Line Tools.'
require lsof 'lsof is required to check whether the development port is occupied.'

bun_version=$(bun -p 'require("./package.json").packageManager.split("@")[1]')
[[ $(bun --version) == "$bun_version" ]] || fail "Use Bun $bun_version (see https://bun.com/docs/installation)."
toolchain=$(bun -p 'Bun.TOML.parse(await Bun.file("rust-toolchain.toml").text()).toolchain.channel')
rustup run "$toolchain" cargo --version >/dev/null 2>&1 || fail "Install the pinned Rust toolchain: rustup toolchain install $toolchain"
xcrun --sdk macosx --show-sdk-path >/dev/null 2>&1 || fail 'The macOS SDK is unavailable. Open Xcode, finish its setup and select it in Settings → Locations → Command Line Tools.'

dev_dir="$root/.groundwork/tmp/dev"
service_url=${KARABINER_SERVICE_URL:-}
port=${KARABINER_PORT:-43174}
database=${KARABINER_DB:-$dev_dir/karabiner.sqlite}
if [[ -z "$service_url" ]]; then
  [[ "$port" =~ ^[0-9]{1,5}$ ]] && (( 10#$port >= 1 && 10#$port <= 65535 )) || fail 'KARABINER_PORT must be an integer from 1 to 65535.'
  port=$((10#$port))
  service_url="http://127.0.0.1:$port"
fi
service_url=${service_url%/}
[[ "$database" == /* || "$database" == :memory: ]] || database="$root/$database"
mkdir -p "$dev_dir"
service_log="$dev_dir/service.log"
client_log="$dev_dir/client.log"
build_output=$(mktemp "$dev_dir/build.XXXXXX")
work_pid= service_pid= client_pid=

cleanup() {
  local pid
  for pid in "$client_pid" "$service_pid" "$work_pid"; do
    [[ -n "$pid" ]] || continue
    kill -TERM "$pid" 2>/dev/null || true
  done
  for pid in "$client_pid" "$service_pid" "$work_pid"; do
    [[ -n "$pid" ]] || continue
    wait "$pid" 2>/dev/null || true
  done
  rm -f "$build_output"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

run_step() {
  local status=0
  "$@" &
  work_pid=$!
  wait "$work_pid" || status=$?
  work_pid=
  return "$status"
}

service_ready() {
  bun -e '
    try {
      const response = await fetch(process.argv[1] + "/v1/meta", { signal: AbortSignal.timeout(1000) });
      const meta = await response.json();
      process.exit(response.ok && meta?.data?.apiVersion === 1 ? 0 : 1);
    } catch { process.exit(1); }
  ' "$service_url"
}

wait_for_service() {
  local deadline=$((SECONDS + 30))
  while (( SECONDS < deadline )); do
    if ! kill -0 "$service_pid" 2>/dev/null; then
      tail -n 20 "$service_log" >&2
      fail "Service exited during startup. See $service_log."
    fi
    service_ready && return 0
    sleep 0.25
  done
  fail "Service at $service_url did not become API 1 ready within 30 seconds. See $service_log."
}

log 'Installing locked Bun dependencies…'
run_step bun install --frozen-lockfile || fail 'Dependency installation failed. Resolve the error above, then rerun bun run dev.'
log 'Building the native client (incremental; first build can take several minutes)…'
if ! run_step rustup run "$toolchain" cargo build -p karabiner-client --bin karabiner --locked --message-format=json-render-diagnostics >"$build_output"; then
  fail 'Native build failed. Resolve the Cargo error above and rerun. If Metal tools are missing, install the Metal Toolchain in Xcode Settings → Components (or run xcodebuild -downloadComponent MetalToolchain).'
fi
client_binary=$(bun -e '
  const messages = (await Bun.file(process.argv[1]).text()).trim().split("\n").map(line => JSON.parse(line));
  const artifact = messages.findLast(message => message.reason === "compiler-artifact" && message.target.name === "karabiner" && message.executable);
  if (!artifact) throw new Error("Cargo did not report a native client executable");
  console.log(artifact.executable);
' "$build_output")

log "Endpoint: $service_url"
if service_ready; then
  log 'Reusing the compatible running service; it will keep running when this launcher exits.'
  log 'Database and service logs: controlled by that service; KARABINER_DB is not applied.'
elif [[ -n ${KARABINER_SERVICE_URL:-} ]]; then
  fail "External service at $service_url is unavailable or incompatible (expected /v1/meta with API 1). Start/fix that service and rerun."
elif lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
  fail "Port $port is occupied by an unavailable or incompatible service (expected API 1). Choose another KARABINER_PORT or set KARABINER_SERVICE_URL to a compatible service."
else
  log "Database: $database"
  log "Service log: $service_log"
  bun packages/service/src/main.ts --port "$port" --db "$database" >>"$service_log" 2>&1 &
  service_pid=$!
  wait_for_service
  log 'Development service ready (API 1).'
fi

log "Client log: $client_log"
log 'Launching Karabiner. Quit the app or press Ctrl-C here to stop this session and its owned service.'
log 'After source changes, stop and rerun bun run dev to rebuild. Your database and configuration are preserved.'
KARABINER_SERVICE_URL="$service_url" "$client_binary" >>"$client_log" 2>&1 &
client_pid=$!
status=0
wait "$client_pid" || status=$?
client_pid=
if (( status != 0 )); then
  tail -n 20 "$client_log" >&2
  fail "Client exited with status $status. See $client_log."
fi
log 'Client exited; stopping this session’s owned processes.'
