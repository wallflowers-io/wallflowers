//! PREPROCESS — the only sanctioned way media becomes a delta.
//!
//! The rule this module exists to enforce: **nothing raw ever reaches a delta.**
//! Not a HEIC off the picker, not a JPEG, not an untouched MP4. What travels is
//! always our own container — downsampled, block-coded, fixed-rate — and the
//! facts the platform reported about the source travel as typed fields beside it
//! rather than being lost on the way in.
//!
//! # Why this takes pixels and not a file
//!
//! This crate has no dependencies, by design, so it cannot decode HEIC or JPEG
//! or demux an MP4. That is the platform's job, and it is also where the
//! orientation transform has to be applied — iOS hands back an image whose
//! pixels are rotated relative to how it should display, with the correction
//! carried separately in EXIF or in a track's `preferredTransform`. So the
//! contract is: **the caller decodes and orients; this module downsamples,
//! encodes and records.** [`SourceFacts::orientation_applied`] is the caller
//! asserting it held up its end.
//!
//! # What gets dropped
//!
//! Preprocessing is lossy on purpose, and the losses are reported rather than
//! silent — see [`Discard`]. An HDR gain map vanishing without a word is the
//! kind of thing that surfaces later as "photos look flatter in Pacific than in
//! Photos", with nothing in the code to explain it.

use crate::anim;
use crate::media::{Delivery, MediaError, MediaKind, MediaRef, Slot};
use crate::pyramid;

/// The mime inline media carries once preprocessed. Inline bodies are ALWAYS
/// this — that is what makes "nothing raw hits a delta" a checkable property
/// rather than a habit.
pub const CONTAINER_MIME: &str = "application/vnd.pacific.media";

/// How the platform said the source needed rotating. Recorded for provenance;
/// the pixels handed to [`preprocess`] must already be upright.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Up,
    Down,
    Left,
    Right,
    Mirrored,
}

/// What the platform reported about the media the user picked. Everything here
/// is knowable at ingest for free — `CGImageSourceCopyPropertiesAtIndex` for
/// stills, `AVAsset` for video — and is lost forever if not captured there.
#[derive(Clone, Debug, Default)]
pub struct SourceFacts {
    /// What it was before us: `image/heic`, `video/mp4`, and so on.
    pub source_mime: String,
    pub source_bytes: usize,
    /// Pixel dimensions AS SUPPLIED, i.e. after orientation.
    pub width: u32,
    pub height: u32,
    pub duration_ms: u32,
    pub orientation: Orientation,
    /// Caller asserts the pixels are upright. False is a programming error, not
    /// a recoverable condition, so it is refused rather than guessed at.
    pub orientation_applied: bool,
    /// iOS 17+ stills carry one. We do not.
    pub has_gain_map: bool,
    /// Portrait mode ships a per-pixel subject matte. Worth knowing we had one
    /// even before anything consumes it.
    pub has_depth_matte: bool,
    /// A Live Photo is a still plus a short clip.
    pub live_photo: bool,
    pub has_audio: bool,
}

/// Something the source carried that the delta will not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discard {
    /// HDR gain map: tone-mapped to SDR at ingest.
    GainMap,
    /// Depth/portrait matte not yet carried.
    DepthMatte,
    /// The motion half of a Live Photo.
    LivePhotoMotion,
    /// Audio track.
    Audio,
    /// Resolution above the slot's target base.
    Resolution { from: u32, to: u32 },
    /// Frames beyond the animation length cap.
    Frames { from: usize, to: usize },
}

/// The result of ingest: what travels, and an account of what did not.
#[derive(Clone, Debug)]
pub struct Preprocessed {
    pub media: MediaRef,
    /// Longest edge of the stored version.
    pub base: u32,
    pub source_bytes: usize,
    pub wire_b64: usize,
    pub discarded: Vec<Discard>,
}

impl Preprocessed {
    /// How much smaller the delta is than what the user handed us.
    pub fn shrink_factor(&self) -> f32 {
        if self.wire_b64 == 0 {
            return 0.0;
        }
        self.source_bytes as f32 / self.wire_b64 as f32
    }
}

impl Slot {
    /// Longest edge this slot stores. Distinct from [`Slot::max_b64`], which is
    /// the legacy ceiling a peer's delta must clear — this is the size we
    /// actually author at, and it is the number to move when the codec improves.
    pub const fn target_base(self) -> u32 {
        match self {
            Slot::Avatar => 128,
            Slot::Photo => 160,
            Slot::Icon => 96,
            Slot::Banner => 256,
            Slot::Cover => 256,
            Slot::Clip => 96,
            Slot::Message => 160,
            Slot::Live => 0,
            Slot::Document => 0,
        }
    }
}

/// Ingest oriented RGBA and produce the thing that goes on the wire.
///
/// `rgba` is tightly packed, 4 bytes per pixel, `w * h * 4` long, already
/// upright. Returns the encoded ref plus the account of what was dropped.
pub fn preprocess(
    rgba: &[u8],
    w: u32,
    h: u32,
    slot: Slot,
    facts: &SourceFacts,
) -> Result<Preprocessed, MediaError> {
    if slot == Slot::Live {
        return Err(MediaError::KindNotAllowed);
    }
    if !slot.accepts().contains(&MediaKind::Still) {
        // A Clip slot wants moving media, and this path produces a still.
        return Err(MediaError::KindNotAllowed);
    }
    if w == 0 || h == 0 || rgba.len() < (w as usize * h as usize * 4) {
        return Err(MediaError::HalfSet);
    }
    if !facts.orientation_applied {
        // Refusing beats guessing: an upside-down avatar that fanned out to
        // every Connection cannot be un-sent.
        return Err(MediaError::HalfSet);
    }

    let mut discarded = Vec::new();
    if facts.has_gain_map {
        discarded.push(Discard::GainMap);
    }
    if facts.has_depth_matte {
        discarded.push(Discard::DepthMatte);
    }
    if facts.live_photo {
        discarded.push(Discard::LivePhotoMotion);
    }
    if facts.has_audio {
        discarded.push(Discard::Audio);
    }

    // Longest edge to the slot's target, aspect preserved, both edges snapped to
    // a multiple of 4 so no partial blocks sit on the right and bottom edges.
    let (nw, nh) = target_size(w, h, slot);
    if nw != w || nh != h {
        discarded.push(Discard::Resolution {
            from: w.max(h),
            to: nw.max(nh),
        });
    }

    // RGBA -> RGB. Alpha is not carried: the block format is colour-only, which
    // is what keeps it a flat 8 bytes per 4x4 with nothing to signal.
    let n = (w * h) as usize;
    let mut rgb = Vec::with_capacity(n * 3);
    for i in 0..n {
        rgb.push(rgba[i * 4]);
        rgb.push(rgba[i * 4 + 1]);
        rgb.push(rgba[i * 4 + 2]);
    }

    let small = if nw == w && nh == h {
        rgb
    } else {
        pyramid::resample_area(&rgb, w, h, nw, nh)
    };

    // Base level only. Mips are derivable on device by downsampling, so paying
    // +33% to ship them would be paying for something the decoder regenerates.
    let container = pyramid::Pyramid {
        levels: vec![pyramid::Level {
            w: nw,
            h: nh,
            blocks: pyramid::encode_level(&small, nw, nh),
        }],
    }
    .serialize();

    let body = b64_encode(&container);
    let wire_b64 = body.len();

    // Stills only, and the ref says so. There is no frame-sequence codec yet, so
    // labelling this a Clip because the SOURCE had a duration would produce a ref
    // claiming N seconds of video while carrying exactly one encoded frame. A
    // descriptor that lies about its own contents is worse than a missing
    // feature: every consumer downstream believes it.
    //
    // Timed media takes the poster-plus-detached shape instead — see
    // [`poster_and_clip`].
    let media = MediaRef {
        kind: MediaKind::Still,
        mime: CONTAINER_MIME.to_string(),
        delivery: Delivery::Inline { data: body },
        width: nw,
        height: nh,
        duration_ms: 0,
    };

    media.validate(slot)?;

    Ok(Preprocessed {
        media,
        base: nw.max(nh),
        source_bytes: facts.source_bytes,
        wire_b64,
        discarded,
    })
}


/// Timed media: a preprocessed poster that rides in the delta, plus a detached
/// reference to the footage itself.
///
/// Video does not go inline. `AVAssetExportSession` presets target a quality
/// level, not a byte budget, so an inline clip is a bet on the encoder landing
/// under the cap — and losing that bet means a delta that every Connection
/// refuses. Detached moves the bytes off the delta entirely: what travels is a
/// digest, which is also what makes a fetched body verifiable as the one that
/// was signed for.
///
/// `poster_rgba` is the oriented frame to show before playback starts.
pub fn poster_and_clip(
    poster_rgba: &[u8],
    w: u32,
    h: u32,
    poster_slot: Slot,
    digest: [u8; 32],
    clip_bytes: u64,
    facts: &SourceFacts,
) -> Result<(Preprocessed, MediaRef), MediaError> {
    let poster = preprocess(poster_rgba, w, h, poster_slot, facts)?;
    let clip = MediaRef {
        kind: MediaKind::Clip,
        mime: if facts.source_mime.is_empty() {
            "video/mp4".to_string()
        } else {
            facts.source_mime.clone()
        },
        delivery: Delivery::Detached {
            digest,
            bytes: clip_bytes,
        },
        width: facts.width,
        height: facts.height,
        duration_ms: facts.duration_ms,
    };
    clip.validate_for_authoring(Slot::Clip)?;
    Ok((poster, clip))
}

/// Frames a Live Photo loop is trimmed to. Roughly two seconds at 12 fps —
/// enough to read as motion, short enough that replaying patches to reach the
/// last frame stays trivial.
pub const ANIM_MAX_FRAMES: usize = 24;
pub const ANIM_FPS: u8 = 12;

/// Per-block error budget before a block is re-sent. Tuned to ignore sensor
/// noise in a handheld capture, which otherwise re-sends most of a static frame
/// for changes nobody can see.
pub const ANIM_THRESHOLD: i64 = 900;

/// A Live Photo becomes an ANIMATION — the motion we were previously discarding.
///
/// `frames` are oriented RGBA buffers, all `w`x`h`, in capture order. They are
/// downsampled to the slot's target base and encoded as a patch sequence, so a
/// mostly-static handheld capture costs little more than the still it came with.
///
/// Animation rather than Clip on purpose: this loops, has no controls and no
/// audio, which is exactly what the kind means and exactly what a Live Photo is.
pub fn preprocess_live_photo(
    frames: &[&[u8]],
    w: u32,
    h: u32,
    slot: Slot,
    facts: &SourceFacts,
) -> Result<Preprocessed, MediaError> {
    if frames.is_empty() {
        return Err(MediaError::HalfSet);
    }
    if !slot.accepts().contains(&MediaKind::Animation) {
        return Err(MediaError::KindNotAllowed);
    }
    if !facts.orientation_applied {
        return Err(MediaError::HalfSet);
    }
    let need = w as usize * h as usize * 4;
    if w == 0 || h == 0 || frames.iter().any(|f| f.len() < need) {
        return Err(MediaError::HalfSet);
    }

    let mut discarded = Vec::new();
    if facts.has_gain_map {
        discarded.push(Discard::GainMap);
    }
    if facts.has_depth_matte {
        discarded.push(Discard::DepthMatte);
    }
    if facts.has_audio {
        discarded.push(Discard::Audio);
    }
    // Note what is NOT here: LivePhotoMotion. That is the point of this path.

    // Even sampling across the capture rather than the first N frames, so a
    // trimmed loop still covers the whole gesture instead of its opening moment.
    let take = frames.len().min(ANIM_MAX_FRAMES);
    if frames.len() > take {
        discarded.push(Discard::Frames {
            from: frames.len(),
            to: take,
        });
    }

    let (nw, nh) = target_size(w, h, slot);
    if nw != w || nh != h {
        discarded.push(Discard::Resolution {
            from: w.max(h),
            to: nw.max(nh),
        });
    }

    let mut small = Vec::with_capacity(take);
    for i in 0..take {
        let src = frames[i * frames.len() / take.max(1)].min_len(need);
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for p in 0..(w * h) as usize {
            rgb.push(src[p * 4]);
            rgb.push(src[p * 4 + 1]);
            rgb.push(src[p * 4 + 2]);
        }
        small.push(if nw == w && nh == h {
            rgb
        } else {
            pyramid::resample_area(&rgb, w, h, nw, nh)
        });
    }

    let container = anim::encode_anim(&small, nw, nh, ANIM_FPS, ANIM_THRESHOLD)
        .ok_or(MediaError::HalfSet)?;
    let body = b64_encode(&container);
    let wire_b64 = body.len();

    let media = MediaRef {
        kind: MediaKind::Animation,
        mime: CONTAINER_MIME.to_string(),
        delivery: Delivery::Inline { data: body },
        width: nw,
        height: nh,
        duration_ms: (take as u32 * 1000) / ANIM_FPS as u32,
    };
    media.validate(slot)?;

    Ok(Preprocessed {
        media,
        base: nw.max(nh),
        source_bytes: facts.source_bytes,
        wire_b64,
        discarded,
    })
}

trait MinLen {
    fn min_len(&self, n: usize) -> &Self;
}
impl MinLen for [u8] {
    fn min_len(&self, n: usize) -> &Self {
        &self[..n.min(self.len())]
    }
}

/// Longest edge to the slot target, aspect preserved, both edges snapped to a
/// whole number of blocks.
fn target_size(w: u32, h: u32, slot: Slot) -> (u32, u32) {
    let base = slot.target_base();
    let longest = w.max(h);
    let (mut nw, mut nh) = if longest <= base {
        (w, h)
    } else {
        let s = base as f32 / longest as f32;
        (
            ((w as f32 * s).round() as u32).max(4),
            ((h as f32 * s).round() as u32).max(4),
        )
    };
    nw = nw.div_ceil(4) * 4;
    nh = nh.div_ceil(4) * 4;
    (nw, nh)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64, written out here because this crate carries no dependencies.
pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let v = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(v >> 18) as usize & 63] as char);
        out.push(B64[(v >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            B64[(v >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            B64[v as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic gradient — this exercises the plumbing, not image quality.
    fn pixels(w: u32, h: u32, seed: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                v.push((x as u8).wrapping_mul(3).wrapping_add(seed));
                v.push((y as u8).wrapping_mul(5));
                v.push(seed);
                v.push(255);
            }
        }
        v
    }

    fn facts(w: u32, h: u32) -> SourceFacts {
        SourceFacts {
            source_mime: "image/heic".into(),
            source_bytes: (w * h * 3 / 4) as usize,
            width: w,
            height: h,
            orientation_applied: true,
            ..Default::default()
        }
    }


    /// A static background with a small square moving across it — the shape of a
    /// handheld Live Photo, where almost nothing changes between frames.
    fn moving_square(w: u32, h: u32, n: usize, travel: bool) -> Vec<Vec<u8>> {
        (0..n)
            .map(|f| {
                let mut v = vec![0u8; (w * h * 4) as usize];
                for y in 0..h {
                    for x in 0..w {
                        let o = ((y * w + x) * 4) as usize;
                        v[o] = 40;
                        v[o + 1] = 90;
                        v[o + 2] = 140;
                        v[o + 3] = 255;
                    }
                }
                let cx = if travel { 8 + f as u32 * 3 } else { w / 2 };
                for y in (h / 2)..(h / 2 + 12).min(h) {
                    for x in cx..(cx + 12).min(w) {
                        let o = ((y * w + x) * 4) as usize;
                        v[o] = 250;
                        v[o + 1] = 240;
                        v[o + 2] = 60;
                    }
                }
                v
            })
            .collect()
    }

    fn refs(v: &[Vec<u8>]) -> Vec<&[u8]> {
        v.iter().map(|f| f.as_slice()).collect()
    }

    #[test]
    fn a_live_photo_becomes_an_animation_and_keeps_its_motion() {
        let f = moving_square(256, 256, 20, true);
        let mut facts = facts(256, 256);
        facts.live_photo = true;
        let p = preprocess_live_photo(&refs(&f), 256, 256, Slot::Avatar, &facts).unwrap();

        assert_eq!(p.media.kind, MediaKind::Animation);
        assert!(p.media.is_preprocessed());
        assert!(p.media.validate_for_authoring(Slot::Avatar).is_ok());
        // The whole point: the motion is no longer thrown away.
        assert!(!p.discarded.contains(&Discard::LivePhotoMotion));
        assert!(p.media.duration_ms > 0);
    }

    #[test]
    fn a_still_capture_costs_almost_nothing_after_the_first_frame() {
        // Conditional replenishment earning its keep: if nothing moves, nothing
        // is re-sent, and 20 frames cost about what one frame costs.
        let f = moving_square(256, 256, 20, false);
        let p = preprocess_live_photo(&refs(&f), 256, 256, Slot::Avatar, &facts(256, 256)).unwrap();
        let one = preprocess(&f[0], 256, 256, Slot::Avatar, &facts(256, 256)).unwrap();
        assert!(
            p.wire_b64 < one.wire_b64 * 3 / 2,
            "20 static frames cost {} vs {} for one still",
            p.wire_b64,
            one.wire_b64
        );
    }

    #[test]
    fn only_the_blocks_that_moved_are_re_sent() {
        let f = moving_square(256, 256, 12, true);
        let small: Vec<Vec<u8>> = f
            .iter()
            .map(|fr| {
                let mut rgb = Vec::new();
                for p in 0..(256 * 256) {
                    rgb.push(fr[p * 4]);
                    rgb.push(fr[p * 4 + 1]);
                    rgb.push(fr[p * 4 + 2]);
                }
                pyramid::resample_area(&rgb, 256, 256, 128, 128)
            })
            .collect();
        let c = anim::encode_anim(&small, 128, 128, ANIM_FPS, ANIM_THRESHOLD).unwrap();
        let a = anim::Anim::parse(&c).unwrap();
        let total = (128 / 4) * (128 / 4);
        let n = a.patch_count(5).unwrap();
        assert!(n > 0, "a moving subject must re-send something");
        assert!(
            n < total / 4,
            "re-sent {n} of {total} blocks — replenishment is not paying off"
        );
    }

    #[test]
    fn frames_replay_to_the_right_state() {
        let f = moving_square(128, 128, 8, true);
        let small: Vec<Vec<u8>> = f
            .iter()
            .map(|fr| {
                let mut rgb = Vec::new();
                for p in 0..(128 * 128) {
                    rgb.push(fr[p * 4]);
                    rgb.push(fr[p * 4 + 1]);
                    rgb.push(fr[p * 4 + 2]);
                }
                rgb
            })
            .collect();
        let c = anim::encode_anim(&small, 128, 128, 12, 0).unwrap();
        let a = anim::Anim::parse(&c).unwrap();
        assert_eq!(a.frames, 8);
        assert_eq!((a.w, a.h), (128, 128));
        // Every frame materialises to a full, decodable block plane.
        let want = ((128 / 4) * (128 / 4)) as usize * 8;
        for i in 0..8 {
            assert_eq!(a.blocks_at(i).unwrap().len(), want, "frame {i}");
        }
        // And the subject really has moved between first and last.
        assert_ne!(a.blocks_at(0).unwrap(), a.blocks_at(7).unwrap());
    }

    #[test]
    fn an_animation_fits_the_slot_it_is_authored_for() {
        for slot in [Slot::Avatar, Slot::Icon, Slot::Cover] {
            let f = moving_square(400, 400, 24, true);
            let p = preprocess_live_photo(&refs(&f), 400, 400, slot, &facts(400, 400))
                .unwrap_or_else(|e| panic!("{slot:?}: {e}"));
            assert!(p.wire_b64 <= slot.max_b64(), "{slot:?} overran its cap");
            assert!(p.media.validate_for_authoring(slot).is_ok(), "{slot:?}");
        }
    }

    #[test]
    fn a_long_capture_is_trimmed_and_says_so() {
        let f = moving_square(128, 128, 90, true);
        let p = preprocess_live_photo(&refs(&f), 128, 128, Slot::Avatar, &facts(128, 128)).unwrap();
        assert!(p
            .discarded
            .iter()
            .any(|d| matches!(d, Discard::Frames { from: 90, to: 24 })));
    }

    #[test]
    fn a_clip_slot_still_refuses_an_animation() {
        // Kind rules hold: a looping animation is not seekable footage.
        let f = moving_square(128, 128, 6, true);
        assert_eq!(
            preprocess_live_photo(&refs(&f), 128, 128, Slot::Clip, &facts(128, 128)).unwrap_err(),
            MediaError::KindNotAllowed
        );
    }

    #[test]
    fn a_malformed_animation_container_is_rejected() {
        assert!(anim::Anim::parse(b"nope").is_none());
        assert!(anim::Anim::parse(&[0u8; 64]).is_none());
        // Right magic, truncated body.
        let mut short = b"LOFA".to_vec();
        short.extend_from_slice(&[1, 4, 64, 0, 64, 0, 12]);
        assert!(anim::Anim::parse(&short).is_none());
    }

    #[test]
    fn nothing_raw_can_reach_a_delta() {
        // The rule this module exists for. A HEIC/JPEG body offered straight to
        // a slot is refused at the authoring gate…
        let raw = MediaRef::inline("image/jpeg", "A".repeat(4000)).unwrap();
        assert_eq!(
            raw.validate_for_authoring(Slot::Avatar),
            Err(MediaError::RawNotAllowed)
        );
        // …while preprocessed output passes it.
        let p = preprocess(&pixels(512, 512, 9), 512, 512, Slot::Avatar, &facts(512, 512)).unwrap();
        assert!(p.media.is_preprocessed());
        assert!(p.media.validate_for_authoring(Slot::Avatar).is_ok());
    }

    #[test]
    fn the_fold_stays_permissive_where_authoring_does_not() {
        // Deltas already in history predate the rule and must still fold.
        // Tightening authoring is free; tightening the fold would reject stored
        // history, so the two gates differ on exactly this one point.
        let raw = MediaRef::inline("image/jpeg", "A".repeat(4000)).unwrap();
        assert!(raw.validate(Slot::Avatar).is_ok());
        assert!(raw.validate_for_authoring(Slot::Avatar).is_err());
    }

    #[test]
    fn unoriented_pixels_are_refused_rather_than_guessed_at() {
        let mut f = facts(256, 256);
        f.orientation_applied = false;
        assert!(preprocess(&pixels(256, 256, 1), 256, 256, Slot::Avatar, &f).is_err());
    }

    #[test]
    fn a_large_source_is_stored_downsampled() {
        let p = preprocess(&pixels(1024, 1024, 3), 1024, 1024, Slot::Avatar, &facts(1024, 1024))
            .unwrap();
        assert_eq!(p.base, Slot::Avatar.target_base());
        assert_eq!(p.media.width, 128);
        assert_eq!(p.media.height, 128);
        assert!(p
            .discarded
            .iter()
            .any(|d| matches!(d, Discard::Resolution { from: 1024, to: 128 })));
    }

    #[test]
    fn aspect_is_preserved_and_edges_snap_to_whole_blocks() {
        // 4:3 into a 128 slot. Longest edge hits the target; the short edge
        // keeps the ratio and rounds up to a multiple of 4 so no partial blocks
        // sit on the right or bottom.
        let p = preprocess(&pixels(800, 600, 2), 800, 600, Slot::Avatar, &facts(800, 600)).unwrap();
        assert_eq!(p.media.width, 128);
        assert_eq!(p.media.height, 96);
        assert_eq!(p.media.width % 4, 0);
        assert_eq!(p.media.height % 4, 0);
    }

    #[test]
    fn a_small_source_is_not_upscaled() {
        let p = preprocess(&pixels(64, 64, 4), 64, 64, Slot::Avatar, &facts(64, 64)).unwrap();
        assert_eq!((p.media.width, p.media.height), (64, 64));
        assert!(!p
            .discarded
            .iter()
            .any(|d| matches!(d, Discard::Resolution { .. })));
    }

    #[test]
    fn rate_is_fixed_regardless_of_content() {
        // Two unrelated pictures into the same slot cost the same wire bytes.
        // That is the property the delta budget is planned against.
        let a = preprocess(&pixels(512, 512, 11), 512, 512, Slot::Avatar, &facts(512, 512)).unwrap();
        let b = preprocess(&pixels(512, 512, 200), 512, 512, Slot::Avatar, &facts(512, 512)).unwrap();
        assert_eq!(a.wire_b64, b.wire_b64);
    }

    #[test]
    fn what_the_source_carried_and_we_drop_is_reported() {
        // Silent loss is the failure mode here: an HDR gain map vanishing with
        // nothing in the record is how "looks flatter than Photos" becomes an
        // unexplainable bug months later.
        let mut f = facts(400, 400);
        f.has_gain_map = true;
        f.has_depth_matte = true;
        f.live_photo = true;
        f.has_audio = true;
        let p = preprocess(&pixels(400, 400, 6), 400, 400, Slot::Avatar, &f).unwrap();
        for d in [
            Discard::GainMap,
            Discard::DepthMatte,
            Discard::LivePhotoMotion,
            Discard::Audio,
        ] {
            assert!(p.discarded.contains(&d), "{d:?} not reported");
        }
    }

    #[test]
    fn the_delta_is_far_smaller_than_what_the_user_handed_us() {
        let mut f = facts(1024, 1024);
        f.source_bytes = 2_400_000; // a plausible HEIC off the camera
        let p = preprocess(&pixels(1024, 1024, 7), 1024, 1024, Slot::Avatar, &f).unwrap();
        assert!(p.wire_b64 < 12_000, "wire b64 was {}", p.wire_b64);
        assert!(p.shrink_factor() > 100.0, "only {}x", p.shrink_factor());
    }

    #[test]
    fn every_slot_authors_within_its_own_budget() {
        // Clip is absent on purpose: it takes moving media, and this path
        // produces stills. Timed media goes through `poster_and_clip`.
        for slot in [
            Slot::Avatar,
            Slot::Photo,
            Slot::Icon,
            Slot::Banner,
            Slot::Cover,
        ] {
            let p = preprocess(&pixels(900, 900, 5), 900, 900, slot, &facts(900, 900))
                .unwrap_or_else(|e| panic!("{slot:?}: {e}"));
            assert!(p.wire_b64 <= slot.max_b64(), "{slot:?} overran its cap");
            assert!(p.media.validate_for_authoring(slot).is_ok(), "{slot:?}");
        }
    }

    #[test]
    fn a_live_slot_has_no_pixels_to_preprocess() {
        assert_eq!(
            preprocess(&pixels(64, 64, 1), 64, 64, Slot::Live, &facts(64, 64)).unwrap_err(),
            MediaError::KindNotAllowed
        );
    }

    #[test]
    fn base64_matches_the_reference_vectors() {
        // RFC 4648 test vectors — the encoder is hand-written, so it gets pinned.
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(b64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_still_never_claims_to_be_a_clip() {
        // Regression: kind was derived from the SOURCE duration, so a video
        // produced a ref labelled Clip carrying one encoded frame.
        let mut f = facts(640, 640);
        f.duration_ms = 2_400;
        let p = preprocess(&pixels(640, 640, 8), 640, 640, Slot::Cover, &f).unwrap();
        assert_eq!(p.media.kind, MediaKind::Still);
        assert_eq!(p.media.duration_ms, 0);
        // And a Clip slot refuses a still outright.
        assert_eq!(
            preprocess(&pixels(640, 640, 8), 640, 640, Slot::Clip, &f).unwrap_err(),
            MediaError::KindNotAllowed
        );
    }

    #[test]
    fn timed_media_travels_as_poster_plus_digest() {
        let mut f = facts(1920, 1080);
        f.source_mime = "video/mp4".into();
        f.duration_ms = 30_000;
        f.width = 1920;
        f.height = 1080;
        let (poster, clip) =
            poster_and_clip(&pixels(640, 640, 2), 640, 640, Slot::Cover, [0xab; 32], 40 << 20, &f)
                .unwrap();
        assert!(poster.media.is_preprocessed());
        assert_eq!(poster.media.kind, MediaKind::Still);
        // 40 MB of footage, and the delta carries under 128 bytes for it.
        assert!(clip.delivery.wire_len() < 128);
        assert_eq!(clip.duration_ms, 30_000);
        assert!(clip.validate_for_authoring(Slot::Clip).is_ok());
    }
}
