#!/usr/bin/env bash
# three-sim-sysiphus-kg-demo.sh — the KG moving REAL data across three iOS sims.
#
# Three independent simulators, each signed into the REAL Sysiphus backend (:8787) as a
# different employee, share one ChatRoom over real MLS transport. A message that NAMES a real
# Sysiphus design ("Ram Cargo Pant" — a live product name, present in NO mock seed) is posted on
# one device, syncs to the others, and on EVERY device the design resolves + highlights from that
# device's own credentialed Sysiphus pull. Real data source (Sysiphus) + real transport (MLS) +
# on-device KG resolution, all three at once.
#
# Drives the app's pacific:// dev seam (DevLinks.swift) — every handler is a real core / System
# call. Prereqs: WallFlowers.app built (ui/build/dd), relay built (../Arc/target/release/semaphore),
# Sysiphus up on localhost:8787, three booted sims.
set -euo pipefail

SIM_A="${SIM_A:-1DECD6A0-A955-4644-BCCB-7F25258AAB63}"   # iPhone 16 Pro — Alice / signs in as sofia
SIM_B="${SIM_B:-FBC887EE-9ABE-4E67-B368-7D01620B15D3}"   # iPhone 16 — Bob / fergus
SIM_C="${SIM_C:-C3D941F2-D1EC-4822-BF40-86E40065EFFD}"   # iPhone SE — Carol / nye
SIMS=("$SIM_A" "$SIM_B" "$SIM_C")
NAMES=("Alice" "Bob" "Carol")
EMPLOYEES=("sofia" "fergus" "nye")
BUNDLE_ID="network.pacific"
# Repo root derived from this script's location (scripts/ -> repo root); the app builds
# into ui/build/dd. Override APP to point at a different DerivedData products dir.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="${APP:-$REPO_ROOT/ui/build/dd/Build/Products/Debug-iphonesimulator/WallFlowers.app}"
[ -d "$APP" ] || { echo "WallFlowers.app not found at $APP — build first."; exit 1; }

curl -s -m 4 http://localhost:8787/health >/dev/null 2>&1 || { echo "Sysiphus down: nothing answers on localhost:8787"; exit 1; }

enc() { python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$1"; }
data_file() { local c; c=$(xcrun simctl get_app_container "$1" "$BUNDLE_ID" data); cat "$c/Library/Application Support/$2" 2>/dev/null; }
contact()   { data_file "$1" "pacific/dev-contact.txt"; }
room_id()   { data_file "$1" "pacific/dev-room.txt" | head -1; }
thread()    { data_file "$1" "pacific/dev-thread.txt"; }
url()       { xcrun simctl openurl "$1" "$2" 2>/dev/null || true; }
sync_all()  { for s in "${SIMS[@]}"; do url "$s" "pacific://sync"; done; sleep "${1:-3}"; }

listening() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
PORT=8810; while listening "$PORT"; do PORT=$((PORT + 1)); done
RELAY_URL="ws://127.0.0.1:${PORT}"
echo "— local relay ($RELAY_URL) + Sysiphus (:8787, $(curl -s http://localhost:8787/health)) —"
[ -x ../Arc/target/release/semaphore ] || { echo "relay missing: (cd ../Arc && cargo build -p relay --release)"; exit 1; }
RELAY_BIND="127.0.0.1:$PORT" nohup ../Arc/target/release/semaphore >"/tmp/semaphore-$PORT.log" 2>&1 &
RELAY_PID=$!
trap 'kill "$RELAY_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 30); do listening "$PORT" && break; sleep 0.3; done
listening "$PORT" || { echo "relay failed to start"; exit 1; }
echo "  semaphore up (pid $RELAY_PID)"

echo "— boot + clean-install 3 users, each SIGNS INTO real Sysiphus —"
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
  echo "  [$i] ${NAMES[$i]} → signs in as Sysiphus '${EMPLOYEES[$i]}'"
done
sleep 8

B_BUNDLE=$(contact "$SIM_B" | sed -n 2p); [ -n "$B_BUNDLE" ] || { echo "FAILED: no Bob bundle"; exit 1; }
C_BUNDLE=$(contact "$SIM_C" | sed -n 2p); [ -n "$C_BUNDLE" ] || { echo "FAILED: no Carol bundle"; exit 1; }

echo "— Alice mints the ChatRoom + adds Bob and Carol (real MLS) —"
url "$SIM_A" "pacific://new-room"; sleep 3
ROOM=$(room_id "$SIM_A"); echo "  room id: $ROOM"
[ -n "$ROOM" ] || { echo "FAILED: no room id"; exit 1; }
url "$SIM_A" "pacific://room-add-member?object=$ROOM&bundle=$(enc "$B_BUNDLE")"; sleep 6
url "$SIM_B" "pacific://sync"; sleep 4
url "$SIM_A" "pacific://room-add-member?object=$ROOM&bundle=$(enc "$C_BUNDLE")"; sleep 6
url "$SIM_C" "pacific://sync"; sleep 4
sync_all 4

echo "— post a thread naming REAL Sysiphus designs (resolve on every device) —"
url "$SIM_A" "pacific://room-post?object=$ROOM&text=$(enc "Ram Cargo Pant ships Friday — who's shooting it?")"; sleep 4
sync_all 4
url "$SIM_B" "pacific://room-reply?object=$ROOM&text=$(enc "I'll shoot the Ram Cargo Pant with the Arise Hoodie")"; sleep 4
sync_all 4
url "$SIM_C" "pacific://room-reply?object=$ROOM&text=$(enc "Astral Track Jacket stock is low — flag it")"; sleep 4
sync_all 5

echo "— assert the thread text converged across all three —"
for s in "${SIMS[@]}"; do url "$s" "pacific://dump-thread?object=$ROOM"; done; sleep 3
TA=$(thread "$SIM_A"); TB=$(thread "$SIM_B"); TC=$(thread "$SIM_C")
printf '%s\n' "$TA"
if [ -n "$TA" ] && [ "$TA" = "$TB" ] && [ "$TA" = "$TC" ]; then
  echo "✓ CONVERGED across 3 devices over real MLS"
else
  echo "✗ DIVERGED"; echo "--B--"; printf '%s\n' "$TB"; echo "--C--"; printf '%s\n' "$TC"
fi

echo "— NOW each device signs into its OWN Sysiphus (feeds its on-device catalogue) —"
for i in 0 1 2; do url "${SIMS[$i]}" "pacific://sysiphus-signin?u=${EMPLOYEES[$i]}"; echo "  ${NAMES[$i]} → Sysiphus '${EMPLOYEES[$i]}'"; done
sleep 10                                       # each pulls its Fields + re-indexes

echo "— open the ChatRoom on each + screenshot (design names should highlight from live Sysiphus) —"
for s in "${SIMS[@]}"; do url "$s" "pacific://open-room?object=$ROOM"; done
sleep 4
mkdir -p /tmp/sysiphus-kg-demo
for i in 0 1 2; do xcrun simctl io "${SIMS[$i]}" screenshot "/tmp/sysiphus-kg-demo/${i}-${NAMES[$i]}.png" 2>/dev/null || true; done
echo "screenshots: /tmp/sysiphus-kg-demo/"
