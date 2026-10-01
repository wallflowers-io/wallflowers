//! training_wheels' PostgreSQL (P0): `action` and `delta`, on the local socket, one
//! connection, remade after any failure. Every write is idempotent on its row's key, so a
//! batch delivered twice (a drain cut short after the commit) lands once. Parameterised
//! statements only: no body ever reaches a statement's text, or PostgreSQL's log (B4).

use std::sync::Mutex;

use super::Row;

/// P0's tables. `action` rows keep their UUIDv7; a backfilled row is keyed on its Delta, so a
/// second first run seeds nothing. `delta` is the MLS model's own record, on (object, Delta).
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS action (
  id         uuid PRIMARY KEY,
  at         timestamptz NOT NULL,
  account    text,
  session    text,
  route      text NOT NULL,
  kind       text,
  op         text,
  object     text,
  args       jsonb,
  outcome    jsonb NOT NULL,
  backfilled boolean NOT NULL DEFAULT false
);
CREATE UNIQUE INDEX IF NOT EXISTS action_backfilled ON action (object, (outcome->>'delta')) WHERE backfilled;
CREATE TABLE IF NOT EXISTS delta (
  object   text NOT NULL,
  delta_id text NOT NULL,
  author   text,
  kind     text,
  op       text,
  args     jsonb,
  epoch    bigint,
  seq      bigint,
  gen      bigint,
  at       timestamptz NOT NULL,
  PRIMARY KEY (object, delta_id)
);
";

pub struct Pg {
    dsn: String,
    conn: Mutex<Option<postgres::Client>>,
}

impl Pg {
    pub fn new(dsn: &str) -> Self {
        Self { dsn: dsn.to_string(), conn: Mutex::new(None) }
    }

    /// Every row in one transaction, or none. A failure drops the connection; the next put
    /// connects again.
    pub fn put(&self, rows: &[Row]) -> Result<(), String> {
        let mut held = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        if held.is_none() {
            let mut c = postgres::Client::connect(&self.dsn, postgres::NoTls).map_err(|e| format!("connect: {}", why(&e)))?;
            c.batch_execute(SCHEMA).map_err(|e| format!("schema: {}", why(&e)))?;
            *held = Some(c);
        }
        let out = Self::write(held.as_mut().expect("connected"), rows);
        if out.is_err() {
            *held = None;
        }
        out
    }

    fn write(c: &mut postgres::Client, rows: &[Row]) -> Result<(), String> {
        let mut tx = c.transaction().map_err(|e| why(&e))?;
        for r in rows {
            let s = |k: &str| r.get(k).and_then(|v| v.as_str()).map(str::to_string);
            let n = |k: &str| r.get(k).and_then(|v| v.as_i64());
            let at = n("at").unwrap_or(0) as f64 / 1000.0;
            match r.get("table").and_then(|t| t.as_str()) {
                Some("action") => tx.execute(
                    "INSERT INTO action (id, at, account, session, route, kind, op, object, args, outcome, backfilled)
                     VALUES ($1::text::uuid, to_timestamp($2), $3, $4, $5, $6, $7, $8, $9, $10, $11)
                     ON CONFLICT DO NOTHING",
                    &[
                        &s("id").unwrap_or_default(),
                        &at,
                        &s("account"),
                        &s("session"),
                        &s("route").unwrap_or_default(),
                        &s("kind"),
                        &s("op"),
                        &s("object"),
                        &postgres::types::Json(r.get("args").cloned().unwrap_or_default()),
                        &postgres::types::Json(r.get("outcome").cloned().unwrap_or_default()),
                        &r.get("backfilled").and_then(|b| b.as_bool()).unwrap_or(false),
                    ],
                ),
                Some("delta") => tx
                    .execute(
                        "INSERT INTO delta (object, delta_id, author, kind, op, args, epoch, seq, gen, at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, to_timestamp($10))
                         ON CONFLICT DO NOTHING",
                        &[
                            &s("object").unwrap_or_default(),
                            &s("delta_id").unwrap_or_default(),
                            &s("author"),
                            &s("kind"),
                            &s("op"),
                            &postgres::types::Json(r.get("args").cloned().unwrap_or_default()),
                            &n("epoch"),
                            &n("seq"),
                            &n("gen"),
                            &at,
                        ],
                    )
                    // THE BACKFILL (Ralph, 28 Sep: keep the mint): a Delta held from before
                    // the first action this database recorded seeds one action, marked, by
                    // its author, op and args. Once: the backfilled index is on (object,
                    // Delta), and a Delta a recorded action already names is left to it. A
                    // Delta's time is in whole seconds, so "before" is before that action's
                    // second: one made in it, by it or after, is not history.
                    .and_then(|_| {
                        tx.execute(
                            "INSERT INTO action (id, at, account, session, route, kind, op, object, args, outcome, backfilled)
                             SELECT $1::text::uuid, to_timestamp($2), $3, NULL, 'backfill', $4, $5, $6, $7,
                                    jsonb_build_object('delta', $8::text), true
                             WHERE to_timestamp($2) < (SELECT date_trunc('second', coalesce(min(at), 'infinity'::timestamptz)) FROM action WHERE NOT backfilled)
                               AND NOT EXISTS (SELECT 1 FROM action WHERE object = $6 AND outcome->>'delta' = $8::text)
                             ON CONFLICT DO NOTHING",
                            &[
                                &uuid::Uuid::now_v7().to_string(),
                                &at,
                                &s("author"),
                                &s("kind"),
                                &s("op"),
                                &s("object").unwrap_or_default(),
                                &postgres::types::Json(r.get("args").cloned().unwrap_or_default()),
                                &s("delta_id").unwrap_or_default(),
                            ],
                        )
                    }),
                other => return Err(format!("a row for no table: {other:?}")),
            }
            .map_err(|e| why(&e))?;
        }
        tx.commit().map_err(|e| why(&e))
    }
}

/// A failure in words that carry no row: a database error by its SQLSTATE alone, since its
/// message and detail can quote the failing row ("Failing row contains …"), and this goes to
/// the journal, outside the volume (B4).
fn why(e: &postgres::Error) -> String {
    match e.code() {
        Some(c) => format!("sqlstate {}", c.code()),
        None if e.is_closed() => "the connection is closed".into(),
        None => "no connection".into(),
    }
}
