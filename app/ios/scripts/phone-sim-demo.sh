#!/usr/bin/env bash
# phone-sim-demo.sh — a REAL iPhone and a simulator messaging over a local Arc relay.
#
# The sim-only scripts (sim-users.sh, two-sim-mls-demo.sh) can talk to ws://127.0.0.1
# because a simulator shares the host's network stack. A physical phone cannot: it has
# to reach the Mac across the LAN. That changes three things, and each one is a trap:
#
#   1. The relay must bind 0.0.0.0, not 127.0.0.1, and both peers must be pointed at
#      the Mac's LAN address — the sim included, so the two agree on one relay.
#   2. iOS gates a raw TCP connect to a private-range address behind the Local Network
#      permission. pacific-core dials the relay with tokio-tungstenite (a plain socket,
#      not URLSession), so the app needs NSLocalNetworkUsageDescription in its plist or
#      the connect is denied with no prompt. The simulator is exempt, which is why this
#      only ever bites on device. (project.yml carries the key.)
#   3. There is no `devicectl openurl`: the pacific:// dev seam that drives the sims does
#      not exist for a real device. So the phone's half — scan, accept, reply — is done
#      by hand, and this script stops and waits at each of those points.
#
# Prereqs: relay built (cd ../arc && just build), app built for BOTH destinations
#   (sim + device, see the header of scripts/version.sh for the device build line),
#   phone unlocked, paired, Developer Mode on, on the same Wi-Fi as this Mac.
# Usage: scripts/phone-sim-demo.sh
set -uo pipefail
cd "$(dirname "$0")/.."

SIM="${SIM:-4244D0F1-C55A-4ADD-AE15-06ACD9C1627D}"          # iPhone 17 Pro
DEVICE="${DEVICE:-47C32ABC-D993-5C11-A181-99F8BDC2B86D}"     # Ralph's iPhone (CoreDevice id)
BUNDLE_ID="network.pacific"
PORT="${PORT:-8787}"
LAN_IP="${LAN_IP:-$(ipconfig getifaddr en0)}"
RELAY_URL="ws://${LAN_IP}:${PORT}"
SEM="../arc/target/release/semaphore"
DD="$HOME/Library/Developer/Xcode/DerivedData/Pacific-hbaxaeikvebnczdxvinhflylvkdy/Build/Products"
SHOTS="${SHOTS:-/tmp/phone-sim}"; mkdir -p "$SHOTS"

[ -n "$LAN_IP" ] || { echo "no LAN address on en0 — is Wi-Fi up?"; exit 1; }
enc()  { python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$1"; }
url()  { xcrun simctl openurl "$SIM" "$1"; }
pause(){ printf '\n\033[1m>> %s\033[0m\nPress return when done… ' "$1"; read -r _; }

# ── relay ─────────────────────────────────────────────────────────────────────
# 0.0.0.0, not loopback: the phone dials this from another host on the LAN.
if ! nc -z "$LAN_IP" "$PORT" 2>/dev/null; then
  [ -x "$SEM" ] || { echo "semaphore not built — (cd ../arc && just build)"; exit 1; }
  echo "▸ starting relay on 0.0.0.0:$PORT"
  RELAY_BIND="0.0.0.0:$PORT" nohup "$SEM" >"$SHOTS/relay.log" 2>&1 &
  sleep 1
fi
echo "▸ relay $RELAY_URL"

# ── sim: fresh identity, pointed at the LAN relay ─────────────────────────────
echo "▸ sim: clean install + launch"
xcrun simctl bootstatus "$SIM" -b >/dev/null
xcrun simctl uninstall "$SIM" "$BUNDLE_ID" 2>/dev/null
xcrun simctl install "$SIM" "$DD/Debug-iphonesimulator/WallFlowers.app" || exit 1
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" \
SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="${SIM_NAME:-Sim Peer}" \
  xcrun simctl launch "$SIM" "$BUNDLE_ID" >/dev/null || exit 1

# ── phone: same relay, via devicectl's env passthrough ────────────────────────
# CoreService persists PACIFIC_RELAY_URL into the "pacific.relayUrl" default, so a later
# home-screen relaunch keeps this relay instead of falling back to the production Arc.
echo "▸ phone: install + launch"
xcrun devicectl device install app --device "$DEVICE" "$DD/Debug-iphoneos/WallFlowers.app" >/dev/null || exit 1
xcrun devicectl device process launch --device "$DEVICE" --terminate-existing \
  -e "{\"PACIFIC_RELAY_URL\":\"$RELAY_URL\"}" "$BUNDLE_ID" >/dev/null || {
  echo "launch failed — unlock the phone (SBMainWorkspace refuses to launch onto a locked screen)"; exit 1; }

# ── pairing: the phone scans the sim (a sim has no camera, so it can only be scanned) ──
C=$(xcrun simctl get_app_container "$SIM" "$BUNDLE_ID" data)
SIM_SPACE=$(sed -n 1p "$C/Library/Application Support/pacific/dev-contact.txt")
echo "▸ sim identity $SIM_SPACE"
url "pacific://my-qr"
pause "On the PHONE: allow Local Network when asked, then Me → Connect → Scan and scan the QR on the sim.
   Then ACCEPT on both sides (phone, and the sim's pairing banner) BEFORE anyone messages —
   replying on a stale epoch poisons the peer's inbox."

url "pacific://sync"; sleep 3

# The phone's space id lands in the sim's peers table once the handshake completes.
DB="$C/Library/Application Support/pacific/pacific.db"
PHONE_SPACE="space$(sqlite3 "$DB" "select lower(hex(peer_id)) from peers limit 1;" 2>/dev/null)"
[ "$PHONE_SPACE" = "space" ] && { echo "!! no peer in the sim's db — the handshake did not complete"; exit 1; }
echo "▸ phone identity $PHONE_SPACE"

# ── the exchange ──────────────────────────────────────────────────────────────
url "pacific://dm-send?peer=$PHONE_SPACE&text=$(enc 'From the simulator, over your Arc. No cloud in the path.')"
sleep 4; url "pacific://sync"; sleep 3
pause "On the PHONE: open the chat, confirm the message arrived, and reply."

url "pacific://sync"; sleep 4
url "pacific://open-dm?peer=$PHONE_SPACE"; sleep 3
xcrun simctl io "$SIM" screenshot "$SHOTS/sim.png" >/dev/null 2>&1
echo "▸ sim view: $SHOTS/sim.png   relay log: $SHOTS/relay.log"
echo "Both directions crossed the relay iff the sim shows your phone's reply."
