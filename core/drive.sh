#!/usr/bin/env bash
#
# drive.sh — bring up everything you can click on, in one command.
#
#   ./drive.sh          start it all and print the URLs
#   ./drive.sh signup   re-copy ONLY the signup pages (after you edit them)
#   ./drive.sh stop     stop what this script started
#
# WHY THIS EXISTS. Three of these surfaces need something non-obvious done to
# them before they will answer, and none of it is written down anywhere else:
#
#   * THE SIGNUP PAGES ARE NOT IN THE IMAGE. The running site container was built
#     on 2 September, before site/soon/ existed, so /signup 404s behind the lock
#     no matter what is in your working tree. They have to be copied in.
#
#   * THE COPY GOES TO `kenjin-gatetest`, NOT `gatetest-tls`. Two containers:
#     gatetest-tls is only a TLS terminator and proxies everything to the other
#     one. Copying into gatetest-tls looks like it works — docker cp reports
#     success — and changes nothing, because nothing is served from there.
#
#   * THE CRYPTO PAYLOAD IS GITIGNORED BUILD OUTPUT. site/assets/pacific/ (the
#     account module + a 3.5 MB wasm) is produced by app/web/account/stage.sh and
#     is not in git. Without it the page loads and its <script> 404s, so
#     PacificAccount is undefined and the account step dies silently.
#
# All three are undone by a container restart, so this is re-runnable and
# idempotent rather than a one-time fix.
set -euo pipefail
# drive.sh lives in core/; the product it drives is core's parent, and the
# workspace (business/ and the .venv) is the product's parent.
cd "$(dirname "$0")/.."
ROOT="$PWD"
WS="$(cd .. && pwd)"
SITE="$WS/business/website/site"
WEB="$ROOT/app/web"
SERVER=kenjin-gatetest          # the one that serves; NOT gatetest-tls
AUTH=kenjin-auth                # the auth service behind /auth/ — also a baked image
TLS_URL=https://127.0.0.1:8793  # the terminator in front of it

say() { printf '  %-14s %s\n' "$1" "$2"; }

stage_signup() {
  # ALWAYS, not just when it is absent. This used to stage only if the file was
  # missing, so every edit to pacific-account.js after the first run was copied
  # into the container from a stale staged copy — the page ran the old module and
  # nothing anywhere said which one it was running. stage.sh is idempotent and
  # refuses a stale core, so there is nothing to save by skipping it.
  echo "  staging the account module (app/web/account/stage.sh)…"
  "$WEB/account/stage.sh" >/dev/null
  docker exec "$SERVER" sh -c 'mkdir -p /usr/share/nginx/soon /usr/share/nginx/html/assets/pacific'
  for f in signup signin profile; do
    docker cp "$SITE/soon/$f.html" "$SERVER:/usr/share/nginx/soon/$f.html" >/dev/null
  done
  docker cp "$SITE/assets/pacific/pacific-account.js" \
            "$SERVER:/usr/share/nginx/html/assets/pacific/pacific-account.js" >/dev/null
  docker cp "$SITE/assets/pacific/core" "$SERVER:/usr/share/nginx/html/assets/pacific/" >/dev/null

  # AND THE SCRIPTS THE PAGES ACTUALLY RUN. For a long time this copied the three
  # pages and the account module and nothing else, so `assets/signin.js` — the
  # file that decides what a signed-in person is shown — stayed at whatever the
  # September image baked in. An edit to it changed nothing and said nothing.
  #
  # The list is READ OUT OF THE PAGES rather than written here, so a page that
  # starts loading a new script is covered without anyone remembering to come
  # back. Only `assets/...` (relative, same tree) — /brand/ and the pacific/
  # subtree are handled above and below.
  for f in signup signin profile; do
    grep -oE 'src="assets/[^"]+"' "$SITE/soon/$f.html" 2>/dev/null \
      | sed 's/^src="//; s/"$//; s/?.*$//'
  done | sort -u | grep -v '^assets/pacific/' | while read -r rel; do
    [ -f "$SITE/$rel" ] || { echo "  MISSING  $rel (referenced by a gated page)"; continue; }
    docker exec "$SERVER" mkdir -p "/usr/share/nginx/html/$(dirname "$rel")"
    docker cp "$SITE/$rel" "$SERVER:/usr/share/nginx/html/$rel" >/dev/null
  done
}

# THE AUTH CONTAINER RUNS A BAKED IMAGE TOO, and this was rediscovered the hard
# way on 17 Sep: `kenjin-auth:gatetest` was built on 15 September, so every edit
# to site/auth/app/ since then was in the tree and not in the thing
# answering /auth/. It fails convincingly — the service is up, health is 200, and
# only the NEW behaviour is missing, which reads as "my change does not work"
# rather than "my change is not there".
#
# Same remedy as the signup pages above, and for the same reason: copy in rather
# than rebuild, because a rebuild is slow and the image is a fixture anyway. Also
# undone by a container restart from the image, so it is re-runnable.
stage_auth() {
  docker ps --format '{{.Names}}' | grep -qx "$AUTH" || {
    echo "  no $AUTH container — skipping the auth app copy"
    return 0
  }
  for f in "$WS/business/website/auth/app/"*.py; do
    docker cp "$f" "$AUTH:/srv/app/$(basename "$f")" >/dev/null
  done
  docker restart "$AUTH" >/dev/null
  # uvicorn needs a moment, and a caller that races it sees connection refused
  # and blames the copy.
  for _ in 1 2 3 4 5 6 7 8; do
    curl -sk "$TLS_URL/auth/health" >/dev/null 2>&1 && break
    sleep 1
  done
}

case "${1:-up}" in
  stop)
    "$WEB/run.sh" stop || true
    pkill -f "http.server 8110" || true
    echo "stopped (the containers and the release relay were not started by this script)"
    exit 0 ;;
  signup)
    stage_signup; echo "  signup pages re-copied — hard-reload the tab"; exit 0 ;;
  auth)
    stage_auth; echo "  auth app re-copied and restarted"; exit 0 ;;
esac

# 1 · the relay two devices converge through.
if ! nc -z 127.0.0.1 8787 2>/dev/null; then
  echo "  no relay on 8787 — start one:"
  echo "    cd arc && cargo build -p relay && \\"
  echo "      RELAY_STORE=/tmp/relay.db ./target/debug/semaphore &"
fi

# 2 · the web fixture origins (8100 docs, 8103/8104 two keyholder origins).
"$WEB/run.sh" >/dev/null 2>&1 || true

# 3 · the webapp, which run.sh does not serve.
pgrep -f "http.server 8110" >/dev/null || \
  (cd "$WEB/demo" && nohup python3 -m http.server 8110 --bind 127.0.0.1 >/dev/null 2>&1 &)

# 4 · your signup pages, into the container that serves them.
stage_signup

# 5 · and the auth app, into the container that answers /auth/.
stage_auth

cat <<EOF

REAL PASSKEYS, REAL DEVICES — the only way to test interdevice sync
  site/auth/dev/live.sh        (runs the tunnel; ctrl-c stops it)
    SIGN UP    https://dev.wallflowers.io/signup
    SIGN IN    https://dev.wallflowers.io/signin

  WHY NOT localhost. WebAuthn allows localhost as a secure context, so a passkey
  made there is real — and its RP ID is \`localhost\`, so it is STRANDED: it can
  never be presented at wallflowers.io and no second device will ever see it. A
  localhost run cannot tell you anything about sync. The RP ID here is
  wallflowers.io, the registrable domain, so a credential made at dev. is the
  same credential at the apex and iCloud Keychain carries it between your devices.

LOCAL, for layout and flow — NOT for passkeys
EOF
say "SIGNUP"    "$TLS_URL/signup      (behind the lock — SITE_PW in site/soon/.env)"
say "SIGN IN"   "$TLS_URL/signin"
say "PROFILE"   "$TLS_URL/profile"
say "WEBAPP"    "http://localhost:8110/"
cat <<EOF

THE INTERIOR, over a captured site
EOF
say "STOMA"     "http://localhost:8100/pacific-site.html?site=stoma"
say "TWO DEV"   "http://localhost:8100/pacific-two-devices.html"
say "THE DOOR"  "http://localhost:8103/"
cat <<EOF

ON THE PHONE, in $WS/ios
  just run            build, install and launch on the simulator
  ./watch.sh          SEE WHAT IT IS DOING — its own narration
  ./watch.sh fresh    the same, relaunched at the door
  ./watch.sh all      every line, Apple's included

THE HARNESS, if you want to watch the rules rather than click them
  .venv/bin/python -m harness render     the cases, live
  .venv/bin/python -m harness all        every leg; exit 0 only if green

EOF
