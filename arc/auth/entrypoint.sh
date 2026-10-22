#!/bin/sh
# Restore-if-absent, then run the service under Litestream.
#
# THE RESTORE IS THE POINT. A fresh container with an empty volume is
# indistinguishable from a rebuilt box, and in both cases the right move is to
# pull the database back from R2 rather than start empty and silently mint new
# accounts for returning users. -if-replica-exists makes the genuine first-ever
# boot (no replica yet) a no-op instead of a failure.
set -e

DB="${DB_PATH:-/data/auth.db}"

if [ ! -f "$DB" ]; then
  echo "entrypoint: no database at $DB — restoring from replica if one exists"
  litestream restore -if-replica-exists -config /etc/litestream.yml "$DB"
  if [ -f "$DB" ]; then
    echo "entrypoint: restored $DB from R2"
  else
    echo "entrypoint: no replica found — starting with an empty database"
  fi
fi

# -exec makes Litestream the supervisor: it replicates until uvicorn exits, then
# exits with it, so the container's lifetime and replication's are the same.
# Running uvicorn separately would let the service outlive its own backups.
exec litestream replicate -config /etc/litestream.yml \
  -exec "uvicorn app.main:app --host 0.0.0.0 --port 8000 --workers 1"
