#!/usr/bin/env bash
#
# The whole chain, from a seed to a delta on the other device.
#
#   ./smoke/run.sh          bring everything up and run both legs
#   ./smoke/run.sh stop     stop what this started (not the Arc)
#
# WHAT RUNS, and what each leg is for:
#
#   e2e.py       the auth service and the relay, driven by an independent
#                implementation. A seed, the wrap that registers the account,
#                the public wrap read, the five ways a signature is refused —
#                four asking whether it is real and one asking whether it is the
#                signer's to use here — the history behind one, an address
#                claimed and held against a second account, and a sealed delta
#                crossing between two devices — the second of which has never
#                held the seed and has to open the wrap to get it. It also puts
#                its own four derivations against pacific-core, through
#                smoke/core-vectors.mjs and the wasm in docs/core: a retyped
#                constant that has drifted is a failure nothing else in this
#                suite can see — and if that cross-check cannot be run at all,
#                the leg exits 3 rather than reporting a green run that never
#                asked.
#
#   browser.py   the same account handed to a real browser, where keyholder.js
#                fetches the wrap, opens it with the passkey, derives the channel
#                from the seed and publishes through its own store.commit. This
#                process, subscribed to the tag IT derived, reads what the
#                browser sealed. Two implementations agreeing is the only thing
#                that can catch them drifting apart.
#
# THERE IS NO WEBAUTHN IN THE SERVICE, and that is why this file changed. Ruled
# 13 Sep 2026: the seed is the root and the passkey is a local unlock that never
# reaches the Arc. /auth/passkey/*, /auth/session and /auth/signout are gone, and
# so are the tests that drove them. What the Arc does is issue a nonce and check
# a signature over it.
#
# WHAT IT DOES NOT COVER, so nobody reads a green run as more than it is:
#
#   · A real authenticator. Touch ID needs a person; smoke/authn.py and
#     keyholder/_authn-shim.js stand in, the way Chrome's and Safari's virtual
#     authenticators do. What keeps that honest is that the wrap this process
#     seals has to open in the browser: a PRF that is wrong is a wrap that does
#     not open, and nothing downstream of it happens at all.
#   · The words. `recovery_key_from_seed` is the core's, and restoring from 24
#     words on a device that holds nothing is the one door neither leg walks.
#   · The head record (PUT/GET /auth/users/<pk>/head). New on the service, and
#     not exercised here.
#   · TLS. The keyholder proxy strips Secure from cookies on the loopback hop
#     and says so where it does it.
#
# THE ARC IS NOT STARTED HERE. The Arc launcher owns the gateway, the relay and
# three other planes, and starting a second copy of any of them from a smoke
# script is how you end up debugging the harness. If it is not up, this says so
# and stops.

set -uo pipefail
cd "$(dirname "$0")/.."

WEB="$(pwd)"
VENV="$(cd ../../.. && pwd)/.venv"
PY="$VENV/bin/python"
AUTH_PORT=8021
RELAY=ws://localhost:8787
GATEWAY=http://127.0.0.1:8080

# BOTH LEGS TALK TO THE KEYHOLDER'S ORIGIN, not to :8021 directly. That origin
# proxies /auth, and it is what `location.host` gives keyholder.js — and the host
# is inside the wrap's AAD and inside the bytes a device signs. Seal or sign for
# a different one and nothing opens and nothing verifies.
export AUTH=http://localhost:8103

if [ "${1:-}" = "stop" ]; then
  ./auth-dev.sh stop
  ./run.sh stop
  rm -f keyholder/_smoke-cred.json
  exit 0
fi

# A credential left over from a run that died is a credential the next browser
# session can fetch off a served origin. Never inherit one.
rm -f keyholder/_smoke-cred.json

fail() { printf '\n\033[31m%s\033[0m\n' "$*" >&2; exit 1; }

curl -fs -o /dev/null --max-time 3 "$GATEWAY/v1/arc" \
  || fail "no Arc on $GATEWAY — start it with the Arc launcher (arc/arcup)"
"$PY" - <<EOF >/dev/null 2>&1 || fail "no relay on $RELAY — the Arc launcher starts it"
import asyncio, websockets
asyncio.run(websockets.connect("$RELAY").__aenter__())
EOF

echo "» the real auth service"
./auth-dev.sh >/dev/null || fail "the auth service would not start — see /tmp/kenjin-smoke-auth.log"

echo "» the keyholder origins, pointed at it"
AUTH_UPSTREAM="http://127.0.0.1:$AUTH_PORT" ./run.sh >/dev/null || fail "run.sh failed"

# WHAT EACH CODE MEANS, and every leg uses the same set:
#
#   0  everything the leg could ask, it asked, and the answer was yes.
#   1  IT RAN AND DISAGREED. The only code that means something is wrong with
#      the system under test rather than with the harness around it.
#   2  IT REFUSED TO RUN. The browser leg's driver page is stale, and the leg
#      says exactly what the page has to call instead. Kept distinct from 1
#      because a leg that cannot start and a leg that started and disagreed are
#      two different pieces of news.
#   3  IT RAN BUT COULD NOT ASK EVERYTHING. e2e.py returns this when its core
#      cross-check had no node or no staged wasm to run against. Nothing failed
#      and the run is still not green: the check was not answered.
#
# EACH LEG'S CODE IS KEPT, and the run's code is decided once, from both. The
# shape this replaces assigned the browser leg's code straight over the e2e
# leg's, so a genuine e2e FAILURE sitting behind today's browser refusal left
# the run reporting only the refusal — the worse news masked by the lesser, in
# the one script whose whole job is to report accurately.
e=0
b=0
"$PY" smoke/e2e.py || e=$?

SHOT="${SHOT:-}" "$PY" smoke/browser.py || b=$?

if [ -n "${SHOT:-}" ]; then
  echo "  the page's own log: smoke/browser.png"
else
  echo
  echo "  browser.py needs a headless browser to drive the page:"
  echo "    SHOT=/path/to/shot ./smoke/run.sh"
fi

# 1 wins outright: a leg that ran and disagreed is never masked by a leg that
# would not start or could not finish. Below that the higher code wins, so an
# incomplete run (3) is not reported as a refusal (2) and neither is reported
# as success.
if [ "$e" -eq 1 ] || [ "$b" -eq 1 ]; then
  rc=1
elif [ "$e" -gt "$b" ]; then
  rc="$e"
else
  rc="$b"
fi

leg() {
  case "$1" in
    0) printf 'passed' ;;
    1) printf '\033[31mFAILED\033[0m' ;;
    2) printf '\033[33mrefused to run\033[0m' ;;
    3) printf '\033[33mINCOMPLETE: a check could not be run\033[0m' ;;
    *) printf 'exit %s' "$1" ;;
  esac
}

# Said out loud, because an exit code nobody reads is not a report.
echo
printf '» e2e %s    browser %s    → exit %s\n' "$(leg "$e")" "$(leg "$b")" "$rc"
exit $rc
