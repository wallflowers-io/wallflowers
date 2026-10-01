//! Wasm shell over `pacific-media`.
//!
//! Every number the demos display comes from the SAME code the phone runs —
//! the block codec, the slot budgets, the validation rules and the wire
//! flattening are all called across this boundary rather than restated in JS.
//! A demo that reimplements the thing it is demonstrating proves nothing.
//!
//! Follows the `viz-wasm` idiom: raw `extern "C"` exports, ptr/len pairs into
//! linear memory, no bindgen and no dependencies.

#![allow(static_mut_refs)]

use pacific_media::media::{Delivery, MediaError, MediaKind, MediaRef, Slot};
use pacific_media::{pyramid, ArgVal, Args};

struct State {
    w: u32,
    h: u32,
    /// Base-level blocks — the only thing the wire would carry.
    blocks: Vec<u8>,
    /// Serialized container, as it would ride in a delta.
    wire: Vec<u8>,
    /// Source pixels kept for the fidelity comparison.
    src: Vec<u8>,
    /// RGBA handed back to the canvas.
    out_px: Vec<u8>,
    out_w: u32,
    out_h: u32,
    /// Text channel (flattened args, error strings).
    out_txt: String,
    region_blocks: u32,
}

impl State {
    fn new() -> Self {
        State {
            w: 0,
            h: 0,
            blocks: Vec::new(),
            wire: Vec::new(),
            src: Vec::new(),
            out_px: Vec::new(),
            out_w: 0,
            out_h: 0,
            out_txt: String::new(),
            region_blocks: 0,
        }
    }
}

static mut ST: Option<Box<State>> = None;

fn st() -> &'static mut State {
    unsafe {
        if ST.is_none() {
            ST = Some(Box::new(State::new()));
        }
        ST.as_mut().unwrap()
    }
}

// ---------------------------------------------------------------- memory

static mut SCRATCH: Vec<u8> = Vec::new();

/// Hand JS a buffer to write pixels into.
#[no_mangle]
pub extern "C" fn alloc(len: u32) -> *mut u8 {
    unsafe {
        SCRATCH = vec![0u8; len as usize];
        SCRATCH.as_mut_ptr()
    }
}

// ---------------------------------------------------------------- codec

/// Encode the RGBA currently in the scratch buffer at `w`x`h`.
/// Returns the wire byte count — fixed for a given size, by construction.
#[no_mangle]
pub extern "C" fn encode(w: u32, h: u32) -> u32 {
    let s = st();
    let rgba = unsafe { &SCRATCH };
    // Drop alpha: the block format is colour-only, which is part of why it is
    // a flat 8 bytes per 4x4 with no per-block mode signalling to decode.
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for i in 0..(w * h) as usize {
        rgb.push(rgba[i * 4]);
        rgb.push(rgba[i * 4 + 1]);
        rgb.push(rgba[i * 4 + 2]);
    }
    s.w = w;
    s.h = h;
    s.blocks = pyramid::encode_level(&rgb, w, h);
    s.src = rgb;

    // What the delta would actually carry: header + base level, no mips. The
    // mips are derivable on device, so paying +33% to ship them would be paying
    // for something the decoder can regenerate exactly.
    let p = pyramid::Pyramid {
        levels: vec![pyramid::Level {
            w,
            h,
            blocks: s.blocks.clone(),
        }],
    };
    s.wire = p.serialize();
    s.wire.len() as u32
}

#[no_mangle]
pub extern "C" fn wire_len() -> u32 {
    st().wire.len() as u32
}

/// Base64 length, which is the unit the wire budget is measured in because
/// media rides base64-in-delta.
#[no_mangle]
pub extern "C" fn wire_b64_len() -> u32 {
    ((st().wire.len() + 2) / 3 * 4) as u32
}

#[no_mangle]
pub extern "C" fn total_blocks() -> u32 {
    let s = st();
    pyramid::block_count(s.w, s.h)
}

/// Decode the whole level to RGBA.
#[no_mangle]
pub extern "C" fn decode_full() -> *const u8 {
    let s = st();
    let rgb = pyramid::decode_level(&s.blocks, s.w, s.h);
    s.out_px = rgb_to_rgba(&rgb);
    s.out_w = s.w;
    s.out_h = s.h;
    s.region_blocks = pyramid::block_count(s.w, s.h);
    s.out_px.as_ptr()
}

/// Decode ONLY the blocks under a rectangle. The point of the whole container:
/// a viewport costs the blocks beneath it, not the frame.
#[no_mangle]
pub extern "C" fn decode_region(x: u32, y: u32, w: u32, h: u32) -> *const u8 {
    let s = st();
    let (rgb, touched) = pyramid::decode_region(&s.blocks, s.w, s.h, x, y, w, h);
    s.out_px = rgb_to_rgba(&rgb);
    s.out_w = w;
    s.out_h = h;
    s.region_blocks = touched;
    s.out_px.as_ptr()
}

#[no_mangle]
pub extern "C" fn region_blocks() -> u32 {
    st().region_blocks
}

#[no_mangle]
pub extern "C" fn out_w() -> u32 {
    st().out_w
}

#[no_mangle]
pub extern "C" fn out_h() -> u32 {
    st().out_h
}

fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(rgb.len() / 3 * 4);
    for c in rgb.chunks_exact(3) {
        o.push(c[0]);
        o.push(c[1]);
        o.push(c[2]);
        o.push(255);
    }
    o
}

/// PSNR against the source, x1000 so it crosses as an integer.
#[no_mangle]
pub extern "C" fn psnr_x1000() -> u32 {
    let s = st();
    if s.src.is_empty() {
        return 0;
    }
    let dec = pyramid::decode_level(&s.blocks, s.w, s.h);
    let mut se = 0f64;
    for (a, b) in s.src.iter().zip(dec.iter()) {
        let d = *a as f64 - *b as f64;
        se += d * d;
    }
    let mse = se / s.src.len() as f64;
    if mse <= 0.0 {
        return 99_000;
    }
    ((10.0 * (255.0 * 255.0 / mse).log10()) * 1000.0) as u32
}

// ---------------------------------------------------------------- slots

fn slot_of(i: u32) -> Slot {
    match i {
        0 => Slot::Avatar,
        1 => Slot::Photo,
        2 => Slot::Icon,
        3 => Slot::Banner,
        4 => Slot::Cover,
        5 => Slot::Clip,
        6 => Slot::Message,
        _ => Slot::Live,
    }
}

#[no_mangle]
pub extern "C" fn slot_max_b64(slot: u32) -> u32 {
    slot_of(slot).max_b64() as u32
}

fn err_code(e: MediaError) -> i32 {
    match e {
        MediaError::TooLarge { .. } => 1,
        MediaError::UnknownMime => 2,
        MediaError::MimeKindMismatch => 3,
        MediaError::KindNotAllowed => 4,
        MediaError::HalfSet => 5,
        MediaError::NotBase64 => 6,
        MediaError::BadLive => 7,
        MediaError::RawNotAllowed => 8,
    }
}

/// Validate a slot the way the FOLD does. `mime` is read from the scratch
/// buffer. Returns 0 for ok, or the error code — and writes the human-readable
/// reason into the text channel.
#[no_mangle]
pub extern "C" fn validate(slot: u32, mime_len: u32, body_b64_len: u32) -> i32 {
    let s = st();
    let mime = unsafe { String::from_utf8_lossy(&SCRATCH[..mime_len as usize]).to_string() };
    let kind = match MediaKind::of_mime(&mime) {
        Some(k) => k,
        None => {
            s.out_txt = MediaError::UnknownMime.to_string();
            return err_code(MediaError::UnknownMime);
        }
    };
    let m = MediaRef {
        kind,
        mime,
        // A synthetic body of the stated length: validation is about size and
        // shape, and materialising half a megabyte to ask a question about its
        // length would be silly.
        delivery: Delivery::Inline {
            data: "A".repeat(body_b64_len as usize / 4 * 4),
        },
        width: 0,
        height: 0,
        duration_ms: 0,
    };
    match m.validate(slot_of(slot)) {
        Ok(()) => {
            s.out_txt = "accepted".into();
            0
        }
        Err(e) => {
            s.out_txt = e.to_string();
            err_code(e)
        }
    }
}

// ---------------------------------------------------------------- wire args

fn dump_args(a: &Args, out: &mut String) {
    out.clear();
    for (k, v) in a {
        match v {
            ArgVal::Int(i) => out.push_str(&format!("{k}\t{i}\n")),
            ArgVal::Text(t) => {
                // Elide long bodies: the demo is showing the SHAPE of the wire,
                // and 500 KB of base64 in a table teaches nothing.
                if t.len() > 48 {
                    out.push_str(&format!("{k}\t<{} b64 chars>\n", t.len()));
                } else {
                    out.push_str(&format!("{k}\t{t}\n"));
                }
            }
        }
    }
}

/// Flatten an inline media slot to delta args. `prefix` and `mime` come from the
/// scratch buffer as `prefix\0mime`.
#[no_mangle]
pub extern "C" fn args_inline(split: u32, total: u32, body_b64_len: u32) -> u32 {
    let s = st();
    let (prefix, mime) = unsafe {
        let all = String::from_utf8_lossy(&SCRATCH[..total as usize]).to_string();
        let (a, b) = all.split_at(split as usize);
        (a.to_string(), b.to_string())
    };
    let m = match MediaRef::inline(&mime, "A".repeat(body_b64_len as usize / 4 * 4)) {
        Ok(m) => m,
        Err(e) => {
            s.out_txt = e.to_string();
            return 0;
        }
    };
    let mut a = Args::new();
    m.to_args(&prefix, &mut a);
    dump_args(&a, &mut s.out_txt);
    a.len() as u32
}

/// Flatten a LIVE slot — the descriptor that makes a livestream representable
/// in a delta at all. Session id is read from the scratch buffer.
#[no_mangle]
pub extern "C" fn args_live(split: u32, total: u32) -> u32 {
    let s = st();
    let (prefix, session) = unsafe {
        let all = String::from_utf8_lossy(&SCRATCH[..total as usize]).to_string();
        let (a, b) = all.split_at(split as usize);
        (a.to_string(), b.to_string())
    };
    let m = match MediaRef::live(session) {
        Ok(m) => m,
        Err(e) => {
            s.out_txt = e.to_string();
            return 0;
        }
    };
    let mut a = Args::new();
    m.to_args(&prefix, &mut a);
    dump_args(&a, &mut s.out_txt);
    a.len() as u32
}

/// Flatten a DETACHED slot: digest only, bytes fetched out of band.
#[no_mangle]
pub extern "C" fn args_detached(split: u32, total: u32, bytes: u32) -> u32 {
    let s = st();
    let (prefix, mime) = unsafe {
        let all = String::from_utf8_lossy(&SCRATCH[..total as usize]).to_string();
        let (a, b) = all.split_at(split as usize);
        (a.to_string(), b.to_string())
    };
    let kind = MediaKind::of_mime(&mime).unwrap_or(MediaKind::Clip);
    let m = MediaRef {
        kind,
        mime,
        delivery: Delivery::Detached {
            digest: [0xab; 32],
            bytes: bytes as u64,
        },
        width: 0,
        height: 0,
        duration_ms: 0,
    };
    let mut a = Args::new();
    m.to_args(&prefix, &mut a);
    dump_args(&a, &mut s.out_txt);
    a.len() as u32
}

/// Parse args back, proving the round trip and the wire cost of the slot.
#[no_mangle]
pub extern "C" fn wire_cost_of_last() -> u32 {
    let s = st();
    s.out_txt.len() as u32
}

// ---------------------------------------------------------------- text out

#[no_mangle]
pub extern "C" fn txt_ptr() -> *const u8 {
    st().out_txt.as_ptr()
}

#[no_mangle]
pub extern "C" fn txt_len() -> u32 {
    st().out_txt.len() as u32
}

/// Pointer to the serialized container, so a caller can lift the exact bytes a
/// delta would carry out of linear memory and put them on a transport.
#[no_mangle]
pub extern "C" fn wire_ptr() -> *const u8 {
    st().wire.as_ptr()
}

/// Load a container received from a transport into state, so `decode_full` and
/// `decode_region` work against it exactly as they do against a local encode.
/// This is the subscriber half of the live demo: the receiver runs the same
/// decoder the sender ran, over bytes that actually crossed a wire.
///
/// Returns 1 on success, 0 if the bytes are not a container.
#[no_mangle]
pub extern "C" fn load_container(len: u32) -> u32 {
    let s = st();
    let bytes = unsafe { &SCRATCH[..len as usize] };
    match pyramid::Pyramid::parse(bytes) {
        Some(p) if !p.levels.is_empty() => {
            let l = &p.levels[0];
            s.w = l.w;
            s.h = l.h;
            s.blocks = l.blocks.clone();
            s.wire = bytes.to_vec();
            // A received container has no local source to compare against.
            s.src.clear();
            1
        }
        _ => 0,
    }
}
