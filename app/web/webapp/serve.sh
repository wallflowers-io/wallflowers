#!/usr/bin/env bash
#
# Serve the webapp. Paths derive from this script's own location.
#
#   ./serve.sh            http://127.0.0.1:8231 — reads app/door/run.sh on 8233
#
# BIND EXPLICITLY, AND CHECK. Two servers can hold one port on macOS — an
# IPv4 listener and an IPv6 wildcard one bind happily side by side, and which
# answers is whoever got there first. That cost a debugging round when another
# process on this machine was already on the port and served ITS index.html to
# a screenshot run that then reported this page broken. So: a distinctive port,
# --bind 127.0.0.1, and a refusal if the port is taken rather than a race.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
port="${PORT:-8231}"

if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
  echo "port $port is already held:" >&2
  lsof -nP -iTCP:"$port" -sTCP:LISTEN >&2
  echo "pick another with PORT=<n> $0" >&2
  exit 1
fi

echo "http://127.0.0.1:$port/"
exec python3 "$here/serve.py" "$port"
