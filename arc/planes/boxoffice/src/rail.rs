//! The payment-rail seam — ONE trait between the box office and whatever moves the money.
//!
//! THE INVARIANT (provider-agnostic, and it lives here so every adapter inherits it):
//! "Pacific nets exactly the 10¢-equivalent per ticket; processing costs ride the
//! organizer's side."
//!
//! Concretely: an adapter charges the buyer on the ORGANIZER's account and collects
//! `fee_minor × qty` as the platform's application fee — the organizer's payout bears the
//! rail's processing cost, Pacific's cut is exactly the fee and never a penny of spread.
//! Stripe (`stripe.rs`, direct charges) is the first adapter; Toss/USDC are backlog
//! adapters behind this same trait (Ralph: "Stripe may not be the best option").

use crate::store::RegisteredListing;
use axum::http::HeaderMap;

#[derive(Debug, thiserror::Error)]
pub enum RailError {
    /// The rail could not be reached (DNS, TLS, timeout) — retryable, nobody was charged.
    #[error("rail transport: {0}")]
    Transport(String),
    /// The rail answered and said no.
    #[error("rail refused ({status}): {message}")]
    Rail { status: u16, message: String },
    /// A webhook that does not prove it came from the rail. Never processed.
    #[error("webhook signature rejected: {0}")]
    BadSignature(String),
    /// A payload we cannot make sense of — refused loudly, never guessed at.
    #[error("malformed rail payload: {0}")]
    Malformed(String),
}

/// A checkout session the buyer is sent to. `pi` is the payment identity every later step
/// (webhook, verify, fulfill, refund) is keyed by.
#[derive(Debug, Clone)]
pub struct CheckoutSession {
    pub url: String,
    pub pi: String,
    pub session_id: String,
    pub expires_at_ms: i64,
}

/// What a verified webhook said. Adapters translate their event vocabulary into this;
/// anything the box office does not act on arrives as `Ignored` (received, logged, done).
#[derive(Debug, Clone)]
pub enum RailEvent {
    CheckoutCompleted {
        pi: String,
        listing: String,
        buyer_pk: String,
        qty: i64,
        amount_minor: i64,
        currency: String,
        /// The PER-TICKET fee snapshot stamped at checkout creation (minor units).
        fee_minor: i64,
    },
    Ignored {
        kind: String,
    },
}

/// The rail's answer to "did this payment actually happen" — the 2B agent's re-verify
/// sweep asks this each sync for pending sessions until event start.
#[allow(dead_code)] // 2B seam: no caller until the embedded fulfillment node lands
#[derive(Debug, Clone)]
pub enum PaymentState {
    Paid { amount_minor: i64, currency: String },
    Unpaid { status: String },
}

/// An onboarding link for an organizer who does not yet have a rail account (D29 flow:
/// onboard first, register the priced listing with the resulting `acct` after).
#[derive(Debug, Clone)]
pub struct OnboardLink {
    pub url: String,
    pub acct: String,
}

/// The seam. One implementation per rail; the box office holds a `dyn Rail` and never
/// learns which one. `acct` on `retrieve`/`refund` is the organizer's connected account:
/// under direct charges the payment object LIVES on that account, so the adapter cannot
/// find it without being told whose books to open.
#[async_trait::async_trait]
pub trait Rail: Send + Sync {
    /// Open a checkout session for `qty` tickets of `listing`, collecting
    /// `fee_minor × qty` as the platform fee. Direct charge on the organizer's account —
    /// the invariant above is enforced HERE, at money-creation time.
    async fn create_checkout(
        &self,
        listing: &RegisteredListing,
        qty: u32,
        buyer_pk: &str,
        fee_minor: i64,
    ) -> Result<CheckoutSession, RailError>;

    /// Verify a webhook actually came from the rail and translate it. Pure computation —
    /// a signature check must never need the network.
    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8]) -> Result<RailEvent, RailError>;

    #[allow(dead_code)] // 2B seam: the agent's re-verify sweep (4A expiry) calls this
    async fn retrieve(&self, pi: &str, acct: Option<&str>) -> Result<PaymentState, RailError>;

    async fn refund(&self, pi: &str, acct: Option<&str>) -> Result<(), RailError>;

    async fn onboard(&self, country: &str) -> Result<OnboardLink, RailError>;

    /// Can this account take charges yet? (`charges_enabled` — the D29 gate the composer
    /// blocks priced listings on.)
    async fn account_status(&self, acct: &str) -> Result<bool, RailError>;
}

// -- mock rail (TEST-ONLY) ----------------------------------------------------

/// TEST-ONLY rail: no money exists behind it. Used by the route tests and, for the
/// in-repo harness, by `--mock-rail` — which `main` refuses to honour unless
/// `ARC_BOXOFFICE_MOCK_RAIL=1` is EXPLICITLY set. It is never a silent fallback: a
/// missing Stripe key yields NO rail (503 on checkout), not this.
#[derive(Default)]
pub struct MockRail {
    counter: std::sync::atomic::AtomicU64,
}

impl MockRail {
    /// The header a mock webhook must carry to pass "signature" verification — so even
    /// the mock path exercises the refuse-unsigned-webhooks branch.
    pub const SIG_HEADER: &'static str = "mock-signature";
    pub const SIG_VALUE: &'static str = "mock-valid";
}

#[async_trait::async_trait]
impl Rail for MockRail {
    async fn create_checkout(
        &self,
        _listing: &RegisteredListing,
        _qty: u32,
        _buyer_pk: &str,
        _fee_minor: i64,
    ) -> Result<CheckoutSession, RailError> {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Ok(CheckoutSession {
            url: format!("mock://checkout/pi_mock_{n}"),
            pi: format!("pi_mock_{n}"),
            session_id: format!("cs_mock_{n}"),
            expires_at_ms: now + 35 * 60 * 1000,
        })
    }

    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8]) -> Result<RailEvent, RailError> {
        let sig = headers.get(Self::SIG_HEADER).and_then(|v| v.to_str().ok());
        if sig != Some(Self::SIG_VALUE) {
            return Err(RailError::BadSignature(
                "mock signature missing or wrong".into(),
            ));
        }
        let v: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| RailError::Malformed(format!("mock webhook body: {e}")))?;
        let kind = v["type"].as_str().unwrap_or("").to_string();
        if kind != "checkout.session.completed" {
            return Ok(RailEvent::Ignored { kind });
        }
        let s = |k: &str| -> Result<String, RailError> {
            v[k].as_str()
                .map(str::to_string)
                .ok_or_else(|| RailError::Malformed(format!("mock webhook missing {k}")))
        };
        Ok(RailEvent::CheckoutCompleted {
            pi: s("pi")?,
            listing: s("listing")?,
            buyer_pk: s("buyer_pk")?,
            qty: v["qty"].as_i64().unwrap_or(1),
            amount_minor: v["amount_minor"].as_i64().unwrap_or(0),
            currency: v["currency"].as_str().unwrap_or("usd").to_string(),
            fee_minor: v["fee_minor"].as_i64().unwrap_or(0),
        })
    }

    async fn retrieve(&self, _pi: &str, _acct: Option<&str>) -> Result<PaymentState, RailError> {
        Ok(PaymentState::Unpaid {
            status: "mock_unverified".into(),
        })
    }

    async fn refund(&self, _pi: &str, _acct: Option<&str>) -> Result<(), RailError> {
        Ok(())
    }

    async fn onboard(&self, country: &str) -> Result<OnboardLink, RailError> {
        Ok(OnboardLink {
            url: format!("mock://onboard/{country}"),
            acct: "acct_mock".into(),
        })
    }

    async fn account_status(&self, _acct: &str) -> Result<bool, RailError> {
        Ok(true)
    }
}
