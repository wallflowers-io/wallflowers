#!/usr/bin/env bash
# two-sim-mls-demo.sh — two iOS simulators talking over MLS via a local relay.
#
# Drives the REAL pairing handshake + DMs through the app's pacific:// dev seam
# (DevLinks.swift — every handler is a real core call). Sequence matters:
# BOTH sides accept before anyone messages — replying on a stale epoch poisons
# the peer's inbox (known core issue: sync aborts on an unprocessable blob
# instead of skipping it; see summary note).
#
# Prereqs: app built for x86_64 sim (WallFlowers.app in DerivedData), relay built
#   ((cd ../Arc && cargo build -p relay --release)), two simulators (UDIDs below).
# Usage: scripts/two-sim-mls-demo.sh
set -euo pipefail

SIM_A="${SIM_A:-1DECD6A0-A955-4644-BCCB-7F25258AAB63}"   # iPhone 16 Pro
SIM_B="${SIM_B:-FBC887EE-9ABE-4E67-B368-7D01620B15D3}"   # iPhone 16
RELAY_URL="ws://127.0.0.1:8787"
APP=$(ls -d "$HOME"/Library/Developer/Xcode/DerivedData/Pacific-*/Build/Products/Debug-iphonesimulator/WallFlowers.app | head -1)
BUNDLE_ID="network.pacific"

enc() { python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$1"; }
contact() { # $1=udid → prints "spaceid\nbundle"
  local c; c=$(xcrun simctl get_app_container "$1" "$BUNDLE_ID" data)
  cat "$c/Library/Application Support/pacific/dev-contact.txt"
}
open_url() { xcrun simctl openurl "$1" "$2"; }

echo "— relay check —"
nc -z 127.0.0.1 8787 || { echo "start the relay first: RELAY_BIND=127.0.0.1:8787 ../Arc/target/release/semaphore &"; exit 1; }

echo "— fresh install (fresh identities) —"
for S in "$SIM_A" "$SIM_B"; do
  xcrun simctl bootstatus "$S" -b >/dev/null
  xcrun simctl uninstall "$S" "$BUNDLE_ID" 2>/dev/null || true
  xcrun simctl install "$S" "$APP"
done
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="Nye Thompson"  xcrun simctl launch "$SIM_A" "$BUNDLE_ID"
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="Harvey Deane" xcrun simctl launch "$SIM_B" "$BUNDLE_ID"
sleep 8

A_SPACE=$(contact "$SIM_A" | head -1);  A_BUNDLE=$(contact "$SIM_A" | sed -n 2p)
B_SPACE=$(contact "$SIM_B" | head -1)
echo "A=$A_SPACE"
echo "B=$B_SPACE"

echo "— handshake (both accepts BEFORE any message) —"
open_url "$SIM_B" "pacific://pair-scan?bundle=$(enc "$A_BUNDLE")"; sleep 6   # B scans A → Welcome via relay
open_url "$SIM_A" "pacific://sync"; sleep 5                                  # A ingests the Welcome
open_url "$SIM_A" "pacific://pair-accept?peer=$B_SPACE"; sleep 2             # A accepts
open_url "$SIM_B" "pacific://pair-accept?peer=$A_SPACE"; sleep 2             # B accepts (double-opt-in)
open_url "$SIM_B" "pacific://sync"; sleep 5                                  # B catches the current epoch

echo "— messages —"
open_url "$SIM_A" "pacific://dm-send?peer=$B_SPACE&text=$(enc 'Harvey — this is the real thing. two devices, two identities, one relay. no whatsapp.')"; sleep 5
open_url "$SIM_B" "pacific://sync"; sleep 5
open_url "$SIM_B" "pacific://dm-send?peer=$A_SPACE&text=$(enc 'receiving you loud and clear. MLS says hi 🤝')"; sleep 5
open_url "$SIM_A" "pacific://sync"; sleep 5

echo "— open the conversation on both + screenshots —"
open_url "$SIM_A" "pacific://open-dm?peer=$B_SPACE"
open_url "$SIM_B" "pacific://open-dm?peer=$A_SPACE"
sleep 3
xcrun simctl io "$SIM_A" screenshot /tmp/mls-demo-A.png
xcrun simctl io "$SIM_B" screenshot /tmp/mls-demo-B.png
echo "screenshots: /tmp/mls-demo-A.png /tmp/mls-demo-B.png"
