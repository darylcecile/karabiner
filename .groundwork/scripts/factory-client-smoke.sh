#!/bin/zsh
# Native factory builder journey (macOS). Starts its own service on an ephemeral port
# with a temporary database, seeds a deterministic custom-harness factory fixture,
# launches the built client against it, drives the Workflow canvas and task run through
# real clicks/keys, captures window screenshots and asserts persisted service state.
# Only processes started here are stopped. Visual parity is reviewed from the captures
# against the copied React Flow references in rf-reference/.
#
# Needs: target/debug/karabiner (or KARABINER_CLIENT_BIN), bun, cliclick, swift, and
# Screen Recording + Accessibility permission for the terminal.
set -u
ROOT=${0:A:h:h:h}
EVIDENCE=${GROUNDWORK_EVIDENCE_DIR:-$ROOT/.groundwork/tmp/factory-client-smoke}
CLIENT=${KARABINER_CLIENT_BIN:-$ROOT/target/debug/karabiner}
WORK=$(mktemp -d)
mkdir -p "$EVIDENCE/shots" "$EVIDENCE/state" "$EVIDENCE/rf-reference"
SERVICE_PID= APP_PID= WIN= WX= WY= WW= WH= URL= FAILURE= PROJECT= TASK= FACTORY0=
typeset -a PASSED

cleanup() {
  [[ -n $APP_PID ]] && kill $APP_PID 2>/dev/null
  [[ -n $SERVICE_PID ]] && kill $SERVICE_PID 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

fail() { FAILURE=${FAILURE:-$1}; echo "FAIL: $1" >> "$EVIDENCE/journey.txt"; }
note() { echo "$1" >> "$EVIDENCE/journey.txt"; }
pass() { PASSED+=("$1"); note "ok: $1"; }

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
  print(w["kCGWindowNumber"]!, b["X"]!, b["Y"]!, b["Width"]!, b["Height"]!); break
}
SWIFT
swiftc -O -o "$WORK/win" "$WORK/win.swift" 2>/dev/null || { echo "failed: swiftc unavailable"; exit 1; }

# probe ocr PNG X Y W H        → recognised text in a window-point region
# probe box PNG X Y W H R G B  → "x y w h" (window points) of pixels near colour RGB, or nothing
cat > "$WORK/probe.swift" <<'SWIFT'
import Foundation
import CoreGraphics
import ImageIO
import Vision
let a = CommandLine.arguments
let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: a[2]) as CFURL, nil)!
let full = CGImageSourceCreateImageAtIndex(src, 0, nil)!
let s = 2.0, r = CGRect(x: Double(a[3])! * s, y: Double(a[4])! * s, width: Double(a[5])! * s, height: Double(a[6])! * s)
let img = full.cropping(to: r)!
if a[1] == "ocr" || a[1] == "find" {
  let req = VNRecognizeTextRequest(); req.recognitionLevel = .accurate
  try! VNImageRequestHandler(cgImage: img).perform([req])
  let results = req.results ?? []
  if a[1] == "ocr" { print(results.compactMap { $0.topCandidates(1).first?.string }.joined(separator: " | ")) }
  else {
    // find PNG X Y W H TEXT [N]: centre (window points) of the Nth line equal to TEXT (else containing it).
    let want = a[7], nth = a.count > 8 ? Int(a[8])! : 1
    let lines = results.compactMap { o -> (String, CGRect)? in o.topCandidates(1).first.map { ($0.string, o.boundingBox) } }
    let exact = lines.filter { $0.0 == want }, loose = lines.filter { $0.0.contains(want) }
    let hits = (exact.isEmpty ? loose : exact).sorted { $0.1.minY > $1.1.minY }
    if hits.count >= nth {
      let b = hits[nth - 1].1, w = Double(img.width), h = Double(img.height)
      print(Int((r.minX + b.midX * w) / s), Int((r.minY + (1 - b.midY) * h) / s))
    }
  }
} else {
  let w = img.width, h = img.height
  var px = [UInt8](repeating: 0, count: w * h * 4)
  let ctx = CGContext(data: &px, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
  let t = [Int(a[7])!, Int(a[8])!, Int(a[9])!]
  var x0 = w, y0 = h, x1 = -1, y1 = -1
  for y in 0..<h { for x in 0..<w {
    let i = (y * w + x) * 4
    if abs(Int(px[i]) - t[0]) < 28 && abs(Int(px[i+1]) - t[1]) < 28 && abs(Int(px[i+2]) - t[2]) < 28 {
      x0 = min(x0, x); y0 = min(y0, y); x1 = max(x1, x); y1 = max(y1, y) }
  } }
  if x1 >= 0 { print(Int(r.minX / s) + x0 / 2, Int(r.minY / s) + y0 / 2, (x1 - x0) / 2, (y1 - y0) / 2) }
}
SWIFT
swiftc -O -o "$WORK/probe" "$WORK/probe.swift" 2>/dev/null || { echo "failed: probe compile"; exit 1; }
probe() { "$WORK/probe" "$@"; }

find_window() { read WIN WX WY WW WH <<< "$("$WORK/win" $APP_PID)"; [[ -n ${WIN:-} ]]; }
service_ready() {
  URL=$(grep -m1 '"event":"ready"' "$EVIDENCE/service.log" | bun -e 'const l=await Bun.stdin.text(); if(l) { const e=JSON.parse(l); if (e.pid === Number(process.argv[1])) console.log(e.url) }' $SERVICE_PID)
  [[ -n $URL ]] && curl -sf $URL/v1/meta >/dev/null
}

# api PATH [JS]: GET PATH, run JS over `d` (the data envelope), print result.
api() { curl -sf "$URL$1" | bun -e "const d=(await Bun.stdin.json()).data; ${2:-console.log(JSON.stringify(d))}"; }
snapshot() { api "$2" > "$EVIDENCE/state/$1.json" || fail "snapshot $1 failed"; }
config_js() { api "/v1/projects/$PROJECT/config" "$1"; }
workspace() { api /v1/workspace "$1"; }
active_kind_is() { [[ $(workspace 'console.log(d.tabs.find(t => t.id === d.activeTabId)?.kind ?? "")') == $1 ]]; }
factory_js() { api "/v1/projects/$PROJECT/factory" "$1"; }
run_js() { api "/v1/runs?taskId=$TASK" "const r=d[0]; $1"; }
run_state_is() { [[ $(run_js 'console.log(r?.state ?? "")') == $1 ]]; }
pending_kind_is() { [[ $(run_js 'console.log(r?.questions.find(q => q.state === "pending")?.kind ?? "")') == $1 ]]; }
stage_count() { config_js 'console.log(d.workflows[0].stages.length)'; }
edge_count() { config_js 'console.log(d.workflows[0].graph.edges.length)'; }
revision() { config_js 'console.log(d.revision)'; }
revision_above() { (( $(revision) > $1 )); }

front() { osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $APP_PID) to true"; sleep 0.3; }
key() { osascript -e "tell application \"System Events\" to keystroke \"$1\" using {$2}"; sleep 0.3; }
keycode() { local mods=; [[ -n ${2:-} ]] && mods=" using {$2}"; osascript -e "tell application \"System Events\" to key code $1$mods"; sleep 0.3; }
type_text() { osascript -e "tell application \"System Events\" to keystroke \"$1\""; sleep 0.3; }
# Window-relative points; bounds are re-read so moves/resizes stay correct.
at() { find_window || { fail "client window disappeared"; return 1; }; print -n "$((WX + $1)),$((WY + $2))"; }
click() { cliclick c:$(at $1 $2); sleep 0.3; }
dclick() { cliclick dc:$(at $1 $2); sleep 0.3; }
hover() { cliclick m:$(at $1 $2); sleep 0.4; }
# drag X1 Y1 X2 Y2 [shot]: press, move in steps, optionally capture mid-drag, release.
drag() {
  local a=$(at $1 $2) b=$(at $3 $4) mx=$(( ($1 + $3) / 2 )) my=$(( ($2 + $4) / 2 ))
  cliclick dd:$a w:120 dm:$(at $mx $my) w:120 dm:$b w:200
  [[ -n ${5:-} ]] && shot $5
  cliclick du:$b; sleep 0.4
}
scrollpx() { osascript -l JavaScript -e "ObjC.import('CoreGraphics'); \$.CGEventPost(0, \$.CGEventCreateScrollWheelEvent(null,0,1,$1));"; sleep 0.3; }
scroll() { osascript -l JavaScript -e "ObjC.import('CoreGraphics'); const e=\$.CGEventCreateScrollWheelEvent(null,1,2,$2,$1); \$.CGEventPost(0,e);"; sleep 0.3; }
cshot() { screencapture -x -o -C -l $WIN "$EVIDENCE/shots/$1.png" || fail "screenshot $1 failed"; }
shot() { screencapture -x -o -l $WIN "$EVIDENCE/shots/$1.png" || fail "screenshot $1 failed (window ${WIN:-none} of pid $APP_PID)"; }
# crop NAME X Y W H: close-up of a window-relative region (points; Retina doubles).
crop() { local s=$(( $(sips -g pixelWidth "$EVIDENCE/shots/$1.png" | awk '/pixelWidth/{print $2}') / WW ))
  sips -c $(( $5 * s )) $(( $4 * s )) --cropOffset $(( $3 * s )) $(( $2 * s )) "$EVIDENCE/shots/$1.png" --out "$EVIDENCE/shots/crop-$1.png" >/dev/null 2>&1; }
resize() { osascript -e "tell application \"System Events\" to set size of first window of (first process whose unix id is $APP_PID) to {$1, $2}"; sleep 0.6; }

copy_references() {
  local s
  for s in overview-default hover-node selected-node dragging-node hover-handle connecting-free connecting-snap connected selected-edge; do
    cp "$ROOT/.groundwork/tmp/rf-reference/$s.png" "$EVIDENCE/rf-reference/" 2>/dev/null || note "reference $s missing"
    cp "$ROOT/.groundwork/tmp/rf-reference/crop-$s.png" "$EVIDENCE/rf-reference/" 2>/dev/null
  done
}

# Window points at the default 1280×800 window, found by native discovery. Canvas points
# assume fit view followed by three zoom-ins on the seeded layout.
answer() { click $1 $2; sleep 0.5; type_text "$3"; keycode 36; }
edges_include() { [[ $(config_js "console.log(d.workflows[0].graph.edges.some(e => e.sourceStageId === '$1' && e.targetStageId === '$2'))") == true ]]; }

canvas_journey() {
  click 107 253; sleep 1                          # sidebar project
  click 442 176; sleep 1.5; shot 01-workflow-initial
  click 489 736; sleep 0.8; shot 02-fit-overview  # fit view
  for i in 1 2 3; do click 489 684; done; sleep 0.5; shot 03-zoomed
  hover 594 376; shot 04-hover-node
  click 594 376; shot 05-selected-node            # check: pink border, incident edges unchanged
  local rev=$(revision) factory=$(factory_js 'console.log(d.id)')
  # Connect repair's Return outcome (bottom-left handle) to check's top input.
  hover 561 556; shot 06-hover-handle
  # Invalid link: repair's Failure (75%) onto check would form a success/failure cycle.
  cliclick m:$(at 626 556) w:300 dd:$(at 626 556) w:200 dm:$(at 610 500) w:100 dm:$(at 596 365) w:100 dm:$(at 594 358) w:500
  cliclick du:$(at 594 358); sleep 0.3; shot 06b-invalid-link-feedback
  [[ $(revision) == $rev ]] && ! edges_include repair check && pass "cycle-forming failure link was refused" || fail "cycle-forming link was accepted"
  click 900 300; sleep 0.3                        # clear any feedback before connecting
  cliclick m:$(at 561 556) w:300 dd:$(at 561 556) w:200 dm:$(at 556 530) w:100 dm:$(at 540 490) w:100 dm:$(at 520 450) w:500; shot 07-connecting-free
  cliclick dm:$(at 560 380) w:100 dm:$(at 590 362) w:100 dm:$(at 594 358) w:500; shot 08-connecting-snap
  cliclick du:$(at 594 358); sleep 1; shot 09-connected
  click 1238 770
  wait_until 5 revision_above $rev || fail "connection save did not persist"
  edges_include repair check && pass "connected repair→check (return) persisted" || fail "repair→check edge missing"
  [[ $(factory_js 'console.log(d.id)') != $factory ]] && pass "edge change recompiled the factory" || fail "edge change kept the old factory"
  # Minimap click pans, then reconnect the selected return edge's target to build.
  rev=$(revision)
  click 489 710; sleep 0.5                        # zoom out once
  click 772 688; sleep 1; shot 10-minimap-panned
  cliclick m:$(at 911 438) w:300 dd:$(at 911 438) w:200 dm:$(at 880 450) w:100 dm:$(at 800 460) w:100 dm:$(at 740 445) w:100 dm:$(at 725 439) w:500; shot 11-reconnecting
  cliclick du:$(at 725 439); sleep 1; shot 12-reconnected
  click 1238 770
  wait_until 5 revision_above $rev || fail "reconnect save did not persist"
  edges_include repair build && ! edges_include repair check && edges_include review build && pass "minimap pan, then reconnect moved the new return edge to build; review→build return intact" || fail "reconnect did not persist or changed review→build"
  # Keyboard move is layout-only.
  rev=$(revision); factory=$(factory_js 'console.log(d.id)')
  local y=$(config_js 'console.log(d.workflows[0].layout.positions.check.y)')
  click 911 460; for i in 1 2 3 4 5; do keycode 125; done; shot 13-keyboard-moved
  click 1238 770
  wait_until 5 revision_above $rev || fail "keyboard move did not save"
  [[ $(config_js 'console.log(d.workflows[0].layout.positions.check.y)') != $y && $(factory_js 'console.log(d.id)') == $factory ]] && pass "arrow keys moved check; compiled factory $factory unchanged" || fail "keyboard move missing or changed the factory"
  snapshot config-after-edit "/v1/projects/$PROJECT/config"
  snapshot factory-after-edit "/v1/projects/$PROJECT/factory"
  resize 900 800; shot 14-narrow-workflow; resize 1280 800
  note "baseline (unaffected, not repeated): stale-config Keep mine and dirty-close Cancel"
}

# Hold a node drag and re-validate while the run streams updates.
streaming_checks() {
  # One owned run whose stream-hold agent prints progress every ~150ms until released,
  # so its revision advances while the node drag is held on the same project's canvas.
  local hold rid before during rev=$(revision)
  local runrev='api "/v1/runs/$rid" "console.log(d.revision)"'
  hold=$(curl -sf -X POST -H 'content-type: application/json' -d "{\"projectId\":\"$PROJECT\",\"title\":\"Hold for live updates\",\"workflowId\":\"hold\",\"agentId\":\"stream-hold\"}" $URL/v1/tasks | bun -e 'console.log((await Bun.stdin.json()).data.id)')
  rid=$(curl -sf -X POST -H 'content-type: application/json' -d '{}' $URL/v1/tasks/$hold/run | bun -e 'console.log((await Bun.stdin.json()).data.id)')
  wait_until 15 test -f "$WORK/project/.fixture/$rid/ready.json" || { fail "stream-hold run did not start"; return; }
  click 334 20; sleep 1                           # project tab
  click 1238 74; sleep 0.5                        # close the task peek
  click 442 176; sleep 1                          # Workflow
  click 489 736; for i in 1 2 3; do click 489 684; done; sleep 0.5
  before=$(eval $runrev)
  cliclick m:$(at 594 397) w:200 dd:$(at 594 397) w:150 dm:$(at 585 405) w:1000 dm:$(at 570 420) w:1000 dm:$(at 560 430) w:1000
  during=$(eval $runrev)
  shot 19-drag-held-while-streaming
  cliclick du:$(at 560 430); sleep 0.5; shot 19b-after-release-while-streaming
  note "run $rid revision $before → $during while the drag was held"
  (( during > before )) || fail "run revision did not advance during the held drag ($before → $during)"
  click 1238 770
  wait_until 5 revision_above $rev && pass "node drag held across run revisions $before→$during and saved" || fail "drag during live updates did not persist"
  snapshot layout-after-streaming-drag "/v1/projects/$PROJECT/config"
  click 1191 100; sleep 3; shot 20-validate-while-streaming
  note "review 19/19b (node follows the pointer) and 20 (Validate result still shown after run updates)"
  touch "$WORK/project/.fixture/$rid/release"
  wait_until 15 eval '[[ $(api "/v1/runs/$rid" "console.log(d.state)") == completed ]]' || fail "stream-hold run did not complete after release"
  click 494 20; sleep 1                           # task tab
}


run_journey() {
  click 362 58; sleep 0.8                         # List tab
  click 370 348; sleep 0.8                        # task row → peek
  click 1186 74; sleep 0.8                        # open full task
  click 360 546; sleep 0.8; shot 15-runs-empty
  click 887 746                                   # Start run
  wait_until 30 pending_kind_is question || { fail "run did not reach a question"; return; }
  sleep 1.5; shot 16-waiting-question
  hover 600 600; scroll 0 300; sleep 0.5
  answer 887 646 "Ship the observed change"
  wait_until 20 pending_kind_is approval || { fail "native answer did not reach the approval gate"; return; }
  # From the top, a fixed pixel scroll brings the decision step's approval panel into view.
  sleep 1.5; hover 600 700; scroll 0 300; sleep 0.5; scrollpx -250; sleep 1; shot 17-approval-in-run
  click ${=APPROVE_AT:-885 722}                    # run card Approve on the decision step
  wait_until 30 run_state_is completed || fail "run did not complete after native Approve"
  [[ $(run_js 'console.log(r.artifacts.some(a => a.kind === "document"))') == true ]] && pass "run card Approve completed the run with an output document" || fail "no output document artifact"
  sleep 2; hover 600 700; scroll 0 300; sleep 0.5; shot 18-completed   # header: state and step count
  scrollpx -250; sleep 1; shot 18b-completed-outputs
  click 908 406; sleep 1.5; shot 21-output-open    # Outputs → Open (first output)
  active_kind_is document && pass "opened the run's output document from the run card" || fail "output document not opened (active tab: $(workspace 'console.log(d.tabs.find(t => t.id === d.activeTabId)?.kind ?? "")'))"
  snapshot run "/v1/runs?taskId=$TASK"
  streaming_checks                                # the delivery run must finish first (one run at a time)
}

journey() { if [[ -n ${FACTORY_TAIL:-} ]]; then regression_tail; else canvas_journey; [[ -z $FAILURE ]] && run_journey; fi; }

# Focused regression tail (FACTORY_TAIL=1): only the states fixed after the reviewed run
# 2026-11-25T14-26-09-552Z_28242d52. Everything else keeps its earlier evidence.
PINK="236 110 180"                               # selected node border
node_box() { shot $1; probe box "$EVIDENCE/shots/$1.png" 300 120 700 640 ${=PINK}; }
bar_text() { probe ocr "$EVIDENCE/shots/$1.png" 300 740 980 55; }
toolbar_text() { probe ocr "$EVIDENCE/shots/$1.png" 880 80 400 40; }
# follows NAME DX DY X0 Y0: selected rect moved by the pointer delta (±12pt) and still holds the pointer.
follows() {
  local x y w h; read x y w h <<< "$(node_box $1)"
  [[ -n ${x:-} ]] || { note "$1: selected node not found"; return 1; }
  note "$1: rect $x,$y ${w}x$h (start $4,$5, pointer delta $2,$3)"
  (( ${$(( x - $4 - $2 ))#-} <= 12 && ${$(( y - $5 - $3 ))#-} <= 12 ))
}
settled_saved() { shot $1; local t; t=$(bar_text $1) || return 1; [[ $t != *Unsaved* ]]; }
ms() { bun -e 'console.log(Date.now())'; }
# save_timed NAME: click Save, log click → accepted revision → bar settled (ms).
save_timed() {
  local rev=$(revision) t0=$(ms) t1 t2
  click 1238 770
  wait_until 5 revision_above $rev || { fail "$1: save not accepted"; return 1; }
  t1=$(ms)
  local deadline=$(( $(date +%s) + 5 ))
  until settled_saved $1; do (( $(date +%s) >= deadline )) && break; sleep 0.3; done
  t2=$(ms)
  note "$1 timing: accepted +$(( t1 - t0 ))ms, bar settled +$(( t2 - t0 ))ms"
  settled_saved $1
}
validate_result() {
  local t0=$(ms); click 1191 100
  local deadline=$(( $(date +%s) + 6 )) text
  while (( $(date +%s) < deadline )); do
    sleep 0.5; shot $1; text=$(toolbar_text $1)
    [[ $text != *Validating* && $text == *(alid|rror|ssue|arning)* ]] && { note "$1 timing: result +$(( $(ms) - t0 ))ms"; print -r -- $text; return 0; }
  done
  note "$1 timing: no result after $(( $(ms) - t0 ))ms"
  print -r -- $text; return 1
}

regression_tail() {
  local rev x0 y0 w h cx cy before during text1 text2 p0 p1
  click 107 253; sleep 1; click 442 176; sleep 2
  shot T01-initial-fit-minimap
  click 640 412; sleep 0.5                        # check at the initial fit
  read x0 y0 w h <<< "$(node_box T02-check-selected)"
  [[ -n ${x0:-} && $(probe ocr "$EVIDENCE/shots/T02-check-selected.png" 960 130 320 100) == *"Name | check"* ]] \
    || { fail "selected stage is not check (inspector: $(probe ocr "$EVIDENCE/shots/T02-check-selected.png" 960 130 320 100))"; return; }
  pass "selected stage identified as check by the inspector; rendered at $x0,$y0 ${w}x$h"
  # Save settles: no Unsaved bar after the service accepts a save, with no further edits.
  keycode 125; sleep 0.3
  save_timed T03-after-save && pass "Unsaved bar cleared after accepted save" || fail "Unsaved bar remained after accepted save: $(bar_text T03-after-save)"
  text1=$(validate_result T04-validate-idle) && pass "Validate shows a result: $text1" || fail "Validate showed no result: $text1"
  # Live updates: a stream-hold run advances revisions while the drag is held.
  local hold rid
  hold=$(curl -sf -X POST -H 'content-type: application/json' -d "{\"projectId\":\"$PROJECT\",\"title\":\"Hold for live updates\",\"workflowId\":\"hold\",\"agentId\":\"stream-hold\"}" $URL/v1/tasks | bun -e 'console.log((await Bun.stdin.json()).data.id)')
  rid=$(curl -sf -X POST -H 'content-type: application/json' -d '{}' $URL/v1/tasks/$hold/run | bun -e 'console.log((await Bun.stdin.json()).data.id)')
  wait_until 15 test -f "$WORK/project/.fixture/$rid/ready.json" || { fail "stream-hold run did not start"; return; }
  sleep 1
  read x0 y0 w h <<< "$(node_box T05-before-drag)"
  [[ -n ${x0:-} && $(probe ocr "$EVIDENCE/shots/T05-before-drag.png" 960 130 320 100) == *"Name | check"* ]] || { fail "check not selected and rendered before drag"; return; }
  cx=$(( x0 + w / 2 )) cy=$(( y0 + h / 2 ))
  # Rendered width / 150 (React Flow default node width) gives the zoom for the expected delta.
  local zoom=$(bun -e "console.log(($w / 150).toFixed(3))")
  p0=$(config_js 'const p=d.workflows[0].layout.positions.check; console.log(p.x, p.y)')
  note "before drag: check world $p0, rendered $x0,$y0 ${w}x$h, zoom $zoom, press at $cx,$cy"
  before=$(api "/v1/runs/$rid" 'console.log(d.revision)')
  cliclick m:$(at $cx $cy) w:200 dd:$(at $cx $cy) w:300
  cshot T05b-mousedown-on-check                   # cursor over the selected rectangle before moving
  cliclick dm:$(at $(( cx + 20 )) $(( cy + 15 ))) w:150 dm:$(at $(( cx + 40 )) $(( cy + 30 ))) w:1500
  follows T06-drag-held-1 40 30 $x0 $y0 && pass "node follows the pointer mid-drag" || fail "node not under the pointer mid-drag (see T06)"
  cliclick dm:$(at $(( cx + 60 )) $(( cy + 40 ))) w:1500
  during=$(api "/v1/runs/$rid" 'console.log(d.revision)')
  follows T07-drag-held-2 60 40 $x0 $y0 && pass "node still follows after further live updates" || fail "node drifted from the pointer during live updates (see T07)"
  note "run $rid revision $before → $during while the drag was held"
  (( during > before )) || fail "run revision did not advance during the held drag ($before → $during)"
  cliclick du:$(at $(( cx + 60 )) $(( cy + 40 ))); sleep 0.3
  save_timed T08-after-save-streaming && pass "Unsaved bar cleared after save during live updates" || fail "Unsaved bar remained during live updates: $(bar_text T08-after-save-streaming)"
  p1=$(config_js 'const p=d.workflows[0].layout.positions.check; console.log(p.x, p.y)')
  local expect=$(bun -e "const [x,y]='$p0'.split(' ').map(Number); console.log(Math.round(x + 60 / $zoom), Math.round(y + 40 / $zoom))")
  note "check world $p0 → $p1; expected ≈ $expect (60,40pt at zoom $zoom, ±4)"
  bun -e "const a='$p1'.split(' ').map(Number), e='$expect'.split(' ').map(Number); process.exit(Math.abs(a[0]-e[0]) <= 4 && Math.abs(a[1]-e[1]) <= 4 ? 0 : 1)" \
    && pass "saved position matches the screen drag ($p0 → $p1, expected $expect)" || fail "saved position $p1 differs from expected $expect"
  snapshot layout-after-streaming-drag "/v1/projects/$PROJECT/config"
  text1=$(validate_result T09-validate-streaming) || fail "Validate showed no result during live updates: $text1"
  sleep 2; shot T10-validate-after-updates; text2=$(toolbar_text T10-validate-after-updates)
  [[ -n $text1 && $text2 == *$text1* ]] && pass "Validate result stayed through live updates: $text1" || fail "Validate result changed during live updates: '$text1' → '$text2'"
  touch "$WORK/project/.fixture/$rid/release"
  wait_until 15 eval '[[ $(api "/v1/runs/$rid" "console.log(d.state)") == completed ]]' || fail "stream-hold run did not complete after release"
  # Narrow allocation: panels open, then toggled closed (toggle point found on the new build).
  resize 900 800; sleep 0.8; shot T11-narrow-panels-open
  click ${=NARROW_TOGGLE:-833 100}; sleep 0.8; shot T12-narrow-inspector-closed   # toolbar inspector toggle at 900pt
  resize 1280 800
}

# Sourced by factory-complete-client.sh for its helpers only.
[[ -n ${FACTORY_LIB:-} ]] && return 0

cd "$ROOT"
: > "$EVIDENCE/journey.txt"
copy_references
[[ -x $CLIENT ]] || { echo "failed: client binary $CLIENT missing"; exit 1; }
# Run a byte-identical copy so root rebuilds can't replace the executable mid-run.
cp -p "$CLIENT" "$WORK/karabiner" && cmp -s "$CLIENT" "$WORK/karabiner" || { echo "failed: could not copy $CLIENT"; exit 1; }
note "client $CLIENT sha256 $(shasum -a 256 "$WORK/karabiner" | cut -d' ' -f1) built $(stat -f %Sm "$CLIENT")"
CLIENT=$WORK/karabiner
KARABINER_DB=$WORK/karabiner.sqlite bun packages/service/src/main.ts --port 0 > "$EVIDENCE/service.log" 2>&1 &
SERVICE_PID=$!
if ! wait_until 10 service_ready; then
  fail "owned service (pid $SERVICE_PID) did not become ready"
elif ! KARABINER_SERVICE_URL=$URL bun .groundwork/scripts/factory-client-fixture.ts "$WORK/project" > "$EVIDENCE/state/fixture.json" 2>"$EVIDENCE/fixture.log"; then
  fail "fixture seed failed (see fixture.log)"
else
  read PROJECT TASK FACTORY0 <<< "$(bun -e 'const f=await Bun.file(process.argv[1]).json(); console.log(f.projectId, f.taskId, f.factoryId)' "$EVIDENCE/state/fixture.json")"
  note "service $URL project $PROJECT task $TASK factory $FACTORY0"
  KARABINER_SERVICE_URL=$URL "$CLIENT" > "$EVIDENCE/client.log" 2>&1 &
  APP_PID=$!
  if wait_until 20 find_window; then
    front; journey
  else
    fail "client window (pid $APP_PID) not found"
  fi
fi
grep -qi panic "$EVIDENCE/client.log" 2>/dev/null && fail "client panicked (see client.log)"

STATUS=passed; [[ -n $FAILURE ]] && STATUS=failed
OBSERVED=${FAILURE:-"Owned service $URL with custom-harness fixture: ${(j:; :)PASSED}."}
SHOTS=$(cd "$EVIDENCE" && ls shots/*.png state/*.json rf-reference/*.png 2>/dev/null | bun -e 'console.log(JSON.stringify((await Bun.stdin.text()).trim().split("\n").filter(Boolean)))')
MODE=${FACTORY_TAIL:+regression-tail}; MODE=${MODE:-full-journey}
OBSERVED=$OBSERVED bun -e 'console.log(JSON.stringify({observations:[{status:process.argv[1],observed:(process.argv[3]==="regression-tail"?"Regression tail only (canvas selection/drag during live updates, save settle, Validate, 900pt panels); no run approval or output journey in this run. ":"")+process.env.OBSERVED,evidence:["journey.txt",...JSON.parse(process.argv[2])]}],context:{mode:process.argv[3]}}))' $STATUS "$SHOTS" $MODE > "$EVIDENCE/result.json"
echo "$STATUS: $OBSERVED"
[[ $STATUS == passed ]]
