#!/usr/bin/env bash
# backup.sh — K-48: a box's state, encrypted, off the box (srr/scmp.md § 11).
#
#   backup.sh arc [full]     kenjin-01: the account store first, then /opt/arc/state
#   backup.sh door [full]    door-01: the seals in /var/lib/door/seals, and training_wheels
#                            beside them: its database's pg_dump and its spool
#
# Run by backup-<role>.service: its timer every 15 minutes (door five minutes after arc),
# or a deploy (deploy-arc.sh, deploy-door.sh), which asks for a full set by leaving
# full.next in the unit's StateDirectory.
#
# EACH TICK COPIES EVERY STORE (arc: auth first), then sends only what changed (Ralph,
# 27 Sep): a store goes up when the sha256 of its COPY differs from the one the ledger
# last sent. Whenever anything goes up, or a store came or went, a manifest goes up
# naming, for every store, the object of its newest content, its hash and copy time; a
# restore reads one manifest and has a whole set. A seal set pairs with the newest arc
# manifest before it. The first tick after 00:00 UTC, a ledger missing or unreadable, a
# last full over 24 hours old, `full`, or a deploy's full.next: every store and its
# manifest, marked full. Retention (183 days) is the bucket's, not this script's.
#
# TRAINING_WHEELS, the door role's (runbook § 1b, srr/scmp.md § 11; Ralph, 29 Sep: "a backup to
# rebuild the database if the MLS system goes down … kept up to date"): `pg_dump -Fc` of the
# database, run as postgres into this run's RuntimeDirectory, the cluster's roles
# (`pg_dumpall --globals-only --no-role-passwords`: the owners the dump names, so a rebuild is
# the roles, then the dump), and each file of the spool
# (/var/lib/door/tw-spool) as it is. A dump's bytes differ from run to run, so the dump is taken
# only when the WAL's insert position (`pg_current_wal_insert_lsn()`) has moved since the dump
# the ledger names, or for a full set; otherwise the ledger's dump stands. The insert position,
# not the write position (`pg_current_wal_lsn()`), which moves on an idle cluster as WAL already
# inserted is flushed. It is the cluster's, so a write anywhere in it takes a dump, and after a
# burst of writes it moves once or twice more with none (a first read pruning a page, a standby
# snapshot): a dump or two too many, never one too few. With the database unreachable, the seals still go up, the
# manifest names the last dump and roles, and the run fails, naming training_wheels. The roles
# are taken and sent with each dump, and stand with it otherwise: a role changed writes WAL. On where
# /var/lib/door/tw-spool exists (BACKUP_TW=auto), required with BACKUP_TW=on, off with off.
#
# Objects: <box>/<UTC stamp>/<file>.age, then <box>/<UTC stamp>/manifest.json.age last.
# THE LEDGER, ledger.json in the unit's StateDirectory, holds per store its name, sha256,
# size, object key and copy time, and for a database its integrity_check and row counts:
# no content. Encrypted to BACKUP_AGE_RECIPIENT; the identity is on no box, and a run
# refuses where it finds one. Plaintext only in a 0700 root directory, one tick's copies
# at a time, removed on every exit and again at the next run's start: for the arc role
# its StateDirectory, beside its sources on the disk that holds them as plaintext; for
# the door role its RuntimeDirectory, a tmpfs, with its StateDirectory holding only the
# ledger. THE DOOR ROLE STREAMS (door-test, run 74, 29 Sep: 1,284 seals, 733 MiB, did not fit /run's
# 793 MiB tmpfs beside the dump, and nothing went up): each store is hashed where it lies and, if it
# changed, read once straight into age, so its plaintext is in no file at all; its ciphertext waits
# on the StateDirectory's disk until it is up. The tmpfs holds the run's small files alone.
#
# /etc/backup/backup.env, 0600 root's: BACKUP_R2_ENDPOINT, BACKUP_R2_BUCKET,
# BACKUP_R2_ACCESS_KEY_ID, BACKUP_R2_SECRET_ACCESS_KEY, BACKUP_AGE_RECIPIENT. Refused if
# any is missing, or if the file is anyone's but root's.
#
# For a test on a laptop: BACKUP_DEST_DIR writes the objects to a directory in place of R2
# (the env file and the directories may then be the tester's own, still 0600 and 0700),
# and BACKUP_ENV, BACKUP_ARC_STATE, BACKUP_SEALS, BACKUP_AUTH_CONTAINER, BACKUP_BOX,
# BACKUP_ROOT_HOME and BACKUP_NOW point it at scratch copies and a clock; BACKUP_TW_SPOOL,
# BACKUP_TW_DB, BACKUP_PGBIN and BACKUP_PG_AS (the prefix that runs a client as postgres,
# `runuser -u postgres --`; empty for a tester's own cluster) at a scratch training_wheels.
set -euo pipefail

ROLE=${1:-}
MODE=${2:-}
ENV_FILE=${BACKUP_ENV:-/etc/backup/backup.env}
ARC_STATE=${BACKUP_ARC_STATE:-/opt/arc/state}
SEALS=${BACKUP_SEALS:-/var/lib/door/seals}
AUTH_CONTAINER=${BACKUP_AUTH_CONTAINER:-kenjin-auth}
AUTH_DB=${BACKUP_AUTH_DB:-/data/auth.db}
DEST=${BACKUP_DEST_DIR:-}
TW=${BACKUP_TW:-auto}
TW_SPOOL=${BACKUP_TW_SPOOL:-/var/lib/door/tw-spool}
TW_DB=${BACKUP_TW_DB:-training_wheels}
PGBIN=${BACKUP_PGBIN:-}
PG_AS=${BACKUP_PG_AS-runuser -u postgres --}
TW_FAILED=

say() { echo "backup: $*"; }
die() { echo "backup: $*" >&2; exit 1; }

case "$ROLE" in arc | door) ;; *) die "usage: backup.sh arc|door [full]" ;; esac
case "$MODE" in "" | full) ;; *) die "usage: backup.sh arc|door [full]" ;; esac
case "$TW" in auto | on | off) ;; *) die "BACKUP_TW is auto, on or off, not $TW" ;; esac
command -v age >/dev/null || die "age is not on PATH: nothing is sent unencrypted"
command -v python3 >/dev/null || die "python3 is not on PATH"

# THE ENV FILE: root's alone, and only the five names are read from it (never sourced).
[ -f "$ENV_FILE" ] || die "$ENV_FILE is missing"
read -r MODEBITS OWNER < <(python3 -c 'import os,sys; s=os.stat(sys.argv[1]); print(oct(s.st_mode & 0o777)[2:], s.st_uid)' "$ENV_FILE")
if [ $((8#$MODEBITS & 8#077)) -ne 0 ]; then die "$ENV_FILE is mode $MODEBITS: anyone but its owner may read it"; fi
if [ -z "$DEST" ] && [ "$OWNER" != 0 ]; then die "$ENV_FILE is not root's (uid $OWNER)"; fi
while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in
    BACKUP_R2_ENDPOINT=* | BACKUP_R2_BUCKET=* | BACKUP_R2_ACCESS_KEY_ID=* | BACKUP_R2_SECRET_ACCESS_KEY=* | BACKUP_AGE_RECIPIENT=*)
      val=${line#*=}
      case "$val" in \"*\" | \'*\') val=${val:1:${#val}-2} ;; esac
      printf -v "${line%%=*}" '%s' "$val" ;;
  esac
done < "$ENV_FILE"
for v in BACKUP_R2_ENDPOINT BACKUP_R2_BUCKET BACKUP_R2_ACCESS_KEY_ID BACKUP_R2_SECRET_ACCESS_KEY BACKUP_AGE_RECIPIENT; do
  [ -n "${!v:-}" ] || die "$ENV_FILE names no $v"
done
case "$BACKUP_AGE_RECIPIENT" in age1*) ;; *) die "BACKUP_AGE_RECIPIENT is not an age recipient (age1…)" ;; esac

# NO IDENTITY ON THE BOX (SECURITY): a box that could read its own backups is refused, and
# the place that could is named. Where a key would be put, not the whole disk. A key's own
# shape (bech32 after its "1"), so this script's pattern is not itself a finding; in any
# case, as bech32 is read.
KEY='AGE-SECRET-KEY-[A-Z0-9-]*1[QPZRY9X8GF2TVDW0S3JN54KHCE6MUA7L]{20,}'
held=$(env | grep -iE "^[A-Za-z_][A-Za-z0-9_]*=.*$KEY" | cut -d= -f1 | head -1 || true)
[ -z "$held" ] || die "an age identity is in the environment, in $held: a box must not hold the key its backups open with"
SELF=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT_HOME=${BACKUP_ROOT_HOME:-$(python3 -c 'import pwd; print(pwd.getpwnam("root").pw_dir)')}
found=$( { grep -rliIE "$KEY" /etc/backup /opt/backup "$SELF" "$ROOT_HOME/.config/age" 2>/dev/null || true
  find "$ROOT_HOME" -maxdepth 1 -type f -exec grep -liIE "$KEY" {} + 2>/dev/null || true; } | head -1)
[ -z "$found" ] || die "an age identity is on this box, in $found: a box must not hold the key its backups open with"

# THE DIRECTORIES, 0700 root's: the copies' (arc: StateDirectory, beside its sources; door:
# RuntimeDirectory, a tmpfs) and the ledger's (the StateDirectory, for both).
mode_of() { python3 -c 'import os,sys; s=os.stat(sys.argv[1]); print(oct(s.st_mode & 0o777)[2:], s.st_uid)' "$1"; }
private() { # <dir> <what>
  [ -n "$1" ] && [ -d "$1" ] || die "no $2: run as backup-$ROLE.service"
  local m u
  read -r m u < <(mode_of "$1")
  [ "$m" = 700 ] || die "$1 is mode $m, not 0700"
  if [ -z "$DEST" ] && [ "$u" != 0 ]; then die "$1 is not root's (uid $u)"; fi
}
LEDGERDIR=${STATE_DIRECTORY:-}
private "$LEDGERDIR" StateDirectory
if [ "$ROLE" = arc ]; then
  ROOTDIR=$LEDGERDIR
  python3 -c 'import os,sys; sys.exit(os.stat(sys.argv[1]).st_dev != os.stat(sys.argv[2]).st_dev)' "$ROOTDIR" "$ARC_STATE" ||
    die "$ROOTDIR is not on $ARC_STATE's filesystem: the copies would reach a disk the plaintext is not already on"
else
  ROOTDIR=${RUNTIME_DIRECTORY:-}
  private "$ROOTDIR" RuntimeDirectory
  if [ -z "$DEST" ] && [ "$(stat -f -c %T "$ROOTDIR")" != tmpfs ]; then die "$ROOTDIR is not a tmpfs: plaintext would reach a disk"; fi
fi
LEDGER=$LEDGERDIR/ledger.json
umask 077
# One run at a time over a ledger (a deploy's full beside a tick).
if command -v flock >/dev/null; then exec 9>"$LEDGERDIR/.lock"; flock 9; fi
# A run killed outright (kill -9) left its copies: gone before anything else is copied.
rm -rf "$ROOTDIR"/set.*
WORK=$(mktemp -d "$ROOTDIR/set.XXXXXX")
OUT=
if [ "$ROLE" = door ]; then
  rm -rf "$LEDGERDIR"/out.*
  OUT=$(mktemp -d "$LEDGERDIR/out.XXXXXX")
fi
# A deploy's request for a full set is taken now, and put back if this run does not finish.
if [ -e "$LEDGERDIR/full.next" ] || [ -e "$LEDGERDIR/full.taken" ]; then
  MODE=full
  if [ -e "$LEDGERDIR/full.next" ]; then mv "$LEDGERDIR/full.next" "$LEDGERDIR/full.taken"; fi
fi
finish() {
  local st=$?
  rm -rf "$WORK" ${OUT:+"$OUT"}
  if [ "$st" != 0 ] && [ -e "$LEDGERDIR/full.taken" ]; then mv "$LEDGERDIR/full.taken" "$LEDGERDIR/full.next"; fi
  exit "$st"
}
trap finish EXIT
trap 'exit 1' INT TERM HUP

BOX=${BACKUP_BOX:-$(hostname -s)}
NOW=${BACKUP_NOW:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}
STAMP=$(echo "$NOW" | tr -d ':-')

PY="$WORK/db.py"
cat > "$PY" <<'PYEND'
import datetime, hashlib, json, os, sqlite3, sys

# Opened as its service opens it: read-only, a WAL database with no -shm beside it (its
# service stopped) will not open at all; the backup API only reads it. The copy is one
# file, out of WAL, so it can be read read-only, hashed and sent whole. An unchanged
# database copies to the same bytes, so its hash is the same.
def backup(src, dst):
    s = sqlite3.connect(src)
    d = sqlite3.connect(dst)
    s.backup(d)
    d.execute("PRAGMA journal_mode=DELETE")
    d.close(); s.close()

def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()

def ledger(path):
    """The ledger, or None and why not: missing or unreadable is a full set, never a skip."""
    try:
        with open(path) as f:
            l = json.load(f)
        if not isinstance(l, dict) or not isinstance(l.get("stores"), dict):
            return None, "the ledger holds no stores"
        return l, ""
    except FileNotFoundError:
        return None, "no ledger"
    except (OSError, ValueError) as e:
        return None, f"the ledger is unreadable ({e.__class__.__name__})"

def when(s):
    return datetime.datetime.strptime(s, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=datetime.timezone.utc)

cmd = sys.argv[1]
if cmd == "backup":
    backup(sys.argv[2], sys.argv[3])
elif cmd == "free":
    s = os.statvfs(sys.argv[2])
    print(s.f_bavail * s.f_frsize)
elif cmd == "sizes":   # the sum and the largest
    z = [os.path.getsize(p) for p in sys.argv[2:] if os.path.isfile(p)]
    print(sum(z), max(z or [0]))
elif cmd == "sha256":
    print(sha256(sys.argv[2]))
elif cmd == "due":     # <ledger> <now> <mode>: "full <why>" or "changes"
    l, why = ledger(sys.argv[2])
    now = when(sys.argv[3])
    if sys.argv[4] == "full":
        print("full asked")
    elif l is None:
        print("full", why)
    elif not l.get("full"):
        print("full the ledger names no full set")
    elif when(l["full"]).date() != now.date():   # so never more than a day apart
        print("full the day's first")
    else:
        print("changes")
elif cmd == "lsn":     # <ledger> <name>: the WAL position the ledger's dump was taken at
    l, _ = ledger(sys.argv[2])
    print(((l or {}).get("stores", {}).get(sys.argv[3]) or {}).get("lsn", ""))
elif cmd == "plan":    # <ledger> <stores> <work> <kind>: what goes up, and what the ledger keeps
    l, _ = ledger(sys.argv[2])
    known = (l or {}).get("stores", {})
    full = sys.argv[5] == "full"
    names = []
    with open(sys.argv[3]) as f, open(os.path.join(sys.argv[4], "kept.jsonl"), "w") as kept:
        for line in f:
            name, kind, copied = line.rstrip("\n").split("|")
            # Not copied this tick (a dump not due, or not to be had): the ledger's entry stands,
            # where it has one.
            if kind == "carry":
                if name in known:
                    names.append(name)
                    kept.write(json.dumps(known[name]) + "\n")
                    print(f"keep|{name}|{kind}|{copied}")
                continue
            names.append(name)
            h = sha256(os.path.join(sys.argv[4], name))
            # The roles go with each dump, whether or not their text changed.
            if not full and kind != "pgglobals" and name in known and known[name].get("sha256") == h:
                kept.write(json.dumps(known[name]) + "\n")
                print(f"keep|{name}|{kind}|{copied}")
            else:
                print(f"send|{name}|{kind}|{copied}")
    if sorted(names) != sorted(known):
        print("set|||")
elif cmd == "hash":    # <path>: its sha256 and size, read in place
    h, n = hashlib.sha256(), 0
    with open(sys.argv[2], "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
            n += len(chunk)
    print(h.hexdigest(), n)
elif cmd == "seal":    # <src> <out> <recipient>: read once, straight into age; the sha256 and size of what went in
    import subprocess
    src, out, rec = sys.argv[2:5]
    h, n = hashlib.sha256(), 0
    p = subprocess.Popen(["age", "-r", rec, "-o", out], stdin=subprocess.PIPE)
    with open(src, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
            n += len(chunk)
            p.stdin.write(chunk)
    p.stdin.close()
    if p.wait() != 0:
        sys.exit("age did not seal " + os.path.basename(src))
    print(h.hexdigest(), n)
elif cmd == "sink":    # <meta>: stdin through to stdout, and its sha256 and size to <meta> at the end
    h, n = hashlib.sha256(), 0
    inp, outp = sys.stdin.buffer, sys.stdout.buffer
    for chunk in iter(lambda: inp.read(1 << 20), b""):
        h.update(chunk)
        n += len(chunk)
        outp.write(chunk)
    outp.flush()
    with open(sys.argv[2], "w") as f:
        f.write(f"{h.hexdigest()} {n}\n")
elif cmd == "known":   # <ledger> <name>: the ledger's sha256 of it, or nothing
    l, _ = ledger(sys.argv[2])
    print(((l or {}).get("stores", {}).get(sys.argv[3]) or {}).get("sha256", ""))
elif cmd == "entry":   # <ledger> <name>: the ledger's entry for it, or nothing
    l, _ = ledger(sys.argv[2])
    e = (l or {}).get("stores", {}).get(sys.argv[3])
    print(json.dumps(e) if e else "")
elif cmd == "entry_new":  # <name> <kind> <key> <copied> <sha256> <size> [<lsn>]: a store sent, described
    name, kind, key, copied, sha, size = sys.argv[2:8]
    out = {"name": name, "size": int(size), "sha256": sha, "key": key, "copied": copied}
    if kind == "pgdump":
        out["format"] = "pg_dump -Fc"
        out["lsn"] = sys.argv[8]
    if kind == "pgglobals":
        out["format"] = "pg_dumpall --globals-only"
    print(json.dumps(out))
elif cmd == "setcheck":  # <ledger> <names>: "set" where this tick's stores are not the ledger's
    l, _ = ledger(sys.argv[2])
    names = {x.strip() for x in open(sys.argv[3]) if x.strip()}
    print("set" if names != set((l or {}).get("stores", {})) else "")
elif cmd == "describe":  # <path> <name> <kind> <key> <copied>
    path, name, kind, key, copied = sys.argv[2:7]
    out = {"name": name, "size": os.path.getsize(path), "sha256": sha256(path), "key": key, "copied": copied}
    if kind == "pgdump":
        out["format"] = "pg_dump -Fc"
        with open(path + ".lsn") as f:
            out["lsn"] = f.read().strip()
    if kind == "pgglobals":
        out["format"] = "pg_dumpall --globals-only"
    if kind == "db":
        c = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
        out["integrity"] = c.execute("PRAGMA integrity_check").fetchone()[0]
        tables = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        out["rows"] = {t: c.execute('SELECT count(*) FROM "%s"' % t.replace('"', '""')).fetchone()[0] for t in tables}
        c.close()
    print(json.dumps(out))
elif cmd == "compose":   # <work> <ledger> <box> <role> <stamp> <now> <full>: the manifest, and the ledger after it
    work, lpath, box, role, stamp, now, full = sys.argv[2:9]
    files = []
    for part in ("kept.jsonl", "sent.jsonl"):
        p = os.path.join(work, part)
        if os.path.exists(p):
            files += [json.loads(x) for x in open(p) if x.strip()]
    files.sort(key=lambda e: e["name"])
    old, _ = ledger(lpath)
    last_full = now if full == "1" else (old or {}).get("full")
    m = {"v": 2, "box": box, "role": role, "stamp": stamp, "at": now, "full": full == "1", "files": files}
    with open(os.path.join(work, "manifest.json"), "w") as f:
        json.dump(m, f, indent=1)
    new = {"v": 1, "box": box, "role": role, "manifest": f"{box}/{stamp}/manifest.json.age", "full": last_full,
           "stores": {e["name"]: e for e in files}}
    with open(os.path.join(work, "ledger.next"), "w") as f:
        json.dump(new, f, indent=1)
PYEND

# One object up: to BACKUP_DEST_DIR in a test; else R2 by curl's SigV4, the key and secret
# on curl's stdin, never its command line; the aws-cli container only if curl fails.
put() {
  local file=$1 key=$2
  if [ -n "$DEST" ]; then
    mkdir -p "$(dirname "$DEST/$key")"
    cp "$file" "$DEST/$key"
    return
  fi
  local sha
  sha=$(python3 "$PY" sha256 "$file")
  if printf 'user = "%s:%s"\n' "$BACKUP_R2_ACCESS_KEY_ID" "$BACKUP_R2_SECRET_ACCESS_KEY" |
    curl -fsS --retry 3 --retry-all-errors -K - --aws-sigv4 "aws:amz:auto:s3" \
      -H "x-amz-content-sha256: $sha" -T "$file" "$BACKUP_R2_ENDPOINT/$BACKUP_R2_BUCKET/$key" -o /dev/null; then
    return
  fi
  say "curl could not put $key: trying the aws-cli container"
  AWS_ACCESS_KEY_ID=$BACKUP_R2_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY=$BACKUP_R2_SECRET_ACCESS_KEY \
    docker run --rm -e AWS_ACCESS_KEY_ID -e AWS_SECRET_ACCESS_KEY -e AWS_DEFAULT_REGION=auto \
      -v "$file:/set/object:ro" amazon/aws-cli --endpoint-url "$BACKUP_R2_ENDPOINT" \
      s3 cp /set/object "s3://$BACKUP_R2_BUCKET/$key" >/dev/null ||
    die "could not put $key, by curl or the aws-cli container"
  say "put $key by the aws-cli container"
}

# ROOM FOR THE TICK: every store's copy, the largest's ciphertext beside it, and a margin,
# before anything is copied, or a refusal that names what needs it.
MARGIN=$((16 << 20))
roomy() { # <sum> <largest> <what>
  local free need
  free=$(python3 "$PY" free "$WORK")
  need=$(($1 + $2 + MARGIN))
  [ "$free" -ge "$need" ] || die "$ROOTDIR has $((free >> 20)) MiB free; $3 needs $((need >> 20)) MiB (every copy, the largest again, and 16): free room there"
}

stamp_now() { date -u +%Y-%m-%dT%H:%M:%SZ; }
STORES="$WORK/stores"
: > "$STORES"

# The account store's snapshot, taken in its container by sqlite3 backup() to `tmp` there,
# then streamed out ("stream", and removed) or left for docker cp ("keep").
AUTH_PY='import os, sqlite3, sys
src, tmp, how = sys.argv[1], sys.argv[2], sys.argv[3]
try:
    s = sqlite3.connect(src)
    d = sqlite3.connect(tmp)
    s.backup(d)
    d.execute("PRAGMA journal_mode=DELETE")
    d.close(); s.close()
    if how == "stream":
        with open(tmp, "rb") as f:
            sys.stdout.buffer.write(f.read())
finally:
    if how == "stream" and os.path.exists(tmp):
        os.remove(tmp)'

copy_arc() {
  local db f size shm sum largest at
  for f in relay.db pacific.db waker.db boxoffice.db id_ed25519 routes; do
    [ -f "$ARC_STATE/$f" ] || die "$ARC_STATE/$f is missing"
  done
  read -r size shm < <(docker exec "$AUTH_CONTAINER" python -c 'import os,sys; s=os.statvfs("/dev/shm"); print(os.path.getsize(sys.argv[1]), s.f_bavail*s.f_frsize)' "$AUTH_DB") ||
    die "the account store's size could not be read in $AUTH_CONTAINER"
  read -r sum largest < <(python3 "$PY" sizes "$ARC_STATE"/relay.db "$ARC_STATE"/pacific.db "$ARC_STATE"/waker.db "$ARC_STATE"/boxoffice.db "$ARC_STATE"/id_ed25519 "$ARC_STATE"/routes)
  if [ "$size" -gt "$largest" ]; then largest=$size; fi
  roomy $((sum + size)) "$largest" "this tick's copies"

  # THE ACCOUNT STORE FIRST (SECURITY: the arc set leads, the seals follow): through the
  # container's own /dev/shm where it fits, else through the store's own directory, which
  # holds it as plaintext already, and docker cp; removed there either way.
  at=$(stamp_now)
  if [ "$shm" -ge $((size + (8 << 20))) ]; then
    docker exec -i "$AUTH_CONTAINER" python -c "$AUTH_PY" "$AUTH_DB" /dev/shm/backup-auth.db stream > "$WORK/auth.db" ||
      die "the account store would not back up in $AUTH_CONTAINER"
  else
    local kept ok=0
    kept="$(dirname "$AUTH_DB")/.backup-auth.db"
    say "$AUTH_CONTAINER's /dev/shm has $((shm >> 20)) MiB free for a $((size >> 20)) MiB auth.db: copying through $kept"
    { docker exec "$AUTH_CONTAINER" python -c "$AUTH_PY" "$AUTH_DB" "$kept" keep &&
      docker cp "$AUTH_CONTAINER:$kept" "$WORK/auth.db" >/dev/null; } || ok=$?
    docker exec "$AUTH_CONTAINER" rm -f "$kept" || say "could not remove $kept in $AUTH_CONTAINER"
    [ "$ok" = 0 ] || die "the account store would not back up in $AUTH_CONTAINER"
  fi
  echo "auth.db|db|$at" >> "$STORES"
  for db in relay.db pacific.db waker.db boxoffice.db; do
    at=$(stamp_now)
    python3 "$PY" backup "$ARC_STATE/$db" "$WORK/$db"
    echo "$db|db|$at" >> "$STORES"
  done
  for f in id_ed25519 routes; do
    at=$(stamp_now)
    cp "$ARC_STATE/$f" "$WORK/$f"
    echo "$f|file|$at" >> "$STORES"
  done
}

# THE DOOR ROLE, one store at a time: hashed in place; if changed (or for a full set), read once
# into age, its ciphertext on the StateDirectory's disk until it is up, then gone.
NAMES="$WORK/names"
: > "$NAMES"
: > "$WORK/sent.jsonl"
: > "$WORK/kept.jsonl"
sent=0 kept=0 reset=0

# Room on the ciphertext's disk for <bytes> and a MiB.
room_on_disk() { [ "$(python3 "$PY" free "$OUT")" -ge $(($1 + (1 << 20))) ]; }

up() { # <name> <kind> <copied> <sha256> <size> [<lsn>]: its ciphertext in $OUT goes up, described
  python3 "$PY" entry_new "$1" "$2" "$BOX/$STAMP/$1.age" "$3" "$4" "$5" ${6:+"$6"} >> "$WORK/sent.jsonl"
  echo "$1" >> "$NAMES"
  put "$OUT/$1.age" "$BOX/$STAMP/$1.age"
  rm -f "$OUT/$1.age"
  sent=$((sent + 1))
}

carry() { # <name>: not taken this tick; the ledger's entry stands, where it has one
  local e
  e=$(python3 "$PY" entry "$LEDGER" "$1")
  [ -n "$e" ] || return 0
  echo "$1" >> "$NAMES"
  echo "$e" >> "$WORK/kept.jsonl"
  kept=$((kept + 1))
}

send_file() { # <src> <name>: change-only, by its hash in place
  local src=$1 name=$2 at out sha size
  at=$(stamp_now)
  read -r sha size < <(python3 "$PY" hash "$src")
  if [ "$KIND" != full ] && [ -n "$sha" ] && [ "$sha" = "$(python3 "$PY" known "$LEDGER" "$name")" ]; then
    carry "$name"
    return 0
  fi
  room_on_disk "$size" || die "$OUT has no room for $name ($((size >> 20)) MiB and one more): free room there"
  out=$(python3 "$PY" seal "$src" "$OUT/$name.age" "$BACKUP_AGE_RECIPIENT") || die "$name could not be sealed"
  read -r sha size <<< "$out"
  up "$name" file "$at" "$sha" "$size"
}

send_stream() { # <name> <kind> <lsn|""> <command…>: its output read once into age, hashed as it passes
  local name=$1 kind=$2 lsn=$3 at sha size
  shift 3
  at=$(stamp_now)
  if ! "$@" 2>"$WORK/pg.err" | python3 "$PY" sink "$WORK/$name.meta" | age -r "$BACKUP_AGE_RECIPIENT" -o "$OUT/$name.age"; then
    rm -f "$OUT/$name.age"
    return 1
  fi
  read -r sha size < "$WORK/$name.meta"
  up "$name" "$kind" "$at" "$sha" "$size" "$lsn"
}

door_stream() {
  # Each seal is written beside itself and renamed over it: a read gets a whole file. The
  # lock files and a seal being written (*.next) are not seals.
  [ -d "$SEALS" ] || die "$SEALS is missing"
  local f name
  for f in "$SEALS"/*; do
    [ -f "$f" ] || continue
    name=$(basename "$f")
    case "$name" in *.lock | *.next) continue ;; esac
    send_file "$f" "$name"
  done
  stream_tw
  if [ "$(python3 "$PY" setcheck "$LEDGER" "$NAMES")" = set ]; then reset=1; fi
}

# A client of training_wheels' cluster, as postgres. $PG_AS splits into its words on purpose.
pgq() { $PG_AS "${PGBIN:+$PGBIN/}psql" -X -q -At -v ON_ERROR_STOP=1 -d "$TW_DB" -c "$1"; }

stream_tw() {
  case "$TW" in
    off) return 0 ;;
    auto) [ -d "$TW_SPOOL" ] || return 0 ;;
  esac
  [ -d "$TW_SPOOL" ] || die "$TW_SPOOL is missing"
  local f lsn size
  # THE SPOOL, as it is: rows the database could not take, sealed; each file its own store.
  for f in "$TW_SPOOL"/*; do
    [ -f "$f" ] || continue
    send_file "$f" "tw-spool.$(basename "$f")"
  done
  # THE DUMP AND ITS ROLES, when the WAL has moved since the ledger's dump, or for a full set;
  # the position is read first, so a write racing the dump is in it or moves the next tick's.
  # Whatever becomes of them, the seals have gone up: a failure here fails the run at its end.
  if ! lsn=$(pgq "select pg_current_wal_insert_lsn()" 2>"$WORK/pg.err"); then
    TW_FAILED="its WAL position could not be read: $(tail -1 "$WORK/pg.err")"
    carry training_wheels.dump
    carry training_wheels.globals.sql
    return 0
  fi
  if [ "$KIND" != full ] && [ "$lsn" = "$(python3 "$PY" lsn "$LEDGER" training_wheels.dump)" ]; then
    carry training_wheels.dump
    carry training_wheels.globals.sql
    return 0
  fi
  size=$(pgq "select pg_database_size(current_database())" 2>/dev/null || echo 0)
  if ! room_on_disk "$size"; then
    TW_FAILED="no room for its dump: $OUT has $(($(python3 "$PY" free "$OUT") >> 20)) MiB free, the database is $((size >> 20)) MiB"
    carry training_wheels.dump
    carry training_wheels.globals.sql
    return 0
  fi
  # $PG_AS splits into its words on purpose.
  # shellcheck disable=SC2086
  if ! send_stream training_wheels.globals.sql pgglobals "" $PG_AS "${PGBIN:+$PGBIN/}pg_dumpall" --globals-only --no-role-passwords; then
    TW_FAILED="pg_dumpall --globals-only failed: $(tail -1 "$WORK/pg.err")"
    carry training_wheels.globals.sql
  fi
  # shellcheck disable=SC2086
  if ! send_stream training_wheels.dump pgdump "$lsn" $PG_AS "${PGBIN:+$PGBIN/}pg_dump" -Fc -d "$TW_DB"; then
    TW_FAILED="${TW_FAILED:+$TW_FAILED; }pg_dump failed: $(tail -1 "$WORK/pg.err")"
    carry training_wheels.dump
  fi
}

# The run fails, after what it could send, where training_wheels could not be dumped.
tw_verdict() {
  [ -z "$TW_FAILED" ] || die "training_wheels could not be dumped ($TW_FAILED): the seals went up; the manifest names the last dump and roles"
}

DUE=$(python3 "$PY" due "$LEDGER" "$NOW" "$MODE")
read -r KIND WHY <<< "$DUE"
say "$ROLE $BOX/$STAMP: ${KIND}${WHY:+ ($WHY)}"
if [ "$ROLE" = door ]; then
  door_stream
else
copy_arc

# What changed, by the copies' hashes against the ledger's; then only that goes up.
python3 "$PY" plan "$LEDGER" "$STORES" "$WORK" "$KIND" > "$WORK/plan"
while IFS='|' read -r what name kind copied; do
  case "$what" in
    set) reset=1 ;;
    keep) rm -f "$WORK/$name"; kept=$((kept + 1)) ;;
    send)
      python3 "$PY" describe "$WORK/$name" "$name" "$kind" "$BOX/$STAMP/$name.age" "$copied" >> "$WORK/sent.jsonl"
      age -r "$BACKUP_AGE_RECIPIENT" -o "$WORK/$name.age" "$WORK/$name"
      rm -f "$WORK/$name"
      put "$WORK/$name.age" "$BOX/$STAMP/$name.age"
      rm -f "$WORK/$name.age"
      sent=$((sent + 1)) ;;
  esac
done < "$WORK/plan"
fi

if [ "$sent" = 0 ] && [ "$reset" = 0 ] && [ "$KIND" != full ]; then
  say "$ROLE $BOX/$STAMP: nothing changed ($kept stores as the ledger has them); nothing sent"
  tw_verdict
  exit 0
fi
FULL=0
if [ "$KIND" = full ]; then FULL=1; fi
python3 "$PY" compose "$WORK" "$LEDGER" "$BOX" "$ROLE" "$STAMP" "$NOW" "$FULL"
age -r "$BACKUP_AGE_RECIPIENT" -o "$WORK/manifest.json.age" "$WORK/manifest.json"
rm -f "$WORK/manifest.json"
put "$WORK/manifest.json.age" "$BOX/$STAMP/manifest.json.age"
# The ledger moves only once the manifest is up: a run that fails sends it all again.
mv "$WORK/ledger.next" "$LEDGER"
rm -f "$LEDGERDIR/full.taken"
say "$ROLE $BOX/$STAMP up: $sent sent, $kept unchanged$([ "$FULL" = 1 ] && echo ', full'), and the manifest"
tw_verdict
