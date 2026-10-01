#!/usr/bin/env bash
#
# The REAL auth service, on localhost, for the keyholder harness.
#
#   ./auth-dev.sh          start it on :8021, print the env it chose
#   ./auth-dev.sh stop     stop it
#
# This is site/auth run unmodified. Nothing here is a stand-in: the
# passkey ceremonies are py_webauthn's, the session cookie is the sealed JWT the
# deployed service issues, and /auth/signout is the endpoint under test.
#
# WHAT IS DELIBERATELY DIFFERENT FROM PRODUCTION, and the price of each:
#
#   WEBAUTHN_RP_ID=localhost   Production uses kenjin.cc, the registrable domain,
#                              so a passkey made in dev is the same passkey in
#                              production. A credential minted here is bound to
#                              RP ID `localhost` and is worthless anywhere else
#                              — which is right for a throwaway smoke database
#                              and would be a serious mistake anywhere near a
#                              real account. Hence a scratch DB, below.
#
#   WEBAUTHN_ORIGINS=…:8103,   Two origins, one RP ID. That is not a convenience:
#                    …:8104    it is the whole two-device claim. Both keyholder
#                              ports share RP ID `localhost`, so ONE passkey is
#                              usable from both — which is exactly what a passkey
#                              synced through iCloud Keychain is, and what makes
#                              two devices derive the same channel.
#
#   APPLE_*, PASSPORT_* unset  Both refuse http and localhost return URLs, so
#                              those two doors cannot complete from here however
#                              they are configured. The service says so on boot
#                              rather than offering a door that dead-ends.
#
#   ORECLOUD_TOKEN unset       Sign-in works; no store is written. The service
#                              logs this itself.
#
# The database is a scratch file under /tmp and is deleted on every start. An
# accumulating local account table is how a "returning user" test starts passing
# for the wrong reason.

set -euo pipefail
cd "$(dirname "$0")"

AUTH_DIR="$(cd ../../../business/website/auth && pwd)"
VENV="$(cd ../../.. && pwd)/.venv"
PORT=8021
DB=/tmp/kenjin-smoke-auth.db

stop() {
  lsof -ti:"$PORT" 2>/dev/null | xargs -r kill 2>/dev/null || true
  echo "stopped"
}
if [ "${1:-}" = "stop" ]; then stop; exit 0; fi
stop >/dev/null 2>&1 || true

rm -f "$DB"

# Fixed rather than random, so restarting the service does not invalidate a
# session a browser tab is already holding — a smoke run that has to re-sign-in
# after every restart tests the harness, not the code.
export SESSION_SECRET="smoke-only-session-secret-not-for-any-real-deployment"
export DB_PATH="$DB"
export PUBLIC_ORIGIN="http://localhost:8103"
export WEBAUTHN_RP_ID="localhost"
export WEBAUTHN_ORIGINS="http://localhost:8103,http://localhost:8104"
export WEBAUTHN_RP_NAME="Kenjin (smoke)"
# The Arc's ADDRESS, not its relay -- the service discovers the relay from it by
# asking GET $ARC_URL/v1/arc, and records both. hosting/run-arc-local.sh puts the
# gateway on :8080 with every plane behind it; if it is not running, discovery
# fails, no Arc is recorded, and sign-in still works. That is the service's own
# rule -- "an Arc that is down must not stop somebody signing in" -- and the
# smoke run says which of the two it got.
export ARC_URL="${ARC_URL:-http://127.0.0.1:8080}"
# Both land back on the keyholder door rather than the site's /site and /profile,
# which do not exist on this origin.
export AFTER_SIGNIN_PATH="/"
export FIRST_RUN_PATH="/"

( cd "$AUTH_DIR" && ( nohup "$VENV/bin/python" -m uvicorn app.main:app \
      --host 127.0.0.1 --port "$PORT" --log-level warning \
      </dev/null >>/tmp/kenjin-smoke-auth.log 2>&1 & ) )

for _ in $(seq 40); do
  if curl -fs -o /dev/null --max-time 1 "http://127.0.0.1:$PORT/auth/health"; then
    echo "auth service  http://127.0.0.1:$PORT   (rp_id=localhost, db=$DB)"
    curl -s "http://127.0.0.1:$PORT/auth/health"; echo
    echo
    echo "point the keyholder at it:"
    echo "  AUTH_UPSTREAM=http://127.0.0.1:$PORT ./run.sh"
    exit 0
  fi
  sleep 0.25
done
echo "auth service FAILED to come up on $PORT — see /tmp/kenjin-smoke-auth.log" >&2
tail -20 /tmp/kenjin-smoke-auth.log >&2 || true
exit 1
