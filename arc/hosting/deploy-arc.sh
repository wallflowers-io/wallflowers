#!/usr/bin/env bash
# deploy-arc.sh — cross-build the Arc on this laptop, ship it to kenjin-01.
#
# Supersedes run-arc-local.sh, which served arc.kenjin.cc off the laptop while
# there was no way onto the box. The Arc now runs on kenjin-01 under systemd and
# survives reboots; this is how new code gets there.
#
#   ./deploy-arc.sh            build + ship + restart + verify
#   ./deploy-arc.sh verify     just check what is live
#   ./deploy-arc.sh rollback   each plane's .prev back (a swap: run again to undo), the
#                              manifest from the binaries' own --version, a restart
#
# A REHEARSAL BOX (door-test, K-43): name it, and `setup` gives it production's shape,
# units from hosting/units and /etc/arc/arc.env from hosting/arc.env.rehearsal:
#   ARC_BOX=lima-door-test ARC_SSH_CONFIG=~/.lima/door-test/ssh.config ARC_USER=ralph \
#   ARC_TARGET=aarch64-unknown-linux-gnu ./deploy-arc.sh setup | deploy | verify
#
# THE KIOSK KEYS (A-3): every deploy ships hosting/claim-keys.json to /etc/arc, the list
# the node admits a claim against and the Door checks one against (deploy-door.sh ships the
# same file). ARC_CLAIM_KEYS_FILE names another: a rehearsal's, claim-keys.rehearsal.json,
# is the test vector's key, whose seed is public in core's tests.
#
# THE ENV'S NAMES (deploy.md U5): a deploy refuses, before anything is built, a box whose
# /etc/arc/arc.env leaves unset or empty a name in REQUIRED_ENV. Unset, a plane runs on its
# default or not at all: without ARC_CLAIM_KEYS the node holds no kiosk key and refuses every
# claim (28 Sep). Only names leave the box, never a value; the file is Ralph's (D-51).
#
# WHY CROSS-COMPILE. kenjin-01 is a `small` m0 VM — 1 vCPU, 961Mi, no swap. It
# runs the Arc comfortably (all five planes are ~75MB RSS) but it cannot BUILD it:
# mls-rs plus bundled SQLite would take hours and OOM. So the laptop builds for
# x86_64-linux and ships binaries. No Docker, no registry, no build VM.
#
# HOW. zig is the cross-linker (cargo-zigbuild), targeting glibc 2.39 to match
# Ubuntu 24.04 on the box. All five planes cross-compile clean: `node` took
# pacific-core's `lode` and with it OpenBLAS until O-41 (83c3295), and no longer does.
#
# ── PROVENANCE ───────────────────────────────────────────────────────────────
# A deploy ships the COMMIT, not whatever happens to be in the working tree. The
# same reasoning as site's justfile, which says it first and best; the
# mechanism differs because the Arc ships BINARIES, so there is no image to label
# and no tag to push. The commit is therefore compiled INTO each plane
# (lib/arc-build) and the binary answers for itself:
#
#   over the wire     GET https://arc.wallflowers.io/v1/version
#   from the box      /opt/arc/bin/<plane> --version
#   without running   strings /opt/arc/bin/<plane> | grep '@(#)arc-build'
#   all five at once  /opt/arc/build.json   (survives a restart; see MANIFEST below)
#
# This existed because it was absent. On 18 Sep 2026 the live binaries were dated
# 10 Sep 03:19 and matched NO commit in the arc repo — its log jumps 13 Aug ->
# 16 Sep — so they were a working-tree build from a day nobody recorded, and the
# only way to establish what was running was to probe the wire for behavioural
# tells (it answered a media frame, so it postdated 13 Aug; it had no service
# gate, so it predated 16 Sep). That is not a thing anyone should have to do.
#
# WHAT IS STAMPED. The workspace [patch] in arc/Cargo.toml redirects pacific-core
# and pacific-wire at core/ — roughly half of `node`. Since 18 Sep 2026 core/ is a
# directory of the SAME repository as arc/, so the one SHA names both, and the
# stamp's `core=<sha>[-dirty]` is that SHA judged on core's own build inputs.
#
# CORE DIRT IS REFUSED, like arc dirt, since 19 Sep 2026. It used to be only
# warned about, as a concession to a separate repository that could not yet hold a
# clean tree through a deploy; that concession said it would expire when core
# could, and the monorepo is what made it able to. The way to deploy while other
# work sits uncommitted is a clean worktree of the commit, not ALLOW_DIRTY:
#
#     git worktree add --detach <dir> HEAD && <dir>/arc/hosting/deploy-arc.sh
#
# (a worktree needs core/.cargo/config.toml copied in — it is local and gitignored).
#
# ROLLBACK. The swap keeps each plane's previous binary beside it as <plane>.prev, so
# going back is a `mv` per plane and a restart, never a rebuild of something that may
# match no commit. The manifest is then stale until the next deploy, and says a build
# that is not running — rewrite it or redeploy.
#
# THE ICD PINNED (O-68, mdr/icd-pin.md): a deploy refuses, before anything is built or sent,
# a tree whose ICD is not its pin, whose web op bindings are not its ICD's, or whose pin is
# not the newest icd/* tag's, ALLOW_DIRTY=1 or not (core/coordination/icd-pin.sh). The Arc serves no ICD: its model is the core it is
# built from, so the check is on the tree.
#
# MANIFEST. After the swap, the box is asked what it now holds — each new binary's
# own `--version` plus its sha256 — and that is written to /opt/arc/build.json,
# which the gateway serves under `deploy` at /v1/version. It reports what is ON
# DISK, not what this script meant to put there; a binary that does not answer
# --version is recorded as exactly that rather than skipped.
set -uo pipefail

# cargo and cargo-zigbuild live here and ~/.cargo/bin is not on a non-login
# shell's PATH, which made this script report cargo-zigbuild "missing" when it was installed.
export PATH="$HOME/.cargo/bin:$PATH"

# Derived, not written down: this script lives at arc/hosting/, so the arc is its
# parent and core is the arc's sibling. A literal path here broke the day the
# workspace was renamed, and would break again on the next one.
ARC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# core/, which the workspace [patch] compiles in. Not a copy of a path from
# arc/Cargo.toml for the build to use — only for judging its build inputs.
CORE="$(cd "$ARC/../core" && pwd)"
# PRODUCTION UNLESS NAMED. ARC_BOX and ARC_KEY name another box; with ARC_SSH_CONFIG,
# ARC_BOX is that config's Host alias (a Lima VM). ARC_TARGET and ARC_GLIBC build for it.
# A named box is a rehearsal: verify then asks the box itself and never the public names.
BOX=${ARC_BOX:-ubuntu@167.99.83.136}
KEY=${ARC_KEY:-$HOME/.ssh/machine0_key}
TARGET=${ARC_TARGET:-x86_64-unknown-linux-gnu}
GLIBC=${ARC_GLIBC:-2.39}
PLANES="arc-gateway semaphore node waker boxoffice"
UNITS="arc-relay arc-node arc-boxoffice arc-waker arc-gateway"
MANIFEST=/opt/arc/build.json
CLAIM_KEYS=${ARC_CLAIM_KEYS_FILE:-$ARC/hosting/claim-keys.json}
# Absolute from here on: the deploy runs from $ARC later, where a path given relative to the
# caller names nothing (door-test, 29 Sep: the keys' scp failed after the swap, SCM).
case "$CLAIM_KEYS" in /*) ;; *) CLAIM_KEYS="$PWD/$CLAIM_KEYS" ;; esac
if [ -n "${ARC_SSH_CONFIG:-}" ]; then
  SSHC="ssh -F $ARC_SSH_CONFIG -o BatchMode=yes -o ConnectTimeout=15"
  SCP="scp -q -F $ARC_SSH_CONFIG -o BatchMode=yes"
else
  SSHC="ssh -o BatchMode=yes -o ConnectTimeout=15 -i $KEY"
  SCP="scp -q -o BatchMode=yes -i $KEY"
fi

note() { printf '» %s\n' "$*"; }
die()  { printf '✗ %s\n' "$*" >&2; exit 1; }

# The git paths whose contents BECOME the shipped binaries. Scoped like site's
# _guard-clean, and for the same reason: the guard's job is "is this binary the commit
# it will claim to be", so it covers the build inputs and nothing else. A dirty README
# or .gitignore changes no instruction in the output and must not block a deploy —
# a guard that cries wolf is a guard that gets ALLOW_DIRTY'd by reflex.
BUILD_INPUTS="gateway planes lib Cargo.toml Cargo.lock"

# Is the arc working tree dirty in the paths that become the binaries? Untracked files
# count: a new .rs under planes/ is as much a difference from the commit as an edited one.
arc_dirty() { [ -n "$(cd "$ARC" && git status --porcelain -- $BUILD_INPUTS)" ]; }

# The same question of core/, on the paths of it the arc compiles in (the [patch]
# crates, and the workspace manifest they may inherit from).
CORE_INPUTS="pacific-core pacific-wire Cargo.toml"
core_dirty() { [ -n "$(cd "$CORE" && git status --porcelain -- $CORE_INPUTS)" ]; }

# Refuse to ship a working tree that is not the commit it claims to be.
guard_clean() {
  arc_dirty || core_dirty || return 0
  printf '✗ REFUSING: uncommitted changes in what becomes the binaries\n' >&2
  echo "  A deploy ships a commit, not a working tree -- otherwise nothing on the box" >&2
  echo "  corresponds to anything in history. That is exactly how the 10 Sep binaries" >&2
  echo "  came to exist. Commit, deploy from a clean worktree (see the header), or" >&2
  echo "  ALLOW_DIRTY=1 for a genuine emergency." >&2
  echo >&2
  (cd "$ARC" && git status --short -- $BUILD_INPUTS) >&2
  (cd "$CORE" && git status --short -- $CORE_INPUTS) >&2
  echo >&2
  [ "${ALLOW_DIRTY:-}" = "1" ] || exit 1
  echo "⚠ ALLOW_DIRTY=1 set -- shipping a dirty tree anyway. It will be stamped" >&2
  echo "  dirty:true, and /v1/version will say so for as long as it is deployed." >&2
}

# The names this build needs a value for: each read by a plane, production's value not its
# default. Not ARC_GATEWAY_PORT (no plane reads it); ARC_PUBLIC_URL joins with O-73.
REQUIRED_ENV="ARC_NAME ARC_CLAIM_KEYS PACIFIC_STATE_DIR
  ARC_RELAY_UPSTREAM ARC_MEMBERSHIP_UPSTREAM ARC_BOXOFFICE_UPSTREAM ARC_WAKER_UPSTREAM
  RELAY_BIND RELAY_STORE ARC_RELAY_URL RELAY_MAX_STORE_BYTES RELAY_MAX_BLOB_BYTES
  RELAY_R2_ENDPOINT RELAY_R2_BUCKET RELAY_R2_ACCESS_KEY_ID RELAY_R2_SECRET_ACCESS_KEY RELAY_R2_REGION"
# The names of REQUIRED_ENV the box's arc.env leaves unset or empty. Read on the box by root;
# only names come back. ARC_ENV_PATH only so this can run against a scratch file.
env_unset() {
  local have
  have=$($SSHC "$BOX" "sudo python3 - ${ARC_ENV_PATH:-/etc/arc/arc.env}" <<'PY'
import re, sys
for l in open(sys.argv[1]):
    m = re.match(r"\s*([A-Za-z_]\w*)=(.*)", l)
    if m and m.group(2).strip().strip("\"'"):
        print(m.group(1))
PY
) || return 1
  comm -23 <(printf '%s\n' $REQUIRED_ENV | sort -u) <(printf '%s\n' "$have" | sort -u)
}

# git describe for a checkout that may not exist (a fresh clone with no sibling ../core).
# Optional paths after the dir scope the dirtiness to them (default: the whole dir).
git_stamp() {
  local dir=$1; shift
  local scope=("${@:-.}")
  # Not `[ -d "$dir/.git" ]`: arc and core are directories of one repository, and
  # that test would report both "absent".
  git -C "$dir" rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "absent"; return; }
  local sha
  sha=$(cd "$dir" && git rev-parse --short=12 HEAD 2>/dev/null) || { echo "unknown"; return; }
  if [ -n "$(cd "$dir" && git status --porcelain -- "${scope[@]}" 2>/dev/null)" ]; then echo "$sha-dirty"; else echo "$sha"; fi
}

# Renders GET /v1/version for a human. Kept as a heredoc-emitting function rather than an
# inline `python3 -c '...'` so the program can use quotes freely and stdin stays the curl pipe.
provenance_reader() {
  cat <<'PY'
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print("  /v1/version did not answer with JSON at all — the Arc is unreachable, or")
    print("  something that is not the gateway is answering for it. Check /v1/health above.")
    sys.exit(0)
# An Arc without the route answers the router's 404 envelope, which IS valid JSON — so
# "no gateway block" means no such route, not an unstamped build. Two different facts:
# one is "this Arc predates provenance", the other is "it has provenance and it is bad".
if "gateway" not in d:
    print("  this Arc has no /v1/version (%s)." % (((d.get("error") or {}).get("code")) or "no gateway block"))
    print("  It predates the build stamp — arc commit ac85d89 or earlier. Nothing to read until deployed.")
    sys.exit(0)
g = d.get("gateway") or {}
if not g.get("stamped", False):
    print("  gateway   UNSTAMPED BUILD — the live gateway does not know which commit it is.")
else:
    dirty = "  +DIRTY-TREE" if g.get("dirty") else ""
    print("  gateway   %s (%s)%s" % (g.get("short", "?"), g.get("ref", "?"), dirty))
    print("            core=%s  built=%s" % (g.get("core", "?"), g.get("built", "?")))
dep = d.get("deploy")
if not dep:
    print("  deploy    no manifest — %s" % d.get("deploy_note", "reason not given"))
else:
    print("  deploy    %s at %s" % (dep.get("commit", "?")[:12], dep.get("deployed", "?")))
    for name, pl in sorted((dep.get("planes") or {}).items()):
        st = pl.get("stamp") or {}
        mark = st.get("short") or ("UNSTAMPED" if st.get("stamped") is False else "?")
        note = "  (%s)" % st["note"] if st.get("note") else ""
        print("            %-12s %-14s sha256:%s%s" % (name, mark, (pl.get("sha256") or "?")[:12], note))
    # The one disagreement worth shouting about: binaries swapped, unit never restarted.
    live = g.get("commit")
    on_disk = (((dep.get("planes") or {}).get("arc-gateway") or {}).get("stamp") or {}).get("commit")
    if live and on_disk and live != on_disk:
        print("  !! THE RUNNING GATEWAY IS NOT THE DEPLOYED BINARY —")
        print("     running %s, on disk %s. Restart arc-gateway." % (live[:12], on_disk[:12]))
PY
}

verify() {
  if [ -n "${ARC_BOX:-}" ]; then
    # A rehearsal box has no public name, and a check of arc.wallflowers.io from here
    # would describe production, not it. The box answers for itself.
    echo
    $SSHC "$BOX" "for u in $UNITS; do printf '  %-18s %s\\n' \$u \$(systemctl is-active \$u); done; python3 -c 'import json; d=json.load(open(\"$MANIFEST\")); print(\"  commit\", d[\"commit\"])' 2>/dev/null || echo '  no manifest'"
    echo
    return
  fi
  # A machine that resolved arc.wallflowers.io before the record existed caches the
  # NXDOMAIN and reads 000 on a perfectly healthy Arc. Pin the edge for the check.
  local edge pin
  edge=$(dig +short arc.wallflowers.io @1.1.1.1 2>/dev/null | grep -E '^[0-9.]+$' | head -1)
  pin=${edge:+--resolve arc.wallflowers.io:443:$edge}
  echo
  for p in /v1/health /v1/arc /v1/version /v1/box/health /v1/wake/health; do
    printf '  %-20s %s\n' "$p" "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 $pin "https://arc.wallflowers.io$p")"
  done
  printf '  %-20s %s (101 = upgraded)\n' "/v1/relay (wss)" \
    "$(curl -s -o /dev/null -w '%{http_code}' --http1.1 --max-time 20 $pin \
       -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
       -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
       https://arc.wallflowers.io/v1/relay)"
  printf '  %-20s %s\n' "https://wallflowers.io/" \
    "$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 https://wallflowers.io/)"
  echo
  $SSHC "$BOX" 'for u in arc-relay arc-node arc-boxoffice arc-waker arc-gateway cloudflared-arc; do printf "  %-18s %s\n" "$u" "$(systemctl is-active $u)"; done' 2>/dev/null

  # WHAT IS DEPLOYED. Health says the planes answer; this says WHICH BUILD answers.
  # Read over the wire, from the gateway that is actually serving, because that is the
  # question an operator has at 2am and the box may not be reachable.
  echo
  echo "  ── provenance ──"
  curl -s --max-time 20 $pin "https://arc.wallflowers.io/v1/version" | python3 -c "$(provenance_reader)"
  echo
}

setup() {
  # A NAMED BOX ONLY. Production's units and env exist, and they are Ralph's (D-51).
  [ -n "${ARC_BOX:-}" ] || die "setup is for a named box (ARC_BOX): production's units are not this script's to write"
  local user=${ARC_USER:-ubuntu} envf=${ARC_ENV_FILE:-$ARC/hosting/arc.env.rehearsal}
  note "setting up $BOX: /opt/arc, /etc/arc/arc.env, the five units as $user"
  for u in $UNITS; do
    sed "s|__ARC_USER__|$user|" "$ARC/hosting/units/$u.service" > "/tmp/arc-$u.service"
    $SCP "/tmp/arc-$u.service" "$BOX:/tmp/$u.service" || die "scp $u"
  done
  $SCP "$envf" "$BOX:/tmp/arc.env" || die "scp env"
  # The env file is the operator's once it exists: never overwritten.
  $SSHC "$BOX" "set -e
    sudo mkdir -p /opt/arc/bin /opt/arc/state /etc/arc
    # The deploy writes /opt/arc/bin and /opt/arc/build.json as the ssh user, without sudo.
    sudo chown $user /opt/arc && sudo chown -R $user /opt/arc/bin /opt/arc/state
    [ -f /etc/arc/arc.env ] || sudo install -m 0640 -g \$(id -gn $user) /tmp/arc.env /etc/arc/arc.env
    for u in $UNITS; do sudo install -m 0644 /tmp/\$u.service /etc/systemd/system/\$u.service; done
    sudo systemctl daemon-reload
    sudo systemctl enable $UNITS >/dev/null 2>&1" || die "setup failed"
  note "the units start at the first deploy, with the binaries"
}

# Record what the box now HOLDS, by asking each binary rather than asserting it. Written
# atomically (.new + mv) for the same reason the binaries are: the gateway reads this file
# per request, so it must never observe a half-written one.
# Parameterised only so this block can be exercised against a scratch dir without
# going near /opt/arc; the default IS production and the deploy never overrides it.
# $1 the commit and $2 its ref, or both empty for "read them from the binaries".
write_manifest() {
  note "recording the manifest at $MANIFEST"
  $SSHC "$BOX" "ARC_SHA='$1' ARC_REF='$2' ARC_BY='hosting/deploy-arc.sh${3:+ $3}' ARC_MANIFEST='$MANIFEST' bash -s $PLANES" <<'REMOTE' || die "manifest failed"
set -uo pipefail
# Parameterised only so this block can be exercised against a scratch dir without
# going near /opt/arc; the default IS production and the deploy never overrides it.
cd "${ARC_BINDIR:-/opt/arc/bin}" || exit 1
# A rollback names no commit: the binaries now in place say which they are (NC-52).
if [ -z "$ARC_SHA" ]; then
  eval "$(timeout 5 ./arc-gateway --version </dev/null 2>/dev/null | python3 -c 'import json,sys,shlex; s=json.load(sys.stdin); print("ARC_SHA=%s ARC_REF=%s" % (shlex.quote(s["commit"]), shlex.quote(s["ref"])))')"
  [ -n "$ARC_SHA" ] || { echo "arc-gateway does not answer --version" >&2; exit 1; }
fi
{
  printf '{\n'
  printf '  "commit": "%s",\n'   "$ARC_SHA"
  printf '  "ref": "%s",\n'      "$ARC_REF"
  printf '  "deployed": "%s",\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf '  "by": "%s",\n'       "$ARC_BY"
  printf '  "host": "%s",\n'     "$(hostname)"
  printf '  "planes": {\n'
  sep=""
  for b in "$@"; do
    # The binary's OWN word. A binary that cannot answer is recorded as such, never
    # skipped: a short list reads as "nothing else is here", which would be a lie.
    stamp=$(timeout 5 "./$b" --version </dev/null 2>/dev/null)
    case "$stamp" in
      \{*) : ;;
      *) stamp='{"stamped": false, "note": "does not answer --version — predates lib/arc-build, or is not an Arc plane"}' ;;
    esac
    printf '%b    "%s": { "stamp": %s, "sha256": "%s", "mtime": "%s" }' \
      "$sep" "$b" "$stamp" \
      "$(sha256sum "$b" | cut -d' ' -f1)" \
      "$(date -u -r "$b" +%Y-%m-%dT%H:%M:%SZ)"
    sep=",\n"
  done
  printf '\n  }\n}\n'
} > "$ARC_MANIFEST.new" 2>/dev/null
python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$ARC_MANIFEST.new" \
  || { echo "manifest is not valid JSON — leaving the old one in place" >&2; rm -f "$ARC_MANIFEST.new"; exit 1; }
mv "$ARC_MANIFEST.new" "$ARC_MANIFEST"
REMOTE
}

# K-48: a full arc set before the binaries change, the planes still running: sqlite's backup()
# is consistent under them, and the seals' rule asks only that arc and auth come first
# (backup-arc.service, arc/hosting/backup). A box without the unit goes on as before and
# says so; a backup that fails is said, and the deploy goes on.
BACKUP_FIRST='if systemctl cat backup-arc.service >/dev/null 2>&1; then
    sudo install -d -m 0700 /var/lib/backup-arc && sudo touch /var/lib/backup-arc/full.next &&
      sudo systemctl start backup-arc.service || echo "deploy-arc: the backup before this change failed (journalctl -u backup-arc); it goes on" >&2
  else
    echo "deploy-arc: backup-arc.service is not installed: no backup before this change" >&2
  fi'
backup_first() { $SSHC "$BOX" "$BACKUP_FIRST" || note "the backup step could not be reached on $BOX; going on"; }

rollback() {
  # A SWAP per plane, so a rollback can itself be undone, then the manifest from what
  # the binaries now say, then the restart (NC-52).
  backup_first
  $SSHC "$BOX" "set -e; cd /opt/arc/bin
    for b in $PLANES; do test -f \$b.prev || { echo \"no previous \$b\" >&2; exit 1; }; done
    for b in $PLANES; do mv \$b \$b.swap && mv \$b.prev \$b && mv \$b.swap \$b.prev; done" || die "rollback failed"
  write_manifest "" "" rollback
  $SSHC "$BOX" "sudo systemctl restart $UNITS" || die "restart failed"
  sleep 8
  verify
}

[ "${1:-deploy}" = "verify" ] && { verify; exit 0; }
[ "${1:-deploy}" = "setup" ] && { setup; exit 0; }
[ "${1:-deploy}" = "rollback" ] && { rollback; exit 0; }

# THE PIN FIRST (O-68): before the dirt guard, so ALLOW_DIRTY=1 cannot pass it.
"$CORE/coordination/icd-pin.sh" deploy || die "REFUSING: the ICD is not the pinned model; nothing built, nothing uploaded"
# PROVENANCE GATE — before any toolchain check: a dirty tree is refused whether or
# not this laptop could have built it.
guard_clean
# {kid: base64url key}, at least one, each kid its own key's, or the node would not start.
python3 - "$CLAIM_KEYS" <<'PY' || die "$CLAIM_KEYS: not a non-empty {kid: key} with each kid its key's; nothing built, nothing uploaded"
import base64, hashlib, json, sys
k = json.load(open(sys.argv[1]))
assert isinstance(k, dict) and k
assert all(hashlib.sha256(base64.urlsafe_b64decode(v + "=" * (-len(v) % 4))).hexdigest()[:16] == i for i, v in k.items())
PY
# THE ENV'S NAMES, before the build: ALLOW_DIRTY=1 does not pass it.
unset_names=$(env_unset) || die "REFUSING: $BOX: /etc/arc/arc.env's names could not be read; nothing built, nothing uploaded"
[ -z "$unset_names" ] || die "REFUSING: $BOX: /etc/arc/arc.env sets no value for $(echo $unset_names); nothing built, nothing uploaded"

command -v cargo-zigbuild >/dev/null || die "cargo-zigbuild missing — brew install zig && cargo install cargo-zigbuild"

cd "$ARC" || die "no $ARC"

# The stamp. Computed from git BEFORE the Cargo.toml edit below, so the manifest sed
# cannot be what makes the tree look dirty. lib/arc-build's build.rs declares each of
# these rerun-if-env-changed, so a new SHA rebuilds the stamp rather than being served
# from cargo's cache — a cached stamp is last week's SHA on this week's binary, which
# is worse than no stamp because it looks right.
export ARC_BUILD_SHA=$(git rev-parse HEAD)
export ARC_BUILD_REF=$(git rev-parse --abbrev-ref HEAD)
export ARC_BUILD_DIRTY=$( (arc_dirty || core_dirty) && echo 1 || echo 0)
export ARC_BUILD_CORE=$(git_stamp "$CORE" $CORE_INPUTS)
export ARC_BUILD_TIME=$(date -u +%Y-%m-%dT%H:%M:%SZ)
note "stamping ${ARC_BUILD_SHA:0:12} ($ARC_BUILD_REF) dirty=$ARC_BUILD_DIRTY core=$ARC_BUILD_CORE"

# console/traffic is not an image plane and its arc-resolver path dep points into
# the pacific monorepo, which is outside this tree. The Dockerfile drops it the
# same way. Restore on EVERY exit so an interrupted build cannot leave the
# manifest edited.
cp Cargo.toml /tmp/arc-Cargo.toml.deploy.bak
trap 'cp /tmp/arc-Cargo.toml.deploy.bak "$ARC/Cargo.toml"' EXIT
sed -i '' '/"console\/traffic"/d' Cargo.toml

note "cross-building for $TARGET (glibc $GLIBC)"
cargo zigbuild --release --offline \
  --target "$TARGET.$GLIBC" -p arc-gateway -p relay -p waker -p boxoffice -p node \
  || die "build failed"

OUT="${CARGO_TARGET_DIR:-target}/$TARGET/release"
for b in $PLANES; do [ -f "$OUT/$b" ] || die "missing binary: $b"; done

# The stamp has to have LANDED, not merely been exported. A cargo cache hit that
# silently reused an unstamped object is the exact failure this whole change exists
# to make impossible, so prove it on the artefacts before they leave the laptop.
for b in $PLANES; do
  LC_ALL=C grep -aq "@(#)arc-build commit=$ARC_BUILD_SHA" "$OUT/$b" \
    || die "$b carries no stamp for $ARC_BUILD_SHA — refusing to ship an unprovenanced binary.
       Try: cargo clean -p arc-build && rerun."
done
note "all $(echo $PLANES | wc -w | tr -d ' ') binaries carry ${ARC_BUILD_SHA:0:12}"

note "shipping to $BOX"
# Ship beside the live ones, then swap: a half-finished scp must never be what
# systemd restarts into.
for b in $PLANES; do
  $SCP "$OUT/$b" "$BOX:/opt/arc/bin/$b.new" || die "scp $b failed"
done
# The keys before the swap: a failure here leaves the live binaries as they were, and a failure
# between the swap and the restart would leave new binaries on disk under old processes. The
# running node reads its keys only at start, so installing them first changes nothing live.
$SCP "$CLAIM_KEYS" "$BOX:/tmp/claim-keys.json" || die "scp claim keys; nothing swapped"
$SSHC "$BOX" "sudo install -m 0644 /tmp/claim-keys.json /etc/arc/claim-keys.json" || die "claim keys; nothing swapped"
backup_first
$SSHC "$BOX" "cd /opt/arc/bin && for b in $PLANES; do if [ -f \$b ]; then cp -p \$b \$b.prev; fi; mv \$b.new \$b && chmod +x \$b; done" \
  || die "swap failed"

write_manifest "$ARC_BUILD_SHA" "$ARC_BUILD_REF"

$SSHC "$BOX" "sudo systemctl restart $UNITS" || die "restart failed"

sleep 8
verify
