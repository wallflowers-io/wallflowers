//! Fixed-rate 4x4 block colour codec.
//!
//! Layout is the published S3TC/BC1 one: 8 bytes per 4x4 block, holding two
//! RGB565 endpoints followed by sixteen 2-bit palette indices. Endpoint order
//! selects the mode -- c0 > c1 gives four colours (two endpoints, two
//! interpolants), c0 <= c1 gives three colours plus a spare slot.
//!
//! The reason this shape is worth having over a transform codec: every block
//! decodes independently of every other block, so a viewport at a given zoom
//! decodes only the blocks it actually touches. That is the whole pinch-to-zoom
//! story, and it is a property DCT/wavelet codecs do not have.

pub const BLOCK_BYTES: usize = 8;

/// Luma-ish channel weights (x256). Faces live in a narrow chroma band, so
/// weighting error toward green tracks perceived skin-shading error far better
/// than a flat RGB metric.
const WR: i32 = 76;
const WG: i32 = 150;
const WB: i32 = 29;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb565(pub u16);

impl Rgb565 {
    /// Round to the NEAREST representable 565 value, not the truncation.
    /// Truncating (`>>3`) biases every candidate endpoint darker by up to 4/255
    /// per channel, which on skin gradients is a systematic shift rather than
    /// noise -- and it makes the search start from a worse place than it needs to.
    pub fn from_rgb(c: [u8; 3]) -> Self {
        let q = |v: u8, levels: u16| ((v as u16 * levels + 127) / 255).min(levels);
        Rgb565((q(c[0], 31) << 11) | (q(c[1], 63) << 5) | q(c[2], 31))
    }
    /// 565 -> 888 with high-bit replication, which is what decoders do.
    pub fn to_rgb(self) -> [u8; 3] {
        let (r, g, b) = ((self.0 >> 11) & 0x1f, (self.0 >> 5) & 0x3f, self.0 & 0x1f);
        [
            ((r << 3) | (r >> 2)) as u8,
            ((g << 2) | (g >> 4)) as u8,
            ((b << 3) | (b >> 2)) as u8,
        ]
    }
    fn parts(self) -> [i32; 3] {
        [
            ((self.0 >> 11) & 0x1f) as i32,
            ((self.0 >> 5) & 0x3f) as i32,
            (self.0 & 0x1f) as i32,
        ]
    }
    fn from_parts(p: [i32; 3]) -> Self {
        let r = p[0].clamp(0, 31) as u16;
        let g = p[1].clamp(0, 63) as u16;
        let b = p[2].clamp(0, 31) as u16;
        Rgb565((r << 11) | (g << 5) | b)
    }
}

#[inline]
fn err(a: [u8; 3], b: [u8; 3]) -> i32 {
    let dr = a[0] as i32 - b[0] as i32;
    let dg = a[1] as i32 - b[1] as i32;
    let db = a[2] as i32 - b[2] as i32;
    (WR * dr * dr + WG * dg * dg + WB * db * db) >> 8
}

fn lerp(a: [u8; 3], b: [u8; 3], num: u32, den: u32) -> [u8; 3] {
    let mut o = [0u8; 3];
    for i in 0..3 {
        let av = a[i] as u32 * (den - num);
        let bv = b[i] as u32 * num;
        o[i] = ((av + bv + den / 2) / den) as u8;
    }
    o
}

/// Four-colour palette: both endpoints plus the 1/3 and 2/3 interpolants.
pub fn palette4(c0: Rgb565, c1: Rgb565) -> [[u8; 3]; 4] {
    let (a, b) = (c0.to_rgb(), c1.to_rgb());
    [a, b, lerp(a, b, 1, 3), lerp(a, b, 2, 3)]
}

/// Three-colour palette: both endpoints plus the midpoint.
pub fn palette3(c0: Rgb565, c1: Rgb565) -> [[u8; 3]; 3] {
    let (a, b) = (c0.to_rgb(), c1.to_rgb());
    [a, b, lerp(a, b, 1, 2)]
}

/// Best achievable error for these endpoints in the given mode, plus indices.
fn fit(px: &[[u8; 3]; 16], pal: &[[u8; 3]], out: &mut [u8; 16]) -> i32 {
    let mut total = 0;
    for (i, p) in px.iter().enumerate() {
        let mut best = i32::MAX;
        let mut bi = 0u8;
        for (j, q) in pal.iter().enumerate() {
            let e = err(*p, *q);
            if e < best {
                best = e;
                bi = j as u8;
            }
        }
        out[i] = bi;
        total += best;
    }
    total
}

#[derive(Clone, Copy)]
struct Fit {
    c0: Rgb565,
    c1: Rgb565,
    four: bool,
    error: i32,
}

fn eval(px: &[[u8; 3]; 16], c0: Rgb565, c1: Rgb565, four: bool) -> i32 {
    let mut idx = [0u8; 16];
    if four {
        fit(px, &palette4(c0, c1), &mut idx)
    } else {
        fit(px, &palette3(c0, c1), &mut idx)
    }
}

/// Local endpoint refinement: walk each 565 component of each endpoint by a
/// small delta and keep anything that lowers error. Cheap, and it recovers most
/// of what an exhaustive endpoint search would find.
fn refine(px: &[[u8; 3]; 16], mut f: Fit) -> Fit {
    for _ in 0..2 {
        let mut improved = false;
        for ch in 0..3 {
            for which in 0..2 {
                for d in [-2i32, -1, 1, 2] {
                    let mut c0 = f.c0;
                    let mut c1 = f.c1;
                    let target = if which == 0 { &mut c0 } else { &mut c1 };
                    let mut p = target.parts();
                    p[ch] += d;
                    *target = Rgb565::from_parts(p);
                    let e = eval(px, c0, c1, f.four);
                    if e < f.error {
                        f.c0 = c0;
                        f.c1 = c1;
                        f.error = e;
                        improved = true;
                    }
                }
            }
        }
        if !improved {
            break;
        }
    }
    f
}

/// Emit a block, fixing up endpoint order so the stored pair selects the
/// intended mode, and remapping indices to match the swap.
fn emit(c0: Rgb565, c1: Rgb565, four: bool, idx: &[u8; 16]) -> [u8; BLOCK_BYTES] {
    let (a, b, swap) = if four {
        if c0.0 > c1.0 {
            (c0, c1, false)
        } else {
            (c1, c0, true)
        }
    } else if c0.0 <= c1.0 {
        (c0, c1, false)
    } else {
        (c1, c0, true)
    };

    let mut out = [0u8; BLOCK_BYTES];
    out[0] = (a.0 & 0xff) as u8;
    out[1] = (a.0 >> 8) as u8;
    out[2] = (b.0 & 0xff) as u8;
    out[3] = (b.0 >> 8) as u8;

    for i in 0..16 {
        let mut v = idx[i];
        if swap {
            v = if four {
                match v {
                    0 => 1,
                    1 => 0,
                    2 => 3,
                    _ => 2,
                }
            } else {
                match v {
                    0 => 1,
                    1 => 0,
                    other => other,
                }
            };
        }
        out[4 + i / 4] |= v << ((i % 4) * 2);
    }
    out
}

/// Encode one 4x4 block. Trivial blocks short-circuit; the rest get an
/// exhaustive search over endpoint pairs drawn from the block's own colours in
/// both modes, then local refinement of whichever won.
pub fn encode_block(px: &[[u8; 3]; 16]) -> [u8; BLOCK_BYTES] {
    let mut uniq: Vec<Rgb565> = Vec::with_capacity(16);
    for p in px.iter() {
        let q = Rgb565::from_rgb(*p);
        if !uniq.contains(&q) {
            uniq.push(q);
        }
    }

    if uniq.len() == 1 {
        // Flat block: one endpoint, every index 0. Costs nothing to detect and
        // is common in backgrounds and blown-out highlights.
        return emit(uniq[0], uniq[0], false, &[0u8; 16]);
    }

    let mut best = Fit {
        c0: uniq[0],
        c1: uniq[1],
        four: true,
        error: i32::MAX,
    };

    for i in 0..uniq.len() {
        for j in (i + 1)..uniq.len() {
            for four in [true, false] {
                let e = eval(px, uniq[i], uniq[j], four);
                if e < best.error {
                    best = Fit {
                        c0: uniq[i],
                        c1: uniq[j],
                        four,
                        error: e,
                    };
                }
            }
        }
    }

    best = refine(px, best);

    let mut idx = [0u8; 16];
    if best.four {
        fit(px, &palette4(best.c0, best.c1), &mut idx);
    } else {
        fit(px, &palette3(best.c0, best.c1), &mut idx);
    }
    emit(best.c0, best.c1, best.four, &idx)
}

/// Decode one 4x4 block back to 16 RGB triples.
pub fn decode_block(blk: &[u8]) -> [[u8; 3]; 16] {
    let c0 = Rgb565(blk[0] as u16 | ((blk[1] as u16) << 8));
    let c1 = Rgb565(blk[2] as u16 | ((blk[3] as u16) << 8));
    let four = c0.0 > c1.0;
    let p4 = palette4(c0, c1);
    let p3 = palette3(c0, c1);

    let mut out = [[0u8; 3]; 16];
    for i in 0..16 {
        let v = (blk[4 + i / 4] >> ((i % 4) * 2)) & 0x3;
        out[i] = if four {
            p4[v as usize]
        } else if v < 3 {
            p3[v as usize]
        } else {
            // Spare slot in three-colour mode. Opaque black in the classic
            // layout; we never emit it, but a decoder must handle it.
            [0, 0, 0]
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(c: [u8; 3]) -> [[u8; 3]; 16] {
        [c; 16]
    }

    #[test]
    fn flat_block_round_trips_to_565_grid() {
        // A flat block must come back exactly as its 565 quantisation -- any
        // drift here means the endpoint or index path is wrong.
        for c in [[0, 0, 0], [255, 255, 255], [137, 92, 71], [12, 200, 40]] {
            let want = Rgb565::from_rgb(c).to_rgb();
            let got = decode_block(&encode_block(&flat(c)));
            for p in got.iter() {
                assert_eq!(*p, want, "flat {c:?}");
            }
        }
    }

    #[test]
    fn two_colour_block_is_exact_on_the_565_grid() {
        // Two distinct colours fit in either mode as literal endpoints, so the
        // only loss allowed is the 565 quantisation itself.
        let a = [200u8, 40, 40];
        let b = [40u8, 40, 200];
        let mut px = flat(a);
        for i in 8..16 {
            px[i] = b;
        }
        let got = decode_block(&encode_block(&px));
        // Exact means "nearest representable on the 565 grid", not the naive
        // truncation -- the encoder is allowed to be better than truncation.
        let (qa, qb) = (Rgb565::from_rgb(a).to_rgb(), Rgb565::from_rgb(b).to_rgb());
        for i in 0..8 {
            assert_eq!(got[i], qa);
        }
        for i in 8..16 {
            assert_eq!(got[i], qb);
        }
    }

    #[test]
    fn endpoint_order_selects_the_intended_mode() {
        // The stored pair encodes the mode. If emit() gets the ordering or the
        // index remap wrong, decode silently reads the other palette.
        let mut px = flat([10, 10, 10]);
        for i in 0..16 {
            let v = (i * 16) as u8;
            px[i] = [v, v, v];
        }
        let blk = encode_block(&px);
        let c0 = Rgb565(blk[0] as u16 | ((blk[1] as u16) << 8));
        let c1 = Rgb565(blk[2] as u16 | ((blk[3] as u16) << 8));
        let four = c0.0 > c1.0;
        let decoded = decode_block(&blk);
        let pal4 = palette4(c0, c1);
        let pal3 = palette3(c0, c1);
        for p in decoded.iter() {
            let ok = if four {
                pal4.contains(p)
            } else {
                pal3.contains(p) || *p == [0, 0, 0]
            };
            assert!(ok, "decoded {p:?} not in the selected palette");
        }
    }

    #[test]
    fn gradient_error_beats_naive_minmax_endpoints() {
        // The search has to earn its cost. Compare against the usual heuristic
        // of taking the darkest and lightest pixel as endpoints.
        let mut px = flat([0, 0, 0]);
        for i in 0..16 {
            px[i] = [(i * 17) as u8, (255 - i * 15) as u8, 128];
        }
        let searched = decode_block(&encode_block(&px));

        let mut lo = px[0];
        let mut hi = px[0];
        for p in px.iter() {
            let l = |c: [u8; 3]| c[0] as u32 + c[1] as u32 + c[2] as u32;
            if l(*p) < l(lo) {
                lo = *p;
            }
            if l(*p) > l(hi) {
                hi = *p;
            }
        }
        let naive_pal = palette4(Rgb565::from_rgb(hi), Rgb565::from_rgb(lo));
        let mut naive_err = 0i64;
        let mut searched_err = 0i64;
        for (i, p) in px.iter().enumerate() {
            let best = naive_pal.iter().map(|q| err(*p, *q)).min().unwrap();
            naive_err += best as i64;
            searched_err += err(*p, searched[i]) as i64;
        }
        assert!(
            searched_err <= naive_err,
            "search {searched_err} worse than naive min/max {naive_err}"
        );
    }
}
