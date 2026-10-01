//! The box-office ledger-adjacent state: listings, checkout sessions, redemptions.
//!
//! BACKEND NOTE: this is rusqlite at `$ARC_STATE_DIR/boxoffice.db` FOR NOW. Ralph's
//! direction for the deployed plane is Postgres — the design record names a
//! "pg redemption table" — so everything SQL lives behind this `Store` struct and the
//! swap is a deploy-time backend change, not an API change. Nothing outside this module
//! writes SQL.
//!
//! What this is NOT: the source of truth for sales. The GROUP LEDGER is (design §2 —
//! redelivery derives from the ledger, never the box-office 'fulfilled' flag). This
//! store is the money-side working state: which sessions exist, what state they are in,
//! and the online door's first-writer redemption table (D24).

use rusqlite::{params, Connection, OptionalExtension};

/// A registered listing — the owner-signed record checkout prices are read FROM (never
/// from the client, D-review). LWW by `rev`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegisteredListing {
    /// hex(sha256(event_group_id ‖ owner_pk)) — minted by the core, opaque here.
    pub listing: String,
    pub owner_pk: String,
    /// The Arc's fulfillment-delegate pk. Required iff priced; absent on free events.
    #[serde(default)]
    pub delegate_pk: Option<String>,
    pub unit_cents: i64,
    pub currency: String,
    pub capacity: i64,
    pub open: bool,
    /// The rail's connected account (direct charges land on it). Required iff priced.
    #[serde(default)]
    pub acct: Option<String>,
    pub rev: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Session {
    pub pi: String,
    pub listing: String,
    pub buyer_pk: String,
    pub qty: i64,
    pub amount_cents: i64,
    /// The fee SNAPSHOT for the whole session (per-ticket fee × qty), minor units.
    pub fee_cents: i64,
    pub currency: String,
    pub url: String,
    pub state: String,
    pub created_ms: i64,
}

/// How long an 'open' session holds capacity and satisfies the 9A dedup. Mirrors the
/// 35-minute expiry stamped on the rail's checkout session (`stripe::SESSION_TTL_S`);
/// the full 4A expiry sweep (expire + refund-if-paid) is the 2B agent's job.
pub const OPEN_SESSION_TTL_MS: i64 = 35 * 60 * 1000;

/// Outcome of an idempotent state mark.
#[derive(Debug, PartialEq, Eq)]
pub enum Mark {
    Applied,
    /// Already in (or past) the requested state — a replayed webhook, a retried refund.
    Noop,
    NotFound,
}

/// Outcome of a fulfill attempt — the route maps these onto 200/409/404.
#[derive(Debug, PartialEq, Eq)]
pub enum Fulfill {
    Fulfilled,
    /// The 409: fulfill is called LAST and exactly once per session (D23).
    AlreadyFulfilled,
    NotPaid,
    NotFound,
}

/// Outcome of a door scan — first writer wins, a second scan is a REPORT, not an error
/// (double-scan folds to a flag, never a rejection — design §2).
#[derive(Debug, PartialEq, Eq)]
pub enum Redeem {
    Admitted,
    Already { door_pk: String, redeemed_ms: i64 },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Redemption {
    pub listing: String,
    pub ticket: String,
    pub door_pk: String,
    pub redeemed_ms: i64,
}

pub struct Store {
    // One connection behind a mutex: this plane is low-volume request/response, and the
    // pg swap replaces the whole field with a pool — callers never see either.
    conn: std::sync::Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS listings (
  listing     TEXT PRIMARY KEY,
  owner_pk    TEXT NOT NULL,
  delegate_pk TEXT,
  unit_cents  INTEGER NOT NULL,
  currency    TEXT NOT NULL,
  capacity    INTEGER NOT NULL,
  open        INTEGER NOT NULL,
  acct        TEXT,
  rev         INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
  pi           TEXT PRIMARY KEY,
  listing      TEXT NOT NULL,
  buyer_pk     TEXT NOT NULL,
  qty          INTEGER NOT NULL,
  amount_cents INTEGER NOT NULL,
  fee_cents    INTEGER NOT NULL,
  currency     TEXT NOT NULL,
  url          TEXT NOT NULL,
  state        TEXT NOT NULL CHECK(state IN ('open','paid','fulfilled','refunded','expired')),
  created_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_by_listing ON sessions(listing, state);
CREATE TABLE IF NOT EXISTS redemptions (
  listing     TEXT NOT NULL,
  ticket      TEXT NOT NULL,
  door_pk     TEXT NOT NULL,
  redeemed_ms INTEGER NOT NULL,
  PRIMARY KEY (listing, ticket)
);
"#;

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    // -- listings -------------------------------------------------------------

    /// LWW by rev: applied only when `rev` is strictly newer than what is registered.
    /// `Ok(false)` = stale, nothing written (the route answers 409). Equal rev is stale
    /// too — a re-send of the same registration changes nothing and says so.
    pub fn upsert_listing(&self, l: &RegisteredListing) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("store mutex");
        let existing: Option<i64> = conn
            .query_row(
                "SELECT rev FROM listings WHERE listing = ?1",
                [&l.listing],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(rev) = existing {
            if rev >= l.rev {
                return Ok(false);
            }
        }
        conn.execute(
            "INSERT INTO listings (listing, owner_pk, delegate_pk, unit_cents, currency, capacity, open, acct, rev)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(listing) DO UPDATE SET
               owner_pk = ?2, delegate_pk = ?3, unit_cents = ?4, currency = ?5,
               capacity = ?6, open = ?7, acct = ?8, rev = ?9",
            params![
                l.listing, l.owner_pk, l.delegate_pk, l.unit_cents, l.currency, l.capacity,
                l.open, l.acct, l.rev
            ],
        )?;
        Ok(true)
    }

    pub fn get_listing(&self, listing: &str) -> rusqlite::Result<Option<RegisteredListing>> {
        let conn = self.conn.lock().expect("store mutex");
        conn.query_row(
            "SELECT listing, owner_pk, delegate_pk, unit_cents, currency, capacity, open, acct, rev
             FROM listings WHERE listing = ?1",
            [listing],
            |r| {
                Ok(RegisteredListing {
                    listing: r.get(0)?,
                    owner_pk: r.get(1)?,
                    delegate_pk: r.get(2)?,
                    unit_cents: r.get(3)?,
                    currency: r.get(4)?,
                    capacity: r.get(5)?,
                    open: r.get(6)?,
                    acct: r.get(7)?,
                    rev: r.get(8)?,
                })
            },
        )
        .optional()
    }

    // -- sessions -------------------------------------------------------------

    /// The 9A dedup: the buyer's existing fresh OPEN session for this listing, if any —
    /// checkout returns ITS url instead of minting a second hold on capacity.
    pub fn open_session_for(
        &self,
        listing: &str,
        buyer_pk: &str,
        now_ms: i64,
    ) -> rusqlite::Result<Option<Session>> {
        let conn = self.conn.lock().expect("store mutex");
        conn.query_row(
            &format!(
                "{SESSION_SELECT} WHERE listing = ?1 AND buyer_pk = ?2 AND state = 'open'
              AND created_ms > ?3 ORDER BY created_ms DESC LIMIT 1"
            ),
            params![listing, buyer_pk, now_ms - OPEN_SESSION_TTL_MS],
            row_to_session,
        )
        .optional()
    }

    pub fn insert_session(&self, s: &Session) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("store mutex");
        conn.execute(
            "INSERT INTO sessions (pi, listing, buyer_pk, qty, amount_cents, fee_cents, currency, url, state, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                s.pi, s.listing, s.buyer_pk, s.qty, s.amount_cents, s.fee_cents, s.currency,
                s.url, s.state, s.created_ms
            ],
        )?;
        Ok(())
    }

    pub fn get_session(&self, pi: &str) -> rusqlite::Result<Option<Session>> {
        let conn = self.conn.lock().expect("store mutex");
        conn.query_row(
            &format!("{SESSION_SELECT} WHERE pi = ?1"),
            [pi],
            row_to_session,
        )
        .optional()
    }

    /// Webhook-driven, idempotent: open → paid applies; paid/fulfilled (a replayed
    /// webhook) is a Noop; refunded/expired stays as it is — a late webhook must never
    /// resurrect a session the 4A sweep or a refund already closed.
    pub fn mark_paid(&self, pi: &str) -> rusqlite::Result<Mark> {
        let conn = self.conn.lock().expect("store mutex");
        let n = conn.execute(
            "UPDATE sessions SET state = 'paid' WHERE pi = ?1 AND state = 'open'",
            [pi],
        )?;
        if n == 1 {
            return Ok(Mark::Applied);
        }
        let exists: Option<String> = conn
            .query_row("SELECT state FROM sessions WHERE pi = ?1", [pi], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(if exists.is_some() {
            Mark::Noop
        } else {
            Mark::NotFound
        })
    }

    /// Fulfill is marked LAST and exactly once (D23): paid → fulfilled applies; a second
    /// call reports AlreadyFulfilled (the route's 409) so the caller KNOWS delivery
    /// already happened rather than silently double-running.
    pub fn mark_fulfilled(&self, pi: &str) -> rusqlite::Result<Fulfill> {
        let conn = self.conn.lock().expect("store mutex");
        let n = conn.execute(
            "UPDATE sessions SET state = 'fulfilled' WHERE pi = ?1 AND state = 'paid'",
            [pi],
        )?;
        if n == 1 {
            return Ok(Fulfill::Fulfilled);
        }
        let state: Option<String> = conn
            .query_row("SELECT state FROM sessions WHERE pi = ?1", [pi], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(match state.as_deref() {
            None => Fulfill::NotFound,
            Some("fulfilled") => Fulfill::AlreadyFulfilled,
            Some(_) => Fulfill::NotPaid,
        })
    }

    /// paid → refunded applies; refunded again is a Noop. The route refuses fulfilled
    /// BEFORE the rail is ever asked (D19 — voidTicket does not exist yet), so this
    /// method only ever sees paid/refunded.
    pub fn mark_refunded(&self, pi: &str) -> rusqlite::Result<Mark> {
        let conn = self.conn.lock().expect("store mutex");
        let n = conn.execute(
            "UPDATE sessions SET state = 'refunded' WHERE pi = ?1 AND state = 'paid'",
            [pi],
        )?;
        if n == 1 {
            return Ok(Mark::Applied);
        }
        let exists: Option<String> = conn
            .query_row("SELECT state FROM sessions WHERE pi = ?1", [pi], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(if exists.is_some() {
            Mark::Noop
        } else {
            Mark::NotFound
        })
    }

    /// Tickets sold and holding: the D22 capacity gate is capacity − paid − open.
    pub fn paid_count(&self, listing: &str) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().expect("store mutex");
        conn.query_row(
            "SELECT COALESCE(SUM(qty), 0) FROM sessions
             WHERE listing = ?1 AND state IN ('paid', 'fulfilled')",
            [listing],
            |r| r.get(0),
        )
    }

    /// Only FRESH open sessions hold capacity — one that outlived the rail session's
    /// 35-minute expiry no longer blocks a sale (the 2B sweep will mark it expired).
    pub fn open_count(&self, listing: &str, now_ms: i64) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().expect("store mutex");
        conn.query_row(
            "SELECT COALESCE(SUM(qty), 0) FROM sessions
             WHERE listing = ?1 AND state = 'open' AND created_ms > ?2",
            params![listing, now_ms - OPEN_SESSION_TTL_MS],
            |r| r.get(0),
        )
    }

    // -- redemptions (the online door, D24) -----------------------------------

    /// First writer wins; a losing scan learns WHO admitted the ticket and when.
    pub fn redeem(
        &self,
        listing: &str,
        ticket: &str,
        door_pk: &str,
        now_ms: i64,
    ) -> rusqlite::Result<Redeem> {
        let conn = self.conn.lock().expect("store mutex");
        let n = conn.execute(
            "INSERT INTO redemptions (listing, ticket, door_pk, redeemed_ms)
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT(listing, ticket) DO NOTHING",
            params![listing, ticket, door_pk, now_ms],
        )?;
        if n == 1 {
            return Ok(Redeem::Admitted);
        }
        conn.query_row(
            "SELECT door_pk, redeemed_ms FROM redemptions WHERE listing = ?1 AND ticket = ?2",
            params![listing, ticket],
            |r| {
                Ok(Redeem::Already {
                    door_pk: r.get(0)?,
                    redeemed_ms: r.get(1)?,
                })
            },
        )
    }

    pub fn redemption(&self, listing: &str, ticket: &str) -> rusqlite::Result<Option<Redemption>> {
        let conn = self.conn.lock().expect("store mutex");
        conn.query_row(
            "SELECT listing, ticket, door_pk, redeemed_ms FROM redemptions
             WHERE listing = ?1 AND ticket = ?2",
            params![listing, ticket],
            |r| {
                Ok(Redemption {
                    listing: r.get(0)?,
                    ticket: r.get(1)?,
                    door_pk: r.get(2)?,
                    redeemed_ms: r.get(3)?,
                })
            },
        )
        .optional()
    }
}

const SESSION_SELECT: &str = "SELECT pi, listing, buyer_pk, qty, amount_cents, fee_cents, \
     currency, url, state, created_ms FROM sessions";

fn row_to_session(r: &rusqlite::Row) -> rusqlite::Result<Session> {
    Ok(Session {
        pi: r.get(0)?,
        listing: r.get(1)?,
        buyer_pk: r.get(2)?,
        qty: r.get(3)?,
        amount_cents: r.get(4)?,
        fee_cents: r.get(5)?,
        currency: r.get(6)?,
        url: r.get(7)?,
        state: r.get(8)?,
        created_ms: r.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(rev: i64) -> RegisteredListing {
        RegisteredListing {
            listing: "l1".into(),
            owner_pk: "owner".into(),
            delegate_pk: Some("delegate".into()),
            unit_cents: 1200,
            currency: "usd".into(),
            capacity: 10,
            open: true,
            acct: Some("acct_1".into()),
            rev,
        }
    }

    fn session(pi: &str, listing: &str, buyer: &str, qty: i64, created_ms: i64) -> Session {
        Session {
            pi: pi.into(),
            listing: listing.into(),
            buyer_pk: buyer.into(),
            qty,
            amount_cents: 1200 * qty,
            fee_cents: 10 * qty,
            currency: "usd".into(),
            url: format!("mock://checkout/{pi}"),
            state: "open".into(),
            created_ms,
        }
    }

    /// LWW by rev: newer applies, equal and older are rejected untouched.
    #[test]
    fn upsert_listing_is_lww_by_rev() {
        let s = Store::open_in_memory().unwrap();
        assert!(
            s.upsert_listing(&listing(1)).unwrap(),
            "first registration applies"
        );
        assert!(
            !s.upsert_listing(&listing(1)).unwrap(),
            "equal rev is stale"
        );
        assert!(
            !s.upsert_listing(&listing(0)).unwrap(),
            "lower rev is stale"
        );
        let mut newer = listing(2);
        newer.capacity = 50;
        assert!(s.upsert_listing(&newer).unwrap(), "higher rev applies");
        let got = s.get_listing("l1").unwrap().unwrap();
        assert_eq!(got.capacity, 50);
        assert_eq!(got.rev, 2);
        // The stale write really wrote nothing.
        assert!(!s.upsert_listing(&listing(1)).unwrap());
        assert_eq!(s.get_listing("l1").unwrap().unwrap().capacity, 50);
    }

    /// 9A dedup: same (listing, buyer) finds the fresh open session; a different buyer,
    /// a paid session, or a stale one does not.
    #[test]
    fn open_session_dedup() {
        let s = Store::open_in_memory().unwrap();
        let now = 1_000_000_000_000;
        s.insert_session(&session("pi_1", "l1", "buyerA", 2, now))
            .unwrap();
        let hit = s.open_session_for("l1", "buyerA", now).unwrap().unwrap();
        assert_eq!(hit.pi, "pi_1");
        assert!(s.open_session_for("l1", "buyerB", now).unwrap().is_none());
        assert!(s.open_session_for("l2", "buyerA", now).unwrap().is_none());
        // Stale: past the TTL the hold no longer dedups (or counts — see below).
        assert!(s
            .open_session_for("l1", "buyerA", now + OPEN_SESSION_TTL_MS + 1)
            .unwrap()
            .is_none());
        // Paid sessions never dedup — the buyer may legitimately buy again.
        s.mark_paid("pi_1").unwrap();
        assert!(s.open_session_for("l1", "buyerA", now).unwrap().is_none());
    }

    /// mark_paid is idempotent: the replayed webhook is a Noop, never an error and never
    /// a second application.
    #[test]
    fn mark_paid_idempotent() {
        let s = Store::open_in_memory().unwrap();
        s.insert_session(&session("pi_1", "l1", "b", 1, 0)).unwrap();
        assert_eq!(s.mark_paid("pi_1").unwrap(), Mark::Applied);
        assert_eq!(s.mark_paid("pi_1").unwrap(), Mark::Noop);
        assert_eq!(s.mark_paid("pi_missing").unwrap(), Mark::NotFound);
        assert_eq!(s.get_session("pi_1").unwrap().unwrap().state, "paid");
    }

    /// A late webhook must never resurrect a refunded session.
    #[test]
    fn mark_paid_never_resurrects_refunded() {
        let s = Store::open_in_memory().unwrap();
        s.insert_session(&session("pi_1", "l1", "b", 1, 0)).unwrap();
        s.mark_paid("pi_1").unwrap();
        assert_eq!(s.mark_refunded("pi_1").unwrap(), Mark::Applied);
        assert_eq!(s.mark_paid("pi_1").unwrap(), Mark::Noop);
        assert_eq!(s.get_session("pi_1").unwrap().unwrap().state, "refunded");
    }

    /// Fulfill: once, from paid only; the repeat is the 409.
    #[test]
    fn mark_fulfilled_once() {
        let s = Store::open_in_memory().unwrap();
        s.insert_session(&session("pi_1", "l1", "b", 1, 0)).unwrap();
        assert_eq!(
            s.mark_fulfilled("pi_1").unwrap(),
            Fulfill::NotPaid,
            "open is not paid"
        );
        s.mark_paid("pi_1").unwrap();
        assert_eq!(s.mark_fulfilled("pi_1").unwrap(), Fulfill::Fulfilled);
        assert_eq!(s.mark_fulfilled("pi_1").unwrap(), Fulfill::AlreadyFulfilled);
        assert_eq!(s.mark_fulfilled("pi_missing").unwrap(), Fulfill::NotFound);
    }

    #[test]
    fn mark_refunded_from_paid_only() {
        let s = Store::open_in_memory().unwrap();
        s.insert_session(&session("pi_1", "l1", "b", 1, 0)).unwrap();
        s.mark_paid("pi_1").unwrap();
        assert_eq!(s.mark_refunded("pi_1").unwrap(), Mark::Applied);
        assert_eq!(s.mark_refunded("pi_1").unwrap(), Mark::Noop);
        assert_eq!(s.mark_refunded("pi_missing").unwrap(), Mark::NotFound);
    }

    /// The D22 arithmetic: paid counts paid+fulfilled qty; open counts only FRESH holds.
    #[test]
    fn capacity_counts() {
        let s = Store::open_in_memory().unwrap();
        let now = 1_000_000_000_000;
        s.insert_session(&session("pi_1", "l1", "a", 2, now))
            .unwrap();
        s.insert_session(&session("pi_2", "l1", "b", 3, now))
            .unwrap();
        s.insert_session(&session(
            "pi_3",
            "l1",
            "c",
            1,
            now - OPEN_SESSION_TTL_MS - 1,
        ))
        .unwrap();
        s.mark_paid("pi_1").unwrap();
        assert_eq!(s.paid_count("l1").unwrap(), 2);
        assert_eq!(
            s.open_count("l1", now).unwrap(),
            3,
            "the stale hold pi_3 does not count"
        );
        s.mark_paid("pi_2").unwrap();
        s.mark_fulfilled("pi_2").unwrap();
        assert_eq!(
            s.paid_count("l1").unwrap(),
            5,
            "fulfilled still counts as sold"
        );
        assert_eq!(s.open_count("l1", now).unwrap(), 0);
        assert_eq!(s.paid_count("l2").unwrap(), 0);
    }

    /// The door: first scan admits, the second learns who scanned first (never an error).
    #[test]
    fn redeem_first_wins() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(
            s.redeem("l1", "t1", "door_a", 100).unwrap(),
            Redeem::Admitted
        );
        assert_eq!(
            s.redeem("l1", "t1", "door_b", 200).unwrap(),
            Redeem::Already {
                door_pk: "door_a".into(),
                redeemed_ms: 100
            }
        );
        // A different ticket (and the same ticket on a different listing) is fresh.
        assert_eq!(
            s.redeem("l1", "t2", "door_b", 300).unwrap(),
            Redeem::Admitted
        );
        assert_eq!(
            s.redeem("l2", "t1", "door_b", 300).unwrap(),
            Redeem::Admitted
        );
    }

    #[test]
    fn redemption_read_back() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.redemption("l1", "t1").unwrap().is_none());
        s.redeem("l1", "t1", "door_a", 100).unwrap();
        let r = s.redemption("l1", "t1").unwrap().unwrap();
        assert_eq!((r.door_pk.as_str(), r.redeemed_ms), ("door_a", 100));
    }
}
