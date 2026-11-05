#!/usr/bin/env bash
# three-sim-journey-e2e.sh — the one-pager's BEATS 1–8
# driven end-to-end across THREE iOS simulators over real MLS transport, then walked
# as a UI journey on one of them.
#
# WHY THREE. Beats 4 and 5 cannot exist on a single device. `post-author` renders only
# for a post that is NOT yours (ThreadView: "Yours is not a link — you are already
# here"), and the gold DM circle opens a conversation with somebody else. So the world
# has to be built by real, separate identities:
#
#   ALICE (SIM_A)  walks the eight beats.
#   BOB   (SIM_B)  is a member of the Space, and AUTHORS THE POST beat 4 taps.
#   CAROL (SIM_C)  is the second member — she proves the channel is a room, not a pair.
#
# Nothing here is seeded. Every object is minted by a device through the same core call
# the UI makes, over a real relay:
#
#   pair A↔B, A↔C          real MLS, double-opt-in (the QR handshake, headless)
#   Alice: space-new       a real Group GroupObject          → beat 2's HOST
#   Alice: room-add-member Bob and Carol join over the relay
#   Alice: event-new       a real Event GroupObject          → beat 1's card
#   Alice: space-affiliate the Space CLAIMS the night        → the `created` edge
#   Alice: space-forum-new a real Forum + base.setPart     → beat 3's channel
#   Bob:   room-post       the post beat 4 taps              ← THE REASON FOR THE FLEET
#   Carol: room-post       a second voice in the room
#
# …and then XCUITest walks beats 1→8 on Alice.
#
# THE SEAM. Commands go in through DevCommands' file drain, not `simctl openurl`: iOS 26
# confirms every openurl with a SpringBoard alert no script can tap. The app polls
# dev-commands.txt once a second and runs each line through the identical DevLinks
# handler, so this drives the real app, not a test double.
#
# Prereqs: the app built for the simulator (`just build`), and the relay binary
#   (cd ../arc && cargo build -p relay --release).
# Usage: scripts/three-sim-journey-e2e.sh
#   Override devices: SIM_A=<udid> SIM_B=<udid> SIM_C=<udid> scripts/three-sim-journey-e2e.sh
#   Skip the UI walk (world only):  WORLD_ONLY=1 scripts/three-sim-journey-e2e.sh
set -euo pipefail
cd "$(dirname "$0")/.."

SIM_A="${SIM_A:-4244D0F1-C55A-4ADD-AE15-06ACD9C1627D}"   # iPhone 17 Pro     — Alice
SIM_B="${SIM_B:-7CCD31D2-6A91-4DEB-8A88-752B9996B93D}"   # iPhone 17 Pro Max — Bob
SIM_C="${SIM_C:-E36E02F6-A025-4664-B47C-FE1FD2C308EA}"   # iPhone 17         — Carol
SIMS=("$SIM_A" "$SIM_B" "$SIM_C")
NAMES=("Alice" "Bob" "Carol")
BUNDLE_ID="network.pacific"
APP="ui/build/Build/Products/Debug-iphonesimulator/WallFlowers.app"
SHOTS="/tmp/journey-e2e"

[ -d "$APP" ] || { echo "WallFlowers.app not found — run: just build"; exit 1; }
RELAY_BIN="../arc/target/release/semaphore"
[ -x "$RELAY_BIN" ] || { echo "relay missing: (cd ../arc && cargo build -p relay --release)"; exit 1; }

enc() { python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$1"; }
support() { echo "$(xcrun simctl get_app_container "$1" "$BUNDLE_ID" data)/Library/Application Support/pacific"; }
data_file() { cat "$(support "$1")/$2" 2>/dev/null; }
# THE SEAM: append, never openurl (see the header).
cmd() { echo "$2" >> "$(support "$1")/dev-commands.txt"; }
sync_all() { for s in "${SIMS[@]}"; do cmd "$s" "pacific://sync"; done; sleep "${1:-4}"; }
contact() { data_file "$1" "dev-contact.txt"; }
# A FRESH bundle each time: MLS key packages are single-use, so a second add that
# reuses a consumed one cannot complete (CoreService.exportDevContact's own note).
fresh_bundle() { cmd "$1" "pacific://export-contact"; sleep 3; contact "$1" | sed -n 2p; }
shot() { xcrun simctl io "$1" screenshot "$SHOTS/$2.png" >/dev/null 2>&1 || true; }

echo "— relay —"
listening() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
PORT=8810; while listening "$PORT"; do PORT=$((PORT + 1)); done
RELAY_URL="ws://127.0.0.1:${PORT}"
RELAY_BIND="127.0.0.1:$PORT" nohup "$RELAY_BIN" >"/tmp/semaphore-$PORT.log" 2>&1 &
RELAY_PID=$!
trap 'kill "$RELAY_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 40); do listening "$PORT" && break; sleep 0.3; done
listening "$PORT" || { echo "relay failed to start (see /tmp/semaphore-$PORT.log)"; exit 1; }
echo "  semaphore up on $RELAY_URL (pid $RELAY_PID)"

echo "— three fresh identities (clean install; NO seed) —"
mkdir -p "$SHOTS"
for i in 0 1 2; do
  S="${SIMS[$i]}"
  xcrun simctl bootstatus "$S" -b >/dev/null
  xcrun simctl terminate "$S" "$BUNDLE_ID" 2>/dev/null || true
  xcrun simctl uninstall  "$S" "$BUNDLE_ID" 2>/dev/null || true
  xcrun simctl install    "$S" "$APP"
  SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" \
  SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="${NAMES[$i]}" \
  SIMCTL_CHILD_PACIFIC_SKIP_MOCK_SEED=1 \
    xcrun simctl launch "$S" "$BUNDLE_ID" >/dev/null
  echo "  [${NAMES[$i]}] $S"
done
sleep 10

A_SPACE=$(contact "$SIM_A" | head -1)
B_SPACE=$(contact "$SIM_B" | head -1)
C_SPACE=$(contact "$SIM_C" | head -1)
[ -n "$A_SPACE" ] && [ -n "$B_SPACE" ] && [ -n "$C_SPACE" ] || { echo "FAILED: an identity did not export"; exit 1; }
echo "  Alice=${A_SPACE:0:16}…  Bob=${B_SPACE:0:16}…  Carol=${C_SPACE:0:16}…"

echo "— pair Alice↔Bob, Alice↔Carol (double opt-in, real Welcomes) —"
A_BUNDLE=$(fresh_bundle "$SIM_A")
cmd "$SIM_B" "pacific://pair-scan?bundle=$(enc "$A_BUNDLE")"; sleep 7
cmd "$SIM_A" "pacific://sync"; sleep 5
cmd "$SIM_A" "pacific://pair-accept?peer=$B_SPACE"; sleep 3
cmd "$SIM_B" "pacific://pair-accept?peer=$A_SPACE"; sleep 3
cmd "$SIM_B" "pacific://sync"; sleep 5

A_BUNDLE=$(fresh_bundle "$SIM_A")            # a NEW key package for Carol's Welcome
cmd "$SIM_C" "pacific://pair-scan?bundle=$(enc "$A_BUNDLE")"; sleep 7
cmd "$SIM_A" "pacific://sync"; sleep 5
cmd "$SIM_A" "pacific://pair-accept?peer=$C_SPACE"; sleep 3
cmd "$SIM_C" "pacific://pair-accept?peer=$A_SPACE"; sleep 3
sync_all 5

echo "— Alice constitutes the Space, and Bob + Carol join it —"
cmd "$SIM_A" "pacific://space-new?name=$(enc 'Monroe')&purpose=$(enc 'Broadway room. Late.')"; sleep 4
SPACE=$(data_file "$SIM_A" "dev-space.txt" | head -1)
[ -n "$SPACE" ] || { echo "FAILED: no Space id"; exit 1; }
echo "  space=${SPACE:0:16}…"

B_BUNDLE=$(fresh_bundle "$SIM_B")
cmd "$SIM_A" "pacific://room-add-member?object=$SPACE&bundle=$(enc "$B_BUNDLE")"; sleep 7
cmd "$SIM_B" "pacific://sync"; sleep 5
C_BUNDLE=$(fresh_bundle "$SIM_C")
cmd "$SIM_A" "pacific://room-add-member?object=$SPACE&bundle=$(enc "$C_BUNDLE")"; sleep 7
cmd "$SIM_C" "pacific://sync"; sleep 5
sync_all 5

echo "— the night, and the Space's claim on it (beat 2's edge) —"
START=$(python3 -c "import time;print(int((time.time()+86400)*1000))")
cmd "$SIM_A" "pacific://event-new?title=$(enc 'Monroe presents UPWARD')&descriptor=$(enc 'Apex Martin · Cloopez · Joey Trip')&start=$START&venue=$(enc 'Monroe, 473 Broadway, SF')&capacity=120"; sleep 5
EVENT=$(data_file "$SIM_A" "dev-event.txt" | head -1)
[ -n "$EVENT" ] || { echo "FAILED: no Event id"; exit 1; }
echo "  event=${EVENT:0:16}…"
cmd "$SIM_A" "pacific://space-affiliate?object=$SPACE&peer=$EVENT&rel=created&name=$(enc 'Monroe presents UPWARD')"; sleep 4

echo "— the CHANNEL (beat 3), and the members it pulls in —"
cmd "$SIM_A" "pacific://space-forum-new?object=$SPACE&name=$(enc 'general')"; sleep 8
FORUM=$(data_file "$SIM_A" "dev-forum.txt" | head -1)
[ -n "$FORUM" ] || { echo "FAILED: no Forum id"; exit 1; }
echo "  channel=${FORUM:0:16}…"
sync_all 6

echo "— BOB writes the post beat 4 taps (and Carol answers) —"
cmd "$SIM_B" "pacific://room-post?object=$FORUM&text=$(enc 'is the rooftop still on for Saturday')"; sleep 6
sync_all 5
cmd "$SIM_C" "pacific://room-post?object=$FORUM&text=$(enc 'doors 22:00 — bring the decks')"; sleep 6
sync_all 6

echo "— assert the world converged BEFORE walking it —"
for s in "${SIMS[@]}"; do cmd "$s" "pacific://dump-thread?object=$FORUM"; done; sleep 4
TA=$(data_file "$SIM_A" "dev-thread.txt")
TB=$(data_file "$SIM_B" "dev-thread.txt")
echo "--- Alice's fold of the channel ---"; printf '%s\n' "$TA"
[ -n "$TA" ] || { echo "✗ Alice's channel is empty — the fleet did not converge"; exit 1; }
# The identity RENDERS as `ed25519:<hex>` (the IdentityKey rename), but a post's
# author in the fold is the raw hex — so compare on the hex, not the rendering.
BOB8=$(printf '%s' "$B_SPACE" | sed 's/^ed25519://' | cut -c1-8)
printf '%s\n' "$TA" | grep -q "$BOB8" || {
  echo "✗ Bob's post is NOT in Alice's fold (beat 4 would have nothing to tap)"; exit 1; }
echo "✓ Bob's post ($BOB8) folded onto Alice's device"
[ "$TA" = "$TB" ] && echo "✓ Alice and Bob fold the identical thread" || echo "… Alice/Bob folds differ (ordering) — beat 4 still stands"

for i in 0 1 2; do shot "${SIMS[$i]}" "world-${NAMES[$i]}"; done

if [ "${WORLD_ONLY:-0}" = "1" ]; then
  echo; echo "world built. screenshots: $SHOTS"; exit 0
fi

echo
echo "— WALK THE EIGHT BEATS on Alice —"
# The walk runs against the world just built, so the app must reach the SAME relay
# (beat 8 adds a real member). The handshake is a FILE, not `TEST_RUNNER_*`: those
# build settings never reach the runner's environment in this project (probed — it
# carries no PACIFIC_* keys at all), so a walk driven that way silently SKIPPED and
# the harness called it a pass. A green that can be produced by doing nothing is
# worse than a red.
HANDSHAKE=/tmp/pacific-journey.env
printf 'peer=Bob\nrelay=%s\nspace=%s\nevent=%s\nforum=%s\n' \
  "$RELAY_URL" "$SPACE" "$EVENT" "$FORUM" > "$HANDSHAKE"
trap 'kill "$RELAY_PID" 2>/dev/null || true; rm -f "$HANDSHAKE"' EXIT

set +e
xcodebuild -project ui/WallFlowers.xcodeproj -scheme WallFlowers -sdk iphonesimulator \
  -destination "platform=iOS Simulator,id=$SIM_A" -derivedDataPath ui/build \
  -only-testing:WallFlowersUITests/OnePagerJourneyUITests/test_beats1to8_fullJourney_onAPreparedFleet \
  test > /tmp/journey-walk.log 2>&1
WALK=$?
set -e
grep -E "Test Case '.*' (passed|failed|skipped)|error:" /tmp/journey-walk.log | head -20 || true

# A SKIP IS NOT A PASS. xcodebuild exits 0 on a skipped test, so the harness has to
# say so itself or it will report a walk that never happened.
if grep -q "skipped (" /tmp/journey-walk.log; then
  echo "✗ the walk SKIPPED — the handshake at $HANDSHAKE did not reach the test"
  WALK=1
elif ! grep -q "test_beats1to8_fullJourney_onAPreparedFleet]' passed" /tmp/journey-walk.log; then
  echo "✗ the walk did not report a pass"
  [ "$WALK" -eq 0 ] && WALK=1
fi

# The beats' own screenshots ride in the xcresult; pull them out beside the world's.
R=$(ls -dt ui/build/Logs/Test/*.xcresult 2>/dev/null | head -1)
if [ -n "$R" ]; then
  rm -rf "$SHOTS/beats"; xcrun xcresulttool export attachments --path "$R" --output-path "$SHOTS/beats" >/dev/null 2>&1 || true
  python3 - "$SHOTS/beats" <<'PY' || true
import json,os,shutil,sys
d=sys.argv[1]
try: m=json.load(open(os.path.join(d,'manifest.json')))
except Exception: raise SystemExit
for t in m:
    for a in t.get('attachments',[]):
        n=a.get('suggestedHumanReadableName','')
        if n.startswith('fleet-beat'):
            shutil.copy(os.path.join(d,a['exportedFileName']), os.path.join(d,'..',n.split('_')[0]+'.png'))
PY
fi

echo
if [ "$WALK" -eq 0 ]; then
  echo "✓ BEATS 1–8 WALKED END-TO-END on a three-identity world"
else
  echo "✗ the walk failed — see /tmp/journey-walk.log"
fi
echo "screenshots: $SHOTS"
exit "$WALK"
