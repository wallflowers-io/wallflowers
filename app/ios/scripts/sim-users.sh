#!/usr/bin/env bash
# sim-users.sh — N iOS simulators as N REAL Pacific users over a local relay.
#
# THE robust "spin up real users" harness for manual/visual testing. Unlike the
# older two-sim scripts, this one:
#   • sets PACIFIC_SKIP_MOCK_SEED=1 so NO demo/sample threads are seeded — every
#     conversation you see is the residue of real ops (fixes "demo data back in
#     the chats"; the old scripts launched without the gate, so ChatSeed minted
#     Sample.threads on top of the real DM);
#   • auto-starts the local relay if it isn't already up;
#   • boots as many sims as it needs and clean-installs a FRESH identity on each;
#   • pairs everyone into a full mesh (real MLS double-opt-in) and seeds a hello.
#
# The headless proof of N-user convergence is `cargo test -p pacific-core` (the
# `common::Harness` + m3/m4 suites); THIS is the on-device, eyes-on companion.
#
# Prereqs: app built for a simulator (WallFlowers.app in DerivedData), relay built
#   ((cd ../Arc && cargo build -p relay --release) → ../Arc/target/release/semaphore).
# Usage:  scripts/sim-users.sh [N]          # N defaults to 3 (Alice/Bob/Carol)
#   Override devices: DEVICES="udidA udidB udidC" scripts/sim-users.sh
#   Override port:    PORT=8790 scripts/sim-users.sh
set -euo pipefail

N="${1:-3}"
NAMES=(Alice Bob Carol Dana Erin)
DEFAULT_DEVICES=(
  1DECD6A0-A955-4644-BCCB-7F25258AAB63   # iPhone 16 Pro
  FBC887EE-9ABE-4E67-B368-7D01620B15D3   # iPhone 16
  C3D941F2-D1EC-4822-BF40-86E40065EFFD   # iPhone SE (3rd generation)
  8C8EF16D-0787-49C7-8985-8C26D2D8316B   # iPhone 16 Pro Max
  8430F6D4-9459-41D6-87E6-A4F7B18044E1   # iPhone 16 Plus
)
read -r -a DEVICES <<< "${DEVICES:-${DEFAULT_DEVICES[*]}}"
PORT="${PORT:-8787}"
RELAY_URL="ws://127.0.0.1:${PORT}"
BUNDLE_ID="network.pacific"
APP=$(ls -d "$HOME"/Library/Developer/Xcode/DerivedData/Pacific-*/Build/Products/Debug-iphonesimulator/WallFlowers.app 2>/dev/null | head -1)

enc()  { python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$1"; }
url()  { xcrun simctl openurl "$1" "$2"; }
# Tolerant openurl for the cosmetic tail (a sim can drop under the load of several
# running at once; a failed hello/screenshot must not abort the whole harness).
try_url() { xcrun simctl openurl "$1" "$2" 2>/dev/null || true; }
# Re-boot any device that has dropped to Shutdown (idempotent on booted ones).
ensure_booted() { for d in "$@"; do xcrun simctl bootstatus "$d" -b >/dev/null 2>&1 || true; done; }
# "spaceid\nbundle" that the app exports to its sandbox on first launch.
contact() {
  local c; c=$(xcrun simctl get_app_container "$1" "$BUNDLE_ID" data)
  cat "$c/Library/Application Support/pacific/dev-contact.txt"
}
# Retry reading the exported contact until the identity has minted (≤ ~15s).
wait_contact() {
  for _ in $(seq 1 30); do
    if contact "$1" >/dev/null 2>&1 && [ -n "$(contact "$1" | head -1)" ]; then return 0; fi
    sleep 0.5
  done
  echo "!! $1 never exported its contact (identity mint failed?)"; return 1
}

[ -n "$APP" ] || { echo "WallFlowers.app not found in DerivedData — build the app for a simulator first."; exit 1; }
(( N >= 2 )) || { echo "need at least 2 users"; exit 1; }
(( N <= ${#DEVICES[@]} )) || { echo "need $N devices, only ${#DEVICES[@]} configured"; exit 1; }

# Always run OUR OWN relay on a guaranteed-free port. Do NOT trust an existing
# listener on a fixed port — another service (or a stale process) may be squatting
# it, and the app would then talk to the wrong server (a silent-wrong-relay trap).
listening() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
while listening "$PORT"; do PORT=$((PORT + 1)); done
RELAY_URL="ws://127.0.0.1:${PORT}"
echo "— local relay ($RELAY_URL) —"
RELAY_BIND="127.0.0.1:$PORT" nohup ../Arc/target/release/semaphore >"/tmp/semaphore-$PORT.log" 2>&1 &
RELAY_PID=$!
for _ in $(seq 1 30); do listening "$PORT" && break; sleep 0.3; done
listening "$PORT" || { echo "  relay failed to start (see /tmp/semaphore-$PORT.log)"; exit 1; }
echo "  semaphore up (pid $RELAY_PID) on $RELAY_URL"

echo "— boot + clean-install $N users (NO mock seed) —"
declare -a SPACE
for ((i=0; i<N; i++)); do
  D="${DEVICES[$i]}"; NAME="${NAMES[$i]}"
  xcrun simctl bootstatus "$D" -b >/dev/null                    # boots if shutdown, waits till ready
  xcrun simctl terminate "$D" "$BUNDLE_ID" 2>/dev/null || true
  xcrun simctl uninstall "$D" "$BUNDLE_ID" 2>/dev/null || true  # clean slate — no stale/demo data
  xcrun simctl install "$D" "$APP"
  SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" \
  SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="$NAME" \
  SIMCTL_CHILD_PACIFIC_SKIP_MOCK_SEED=1 \
    xcrun simctl launch "$D" "$BUNDLE_ID" >/dev/null
  echo "  [$i] $NAME → $D"
done

echo "— read contact bundles —"
for ((i=0; i<N; i++)); do
  wait_contact "${DEVICES[$i]}"
  SPACE[$i]=$(contact "${DEVICES[$i]}" | head -1)
  echo "  ${NAMES[$i]} = ${SPACE[$i]}"
done

# i scans j's bundle → connection forms; the Welcome lands on j; both opt in.
# Space ids are passed in (si=i's, sj=j's) rather than read from a global inside
# the function — clearer, and no `set -u` surprises on array-element scope.
pair() {
  local i="$1" j="$2" si="$3" sj="$4" jb old
  # Fresh key package per add: MLS key packages are single-use, so in a mesh (where
  # j is added to multiple groups) j must re-export a NEW bundle before each add or
  # the 2nd join hits "key package not found". export-contact is async (openurl →
  # main-actor write), so POLL until the bundle actually changes — a fixed sleep
  # races the write and the scanner would grab the STALE, already-consumed KP.
  old=$(contact "${DEVICES[$j]}" | sed -n 2p)
  url "${DEVICES[$j]}" "pacific://export-contact"
  jb=""
  for _ in $(seq 1 24); do
    jb=$(contact "${DEVICES[$j]}" | sed -n 2p)
    { [ -n "$jb" ] && [ "$jb" != "$old" ]; } && break
    sleep 0.5
  done
  if [ -z "$jb" ]; then
    echo "  !! ${NAMES[$j]} (${DEVICES[$j]}) is unreachable — cannot pair ${NAMES[$i]} ⇄ ${NAMES[$j]}"
    echo "     (this Mac may not sustain $N simulators at once; see the note the harness prints)"
    return 1
  fi
  url "${DEVICES[$i]}" "pacific://pair-scan?bundle=$(enc "$jb")"; sleep 6   # i scans j
  url "${DEVICES[$j]}" "pacific://sync";                sleep 5             # j ingests the Welcome
  url "${DEVICES[$j]}" "pacific://pair-accept?peer=$si"; sleep 2            # j accepts i
  url "${DEVICES[$i]}" "pacific://pair-accept?peer=$sj"; sleep 2            # i accepts j (double opt-in)
  url "${DEVICES[$i]}" "pacific://sync";                sleep 4
  echo "  paired ${NAMES[$i]} ⇄ ${NAMES[$j]}"
}

echo "— pair everyone into a mesh (real MLS handshakes) —"
FAILED=()
for ((i=0; i<N; i++)); do
  for ((j=i+1; j<N; j++)); do
    ensure_booted "${DEVICES[$i]}" "${DEVICES[$j]}"   # re-boot either if it dropped
    pair "$i" "$j" "${SPACE[$i]}" "${SPACE[$j]}" || FAILED+=("${NAMES[$i]}⇄${NAMES[$j]}")
  done
done

echo "— settle (drain any pending Welcomes/commits so every side is at the latest) —"
ensure_booted "${DEVICES[@]:0:$N}"
for _ in 1 2; do
  for ((i=0; i<N; i++)); do try_url "${DEVICES[$i]}" "pacific://sync"; sleep 2; done
done

echo "— a ring of real hellos —"
ensure_booted "${DEVICES[@]:0:$N}"     # re-boot any sim that dropped during pairing
for ((i=0; i<N; i++)); do
  j=$(( (i+1) % N ))
  try_url "${DEVICES[$i]}" "pacific://dm-send?peer=${SPACE[$j]}&text=$(enc "hello from ${NAMES[$i]} 👋")"; sleep 4
done
for ((i=0; i<N; i++)); do try_url "${DEVICES[$i]}" "pacific://sync"; sleep 3; done   # settle

echo "— open a conversation on each + screenshots —"
for ((i=0; i<N; i++)); do
  j=$(( (i+1) % N ))
  try_url "${DEVICES[$i]}" "pacific://open-dm?peer=${SPACE[$j]}"
done
sleep 3
mkdir -p /tmp/sim-users
for ((i=0; i<N; i++)); do
  xcrun simctl io "${DEVICES[$i]}" screenshot "/tmp/sim-users/${i}-${NAMES[$i]}.png" 2>/dev/null || true
done

echo
if [ "${#FAILED[@]}" -eq 0 ]; then
  echo "✓ $N real users live on $RELAY_URL — no mock data, all conversations are real ops."
else
  echo "⚠ $N users launched, but these pairings FAILED (a sim dropped): ${FAILED[*]}"
  echo "  This Mac may not sustain $N simulators at once. The RELIABLE N-user proof is"
  echo "  headless: \`cargo test -p pacific-core --test m4_three_user_group\` (real relay, real MLS)."
fi
for ((i=0; i<N; i++)); do echo "  [$i] ${NAMES[$i]}  ${DEVICES[$i]}  ${SPACE[$i]}"; done
echo "  screenshots: /tmp/sim-users/"
echo "  drive more:  xcrun simctl openurl <udid> 'pacific://dm-send?peer=<space>&text=...'"
