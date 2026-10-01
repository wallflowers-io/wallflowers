#!/usr/bin/env bash
# founder-r3.sh — R3's one founder paste, with the Arc's fresh bundle inlined: founder-r3.js with its
# BUNDLE_SET line filled, written to OUT (default: stdout). Run it when the Arc and the Door are live,
# and hand the file over to paste; nothing in it is to be filled in.
#
#   app/door/hosting/one-signin/founder-r3.sh founder-r3.paste.js
#
# A bundle is a key package used once: one file per run, and a re-run regenerates. ARC_BUNDLE_URL
# names another source (the test's stand-in). A bundle that fails or comes back empty stops it.
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
URL=${ARC_BUNDLE_URL:-https://arc.wallflowers.io/v1/bundle}
OUT=${1:-/dev/stdout}
b=$(curl -fsS --max-time 20 "$URL") || { echo "founder-r3.sh: $URL did not answer 200" >&2; exit 1; }
[ -n "$(printf '%s' "$b" | tr -d '[:space:]')" ] || { echo "founder-r3.sh: the bundle is empty" >&2; exit 1; }
# A JSON string is a JavaScript string literal: node quotes whatever the Arc sent.
printf '%s' "$b" | node -e '
  const fs = require("fs");
  const b = fs.readFileSync(0, "utf8").trim();
  const src = fs.readFileSync(process.argv[1], "utf8");
  const line = "const BUNDLE_SET = '"'"'<the Arc>'"'"';";
  if (src.split(line).length !== 2) { console.error("founder-r3.sh: founder-r3.js has no single BUNDLE_SET line"); process.exit(1); }
  fs.writeFileSync(process.argv[2], src.replace(line, "const BUNDLE_SET = " + JSON.stringify(b) + ";"));' "$HERE/founder-r3.js" "$OUT"
