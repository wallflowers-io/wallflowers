"""A database from the OIDC generation must open, not raise.

THIS IS THE TEST THAT DID NOT EXIST, and its absence cost two failed production
deploys on 18 September 2026. Every other test here starts from `:memory:`, so
every schema they exercise is a FRESH one — and the schema was fine. What was
never exercised was the upgrade: a real database, written by the previous
generation of this service, opened by this one.

The two failures, in order, because each hid the next:

  1. `no such column: identity_pk` — `CREATE TABLE IF NOT EXISTS sites` skipped
     the old `sites(uid, …)`, and the index after it had nothing to index.
  2. `FOREIGN KEY constraint failed` — the old `sites` holds a reference to
     `users`, which `_DEAD_TABLES` drops, so removing the collision by renaming
     the table only moved the failure.

Neither message named the cause. Both read as a bug in db.py rather than as a
table left over from a schema nothing runs any more.
"""
from __future__ import annotations

import sqlite3

from app.db import Store

# Exactly the shape found on the box: `sites` keyed on a server-minted uid, with
# the foreign key that made the second failure.
OIDC = """
CREATE TABLE users (uid TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
CREATE TABLE identities (
    iss TEXT NOT NULL, sub TEXT NOT NULL,
    uid TEXT NOT NULL REFERENCES users(uid), PRIMARY KEY (iss, sub));
CREATE TABLE sites (
    slug TEXT PRIMARY KEY,
    uid  TEXT REFERENCES users(uid),
    created_at INTEGER NOT NULL);
CREATE TABLE passkeys (credential_id TEXT PRIMARY KEY, handle TEXT NOT NULL);
-- The one that broke the THIRD deploy: _DEAD_TABLES drops `users` before
-- `identities`, and `identities` references it. Present here so the fixture can
-- fail the way production did.
CREATE TABLE refresh_tokens (
    uid TEXT NOT NULL REFERENCES users(uid),
    provider TEXT NOT NULL, PRIMARY KEY (uid, provider));
"""


def _oidc_db(tmp_path):
    p = tmp_path / "auth.db"
    db = sqlite3.connect(p)
    db.executescript(OIDC)
    db.execute("INSERT INTO users VALUES ('u1', 1)")
    db.execute("INSERT INTO identities VALUES ('iss', 'sub', 'u1')")
    db.execute("INSERT INTO refresh_tokens VALUES ('u1', 'apple')")
    db.execute("INSERT INTO sites VALUES ('taken', 'u1', 1)")
    db.commit()
    db.close()
    return str(p)


def test_an_oidc_database_upgrades_instead_of_raising(tmp_path):
    path = _oidc_db(tmp_path)
    Store(path).close()          # the whole assertion: it opens

    db = sqlite3.connect(path)
    cols = {r[1] for r in db.execute("PRAGMA table_info(sites)")}
    assert "identity_pk" in cols and "uid" not in cols, (
        f"sites is still the OIDC table: {sorted(cols)}"
    )
    live = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    # D4, 13 Sep 2026: nothing is carried across, because no OIDC row has an
    # Ed25519 key and a browser cannot mint one for it.
    assert not (live & {"users", "identities", "passkeys", "refresh_tokens"}), (
        f"dead tables survive: {sorted(live)}"
    )
    assert {"accounts", "wraps", "heads", "histories", "sites"} <= live


def test_upgrading_twice_is_not_an_error(tmp_path):
    """It runs on every boot, so it has to be safe on every boot — including the
    boot after the one that did the work."""
    path = _oidc_db(tmp_path)
    Store(path).close()
    Store(path).close()
    db = sqlite3.connect(path)
    assert {r[1] for r in db.execute("PRAGMA table_info(sites)")} >= {"slug", "identity_pk"}


def test_a_fresh_database_is_untouched_by_the_upgrade(tmp_path):
    p = str(tmp_path / "fresh.db")
    Store(p).close()
    db = sqlite3.connect(p)
    assert {r[1] for r in db.execute("PRAGMA table_info(sites)")} >= {"slug", "identity_pk"}
