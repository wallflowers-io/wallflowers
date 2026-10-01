#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# arc-node-bootstrap.sh — prepare ONE Arc ON a machine0 VM.
#
# One machine0 VM = one self-contained Arc. This script starts the Semaphore MLS relay and
# the Arc's membership/signup server (arc-node), each when its binary is given. It does NOT
# dial any hub — there is no hub.
#
# It requires what it needs and exits non-zero if a piece is missing — no stubs.
#
# Env:
#   ARC_RELAY_BIN  (optional) path on the VM to a compiled `relay` binary (Semaphore)
#   ARC_NODE_BIN   (optional) path on the VM to a compiled `arc-node` (membership/signup)
#   ARC_NAME       (optional) this Arc's display name (default "Arc")
#   ARC_PUBLIC_URL (optional) public wss URL of this Arc's relay — stamped into every tether
#   ARC_SIGNUP_PORT(optional) port arc-node serves signup on (default 8790)
#   ARC_RELAY_PORT (optional) port the Semaphore relay listens on (default 8787)
set -euo pipefail

# 1. Semaphore MLS relay (people↔people; the `relay` crate) — members reach it to sync ---
if [ -n "${ARC_RELAY_BIN:-}" ]; then
  if [ ! -x "$ARC_RELAY_BIN" ]; then
    echo "error: ARC_RELAY_BIN=$ARC_RELAY_BIN is not an executable on this VM" >&2; exit 1
  fi
  ARC_RELAY_PORT="${ARC_RELAY_PORT:-8787}"
  export RELAY_BIND="0.0.0.0:$ARC_RELAY_PORT"   # listen on all interfaces so members can reach it
  if command -v ufw >/dev/null 2>&1; then
    echo "» opening the relay port ($ARC_RELAY_PORT/tcp)"
    sudo ufw allow "$ARC_RELAY_PORT/tcp" || echo "warn: could not open relay port via ufw" >&2
  fi
  echo "» starting Semaphore MLS relay ($ARC_RELAY_BIN) on :$ARC_RELAY_PORT"
  nohup "$ARC_RELAY_BIN" >"$HOME/arc-relay.log" 2>&1 &
  echo "✓ relay pid $!  (log: ~/arc-relay.log)"
fi

# 2. Arc membership / signup server (arc-node) --------------------------------
# THE signup surface, and the Pacific signup itself: a user POSTs their contact bundle to
# this Arc, and arc-node owner-adds them into a 2-member MLS tether (pacific-core `pair_scan`),
# sealing the Welcome to the relay. On the user's next sync they fold the tether and are a
# member. Signing up to an Arc IS the Pacific signup — and because the protocol is open,
# anyone running this stands up an Arc. Needs the Semaphore relay above (the Welcome is
# published there). Set ARC_NODE_BIN to a compiled `arc-node` pushed onto this VM.
if [ -n "${ARC_NODE_BIN:-}" ]; then
  if [ ! -x "$ARC_NODE_BIN" ]; then
    echo "error: ARC_NODE_BIN=$ARC_NODE_BIN is not an executable on this VM" >&2; exit 1
  fi
  if [ -z "${ARC_RELAY_BIN:-}" ]; then
    echo "error: arc-node needs the Semaphore relay — set ARC_RELAY_BIN too (it seals the Welcome there)" >&2; exit 1
  fi
  ARC_SIGNUP_PORT="${ARC_SIGNUP_PORT:-8790}"
  export PACIFIC_STATE_DIR="${PACIFIC_STATE_DIR:-$HOME/.arc-node}"   # the Arc's sovereign identity store
  export ARC_NAME="${ARC_NAME:-Arc}"
  export ARC_RELAY_URL="${ARC_RELAY_URL:-ws://127.0.0.1:8787}"       # local relay the Welcome is sealed to
  export ARC_SIGNUP_BIND="0.0.0.0:$ARC_SIGNUP_PORT"
  # ARC_PUBLIC_URL (the wss the user reaches this Arc's relay on) is stamped into each tether
  # so the joining device routes back here; pass it through if the operator set it.
  [ -n "${ARC_PUBLIC_URL:-}" ] && export ARC_PUBLIC_URL
  mkdir -p "$PACIFIC_STATE_DIR"
  if command -v ufw >/dev/null 2>&1; then
    echo "» opening the signup port ($ARC_SIGNUP_PORT/tcp)"
    sudo ufw allow "$ARC_SIGNUP_PORT/tcp" || echo "warn: could not open signup port via ufw" >&2
  fi
  echo "» starting arc-node membership/signup (users POST /signup on :$ARC_SIGNUP_PORT)"
  nohup "$ARC_NODE_BIN" >"$HOME/arc-node.log" 2>&1 &
  echo "✓ arc-node pid $!  (log: ~/arc-node.log)"
fi

echo "✓ Arc ready"
