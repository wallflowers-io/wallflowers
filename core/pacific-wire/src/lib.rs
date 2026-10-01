//! pacific-wire — the wire types SHARED by the relay and the client, so they cannot drift.
//!
//! The relay is a dumb mailbox keyed by an opaque rotating `tag`: a client PUBs a
//! sealed blob to a tag, and SUBs to the tags pointed at it. The relay never sees
//! identities, content, the MLS `group_id`, or the social graph — only opaque
//! tags + sealed blobs.
//!
//! v0 transport encoding is JSON text frames (trivially debuggable; a CBOR/binary
//! framing is a later optimisation). By convention `tag` is hex(32 bytes) and
//! `blob` is base64 — but both are OPAQUE to the relay, which never decodes them.
//!
//! # There is exactly ONE `Frame`
//!
//! This one. `arc/planes/relay/src/wire.rs` re-exports it and defines nothing; the
//! server and the client compile the same enum. That is not tidiness, it is the fix
//! for a live outage: the relay had vendored its own copy, the copies grew apart
//! into three incompatible generations (5 variants at arc HEAD, 6 here, 10 on the
//! deployed box), and an unknown variant is FATAL on the client —
//! [`Frame::from_json`] failing inside `RelaySession::next_frame` becomes a
//! `CoreError::Transport` and the sync drain dies. Asking the live relay for
//! `media_get` really did kill a client that had never heard of `media_url`.
//!
//! Two things keep it one enum:
//!   * nothing else may define a relay `Frame` — re-export this;
//!   * [`WIRE_SPEC_JSON`] is a third party neither side controls, and BOTH repos
//!     test against it ([`Frame::wire_tag`], [`Frame::dir`] and [`Frame::min_v`]
//!     are exhaustive matches, so a new variant does not compile until it is named
//!     in all three — and then fails the fixture tests until the spec names it too).

use serde::{Deserialize, Serialize};

/// The Semaphore wire spec: the frame vocabulary as data, pinned by tests in BOTH
/// repos (`pacific-wire`'s `wire_spec` module and `relay::wire`'s). This is the
/// document `arc/docs/arc-api-icd-v1.html` §6 means by "the wire protocol is
/// specified separately (Semaphore wire spec)".
///
/// It travels inside the crate rather than as a sibling path so the relay reaches
/// it through the dependency it already has, whether that resolves to the local
/// path patch or the pinned git rev — no assumption about repos being checked out
/// next to each other.
pub const WIRE_SPEC_JSON: &str = include_str!("../wire-spec.v1.json");

/// Vocabulary level 0: `Pub`, `Sub`, `Msg`, `Ack`, `Eose`. Every build ever shipped.
///
/// A client declares its level in [`Frame::Sub`]`::v`, and the relay may send it
/// nothing above that. See [`Frame::min_v`].
pub const V_BASE: u8 = 0;
/// Vocabulary level 1: adds [`Frame::Gap`] — the replayed backlog may be incomplete.
pub const V_GAP: u8 = 1;
/// Vocabulary level 2: adds the four media frames (presigned object-storage offload).
///
/// No shipping client declares this yet, which is exactly why it is a level: the
/// media frames are deployed and server-only, so until a client asks for them by
/// sending `Sub { v: 2 }` the relay must never emit one.
pub const V_MEDIA: u8 = 2;
/// The highest vocabulary level this build knows how to parse. A client may send
/// this as `Sub.v` to opt into everything; it is NOT the default, because opting in
/// to a frame means deciding what to DO with it.
pub const V_MAX: u8 = V_MEDIA;

/// Which way a frame travels. Used to state the contract in one place: a relay
/// never sends a [`Dir::ToRelay`] frame, and ignores one it receives in reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// client → relay: a request.
    ToRelay,
    /// relay → client: a delivery or a verdict.
    ToClient,
}

impl Dir {
    /// The spelling used in [`WIRE_SPEC_JSON`].
    pub fn as_str(self) -> &'static str {
        match self {
            Dir::ToRelay => "to_relay",
            Dir::ToClient => "to_client",
        }
    }
}

/// A relay protocol frame. Encoded as tagged JSON, e.g.
/// `{"t":"pub","tag":"ab12…","blob":"…"}`.
///
/// **Adding a variant is a wire change.** It must be named in [`Frame::wire_tag`],
/// [`Frame::dir`], [`Frame::min_v`], [`Frame::TAGS`] and `wire-spec.v1.json`, and
/// it must sit at a `min_v` above every level a shipped client declares — otherwise
/// it reaches a build that cannot parse it and kills that build's drain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Frame {
    /// client→relay: store this blob under `tag` and deliver it to subscribers.
    ///
    /// `commit: true` claims the tag's single COMMIT SLOT (the blind sequencer:
    /// RFC 9750 §5.2.1's "ordering server" needs zero content visibility — the
    /// relay orders opaque blobs by arrival). Exactly one commit-flagged blob is
    /// accepted per tag; since Pacific derives one tag per (group, epoch), that
    /// is one MLS commit per epoch, first-writer-wins. Losers get `Ack{ok:false}`
    /// and MUST rebase onto the winning commit. The flag is the ONLY thing the
    /// relay learns (a blob is commit-shaped) — never content, sender or group.
    Pub {
        tag: String,
        blob: String,
        #[serde(default, skip_serializing_if = "is_false")]
        commit: bool,
        /// The signature by the key this tag IS, over [`address::pub_payload`], hex.
        /// See [`address`]: a party may publish only to an address whose secret it
        /// holds. Empty on the wire from a client that predates it, which a relay
        /// that checks refuses with `malformed_sig`.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        sig: String,
    },
    /// client→relay: subscribe to these tags; replay stored blobs with `seq > since`,
    /// then stream live. `since = 0` replays everything.
    Sub {
        tags: Vec<String>,
        #[serde(default)]
        since: u64,
        /// This client's frame-vocabulary version, so the relay knows what it is
        /// safe to send back. It matters because an unknown frame is FATAL here:
        /// [`crate::Frame::from_json`] failing in `RelaySession::next_frame` becomes
        /// a `CoreError::Transport` and the drain dies. A relay that learned a new
        /// relay→client frame therefore cannot just start sending it.
        ///   v = 0 (absent) — Msg/Ack/Eose only.
        ///   v >= 1         — may also receive `Gap`.
        ///   v >= 2         — may also send/receive the media frames.
        /// Omitted when 0, so an unchanged client is byte-identical on the wire and
        /// an older relay (which ignores unknown fields) is unaffected either way.
        ///
        /// It is a property of the BUILD, not of one subscribe: a client cannot
        /// un-learn a variant mid-connection, so the relay takes the high-water
        /// mark of what a connection has declared.
        #[serde(default, skip_serializing_if = "is_zero")]
        v: u8,
    },
    /// relay→client: a stored or live blob on `tag`, with its monotonic `seq`.
    Msg { tag: String, seq: u64, blob: String },
    /// relay→client: the verdict on a `Pub`. `ok:false` means the blob was NOT
    /// stored and NOT fanned out. `ok` defaults true when absent so pre-slot relays
    /// keep parsing.
    ///
    /// `reason` says why, as a stable token, and is absent in exactly one refusal:
    /// a commit that lost its tag's slot, which is not an error but an instruction
    /// to rebase — so a client that predates `reason` reads it exactly as before.
    /// Every other refusal names itself: `malformed_tag`, `not_a_key`,
    /// `malformed_sig`, `bad_signature` (see [`address::PubRefusal`]), `too_large`,
    /// `rate_limited`, `busy`, `full`. Level 0: a field an older client does not
    /// know is ignored, so it cannot kill a drain the way a new variant would.
    Ack {
        seq: u64,
        #[serde(default = "ack_ok_default", skip_serializing_if = "is_true")]
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// relay→client: the backlog just replayed for `tag` is INCOMPLETE — blobs at
    /// or below `floor` were evicted before this `Sub` arrived.
    ///
    /// The relay is blind: it cannot know whether a subscriber drained, so
    /// retention can only ever be by age. That makes a hole in a long-offline
    /// device's history POSSIBLE — and without this frame it would also be
    /// INVISIBLE, because `Sub` replays `seq > since` whether or not anything
    /// below `since` still exists. This is the difference between a bounded,
    /// announced loss and a silent one.
    ///
    /// Only ever sent to a `Sub { v >= 1 }`. Parsing it is safe on any build that
    /// has this variant; ACTING on it (surfacing the hole to the user) is a
    /// separate, deliberate step.
    Gap { tag: String, floor: u64 },
    /// client→relay: presign an UPLOAD of `key`, exactly `len` bytes.
    ///
    /// Media does not travel through the relay. The client uploads ciphertext
    /// straight to object storage with the returned URL and then `Pub`s an
    /// ordinary sealed packet naming the key, so the relay stays blind and never
    /// pays for a byte of it (see the relay's `media.rs`).
    ///
    /// `len` is not advisory: it is signed into the URL, so the upload must be
    /// exactly this size or the store rejects it. That is what stops a presigned
    /// PUT from being a blank cheque.
    ///
    /// Requires `v >= 2` — see [`Frame::min_v`]. A relay that receives this on a
    /// connection which has not declared level 2 DROPS IT IN SILENCE, because
    /// every answer it could give (`MediaUrl`, `MediaErr`) is itself level 2 and
    /// would be the fatal unknown frame.
    MediaPut { key: String, len: u64 },
    /// client→relay: presign a DOWNLOAD of `key`. Requires `v >= 2`.
    ///
    /// The relay does not check existence — a blind relay has no business
    /// tracking which keys are live, and the client finds out on fetch anyway.
    MediaGet { key: String },
    /// relay→client: a short-lived presigned URL for `key`. `method` is the verb
    /// the URL is signed for ("PUT" or "GET"); using it with any other verb fails
    /// the signature. Only ever sent to a `Sub { v >= 2 }`.
    ///
    /// A `MediaPut` URL is only valid for a request carrying exactly the
    /// `Content-Length` that was requested.
    MediaUrl {
        key: String,
        method: String,
        url: String,
        expires_in: u32,
    },
    /// relay→client: the presign was refused. `reason` is a stable token —
    /// `unconfigured` (relay has no object storage), `bad_key` (not opaque
    /// 32-byte hex), `too_large` (over the relay's cap), `over_budget` or
    /// `budget_stale`. Only ever sent to a `Sub { v >= 2 }`.
    MediaErr { key: String, reason: String },
    /// relay→client: end of the stored backlog for the current `Sub`; live follows.
    Eose,
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(v: &u8) -> bool {
    *v == 0
}
fn is_true(b: &bool) -> bool {
    *b
}
fn ack_ok_default() -> bool {
    true
}

impl Frame {
    /// Every frame tag in the vocabulary, in spec order. Held to the enum by the
    /// exhaustive match in [`Frame::wire_tag`] and to the fixture by
    /// `the_fixture_names_every_variant`.
    pub const TAGS: &'static [&'static str] = &[
        "pub",
        "sub",
        "msg",
        "ack",
        "gap",
        "eose",
        "media_put",
        "media_get",
        "media_url",
        "media_err",
    ];

    /// Serialize to the on-wire JSON text.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("Frame always serializes")
    }

    /// Parse from on-wire JSON text.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// The `t` discriminant this variant encodes as.
    ///
    /// EXHAUSTIVE ON PURPOSE: a new `Frame` variant does not compile until it is
    /// named here, which is the moment to notice it is a wire change.
    pub fn wire_tag(&self) -> &'static str {
        match self {
            Frame::Pub { .. } => "pub",
            Frame::Sub { .. } => "sub",
            Frame::Msg { .. } => "msg",
            Frame::Ack { .. } => "ack",
            Frame::Gap { .. } => "gap",
            Frame::Eose => "eose",
            Frame::MediaPut { .. } => "media_put",
            Frame::MediaGet { .. } => "media_get",
            Frame::MediaUrl { .. } => "media_url",
            Frame::MediaErr { .. } => "media_err",
        }
    }

    /// Which way this frame travels. Exhaustive for the same reason as
    /// [`Frame::wire_tag`].
    pub fn dir(&self) -> Dir {
        match self {
            Frame::Pub { .. }
            | Frame::Sub { .. }
            | Frame::MediaPut { .. }
            | Frame::MediaGet { .. } => Dir::ToRelay,
            Frame::Msg { .. }
            | Frame::Ack { .. }
            | Frame::Gap { .. }
            | Frame::MediaUrl { .. }
            | Frame::MediaErr { .. }
            | Frame::Eose => Dir::ToClient,
        }
    }

    /// The minimum vocabulary level a connection must have declared (`Sub.v`)
    /// before this frame may cross it — in EITHER direction.
    ///
    /// For a relay→client frame this is a hard send gate: above the declared level
    /// the client cannot parse it and the drain dies, so the relay stays silent.
    /// For a client→relay frame it is the matching act gate: the relay will not
    /// service a request it cannot answer, because every answer is at the same
    /// level as the request.
    ///
    /// Exhaustive for the same reason as [`Frame::wire_tag`]: a new variant must
    /// be given a level deliberately, and it must be a level above everything a
    /// shipped client declares.
    pub fn min_v(&self) -> u8 {
        match self {
            Frame::Pub { .. }
            | Frame::Sub { .. }
            | Frame::Msg { .. }
            | Frame::Ack { .. }
            | Frame::Eose => V_BASE,
            Frame::Gap { .. } => V_GAP,
            Frame::MediaPut { .. }
            | Frame::MediaGet { .. }
            | Frame::MediaUrl { .. }
            | Frame::MediaErr { .. } => V_MEDIA,
        }
    }

    /// May a connection that declared `v` be sent (or serviced for) this frame?
    pub fn allowed_at(&self, v: u8) -> bool {
        v >= self.min_v()
    }
}

// --- field codecs (tags + blobs are opaque to the relay) --------------------
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

/// Encode raw bytes for the base64 `blob` field of a `Frame`.
pub fn blob_b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

/// Decode a `blob` field back to raw bytes.
pub fn blob_unb64(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    B64.decode(s)
}

/// Encode a 32-byte routing tag as the hex `tag` field of a `Frame`.
pub fn tag_hex(t: &[u8; 32]) -> String {
    hex::encode(t)
}

/// Decode a hex `tag` field back to the 32 bytes it addresses. The relay never needs this — it
/// treats a tag as an opaque string — but a MESH peer does: it stores and dedupes by tag, so it
/// has to hold the real bytes rather than one of the two hex spellings of them.
pub fn tag_unhex(s: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(s).map_err(|e| e.to_string())?;
    raw.try_into()
        .map_err(|_| format!("tag must be 32 bytes, got {}", s.len() / 2))
}

/// The wire spec as parsed data, so BOTH repos can hold their enum to the same
/// fixture without each writing a JSON reader.
///
/// This is deliberately a hand-rolled reader over `serde_json::Value` rather than
/// a `Deserialize` struct: the fixture is the authority, and a typed struct that
/// silently ignores a field it does not know is how a spec quietly stops being one.
pub mod spec {
    use super::{Dir, Frame, WIRE_SPEC_JSON};

    /// What [`assert_matches_spec`] needs of a relay frame type.
    ///
    /// It is a TRAIT, and that is the whole point of this module: the assertions
    /// then run against *whatever type the calling crate actually puts on its
    /// socket*, not against [`Frame`] by name. If the relay ever re-vendors its own
    /// enum — the thing that produced three incompatible generations at once — it
    /// has to implement this for the vendored type, and the fixture catches the
    /// divergence on the first byte that differs.
    pub trait WireFrame: Sized + std::fmt::Debug {
        /// Every tag this type can encode as.
        const TAGS: &'static [&'static str];
        /// Parse on-wire JSON text; `Err` carries a human-readable reason.
        fn parse(s: &str) -> Result<Self, String>;
        /// Encode to on-wire JSON text.
        fn encode(&self) -> String;
        /// The `t` discriminant.
        fn wire_tag(&self) -> &'static str;
        /// Which way it travels.
        fn dir(&self) -> Dir;
        /// The vocabulary level it sits at.
        fn min_v(&self) -> u8;
    }

    impl WireFrame for Frame {
        const TAGS: &'static [&'static str] = Frame::TAGS;
        fn parse(s: &str) -> Result<Self, String> {
            Frame::from_json(s).map_err(|e| e.to_string())
        }
        fn encode(&self) -> String {
            self.to_json()
        }
        fn wire_tag(&self) -> &'static str {
            Frame::wire_tag(self)
        }
        fn dir(&self) -> Dir {
            Frame::dir(self)
        }
        fn min_v(&self) -> u8 {
            Frame::min_v(self)
        }
    }

    /// One frame's entry in the spec.
    #[derive(Debug, Clone)]
    pub struct SpecFrame {
        /// The `t` discriminant.
        pub t: String,
        /// Which way it travels.
        pub dir: Dir,
        /// The vocabulary level it belongs to.
        pub min_v: u8,
        /// Byte-exact encodings: each must parse AND re-serialize to itself.
        pub encodings: Vec<String>,
        /// Inputs that must parse but are not canonical output (back-compat,
        /// omitted defaults, explicit defaults).
        pub accepts: Vec<String>,
    }

    /// Parse [`WIRE_SPEC_JSON`]. Panics with a precise message if the fixture is
    /// malformed — it is a checked-in constant, so a failure here is a broken
    /// build, not a runtime condition.
    pub fn frames() -> Vec<SpecFrame> {
        let v: serde_json::Value =
            serde_json::from_str(WIRE_SPEC_JSON).expect("wire-spec.v1.json is valid JSON");
        assert_eq!(
            v["spec"].as_str(),
            Some("semaphore-wire"),
            "wire-spec.v1.json is not the Semaphore wire spec"
        );
        assert_eq!(
            v["discriminant"].as_str(),
            Some("t"),
            "the spec's discriminant must match #[serde(tag = \"t\")]"
        );
        let strings = |node: &serde_json::Value| -> Vec<String> {
            node.as_array()
                .map(|a| {
                    a.iter()
                        .map(|s| {
                            s.as_str()
                                .expect("encodings/accepts are JSON strings")
                                .to_string()
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        v["frames"]
            .as_array()
            .expect("spec has a frames array")
            .iter()
            .map(|f| SpecFrame {
                t: f["t"].as_str().expect("frame has t").to_string(),
                dir: match f["dir"].as_str().expect("frame has dir") {
                    "to_relay" => Dir::ToRelay,
                    "to_client" => Dir::ToClient,
                    other => panic!("unknown dir {other:?} in the wire spec"),
                },
                min_v: f["min_v"].as_u64().expect("frame has min_v") as u8,
                encodings: strings(&f["encodings"]),
                accepts: strings(&f["accepts"]),
            })
            .collect()
    }

    /// Hold a frame type to the spec. Called from BOTH repos — this crate's tests
    /// for the client side, `arc/planes/relay`'s for the server side — so the
    /// fixture, not either enum, is the authority both are measured against.
    ///
    /// Checks, per spec frame: every canonical encoding parses, reports the spec's
    /// `t`/`dir`/`min_v`, and re-serializes BYTE-IDENTICALLY (which pins the
    /// `skip_serializing_if` defaults, the field order and the tag spellings); and
    /// every `accepts` input parses to the same variant. Then checks that the spec
    /// and `F::TAGS` name exactly the same set of frames.
    pub fn assert_matches_spec<F: WireFrame>() {
        let spec = frames();
        let mut seen: Vec<String> = Vec::new();
        for sf in &spec {
            assert!(
                !sf.encodings.is_empty(),
                "spec frame {:?} has no canonical encoding to pin",
                sf.t
            );
            for enc in &sf.encodings {
                let f = F::parse(enc)
                    .unwrap_or_else(|e| panic!("spec encoding {enc} does not parse: {e}"));
                assert_eq!(f.wire_tag(), sf.t, "wrong tag for {enc}");
                assert_eq!(f.dir(), sf.dir, "wrong direction for {enc}");
                assert_eq!(f.min_v(), sf.min_v, "wrong vocabulary level for {enc}");
                assert_eq!(
                    &f.encode(),
                    enc,
                    "{} does not re-serialize to its canonical spec encoding",
                    sf.t
                );
            }
            for acc in &sf.accepts {
                let f = F::parse(acc)
                    .unwrap_or_else(|e| panic!("spec-accepted input {acc} does not parse: {e}"));
                assert_eq!(f.wire_tag(), sf.t, "wrong tag for accepted input {acc}");
            }
            seen.push(sf.t.clone());
        }
        let mut expected: Vec<String> = F::TAGS.iter().map(|s| s.to_string()).collect();
        let mut got = seen;
        expected.sort();
        got.sort();
        assert_eq!(
            got, expected,
            "the wire spec and the frame type name different frames — one was changed alone"
        );
    }

    /// The spec accounts for every tag `F` can produce, and for no tag it cannot.
    /// Adding a variant is a compile error in the exhaustive matches first; this is
    /// what stops someone adding the arms and shipping without writing it down.
    pub fn assert_fixture_names_every_variant<F: WireFrame>() {
        let spec = frames();
        assert_eq!(
            spec.len(),
            F::TAGS.len(),
            "wire-spec.v1.json describes {} frames but the frame type has {}",
            spec.len(),
            F::TAGS.len()
        );
        for tag in F::TAGS.iter().copied() {
            assert!(
                spec.iter().any(|f| f.t == tag),
                "frame variant {tag:?} is not in wire-spec.v1.json — a wire change nobody wrote down"
            );
        }
    }

    /// The vocabulary levels declared in the fixture's prose agree with the `min_v`
    /// on every frame entry. Two places in the fixture say what level a frame is
    /// at; they must not disagree, and every frame must belong to exactly one.
    pub fn assert_levels_agree<F: WireFrame>() {
        let raw: serde_json::Value = serde_json::from_str(WIRE_SPEC_JSON).unwrap();
        let spec = frames();
        let levels = raw["vocabulary"]["levels"]
            .as_array()
            .expect("spec declares vocabulary levels");
        let mut counted = 0usize;
        for level in levels {
            let v = level["v"].as_u64().expect("level has v") as u8;
            for added in level["adds"].as_array().expect("level adds frames") {
                let t = added.as_str().expect("added frame is a string");
                let sf = spec
                    .iter()
                    .find(|f| f.t == t)
                    .unwrap_or_else(|| panic!("level {v} adds {t:?}, which has no frame entry"));
                assert_eq!(
                    sf.min_v, v,
                    "{t:?} is declared at level {v} but its entry says min_v {}",
                    sf.min_v
                );
                counted += 1;
            }
        }
        assert_eq!(
            counted,
            F::TAGS.len(),
            "a frame belongs to no declared vocabulary level"
        );
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        for f in [
            Frame::Pub { tag: "aa".into(), blob: "Zm9v".into(), commit: false, sig: String::new() },
            Frame::Pub { tag: "aa".into(), blob: "Zm9v".into(), commit: true, sig: String::new() },
            Frame::Sub { tags: vec!["aa".into(), "bb".into()], since: 7, v: 0 },
            Frame::Sub { tags: vec!["aa".into()], since: 7, v: 1 },
            Frame::Msg { tag: "aa".into(), seq: 3, blob: "YmFy".into() },
            Frame::Ack { seq: 3, ok: true, reason: None },
            Frame::Ack { seq: 0, ok: false, reason: None },
            Frame::Gap { tag: "aa".into(), floor: 42 },
            Frame::MediaPut { key: "ab".into(), len: 1024 },
            Frame::MediaGet { key: "ab".into() },
            Frame::MediaUrl {
                key: "ab".into(),
                method: "PUT".into(),
                url: "https://o/b/ab?X-Amz-Signature=deadbeef".into(),
                expires_in: 300,
            },
            Frame::MediaErr { key: "ab".into(), reason: "too_large".into() },
            Frame::Eose,
        ] {
            assert_eq!(Frame::from_json(&f.to_json()).unwrap(), f);
        }
    }

    #[test]
    fn sub_since_defaults_to_zero() {
        let f = Frame::from_json(r#"{"t":"sub","tags":["aa"]}"#).unwrap();
        assert_eq!(f, Frame::Sub { tags: vec!["aa".into()], since: 0, v: 0 });
    }

    /// The retention vocabulary must be invisible to a pre-retention client. A
    /// `Sub` from an old build parses with `v = 0`, and a `v = 0` Sub encodes
    /// byte-identically to what those builds already send — so nothing about
    /// adding `Gap` changes a single byte on an existing device's wire.
    #[test]
    fn gap_vocabulary_is_opt_in() {
        let f = Frame::from_json(r#"{"t":"sub","tags":["aa"],"since":5}"#).unwrap();
        assert_eq!(f, Frame::Sub { tags: vec!["aa".into()], since: 5, v: 0 });
        assert_eq!(
            Frame::Sub { tags: vec!["aa".into()], since: 5, v: 0 }.to_json(),
            r#"{"t":"sub","tags":["aa"],"since":5}"#
        );
        assert_eq!(
            Frame::Sub { tags: vec!["aa".into()], since: 5, v: 1 }.to_json(),
            r#"{"t":"sub","tags":["aa"],"since":5,"v":1}"#
        );
        assert_eq!(
            Frame::Gap { tag: "aa".into(), floor: 7 }.to_json(),
            r#"{"t":"gap","tag":"aa","floor":7}"#
        );
    }

    /// The media vocabulary is a LEVEL, not a request-gated extra.
    ///
    /// It was argued (in the relay's vendored copy of this enum) that media needed
    /// no `v` bump because every media frame is a reply to a media request, so a
    /// pre-media client could never be handed one. That argument is wrong in the
    /// only way that matters: it assumes the only thing that can send `media_get`
    /// on a connection is a build that can read `media_url`. Anything that speaks
    /// the protocol can — and when it did, against the live relay, the `media_url`
    /// that came back was an unknown variant to the client enum, became
    /// `CoreError::Transport`, and killed the drain.
    ///
    /// So media sits at `v >= 2`, gated exactly like `Gap` at `v >= 1`: the relay
    /// may not send a media frame to a connection that has not declared level 2,
    /// and may not act on a media request from one either — there is no answer it
    /// could give that the client could read.
    #[test]
    fn media_vocabulary_is_level_two() {
        // Pre-media clients send Sub/Pub only; nothing here changes their bytes.
        assert_eq!(
            Frame::Sub { tags: vec!["aa".into()], since: 5, v: 0 }.to_json(),
            r#"{"t":"sub","tags":["aa"],"since":5}"#
        );
        // Declaring level 2 is a single extra field.
        assert_eq!(
            Frame::Sub { tags: vec!["aa".into()], since: 5, v: V_MEDIA }.to_json(),
            r#"{"t":"sub","tags":["aa"],"since":5,"v":2}"#
        );
        // Every media frame — in BOTH directions — sits at level 2.
        for f in [
            Frame::MediaPut { key: "ab".into(), len: 7 },
            Frame::MediaGet { key: "ab".into() },
            Frame::MediaUrl {
                key: "ab".into(),
                method: "GET".into(),
                url: "https://o/b/ab?X-Amz-Signature=deadbeef".into(),
                expires_in: 300,
            },
            Frame::MediaErr { key: "ab".into(), reason: "bad_key".into() },
        ] {
            assert_eq!(f.min_v(), V_MEDIA, "{} must be level 2", f.wire_tag());
            assert!(!f.allowed_at(V_BASE), "{} must not reach v0", f.wire_tag());
            assert!(!f.allowed_at(V_GAP), "{} must not reach v1", f.wire_tag());
            assert!(f.allowed_at(V_MEDIA));
        }
        // And nothing a v0 client already speaks was pushed up a level.
        for f in [
            Frame::Pub { tag: "aa".into(), blob: "Zm9v".into(), commit: false, sig: String::new() },
            Frame::Sub { tags: vec!["aa".into()], since: 0, v: 0 },
            Frame::Msg { tag: "aa".into(), seq: 1, blob: "Zm9v".into() },
            Frame::Ack { seq: 1, ok: true, reason: None },
            Frame::Eose,
        ] {
            assert!(f.allowed_at(V_BASE), "{} must stay at v0", f.wire_tag());
        }
        assert_eq!(Frame::Gap { tag: "aa".into(), floor: 1 }.min_v(), V_GAP);
    }

    /// THE OTHER HALF OF THE FIX — the half the send gate cannot deliver.
    ///
    /// Gating lives in the relay, so it protects a client only once the relay
    /// carrying it is DEPLOYED. The box serving `wss://arc.kenjin.cc` on 15 Sep
    /// 2026 was not carrying it: it answered a `media_get` from a connection that
    /// had declared nothing with a `media_url`, the client enum of the day had
    /// never heard of that variant, `from_json` failed, `transport.rs` mapped that
    /// to `CoreError::Transport`, and the sync drain died.
    ///
    /// A client therefore has to survive an UNGATED relay too, and having the
    /// variant is what makes that possible: these frames now parse, and each
    /// reports a `min_v` above the `v` the drain declares — which is exactly the
    /// fact a client needs in order to DROP one instead of dying on it.
    ///
    /// The shapes below are the ones that capture returned. The key and the signed
    /// URL are stand-ins, not the captured bytes: what is pinned here is the
    /// vocabulary, not one afternoon's presign.
    #[test]
    fn an_ungated_relay_can_no_longer_kill_the_drain() {
        let key = "ab".repeat(32);
        // What `RelaySession::drain` declares in `Sub.v` today.
        let declared = V_BASE;
        for text in [
            format!(
                r#"{{"t":"media_url","key":"{key}","method":"GET","url":"https://acct.r2.cloudflarestorage.com/m/{key}?X-Amz-Signature=deadbeef","expires_in":300}}"#
            ),
            format!(
                r#"{{"t":"media_url","key":"{key}","method":"PUT","url":"https://acct.r2.cloudflarestorage.com/m/{key}?X-Amz-Signature=deadbeef","expires_in":300}}"#
            ),
            String::from(r#"{"t":"media_err","key":"ab","reason":"bad_key"}"#),
        ] {
            let f = Frame::from_json(&text).unwrap_or_else(|e| {
                panic!("an ungated relay's {text} must PARSE, not kill the drain: {e}")
            });
            assert_eq!(f.dir(), Dir::ToClient);
            assert!(
                !f.allowed_at(declared),
                "{} arrived above the declared vocabulary: it must be droppable, not fatal",
                f.wire_tag()
            );
        }
    }

    /// Back-compat: pre-slot frames parse with the new defaults, and plain
    /// frames serialize byte-identically to the pre-slot encoding (a plain Pub
    /// carries no `commit` field; an ok Ack carries no `ok` field).
    #[test]
    fn slot_fields_are_backward_compatible() {
        let f = Frame::from_json(r#"{"t":"pub","tag":"aa","blob":"Zm9v"}"#).unwrap();
        assert_eq!(f, Frame::Pub { tag: "aa".into(), blob: "Zm9v".into(), commit: false, sig: String::new() });
        let f = Frame::from_json(r#"{"t":"ack","seq":9}"#).unwrap();
        assert_eq!(f, Frame::Ack { seq: 9, ok: true, reason: None });
        assert_eq!(
            Frame::Pub { tag: "aa".into(), blob: "Zm9v".into(), commit: false, sig: String::new() }.to_json(),
            r#"{"t":"pub","tag":"aa","blob":"Zm9v"}"#
        );
        assert_eq!(Frame::Ack { seq: 9, ok: true, reason: None }.to_json(), r#"{"t":"ack","seq":9}"#);
    }
}

/// THE CROSS-REPO PIN, client side.
///
/// `arc/planes/relay/src/wire.rs` has the twin of this module and runs the same
/// three assertions against the same fixture from the other repo, over the type the
/// relay actually puts on its socket. Before this existed, the relay's 44 tests
/// asserted its enum against itself and this crate's asserted its own against
/// itself — which is how three incompatible generations of the same enum shipped at
/// once without a single red test.
#[cfg(test)]
mod wire_spec {
    use super::*;

    /// The client enum encodes and parses exactly what the spec says, byte for byte.
    #[test]
    fn the_client_enum_matches_the_wire_spec() {
        spec::assert_matches_spec::<Frame>();
    }

    /// The fixture accounts for every variant this enum has.
    #[test]
    fn the_fixture_names_every_variant() {
        spec::assert_fixture_names_every_variant::<Frame>();
    }

    /// The vocabulary levels declared in the fixture agree with each frame's `min_v`,
    /// and the constants in this crate name the same top level.
    #[test]
    fn the_declared_levels_agree_with_min_v() {
        spec::assert_levels_agree::<Frame>();
        assert_eq!(V_MAX, V_MEDIA, "V_MAX must name the highest level in the spec");
    }
}

/// The Arc↔client stream: ONE object owning the tunnel on both ends (sans-io).
pub mod tunnel;
pub mod address;
