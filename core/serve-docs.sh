#!/usr/bin/env bash
#
# serve-docs.sh — serve docs/ with a core that is actually there and actually current.
#
#   ./serve-docs.sh          build the core if stale, check it, serve, print the URL
#   ./serve-docs.sh build    build and check only — no server
#   ./serve-docs.sh stop     stop the server this script started
#   PORT=8131 ./serve-docs.sh
#   FORCE=1 ./serve-docs.sh  rebuild even if nothing changed
#
# WHY THIS EXISTS. docs/wallflowers-protocol.html does every derivation, seal and
# open through pacific-core compiled to wasm. Opening the file with file:// gives
# a blank page, and hand-starting `python3 -m http.server` in docs/ gives a page
# whose core may be months old, because docs/core/ is build output that nothing
# was rebuilding. Three failures, all silent, all fixed here:
#
#   * NO CORE AT ALL. docs/core/ is gitignored build output. A fresh clone has no
#     module, the glue 404s, and the page reports a boot error nobody reads as
#     "you never built it".
#
#   * A STALE CORE. Add an export in core-wasm, forget the wasm step, and the page
#     calls a function that is not there. The staleness check below compares the
#     module against every .rs in the workspace and the ICD.
#
#   * A CACHED CORE. A 3.6 MB module the browser already has is the one it keeps.
#     Rebuild, reload, see the old behaviour, disbelieve the rebuild. The server
#     here sends no-store, so a rebuild is one reload away.
#
# ONE BUILDER, NOT TWO. The build is app/web/build-wasm.sh with OUT_DIR pointed
# here. That script owns the two-stage wasm-bindgen dance, the export assertions
# and the getrandom-backend guard; a second copy of any of that in this file
# would be a second source of truth that rots the first time mls-rs moves. This
# script adds only the check that script cannot make — that the exports THIS
# PAGE reaches for exist — because it does not know which pages exist.
#
# app/web is a separate repo and gets its own module under its own tree. Nothing
# here writes to it. When docs/ moves into app/web this script collapses into
# app/web/run.sh and the OUT_DIR override goes away.
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$PWD"
PORT="${PORT:-8130}"
CORE_SRC="$ROOT"                      # this script lives in core/
WASM="$ROOT/docs/core/core_wasm_bg.wasm"
PIDFILE="$ROOT/.serve-docs.pid"
export PATH="$HOME/.cargo/bin:$PATH"

say() { printf '%s\n' "$*"; }

stop() {
  if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
    kill "$(cat "$PIDFILE")" && say "stopped $(cat "$PIDFILE")"
  else
    say "nothing to stop"
  fi
  rm -f "$PIDFILE"
}

# ── is the module older than the code it was built from? ────────────────────
stale() {
  [ -f "$WASM" ] || { say "  no module at docs/core/ — building"; return 0; }
  [ "${FORCE:-}" = "1" ] && { say "  FORCE=1 — building"; return 0; }
  local newer
  newer="$(find "$CORE_SRC" -name target -prune -o -name vendor -prune -o \
             \( -name '*.rs' -o -name '*.toml' -o -name 'delta-graph.icd.json' \) \
             -newer "$WASM" -print 2>/dev/null | head -3)"
  if [ -n "$newer" ]; then
    say "  core changed since the module was built:"
    printf '    %s\n' $(printf '%s\n' "$newer" | sed "s|$ROOT/||")
    return 0
  fi
  return 1
}

build() {
  if stale; then
    ( cd "$ROOT/../app/web" && OUT_DIR="$ROOT/docs" ./build-wasm.sh )
  else
    say "  module is current — $(stat -f%z "$WASM") bytes, built $(stat -f%Sm -t '%d %b %H:%M' "$WASM")"
  fi
  check_surface
}

# ── does the module export what the pages actually call? ────────────────────
#
# Derived from both ends: the call names come out of the HTML, the export names
# come out of the .wasm. Neither is written down here, so neither can rot. This
# is the check that turns "TypeError: W.foo is not a function", thrown three
# clicks deep in a handler, into a line before the server even starts.
check_surface() {
  node - "$ROOT" <<'NODE'
const fs = require("fs"), path = require("path");
const root = process.argv[2], docs = path.join(root, "docs");
const wasm = path.join(docs, "core/core_wasm_bg.wasm");
const mod = new WebAssembly.Module(fs.readFileSync(wasm));
const exports = new Set(WebAssembly.Module.exports(mod).map(e => e.name));
let pages = 0, calls = 0, bad = [];
for (const f of fs.readdirSync(docs).filter(f => f.endsWith(".html"))) {
  const src = fs.readFileSync(path.join(docs, f), "utf8");
  // Either door: the bindgen glue, or a page that instantiates the module by
  // hand against the alloc/out_ptr/out_len/erred ABI (wallflowers-protocol does).
  if (!src.includes("core/core_wasm")) continue;
  pages++;
  const seen = new Set();
  for (const m of src.matchAll(/\b(?:raw|S|J)\('([a-z][a-z0-9_]*)'/g)) seen.add(m[1]);
  calls += seen.size;
  for (const name of seen) if (!exports.has(name)) bad.push(`${f} calls ${name}`);
}
if (bad.length) {
  console.error("  MISSING FROM THE MODULE:");
  for (const b of bad) console.error("    " + b);
  console.error("  Either the export was never added to core-wasm, or the page has a typo.");
  process.exit(1);
}
console.log(`  surface: ${pages} core page(s), ${calls} distinct calls, all exported`);
NODE
}

# ── a server that tells the truth about .wasm and never caches it ───────────
#
# Not `python3 -m http.server`. Two headers matter and it gets both wrong:
# instantiateStreaming refuses anything that is not application/wasm and the glue
# then falls back without saying so, and a cached 3.6 MB module survives every
# rebuild you make.
read -r -d "" SERVER_PY <<'PYSRC' || true
import sys, http.server, socketserver, functools

class H(http.server.SimpleHTTPRequestHandler):
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map,
                      ".wasm": "application/wasm", ".mjs": "text/javascript"}
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()
    def log_message(self, *a): pass

d, port = sys.argv[1], int(sys.argv[2])
socketserver.TCPServer.allow_reuse_address = True
with socketserver.TCPServer(("127.0.0.1", port),
                            functools.partial(H, directory=d)) as s:
    s.serve_forever()
PYSRC

serve() {
  if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
    say "port $PORT is already listening — ./serve-docs.sh stop, or set PORT=" 
    exit 1
  fi
  python3 -c "$SERVER_PY" "$ROOT/docs" "$PORT" >/dev/null 2>&1 &
  echo $! > "$PIDFILE"
  sleep 1
  if ! kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
    rm -f "$PIDFILE"; say "server failed to start"; exit 1
  fi
  say ""
  say "  http://localhost:$PORT/wallflowers-protocol.html"
  say "  http://localhost:$PORT/            everything else in docs/"
  say ""
  say "  ./serve-docs.sh stop    when you are done"
}

case "${1:-serve}" in
  stop)  stop ;;
  build) build ;;
  serve) build; serve ;;
  *)     say "usage: ./serve-docs.sh [serve|build|stop]"; exit 1 ;;
esac
