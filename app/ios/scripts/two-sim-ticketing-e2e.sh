#!/usr/bin/env bash
# two-sim-ticketing-e2e.sh — the whole ticket, end to end, on two real simulators:
#
#   A LISTS a ticketed event → B BUYS → A SCANS B's ticket at the door → BOTH GO GREEN.
#
# Every step is a REAL core call driven through the app's `pacific://` dev seam
# (DevLinks.swift). Nothing is faked: two devices, two identities, one relay, the real
# MLS pairing handshake, real signed credentials, and the real DoorScanSheet rendering
# its verdict. The seam exists because a simulator has NO CAMERA and a host script has
# no fingers — `door-scan` hands the wallet's verbatim QR to the same `onScanned` path a
# camera would, and `show-tickets?ticket=` opens the cover a thumb would.
#
# THE TWO GREENS, and why they are two different facts:
#   · A (door)   — `event.redeem` lands in the EVENT fold; the sheet shows VALID.
#   · B (holder) — the buyer is DELIBERATELY NOT a member of the Event group, so the
#                  redemption is invisible to them. `contact.admitTicket` on the pairwise
#                  channel is the only truthful route, and it is what turns B's ticket
#                  green. No receipt, no green — by design, not by omission.
#
# HONEST LIMITATION — the PRICED path cannot complete today. A priced sale needs the
# Arc-side fulfilment delegate (Slice 2B) to verify payment and author `recordSale`; that
# agent is unbuilt, so nothing can author the sale and no credential is ever issued.
# `--priced` therefore SKIPS with that reason rather than pretending. The FREE path this
# harness drives is phone-fulfilled and exercises the entire state machine minus the money
# hops: listing → request → sale → credential → delivery → redemption → receipt.
#
# Usage:
#   scripts/two-sim-ticketing-e2e.sh            # the free path, end to end
#   scripts/two-sim-ticketing-e2e.sh --priced   # skips, loudly, with the reason
#   SIM_A=<udid> SIM_B=<udid> scripts/…         # bring your own simulators
#   RELAY_URL=wss://arc.wallflowers.io/v1/relay scripts/…   # a deployed relay instead of a local one
#   SKIP_BUILD=1 scripts/…                      # reuse the app already in ui/build
#
# RELAY: the default is a LOCAL relay on the first free port from 8801, started here and
# killed on exit — the two-sim scripts' choice, because a green run must not depend on the
# internet or on someone else's deploy. RELAY_URL overrides it.
#
# Idempotent: it owns two named simulators (E2E-Ticketing-A/B), creates them when absent,
# reuses them when present, and always uninstalls + resets the keychain so every run
# starts from freshly minted identities. No manual cleanup between runs.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UI_DIR="$ROOT/ui"
ARC_DIR="$(cd "$ROOT/../Arc" 2>/dev/null && pwd || true)"
BUNDLE_ID="network.pacific"
SHOTS="$UI_DIR/build/shots"
MODEL_A="${MODEL_A:-iPhone 17 Pro}"
MODEL_B="${MODEL_B:-iPhone 17}"

STEP="startup"
step() { STEP="$1"; printf '\n— %s —\n' "$1"; }
fail() { echo; echo "✗ FAILED at [$STEP]: $*" >&2; exit 1; }

# ── --priced: ready for the day Slice 2B lands, honest until then ─────────────
if [ "${1:-}" = "--priced" ]; then
  cat >&2 <<'EOF'
SKIPPED: the priced path requires Slice 2B: the delegate agent.

A priced listing names an Arc as its fulfilment `delegate`. That agent is what verifies
the payment intent and authors `event.recordSale` — the hop that turns a paid checkout
into a credential. It is unbuilt, so a priced sale stalls at the request with nothing to
answer it, and any "pass" printed here would be theatre.

The FREE path (this script with no flags) drives the identical state machine minus the
money hops: listing → request → sale → credential → delivery → redemption → receipt.
EOF
  exit 0
fi
[ -z "${1:-}" ] || fail "unknown argument '$1' (only --priced is understood)"

enc() { python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$1"; }
dev_dir()  { # $1=udid → the app's own pacific/ support dir on the host filesystem
  local c; c=$(xcrun simctl get_app_container "$1" "$BUNDLE_ID" data 2>/dev/null) || return 1
  echo "$c/Library/Application Support/pacific"
}
dev_file() { # $1=udid $2=filename → stdout (empty when absent)
  local d; d=$(dev_dir "$1") || return 0
  cat "$d/$2" 2>/dev/null || true
}
# A `pacific://` link into a RUNNING app. NOT `simctl openurl`: on iOS 26 every custom
# scheme open raises an "Open in Pacific?" SpringBoard alert, and nothing on the host can
# tap it (the simulator takes no synthetic touches from a shell — see the ios-ui-debug
# note). The app drains this append-only file every second through the SAME
# DevLinks.handle that `.onOpenURL` calls, so no step is weakened by the change of door.
open_url() { # <udid> <pacific:// url>
  local d; d=$(dev_dir "$1") || return 0
  mkdir -p "$d"
  printf '%s\n' "$2" >> "$d/dev-commands.txt"
  sleep "${LINK_SETTLE:-1.5}"   # the app's drain interval is 1s
}
# Poll a predicate until it holds. Every wait here is a real convergence wait — never a
# blind sleep long enough to look green.
until_ok() { # <budget seconds> <label> <cmd…>  — wall-clock, since a probe may itself be slow
  local budget="$1" label="$2"; shift 2
  local deadline=$(( $(date +%s) + budget ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if "$@"; then return 0; fi
    sleep 1
  done
  fail "$label did not converge in ${budget}s"
}

# ── the relay ────────────────────────────────────────────────────────────────
step "relay"
if [ -n "${RELAY_URL:-}" ]; then
  echo "using $RELAY_URL (external — no local relay started)"
else
  [ -n "$ARC_DIR" ] || fail "no ../Arc checkout for the relay; set RELAY_URL to use a deployed one"
  RELAY_BIN="$ARC_DIR/target/release/semaphore"
  [ -x "$RELAY_BIN" ] || RELAY_BIN="$ARC_DIR/target/debug/semaphore"
  if [ ! -x "$RELAY_BIN" ]; then
    echo "building the relay (cargo build -p relay)…"
    (cd "$ARC_DIR" && ~/.cargo/bin/cargo build -p relay) || fail "the relay would not build"
    RELAY_BIN="$ARC_DIR/target/debug/semaphore"
  fi
  PORT=8801
  while lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; do PORT=$((PORT + 1)); done
  RELAY_URL="ws://127.0.0.1:$PORT"
  RELAY_LOG="$UI_DIR/build/e2e-relay.log"
  mkdir -p "$(dirname "$RELAY_LOG")"
  RELAY_BIND="127.0.0.1:$PORT" "$RELAY_BIN" > "$RELAY_LOG" 2>&1 &
  RELAY_PID=$!
  trap 'kill $RELAY_PID 2>/dev/null || true' EXIT
  sleep 1
  kill -0 "$RELAY_PID" 2>/dev/null || fail "the relay died on startup (see $RELAY_LOG)"
  echo "local relay $RELAY_URL (pid $RELAY_PID, log $RELAY_LOG)"
fi

# ── the app ──────────────────────────────────────────────────────────────────
step "build"
APP="$UI_DIR/build/Build/Products/Debug-iphonesimulator/WallFlowers.app"
if [ "${SKIP_BUILD:-0}" = "1" ] && [ -d "$APP" ]; then
  echo "reusing $APP (SKIP_BUILD=1)"
else
  xcodebuild -project "$UI_DIR/WallFlowers.xcodeproj" -scheme WallFlowers -sdk iphonesimulator \
    -destination "platform=iOS Simulator,name=$MODEL_A" -derivedDataPath "$UI_DIR/build" \
    build -quiet || fail "xcodebuild"
fi
[ -d "$APP" ] || fail "no WallFlowers.app at $APP"
echo "app $APP"

# ── two simulators, fresh identities ─────────────────────────────────────────
step "simulators"
sim_for() { # <name> <model> → udid (created when absent)
  local udid
  udid=$(xcrun simctl list devices | grep -F "$1 (" | grep -oE '[0-9A-F-]{36}' | head -1 || true)
  [ -n "$udid" ] || udid=$(xcrun simctl create "$1" "$2")
  echo "$udid"
}
SIM_A="${SIM_A:-$(sim_for "E2E-Ticketing-A" "$MODEL_A")}"
SIM_B="${SIM_B:-$(sim_for "E2E-Ticketing-B" "$MODEL_B")}"
echo "A (organizer/door) $SIM_A"
echo "B (buyer/holder)   $SIM_B"
for S in "$SIM_A" "$SIM_B"; do
  xcrun simctl boot "$S" 2>/dev/null || true
  xcrun simctl bootstatus "$S" -b >/dev/null
  xcrun simctl terminate "$S" "$BUNDLE_ID" 2>/dev/null || true
  # An uninstall that silently fails leaves the OLD sealed store in place while the
  # keychain reset destroys its key; every later step then dies on a wedged launch.
  for _ in 1 2 3; do
    xcrun simctl uninstall "$S" "$BUNDLE_ID" 2>/dev/null || true
    xcrun simctl get_app_container "$S" "$BUNDLE_ID" data >/dev/null 2>&1 || break
    sleep 1
  done
  if xcrun simctl get_app_container "$S" "$BUNDLE_ID" data >/dev/null 2>&1; then
    fail "$BUNDLE_ID still installed on $S after 3 uninstalls"
  fi
  xcrun simctl keychain "$S" reset 2>/dev/null || true
  xcrun simctl install "$S" "$APP" || fail "install on $S"
  # Pre-grant what the app asks for (location, photos…) so a permission alert cannot sit
  # over the screen we are about to photograph. Best effort — `notifications` is not a
  # simctl-grantable service, so its prompt may still appear briefly; it never blocks the
  # assertions, which read the fold, not the pixels.
  xcrun simctl privacy "$S" grant all "$BUNDLE_ID" 2>/dev/null || true
done

step "launch"
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="Nye Thompson" \
  xcrun simctl launch "$SIM_A" "$BUNDLE_ID" >/dev/null
SIMCTL_CHILD_PACIFIC_RELAY_URL="$RELAY_URL" SIMCTL_CHILD_PACIFIC_IDENTITY_NAME="Harvey Deane" \
  xcrun simctl launch "$SIM_B" "$BUNDLE_ID" >/dev/null
both_identities() {
  [ -n "$(dev_file "$SIM_A" dev-contact.txt)" ] && [ -n "$(dev_file "$SIM_B" dev-contact.txt)" ]
}
until_ok 60 "both identities" both_identities
A_SPACE=$(dev_file "$SIM_A" dev-contact.txt | head -1)
A_BUNDLE=$(dev_file "$SIM_A" dev-contact.txt | sed -n 2p)
B_SPACE=$(dev_file "$SIM_B" dev-contact.txt | head -1)
[ -n "$A_BUNDLE" ] || fail "A exported no contact bundle"
echo "A=$A_SPACE"
echo "B=$B_SPACE"

# ── 1. pair (the real handshake) ─────────────────────────────────────────────
# Sequence matters: BOTH sides accept before anything else is authored — replying on a
# stale epoch poisons the peer's inbox. The scanner (B) is Connected immediately; the
# scanned side (A) sees the request and accepts it.
step "pair A ↔ B"
open_url "$SIM_B" "pacific://pair-scan?bundle=$(enc "$A_BUNDLE")"; sleep 6
open_url "$SIM_A" "pacific://sync"; sleep 5
open_url "$SIM_A" "pacific://pair-accept?peer=$B_SPACE"; sleep 2
open_url "$SIM_B" "pacific://pair-accept?peer=$A_SPACE"; sleep 2
open_url "$SIM_B" "pacific://sync"; sleep 4
open_url "$SIM_A" "pacific://dump-profiles"; sleep 1
[ -n "$(dev_file "$SIM_A" dev-profiles.txt)" ] || fail "A holds no connection to B after the handshake"
echo "connected"

# ── 2. A lists a FREE ticketed event ─────────────────────────────────────────
step "A lists a free ticketed event"
START_MS=$(python3 -c "import time;print(int((time.time()+7200)*1000))")
open_url "$SIM_A" "pacific://event-new?title=$(enc 'Door Test — Free Entry')&start=$START_MS&price=0&currency=usd&capacity=50"
a_listed() { [ -n "$(dev_file "$SIM_A" dev-event.txt | sed -n 2p)" ]; }
until_ok 30 "A's event mint" a_listed
EVENT_ID=$(dev_file "$SIM_A" dev-event.txt | head -1)
LISTING=$(dev_file "$SIM_A" dev-event.txt | sed -n 2p)
echo "event   $EVENT_ID"
echo "listing $LISTING"

# ── 3. B buys — request-first, exactly as the GET TICKET capsule does ────────
step "B requests a ticket"
open_url "$SIM_B" "pacific://ticket-request?peer=$A_SPACE&listing=$LISTING&qty=1"
sleep 3
a_has_request() {
  open_url "$SIM_A" "pacific://sync"
  open_url "$SIM_A" "pacific://dump-tickets"
  sleep 1
  dev_file "$SIM_A" dev-tickets.txt | grep -qF "$LISTING"
}
until_ok 45 "B's request reaching A" a_has_request
echo "A holds the request: $(dev_file "$SIM_A" dev-tickets.txt | head -1)"

# ── 4. A fulfils on its own sweep; B's wallet fills ──────────────────────────
# Nothing is driven here but syncs: TicketSweep.fulfilFree answers the pending request on
# A's sync tick (recordSale → issue → deliver). That is the product's behaviour, not a
# harness shortcut — the script only waits for it.
step "A fulfils, B's wallet fills"
b_has_ticket() {
  open_url "$SIM_B" "pacific://sync"
  open_url "$SIM_B" "pacific://dump-wallet"
  sleep 1
  dev_file "$SIM_B" dev-wallet.txt | grep -qF "$LISTING"
}
until_ok 120 "B's wallet" b_has_ticket
WALLET_LINE=$(dev_file "$SIM_B" dev-wallet.txt | grep -F "$LISTING" | head -1)
TICKET_ID=$(cut -d'|' -f1 <<<"$WALLET_LINE")
VERIFIED=$(cut -d'|' -f4 <<<"$WALLET_LINE")
QR=$(cut -d'|' -f6- <<<"$WALLET_LINE")
[ "$VERIFIED" = "1" ] || fail "B's credential does not verify against A's identity"
[ -n "$QR" ] || fail "B's wallet row carries no QR payload"
echo "ticket  $TICKET_ID (signature verified)"
open_url "$SIM_A" "pacific://dump-events"; sleep 1
echo "A ledger: $(dev_file "$SIM_A" dev-events.txt | head -1)"

# ── 5. B holds the ticket up — the QR full-screen, as at a real door ────────
# Opened BEFORE the scan on purpose: this is the live path. From here B's screen is never
# driven again — only read (dump-wallet) — so whatever green it ends on, the receipt and
# the sync tick put there, not the harness.
step "B presents the ticket"
open_url "$SIM_B" "pacific://show-tickets?ticket=$TICKET_ID"
sleep 3
mkdir -p "$SHOTS"
xcrun simctl io "$SIM_B" screenshot "$SHOTS/e2e-wallet-B-before.png" >/dev/null 2>&1 || true

# ── 6. A scans B at the door ─────────────────────────────────────────────────
step "A scans B's ticket at the door"
open_url "$SIM_A" "pacific://door-scan?object=$EVENT_ID&qr=$(enc "$QR")"
a_has_verdict() { [ -n "$(dev_file "$SIM_A" dev-door.txt)" ]; }
until_ok 30 "A's door verdict" a_has_verdict
DOOR=$(dev_file "$SIM_A" dev-door.txt | head -1)
VERDICT=$(cut -d'|' -f1 <<<"$DOOR")
RECEIPT=$(cut -d'|' -f4 <<<"$DOOR")
echo "verdict $DOOR"
[ "$VERDICT" = "valid" ] || fail "the door read '$VERDICT', not valid"
[ "$RECEIPT" = "sent" ] || fail "the admit receipt was not sent — B could never learn it was let in"

# ── 7. B goes green: the receipt lands on the holder's channel ───────────────
step "B's ticket shows ADMITTED"
b_admitted() {
  open_url "$SIM_B" "pacific://sync"
  open_url "$SIM_B" "pacific://dump-wallet"
  sleep 1
  local ms; ms=$(dev_file "$SIM_B" dev-wallet.txt | grep -F "$TICKET_ID" | cut -d'|' -f5)
  [ -n "$ms" ] && [ "$ms" -gt 0 ] 2>/dev/null
}
until_ok 120 "B's admit receipt" b_admitted
ADMITTED_MS=$(dev_file "$SIM_B" dev-wallet.txt | grep -F "$TICKET_ID" | cut -d'|' -f5)
echo "admittedMs=$ADMITTED_MS"

# ── 8. BOTH GREENS, on screen, at the same moment ───────────────────────────
# A's door cover still stands on its VALID verdict (nothing tapped it away). B's cover
# was opened BEFORE the scan and has not been touched since: it turned green on its own,
# on the sync tick that folded the receipt.
step "screenshots"
sleep 3
xcrun simctl io "$SIM_A" screenshot "$SHOTS/e2e-door-A.png"   >/dev/null 2>&1 || fail "screenshot A"
xcrun simctl io "$SIM_B" screenshot "$SHOTS/e2e-wallet-B.png" >/dev/null 2>&1 || fail "screenshot B"

# ── 9. the redemption really is in A's event fold ───────────────────────────
# Proved the only honest way: present the same credential again. A ticket that is in the
# ledger AND in the redemptions can never read valid twice. This turns A's screen RED, so
# it runs AFTER the green shots and keeps its own screenshot as the refusal's evidence.
step "A's fold holds the redemption"
open_url "$SIM_A" "pacific://door-scan?object=$EVENT_ID&qr=$(enc "$QR")"
a_refuses() {
  case "$(dev_file "$SIM_A" dev-door.txt | head -1 | cut -d'|' -f1)" in
    redeemed|double) return 0;; *) return 1;;
  esac
}
until_ok 30 "A's re-scan verdict" a_refuses
sleep 2
xcrun simctl io "$SIM_A" screenshot "$SHOTS/e2e-door-A-rescan.png" >/dev/null 2>&1 || true
echo "re-scan $(dev_file "$SIM_A" dev-door.txt | head -1)  (the redemption converged)"

echo
echo "✓ BOTH GREEN"
echo "  A (door)   verdict=valid, receipt sent, redemption in the event fold"
echo "  B (holder) admittedMs=$ADMITTED_MS on ticket $TICKET_ID — the cover flipped LIVE,"
echo "             opened before the scan and never touched again"
echo "  $SHOTS/e2e-wallet-B-before.png  (B's wallet BEFORE the scan — no ADMITTED anywhere)"
echo "  $SHOTS/e2e-door-A.png"
echo "  $SHOTS/e2e-wallet-B.png"
echo "  $SHOTS/e2e-door-A-rescan.png    (the same ticket, honestly refused)"
