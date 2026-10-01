//! nango-rag — a Nango connector as on-demand RAG context for the Core Loop Agent (the CLA).
//!
//! The CLA calls a connector *live* with a query (a `SystemTool` on the device → the Arc's
//! `/systems/<provider>/search` endpoint → here). This crate does the two backend-side steps:
//!
//! 1. [`fetch_gmail`] (async) searches the provider through the Nango proxy and pulls each hit
//!    into a typed item. The Nango secret lives on the Arc; the device only supplies its
//!    [`nango::NangoConnection`].
//! 2. [`render_gmail_hits`] (pure) turns the hits into the citable text the model reads as
//!    `tool_result` — one line per hit, each carrying its [`ExternalRef`] so the CLA can cite
//!    the source (and a follow-up can open it) without anything being copied into the graph.
//!
//! Nothing is synced or persisted: this is retrieval, evaluated per query.

mod external;

pub use external::ExternalRef;

use gmail::{GmailBridge, GmailError, Message};
use nango::NangoTransport;

/// The default number of Gmail hits to pull for one query — enough to ground an answer,
/// few enough to keep the tool_result and the follow-up `get_message` calls cheap.
pub const DEFAULT_MAX_HITS: u32 = 8;

/// Search a Gmail connection and pull each hit into a full [`Message`] (Gmail search returns
/// ids only, so each is fetched for its headers + snippet + date). The "search" half; runs on
/// the Arc with the connection the device holds.
pub async fn fetch_gmail<T: NangoTransport>(
    bridge: &GmailBridge<T>,
    query: &str,
    max: u32,
) -> Result<Vec<Message>, GmailError> {
    let refs = bridge.search_messages(query, max).await?;
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        out.push(bridge.get_message(&r.id).await?);
    }
    Ok(out)
}

/// The citation back to one source record, built from a Gmail message.
fn gmail_ref(m: &Message) -> ExternalRef {
    let mut r = ExternalRef::new("gmail", &m.id)
        // `#all/<id>` opens the message in Gmail's web UI.
        .with_url(format!("https://mail.google.com/mail/#all/{}", m.id));
    if let Some(subject) = m.subject.as_deref().filter(|s| !s.is_empty()) {
        r = r.with_title(subject);
    }
    r
}

/// Render one Gmail hit as a single citable line for the CLA: a `[gmail · Subject]` tag, the
/// sender and snippet, and the source id so a follow-up can re-read it.
fn render_gmail_hit(m: &Message) -> String {
    let subject = m.subject.as_deref().filter(|s| !s.is_empty()).unwrap_or("(no subject)");
    let mut parts = Vec::new();
    if let Some(from) = m.from.as_deref().filter(|s| !s.is_empty()) {
        parts.push(format!("from {from}"));
    }
    if let Some(snippet) = m.snippet.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(snippet.to_string());
    }
    let body = parts.join(" — ");
    format!("[gmail · {subject}] {body} (id: {})", m.id)
}

/// Render a set of Gmail hits into the `tool_result` text the CLA reads, plus the parallel
/// [`ExternalRef`] citations. Honest-empty: an empty result says so rather than inventing.
/// The text is what the model sees; the refs let the host render tappable sources.
pub fn render_gmail_hits(query: &str, hits: &[Message]) -> (String, Vec<ExternalRef>) {
    if hits.is_empty() {
        return (format!("No Gmail messages match \"{query}\"."), Vec::new());
    }
    let text = hits.iter().map(render_gmail_hit).collect::<Vec<_>>().join("\n");
    let refs = hits.iter().map(gmail_ref).collect();
    (text, refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, subject: Option<&str>, from: Option<&str>, snippet: Option<&str>) -> Message {
        Message {
            id: id.into(),
            thread_id: "t".into(),
            snippet: snippet.map(Into::into),
            subject: subject.map(Into::into),
            from: from.map(Into::into),
            internal_date_ms: None,
        }
    }

    #[test]
    fn renders_citable_line_per_hit() {
        let hits = vec![
            msg("m1", Some("Launch plan"), Some("ada@example.com"), Some("ship Friday")),
            msg("m2", Some("Re: Launch"), Some("bo@example.com"), Some("sounds good")),
        ];
        let (text, refs) = render_gmail_hits("launch", &hits);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "[gmail · Launch plan] from ada@example.com — ship Friday (id: m1)");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].provider, "gmail");
        assert_eq!(refs[0].id, "m1");
        assert_eq!(refs[0].title.as_deref(), Some("Launch plan"));
        assert!(refs[0].url.as_deref().unwrap().ends_with("/m1"));
    }

    #[test]
    fn honest_empty() {
        let (text, refs) = render_gmail_hits("nope", &[]);
        assert_eq!(text, "No Gmail messages match \"nope\".");
        assert!(refs.is_empty());
    }

    #[test]
    fn tolerates_missing_fields() {
        let hits = vec![msg("m3", None, None, None)];
        let (text, _) = render_gmail_hits("x", &hits);
        assert_eq!(text, "[gmail · (no subject)]  (id: m3)");
    }
}
