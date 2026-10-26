#!/usr/bin/env bash
# deploy-auth.sh — the account store (arc/auth) to the Arc's box: the commit's tree, built
# into the kenjin-auth image there and swapped behind a health gate (deploy-auth-remote.sh).
#
#   ./deploy-auth.sh            ship, build, swap, verify
#   ./deploy-auth.sh verify     just check what is live
#   ./deploy-auth.sh changed    exit 0 when arc/auth differs from what is live (or live is
#                               not a commit here), 1 when it does not
#
# AN AUTH RESTART CUTS DOOR SIGN-INS for its seconds (K-49), so `make deploy-arc` ships it
# only when `changed` says so. The secrets stay on the box (~/kenjin-auth-secrets, placed
# once by push-auth-secrets.sh); a deploy never carries one.
#
# ARC_BOX and ARC_KEY as deploy-arc.sh's.
set -euo pipefail
ARC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="$(cd "$ARC/.." && pwd)"
BOX=${ARC_BOX:-ubuntu@167.99.83.136}
KEY=${ARC_KEY:-$HOME/.ssh/machine0_key}
SSHC="ssh -o BatchMode=yes -o ConnectTimeout=15 -i $KEY"
HEALTH=https://arc.wallflowers.io/auth/health

note() { printf '» %s\n' "$*"; }
die()  { printf '✗ %s\n' "$*" >&2; exit 1; }

live() { $SSHC "$BOX" "docker inspect kenjin-auth --format '{{index .Config.Labels \"org.opencontainers.image.revision\"}}'" 2>/dev/null; }

verify() {
  printf '  %-16s %s\n' revision "$(live || echo none)"
  printf '  %-16s %s\n' health "$(curl -fsS --max-time 20 "$HEALTH" || echo unreachable)"
}

case "${1:-deploy}" in
  verify) verify; exit 0 ;;
  changed)
    rev=$(live) || exit 0
    git -C "$REPO" rev-parse -q --verify "$rev^{commit}" >/dev/null || exit 0
    git -C "$REPO" diff --quiet "$rev" HEAD -- arc/auth && exit 1 || exit 0 ;;
  deploy) ;;
  *) die "deploy | verify | changed" ;;
esac

# A DEPLOY SHIPS A COMMIT: what goes is `git archive`, so nothing uncommitted can.
[ -z "$(git -C "$REPO" status --porcelain -- arc/auth)" ] || [ "${ALLOW_DIRTY:-}" = 1 ] \
  || die "REFUSING: uncommitted changes under arc/auth; they would not ship, and the label would claim they did"
SHA=$(git -C "$REPO" rev-parse --short=7 HEAD)
note "shipping arc/auth at $SHA to $BOX"
git -C "$REPO" archive HEAD:arc/auth | $SSHC "$BOX" 'set -e
  rm -rf ~/kenjin-auth.new && mkdir ~/kenjin-auth.new && tar -x -C ~/kenjin-auth.new
  rm -rf ~/kenjin-auth && mv ~/kenjin-auth.new ~/kenjin-auth' || die "ship failed"
$SSHC "$BOX" "DEPLOY_SHA='$SHA' bash -s" < "$ARC/hosting/deploy-auth-remote.sh" || die "swap failed"
verify
