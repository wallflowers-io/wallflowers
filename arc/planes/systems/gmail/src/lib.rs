//! gmail — a Pacific System on the Nango seam: Gmail (search, get, send).
//!
//! This is the *connector / action* surface for Gmail (search threads, read a
//! message, send mail on demand) — distinct from Pacific's on-device index
//! (LodeDB), which keeps mail content local. Calls are proxied through
//! Nango, so they transit the broker; nothing is indexed here.

use base64::Engine;
use nango::{NangoConnection, NangoError, NangoTransport, ProxyRequest};

/// The Nango integration key this connector binds to (Nango's Gmail provider is
/// `google-mail`; match the integration's Unique Key in Nango).
pub const PROVIDER_CONFIG_KEY: &str = "google-mail";

/// A light reference (Gmail's `messages.list` returns ids only).
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct MessageRef {
    pub id: String,
    #[serde(rename = "threadId")]
    pub thread_id: String,
}

/// A fetched message's metadata + snippet.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub snippet: Option<String>,
    pub subject: Option<String>,
    pub from: Option<String>,
    /// Gmail `internalDate` — epoch **milliseconds** the message was received.
    /// Returned on the message resource regardless of `format`; the event-time
    /// anchor when the message becomes a graph episode.
    pub internal_date_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GmailError {
    Transport(NangoError),
    Decode(String),
}

impl From<NangoError> for GmailError {
    fn from(e: NangoError) -> Self {
        GmailError::Transport(e)
    }
}

pub struct GmailBridge<T: NangoTransport> {
    pub transport: T,
    pub connection: NangoConnection,
}

impl<T: NangoTransport> GmailBridge<T> {
    pub fn new(transport: T, connection: NangoConnection) -> Self {
        Self { transport, connection }
    }

    pub async fn search_messages(&self, q: &str, max: u32) -> Result<Vec<MessageRef>, GmailError> {
        let mut req = ProxyRequest::get("/gmail/v1/users/me/messages");
        req.query.push(("q".into(), q.into()));
        req.query.push(("maxResults".into(), max.to_string()));
        let resp = self.transport.proxy(req, &self.connection).await?;
        ok_2xx(resp.status)?;
        parse_search(&resp.body)
    }

    pub async fn get_message(&self, id: &str) -> Result<Message, GmailError> {
        let mut req = ProxyRequest::get(format!("/gmail/v1/users/me/messages/{id}"));
        req.query.push(("format".into(), "metadata".into()));
        req.query.push(("metadataHeaders".into(), "Subject".into()));
        req.query.push(("metadataHeaders".into(), "From".into()));
        let resp = self.transport.proxy(req, &self.connection).await?;
        ok_2xx(resp.status)?;
        parse_message(&resp.body)
    }

    pub async fn send_message(&self, to: &str, subject: &str, body_text: &str) -> Result<String, GmailError> {
        let raw = format!("To: {to}\r\nSubject: {subject}\r\n\r\n{body_text}");
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes());
        let body = serde_json::to_vec(&serde_json::json!({ "raw": encoded }))
            .map_err(|e| GmailError::Decode(e.to_string()))?;
        let req = ProxyRequest::post("/gmail/v1/users/me/messages/send")
            .with_header("Content-Type", "application/json")
            .with_body(body);
        let resp = self.transport.proxy(req, &self.connection).await?;
        ok_2xx(resp.status)?;
        let sent: SentRef = serde_json::from_slice(&resp.body).map_err(|e| GmailError::Decode(e.to_string()))?;
        Ok(sent.id)
    }
}

fn ok_2xx(status: u16) -> Result<(), GmailError> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(GmailError::Transport(NangoError::Proxy { status }))
    }
}

pub fn parse_search(body: &[u8]) -> Result<Vec<MessageRef>, GmailError> {
    let r: MessageList = serde_json::from_slice(body).map_err(|e| GmailError::Decode(e.to_string()))?;
    Ok(r.messages.unwrap_or_default())
}

pub fn parse_message(body: &[u8]) -> Result<Message, GmailError> {
    let m: RawMessage = serde_json::from_slice(body).map_err(|e| GmailError::Decode(e.to_string()))?;
    let subject = find_header(&m, "Subject");
    let from = find_header(&m, "From");
    // Gmail sends internalDate as a string of epoch milliseconds.
    let internal_date_ms = m.internal_date.as_deref().and_then(|s| s.parse::<i64>().ok());
    let RawMessage { id, thread_id, snippet, .. } = m;
    Ok(Message { id, thread_id, snippet, subject, from, internal_date_ms })
}

fn find_header(m: &RawMessage, name: &str) -> Option<String> {
    m.payload
        .as_ref()
        .and_then(|p| p.headers.as_ref())
        .and_then(|hs| hs.iter().find(|h| h.name.eq_ignore_ascii_case(name)))
        .map(|h| h.value.clone())
}

#[derive(serde::Deserialize)]
struct MessageList {
    messages: Option<Vec<MessageRef>>,
}
#[derive(serde::Deserialize)]
struct RawMessage {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: String,
    snippet: Option<String>,
    #[serde(rename = "internalDate")]
    internal_date: Option<String>,
    payload: Option<Payload>,
}
#[derive(serde::Deserialize)]
struct Payload {
    headers: Option<Vec<HeaderKV>>,
}
#[derive(serde::Deserialize)]
struct HeaderKV {
    name: String,
    value: String,
}
#[derive(serde::Deserialize)]
struct SentRef {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_key_is_googles() {
        assert_eq!(PROVIDER_CONFIG_KEY, "google-mail");
    }

    #[test]
    fn parse_search_ids() {
        let body = br#"{"messages":[{"id":"m1","threadId":"t1"},{"id":"m2","threadId":"t2"}],"resultSizeEstimate":2}"#;
        let v = parse_search(body).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].id, "m1");
        assert_eq!(v[0].thread_id, "t1");
    }

    #[test]
    fn parse_message_extracts_headers_and_date() {
        let body = br#"{"id":"m1","threadId":"t1","snippet":"hi","internalDate":"1559347200000","payload":{"headers":[{"name":"Subject","value":"Hello"},{"name":"From","value":"a@b.com"}]}}"#;
        let m = parse_message(body).unwrap();
        assert_eq!(m.subject.as_deref(), Some("Hello"));
        assert_eq!(m.from.as_deref(), Some("a@b.com"));
        assert_eq!(m.snippet.as_deref(), Some("hi"));
        assert_eq!(m.internal_date_ms, Some(1_559_347_200_000));
    }
}
