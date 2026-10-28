#!/usr/bin/env bash
# cut-tiles.sh — the map /map draws: one area of the Protomaps basemap, cut out of
# a dated public build into play/assets/map/tiles/.
#
#     scripts/cut-tiles.sh
#
# Needs `pmtiles` (github.com/protomaps/go-pmtiles) on PATH. It reads the build
# by range request: megabytes come down, not the planet's 138 GB.
#
# WHICH BUILD AND WHICH AREA are assets/map/VENDOR.json's, not this file's:
# change them there, cut, and put the new size and sha256 back (the cut prints
# them). The archive is not committed (.gitignore); tests/test_vendor.js checks
# one that is present against the lock.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
map="$here/../assets/map"
read -r from path bbox maxzoom < <(python3 - "$map/VENDOR.json" <<'PY'
import json, sys
t = json.load(open(sys.argv[1]))["tiles"]
print(t["from"], t["path"], ",".join(str(v) for v in t["bbox"]), t["maxzoom"])
PY
)
mkdir -p "$(dirname "$map/$path")"
pmtiles extract "$from" "$map/$path" --bbox="$bbox" --maxzoom="$maxzoom"
printf 'bytes  %s\nsha256 %s\n' "$(wc -c < "$map/$path" | tr -d ' ')" "$(shasum -a 256 "$map/$path" | cut -d' ' -f1)"
