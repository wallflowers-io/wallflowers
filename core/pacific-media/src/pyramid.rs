//! Mip-pyramid container over the block codec.
//!
//! The wire carries the bottom of the pyramid. Zooming past the base level is
//! not "scale the image up" -- it is a level the container simply does not have,
//! which is precisely the seam a learned decoder stage plugs into later.

use crate::bc1::{decode_block, encode_block, BLOCK_BYTES};

pub const MAGIC: &[u8; 4] = b"LOFI";
pub const VERSION: u8 = 1;

pub struct Level {
    pub w: u32,
    pub h: u32,
    pub blocks: Vec<u8>,
}

pub struct Pyramid {
    pub levels: Vec<Level>,
}

fn blocks_for(w: u32, h: u32) -> (u32, u32) {
    ((w + 3) / 4, (h + 3) / 4)
}

/// Encode one RGB level. Edges replicate into the block padding so partial
/// blocks do not drag a dark fringe along the right and bottom.
pub fn encode_level(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (bw, bh) = blocks_for(w, h);
    let mut out = Vec::with_capacity((bw * bh) as usize * BLOCK_BYTES);
    for by in 0..bh {
        for bx in 0..bw {
            let mut px = [[0u8; 3]; 16];
            for y in 0..4u32 {
                for x in 0..4u32 {
                    let sx = (bx * 4 + x).min(w - 1);
                    let sy = (by * 4 + y).min(h - 1);
                    let o = ((sy * w + sx) * 3) as usize;
                    px[(y * 4 + x) as usize] = [rgb[o], rgb[o + 1], rgb[o + 2]];
                }
            }
            out.extend_from_slice(&encode_block(&px));
        }
    }
    out
}

pub fn decode_level(blocks: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (bw, bh) = blocks_for(w, h);
    let mut out = vec![0u8; (w * h * 3) as usize];
    for by in 0..bh {
        for bx in 0..bw {
            let bi = ((by * bw + bx) as usize) * BLOCK_BYTES;
            let px = decode_block(&blocks[bi..bi + BLOCK_BYTES]);
            for y in 0..4u32 {
                for x in 0..4u32 {
                    let dx = bx * 4 + x;
                    let dy = by * 4 + y;
                    if dx >= w || dy >= h {
                        continue;
                    }
                    let o = ((dy * w + dx) * 3) as usize;
                    let c = px[(y * 4 + x) as usize];
                    out[o] = c[0];
                    out[o + 1] = c[1];
                    out[o + 2] = c[2];
                }
            }
        }
    }
    out
}

/// Box-filter halving. Deliberately plain: a sharper kernel would fight the
/// block quantiser and buy nothing at this rate.
pub fn halve(rgb: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; (nw * nh * 3) as usize];
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..3usize {
                let mut acc = 0u32;
                for dy in 0..2u32 {
                    for dx in 0..2u32 {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        acc += rgb[((sy * w + sx) * 3) as usize + c] as u32;
                    }
                }
                out[((y * nw + x) * 3) as usize + c] = (acc / 4) as u8;
            }
        }
    }
    (out, nw, nh)
}

impl Pyramid {
    /// Build from a base-resolution RGB buffer, halving down to 4x4.
    pub fn build(rgb: &[u8], w: u32, h: u32) -> Self {
        let mut levels = Vec::new();
        let (mut cur, mut cw, mut ch) = (rgb.to_vec(), w, h);
        loop {
            levels.push(Level {
                w: cw,
                h: ch,
                blocks: encode_level(&cur, cw, ch),
            });
            if cw <= 4 || ch <= 4 {
                break;
            }
            let (n, nw, nh) = halve(&cur, cw, ch);
            cur = n;
            cw = nw;
            ch = nh;
        }
        Pyramid { levels }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(self.levels.len() as u8);
        let b = &self.levels[0];
        out.extend_from_slice(&(b.w as u16).to_le_bytes());
        out.extend_from_slice(&(b.h as u16).to_le_bytes());
        for l in &self.levels {
            out.extend_from_slice(&l.blocks);
        }
        out
    }

    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 10 || &buf[0..4] != MAGIC || buf[4] != VERSION {
            return None;
        }
        let n = buf[5] as usize;
        let mut w = u16::from_le_bytes([buf[6], buf[7]]) as u32;
        let mut h = u16::from_le_bytes([buf[8], buf[9]]) as u32;
        let mut off = 10;
        let mut levels = Vec::with_capacity(n);
        for _ in 0..n {
            let (bw, bh) = blocks_for(w, h);
            let sz = (bw * bh) as usize * BLOCK_BYTES;
            levels.push(Level {
                w,
                h,
                blocks: buf[off..off + sz].to_vec(),
            });
            off += sz;
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        Some(Pyramid { levels })
    }
}

/// Decode ONLY the blocks a rectangle touches, and report how many that was.
///
/// This is the property the whole container shape exists for. A transform codec
/// has to run the entropy decoder from the start of the frame to reach any
/// pixel, so "show me this crop at this zoom" costs a full decode. Here a
/// viewport costs the blocks under it and nothing else, which is what makes
/// pinch-to-zoom a partial decode rather than a full one.
///
/// Returns the RGB buffer for the requested rect and the number of 4x4 blocks
/// that had to be touched to produce it.
pub fn decode_region(
    blocks: &[u8],
    w: u32,
    h: u32,
    rx: u32,
    ry: u32,
    rw: u32,
    rh: u32,
) -> (Vec<u8>, u32) {
    let (bw, _bh) = blocks_for(w, h);
    let rx1 = (rx + rw).min(w);
    let ry1 = (ry + rh).min(h);
    let mut out = vec![0u8; (rw * rh * 3) as usize];
    if rx >= w || ry >= h {
        return (out, 0);
    }

    // Only the block rows/cols the rect overlaps.
    let bx0 = rx / 4;
    let by0 = ry / 4;
    let bx1 = (rx1 + 3) / 4;
    let by1 = (ry1 + 3) / 4;
    let mut touched = 0u32;

    for by in by0..by1 {
        for bx in bx0..bx1 {
            let bi = ((by * bw + bx) as usize) * BLOCK_BYTES;
            if bi + BLOCK_BYTES > blocks.len() {
                continue;
            }
            let px = decode_block(&blocks[bi..bi + BLOCK_BYTES]);
            touched += 1;
            for y in 0..4u32 {
                for x in 0..4u32 {
                    let sx = bx * 4 + x;
                    let sy = by * 4 + y;
                    if sx < rx || sy < ry || sx >= rx1 || sy >= ry1 {
                        continue;
                    }
                    let o = (((sy - ry) * rw + (sx - rx)) * 3) as usize;
                    let c = px[(y * 4 + x) as usize];
                    out[o] = c[0];
                    out[o + 1] = c[1];
                    out[o + 2] = c[2];
                }
            }
        }
    }
    (out, touched)
}

/// Total blocks in a level — the denominator for "how much did the viewport
/// actually cost".
pub fn block_count(w: u32, h: u32) -> u32 {
    let (bw, bh) = blocks_for(w, h);
    bw * bh
}

/// Area-average resample to an arbitrary size.
///
/// Deliberately a box filter. A sharper kernel would inject ringing that the
/// block quantiser then has to spend endpoints representing, so it costs bytes
/// to look worse.
pub fn resample_area(rgb: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (nw * nh * 3) as usize];
    if w == 0 || h == 0 || nw == 0 || nh == 0 {
        return out;
    }
    for oy in 0..nh {
        let y0 = oy * h / nh;
        let y1 = (((oy + 1) * h + nh - 1) / nh).max(y0 + 1).min(h);
        for ox in 0..nw {
            let x0 = ox * w / nw;
            let x1 = (((ox + 1) * w + nw - 1) / nw).max(x0 + 1).min(w);
            let mut acc = [0u32; 3];
            let mut n = 0u32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let o = ((sy * w + sx) * 3) as usize;
                    acc[0] += rgb[o] as u32;
                    acc[1] += rgb[o + 1] as u32;
                    acc[2] += rgb[o + 2] as u32;
                    n += 1;
                }
            }
            let o = ((oy * nw + ox) * 3) as usize;
            if n > 0 {
                out[o] = (acc[0] / n) as u8;
                out[o + 1] = (acc[1] / n) as u8;
                out[o + 2] = (acc[2] / n) as u8;
            }
        }
    }
    out
}
