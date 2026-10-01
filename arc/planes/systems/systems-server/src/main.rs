//! systems-server — the Arc's `/systems` plane as a standalone service.
//!
//! Runs the Nango connectors backend-side (the Nango secret is here, in the env, never on the
//! device) and returns citable RAG text for the Core Loop Agent. The device holds only its
//! `NangoConnection` id and calls `POST /systems/gmail/search`. No sync, no persistence — each
//! query is live. Mirrors `arc-node`'s systems route but carries none of arc-node's pacific-core
//! (signup) baggage, so it builds and deploys on its own.
//!
//! Env:
//!   PORT              bind port (Railway sets it; default 8790)
//!   NANGO_SECRET_KEY  the Nango secret (required for a real search; absent → 501)
//!   NANGO_BASE_URL    the Nango broker base (default https://api.nango.dev)
//!
//! Routes:
//!   GET  /health                 -> "ok"
//!   POST /systems/gmail/search   -> { text, refs }   body: { connection_id, query, max? }

use gmail::{GmailBridge, PROVIDER_CONFIG_KEY};
use nango::{NangoConnection, NangoEnvironment, NangoHttpTransport};
use serde::Deserialize;
use serde_json::json;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn respond(req: tiny_http::Request, code: u16, ctype: &str, body: String) {
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes())
        .expect("static header");
    let resp = tiny_http::Response::from_string(body)
        .with_status_code(code)
        .with_header(header);
    let _ = req.respond(resp);
}

fn err_json(msg: &str) -> String {
    json!({ "error": msg }).to_string()
}

/// A device's request to search its connected Gmail.
#[derive(Deserialize)]
struct GmailSearchRequest {
    connection_id: String,
    query: String,
    #[serde(default)]
    max: Option<u32>,
}

/// Run a live Gmail search for one device connection and render hits as RAG text. Returns the
/// JSON body, or `(http_code, message)` — LOUD on failure, never a fabricated empty result.
fn gmail_search(body: &str) -> Result<String, (u16, String)> {
    let req: GmailSearchRequest = serde_json::from_str(body)
        .map_err(|e| (400, format!("bad body (want {{connection_id, query, max?}}): {e}")))?;
    let connection_id = req.connection_id.trim();
    let query = req.query.trim();
    if connection_id.is_empty() {
        return Err((400, "connection_id is required".into()));
    }
    if query.is_empty() {
        return Err((400, "query is required".into()));
    }

    let secret = std::env::var("NANGO_SECRET_KEY")
        .map_err(|_| (501, "Gmail is unavailable: NANGO_SECRET_KEY is not configured".to_string()))?;
    let base = env_or("NANGO_BASE_URL", &NangoEnvironment::cloud().base_url);
    let max = req.max.unwrap_or(nango_rag::DEFAULT_MAX_HITS);

    let transport = NangoHttpTransport::new(base, secret);
    let connection = NangoConnection::new(connection_id, PROVIDER_CONFIG_KEY);
    let bridge = GmailBridge::new(transport, connection);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| (500, format!("runtime: {e}")))?;
    let hits = rt
        .block_on(nango_rag::fetch_gmail(&bridge, query, max))
        .map_err(|e| (502, format!("gmail search failed: {e:?}")))?;

    let (text, refs) = nango_rag::render_gmail_hits(query, &hits);
    Ok(json!({ "text": text, "refs": refs }).to_string())
}

fn main() {
    let port = env_or("PORT", "8790");
    let bind = format!("0.0.0.0:{port}");
    let server = tiny_http::Server::http(&bind).unwrap_or_else(|e| {
        eprintln!("systems-server: could not bind {bind}: {e}");
        std::process::exit(1);
    });
    let has_secret = std::env::var("NANGO_SECRET_KEY").is_ok();
    eprintln!("systems-server: listening on http://{bind}  (NANGO_SECRET_KEY {})",
              if has_secret { "set" } else { "MISSING — searches will 501" });

    for mut req in server.incoming_requests() {
        let method = req.method().clone();
        let path = req.url().split('?').next().unwrap_or("/").to_string();

        if method == tiny_http::Method::Get && path == "/health" {
            respond(req, 200, "text/plain", "ok".into());
        } else if method == tiny_http::Method::Post && path == "/systems/gmail/search" {
            let mut body = String::new();
            if req.as_reader().read_to_string(&mut body).is_err() {
                respond(req, 400, "application/json", err_json("could not read request body"));
                continue;
            }
            match gmail_search(&body) {
                Ok(json) => respond(req, 200, "application/json", json),
                Err((code, msg)) => respond(req, code, "application/json", err_json(&msg)),
            }
        } else {
            respond(req, 404, "application/json",
                    err_json("not found — try GET /health or POST /systems/gmail/search"));
        }
    }
}
