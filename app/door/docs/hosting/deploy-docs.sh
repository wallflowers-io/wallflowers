#!/usr/bin/env bash
# deploy-docs.sh — docs.wallflowers.io: app/door/docs/dist at a commit, to its own container
# on the Arc's box (nginx.conf beside this file says how it is served).
#
#   ./deploy-docs.sh            check, ship, swap, verify
#   ./deploy-docs.sh verify     just check what is live
#   ./deploy-docs.sh rollback   the previous release back (a swap: run again to undo)
#
# A DEPLOY SHIPS A COMMIT: what goes is `git archive`, and dist/ must be what the sources
# build (build.mjs --check), so the pages served are the pages the commit describes.
#
# ON THE BOX, ~/wallflowers-docs: releases/<commit>/ (docs/, hack-seoul.html), `current` and
# `previous` naming two of them, conf/ the nginx.conf live. The container, wallflowers-docs,
# serves it read-only on 127.0.0.1:8082; the `wallflowers` tunnel sends docs.wallflowers.io
# there. It is made, or remade on a new IMAGE, by the deploy itself.
#
# ARC_BOX and ARC_KEY as deploy-arc.sh's: the docs share the Arc's box.
set -euo pipefail
HOSTING="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HOSTING/../../../.." && pwd)"
BOX=${ARC_BOX:-ubuntu@167.99.83.136}
KEY=${ARC_KEY:-$HOME/.ssh/machine0_key}
SSHC="ssh -o BatchMode=yes -o ConnectTimeout=15 -i $KEY"
# Pinned by digest (SEC-24): the stable line, at least two weeks old when chosen.
IMAGE=nginx:1.30.5-alpine@sha256:0985e772fb9f729e6fa0980da05fca5d9c468e870eed43071545afa9d2e27d94
KEEP=5

note() { printf '» %s\n' "$*"; }
die()  { printf '✗ %s\n' "$*" >&2; exit 1; }

# Every file of dist/ at the live release, read through the container, against this commit's.
verify() {
  local want got
  want=$(git -C "$REPO" ls-tree -r HEAD:app/door/docs/dist | awk '{print $3}' | while read -r b; do git -C "$REPO" cat-file blob "$b" | shasum -a 256 | cut -c1-64; done | sort | shasum -a 256 | cut -c1-16)
  got=$($SSHC "$BOX" 'cd ~/wallflowers-docs/current/docs && find . -type f | sort | while read -r f; do sha256sum "$f" | cut -c1-64; done | sort | sha256sum | cut -c1-16' 2>/dev/null || echo none)
  printf '  %-16s %s\n' release "$($SSHC "$BOX" 'basename "$(readlink ~/wallflowers-docs/current)"' 2>/dev/null || echo none)"
  printf '  %-16s %s\n' container "$($SSHC "$BOX" 'docker inspect -f "{{.State.Status}}" wallflowers-docs' 2>/dev/null || echo none)"
  printf '  %-16s %s\n' served "$($SSHC "$BOX" 'curl -s -o /dev/null -w "%{http_code}" --max-time 10 http://127.0.0.1:8082/' 2>/dev/null || echo none)"
  printf '  %-16s %s\n' public "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 https://docs.wallflowers.io/)"
  printf '  %-16s %s\n' csp "$($SSHC "$BOX" 'curl -sI --max-time 10 http://127.0.0.1:8082/ | tr -d "\r" | grep -i "^content-security-policy" | cut -d" " -f2-' 2>/dev/null)"
  printf '  %-16s %s\n' files "$([ "$want" = "$got" ] && echo "this commit's dist/" || echo "NOT this commit's dist/ ($got, want $want)")"
  [ "$want" = "$got" ]
}

case "${1:-deploy}" in
  verify) verify; exit $? ;;
  rollback)
    $SSHC "$BOX" 'set -e; cd ~/wallflowers-docs
      p=$(readlink previous) || { echo "no previous release" >&2; exit 1; }
      c=$(readlink current)
      ln -sfn "$p" current.new && mv -T current.new current && ln -sfn "$c" previous
      cp "$p/nginx.conf" conf/default.conf && docker exec wallflowers-docs nginx -t -q && docker exec wallflowers-docs nginx -s reload' || die "rollback failed"
    $SSHC "$BOX" 'basename "$(readlink ~/wallflowers-docs/current)"'; exit 0 ;;
  deploy) ;;
  *) die "deploy | verify | rollback" ;;
esac

(cd "$REPO" && node app/door/docs/build.mjs --check) || die "REFUSING: dist/ is not what the sources build"
[ -z "$(git -C "$REPO" status --porcelain -- app/door/docs)" ] || die "REFUSING: uncommitted changes under app/door/docs; they would not ship"
SHA=$(git -C "$REPO" rev-parse HEAD)
note "shipping docs at ${SHA:0:12} to $BOX"
# dist/ as docs/, and this directory's two files, in one archive.
TMP=$(mktemp -d "${TMPDIR:-/tmp}/docs.XXXXXX"); trap 'rm -rf "$TMP"' EXIT
git -C "$REPO" archive --prefix=docs/ HEAD:app/door/docs/dist | tar -x -C "$TMP"
git -C "$REPO" show "HEAD:app/door/docs/hosting/hack-seoul.html" > "$TMP/hack-seoul.html"
git -C "$REPO" show "HEAD:app/door/docs/hosting/nginx.conf" > "$TMP/nginx.conf"
COPYFILE_DISABLE=1 tar --no-xattrs -C "$TMP" -czf - docs hack-seoul.html nginx.conf | $SSHC "$BOX" "set -e
  mkdir -p ~/wallflowers-docs/releases ~/wallflowers-docs/conf && cd ~/wallflowers-docs
  rm -rf releases/$SHA.new && mkdir releases/$SHA.new && tar -xz -C releases/$SHA.new
  rm -rf releases/$SHA && mv releases/$SHA.new releases/$SHA" || die "ship failed"
$SSHC "$BOX" "set -e; cd ~/wallflowers-docs
  c=\$(readlink current || true)
  ln -sfn releases/$SHA current.new && mv -T current.new current
  [ -z \"\$c\" ] || [ \"\$c\" = releases/$SHA ] || ln -sfn \"\$c\" previous
  cp releases/$SHA/nginx.conf conf/default.conf
  if [ \"\$(docker inspect -f '{{.Config.Image}}' wallflowers-docs 2>/dev/null)\" != '$IMAGE' ]; then
    docker rm -f wallflowers-docs >/dev/null 2>&1 || true
    docker run -d --name wallflowers-docs --restart unless-stopped \\
      -p 127.0.0.1:8082:8080 --read-only --tmpfs /var/cache/nginx --tmpfs /var/run --cap-drop ALL \\
      --cap-add CHOWN --cap-add SETUID --cap-add SETGID \\
      -v ~/wallflowers-docs:/srv:ro -v ~/wallflowers-docs/conf:/etc/nginx/conf.d:ro '$IMAGE' >/dev/null
  else
    docker exec wallflowers-docs nginx -t -q && docker exec wallflowers-docs nginx -s reload
  fi
  # The newest $KEEP releases kept, and whatever current and previous name.
  keep=\"\$(readlink current) \$(readlink previous 2>/dev/null || true)\"
  ls -1t releases | tail -n +$((KEEP + 1)) | while read -r r; do case \" \$keep \" in *\" releases/\$r \"*) ;; *) rm -rf \"releases/\$r\" ;; esac; done" \
  || die "swap failed"
sleep 2
verify
