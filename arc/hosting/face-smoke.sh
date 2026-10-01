#!/usr/bin/env bash
# THE HOSTING STACK, END TO END, ON THIS MACHINE.
#
# Starts the three real binaries — relay, arc-node, gateway — and the real address
# authority (the auth service), mints a REAL second device (core's `publish_a_face`
# example, an ordinary Node), publishes a face onto a Host with the Arc on its roster,
# and then asks the gateway for it over HTTP. Nothing here is stubbed: every byte the
# gateway serves was folded out of an MLS group the Arc is a member of.
#
# THE ADDRESS IS THE SIGNUP CLAIM'S (ruled 22 Sep 2026). So the smoke proves both halves:
# the Arc REFUSES the face while its slug is unclaimed, and serves it once the owner has
# claimed the slug and bound it to the Host — at the auth service, signed by the owner.
#
# The auth service lives in business/website/auth, beside product/. The smoke starts it
# from there (or from FACE_SMOKE_AUTH_DIR), or uses one already running at
# FACE_SMOKE_AUTHORITY; with neither it stops and says so rather than skipping the half
# of the path that decides who may use a name.
#
# It is a smoke test, not a deploy: it binds to 127.0.0.1 on high ports and puts all its
# state under a temp directory it removes on exit (keep it with FACE_SMOKE_KEEP=1).
#
#   arc/hosting/face-smoke.sh [slug]
#
# Paths are derived from this script's own location, never written down.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
arc="$(dirname "$here")"
core="$(dirname "$arc")/core"
export PATH="$HOME/.cargo/bin:$PATH"

slug="${1:-allotments}"
relay_port="${FACE_SMOKE_RELAY_PORT:-18787}"
node_port="${FACE_SMOKE_NODE_PORT:-18790}"
gw_port="${FACE_SMOKE_GW_PORT:-18791}"
auth_port="${FACE_SMOKE_AUTH_PORT:-18792}"
workspace="$(dirname "$(dirname "$arc")")"
auth_dir="${FACE_SMOKE_AUTH_DIR:-$workspace/business/website/auth}"
uvicorn="$workspace/.venv/bin/uvicorn"
work="$(mktemp -d "${TMPDIR:-/tmp}/face-smoke.XXXXXX")"
pids=()

cleanup() {
  for p in "${pids[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done
  if [ "${FACE_SMOKE_KEEP:-0}" = "1" ]; then
    echo "kept: $work"
  else
    rm -rf "$work"
  fi
}
trap cleanup EXIT

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
wait_for() { # wait_for <url> <what>
  for _ in $(seq 1 100); do
    curl -fsS -o /dev/null "$1" 2>/dev/null && return 0
    sleep 0.2
  done
  echo "face-smoke: $2 never came up at $1" >&2
  return 1
}

say "building (relay, arc-node, gateway, and the owner's device)"
( cd "$arc" && cargo build -p relay -p node -p arc-gateway )
( cd "$core" && cargo build -p pacific-core --example publish_a_face )

say "starting the relay on :$relay_port"
RELAY_BIND="127.0.0.1:$relay_port" "$arc/target/debug/semaphore" >"$work/relay.log" 2>&1 &
pids+=($!)

if [ -n "${FACE_SMOKE_AUTHORITY:-}" ]; then
  authority="${FACE_SMOKE_AUTHORITY%/}"
  say "using the address authority already at $authority"
elif [ -f "$auth_dir/app/main.py" ] && [ -x "$uvicorn" ]; then
  authority="http://127.0.0.1:$auth_port"
  say "starting the address authority (the auth service) on :$auth_port"
  ( cd "$auth_dir" && DB_PATH="$work/auth.db" PUBLIC_ORIGIN="$authority" \
      exec "$uvicorn" app.main:app --host 127.0.0.1 --port "$auth_port" ) >"$work/auth.log" 2>&1 &
  pids+=($!)
  wait_for "$authority/auth/health" "the auth service"
else
  echo "face-smoke: no address authority. The Arc serves a slug only from the Host its" >&2
  echo "  signup claim is bound to, so this smoke needs the auth service: check out" >&2
  echo "  business/website beside product/ (looked in $auth_dir, with $uvicorn)," >&2
  echo "  or set FACE_SMOKE_AUTH_DIR, or FACE_SMOKE_AUTHORITY=<url of a running one>." >&2
  exit 1
fi
audience="${authority#*://}"

say "starting arc-node on :$node_port"
PACIFIC_STATE_DIR="$work/arc" \
ARC_STATE_DIR="$work/arc-state" \
ARC_NAME="Smoke Arc" \
ARC_RELAY_URL="ws://127.0.0.1:$relay_port" \
ARC_SIGNUP_BIND="127.0.0.1:$node_port" \
ARC_SLUG_AUTHORITY="$authority" \
ARC_SYNC_MS=1000 \
  "$arc/target/debug/node" >"$work/node.log" 2>&1 &
pids+=($!)
wait_for "http://127.0.0.1:$node_port/health" "arc-node"

say "starting the gateway on :$gw_port"
PORT="$gw_port" \
ARC_MEMBERSHIP_UPSTREAM="http://127.0.0.1:$node_port" \
ARC_RELAY_UPSTREAM="ws://127.0.0.1:$relay_port" \
  "$arc/target/debug/arc-gateway" >"$work/gateway.log" 2>&1 &
pids+=($!)
wait_for "http://127.0.0.1:$gw_port/v1/health" "gateway"

arc_identity="$(curl -fsS "http://127.0.0.1:$node_port/arc" | python3 -c 'import json,sys; print(json.load(sys.stdin)["identity_key"])')"
curl -fsS "http://127.0.0.1:$node_port/bundle" >"$work/arc-bundle.json"
echo "the Arc is $arc_identity"

say "the owner's device publishes a face at /$slug"
owner_dev="$core/target/debug/examples/publish_a_face"
PACIFIC_STATE_DIR="$work/owner" \
  "$owner_dev" \
    --relay "ws://127.0.0.1:$relay_port" \
    --bundle "$work/arc-bundle.json" \
    --arc "$arc_identity" \
    --slug "$slug" \
    --name "Mill Road Allotments" | tee "$work/owner.out"
owner_pk="$(awk '/^owner /{print $3}' "$work/owner.out")"; owner_pk="${owner_pk#ed25519:}"
host_id="$(awk '/^host /{print $3}' "$work/owner.out")"
[ -n "$owner_pk" ] && [ -n "$host_id" ] || { echo "face-smoke: the owner's device did not say who it is and which Host it made"; exit 1; }

say "the Arc refuses /$slug while nobody has claimed it"
refused=""
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:$node_port/faces" | grep -q "never claimed"; then refused=1; break; fi
  sleep 0.5
done
[ -n "$refused" ] || { echo "face-smoke: the Arc did not refuse an unclaimed slug"; curl -fsS "http://127.0.0.1:$node_port/faces"; echo; tail -40 "$work/node.log"; exit 1; }
curl -fsS "http://127.0.0.1:$node_port/faces" | grep -q "\"$slug\"" && { echo "face-smoke: an unclaimed slug was served"; exit 1; }
echo "  refused, with the reason: never claimed"

# One signed request to the authority, the way every write to it is signed: a fresh
# single-use challenge, and the owner's device signing it with its own key.
signed() { # signed <method> <path> <content-type> <body-file>
  local nonce sig
  nonce="$(curl -fsS "$authority/auth/challenge" | python3 -c 'import json,sys; print(json.load(sys.stdin)["nonce"])')"
  sig="$(PACIFIC_STATE_DIR="$work/owner" "$owner_dev" --sign --audience "$audience" --nonce "$nonce")"
  curl -fsS -X "$1" "$authority$2" -H "Content-Type: $3" \
       -H "X-Pacific-Challenge: $nonce" -H "X-Pacific-Signature: $sig" --data-binary "@$4"
  echo
}

say "the owner registers, claims /$slug and binds it to their Host"
PACIFIC_STATE_DIR="$work/owner" "$owner_dev" --wrap --host "$audience" \
  | python3 -c 'import sys,binascii; sys.stdout.buffer.write(binascii.unhexlify(sys.stdin.read().strip()))' >"$work/wrap.bin"
signed PUT "/auth/users/$owner_pk" application/octet-stream "$work/wrap.bin"
printf '{"slug":"%s"}' "$slug" >"$work/claim.json"
signed POST "/auth/users/$owner_pk/sites" application/json "$work/claim.json"
printf '{"host":"%s"}' "$host_id" >"$work/bind.json"
signed PUT "/auth/users/$owner_pk/sites/$slug/host" application/json "$work/bind.json"
bound="$(curl -fsS "$authority/auth/site/$slug/host")"
echo "  the authority now says: $bound"
case "$bound" in *"$host_id"*) ;; *) echo "face-smoke: the binding did not take"; exit 1;; esac
case "$bound" in *"$owner_pk"*) echo "face-smoke: the authority named the owner"; exit 1;; esac

say "waiting for the Arc to fold the Host and the gateway's cache to turn over"
served=""
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:$node_port/faces" | grep -q "\"$slug\""; then served=1; break; fi
  sleep 0.5
done
[ -n "$served" ] || { echo "face-smoke: the Arc never served /$slug"; echo "--- node.log ---"; tail -40 "$work/node.log"; exit 1; }
curl -fsS "http://127.0.0.1:$node_port/faces" | head -c 400; echo

say "GET /v1/face/$slug"
curl -fsS -D "$work/h" "http://127.0.0.1:$gw_port/v1/face/$slug" -o "$work/face.html"
grep -Ei 'content-security-policy|content-type' "$work/h" || true
printf 'bytes: %s\n' "$(wc -c <"$work/face.html" | tr -d ' ')"
grep -o '<title>[^<]*</title>' "$work/face.html" || true
for must in 'Mill Road Allotments' 'Dig day' '/m/mark'; do
  grep -q "$must" "$work/face.html" || { echo "face-smoke: the page is missing \"$must\""; exit 1; }
  echo "  carries: $must"
done
grep -qi '<script' "$work/face.html" && { echo "face-smoke: the served face contains script"; exit 1; }
echo "  and no <script> anywhere in it"

for path in "escape" "door"; do
  say "GET /v1/face/$slug/$path"
  curl -fsS -o "$work/$path.html" "http://127.0.0.1:$gw_port/v1/face/$slug/$path"
  printf 'bytes: %s\n' "$(wc -c <"$work/$path.html" | tr -d ' ')"
done

say "GET /v1/face/$slug/m/mark"
curl -fsS -D "$work/hm" -o "$work/mark.png" "http://127.0.0.1:$gw_port/v1/face/$slug/m/mark"
grep -i 'content-type' "$work/hm" || true
file "$work/mark.png" 2>/dev/null || printf 'bytes: %s\n' "$(wc -c <"$work/mark.png" | tr -d ' ')"

say "a slug nobody published is a 404"
code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$gw_port/v1/face/nobody-here")"
echo "GET /v1/face/nobody-here -> $code"
[ "$code" = "404" ] || { echo "face-smoke: expected 404"; exit 1; }

say "the whole hosting path works on this machine."
