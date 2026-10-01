//! ANIM — looping media, as a sequence of block patches.
//!
//! This is what a Live Photo becomes. The motion half of a Live Photo is a short,
//! mostly-static clip: the camera barely moves, and a subject shifts a little in
//! the middle of the frame. Encoding every frame in full would pay full price for
//! a picture that is largely the same picture.
//!
//! So frame 0 is complete and every later frame carries ONLY the 4x4 blocks that
//! actually changed — conditional replenishment, the oldest trick in video
//! conferencing, and the same instinct as the ATE texture format that let one
//! palette serve a whole animation instead of one per frame.
//!
//! # Why the comparison is against what is on screen
//!
//! A block is re-sent when it drifts too far from **the block the viewer is
//! currently looking at**, not from the previous frame's source pixels. Comparing
//! source-to-source lets error accumulate silently: a slow gradual change never
//! trips the threshold on any single step while wandering arbitrarily far from
//! what is displayed. Comparing against the displayed block bounds the drift.

use crate::bc1::{decode_block, encode_block, BLOCK_BYTES};

pub const ANIM_MAGIC: &[u8; 4] = b"LOFA";
pub const ANIM_VERSION: u8 = 1;

/// Header is magic(4) + version(1) + frames(1) + w(2) + h(2) + fps(1) = 11.
const HEADER: usize = 11;

fn blocks_wide(w: u32) -> u32 {
    w.div_ceil(4)
}
fn blocks_total(w: u32, h: u32) -> usize {
    (blocks_wide(w) * h.div_ceil(4)) as usize
}

/// Squared error between a 4x4 source patch and a decoded block, luma-weighted
/// to match the encoder's own metric.
fn patch_error(src: &[[u8; 3]; 16], blk: &[u8]) -> i64 {
    let dec = decode_block(blk);
    let mut acc = 0i64;
    for i in 0..16 {
        let dr = src[i][0] as i64 - dec[i][0] as i64;
        let dg = src[i][1] as i64 - dec[i][1] as i64;
        let db = src[i][2] as i64 - dec[i][2] as i64;
        acc += (76 * dr * dr + 150 * dg * dg + 29 * db * db) >> 8;
    }
    acc
}

fn gather(rgb: &[u8], w: u32, h: u32, bx: u32, by: u32) -> [[u8; 3]; 16] {
    let mut px = [[0u8; 3]; 16];
    for y in 0..4u32 {
        for x in 0..4u32 {
            let sx = (bx * 4 + x).min(w - 1);
            let sy = (by * 4 + y).min(h - 1);
            let o = ((sy * w + sx) * 3) as usize;
            px[(y * 4 + x) as usize] = [rgb[o], rgb[o + 1], rgb[o + 2]];
        }
    }
    px
}

/// Encode a frame sequence. `frames` are RGB buffers, all `w`x`h`.
///
/// `threshold` is the per-block squared-error budget before a block is re-sent.
/// Zero re-sends anything that is not bit-identical; larger values trade motion
/// fidelity for bytes, which for a Live Photo loop is usually the right trade.
pub fn encode_anim(frames: &[Vec<u8>], w: u32, h: u32, fps: u8, threshold: i64) -> Option<Vec<u8>> {
    if frames.is_empty() || frames.len() > 255 || w == 0 || h == 0 {
        return None;
    }
    let nblocks = blocks_total(w, h);
    // Block indices travel as u16, which is the practical ceiling on frame size
    // here (a 1024x1024 frame is 65,536 blocks and would not fit a delta anyway).
    if nblocks > u16::MAX as usize {
        return None;
    }
    let bw = blocks_wide(w);

    let mut out = Vec::with_capacity(HEADER + nblocks * BLOCK_BYTES);
    out.extend_from_slice(ANIM_MAGIC);
    out.push(ANIM_VERSION);
    out.push(frames.len() as u8);
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    out.push(fps);

    // Frame 0 in full. This is also the state every later frame patches.
    let mut shown = vec![0u8; nblocks * BLOCK_BYTES];
    for bi in 0..nblocks {
        let (bx, by) = (bi as u32 % bw, bi as u32 / bw);
        let blk = encode_block(&gather(&frames[0], w, h, bx, by));
        shown[bi * BLOCK_BYTES..(bi + 1) * BLOCK_BYTES].copy_from_slice(&blk);
    }
    out.extend_from_slice(&shown);

    for f in &frames[1..] {
        let mut patches: Vec<(u16, [u8; BLOCK_BYTES])> = Vec::new();
        for bi in 0..nblocks {
            let (bx, by) = (bi as u32 % bw, bi as u32 / bw);
            let src = gather(f, w, h, bx, by);
            let cur = &shown[bi * BLOCK_BYTES..(bi + 1) * BLOCK_BYTES];
            if patch_error(&src, cur) <= threshold {
                continue; // close enough to what is already on screen
            }
            let blk = encode_block(&src);
            if blk[..] == cur[..] {
                continue; // re-encoded to the same thing; nothing to send
            }
            patches.push((bi as u16, blk));
        }
        out.extend_from_slice(&(patches.len() as u16).to_le_bytes());
        for (bi, blk) in &patches {
            out.extend_from_slice(&bi.to_le_bytes());
            out.extend_from_slice(blk);
            shown[*bi as usize * BLOCK_BYTES..(*bi as usize + 1) * BLOCK_BYTES]
                .copy_from_slice(blk);
        }
    }
    Some(out)
}

/// A parsed animation. Frames are materialised by replaying patches, which for a
/// loop of a couple of dozen frames is cheaper than storing each one.
pub struct Anim {
    pub w: u32,
    pub h: u32,
    pub fps: u8,
    pub frames: usize,
    data: Vec<u8>,
}

impl Anim {
    pub fn parse(buf: &[u8]) -> Option<Anim> {
        if buf.len() < HEADER || &buf[0..4] != ANIM_MAGIC || buf[4] != ANIM_VERSION {
            return None;
        }
        let frames = buf[5] as usize;
        let w = u16::from_le_bytes([buf[6], buf[7]]) as u32;
        let h = u16::from_le_bytes([buf[8], buf[9]]) as u32;
        let fps = buf[10];
        if frames == 0 || w == 0 || h == 0 {
            return None;
        }
        if buf.len() < HEADER + blocks_total(w, h) * BLOCK_BYTES {
            return None;
        }
        Some(Anim {
            w,
            h,
            fps,
            frames,
            data: buf.to_vec(),
        })
    }

    /// Block plane for frame `index`, replayed from frame 0.
    pub fn blocks_at(&self, index: usize) -> Option<Vec<u8>> {
        let nblocks = blocks_total(self.w, self.h);
        let mut shown = self.data[HEADER..HEADER + nblocks * BLOCK_BYTES].to_vec();
        let mut off = HEADER + nblocks * BLOCK_BYTES;
        let want = index.min(self.frames.saturating_sub(1));

        for _ in 0..want {
            if off + 2 > self.data.len() {
                return None;
            }
            let n = u16::from_le_bytes([self.data[off], self.data[off + 1]]) as usize;
            off += 2;
            for _ in 0..n {
                if off + 2 + BLOCK_BYTES > self.data.len() {
                    return None;
                }
                let bi =
                    u16::from_le_bytes([self.data[off], self.data[off + 1]]) as usize;
                off += 2;
                if bi >= nblocks {
                    return None;
                }
                shown[bi * BLOCK_BYTES..(bi + 1) * BLOCK_BYTES]
                    .copy_from_slice(&self.data[off..off + BLOCK_BYTES]);
                off += BLOCK_BYTES;
            }
        }
        Some(shown)
    }

    /// How many blocks frame `index` re-sent. The number that says whether
    /// conditional replenishment is earning its keep on this clip.
    pub fn patch_count(&self, index: usize) -> Option<usize> {
        if index == 0 || index >= self.frames {
            return None;
        }
        let nblocks = blocks_total(self.w, self.h);
        let mut off = HEADER + nblocks * BLOCK_BYTES;
        for f in 1..=index {
            if off + 2 > self.data.len() {
                return None;
            }
            let n = u16::from_le_bytes([self.data[off], self.data[off + 1]]) as usize;
            if f == index {
                return Some(n);
            }
            off += 2 + n * (2 + BLOCK_BYTES);
        }
        None
    }
}
