#!/usr/bin/env bash
# pinned.sh — run a suite and VOID the result if the tree moved underneath it.
#
# WHY. A green suite proves the tree was self-consistent at some instant. It does
# NOT prove the tree still holds the work being tested: on 24 Sep a backgrounded
# script wrote an hours-old buffer of node.rs back to disk mid-session, and three
# separate runs went green across the loss because the code and the tests moved
# together. "If the code can revert, so can the tests, so green means nothing."
#
# So a run now carries a pin: the digest of every source file before and after.
# If they differ the run is VOID — not failed, void, because its result describes
# a tree that no longer exists. The files that moved are named.
#
#   tools/pinned.sh <label> <command...>
#   tools/pinned.sh --self-test          the summary's own checks (NC-122)
#
# Exit: the command's own status, or 2 if the tree moved.

set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
product="$(cd "$here/.." && pwd)"

# THE SUMMARY SAYS WHAT IT COUNTS, AND WHAT IT CANNOT (NC-122). A test binary killed by a
# signal (a stack overflow's SIGABRT) prints no "test result" line, so a count of those lines
# read "N passed, M failed" with the dead binary missing (28 Sep: o69_fc1_on_off, found by
# build-worker-a). Cargo names it on "process didn't exit successfully"; so does this.
if [ "${1:-}" = "--self-test" ]; then
  fails=0
  check() { # <what> <expected substring> <script>
    local got; got="$("${BASH_SOURCE[0]}" pinned-self-test bash -c "$3" 2>/dev/null | head -1)"
    if [[ "$got" == *"$2"* ]]; then echo "ok   $1"; else echo "FAIL $1: got '$got', want '$2'"; fails=$((fails+1)); fi
  }
  green='echo "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"'
  red='echo "test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out"'
  died='printf "%s\n" "error: test failed, to rerun pass \`-p pacific-core --test o69_fc1_on_off\`" "Caused by:" "  process didn'"'"'t exit successfully: \`/t/target/debug/deps/o69_fc1_on_off-0123456789abcdef --no-capture\` (signal: 6, SIGABRT: process abort signal)"'
  exit101='echo "  process didn'"'"'t exit successfully: \`/t/target/debug/deps/a_test-0123456789abcdef\` (exit status: 101)"'
  check "results only: counted as before" "3 passed, 0 failed" "$green"
  check "a result line with failures: counted" "5 passed, 1 failed" "$green; $red; exit 101"
  check "a binary killed by a signal: named" "3 passed, 0 failed; 1 test binary died without a result: o69_fc1_on_off (signal 6)" "$green; $died; exit 101"
  check "a binary that exits 101 printed its result: not a death" "3 passed, 0 failed" "$green; $exit101; exit 101"
  check "no result lines, a death: still named" "died without a result: o69_fc1_on_off (signal 6)" "$died; exit 101"
  exit $(( fails > 0 ))
fi
label="${1:?usage: pinned.sh <label> <command...>}"; shift
[ $# -gt 0 ] || { echo "pinned.sh: no command" >&2; exit 64; }

# A template with its X's, which BSD and GNU mktemp both take; `-t prefix` is BSD's alone.
tmp="${TMPDIR:-/tmp}"
run="$(mktemp "${tmp%/}/pinned-$label.XXXXXXXX")"
before="$(mktemp "${tmp%/}/pin-before.XXXXXXXX")"
after="$(mktemp "${tmp%/}/pin-after.XXXXXXXX")"
trap 'rm -f "$run" "$before" "$after"' EXIT

# Every source file git can see, tracked or not. Sorted, so the manifest is a
# function of content and not of listing order. A subshell: the command runs where
# its caller stands, not in the product root (NC-14).
manifest() (
  cd "$product" || exit 1
  {
    git ls-files -z -- '*.rs' '*.toml' '*.json' '*.py' '*.js' '*.sh' '*.mjs'
    git ls-files -z --others --exclude-standard -- '*.rs' '*.toml' '*.json' '*.py' '*.js' '*.sh' '*.mjs'
  } | xargs -0 shasum -a 256 2>/dev/null | sort -k2
)

manifest > "$before" || { echo "pinned.sh: cannot read the tree at $product" >&2; exit 1; }
# The ceiling belongs INSIDE: wrapping this script in `timeout` kills the script
# before it can verify the pin or report anything, which is how the first real run
# of it ended — 25 minutes of suite, and the only output was "Terminated".
if [ -n "${PINNED_TIMEOUT:-}" ] && command -v timeout >/dev/null 2>&1; then
  timeout "$PINNED_TIMEOUT" "$@" > "$run" 2>&1
else
  "$@" > "$run" 2>&1
fi
status=$?
manifest > "$after" || { echo "pinned.sh: cannot read the tree at $product" >&2; exit 1; }

# The summary line, whatever suite produced it.
summary="$(grep -hE '^test result:' "$run" 2>/dev/null \
  | awk '{ok+=$4; bad+=$6} END { if (NR) printf "%d passed, %d failed", ok, bad }')"
# Test binaries a signal ended, by name (the hash suffix off), which no result line counts.
died="$(grep -hoE "process didn't exit successfully: \`[^\`]*\` \(signal: [0-9]+" "$run" 2>/dev/null \
  | sed -E "s/^[^\`]*\`([^\` ]*)[^\`]*\` \(signal: ([0-9]+)$/\1 (signal \2)/; s#^.*/##; s/-[0-9a-f]{16} / /" | sort -u)"
if [ -n "$died" ]; then
  n="$(printf '%s\n' "$died" | wc -l | tr -d ' ')"
  what="$n test binary died without a result"; [ "$n" = 1 ] || what="$n test binaries died without a result"
  summary="${summary:+$summary; }$what: $(printf '%s\n' "$died" | paste -sd, - | sed 's/,/, /g')"
fi
[ -n "$summary" ] || summary="$(tail -1 "$run" 2>/dev/null | cut -c1-72)"
echo "$summary"

if ! cmp -s "$before" "$after"; then
  echo "  VOID — the tree moved while '$label' ran; this result describes a tree that no longer exists:" >&2
  diff "$before" "$after" | grep -E '^[<>]' | awk '{print "    " $1 " " $3}' | sort -u | head -20 >&2
  echo "    full output: $run" >&2
  trap - EXIT; rm -f "$before" "$after"
  exit 2
fi

if [ $status -eq 124 ]; then
  echo "    TIMED OUT after ${PINNED_TIMEOUT}s — $label; partial output: $run" >&2
  echo "    (the pin held: nothing moved, so what did run is trustworthy as far as it got)" >&2
  trap - EXIT; rm -f "$before" "$after"
  exit 124
fi

if [ $status -ne 0 ]; then
  echo "    RED — $label; full output: $run" >&2
  trap - EXIT; rm -f "$before" "$after"
fi
exit $status
