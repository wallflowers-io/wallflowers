//! On-device filesystem layout, shared with the CLI.
//!
//! The state dir is `$PACIFIC_STATE_DIR` else `~/.pacific` — the same resolver the CLI's
//! `relay set` already uses, so the two never disagree about where state lives.

use std::path::PathBuf;

/// The on-device state directory: `$PACIFIC_STATE_DIR`, else `~/.pacific`, else `./.pacific`.
pub fn state_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("PACIFIC_STATE_DIR") {
        return PathBuf::from(d);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pacific")
}

/// The SQLite database (directory + delta log + MLS group/key-package storage).
pub fn db_path() -> PathBuf {
    state_dir().join("pacific.db")
}

/// The relay URL file (`relay set` writes it; pacific-core reads it). Superseded by
/// `routes_path` — read as a fallback so a device configured before the router still works.
pub fn relay_url_path() -> PathBuf {
    state_dir().join("relay_url")
}

/// The device's TRANSPORT SET — a comma-separated route spec (`wss://…,mesh:`), owned by the
/// core rather than passed in on every call. One file, one decision, one place to look when
/// messages are not arriving.
pub fn routes_path() -> PathBuf {
    state_dir().join("routes")
}

/// The canonical production relay — **Semaphore**, the BLIND PUBLIC TIER — reached
/// through the Arc's single public origin at `arc.wallflowers.io` (`arc.kenjin.cc`
/// until 19 Sep 2026, now retired below). TLS terminates at the
/// tunnel edge, so clients use `wss://`; the relay itself binds to loopback and is
/// never exposed directly.
///
/// It is a GATEWAY PLANE: the URL carries the `/v1/relay` path, not the host root.
/// relay / rail / waker were three public names; they are one origin now, because
/// the gateway already owned the `/v1` contract and three hostnames bought nothing
/// but three things to keep alive. Independent processes still — the gateway
/// reverse-proxies each — so failure and scale stay separate; only the front door
/// is shared.
///
/// It still holds nothing, gates nothing, and can read nothing — tags and blobs are
/// opaque to it (coordination/RINGFENCE.txt, "the two-tier split"). Consolidating the
/// front door does not move the ringfence: what the relay may KNOW is unchanged.
/// Default when no URL is configured on device; override with `relay set <ws-url>`.
pub const DEFAULT_RELAY_URL: &str = "wss://arc.wallflowers.io/v1/relay";

/// Relays this build has RETIRED. A device that persisted one of these is migrated
/// back to [`DEFAULT_RELAY_URL`] at launch — otherwise a stale on-device default
/// silently outranks the shipped one and messaging keeps talking to a dead host.
/// An object STAMPED with one routes through the device's own relay instead
/// (node.rs `routes_for`), because the stamp outlives any setting.
pub const RETIRED_RELAY_URLS: &[&str] = &[
    "wss://io.gopacific.ai",
    "wss://arc-production-0d8e.up.railway.app/v1/relay",
    // kenjin.ai is GONE — the domain is not ours and now resolves to someone
    // else's Cloudflare account. Devices shipped up to 0.1.0 (109) persisted this
    // as their relay, so leaving it in place would have them opening a wss://
    // session to a host we do not control. A blind relay cannot read a group, but
    // it arbitrates the commit slot (store.rs, first-writer-wins), so a hostile
    // one can withhold or reorder commits and fork a group. Retiring it is a
    // SECURITY migration, not tidying.
    "wss://relay.kenjin.ai",
    // The per-plane hostnames, superseded by the single `arc.kenjin.cc` origin.
    // Never shipped in a build, but a dev device may have persisted one.
    "wss://relay.kenjin.cc",
    // kenjin.cc is being given up as a domain (the site left it on 15 Sep 2026,
    // the Arc on 19 Sep). Today it still reaches the same Arc as
    // arc.wallflowers.io, which is exactly why it is retired NOW: moving every
    // device and every stamped object off it costs nothing while both names
    // answer, and it is the kenjin.ai exposure above if it waits until one
    // doesn't.
    "wss://arc.kenjin.cc/v1/relay",
];

/// Whether `url` is one of [`RETIRED_RELAY_URLS`] — the one comparison the app, the
/// FFI and the router all make, so they cannot disagree about which relays are dead.
/// Case-insensitive and blind to a trailing slash, as a persisted value may carry
/// either.
pub fn is_retired_relay_url(url: &str) -> bool {
    let u = url.trim().trim_end_matches('/');
    RETIRED_RELAY_URLS
        .iter()
        .any(|r| r.trim_end_matches('/').eq_ignore_ascii_case(u))
}

/// The Arc URL file (`arc set` writes it; pacific-core reads it). Mirrors the relay
/// resolver so the CLI and core never disagree about which Arc serves this device.
pub fn arc_url_path() -> PathBuf {
    state_dir().join("arc_url")
}

/// The default Arc — the federated LLM compute hub that serves inference when no
/// Arc is configured on device. Placeholder until the mesh directory selects the
/// closest one; override with `arc set <ws-url>`. (Server mirror: `arc::config::DEFAULT_ARC_URL`.)
// gopacific.ai is a DIFFERENT COMPANY's domain. This constant is stamped onto
// every object at creation (node.rs `object_new` -> `my_default_arc`) and is fed
// straight to `routes.with_relay()` by `routes_for`, so it is not merely an
// inference hint: it is the relay a group's traffic goes through. Leaving it
// pointed at infrastructure we do not control is the same exposure that retiring
// `relay.kenjin.ai` was a SECURITY migration for -- a relay arbitrates the commit
// slot, so a hostile one can withhold or reorder commits and fork a group.
//
// It moves WITH the relay default, every time: arc.kenjin.cc once, when this was
// the step that was missed, and arc.wallflowers.io on 19 Sep 2026.
pub const DEFAULT_ARC_URL: &str = "wss://arc.wallflowers.io/v1/relay";

/// The Ed25519 identity key file (0600).
pub fn id_key_path() -> PathBuf {
    state_dir().join("id_ed25519")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A default that is also retired would migrate every device onto the host it was
    /// just migrated off, once per launch, forever.
    #[test]
    fn no_default_is_retired() {
        assert!(!is_retired_relay_url(DEFAULT_RELAY_URL));
        assert!(!is_retired_relay_url(DEFAULT_ARC_URL));
    }

    #[test]
    fn the_kenjin_arc_is_retired_however_it_was_stored() {
        for stored in [
            "wss://arc.kenjin.cc/v1/relay",
            "wss://arc.kenjin.cc/v1/relay/",
            " WSS://ARC.KENJIN.CC/v1/relay ",
        ] {
            assert!(is_retired_relay_url(stored), "{stored:?} should be retired");
        }
        assert!(!is_retired_relay_url("wss://arc.wallflowers.io/v1/relay"));
    }
}
