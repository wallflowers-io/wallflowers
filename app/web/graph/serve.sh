#!/usr/bin/env bash
#
# Serve the graph explorer. Paths derive from this script's own location.
#
#   ./serve.sh            http://127.0.0.1:8232/graph/
#
# THE DOCUMENT ROOT IS app/web, NOT graph/. The page reads three files it does
# not own — the account module (the core loader), the ICD-generated op bindings,
# and the staged wasm — and reaching them by relative path is what keeps this
# surface from becoming a fourth staging target in account/stage.sh. The cost is
# that the root is one level up; the benefit is that nothing here can hold a
# stale copy of somebody else's file.
#
# BIND EXPLICITLY, AND CHECK. Two servers can hold one port on macOS — an IPv4
# listener and an IPv6 wildcard one bind happily side by side, and which answers
# is whoever got there first. So: a distinctive port, --bind 127.0.0.1, and a
# refusal if the port is taken rather than a race (the same discipline as
# mobile/serve.sh, which learned it the expensive way).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
web="$(cd "$here/.." && pwd)"
port="${PORT:-8232}"

[ -f "$web/docs/core/core_wasm_bg.wasm" ] || {
  echo "core not staged at app/web/docs/core — run app/web/build-wasm.sh" >&2; exit 1; }
[ -f "$web/shared/wallflowers-ops.js" ] || {
  echo "app/web/shared/wallflowers-ops.js is missing — regenerate it with" >&2
  echo "  core/coordination/gen-web-ops.py" >&2; exit 1; }

if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
  echo "port $port is already held:" >&2
  lsof -nP -iTCP:"$port" -sTCP:LISTEN >&2
  echo "pick another with PORT=<n> $0" >&2
  exit 1
fi

# NO-CACHE, AND NOT OPTIONAL. `python3 -m http.server` sends Last-Modified and
# nothing else, so a browser caches the wasm heuristically and the page runs
# yesterday's module against today's core — which reads as "fn is not a
# function", not as a stale file. `docs-serve.py` is the same server with
# `Cache-Control: no-cache`, and it already exists; it takes the root.
echo "http://127.0.0.1:$port/graph/"
exec python3 "$web/docs-serve.py" "$port" "$web"
