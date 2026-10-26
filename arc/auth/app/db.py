"""The account, and everything the Arc keeps for it.

THE KEY IS THE KEY. An account IS an Ed25519 identity public key -- the one the
user owns, derived from a seed they hold and can write down as 24 words. There
is no uid, no issuer, no subject and no handle indirection: those existed
because a passkey could not be an identifier a person owns, so one had to be
invented and handed to the authenticator to remember. The key is the
identifier now, and it is the primary key of every table here.

WHAT THIS SERVICE HOLDS, and the difference between the three blobs:

  · THE WRAP     the 32-byte seed sealed under the passkey's PRF output. Served
                 to ANYONE who asks for it by public key, because that read is
                 how a new device obtains the key it would authenticate with.
                 It is ciphertext whose key never leaves an authenticator.
  · THE HISTORY  the archive and the MLS state, sealed under a key derived from
                 the SEED. Served only against a signature, because whoever
                 opens it holds the conversations. Two slots, so a bad upload
                 lands beside the last good one rather than on top of it.
  · THE HEAD     where the account's archive chain currently ends: a tail hash
                 and a monotonic CHAIN POSITION, sealed under a seed-derived key
                 (`pacific_core::head`). Served to ANYONE who asks, like the
                 wrap and for a different reason -- an anchor nobody can watch
                 is an anchor an arc can move. One row per account, and the only
                 write in this file that can be REFUSED on its content: the
                 position must not go backwards.

That split is why this file no longer has recovery contacts or one-time codes.
They existed to gate the release of a blob that carried everything; the public
half now carries only ciphertext of a key, and the private half is sealed under
something the Arc never sees. There is nothing left for a second factor to
guard, and so no reason to hold anybody's email address or phone number.

sqlite because the row count here is "people who signed up", the writes are one
per registration, and a second service to operate is a worse trade than a file.
It MUST live on a persistent volume: lose this file and every returning user is
a new user -- and every history is gone with them.

Calls are synchronous under a lock. At this volume the event loop pause is
microseconds; if that stops being true this is the file to move behind
asyncio.to_thread, not the place to add a connection pool.
"""
from __future__ import annotations

import os
import sqlite3
import threading
import time

_SCHEMA = """
CREATE TABLE IF NOT EXISTS accounts (
    identity_pk      TEXT PRIMARY KEY,   -- 64 hex: the Ed25519 identity public key
    created_at       INTEGER NOT NULL,
    -- WHICH ARC THIS ACCOUNT IS HOSTED ON. Per account, not per deployment:
    -- anyone can run an Arc, and the same flow registers against whichever one
    -- a person chose.
    arc_url          TEXT,
    -- TOFU-pinned when the account is first hosted. A later mismatch means the
    -- Arc was rebuilt or replaced, and the tether it is waiting for cannot be
    -- the one it was promised.
    arc_identity_key TEXT,
    -- WHERE A DEVICE ACTUALLY DIALS, which is not arc_url: the blind relay is a
    -- websocket endpoint the Arc advertises separately. Stored rather than
    -- looked up each time, because a device has to know where to sync even in
    -- the minutes an Arc is unreachable.
    arc_relay_url    TEXT,
    arc_seen_at      INTEGER
);
CREATE TABLE IF NOT EXISTS wraps (
    identity_pk TEXT PRIMARY KEY REFERENCES accounts(identity_pk),
    blob        BLOB NOT NULL,      -- PWR1: the seed, sealed under the passkey
    size        INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS histories (
    identity_pk TEXT NOT NULL REFERENCES accounts(identity_pk),
    slot        INTEGER NOT NULL,   -- 0 = current, 1 = the one before it
    blob        BLOB NOT NULL,      -- PHS1: sealed under a seed-derived key
    size        INTEGER NOT NULL,
    exported_at INTEGER,            -- the client's clock, from the header
    uploaded_at INTEGER NOT NULL,   -- ours
    PRIMARY KEY (identity_pk, slot)
);
-- WHERE THE CHAIN ENDS, sealed under the seed and anchored outside itself.
-- A chain of sealed entries catches modification, reordering, dropped middles
-- and forks; it does NOT catch truncation of the tail, because a prefix of a
-- valid chain is itself a valid chain. Only an anchor the reader already holds
-- catches that -- and a borrowed laptop holds none, which is why this row is
-- here rather than in the client.
--
-- Additive: a deployment that has never seen a head simply gains an empty
-- table on the next boot. Nothing migrates, and no existing row changes.
CREATE TABLE IF NOT EXISTS heads (
    identity_pk TEXT PRIMARY KEY REFERENCES accounts(identity_pk),
    blob        BLOB NOT NULL,      -- PHD1: sealed under a seed-derived key
    size        INTEGER NOT NULL,
    -- THE ONE FIELD THIS SERVICE READS. The blob is opaque to us, so the
    -- compare-and-set has to run on a number the client declares in the clear.
    -- The same number is sealed inside, so a client that opens the blob can
    -- catch us reporting a different one (`head::open_declared`).
    position    INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
-- A PERSON'S SEALED STATE ON ONE DOOR NODE (D-34 (c)): the counter of the last
-- seal a Door stored, and nothing of the state itself. It moves only forward, and
-- only from the value the writer says it holds (`put_seal`), so a Door that
-- resumed from anything but the latest seal is refused rather than believed.
-- Additive, like `heads`: an older deployment gains an empty table.
CREATE TABLE IF NOT EXISTS seals (
    identity_pk TEXT NOT NULL REFERENCES accounts(identity_pk),
    node        TEXT NOT NULL,      -- the Door node: door.wallflowers.io, or host:port
    counter     INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (identity_pk, node)
);
CREATE TABLE IF NOT EXISTS sites (
    slug        TEXT PRIMARY KEY,   -- the address, lowercased; this IS the identity
    identity_pk TEXT REFERENCES accounts(identity_pk),
    created_at  INTEGER NOT NULL,
    -- Set when the owner's device has authored the site GroupObject. Until then
    -- the slug is RESERVED and nothing exists: a site is a GroupObject with
    -- Authority = Owner, so only the owner's device can bring it into being.
    group_id    TEXT,
    -- THE HOST THIS ADDRESS IS SERVED FROM, set by the owner when they publish.
    -- Not the site's Group: the Arc never joins that one (the-seam.html §03), so
    -- it serves from a Host, a second object whose roster is the owner and the
    -- Arc. The Arc asks this table which Host an address is bound to and serves
    -- from no other -- which is what makes the claim at signup, and not the
    -- first Host an Arc happens to fold, the owner of a name (ruled 22 Sep 2026).
    host_id     TEXT
);
CREATE INDEX IF NOT EXISTS sites_account ON sites(identity_pk);
CREATE TABLE IF NOT EXISTS challenges (
    nonce     TEXT PRIMARY KEY,
    issued_at INTEGER NOT NULL
);
-- A HOLD on an address while someone is signing up for it. Step 1 of signup takes
-- it; the claim at the end consumes it; closing the page releases it; and it lapses
-- on its own at `expires_at` whatever happens. No account exists yet, so the holder
-- is the only thing there is to hold it to: a KEYED HASH of the requester's IP
-- (`hold_key`), never the IP itself.
CREATE TABLE IF NOT EXISTS site_holds (
    slug       TEXT PRIMARY KEY,
    holder     TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS site_holds_holder ON site_holds(holder);
-- Small service-owned values that must survive a restart (`hold_key`).
CREATE TABLE IF NOT EXISTS service_kv (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);
"""

# Everything the old account model left behind.
#
# `users`, `identities`, `passkeys` and `backups` are the passkey-as-account
# shape: a server-minted uid, an (iss, sub) pair, a credential row that WAS the
# account, and a blob released on two proofs. `recovery_contacts` and `otps`
# are the second factor those required -- and the only personal data this
# service ever held. Dropped rather than migrated, because nothing can be
# carried across: no existing row has an Ed25519 key, and a browser cannot mint
# one. Affected accounts re-enrol (ruling D4, 13 Sep 2026).
_DEAD_TABLES = (
    "refresh_tokens", "memory_reads",
    "users", "identities", "passkeys", "backups", "recovery_contacts", "otps",
)

SLOT_CURRENT, SLOT_PREVIOUS = 0, 1


class StaleHistory(Exception):
    """An export older than the one already stored. Carries both stamps so the
    caller can be told what it is behind, rather than merely that it lost."""

    def __init__(self, *, held: int, offered: int):
        super().__init__(f"held {held}, offered {offered}")
        self.held = held
        self.offered = offered


class StaleHead(Exception):
    """A head write that does not move the chain forward.

    Two shapes, and they are not the same event, so the caller is told which:

      · `behind` -- an older position than the one held. A session that has been
        asleep, or a replica that came back with a stale view. Harmless, and
        refusing it is the entire point of the position.
      · `fork`   -- the SAME position, different ciphertext. Two writers appended
        against one predecessor. Not a race that anybody won: the two chains now
        disagree about what happened at that position, and whoever reads next
        cannot tell which is real. Refused rather than picked between.

    An identical re-write at the same position is neither -- it is a retry whose
    reply was lost, and it succeeds as a no-op.
    """

    def __init__(self, *, held: int, offered: int, reason: str):
        super().__init__(f"held {held}, offered {offered} ({reason})")
        self.held = held
        self.offered = offered
        self.reason = reason


class StaleSeal(Exception):
    """A seal write whose held counter is not the one stored: the state it was
    sealed from has moved on. Carries both, so the Door is told how far."""

    def __init__(self, *, held: int, offered: int):
        super().__init__(f"held {held}, offered {offered}")
        self.held = held
        self.offered = offered


class Store:
    def __init__(self, path: str):
        if path != ":memory:":
            os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        self._lock = threading.Lock()
        self._db = sqlite3.connect(path, check_same_thread=False)
        self._db.row_factory = sqlite3.Row
        with self._lock:
            self._db.execute("PRAGMA journal_mode=WAL")
            self._db.execute("PRAGMA foreign_keys=ON")
            self._drop_oidc_sites()
            self._db.executescript(_SCHEMA)
            self._add_sites_host_id()
            # CONSTRAINTS OFF FOR THE DEMOLITION. `identities`, `refresh_tokens`
            # and the rest hold foreign keys on `users`, and `users` is dropped
            # before them, so with constraints on this raises FOREIGN KEY
            # constraint failed and the service will not start. It never showed
            # up because a fresh database has none of these tables, so the loop
            # was a no-op everywhere except the one place it had to work.
            #
            # Reordering the tuple would fix today and rot: it would encode a
            # dependency graph between tables that are all being destroyed, and
            # the next one added would have to be inserted in the right place by
            # someone who knew that was the rule. They are dead; the order
            # between them is not information worth keeping.
            self._db.execute("PRAGMA foreign_keys=OFF")
            for table in _DEAD_TABLES:
                self._db.execute(f"DROP TABLE IF EXISTS {table}")
            self._db.execute("PRAGMA foreign_keys=ON")
            self._db.commit()

    def _add_sites_host_id(self) -> None:
        """`sites.host_id` on a database made before it existed.

        `CREATE TABLE IF NOT EXISTS` skips a table that is already there, so a
        column added to the schema reaches fresh databases only. This adds it to
        the live one, on every boot, and is a no-op once it has run. Nothing is
        backfilled: no Host was ever bound before the column existed.
        """
        cols = {r["name"] for r in self._db.execute("PRAGMA table_info(sites)")}
        if cols and "host_id" not in cols:
            self._db.execute("ALTER TABLE sites ADD COLUMN host_id TEXT")

    def _drop_oidc_sites(self) -> None:
        """The one dead table `_DEAD_TABLES` cannot name.

        `sites` exists in both generations with different keys -- `uid` then, an
        Ed25519 `identity_pk` now -- so it cannot go in that list beside `users`
        and `identities` without destroying the live one. The result was a
        database that could not be upgraded by the code that owns it, and it
        failed twice over, neither time saying what was wrong:

          1. `CREATE TABLE IF NOT EXISTS sites` skips the old table, and the
             `sites_account` index that follows then raises `no such column:
             identity_pk` -- which reads as a bug in this file rather than as a
             table left over from a schema nobody runs any more.
          2. Rename it out of the way by hand and the FOREIGN KEY it still holds
             on `users` blocks `users` from being dropped, so the failure just
             moves.

        So the old one is identified by SHAPE and removed before the schema
        runs. Nothing is carried across: no OIDC row has an Ed25519 key and a
        browser cannot mint one for it, which is the same reasoning that dropped
        `users` (ruling D4, 13 Sep 2026). Anything already migrated has an
        `identity_pk` column and is left alone -- this must be safe to run on
        every boot, because it does.
        """
        cols = {r["name"] for r in self._db.execute("PRAGMA table_info(sites)")}
        if not cols or "identity_pk" in cols:
            return          # absent, or already this generation
        # Dropped with the constraint off: it points at `users`, which is about
        # to go too, and the order between two dead tables is not worth encoding.
        self._db.execute("PRAGMA foreign_keys=OFF")
        self._db.execute("DROP TABLE sites")
        self._db.execute("PRAGMA foreign_keys=ON")

    def close(self) -> None:
        with self._lock:
            self._db.close()

    # ---- accounts ---------------------------------------------------------
    def upsert(self, identity_pk: str) -> bool:
        """Ensure an account row exists. Returns True if this created it.

        Registration is idempotent by construction: the key names the account,
        so registering twice is registering the same account. There is nothing
        to collide with and nobody to squat -- proving possession of the key IS
        the claim to it.
        """
        now = int(time.time())
        with self._lock:
            row = self._db.execute(
                "SELECT 1 FROM accounts WHERE identity_pk=?", (identity_pk,)
            ).fetchone()
            if row:
                return False
            self._db.execute(
                "INSERT INTO accounts (identity_pk, created_at) VALUES (?, ?)",
                (identity_pk, now),
            )
            self._db.commit()
        return True

    def account(self, identity_pk: str) -> dict | None:
        with self._lock:
            row = self._db.execute(
                "SELECT identity_pk, created_at, arc_url, arc_identity_key, "
                "arc_relay_url, arc_seen_at FROM accounts WHERE identity_pk=?",
                (identity_pk,),
            ).fetchone()
        return dict(row) if row else None

    def host_on_arc(self, identity_pk: str, url: str, identity_key: str,
                    relay_url: str = "") -> str | None:
        """Record the Arc this account lives on, TOFU-pinning its identity.

        Returns the PREVIOUSLY pinned identity_key when it differs from what we
        just saw, so the caller can say so. The pin is never silently rewritten:
        an Arc whose key changed is not the Arc the account was tethered to.
        """
        now = int(time.time())
        with self._lock:
            row = self._db.execute(
                "SELECT arc_identity_key FROM accounts WHERE identity_pk=?", (identity_pk,)
            ).fetchone()
            was = row["arc_identity_key"] if row else None
            if was and identity_key and was != identity_key:
                self._db.execute(
                    "UPDATE accounts SET arc_seen_at=? WHERE identity_pk=?", (now, identity_pk))
                self._db.commit()
                return was
            self._db.execute(
                "UPDATE accounts SET arc_url=?, arc_identity_key=?, arc_relay_url=?, "
                "arc_seen_at=? WHERE identity_pk=?",
                (url, identity_key, relay_url, now, identity_pk),
            )
            self._db.commit()
        return None

    def arc_of(self, identity_pk: str) -> dict | None:
        row = self.account(identity_pk)
        if not row or not row.get("arc_url"):
            return None
        return {k: row[k] for k in
                ("arc_url", "arc_identity_key", "arc_relay_url", "arc_seen_at")}

    # ---- the wrap ---------------------------------------------------------
    def put_wrap(self, identity_pk: str, blob: bytes) -> dict:
        """Store or replace the wrap. ONE PER ACCOUNT, which is what makes
        enrolling a passkey revoke the one it replaces: there is no second row
        for an old credential to keep opening."""
        now = int(time.time())
        with self._lock:
            self._db.execute(
                "INSERT INTO wraps (identity_pk, blob, size, updated_at) VALUES (?,?,?,?) "
                "ON CONFLICT(identity_pk) DO UPDATE SET "
                "blob=excluded.blob, size=excluded.size, updated_at=excluded.updated_at",
                (identity_pk, blob, len(blob), now),
            )
            self._db.commit()
        return {"size": len(blob), "updated_at": now}

    def wrap(self, identity_pk: str) -> dict | None:
        with self._lock:
            row = self._db.execute(
                "SELECT blob, size, updated_at FROM wraps WHERE identity_pk=?",
                (identity_pk,),
            ).fetchone()
        return dict(row) if row else None

    # ---- the history ------------------------------------------------------
    def put_history(self, identity_pk: str, blob: bytes, *, exported_at: int | None) -> dict:
        """Store the new current history and demote the old one. One
        transaction: there is never a moment with zero copies, and never one
        with three.

        REFUSES A HISTORY OLDER THAN THE ONE HELD. An account has devices, and
        devices go offline; a phone that wakes with a week-old view and exports
        would otherwise overwrite a laptop's current one, and the account's
        stored history would move BACKWARDS. Two slots survive one such write
        and not two in a row.

        The comparison is on the exporter's own clock, which is the only clock
        that knows when the state was captured -- and it is advisory rather than
        trusted: the caller already proved it owns this account by signing, so
        the guard is against staleness, not against an attacker, who could
        simply send a newer stamp. An export carrying no stamp at all is
        accepted, because refusing it would lock out a client that predates the
        header.
        """
        now = int(time.time())
        with self._lock:
            if exported_at is not None:
                row = self._db.execute(
                    "SELECT exported_at FROM histories WHERE identity_pk=? AND slot=?",
                    (identity_pk, SLOT_CURRENT),
                ).fetchone()
                held = row["exported_at"] if row else None
                if held is not None and exported_at < held:
                    raise StaleHistory(held=held, offered=exported_at)
            self._db.execute(
                "DELETE FROM histories WHERE identity_pk=? AND slot=?",
                (identity_pk, SLOT_PREVIOUS))
            self._db.execute(
                "UPDATE histories SET slot=? WHERE identity_pk=? AND slot=?",
                (SLOT_PREVIOUS, identity_pk, SLOT_CURRENT))
            self._db.execute(
                "INSERT INTO histories (identity_pk, slot, blob, size, exported_at, uploaded_at) "
                "VALUES (?,?,?,?,?,?)",
                (identity_pk, SLOT_CURRENT, blob, len(blob), exported_at, now),
            )
            self._db.commit()
        return {"size": len(blob), "exported_at": exported_at, "uploaded_at": now}

    def history(self, identity_pk: str, slot: int = SLOT_CURRENT) -> dict | None:
        with self._lock:
            row = self._db.execute(
                "SELECT blob, size, exported_at, uploaded_at FROM histories "
                "WHERE identity_pk=? AND slot=?", (identity_pk, slot),
            ).fetchone()
        return dict(row) if row else None

    def history_meta(self, identity_pk: str, slot: int = SLOT_CURRENT) -> dict | None:
        with self._lock:
            row = self._db.execute(
                "SELECT size, exported_at, uploaded_at FROM histories "
                "WHERE identity_pk=? AND slot=?", (identity_pk, slot),
            ).fetchone()
        return dict(row) if row else None

    # ---- the head ---------------------------------------------------------
    def put_head(self, identity_pk: str, blob: bytes, position: int) -> dict:
        """Move the chain anchor forward, or refuse.

        THE COMPARE-AND-SET, and why it is stricter than the history's guard.
        `put_history` compares the exporter's own CLOCK and is advisory: a clock
        is a claim, and two devices can honestly disagree about the time. A chain
        position is not a claim, it is a count -- position n names exactly one
        tail, forever -- so this is an enforceable ordering rather than a hint,
        and it is what lets the histories guard eventually go away.

        Three outcomes, in the order they are checked:

          · offered > held        stored. The chain grew.
          · offered == held, same bytes   a no-op success. A retry whose reply
            was lost must not be punished for arriving twice.
          · offered == held, different bytes   refused as a `fork`.
          · offered < held        refused as `behind`.

        The refusal leaves the stored head EXACTLY as it was -- losing the race
        must not also cost the account the anchor it already had.

        THE POSITION IS ALREADY BOUNDED when it gets here (`main.CHAIN_POSITION_MAX`),
        and the column is why that guard cannot be dropped: a head seals its
        position as a u64, and SQLite's INTEGER is SIGNED 64-bit, so a value in
        the top half of that range does not store -- it raises OverflowError on
        the way in, which is an unhandled 500 out of a route whose contract is
        that failure is a JSON error code.
        """
        now = int(time.time())
        with self._lock:
            row = self._db.execute(
                "SELECT blob, size, position, updated_at FROM heads WHERE identity_pk=?",
                (identity_pk,),
            ).fetchone()
            if row is not None:
                held = row["position"]
                if position < held:
                    raise StaleHead(held=held, offered=position, reason="behind")
                if position == held:
                    if row["blob"] != blob:
                        raise StaleHead(held=held, offered=position, reason="fork")
                    # Byte-identical at the same position: the write already
                    # happened. `updated_at` is NOT refreshed, because a witness
                    # watching this record should see the moment the anchor moved,
                    # not the moment a client retried.
                    return {"size": row["size"], "position": held,
                            "updated_at": row["updated_at"], "stored": False}
            self._db.execute(
                "INSERT INTO heads (identity_pk, blob, size, position, updated_at) "
                "VALUES (?,?,?,?,?) ON CONFLICT(identity_pk) DO UPDATE SET "
                "blob=excluded.blob, size=excluded.size, position=excluded.position, "
                "updated_at=excluded.updated_at",
                (identity_pk, blob, len(blob), position, now),
            )
            self._db.commit()
        return {"size": len(blob), "position": position, "updated_at": now, "stored": True}

    def head(self, identity_pk: str) -> dict | None:
        with self._lock:
            row = self._db.execute(
                "SELECT blob, size, position, updated_at FROM heads WHERE identity_pk=?",
                (identity_pk,),
            ).fetchone()
        return dict(row) if row else None

    def head_meta(self, identity_pk: str) -> dict | None:
        with self._lock:
            row = self._db.execute(
                "SELECT size, position, updated_at FROM heads WHERE identity_pk=?",
                (identity_pk,),
            ).fetchone()
        return dict(row) if row else None

    # ---- seals (D-34 (c)) ---------------------------------------------------
    def put_seal(self, identity_pk: str, node: str, held: int, counter: int) -> dict:
        """Move one node's seal counter from `held` to `counter`, or refuse.

          · stored == counter          a retry whose reply was lost: a no-op success,
                                       `updated_at` untouched, as the head's.
          · stored == held (0: none)   stored.
          · anything else              `StaleSeal`, the row left as it was.

        `counter > held` and both within bounds are the route's to check.
        """
        now = int(time.time())
        with self._lock:
            row = self._db.execute(
                "SELECT counter, updated_at FROM seals WHERE identity_pk=? AND node=?",
                (identity_pk, node),
            ).fetchone()
            stored = row["counter"] if row else 0
            if row is not None and stored == counter:
                return {"counter": counter, "stored": False}
            if stored != held:
                raise StaleSeal(held=stored, offered=counter)
            self._db.execute(
                "INSERT INTO seals (identity_pk, node, counter, updated_at) VALUES (?,?,?,?) "
                "ON CONFLICT(identity_pk, node) DO UPDATE SET "
                "counter=excluded.counter, updated_at=excluded.updated_at",
                (identity_pk, node, counter, now),
            )
            self._db.commit()
        return {"counter": counter, "stored": True}

    def seal(self, identity_pk: str, node: str) -> dict | None:
        with self._lock:
            row = self._db.execute(
                "SELECT counter, updated_at FROM seals WHERE identity_pk=? AND node=?",
                (identity_pk, node),
            ).fetchone()
        return dict(row) if row else None

    # ---- challenges -------------------------------------------------------
    def issue(self, nonce: str, ttl: int) -> str:
        """Record a nonce as live. Expired ones are pruned here rather than on a
        timer: there is no scheduler to keep honest, and a challenge nobody
        spent is worthless the moment it ages out."""
        now = int(time.time())
        with self._lock:
            self._db.execute("DELETE FROM challenges WHERE issued_at < ?", (now - ttl,))
            self._db.execute(
                "INSERT INTO challenges (nonce, issued_at) VALUES (?, ?)", (nonce, now))
            self._db.commit()
        return nonce

    def spend(self, nonce: str, ttl: int) -> bool:
        """Spend a challenge. SINGLE USE: a replayed signature finds its nonce
        already gone, which is what makes a captured one worth nothing."""
        now = int(time.time())
        with self._lock:
            row = self._db.execute(
                "SELECT issued_at FROM challenges WHERE nonce=?", (nonce,)).fetchone()
            if row is None or row["issued_at"] < now - ttl:
                return False
            self._db.execute("DELETE FROM challenges WHERE nonce=?", (nonce,))
            self._db.commit()
        return True

    # ---- sites ------------------------------------------------------------
    def site(self, slug: str) -> dict | None:
        with self._lock:
            row = self._db.execute(
                "SELECT slug, identity_pk, created_at, group_id FROM sites WHERE slug=?",
                (slug.lower(),),
            ).fetchone()
        return dict(row) if row else None

    def claim_site(self, slug: str, identity_pk: str,
                   holder: str | None = None) -> tuple[bool, str]:
        """Reserve a slug for an account. IDEMPOTENT BY DESIGN.

        Claiming a slug you already hold succeeds and changes nothing, so a
        retried request, a double-tapped button or a resumed wizard all land in
        the same place. Claiming one somebody else holds fails without saying
        who holds it — and that includes a live HOLD by another signup
        (`hold_site`). A hold by this same `holder` is consumed by the claim.

        Returns (ok, reason). A reservation is not a site: the site is a
        GroupObject the owner's device must author.
        """
        slug = slug.lower()
        now = int(time.time())
        with self._lock:
            row = self._db.execute(
                "SELECT identity_pk FROM sites WHERE slug=?", (slug,)).fetchone()
            if row:
                mine = row["identity_pk"] == identity_pk
                return mine, ("yours" if mine else "held")
            hold = self._db.execute(
                "SELECT holder FROM site_holds WHERE slug=? AND expires_at>?",
                (slug, now)).fetchone()
            if hold and hold["holder"] != holder:
                return False, "held"
            self._db.execute(
                "INSERT INTO sites (slug, identity_pk, created_at, group_id) VALUES (?,?,?,NULL)",
                (slug, identity_pk, now),
            )
            self._db.execute("DELETE FROM site_holds WHERE slug=?", (slug,))
            self._db.commit()
        return True, "claimed"

    def bind_host(self, slug: str, identity_pk: str, host_id: str) -> tuple[bool, str]:
        """Point an address the caller claimed at the Host that serves it.

        Only the account that claimed the slug may bind it, and it may re-bind it
        to another of its Hosts at any time -- an owner moving their own address
        is not a conflict. Idempotent: binding the Host already bound changes
        nothing. Returns (ok, reason): `bound`, `unchanged`, `unclaimed`, or
        `not_yours`, the last without saying whose it is.
        """
        slug = slug.lower()
        with self._lock:
            row = self._db.execute(
                "SELECT identity_pk, host_id FROM sites WHERE slug=?", (slug,)).fetchone()
            if not row:
                return False, "unclaimed"
            if row["identity_pk"] != identity_pk:
                return False, "not_yours"
            if row["host_id"] == host_id:
                return True, "unchanged"
            self._db.execute("UPDATE sites SET host_id=? WHERE slug=?", (host_id, slug))
            self._db.commit()
        return True, "bound"

    def site_host(self, slug: str) -> dict | None:
        """Which Host an address is served from, and nothing about who owns it.

        None when the address was never claimed. A claimed address with no Host
        bound yet comes back with `host` None: reserved, not published.
        """
        with self._lock:
            row = self._db.execute(
                "SELECT slug, host_id FROM sites WHERE slug=?", (slug.lower(),)).fetchone()
        return {"slug": row["slug"], "host": row["host_id"]} if row else None

    # ─────────────────────────── holds during signup ──────────────────────────

    def hold_key(self) -> bytes:
        """The service's key for hashing requester IPs into hold owners. Minted once
        and kept, so a restart does not orphan every live hold."""
        with self._lock:
            row = self._db.execute("SELECT v FROM service_kv WHERE k='hold_key'").fetchone()
            if row:
                return bytes.fromhex(row["v"])
            key = os.urandom(32)
            self._db.execute("INSERT INTO service_kv (k, v) VALUES ('hold_key', ?)", (key.hex(),))
            self._db.commit()
            return key

    def hold_site(self, slug: str, holder: str, ttl: int,
                  max_per_holder: int) -> tuple[bool, str, int]:
        """Hold `slug` for `holder` for `ttl` seconds. Returns (ok, reason, expires_at).

        Refused when the address is already a site (`taken`), held by someone else
        and not yet lapsed (`held`), or when this holder already holds
        `max_per_holder` other addresses (`too_many`) — one person signing up needs
        one, and a cap is what stops a script holding the namespace. Holding an
        address you already hold renews it.
        """
        slug = slug.lower()
        now = int(time.time())
        with self._lock:
            self._db.execute("DELETE FROM site_holds WHERE expires_at<=?", (now,))
            if self._db.execute("SELECT 1 FROM sites WHERE slug=?", (slug,)).fetchone():
                self._db.commit()
                return False, "taken", 0
            row = self._db.execute(
                "SELECT holder FROM site_holds WHERE slug=?", (slug,)).fetchone()
            if row and row["holder"] != holder:
                self._db.commit()
                return False, "held", 0
            others = self._db.execute(
                "SELECT COUNT(*) FROM site_holds WHERE holder=? AND slug<>?",
                (holder, slug)).fetchone()[0]
            if not row and others >= max_per_holder:
                self._db.commit()
                return False, "too_many", 0
            expires = now + ttl
            self._db.execute(
                "INSERT INTO site_holds (slug, holder, expires_at) VALUES (?,?,?) "
                "ON CONFLICT(slug) DO UPDATE SET expires_at=excluded.expires_at",
                (slug, holder, expires),
            )
            self._db.commit()
        return True, "held", expires

    def release_site(self, slug: str, holder: str) -> bool:
        """Drop a hold — only the holder's own. Returns whether one was dropped."""
        with self._lock:
            n = self._db.execute("DELETE FROM site_holds WHERE slug=? AND holder=?",
                                 (slug.lower(), holder)).rowcount
            self._db.commit()
        return n > 0

    def site_hold(self, slug: str) -> dict | None:
        """The live hold on `slug`, if any: {holder, expires_at}."""
        with self._lock:
            row = self._db.execute(
                "SELECT holder, expires_at FROM site_holds WHERE slug=? AND expires_at>?",
                (slug.lower(), int(time.time()))).fetchone()
        return dict(row) if row else None

    def sites_of(self, identity_pk: str) -> list[dict]:
        with self._lock:
            rows = self._db.execute(
                "SELECT slug, created_at, group_id, host_id FROM sites WHERE identity_pk=? "
                "ORDER BY created_at", (identity_pk,),
            ).fetchall()
        return [dict(r) for r in rows]
