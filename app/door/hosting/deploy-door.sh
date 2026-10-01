#!/usr/bin/env bash
# deploy-door.sh — cross-build the Door on this laptop and ship it to a Linux host
# under systemd, behind Caddy for TLS. deploy-arc.sh's pattern (mdr/door.md §8).
#
#   DOOR_BOX=user@host DOOR_KEY=~/.ssh/key DOOR_HOST=app.wallflowers.io ./deploy-door.sh
#   ... ./deploy-door.sh verify          just check what is live
#   ... ./deploy-door.sh rollback        the previous binary and webapp back (a swap: run
#                                        again to undo), COMMIT from the binary's stamp
#   ... ./deploy-door.sh stage           the webapp as it would ship, in a temp dir it names,
#                                        and its pinned names; nothing sent
#   ... ./deploy-door.sh prune <commit|name>...
#                                        pinned versions removed at once (a security fix), from
#                                        the live webapp and the previous; says what went
#   DOOR_CLAIM_KEYS_FILE=<file>         the kiosk keys /join checks (A-3); default the Arc
#                                        node's own, arc/hosting/claim-keys.json
#   DOOR_CLIENTS_FILE=<file> ... ./deploy-door.sh clients
#                                        the client registrations only (D-32's stand-in),
#                                        and a restart: every session ends head-first (NC-47)
#
# A host reached through an ssh config (a Lima VM): DOOR_SSH_CONFIG=<file> and DOOR_BOX=<its
# Host alias>; DOOR_KEY is then not needed.
#
# THE HOST IS D-30's, so nothing here names one. DOOR_TARGET picks the arch
# (x86_64-unknown-linux-gnu by default; aarch64-unknown-linux-gnu for an arm VM) and
# DOOR_GLIBC the glibc the host has. DOOR_TLS=internal gives a rehearsal host with no
# public name Caddy's own certificate authority instead of a real certificate.
#
# DOOR_ASSETS_COMMIT is the website commit /assets are baked from (D-54; default the
# website's HEAD). DOOR_EGRESS the prefixes the Door may reach besides localhost
# (default Cloudflare's published ranges, "none" for a rehearsal on loopback).
# DOOR_MEMORY_MAX sizes door.service's MemoryMax (default 75% of the host, until D-30);
# the Door fits its session caps under it (NC-48).
#
# FIRST RUN on a host: the `door` user, /opt/door, the two units (door, door-edge), Caddy
# and /opt/door/door.env (from door.env.example, edited for the host) are set up by
# `./deploy-door.sh setup`. The env file is the operator's: this script never overwrites
# it. The edge is the Door's own Caddy: /etc/caddy and a host's caddy.service are not touched.
#
# A DEPLOY SHIPS A COMMIT. As deploy-arc.sh says it: uncommitted changes in what becomes
# the binary are refused; deploy from a clean worktree of the commit.
#
# THE ICD PINNED (O-68, mdr/icd-pin.md): a deploy refuses, before anything is built or sent,
# a tree whose ICD is not its pin, whose web op bindings are not its ICD's, or whose pin is
# not the newest icd/* tag's, ALLOW_DIRTY=1 or not (core/coordination/icd-pin.sh); verify fails when the Door's /v2/icd is not the pin.
set -uo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

# Derived, not written down: this script lives at app/door/hosting/.
DOOR_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRODUCT="$(cd "$DOOR_DIR/../.." && pwd)"
BOX=${DOOR_BOX:?DOOR_BOX=user@host}
HOST=${DOOR_HOST:?DOOR_HOST=the name the Door answers to}
TARGET=${DOOR_TARGET:-x86_64-unknown-linux-gnu}
# The client registrations shipped: D-32's production stand-in unless a rehearsal
# names another.
CLIENTS=${DOOR_CLIENTS_FILE:-$DOOR_DIR/clients.stand-in.json}
# The kiosk keys a claim is checked against at /join (A-3): one file for the Door and the
# Arc's node, so the two lists cannot drift (Software Security).
CLAIM_KEYS=${DOOR_CLAIM_KEYS_FILE:-$PRODUCT/arc/hosting/claim-keys.json}
GLIBC=${DOOR_GLIBC:-2.39}
if [ -n "${DOOR_SSH_CONFIG:-}" ]; then
  SSHC="ssh -F $DOOR_SSH_CONFIG -o BatchMode=yes -o ConnectTimeout=15"
  SCP="scp -q -F $DOOR_SSH_CONFIG -o BatchMode=yes"
else
  KEY=${DOOR_KEY:?DOOR_KEY=path to the ssh key}
  SSHC="ssh -o BatchMode=yes -o ConnectTimeout=15 -i $KEY"
  SCP="scp -q -o BatchMode=yes -i $KEY"
fi

note() { printf '» %s\n' "$*"; }
die()  { printf '✗ %s\n' "$*" >&2; exit 1; }
ICD_PIN=$PRODUCT/core/coordination/icd-pin.sh

# {kid: base64url key}, each kid its own key's, or the Door would not start.
keys_ok() {
  python3 - "$CLAIM_KEYS" <<'PY' || die "$CLAIM_KEYS: not {kid: key} with each kid its key's"
import base64, hashlib, json, sys
k = json.load(open(sys.argv[1]))
assert all(hashlib.sha256(base64.urlsafe_b64decode(v + "=" * (-len(v) % 4))).hexdigest()[:16] == i for i, v in k.items())
PY
}

# ── /assets (D-54, HV-8) ─────────────────────────────────────────────────────
# The webapp's /assets are the website's, baked into the deploy from one committed website
# commit and served by the Door with the rest of the webapp: nothing is fetched at run
# time. What ships is what index.html loads, and what its stylesheets load (url(),
# @import); its scripts load nothing. They run as the Door's origin, beside the PRF.
# The website sits beside this repository's main checkout, wherever the worktree is.
WEBSITE="$(cd "$(git -C "$PRODUCT" rev-parse --path-format=absolute --git-common-dir)/../../business/website" 2>/dev/null && pwd)"
# DOOR_ASSETS_COMMIT, else the website's HEAD.
assets_commit() {
  [ -d "$WEBSITE/.git" ] || die "no website checkout beside $PRODUCT: /assets cannot be baked"
  git -C "$WEBSITE" rev-parse -q --verify "${DOOR_ASSETS_COMMIT:-HEAD}^{commit}" \
    || die "website commit ${DOOR_ASSETS_COMMIT:-HEAD} is not in $WEBSITE: fetch it"
}
# The Caddyfile for this host.
render_caddyfile() {
  local tls="" global=""
  [ "${DOOR_TLS:-}" = "internal" ] && { tls="tls internal"; global="skip_install_trust"; }
  sed -e "/^#/!s|__DOOR_HOST__|$HOST|" -e "/^#/!s|__TLS__|$tls|" -e "/^#/!s|__GLOBAL__|$global|" \
    "$DOOR_DIR/hosting/Caddyfile.template" > /tmp/door-Caddyfile || die "render Caddyfile"
}
# The webapp as shipped, in $WORK/webapp and $WORK/webapp.tgz: its own files, and under
# assets/ what they load from website commit $1, each script and stylesheet index.html
# names with its sha384. A directory per run, so a dry run never touches a deploy's.
#
# WHAT webapp.js LOADS ITSELF (2.2.2's Face editor, when first wanted) is baked as what
# index.html names: its quoted "assets/…" literals, comments aside. Only webapp.js is read so;
# a baked script's own literals are not followed.
#
# PINNED NAMES, for a site that mounts the webapp (Egregore's Echoes): webapp.js,
# webapp.css when there is one, and LIVERY are also staged as <name>.<16 hex of their
# sha384>.<ext>, listed with their SRI in $WORK/pins.list. index.html loads webapp.js and
# webapp.css by those names. The plain names stay: a page cached before this deploy still
# asks for them, and they are not served immutable, so they may change.
stage_webapp() {
  local tmp=${TMPDIR:-/tmp} f follow
  WORK=$(mktemp -d "${tmp%/}/door-webapp.XXXXXX") || die "stage"
  local stage=$WORK/webapp
  # THE WEBAPP'S OWN FILES (NC-56): WEBAPP_KEEP, and what the pages name, followed from
  # index.html's src and href, webapp.js's quoted relative paths and a followed stylesheet's
  # url() and @import. One named but not held, or outside the webapp, stops the stage.
  python3 - "$WEBAPP_SRC" > "$WORK/webapp.list" <<'PY' || die "stage webapp: the pages name what it does not hold"
import os, posixpath, re, sys
src = sys.argv[1]
def inside(p, by):
    q = posixpath.normpath(p)
    if q == ".." or q.startswith("../") or q.startswith("/"): sys.exit(f"{p} (named by {by}): outside the webapp")
    return q
foreign = re.compile(r"(assets/|/|[A-Za-z][A-Za-z0-9+.-]*:)")
page = open(os.path.join(src, "index.html")).read()
js = open(os.path.join(src, "webapp.js")).read()
js = re.sub(r"(?m)^\s*//.*$", "", re.sub(r"/\*.*?\*/", "", js, flags=re.S))
todo = [(u, "index.html") for u in re.findall(r'(?:src|href)="([^"#?]+)', page)]
todo += [(u, "webapp.js") for u in re.findall(r"""['"]([A-Za-z0-9_.][A-Za-z0-9_./-]*\.(?:m?js|css|svg|png|webp|jpe?g|gif|woff2?|json|html|pdf))['"]""", js)]
done = []
while todo:
    u, by = todo.pop(0)
    if foreign.match(u): continue
    p = inside(u, by)
    if p in done: continue
    if not os.path.isfile(os.path.join(src, p)): sys.exit(f"{p} (named by {by}): not in the webapp")
    done.append(p)
    if p.endswith(".css"):
        for m in re.finditer(r'url\(\s*["\']?([^"\')\s]+)|@import\s+["\']([^"\']+)', open(os.path.join(src, p)).read()):
            v = re.split(r"[?#]", m.group(1) or m.group(2))[0]
            if not foreign.match(v): todo.append((posixpath.join(posixpath.dirname(p), v), p))
print("\n".join(done))
PY
  follow=$(cat "$WORK/webapp.list")
  mkdir "$stage" && (cd "$WEBAPP_SRC" && cp -R $WEBAPP_KEEP "$stage/") || die "stage webapp"
  for f in $follow; do
    [ -e "$stage/$f" ] && continue
    mkdir -p "$stage/$(dirname "$f")" && cp "$WEBAPP_SRC/$f" "$stage/$f" || die "stage webapp: $f"
  done
  python3 - "$stage" "$WEBSITE" "$1" "$LIVERY" "$WITH_FACE" "$WORK/pins.list" > "$WORK/assets.list" <<'PY' || die "bake /assets"
import base64, hashlib, os, posixpath, re, subprocess, sys
stage, web, commit, livery, with_face, pins_out = sys.argv[1:]
livery = livery.split()
page = os.path.join(stage, "index.html")
s = open(page).read()
js = open(os.path.join(stage, "webapp.js")).read()
js = re.sub(r"(?m)^\s*//.*$", "", re.sub(r"/\*.*?\*/", "", js, flags=re.S))
done = set()
def bake(todo):
    while todo:
        p = posixpath.normpath(todo.pop())
        if p in done: continue
        if not p.startswith("assets/"): sys.exit(f"{p}: outside /assets")
        r = subprocess.run(["git", "-C", web, "cat-file", "blob", f"{commit}:site/{p}"], capture_output=True)
        if r.returncode: sys.exit(f"{p}: not in website {commit[:12]}")
        os.makedirs(os.path.join(stage, posixpath.dirname(p)), exist_ok=True)
        open(os.path.join(stage, p), "wb").write(r.stdout)
        done.add(p)
        if p.endswith(".css"):
            for m in re.finditer(r'url\(\s*["\']?([^"\')\s]+)|@import\s+["\']([^"\']+)', r.stdout.decode()):
                u = re.split(r"[?#]", m.group(1) or m.group(2))[0]
                if re.match(r"[A-Za-z][A-Za-z0-9+.-]*:", u): continue
                todo.append(u[1:] if u.startswith("/") else posixpath.join(posixpath.dirname(p), u))
bake(re.findall(r'(?:src|href)="(assets/[^"?#]+)', s) + livery + re.findall(r"""['"](assets/[^'"?#]+)['"]""", js))
if "assets/face/face.js" in done:
    for d in with_face.split():
        ls = subprocess.run(["git", "-C", web, "ls-tree", "-r", "--name-only", commit, f"site/{d}/"], capture_output=True, text=True)
        files = [f[len("site/"):] for f in ls.stdout.split()]
        if ls.returncode or not files: sys.exit(f"{d}/: not in website {commit[:12]}")
        bake(files)
sri = lambda p: "sha384-" + base64.b64encode(hashlib.sha384(open(os.path.join(stage, p), "rb").read()).digest()).decode()
s = re.sub(r'((?:src|href)="(assets/[^"?#]+\.(?:js|css))[^"]*")', lambda m: f'{m.group(1)} integrity="{sri(m.group(2))}"', s)
pins = []
for p in [f for f in ("webapp.js", "webapp.css") if os.path.exists(os.path.join(stage, f))] + livery:
    body = open(os.path.join(stage, p), "rb").read()
    stem, ext = posixpath.splitext(p)
    q = f"{stem}.{hashlib.sha384(body).hexdigest()[:16]}{ext}"
    open(os.path.join(stage, q), "wb").write(body)
    pins.append((p, q, sri(q)))
    if "/" not in p:
        s, n = re.subn(r'((?:src|href)=")' + re.escape(p) + r'(?:[?#][^"]*)?"', lambda m: f'{m.group(1)}{q}" integrity="{sri(q)}"', s)
        if not n and p == "webapp.js": sys.exit("index.html does not load webapp.js")
open(pins_out, "w").write("".join(f"{p} {q} {i}\n" for p, q, i in pins))
open(page, "w").write(s)
print("\n".join(sorted(done)))
PY
  local p miss=""
  for p in $(grep -oE '(src|href)="assets/[^"?#]+' "$stage/index.html" | cut -d'"' -f2); do
    [ -f "$stage/$p" ] || miss="$miss $p"
  done
  [ -z "$miss" ] || die "index.html loads what is not staged:$miss"
  [ -z "$(git -C "$WEBSITE" status --porcelain -- $(sed 's|^|site/|' "$WORK/assets.list"))" ] \
    || die "the website is dirty in what ships: commit it"
  COPYFILE_DISABLE=1 tar --no-xattrs -C "$stage" -czf "$WORK/webapp.tgz" $(cd "$stage" && ls -A) || die "tar webapp"
  note "/assets: $(wc -l < "$WORK/assets.list" | tr -d ' ') files, $(du -sh "$stage/assets" | cut -f1), from website $1"
  while read -r f q i; do note "pinned: /$q $i (${f})"; done < "$WORK/pins.list"
}
# ONLY WHAT THE PAGES LOAD (NC-56), followed from them (stage_webapp); kept here only what
# no staged page names: index.html, the page itself; embed.js, which a site that mounts the
# webapp loads, not index.html; and brand/, whole, whose fonts and marks such a site and
# index.html's own styles use.
WEBAPP_KEEP="index.html embed.js brand"
# THE LIVERY a site mounting the webapp themes itself with: a face's look, the fonts it
# names, and their @font-face rules (whose font files the bake follows). Baked and pinned
# whether or not index.html loads them.
LIVERY="assets/face/look.js assets/face/vendor/face-assets.js assets/face/vendor/fonts.css"
# THE FACE EDITOR's stickers: face.js builds their names at run time (its own URL's vendor/
# and face-assets.js's "file" entries), so neither page names them. Baked whole, and only
# when face.js is.
WITH_FACE="assets/face/vendor/stickers"
# The webapp staged: this checkout's. `stage` alone takes another (DOOR_WEBAPP), to try one.
WEBAPP_SRC="$PRODUCT/app/web/webapp"

# What becomes the binary, and what it serves.
INPUTS="app/door/src app/door/Cargo.toml app/door/Cargo.lock app/door/clients.stand-in.json arc/lib/arc-build arc/lib/face-render app/web/door app/web/webapp core/pacific-core core/pacific-wire core/pacific-media core/coordination/delta-graph.icd.json"

DIRTY=0
guard_clean() {
  local dirty
  dirty=$(cd "$PRODUCT" && git status --porcelain -- $INPUTS)
  [ -z "$dirty" ] && return 0
  DIRTY=1
  printf '✗ REFUSING: uncommitted changes in what becomes the Door\n%s\n' "$dirty" >&2
  echo "  Deploy from a clean worktree: git worktree add --detach <dir> HEAD" >&2
  [ "${ALLOW_DIRTY:-}" = "1" ] || exit 1
  echo "⚠ ALLOW_DIRTY=1: shipping a dirty tree anyway." >&2
}

# The checks, as one script with no single quote in it: run here, or on the box.
CHECKS='for p in /v2/icd /signin /v2/signin.js; do
  printf "  %-16s %s\n" "$p" "$(curl -s $CA -o /dev/null -w "%{http_code}" --max-time 20 "https://$H$p")"; done
printf "  %-16s %s\n" frame-ancestors "$(curl -s $CA -D - -o /dev/null --max-time 20 "https://$H/v2/icd" | tr -d "\r" | grep -i "^content-security-policy" | cut -d" " -f2-)"
printf "  %-16s %s\n" hsts "$(curl -s $CA -D - -o /dev/null --max-time 20 "https://$H/v2/icd" | tr -d "\r" | grep -i "^strict-transport-security" | cut -d" " -f2-)"
printf "  %-16s %s (401 expected)\n" /v2/me "$(curl -s $CA -o /dev/null -w "%{http_code}" --max-time 20 "https://$H/v2/me")"'

# Every file index.html hashes (its /assets and the pinned webapp), fetched through the
# edge, against the hash baked with it; and every file its stylesheets load, there (HV-8).
ASSET_CHECK='page=$(curl -s $CA --max-time 20 "https://$H/")
n=0; bad=""
for pair in $(printf "%s" "$page" | grep -oE "(src|href)=\"[^\"]+\" integrity=\"sha384-[^\"]+\"" | sed -E "s/^(src|href)=\"([^\"]+)\" integrity=\"([^\"]+)\"/\2|\3/"); do
  p=${pair%%|*}; want=${pair#*|}; n=$((n+1))
  got="sha384-$(curl -s $CA --max-time 20 "https://$H/$p" | openssl dgst -sha384 -binary | base64)"
  [ "$got" = "$want" ] || bad="$bad $p"
done
r=${bad:+differ:$bad}; [ $n = 0 ] && r="none to check"
printf "  %-16s %s\n" assets "$n hashed, ${r:-all match}"
m=0; miss=""
for css in $(printf "%s" "$page" | grep -oE "href=\"assets/[^\"?#]+\.css" | cut -d\" -f2); do
  for u in $(curl -s $CA --max-time 20 "https://$H/$css" | grep -oE "url\([^)]+\)" | sed -E "s/^url\(\"?//; s/\"?\)$//; s/[?#].*//" | grep -v :); do
    m=$((m+1)); [ "$(curl -s $CA -o /dev/null -w "%{http_code}" --max-time 20 "https://$H/${css%/*}/$u")" = 200 ] || miss="$miss ${css%/*}/$u"
  done
done
r=${miss:+missing:$miss}
printf "  %-16s %s\n" "assets loaded" "$m, ${r:-all 200}"'

# Every pinned name this deploy staged ($P: path|sri ...), through the edge, against its
# SRI, and the Cache-Control it is served with.
PIN_CHECK='n=0; bad=""; imm=0
for pair in $P; do
  p=${pair%%|*}; want=${pair#*|}; n=$((n+1))
  got="sha384-$(curl -s $CA --max-time 20 "https://$H/$p" | openssl dgst -sha384 -binary | base64)"
  [ "$got" = "$want" ] || bad="$bad $p"
  curl -s $CA -D - -o /dev/null --max-time 20 "https://$H/$p" | tr -d "\r" | grep -qi "^cache-control:.*immutable" && imm=$((imm+1))
done
r=${bad:+differ:$bad}
printf "  %-16s %s\n" pinned "$n kept, ${r:-all match}; $imm immutable"'

verify() {
  # A public name is checked from here, as its visitors reach it. A `tls internal` host has
  # no public route: the checks run on the box, through the edge, trusting only its CA.
  echo
  # Every pin the host keeps (pins.py's manifest), not only this deploy's.
  local PINS
  PINS=$($SSHC "$BOX" "python3 - list /opt/door/PINS" < "$DOOR_DIR/hosting/pins.py" 2>/dev/null)
  if [ "${DOOR_TLS:-}" = "internal" ]; then
    $SSHC "$BOX" "H=$HOST CA='--cacert /opt/door/edge-root.crt --resolve $HOST:443:127.0.0.1' sh -c '$CHECKS'"
    $SSHC "$BOX" "H=$HOST CA='--cacert /opt/door/edge-root.crt --resolve $HOST:443:127.0.0.1' sh -c '$ASSET_CHECK'"
    [ -z "${PINS:-}" ] || $SSHC "$BOX" "H=$HOST P='$PINS' CA='--cacert /opt/door/edge-root.crt --resolve $HOST:443:127.0.0.1' sh -c '$PIN_CHECK'"
  else
    H=$HOST CA="" sh -c "$CHECKS"
    H=$HOST CA="" sh -c "$ASSET_CHECK"
    [ -z "${PINS:-}" ] || H=$HOST P="$PINS" CA="" sh -c "$PIN_CHECK"
  fi
  $SSHC "$BOX" 'printf "  %-16s %s\n" version "$(/opt/door/bin/door --version 2>/dev/null)"; for u in door door-edge; do printf "  %-16s %s\n" "$u" "$(systemctl is-active $u)"; done; printf "  %-16s %s\n" commit "$(cat /opt/door/COMMIT 2>/dev/null || echo none)"; printf "  %-16s %s\n" "website assets" "$(cat /opt/door/ASSETS 2>/dev/null || echo none)"' 2>/dev/null
  # TRAINING_WHEELS (P0): what the running Door said at start; configured and off fails.
  local tw=0 said
  said=$($SSHC "$BOX" 'sudo journalctl -u door --since "$(systemctl show door -p ActiveEnterTimestamp --value)" --no-pager -o cat | grep -m1 "door: training_wheels "' 2>/dev/null)
  printf "  %-16s %s\n" training_wheels "${said#door: training_wheels }"
  if $SSHC "$BOX" "sudo grep -q '^DOOR_TW_DSN=' /opt/door/door.env" 2>/dev/null; then
    if [ "${said#door: training_wheels }" != on ]; then
      echo "✗ training_wheels is configured in door.env and the Door started it off"; tw=1
    fi
    # Software Security's B4: PostgreSQL's log, outside the volume, never takes a statement or
    # a parameter. Read as the Door's own login.
    local logs
    logs=$($SSHC "$BOX" "sudo -u door psql 'host=/var/run/postgresql dbname=training_wheels user=door' -Atc \"SELECT concat_ws('|', current_setting('log_statement'), current_setting('log_parameter_max_length'), current_setting('log_parameter_max_length_on_error'), current_setting('log_min_duration_statement'))\"" 2>/dev/null)
    printf "  %-16s %s\n" "tw logging" "${logs:-unread}"
    [ "$logs" = "none|0|0|-1" ] || { echo "✗ PostgreSQL's logging is not B4's (log_statement none, both parameter lengths 0, no duration log)"; tw=1; }
  fi
  # The model it serves, against the pin (PIN-5): verify fails on any other.
  local icd=0
  if [ "${DOOR_TLS:-}" = "internal" ]; then
    $SSHC "$BOX" "curl -fsS --max-time 20 --cacert /opt/door/edge-root.crt --resolve $HOST:443:127.0.0.1 https://$HOST/v2/icd" | "$ICD_PIN" served - || icd=1
  else
    "$ICD_PIN" served "https://$HOST/v2/icd" || icd=1
  fi
  echo
  return $(( icd | tw ))
}

setup() {
  note "setting up $BOX: the door user, /opt/door, the units door and door-edge, Caddy"
  # Cloudflare's published ranges (cloudflare.com/ips, as of 27 Sep 2026): the relay and
  # the auth service are behind it. "none" for a rehearsal whose services are on loopback.
  local cf="173.245.48.0/20 103.21.244.0/22 103.22.200.0/22 103.31.4.0/22 141.101.64.0/18 108.162.192.0/18 190.93.240.0/20 188.114.96.0/20 197.234.240.0/22 198.41.128.0/17 162.158.0.0/15 104.16.0.0/13 104.24.0.0/14 172.64.0.0/13 131.0.72.0/22 2400:cb00::/32 2606:4700::/32 2803:f800::/32 2405:b500::/32 2405:8100::/32 2a06:98c0::/29 2c0f:f248::/32"
  local egress=${DOOR_EGRESS:-$cf}
  if [ "$egress" = none ]; then
    sed -e "s|__MEMORY_MAX__|${DOOR_MEMORY_MAX:-75%}|" -e "/__EGRESS__/d" "$DOOR_DIR/hosting/door.service" > /tmp/door.service
  else
    sed -e "s|__MEMORY_MAX__|${DOOR_MEMORY_MAX:-75%}|" -e "s|__EGRESS__|$egress|" "$DOOR_DIR/hosting/door.service" > /tmp/door.service
  fi
  $SCP /tmp/door.service "$BOX:/tmp/door.service" || die "scp unit"
  $SCP "$DOOR_DIR/hosting/door-edge.service" "$BOX:/tmp/door-edge.service" || die "scp edge unit"
  $SCP "$DOOR_DIR/hosting/door.env.example" "$BOX:/tmp/door.env.example" || die "scp env"
  render_caddyfile
  $SCP /tmp/door-Caddyfile "$BOX:/tmp/door-Caddyfile" || die "scp Caddyfile"
  $SSHC "$BOX" "set -e
    id door >/dev/null 2>&1 || sudo useradd --system --no-create-home --shell /usr/sbin/nologin door
    sudo mkdir -p /opt/door/bin /opt/door/webapp
    if ! command -v caddy >/dev/null; then
      sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq caddy
      # The package starts its own caddy.service on :80; the Door's edge is door-edge.
      sudo systemctl disable --now caddy >/dev/null 2>&1 || true
    fi
    sed -i \"s|__CADDY__|\$(command -v caddy)|\" /tmp/door-edge.service
    sudo install -m 0644 /tmp/door.service /etc/systemd/system/door.service
    sudo install -m 0644 /tmp/door-edge.service /etc/systemd/system/door-edge.service
    sudo install -m 0644 /tmp/door-Caddyfile /opt/door/Caddyfile
    [ -f /opt/door/door.env ] || sudo install -m 0640 -g door /tmp/door.env.example /opt/door/door.env
    sudo systemctl daemon-reload
    sudo systemctl enable door door-edge >/dev/null 2>&1
    sudo systemctl restart door-edge" || die "setup failed"
  if [ "${DOOR_TLS:-}" = "internal" ]; then
    # The edge's own root, for the checks and the clients that are to trust it and no other.
    $SSHC "$BOX" 'for i in $(seq 1 30); do sudo test -f /var/lib/private/door-edge/pki/authorities/local/root.crt && break; sleep 1; done
      sudo install -m 0644 /var/lib/private/door-edge/pki/authorities/local/root.crt /opt/door/edge-root.crt' || die "the edge made no CA"
  fi
  note "edit /opt/door/door.env on the host for DOOR_PUBLIC, DOOR_AUTH, DOOR_RELAY, then deploy"
}

clients() {
  python3 -m json.tool < "$CLIENTS" > /dev/null || die "$CLIENTS is not JSON: the Door would not start"
  note "shipping $CLIENTS to $BOX; the Door restarts, and every session ends head-first (NC-47)"
  $SCP "$CLIENTS" "$BOX:/tmp/clients.json" || die "scp clients"
  $SSHC "$BOX" "set -e; sudo install -m 0644 /tmp/clients.json /opt/door/clients.json; $RESTART_DOOR" || die "clients failed"
  sleep 3; verify
}

# K-48: the Door is stopped, so every session ends and seals, and a full seal set is backed
# up before the next Door starts (backup-door.service, arc/hosting/backup). A box without the
# unit restarts as before, and says so; a backup that fails is said, and the Door starts.
RESTART_DOOR='sudo systemctl stop door
    if systemctl cat backup-door.service >/dev/null 2>&1; then
      sudo install -d -m 0700 /var/lib/backup-door && sudo touch /var/lib/backup-door/full.next &&
        sudo systemctl start backup-door.service || echo "deploy-door: the backup before this start failed (journalctl -u backup-door); the Door starts" >&2
    else
      echo "deploy-door: backup-door.service is not installed: no backup before this start" >&2
    fi
    sudo systemctl start door'

rollback() {
  # A SWAP, binary and webapp together, so a rollback can itself be undone. COMMIT is
  # rewritten from the binary's own stamp, not remembered (NC-51). The clients file is
  # configuration, not a build, and stays.
  $SSHC "$BOX" 'set -e
    cd /opt/door
    test -f bin/door.prev || { echo "no previous binary" >&2; exit 1; }
    sudo mv bin/door bin/door.swap && sudo mv bin/door.prev bin/door && sudo mv bin/door.swap bin/door.prev
    if [ -d webapp.prev ]; then sudo mv webapp webapp.swap && sudo mv webapp.prev webapp && sudo mv webapp.swap webapp.prev; fi
    if [ -f ASSETS.prev ]; then sudo mv ASSETS ASSETS.swap && sudo mv ASSETS.prev ASSETS && sudo mv ASSETS.swap ASSETS.prev; fi
    bin/door --version | python3 -c "import json,sys; s=json.load(sys.stdin); print(s[\"commit\"] + (\"-dirty\" if s.get(\"dirty\") else \"\"))" | sudo tee COMMIT >/dev/null
    '"$RESTART_DOOR" || die "rollback failed"
  # The pins a site may hold stay served: every retained one into the webapp now live.
  $SSHC "$BOX" "sudo python3 - carry /opt/door/webapp /opt/door/webapp.prev /opt/door/PINS" < "$DOOR_DIR/hosting/pins.py" || die "carry pins"
  sleep 3; verify
}

# Pinned versions out at once (SECURITY: a security fix prunes the affected versions, and
# the site is told to re-pin). Refused for a pin the live index.html loads.
prune() {
  [ $# -gt 0 ] || die "prune <commit|name>..."
  $SSHC "$BOX" "sudo python3 - prune /opt/door/webapp /opt/door/webapp.prev /opt/door/PINS $*" < "$DOOR_DIR/hosting/pins.py" || die "prune refused"
  verify
}

SHIP=0
case "${1:-deploy}" in
  verify) verify; exit $? ;;
  setup) setup; exit 0 ;;
  rollback) rollback; exit $? ;;
  clients) clients; exit $? ;;
  prune) shift; prune "$@"; exit $? ;;
  stage) WEBAPP_SRC=${DOOR_WEBAPP:-$WEBAPP_SRC}; WCOMMIT=$(assets_commit) || exit 1; stage_webapp "$WCOMMIT"; note "$WORK/webapp"; exit 0 ;;
  # Build and upload to /opt/door/bin/door.next, nothing swapped or restarted: training_wheels'
  # spool key is made with it first (`door.next tw-keygen`, runbook § 2t), then `deploy`.
  ship) SHIP=1 ;;
  deploy) ;;
  *) die "deploy | ship | setup | verify | rollback | clients | stage | prune" ;;
esac

# THE PIN FIRST (O-68): before the dirt guard, so ALLOW_DIRTY=1 cannot pass it.
"$ICD_PIN" deploy || die "REFUSING: the ICD is not the pinned model; nothing built, nothing uploaded"
# THE FOLD CACHE'S TEST SWITCHES NEVER SHIP (O-69): a stamped build names its own model, and
# a verified hit that differed would end a person's session.
# A file that is missing or unreadable, or a sudo that fails, refuses: only grep's "none
# found" (exit 1) counts as zero, and the count must be a number.
SWITCHES=$($SSHC "$BOX" "sudo test -r /opt/door/door.env && { sudo grep -cE '^[[:space:]]*(export[[:space:]]+)?PACIFIC_FOLD_CACHE_' /opt/door/door.env; [ \$? -le 1 ]; }") \
  || die "REFUSING: /opt/door/door.env could not be read; nothing built, nothing uploaded"
case "$SWITCHES" in ''|*[!0-9]*) die "REFUSING: /opt/door/door.env's switches could not be counted; nothing built, nothing uploaded" ;; esac
[ "$SWITCHES" = 0 ] || die "REFUSING: /opt/door/door.env sets PACIFIC_FOLD_CACHE_*; nothing built, nothing uploaded"
# TRAINING_WHEELS (P0): door.env names all four of its settings or none; half is a mistake the
# Door would only report as "off". Not for `ship`, which comes before the key is made.
if [ "$SHIP" = 0 ]; then
  TW=$($SSHC "$BOX" "sudo grep -cE '^(DOOR_TW_DSN|DOOR_TW_SPOOL|DOOR_TW_SPOOL_PUB|DOOR_TW_SPOOL_KEY)=' /opt/door/door.env; true") \
    || die "REFUSING: /opt/door/door.env could not be read; nothing built, nothing uploaded"
  case "${TW:-0}" in
    0|4) ;;
    *) die "REFUSING: /opt/door/door.env names $TW of training_wheels' four settings (runbook § 2t); nothing built, nothing uploaded" ;;
  esac
fi
guard_clean
command -v cargo-zigbuild >/dev/null || die "cargo-zigbuild missing — brew install zig && cargo install cargo-zigbuild"
COMMIT=$(cd "$PRODUCT" && git rev-parse HEAD)

LABEL=$COMMIT; [ "$DIRTY" = 1 ] && LABEL=$COMMIT-dirty
# THE STAMP (arc/lib/arc-build), compiled in: `door --version` answers for the binary.
# Core is in this repository, so its commit is this one.
export ARC_BUILD_SHA=$COMMIT
export ARC_BUILD_REF=$(cd "$PRODUCT" && git rev-parse --abbrev-ref HEAD)
export ARC_BUILD_DIRTY=$DIRTY
export ARC_BUILD_CORE=$LABEL
export ARC_BUILD_TIME=$(date -u +%Y-%m-%dT%H:%M:%SZ)
note "cross-building the Door at ${LABEL:0:18} for $TARGET (glibc $GLIBC)"
python3 -m json.tool < "$CLIENTS" > /dev/null || die "$CLIENTS is not JSON: the Door would not start"
keys_ok
WCOMMIT=$(assets_commit) || exit 1
trap 'rm -rf "${WORK:-}"' EXIT
stage_webapp "$WCOMMIT"
render_caddyfile
(cd "$DOOR_DIR" && cargo zigbuild --release --target "$TARGET.$GLIBC") || die "build failed"
BIN="${CARGO_TARGET_DIR:-$DOOR_DIR/target}/$TARGET/release/door"
[ -f "$BIN" ] || die "missing binary: $BIN"
LC_ALL=C grep -aq "@(#)arc-build commit=$COMMIT" "$BIN" || die "the binary carries no stamp for $COMMIT: refusing to ship it"

note "shipping to $BOX"
# Beside the live one, then swap: a half-finished copy must never be what systemd starts.
$SCP "$BIN" "$BOX:/tmp/door.new" || die "scp door"
if [ "$SHIP" = 1 ]; then
  $SSHC "$BOX" "sudo install -m 0755 /tmp/door.new /opt/door/bin/door.next" || die "ship failed"
  note "shipped to /opt/door/bin/door.next at ${LABEL:0:12}; nothing swapped or restarted"
  exit 0
fi
$SCP "$CLIENTS" "$BOX:/tmp/clients.json" || die "scp clients"
$SCP "$CLAIM_KEYS" "$BOX:/tmp/claim-keys.json" || die "scp claim keys"
$SCP "$WORK/webapp.tgz" "$BOX:/tmp/door-webapp.tgz" || die "scp webapp"
$SCP "$WORK/pins.list" "$BOX:/tmp/door-pins.list" || die "scp pins"
$SCP "$DOOR_DIR/hosting/pins.py" "$BOX:/tmp/door-pins.py" || die "scp pins.py"
$SCP /tmp/door-Caddyfile "$BOX:/tmp/door-Caddyfile" || die "scp Caddyfile"
$SSHC "$BOX" "set -e
  cd /opt/door/bin
  if [ -f door ]; then sudo cp -p door door.prev; fi
  sudo install -m 0755 /tmp/door.new door
  sudo install -m 0644 /tmp/clients.json /opt/door/clients.json
  sudo install -m 0644 /tmp/claim-keys.json /opt/door/claim-keys.json
  sudo rm -rf /opt/door/webapp.new && sudo mkdir -p /opt/door/webapp.new
  sudo tar -C /opt/door/webapp.new -xzf /tmp/door-webapp.tgz
  # Earlier deploys' pins kept beside this one's, the rest pruned (SECURITY's rule, pins.py).
  sudo python3 /tmp/door-pins.py ship /opt/door/webapp /opt/door/webapp.new /opt/door/PINS $LABEL /tmp/door-pins.list
  sudo rm -rf /opt/door/webapp.prev; [ -d /opt/door/webapp ] && sudo mv /opt/door/webapp /opt/door/webapp.prev
  sudo mv /opt/door/webapp.new /opt/door/webapp
  echo $LABEL | sudo tee /opt/door/COMMIT >/dev/null
  if [ -f /opt/door/ASSETS ]; then sudo cp -p /opt/door/ASSETS /opt/door/ASSETS.prev; fi
  echo $WCOMMIT | sudo tee /opt/door/ASSETS >/dev/null
  # The edge restarts only when its config changed.
  sudo cmp -s /tmp/door-Caddyfile /opt/door/Caddyfile || { sudo install -m 0644 /tmp/door-Caddyfile /opt/door/Caddyfile && sudo systemctl restart door-edge; }
  $RESTART_DOOR" || die "swap failed"

sleep 4
verify
