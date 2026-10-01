//! THE MODEL, SERVED SMALL AND CACHED (UX's perf audit, mdr/perf-audit.html: 126 KB, no-store,
//! uncompressed, 0.8 s warm from Korea on every signed-in landing). The ICD this binary embeds,
//! gzip-compressed once, at start (the router asks its hash), served two ways:
//!
//! - `/v2/icd/<its sha256>`: immutable, for a year; any other hash is 404. The webapp's page
//!   names the hash, so a landing fetches the model once, and again only when it changes.
//! - `/v2/icd`: for PIN-5's verify and a page that names no hash: a minute's cache, an ETag.
//!
//! Each is gzip where the request accepts it and the bytes themselves otherwise, with `Vary:
//! Accept-Encoding`. Brotli waits on its encoder crate, which is not in the offline cache.

use std::io::Write;
use std::sync::OnceLock;

use axum::{
    extract::Path,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use sha2::Digest;

/// The model, as this binary was built with it.
pub const BYTES: &str = include_str!("../../../core/coordination/delta-graph.icd.json");

/// The model itself, parsed once from `BYTES`: what the Door reads of the ICD, it reads here.
pub fn model() -> &'static serde_json::Value {
    static M: OnceLock<serde_json::Value> = OnceLock::new();
    M.get_or_init(|| serde_json::from_str(BYTES).expect("the ICD the Door serves parses"))
}

struct Served {
    hash: String,
    gzip: Vec<u8>,
}

fn served() -> &'static Served {
    static S: OnceLock<Served> = OnceLock::new();
    S.get_or_init(|| {
        let mut z = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(BYTES.as_bytes()).expect("gzip into memory");
        Served { hash: hex::encode(sha2::Sha256::digest(BYTES)), gzip: z.finish().expect("gzip into memory") }
    })
}

/// The model's sha256, hex: the pin's, where `make check-icd` passes.
pub fn hash() -> &'static str {
    &served().hash
}

/// Whether an Accept-Encoding takes gzip: named with q above 0, or, unnamed, `*` with q above 0.
pub fn takes_gzip(accept: &str) -> bool {
    let mut star = false;
    for item in accept.split(',') {
        let mut parts = item.split(';');
        let name = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let q = parts
            .find_map(|p| {
                let p = p.trim().to_ascii_lowercase();
                p.strip_prefix("q=").map(|v| v.trim().parse::<f32>().unwrap_or(0.0))
            })
            .unwrap_or(1.0);
        match name.as_str() {
            "gzip" | "x-gzip" => return q > 0.0,
            "*" => star = q > 0.0,
            _ => {}
        }
    }
    star
}

/// The model in the encoding `h` accepts, under `cache`; 304 where `h` already holds it.
fn answer(h: &HeaderMap, cache: &'static str) -> Response {
    let s = served();
    let gz = h.get(header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()).is_some_and(takes_gzip);
    // One tag per representation (RFC 9110 §8.8.3): the gzip is not the bytes.
    let etag = if gz { format!("\"{}-gz\"", s.hash) } else { format!("\"{}\"", s.hash) };
    let mut r = if h.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|m| m.split(',').any(|t| t.trim() == etag || t.trim() == "*")) {
        StatusCode::NOT_MODIFIED.into_response()
    } else if gz {
        ([(header::CONTENT_TYPE, "application/json"), (header::CONTENT_ENCODING, "gzip")], s.gzip.clone()).into_response()
    } else {
        ([(header::CONTENT_TYPE, "application/json")], BYTES).into_response()
    };
    let head = r.headers_mut();
    head.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    head.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    head.insert(header::ETAG, HeaderValue::from_str(&etag).expect("hex"));
    r
}

/// `GET /v2/icd`.
pub async fn current(h: HeaderMap) -> Response {
    answer(&h, "public, max-age=60")
}

/// `GET /v2/icd/<hash>`: this binary's model, for good; any other, 404.
pub async fn by_hash(Path(hash): Path<String>, h: HeaderMap) -> Response {
    if hash != served().hash {
        return (StatusCode::NOT_FOUND, "not this Door's model").into_response();
    }
    answer(&h, "public, max-age=31536000, immutable")
}

/// The webapp's page, naming the model's hash in `<meta id="wallflowers-icd">`: revalidated on
/// every load, since what it names changes with a deploy.
pub async fn page(index: &std::path::Path, h: &HeaderMap) -> Response {
    let Ok(html) = tokio::fs::read_to_string(index).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let meta = format!("<meta name=\"wallflowers-icd\" id=\"wallflowers-icd\" content=\"{}\">\n</head>", hash());
    let html = html.replacen("</head>", &meta, 1);
    let etag = format!("\"{}\"", &hex::encode(sha2::Sha256::digest(&html))[..32]);
    let mut r = if h.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|m| m.split(',').any(|t| t.trim() == etag)) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
    };
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    r.headers_mut().insert(header::ETAG, HeaderValue::from_str(&etag).expect("hex"));
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn with(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(k.clone(), v.parse().unwrap());
        }
        h
    }

    async fn body(r: Response) -> Vec<u8> {
        axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()
    }

    /// The hash is the pin's, and what is served, decompressed or not, is the pinned bytes.
    #[tokio::test]
    async fn the_model_served_is_the_pins_in_either_encoding() {
        let pin = include_str!("../../../core/coordination/delta-graph.icd.sha256");
        assert_eq!(Some(hash()), pin.split_whitespace().next(), "the hashed path names the pin");
        let plain = body(current(HeaderMap::new()).await).await;
        assert_eq!(hex::encode(sha2::Sha256::digest(&plain)), hash());
        let r = by_hash(Path(hash().to_string()), with(&[(header::ACCEPT_ENCODING, "gzip, deflate, br")])).await;
        assert_eq!(r.headers()[header::CONTENT_ENCODING], "gzip");
        let gz = body(r).await;
        assert!(gz.len() * 3 < plain.len(), "gzip {} of {} bytes", gz.len(), plain.len());
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut out).unwrap();
        assert_eq!(out, plain);
    }

    #[test]
    fn gzip_goes_where_it_is_accepted() {
        for (accept, gz) in [
            ("gzip", true),
            ("gzip, deflate, br, zstd", true),
            ("br;q=1.0, gzip;q=0.8", true),
            ("GZIP", true),
            ("*", true),
            ("gzip;q=0", false),
            ("gzip; Q=0, *", false),
            ("*;q=0", false),
            ("identity", false),
            ("br", false),
            ("", false),
        ] {
            assert_eq!(takes_gzip(accept), gz, "{accept:?}");
        }
    }

    /// Asked for without gzip (PIN-5's curl), the bytes; with it, gzip; either way, Vary.
    #[tokio::test]
    async fn the_encoding_follows_accept_encoding_and_varies_on_it() {
        let plain = current(HeaderMap::new()).await;
        assert!(plain.headers().get(header::CONTENT_ENCODING).is_none());
        let gz = current(with(&[(header::ACCEPT_ENCODING, "gzip")])).await;
        assert_eq!(gz.headers()[header::CONTENT_ENCODING], "gzip");
        for r in [&plain, &gz] {
            assert_eq!(r.headers()[header::VARY], "accept-encoding");
            assert_eq!(r.headers()[header::CONTENT_TYPE], "application/json");
        }
        assert_ne!(plain.headers()[header::ETAG], gz.headers()[header::ETAG], "one tag per representation");
    }

    /// Immutable on the hashed path alone; a minute, and revalidation by ETag, on /v2/icd.
    #[tokio::test]
    async fn only_the_hashed_path_is_immutable_and_another_hash_is_404() {
        let r = by_hash(Path(hash().to_string()), HeaderMap::new()).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
        let r = current(HeaderMap::new()).await;
        assert_eq!(r.headers()[header::CACHE_CONTROL], "public, max-age=60");
        let tag = r.headers()[header::ETAG].to_str().unwrap().to_string();
        let again = current(with(&[(header::IF_NONE_MATCH, &tag)])).await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        assert!(body(again).await.is_empty());
        let stale = current(with(&[(header::IF_NONE_MATCH, "\"0000\"")])).await;
        assert_eq!(stale.status(), StatusCode::OK);

        for other in ["0".repeat(64), hash()[..63].to_string(), hash().to_uppercase(), "latest".into()] {
            let r = by_hash(Path(other.clone()), HeaderMap::new()).await;
            assert_eq!(r.status(), StatusCode::NOT_FOUND, "{other}");
            assert!(r.headers().get(header::CACHE_CONTROL).is_none(), "a 404 is not cached for good");
        }
    }

    /// The page names the hash in its head, once, and is revalidated on every load.
    #[tokio::test]
    async fn the_page_names_the_models_hash() {
        let dir = std::env::temp_dir().join(format!("door-icd-page-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let index = dir.join("index.html");
        std::fs::write(&index, "<!doctype html><html><head><title>x</title></head><body></head></body></html>").unwrap();
        let r = page(&index, &HeaderMap::new()).await;
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-cache");
        let tag = r.headers()[header::ETAG].to_str().unwrap().to_string();
        let html = String::from_utf8(body(r).await).unwrap();
        let meta = format!("<meta name=\"wallflowers-icd\" id=\"wallflowers-icd\" content=\"{}\">", hash());
        assert_eq!(html.matches(&meta).count(), 1, "{html}");
        assert!(html.find(&meta).unwrap() < html.find("<body>").unwrap(), "in the head");
        assert_eq!(page(&index, &with(&[(header::IF_NONE_MATCH, &tag)])).await.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(page(&dir.join("absent.html"), &HeaderMap::new()).await.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
