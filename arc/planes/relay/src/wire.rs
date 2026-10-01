//! The relay's wire vocabulary — RE-EXPORTED from `pacific-wire`, never redefined.
//!
//! # Why this file defines nothing
//!
//! It used to hold a copy of the `Frame` enum, and the copy drifted. On 15 Sep 2026
//! the same protocol existed in three incompatible generations at once:
//!
//! | where | variants |
//! |---|---|
//! | `arc` git HEAD | 5 — `Pub Sub Msg Ack Eose` |
//! | `core/pacific-wire` (the client) | 6 — `+ Gap` |
//! | the `arc` working tree AND the live box | 10 — `+ MediaPut MediaGet MediaUrl MediaErr` |
//!
//! That is not a cosmetic split, and it is not historical. Reproduced against
//! `wss://arc.kenjin.cc/v1/relay` on 15 Sep 2026, on a connection that had sent no
//! `Sub` at all and had therefore declared nothing:
//!
//! ```text
//!   -> {"t":"media_get","key":"abab…ab"}
//!   <- {"t":"media_url","key":"abab…ab","method":"GET","url":"https://….r2.clou…
//!   -> {"t":"media_get","key":"ab"}
//!   <- {"t":"media_err","key":"ab","reason":"bad_key"}
//! ```
//!
//! Feed either reply to the client enum at `core` HEAD and it is "unknown variant" —
//! `Frame::from_json` fails, `core/pacific-core/src/transport.rs` maps that to
//! `CoreError::Transport`, and the sync drain DIES. A relay frame the client cannot
//! parse is an outage, not a warning.
//!
//! It survived because nothing ever compared the two enums. The relay's tests
//! asserted its `Frame` against ITSELF and the client's asserted its own against
//! ITSELF, so both suites were green the entire time the protocol was broken.
//!
//! The fix has two halves and this file is both of them:
//!
//! 1. **One definition.** [`Frame`] here IS [`pacific_wire::Frame`]. The relay and
//!    the client compile the same type; there is nothing left to drift.
//! 2. **A third party.** `pacific_wire::WIRE_SPEC_JSON` — the Semaphore wire spec
//!    that `docs/arc-api-icd-v1.html` §6 says is "specified separately" and which
//!    did not exist until now — states the vocabulary as data. This module tests
//!    the SERVER against it; `pacific-wire` tests the CLIENT against the same
//!    bytes. The assertions are generic over the frame type
//!    ([`pacific_wire::spec::WireFrame`]), so if anyone ever re-vendors a local
//!    enum here it is that enum the fixture measures.

pub use pacific_wire::{
    blob_b64, blob_unb64, tag_hex, tag_unhex, Dir, Frame, V_BASE, V_GAP, V_MAX, V_MEDIA,
};

/// The one write rule — a tag is a key, a `Pub` is its signature — re-exported, not
/// restated: the relay calls the same `verify_pub` the client's own tests call.
pub use pacific_wire::address;

/// May the relay send `frame` down a connection that declared vocabulary level `v`
/// (the `v` field of its `Sub`)?
///
/// THE SEND GATE. An unknown frame is fatal on the client, so this is the last
/// thing between a new relay capability and a dead drain on an old build. Silence
/// is the only safe alternative: there is no "sorry, you are too old" frame,
/// because that frame would itself be unknown.
pub fn may_send_to(frame: &Frame, v: u8) -> bool {
    frame.allowed_at(v)
}

/// May the relay ACT on an inbound `frame` from a connection at level `v`?
///
/// The same gate, for the same reason. Every answer the relay could give to a
/// request sits at that request's own level — `MediaPut` is answered with
/// `MediaUrl` or `MediaErr`, all three at level 2 — so servicing a request the
/// connection has not declared means replying with a frame it cannot read. A
/// request above the declared level is therefore dropped in silence and counted.
pub fn may_service(frame: &Frame, v: u8) -> bool {
    frame.allowed_at(v)
}

#[cfg(test)]
mod wire_spec {
    use super::*;
    use pacific_wire::spec;

    /// THE CROSS-REPO PIN, server side. `pacific-wire`'s `wire_spec` module runs
    /// this same assertion against the client, over the same fixture bytes.
    ///
    /// Byte-exact: every canonical encoding in the spec must parse AND re-serialize
    /// to itself, which pins the tag spellings, the field order and every
    /// `skip_serializing_if` default. A renamed field or a dropped default is a
    /// failure here, in the repo that would have shipped it.
    #[test]
    fn the_server_enum_matches_the_wire_spec() {
        spec::assert_matches_spec::<Frame>();
    }

    /// Nothing the relay can put on a socket is missing from the spec.
    #[test]
    fn the_fixture_names_every_variant() {
        spec::assert_fixture_names_every_variant::<Frame>();
    }

    /// The fixture's declared vocabulary levels agree with each frame's `min_v`.
    #[test]
    fn the_declared_levels_agree_with_min_v() {
        spec::assert_levels_agree::<Frame>();
    }

    /// The gate the relay actually calls agrees with the spec, frame by frame and
    /// level by level. This is the server-side half the client cannot check: it is
    /// about what the relay is permitted to EMIT, not about what parses.
    #[test]
    fn the_send_gate_agrees_with_the_spec() {
        for sf in spec::frames() {
            let frame = Frame::from_json(&sf.encodings[0]).expect("spec encoding parses");
            for v in 0..=V_MAX {
                let expected = v >= sf.min_v;
                assert_eq!(
                    may_send_to(&frame, v),
                    expected,
                    "{}: a client at v={v} must {} be sent this frame (min_v {})",
                    sf.t,
                    if expected { "" } else { "NOT" },
                    sf.min_v
                );
                assert_eq!(
                    may_service(&frame, v),
                    expected,
                    "{}: a request at v={v} must {} be serviced (min_v {})",
                    sf.t,
                    if expected { "" } else { "NOT" },
                    sf.min_v
                );
            }
        }
    }

    /// The concrete regression. `media_url` is exactly the frame that killed a live
    /// client: it must be unreachable for anything that has not declared level 2,
    /// and `media_get` — the request that provoked it — must be equally unreachable,
    /// because servicing it is what produces the `media_url`.
    #[test]
    fn a_pre_media_client_can_neither_provoke_nor_receive_a_media_frame() {
        let url = Frame::MediaUrl {
            key: "ab".into(),
            method: "GET".into(),
            url: "https://o/b/ab?X-Amz-Signature=deadbeef".into(),
            expires_in: 300,
        };
        let get = Frame::MediaGet { key: "ab".into() };
        for v in [V_BASE, V_GAP] {
            assert!(!may_send_to(&url, v), "media_url must not reach a v={v} client");
            assert!(!may_service(&get, v), "media_get must not be serviced at v={v}");
        }
        assert!(may_send_to(&url, V_MEDIA));
        assert!(may_service(&get, V_MEDIA));
    }

    /// Adding `Gap` and the media frames must not have changed one byte of what an
    /// unchanged client sends. `Sub.v` is omitted when 0, so a build that predates
    /// both levels is byte-identical on the wire to what it always was — and is
    /// therefore never handed a frame it cannot read.
    #[test]
    fn the_levels_are_invisible_to_a_client_that_did_not_opt_in() {
        assert_eq!(
            Frame::Sub { tags: vec!["aa".into()], since: 5, v: V_BASE }.to_json(),
            r#"{"t":"sub","tags":["aa"],"since":5}"#
        );
        assert_eq!(
            Frame::from_json(r#"{"t":"sub","tags":["aa"],"since":5}"#).unwrap(),
            Frame::Sub { tags: vec!["aa".into()], since: 5, v: V_BASE }
        );
    }

    /// The relay is the SERVER: it sends the `to_client` frames and receives the
    /// `to_relay` ones. Stated once, from the fixture, so the dispatch in `lib.rs`
    /// has something to be wrong against.
    #[test]
    fn the_spec_assigns_every_frame_a_side() {
        let (to_relay, to_client): (Vec<_>, Vec<_>) = spec::frames()
            .into_iter()
            .partition(|f| f.dir == Dir::ToRelay);
        let names = |v: &[spec::SpecFrame]| {
            let mut n: Vec<&str> = v.iter().map(|f| f.t.as_str()).collect();
            n.sort();
            n.join(",")
        };
        assert_eq!(names(&to_relay), "media_get,media_put,pub,sub");
        assert_eq!(names(&to_client), "ack,eose,gap,media_err,media_url,msg");
    }
}

#[cfg(test)]
mod codec_tests {
    use super::*;

    #[test]
    fn blob_and_tag_roundtrip() {
        let raw = b"\x00\x01\xfe\xff sealed bytes";
        assert_eq!(blob_unb64(&blob_b64(raw)).unwrap(), raw);
        let mut t = [0u8; 32];
        t[0] = 0xab;
        t[31] = 0xcd;
        assert_eq!(tag_hex(&t).len(), 64);
        assert!(tag_hex(&t).starts_with("ab"));
        assert_eq!(tag_unhex(&tag_hex(&t)).unwrap(), t);
    }
}
