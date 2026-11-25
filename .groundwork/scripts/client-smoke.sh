#!/bin/zsh
# Native client journey (macOS). Starts its own service on an ephemeral port with a
# temporary database, seeds labelled demo data, launches the built client against it,
# drives real shortcuts/clicks, and records window screenshots plus persisted state.
# Only processes started here are stopped. Visual outcomes (07/08, frames) are captured
# for review; the script asserts what the service can confirm.
#
# Needs: target/debug/karabiner, bun, cliclick, swift, and Screen Recording +
# Accessibility permission for the terminal.
set -u
ROOT=${0:A:h:h:h}
EVIDENCE=${GROUNDWORK_EVIDENCE_DIR:-$ROOT/.groundwork/tmp/client-smoke}
WORK=$(mktemp -d)
mkdir -p "$EVIDENCE"
SERVICE_PID= APP_PID= WIN= WX= WY= URL= FAILURE=

cleanup() {
  [[ -n $APP_PID ]] && kill $APP_PID 2>/dev/null
  [[ -n $SERVICE_PID ]] && kill $SERVICE_PID 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

fail() { FAILURE=${FAILURE:-$1}; }

# wait_until SECONDS COMMAND...: retry every 100ms until COMMAND succeeds.
wait_until() {
  local deadline=$(( $(date +%s) + $1 )); shift
  until "$@"; do (( $(date +%s) >= deadline )) && return 1; sleep 0.1; done
}

cat > "$WORK/win.swift" <<'SWIFT'
import CoreGraphics
let pid = Int32(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for w in list where (w["kCGWindowOwnerPID"] as? Int32) == pid && (w["kCGWindowLayer"] as? Int) == 0 {
  let b = w["kCGWindowBounds"] as! [String: Any]
  print(w["kCGWindowNumber"]!, b["X"]!, b["Y"]!); break
}
SWIFT
swiftc -O -o "$WORK/win" "$WORK/win.swift" 2>/dev/null || { echo "failed: swiftc unavailable"; exit 1; }

find_window() { read WIN WX WY <<< "$("$WORK/win" $APP_PID)"; [[ -n ${WIN:-} ]]; }
service_ready() {
  URL=$(grep -m1 '"event":"ready"' "$EVIDENCE/service.log" | bun -e 'const l=await Bun.stdin.text(); if(l) { const e=JSON.parse(l); if (e.pid === Number(process.argv[1])) console.log(e.url) }' $SERVICE_PID)
  [[ -n $URL ]] && curl -sf $URL/v1/meta >/dev/null
}
workspace() { curl -sf $URL/v1/workspace | bun -e "const d=(await Bun.stdin.json()).data; $1"; }
tab_count_is() { [[ $(workspace 'console.log(d.tabs.length)') == $1 ]]; }
closed_count_is() { [[ $(workspace 'console.log(d.recentlyClosed.length)') == $1 ]]; }
active_kind_is() { [[ $(workspace 'console.log(d.tabs.find(t => t.id === d.activeTabId)?.kind ?? "")') == $1 ]]; }
state_line() { workspace 'console.log(d.tabs.length, d.activeTabId ?? "none", d.recentlyClosed.length)'; }

key() { osascript -e "tell application \"System Events\" to keystroke \"$1\" using {$2}"; sleep 0.3; }
# Re-read bounds per click: the window can still move after it first appears.
click() { find_window || { fail "client window disappeared"; return 1; }; cliclick c:$((WX + $1)),$((WY + $2)); }
shot() { screencapture -x -o -l $WIN "$EVIDENCE/$1.png" || fail "screenshot $1 failed (Screen Recording permission?)"; }
cli_task() { KARABINER_SERVICE_URL=$URL bun run cli task create "$1" --project=$PROJECT --json >/dev/null 2>&1 || fail "CLI task create failed: $1"; }

journey() {
  osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $APP_PID) to true"
  wait_until 10 tab_count_is 0 || { fail "workspace not readable"; return; }
  sleep 0.5; shot 01-home
  echo "start: $(state_line)" > "$EVIDENCE/journey.txt"

  key t "command down"; sleep 0.5
  osascript -e 'tell application "System Events" to keystroke "keyb"'; sleep 0.4
  shot 02-quick-open
  osascript -e 'tell application "System Events" to key code 36'
  wait_until 5 tab_count_is 1 || fail "quick open did not persist a tab"
  sleep 0.5; shot 03-task-tab
  echo "after quick open: $(state_line)" >> "$EVIDENCE/journey.txt"

  key w "command down"
  wait_until 5 tab_count_is 0 && wait_until 5 closed_count_is 1 || fail "close did not persist"
  echo "after close: $(state_line)" >> "$EVIDENCE/journey.txt"
  key t "command down, shift down"
  wait_until 5 tab_count_is 1 || fail "reopen did not persist"
  echo "after reopen: $(state_line)" >> "$EVIDENCE/journey.txt"

  key '\\' "command down"; sleep 0.5; shot 04-sidebar-collapsed; key '\\' "command down"; sleep 0.5
  # Sidebar width animation: back-to-back frames after clicking the toggle.
  click 88 18
  for frame in 1 2 3 4 5; do screencapture -x -o -l $WIN "$EVIDENCE/06-sidebar-frame-$frame.png"; done
  sleep 0.5; click 88 18; sleep 0.5

  # Edits made outside the app appear without restarting it.
  PROJECT=$(curl -sf $URL/v1/projects | bun -e 'console.log((await Bun.stdin.json()).data[0].id)')
  click 90 230   # first sidebar project replaces the active tab
  wait_until 5 active_kind_is project || { fail "project row did not open the project"; return; }
  sleep 1
  cli_task "CLI task (manual refresh)"
  key r "command down"; sleep 1.5
  shot 07-cli-task-after-refresh
  cli_task "CLI task (focus refresh)"
  osascript -e "tell application \"System Events\" to set visible of (first process whose unix id is $APP_PID) to false"; sleep 0.5
  osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $APP_PID) to true"; sleep 1.5
  shot 08-cli-task-after-focus
  echo "created via CLI: 'CLI task (manual refresh)' before 07, 'CLI task (focus refresh)' before 08" >> "$EVIDENCE/journey.txt"

  kill $SERVICE_PID; wait $SERVICE_PID 2>/dev/null; SERVICE_PID=
  key r "command down"; sleep 1.5
  shot 05-service-unavailable
}

cd "$ROOT"
KARABINER_DB=$WORK/karabiner.sqlite bun packages/service/src/main.ts --port 0 > "$EVIDENCE/service.log" 2>&1 &
SERVICE_PID=$!
if ! wait_until 10 service_ready; then
  fail "owned service (pid $SERVICE_PID) did not become ready"
elif ! KARABINER_SERVICE_URL=$URL bun run cli demo seed --json > "$EVIDENCE/seed.json" 2>&1; then
  fail "demo seed failed"
else
  KARABINER_SERVICE_URL=$URL ./target/debug/karabiner > "$EVIDENCE/client.log" 2>&1 &
  APP_PID=$!
  if wait_until 20 find_window; then
    journey
  else
    fail "client window (pid $APP_PID) not found"
  fi
fi
grep -qi panic "$EVIDENCE/client.log" 2>/dev/null && fail "client panicked (see client.log)"

STATUS=passed; [[ -n $FAILURE ]] && STATUS=failed
OBSERVED=${FAILURE:-"Owned service $URL; quick open, close and reopen persisted via /v1/workspace; project row opened a project tab. Captured for visual review: CLI-created tasks after ⌘R (07) and refocus (08), sidebar animation frames (06-*), unavailable banner (05)."}
cat > "$EVIDENCE/result.json" <<JSON
{"observations":[{"status":"$STATUS","observed":"$OBSERVED","evidence":["journey.txt","01-home.png","02-quick-open.png","03-task-tab.png","04-sidebar-collapsed.png","06-sidebar-frame-1.png","07-cli-task-after-refresh.png","08-cli-task-after-focus.png","05-service-unavailable.png"]}]}
JSON
echo "$STATUS: $OBSERVED"
[[ $STATUS == passed ]]
