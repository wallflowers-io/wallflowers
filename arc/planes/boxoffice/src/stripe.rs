//! The Stripe adapter — thin, hand-rolled reqwest (decision 7A: NOT the async-stripe
//! crate; five calls do not justify a 100-module dependency), form-encoded bodies,
//! pinned `Stripe-Version`.
//!
//! DIRECT CHARGES (decisions OV2/D20/D25): every checkout session is created ON the
//! organizer's connected account (`Stripe-Account` header), with
//! `payment_intent_data[application_fee_amount] = fee_minor × qty` — so the organizer's
//! payout bears Stripe's ~2.9%+30¢ processing and Pacific nets exactly the
//! 10¢-equivalent per ticket. Destination charges would invert that and lose money on
//! every ticket; they are not an option here.
//!
//! Webhook verification is Stripe's documented scheme: `Stripe-Signature: t=...,v1=...`,
//! HMAC-SHA256 over `{t}.{body}` with the endpoint secret, 300s timestamp tolerance,
//! constant-time compare. Verified BEFORE anything is parsed as an event.

use crate::rail::{CheckoutSession, OnboardLink, PaymentState, Rail, RailError, RailEvent};
use crate::store::RegisteredListing;
use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;

const API: &str = "https://api.stripe.com";
/// Pinned so a Stripe API evolution is a deliberate diff here, never a surprise in prod.
const STRIPE_VERSION: &str = "2024-06-20";
/// Stripe's documented default tolerance for webhook timestamps.
const TOLERANCE_S: i64 = 300;
/// Checkout sessions expire after 35 minutes (Stripe's floor is 30) — short enough that
/// an abandoned session frees its capacity hold quickly. Mirrored by
/// `store::OPEN_SESSION_TTL_MS`, which is how the capacity gate stops counting it.
const SESSION_TTL_S: i64 = 35 * 60;

pub struct StripeRail {
    http: reqwest::Client,
    secret: String,
    webhook_secret: String,
    /// The plain hosted "return to Pacific" page (no deep-link machinery, D21). Also the
    /// stopgap refresh/return target for Express onboarding until Slice 3 wires D29's
    /// resumable flow.
    success_url: String,
}

impl StripeRail {
    pub fn new(secret: String, webhook_secret: String, success_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            secret,
            webhook_secret,
            success_url,
        }
    }

    /// POST a form to Stripe, optionally on behalf of a connected account, and parse the
    /// JSON answer. Non-2xx becomes `RailError::Rail` carrying Stripe's own message.
    async fn post_form(
        &self,
        path: &str,
        acct: Option<&str>,
        form: String,
    ) -> Result<serde_json::Value, RailError> {
        let mut rb = self
            .http
            .post(format!("{API}{path}"))
            .header("Authorization", format!("Bearer {}", self.secret))
            .header("Stripe-Version", STRIPE_VERSION)
            .header("Content-Type", "application/x-www-form-urlencoded");
        if let Some(a) = acct {
            rb = rb.header("Stripe-Account", a);
        }
        let resp = rb
            .body(form)
            .send()
            .await
            .map_err(|e| RailError::Transport(e.to_string()))?;
        let status = resp.status();
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| RailError::Malformed(e.to_string()))?;
        if !status.is_success() {
            return Err(RailError::Rail {
                status: status.as_u16(),
                message: v["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown stripe error")
                    .into(),
            });
        }
        Ok(v)
    }

    async fn get(&self, path: &str, acct: Option<&str>) -> Result<serde_json::Value, RailError> {
        let mut rb = self
            .http
            .get(format!("{API}{path}"))
            .header("Authorization", format!("Bearer {}", self.secret))
            .header("Stripe-Version", STRIPE_VERSION);
        if let Some(a) = acct {
            rb = rb.header("Stripe-Account", a);
        }
        let resp = rb
            .send()
            .await
            .map_err(|e| RailError::Transport(e.to_string()))?;
        let status = resp.status();
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| RailError::Malformed(e.to_string()))?;
        if !status.is_success() {
            return Err(RailError::Rail {
                status: status.as_u16(),
                message: v["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown stripe error")
                    .into(),
            });
        }
        Ok(v)
    }
}

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Percent-encode a form VALUE (RFC 3986 unreserved passes through). Keys are our own
/// static strings — brackets included — and go verbatim, which Stripe accepts.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The exact checkout-session form body — a pure function so the golden test can pin it
/// byte-for-byte. `metadata[*]` is what the webhook hands back; `metadata[fee]` is the
/// PER-TICKET fee snapshot (minor units) recorded per sale (design §4).
fn checkout_form(
    listing: &RegisteredListing,
    qty: u32,
    buyer_pk: &str,
    fee_minor: i64,
    success_url: &str,
    expires_at_s: i64,
) -> String {
    let name = format!(
        "Ticket {}",
        &listing.listing[..listing.listing.len().min(8)]
    );
    let pairs: [(&str, String); 12] = [
        ("mode", "payment".into()),
        (
            "line_items[0][price_data][currency]",
            listing.currency.clone(),
        ),
        (
            "line_items[0][price_data][unit_amount]",
            listing.unit_cents.to_string(),
        ),
        ("line_items[0][price_data][product_data][name]", name),
        ("line_items[0][quantity]", qty.to_string()),
        // The invariant, at money-creation time: Pacific's cut is exactly fee × qty.
        (
            "payment_intent_data[application_fee_amount]",
            (fee_minor * qty as i64).to_string(),
        ),
        ("success_url", success_url.into()),
        ("expires_at", expires_at_s.to_string()),
        ("metadata[listing]", listing.listing.clone()),
        ("metadata[buyer_pk]", buyer_pk.into()),
        ("metadata[qty]", qty.to_string()),
        ("metadata[fee]", fee_minor.to_string()),
    ];
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Verify a `Stripe-Signature` header against the raw body: HMAC-SHA256 over `{t}.{body}`,
/// timestamp within `tolerance_s` of `now_s`, constant-time compare (`Mac::verify_slice`).
/// Multiple `v1=` entries are each tried (Stripe sends several during secret rotation).
fn verify_sig(
    secret: &str,
    sig_header: &str,
    body: &[u8],
    now_s: i64,
    tolerance_s: i64,
) -> Result<(), RailError> {
    let mut t: Option<i64> = None;
    let mut v1s: Vec<&str> = Vec::new();
    for part in sig_header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = v.parse().ok(),
            Some(("v1", v)) => v1s.push(v),
            _ => {}
        }
    }
    let t = t.ok_or_else(|| RailError::BadSignature("no t= in Stripe-Signature".into()))?;
    if v1s.is_empty() {
        return Err(RailError::BadSignature("no v1= in Stripe-Signature".into()));
    }
    if (now_s - t).abs() > tolerance_s {
        return Err(RailError::BadSignature(format!(
            "timestamp {t} outside ±{tolerance_s}s tolerance"
        )));
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| RailError::BadSignature("unusable webhook secret".into()))?;
    mac.update(t.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    for v1 in v1s {
        if let Ok(sig) = hex::decode(v1) {
            if mac.clone().verify_slice(&sig).is_ok() {
                return Ok(());
            }
        }
    }
    Err(RailError::BadSignature("no v1 signature matched".into()))
}

/// Translate a VERIFIED event body. Only `checkout.session.completed` is acted on; every
/// other type is `Ignored` (received and done — Stripe stops retrying).
fn parse_event(body: &[u8]) -> Result<RailEvent, RailError> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| RailError::Malformed(e.to_string()))?;
    let kind = v["type"].as_str().unwrap_or("").to_string();
    if kind != "checkout.session.completed" {
        return Ok(RailEvent::Ignored { kind });
    }
    let o = &v["data"]["object"];
    let meta = &o["metadata"];
    let s = |val: &serde_json::Value, what: &str| -> Result<String, RailError> {
        val.as_str().map(str::to_string).ok_or_else(|| {
            RailError::Malformed(format!("checkout.session.completed missing {what}"))
        })
    };
    // Metadata values arrive as strings (Stripe metadata is string→string).
    let int = |val: &serde_json::Value, what: &str| -> Result<i64, RailError> {
        val.as_str()
            .and_then(|x| x.parse().ok())
            .ok_or_else(|| RailError::Malformed(format!("checkout.session.completed bad {what}")))
    };
    Ok(RailEvent::CheckoutCompleted {
        pi: s(&o["payment_intent"], "payment_intent")?,
        listing: s(&meta["listing"], "metadata.listing")?,
        buyer_pk: s(&meta["buyer_pk"], "metadata.buyer_pk")?,
        qty: int(&meta["qty"], "metadata.qty")?,
        amount_minor: o["amount_total"].as_i64().unwrap_or(0),
        currency: o["currency"].as_str().unwrap_or("").to_string(),
        fee_minor: int(&meta["fee"], "metadata.fee")?,
    })
}

#[async_trait::async_trait]
impl Rail for StripeRail {
    async fn create_checkout(
        &self,
        listing: &RegisteredListing,
        qty: u32,
        buyer_pk: &str,
        fee_minor: i64,
    ) -> Result<CheckoutSession, RailError> {
        // Direct charge: the session is created ON the connected account, so a listing
        // with no acct cannot be charged for — refused here even if the route missed it.
        let acct = listing.acct.clone().ok_or_else(|| {
            RailError::Malformed("priced listing has no connected account (acct)".into())
        })?;
        let expires_at_s = now_s() + SESSION_TTL_S;
        let form = checkout_form(
            listing,
            qty,
            buyer_pk,
            fee_minor,
            &self.success_url,
            expires_at_s,
        );
        let v = self
            .post_form("/v1/checkout/sessions", Some(&acct), form)
            .await?;
        let session_id = v["id"].as_str().unwrap_or("").to_string();
        // Recent API versions may defer PaymentIntent creation to payment submission; the
        // session id then keys the row until the webhook arrives carrying the real pi
        // (the webhook handler inserts by pi when it finds no row).
        let pi = v["payment_intent"]
            .as_str()
            .unwrap_or(&session_id)
            .to_string();
        let url = v["url"]
            .as_str()
            .ok_or_else(|| RailError::Malformed("checkout session has no url".into()))?
            .to_string();
        let expires_at_ms = v["expires_at"].as_i64().unwrap_or(expires_at_s) * 1000;
        Ok(CheckoutSession {
            url,
            pi,
            session_id,
            expires_at_ms,
        })
    }

    fn verify_webhook(&self, headers: &HeaderMap, body: &[u8]) -> Result<RailEvent, RailError> {
        let sig = headers
            .get("stripe-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| RailError::BadSignature("no Stripe-Signature header".into()))?;
        verify_sig(&self.webhook_secret, sig, body, now_s(), TOLERANCE_S)?;
        parse_event(body)
    }

    async fn retrieve(&self, pi: &str, acct: Option<&str>) -> Result<PaymentState, RailError> {
        let v = self.get(&format!("/v1/payment_intents/{pi}"), acct).await?;
        match v["status"].as_str().unwrap_or("") {
            "succeeded" => Ok(PaymentState::Paid {
                amount_minor: v["amount_received"].as_i64().unwrap_or(0),
                currency: v["currency"].as_str().unwrap_or("").to_string(),
            }),
            other => Ok(PaymentState::Unpaid {
                status: other.to_string(),
            }),
        }
    }

    async fn refund(&self, pi: &str, acct: Option<&str>) -> Result<(), RailError> {
        // Direct charge ⇒ the refund is created on the connected account. The application
        // fee is NOT auto-refunded — the fee policy pass is a deferred follow-up
        // (product-backlog Epic 12), not an accidental default flipped here.
        let form = format!("payment_intent={}", enc(pi));
        self.post_form("/v1/refunds", acct, form).await.map(|_| ())
    }

    async fn onboard(&self, country: &str) -> Result<OnboardLink, RailError> {
        let acct_v = self
            .post_form(
                "/v1/accounts",
                None,
                format!("type=express&country={}", enc(country)),
            )
            .await?;
        let acct = acct_v["id"]
            .as_str()
            .ok_or_else(|| RailError::Malformed("account create returned no id".into()))?
            .to_string();
        let form = format!(
            "account={}&type=account_onboarding&refresh_url={}&return_url={}",
            enc(&acct),
            enc(&self.success_url),
            enc(&self.success_url)
        );
        let link = self.post_form("/v1/account_links", None, form).await?;
        let url = link["url"]
            .as_str()
            .ok_or_else(|| RailError::Malformed("account link returned no url".into()))?
            .to_string();
        Ok(OnboardLink { url, acct })
    }

    async fn account_status(&self, acct: &str) -> Result<bool, RailError> {
        let v = self.get(&format!("/v1/accounts/{acct}"), None).await?;
        Ok(v["charges_enabled"].as_bool().unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "whsec_test_secret";

    fn fixture_body() -> Vec<u8> {
        serde_json::json!({
            "id": "evt_1",
            "type": "checkout.session.completed",
            "data": { "object": {
                "id": "cs_test_1",
                "payment_intent": "pi_golden_1",
                "amount_total": 2400,
                "currency": "usd",
                "metadata": {
                    "listing": "aabbccddeeff0011",
                    "buyer_pk": "deadbeef",
                    "qty": "2",
                    "fee": "10"
                }
            }}
        })
        .to_string()
        .into_bytes()
    }

    fn sign(secret: &str, t: i64, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// GOLDEN: a correctly signed webhook verifies and every field the box office acts on
    /// is extracted — including the fee snapshot the invariant is audited against.
    #[test]
    fn valid_signature_verifies_and_extracts() {
        let body = fixture_body();
        let now = now_s();
        let header = sign(SECRET, now, &body);
        verify_sig(SECRET, &header, &body, now, TOLERANCE_S).expect("valid sig must pass");
        match parse_event(&body).unwrap() {
            RailEvent::CheckoutCompleted {
                pi,
                listing,
                buyer_pk,
                qty,
                amount_minor,
                currency,
                fee_minor,
            } => {
                assert_eq!(pi, "pi_golden_1");
                assert_eq!(listing, "aabbccddeeff0011");
                assert_eq!(buyer_pk, "deadbeef");
                assert_eq!(qty, 2);
                assert_eq!(amount_minor, 2400);
                assert_eq!(currency, "usd");
                assert_eq!(fee_minor, 10);
            }
            other => panic!("expected CheckoutCompleted, got {other:?}"),
        }
    }

    /// A signature minted with the WRONG secret is refused.
    #[test]
    fn wrong_secret_fails() {
        let body = fixture_body();
        let now = now_s();
        let header = sign("whsec_wrong", now, &body);
        assert!(matches!(
            verify_sig(SECRET, &header, &body, now, TOLERANCE_S),
            Err(RailError::BadSignature(_))
        ));
    }

    /// A body altered AFTER signing is refused — the HMAC covers `{t}.{body}` exactly.
    #[test]
    fn tampered_body_fails() {
        let body = fixture_body();
        let now = now_s();
        let header = sign(SECRET, now, &body);
        let tampered = String::from_utf8(body)
            .unwrap()
            .replace("\"qty\":\"2\"", "\"qty\":\"9\"");
        assert!(matches!(
            verify_sig(SECRET, &header, tampered.as_bytes(), now, TOLERANCE_S),
            Err(RailError::BadSignature(_))
        ));
    }

    /// A correctly signed but STALE event (replay) is refused at the 300s fence.
    #[test]
    fn stale_timestamp_fails() {
        let body = fixture_body();
        let now = now_s();
        let old = now - TOLERANCE_S - 1;
        let header = sign(SECRET, old, &body);
        assert!(matches!(
            verify_sig(SECRET, &header, &body, now, TOLERANCE_S),
            Err(RailError::BadSignature(_))
        ));
        // The same event inside the fence passes — the fence, not the sig, was the refusal.
        let fresh = sign(SECRET, now - TOLERANCE_S + 5, &body);
        verify_sig(SECRET, &fresh, &body, now, TOLERANCE_S).expect("inside tolerance must pass");
    }

    /// An event type we do not act on is Ignored, never an error — Stripe must see 200.
    #[test]
    fn unhandled_event_type_is_ignored() {
        let body = serde_json::json!({"type": "invoice.paid", "data": {"object": {}}})
            .to_string()
            .into_bytes();
        assert!(matches!(
            parse_event(&body).unwrap(),
            RailEvent::Ignored { kind } if kind == "invoice.paid"
        ));
    }

    /// GOLDEN (snapshot): the exact form encoding of a checkout session — direct-charge
    /// fee key included. If this changes, a Stripe API contract changed on purpose.
    #[test]
    fn checkout_form_encoding_is_pinned() {
        let listing = RegisteredListing {
            listing: "aabbccddeeff0011".into(),
            owner_pk: "ownerpk".into(),
            delegate_pk: Some("delegatepk".into()),
            unit_cents: 1200,
            currency: "usd".into(),
            capacity: 100,
            open: true,
            acct: Some("acct_1".into()),
            rev: 1,
        };
        let form = checkout_form(
            &listing,
            2,
            "deadbeef",
            10,
            "https://pacific.example/return",
            1750000000,
        );
        assert_eq!(
            form,
            "mode=payment\
             &line_items[0][price_data][currency]=usd\
             &line_items[0][price_data][unit_amount]=1200\
             &line_items[0][price_data][product_data][name]=Ticket%20aabbccdd\
             &line_items[0][quantity]=2\
             &payment_intent_data[application_fee_amount]=20\
             &success_url=https%3A%2F%2Fpacific.example%2Freturn\
             &expires_at=1750000000\
             &metadata[listing]=aabbccddeeff0011\
             &metadata[buyer_pk]=deadbeef\
             &metadata[qty]=2\
             &metadata[fee]=10"
        );
    }
}
