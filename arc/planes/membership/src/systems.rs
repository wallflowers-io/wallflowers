//! systems — the Arc's `/systems` plane: run a Nango connector on behalf of a device and
//! return citable RAG context for the Core Loop Agent (the CLA).
//!
//! This is the backend half of Nango's credential model: the SECRET key lives here (from the
//! Arc's env / secret store), never on the device. The device holds only its `NangoConnection`
//! id (established once via a Connect session) and supplies it with each query. Nothing is
//! synced — the connector is queried live, per request, and the hits are rendered to the text
//! the CLA reads as a tool_result.

use gmail::{GmailBridge, PROVIDER_CONFIG_KEY};
use nango::{NangoConnection, NangoEnvironment, NangoHttpTransport};
use serde::Deserialize;
use serde_json::json;

/// A device's request to search its connected Gmail: which connection, what query, how many.
#[derive(Deserialize)]
struct GmailSearchRequest {
    /// The Nango connection id the device holds for its Gmail link.
    connection_id: String,
    /// The Gmail search query (Gmail query syntax, or plain terms).
    query: String,
    /// Cap on hits; defaults to `nango_rag::DEFAULT_MAX_HITS`.
    #[serde(default)]
    max: Option<u32>,
}

/// Run a live Gmail search for one device connection and render the hits as RAG text.
/// Returns the JSON body on success, or `(http_code, message)` on failure — LOUD, never a
/// fabricated empty result: a missing secret is 501, a bad request 400, a proxy failure 502.
pub fn gmail_search(body: &str) -> Result<String, (u16, String)> {
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

    // The Nango secret is Arc-side only. Absent → the Arc simply can't offer Gmail; say so.
    let secret = std::env::var("NANGO_SECRET_KEY")
        .map_err(|_| (501, "Gmail is unavailable: this Arc has no NANGO_SECRET_KEY configured".to_string()))?;
    let base = std::env::var("NANGO_BASE_URL").unwrap_or_else(|_| NangoEnvironment::cloud().base_url);
    let max = req.max.unwrap_or(nango_rag::DEFAULT_MAX_HITS);

    let transport = NangoHttpTransport::new(base, secret);
    let connection = NangoConnection::new(connection_id, PROVIDER_CONFIG_KEY);
    let bridge = GmailBridge::new(transport, connection);

    // The connector is async; run it on a per-call current-thread runtime (mirrors this
    // server's low-volume, per-request model — see main.rs `block_on`).
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
