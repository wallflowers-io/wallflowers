#!/usr/bin/env bash
# exterior-e2e.sh — the exterior and its door (#76), by real taps on two simulators and a
# local relay.
#
#   OWNER  makes an Event at Network with words for outsiders and "Ask to join"
#   ASKER  (the owner's connection, not in it) sees its card, opens its hosted outside
#          (media.hosted, through the relay's object storage), and attends
#   OWNER  finds the ask under the bell and admits
#   ASKER  is inside: the exterior card is gone
#
# Every exterior step is an XCUITest tap (ExteriorUITests). Pairing is not what is tested,
# so the two are paired over the dev seam, as scripts/two-sim-*.sh pair. The relay and both
# simulators are this run's own, and are removed when it ends, however it ends.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd ../.. && pwd)"

MODEL="${MODEL:-iPhone 17 Pro}"
OWNER_NAME="Olive Owner"
ASKER_NAME="Sam Asker"
CONFIG=/tmp/pacific-exterior.env
DD=ui/build/exterior
LOGS="${LOGS:-$DD/logs}"
LINK_SETTLE=1.5
STEP=start
rm -rf "$LOGS"; mkdir -p "$LOGS"

step() { STEP="$1"; echo "== $1"; }
fail() { echo "✗ FAILED at [$STEP]"; exit 1; }

RELAY_PID=""; STORE_PID=""; SIM_O=""; SIM_A=""
cleanup() {
  [ -n "$RELAY_PID" ] && kill "$RELAY_PID" 2>/dev/null || true
  [ -n "$STORE_PID" ] && kill "$STORE_PID" 2>/dev/null || true
  for S in "$SIM_O" "$SIM_A"; do
    [ -n "$S" ] || continue
    # The apps' own logs, kept for reading: the simulator goes with them.
    c=$(xcrun simctl get_app_container "$S" network.pacific data 2>/dev/null || true)
    [ -n "$c" ] && cp "$c/Library/Logs/wallflowers.log" "$LOGS/app-$S.log" 2>/dev/null || true
    # KEEP=1 leaves the simulators for reading; delete them by hand after.
    [ "${KEEP:-0}" = 1 ] && { echo "   kept $S"; continue; }
    xcrun simctl shutdown "$S" 2>/dev/null || true
    xcrun simctl delete "$S" 2>/dev/null || true
  done
  rm -f "$CONFIG"
}
trap cleanup EXIT

step "relay"
(cd "$ROOT" && ~/.cargo/bin/cargo build -q -p relay --bin semaphore)
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
# The relay's object storage, this run's own: where the hosted outside lives.
SPORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
python3 scripts/object-store.py "$SPORT" "$LOGS/objects" &
STORE_PID=$!
RELAY_BIND="127.0.0.1:$PORT" RELAY_R2_ENDPOINT="http://127.0.0.1:$SPORT" RELAY_R2_BUCKET=pacific-media \
  RELAY_R2_ACCESS_KEY_ID=k RELAY_R2_SECRET_ACCESS_KEY=s RELAY_R2_BUDGET_BYTES=0 \
  "$ROOT/target/debug/semaphore" > "$LOGS/relay.log" 2>&1 &
RELAY_PID=$!
for _ in $(seq 1 30); do nc -z 127.0.0.1 "$PORT" && break; sleep 0.5; done
nc -z 127.0.0.1 "$PORT" || fail
RELAY_URL="ws://127.0.0.1:$PORT"
echo "   relay $RELAY_URL (pid $RELAY_PID)"

step "simulators"
SIM_O=$(xcrun simctl create "Exterior Owner $$" "$MODEL")
SIM_A=$(xcrun simctl create "Exterior Asker $$" "$MODEL")
for S in "$SIM_O" "$SIM_A"; do xcrun simctl boot "$S"; xcrun simctl bootstatus "$S" -b >/dev/null; done

step "build"
xcodebuild -project ui/WallFlowers.xcodeproj -scheme WallFlowers -sdk iphonesimulator \
  -destination "platform=iOS Simulator,id=$SIM_O" -derivedDataPath "$DD" \
  build-for-testing > "$LOGS/build.log" 2>&1 || { grep -E "error:" "$LOGS/build.log" | head -5; fail; }
XCTESTRUN=$(ls -t "$DD"/Build/Products/*.xctestrun | head -1)
APP="$DD/Build/Products/Debug-iphonesimulator/WallFlowers.app"
for S in "$SIM_O" "$SIM_A"; do
  xcrun simctl install "$S" "$APP"
  xcrun simctl privacy "$S" grant all network.pacific 2>/dev/null || true
done

dev_dir()  { echo "$(xcrun simctl get_app_container "$1" network.pacific data)/Library/Application Support/pacific"; }
dev_file() { cat "$(dev_dir "$1")/$2" 2>/dev/null || true; }
open_url() { mkdir -p "$(dev_dir "$1")"; echo "$2" >> "$(dev_dir "$1")/dev-commands.txt"; sleep "$LINK_SETTLE"; }
enc() { python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=''))" "$1"; }

step "identities"
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="$OWNER_NAME" \
  xcrun simctl launch "$SIM_O" network.pacific >/dev/null
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="$ASKER_NAME" \
  xcrun simctl launch "$SIM_A" network.pacific >/dev/null
t0=$(date +%s)
while [ -z "$(dev_file "$SIM_O" dev-contact.txt)" ] || [ -z "$(dev_file "$SIM_A" dev-contact.txt)" ]; do
  [ $(( $(date +%s) - t0 )) -lt 90 ] || { echo "timeout: identities"; fail; }
  sleep 1
done
O_SPACE=$(dev_file "$SIM_O" dev-contact.txt | sed -n 1p)
O_BUNDLE=$(dev_file "$SIM_O" dev-contact.txt | sed -n 2p)
A_SPACE=$(dev_file "$SIM_A" dev-contact.txt | sed -n 1p)

step "pair"
open_url "$SIM_A" "pacific://pair-scan?bundle=$(enc "$O_BUNDLE")"; sleep 6
open_url "$SIM_O" "pacific://sync"; sleep 5
open_url "$SIM_O" "pacific://pair-accept?peer=$A_SPACE"
open_url "$SIM_A" "pacific://pair-accept?peer=$O_SPACE"
open_url "$SIM_A" "pacific://sync"
t0=$(date +%s)
until [ -n "$(dev_file "$SIM_O" dev-profiles.txt)" ]; do
  [ $(( $(date +%s) - t0 )) -lt 90 ] || { echo "timeout: pairing"; fail; }
  open_url "$SIM_O" "pacific://sync"; open_url "$SIM_O" "pacific://dump-profiles"
done
echo "   paired: the owner sees $(dev_file "$SIM_O" dev-profiles.txt | head -1)"
for S in "$SIM_O" "$SIM_A"; do xcrun simctl terminate "$S" network.pacific 2>/dev/null || true; done

name_of() { xcrun simctl list devices -j | python3 -c "
import json,sys
d=json.load(sys.stdin)['devices']
print(next(x['name'] for rt in d for x in d[rt] if x['udid']=='$1'))"; }
{ printf 'relay=%s\n' "$RELAY_URL"
  printf 'role.%s=owner\n' "$(name_of "$SIM_O")"
  printf 'role.%s=asker\n' "$(name_of "$SIM_A")"
  printf 'name.owner=%s\nname.asker=%s\n' "$OWNER_NAME" "$ASKER_NAME"
} > "$CONFIG"

# The OTHER phone on the desk stays on: its app runs and syncs while this one takes its
# step, as a person's would. An XCUITest run ends its own app when it ends.
other_on() {
  local other=$1 name=$2
  SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="$name" \
    xcrun simctl launch "$other" network.pacific >/dev/null 2>&1 || true
}

# One step, one device. A skip is not a pass.
run() {
  local sim=$1 test=$2 log="$LOGS/$2.log"
  STEP="$test"
  echo "== $test"
  if [ "$sim" = "$SIM_O" ]; then other_on "$SIM_A" "$ASKER_NAME"; else other_on "$SIM_O" "$OWNER_NAME"; fi
  xcodebuild test-without-building -xctestrun "$XCTESTRUN" \
    -destination "platform=iOS Simulator,id=$sim" \
    -only-testing:"WallFlowersUITests/ExteriorUITests/$test" -resultBundlePath "$LOGS/$test.xcresult" \
    > "$log" 2>&1 || true
  if grep -q "skipped (" "$log"; then echo "  ✗ skipped"; fail; fi
  grep -q "$test\]' passed" "$log" || { grep -E "error:|XCTAssert" "$log" | head -3; fail; }
  echo "  ✓"
}

run "$SIM_O" test_1_owner_writesAnExterior
run "$SIM_A" test_2_asker_asksToJoin
run "$SIM_O" test_3_owner_admits
run "$SIM_A" test_4_asker_isInside
echo "✓ the exterior, end to end — screenshots in $LOGS/*.xcresult"
