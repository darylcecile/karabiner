#!/bin/zsh
# Managed-service first run (macOS). The app is launched WITHOUT KARABINER_SERVICE_URL so it
# downloads, verifies and starts its own service. Everything is owned and isolated:
#   KARABINER_PORT (free ephemeral), KARABINER_DB / KARABINER_SUPPORT_DIR (temp dirs),
#   KARABINER_SERVICE_ASSET_URL (loopback asset fixture serving a service compiled here).
# Only processes started here (app, asset server, the service it spawns on our port) are stopped.
# MANAGED_PUBLIC=<path to the released Karabiner .zip>: unzip it into owned temp and launch the bundle's
# own executable in place, with only PORT/DB/SUPPORT_DIR set, so the embedded public URL is used.
# MANAGED_PUBLIC_SHA=<published service sha256> is compared with the downloaded cache.
#
# Needs: built client (KARABINER_CLIENT_BIN), bun, cliclick, swift, Screen Recording + Accessibility.
set -u
setopt extendedglob
export GROUNDWORK_EVIDENCE_DIR=${GROUNDWORK_EVIDENCE_DIR:-${0:A:h:h}/tmp/client-managed-service}
FACTORY_LIB=1 source "${0:A:h}/factory-client-smoke.sh"     # window/probe/shot helpers only

ASSET=karabiner-service-macos-arm64
SUPPORT=$WORK/support DB=$WORK/db.sqlite ASSETS=$WORK/assets
ASSET_LOG=$EVIDENCE/state/asset-requests.jsonl ASSET_MODE=$WORK/asset-mode
mkdir -p "$SUPPORT" "$ASSETS"
PORT=$(bun -e 'const s=Bun.serve({port:0,fetch(){return new Response()}}); console.log(s.port); s.stop()')
ASSET_PID= ASSET_PORT=

# owned_service_pid: pid from the managed service's ready line in OUR support log, accepted only if
# that process's executable lives under OUR support dir. Ports are assertions, never ownership.
owned_service_pid() {
  local pid; pid=$(grep -ho '"event":"ready"[^}]*"pid":[0-9]*' "$SUPPORT"/logs/service.log 2>/dev/null | tail -1 | grep -o '[0-9]*$')
  local exe; exe=$(ps -o comm= -p ${pid:-0} 2>/dev/null); [[ -n $pid && ( ${exe:A} == ${SUPPORT:A}/* || ${exe:A} == ${WORK:A}/real-service ) ]] && print $pid
}
stop_owned_service() {
  local p; p=$(owned_service_pid); [[ -n $p ]] || return 0
  kill $p; wait_until 10 eval '! kill -0 $p 2>/dev/null' || { fail "owned service $p did not stop"; return 1; }
}
cleanup_owned() {
  [[ -n $APP_PID ]] && kill $APP_PID 2>/dev/null
  [[ -n $ASSET_PID ]] && kill $ASSET_PID 2>/dev/null
  stop_owned_service; [[ -n ${EXT_PID:-} ]] && kill $EXT_PID 2>/dev/null
  pkill -f "$WORK/hang-marker" 2>/dev/null   # owned hung fixture only (unique temp path in argv)
  rm -rf "$WORK"
}
trap cleanup_owned EXIT

# --- fixtures ------------------------------------------------------------------------------------
build_service() {
  bun build --compile packages/service/src/main.ts --outfile "$ASSETS/$ASSET" > "$EVIDENCE/state/service-build.log" 2>&1 \
    || { echo "failed: service build"; exit 1; }
  print -r -- "$(shasum -a 256 "$ASSETS/$ASSET" | cut -d' ' -f1)  $ASSET" > "$ASSETS/$ASSET.sha256"
  cp -p "$ASSETS/$ASSET" "$WORK/real-service"
  note "service asset sha256 $(cut -d' ' -f1 < "$ASSETS/$ASSET.sha256")"
}
# Asset server modes (file $ASSET_MODE): ok | badsum | fail | slow
start_assets() {
  print ok > "$ASSET_MODE"
  ASSETS=$ASSETS ASSET_MODE=$ASSET_MODE ASSET_LOG=$ASSET_LOG bun -e '
    import { appendFileSync, readFileSync } from "node:fs";
    const s = Bun.serve({ hostname: "127.0.0.1", port: 0, async fetch(r) {
      const p = new URL(r.url).pathname.slice(1), mode = readFileSync(process.env.ASSET_MODE!, "utf8").trim();
      appendFileSync(process.env.ASSET_LOG!, JSON.stringify({ at: Date.now(), path: p, mode }) + "\n");
      if (mode === "fail") return new Response("fixture failure", { status: 503 });
      if (mode === "slow") await Bun.sleep(4000);
      if (mode === "badsum" && p.endsWith(".sha256")) return new Response("0".repeat(64) + "  " + p.replace(/\.sha256$/, "") + "\n");
      const f = Bun.file(process.env.ASSETS + "/" + p);
      return (await f.exists()) ? new Response(f) : new Response("missing", { status: 404 });
    } });
    console.log(s.port);' > "$WORK/asset.port" &
  ASSET_PID=$!
  wait_until 5 test -s "$WORK/asset.port" || { echo "failed: asset server"; exit 1; }
  ASSET_PORT=$(< "$WORK/asset.port")
}
asset_mode() { print $1 > "$ASSET_MODE"; }
fetches() { [[ -f $ASSET_LOG ]] && wc -l < "$ASSET_LOG" | tr -d ' ' || echo 0; }

# --- app / service ---------------------------------------------------------------------------------
launch() {   # launch [external-url]
  if [[ -n ${1:-} ]]; then
    KARABINER_SERVICE_URL=$1 KARABINER_DB=$DB KARABINER_SUPPORT_DIR=$SUPPORT "$CLIENT" >> "$EVIDENCE/client.log" 2>&1 &
  else
    KARABINER_PORT=$PORT KARABINER_DB=$DB KARABINER_SUPPORT_DIR=$SUPPORT KARABINER_SERVICE_START_TIMEOUT_SECS=5 \
      KARABINER_SERVICE_ASSET_URL=http://127.0.0.1:$ASSET_PORT/$ASSET "$CLIENT" >> "$EVIDENCE/client.log" 2>&1 &
  fi
  APP_PID=$!; wait_until 20 find_window || { fail "app window not found"; return 1; }
  caffeinate -di -w $APP_PID & front; resize 1280 800; note "app pid $APP_PID"
}
quit_app() {
  [[ -n $APP_PID ]] || return 0
  # Request normal quit on the owned PID, independently of the foreground app.
  osascript -l JavaScript -e "ObjC.import('AppKit'); \$.NSRunningApplication.runningApplicationWithProcessIdentifier($APP_PID).terminate" >/dev/null
  wait_until 10 eval '! kill -0 $APP_PID 2>/dev/null' || { note "normal quit timed out; sending TERM to owned app pid $APP_PID"; kill "$APP_PID" 2>/dev/null; }
  wait_until 10 eval '! kill -0 $APP_PID 2>/dev/null' || { fail "owned app $APP_PID did not stop"; exit 1; }
  APP_PID=
}
service_pid() { owned_service_pid; }
port_listening() { lsof -tiTCP:$PORT -sTCP:LISTEN >/dev/null 2>&1; }
meta_ok() { curl -sf "http://127.0.0.1:$PORT/v1/meta" > /dev/null; }
banner() { shot $1; probe ocr "$EVIDENCE/shots/$1.png" 0 0 1280 160; }
# ready NAME: service answers and the app shows its normal content (sidebar Projects) with no setup banner.
ready() {
  wait_until 60 meta_ok || { fail "$1: service never answered on :$PORT within 60s"; return 1; }
  local i b; for i in $(seq 30); do sleep 0.5; b=$(banner $1); [[ $b != *(Setting up|Downloading|Starting)* ]] && break; done
  [[ $(probe ocr "$EVIDENCE/shots/$1.png" 0 0 400 800) == *Projects* && $b != *(Setting up|Downloading|Starting|Retry)* ]]
}
# see_banner NAME PATTERN SECONDS: capture until the top area shows PATTERN.
see_banner() { local i; for i in $(seq $(( $3 * 4 ))); do sleep 0.25; [[ $(banner $1) == *($2)* ]] && return 0; done; return 1; }

at_text() { shot _find; probe find "$EVIDENCE/shots/_find.png" ${2:-0} ${3:-0} ${4:-1280} ${5:-800} "$1" ${6:-1}; }
tap() { local p; p=$(at_text "$@"); [[ -n $p ]] || { note "control '$1' not found"; return 1; }; click ${=p}; sleep 0.4; }
# error_banner NAME SECONDS: the setup banner left "Setting up" and shows an error with an enabled Retry.
error_banner() { local i b; for i in $(seq $(( $2 * 4 ))); do sleep 0.25; b=$(banner $1); [[ $b == *Retry* && $b != *"Setting up"* ]] && { note "$1 banner: ${b:0:200}"; return 0; }; done; note "$1 banner: ${b:0:200}"; return 1; }
retry_banner() { local r; r=$(at_text Retry 900 30 380 60) && [[ -n $r ]] && { click ${=r}; sleep 0.5; } || note "banner Retry not found"; }

# --- cases ------------------------------------------------------------------------------------------
case_fresh() {
  launch || return
  see_banner M01-setting-up "Setting up|Downloading the Karabiner service|Starting the Karabiner service" 10 && pass "setup banner shown on first launch" || note "setup banner not captured (fast path)"
  ready M02-ready && pass "fresh app downloaded, verified and started its service; UI ready" || fail "fresh first run not ready (M02)"
  local f=$(fetches); [[ $f == 2 ]] && pass "exactly two asset requests (binary + checksum)" || fail "asset requests: $f"
  ls -R "$SUPPORT" > "$EVIDENCE/state/support-after-fresh.txt"
  FIRST_PID=$(service_pid); note "managed service pid $FIRST_PID on :$PORT"
}
case_quit_and_reuse() {
  quit_app; sleep 1
  [[ -n $FIRST_PID ]] && kill -0 $FIRST_PID 2>/dev/null && meta_ok && pass "service survives app quit" || fail "service gone after app quit"
  local f0=$(fetches); launch || return; ready M03-relaunch-ready || fail "relaunch not ready (M03)"
  [[ $(service_pid) == $FIRST_PID && $(fetches) == $f0 ]] && pass "relaunch reused running service $FIRST_PID, no fetch" || fail "relaunch: pid $(service_pid) vs $FIRST_PID, fetches $f0→$(fetches)"
}
case_offline_cache() {
  quit_app; stop_owned_service
  kill $ASSET_PID; ASSET_PID=; local f0=$(fetches)
  launch || return; ready M04-offline-ready && [[ $(fetches) == $f0 ]] \
    && pass "with assets offline, relaunch started the cached service ($(service_pid))" || fail "offline cached start failed (M04)"
  quit_app; stop_owned_service; start_assets
}
case_external() {
  local ext=$WORK/ext; mkdir -p $ext
  KARABINER_DB=$ext/db.sqlite bun packages/service/src/main.ts --port 0 > "$ext/log" 2>&1 & EXT_PID=$!
  wait_until 10 eval 'grep -q ready "$ext/log"'; local url=$(grep -o 'http://[0-9.:]*' "$ext/log" | head -1)
  local f0=$(fetches); launch "$url" || return; sleep 3; banner M05-external > /dev/null
  [[ $(fetches) == $f0 ]] && ! port_listening && pass "explicit KARABINER_SERVICE_URL: no download, no spawn" || fail "external mode fetched/spawned"
  quit_app; kill $EXT_PID; EXT_PID=
}
case_bad_checksum() {
  rm -rf "$SUPPORT"; mkdir -p "$SUPPORT"; asset_mode badsum
  launch || return
  error_banner M06-badsum 20 && pass "checksum mismatch shows a Retry error" || fail "no Retry after checksum mismatch (M06)"
  ! port_listening && pass "unverified binary not started" || fail "service started despite bad checksum"
  resize 900 800; sleep 0.6; banner M07-badsum-narrow > /dev/null; resize 1280 800
  asset_mode ok; retry_banner; ready M08-retry-ready && pass "Retry after fixture correction reached Ready" || fail "Retry did not recover (M08)"
  quit_app; stop_owned_service
}
case_download_failure() {
  stop_owned_service || return
  rm -rf "$SUPPORT"; mkdir -p "$SUPPORT"; asset_mode fail
  launch || return
  error_banner M09-download-failed 20 && pass "download failure shows a plain Retry" || fail "no Retry after download failure (M09)"
  ! port_listening && pass "nothing started without a verified download" || fail "something listens after failed download"
  resize 900 800; sleep 0.8; shot M09-download-failed-900; resize 1280 800; sleep 0.8
  asset_mode ok; local f0=$(fetches) t0=$(ms); retry_banner; shot M09a-after-retry-click
  if ready M09b-retry-ready; then pass "Retry after the asset server recovered reached Ready in $(( $(ms) - t0 ))ms"
  else fail "Retry did not reach Ready (M09b)"; fi
  note "asset requests after Retry: $(tail -n +$(( f0 + 1 )) "$ASSET_LOG" | tr '\n' ' ')"
  cp "$SUPPORT"/logs/service.log "$EVIDENCE/state/service.log" 2>/dev/null; ls -lR "$SUPPORT" > "$EVIDENCE/state/support-after-retry.txt"
  quit_app; stop_owned_service
}
# Constant fixture asset: exits (or hangs) until $WORK/recover exists, then execs the real service.
wrapper_asset() {   # wrapper_asset exit|hang
  local bad='exit 1'; [[ $1 == hang ]] && bad="exec $(command -v bun) -e 'setInterval(()=>{},1e9)' '$WORK/hang-marker'"
  print -r -- "#!/bin/sh
[ -f '$WORK/recover' ] && exec '$WORK/real-service' \"\$@\"
$bad" > "$ASSETS/$ASSET"; chmod +x "$ASSETS/$ASSET"
  print -r -- "$(shasum -a 256 "$ASSETS/$ASSET" | cut -d' ' -f1)  $ASSET" > "$ASSETS/$ASSET.sha256"
}
case_bad_start() {   # case_bad_start exit|hang
  rm -rf "$SUPPORT" "$WORK/recover"; mkdir -p "$SUPPORT"; wrapper_asset $1
  local t0=$(ms); launch || return
  error_banner M10-$1 20 && pass "service that does '$1' fails with Retry after $(( $(ms) - t0 ))ms" || fail "no bounded failure for '$1' (M10-$1)"
  touch "$WORK/recover"; retry_banner
  ready M11-$1-retry && pass "Retry after the environment recovered reached Ready ($1)" || fail "Retry did not recover ($1, M11-$1)"
  quit_app; stop_owned_service; pkill -f "$WORK/hang-marker" 2>/dev/null
}
case_early_exit() { case_bad_start exit; }
case_hung() { case_bad_start hang; }

cd "$ROOT"
: > "$EVIDENCE/journey.txt"
if [[ -n ${MANAGED_PUBLIC:-} ]]; then
  ditto -x -k "$MANAGED_PUBLIC" "$WORK/app" || { echo "failed: unzip $MANAGED_PUBLIC"; exit 1; }
  APP=$(ls -d "$WORK"/app/*.app | head -1); EXE=$APP/Contents/MacOS/$(defaults read "$APP/Contents/Info" CFBundleExecutable)
  note "zip sha256 $(shasum -a 256 "$MANAGED_PUBLIC" | cut -d' ' -f1); bundle $(defaults read "$APP/Contents/Info" CFBundleIdentifier) $(defaults read "$APP/Contents/Info" CFBundleShortVersionString); executable sha256 $(shasum -a 256 "$EXE" | cut -d' ' -f1)"
  plutil -p "$APP/Contents/Info.plist" > "$EVIDENCE/state/Info.plist.txt"
  note "owned port $PORT db $DB support $SUPPORT; cache before: $(find "$SUPPORT" -type f | wc -l | tr -d ' ') files"
  env -u KARABINER_SERVICE_URL -u KARABINER_SERVICE_ASSET_URL KARABINER_PORT=$PORT KARABINER_DB=$DB KARABINER_SUPPORT_DIR=$SUPPORT "$EXE" >> "$EVIDENCE/client.log" 2>&1 &
  APP_PID=$!; wait_until 20 find_window || fail "app window not found"; caffeinate -di -w $APP_PID & front; resize 1280 800; note "app pid $APP_PID ($EXE)"
  see_banner P01-setting-up "Setting up|Downloading the Karabiner service|Starting the Karabiner service" 20 && pass "setup banner shown" || note "setup banner not captured"
  if ready P02-ready; then pass "packaged client downloaded its pinned public service and reached Ready"; else fail "public first launch not ready (P02)"; fi
  ls -lR "$SUPPORT" > "$EVIDENCE/state/support-public.txt"; cp "$SUPPORT"/logs/service.log "$EVIDENCE/state/service.log" 2>/dev/null
  local_bin=$(find "$SUPPORT" -type f -name 'karabiner-service*' ! -name '*.sha256' | head -1)
  got=$(shasum -a 256 "$local_bin" 2>/dev/null | cut -d' ' -f1); note "cached service $local_bin sha256 $got; owned service pid $(owned_service_pid)"
  [[ -n ${MANAGED_PUBLIC_SHA:-} ]] && { [[ $got == $MANAGED_PUBLIC_SHA ]] && pass "cached service matches the published sha256" || fail "cached sha $got != published $MANAGED_PUBLIC_SHA"; }
  curl -sf "http://127.0.0.1:$PORT/v1/meta" > "$EVIDENCE/state/meta.json" && pass "owned service answers /v1/meta"
  quit_app; stop_owned_service
else
  [[ -x $CLIENT ]] || { echo "failed: client binary $CLIENT missing"; exit 1; }
  cp -p "$CLIENT" "$WORK/karabiner"; note "client $CLIENT sha256 $(shasum -a 256 "$WORK/karabiner" | cut -d' ' -f1)"; CLIENT=$WORK/karabiner
  note "owned port $PORT db $DB support $SUPPORT"
  build_service; start_assets
  for c in ${=MANAGED_CASES:-case_fresh case_quit_and_reuse case_offline_cache case_external case_bad_checksum case_download_failure case_early_exit case_hung}; do $c; done
fi
grep -qi panic "$EVIDENCE/client.log" 2>/dev/null && fail "client panicked"
STATUS=passed; [[ -n $FAILURE ]] && STATUS=failed
SHOTS=$(cd "$EVIDENCE" && ls shots/*.png state/* 2>/dev/null | bun -e 'console.log(JSON.stringify((await Bun.stdin.text()).trim().split("\n").filter(Boolean)))')
OBSERVED=${FAILURE:-"${(j:; :)PASSED}"} bun -e 'console.log(JSON.stringify({observations:[{status:process.argv[1],observed:process.env.OBSERVED,evidence:["journey.txt",...JSON.parse(process.argv[2])]}]}))' $STATUS "$SHOTS" > "$EVIDENCE/result.json"
echo "$STATUS"; [[ $STATUS == passed ]]
