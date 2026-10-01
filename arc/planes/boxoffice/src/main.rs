//! boxoffice — the Arc's `/v1/box` plane: the event-ticketing money machinery (Slice 2A).
//!
//! What lives here: listing registration (owner-signed, LWW by rev), rail checkout with
//! the D22 capacity soft-gate and the 9A open-session dedup, webhook verification, the
//! paid/fulfilled/refunded session state machine, and the online door's redemption table
//! (D24). What does NOT live here yet: the embedded pacific-core FULFILLMENT DELEGATE
//! (Slice 2B) — the node that member-tethers into each Event group, authors recordSale +
//! deliverTicket, and runs the 4A expiry sweep. Its seams are here waiting: the webhook
//! marks `paid` (its wake-up), `/fulfill` is the mark it sets LAST (D23), and
//! `Rail::retrieve` is its re-verify sweep.
//!
//! THE INVARIANT (rail.rs): Pacific nets exactly the 10¢-equivalent per ticket;
//! processing costs ride the organizer's side.
//!
//! Fail-loud posture: missing STRIPE_SECRET_KEY / STRIPE_WEBHOOK_SECRET logs an error at
//! boot and the plane serves ONLY the free-path routes — priced checkout answers 503
//! `rail_unconfigured`. The free path (listing / verify / door / health) must never be
//! blocked by missing rail keys: free events are phone-fulfilled and owe nothing to Stripe.
//!
//! Env:
//!   PORT                        bind port (default 8793; arcup passes ARC_BOXOFFICE_PORT)
//!   ARC_STATE_DIR               sqlite lives at $ARC_STATE_DIR/boxoffice.db (default .)
//!   STRIPE_SECRET_KEY           the platform secret (sk_...)
//!   STRIPE_WEBHOOK_SECRET       the endpoint secret (whsec_...)
//!   ARC_BOXOFFICE_SUCCESS_URL   the hosted "return to Pacific" page (D21 — no deep links)
//!   ARC_BOXOFFICE_MOCK_RAIL     "1" + --mock-rail = TEST-ONLY mock rail (harness only)
//!
//! Routes (all proxied by arc-gateway at Guest — deliberately enrolled, design §5 v1.1;
//! auth is PLANE-LEVEL: owner/delegate Ed25519 signatures and the rail's webhook HMAC):
//!   POST /v1/box/listing              owner-signed    register/update listing (LWW by rev)
//!   POST /v1/box/onboard              signed          rail onboarding link (Express)
//!   GET  /v1/box/acct/:acct           signed          charges_enabled
//!   POST /v1/box/checkout             open            {listing, qty, buyer_pk} ONLY
//!   POST /v1/box/stripe               rail HMAC       webhook → paid (wakes the 2B agent)
//!   GET  /v1/box/verify/:pi           open            paid/fulfilled/amounts/buyer_pk
//!   POST /v1/box/fulfill/:pi          owner/delegate  idempotent; 409 on repeat (D23)
//!   POST /v1/box/refund/:pi           owner-signed    refuses if fulfilled (D19)
//!   GET/POST /v1/box/door/:l/:ticket  owner/delegate  redemption read/write (D24)
//!   GET  /v1/box/health (+ /health)   open

mod auth;
mod rail;
mod rates;
mod store;
mod stripe;

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::json;

use rail::{Rail, RailEvent};
use store::{RegisteredListing, Session, Store};

struct BoxOffice {
    store: Store,
    /// `None` = rail unconfigured. The free path serves regardless; priced checkout 503s.
    rail: Option<Arc<dyn Rail>>,
}

type App = Arc<BoxOffice>;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A `{ "error": { "code", "message" } }` envelope with an HTTP status — the gateway's
/// own idiom (ICD §11), so a caller sees one error shape on both sides of the proxy.
fn err(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    let body = json!({ "error": { "code": code, "message": message.into() } });
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn ok(body: serde_json::Value) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn storage_err(e: rusqlite::Error) -> Response {
    eprintln!("boxoffice: storage error: {e}");
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage",
        "storage error",
    )
}

fn unauthorized(e: auth::AuthError) -> Response {
    err(StatusCode::UNAUTHORIZED, "unauthorized", e.to_string())
}

/// Is `pk` the listing's owner or its fulfillment delegate? (2B TODO: door routes should
/// accept any EVENT-group member via the embedded node's fold; until then doors are
/// owner-or-delegate — see auth.rs.)
fn owner_or_delegate(l: &RegisteredListing, pk: &str) -> bool {
    pk == l.owner_pk || l.delegate_pk.as_deref() == Some(pk)
}

// -- handlers -----------------------------------------------------------------

async fn health(State(app): State<App>) -> Response {
    ok(json!({ "status": "ok", "rail": app.rail.is_some() }))
}

/// POST /v1/box/listing — register or update a listing. First registration must be
/// signed by the pk INSIDE the payload (`owner_pk` proves itself); an update must be
/// signed by the ALREADY-REGISTERED owner — so a listing can never be stolen by
/// re-registering it with a new owner_pk.
async fn register_listing(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Response {
    let pk = match auth::verify(&headers, "POST", "/v1/box/listing", &body, now_ms()) {
        Ok(pk) => pk,
        Err(e) => return unauthorized(e),
    };
    let l: RegisteredListing = match serde_json::from_slice(&body) {
        Ok(l) => l,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_request", e.to_string()),
    };
    let expected_signer = match app.store.get_listing(&l.listing) {
        Ok(Some(existing)) => existing.owner_pk,
        Ok(None) => l.owner_pk.clone(),
        Err(e) => return storage_err(e),
    };
    if pk != expected_signer {
        return err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "listing registration must be signed by the listing's owner",
        );
    }
    if l.unit_cents > 0 {
        // acct+delegate required iff priced (design §2) — a priced listing with no
        // connected account or no fulfillment delegate could take money it cannot serve.
        if l.acct.is_none() || l.delegate_pk.is_none() {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid_listing",
                "a priced listing requires both acct and delegate_pk",
            );
        }
        // Refuse an un-priceable currency at registration, not at the first lost sale.
        if rates::fee_minor(&l.currency).is_none() {
            return err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "unknown_currency",
                format!(
                    "no 10¢-equivalent known for '{}' — refusing rather than guessing",
                    l.currency
                ),
            );
        }
    }
    match app.store.upsert_listing(&l) {
        Ok(true) => ok(json!({ "listing": l.listing, "rev": l.rev })),
        Ok(false) => err(
            StatusCode::CONFLICT,
            "stale_rev",
            "a registration with an equal or higher rev already exists (LWW by rev)",
        ),
        Err(e) => storage_err(e),
    }
}

#[derive(serde::Deserialize)]
struct CheckoutReq {
    listing: String,
    qty: u32,
    buyer_pk: String,
}

/// POST /v1/box/checkout — open. `{listing, qty, buyer_pk}` ONLY: price, currency and
/// account come from the REGISTERED listing, never from the client.
async fn checkout(State(app): State<App>, body: Bytes) -> Response {
    let req: CheckoutReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_request", e.to_string()),
    };
    if req.qty == 0 {
        return err(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "qty must be at least 1",
        );
    }
    let now = now_ms();
    let listing = match app.store.get_listing(&req.listing) {
        Ok(Some(l)) => l,
        Ok(None) => {
            return err(
                StatusCode::NOT_FOUND,
                "unknown_listing",
                "listing not registered",
            )
        }
        Err(e) => return storage_err(e),
    };
    if !listing.open {
        return err(
            StatusCode::CONFLICT,
            "closed",
            "ticket sales are closed for this listing",
        );
    }
    if listing.unit_cents == 0 {
        return err(
            StatusCode::BAD_REQUEST,
            "free_listing",
            "free events are phone-fulfilled — there is no checkout to open",
        );
    }
    // 9A dedup FIRST: the buyer's existing open session is THE answer, even when the
    // remaining capacity has since gone to zero — their hold is part of that count.
    match app.store.open_session_for(&req.listing, &req.buyer_pk, now) {
        Ok(Some(s)) => {
            return ok(json!({ "url": s.url, "pi": s.pi, "deduped": true }));
        }
        Ok(None) => {}
        Err(e) => return storage_err(e),
    }
    // D22 capacity SOFT-gate: capacity − paid − open ≤ 0 → sold out. Soft on purpose —
    // the delegate's recordSale is the final gate (2B); boundary losers are auto-refunded.
    let (paid, open) = match (
        app.store.paid_count(&req.listing),
        app.store.open_count(&req.listing, now),
    ) {
        (Ok(p), Ok(o)) => (p, o),
        (Err(e), _) | (_, Err(e)) => return storage_err(e),
    };
    if listing.capacity - paid - open <= 0 {
        return err(
            StatusCode::CONFLICT,
            "sold_out",
            "no capacity left for this listing",
        );
    }
    let Some(rail) = app.rail.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "rail_unconfigured",
            "no payment rail is configured on this Arc — priced checkout cannot open (the free path is unaffected)",
        );
    };
    let Some(fee_minor) = rates::fee_minor(&listing.currency) else {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_currency",
            format!(
                "no 10¢-equivalent known for '{}' — refusing rather than guessing",
                listing.currency
            ),
        );
    };
    let cs = match rail
        .create_checkout(&listing, req.qty, &req.buyer_pk, fee_minor)
        .await
    {
        Ok(cs) => cs,
        Err(e) => {
            eprintln!("boxoffice: create_checkout failed for {}: {e}", req.listing);
            return err(StatusCode::BAD_GATEWAY, "rail_failed", e.to_string());
        }
    };
    // The rail session id is the handle the Stripe dashboard shows — log the pair once
    // so a money question can be answered from either side's identifier.
    eprintln!(
        "boxoffice: CHECKOUT listing={} qty={} pi={} session={} fee_minor={fee_minor}",
        listing.listing, req.qty, cs.pi, cs.session_id
    );
    let session = Session {
        pi: cs.pi.clone(),
        listing: listing.listing.clone(),
        buyer_pk: req.buyer_pk.clone(),
        qty: req.qty as i64,
        amount_cents: listing.unit_cents * req.qty as i64,
        fee_cents: fee_minor * req.qty as i64,
        currency: listing.currency.clone(),
        url: cs.url.clone(),
        state: "open".into(),
        created_ms: now,
    };
    if let Err(e) = app.store.insert_session(&session) {
        return storage_err(e);
    }
    ok(json!({ "url": cs.url, "pi": cs.pi, "expires_at_ms": cs.expires_at_ms, "deduped": false }))
}

/// POST /v1/box/stripe — the rail webhook. Verified by the adapter (HMAC, ±300s) before
/// a byte of it is believed; marking paid is idempotent, so Stripe's retries are safe.
async fn rail_webhook(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(rail) = app.rail.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "rail_unconfigured",
            "no payment rail is configured — a webhook cannot be verified without its secret",
        );
    };
    match rail.verify_webhook(&headers, &body) {
        Err(e) => err(StatusCode::BAD_REQUEST, "bad_signature", e.to_string()),
        Ok(RailEvent::Ignored { kind }) => ok(json!({ "received": true, "ignored": kind })),
        Ok(RailEvent::CheckoutCompleted {
            pi,
            listing,
            buyer_pk,
            qty,
            amount_minor,
            currency,
            fee_minor,
        }) => {
            // Webhook-first arrival: a session this process never saw (created before a
            // crash, or created under a session-id key because the rail deferred the pi)
            // is reconstructed from the event's own metadata rather than dropped.
            match app.store.get_session(&pi) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    let s = Session {
                        pi: pi.clone(),
                        listing,
                        buyer_pk,
                        qty,
                        amount_cents: amount_minor,
                        fee_cents: fee_minor * qty,
                        currency,
                        url: String::new(),
                        state: "open".into(),
                        created_ms: now_ms(),
                    };
                    if let Err(e) = app.store.insert_session(&s) {
                        return storage_err(e);
                    }
                }
                Err(e) => return storage_err(e),
            }
            match app.store.mark_paid(&pi) {
                Ok(_) => ok(json!({ "received": true })), // Applied or Noop — both are done
                Err(e) => storage_err(e),
            }
        }
    }
}

/// GET /v1/box/verify/:pi — open: anyone holding a pi may ask what it bought.
async fn verify_pi(State(app): State<App>, Path(pi): Path<String>) -> Response {
    match app.store.get_session(&pi) {
        Ok(Some(s)) => ok(json!({
            "pi": s.pi,
            "listing": s.listing,
            "buyer_pk": s.buyer_pk,
            "qty": s.qty,
            "state": s.state,
            "paid": s.state == "paid" || s.state == "fulfilled",
            "fulfilled": s.state == "fulfilled",
            "amount_cents": s.amount_cents,
            "fee_cents": s.fee_cents,
            "currency": s.currency,
        })),
        Ok(None) => err(
            StatusCode::NOT_FOUND,
            "unknown_pi",
            "no session under that pi",
        ),
        Err(e) => storage_err(e),
    }
}

/// POST /v1/box/fulfill/:pi — owner-or-delegate signed. Called LAST in the fulfillment
/// flow (D23); the repeat answers 409 so a crashed-and-retrying agent KNOWS delivery
/// already happened and reconciles from the group ledger instead of re-running.
async fn fulfill(
    State(app): State<App>,
    Path(pi): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = format!("/v1/box/fulfill/{pi}");
    let pk = match auth::verify(&headers, "POST", &path, &body, now_ms()) {
        Ok(pk) => pk,
        Err(e) => return unauthorized(e),
    };
    let session = match app.store.get_session(&pi) {
        Ok(Some(s)) => s,
        Ok(None) => {
            return err(
                StatusCode::NOT_FOUND,
                "unknown_pi",
                "no session under that pi",
            )
        }
        Err(e) => return storage_err(e),
    };
    let listing = match app.store.get_listing(&session.listing) {
        Ok(Some(l)) => l,
        Ok(None) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage",
                "session without listing",
            )
        }
        Err(e) => return storage_err(e),
    };
    if !owner_or_delegate(&listing, &pk) {
        return err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "fulfill requires the owner or delegate key",
        );
    }
    match app.store.mark_fulfilled(&pi) {
        Ok(store::Fulfill::Fulfilled) => ok(json!({ "pi": pi, "state": "fulfilled" })),
        Ok(store::Fulfill::AlreadyFulfilled) => err(
            StatusCode::CONFLICT,
            "already_fulfilled",
            "this session is already fulfilled — reconcile from the group ledger, not by retrying",
        ),
        Ok(store::Fulfill::NotPaid) => err(
            StatusCode::CONFLICT,
            "not_paid",
            "only a paid session can be fulfilled",
        ),
        Ok(store::Fulfill::NotFound) => err(
            StatusCode::NOT_FOUND,
            "unknown_pi",
            "no session under that pi",
        ),
        Err(e) => storage_err(e),
    }
}

/// POST /v1/box/refund/:pi — owner-signed. REFUSES a fulfilled session: the ticket is
/// delivered and voidTicket does not exist yet (D19) — a refund here would be
/// self-refund-keep-ticket. Refund-of-refunded is an idempotent 200.
async fn refund(
    State(app): State<App>,
    Path(pi): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = format!("/v1/box/refund/{pi}");
    let pk = match auth::verify(&headers, "POST", &path, &body, now_ms()) {
        Ok(pk) => pk,
        Err(e) => return unauthorized(e),
    };
    let session = match app.store.get_session(&pi) {
        Ok(Some(s)) => s,
        Ok(None) => {
            return err(
                StatusCode::NOT_FOUND,
                "unknown_pi",
                "no session under that pi",
            )
        }
        Err(e) => return storage_err(e),
    };
    let listing = match app.store.get_listing(&session.listing) {
        Ok(Some(l)) => l,
        Ok(None) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage",
                "session without listing",
            )
        }
        Err(e) => return storage_err(e),
    };
    if pk != listing.owner_pk {
        return err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "refund requires the owner key",
        );
    }
    match session.state.as_str() {
        "fulfilled" => err(
            StatusCode::CONFLICT,
            "fulfilled",
            "a delivered ticket cannot be refunded — voidTicket does not exist yet (D19)",
        ),
        "refunded" => ok(json!({ "pi": pi, "state": "refunded" })),
        "open" | "expired" => err(
            StatusCode::CONFLICT,
            "not_paid",
            "nothing was paid on this session",
        ),
        "paid" => {
            let Some(rail) = app.rail.clone() else {
                return err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "rail_unconfigured",
                    "no payment rail is configured — cannot move money back",
                );
            };
            if let Err(e) = rail.refund(&pi, listing.acct.as_deref()).await {
                eprintln!("boxoffice: refund failed for {pi}: {e}");
                return err(StatusCode::BAD_GATEWAY, "rail_failed", e.to_string());
            }
            match app.store.mark_refunded(&pi) {
                Ok(_) => ok(json!({ "pi": pi, "state": "refunded" })),
                Err(e) => storage_err(e),
            }
        }
        other => err(
            StatusCode::CONFLICT,
            "bad_state",
            format!("session is {other}"),
        ),
    }
}

/// GET /v1/box/door/:listing/:ticket — has this ticket been redeemed? (D24 online door.)
async fn door_get(
    State(app): State<App>,
    Path((listing_id, ticket)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let path = format!("/v1/box/door/{listing_id}/{ticket}");
    let pk = match auth::verify(&headers, "GET", &path, &[], now_ms()) {
        Ok(pk) => pk,
        Err(e) => return unauthorized(e),
    };
    let listing = match app.store.get_listing(&listing_id) {
        Ok(Some(l)) => l,
        Ok(None) => {
            return err(
                StatusCode::NOT_FOUND,
                "unknown_listing",
                "listing not registered",
            )
        }
        Err(e) => return storage_err(e),
    };
    if !owner_or_delegate(&listing, &pk) {
        return err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the door requires the owner or delegate key",
        );
    }
    match app.store.redemption(&listing_id, &ticket) {
        Ok(Some(r)) => ok(json!({
            "listing": listing_id, "ticket": ticket,
            "redeemed": true, "door_pk": r.door_pk, "redeemed_ms": r.redeemed_ms,
        })),
        Ok(None) => ok(json!({ "listing": listing_id, "ticket": ticket, "redeemed": false })),
        Err(e) => storage_err(e),
    }
}

/// POST /v1/box/door/:listing/:ticket — scan. First writer admits; a double scan is a
/// 200 carrying who admitted it first — a flag, never a rejection (design §2).
async fn door_post(
    State(app): State<App>,
    Path((listing_id, ticket)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = format!("/v1/box/door/{listing_id}/{ticket}");
    let pk = match auth::verify(&headers, "POST", &path, &body, now_ms()) {
        Ok(pk) => pk,
        Err(e) => return unauthorized(e),
    };
    let listing = match app.store.get_listing(&listing_id) {
        Ok(Some(l)) => l,
        Ok(None) => {
            return err(
                StatusCode::NOT_FOUND,
                "unknown_listing",
                "listing not registered",
            )
        }
        Err(e) => return storage_err(e),
    };
    if !owner_or_delegate(&listing, &pk) {
        return err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the door requires the owner or delegate key",
        );
    }
    match app.store.redeem(&listing_id, &ticket, &pk, now_ms()) {
        Ok(store::Redeem::Admitted) => ok(json!({ "admitted": true })),
        Ok(store::Redeem::Already {
            door_pk,
            redeemed_ms,
        }) => ok(json!({
            "admitted": false,
            "already": { "door_pk": door_pk, "redeemed_ms": redeemed_ms },
        })),
        Err(e) => storage_err(e),
    }
}

#[derive(serde::Deserialize)]
struct OnboardReq {
    country: String,
}

/// POST /v1/box/onboard — a signed request for a rail onboarding link. No listing exists
/// yet at this point (D29: onboard first, register the priced listing with the resulting
/// acct after), so "owner-signed" here means the prospective owner proves key possession.
async fn onboard(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(e) = auth::verify(&headers, "POST", "/v1/box/onboard", &body, now_ms()) {
        return unauthorized(e);
    }
    let req: OnboardReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_REQUEST, "bad_request", e.to_string()),
    };
    let Some(rail) = app.rail.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "rail_unconfigured",
            "no payment rail is configured — onboarding needs one",
        );
    };
    match rail.onboard(&req.country).await {
        Ok(link) => ok(json!({ "url": link.url, "acct": link.acct })),
        Err(e) => err(StatusCode::BAD_GATEWAY, "rail_failed", e.to_string()),
    }
}

/// GET /v1/box/acct/:acct — charges_enabled (the D29 gate the composer polls).
async fn acct_status(
    State(app): State<App>,
    Path(acct): Path<String>,
    headers: HeaderMap,
) -> Response {
    let path = format!("/v1/box/acct/{acct}");
    if let Err(e) = auth::verify(&headers, "GET", &path, &[], now_ms()) {
        return unauthorized(e);
    }
    let Some(rail) = app.rail.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "rail_unconfigured",
            "no payment rail is configured",
        );
    };
    match rail.account_status(&acct).await {
        Ok(enabled) => ok(json!({ "acct": acct, "charges_enabled": enabled })),
        Err(e) => err(StatusCode::BAD_GATEWAY, "rail_failed", e.to_string()),
    }
}

fn router(app: App) -> Router {
    Router::new()
        // Bare /health too: arcup and the gateway's liveness probe expect it on planes.
        .route("/health", get(health))
        .route("/v1/box/health", get(health))
        .route("/v1/box/listing", post(register_listing))
        .route("/v1/box/onboard", post(onboard))
        .route("/v1/box/acct/:acct", get(acct_status))
        .route("/v1/box/checkout", post(checkout))
        .route("/v1/box/stripe", post(rail_webhook))
        .route("/v1/box/verify/:pi", get(verify_pi))
        .route("/v1/box/fulfill/:pi", post(fulfill))
        .route("/v1/box/refund/:pi", post(refund))
        .route(
            "/v1/box/door/:listing/:ticket",
            get(door_get).post(door_post),
        )
        .with_state(app)
}

#[tokio::main]
async fn main() {
    // PROVENANCE, first thing and before any side effect: log which commit this binary is,
    // or answer `--version` and exit. An unstamped build says so in words. See lib/arc-build.
    arc_build::stamp!();

    let state_dir = std::env::var("ARC_STATE_DIR").unwrap_or_else(|_| ".".to_string());
    let db = format!("{state_dir}/boxoffice.db");
    let store = Store::open(&db).unwrap_or_else(|e| {
        eprintln!("boxoffice: cannot open {db}: {e}");
        std::process::exit(1);
    });

    let rail: Option<Arc<dyn Rail>> = if std::env::args().any(|a| a == "--mock-rail") {
        // TEST-ONLY, and it must be asked for TWICE (flag + env) — never a fallback a
        // production misconfiguration could drift into.
        if std::env::var("ARC_BOXOFFICE_MOCK_RAIL").as_deref() != Ok("1") {
            eprintln!(
                "boxoffice: --mock-rail given but ARC_BOXOFFICE_MOCK_RAIL=1 is not set — \
                 the mock rail is TEST-ONLY and must be enabled explicitly; refusing to start"
            );
            std::process::exit(2);
        }
        eprintln!("boxoffice: MOCK RAIL — harness mode, no real money can move");
        Some(Arc::new(rail::MockRail::default()))
    } else {
        match (
            std::env::var("STRIPE_SECRET_KEY").ok(),
            std::env::var("STRIPE_WEBHOOK_SECRET").ok(),
        ) {
            (Some(sk), Some(ws)) if !sk.trim().is_empty() && !ws.trim().is_empty() => {
                let success_url = std::env::var("ARC_BOXOFFICE_SUCCESS_URL")
                    .unwrap_or_else(|_| "https://kenjin.cc/tickets/return".to_string());
                Some(Arc::new(stripe::StripeRail::new(sk, ws, success_url)))
            }
            _ => {
                eprintln!(
                    "boxoffice: ERROR — STRIPE_SECRET_KEY / STRIPE_WEBHOOK_SECRET unset: the \
                     payment rail is DOWN. Serving the free path only (listing/verify/door/health); \
                     priced checkout answers 503 rail_unconfigured. The free path is never \
                     blocked by missing rail keys."
                );
                None
            }
        }
    };

    // FX staleness is loud but never blocking (design §4): last-known always serves.
    let stale = rates::staleness_days(now_ms());
    if stale > 7 {
        eprintln!(
            "boxoffice: FX table is {stale} days stale (>7) — refresh rates.json; \
             last-known rates still serve, a rates outage never blocks sales"
        );
    }

    let app = router(Arc::new(BoxOffice { store, rail }));
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8793);
    let bind = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|e| {
            eprintln!("boxoffice: could not bind {bind}: {e}");
            std::process::exit(1);
        });
    eprintln!("boxoffice: /v1/box plane on http://{bind} (db {db})");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|e| eprintln!("boxoffice: serve error: {e}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use ed25519_dalek::SigningKey;
    use tower::ServiceExt;

    fn sk(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn pk_hex(key: &SigningKey) -> String {
        hex::encode(key.verifying_key().to_bytes())
    }

    fn app_with_mock_rail() -> Router {
        let store = Store::open_in_memory().unwrap();
        router(Arc::new(BoxOffice {
            store,
            rail: Some(Arc::new(rail::MockRail::default())),
        }))
    }

    fn app_without_rail() -> Router {
        let store = Store::open_in_memory().unwrap();
        router(Arc::new(BoxOffice { store, rail: None }))
    }

    /// Drive the router in-process. `sign` mints an X-Pacific-Sig over exactly what is
    /// sent; `extra` carries the mock webhook's signature header.
    async fn call(
        app: &Router,
        method: &str,
        path: &str,
        body: serde_json::Value,
        sign: Option<&SigningKey>,
        extra: &[(&str, &str)],
    ) -> (u16, serde_json::Value) {
        let bytes = if body.is_null() {
            Vec::new()
        } else {
            body.to_string().into_bytes()
        };
        let mut rb = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(key) = sign {
            rb = rb.header(
                auth::SIG_HEADER,
                auth::sign_header(key, method, path, &bytes, now_ms()),
            );
        }
        for (k, v) in extra {
            rb = rb.header(*k, *v);
        }
        let resp = app
            .clone()
            .oneshot(rb.body(Body::from(bytes)).unwrap())
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let b = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let v = if b.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&b).unwrap_or(serde_json::Value::Null)
        };
        (status, v)
    }

    fn listing_payload(
        owner: &SigningKey,
        delegate: &SigningKey,
        capacity: i64,
        rev: i64,
    ) -> serde_json::Value {
        json!({
            "listing": "l1", "owner_pk": pk_hex(owner), "delegate_pk": pk_hex(delegate),
            "unit_cents": 1200, "currency": "usd", "capacity": capacity, "open": true,
            "acct": "acct_1", "rev": rev,
        })
    }

    fn mock_paid_event(pi: &str, buyer_pk: &str, qty: i64) -> serde_json::Value {
        json!({
            "type": "checkout.session.completed", "pi": pi, "listing": "l1",
            "buyer_pk": buyer_pk, "qty": qty, "amount_minor": 1200 * qty,
            "currency": "usd", "fee_minor": 10,
        })
    }

    /// The full happy path: register → checkout → webhook → verify → fulfill → 409.
    #[tokio::test]
    async fn happy_path_to_fulfilled_and_409_on_repeat() {
        let app = app_with_mock_rail();
        let (owner, delegate, buyer) = (sk(1), sk(2), pk_hex(&sk(3)));

        let (st, _) = call(
            &app,
            "POST",
            "/v1/box/listing",
            listing_payload(&owner, &delegate, 5, 1),
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(st, 200);

        let (st, v) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l1", "qty": 2, "buyer_pk": buyer}),
            None,
            &[],
        )
        .await;
        assert_eq!(st, 200);
        let pi = v["pi"].as_str().unwrap().to_string();
        assert!(v["url"].as_str().unwrap().starts_with("mock://checkout/"));
        assert_eq!(v["deduped"], false);

        let (st, _) = call(
            &app,
            "POST",
            "/v1/box/stripe",
            mock_paid_event(&pi, &buyer, 2),
            None,
            &[(rail::MockRail::SIG_HEADER, rail::MockRail::SIG_VALUE)],
        )
        .await;
        assert_eq!(st, 200);

        let (st, v) = call(
            &app,
            "GET",
            &format!("/v1/box/verify/{pi}"),
            serde_json::Value::Null,
            None,
            &[],
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["paid"], true);
        assert_eq!(v["fulfilled"], false);
        assert_eq!(v["buyer_pk"].as_str().unwrap(), buyer);
        assert_eq!(v["fee_cents"], 20, "the 10¢-equivalent × qty snapshot");

        let (st, v) = call(
            &app,
            "POST",
            &format!("/v1/box/fulfill/{pi}"),
            json!({}),
            Some(&delegate),
            &[],
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["state"], "fulfilled");

        // The repeat is the designed 409 — a retrying agent reconciles, never re-runs.
        let (st, v) = call(
            &app,
            "POST",
            &format!("/v1/box/fulfill/{pi}"),
            json!({}),
            Some(&delegate),
            &[],
        )
        .await;
        assert_eq!(st, 409);
        assert_eq!(v["error"]["code"], "already_fulfilled");

        let (st, v) = call(
            &app,
            "GET",
            &format!("/v1/box/verify/{pi}"),
            serde_json::Value::Null,
            None,
            &[],
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["fulfilled"], true);
    }

    /// D22: open holds count against capacity — the second buyer is refused sold_out.
    #[tokio::test]
    async fn sold_out_refusal() {
        let app = app_with_mock_rail();
        let (owner, delegate) = (sk(1), sk(2));
        call(
            &app,
            "POST",
            "/v1/box/listing",
            listing_payload(&owner, &delegate, 2, 1),
            Some(&owner),
            &[],
        )
        .await;

        let (st, _) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l1", "qty": 2, "buyer_pk": pk_hex(&sk(3))}),
            None,
            &[],
        )
        .await;
        assert_eq!(st, 200);
        let (st, v) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l1", "qty": 1, "buyer_pk": pk_hex(&sk(4))}),
            None,
            &[],
        )
        .await;
        assert_eq!(st, 409);
        assert_eq!(v["error"]["code"], "sold_out");
    }

    /// 9A: a second checkout by the same buyer returns the SAME session, not a new hold.
    #[tokio::test]
    async fn checkout_dedup_returns_existing_session() {
        let app = app_with_mock_rail();
        let (owner, delegate, buyer) = (sk(1), sk(2), pk_hex(&sk(3)));
        call(
            &app,
            "POST",
            "/v1/box/listing",
            listing_payload(&owner, &delegate, 10, 1),
            Some(&owner),
            &[],
        )
        .await;

        let (_, first) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l1", "qty": 1, "buyer_pk": buyer}),
            None,
            &[],
        )
        .await;
        let (st, second) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l1", "qty": 1, "buyer_pk": buyer}),
            None,
            &[],
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(second["deduped"], true);
        assert_eq!(second["pi"], first["pi"]);
        assert_eq!(second["url"], first["url"]);
    }

    /// D19: a fulfilled session cannot be refunded (voidTicket does not exist yet), and a
    /// paid one can — by the owner only.
    #[tokio::test]
    async fn refund_rules() {
        let app = app_with_mock_rail();
        let (owner, delegate, buyer) = (sk(1), sk(2), pk_hex(&sk(3)));
        call(
            &app,
            "POST",
            "/v1/box/listing",
            listing_payload(&owner, &delegate, 10, 1),
            Some(&owner),
            &[],
        )
        .await;

        // Session A: paid then fulfilled — refund refused.
        let (_, v) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l1", "qty": 1, "buyer_pk": buyer}),
            None,
            &[],
        )
        .await;
        let pi_a = v["pi"].as_str().unwrap().to_string();
        call(
            &app,
            "POST",
            "/v1/box/stripe",
            mock_paid_event(&pi_a, &buyer, 1),
            None,
            &[(rail::MockRail::SIG_HEADER, rail::MockRail::SIG_VALUE)],
        )
        .await;
        call(
            &app,
            "POST",
            &format!("/v1/box/fulfill/{pi_a}"),
            json!({}),
            Some(&delegate),
            &[],
        )
        .await;
        let (st, v) = call(
            &app,
            "POST",
            &format!("/v1/box/refund/{pi_a}"),
            json!({}),
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(st, 409);
        assert_eq!(v["error"]["code"], "fulfilled");

        // Session B: paid, not fulfilled — delegate may NOT refund, owner may.
        let buyer_b = pk_hex(&sk(4));
        let (_, v) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l1", "qty": 1, "buyer_pk": buyer_b}),
            None,
            &[],
        )
        .await;
        let pi_b = v["pi"].as_str().unwrap().to_string();
        call(
            &app,
            "POST",
            "/v1/box/stripe",
            mock_paid_event(&pi_b, &buyer_b, 1),
            None,
            &[(rail::MockRail::SIG_HEADER, rail::MockRail::SIG_VALUE)],
        )
        .await;
        let (st, _) = call(
            &app,
            "POST",
            &format!("/v1/box/refund/{pi_b}"),
            json!({}),
            Some(&delegate),
            &[],
        )
        .await;
        assert_eq!(st, 403, "refund is owner-only (D19)");
        let (st, v) = call(
            &app,
            "POST",
            &format!("/v1/box/refund/{pi_b}"),
            json!({}),
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["state"], "refunded");
        // Idempotent repeat.
        let (st, _) = call(
            &app,
            "POST",
            &format!("/v1/box/refund/{pi_b}"),
            json!({}),
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(st, 200);
    }

    /// Unsigned or wrong-key fulfill never passes.
    #[tokio::test]
    async fn fulfill_requires_owner_or_delegate_signature() {
        let app = app_with_mock_rail();
        let (owner, delegate, buyer) = (sk(1), sk(2), pk_hex(&sk(3)));
        call(
            &app,
            "POST",
            "/v1/box/listing",
            listing_payload(&owner, &delegate, 10, 1),
            Some(&owner),
            &[],
        )
        .await;
        let (_, v) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l1", "qty": 1, "buyer_pk": buyer}),
            None,
            &[],
        )
        .await;
        let pi = v["pi"].as_str().unwrap().to_string();
        call(
            &app,
            "POST",
            "/v1/box/stripe",
            mock_paid_event(&pi, &buyer, 1),
            None,
            &[(rail::MockRail::SIG_HEADER, rail::MockRail::SIG_VALUE)],
        )
        .await;

        let (st, _) = call(
            &app,
            "POST",
            &format!("/v1/box/fulfill/{pi}"),
            json!({}),
            None,
            &[],
        )
        .await;
        assert_eq!(st, 401, "unsigned fulfill is refused");
        let (st, _) = call(
            &app,
            "POST",
            &format!("/v1/box/fulfill/{pi}"),
            json!({}),
            Some(&sk(9)),
            &[],
        )
        .await;
        assert_eq!(st, 403, "a valid signature by the WRONG key is refused");
        let (st, _) = call(
            &app,
            "POST",
            &format!("/v1/box/fulfill/{pi}"),
            json!({}),
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(
            st, 200,
            "the owner is also a valid fulfiller (delegate-less events)"
        );
    }

    /// A webhook that does not prove its origin is never processed.
    #[tokio::test]
    async fn webhook_bad_signature_refused() {
        let app = app_with_mock_rail();
        let (owner, delegate, buyer) = (sk(1), sk(2), pk_hex(&sk(3)));
        call(
            &app,
            "POST",
            "/v1/box/listing",
            listing_payload(&owner, &delegate, 10, 1),
            Some(&owner),
            &[],
        )
        .await;
        let (_, v) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l1", "qty": 1, "buyer_pk": buyer}),
            None,
            &[],
        )
        .await;
        let pi = v["pi"].as_str().unwrap().to_string();

        let (st, _) = call(
            &app,
            "POST",
            "/v1/box/stripe",
            mock_paid_event(&pi, &buyer, 1),
            None,
            &[],
        )
        .await;
        assert_eq!(st, 400, "no signature header");
        let (st, _) = call(
            &app,
            "POST",
            "/v1/box/stripe",
            mock_paid_event(&pi, &buyer, 1),
            None,
            &[(rail::MockRail::SIG_HEADER, "wrong")],
        )
        .await;
        assert_eq!(st, 400, "wrong signature value");
        let (_, v) = call(
            &app,
            "GET",
            &format!("/v1/box/verify/{pi}"),
            serde_json::Value::Null,
            None,
            &[],
        )
        .await;
        assert_eq!(v["paid"], false, "the unverified webhook changed nothing");
    }

    /// LWW by rev at the route: a stale re-registration answers 409.
    #[tokio::test]
    async fn listing_lww_and_owner_binding() {
        let app = app_with_mock_rail();
        let (owner, delegate) = (sk(1), sk(2));
        let (st, _) = call(
            &app,
            "POST",
            "/v1/box/listing",
            listing_payload(&owner, &delegate, 5, 2),
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(st, 200);
        let (st, v) = call(
            &app,
            "POST",
            "/v1/box/listing",
            listing_payload(&owner, &delegate, 9, 2),
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(st, 409);
        assert_eq!(v["error"]["code"], "stale_rev");
        // A THIEF re-registering with their own owner_pk signs validly — and is refused,
        // because updates must be signed by the ALREADY-registered owner.
        let thief = sk(9);
        let mut stolen = listing_payload(&thief, &delegate, 5, 3);
        stolen["listing"] = json!("l1");
        let (st, _) = call(&app, "POST", "/v1/box/listing", stolen, Some(&thief), &[]).await;
        assert_eq!(st, 403);
    }

    /// The door: first scan admits; the second reports who scanned first, as a 200.
    #[tokio::test]
    async fn door_first_wins_and_reports() {
        let app = app_with_mock_rail();
        let (owner, delegate) = (sk(1), sk(2));
        call(
            &app,
            "POST",
            "/v1/box/listing",
            listing_payload(&owner, &delegate, 5, 1),
            Some(&owner),
            &[],
        )
        .await;

        let path = "/v1/box/door/l1/ticket1";
        let (st, _) = call(&app, "POST", path, serde_json::Value::Null, None, &[]).await;
        assert_eq!(st, 401, "the door is a signed surface");
        let (st, v) = call(
            &app,
            "POST",
            path,
            serde_json::Value::Null,
            Some(&delegate),
            &[],
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["admitted"], true);
        let (st, v) = call(
            &app,
            "POST",
            path,
            serde_json::Value::Null,
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(st, 200, "a double scan is a flag, never a rejection");
        assert_eq!(v["admitted"], false);
        assert_eq!(v["already"]["door_pk"].as_str().unwrap(), pk_hex(&delegate));
        let (st, v) = call(
            &app,
            "GET",
            path,
            serde_json::Value::Null,
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["redeemed"], true);
    }

    /// No rail: the free path serves in full; only the money routes answer 503.
    #[tokio::test]
    async fn free_path_survives_missing_rail_keys() {
        let app = app_without_rail();
        let owner = sk(1);
        // A FREE listing needs no acct/delegate and registers fine.
        let free = json!({
            "listing": "l1", "owner_pk": pk_hex(&owner), "unit_cents": 0, "currency": "usd",
            "capacity": 50, "open": true, "rev": 1,
        });
        let (st, _) = call(&app, "POST", "/v1/box/listing", free, Some(&owner), &[]).await;
        assert_eq!(st, 200);

        let (st, v) = call(
            &app,
            "GET",
            "/v1/box/health",
            serde_json::Value::Null,
            None,
            &[],
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["rail"], false, "health is honest about the missing rail");

        // The door still works — free events still have doors.
        let (st, v) = call(
            &app,
            "POST",
            "/v1/box/door/l1/t1",
            serde_json::Value::Null,
            Some(&owner),
            &[],
        )
        .await;
        assert_eq!(st, 200);
        assert_eq!(v["admitted"], true);

        // Priced checkout is the ONLY thing that dies, and it dies loudly.
        let priced = json!({
            "listing": "l2", "owner_pk": pk_hex(&owner), "delegate_pk": pk_hex(&sk(2)),
            "unit_cents": 500, "currency": "usd", "capacity": 5, "open": true,
            "acct": "acct_1", "rev": 1,
        });
        call(&app, "POST", "/v1/box/listing", priced, Some(&owner), &[]).await;
        let (st, v) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "l2", "qty": 1, "buyer_pk": pk_hex(&sk(3))}),
            None,
            &[],
        )
        .await;
        assert_eq!(st, 503);
        assert_eq!(v["error"]["code"], "rail_unconfigured");
    }

    /// Checkout guards: free listings have no checkout; a priced listing in a currency
    /// the FX table does not know is refused at REGISTRATION (loudly, per design §4).
    #[tokio::test]
    async fn checkout_guards() {
        let app = app_with_mock_rail();
        let owner = sk(1);
        let free = json!({
            "listing": "lf", "owner_pk": pk_hex(&owner), "unit_cents": 0, "currency": "usd",
            "capacity": 50, "open": true, "rev": 1,
        });
        call(&app, "POST", "/v1/box/listing", free, Some(&owner), &[]).await;
        let (st, v) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "lf", "qty": 1, "buyer_pk": pk_hex(&sk(3))}),
            None,
            &[],
        )
        .await;
        assert_eq!(st, 400);
        assert_eq!(v["error"]["code"], "free_listing");

        let (st, v) = call(
            &app,
            "POST",
            "/v1/box/checkout",
            json!({"listing": "nope", "qty": 1, "buyer_pk": pk_hex(&sk(3))}),
            None,
            &[],
        )
        .await;
        assert_eq!(st, 404);
        assert_eq!(v["error"]["code"], "unknown_listing");

        let weird = json!({
            "listing": "lx", "owner_pk": pk_hex(&owner), "delegate_pk": pk_hex(&sk(2)),
            "unit_cents": 500, "currency": "xxx", "capacity": 5, "open": true,
            "acct": "acct_1", "rev": 1,
        });
        let (st, v) = call(&app, "POST", "/v1/box/listing", weird, Some(&owner), &[]).await;
        assert_eq!(st, 422);
        assert_eq!(v["error"]["code"], "unknown_currency");

        // A priced listing missing acct/delegate is refused outright.
        let bare = json!({
            "listing": "lb", "owner_pk": pk_hex(&owner), "unit_cents": 500, "currency": "usd",
            "capacity": 5, "open": true, "rev": 1,
        });
        let (st, v) = call(&app, "POST", "/v1/box/listing", bare, Some(&owner), &[]).await;
        assert_eq!(st, 400);
        assert_eq!(v["error"]["code"], "invalid_listing");
    }
}
