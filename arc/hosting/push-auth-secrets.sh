#!/usr/bin/env bash
# Runs LOCALLY. Places kenjin-auth's secrets on the Arc's box, once.
#
#   LOCAL_ENV=<the local secrets file> arc/hosting/push-auth-secrets.sh
#
# ARC_BOX and ARC_KEY as deploy-arc.sh's.
#
# These never enter git and never enter the image. They are mounted at run time
# (see deploy-auth-remote.sh), so they survive every subsequent deploy and a
# rebuild cannot bake the Apple signing key into a layer.
#
# THEY LIVE OUTSIDE THE SHIPPED DIRECTORY, and that is not a preference.
# deploy-auth.sh replaces ~/kenjin-auth with the commit's arc/auth: put the real
# secrets there and the next deploy removes them. The local source is outside every
# repository, named by LOCAL_ENV, so no commit can carry it (NC-78).
#
# What is derived rather than copied verbatim: the local .env.auth points
# APPLE_PRIVATE_KEY_FILE at a path on this laptop and DB_PATH at a scratch file.
# Both are rewritten for the container's filesystem here.
set -euo pipefail

BOX=${ARC_BOX:-ubuntu@167.99.83.136}
KEY_SSH=${ARC_KEY:-$HOME/.ssh/machine0_key}
SSHC="ssh -o BatchMode=yes -o ConnectTimeout=15 -i $KEY_SSH"
REMOTE="${AUTH_SECRETS:-\$HOME/kenjin-auth-secrets}"
LOCAL_ENV="${LOCAL_ENV:?LOCAL_ENV=the local secrets file, outside every repository}"

[ -f "$LOCAL_ENV" ] || { echo "FATAL: $LOCAL_ENV not found"; exit 1; }

KEY=$(sed -n 's/^APPLE_PRIVATE_KEY_FILE=//p' "$LOCAL_ENV" | tr -d '"'"'"'')
[ -f "$KEY" ] || { echo "FATAL: APPLE_PRIVATE_KEY_FILE points at $KEY, which does not exist"; exit 1; }

# THE AUDIENCE IS SET HERE, and moving it breaks every sign-in. The service signs for
# PUBLIC_ORIGIN's host (or AUTH_AUDIENCE), and the Door signs a challenge for no audience
# but its auth service's own (app/door/src/auth.rs). So the origin is the one
# the service answers as today, arc.wallflowers.io (GET /auth/challenge names it); this
# line once wrote https://kenjin.cc, which would have failed every Door sign-in (O-54).
# Another is AUTH_PUBLIC_ORIGIN=..., and one that moves the live audience is refused
# unless AUTH_MOVE_AUDIENCE=1 says that is the intent.
LIVE_ORIGIN=https://arc.wallflowers.io
ORIGIN="${AUTH_PUBLIC_ORIGIN:-$LIVE_ORIGIN}"
WANT="${ORIGIN#https://}"
if LIVE=$(curl -fsS --max-time 20 "$LIVE_ORIGIN/auth/challenge" | python3 -c 'import json,sys; print(json.load(sys.stdin)["audience"])'); then
  [ "$LIVE" = "$WANT" ] || [ "${AUTH_MOVE_AUDIENCE:-}" = 1 ] \
    || { echo "FATAL: the live service signs for $LIVE; this would move it to $WANT (AUTH_MOVE_AUDIENCE=1 if that is the intent)"; exit 1; }
else
  echo "==> no service answers at $LIVE_ORIGIN: a new box, whose audience is $WANT"
fi
SET=$(sed -n 's/^AUTH_AUDIENCE=//p' "$LOCAL_ENV" | tr -d '"'"'"'')
[ -z "$SET" ] || [ "$SET" = "$WANT" ] || { echo "FATAL: $LOCAL_ENV sets AUTH_AUDIENCE=$SET, not $WANT"; exit 1; }
echo "==> audience $WANT"

echo "==> creating $REMOTE on $BOX"
$SSHC "$BOX" "mkdir -p $REMOTE && chmod 700 $REMOTE"

echo "==> production .env (origin + paths rewritten for the container)"
# PUBLIC_ORIGIN is always written, whatever the local file holds: without it the service
# falls back to its own default (arc/auth/app/config.py), which is not this audience.
{ sed -E \
  -e '/^PUBLIC_ORIGIN=/d' \
  -e 's|^DB_PATH=.*|DB_PATH=/data/auth.db|' \
  -e 's|^APPLE_PRIVATE_KEY_FILE=.*|APPLE_PRIVATE_KEY_FILE=/run/secrets/AuthKey.p8|' \
  -e '/^STORE_PREFIX=/d' \
  "$LOCAL_ENV"; echo "PUBLIC_ORIGIN=$ORIGIN"; } \
  | $SSHC "$BOX" "cat > $REMOTE/.env && chmod 600 $REMOTE/.env"

echo "==> Apple signing key"
# Owned by the CONTAINER's user (uid 10001, `kenjin` in arc/auth/Dockerfile), not by
# the login user. The file is bind-mounted read-only into the container, and a
# 600 file owned by ubuntu is simply unreadable to uid 10001 -- which surfaces
# as "Permission denied" at startup, after a successful build and a successful
# mount, which is a slow way to learn about file ownership.
#
# The .env deliberately does NOT get this treatment: --env-file is read by the
# Docker CLI on the host, as the login user, before the container exists.
#
# 400 and not 600: the container only ever reads it, and the login user should
# need sudo to see an Apple signing key.
$SSHC "$BOX" "cat > $REMOTE/AuthKey.p8 \
  && sudo chown 10001:10001 $REMOTE/AuthKey.p8 \
  && sudo chmod 400 $REMOTE/AuthKey.p8" < "$KEY"

echo "==> verifying (names only, no values)"
$SSHC "$BOX" "
  echo '    .env keys:'; sed -n 's/^\([A-Z_][A-Z_0-9]*\)=.*/      \1/p' $REMOTE/.env
  echo -n '    AuthKey.p8: '; sudo stat -c 'uid=%u gid=%g mode=%a size=%s' $REMOTE/AuthKey.p8
  ls -la $REMOTE | sed 's/^/    /'
"
echo "==> done. Secrets are on the box and are NOT part of any deploy."
