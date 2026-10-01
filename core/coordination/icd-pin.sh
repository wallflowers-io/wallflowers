#!/usr/bin/env bash
# icd-pin.sh — O-68: the ICD pinned (core/docs/launch/mdr/icd-pin.md). The one helper
# `make check-icd` and both deploys call.
#
#   icd-pin.sh check          the ICD's bytes hash to its pin; what a release declared kept;
#                             icd.rs conformance; the web's
#                             op bindings current (PIN-1, PIN-2, PIN-7): make check-icd
#   icd-pin.sh deploy         the hash, the op bindings, and the tree's pin the newest icd/*
#                             tag's, or "unreleased" before the first (PIN-8): deploy-door.sh,
#                             deploy-arc.sh. Not icd.rs while NC-8 keeps it red; PIN-10 adds it
#                             when NC-8 closes (mdr/icd-pin.md § Departures)
#   icd-pin.sh served <url|-> sha256 of a Door's GET /v2/icd, fetched or on stdin, is the
#                             pin of record (PIN-5): deploy-door.sh verify
#
# The pin is delta-graph.icd.sha256 beside the model, one line in shasum's format. A file
# in the tree it guards could be rewritten to turn its own check green, so the pin of
# record is the newest icd/* tag's, by version (icd/10.0.0 > icd/2.1.0: the harness takes
# it the same way, PIN-9). Exit 1, naming both hashes, on any mismatch.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE="$(cd "$HERE/.." && pwd)"
ICD=$HERE/delta-graph.icd.json
PIN=$HERE/delta-graph.icd.sha256
# The pin's path at a tag, from the repository's root.
AT="$(git -C "$HERE" rev-parse --show-prefix)delta-graph.icd.sha256"

die() { printf '✗ icd: %s\n' "$*" >&2; exit 1; }
say() { printf 'icd: %s\n' "$*"; }
line() { sed -n '1{/^[0-9a-f]\{64\}  delta-graph\.icd\.json$/s/ .*//p;}'; }   # the hex of a pin's one line

tree_pin() {
  [ -f "$PIN" ] || die "no pin: $PIN"
  local p
  p=$(line < "$PIN")
  [ -n "$p" ] && [ "$(wc -l < "$PIN" | tr -d ' ')" = 1 ] || die "$PIN is not one line '<sha256>  delta-graph.icd.json'"
  echo "$p"
}

# "<hex> <icd/tag>" at the newest tag, or "<hex> unreleased" from the tree before the first.
of_record() {
  local tag p
  tag=$(git -C "$HERE" for-each-ref --sort=-v:refname --format='%(refname:short)' refs/tags/icd/ | head -1)
  if [ -z "$tag" ]; then
    p=$(tree_pin) || exit 1
    echo "$p unreleased"
    return
  fi
  p=$(git -C "$HERE" show "$tag:$AT" 2>/dev/null | line)
  [ -n "$p" ] || die "$tag holds no pin at $AT"
  echo "$p $tag"
}

# The ICD's bytes against the tree's pin, then the web's bindings against the ICD.
bytes() {
  local want got
  want=$(tree_pin) || exit 1
  got=$(shasum -a 256 < "$ICD" | cut -d' ' -f1)
  [ "$got" = "$want" ] || die "delta-graph.icd.json is $got; the pin is $want"
  python3 "$HERE/gen-web-ops.py" --check >/dev/null || die "the web's op bindings are not the ICD's (gen-web-ops.py --check)"
  say "$got is the pin; wallflowers-ops.js current"
}

# What a release declared, kept: every kind, op, id, authority, fold and arg of the ICD at the
# newest icd/* tag (the mint's commit before the first) still in the tree's, only added to
# (PIN-7's first half, icd-released.py).
kept() {
  python3 "$HERE/icd-released.py" || die "the tree's ICD breaks what a release declared (icd-released.py)"
}

# The open NC in status.md whose row names test $1, if one does.
STATUS=$CORE/docs/launch/status.md
nc_for() { grep -E '^\| NC-[0-9]+ \| Open \|' "$STATUS" 2>/dev/null | grep -F "\`$1\`" | sed -E 's/^\| (NC-[0-9]+) .*/\1/' | head -1; }

# icd.rs, each failing test named with the open NC that names it. A red no open NC names is
# not the known one, and says so (Software Security): nothing is waived, and a second red
# cannot pass as the first.
conformance() {
  local out n failed t nc
  if out=$(cd "$CORE" && cargo test -q -p pacific-core --lib icd:: 2>&1); then
    # A filter that matched nothing would pass: at least one of icd.rs's tests must run.
    n=$(printf '%s\n' "$out" | sed -n 's/^test result: ok\. \([0-9]*\) passed.*/\1/p' | head -1)
    [ "${n:-0}" -gt 0 ] || { printf '%s\n' "$out" >&2; die "icd.rs: no conformance test ran"; }
    say "icd.rs $n passed"
    return
  fi
  failed=$(printf '%s\n' "$out" | sed -n '/^failures:$/,/^test result/p' | sed -n 's/^    \([A-Za-z0-9_:]*\)$/\1/p' | sort -u)
  [ -n "$failed" ] || { printf '%s\n' "$out" >&2; die "icd.rs did not run"; }
  for t in $failed; do
    nc=$(nc_for "${t##*::}")
    if [ -n "$nc" ]; then
      printf '✗ icd: icd.rs %s failed: %s, open\n' "${t##*::}" "$nc" >&2
    else
      printf '%s\n' "$out" | awk -v h="---- $t stdout ----" '$0 == h {p=1; print; next} p && /^(---- |failures:$)/ {p=0} p' >&2
      printf '✗ icd: NEW RED: icd.rs %s failed, and no open NC in status.md names it\n' "${t##*::}" >&2
    fi
  done
  die "icd.rs: the code and the ICD disagree"
}

released() {
  local want rec
  want=$(tree_pin) || exit 1
  rec=$(of_record) || exit 1
  case "$rec" in
    *" unreleased") say "unreleased: no icd/* tag; the tree's pin $want" ;;
    "$want "*) say "$want is ${rec#* }'s" ;;
    *) die "the tree's pin is $want; ${rec#* }'s is ${rec%% *}" ;;
  esac
}

served() {
  local src=${1:?served <url|->} rec got
  rec=$(of_record) || exit 1
  BODY=$(mktemp "${TMPDIR:-/tmp}/icd-served.XXXXXX") || exit 1
  trap 'rm -f "$BODY"' EXIT
  if [ "$src" = - ]; then cat > "$BODY"; else curl -fsS --max-time 20 "$src" -o "$BODY" || die "GET $src failed"; fi
  [ -s "$BODY" ] || die "nothing served at /v2/icd"
  got=$(shasum -a 256 < "$BODY" | cut -d' ' -f1)
  [ "$got" = "${rec%% *}" ] || die "/v2/icd is $got; the pin (${rec#* }) is ${rec%% *}"
  say "/v2/icd is ${rec%% *}, the pin (${rec#* })"
}

case "${1:-}" in
  check) bytes && kept && conformance ;;
  deploy) bytes && released ;;
  served) served "${2:-}" ;;
  *) die "check | deploy | served <url|->" ;;
esac
