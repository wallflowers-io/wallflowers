//! land — the Arc's `/land` plane: UK land tenure for Place anchors.
//!
//! A Pacific Place can be anchored to anything physical — a bench, a tree, an allotment
//! gate — and the question that follows is *whose land is that*. This plane answers it from
//! HM Land Registry's published data, and is explicit about the large part it cannot
//! answer.
//!
//! # Why Arc-side and not on the device
//!
//! The source data is monthly bulk files measured in gigabytes: INSPIRE Index Polygons are
//! millions of polygons for England & Wales. A phone does not ingest that. The device asks
//! this plane, receives an answer WITH ITS PROVENANCE, and authors it as a `LandClaim`
//! delta on its Place (pacific-core `place.rs`) — signed, with the dataset vintage, so
//! every reader can judge how stale it is.
//!
//! # What it can and cannot do
//!
//! With free data only:
//!   * coordinate → parcel                 YES  (INSPIRE)
//!   * title number → proprietor           YES  (CCOD/OCOD, corporate bodies only)
//!   * coordinate → proprietor             NO   (no published parcel↔title crosswalk)
//!
//! HMLR states that CCOD/OCOD "contain title numbers, but no direct way of linking records
//! to the polygons in the INSPIRE Index open dataset". Licensing the National Polygon
//! Dataset closes it; until then every response says so, in the response, not in a doc.
//! See `store.rs` for the full reasoning.
//!
//! Nothing here guesses. No inferring ownership from a nearby address, no proximity
//! heuristics — this service must never emit a confident assertion about someone's
//! property.
//!
//! # A register is not permission
//!
//! Even a complete answer tells you only WHO TO ASK. Whether they agreed is a social fact
//! no register holds, which is why `LandClaim.permission` is a separate self-reported field
//! defaulting to `unknown`. This plane never returns a permission.
//!
//! Env:
//!   PORT       bind port (default 8791)
//!   LAND_DB    sqlite path (default ./land.db)
//!
//! Routes:
//!   GET /health                      -> "ok"
//!   GET /land/datasets               -> what is loaded, vintages, licences, capability
//!   GET /land/parcel?lat=&lng=       -> resolve a coordinate
//!   GET /land/proprietor?title=      -> title number -> corporate proprietor
//!
//! Ingest (same binary, run out-of-band — these are big files):
//!   land ingest inspire <file.gml|.csv> <as_of_unix_ms>
//!   land ingest ccod    <file.csv>      <as_of_unix_ms>
//!   land ingest ocod    <file.csv>      <as_of_unix_ms>
//!   land ingest npd     <file.csv>      <as_of_unix_ms>   (licensed crosswalk)

mod ingest;
mod store;

use serde_json::json;
use store::Store;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn respond(req: tiny_http::Request, code: u16, body: String) {
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header");
    let resp = tiny_http::Response::from_string(body)
        .with_status_code(code)
        .with_header(header);
    let _ = req.respond(resp);
}

/// Parse `?a=1&b=2` without pulling a URL crate in for it.
fn query(url: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    if let Some(qs) = url.split_once('?').map(|(_, q)| q) {
        for pair in qs.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                out.insert(k.to_string(), v.to_string());
            }
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let db = env_or("LAND_DB", "./land.db");

    if args.len() > 1 && args[1] == "ingest" {
        if args.len() < 5 {
            eprintln!("usage: land ingest <inspire|ccod|ocod|npd> <file> <as_of_unix_ms>");
            std::process::exit(2);
        }
        let (kind, file) = (args[2].as_str(), args[3].as_str());
        let as_of: i64 = args[4].parse().unwrap_or_else(|_| {
            eprintln!("as_of must be unix ms — the DATASET vintage, not now");
            std::process::exit(2);
        });
        let store = Store::open(&db).expect("open land db");
        match ingest::run(&store, kind, file, as_of, now_ms()) {
            Ok(n) => println!("ingested {n} rows of {kind} (as_of {as_of}) into {db}"),
            Err(e) => {
                eprintln!("ingest failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let port = env_or("PORT", "8791");
    let store = Store::open(&db).expect("open land db");
    let server = tiny_http::Server::http(format!("0.0.0.0:{port}")).expect("bind");
    eprintln!("land plane listening on :{port} (db {db})");

    for req in server.incoming_requests() {
        let url = req.url().to_string();
        let path = url.split('?').next().unwrap_or("").to_string();
        let q = query(&url);

        match path.as_str() {
            "/health" => respond(req, 200, json!({ "status": "ok" }).to_string()),

            // Honest capability, in the gateway's own idiom: say what is loaded and what
            // that does and does not make answerable.
            "/land/datasets" => {
                let sets = store.datasets().unwrap_or_default();
                let crosswalk = store.crosswalk_rows().unwrap_or(0);
                respond(
                    req,
                    200,
                    json!({
                        "datasets": sets,
                        "crosswalk_rows": crosswalk,
                        "can": {
                            "coordinate_to_parcel": sets.iter().any(|d| d.name == "inspire"),
                            "title_to_proprietor": sets.iter().any(|d| d.name == "ccod" || d.name == "ocod"),
                            // The one that matters, and the one that is false by default.
                            "coordinate_to_proprietor": crosswalk > 0,
                        },
                        "note": if crosswalk == 0 {
                            "coordinate-to-proprietor is unavailable: the free INSPIRE dataset \
                             carries no title number, so there is no published crosswalk to \
                             CCOD/OCOD. Licence HM Land Registry's National Polygon Dataset to \
                             enable it. A register never tells you whether the owner agreed."
                        } else {
                            "a licensed crosswalk is loaded. A register still never tells you \
                             whether the owner agreed — permission is recorded separately."
                        }
                    })
                    .to_string(),
                )
            }

            "/land/parcel" => {
                let lat = q.get("lat").and_then(|v| v.parse::<f64>().ok());
                let lng = q.get("lng").and_then(|v| v.parse::<f64>().ok());
                let (Some(lat), Some(lng)) = (lat, lng) else {
                    respond(req, 400, json!({ "error": "lat and lng are required" }).to_string());
                    continue;
                };
                if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
                    respond(req, 400, json!({ "error": "lat/lng out of range" }).to_string());
                    continue;
                }
                match store.resolve(lat, lng) {
                    // Not found is a real answer: ~a seventh of England & Wales is
                    // unregistered, and Scotland/NI are separate registers this plane does
                    // not hold. Say which, rather than implying nobody owns it.
                    Ok(None) => respond(
                        req,
                        404,
                        json!({
                            "error": "no registered parcel found at that point",
                            "note": "this is not evidence the land is unowned: unregistered \
                                     land exists, INSPIRE covers freehold in England & Wales \
                                     only, and Scotland (ScotLIS) and Northern Ireland are \
                                     separate registers not held here."
                        })
                        .to_string(),
                    ),
                    Ok(Some(r)) => respond(req, 200, serde_json::to_string(&r).unwrap_or_default()),
                    Err(e) => respond(req, 500, json!({ "error": e.to_string() }).to_string()),
                }
            }

            "/land/proprietor" => {
                let Some(title) = q.get("title") else {
                    respond(req, 400, json!({ "error": "title is required" }).to_string());
                    continue;
                };
                match store.proprietor(title) {
                    Ok(Some((proprietor, company_no, source))) => respond(
                        req,
                        200,
                        json!({
                            "title": title,
                            "proprietor": proprietor,
                            "company_no": company_no,
                            "source": source,
                            "licence": store::licence_for(&source),
                        })
                        .to_string(),
                    ),
                    Ok(None) => respond(
                        req,
                        404,
                        json!({
                            "error": "no corporate proprietor for that title",
                            "note": "CCOD/OCOD cover companies and public bodies only. A \
                                     private individual, a charity, or a UK company with an \
                                     overseas address is in no free dataset — absence here is \
                                     not absence of an owner."
                        })
                        .to_string(),
                    ),
                    Err(e) => respond(req, 500, json!({ "error": e.to_string() }).to_string()),
                }
            }

            _ => respond(
                req,
                404,
                json!({ "error": "unknown route — see /land/datasets" }).to_string(),
            ),
        }
    }
}
