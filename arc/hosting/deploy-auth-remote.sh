#!/usr/bin/env bash
# Runs ON the Arc's box (piped in by deploy-auth.sh). Builds the kenjin-auth image
# from the shipped source and swaps the container, health-gated with rollback.
#
# THE SECRETS ARE NOT SYNCED, NOT IN THE IMAGE, AND NOT IN THIS DIRECTORY.
# They live in ~/kenjin-auth-secrets, deliberately outside the shipped tree:
# deploy-auth.sh replaces ~/kenjin-auth with the commit's arc/auth on every deploy,
# so a secret kept there would be removed by the next one.
#
# They are mounted at run time, never built in -- a rebuild must not be able to
# bake the Apple signing key into a layer cache.
set -euo pipefail

# PUBLISHED ON LOOPBACK so the ARC TUNNEL can reach it. `cloudflared-arc` fronts
# arc.wallflowers.io and runs on the host, not in the `kenjin` docker network, so
# a container reachable only by its network alias is a container that tunnel
# cannot route to. 127.0.0.1 and not 0.0.0.0: the box exposes no ports to the
# internet and this must not become the first one.
#
# It has to live in THIS script rather than being set by hand, because
# deploy-auth.sh replaces the tree wholesale and every deploy recreates the
# container -- a port added by hand disappears on the next deploy and
# arc.wallflowers.io/auth/ starts 502ing with nothing in the diff to explain it.
SRC=~/kenjin-auth
SECRETS=~/kenjin-auth-secrets
NAME=kenjin-auth
NET=kenjin
KEEP=5

cd "$SRC"

[ -f "$SECRETS/.env" ] || { echo "FATAL: $SECRETS/.env missing: run arc/hosting/push-auth-secrets.sh first" >&2; exit 1; }
[ -f "$SECRETS/AuthKey.p8" ] || { echo "FATAL: $SECRETS/AuthKey.p8 missing: run arc/hosting/push-auth-secrets.sh first" >&2; exit 1; }

# A commit carries no .env, .env.dev or scratch db (NC-78), and the replaced tree
# drops any an older deploy left. None is read by anything, but a stale .env beside
# the source is the file someone debugs by mistake at 2am, so they are removed here too.
rm -f "$SRC/.env" "$SRC/.env.dev" "$SRC"/*.db "$SRC"/*.db-wal "$SRC"/*.db-shm 2>/dev/null || true

docker network inspect "$NET" >/dev/null 2>&1 || docker network create "$NET" >/dev/null

SHA="${DEPLOY_SHA:-unknown}"

# By ID, not by tag: :latest is about to move onto the new build, so a tag
# captured here would follow it and the rollback target would be the thing we
# just deployed. Same trap the site script documents.
PREV_ID=$(docker inspect "$NAME" --format '{{.Image}}' 2>/dev/null || true)
if [ -n "$PREV_ID" ]; then
  docker tag "$PREV_ID" "$NAME:previous"
else
  echo "==> no running container: first deploy, nothing to roll back to yet"
fi

echo "==> building $SHA"
docker build -q -t "$NAME:$SHA" \
  --label "org.opencontainers.image.revision=$SHA" \
  --label "org.opencontainers.image.created=$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  . >/dev/null
docker tag "$NAME:$SHA" "$NAME:latest"

run_container() {           # $1 = image ref
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  # Published on loopback only, for the Arc tunnel (above); the site container
  # reaches it over the shared network. Never on 0.0.0.0.
  docker run -d --name "$NAME" --restart unless-stopped \
    --network "$NET" \
    -p 127.0.0.1:8021:8000 \
    --env-file "$SECRETS/.env" \
    -v kenjin-auth-data:/data \
    -v "$SECRETS/AuthKey.p8:/run/secrets/AuthKey.p8:ro" \
    "$1" >/dev/null
}

echo "==> swapping container"
run_container "$NAME:latest"

echo "==> health gate"
ok=0
for _ in $(seq 1 25); do
  # From inside the network, since nothing is published to the host.
  if docker run --rm --network "$NET" curlimages/curl:latest \
        -fsS -o /dev/null --max-time 3 "http://$NAME:8000/auth/health" 2>/dev/null; then ok=1; break; fi
  sleep 1
done

if [ "$ok" != 1 ]; then
  echo "!! health check FAILED" >&2
  docker logs --tail 40 "$NAME" 2>&1 | sed 's/^/   /' >&2
  if docker image inspect "$NAME:previous" >/dev/null 2>&1; then
    echo "!! rolling back to :previous" >&2
    run_container "$NAME:previous"
    docker tag "$NAME:previous" "$NAME:latest"
  else
    echo "   No :previous to roll back to; the failed build is left for inspection." >&2
  fi
  exit 1
fi

docker images "$NAME" --format '{{.Tag}} {{.CreatedAt}}' \
  | grep -vE '^(latest|previous) ' | sort -k2 -r | tail -n +$((KEEP + 1)) \
  | awk '{print $1}' | while read -r t; do docker rmi "$NAME:$t" >/dev/null 2>&1 || true; done
docker image prune -f >/dev/null 2>&1 || true

echo "==> deployed OK  $SHA"
