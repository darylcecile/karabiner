#!/bin/zsh
# Native feature-complete GUI acceptance (macOS). One controlled actor: an owned service on an
# ephemeral port with a temporary SQLite database (real startService with only GitHub/Linear
# transports injected), a temporary git fixture project, controlled provider peers from fixtures/native-harness-peer.ts,
# and a byte-identical copy of the built client. Configuration under claim is done through the
# GUI; the API is used only to seed initial state and to assert what the GUI persisted/caused.
#
# Reuses factory-client-smoke.sh helpers (window/click/drag/shot, OCR + pink node box probes,
# api/config_js/revision waits). Each journey records ok/FAIL/unverified lines in journey.txt;
# a journey whose controls were not located on the released build records "unverified" with
# the reason and never passes.
#
# Needs: target/debug/karabiner (or KARABINER_CLIENT_BIN), bun, cliclick, swift, and Screen
# Recording + Accessibility permission. Never touches other services, accounts or remotes.
set -u
setopt extendedglob
export GROUNDWORK_EVIDENCE_DIR=${GROUNDWORK_EVIDENCE_DIR:-${0:A:h:h}/tmp/factory-complete-client}
FACTORY_LIB=1 source "${0:A:h}/factory-client-smoke.sh"

unverified() { note "unverified: $1"; }
# GitHub/Linear transports are injected by factory-complete-service.ts (real startService);
# every connector call is logged here for assertions.
export FIXTURE_TRANSPORT_LOG=$EVIDENCE/state/transport.jsonl
# Controlled provider peer (harness owner's native-harness-peer.ts; provider comes from the
# file name). Seeded as the permission-run agent: harness custom, protocol jsonrpc-v2,
# args --fixture-case approve. Its wire log is $WORK/project/.fixture/native-wire.jsonl.
printf '#!%s\nawait import("%s/.groundwork/scripts/fixtures/native-harness-peer.ts");\n' "$(command -v bun)" "$ROOT" > "$WORK/native-custom"; chmod 700 "$WORK/native-custom"

# --- Journeys (filled from bounded discovery on the released build) ------------------------
# Discovery 2026-11-25 on client c2987e35… (window points, 1280×800):
#  General: Source GitHub 1086,532 · repository field 1040,686 · Save 1238,770
#  Sync tab 288,391 · Allow push 1162,474 · Fields: Status 507,682 (after scroll) · Load statuses 1118,639
#  Not planned → Blocked: 701,726 then 698,746 · Push "Todo"→Completed: 721,495 then 708,583
#  Task Push 1049,473 (relay 502 → "Retry same push" at same point)
#  Environment 310,359 · Apple container 1040,330 · Image 836,460 · Env names 836,666 · Readiness Check 1150,676 (after scroll)
#  Agents 294,263 · Add agent 534,492 · Harness 853,378 → Strands 810,497 · Provider 824,526 → OpenAI 792,617 ·
#   Credential 964,590 · Model 1076,378 · Check readiness 811,485 (after scroll)
#  Canvas context menu: right-click node · Parallel library item 302,224 · Join 302,262
#  App Settings 64,187 · Motion Reduce 1020,578 · System 953,578
#  Runs: Allow once 822,646 · Deny 895,646 · question Accept at the card header
# Each lists the GUI path and the assertions that make it count.

PERM_EXE=$WORK/native-custom
scroll_at() { hover $1 $2; local i; for i in $(seq $4); do scroll 0 $3; done; sleep 0.6; }
ocr_region() { shot $1; probe ocr "$EVIDENCE/shots/$1.png" $2 $3 $4 $5; }
cfg() { config_js "console.log(JSON.stringify($1))"; }
relay() { grep '"transport":"relay"' "$FIXTURE_TRANSPORT_LOG" 2>/dev/null | grep -- "$1"; }
open_project() { click 107 253; sleep 1; }
# open_task TITLE: quick open (⌘T) by title, Enter, then the Runs tab.
open_task() {
  keycode 53; click 120 600; sleep 0.3; key t "command down"; sleep 0.5; type_text "$1"
  local i; for i in $(seq 20); do sleep 0.5; shot _qo; [[ $(probe ocr "$EVIDENCE/shots/_qo.png" 260 180 760 160) != *Loading* ]] && break; done
  if [[ $(probe ocr "$EVIDENCE/shots/_qo.png" 260 180 760 160) == *Loading* ]]; then
    note "quick open still loading after 10s for '$1' (shots/qo-stuck.png); opening from the List tab"
    cp "$EVIDENCE/shots/_qo.png" "$EVIDENCE/shots/qo-stuck.png"; keycode 53; discard_if_dirty
    open_project; tap List 250 30 500 200; sleep 0.8; tap "$1" 330 280 700 500 || return 1; sleep 0.8
    click 1186 74; sleep 1.2      # peek → full task
  else keycode 36; sleep 1.5; fi
}
discard_if_dirty() { shot _bar; [[ $(probe ocr "$EVIDENCE/shots/_bar.png" 300 740 980 55) == *Unsaved* ]] && { click 120 600; click 1176 770; sleep 0.8; shot _bar2; }; }
project_tab() { click 334 20; sleep 0.8; }      # first tab is the project
settings_tab() { tap Settings 380 40 600 160; sleep 0.8; }
workflow_tab() { tap Workflow 380 40 600 160; sleep 1; }
# tap TEXT [X Y W H] [N]: click the Nth OCR line TEXT inside a window region (default whole window).
at_text() { shot _find; probe find "$EVIDENCE/shots/_find.png" ${2:-0} ${3:-0} ${4:-1280} ${5:-800} "$1" ${6:-1}; }
tap() { local p; p=$(at_text "$@"); [[ -n $p ]] || { cp "$EVIDENCE/shots/_find.png" "$EVIDENCE/shots/miss-${1// /_}.png"; note "control '$1' not found (shots/miss-${1// /_}.png)"; return 1; }; click ${=p}; sleep 0.4; }
# choose LABEL OPTION: open the picker on the row labelled LABEL and pick OPTION from its menu.
choose() {
  local row; row=$(at_text "$1" 480 240 300 560) || return 1; [[ -n $row ]] || { note "row '$1' not found"; return 1; }
  local y=${row#* }; click 700 $y; sleep 0.5; tap "$2" 600 $(( y - 220 )) 480 440
}

connected_setup() {
  open_project; settings_tab
  tap GitHub 700 200 580 600
  local rf; rf=$(at_text "GitHub repository" 470 200 500 600); [[ -n $rf ]] && { click 1050 ${rf#* }; type_text "owner/repo"; }; click 1238 770
  wait_until 5 eval '[[ $(api "/v1/projects/$PROJECT" "console.log(d.repository)") == owner/repo ]]' || { fail "GitHub repository not saved"; return; }
  tap Sync 250 150 900 300; sleep 0.8; shot C01-sync
  local ap; ap=$(at_text "Allow push" 470 200 500 600); [[ -n $ap ]] && click 1178 $(( ${ap#* } + 15 )); sleep 0.4
  scroll_at 800 600 -10 10
  tap Status 480 240 300 560                                   # add Status to the pushed fields
  click 1238 770; sleep 1.5
  tap "Load statuses" 1000 240 280 560; sleep 2.5; shot C02-statuses
  choose "Not planned" Blocked; click 1238 770; sleep 1.5
  scroll_at 800 600 -10 10; shot C03-mapping
  choose Done Completed; click 1238 770; sleep 1.5
  local i=$(cfg 'd.integration')
  note "integration after GUI setup: $i"
  [[ $i == *'"not_planned":"blocked"'* && $i == *'"done":"completed"'* && $i == *'"pushEnabled":true'* && $i == *'"status"'* && $i != *'"refresh":{"enabled":true'* ]] \
    && pass "GitHub mapping saved with named options; Status pushed; auto-pull off" || fail "integration not as configured: $i"
  choose "Not planned" Default || { keycode 53; fail "incoming mapping cannot be cleared: Not planned menu has no Default/unset option (shots/miss-Default.png)"; }
  click 1238 770; sleep 1.5
  [[ $(cfg 'd.integration.pullStatusMap ?? {}') != *not_planned* ]] && pass "incoming Not planned mapping cleared and saved" || fail "incoming mapping still saved: $(cfg 'd.integration.pullStatusMap')"
  choose Done "Don't change"; click 1238 770; sleep 1.5
  project_tab; settings_tab; tap Sync 250 150 900 300; sleep 1; shot C04-reopened
  [[ $(cfg 'd.integration.pullStatusMap ?? {}') != *not_planned* ]] && pass "cleared incoming mapping stays cleared after reopen" || fail "incoming mapping came back after reopen: $(cfg 'd.integration.pullStatusMap')"
  [[ $(cfg 'd.integration.pushStatusMap ?? {}') != *done* ]] && pass "cleared push mapping (Done → Don't change) stays cleared after reopen" || fail "cleared push mapping came back: $(cfg 'd.integration')"
  choose Done Completed; click 1238 770; sleep 1.5          # restore for the push below
  tap "Sync now" || { scroll_at 800 600 -10 10; tap "Sync now"; }; sleep 3
  local remote=$(api "/v1/tasks?projectId=$PROJECT" 'console.log(d.find(t=>t.external)?.id ?? "")')
  [[ -n $remote ]] || { fail "sync did not import the fixture issue"; return; }
  open_task "Remote issue"
  click 1100 143; sleep 0.6; shot C05-status-menu; click 1102 316; sleep 0.6; click 1238 770; sleep 1
  wait_until 5 eval '[[ $(api "/v1/tasks/$remote" "console.log(d.status)") == done ]]' || fail "task status not saved as done"
  click 1050 473; sleep 2; shot C06-push-lost
  click 1050 473; sleep 2; shot C07-push-retry
  local bodies=$(relay "/v1/tasks/$remote/push" | bun -e 'console.log(new Set((await Bun.stdin.text()).trim().split("\n").map(l=>JSON.stringify(JSON.parse(l).request))).size)')
  local writes=$(grep '"transport":"github"' "$FIXTURE_TRANSPORT_LOG" | grep -c '"method":"PATCH"')
  note "push relay requests: $(relay "/v1/tasks/$remote/push" | wc -l | tr -d ' ') distinct bodies=$bodies provider PATCH=$writes"
  [[ $bodies == 1 ]] && pass "retry resent the identical push body" || fail "retry body differed"
  [[ $writes == 1 ]] && grep '"method":"PATCH"' "$FIXTURE_TRANSPORT_LOG" | grep -q '"state":"closed"' && pass "exactly one provider write, closing the issue as completed" || fail "provider writes: $writes"
  linear_setup
}

linear_setup() {
  # Second project switched to Linear in the GUI; key env and team ENG are typed, catalog loads from the fixture fetch.
  local lp=$(curl -sf -X POST -H 'content-type: application/json' -d '{"name":"Linear fixture"}' $URL/v1/projects | bun -e 'console.log((await Bun.stdin.json()).data.id)')
  open_task "Linear fixture"; settings_tab; shot L01-general
  tap Linear 700 200 580 600; sleep 0.5; shot L02-linear
  local f; f=$(at_text "Linear source" 470 200 500 600); [[ -n $f ]] && { click 1050 ${f#* }; type_text ENG; }
  click 1238 770; sleep 1.5; tap Sync 250 150 900 300; sleep 0.8; shot L03-sync
  local k; k=$(at_text "LINEAR_API_KEY" 900 240 380 560); [[ -n $k ]] && { click ${=k}; key a "command down"; type_text KARABINER_TEST_LINEAR; click 1238 770; sleep 1.5; }
  tap "Load statuses" 1000 240 280 560; sleep 2.5; shot L04-statuses
  scroll_at 800 600 -10 6; choose "Review queue" "In review"; click 1238 770; sleep 1.5
  local c=$(api "/v1/projects/$lp/config" 'console.log(JSON.stringify(d.integration))')
  note "linear integration: $c (fixture transport, not live Linear)"
  [[ $c == *KARABINER_TEST_LINEAR* && $c == *state-review* ]] && pass "Linear named catalog state mapped and saved (fixture transport)" || unverified "Linear mapping: $c"
}

target_setup() {
  open_project; settings_tab; tap Environment 250 150 900 300; sleep 0.8
  click 1040 330; sleep 0.5                      # Apple container
  click 836 460; type_text "ghcr.io/fixture/agent:1"
  click 836 666; type_text "FIXTURE_TOKEN"
  scroll_at 800 600 -10 10
  click 1150 676; sleep 2; local r=$(ocr_region T01-readiness 480 640 720 100)
  [[ $r == *FIXTURE_TOKEN*"not set"* ]] && pass "draft readiness reports the missing env reference without launching" || fail "readiness text: $r"
  click 1238 770; sleep 1.5
  [[ $(cfg '[d.executionTarget,d.targetOptions]') == '["apple-container",{"image":"ghcr.io/fixture/agent:1","envNames":["FIXTURE_TOKEN"]}]' ]] \
    && pass "target fields saved without raw JSON" || fail "targetOptions: $(cfg 'd.targetOptions')"
  scroll_at 800 400 10 10; tap "Local host" 480 240 800 200; sleep 0.8; shot T02-local-host; click 1238 770; sleep 2; shot T03-local-host-saved   # back to Local host for runs
  [[ $(cfg "d.executionTarget") == '"host"' ]] || fail "Local host selection not saved: target $(cfg "d.executionTarget"); bar: $(bar_text T03-local-host-saved)"
}

agent_readiness() {
  open_project; settings_tab; tap Agents 250 150 900 300; sleep 0.8
  click 534 492; sleep 0.5                       # Add agent
  click 853 378; click 810 497                   # Harness → Strands
  click 824 526; click 792 617                   # Provider → OpenAI
  click 964 590; type_text "FIXTURE_OPENAI_KEY"
  click 1076 378; type_text "gpt-fixture"
  click 1238 770; sleep 1.5
  local a=$(cfg 'd.agents.at(-1)')
  [[ $a == *'"harness":"strands"'* && $a == *'"provider":"openai","credentialEnv":"FIXTURE_OPENAI_KEY"'* ]] && pass "Strands provider + credential reference saved" || fail "agent: $a"
  scroll_at 960 600 -10 5
  click 811 485; sleep 3; local r=$(ocr_region A01-readiness 740 460 460 120)
  [[ $r == *installed*"sign-in"* && $r == *FIXTURE_OPENAI_KEY* ]] && pass "readiness separates install, sign-in and credential" || fail "readiness text: $r"
}

pending_is() { [[ $(api "/v1/runs?taskId=$PERM_TASK" 'console.log(d[0]?.questions.find(q=>q.state==="pending")?.kind ?? d[0]?.state ?? "")') == $1 ]]; }
permission_run() {
  # Seed a peer agent + one-stage workflow (initial state); the run and answers are GUI actions.
  config_js "d.agents.push({id:'peer',name:'peer',role:'developer',harness:'custom',model:'fixture-local',executable:'$PERM_EXE',args:['--fixture-case','approve'],protocol:'jsonrpc-v2',mode:'interactive',skillIds:[],instructions:''}); d.workflows.push({id:'perm',name:'Permission',description:'',instructions:'',stages:[{id:'act',name:'act',instructions:'',agentIds:['peer'],returnToStageIds:[]}],graph:{entryStageId:'act',maxTransitions:2,edges:[]},layout:{positions:{act:{x:0,y:0}}}}); d.executionTarget='host'; const r=await fetch('$URL/v1/projects/$PROJECT/config',{method:'PUT',headers:{'content-type':'application/json'},body:JSON.stringify(d)}); console.log(r.status)" >/dev/null
  local t=$(curl -sf -X POST -H 'content-type: application/json' -d "{\"projectId\":\"$PROJECT\",\"title\":\"Permission fixture\",\"workflowId\":\"perm\",\"agentId\":\"peer\"}" $URL/v1/tasks | bun -e 'console.log((await Bun.stdin.json()).data.id)')
  open_task "Permission fixture"; shot P00-task; click 360 546; sleep 1
  tap "Start run" || { fail "Start run not found"; return; }
  PERM_TASK=$t
  wait_until 20 pending_is permission || { shot P01-no-permission; fail "no permission request (runs: $(api "/v1/runs?taskId=$t" 'console.log(JSON.stringify(d.map(r=>[r.state, JSON.stringify(r).match(/"message":"[^"]{0,160}/)?.[0]])))'))"; return; }
  sleep 1; shot P01-permission
  local p=$(probe ocr "$EVIDENCE/shots/P01-permission.png" 330 620 650 220)
  [[ $p == *"Allow once"*Deny* && $p == *tool-write* ]] && pass "permission card shows tool and choices" || fail "permission card: $p"
  tap "Allow once"; sleep 1.5                    # write
  [[ -f $WORK/project/effect-write.txt ]] && pass "Allow wrote the real fixture file" || fail "allowed effect missing"
  wait_until 10 pending_is permission; sleep 1; tap "Allow once"; sleep 1.5   # shell
  wait_until 10 pending_is permission; sleep 1; tap "Allow once"; sleep 1.5   # network
  wait_until 10 pending_is question || { fail "typed form not reached"; return; }
  sleep 1; shot P02-form
  scroll_at 600 600 -10 3
  local cf tg; cf=$(at_text Count 280 60 300 700 1); tg=$(at_text Tags 280 60 300 700 1)
  [[ -n $cf && -n $tg ]] || { fail "typed form fields not located"; return; }
  click 600 $(( ${cf#* } + 28 )); type_text "3"
  click 310 $(( ${tg#* } + 29 )); click 310 $(( ${tg#* } + 57 ))   # tags a, b (enabled stays false)
  shot P03-form-filled; tap Accept; sleep 2; shot P04-form-sent
  local dup; dup=$(probe ocr "$EVIDENCE/shots/P04-form-sent.png" 300 100 900 700)
  [[ $dup != *Decline*Approve* ]] && pass "no generic approval rail beside the question" || fail "approval rail still shown: $dup"
  local a=$(relay "/v1/runs/" | grep '"value"' | tail -1)
  note "form answer relay: ${a:0:400}"
  grep -q '"form-response"' "$WORK/project/.fixture/native-wire.jsonl" && grep '"form-response"' "$WORK/project/.fixture/native-wire.jsonl" | grep -q '"count":3,"enabled":false,"tags":\["a","b"\]' \
    && pass "peer received count 3, enabled false, tags [a,b]" || fail "typed form not delivered (relay: ${a:0:200})"
  # Free-text question: answer "Answer exactly" in the run card.
  wait_until 10 pending_is question; sleep 1; scroll_at 600 400 10 6; shot P05a-text-question
  local an; an=$(at_text Answer 280 100 700 690 1); [[ -n $an ]] && { click 600 $(( ${an#* } + 28 )); type_text "Answer exactly"; tap Send 800 60 480 700; sleep 1.5; }
  grep -q '"question-response"' "$WORK/project/.fixture/native-wire.jsonl" && pass "free-text question answered exactly in the GUI" || fail "text question not answered (shots/P05a-text-question.png)"
  # Live guidance: the peer waits for an operator instruction after the form; Add instruction delivers it to the session.
  sleep 1; tap "Add instruction" 600 500 500 300 && { sleep 0.6; shot P06-guidance-input; type_text "Fixture guidance: finish now"; keycode 36; sleep 2; shot P07-guidance-sent; }
  wait_until 20 eval '[[ $(api "/v1/runs?taskId=$PERM_TASK" "console.log(d[0].state)") == completed ]]' \
    && grep -q guidance "$WORK/project/.fixture/native-wire.jsonl" && pass "live guidance reached the running session; run completed" \
    || fail "guidance not delivered (run $(api "/v1/runs?taskId=$PERM_TASK" 'console.log(d[0].state)'); shots/P06-P07)"
  # Deny: a second run of the same peer; denying the write must leave no file and block the run.
  rm -f "$WORK/project/effect-write.txt"
  t=$(curl -sf -X POST -H 'content-type: application/json' -d "{\"projectId\":\"$PROJECT\",\"title\":\"Denycheck\",\"workflowId\":\"perm\",\"agentId\":\"peer\"}" $URL/v1/tasks | bun -e 'console.log((await Bun.stdin.json()).data.id)'); PERM_TASK=$t
  open_task "Denycheck"; click 360 546; sleep 1; tap "Start run" || { fail "Start run not found (deny)"; return; }
  wait_until 20 pending_is permission || { fail "no permission request (deny)"; return; }
  sleep 1; tap Deny 800 560 400 240; sleep 2; shot P05-denied
  [[ ! -f $WORK/project/effect-write.txt ]] && wait_until 10 eval '[[ $(api "/v1/runs?taskId=$t" "console.log(d[0].state)") == failed ]]' \
    && pass "Deny left no fixture file and blocked the run" || fail "deny effect/state: file $([[ -f $WORK/project/effect-write.txt ]] && echo present || echo absent), run $(api "/v1/runs?taskId=$t" 'console.log(d[0].state)')"
}

# frames_differ NAME X Y W H: two captures of a window region 120ms apart differ.
frames_differ() {
  find_window
  screencapture -x -R$(( WX + $2 )),$(( WY + $3 )),$4,$5 "$EVIDENCE/shots/$1-a.png"; sleep 0.12
  screencapture -x -R$(( WX + $2 )),$(( WY + $3 )),$4,$5 "$EVIDENCE/shots/$1-b.png"
  ! cmp -s "$EVIDENCE/shots/$1-a.png" "$EVIDENCE/shots/$1-b.png"
}
# hold_run TITLE: start a stream-hold run (prints progress until released) and open its Runs tab.
hold_run() {
  local t=$(curl -sf -X POST -H 'content-type: application/json' -d "{\"projectId\":\"$PROJECT\",\"title\":\"$1\",\"workflowId\":\"hold\",\"agentId\":\"stream-hold\"}" $URL/v1/tasks | bun -e 'console.log((await Bun.stdin.json()).data.id)')
  HOLD_RID=$(curl -sf -X POST -H 'content-type: application/json' -d '{}' $URL/v1/tasks/$t/run | bun -e 'console.log((await Bun.stdin.json()).data.id)')
  wait_until 15 test -f "$WORK/project/.fixture/$HOLD_RID/ready.json" || { fail "$1 run did not start: $(api "/v1/runs/$HOLD_RID" 'console.log(d.state)')"; return 1; }
  open_task "$1"; click 360 546; sleep 1.2
}
release_hold() { touch "$WORK/project/.fixture/$HOLD_RID/release"; wait_until 10 eval '[[ $(api "/v1/runs/$HOLD_RID" "console.log(d.state)") == completed ]]'; }
# spinner_check NAME EXPECT(moves|static): observe the running step's spinner while the run is confirmed running.
spinner_check() {
  shot $1; local sp; sp=$(at_text Running 800 560 300 240); [[ -n $sp ]] || { fail "$1: running step not visible"; return; }
  [[ $(api "/v1/runs/$HOLD_RID" 'console.log(d.state)') == running ]] || { fail "$1: run not running at observation"; return; }
  if frames_differ $1 280 $(( ${sp#* } - 12 )) 40 24; then [[ $2 == moves ]]; else [[ $2 == static ]]; fi
}
set_motion() { click 64 187; sleep 1; tap "$1" 900 540 380 80; sleep 1; [[ $(api /v1/settings 'console.log(d.motion)') == ${(L)1} ]]; }
motion_setting() {
  hold_run "Motion system" || return
  spinner_check M01-system moves && pass "running spinner animates with Motion System" || note "spinner static under System (Mac may already reduce motion)"
  release_hold
  set_motion Reduce && pass "Motion Reduce persisted" || fail "motion not reduce"
  hold_run "Motion reduce" || return
  spinner_check M03-reduce static && pass "with Reduce the running spinner is static" || fail "spinner still animates under Reduce (shots/M03-reduce-a/b)"
  release_hold
  set_motion Full && pass "Motion Full persisted" || fail "motion not full"
  hold_run "Motion full" || return
  spinner_check M04-full moves && pass "with Full the spinner animates again" || fail "spinner static under Full while the run is running"
  release_hold
  set_motion System && pass "Motion System restored" || fail "motion not system"
}

wf() { cfg "d.workflows.find(w=>w.id==='delivery')$1"; }
select_fork() { local r; r=$(select_stage Parallel) && [[ -n $r ]] && FORK_AT="$(( ${${=r}[1]} + ${${=r}[3]} / 2 )) $(( ${${=r}[2]} + ${${=r}[4]} / 2 ))"; }
# add_branch STAGE: Add branch opens a stage menu below it (above when near the bottom).
add_branch() {
  local ab; ab=$(at_text "Add branch" 960 140 320 600); [[ -n $ab ]] || { scroll_at 1100 450 -10 3; ab=$(at_text "Add branch" 960 140 320 600); }
  [[ -n $ab ]] || { tap "Add branch" 960 140 320 600; return 1; }
  click ${=ab}; sleep 0.5; shot _menu
  local y=${ab#* } m; m=$(probe find "$EVIDENCE/shots/_menu.png" 960 $(( y + 8 )) 320 $(( 790 - y )) "$1")
  [[ -z $m ]] && m=$(probe find "$EVIDENCE/shots/_menu.png" 960 0 320 $(( y - 8 )) "$1")
  [[ -n $m ]] && click ${=m} || note "branch option $1 not found"; sleep 0.5
}
field_below() { local l; l=$(at_text "$1" 960 100 320 700 1); [[ -n $l ]] && { click ${l% *} $(( ${l#* } + 28 )); key a "command down"; type_text "$2"; }; }
parallel_authoring() {
  open_project; workflow_tab; sleep 0.5; keycode 53; discard_if_dirty; click 489 736; sleep 0.6; click 900 600; sleep 0.3; shot F00-canvas
  local stages0=$(wf '.stages.length')
  tap Parallel 240 150 300 250; sleep 1
  local x y w h; read x y w h <<< "$(node_box F01-parallel-added)"; FORK_AT="$(( x + w / 2 )) $(( y + h / 2 ))"; note "fork node rendered at $x,$y ${w}x$h"
  add_branch review; select_fork; add_branch decision; select_fork; shot F02-branches
  [[ $(probe ocr "$EVIDENCE/shots/F02-branches.png" 960 500 320 300) == *decision* ]] || { add_branch decision; select_fork; shot F02-branches; }
  select_fork; scroll_at 1100 600 -10 10; tap Limits 900 100 380 700; sleep 0.5; scroll_at 1100 600 -10 10
  field_below "Retries per step" 2; field_below "Retry delay (s)" 1; keycode 48; sleep 0.4; shot F03-retry
  click 1018 99; sleep 0.6; shot F04-undo
  local u=$(probe ocr "$EVIDENCE/shots/F04-undo.png" 960 560 320 120)
  click 1054 99; sleep 0.6; shot F05-redo
  local r=$(probe ocr "$EVIDENCE/shots/F05-redo.png" 960 560 320 120)
  note "retry fields after undo: $u | after redo: $r"
  [[ $u != *" 1"* && $r == *1* ]] && pass "undo reverted the typed retry delay as one step; redo restored it" || fail "undo/redo of form edit (undo: $u, redo: $r)"
  click 1238 770; sleep 1.5
  local fork=$(wf '.graph.forks?.[0]') fs=$(wf '.graph.forks?.[0]?.stageId')
  note "fork after save: $fork"
  [[ $fork == *'"entryStageId":"review"'* && $fork == *'"entryStageId":"decision"'* && $fork == *joinStageId* ]] && pass "fork with two branches and a join saved" || fail "fork not saved as authored: $fork"
  local ex=$(wf ".stages.find(s=>s.id===$fs)?.execution")
  [[ $ex == *'"retries":2'* && $ex == *'"retryDelayMs":1000'* ]] && pass "retry count 2 / delay 1s saved from the inspector" || fail "retry fields: $ex"
  # Clipboard: select the fork node on the canvas, copy, paste → a fresh stage id.
  select_fork
  local before=$(wf '.stages.map(s=>s.id).join(",")')
  key c "command down"; key v "command down"; sleep 0.8; shot F06-paste; click 1238 770; sleep 1.5
  local after=$(wf '.stages.map(s=>s.id).join(",")')
  note "stages before paste: $before | after: $after"
  (( ${#${(s:,:)after}} > ${#${(s:,:)before}} )) && pass "paste added stages with fresh ids" || fail "copy/paste added no stage"
  local forks=$(wf '.graph.forks')
  note "forks after paste: $forks"
  [[ $(wf '.graph.forks.length') == 2 && $(cfg 'new Set(d.workflows.find(w=>w.id==="delivery").graph.forks.flatMap(f=>[f.id,f.stageId,f.joinStageId])).size') == 6 ]] \
    && pass "pasted parallel region is a second fork with fresh fork/stage/join ids" || fail "pasted fork ids not fresh: $forks"
  # Pasted region is disconnected until linked: the only issues allowed are unreachable stages that
  # are the GUI-added fork/join or the pasted copy (never original stages, missing refs or remaps).
  discard_if_dirty; local rev=$(revision)
  shot F07-before-validate; clean_bar F07-before-validate || { fail "draft still unsaved before Validate"; return; }
  tap Validate 1100 80 180 40; sleep 2.5; shot F07-validate
  curl -sf -X POST -H 'content-type: application/json' -d '{}' $URL/v1/projects/$PROJECT/factory/validate > "$EVIDENCE/state/validate-after-paste.json"
  local v=$(bun -e 'const d=(await Bun.file(process.argv[1]).json()).data, orig=new Set(["build","check","repair","review","decision"]);
    const bad=d.errors.filter(e=>e.code!=="unreachable_stage" || orig.has(/stage (\S+)/.exec(e.message)?.[1]));
    console.log(bad.length ? "bad "+JSON.stringify(bad) : "ok "+d.errors.length)' "$EVIDENCE/state/validate-after-paste.json")
  note "validate revision $rev: $v; chip: $(probe ocr "$EVIDENCE/shots/F07-validate.png" 780 80 320 40)"
  [[ $v == ok* ]] && pass "disconnected copy: references preserved, only unreachable new stages reported ($v)" || fail "validation issues beyond the disconnected copy: $v"
}
clean_bar() { [[ $(probe ocr "$EVIDENCE/shots/$1.png" 300 740 980 55) != *Unsaved* ]]; }
viewport_keyboard() {
  open_project; workflow_tab; sleep 0.5
  local rev=$(revision) pos0=$(cfg "JSON.stringify(d.workflows.find(w=>w.id==='delivery').layout)")
  click 489 736; sleep 0.4; for i in 1 2 3 4; do click 489 684; done; scroll_at 640 450 -5 3; shot K01-camera
  clean_bar K01-camera && [[ $(revision) == $rev ]] && pass "zoom/pan changed the view without dirtying the factory" || fail "camera change dirtied the draft"
  click 489 736; sleep 0.6                      # fit view so stage labels are legible
  local ck; ck=$(at_text check 470 120 480 600); [[ -n $ck ]] && click ${=ck} || fail "check stage label not found on canvas"; sleep 0.5
  local x y w h; read x y w h <<< "$(node_box K02b-mouse-select)"
  if [[ -n ${x:-} ]]; then
    local cx=$(( x + w / 2 )) cy=$(( y + h / 2 ))
    click 489 762; sleep 0.4; shot K03-locked
    cliclick dd:$(at $cx $cy) w:150 dm:$(at $(( cx + 40 )) $(( cy + 30 ))) w:300 du:$(at $(( cx + 40 )) $(( cy + 30 ))); sleep 0.6; shot K04-locked-drag
    clean_bar K04-locked-drag && pass "locked canvas refused the node drag" || fail "drag changed the factory while locked"
    click 489 762; sleep 0.4                     # unlock
    # Esc aborts an in-progress drag.
    cliclick dd:$(at $cx $cy) w:150 dm:$(at $(( cx + 40 )) $(( cy + 30 ))) w:300; keycode 53; sleep 0.3; cliclick du:$(at $(( cx + 40 )) $(( cy + 30 ))); sleep 0.6; shot K05-esc-drag
    local after; read after <<< "$(node_box K05-esc-drag)"
    note "after Esc drag: ${after:-none} (start $x,$y)"
    clean_bar K05-esc-drag && [[ ${after%% *} == $x ]] && pass "Esc aborted the drag and restored the node" || fail "Esc did not abort the drag ($after vs $x,$y)"
    # Edge auto-pan: hold the node at the canvas edge; the view pans so it travels further than the pointer.
    local p0=$(wf ".layout.positions")
    cliclick dd:$(at $cx $cy) w:150 dm:$(at 470 $cy) w:150 dm:$(at 466 $cy) w:1500 du:$(at 466 $cy); sleep 0.6; shot K06-edge-pan
    click 1238 770; sleep 1.5
    note "positions before edge drag $p0 after $(wf '.layout.positions')"
    local moved=$(wf '.layout.positions.check.x') plain=$(( 260 - (cx - 466) * 150 / w ))
    note "edge drag: pointer moved $(( 466 - cx ))pt (plain ≈ x $plain at zoom $w/150); saved x $moved"
    (( moved < plain - 100 )) && pass "holding a dragged node at the canvas edge auto-panned (x $moved vs $plain without pan)" || fail "no edge auto-pan (x $moved vs $plain)"
  fi
  # Keyboard (React Flow): Tab moves a focus ring and reveals the node without selecting; Enter selects; Esc leaves.
  discard_if_dirty; click 489 736; sleep 0.5; click 900 600; sleep 0.3; shot K02a-before-tab
  keycode 48; sleep 0.5; shot K02-tab-focus
  local sel_after_tab; sel_after_tab=$(probe box "$EVIDENCE/shots/K02-tab-focus.png" 300 120 700 640 ${=PINK})
  local canvas_changed=no c
  for c in K02a-before-tab K02-tab-focus; do sips -c 1200 960 --cropOffset 260 940 "$EVIDENCE/shots/$c.png" --out "$WORK/$c-canvas.png" >/dev/null; done
  cmp -s "$WORK/K02a-before-tab-canvas.png" "$WORK/K02-tab-focus-canvas.png" || canvas_changed=yes
  local insp=$(probe ocr "$EVIDENCE/shots/K02-tab-focus.png" 960 130 320 100)
  [[ $canvas_changed == yes && $insp == Workflow* ]] && clean_bar K02-tab-focus && pass "Tab drew a canvas focus ring without selecting or adding a stage" || fail "Tab: canvas changed=$canvas_changed, inspector '${insp:0:40}', selection '${sel_after_tab:-none}' (shots/K02-tab-focus.png)"
  keycode 36; sleep 0.6
  local kb; kb=$(node_box K03-enter-select); insp=$(probe ocr "$EVIDENCE/shots/K03-enter-select.png" 960 130 320 100)
  [[ -n $kb && $insp == *Stage* ]] && clean_bar K03-enter-select && pass "Enter selected the focused stage (rect $kb)" || fail "Enter did not select the focused stage (inspector '${insp:0:40}')"
  keycode 53; sleep 0.3; keycode 53; sleep 0.3
  # Inspector Tab is ordinary field focus: from Name, Tab then typing lands in Instructions.
  click 120 600; sleep 0.2; local st; st=$(at_text check 360 120 600 600); [[ -n $st ]] && click ${=st}; sleep 0.4
  local nm; nm=$(at_text Name 960 120 320 200 1); [[ -n $nm ]] && { click 1100 $(( ${nm#* } + 28 )); keycode 48; type_text "Q"; sleep 0.4; shot K04-inspector-tab; }
  local o=$(probe ocr "$EVIDENCE/shots/K04-inspector-tab.png" 960 120 320 260)
  [[ $o == *check* && $o != *checkQ* && $o == *Q* ]] && pass "inspector Tab moved from Name to Instructions" || fail "inspector Tab focus: '${o:0:120}'"
  keycode 53; discard_if_dirty
}
fork_run() {
  # Seeded workflow (initial state): fork → branches a (agent) and b (command failing once, retries 1) → join.
  local fail_once="const f=require('fs'),p='.fixture/flaky';if(!f.existsSync(p)){f.mkdirSync('.fixture',{recursive:true});f.writeFileSync(p,'1');console.error('first attempt fails');process.exit(3)}console.log('second attempt ok')"
  export FAIL_ONCE=$fail_once; config_js "d.workflows.push({id:'forkrun',name:'Fork run',description:'',instructions:'',stages:[{id:'split',name:'split',instructions:'',agentIds:[],returnToStageIds:[],kind:'fork'},{id:'a',name:'branch a',instructions:'',agentIds:['builder'],returnToStageIds:[]},{id:'b',name:'branch b',instructions:'',agentIds:[],returnToStageIds:[],execution:{commands:[{id:'flaky',name:'flaky',executable:process.execPath,args:['-e',process.env.FAIL_ONCE],timeoutMs:10000}],retries:1,retryDelayMs:200}},{id:'merge',name:'merge',instructions:'',agentIds:[],returnToStageIds:[],kind:'join'}],graph:{entryStageId:'split',maxTransitions:8,forks:[{id:'fx',stageId:'split',branches:[{id:'ba',entryStageId:'a'},{id:'bb',entryStageId:'b'}],joinStageId:'merge'}],edges:[{id:'a-m',sourceStageId:'a',targetStageId:'merge',on:'success'},{id:'b-m',sourceStageId:'b',targetStageId:'merge',on:'success'}]},layout:{positions:{split:{x:0,y:0},a:{x:-120,y:140},b:{x:120,y:140},merge:{x:0,y:280}}}}); const r=await fetch('$URL/v1/projects/$PROJECT/config',{method:'PUT',headers:{'content-type':'application/json'},body:JSON.stringify(d)}); console.log(r.status, (await r.text()).slice(0,300))" > "$EVIDENCE/state/forkrun-seed.txt"
  [[ $(head -c 3 "$EVIDENCE/state/forkrun-seed.txt") == 200 ]] || { fail "fork run seed rejected: $(cat "$EVIDENCE/state/forkrun-seed.txt")"; return; }
  local t=$(curl -sf -X POST -H 'content-type: application/json' -d "{\"projectId\":\"$PROJECT\",\"title\":\"Forkrun\",\"workflowId\":\"forkrun\"}" $URL/v1/tasks | bun -e 'console.log((await Bun.stdin.json()).data.id)')
  open_task Forkrun; click 360 546; sleep 1; tap "Start run" || { fail "Start run not found (fork)"; return; }
  wait_until 40 eval '[[ $(api "/v1/runs?taskId=$t" "console.log(d[0]?.state)") == completed || $(api "/v1/runs?taskId=$t" "console.log(d[0]?.state)") == failed ]]'
  sleep 2; shot H01-fork-run; scroll_at 600 600 -10 6; shot H02-fork-history
  local run=$(api "/v1/runs?taskId=$t" 'const r=d[0]; console.log(JSON.stringify({state:r.state, steps:(r.stages??[]).map(s=>[s.stageId,s.state,s.branchId??s.branch??null,(s.attempts??[]).length])}))')
  local ui=$(probe ocr "$EVIDENCE/shots/H01-fork-run.png" 300 560 980 240)" | "$(probe ocr "$EVIDENCE/shots/H02-fork-history.png" 300 100 980 700)
  note "fork run: $run"; note "fork run UI: ${ui:0:600}"
  [[ $run == *'"state":"completed"'* ]] && pass "fork run completed after branch b retried" || fail "fork run: $run"
  [[ $ui == *"branch a"* && $ui == *"branch b"* ]] && pass "both branches shown in the run card" || fail "branches not both shown (shots/H01-H02)"
  [[ $ui == *"Attempt 2"* || $ui == *"attempt 2"* || $ui == *"2 attempts"* ]] && pass "retry attempt history shows the second attempt" || fail "attempt history not visible (shots/H02-fork-history.png)"
  resize 900 800; sleep 0.8; shot N05-narrow-run-history; resize 1280 800
}

# diag_trace LABEL: saved stage ids/count and draft bar at a step (diagnostic runs only).
diag_trace() { shot D-$1; note "trace $1: saved $(wf '.stages.map(s=>s.id).join(",")') | bar: $(probe ocr "$EVIDENCE/shots/D-$1.png" 300 740 980 55)"; }
diag_paste() {
  diag_trace start; horizontal_connect; diag_trace after-horizontal; return_multiselect; diag_trace after-returns
  viewport_keyboard; diag_trace after-keyboard
  parallel_authoring; diag_trace after-paste
  curl -sf -X POST -H 'content-type: application/json' -d '{}' $URL/v1/projects/$PROJECT/factory/validate > "$EVIDENCE/state/validate-after-paste.json"
  snapshot config-after-paste "/v1/projects/$PROJECT/config"
  local t0=$(ms); quick_open_after_edit; note "quick open observed for $(( $(ms) - t0 ))ms"
}
# Read-only sample of the owned app while the baseline palette is Loading (no edits).
qo_sample() {
  local title=$(api "/v1/tasks/$TASK" 'console.log(d.title)')
  open_project; workflow_tab; keycode 53; click 900 500; sleep 0.4
  find_window; cliclick c:$(( WX + 640 )),$(( WY + 16 )); front; keycode 53; sleep 0.3
  key t "command down"; sleep 0.8; type_text "$title"
  local i t; for i in $(seq 12); do sleep 0.25; shot S01-palette; t=$(probe ocr "$EVIDENCE/shots/S01-palette.png" 270 165 740 150); [[ $t == *Loading* ]] && break; done
  [[ $t == *Loading* ]] || { note "palette not Loading (results: ${t:0:120}); no sample taken"; return; }
  sleep 3; shot S02-loading-3s; note "sample start $(ms)"
  ps -o pid,%cpu,state,time -p "$APP_PID" > "$EVIDENCE/ps-before.txt"
  sample "$APP_PID" 2 -file "$EVIDENCE/sample-loading.txt" > /dev/null 2>&1
  ps -o pid,%cpu,state,time -p "$APP_PID" > "$EVIDENCE/ps-after.txt"; note "sample end $(ms)"
  shot S03-after-sample; note "after sample: $(probe ocr "$EVIDENCE/shots/S03-after-sample.png" 270 165 740 150)"
  keycode 53
}
# Baseline quick open → one edit + Save → Validate → quick open, each waited on a visible state.
quick_open_cycle() {
  local title=$(api "/v1/tasks/$TASK" 'console.log(d.title)') stages0=$(wf '.stages.map(s=>s.id).join(",")')
  open_project; workflow_tab; keycode 53; click 900 500; sleep 0.4   # empty canvas → Workflow inspector
  quick_open_after_edit "$title"
  # Edit the WORKFLOW instructions only after the inspector positively shows the Workflow entity.
  # Inspector title row only (right panel, directly under the toolbar): exact line "Workflow".
  shot Q05-inspector; local hdr=$(probe find "$EVIDENCE/shots/Q05-inspector.png" 960 130 320 40 Workflow)
  [[ -n $hdr && $(probe ocr "$EVIDENCE/shots/Q05-inspector.png" 960 130 320 40) == (*" | ")#Workflow(" | "*)# ]] \
    || { fail "Workflow inspector title not confirmed ('$(probe ocr "$EVIDENCE/shots/Q05-inspector.png" 960 130 320 40)'); stopping before any edit"; return 1; }
  local ins; ins=$(at_text Instructions 960 140 320 300 1); [[ -n $ins ]] || { fail "Instructions field not found; stopping"; return 1; }
  local rev=$(revision) t0
  click 1100 $(( ${ins#* } + 32 )); type_text "Quick open timing note"
  t0=$(ms); click 1238 770
  wait_until 5 revision_above $rev || { fail "Save not accepted"; return 1; }
  local i; for i in $(seq 20); do sleep 0.25; shot Q06-saved; clean_bar Q06-saved && break; done
  note "save: accepted, bar clean after $(( $(ms) - t0 ))ms"
  clean_bar Q06-saved && pass "Save completed visibly" || fail "Save still pending in the UI (Q06)"
  [[ $(wf '.instructions') == *"Quick open timing note"* && $(wf '.stages.map(s=>s.id).join(",")') == $stages0 ]] \
    && pass "workflow instructions saved; stage identities unchanged" || { fail "unexpected saved change: $(wf '.instructions') / $(wf '.stages.map(s=>s.id).join(",")')"; return 1; }
  t0=$(ms); tap Validate 1100 80 180 40
  local v; for i in $(seq 40); do sleep 0.25; shot Q10-validate; v=$(probe ocr "$EVIDENCE/shots/Q10-validate.png" 780 80 320 40); [[ $v != *Validating* ]] && break; done
  note "validate visible result '$v' after $(( $(ms) - t0 ))ms"
  [[ ( $v == *alid* || -n $(probe box "$EVIDENCE/shots/Q10-validate.png" 1000 85 120 30 70 160 90) ) && $v != *Validating* ]] && pass "Validate result shown ($v)" || fail "Validate pending or wrong ('$v')"
  quick_open_after_edit "$title"
}
# Quick open after edits: distinguish not opened / opened+loading / error / ready.
quick_open_after_edit() {
  local q=${1:-Remote issue}
  shot Q00-before; note "before quick open: toolbar '$(probe ocr "$EVIDENCE/shots/Q00-before.png" 1100 120 500 80)' bar '$(bar_text Q00-before)'"
  find_window; cliclick c:$(( WX + 640 )),$(( WY + 16 )); front; keycode 53; sleep 0.3   # own titlebar, close menus
  key t "command down"; sleep 0.8; shot Q01-after-cmd-t
  local o=$(probe ocr "$EVIDENCE/shots/Q01-after-cmd-t.png" 200 60 900 300)
  if [[ $o != *"Open project"* && $o != *Loading* ]]; then
    note "Cmd-T did not open the palette (top area: ${o:0:120}); trying the titlebar + button"
    click 1170 20; sleep 0.8; shot Q02-after-plus; o=$(probe ocr "$EVIDENCE/shots/Q02-after-plus.png" 200 60 900 300)
    [[ $o == *"Open project"* || $o == *Loading* ]] && fail "quick open: Cmd-T not received; + button opens the palette" || { fail "quick open did not open by Cmd-T or + (state: not opened; shots/Q01, Q02)"; return; }
  fi
  type_text "$q"; local i t; for i in $(seq 20); do sleep 0.5; shot Q03-palette; t=$(probe ocr "$EVIDENCE/shots/Q03-palette.png" 270 165 740 150); [[ $t != *Loading* ]] && break; done   # results area only, below the search field
  ps -o pid,%cpu,state,time -p "$APP_PID" >> "$EVIDENCE/ps-palette.txt"; note "palette result after $(( i * 500 ))ms"
  if [[ $t == *Loading* ]]; then fail "quick open opened but stayed Loading 10s (Q03)"
  elif [[ $t == *rror* || $t == *ailed* ]]; then fail "quick open showed an error: ${t:0:160}"
  elif [[ $t == *"$q"* && $t != *"No match"* ]]; then pass "quick open listed a '$q' result row"
  else fail "quick open opened with no matching result: ${t:0:160}"; fi
  keycode 53; discard_if_dirty
}
responsive_review() {
  resize 900 800; sleep 0.8
  open_project; workflow_tab; sleep 0.5; shot N01-narrow-workflow
  local t=$(probe ocr "$EVIDENCE/shots/N01-narrow-workflow.png" 300 80 600 60)
  [[ $t == *Validate* ]] && pass "narrow workflow toolbar keeps Validate visible" || fail "narrow toolbar: $t"
  settings_tab; tap Sync 250 150 900 300; sleep 1; shot N02-narrow-sync
  open_task "Permission fixture"; click 360 546; sleep 1; shot N03-narrow-run
  resize 1280 800; sleep 0.8; open_project; workflow_tab; sleep 0.5; shot N04-normal-workflow
  note "narrow/normal captures for review: N01–N04"
}

# select_stage NAME: Tab the canvas focus ring until the stage NAME is focused, Enter to select it;
# prints its rendered rectangle (from the pink selection border). No label OCR on the canvas.
# select_stage NAME: at the current (readable) zoom, click the stage's label once and confirm the
# inspector's Name field shows NAME; prints the pink selection rectangle. No keyboard cycling.
select_stage() {
  local l; l=$(at_text "$1" 365 100 590 690); [[ -n $l ]] || { note "stage label $1 not visible"; return 1; }
  click ${=l}; sleep 0.5; shot _sel
  local n=$(probe find "$EVIDENCE/shots/_sel.png" 960 100 320 260 "$1")
  [[ -n $n && $(probe ocr "$EVIDENCE/shots/_sel.png" 960 130 320 40) == (*" | ")#Stage(" | "*)# ]] || { note "inspector does not show stage $1"; return 1; }
  probe box "$EVIDENCE/shots/_sel.png" 360 100 600 680 ${=PINK}
}
# zoom_at X Y TICKS: wheel-zoom in at a window point (zoom follows the pointer).
zoom_at() { hover $1 $2; local i; for i in $(seq $3); do osascript -l JavaScript -e "ObjC.import('CoreGraphics'); \$.CGEventPost(0, \$.CGEventCreateScrollWheelEvent(null,1,1,10));"; sleep 0.1; done; sleep 0.5; }
# --- Direct visual scope (form layout, return multiselect, horizontal canvas) ------------------
# at_widths NAME CMD [WIDTHS]: run CMD (a navigation function) and capture at each width (default 1280 900).
at_widths() { local w; for w in ${=3:-1280 900}; do resize $w 800; sleep 0.8; $2; sleep 0.8; shot $1-$w; done; resize 1280 800; sleep 0.6; }
# unclipped NAME TEXT: TEXT is read whole (no ellipsis/cut) in each width capture of NAME.
unclipped() { local w o bad=; for w in ${=3:-1280 900}; do o=$(probe ocr "$EVIDENCE/shots/$1-$w.png" 0 0 $w 800); [[ $o == *"$2"* ]] || bad+=" $w"; done
  [[ -z $bad ]] && pass "$1: '$2' fully visible at ${3:-1280 900}" || fail "$1: '$2' clipped or missing at$bad (shots/$1-*.png)"; }
nav_env() { open_project; settings_tab; tap Environment 250 150 900 300; sleep 0.8; }
nav_agents() { open_project; settings_tab; tap Agents 250 150 900 300; sleep 0.8; tap stream-hold 280 200 700 560; sleep 0.6; }
nav_sync() { open_project; settings_tab; tap Sync 250 150 900 300; sleep 0.8; }
nav_inspector() { open_project; workflow_tab; click 489 736; sleep 0.5; local c; c=$(at_text ${1:-check} 360 120 600 600); [[ -n $c ]] && click ${=c}; sleep 0.6; }
form_layout_review() {
  at_widths V01-environment nav_env "1280 1134 900"; unclipped V01-environment "Not available here" "1280 1134 900"
  at_widths V02-agent nav_agents;      unclipped V02-agent "Custom executable"
  at_widths V03-sync nav_sync;         unclipped V03-sync "Connection"
  at_widths V04-inspector nav_inspector
  local o=$(probe ocr "$EVIDENCE/shots/V04-inspector-1280.png" 900 100 380 700)
  [[ $o == *Routes* && $o == *Limits* ]] && pass "inspector groups fields under section headers (Routes, Limits)" || fail "inspector sections: ${o:0:160}"
  scroll_at 1100 500 -10 5; tap Limits 900 100 380 700 && { sleep 0.5; shot V05-inspector-limits-open; }
  note "review V01–V05 for single border layer per group, aligned label/control rows, chips on their own line"
}
ret() { wf ".stages.find(s=>s.id==='decision').returnToStageIds"; }
# toggle_returns A B: open decision's return multiselect and toggle stages A and B.
toggle_returns() {
  local r; r=$(at_text "Can return to" 900 100 380 700) || true; [[ -n $r ]] || { scroll_at 1100 450 -10 3; r=$(at_text "Can return to" 900 100 380 700); }
  [[ -n $r ]] || { fail "Can return to control not found"; return 1; }
  click 1150 $(( ${r#* } + 28 )); sleep 0.6; shot R01-return-menu-open
  pick_option $1; pick_option $2; keycode 53; sleep 0.4
}
# pick_option NAME: click the exact option row below the popup's "Find a stage" search field.
pick_option() {
  local f; f=$(at_text "Find a stage" 900 100 380 700); [[ -n $f ]] || { fail "return popup not open"; return 1; }
  local y=$(( ${f#* } + 12 )) o; o=$(probe find "$EVIDENCE/shots/_find.png" 960 $y 300 $(( 790 - y )) "$1")
  [[ -n $o ]] || { fail "option $1 not found in popup"; return 1; }
  click ${=o}; sleep 0.4; shot R-pick-$1
}
return_multiselect() {
  [[ $(ret) == '[]' ]] || { fail "decision has unexpected initial returns: $(ret)"; return; }
  nav_inspector decision; toggle_returns build check || return; shot R02-return-selected; click 1238 770; sleep 1.5
  [[ $(ret) == *build* && $(ret) == *check* ]] && pass "decision returns to build and check from the multiselect: $(ret)" || fail "returns saved: $(ret)"
  project_tab; nav_inspector decision; scroll_at 1100 450 -10 3; shot R03-reopened
  local o=$(probe ocr "$EVIDENCE/shots/R03-reopened.png" 900 100 380 700); note "return summary after reopen: ${o:0:200}"
  [[ $o == *"2 stages"* || ( $o == *build* && $o == *check* ) ]] && pass "selected returns summarised after reopen" || fail "return selection not shown after reopen (R03)"
  local r; r=$(at_text "Can return to" 900 100 380 700); click 1150 $(( ${r#* } + 28 )); sleep 0.6; shot R04-return-menu-clear
  pick_option build; pick_option check; shot R05-return-menu-cleared; keycode 53; click 1238 770; sleep 1.5
  [[ $(ret) == '[]' ]] && pass "returns cleared and saved" || fail "returns not cleared: $(ret)"
}
# pick_at X Y NAME: click a stage body at a window point; succeed only if the inspector Name shows NAME.
pick_at() {
  click $1 $2; sleep 0.5; shot _sel
  local n; n=$(at_text Name 960 100 320 200 1); [[ -n $n ]] || return 1
  [[ $(probe ocr "$EVIDENCE/shots/_sel.png" 960 $(( ${n#* } + 14 )) 320 30) == *"$3"* ]] || return 1
  probe box "$EVIDENCE/shots/_sel.png" 360 100 600 680 ${=PINK}
}
horizontal_connect() {
  # Fresh fixture: build→check→repair→review→decision. Arrange+Fit puts repair (3rd column) at the pane centre.
  open_project; workflow_tab; keycode 53
  click 1248 100; sleep 0.5; tap Arrange 900 80 380 400 || { keycode 53; fail "Arrange action not found"; return; }
  sleep 0.6; click 1238 770; sleep 1.5; click 489 736; sleep 0.8; shot H10-fit-normal
  [[ $(wf '.layout?.positions ?? {}') == '{}' ]] && pass "Arrange cleared saved positions" || fail "positions after Arrange: $(wf '.layout?.positions')"
  [[ $(probe ocr "$EVIDENCE/shots/H10-fit-normal.png" 360 120 600 640) != *"React Flow"* ]] && pass "no visible React Flow label on the canvas" || fail "React Flow label visible"
  # Pane centre (window points) from the canvas area between library (x≈461) and inspector (x≈950).
  local cx=705 cy=454 r d x y w h dx dy dw dh
  r=$(pick_at $cx $cy repair) || { fail "centre node is not repair (shots/_sel.png); stopping"; return; }
  read x y w h <<< "$r"; local zoom1000=$(( w * 1000 / 150 ))
  local dcx=$(( x + w / 2 + 460 * zoom1000 / 1000 )) dcy=$(( y + h / 2 ))
  d=$(pick_at $dcx $dcy decision) || { fail "node two columns right ($dcx,$dcy) is not decision; stopping"; return; }
  read dx dy dw dh <<< "$d"
  note "repair rect $x,$y ${w}x$h (zoom $zoom1000/1000); decision rect $dx,$dy ${dw}x$dh"
  pick_at $(( x + w / 2 )) $(( y + h / 2 )) repair >/dev/null || { fail "could not reselect repair; stopping"; return; }
  keycode 53; sleep 0.2
  local fy=$(( y + h / 2 + 16 * zoom1000 / 1000 )) sx=$(( x + w )) tx=$dx ty=$(( dy + dh / 2 )) edges0=$(wf '.graph.edges.length')
  cliclick m:$(at $sx $fy) w:300 dd:$(at $sx $fy) w:200 dm:$(at $(( tx - 10 )) $ty) w:200 dm:$(at $tx $ty) w:500
  shot H12-connecting; cliclick du:$(at $tx $ty); sleep 0.8; shot H13-connected; click 1238 770; sleep 1.5
  local e=$(wf ".graph.edges.filter(e=>e.sourceStageId==='repair')")
  note "repair edges after connect: $e"
  [[ $(wf '.graph.edges.length') == $(( edges0 + 1 )) && $e == *'"targetStageId":"decision","on":"failure"'* ]] \
    && pass "repair's right failure output connected to decision's left input" || fail "connect failure→decision: $e (H12/H13)"
  # Narrow fit with the 900-width fit button (bottom-left canvas control at that width).
  resize 900 800; sleep 0.8; click 269 736; sleep 0.8; shot H14-fit-narrow; resize 1280 800; sleep 0.8
  config_js "const w=d.workflows.find(w=>w.id==='delivery'); w.graph.edges=w.graph.edges.filter(e=>!(e.sourceStageId==='repair'&&e.targetStageId==='decision')); await fetch('$URL/v1/projects/$PROJECT/config',{method:'PUT',headers:{'content-type':'application/json'},body:JSON.stringify(d)})" >/dev/null
}

# FACTORY_JOURNEYS limits a development run to named journeys; the recorded check runs all.
journey() {
  local j
  for j in ${=FACTORY_JOURNEYS:-connected_setup target_setup agent_readiness permission_run motion_setting fork_run horizontal_connect return_multiselect form_layout_review viewport_keyboard parallel_authoring quick_open_after_edit responsive_review}; do $j; done
}

# --- Setup ----------------------------------------------------------------------------------
cd "$ROOT"
: > "$EVIDENCE/journey.txt"
copy_references
[[ -x $CLIENT ]] || { echo "failed: client binary $CLIENT missing"; exit 1; }
# Provenance guard: refuse a client older than the native sources or a known stale build.
STALE=${FACTORY_STALE_SHAS:-052cd6495a3627b8f79dec774edbc3ba97860560e2adafd553a1ba487027d253}
newer=$(find apps/client/src crates -name '*.rs' -newer "$CLIENT" | head -3)
[[ -z $newer ]] || { echo "failed: client binary is older than source: $newer"; exit 1; }
[[ " $STALE " != *" $(shasum -a 256 "$CLIENT" | cut -d' ' -f1) "* ]] || { echo "failed: client binary matches a known stale build"; exit 1; }
cp -p "$CLIENT" "$WORK/karabiner" && cmp -s "$CLIENT" "$WORK/karabiner" || { echo "failed: could not copy $CLIENT"; exit 1; }
note "client $CLIENT sha256 $(shasum -a 256 "$WORK/karabiner" | cut -d' ' -f1) built $(stat -f %Sm "$CLIENT")"
CLIENT=$WORK/karabiner
KARABINER_DB=$WORK/karabiner.sqlite bun .groundwork/scripts/factory-complete-service.ts > "$EVIDENCE/service.log" 2>&1 &
SERVICE_PID=$!
if ! wait_until 10 service_ready; then
  fail "owned service (pid $SERVICE_PID) did not become ready"
elif ! KARABINER_SERVICE_URL=$URL bun .groundwork/scripts/factory-client-fixture.ts "$WORK/project" > "$EVIDENCE/state/fixture.json" 2>"$EVIDENCE/fixture.log"; then
  fail "fixture seed failed (see fixture.log)"
else
  read PROJECT TASK FACTORY0 <<< "$(bun -e 'const f=await Bun.file(process.argv[1]).json(); console.log(f.projectId, f.taskId, f.factoryId)' "$EVIDENCE/state/fixture.json")"
  note "service $URL project $PROJECT task $TASK"
  KARABINER_SERVICE_URL=$URL KARABINER_TRACE_UI_TIMING=${KARABINER_TRACE_UI_TIMING:-} "$CLIENT" > "$EVIDENCE/client.log" 2>&1 &
  APP_PID=$!
  note "client pid $APP_PID"
  if wait_until 20 find_window; then caffeinate -di -w $APP_PID & front; resize 1280 800; sleep 0.5; journey; else fail "client window (pid $APP_PID) not found"; fi
fi
grep -qi panic "$EVIDENCE/client.log" 2>/dev/null && fail "client panicked (see client.log)"

STATUS=passed
[[ -n $FAILURE ]] && STATUS=failed
grep -q '^unverified:' "$EVIDENCE/journey.txt" && [[ $STATUS == passed ]] && STATUS=inconclusive
SHOTS=$(cd "$EVIDENCE" && ls shots/*.png state/* 2>/dev/null | bun -e 'console.log(JSON.stringify((await Bun.stdin.text()).trim().split("\n").filter(Boolean)))')
OBSERVED=${FAILURE:-"${(j:; :)PASSED}"} bun -e 'console.log(JSON.stringify({observations:[{status:process.argv[1],observed:process.env.OBSERVED+" (see journey.txt for unverified items)",evidence:["journey.txt",...JSON.parse(process.argv[2])]}]}))' $STATUS "$SHOTS" > "$EVIDENCE/result.json"
echo "$STATUS"
[[ $STATUS == passed ]]
