//! SpaceCalibration — the actual mathematics of the embedding space, made
//! explicit (8 Aug ruling: "ground yourself in the actual mathematics of
//! embedding models").
//!
//! A trained sentence-embedding space is ANISOTROPIC: vectors live in a narrow
//! cone around a common direction, and the cosine between two UNRELATED texts is
//! not 0 — it is whatever the model's geometry makes it (typically 0.4–0.7).
//! Consequences, if ignored:
//!   · a mean of unit vectors points mostly at the corpus common direction, so
//!     every stream's "heading" collapses toward the same place;
//!   · resultant length (coherence) saturates high for ANY set — buffeting and
//!     a beam become indistinguishable;
//!   · raw cosines to anchors compress into a narrow band whose absolute values
//!     mean nothing; only their differences carry signal.
//!
//! The corrections here are the standard ones:
//!   CENTER      subtract the corpus mean of unit vectors (removes the cone's
//!               axis), optionally deflate the top principal component(s) —
//!               "all-but-the-top" (Mu & Viswanath 2018). Deflation is powerful
//!               and dangerous in equal measure: on a small, structured corpus
//!               the top PC can BE the signal, so `components` defaults to 0
//!               (mean-only) and is a deliberate dial, not a reflex.
//!   CALIBRATE   estimate the null: the mean and spread of centered cosines
//!               between random pairs of THIS corpus. Every reported alignment
//!               is then a z-score against unrelatedness, squashed to [0,1] by
//!               a logistic — 0.5 literally means "as similar as two unrelated
//!               texts here", which raw cosine never meant.
//!
//! Everything is deterministic: the PC power iteration and the null-pair sample
//! are SplitMix64-seeded from the corpus shape, so the same corpus fits the same
//! calibration on any device.
//!
//! Fit it from the union store's own vectors (a few hundred suffice); refit when
//! the corpus grows materially or the embedder changes. Quantization noise from
//! vector reconstruction is absorbed into the null spread — that is where it
//! belongs.

use crate::vec::{dot, normalize};

#[derive(Clone, Debug)]
pub struct SpaceCalibration {
    /// Mean of the corpus's unit vectors — the cone's axis.
    pub mean: Vec<f32>,
    /// Deflated principal directions (possibly empty).
    pub removed: Vec<Vec<f32>>,
    /// Null statistics: centered cosine between random unrelated pairs.
    pub null_mean: f32,
    pub null_spread: f32,
}

impl SpaceCalibration {
    // MARK: - Fit

    /// Estimate the space from a corpus sample with the default deflation dial
    /// (`components = 0`, mean-only — the Swift default). Nil below 8 vectors —
    /// a null estimated from almost nothing would calibrate lies.
    pub fn fit(vectors: &[Vec<f32>]) -> Option<SpaceCalibration> {
        Self::fit_with_components(vectors, 0)
    }

    /// Estimate the space from a corpus sample. Nil below 8 vectors — a null
    /// estimated from almost nothing would calibrate lies.
    ///
    /// (Swift takes `components: Int` and clamps with `max(0, components)`;
    /// `usize` subsumes that clamp.)
    pub fn fit_with_components(vectors: &[Vec<f32>], components: usize) -> Option<SpaceCalibration> {
        let units: Vec<Vec<f32>> = vectors
            .iter()
            .filter(|v| !v.is_empty())
            .map(|v| normalize(v))
            .collect();
        if units.len() < 8 {
            return None;
        }
        let dim = units[0].len();

        let mut mean = vec![0.0f32; dim];
        for u in &units {
            for d in 0..dim {
                mean[d] += u[d];
            }
        }
        for d in 0..dim {
            mean[d] /= units.len() as f32;
        }

        let mut centered: Vec<Vec<f32>> = units
            .iter()
            .map(|u| u.iter().zip(&mean).map(|(a, m)| a - m).collect())
            .collect();
        let mut removed: Vec<Vec<f32>> = Vec::new();
        for c in 0..components {
            let Some(pc) = Self::principal(&centered, dim, (c as u64).wrapping_add(1)) else {
                break;
            };
            for row in centered.iter_mut() {
                let proj = dotf(row, &pc);
                for d in 0..dim {
                    row[d] -= proj * pc[d];
                }
            }
            removed.push(pc);
        }

        // The null: deterministic random pairs of centered unit vectors.
        let cu: Vec<Vec<f32>> = centered.iter().map(|c| normalize(c)).collect();
        let mut rng = SplitMix64::new(fnv1a(&format!(
            "{}|{}|{}",
            units.len(),
            dim,
            removed.len()
        )));
        let sample = 2000.min(units.len() * (units.len() - 1) / 2);
        let mut cosines: Vec<f32> = Vec::with_capacity(sample);
        for _ in 0..sample {
            let i = (rng.next() % cu.len() as u64) as usize;
            let mut j = (rng.next() % cu.len() as u64) as usize;
            if j == i {
                j = (j + 1) % cu.len();
            }
            cosines.push(dot(&cu[i], &cu[j]));
        }
        let m = cosines.iter().sum::<f32>() / cosines.len() as f32;
        let variance =
            cosines.iter().map(|c| (c - m) * (c - m)).sum::<f32>() / cosines.len() as f32;

        Some(SpaceCalibration {
            mean,
            removed,
            null_mean: m,
            null_spread: variance.sqrt().max(1e-4),
        })
    }

    // MARK: - Apply

    /// A vector moved into the calibrated frame: unit → mean-subtracted →
    /// deflated → re-normalized. A vector that centering annihilates (it WAS
    /// the common direction) falls back to its raw unit self rather than NaN.
    pub fn center(&self, v: &[f32]) -> Vec<f32> {
        let u = normalize(v);
        let mut c: Vec<f32> = u.iter().zip(&self.mean).map(|(a, m)| a - m).collect();
        for pc in &self.removed {
            let proj = dotf(&c, pc);
            for d in 0..c.len() {
                c[d] -= proj * pc[d];
            }
        }
        let n = normalize(&c);
        if n.iter().any(|&x| x != 0.0) {
            n
        } else {
            u
        }
    }

    /// A centered cosine as a z-score against this corpus's unrelated-pair null.
    pub fn z(&self, cosine: f32) -> f32 {
        (cosine - self.null_mean) / self.null_spread
    }

    /// The z squashed to [0,1]: 0.5 = as similar as unrelated texts get HERE.
    pub fn calibrated01(&self, cosine: f32) -> f32 {
        1.0 / (1.0 + (-self.z(cosine)).exp())
    }

    // MARK: - internals

    /// Top principal direction by deterministic power iteration.
    fn principal(rows: &[Vec<f32>], dim: usize, seed: u64) -> Option<Vec<f32>> {
        let mut rng = SplitMix64::new(seed);
        let start: Vec<f32> = (0..dim)
            .map(|_| (rng.next() % 1000) as f32 / 1000.0 - 0.5)
            .collect();
        let mut p = normalize(&start);
        for _ in 0..60 {
            let mut next = vec![0.0f32; dim];
            for r in rows {
                let proj = dotf(r, &p);
                for d in 0..dim {
                    next[d] += proj * r[d];
                }
            }
            let n = normalize(&next);
            if !n.iter().any(|&x| x != 0.0) {
                return None;
            }
            p = n;
        }
        Some(p)
    }
}

/// f32-accumulating dot over the shared prefix — the Swift file-scope `dotf`
/// (internal, so crate-visible here). Bit-identical to `vec::dot`.
pub(crate) fn dotf(a: &[f32], b: &[f32]) -> f32 {
    let mut s: f32 = 0.0;
    for i in 0..a.len().min(b.len()) {
        s += a[i] * b[i];
    }
    s
}

// MARK: - Deterministic randomness (bit-exact duplicates of the Swift internals
// in SpectralClustering.swift, private here until the spectral port unifies them)

/// SplitMix64 — tiny, seedable, stable. The ONLY randomness source in the lens.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

/// FNV-1a over a string — the stable seed derivation (String.hashValue is
/// process-seeded and would break run-to-run determinism).
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

// MARK: - Tests
//
// The anisotropy corrections proven on cone geometry — the shape real
// sentence-embedding spaces actually have.
//
// The synthetic cone: every vector = a strong COMMON direction + a small
// structured residual, unit-normalized — so all raw pairwise cosines sit
// ≈ 0.9, exactly the regime BGE-class models produce. The tests demonstrate
// the FAILURE first (uncentered coherence cannot tell a beam from buffeting;
// uncentered headings of different streams are indistinguishable) and then
// the recovery (centering restores structure; the null calibration makes 0.5
// mean "unrelated"; local-scaled spectral clustering reads k off the cone).

#[cfg(test)]
mod tests {
    use super::*;

    /// dim-8 cone: common axis e0, residual amplitude 0.3 along a group
    /// direction, deterministic jitter. All raw cosines land ≈ 0.9+.
    fn cone(group: usize, n: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut rng = SplitMix64::new(seed);
        (0..n)
            .map(|_| {
                let mut v = vec![0.0f32; 8];
                v[0] = 1.0; // the common direction
                v[1 + group] = 0.3; // the structure
                for d in 0..8 {
                    // jitter, deterministic
                    v[d] += ((rng.next() % 1000) as f32 / 1000.0 - 0.5) * 0.04;
                }
                normalize(&v)
            })
            .collect()
    }

    fn corpus() -> Vec<Vec<f32>> {
        let mut c = cone(0, 10, 1);
        c.extend(cone(1, 10, 2));
        c.extend(cone(2, 10, 3));
        c
    }

    #[test]
    fn the_cone_is_real_before_centering() {
        let c = corpus();
        // Cross-group texts are UNRELATED, yet raw cosine says ≈0.9 — the
        // anisotropy failure this file exists to correct.
        let cross = dot(&c[0], &c[15]);
        assert!(cross > 0.85);
    }

    #[test]
    fn centering_recovers_structure() {
        let c = corpus();
        let cal = SpaceCalibration::fit(&c).unwrap();
        let within = dot(&cal.center(&c[0]), &cal.center(&c[1]));
        let cross = dot(&cal.center(&c[0]), &cal.center(&c[15]));
        assert!(within > 0.6);
        assert!(cross < 0.35);
    }

    #[test]
    fn null_calibration_makes_half_mean_unrelated() {
        let c = corpus();
        let cal = SpaceCalibration::fit(&c).unwrap();
        let within = cal.calibrated01(dot(&cal.center(&c[0]), &cal.center(&c[1])));
        let cross = cal.calibrated01(dot(&cal.center(&c[0]), &cal.center(&c[15])));
        assert!(within > 0.8);
        assert!(cross < 0.55);
    }

    #[test]
    fn deflation_removes_a_dominant_axis_and_its_signal_with_it() {
        // Two groups along ±e1: after mean-centering, PC1 IS the group axis —
        // deflating it erases the distinction. The dial is powerful and
        // dangerous; this pins WHY components defaults to 0.
        let mut c = cone(0, 10, 4);
        c.extend(cone(0, 10, 5).into_iter().map(|v| {
            let mut w = v;
            w[1] = -w[1];
            normalize(&w)
        }));
        let kept = SpaceCalibration::fit_with_components(&c, 0).unwrap();
        let cut = SpaceCalibration::fit_with_components(&c, 1).unwrap();
        let kept_cross = dot(&kept.center(&c[0]), &kept.center(&c[15]));
        let cut_cross = dot(&cut.center(&c[0]), &cut.center(&c[15]));
        assert!(kept_cross < -0.5); // opposed groups, visible
        assert!(cut_cross.abs() < kept_cross.abs()); // deflation blurred them together
    }

    #[test]
    fn deterministic_fit() {
        let c = corpus();
        let a = SpaceCalibration::fit_with_components(&c, 1).unwrap();
        let b = SpaceCalibration::fit_with_components(&c, 1).unwrap();
        assert_eq!(a.mean, b.mean);
        assert_eq!(a.removed, b.removed);
        assert_eq!(a.null_mean, b.null_mean);
    }

    #[test]
    fn tiny_corpus_refuses_to_calibrate() {
        assert!(SpaceCalibration::fit(&cone(0, 5, 9)).is_none());
    }
}
