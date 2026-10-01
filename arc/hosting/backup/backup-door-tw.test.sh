#!/usr/bin/env bash
# backup-door-tw.test.sh — K-48's door role with training_wheels beside the seals (runbook
# § 1b, srr/scmp.md § 11): the database's pg_dump and the spool, age-encrypted into the same
# tick and manifest, change-only by the WAL position, and a dump that rebuilds the database.
#
#   arc/hosting/backup/backup-door-tw.test.sh
#
# On a laptop: throwaway PostgreSQL 16 clusters on unix sockets (PGBIN, else Homebrew's), age and
# age-keygen on PATH (or AGE_BIN, a directory holding both), python3, and for backup-verify.py
# a Python of 3.10 or later (VERIFY_PY, else the workspace's .venv). Nothing leaves the
# machine: the objects go to a directory (BACKUP_DEST_DIR). Exits nonzero on any FAIL; KEEP=1
# keeps its directory.
set -uo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PGBIN=${PGBIN:-/opt/homebrew/opt/postgresql@16/bin}
[ -n "${AGE_BIN:-}" ] && PATH="$AGE_BIN:$PATH"
for c in age age-keygen python3 "$PGBIN/initdb" "$PGBIN/pg_dump"; do
  command -v "$c" >/dev/null || { echo "needs $c"; exit 2; }
done
T=$(mktemp -d "${TMPDIR:-/tmp}/backup-door-tw.XXXXXX")
cleanup() { for m in "$T"/small.*; do [ -d "$m" ] && unmount_small "$m"; done; for c in pg re fresh; do "$PGBIN/pg_ctl" -D "$T/$c" -m immediate stop >/dev/null 2>&1; done; if [ -n "${KEEP:-}" ]; then echo "kept $T"; else rm -rf "$T"; fi; }
# A real filesystem of <MiB>, mounted at <dir>, 0700: a disk image on the Mac, a tmpfs as root on
# Linux. Neither to be had is a FAIL, never a skip.
small_fs() { # <dir> <MiB>
  mkdir -p "$1"
  if command -v hdiutil >/dev/null; then
    hdiutil create -size "${2}m" -fs HFS+ -volname "bk$RANDOM" -quiet "$1.dmg" &&
      hdiutil attach -nobrowse -noverify -mountpoint "$1" -quiet "$1.dmg" || return 1
  elif [ "$(id -u)" = 0 ]; then
    mount -t tmpfs -o "size=${2}m,mode=0700" tmpfs "$1" || return 1
  else
    echo "    a small filesystem needs hdiutil (macOS) or root (a tmpfs)"; return 1
  fi
  chmod 700 "$1"
}
unmount_small() { if command -v hdiutil >/dev/null; then hdiutil detach -quiet -force "$1"; else umount "$1"; fi; rm -f "$1.dmg"; }
trap cleanup EXIT
mkdir -p "$T"/{seals,spool,dest,run,state,home,re.s}
chmod 700 "$T/run" "$T/state"
pass=0 failn=0
ok() { if eval "$2"; then echo "  PASS  $1"; pass=$((pass + 1)); else echo "  FAIL  $1"; failn=$((failn + 1)); fi; }

age-keygen -o "$T/id.txt" 2>/dev/null
REC=$(grep -o 'age1[0-9a-z]*' "$T/id.txt" | head -1)
printf 'BACKUP_R2_ENDPOINT=https://example.r2.cloudflarestorage.com\nBACKUP_R2_BUCKET=b\nBACKUP_R2_ACCESS_KEY_ID=x\nBACKUP_R2_SECRET_ACCESS_KEY=y\nBACKUP_AGE_RECIPIENT=%s\n' "$REC" > "$T/backup.env"
chmod 600 "$T/backup.env"

# training_wheels: its own cluster, the socket only, a table with rows, and settled, so its WAL
# moves only when the test writes: no autovacuum, no standby snapshots (wal_level minimal), and
# the setup's dead catalog tuples vacuumed, which a first read would otherwise prune and log.
"$PGBIN/initdb" -D "$T/pg" -U postgres -A trust --no-sync >/dev/null
pg_up() { "$PGBIN/pg_ctl" -D "$T/pg" -o "-k $T -c listen_addresses='' -c fsync=off -c autovacuum=off -c wal_level=minimal -c max_wal_senders=0" -l "$T/pg.log" -w start >/dev/null; }
pg_up
q() { "$PGBIN/psql" -X -q -At -h "$T" -U postgres -d "${2:-training_wheels}" -v ON_ERROR_STOP=1 -c "$1"; }
q "create database training_wheels" postgres
# The rebuild's own cluster, elsewhere, as a restore would be: its writes are not training_wheels'.
"$PGBIN/initdb" -D "$T/re" -U postgres -A trust --no-sync >/dev/null
"$PGBIN/pg_ctl" -D "$T/re" -o "-k $T/re.s -c listen_addresses='' -c fsync=off" -l "$T/re.log" -w start >/dev/null
rebuilt() { # <dump>: the count of actions it restores to, with --no-owner, as for a dump alone
  local db="r$RANDOM"
  "$PGBIN/psql" -X -q -h "$T/re.s" -U postgres -d postgres -c "create database $db" &&
    "$PGBIN/pg_restore" --no-owner -h "$T/re.s" -U postgres -d "$db" "$1" &&
    "$PGBIN/psql" -X -q -At -h "$T/re.s" -U postgres -d "$db" -c "select count(*) from action"
}
q "create role door login; create role tw_owner nologin"
q "create table action (id bigserial primary key, route text, args jsonb); insert into action (route, args) select '/v2/apply', jsonb_build_object('n', g) from generate_series(1, 500) g; alter table action owner to door"
q "vacuum" && q "checkpoint"

seal() { printf 'aa%062d' "$1"; }
for i in 1 2; do head -c 2000 /dev/urandom > "$T/seals/$(seal $i)"; done
printf '{"sealed":"row-1"}\n' > "$T/spool/live.ndjson"

# run <now> [full], as backup-door.service runs it, the cluster reached as its peer would be.
run() {
  env BACKUP_DEST_DIR="$T/dest" BACKUP_ENV="$T/backup.env" STATE_DIRECTORY="$T/state" RUNTIME_DIRECTORY="$T/run" \
    BACKUP_SEALS="$T/seals" BACKUP_TW_SPOOL="$T/spool" BACKUP_ROOT_HOME="$T/home" BACKUP_BOX=door-01 BACKUP_NOW="$1" \
    BACKUP_PG_AS= BACKUP_PGBIN="$PGBIN" PGHOST="$T" PGUSER=postgres ${BACKUP_TW:+BACKUP_TW=$BACKUP_TW} \
    bash "$HERE/backup.sh" door ${2:-}
}
VPY=${VERIFY_PY:-$HERE/../../../../.venv/bin/python}
[ -x "$VPY" ] || VPY=python3
V() { BACKUP_DEST_DIR="$T/dest" "$VPY" "$HERE/backup-verify.py" "$@"; }
st() { echo "$1" | tr -d ':-'; }
objs() { ls "$T/dest/door-01/$(st "$1")" 2>/dev/null | tr "\n" " "; }
ticks() { ls "$T/dest/door-01" 2>/dev/null | wc -l | tr -d ' '; }
mq() { age -d -i "$T/id.txt" "$T/dest/door-01/$(st "$1")/manifest.json.age" | python3 -c "import json,sys; m=json.load(sys.stdin); d={e['name']: e for e in m['files']}; print($2)"; }
dump_of() { age -d -i "$T/id.txt" "$T/dest/door-01/$(st "$1")/training_wheels.dump.age"; }
SEALS2="$(seal 1).age $(seal 2).age"
TWSET="training_wheels.dump.age training_wheels.globals.sql.age"
globals_of() { age -d -i "$T/id.txt" "$T/dest/door-01/$(st "$1")/training_wheels.globals.sql.age"; }
# A rebuild as § 1b does it, from one tick's set: a fresh cluster, the roles, then the dump with
# its owners (no --no-owner). Answers "<actions> <the table's owner>".
fresh_rebuild() {
  local f=(-X -q -h "$T/fresh.s" -U postgres)
  rm -rf "$T/fresh" "$T/fresh.s" && mkdir -p "$T/fresh.s"
  "$PGBIN/initdb" -D "$T/fresh" -U postgres -A trust --no-sync >/dev/null &&
    "$PGBIN/pg_ctl" -D "$T/fresh" -o "-k $T/fresh.s -c listen_addresses=''" -l "$T/fresh.log" -w start >/dev/null || return 1
  # The roles: "role postgres already exists" is expected, and the rest is made.
  globals_of "$1" | "$PGBIN/psql" "${f[@]}" -d postgres >/dev/null 2>&1
  dump_of "$1" > "$T/fresh.dump"
  "$PGBIN/psql" "${f[@]}" -d postgres -c "create database training_wheels" &&
    "$PGBIN/pg_restore" -h "$T/fresh.s" -U postgres -d training_wheels "$T/fresh.dump" &&
    "$PGBIN/psql" "${f[@]}" -At -d training_wheels -c "select (select count(*) from action) || ' ' || tableowner from pg_tables where tablename = 'action'"
  "$PGBIN/pg_ctl" -D "$T/fresh" -m immediate stop >/dev/null
}

echo "== the first tick: full, the dump and the spool beside the seals"
D1=2026-09-29T10:05:00Z
ok "door runs" 'run $D1 > "$T/d1.log" 2>&1'
sed 's/^/    /' "$T/d1.log"
ok "the tick holds the seals, the dump, the roles, the spool and a manifest" '[ "$(objs $D1)" = "$SEALS2 manifest.json.age $TWSET tw-spool.live.ndjson.age " ]'
ok "the dump is a pg_dump archive (PGDMP)" '[ "$(dump_of $D1 | head -c 5)" = PGDMP ]'
ok "and rebuilds the database elsewhere: 500 actions" 'dump_of $D1 > "$T/tw.dump" && [ "$(rebuilt "$T/tw.dump")" = 500 ]'
ok "the roles, without passwords" 'globals_of $D1 | grep -q "CREATE ROLE door;" && ! globals_of $D1 | grep -qi "password"'
ok "and a rebuild is the roles, then the dump, owners and all, in a fresh cluster" '[ "$(fresh_rebuild $D1)" = "500 door" ]'
ok "the spool goes up as it is" 'cmp -s <(age -d -i "$T/id.txt" "$T/dest/door-01/$(st $D1)/tw-spool.live.ndjson.age") "$T/spool/live.ndjson"'
ok "the manifest names the dump by its format and the WAL position it was taken at" '[ "$(mq $D1 "d[\"training_wheels.dump\"][\"format\"]")" = "pg_dump -Fc" ] && mq $D1 "d[\"training_wheels.dump\"][\"lsn\"]" | grep -qE "^[0-9A-F]+/[0-9A-F]+$"'
ok "backup-verify checks it, the dump's and the roles' headers included" 'V check door-01 latest -i "$T/id.txt" > "$T/v1.log" 2>&1 && grep -q "training_wheels.dump .*pg_dump archive" "$T/v1.log" && grep -q "training_wheels.globals.sql .*roles" "$T/v1.log"'
ok "no plaintext left, the ledger alone in the StateDirectory" '[ -z "$(ls -A "$T/run")" ] && [ "$(ls -A "$T/state")" = ledger.json ]'
ok "the ledger holds no content" 'python3 -c "
import json,sys
for e in json.load(open(sys.argv[1]))[\"stores\"].values():
    assert set(e) <= {\"name\",\"size\",\"sha256\",\"key\",\"copied\",\"integrity\",\"rows\",\"format\",\"lsn\"}, e
" "$T/state/ledger.json"'

echo "== nothing written: nothing sent, though a new dump's bytes would differ"
D2=2026-09-29T10:20:00Z
ok "unchanged" 'run $D2 > "$T/d2.log" 2>&1 && grep -q "nothing changed (5 stores" "$T/d2.log" && [ "$(ticks)" = 1 ]'

echo "== a write moves the WAL: the dump goes up, the rest stays"
q "insert into action (route, args) values ('/v2/mint', '{}')"
D3=2026-09-29T10:35:00Z
ok "the dump with its roles, and a manifest" 'run $D3 > "$T/d3.log" 2>&1 && [ "$(objs $D3)" = "manifest.json.age $TWSET " ]'
ok "it holds the new row" 'dump_of $D3 > "$T/tw3.dump" && [ "$(rebuilt "$T/tw3.dump")" = 501 ]'
ok "the manifest names the older seals and spool, the new dump and roles" '[ "$(mq $D3 "\" \".join(sorted(k + \"@\" + e[\"key\"].split(\"/\")[1] for k, e in d.items()))")" = "$(seal 1)@$(st $D1) $(seal 2)@$(st $D1) training_wheels.dump@$(st $D3) training_wheels.globals.sql@$(st $D3) tw-spool.live.ndjson@$(st $D1)" ]'

echo "== the spool grows: it goes up"
printf '{"sealed":"row-2"}\n' >> "$T/spool/live.ndjson"
D4=2026-09-29T10:50:00Z
ok "the spool alone, and a manifest" 'run $D4 > /dev/null 2>&1 && [ "$(objs $D4)" = "manifest.json.age tw-spool.live.ndjson.age " ]'

echo "== the day's first tick: every store, the dump taken again"
D5=2026-09-30T00:05:00Z
ok "full, with a fresh dump and roles" 'run $D5 > "$T/d5.log" 2>&1 && grep -q "full (the day.s first)" "$T/d5.log" && [ "$(objs $D5)" = "$SEALS2 manifest.json.age $TWSET tw-spool.live.ndjson.age " ]'

echo "== PostgreSQL down: the seals still go up, the manifest names the last dump, and the run fails"
"$PGBIN/pg_ctl" -D "$T/pg" -m fast -w stop >/dev/null
head -c 2000 /dev/urandom > "$T/seals/$(seal 3)"
D6=2026-09-30T00:20:00Z
ok "the run fails" '! run $D6 > "$T/d6.log" 2>&1'
sed 's/^/    /' "$T/d6.log"
ok "naming training_wheels" 'grep -q "training_wheels could not be dumped" "$T/d6.log"'
ok "the new seal went up, with a manifest" '[ "$(objs $D6)" = "$(seal 3).age manifest.json.age " ]'
ok "the manifest names the last dump and roles, from their own tick" '[ "$(mq $D6 "d[\"training_wheels.dump\"][\"key\"].split(\"/\")[1] + d[\"training_wheels.globals.sql\"][\"key\"].split(\"/\")[1]")" = "$(st $D5)$(st $D5)" ]'
ok "and checks" 'V check door-01 latest -i "$T/id.txt" > /dev/null 2>&1'
ok "no plaintext left" '[ -z "$(ls -A "$T/run")" ]'

echo "== PostgreSQL back: the next tick dumps again"
pg_up
D7=2026-09-30T00:35:00Z
ok "the dump and roles go up" 'run $D7 > /dev/null 2>&1 && objs $D7 | grep -q "$TWSET"'

echo "== 1,300 seals and a RuntimeDirectory that cannot hold them all at once (run 74's defect)"
# door-test, 29 Sep: 1,284 seals, 733 MiB, against /run's 793 MiB tmpfs and the dump: nothing went
# up. Here 1,300 seals of 12 KiB (15 MiB) against an 8 MiB RuntimeDirectory, a fresh ledger.
mkdir -p "$T/many" "$T/state.many" && chmod 700 "$T/state.many"
for i in $(seq 1 1300); do head -c 12288 /dev/urandom > "$T/many/$(seal $((1000 + i)))"; done
ok "an 8 MiB RuntimeDirectory" 'small_fs "$T/small.run" 8'
D9=2026-09-30T01:05:00Z
ok "the tick runs" 'env BACKUP_DEST_DIR="$T/dest" BACKUP_ENV="$T/backup.env" STATE_DIRECTORY="$T/state.many" RUNTIME_DIRECTORY="$T/small.run" BACKUP_SEALS="$T/many" BACKUP_TW_SPOOL="$T/spool" BACKUP_ROOT_HOME="$T/home" BACKUP_BOX=door-02 BACKUP_NOW=$D9 BACKUP_PG_AS= BACKUP_PGBIN="$PGBIN" PGHOST="$T" PGUSER=postgres bash "$HERE/backup.sh" door > "$T/d9.log" 2>&1'
sed 's/^/    /' "$T/d9.log" | tail -3
ok "every seal went up, with the dump, the roles, the spool and a manifest" '[ "$(ls "$T/dest/door-02/$(st $D9)" | grep -c "^aa")" = 1300 ] && ls "$T/dest/door-02/$(st $D9)" | grep -q "training_wheels.dump.age" && ls "$T/dest/door-02/$(st $D9)" | grep -q "training_wheels.globals.sql.age" && ls "$T/dest/door-02/$(st $D9)" | grep -q "manifest.json.age"'
ok "and it checks" 'V check door-02 latest -i "$T/id.txt" > "$T/v9.log" 2>&1 && grep -q "1303 files\|1304 files" "$T/v9.log"'
ok "nothing left in the RuntimeDirectory" '[ -z "$(ls -A "$T/small.run" | grep -v "^\.")" ]'
unmount_small "$T/small.run"

echo "== a dump with no room: the seals still go up, and the run fails naming it"
mkdir -p "$T/few" && for i in 1 2 3; do head -c 2000 /dev/urandom > "$T/few/$(seal $((5000 + i)))"; done
ok "an 8 MiB RuntimeDirectory and a 4 MiB StateDirectory" 'small_fs "$T/small.run2" 8 && small_fs "$T/small.state" 4'
D10=2026-09-30T01:20:00Z
ok "the run fails" '! env BACKUP_DEST_DIR="$T/dest" BACKUP_ENV="$T/backup.env" STATE_DIRECTORY="$T/small.state" RUNTIME_DIRECTORY="$T/small.run2" BACKUP_SEALS="$T/few" BACKUP_TW_SPOOL="$T/spool" BACKUP_ROOT_HOME="$T/home" BACKUP_BOX=door-03 BACKUP_NOW=$D10 BACKUP_PG_AS= BACKUP_PGBIN="$PGBIN" PGHOST="$T" PGUSER=postgres bash "$HERE/backup.sh" door > "$T/d10.log" 2>&1'
sed 's/^/    /' "$T/d10.log" | tail -3
ok "naming training_wheels and the room" 'grep -q "training_wheels could not be dumped" "$T/d10.log" && grep -q "room" "$T/d10.log"'
ok "the seals went up, with a manifest" '[ "$(ls "$T/dest/door-03/$(st $D10)" 2>/dev/null | grep -c "^aa")" = 3 ] && ls "$T/dest/door-03/$(st $D10)" | grep -q "manifest.json.age"'
ok "and it checks" 'V check door-03 latest -i "$T/id.txt" > /dev/null 2>&1'
unmount_small "$T/small.run2"; unmount_small "$T/small.state"

echo "== a Door without training_wheels: the seals alone, as before"
rm -rf "$T/spool" "$T/state/ledger.json"
D8=2026-09-30T00:50:00Z
ok "runs, and names neither" 'BACKUP_TW=off run $D8 > "$T/d8.log" 2>&1 && [ "$(objs $D8)" = "$(seal 1).age $(seal 2).age $(seal 3).age manifest.json.age " ]'

echo "== $pass passed, $failn failed"
[ "$failn" = 0 ]
