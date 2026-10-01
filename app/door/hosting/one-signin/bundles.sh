#!/usr/bin/env bash
# bundles.sh — the Arc's fresh bundles for one-signin-2.1.0.js, printed as its one BUNDLES line:
# the Site's, the Host's, then Resources; Skills, Time & Services; Financial Support.
#
#   app/door/hosting/one-signin/bundles.sh        five, from https://arc.wallflowers.io/v1/bundle
#
# Each bundle is a key package used once: run this just before the snippet, and paste its line
# over the snippet's BUNDLES line. ARC_BUNDLE_URL names another source (the test's stand-in).
# A bundle that fails, comes back empty or repeats an earlier one stops it, naming which.
set -euo pipefail
URL=${ARC_BUNDLE_URL:-https://arc.wallflowers.io/v1/bundle}
N=${1:-5}
got=()
for i in $(seq 1 "$N"); do
  b=$(curl -fsS --max-time 20 "$URL") || { echo "bundles.sh: bundle $i of $N: $URL did not answer 200" >&2; exit 1; }
  [ -n "$b" ] || { echo "bundles.sh: bundle $i of $N is empty" >&2; exit 1; }
  for seen in "${got[@]+"${got[@]}"}"; do
    [ "$seen" != "$b" ] || { echo "bundles.sh: bundle $i of $N repeats an earlier one" >&2; exit 1; }
  done
  got+=("$b")
done
# JSON strings are JavaScript string literals: node quotes whatever the Arc sent.
printf '%s\0' "${got[@]}" | node -e '
  const a = require("fs").readFileSync(0, "utf8").split("\0").slice(0, -1);
  process.stdout.write("const BUNDLES = " + JSON.stringify(a) + ";\n");'
