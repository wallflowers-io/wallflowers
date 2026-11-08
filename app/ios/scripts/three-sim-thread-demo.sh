#!/usr/bin/env bash
# three-sim-thread-demo.sh — a 3-way NESTED ChatRoom thread across three iOS
# simulators over real MLS transport, then assert all three fold the identical
# thread tree byte-for-byte and screenshot the Reddit-style ThreadView.
#
# Drives the app's pacific:// dev seam (DevLinks.swift) — every handler is a real
# core call; nothing bypasses the MLS group or invents state. Alice (SIM_A) owns
# the ChatRoom (a forum object); Bob and Carol join and reply, one nested to
# depth 2. The headless twin is `cargo test -p pacific-core --test m4_three_user_thread`.
#
# Prereqs: app built for x86_64 sim (WallFlowers.app in DerivedData), relay built
#   ((cd ../Arc && cargo build -p relay --release) → ../Arc/target/release/semaphore), three sims.
# Usage: scripts/three-sim-thread-demo.sh
#   Override devices: SIM_A=<udid> SIM_B=<udid> SIM_C=<udid> scripts/three-sim-thread-demo.sh
set -euo pipefail

SIM_A="${SIM_A:-1DECD6A0-A955-4644-BCCB-7F25258AAB63}"   # iPhone 16 Pro — Alice (owner)
SIM_B="${SIM_B:-FBC887EE-9ABE-4E67-B368-7D01620B15D3}"   # iPhone 16 — Bob
SIM_C="${SIM_C:-C3D941F2-D1EC-4822-BF40-86E40065EFFD}"   # iPhone SE (3rd gen) — Carol
SIMS=("$SIM_A" "$SIM_B" "$SIM_C")
NAMES=("Alice" "Bob" "Carol")
BUNDLE_ID="network.pacific"
APP=$(ls -d "$HOME"/Library/Developer/Xcode/DerivedData/Pacific-*/Build/Products/Debug-iphonesimulator/WallFlowers.app 2>/dev/null | head -1)
[ -n "$APP" ] || { echo "WallFlowers.app not found in DerivedData — build the app for a simulator first."; exit 1; }

enc() { python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$1"; }
data_file() { local c; c=$(xcrun simctl get_app_container "$1" "$BUNDLE_ID" data); cat "$c/Library/Application Support/$2" 2>/dev/null; }
contact()   { data_file "$1" "pacific/dev-contact.txt"; }
room_id()   { data_file "$1" "pacific/dev-room.txt" | head -1; }
thread()    { data_file "$1" "pacific/dev-thread.txt"; }
url()       { xcrun simctl openurl "$1" "$2" 2>/dev/null || true; }
sync_all()  { for s in "${SIMS[@]}"; do url "$s" "pacific://sync"; done; sleep "${1:-3}"; }

# Our own relay on a guaranteed-free port (never trust a squatter on a fixed one).
listening() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
PORT=8810; while listening "$PORT"; do PORT=$((PORT + 1)); done
RELAY_URL="ws://127.0.0.1:${PORT}"
echo "— local relay ($RELAY_URL) —"
[ -x ../Arc/target/release/semaphore ] || { echo "relay missing: (cd ../Arc && cargo build -p relay --release)"; exit 1; }
RELAY_BIND="127.0.0.1:$PORT" nohup ../Arc/target/release/semaphore >"/tmp/semaphore-$PORT.log" 2>&1 &
RELAY_PID=$!
trap 'kill "$RELAY_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 30); do listening "$PORT" && break; sleep 0.3; done
listening "$PORT" || { echo "relay failed to start (see /tmp/semaphore-$PORT.log)"; exit 1; }
echo "  semaphore up (pid $RELAY_PID)"

echo "— boot + clean-install 3 fresh users (NO mock seed) —"
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
  echo "  [$i] ${NAMES[$i]} → $S"
done
sleep 8

B_BUNDLE=$(contact "$SIM_B" | sed -n 2p); [ -n "$B_BUNDLE" ] || { echo "FAILED: no Bob bundle"; exit 1; }
C_BUNDLE=$(contact "$SIM_C" | sed -n 2p); [ -n "$C_BUNDLE" ] || { echo "FAILED: no Carol bundle"; exit 1; }

echo "— Alice mints the ChatRoom + adds Bob and Carol (real MLS) —"
url "$SIM_A" "pacific://new-room"; sleep 3
ROOM=$(room_id "$SIM_A"); echo "room id: $ROOM"
[ -n "$ROOM" ] || { echo "FAILED: no room id exported"; exit 1; }
url "$SIM_A" "pacific://room-add-member?object=$ROOM&bundle=$(enc "$B_BUNDLE")"; sleep 6
url "$SIM_B" "pacific://sync"; sleep 4                     # Bob drains the Welcome, joins
url "$SIM_A" "pacific://room-add-member?object=$ROOM&bundle=$(enc "$C_BUNDLE")"; sleep 6
url "$SIM_C" "pacific://sync"; sleep 4                     # Carol joins
sync_all 4                                                 # everyone at the same epoch

echo "— build the nested thread (root → 2 replies → 1 depth-2 reply) —"
url "$SIM_A" "pacific://room-post?object=$ROOM&text=$(enc "AW26 shoot — kickoff")"; sleep 4
sync_all 4
url "$SIM_B" "pacific://room-reply?object=$ROOM&text=$(enc "I'll lock the studio for the 14th")"; sleep 4
sync_all 4
url "$SIM_C" "pacific://room-reply?object=$ROOM&text=$(enc "sampling runs late — heads up")"; sleep 4
sync_all 4
# Alice replies to BOB's reply → genuine depth-2 nesting (match by substring).
url "$SIM_A" "pacific://room-reply?object=$ROOM&text=$(enc "perfect, booking it")&match=$(enc "lock the studio")"; sleep 4
sync_all 5

echo "— dump each thread + assert convergence —"
for s in "${SIMS[@]}"; do url "$s" "pacific://dump-thread?object=$ROOM"; done; sleep 3
TA=$(thread "$SIM_A"); TB=$(thread "$SIM_B"); TC=$(thread "$SIM_C")
echo "--- Alice's thread ---"; printf '%s\n' "$TA"
echo "--- Bob's thread ---";   printf '%s\n' "$TB"
echo "--- Carol's thread ---"; printf '%s\n' "$TC"
if [ -n "$TA" ] && [ "$TA" = "$TB" ] && [ "$TA" = "$TC" ]; then
  echo "✓ CONVERGED: all three devices fold the identical thread tree over real transport"
else
  echo "✗ DIVERGED (or empty thread)"; exit 1
fi

echo "— open the ChatRoom on each + screenshots —"
for s in "${SIMS[@]}"; do url "$s" "pacific://open-room?object=$ROOM"; done
sleep 3
mkdir -p /tmp/thread-demo
for i in 0 1 2; do xcrun simctl io "${SIMS[$i]}" screenshot "/tmp/thread-demo/${i}-${NAMES[$i]}.png" 2>/dev/null || true; done
echo "screenshots: /tmp/thread-demo/"
