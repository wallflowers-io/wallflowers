#!/usr/bin/env bash
# backup.test.sh — K-48's backup.sh and backup-verify.py for both roles, against scratch copies
# (srr/scmp.md § 11): change-only ticks, the ledger, composed manifests, the daily and
# pre-deploy fulls, a run that fails after sending, the refusals, and the restore check. The
# suite 3f7d875's run was measured by (57 cases), made portable; training_wheels' part of the
# door role is backup-door-tw.test.sh's, and is off here.
#
#   arc/hosting/backup/backup.test.sh
#
# Needs age and age-keygen on PATH (or AGE_BIN, a directory holding both); docker, with an
# image shaped like kenjin-auth (BACKUP_TEST_AUTH_IMAGE, else kenjin-auth:d57-seals) for the
# account store; for backup-verify.py a Python of 3.10 or later (VERIFY_PY, else the
# workspace's .venv). Nothing leaves the machine. Exits nonzero on any FAIL.
set -uo pipefail
B=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
[ -n "${AGE_BIN:-}" ] && PATH="$AGE_BIN:$PATH"
PY=${VERIFY_PY:-$B/../../../../.venv/bin/python}
[ -x "$PY" ] || PY=python3
AUTH_IMAGE=${BACKUP_TEST_AUTH_IMAGE:-kenjin-auth:d57-seals}
AUTH=bk-auth-test-$$
for c in age age-keygen docker; do command -v "$c" >/dev/null || { echo "needs $c"; exit 2; }; done
docker image inspect "$AUTH_IMAGE" >/dev/null 2>&1 || { echo "needs the image $AUTH_IMAGE (BACKUP_TEST_AUTH_IMAGE)"; exit 2; }
AGE=$(command -v age)
T=$(mktemp -d "${TMPDIR:-/tmp}/backup-test.XXXXXX")
trap 'docker rm -f "$AUTH" >/dev/null 2>&1; rm -rf "$T"' EXIT
mkdir -p "$T"/{state,seals,dest,run,arcwork,doorstate,home,failbin}; chmod 700 "$T/run" "$T/arcwork" "$T/doorstate"
pass=0; failn=0
ok() { if eval "$2"; then echo "  PASS  $1"; pass=$((pass+1)); else echo "  FAIL  $1"; failn=$((failn+1)); fi; }

age-keygen -o "$T/id.txt" 2>/dev/null
REC=$(grep -o 'age1[0-9a-z]*' "$T/id.txt" | head -1)
printf 'BACKUP_R2_ENDPOINT=https://example.r2.cloudflarestorage.com\nBACKUP_R2_BUCKET="wallflowers-backup"\nBACKUP_R2_ACCESS_KEY_ID=x\nBACKUP_R2_SECRET_ACCESS_KEY=y\nBACKUP_AGE_RECIPIENT=%s\n' "$REC" > "$T/backup.env"
chmod 600 "$T/backup.env"

# The Arc's state: four databases with rows, its key and its routes.
for db in relay pacific waker boxoffice; do
  "$PY" - "$T/state/$db.db" "$db" <<'EOF'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1]); n = len(sys.argv[2])
c.execute("PRAGMA journal_mode=WAL")
c.execute("CREATE TABLE blob (tag TEXT, seq INTEGER, body BLOB)")
c.execute('CREATE TABLE "odd ""name""" (x)')
c.executemany("INSERT INTO blob VALUES (?,?,?)", [(f"t{i}", i, b"x" * 100) for i in range(50 * n)])
c.executemany('INSERT INTO "odd ""name""" VALUES (?)', [(i,) for i in range(n)])
c.commit()
EOF
done
head -c 64 /dev/urandom > "$T/state/id_ed25519"; echo "ws://127.0.0.1:8787/v1/relay" > "$T/state/routes"
write() { "$PY" -c 'import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute("INSERT INTO blob VALUES (?,?,?)", (sys.argv[2], 0, b"y")); c.commit()' "$T/state/$1" "$2"; }

# The account store, in a container shaped like kenjin-auth.
# No network (NC-86): nothing in it may reach anything, whatever an edit later runs there.
docker run -d --network none --name "$AUTH" --entrypoint sleep "$AUTH_IMAGE" infinity >/dev/null
docker exec "$AUTH" sh -c 'mkdir -p /data && python -c "
import sqlite3; c=sqlite3.connect(\"/data/auth.db\"); c.execute(\"CREATE TABLE users (uid, pk)\"); c.execute(\"CREATE TABLE seal_counter (node, n)\")
c.executemany(\"INSERT INTO users VALUES (?,?)\", [(i, str(i)) for i in range(7)]); c.execute(\"INSERT INTO seal_counter VALUES (1, 3)\"); c.commit()"'

# Seals: three people's, a lock and one being written.
seal() { printf 'aa%062d' "$1"; }
for i in 1 2 3; do head -c 2000 /dev/urandom > "$T/seals/$(seal $i)"; done
touch "$T/seals/$(seal 1).lock"; head -c 10 /dev/urandom > "$T/seals/$(seal 2).next"

# run <box> <role> <now> [full]: arc on its StateDirectory, beside its sources; door on its
# RuntimeDirectory, with its StateDirectory for the ledger.
run() {
  local dirs=(STATE_DIRECTORY="$T/arcwork" RUNTIME_DIRECTORY=)
  [ "$2" = door ] && dirs=(STATE_DIRECTORY="$T/doorstate" RUNTIME_DIRECTORY="$T/run")
  env BACKUP_TW=off BACKUP_DEST_DIR="$T/dest" BACKUP_ENV="$T/backup.env" "${dirs[@]}" BACKUP_ARC_STATE="$T/state" BACKUP_ROOT_HOME="$T/home" \
    BACKUP_AUTH_CONTAINER="$AUTH" BACKUP_SEALS="$T/seals" BACKUP_BOX="$1" BACKUP_NOW="$3" bash "$B/backup.sh" "$2" ${4:-}
}
V() { BACKUP_DEST_DIR="$T/dest" "$PY" "$B/backup-verify.py" "$@"; }
st() { echo "$1" | tr -d ':-'; }                      # a NOW's stamp
objs() { ls "$T/dest/$1/$(st "$2")" 2>/dev/null | tr "\n" " "; }
ticks() { ls "$T/dest/$1" | wc -l | tr -d ' '; }
man() { age -d -i "$T/id.txt" "$T/dest/$1/$(st "$2")/manifest.json.age"; }   # the manifest, decrypted
mq() { man "$1" "$2" | "$PY" -c "import json,sys; m=json.load(sys.stdin); print($3)"; }
ARCALL="auth.db.age boxoffice.db.age id_ed25519.age manifest.json.age pacific.db.age relay.db.age routes.age waker.db.age "

echo "== the first ticks: no ledger, so full"
A1=2026-09-27T10:00:00Z; D1=2026-09-27T10:05:00Z
ok "arc runs" 'run kenjin-01 arc $A1 > "$T/a1.log" 2>&1'
ok "door runs" 'run door-01 door $D1 > "$T/d1.log" 2>&1'
sed 's/^/    /' "$T/a1.log" "$T/d1.log"
ok "no ledger said, and a full set" 'grep -q "full (no ledger)" "$T/a1.log" && [ "$(mq kenjin-01 $A1 "m[\"full\"]")" = True ]'
ok "the arc tick holds auth, four databases, the key and routes, and a manifest" '[ "$(objs kenjin-01 $A1)" = "$ARCALL" ]'
ok "the door tick holds the three seals, no .lock or .next" '[ "$(objs door-01 $D1 | tr " " "\n" | grep -c "^aa")" = 3 ] && ! objs door-01 $D1 | grep -qE "lock|next"'
ok "no plaintext left in any work directory" '[ -z "$(ls -A "$T/run")" ] && [ "$(ls -A "$T/arcwork")" = ledger.json ]'
ok "the door StateDirectory holds the ledger alone" '[ "$(ls -A "$T/doorstate")" = ledger.json ]'
ok "the ledgers are 0600" '[ "$(stat -f %Lp "$T/arcwork/ledger.json")" = 600 ] && [ "$(stat -f %Lp "$T/doorstate/ledger.json")" = 600 ]'
ok "a ledger holds names, hashes, sizes, keys, copy times and counts: no content" '"$PY" -c "
import json,sys
for p in sys.argv[1:]:
    l=json.load(open(p))
    for e in l[\"stores\"].values():
        assert set(e) <= {\"name\",\"size\",\"sha256\",\"key\",\"copied\",\"integrity\",\"rows\"}, e
" "$T/arcwork/ledger.json" "$T/doorstate/ledger.json"'
ok "every object is age ciphertext" '! find "$T/dest" -type f ! -name "*.age" | grep -q . && for f in $(find "$T/dest" -type f); do head -c 21 "$f" | grep -q "age-encryption.org/v1" || exit 1; done'
ok "the auth backup left nothing in its container" '[ -z "$(docker exec "$AUTH" ls /dev/shm)" ]'
ok "the arc manifest checks" 'V check kenjin-01 latest -i "$T/id.txt" > "$T/v1.log" 2>&1'
ok "the door manifest checks" 'V check door-01 latest -i "$T/id.txt" > "$T/v2.log" 2>&1'
sed 's/^/    /' "$T/v1.log" "$T/v2.log"
ok "the auth rows came through" 'grep -q "auth.db .* 8 rows in 2 tables" "$T/v1.log"'
ok "a database with a quoted table name counts" 'grep -q "relay.db .* 255 rows in 2 tables" "$T/v1.log"'

echo "== unchanged: nothing goes up"
A2=2026-09-27T10:15:00Z; D2=2026-09-27T10:20:00Z
L=$(cat "$T/arcwork/ledger.json")
ok "arc, unchanged" 'run kenjin-01 arc $A2 > "$T/a2.log" 2>&1 && grep -q "nothing changed (7 stores" "$T/a2.log"'
ok "door, unchanged" 'run door-01 door $D2 > "$T/d2.log" 2>&1 && grep -q "nothing changed (3 stores" "$T/d2.log"'
ok "no tick written, and the ledger as it was" '[ "$(ticks kenjin-01)" = 1 ] && [ "$(ticks door-01)" = 1 ] && [ "$(cat "$T/arcwork/ledger.json")" = "$L" ]'
ok "and no plaintext left" '[ -z "$(ls -A "$T/run")" ] && [ "$(ls -A "$T/arcwork")" = ledger.json ]'

echo "== one store changed: it, and a manifest"
A3=2026-09-27T10:30:00Z; A4=2026-09-27T10:45:00Z; A5=2026-09-27T11:00:00Z
write relay.db r1
ok "relay.db changed" 'run kenjin-01 arc $A3 > "$T/a3.log" 2>&1 && [ "$(objs kenjin-01 $A3)" = "manifest.json.age relay.db.age " ]'
sed 's/^/    /' "$T/a3.log"
write waker.db w1
ok "waker.db changed" 'run kenjin-01 arc $A4 > /dev/null 2>&1 && [ "$(objs kenjin-01 $A4)" = "manifest.json.age waker.db.age " ]'
docker exec "$AUTH" python -c 'import sqlite3; c=sqlite3.connect("/data/auth.db"); c.execute("UPDATE seal_counter SET n = 4"); c.commit()'
ok "the account store changed" 'run kenjin-01 arc $A5 > /dev/null 2>&1 && [ "$(objs kenjin-01 $A5)" = "auth.db.age manifest.json.age " ]'
ok "the manifest names every store, not full" '[ "$(mq kenjin-01 $A5 "len(m[\"files\"]), m[\"full\"]")" = "7 False" ]'
ok "each store at its newest object, with that copy's time" '[ "$(mq kenjin-01 $A5 "\" \".join(e[\"name\"]+\"@\"+e[\"key\"].split(\"/\")[1] for e in m[\"files\"])")" = "auth.db@$(st $A5) boxoffice.db@$(st $A1) id_ed25519@$(st $A1) pacific.db@$(st $A1) relay.db@$(st $A3) routes@$(st $A1) waker.db@$(st $A4)" ] && [ "$(mq kenjin-01 $A5 "[e[\"copied\"] for e in m[\"files\"] if e[\"name\"]==\"relay.db\"][0]")" = "$(mq kenjin-01 $A3 "[e[\"copied\"] for e in m[\"files\"] if e[\"name\"]==\"relay.db\"][0]")" ]'

echo "== a restore from a manifest composed of four ticks"
ok "it checks" 'V check kenjin-01 latest -i "$T/id.txt" -o "$T/restore" > "$T/v3.log" 2>&1 && grep -q "7 files from 4 ticks hold to the manifest" "$T/v3.log"'
sed 's/^/    /' "$T/v3.log"
ok "and restores what the box holds now" 'for f in id_ed25519 routes; do cmp -s "$T/state/$f" "$T/restore/$f" || exit 1; done && "$PY" -c "
import sqlite3,sys
d=lambda p: list(sqlite3.connect(p).iterdump())
for db in (\"relay.db\",\"pacific.db\",\"waker.db\",\"boxoffice.db\"):
    assert d(sys.argv[1]+\"/\"+db)==d(sys.argv[2]+\"/\"+db), db
" "$T/state" "$T/restore" && [ "$(docker exec "$AUTH" python -c "import sqlite3; print(list(sqlite3.connect(\"/data/auth.db\").iterdump()))")" = "$("$PY" -c "import sqlite3; print(list(sqlite3.connect(\"$T/restore/auth.db\").iterdump()))")" ]'
ok "the restore is 0700" '[ "$(stat -f %Lp "$T/restore")" = 700 ]'
rm -rf "$T/restore"
ok "an earlier manifest restores its own time" 'V check kenjin-01 $(st $A3) -i "$T/id.txt" > "$T/v4.log" 2>&1 && grep -q "7 files from 2 ticks" "$T/v4.log"'

echo "== seals come and go"
D3=2026-09-27T11:05:00Z; D4=2026-09-27T11:20:00Z
head -c 2000 /dev/urandom > "$T/seals/$(seal 4)"
ok "a new seal: it, and a manifest" 'run door-01 door $D3 > /dev/null 2>&1 && [ "$(objs door-01 $D3)" = "$(seal 4).age manifest.json.age " ]'
rm "$T/seals/$(seal 2)"
ok "a seal gone: a manifest alone, without it" 'run door-01 door $D4 > "$T/d4.log" 2>&1 && [ "$(objs door-01 $D4)" = "manifest.json.age " ] && [ "$(mq door-01 $D4 "len(m[\"files\"])")" = 3 ] && ! mq door-01 $D4 "[e[\"name\"] for e in m[\"files\"]]" | grep -q "$(seal 2)"'
ok "that manifest checks" 'V check door-01 latest -i "$T/id.txt" > "$T/v5.log" 2>&1 && grep -q "3 files from 2 ticks" "$T/v5.log"'
ok "a seal manifest pairs with the newest arc manifest before it" '[ "$(V pair door-01 latest kenjin-01)" = "door-01/$(st $D4) restores with kenjin-01/$(st $A5)" ]'
ok "and the first with the first" '[ "$(V pair door-01 $(st $D1) kenjin-01)" = "door-01/$(st $D1) restores with kenjin-01/$(st $A1)" ]'

echo "== the day's first tick: full"
A6=2026-09-28T00:00:00Z
ok "after 00:00 UTC, every store" 'run kenjin-01 arc $A6 > "$T/a6.log" 2>&1 && grep -q "full (the day.s first)" "$T/a6.log" && [ "$(objs kenjin-01 $A6)" = "$ARCALL" ] && [ "$(mq kenjin-01 $A6 "m[\"full\"]")" = True ]'
A7=2026-09-28T00:15:00Z
ok "and the next tick is change-only again" 'run kenjin-01 arc $A7 > "$T/a7.log" 2>&1 && grep -q "nothing changed" "$T/a7.log"'

echo "== a deploy's full"
A8=2026-09-28T00:30:00Z; A9=2026-09-28T00:45:00Z
ok "backup.sh arc full: every store, unchanged or not" 'run kenjin-01 arc $A8 full > "$T/a8.log" 2>&1 && grep -q "full (asked)" "$T/a8.log" && [ "$(objs kenjin-01 $A8)" = "$ARCALL" ]'
touch "$T/arcwork/full.next"
ok "full.next: every store, and the request taken" 'run kenjin-01 arc $A9 > /dev/null 2>&1 && [ "$(objs kenjin-01 $A9)" = "$ARCALL" ] && [ "$(ls -A "$T/arcwork")" = ledger.json ]'
D5=2026-09-28T00:50:00Z
touch "$T/doorstate/full.next"
ok "the door's full.next too" 'run door-01 door $D5 > /dev/null 2>&1 && [ "$(objs door-01 $D5 | tr " " "\n" | grep -c "^aa")" = 3 ] && [ "$(ls -A "$T/doorstate")" = ledger.json ]'

echo "== the ledger lost: full, never a skip"
A10=2026-09-28T01:00:00Z; A11=2026-09-28T01:15:00Z
rm "$T/arcwork/ledger.json"
ok "no ledger: every store" 'run kenjin-01 arc $A10 > "$T/a10.log" 2>&1 && grep -q "full (no ledger)" "$T/a10.log" && [ "$(objs kenjin-01 $A10)" = "$ARCALL" ]'
echo '{"stores": ' > "$T/arcwork/ledger.json"
ok "an unreadable ledger: every store" 'run kenjin-01 arc $A11 > "$T/a11.log" 2>&1 && grep -q "full (the ledger is unreadable" "$T/a11.log" && [ "$(objs kenjin-01 $A11)" = "$ARCALL" ]'

echo "== a run that fails after sending"
A12=2026-09-28T01:30:00Z; A13=2026-09-28T01:45:00Z
cat > "$T/failbin/age" <<SH
#!/bin/sh
# age, failing at the manifest.
case "\$*" in *manifest.json.age*) echo "age: refused" >&2; exit 1 ;; esac
exec "$AGE" "\$@"
SH
chmod +x "$T/failbin/age"
write pacific.db p1
L=$(cat "$T/arcwork/ledger.json")
touch "$T/arcwork/full.next"
ok "fails at the manifest" '! PATH="$T/failbin:$PATH" run kenjin-01 arc $A12 > "$T/a12.log" 2>&1'
ok "no plaintext left, and the ledger as it was" '[ "$(cat "$T/arcwork/ledger.json")" = "$L" ] && [ -z "$(find "$T/arcwork" -name "set.*")" ]'
ok "the deploy's request put back" '[ -e "$T/arcwork/full.next" ] && [ ! -e "$T/arcwork/full.taken" ]' && rm "$T/arcwork/full.next"
ok "that tick is unfinished, and latest skips it" 'V sets kenjin-01 | grep -q "$(st $A12)  \*" && V check kenjin-01 latest -i "$T/id.txt" | grep -q "$(st $A11)"'
ok "the next tick sends pacific.db again" 'run kenjin-01 arc $A13 > /dev/null 2>&1 && [ "$(objs kenjin-01 $A13)" = "manifest.json.age pacific.db.age " ] && V check kenjin-01 latest -i "$T/id.txt" > /dev/null 2>&1'

echo "== refusals"
chmod 644 "$T/backup.env"
ok "an env file others may read is refused" '! run door-01 door 2026-09-28T02:00:00Z > "$T/r.log" 2>&1 && grep -q "mode 644" "$T/r.log"'
chmod 600 "$T/backup.env"
grep -v AGE "$T/backup.env" > "$T/e2"; chmod 600 "$T/e2"
ok "no recipient is refused" '! BACKUP_ENV="$T/e2" BACKUP_DEST_DIR="$T/dest" STATE_DIRECTORY="$T/doorstate" RUNTIME_DIRECTORY="$T/run" BACKUP_ROOT_HOME="$T/home" BACKUP_SEALS="$T/seals" bash "$B/backup.sh" door > "$T/r.log" 2>&1 && grep -q "BACKUP_AGE_RECIPIENT" "$T/r.log"'
ok "no age on PATH is refused" '! PATH=/usr/bin:/bin BACKUP_ENV="$T/backup.env" BACKUP_DEST_DIR="$T/dest" STATE_DIRECTORY="$T/doorstate" RUNTIME_DIRECTORY="$T/run" BACKUP_ROOT_HOME="$T/home" BACKUP_SEALS="$T/seals" bash "$B/backup.sh" door > "$T/r.log" 2>&1 && grep -q "age is not on PATH" "$T/r.log"'
ok "the door role without a StateDirectory is refused" '! BACKUP_ENV="$T/backup.env" BACKUP_DEST_DIR="$T/dest" BACKUP_SEALS="$T/seals" BACKUP_ROOT_HOME="$T/home" RUNTIME_DIRECTORY="$T/run" STATE_DIRECTORY= bash "$B/backup.sh" door > "$T/r.log" 2>&1 && grep -q "no StateDirectory" "$T/r.log"'
ok "or without a RuntimeDirectory" '! BACKUP_ENV="$T/backup.env" BACKUP_DEST_DIR="$T/dest" BACKUP_SEALS="$T/seals" BACKUP_ROOT_HOME="$T/home" RUNTIME_DIRECTORY= STATE_DIRECTORY="$T/doorstate" bash "$B/backup.sh" door > "$T/r.log" 2>&1 && grep -q "no RuntimeDirectory" "$T/r.log"'
ok "a stray mode argument is refused" '! run kenjin-01 arc 2026-09-28T02:00:00Z nightly > "$T/r.log" 2>&1 && grep -q usage "$T/r.log"'
mv "$T/state/routes" "$T/routes.away"
n=$(find "$T/dest" -type f | wc -l)
ok "a missing source is refused before anything is sent" '! run kenjin-01 arc 2026-09-28T02:00:00Z > "$T/r.log" 2>&1 && grep -q "routes is missing" "$T/r.log" && [ "$(find "$T/dest" -type f | wc -l)" = "$n" ]'
mv "$T/routes.away" "$T/state/routes"

echo "== the check holds"
S=$(st $A1); S5=$(st $A5)
cp "$T/dest/kenjin-01/$S/routes.age" "$T/routes.keep"; cp "$T/dest/kenjin-01/$S/waker.db.age" "$T/dest/kenjin-01/$S/routes.age"
ok "a swapped object in an earlier tick fails a later manifest" '! V check kenjin-01 $S5 -i "$T/id.txt" > "$T/r.log" 2>&1 && grep -q "routes" "$T/r.log"'
cp "$T/routes.keep" "$T/dest/kenjin-01/$S/routes.age"
cp "$T/dest/kenjin-01/$S/routes.age" "$T/dest/kenjin-01/$S5/routes.age"
ok "an object its own tick's manifest does not name fails" '! V check kenjin-01 $S5 -i "$T/id.txt" > "$T/r.log" 2>&1 && grep -q "does not name" "$T/r.log"'
rm "$T/dest/kenjin-01/$S5/routes.age"
man kenjin-01 $A3 | "$PY" -c "import json,sys; m=json.load(sys.stdin); [e.update(key=e['key'].replace('$(st $A1)', '$(st $A5)')) for e in m['files'] if e['name']=='routes']; print(json.dumps(m))" | age -r "$REC" -o "$T/m.age"
cp "$T/dest/kenjin-01/$(st $A3)/manifest.json.age" "$T/m.keep"; cp "$T/m.age" "$T/dest/kenjin-01/$(st $A3)/manifest.json.age"
ok "a manifest naming a later tick's object fails" '! V check kenjin-01 $(st $A3) -i "$T/id.txt" > "$T/r.log" 2>&1 && grep -q "names .routes." "$T/r.log"'
cp "$T/m.keep" "$T/dest/kenjin-01/$(st $A3)/manifest.json.age"
ok "and restored, it checks again" 'V check kenjin-01 $(st $A3) -i "$T/id.txt" > /dev/null 2>&1'
age-keygen -o "$T/other.txt" 2>/dev/null
ok "another identity cannot read a manifest" '! V check kenjin-01 latest -i "$T/other.txt" > /dev/null 2>&1'

echo "backup test: $pass pass, $failn fail"
[ "$failn" = 0 ]
