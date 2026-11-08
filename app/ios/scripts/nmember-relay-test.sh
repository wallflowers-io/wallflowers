#!/usr/bin/env bash
# Multi-node N-member GroupObject integration test — REAL process boundaries.
#
# The in-process pacific-core/tests/m1_nmember.rs proves the MLS mechanics but never
# touches the Directory, sync_once, or the relay. THIS is the missing coverage
# (the PR's "top testing follow-up"): three separate `pacific` processes (each its
# own PACIFIC_STATE_DIR, i.e. its own identity + SQLite) drive a real owner-add and
# posts through a real `semaphore` relay on a loopback socket, and we assert the
# three replicas fold to a byte-identical transcript.
#
# Exit 0 iff the N-member convergence invariant holds. Scenario 2 characterises
# the newcomer-backfill gap (informational until backfill lands, then flip it to
# an assertion).
#
#   ./scripts/nmember-relay-test.sh          # build if needed, run, report
set -uo pipefail
cd "$(dirname "$0")/.."                       # repo root
PACIFIC="$PWD/target/release/pacific"
SEM="$PWD/../Arc/target/release/semaphore"
PORT="${PORT:-8897}"
RELAY="ws://127.0.0.1:$PORT"
D="$(mktemp -d)"
FAIL=0

cleanup(){ pkill -x semaphore 2>/dev/null || true; rm -rf "$D"; }
trap cleanup EXIT INT TERM

echo "▸ building pacific + semaphore…"
cargo build --release -p pacific-cli >/dev/null 2>&1 && (cd ../Arc && cargo build --release -p relay) >/dev/null 2>&1 || { echo "build failed"; exit 2; }

echo "▸ starting semaphore on $RELAY …"
pkill -x semaphore 2>/dev/null || true
RELAY_BIND="0.0.0.0:$PORT" nohup "$SEM" >"$D/relay.log" 2>&1 &
sleep 1

# Three users, each an isolated state dir (identity + pacific.db).
A="$D/alice"; B="$D/bob"; C="$D/carol"; mkdir -p "$A" "$B" "$C"
na(){ PACIFIC_STATE_DIR="$A" "$PACIFIC" "$@"; }
nb(){ PACIFIC_STATE_DIR="$B" "$PACIFIC" "$@"; }
nk(){ PACIFIC_STATE_DIR="$C" "$PACIFIC" "$@"; }
for u in na nb nk; do $u id init >/dev/null 2>&1; $u relay set "$RELAY" >/dev/null 2>&1; done
sync_all(){ for r in 1 2 3; do na sync >/dev/null 2>&1; nb sync >/dev/null 2>&1; nk sync >/dev/null 2>&1; done; }
gid_of(){ na --json object new forum | sed 's/.*"object_id":"//;s/".*//'; }

# ── Scenario 1: add-then-post — must converge byte-identically ────────────────
echo; echo "══ Scenario 1: 3-member convergence (add → post → sync) ══"
BOB_B="$(nb pair --show 2>/dev/null | head -1)"
CAR_B="$(nk pair --show 2>/dev/null | head -1)"
G1="$(gid_of)"
na group add "$G1" "$BOB_B" >/dev/null 2>&1; nb sync >/dev/null 2>&1
na group add "$G1" "$CAR_B" >/dev/null 2>&1; nk sync >/dev/null 2>&1; nb sync >/dev/null 2>&1
na object post "$G1" --text "from Alice" >/dev/null 2>&1
sync_all
nb object post "$G1" --text "from Bob" >/dev/null 2>&1
nk object post "$G1" --text "from Carol" >/dev/null 2>&1
sync_all
na object show "$G1" >"$D/a1"; nb object show "$G1" >"$D/b1"; nk object show "$G1" >"$D/c1"
if diff -q "$D/a1" "$D/b1" >/dev/null && diff -q "$D/a1" "$D/c1" >/dev/null && [ "$(grep -c . "$D/a1")" -eq 3 ]; then
  echo "  ✅ PASS — all 3 replicas byte-identical, 3 posts each"
else
  echo "  ❌ FAIL — replicas diverged:"; diff "$D/a1" "$D/b1"; diff "$D/a1" "$D/c1"; FAIL=1
fi

# ── Scenario 2: late joiner — characterise the backfill gap ───────────────────
echo; echo "══ Scenario 2: newcomer backfill (post BEFORE Carol joins) ══"
BOB_B="$(nb pair --show 2>/dev/null | head -1)"
CAR_B="$(nk pair --show 2>/dev/null | head -1)"
G2="$(gid_of)"
na group add "$G2" "$BOB_B" >/dev/null 2>&1; nb sync >/dev/null 2>&1
na object post "$G2" --text "EARLY (pre-Carol)" >/dev/null 2>&1
na sync >/dev/null 2>&1; nb sync >/dev/null 2>&1
na group add "$G2" "$CAR_B" >/dev/null 2>&1; sync_all
na object post "$G2" --text "LATE (post-Carol)" >/dev/null 2>&1; sync_all
if nk object show "$G2" 2>/dev/null | grep -q "EARLY"; then
  echo "  ✅ backfill WORKS — late joiner sees pre-join history"
else
  echo "  ⚠️  KNOWN GAP — late joiner is missing pre-join history (backfill unbuilt)"
fi

echo
if [ "$FAIL" -eq 0 ]; then echo "RESULT: PASS (N-member convergence invariant holds)"; else echo "RESULT: FAIL"; fi
exit "$FAIL"
