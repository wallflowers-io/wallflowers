//! Ingest — HMLR bulk files into the local store.
//!
//! Run out-of-band, not on a request: these are monthly files and INSPIRE for England &
//! Wales is millions of polygons. `as_of` is supplied by the operator and is the DATASET
//! VINTAGE (the month HMLR published), never the time of the run — every response quotes it
//! so a caller can judge staleness, and stamping "now" would make stale data look fresh.
//!
//! # Formats
//!
//! * **INSPIRE** ships as GML. A streaming XML parse of gigabytes of GML is its own
//!   project, so this reads the *flattened* form — `inspire_id,ring` where ring is
//!   `lng lat,lng lat,…` — which is what `ogr2ogr`/`GeoPandas` emits in one line:
//!
//!       ogr2ogr -f CSV out.csv Land_Registry_Cadastral_Parcels.gml \
//!               -lco GEOMETRY=AS_WKT -sql "SELECT INSPIREID FROM PREDEFINED"
//!
//!   Doing the GML→CSV step with a real geo toolchain is deliberate: hand-rolling a GML
//!   reader to save one command is how you end up silently mis-projecting a nation.
//!
//!   NOTE ON PROJECTION: INSPIRE is published in **EPSG:27700 (British National Grid)**.
//!   It must be reprojected to WGS84 (EPSG:4326) before ingest — `-t_srs EPSG:4326`. This
//!   loader assumes degrees and [`plausible_wgs84`] rejects easting/northing values loudly
//!   rather than indexing metres as if they were degrees.
//!
//! * **CCOD / OCOD** ship as CSV with a header. Columns vary between releases, so the
//!   header is read by NAME rather than position — a positional reader breaks silently when
//!   HMLR adds a column, and "silently" is the problem.
//!
//! * **NPD** (licensed) supplies the parcel↔title crosswalk that free data lacks.

use crate::store::Store;

/// British National Grid eastings/northings are in the hundreds of thousands. Degrees are
/// not. Catching this at ingest rather than at query time is the difference between a loud
/// failure and an index full of nonsense that resolves nothing.
fn plausible_wgs84(lng: f64, lat: f64) -> bool {
    (-180.0..=180.0).contains(&lng) && (-90.0..=90.0).contains(&lat)
}

pub fn run(
    store: &Store,
    kind: &str,
    file: &str,
    as_of: i64,
    now: i64,
) -> Result<i64, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("read {file}: {e}"))?;
    let n = match kind {
        "inspire" => inspire(store, &text)?,
        "ccod" | "ocod" => titles(store, &text, kind)?,
        "npd" => npd(store, &text)?,
        other => return Err(format!("unknown dataset '{other}'")),
    };
    store
        .record_dataset(kind, as_of, n, now)
        .map_err(|e| e.to_string())?;
    Ok(n)
}

/// `inspire_id,ring` where ring is `lng lat,lng lat,…` in WGS84 degrees.
fn inspire(store: &Store, text: &str) -> Result<i64, String> {
    let mut n = 0i64;
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || lineno == 0 && line.to_lowercase().starts_with("inspire") {
            continue; // header
        }
        let Some((id, ring_s)) = line.split_once(',') else { continue };
        let ring: Vec<(f64, f64)> = ring_s
            .trim_matches('"')
            .split(',')
            .filter_map(|p| {
                let mut it = p.split_whitespace();
                Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
            })
            .collect();
        if ring.len() < 3 {
            continue;
        }
        if let Some((lng, lat)) = ring.first() {
            if !plausible_wgs84(*lng, *lat) {
                return Err(format!(
                    "line {}: ({lng}, {lat}) is not WGS84 degrees. INSPIRE is published in \
                     EPSG:27700 — reproject with `-t_srs EPSG:4326` before ingest.",
                    lineno + 1
                ));
            }
        }
        store.put_parcel(id.trim(), &ring).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// CCOD/OCOD CSV, read BY HEADER NAME so an added column cannot silently shift the data.
fn titles(store: &Store, text: &str, source: &str) -> Result<i64, String> {
    let mut lines = text.lines();
    let header = lines.next().ok_or("empty file")?;
    let cols: Vec<String> = split_csv(header).iter().map(|c| c.to_lowercase()).collect();
    let find = |want: &str| cols.iter().position(|c| c.contains(want));

    let i_title = find("title number").ok_or("no 'Title Number' column")?;
    // HMLR numbers the proprietor columns (1..4); the first is the one we key on.
    let i_name = find("proprietor name (1)")
        .or_else(|| find("proprietor name"))
        .ok_or("no 'Proprietor Name (1)' column")?;
    let i_co = find("company registration no. (1)").or_else(|| find("company registration"));

    let mut n = 0i64;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let f = split_csv(line);
        let Some(title) = f.get(i_title) else { continue };
        let name = f.get(i_name).map(String::as_str).unwrap_or("");
        if title.is_empty() || name.is_empty() {
            continue;
        }
        let co = i_co.and_then(|i| f.get(i)).map(String::as_str).unwrap_or("");
        store
            .put_title(title, name, co, source)
            .map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// The licensed crosswalk: `inspire_id,title_no`.
fn npd(store: &Store, text: &str) -> Result<i64, String> {
    let mut n = 0i64;
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || (lineno == 0 && line.to_lowercase().contains("inspire")) {
            continue;
        }
        let Some((id, title)) = line.split_once(',') else { continue };
        store
            .put_crosswalk(id.trim(), title.trim())
            .map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// Minimal CSV field split honouring double-quoted fields (proprietor names contain commas
/// — "SMITH, JONES AND CO LIMITED" is common, and splitting naively corrupts the register).
fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_commas_in_a_proprietor_name_survive() {
        let f = split_csv(r#"BL1,"SMITH, JONES AND CO LIMITED",12345678"#);
        assert_eq!(f[0], "BL1");
        assert_eq!(f[1], "SMITH, JONES AND CO LIMITED");
        assert_eq!(f[2], "12345678");
    }

    #[test]
    fn ccod_is_read_by_header_name_not_position() {
        let s = Store::in_memory().unwrap();
        // A leading column HMLR did not have last month — a positional reader breaks here.
        let csv = "Something New,Title Number,Tenure,Proprietor Name (1),Company Registration No. (1)\n\
                   x,BL123456,Freehold,BRISTOL CITY COUNCIL,\n";
        let n = titles(&s, csv, "ccod").unwrap();
        assert_eq!(n, 1);
        let (p, _, src) = s.proprietor("BL123456").unwrap().unwrap();
        assert_eq!(p, "BRISTOL CITY COUNCIL");
        assert_eq!(src, "ccod");
    }

    /// The mistake that would silently poison the whole index.
    #[test]
    fn british_national_grid_is_refused_loudly() {
        let s = Store::in_memory().unwrap();
        // Eastings/northings, not degrees — real BNG values near Bristol.
        let csv = "inspire_id,ring\nP1,\"358000 172000,358010 172000,358010 172010\"\n";
        let err = inspire(&s, csv).unwrap_err();
        assert!(err.contains("EPSG:27700"), "names the actual problem: {err}");
        assert!(err.contains("t_srs"), "and the fix");
    }

    #[test]
    fn a_wgs84_ring_ingests_and_resolves() {
        let s = Store::in_memory().unwrap();
        let csv = "inspire_id,ring\n\
                   P1,\"-2.5880 51.4544,-2.5870 51.4544,-2.5870 51.4550,-2.5880 51.4550\"\n";
        assert_eq!(inspire(&s, csv).unwrap(), 1);
        s.record_dataset("inspire", 1_750_000_000_000, 1, 0).unwrap();
        let r = s.resolve(51.4547, -2.5875).unwrap().expect("inside the ring");
        assert_eq!(r.parcel, "P1");
        assert!(r.proprietor.is_none(), "still no owner without a licensed crosswalk");
    }

    #[test]
    fn npd_loads_the_crosswalk_that_free_data_lacks() {
        let s = Store::in_memory().unwrap();
        s.put_parcel("P1", &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]).unwrap();
        s.put_title("BL1", "A COUNCIL", "", "ccod").unwrap();
        assert_eq!(s.crosswalk_rows().unwrap(), 0);

        assert_eq!(npd(&s, "inspire_id,title_no\nP1,BL1\n").unwrap(), 1);
        assert_eq!(s.crosswalk_rows().unwrap(), 1);
        let r = s.resolve(0.5, 0.5).unwrap().unwrap();
        assert_eq!(r.proprietor.as_deref(), Some("A COUNCIL"));
    }
}
