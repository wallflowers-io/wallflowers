#!/usr/bin/env bash
#
# The Door, with its own relay. Paths derive from this script's location.
#
#   ./run.sh              http://localhost:8233, relay on 8788, auth at DOOR_AUTH
#
# Sign-in needs the auth service (DOOR_AUTH, default http://127.0.0.1:8021) and a
# passkey, and WebAuthn will not run on an IP address: open it as localhost.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
relay="$(cd "$here/../../arc" && pwd)/target/release/semaphore"

[ -x "$relay" ] || {
  echo "no relay binary at $relay — build it with:" >&2
  echo "  (cd $(cd "$here/../../arc" && pwd) && cargo build --release -p relay)" >&2
  exit 1
}

export DOOR_PORT="${DOOR_PORT:-8233}"
export DOOR_PUBLIC="${DOOR_PUBLIC:-http://localhost:$DOOR_PORT}"
export DOOR_ORIGIN="${DOOR_ORIGIN:-http://127.0.0.1:8232,http://127.0.0.1:8231}"
export DOOR_ROOT="${DOOR_ROOT:-$here/.door-state}"
mkdir -p "$DOOR_ROOT"

# A SECOND DOOR NODE — a second device of everyone who reaches it — joins the
# first's relay instead of starting one:
#   DOOR_PORT=8234 DOOR_ROOT=/tmp/door-b DOOR_RELAY=ws://127.0.0.1:8788/v1/relay ./run.sh
# Otherwise this image owns the relay and gives it a DURABLE store: the chain, the
# lease cells and the commits a pool leaf replays must outlive a restart.
if [ -z "${DOOR_RELAY:-}" ]; then
  export DOOR_RELAY_BIN="$relay"
  export DOOR_RELAY_PORT="${DOOR_RELAY_PORT:-8788}"
  export RELAY_STORE="${RELAY_STORE:-$DOOR_ROOT/relay.db}"
fi

exec "$here/target/release/door"
