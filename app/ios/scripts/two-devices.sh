#!/usr/bin/env bash
#
# two-devices.sh — ONE PERSON, TWO DEVICES. Headed, and driven by hand.
#
#   scripts/two-devices.sh            two simulators, side by side (ERASES them)
#   scripts/two-devices.sh keep       the same, leaving their state alone
#   scripts/two-devices.sh phone      this Mac's simulator + your real iPhone
#   scripts/two-devices.sh stop       stop the relay this script started
#
# WHY THIS EXISTS WHEN THERE ARE ALREADY EIGHT SIM SCRIPTS. Every one of them
# stands up N DIFFERENT PEOPLE — sim-users.sh is "N simulators as N real Pacific
# users", sim-fleet.sh is three identities. None of them has ever put ONE account
# on two devices, which is the case `m21_one_person_two_devices` proves in core
# and the one the product claim rests on. So this is not a variant of those; it
# is the case they do not cover.
#
# IT IS HAND-DRIVEN ON PURPOSE. The `pacific://` dev seam (Connect/DevLinks.swift)
# has verbs for pairing, DMs, orgs, events and tickets — and none for RESTORE or
# for ADD MEMBER, which are the two steps this test is made of. So the script
# sets the stage and then stops: it boots the devices, points them at one relay,
# opens them at the door, and tells you what to tap. Watching it happen is the
# point.
#
# WHAT YOU ARE LOOKING FOR, in order:
#   1. the SAME identity key and the SAME three fingerprint words on both devices
#      — that is one account, not two (m15_identity_restore)
#   2. device 1 can add device 2 to a group and the roster reads TWO PEOPLE with
#      THREE leaves — not two people counted twice (m21, mls::PerLeafIdentity)
#   3. a post from device 2 arrives on device 1 AND DOES NOT DESTROY what was
#      there before it joined (the gen watermark, fixed 15 Sep 2026 —
#      IntroPayload::gen_watermark; before it, device 2's first post overwrote
#      device 1's earlier one on every device that held both)
#
# WHAT WILL NOT WORK, AND IS NOT A BUG. Device 2 arrives with NO HISTORY. MLS
# forward secrecy means a leaf that joins at epoch N cannot read anything sealed
# before it, and `add_member_core` refuses to re-encrypt history precisely so that
# stays true. So device 2 sees only what is posted AFTER it joins. The archive
# chain that would fix this is specified and not built — no surface writes a history yet.
set -uo pipefail
cd "$(dirname "$0")/.."

MODE="${1:-sims}"
# `keep` leaves whatever is already on the simulators alone. The default ERASES
# them, because this test starts by making a NEW account and a simulator that
# already holds an identity cannot reach the door (see boot_and_launch).
KEEP=0; [ "${2:-}" = keep ] && KEEP=1; [ "$MODE" = keep ] && { MODE=sims; KEEP=1; }
BUNDLE=network.pacific
PORT="${PORT:-8787}"
SEM="../arc/target/release/semaphore"
A="${A:-4244D0F1-C55A-4ADD-AE15-06ACD9C1627D}"   # iPhone 17 Pro
B="${B:-E36E02F6-A025-4664-B47C-FE1FD2C308EA}"   # iPhone 17
APP=$(ls -d ui/build/Build/Products/Debug-iphonesimulator/WallFlowers.app 2>/dev/null \
      || ls -d "$HOME"/Library/Developer/Xcode/DerivedData/Pacific-*/Build/Products/Debug-iphonesimulator/WallFlowers.app 2>/dev/null | head -1)

if [ "$MODE" = stop ]; then
  pkill -f "semaphore" 2>/dev/null && echo "relay stopped" || echo "no relay of ours was running"
  exit 0
fi

[ -n "$APP" ] && [ -d "$APP" ] || { echo "no WallFlowers.app — build it first:  just build"; exit 1; }

# ── the relay both devices converge through ──────────────────────────────────
# A simulator shares the host's network stack, so loopback is enough for two of
# them. A PHYSICAL PHONE cannot: it dials this Mac across the LAN, so the relay
# has to bind 0.0.0.0 and BOTH peers must be given the LAN address — the
# simulator included, or the two ends are on different relays and nothing meets.
if [ "$MODE" = phone ]; then
  LAN=$(ipconfig getifaddr en0 || true)
  [ -n "$LAN" ] || { echo "no LAN address on en0 — is Wi-Fi up?"; exit 1; }
  BIND="0.0.0.0:$PORT"; RELAY="ws://$LAN:$PORT"
else
  BIND="127.0.0.1:$PORT"; RELAY="ws://127.0.0.1:$PORT"
fi

if ! nc -z "${BIND%%:*}" "$PORT" 2>/dev/null; then
  [ -x "$SEM" ] || { echo "semaphore not built — (cd ../arc && cargo build --release -p relay)"; exit 1; }
  echo "▸ relay on $BIND"
  RELAY_BIND="$BIND" RELAY_STORE=/tmp/two-devices-relay.db nohup "$SEM" >/tmp/two-devices-relay.log 2>&1 &
  sleep 2
else
  echo "▸ relay already up on $PORT"
fi

boot_and_launch() {          # $1 udid  $2 label
  # ERASED FIRST, and this is not tidiness — it is the only thing that works.
  #
  # `-pacificResetFirstBoot` re-opens the SEQUENCE and never destroys a key, by
  # design: no test seam may take away an identity. So on a simulator that has
  # been used before, the sequence RESUMES at the collecting steps instead of
  # opening the door, and you cannot make a new account on it. Measured here on
  # 15 Sep 2026 — both simulators came up on the FACE step wearing initials from
  # an earlier run ("A7" and "BO") rather than at the door.
  #
  # `simctl uninstall` is NOT enough either: it takes the data container and the
  # identity outlives it, so the app comes back still holding a key. `erase` is
  # the one that gives you a device with none.
  if [ "$KEEP" != 1 ]; then
    xcrun simctl shutdown "$1" >/dev/null 2>&1 || true
    xcrun simctl erase "$1" >/dev/null 2>&1 || true
  fi
  xcrun simctl bootstatus "$1" -b >/dev/null 2>&1 || true
  xcrun simctl install "$1" "$APP" >/dev/null
  xcrun simctl terminate "$1" "$BUNDLE" >/dev/null 2>&1 || true
  SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY" \
  SIMCTL_CHILD_PACIFIC_SKIP_MOCK_SEED=1 \
    xcrun simctl launch "$1" "$BUNDLE" \
      -pacificResetFirstBoot -pacificNoWake -pacificNoMusic -pacificNoArt >/dev/null
  echo "▸ $2 up  ($1)"
}

[ "$KEEP" = 1 ] || echo "▸ erasing both simulators  (pass 'keep' to skip)"
open -a Simulator            # HEADED — you are going to tap these
boot_and_launch "$A" "DEVICE 1"
[ "$MODE" = phone ] || boot_and_launch "$B" "DEVICE 2"

cat <<EOF

  relay   $RELAY   (log: /tmp/two-devices-relay.log)
EOF

if [ "$MODE" = phone ]; then
  cat <<EOF
  DEVICE 2 IS YOUR IPHONE, and three things bite only on device:
    · it must be on THIS Wi-Fi, unlocked, paired, Developer Mode on
    · iOS gates a raw socket to a private address behind the Local Network
      permission — pacific-core dials with tokio-tungstenite, not URLSession, so
      ALLOW the prompt or the connect is denied silently
    · there is no \`devicectl openurl\`, so nothing can be scripted on it at all
  Build and run it from Xcode onto the phone, with PACIFIC_RELAY_URL=$RELAY
  in the scheme's environment.
EOF
fi

cat <<EOF

  NOW, BY HAND — one account, two devices

  The door says "Sign In:" and offers three: Passkey · Recovery key · New account.

  DEVICE 1   New account ▸ name ▸ face ▸ bio
             ▸ the RECOVERY KEY screen — write the 24 words down. They are shown
               ONCE and stored by us nowhere; you need them on device 2 next.
             ▸ fingerprint ▸ ENTER

  DEVICE 2   Recovery key ▸ type those 24 words
             → the SAME three fingerprint words as device 1 = ONE ACCOUNT, and
               that is the first thing worth seeing

  (Passkey is the third door and needs a real HTTPS origin, not a simulator.)

  DEVICE 1   add device 2 to a group, the way you would add anyone
  DEVICE 2   post into it — it arrives on device 1
  DEVICE 1   post into it — it arrives on device 2
             and everything device 1 wrote BEFORE the add is still there

  WATCH IT   cd app/ios && ./watch.sh        device 1's narration
             tail -f /tmp/two-devices-relay.log   what the relay sees

  stop the relay:  scripts/two-devices.sh stop

EOF
