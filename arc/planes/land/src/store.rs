//! The land store — parcels, titles, and the crosswalk that is deliberately empty.
//!
//! # The join problem, which is the whole shape of this module
//!
//! HM Land Registry publishes two free things that between them almost answer "who owns
//! this coordinate", and one paid thing that closes the gap:
//!
//! * **INSPIRE Index Polygons** (free, OGL, monthly GML) — freehold parcel geometry for
//!   England & Wales, keyed by a *Land Registry-INSPIRE ID*. Carries **no proprietor**.
//! * **CCOD / OCOD** (free, monthly CSV) — title number → corporate proprietor. Covers
//!   companies and public bodies; **excludes private individuals**.
//! * **National Polygon Dataset** (CHARGED, signed licence) — polygons carrying **title
//!   numbers**.
//!
//! HMLR states plainly that CCOD/OCOD "contain title numbers, but no direct way of linking
//! records to the polygons in the INSPIRE Index open dataset". So with free data alone the
//! chain coordinate → parcel → title → proprietor is **broken in the middle**, and no
//! amount of engineering closes it.
//!
//! [`Store::crosswalk_rows`] is therefore a first-class part of the API's honesty: it is
//! zero until somebody licenses NPD and ingests it, and every response says so. The
//! alternative — guessing a title from a nearby address, or inferring ownership from
//! proximity — would produce confident assertions about people's property, which is the
//! one output this service must never emit.
//!
//! Downstream enforces the same rule from the other side: pacific-core's place reducer
//! REJECTS a claim that cites `inspire` while naming a proprietor.
//!
//! # Licensing that travels with the data
//!
//! INSPIRE is OGL **but is derived from Ordnance Survey data**, so OS terms apply on top,
//! and attribution is mandatory. CCOD/OCOD carry their own licence. Those strings live in
//! [`Dataset::licence`] so a response can carry them rather than an operator having to
//! remember.

use rusqlite::{params, Connection, OptionalExtension};

/// A loaded dataset and its provenance. `as_of` is the DATASET VINTAGE, not the load time —
/// these are monthly files and ownership moves, so a caller has to be able to judge age.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Dataset {
    pub name: String,
    pub as_of: i64,
    pub rows: i64,
    pub loaded_at: i64,
    pub licence: String,
}

/// What a coordinate resolved to. Every field that could be mistaken for a fact carries its
/// source alongside.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Resolved {
    /// The Land Registry-INSPIRE ID (or NPD title number once licensed).
    pub parcel: String,
    /// Which register this came from — mirrors pacific-core's `LandSource`.
    pub source: String,
    /// Dataset vintage, unix ms.
    pub as_of: i64,
    /// Registered proprietor, only when a licensed crosswalk resolved one.
    pub proprietor: Option<String>,
    pub company_no: Option<String>,
    /// Why no proprietor, when there is none. Present exactly when `proprietor` is None, so
    /// a caller never has to infer whether the gap is "unowned" or "unknowable".
    pub unresolved_because: Option<String>,
    pub licence: String,
}

pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS parcel (
  rowid       INTEGER PRIMARY KEY,
  inspire_id  TEXT UNIQUE NOT NULL,
  -- The ring as "lng lat,lng lat,..." — kept verbatim so the exact test is on the real
  -- geometry rather than the bbox the index searches.
  ring        TEXT NOT NULL
);
-- The spatial index. R*Tree gives us the candidate set; `ring` settles it.
CREATE VIRTUAL TABLE IF NOT EXISTS parcel_bbox USING rtree(
  id, min_lng, max_lng, min_lat, max_lat
);
CREATE TABLE IF NOT EXISTS title (
  title_no    TEXT PRIMARY KEY,
  proprietor  TEXT NOT NULL,
  company_no  TEXT NOT NULL DEFAULT '',
  source      TEXT NOT NULL          -- ccod | ocod
);
-- parcel -> title. EMPTY with free data only; filled by the licensed National Polygon
-- Dataset. Its emptiness is reported, never papered over.
CREATE TABLE IF NOT EXISTS parcel_title (
  inspire_id  TEXT NOT NULL,
  title_no    TEXT NOT NULL,
  PRIMARY KEY (inspire_id, title_no)
);
CREATE TABLE IF NOT EXISTS dataset (
  name        TEXT PRIMARY KEY,      -- inspire | ccod | ocod | npd
  as_of       INTEGER NOT NULL,
  rows        INTEGER NOT NULL,
  loaded_at   INTEGER NOT NULL,
  licence     TEXT NOT NULL DEFAULT ''
);
"#;

/// The licence text that must travel with each dataset. INSPIRE's OS clause is the one
/// operators forget, so it is spelled out rather than left to a wiki page.
pub fn licence_for(dataset: &str) -> &'static str {
    match dataset {
        "inspire" => "OGL, and Ordnance Survey terms apply (HMLR uses OS data to prepare the polygons). Attribution required: 'This information is subject to Crown copyright and is reproduced with the permission of Land Registry'.",
        "ccod" | "ocod" => "HM Land Registry Use Land and Property Data licence — not plain OGL; check onward-use terms before re-supplying.",
        "npd" => "National Polygon Dataset — charged, signed licence required.",
        _ => "",
    }
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn datasets(&self) -> rusqlite::Result<Vec<Dataset>> {
        let mut st = self.conn.prepare(
            "SELECT name, as_of, rows, loaded_at, licence FROM dataset ORDER BY name",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Dataset {
                name: r.get(0)?,
                as_of: r.get(1)?,
                rows: r.get(2)?,
                loaded_at: r.get(3)?,
                licence: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// How many parcel→title links exist. ZERO means the coordinate→proprietor chain is
    /// broken and the API must say so on every response.
    pub fn crosswalk_rows(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM parcel_title", [], |r| r.get(0))
    }

    pub fn record_dataset(&self, name: &str, as_of: i64, rows: i64, now: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO dataset (name, as_of, rows, loaded_at, licence) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(name) DO UPDATE SET as_of=?2, rows=?3, loaded_at=?4, licence=?5",
            params![name, as_of, rows, now, licence_for(name)],
        )?;
        Ok(())
    }

    /// Insert one parcel and index its bounding box.
    pub fn put_parcel(&self, inspire_id: &str, ring: &[(f64, f64)]) -> rusqlite::Result<()> {
        if ring.is_empty() {
            return Ok(());
        }
        let encoded = ring
            .iter()
            .map(|(lng, lat)| format!("{lng} {lat}"))
            .collect::<Vec<_>>()
            .join(",");
        self.conn.execute(
            "INSERT INTO parcel (inspire_id, ring) VALUES (?1, ?2)
             ON CONFLICT(inspire_id) DO UPDATE SET ring=?2",
            params![inspire_id, encoded],
        )?;
        let rowid: i64 = self.conn.query_row(
            "SELECT rowid FROM parcel WHERE inspire_id=?1",
            params![inspire_id],
            |r| r.get(0),
        )?;
        let (mut min_lng, mut max_lng) = (f64::MAX, f64::MIN);
        let (mut min_lat, mut max_lat) = (f64::MAX, f64::MIN);
        for (lng, lat) in ring {
            min_lng = min_lng.min(*lng);
            max_lng = max_lng.max(*lng);
            min_lat = min_lat.min(*lat);
            max_lat = max_lat.max(*lat);
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO parcel_bbox (id, min_lng, max_lng, min_lat, max_lat)
             VALUES (?1,?2,?3,?4,?5)",
            params![rowid, min_lng, max_lng, min_lat, max_lat],
        )?;
        Ok(())
    }

    pub fn put_title(
        &self,
        title_no: &str,
        proprietor: &str,
        company_no: &str,
        source: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO title (title_no, proprietor, company_no, source) VALUES (?1,?2,?3,?4)
             ON CONFLICT(title_no) DO UPDATE SET proprietor=?2, company_no=?3, source=?4",
            params![title_no, proprietor, company_no, source],
        )?;
        Ok(())
    }

    /// Link a parcel to a title. Only the licensed National Polygon Dataset can supply
    /// these; nothing derives them.
    pub fn put_crosswalk(&self, inspire_id: &str, title_no: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO parcel_title (inspire_id, title_no) VALUES (?1,?2)",
            params![inspire_id, title_no],
        )?;
        Ok(())
    }

    /// Look a title number straight up. This works TODAY and needs no geometry — the
    /// useful path when a caller already knows the title (off a paper deed, a council
    /// asset register, a previous lookup).
    pub fn proprietor(&self, title_no: &str) -> rusqlite::Result<Option<(String, String, String)>> {
        self.conn
            .query_row(
                "SELECT proprietor, company_no, source FROM title WHERE title_no=?1",
                params![title_no],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
    }

    /// Resolve a coordinate to a parcel, and to a proprietor ONLY if a licensed crosswalk
    /// can get there.
    ///
    /// Two stages by design: the R*Tree narrows to bbox candidates, then an exact ring test
    /// settles it. A bbox hit alone is not containment — parcels are irregular and a
    /// bounding box routinely covers the neighbour's garden.
    pub fn resolve(&self, lat: f64, lng: f64) -> rusqlite::Result<Option<Resolved>> {
        let mut st = self.conn.prepare(
            "SELECT p.inspire_id, p.ring
               FROM parcel_bbox b JOIN parcel p ON p.rowid = b.id
              WHERE b.min_lng <= ?1 AND b.max_lng >= ?1
                AND b.min_lat <= ?2 AND b.max_lat >= ?2",
        )?;
        let candidates = st.query_map(params![lng, lat], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;

        let mut hit: Option<String> = None;
        for c in candidates {
            let (inspire_id, ring) = c?;
            if point_in_ring(lng, lat, &decode_ring(&ring)) {
                hit = Some(inspire_id);
                break;
            }
        }
        let Some(inspire_id) = hit else { return Ok(None) };

        let inspire = self.dataset("inspire")?;
        let as_of = inspire.as_ref().map(|d| d.as_of).unwrap_or(0);
        let licence = licence_for("inspire").to_string();

        // The crosswalk. Empty with free data only, and the response says which.
        let joined: Option<(String, String, String)> = self
            .conn
            .query_row(
                "SELECT t.proprietor, t.company_no, t.source
                   FROM parcel_title pt JOIN title t ON t.title_no = pt.title_no
                  WHERE pt.inspire_id = ?1",
                params![&inspire_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        Ok(Some(match joined {
            Some((proprietor, company_no, source)) => Resolved {
                parcel: inspire_id,
                source,
                as_of,
                proprietor: Some(proprietor),
                company_no: Some(company_no).filter(|c| !c.is_empty()),
                unresolved_because: None,
                licence,
            },
            None => {
                let reason = if self.crosswalk_rows()? == 0 {
                    "no parcel-to-title crosswalk is loaded: the free INSPIRE dataset carries \
                     no title number, so coordinate-to-proprietor cannot be resolved. Licence \
                     HM Land Registry's National Polygon Dataset and ingest it to enable this."
                } else {
                    "this parcel has no corporate proprietor in CCOD/OCOD — most likely a \
                     private individual, who is published in no free dataset."
                };
                Resolved {
                    parcel: inspire_id,
                    source: "inspire".to_string(),
                    as_of,
                    proprietor: None,
                    company_no: None,
                    unresolved_because: Some(reason.to_string()),
                    licence,
                }
            }
        }))
    }

    fn dataset(&self, name: &str) -> rusqlite::Result<Option<Dataset>> {
        self.conn
            .query_row(
                "SELECT name, as_of, rows, loaded_at, licence FROM dataset WHERE name=?1",
                params![name],
                |r| {
                    Ok(Dataset {
                        name: r.get(0)?,
                        as_of: r.get(1)?,
                        rows: r.get(2)?,
                        loaded_at: r.get(3)?,
                        licence: r.get(4)?,
                    })
                },
            )
            .optional()
    }
}

fn decode_ring(s: &str) -> Vec<(f64, f64)> {
    s.split(',')
        .filter_map(|pair| {
            let mut it = pair.split_whitespace();
            let lng = it.next()?.parse().ok()?;
            let lat = it.next()?.parse().ok()?;
            Some((lng, lat))
        })
        .collect()
}

/// Ray-casting containment. Counts crossings of the horizontal ray to the right of the
/// point; odd means inside. Handles the ring being implicitly closed.
fn point_in_ring(x: f64, y: f64, ring: &[(f64, f64)]) -> bool {
    if ring.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (xi, yi) = ring[i];
        let (xj, yj) = ring[j];
        // Strictly-one-sided y test so a vertex exactly on the ray is counted once, not
        // twice — the classic double-count that reports a point outside its own parcel.
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit square with a notch cut out of it — irregular enough that the bounding box
    /// covers ground the parcel does not.
    fn notched() -> Vec<(f64, f64)> {
        vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 1.0),
            (0.6, 1.0),
            (0.6, 0.4),
            (0.0, 0.4),
        ]
    }

    #[test]
    fn a_bbox_hit_is_not_containment() {
        let s = Store::in_memory().unwrap();
        s.put_parcel("P1", &notched()).unwrap();
        s.record_dataset("inspire", 1_750_000_000_000, 1, 0).unwrap();

        // Inside the parcel proper.
        let r = s.resolve(0.2, 0.2).unwrap().expect("inside");
        assert_eq!(r.parcel, "P1");

        // Inside the BOUNDING BOX but in the notch — must not resolve. This is the case a
        // bbox-only index gets wrong, and it is somebody else's garden.
        assert!(s.resolve(0.8, 0.2).unwrap().is_none(), "the notch is not the parcel");
    }

    #[test]
    fn nothing_resolves_off_the_map() {
        let s = Store::in_memory().unwrap();
        s.put_parcel("P1", &notched()).unwrap();
        assert!(s.resolve(51.4545, -2.5879).unwrap().is_none());
    }

    /// The headline honesty property: with free data only, a parcel resolves but an owner
    /// does not, and the response says exactly why.
    #[test]
    fn free_data_resolves_a_parcel_but_never_an_owner() {
        let s = Store::in_memory().unwrap();
        s.put_parcel("P1", &notched()).unwrap();
        s.record_dataset("inspire", 1_750_000_000_000, 1, 0).unwrap();
        // CCOD loaded — but with no crosswalk it cannot be reached from a coordinate.
        s.put_title("BL123456", "BRISTOL CITY COUNCIL", "", "ccod").unwrap();
        s.record_dataset("ccod", 1_750_000_000_000, 1, 0).unwrap();

        assert_eq!(s.crosswalk_rows().unwrap(), 0);
        let r = s.resolve(0.2, 0.2).unwrap().unwrap();
        assert_eq!(r.source, "inspire");
        assert!(r.proprietor.is_none(), "no join exists, so no owner is asserted");
        let why = r.unresolved_because.expect("must say why");
        assert!(why.contains("National Polygon Dataset"), "names the actual fix: {why}");
        assert!(!r.licence.is_empty(), "and carries its licence terms");
        assert!(r.licence.contains("Ordnance Survey"), "including the OS clause");

        // The title lookup DOES work today — it just needs a title number, not a point.
        let (p, _, src) = s.proprietor("BL123456").unwrap().expect("title lookup");
        assert_eq!(p, "BRISTOL CITY COUNCIL");
        assert_eq!(src, "ccod");
    }

    /// Once NPD is licensed and the crosswalk lands, the same coordinate resolves an owner
    /// and the source changes to the register that actually named them.
    #[test]
    fn a_licensed_crosswalk_completes_the_chain() {
        let s = Store::in_memory().unwrap();
        s.put_parcel("P1", &notched()).unwrap();
        s.record_dataset("inspire", 1_750_000_000_000, 1, 0).unwrap();
        s.put_title("BL123456", "BRISTOL CITY COUNCIL", "", "ccod").unwrap();
        s.put_crosswalk("P1", "BL123456").unwrap();
        s.record_dataset("npd", 1_750_000_000_000, 1, 0).unwrap();

        let r = s.resolve(0.2, 0.2).unwrap().unwrap();
        assert_eq!(r.proprietor.as_deref(), Some("BRISTOL CITY COUNCIL"));
        assert_eq!(r.source, "ccod", "credited to the register that named the owner");
        assert!(r.unresolved_because.is_none());
    }

    /// A parcel with a crosswalk to a title nobody corporate holds: the honest answer is
    /// "probably a private individual", not silence.
    #[test]
    fn a_private_owner_is_reported_as_unpublished() {
        let s = Store::in_memory().unwrap();
        s.put_parcel("P1", &notched()).unwrap();
        s.put_parcel("P2", &[(2.0, 2.0), (3.0, 2.0), (3.0, 3.0), (2.0, 3.0)]).unwrap();
        s.put_title("BL1", "A COUNCIL", "", "ccod").unwrap();
        s.put_crosswalk("P2", "BL1").unwrap();

        let r = s.resolve(0.2, 0.2).unwrap().unwrap();
        assert!(r.proprietor.is_none());
        let why = r.unresolved_because.unwrap();
        assert!(why.contains("private individual"), "distinguishes the two gaps: {why}");
    }
}
