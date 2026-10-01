//! nango — the Nango seam.
//!
//! A host-agnostic transport that lets a Pacific System reach a long-tail SaaS
//! through a Nango connector, without Pacific building or holding the provider's
//! OAuth. Nango runs as a server — Nango Cloud or Pacific-self-hosted — selected
//! by [`NangoEnvironment`], a config value, so the choice is a later flip.
//!
//! The privileged Nango secret key lives in that broker. A device holds only a
//! [`NangoConnection`] (which member, which integration) and obtains short-lived
//! [`NangoConnectSession`]s from the broker to run Nango Connect's OAuth UI. The
//! provider token stays in the broker and is injected at the proxy edge.
//!
//! [`NangoHttpTransport`] is the live reqwest-backed implementation; the secret
//! key is supplied by the caller (loaded from the environment), never hard-coded.

/// The end user a Nango connection is authorized for — becomes Nango's
/// `end_user.id` when a connect session is minted. Kept local to the connector
/// layer so these crates carry no dependency on the core engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndUserId(pub String);

impl EndUserId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Which Nango broker a connector talks to. Point `base_url` at a self-hosted
/// instance to move the broker in-house; nothing else in a connector changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NangoEnvironment {
    pub base_url: String,
}

impl NangoEnvironment {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into() }
    }
    /// Nango Cloud.
    pub fn cloud() -> Self {
        Self::new("https://api.nango.dev")
    }
}

/// A member's authorized link to a provider, held by Nango. The device stores
/// these identifiers; the provider's OAuth token lives in the broker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NangoConnection {
    pub connection_id: String,
    pub provider_config_key: String,
}

impl NangoConnection {
    pub fn new(connection_id: impl Into<String>, provider_config_key: impl Into<String>) -> Self {
        Self {
            connection_id: connection_id.into(),
            provider_config_key: provider_config_key.into(),
        }
    }
}

/// A short-lived session the broker mints (using its secret key) so the device
/// can run Nango Connect's OAuth flow while the secret key stays in the broker.
#[derive(Clone, Debug)]
pub struct NangoConnectSession {
    pub token: String,
    pub expires_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// A provider-relative call to forward through the Nango proxy.
#[derive(Clone, Debug, PartialEq)]
pub struct ProxyRequest {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

impl ProxyRequest {
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: None,
        }
    }
    pub fn get(path: impl Into<String>) -> Self {
        Self::new(Method::Get, path)
    }
    pub fn post(path: impl Into<String>) -> Self {
        Self::new(Method::Post, path)
    }
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NangoError {
    NotConnected { provider_config_key: String },
    Proxy { status: u16 },
    Decoding(String),
}

/// Everything a Nango-backed System calls. Implemented against the broker: the
/// secret key lives there, the device holds only a [`NangoConnection`]. Used
/// generically by connectors (`impl NangoTransport`), so no boxing is forced.
#[allow(async_fn_in_trait)]
pub trait NangoTransport: Send + Sync {
    /// Begin authorizing a provider for `caller`. Returns a short-lived session
    /// the device hands to Nango Connect's OAuth UI; the broker reports back the
    /// resulting connection.
    async fn start_connect(
        &self,
        provider_config_key: &str,
        caller: &EndUserId,
    ) -> Result<NangoConnectSession, NangoError>;

    /// Run a proxied provider call on a bound connection. Nango injects the
    /// provider's auth and forwards it; only the result returns.
    async fn proxy(
        &self,
        request: ProxyRequest,
        connection: &NangoConnection,
    ) -> Result<ProxyResponse, NangoError>;
}

// MARK: - reqwest-backed transport (the broker boundary, wired)

/// A live [`NangoTransport`] over HTTP to a Nango broker. Holds the broker base
/// URL and the secret key; the provider token is injected broker-side, so this
/// client only ever presents the connection.
#[derive(Clone)]
pub struct NangoHttpTransport {
    base_url: String,
    secret_key: String,
    client: reqwest::Client,
}

impl NangoHttpTransport {
    pub fn new(base_url: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            secret_key: secret_key.into(),
            client: reqwest::Client::new(),
        }
    }

    /// A Nango *management* GET (e.g. `/connections`) — authenticated with the
    /// secret key, not proxied to a provider. Returns the raw response for the
    /// caller to parse.
    pub async fn management_get(&self, path: &str) -> Result<ProxyResponse, NangoError> {
        let resp = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .header("Authorization", format!("Bearer {}", self.secret_key))
            .send()
            .await
            .map_err(|e| NangoError::Decoding(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| NangoError::Decoding(e.to_string()))?
            .to_vec();
        Ok(ProxyResponse { status, body })
    }
}

fn to_reqwest_method(m: Method) -> reqwest::Method {
    match m {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Put => reqwest::Method::PUT,
        Method::Patch => reqwest::Method::PATCH,
        Method::Delete => reqwest::Method::DELETE,
    }
}

impl NangoTransport for NangoHttpTransport {
    async fn start_connect(
        &self,
        provider_config_key: &str,
        caller: &EndUserId,
    ) -> Result<NangoConnectSession, NangoError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "end_user": { "id": caller.0 },
            "allowed_integrations": [provider_config_key],
        }))
        .map_err(|e| NangoError::Decoding(e.to_string()))?;
        let resp = self
            .client
            .post(format!("{}/connect/sessions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.secret_key))
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| NangoError::Decoding(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(NangoError::Proxy { status });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| NangoError::Decoding(e.to_string()))?;
        let v: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| NangoError::Decoding(e.to_string()))?;
        let token = v["data"]["token"]
            .as_str()
            .ok_or_else(|| NangoError::Decoding("missing data.token".into()))?
            .to_string();
        let expires_at = v["data"]["expires_at"].as_str().unwrap_or_default().to_string();
        Ok(NangoConnectSession { token, expires_at })
    }

    async fn proxy(
        &self,
        request: ProxyRequest,
        connection: &NangoConnection,
    ) -> Result<ProxyResponse, NangoError> {
        let url = format!("{}/proxy{}", self.base_url, request.path);
        let mut rb = self
            .client
            .request(to_reqwest_method(request.method), url)
            .header("Authorization", format!("Bearer {}", self.secret_key))
            .header("Connection-Id", &connection.connection_id)
            .header("Provider-Config-Key", &connection.provider_config_key);
        if !request.query.is_empty() {
            rb = rb.query(&request.query); // reqwest percent-encodes
        }
        for (k, v) in &request.headers {
            rb = rb.header(k, v);
        }
        if let Some(body) = request.body {
            rb = rb.body(body);
        }
        let resp = rb.send().await.map_err(|e| NangoError::Decoding(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| NangoError::Decoding(e.to_string()))?
            .to_vec();
        Ok(ProxyResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_base_url() {
        assert_eq!(NangoEnvironment::cloud().base_url, "https://api.nango.dev");
    }

    #[test]
    fn self_host_flip_changes_only_the_base_url() {
        // Moving the broker in-house is a one-value change: point base_url at the
        // self-hosted instance and every connector's proxy calls follow it.
        let env = NangoEnvironment::new("https://nango.example.com");
        assert_eq!(env.base_url, "https://nango.example.com");
    }
}
