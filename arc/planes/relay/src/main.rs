//! Semaphore — the Pacific blind relay binary. Bind a socket and run the mailbox.
//!
//! Reached in production at `arc.kenjin.cc`, through the Cloudflare tunnel on
//! kenjin-01 (TLS at the tunnel edge), so clients connect as `wss://`.
//! Bind addr from `RELAY_BIND`, else `0.0.0.0:$PORT`, else 8787.
//!
//! ## Telemetry
//!
//! - `tracing` fmt layer → stderr (so the container log carries it).
//! - `sentry-tracing` layer → Sentry spans/logs/breadcrumbs when `SENTRY_DSN` is set.
//! - Sentry panic integration captures per-connection task panics as issues without
//!   crashing the server.
//! - A 60s cron check-in (monitor slug `semaphore-heartbeat`) proves liveness.
//!
//! Privacy: Sentry runs with `send_default_pii = false`, so client IPs are never
//! attached. The relay never feeds tags/blobs/addresses into telemetry (see `lib.rs`).

use std::time::Duration;

use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// Monitor slug for the Sentry cron liveness check-in.
const HEARTBEAT_SLUG: &str = "semaphore-heartbeat";

fn main() {
    // PROVENANCE, first thing and before any side effect: log which commit this binary is,
    // or answer `--version` and exit. An unstamped build says so in words. See lib/arc-build.
    arc_build::stamp!();

    // Sentry must be initialised on the main thread BEFORE the async runtime so the
    // panic integration and client guard live for the whole process. The returned
    // guard must be held until shutdown, so we keep it in `main`'s scope.
    let _sentry_guard = init_sentry();

    // Now stand up the tracing subscriber: fmt → stderr, plus the Sentry layer.
    // INFO+ events flow to Sentry Logs (logs feature) once Sentry is initialised.
    init_tracing();

    if _sentry_guard.is_some() {
        info!("Sentry enabled");
    }

    // Build the runtime explicitly (instead of #[tokio::main]) so Sentry init and
    // the subscriber are established before any task spawns.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    rt.block_on(async move {
        let bind = std::env::var("RELAY_BIND").unwrap_or_else(|_| {
            let port = std::env::var("PORT").unwrap_or_else(|_| "8787".into());
            format!("0.0.0.0:{port}")
        });
        let listener = TcpListener::bind(&bind).await.expect("bind relay socket");
        // The bind addr is the relay's OWN listen address, not a client address.
        info!(addr = %bind, "semaphore listening");

        // Liveness heartbeat (no-op when Sentry is disabled).
        if _sentry_guard.is_some() {
            tokio::spawn(heartbeat_loop());
        }

        relay::serve(listener).await;
    });
}

/// Initialise Sentry from the environment. Returns the client guard when a DSN is
/// configured; `None` (stderr-only) otherwise. No silent fallback: if the DSN is
/// unset we emit a loud warning rather than quietly running blind.
fn init_sentry() -> Option<sentry::ClientInitGuard> {
    let dsn = match std::env::var("SENTRY_DSN") {
        Ok(d) if !d.trim().is_empty() => d,
        _ => {
            // `init_tracing` has not run yet, so this must reach the user directly.
            // It is intentionally loud: running prod with no error reporting is a
            // misconfiguration we refuse to hide.
            eprintln!(
                "WARN  Sentry disabled (SENTRY_DSN unset) — stderr only. \
                 Set SENTRY_DSN to enable error/log reporting."
            );
            return None;
        }
    };

    let environment =
        std::env::var("SENTRY_ENVIRONMENT").unwrap_or_else(|_| "development".into());

    let traces_sample_rate = std::env::var("SENTRY_TRACES_SAMPLE_RATE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);

    let guard = sentry::init((
        dsn,
        sentry::ClientOptions {
            release: Some(env!("CARGO_PKG_VERSION").into()),
            environment: Some(environment.into()),
            traces_sample_rate,
            // NEVER attach client IPs / request data.
            send_default_pii: false,
            // Send INFO+ tracing events to Sentry Logs (requires the `logs` feature).
            enable_logs: true,
            ..Default::default()
        },
    ));
    Some(guard)
}

/// Build the tracing subscriber: a fmt layer to stderr (so `docker logs` sees
/// human lines) plus the Sentry layer (spans/breadcrumbs/logs). `RUST_LOG` tunes the
/// filter; default is `info`.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        // When Sentry is uninitialised this layer is a harmless no-op.
        .with(sentry_tracing::layer())
        .init();
}

/// Send a Sentry cron check-in for the liveness monitor every 60s. Each check-in is
/// an `Ok` status with the upsert config so the monitor self-registers on Sentry.
/// Carries no routing/content data — just the slug and a fresh check-in id.
async fn heartbeat_loop() {
    use sentry::protocol::{
        EnvelopeItem, MonitorCheckIn, MonitorCheckInStatus, MonitorConfig, MonitorIntervalUnit,
        MonitorSchedule,
    };

    let environment =
        std::env::var("SENTRY_ENVIRONMENT").unwrap_or_else(|_| "development".into());

    let mut ticker = tokio::time::interval(Duration::from_secs(60));
    loop {
        ticker.tick().await;

        let check_in = MonitorCheckIn {
            check_in_id: sentry::types::random_uuid(),
            monitor_slug: HEARTBEAT_SLUG.to_string(),
            status: MonitorCheckInStatus::Ok,
            environment: Some(environment.clone()),
            duration: None,
            // Upsert the monitor on first beat: expect a check-in every 1 minute.
            monitor_config: Some(MonitorConfig {
                schedule: MonitorSchedule::Interval {
                    value: 1,
                    unit: MonitorIntervalUnit::Minute,
                },
                checkin_margin: Some(5),
                max_runtime: None,
                timezone: None,
                failure_issue_threshold: None,
                recovery_threshold: None,
            }),
        };

        if let Some(client) = sentry::Hub::current().client() {
            let mut envelope = sentry::protocol::Envelope::new();
            envelope.add_item(EnvelopeItem::from(check_in));
            client.send_envelope(envelope);
        } else {
            // Should not happen — heartbeat is only spawned when Sentry is enabled —
            // but if the client is gone, say so loudly instead of silently dropping.
            warn!("heartbeat skipped: no active Sentry client");
        }
    }
}
