#!/usr/bin/env bash
# fleet-journey.sh — the one-pager journey across three simulators, driven ONLY by
# real taps through Apple's XCUITest harness.
#
# THE RULE THIS SCRIPT EXISTS TO KEEP: no `pacific://` dev links, no `simctl openurl`,
# no writing into the app's container. The only things this script does to a device
# are what a person with three phones on a desk would do — install, launch, and then
# hand the next device its turn. Every state change inside the app happens because
# XCUITest tapped something. (`scripts/three-sim-*.sh` drive the dev seam instead;
# they are demos of the core, not tests of the interface.)
#
# HOW THE DEVICES HAND OFF. XCUITest drives one target per run, so the fleet is a
# sequence of runs. Two couriers cross between them, and both are the ones a person
# uses: the RELAY (a real Welcome, a real delta) and the CONNECT CODE, read off the
# screen that the real "copy code" button just filled.
#
# Usage: scripts/fleet-journey.sh [step]
#   no step  — the whole sequence
#   boot     — fresh install + first-boot doors on all three
#   pair     — Alice↔Bob and Alice↔Carol, by code
set -euo pipefail
cd "$(dirname "$0")/.."

SIM_A="${SIM_A:-4244D0F1-C55A-4ADD-AE15-06ACD9C1627D}"   # iPhone 17 Pro     — Alice
SIM_B="${SIM_B:-7CCD31D2-6A91-4DEB-8A88-752B9996B93D}"   # iPhone 17 Pro Max — Bob
SIM_C="${SIM_C:-E36E02F6-A025-4664-B47C-FE1FD2C308EA}"   # iPhone 17         — Carol
BUNDLE_ID="network.pacific"
APP="ui/build/Build/Products/Debug-iphonesimulator/WallFlowers.app"
CONFIG=/tmp/pacific-fleet.env
STEP="${1:-all}"

[ -d "$APP" ] || { echo "WallFlowers.app not found — run: just build"; exit 1; }
RELAY_BIN="../arc/target/release/semaphore"
[ -x "$RELAY_BIN" ] || { echo "relay missing: (cd ../arc && cargo build -p relay --release)"; exit 1; }

sim_of() { case "$1" in Alice) echo "$SIM_A";; Bob) echo "$SIM_B";; Carol) echo "$SIM_C";; esac; }

XCTESTRUN=""
prepare_bundle() {
  echo "— build once, for testing —"
  xcodebuild -project ui/WallFlowers.xcodeproj -scheme WallFlowers -sdk iphonesimulator \
    -destination "platform=iOS Simulator,id=$SIM_A" -derivedDataPath ui/build \
    build-for-testing > /tmp/fleet-build.log 2>&1 || {
      grep -E "error:" /tmp/fleet-build.log | head -5; echo "build failed"; exit 1; }
  XCTESTRUN=$(ls -t ui/build/Build/Products/*.xctestrun | head -1)
  echo "  $(basename "$XCTESTRUN")"
}

# ONE run = one device, one step. The config file carries the harness's parameters
# (who this device is, which relay, who it is pairing with) — never a command.
#
# `test-without-building` against the prebuilt bundle, NOT `test`: three concurrent
# `xcodebuild test` invocations fight over one build database ("database is locked
# … two concurrent builds in the same filesystem location") and the losers die. With
# nothing left to build, the three devices can genuinely run at the same time.
# THE ROLE IS THE DEVICE. Three parallel runs share one config file, so a run
# cannot be told who it is by an argument — and environment variables do not reach
# the runner process in this project (probed: `TEST_RUNNER_*` arrives empty). Each
# device therefore looks ITSELF up by name, which is the one thing it always knows.
write_config() {
  local peer=${1:-}
  { printf 'relay=%s\n' "$RELAY_URL"
    printf 'role.%s=Alice\n' "$(name_of "$SIM_A")"
    printf 'role.%s=Bob\n'   "$(name_of "$SIM_B")"
    printf 'role.%s=Carol\n' "$(name_of "$SIM_C")"
    printf 'peer=%s\n' "$peer"
  } > "$CONFIG"
}
name_of() { xcrun simctl list devices -j | python3 -c "
import json,sys
d=json.load(sys.stdin)['devices']
for rt in d:
    for x in d[rt]:
        if x['udid']=='$1': print(x['name']); raise SystemExit
"; }

# Sets LAST_PID rather than echoing it: `pid=$(run_async …)` puts the background job
# inside a COMMAND-SUBSTITUTION SUBSHELL, so the pid is not a child of this shell and
# `wait` refuses it ("not a child of this shell") — returning instantly, which made
# every verdict read a log the test had not written yet. Every step "failed", and
# none of them had actually run.
LAST_PID=""
run_async() {
  local who=$1 test=$2
  xcodebuild test-without-building -xctestrun "$XCTESTRUN" \
      -destination "platform=iOS Simulator,id=$(sim_of "$who")" \
      -only-testing:"WallFlowersUITests/FleetJourneyUITests/$test" \
      > "/tmp/fleet-$who-$test.log" 2>&1 &
  LAST_PID=$!
}

verdict() {
  local who=$1 test=$2 log="/tmp/fleet-$who-$test.log"
  if grep -q "skipped (" "$log" 2>/dev/null; then
    echo "  ✗ $who · $test SKIPPED (a skip is not a pass)"; return 1
  fi
  if grep -q "test_.*]' passed" "$log" 2>/dev/null; then echo "  ✓ $who · $test"; return 0; fi
  grep -E "error:|XCTAssert" "$log" 2>/dev/null | head -2
  echo "  ✗ $who · $test — /tmp/fleet-$who-$test.log"; return 1
}

# A RUNNER DEATH IS NOT A TEST FAILURE. Three screen recordings plus a test run put
# this machine under enough load that the XCUITest runner sometimes dies outright
# ("Restarting after unexpected exit, crash, or test timeout") — the same thing the
# untouched FeedUITests suite does here. That is the harness falling over, not the
# app, so it gets one more go; a real assertion failure is NOT retried.
run() {
  local who=$1 test=$2 peer=${3:-}
  write_config "$peer"
  run_async "$who" "$test"
  wait "$LAST_PID" || true
  if verdict "$who" "$test"; then return 0; fi
  if grep -q "Restarting after unexpected exit" "/tmp/fleet-$who-$test.log" 2>/dev/null; then
    echo "    … runner died, retrying once"
    run_async "$who" "$test"
    wait "$LAST_PID" || true
    verdict "$who" "$test"
    return $?
  fi
  return 1
}

# All three at once — the only steps that genuinely can be, since first boot is
# three separate people setting three separate phones up and nothing crosses.
run_all_parallel() {
  local test=$1 rc=0
  write_config ""
  echo "  ▸ Alice · Bob · Carol — $test, in parallel"
  local pa pb pc
  run_async Alice "$test"; pa=$LAST_PID
  run_async Bob   "$test"; pb=$LAST_PID
  run_async Carol "$test"; pc=$LAST_PID
  wait "$pa" || true; wait "$pb" || true; wait "$pc" || true
  verdict Alice "$test" || rc=1
  verdict Bob   "$test" || rc=1
  verdict Carol "$test" || rc=1
  return $rc
}

# ── the cameras ───────────────────────────────────────────────────────────────
# `simctl io recordVideo` per device, for the whole run. Recording only WATCHES;
# every state change is still a tap.
VIDEO_DIR=/tmp/fleet-video
declare -a REC_PIDS=()
start_cameras() {
  mkdir -p "$VIDEO_DIR"; rm -f "$VIDEO_DIR"/*.mp4 "$VIDEO_DIR"/*.mov
  for who in Alice Bob Carol; do
    xcrun simctl io "$(sim_of "$who")" recordVideo --codec h264 --force \
      "$VIDEO_DIR/$who.mp4" >/dev/null 2>&1 &
    REC_PIDS+=($!)
  done
  sleep 2
  echo "  cameras rolling → $VIDEO_DIR"
}
stop_cameras() {
  # SIGINT, not KILL: recordVideo finalises the container on interrupt, and a
  # -9'd recording leaves an unplayable file.
  for pid in "${REC_PIDS[@]:-}"; do kill -INT "$pid" 2>/dev/null || true; done
  for pid in "${REC_PIDS[@]:-}"; do wait "$pid" 2>/dev/null || true; done
  sleep 2
}

echo "— relay —"
listening() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
PORT=8810; while listening "$PORT"; do PORT=$((PORT + 1)); done
RELAY_URL="ws://127.0.0.1:${PORT}"
RELAY_BIND="127.0.0.1:$PORT" nohup "$RELAY_BIN" >"/tmp/semaphore-$PORT.log" 2>&1 &
RELAY_PID=$!
trap 'kill "$RELAY_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 40); do listening "$PORT" && break; sleep 0.3; done
listening "$PORT" || { echo "relay failed to start"; exit 1; }
echo "  semaphore up on $RELAY_URL"

prepare_bundle
[ "${RECORD:-1}" = "1" ] && start_cameras
FAILED=0

if [ "$STEP" = "all" ] || [ "$STEP" = "boot" ]; then
  echo "— clean install, then WALK the first-boot doors — ALL THREE AT ONCE —"
  rm -f /tmp/pacific-code-*.txt
  for who in Alice Bob Carol; do
    S=$(sim_of "$who")
    xcrun simctl bootstatus "$S" -b >/dev/null
    xcrun simctl terminate "$S" "$BUNDLE_ID" 2>/dev/null || true
    xcrun simctl uninstall  "$S" "$BUNDLE_ID" 2>/dev/null || true
    xcrun simctl install    "$S" "$APP"
    # REDUCE MOTION ON — a device setting, the same one a person can switch on, not
    # a hook into the app. `NoteAmbient` is a `TimelineView(.animation)` that never
    # stops drifting, so XCUITest's "wait for network.pacific to idle" at launch has
    # nothing to settle on and the runner eventually dies there — before a single
    # tap. (It is why the untouched FeedUITests suite dies here too, and why three
    # cameras rolling was enough to tip every Alice step over.) Ambient.swift already
    # freezes to one static frame under Reduce Motion; this asks for that frame.
    xcrun simctl spawn "$S" defaults write com.apple.Accessibility ReduceMotionEnabled -bool true 2>/dev/null || true
  done
  run_all_parallel test_step1_firstBoot || FAILED=1
fi

if [ "$STEP" = "all" ] || [ "$STEP" = "pair" ]; then
  # SEQUENTIAL ON PURPOSE. A handshake has an order — you cannot type a code that
  # has not been held out yet — so this half is three people taking turns, and
  # pretending otherwise would be theatre. Alice holds a FRESH code out per peer:
  # MLS key packages are single-use, and a reused one cannot complete its Welcome.
  echo "— pair, by holding a code out and typing it in —"
  run Alice test_step2_showMyCode         || FAILED=1
  run Bob   test_step3_enterPeerCode Alice || FAILED=1
  run Alice test_step4_acceptPeer    Bob   || FAILED=1

  run Alice test_step2_showMyCode          || FAILED=1
  run Carol test_step3_enterPeerCode Alice || FAILED=1
  run Alice test_step4_acceptPeer    Carol || FAILED=1
fi

if [ "${RECORD:-1}" = "1" ]; then
  stop_cameras
  echo "— stitch the three screens into one —"
  # Scale to a common height, then hstack: Alice | Bob | Carol, left to right.
  # No `drawtext` — this ffmpeg is built without libfreetype and the filter simply
  # does not exist, which failed the whole stitch. The pane order is the caption.
  if [ -f "$VIDEO_DIR/Alice.mp4" ] && [ -f "$VIDEO_DIR/Bob.mp4" ] && [ -f "$VIDEO_DIR/Carol.mp4" ]; then
    ffmpeg -y -loglevel error \
      -i "$VIDEO_DIR/Alice.mp4" -i "$VIDEO_DIR/Bob.mp4" -i "$VIDEO_DIR/Carol.mp4" \
      -filter_complex "\
        [0:v]scale=-2:900,pad=iw+12:ih:6:0:black[a];\
        [1:v]scale=-2:900,pad=iw+12:ih:6:0:black[b];\
        [2:v]scale=-2:900,pad=iw+12:ih:6:0:black[c];\
        [a][b][c]hstack=inputs=3[v]" \
      -map "[v]" -r 30 -pix_fmt yuv420p "$VIDEO_DIR/fleet-three-up.mp4" \
      && echo "  ✓ $VIDEO_DIR/fleet-three-up.mp4  (Alice | Bob | Carol)" \
      || echo "  … stitch failed; the three per-device recordings are still in $VIDEO_DIR"
  else
    echo "  … a recording is missing; check $VIDEO_DIR"
  fi
fi

echo
[ "$FAILED" -eq 0 ] && echo "✓ fleet steps complete ($STEP)" || { echo "✗ fleet had failures ($STEP)"; exit 1; }
