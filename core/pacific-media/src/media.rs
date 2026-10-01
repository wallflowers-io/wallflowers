//! MEDIA — the one module every media producer and consumer routes through.
//!
//! Before this existed, a photo was an untyped `(String /*b64*/, String /*mime*/)`
//! pair re-declared on ContactCard, Thing, Group, Event and Post, with its size
//! cap re-checked at eighteen sites against seven constants that did not agree
//! with each other — `MAX_BANNER_B64` was 716,800 in `post` and 700,000 in
//! `event`, and `thing`/`contact`/`group` all reached into `event` for a cap
//! that has nothing to do with events. The comment at the `contact` fold even
//! said "one cap, one place" while doing the opposite.
//!
//! # The split that makes livestreams representable
//!
//! Images, gifs, clips and livestreams do not sit at one layer, and pretending
//! they do is what breaks this design. A still fits in a Delta; a livestream
//! never can — it is unbounded and its bytes are worthless a second later. So
//! this module separates the DESCRIPTOR from the BYTES:
//!
//!   * the descriptor ([`MediaRef`]) is small, bounded, and travels in the Delta
//!     where it is signed and fanned out like any other state;
//!   * the bytes arrive by a [`Delivery`] strategy — carried inline for things
//!     small enough to ride along, fetched out of band when they are not, or
//!     never at all for a live session that is joined rather than downloaded.
//!
//! That is why one type covers all four media kinds: a livestream is a Delta
//! that carries an *invitation*, not a payload.
//!
//! # What this module does NOT decide
//!
//! It owns shape, budget and validation. It does not transcode, and it does not
//! re-encode oversize input to fit — a producer that hands over something too
//! large gets a refusal, never a silent shrink, because two devices that
//! re-encode the same delta differently would disagree about its content.

use crate::{ArgVal, Args};
use std::fmt;

/// The playback contract a consumer needs, which is a coarser question than the
/// container format. A UI cares whether a thing is a static frame, a looping
/// animation, a seekable clip or a live session; `image/png` vs `image/jpeg` is
/// a decode detail underneath that.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    /// One frame, no timeline.
    Still,
    /// Self-looping, no controls, no audio. GIFs, and anything that should
    /// behave like one.
    Animation,
    /// A finite timeline: seekable, may carry audio.
    Clip,
    /// No timeline and no end — a session that is joined. Carries no bytes.
    Live,
    /// A document to open or save, not to draw inline: a PDF (W-98 Resources).
    Document,
}

impl MediaKind {
    /// The MIME types a producer may present for this kind. Deliberately a
    /// closed set: an open one means every consumer needs a fallback path for
    /// formats no producer actually emits.
    pub const fn accepts(self) -> &'static [&'static str] {
        match self {
            MediaKind::Still => &["image/jpeg", "image/png"],
            MediaKind::Animation => &["image/gif", "image/webp"],
            MediaKind::Clip => &["video/mp4"],
            // A live session is addressed, not decoded. The wire format is a
            // property of the session the descriptor points at.
            MediaKind::Live => &[],
            MediaKind::Document => &["application/pdf"],
        }
    }

    /// The kind a MIME implies, or `None` for anything outside the closed set.
    pub fn of_mime(mime: &str) -> Option<Self> {
        for k in [MediaKind::Still, MediaKind::Animation, MediaKind::Clip, MediaKind::Document] {
            if k.accepts().contains(&mime) {
                return Some(k);
            }
        }
        None
    }

    /// Whether this kind is expected to carry bytes at all.
    pub const fn has_bytes(self) -> bool {
        !matches!(self, MediaKind::Live)
    }
}

/// WHERE a media slot sits, which is what sets its budget. A banner legitimately
/// deserves more bytes than an avatar, so the budget is a property of the slot
/// rather than one global number — collapsing them would either starve banners
/// or inflate every avatar that fans out to every Connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// A person's or Thing's face. Highest fan-out in the system: a profile edit
    /// pushes this to every Connection, so it is the slot most worth shrinking.
    Avatar,
    /// One of an Event's photo set.
    Photo,
    /// A Post's small square.
    Icon,
    /// The wide image at the top of an Event or Post.
    Banner,
    /// A Group's cover still.
    Cover,
    /// A short video in any of the above positions.
    Clip,
    /// Media attached to a chat message.
    ///
    /// The tightest byte budget in the system, and deliberately so. Every other
    /// slot holds ONE image that is replaced when it changes — a profile photo
    /// costs its size once. A conversation accumulates: every member folds and
    /// stores every message's media forever, so the cost is the budget times the
    /// message count times the roster. A cap that is merely generous here is a
    /// cap that fills a phone.
    Message,
    /// A live session descriptor. Carries no bytes, so its budget covers only
    /// the descriptor itself.
    Live,
    /// A Post's document (W-98 Resources): one PDF, held rather than linked.
    Document,
}

impl Slot {
    /// Ceiling in base64 characters, which is the unit the wire actually
    /// measures because media rides base64-in-delta.
    ///
    /// These preserve the pre-existing per-slot maxima EXACTLY. They are the
    /// numbers to tighten once the block codec lands — at which point the change
    /// is one edit here rather than eighteen.
    pub const fn max_b64(self) -> usize {
        match self {
            Slot::Avatar => 500_000,
            Slot::Photo => 500_000,
            Slot::Icon => 512_000,
            Slot::Banner => 716_800,
            Slot::Cover => 716_800,
            Slot::Clip => 1_500_000,
            // ~24 KB: room for the 160px target plus headroom, and 100 media
            // messages still cost a member under 2.5 MB rather than 50.
            Slot::Message => 24_000,
            // No payload — only a session identifier and its intrinsics.
            Slot::Live => 4_096,
            // The carriage ceiling for one Delta's inline body (ICD 2.3.1, post.setDocument's
            // `document` maxLength); pacific-core pins the two equal.
            Slot::Document => 150_000,
        }
    }

    /// Budget in decoded bytes. Base64 is a transport detail; a producer sizing
    /// an encoder wants the real figure.
    pub const fn max_bytes(self) -> usize {
        self.max_b64() / 4 * 3
    }

    /// Kinds this slot will accept. A `Clip` slot takes moving media; a `Cover`
    /// takes a still or an animation but not a video, which is what keeps a
    /// group header from silently becoming a playing clip.
    pub const fn accepts(self) -> &'static [MediaKind] {
        match self {
            Slot::Avatar | Slot::Photo | Slot::Icon => {
                &[MediaKind::Still, MediaKind::Animation]
            }
            Slot::Banner | Slot::Cover => &[MediaKind::Still, MediaKind::Animation],
            Slot::Clip => &[MediaKind::Clip],
            // Stills and animations both; a Live Photo sent to a chat stays a
            // Live Photo. Video takes the detached path.
            Slot::Message => &[MediaKind::Still, MediaKind::Animation],
            Slot::Live => &[MediaKind::Live],
            Slot::Document => &[MediaKind::Document],
        }
    }
}

/// How the bytes reach a consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Bytes ride inside the Delta, base64-encoded. Simple and self-contained:
    /// once a peer has the delta it has the media, with no second round trip and
    /// nothing to go stale. Paid for by every member storing every byte, which
    /// is why the slot budgets exist.
    Inline { data: String },
    /// The Delta carries a content digest; the bytes are fetched separately.
    /// Lets a slot exceed what is sane to fan out, and lets N referents of the
    /// same image cost one copy. The digest is what makes that safe — a fetched
    /// body that does not hash to it is not the media that was signed for.
    Detached { digest: [u8; 32], bytes: u64 },
    /// [`Delivery::Detached`] with O-79's keys: `key` names the bytes in the relay's blob
    /// store, and `secret` is the content key they are sealed with (base64), which only the
    /// object's roster reads, inside the Delta. On the wire it is `Via: detached` with `Key`
    /// and `Secret`; a detached ref without them is the older form, still read.
    Sealed { digest: [u8; 32], bytes: u64, key: String, secret: String },
    /// No bytes now or later: an invitation to a live session. `session` is
    /// opaque here on purpose — resolving it is the transport's job, not this
    /// module's, and baking a URL shape in would tie media to one transport.
    Live { session: String },
}

impl Delivery {
    /// Length this delivery contributes to the delta, in base64 characters.
    pub fn wire_len(&self) -> usize {
        match self {
            Delivery::Inline { data } => data.len(),
            // A digest plus a length, not a payload.
            Delivery::Detached { .. } => 64 + 20,
            Delivery::Sealed { key, secret, .. } => 64 + 20 + key.len() + secret.len(),
            Delivery::Live { session } => session.len(),
        }
    }
}

/// One media slot's worth of state: what it is, how big it is, and how to get it.
///
/// Intrinsics travel with the descriptor so a consumer can reserve layout before
/// any bytes arrive. Without them a feed re-flows as each image resolves, which
/// is the single most visible way a media pipeline feels cheap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRef {
    pub kind: MediaKind,
    /// Container type. Empty exactly when the ref is empty or `Live`.
    pub mime: String,
    pub delivery: Delivery,
    /// Pixel dimensions, `0` when genuinely unknown. Carried so layout can be
    /// reserved ahead of decode.
    pub width: u32,
    pub height: u32,
    /// Duration for timed kinds; `0` for stills and for live.
    pub duration_ms: u32,
}

/// Why a media ref was refused. Distinct from `DeltaRejection` so this module
/// stays usable off the fold path — a composer wants to know WHICH rule it
/// broke, and the fold only needs to know that it broke one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaError {
    /// Body exceeds the slot budget. Refused, never clamped.
    TooLarge { got: usize, max: usize },
    /// MIME outside the closed set for the kind.
    UnknownMime,
    /// MIME and kind disagree — e.g. `video/mp4` presented as a `Still`.
    MimeKindMismatch,
    /// The slot does not take this kind.
    KindNotAllowed,
    /// Data present without a MIME, or a MIME with no data. Half-set media is
    /// how a consumer ends up rendering a broken frame.
    HalfSet,
    /// Base64 body is not decodable.
    NotBase64,
    /// A live descriptor with no session, or a byte-carrying kind delivered live.
    BadLive,
    /// Source bytes offered straight to a delta without preprocessing.
    RawNotAllowed,
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::TooLarge { got, max } => {
                write!(f, "media too large: {got} b64 chars, slot allows {max}")
            }
            MediaError::UnknownMime => write!(f, "mime outside the accepted set"),
            MediaError::MimeKindMismatch => write!(f, "mime does not match media kind"),
            MediaError::KindNotAllowed => write!(f, "slot does not accept this media kind"),
            MediaError::HalfSet => write!(f, "media data and mime must be set together"),
            MediaError::NotBase64 => write!(f, "media body is not valid base64"),
            MediaError::BadLive => write!(f, "malformed live session descriptor"),
            MediaError::RawNotAllowed => {
                write!(f, "raw source bytes cannot enter a delta — run preprocess first")
            }
        }
    }
}

impl std::error::Error for MediaError {}

impl MediaRef {
    /// The empty slot — "this object has no media here". Distinct from absent:
    /// clearing a banner is an operation, and it needs a value to mean it.
    pub fn empty() -> Self {
        MediaRef {
            kind: MediaKind::Still,
            mime: String::new(),
            delivery: Delivery::Inline {
                data: String::new(),
            },
            width: 0,
            height: 0,
            duration_ms: 0,
        }
    }

    /// True when this slot holds nothing.
    pub fn is_empty(&self) -> bool {
        self.mime.is_empty()
            && matches!(&self.delivery, Delivery::Inline { data } if data.is_empty())
    }

    /// Build an inline still/animation/clip from a base64 body, inferring kind
    /// from the MIME. The common producer path.
    pub fn inline(mime: &str, data: impl Into<String>) -> Result<Self, MediaError> {
        let kind = MediaKind::of_mime(mime).ok_or(MediaError::UnknownMime)?;
        Ok(MediaRef {
            kind,
            mime: mime.to_string(),
            delivery: Delivery::Inline { data: data.into() },
            width: 0,
            height: 0,
            duration_ms: 0,
        })
    }

    /// Build a live-session descriptor.
    pub fn live(session: impl Into<String>) -> Result<Self, MediaError> {
        let session = session.into();
        if session.is_empty() {
            return Err(MediaError::BadLive);
        }
        Ok(MediaRef {
            kind: MediaKind::Live,
            mime: String::new(),
            delivery: Delivery::Live { session },
            width: 0,
            height: 0,
            duration_ms: 0,
        })
    }

    pub fn with_intrinsics(mut self, w: u32, h: u32, duration_ms: u32) -> Self {
        self.width = w;
        self.height = h;
        self.duration_ms = duration_ms;
        self
    }

    /// Did this come out of [`crate::preprocess::preprocess`]?
    ///
    /// Inline bodies are always our own container, so the mime alone answers it.
    /// Detached and live slots carry no inline bytes and so cannot be raw.
    pub fn is_preprocessed(&self) -> bool {
        match &self.delivery {
            Delivery::Inline { data } if !data.is_empty() => {
                self.mime == crate::preprocess::CONTAINER_MIME
            }
            _ => true,
        }
    }

    /// The gate for media WE are about to author.
    ///
    /// Stricter than [`MediaRef::validate`] by exactly one rule: nothing raw
    /// reaches a delta. The two differ on purpose — deltas already in history
    /// predate this rule and must still fold, so the fold path stays permissive
    /// while the authoring path does not. Loosening the fold to match would
    /// reject stored history; tightening authoring is free.
    pub fn validate_for_authoring(&self, slot: Slot) -> Result<(), MediaError> {
        self.validate(slot)?;
        if !self.is_preprocessed() {
            return Err(MediaError::RawNotAllowed);
        }
        Ok(())
    }

    /// What a FOLD enforces on media SOMEBODY ELSE authored.
    ///
    /// Size against the slot, the structural both-or-neither rule, and nothing
    /// further. Deliberately does NOT check the mime against our closed set or
    /// the kind against the slot, because a peer may be running an older build,
    /// a newer one, or simply emitting a container we never produce — HEIC is
    /// the obvious case, since it is what iOS makes natively.
    ///
    /// Getting this wrong is expensive in a specific way: media travels inside a
    /// larger record (a ContactCard, a listing), so refusing the attachment
    /// refuses the WHOLE record and takes a real person's profile off the
    /// device. A picture we cannot decode is a far better outcome than a
    /// contact who vanishes.
    pub fn validate_bounds(&self, slot: Slot) -> Result<(), MediaError> {
        if self.is_empty() {
            return match &self.delivery {
                Delivery::Inline { data } if data.is_empty() && self.mime.is_empty() => Ok(()),
                _ => Err(MediaError::HalfSet),
            };
        }

        match &self.delivery {
            Delivery::Inline { data } => {
                // A body with no type is undisplayable; a type with no body is a
                // missing file. Both are structurally broken whoever authored it.
                if data.is_empty() || self.mime.is_empty() {
                    return Err(MediaError::HalfSet);
                }
                if !is_base64(data) {
                    return Err(MediaError::NotBase64);
                }
            }
            Delivery::Detached { .. } | Delivery::Sealed { .. } => {
                if self.mime.is_empty() {
                    return Err(MediaError::HalfSet);
                }
            }
            Delivery::Live { session } => {
                if session.is_empty() {
                    return Err(MediaError::BadLive);
                }
            }
        }

        let len = self.delivery.wire_len();
        let max = slot.max_b64();
        if len > max {
            return Err(MediaError::TooLarge { got: len, max });
        }
        Ok(())
    }

    /// THE gate. Every producer and every fold runs this and nothing else, so
    /// that "what is allowed in a media slot" has exactly one answer.
    pub fn validate(&self, slot: Slot) -> Result<(), MediaError> {
        if self.is_empty() {
            // An empty slot is always legal — it is how media gets cleared. But
            // it must be wholly empty, not half-set.
            return match &self.delivery {
                Delivery::Inline { data } if data.is_empty() && self.mime.is_empty() => Ok(()),
                _ => Err(MediaError::HalfSet),
            };
        }

        if !slot.accepts().contains(&self.kind) {
            return Err(MediaError::KindNotAllowed);
        }

        match self.kind {
            MediaKind::Live => {
                if !self.mime.is_empty() {
                    return Err(MediaError::MimeKindMismatch);
                }
                match &self.delivery {
                    Delivery::Live { session } if !session.is_empty() => {}
                    _ => return Err(MediaError::BadLive),
                }
            }
            _ => {
                if self.mime.is_empty() {
                    return Err(MediaError::HalfSet);
                }
                // The other direction of half-set: a mime with no body. Must be
                // caught HERE, before the base64 check below, or an empty body
                // gets reported as malformed encoding — which sends a composer
                // hunting a corruption bug when the real fault is a missing file.
                if matches!(&self.delivery, Delivery::Inline { data } if data.is_empty()) {
                    return Err(MediaError::HalfSet);
                }
                // Preprocessed media carries the container mime rather than the
                // source's. That is deliberate: it is what lets a fold tell
                // "this went through preprocess" from "someone put a HEIC on
                // the wire" without decoding a byte.
                if self.mime != crate::preprocess::CONTAINER_MIME
                    && !self.kind.accepts().contains(&self.mime.as_str())
                {
                    // Distinguish "no producer emits this" from "you labelled it
                    // wrong" — the two need different fixes.
                    return Err(if MediaKind::of_mime(&self.mime).is_some() {
                        MediaError::MimeKindMismatch
                    } else {
                        MediaError::UnknownMime
                    });
                }
                if matches!(self.delivery, Delivery::Live { .. }) {
                    return Err(MediaError::BadLive);
                }
            }
        }

        let len = self.delivery.wire_len();
        let max = slot.max_b64();
        if len > max {
            return Err(MediaError::TooLarge { got: len, max });
        }

        if let Delivery::Inline { data } = &self.delivery {
            if !is_base64(data) {
                return Err(MediaError::NotBase64);
            }
        }

        Ok(())
    }
}

/// Cheap structural base64 check. Not a decode: the fold runs this on every
/// media-bearing delta, and decoding a 1.5 MB clip to find out whether it is
/// well-formed would put a megabyte of work on the fold path.
fn is_base64(s: &str) -> bool {
    if s.is_empty() || s.len() % 4 != 0 {
        return false;
    }
    let b = s.as_bytes();
    let pad = b.iter().rev().take_while(|c| **c == b'=').count();
    if pad > 2 {
        return false;
    }
    b[..b.len() - pad]
        .iter()
        .all(|c| c.is_ascii_alphanumeric() || *c == b'+' || *c == b'/')
}

/// Wire encoding.
///
/// `ArgVal` is only `Int` or `Text`, so a `MediaRef` cannot travel as a struct —
/// it FLATTENS into prefixed args. That is a feature rather than a workaround:
/// the ICD describes op payloads as flat property maps, so a flattened ref stays
/// describable and the conformance test keeps working.
///
/// The prefix is the slot's arg name, so `event.setMedia` carries `banner*` and
/// `clip*` side by side in one op without collision.
///
/// The primary key (`<prefix>`) still holds the base64 body and `<prefix>Mime`
/// still holds the mime, exactly as before. A peer that predates this encoding
/// therefore still reads inline media correctly, and sees detached or live media
/// as an empty slot — degraded, but never wrong.
impl MediaRef {
    fn kind_tag(k: MediaKind) -> &'static str {
        match k {
            MediaKind::Still => "still",
            MediaKind::Animation => "anim",
            MediaKind::Clip => "clip",
            MediaKind::Live => "live",
            MediaKind::Document => "document",
        }
    }

    fn tag_kind(t: &str) -> Option<MediaKind> {
        Some(match t {
            "still" => MediaKind::Still,
            "anim" => MediaKind::Animation,
            "clip" => MediaKind::Clip,
            "live" => MediaKind::Live,
            "document" => MediaKind::Document,
            _ => return None,
        })
    }

    /// Write this ref into `args` under `prefix`. An empty ref writes the empty
    /// body and mime and nothing else, so "clear this slot" stays the small,
    /// obvious delta it was before.
    pub fn to_args(&self, prefix: &str, args: &mut Args) {
        let put = |args: &mut Args, k: &str, v: String| {
            args.insert(k.to_string(), ArgVal::Text(v));
        };
        let puti = |args: &mut Args, k: &str, v: i64| {
            args.insert(k.to_string(), ArgVal::Int(v));
        };

        if self.is_empty() {
            put(args, prefix, String::new());
            put(args, &format!("{prefix}Mime"), String::new());
            return;
        }

        put(args, &format!("{prefix}Kind"), Self::kind_tag(self.kind).into());
        put(args, &format!("{prefix}Mime"), self.mime.clone());

        match &self.delivery {
            Delivery::Inline { data } => {
                put(args, prefix, data.clone());
                put(args, &format!("{prefix}Via"), "inline".into());
            }
            Delivery::Detached { digest, bytes } => {
                // Body key stays present and empty: an old peer reads "no media"
                // rather than tripping over a missing key.
                put(args, prefix, String::new());
                put(args, &format!("{prefix}Via"), "detached".into());
                put(args, &format!("{prefix}Digest"), hex32(digest));
                puti(args, &format!("{prefix}Bytes"), *bytes as i64);
            }
            Delivery::Sealed { digest, bytes, key, secret } => {
                put(args, prefix, String::new());
                put(args, &format!("{prefix}Via"), "detached".into());
                put(args, &format!("{prefix}Digest"), hex32(digest));
                puti(args, &format!("{prefix}Bytes"), *bytes as i64);
                put(args, &format!("{prefix}Key"), key.clone());
                put(args, &format!("{prefix}Secret"), secret.clone());
            }
            Delivery::Live { session } => {
                put(args, prefix, String::new());
                put(args, &format!("{prefix}Via"), "live".into());
                put(args, &format!("{prefix}Session"), session.clone());
            }
        }

        // Intrinsics are omitted when unknown rather than written as zero, so a
        // delta does not claim a 0x0 image it knows nothing about.
        if self.width > 0 {
            puti(args, &format!("{prefix}W"), self.width as i64);
        }
        if self.height > 0 {
            puti(args, &format!("{prefix}H"), self.height as i64);
        }
        if self.duration_ms > 0 {
            puti(args, &format!("{prefix}Ms"), self.duration_ms as i64);
        }
    }

    /// Read a ref back out of `args`. Returns `None` only when the slot is
    /// absent entirely; a present-but-malformed slot is an error the caller must
    /// turn into a rejection rather than silently treat as empty.
    pub fn from_args(prefix: &str, args: &Args) -> Option<Result<Self, MediaError>> {
        Self::from_args_by(prefix, |k| args.get(k))
    }

    /// [`MediaRef::from_args`], reading each arg through `get`: a reducer's own reader, so
    /// what it reads is what it records (the core's `arg_reads`, NC-9).
    pub fn from_args_by<'a>(prefix: &str, get: impl Fn(&str) -> Option<&'a ArgVal>) -> Option<Result<Self, MediaError>> {
        let text = |k: &str| match get(k) {
            Some(ArgVal::Text(t)) => Some(t.as_str()),
            _ => None,
        };
        let int = |k: &str| match get(k) {
            Some(ArgVal::Int(i)) if *i >= 0 => Some(*i),
            _ => None,
        };

        let body = text(prefix)?;
        let mime = text(&format!("{prefix}Mime")).unwrap_or("");
        let via = text(&format!("{prefix}Via")).unwrap_or("inline");

        if body.is_empty() && mime.is_empty() && via == "inline" {
            return Some(Ok(MediaRef::empty()));
        }

        // Kind is explicit when present; otherwise inferred from mime, which is
        // what keeps pre-encoding deltas readable.
        let kind = match text(&format!("{prefix}Kind")) {
            Some(t) => match Self::tag_kind(t) {
                Some(k) => k,
                None => return Some(Err(MediaError::MimeKindMismatch)),
            },
            None => match MediaKind::of_mime(mime) {
                Some(k) => k,
                None => return Some(Err(MediaError::UnknownMime)),
            },
        };

        let delivery = match via {
            "inline" => Delivery::Inline {
                data: body.to_string(),
            },
            "detached" => {
                let d = match text(&format!("{prefix}Digest")).and_then(unhex32) {
                    Some(d) => d,
                    None => return Some(Err(MediaError::HalfSet)),
                };
                let bytes = match int(&format!("{prefix}Bytes")) {
                    Some(b) => b as u64,
                    None => return Some(Err(MediaError::HalfSet)),
                };
                // O-79: both keys, or neither (the older form). One without the other is a
                // blob nobody can open, or a key to nothing.
                match (text(&format!("{prefix}Key")), text(&format!("{prefix}Secret"))) {
                    (None, None) => Delivery::Detached { digest: d, bytes },
                    (Some(key), Some(secret)) if is_blob_key(key) && is_base64(secret) && secret.len() <= MAX_SECRET_B64 => {
                        Delivery::Sealed { digest: d, bytes, key: key.to_string(), secret: secret.to_string() }
                    }
                    _ => return Some(Err(MediaError::HalfSet)),
                }
            }
            "live" => match text(&format!("{prefix}Session")) {
                Some(s) if !s.is_empty() => Delivery::Live {
                    session: s.to_string(),
                },
                _ => return Some(Err(MediaError::BadLive)),
            },
            _ => return Some(Err(MediaError::HalfSet)),
        };

        Some(Ok(MediaRef {
            kind,
            mime: mime.to_string(),
            delivery,
            width: int(&format!("{prefix}W")).unwrap_or(0) as u32,
            height: int(&format!("{prefix}H")).unwrap_or(0) as u32,
            duration_ms: int(&format!("{prefix}Ms")).unwrap_or(0) as u32,
        }))
    }
}

/// O-79's content key, base64: a 32-byte key and its sealing, with room to spare.
const MAX_SECRET_B64: usize = 256;

/// A blob store's object key: 1 to 512 printable ASCII characters, no spaces.
fn is_blob_key(s: &str) -> bool {
    !s.is_empty() && s.len() <= 512 && s.bytes().all(|b| b.is_ascii_graphic())
}

fn hex32(b: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(n: usize) -> String {
        "A".repeat(n / 4 * 4)
    }


    fn args_of(pairs: &[(&str, &str)]) -> Args {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), ArgVal::Text(v.to_string())))
            .collect()
    }

    #[test]
    fn inline_round_trips_through_args() {
        let m = MediaRef::inline("image/jpeg", body(128))
            .unwrap()
            .with_intrinsics(640, 480, 0);
        let mut a = Args::new();
        m.to_args("photo", &mut a);
        let back = MediaRef::from_args("photo", &a).unwrap().unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn empty_round_trips_and_stays_small() {
        let mut a = Args::new();
        MediaRef::empty().to_args("photo", &mut a);
        // Clearing a slot must not balloon into a dozen keys.
        assert_eq!(a.len(), 2);
        assert!(MediaRef::from_args("photo", &a).unwrap().unwrap().is_empty());
    }

    #[test]
    fn detached_and_live_round_trip() {
        let d = MediaRef {
            kind: MediaKind::Clip,
            mime: "video/mp4".into(),
            delivery: Delivery::Detached {
                digest: [0xab; 32],
                bytes: 40 << 20,
            },
            width: 1920,
            height: 1080,
            duration_ms: 30_000,
        };
        let mut a = Args::new();
        d.to_args("clip", &mut a);
        assert_eq!(MediaRef::from_args("clip", &a).unwrap().unwrap(), d);

        let l = MediaRef::live("sess-xyz").unwrap();
        let mut b = Args::new();
        l.to_args("clip", &mut b);
        assert_eq!(MediaRef::from_args("clip", &b).unwrap().unwrap(), l);
    }

    #[test]
    fn two_slots_in_one_op_do_not_collide() {
        // event.setMedia carries a banner and a clip together.
        let banner = MediaRef::inline("image/png", body(64)).unwrap();
        let clip = MediaRef::inline("video/mp4", body(256)).unwrap();
        let mut a = Args::new();
        banner.to_args("banner", &mut a);
        clip.to_args("clip", &mut a);
        assert_eq!(MediaRef::from_args("banner", &a).unwrap().unwrap(), banner);
        assert_eq!(MediaRef::from_args("clip", &a).unwrap().unwrap(), clip);
    }

    #[test]
    fn pre_encoding_args_still_read() {
        // A delta authored before this encoding carries only the body and mime.
        // It must still parse, with kind inferred — otherwise every existing
        // stored delta becomes unreadable on upgrade.
        let a = args_of(&[("photo", "c3RpbGw="), ("photoMime", "image/jpeg")]);
        let m = MediaRef::from_args("photo", &a).unwrap().unwrap();
        assert_eq!(m.kind, MediaKind::Still);
        assert_eq!(m.mime, "image/jpeg");
        assert!(matches!(m.delivery, Delivery::Inline { .. }));
        assert!(m.validate(Slot::Avatar).is_ok());
    }

    #[test]
    fn absent_slot_is_none_but_malformed_slot_is_an_error() {
        // The distinction that stops a corrupt delta being folded as "no media".
        let empty = Args::new();
        assert!(MediaRef::from_args("photo", &empty).is_none());

        let bad = args_of(&[
            ("photo", ""),
            ("photoMime", "video/mp4"),
            ("photoKind", "clip"),
            ("photoVia", "detached"),
        ]);
        assert_eq!(
            MediaRef::from_args("photo", &bad).unwrap(),
            Err(MediaError::HalfSet)
        );
    }

    #[test]
    fn a_live_slot_without_a_session_is_refused() {
        let bad = args_of(&[
            ("clip", ""),
            ("clipMime", ""),
            ("clipKind", "live"),
            ("clipVia", "live"),
        ]);
        assert_eq!(
            MediaRef::from_args("clip", &bad).unwrap(),
            Err(MediaError::BadLive)
        );
    }

    #[test]
    fn slot_budgets_match_the_caps_they_replace() {
        // These are the pre-existing per-slot maxima. If one changes, it must be
        // because someone MEANT to change a wire budget.
        assert_eq!(Slot::Avatar.max_b64(), 500_000);
        assert_eq!(Slot::Photo.max_b64(), 500_000);
        assert_eq!(Slot::Icon.max_b64(), 512_000);
        assert_eq!(Slot::Banner.max_b64(), 716_800);
        assert_eq!(Slot::Cover.max_b64(), 716_800);
        assert_eq!(Slot::Clip.max_b64(), 1_500_000);
    }

    #[test]
    fn oversize_is_refused_not_clamped() {
        let m = MediaRef::inline("image/jpeg", body(Slot::Avatar.max_b64() + 4)).unwrap();
        match m.validate(Slot::Avatar) {
            Err(MediaError::TooLarge { max, .. }) => assert_eq!(max, 500_000),
            other => panic!("expected TooLarge, got {other:?}"),
        }
        // …and the boundary itself passes.
        let ok = MediaRef::inline("image/jpeg", body(Slot::Avatar.max_b64())).unwrap();
        assert!(ok.validate(Slot::Avatar).is_ok());
    }

    #[test]
    fn empty_is_legal_but_half_set_is_not() {
        assert!(MediaRef::empty().validate(Slot::Avatar).is_ok());

        let mut half = MediaRef::empty();
        half.mime = "image/jpeg".into();
        assert_eq!(half.validate(Slot::Avatar), Err(MediaError::HalfSet));

        let mut other = MediaRef::inline("image/jpeg", body(64)).unwrap();
        other.mime = String::new();
        assert_eq!(other.validate(Slot::Avatar), Err(MediaError::HalfSet));
    }

    #[test]
    fn slot_refuses_a_kind_it_does_not_take() {
        let clip = MediaRef::inline("video/mp4", body(64)).unwrap();
        assert_eq!(clip.validate(Slot::Avatar), Err(MediaError::KindNotAllowed));
        assert!(clip.validate(Slot::Clip).is_ok());

        let still = MediaRef::inline("image/jpeg", body(64)).unwrap();
        assert_eq!(still.validate(Slot::Clip), Err(MediaError::KindNotAllowed));
    }

    #[test]
    fn mime_and_kind_must_agree() {
        let mut m = MediaRef::inline("image/jpeg", body(64)).unwrap();
        m.kind = MediaKind::Clip;
        assert_eq!(m.validate(Slot::Clip), Err(MediaError::MimeKindMismatch));

        let mut bogus = MediaRef::inline("image/jpeg", body(64)).unwrap();
        bogus.mime = "image/tiff".into();
        assert_eq!(bogus.validate(Slot::Avatar), Err(MediaError::UnknownMime));
    }

    #[test]
    fn unknown_mime_is_refused_at_construction() {
        assert_eq!(
            MediaRef::inline("application/zip", body(64)).unwrap_err(),
            MediaError::UnknownMime
        );
    }

    #[test]
    fn gif_is_an_animation_not_a_still_or_a_clip() {
        // The distinction the UI needs: a gif loops and has no controls, an mp4
        // seeks and may carry audio. Both are "moving", and conflating them is
        // how a silent looping avatar acquires a scrub bar.
        let gif = MediaRef::inline("image/gif", body(64)).unwrap();
        assert_eq!(gif.kind, MediaKind::Animation);
        assert!(gif.validate(Slot::Avatar).is_ok());
        assert!(gif.validate(Slot::Cover).is_ok());
        assert_eq!(gif.validate(Slot::Clip), Err(MediaError::KindNotAllowed));
    }

    #[test]
    fn live_carries_a_session_and_no_bytes() {
        let live = MediaRef::live("sess-abc").unwrap();
        assert_eq!(live.kind, MediaKind::Live);
        assert!(live.validate(Slot::Live).is_ok());
        // A live descriptor is not media bytes and must not land in a byte slot.
        assert_eq!(live.validate(Slot::Avatar), Err(MediaError::KindNotAllowed));
        assert_eq!(MediaRef::live("").unwrap_err(), MediaError::BadLive);
    }

    #[test]
    fn a_byte_kind_cannot_be_delivered_live() {
        // Guards the seam: `Live` delivery is only for `Live` kind. Otherwise a
        // producer could smuggle an unbounded stream into a still slot.
        let mut m = MediaRef::inline("image/jpeg", body(64)).unwrap();
        m.delivery = Delivery::Live {
            session: "s".into(),
        };
        assert_eq!(m.validate(Slot::Avatar), Err(MediaError::BadLive));
    }

    #[test]
    fn detached_costs_a_digest_not_a_payload() {
        // The point of Detached: a 40 MB video is legal in a slot whose inline
        // budget is 1.5 MB, because the delta only carries the digest.
        let m = MediaRef {
            kind: MediaKind::Clip,
            mime: "video/mp4".into(),
            delivery: Delivery::Detached {
                digest: [7u8; 32],
                bytes: 40 << 20,
            },
            width: 1920,
            height: 1080,
            duration_ms: 30_000,
        };
        assert!(m.validate(Slot::Clip).is_ok());
        assert!(m.delivery.wire_len() < 128);
    }

    #[test]
    fn malformed_base64_is_caught() {
        let mut m = MediaRef::inline("image/jpeg", body(64)).unwrap();
        m.delivery = Delivery::Inline {
            data: "not!valid!b64!".into(),
        };
        assert_eq!(m.validate(Slot::Avatar), Err(MediaError::NotBase64));
    }

    #[test]
    fn base64_check_accepts_real_padding() {
        assert!(is_base64("c3RpbGw="));
        assert!(is_base64("Y2xpcA=="));
        assert!(is_base64("AAAA"));
        assert!(!is_base64("AAA"));
        assert!(!is_base64("AA==="));
        assert!(!is_base64(""));
    }

    /// O-79: a detached ref carries its blob's object key and its content key, and reads back
    /// whole; the older detached form, with neither, still reads; one without the other is refused.
    #[test]
    fn a_sealed_ref_carries_its_keys_and_half_a_pair_is_refused() {
        let m = MediaRef {
            kind: MediaKind::Document,
            mime: "application/pdf".into(),
            delivery: Delivery::Sealed { digest: [3u8; 32], bytes: 900_000, key: "blobs/ab12".into(), secret: "c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0".into() },
            width: 0,
            height: 0,
            duration_ms: 0,
        };
        let mut a = Args::new();
        m.to_args("document", &mut a);
        assert_eq!(a.get("documentVia"), Some(&ArgVal::Text("detached".into())));
        assert_eq!(MediaRef::from_args("document", &a).unwrap().unwrap(), m);
        assert!(m.validate(Slot::Document).is_ok(), "the Delta carries keys, not the bytes");

        let mut old = a.clone();
        old.remove("documentKey");
        old.remove("documentSecret");
        assert!(matches!(MediaRef::from_args("document", &old).unwrap().unwrap().delivery, Delivery::Detached { .. }));

        for gone in ["documentKey", "documentSecret"] {
            let mut half = a.clone();
            half.remove(gone);
            assert_eq!(MediaRef::from_args("document", &half).unwrap(), Err(MediaError::HalfSet), "{gone} missing");
        }
        let mut bad = a.clone();
        bad.insert("documentSecret".into(), ArgVal::Text("not base64!".into()));
        assert_eq!(MediaRef::from_args("document", &bad).unwrap(), Err(MediaError::HalfSet));
    }

    /// A PDF is its own kind, taken only by the Document slot, inline within its ceiling.
    #[test]
    fn a_pdf_is_a_document_and_only_the_document_slot_takes_it() {
        let m = MediaRef::inline("application/pdf", "JVBERi0xLjQ=").unwrap();
        assert_eq!(m.kind, MediaKind::Document);
        assert!(m.validate(Slot::Document).is_ok());
        assert_eq!(m.validate(Slot::Banner), Err(MediaError::KindNotAllowed));
        let mut a = Args::new();
        m.to_args("document", &mut a);
        assert_eq!(a.get("documentKind"), Some(&ArgVal::Text("document".into())));
        assert_eq!(MediaRef::from_args("document", &a).unwrap().unwrap(), m);
        let big = MediaRef::inline("application/pdf", "A".repeat(Slot::Document.max_b64() + 4)).unwrap();
        assert!(matches!(big.validate(Slot::Document), Err(MediaError::TooLarge { .. })));
    }
}
