//! External references — a citation back to the outside record a RAG hit came from
//! (a Gmail message, a Linear issue, a Slack thread).
//!
//! The CLA reads the rendered hit text; the parallel `ExternalRef` lets the host show a
//! tappable source and lets a follow-up tool re-open the exact record. Nothing is stored —
//! the ref is minted per query alongside the hit.

use serde::{Deserialize, Serialize};

/// A pointer to an outside record a RAG hit was drawn from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRef {
    /// The source system, e.g. `"gmail"`, `"linear"`, `"slack"`, `"outlook"`.
    pub provider: String,
    /// The provider-native identifier (a Gmail message id, a Linear issue id).
    pub id: String,
    /// A stable link back to the record, when the provider offers one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// A short human label (the email subject, the issue title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl ExternalRef {
    pub fn new(provider: impl Into<String>, id: impl Into<String>) -> Self {
        Self { provider: provider.into(), id: id.into(), url: None, title: None }
    }
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_title_omitted_when_absent() {
        let v = serde_json::to_value(ExternalRef::new("slack", "t1")).unwrap();
        assert!(v.get("url").is_none());
        assert!(v.get("title").is_none());
    }

    #[test]
    fn builder_sets_fields() {
        let r = ExternalRef::new("gmail", "m1").with_url("https://mail/x").with_title("Hi");
        assert_eq!(r.provider, "gmail");
        assert_eq!(r.url.as_deref(), Some("https://mail/x"));
        assert_eq!(r.title.as_deref(), Some("Hi"));
    }
}
