#!/usr/bin/env bash
# two-sim-project-demo.sh — reconstruct the AW26 Project across two iOS simulators
# over real MLS transport, then assert both boards converge byte-for-byte.
#
# Drives the app's pacific:// dev seam (DevLinks.swift) — every handler is a real
# core call, nothing bypasses the MLS group or invents state. Nye (SIM_A) is the
# project owner; Harvey (SIM_B) joins the project group and contributes progress.
#
# Prereqs: app built for x86_64 sim (WallFlowers.app in DerivedData), relay running on
#   127.0.0.1:8787 (RELAY_BIND=127.0.0.1:8787 ../Arc/target/release/semaphore &), two sims.
# Usage: scripts/two-sim-project-demo.sh
set -euo pipefail

SIM_A="${SIM_A:-1DECD6A0-A955-4644-BCCB-7F25258AAB63}"   # iPhone 16 Pro — Nye (owner)
SIM_B="${SIM_B:-FBC887EE-9ABE-4E67-B368-7D01620B15D3}"   # iPhone 16 — Harvey
RELAY_URL="${RELAY_URL:-ws://127.0.0.1:8787}"
RELAY_PORT="${RELAY_URL##*:}"
APP=$(ls -d "$HOME"/Library/Developer/Xcode/DerivedData/Pacific-*/Build/Products/Debug-iphonesimulator/WallFlowers.app | head -1)
BUNDLE_ID="network.pacific"

enc() { python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$1"; }
data_file() { # $1=udid $2=path under Application Support → stdout
  local c; c=$(xcrun simctl get_app_container "$1" "$BUNDLE_ID" data)
  cat "$c/Library/Application Support/$2" 2>/dev/null
}
contact()    { data_file "$1" "pacific/dev-contact.txt"; }
project_id() { data_file "$1" "pacific/dev-project.txt" | head -1; }
board()      { data_file "$1" "pacific/dev-board.txt"; }
open_url()   { xcrun simctl openurl "$1" "$2"; }

echo "— relay check (port $RELAY_PORT) —"
nc -z 127.0.0.1 "$RELAY_PORT" || { echo "start the relay: RELAY_BIND=127.0.0.1:$RELAY_PORT ../Arc/target/release/semaphore &"; exit 1; }

echo "— fresh install (fresh identities) —"
for S in "$SIM_A" "$SIM_B"; do
  xcrun simctl bootstatus "$S" -b >/dev/null
  xcrun simctl uninstall "$S" "$BUNDLE_ID" 2>/dev/null || true
  xcrun simctl install "$S" "$APP"
done
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="Nye Thompson"  xcrun simctl launch "$SIM_A" "$BUNDLE_ID"
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="Harvey Deane" xcrun simctl launch "$SIM_B" "$BUNDLE_ID"
sleep 8

B_BUNDLE=$(contact "$SIM_B" | sed -n 2p)
[ -n "$B_BUNDLE" ] || { echo "FAILED: no Harvey bundle"; exit 1; }

echo "— Nye mints the project + adds Harvey (the two-person hinge) —"
open_url "$SIM_A" "pacific://new-project"; sleep 3
PID=$(project_id "$SIM_A")
echo "project id: $PID"
[ -n "$PID" ] || { echo "FAILED: no project id exported"; exit 1; }
open_url "$SIM_A" "pacific://project-add-member?object=$PID&bundle=$(enc "$B_BUNDLE")"; sleep 6
open_url "$SIM_B" "pacific://sync"; sleep 5   # Harvey drains the Welcome, joins the project

echo "— seed the AW26 structure (Nye/owner) + each device's own progress —"
open_url "$SIM_A" "pacific://seed-project-demo?object=$PID"; sleep 6   # Nye: structure + his progress
open_url "$SIM_B" "pacific://sync"; sleep 4                            # Harvey drains the structure
open_url "$SIM_B" "pacific://seed-project-demo?object=$PID"; sleep 6   # Harvey: his progress
open_url "$SIM_A" "pacific://sync"; sleep 5                            # Nye drains Harvey's progress

echo "— dump both boards + assert convergence —"
open_url "$SIM_A" "pacific://dump-board?object=$PID"; sleep 2
open_url "$SIM_B" "pacific://dump-board?object=$PID"; sleep 2
BOARD_A=$(board "$SIM_A"); BOARD_B=$(board "$SIM_B")
echo "--- Nye's board ---";    printf '%s\n' "$BOARD_A"
echo "--- Harvey's board ---"; printf '%s\n' "$BOARD_B"
if [ -n "$BOARD_A" ] && [ "$BOARD_A" = "$BOARD_B" ]; then
  echo "✓ CONVERGED: both devices fold to the identical board over real transport"
else
  echo "✗ DIVERGED (or empty board)"; exit 1
fi

echo "— open the project on both + screenshots —"
open_url "$SIM_A" "pacific://open-project?object=$PID"
open_url "$SIM_B" "pacific://open-project?object=$PID"
sleep 3
xcrun simctl io "$SIM_A" screenshot /tmp/project-demo-A.png
xcrun simctl io "$SIM_B" screenshot /tmp/project-demo-B.png
echo "screenshots: /tmp/project-demo-A.png /tmp/project-demo-B.png"
