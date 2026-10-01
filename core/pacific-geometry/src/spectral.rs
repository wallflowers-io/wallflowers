//! SpectralClustering — the spectral half of the CORE TRANSMISSION (7 Aug
//! directive): categorise the embedding space by its own geometry, no model in
//! the loop.
//!
//! The textbook pipeline, chosen deliberately over anything cleverer:
//!   cosine kNN graph  →  normalized Laplacian L_sym = I − D^-1/2 W D^-1/2
//!   →  eigendecomposition  →  EIGENGAP picks k  →  row-normalized spectral
//!   embedding  →  deterministic k-means++.
//! The eigengap is the point: k is READ OFF the space's own structure, never
//! guessed and never asked of a model. Zero eigenvalues count connected
//! components, the largest gap after them bounds the natural cluster count.
//!
//! DETERMINISM IS THE CONTRACT (house rule, same as ChatChunker): identical
//! vectors in, identical clusters out, on any device, any launch. The k-means
//! seed derives from the input ids (FNV-1a over the sorted list through a
//! SplitMix64 stream) — never SystemRandomNumberGenerator, never Date.
//!
//! The eigensolver is cyclic JACOBI, hand-rolled: ~60 lines, dependency-free,
//! bit-stable across platforms, O(N³) — fine for the lens's windowed N (≤ ~768
//! episodes; the Transmission caps its window). If the window ever grows past
//! that, swap this routine for Accelerate/LAPACK dsyevd behind the same
//! signature; nothing above it changes.
//!
//! Port of PacificStore/SpectralClustering.swift (1:1; Swift Float → f32,
//! Double → f64). This file also owns the crate's deterministic-randomness
//! primitives (SplitMix64, fnv1a) and the Jacobi eigensolver — pub, because
//! SpaceCalibration and others seed from the same stream.

use crate::vec::{dot, sq_dist};

// MARK: - small vector helpers
//
// Swift's SpectralClustering carries its own private `normalize`, which
// multiplies by the reciprocal (`x * (1/√mag)`); `vec::normalize` divides
// (`x / √mag`). The two round DIFFERENTLY in f32 for ~80% of vectors (measured),
// and everything downstream — Laplacian, eigenvalues, embedding — inherits the
// bits. The FFI swap pins Swift-bit-identical output, so this module keeps the
// verbatim Swift form. (`dot`/`sq_dist` accumulate in the same order as Swift
// and are shared from `vec` unchanged.)
fn normalize(v: &[f32]) -> Vec<f32> {
    let mut mag: f32 = 0.0;
    for x in v {
        mag += x * x;
    }
    if !(mag > 0.0) {
        return v.to_vec();
    }
    let inv = 1.0 / mag.sqrt();
    v.iter().map(|x| x * inv).collect()
}

// MARK: - Deterministic randomness

/// SplitMix64 — tiny, seedable, stable. The ONLY randomness source in the lens.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

/// FNV-1a over a string — the stable seed derivation (String.hashValue is
/// process-seeded and would break run-to-run determinism; Rust's DefaultHasher
/// is unspecified across releases — same disease, same cure).
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
    }
    h
}

/// `Float.random(in: 0..<1, using:)` exactly as the Swift stdlib computes it
/// over a 64-bit generator: ONE `next()`, low 24 bits, scaled by
/// ulpOfOne/2 = 2⁻²⁴ (delta = 1, lowerBound = 0, so that IS the result; the
/// == upperBound re-roll can never trigger). Verified bit-exact against the
/// Swift toolchain on this machine — do not "improve" the bit selection.
fn swift_unit_random(rng: &mut SplitMix64) -> f32 {
    const MASK: u64 = (1 << 24) - 1;
    (rng.next() & MASK) as f32 * (f32::EPSILON / 2.0)
}

// MARK: - The clustering

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpectralParams {
    /// Neighbours per row in the similarity graph.
    pub k_nn: usize,
    /// The k the eigengap may choose from (a lens should propose neither one
    /// mega-cluster nor confetti).
    pub k_min: usize,
    pub k_max: usize,
    /// k-means iterations cap; assignment stability usually converges in < 20.
    pub max_iterations: usize,
}

impl Default for SpectralParams {
    fn default() -> Self {
        SpectralParams { k_nn: 10, k_min: 2, k_max: 12, max_iterations: 50 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpectralResult {
    /// Cluster index per input row (aligned with the input order).
    pub assignments: Vec<usize>,
    /// The k the eigengap chose.
    pub k: usize,
    /// Ascending eigenvalues of L_sym — kept so callers can SEE the gap that
    /// justified k (a lens shows its working; it never just asserts).
    pub eigenvalues: Vec<f64>,
}

/// Cluster `vectors` (any dimension, cosine geometry) into a structure the
/// space itself justifies. `ids` seed the deterministic RNG and must be the
/// rows' stable identities. N < kMin collapses to one cluster honestly.
///
/// Pass `calibration` for REAL embedding vectors: trained spaces are
/// anisotropic (a narrow cone), and clustering them raw hands the geometry
/// to the corpus common direction. Centering first is the grounded move.
pub fn cluster(
    ids: &[String],
    vectors: &[Vec<f32>],
    params: &SpectralParams,
    calibration: Option<&crate::space_calibration::SpaceCalibration>,
) -> SpectralResult {
    assert!(ids.len() == vectors.len(), "ids and vectors must align");
    let n = vectors.len();
    if n < 2.max(params.k_min) {
        return SpectralResult {
            assignments: vec![0; n],
            k: if n == 0 { 0 } else { 1 },
            eigenvalues: Vec::new(),
        };
    }

    // 1 · into the calibrated frame (or plain unit vectors when raw).
    let unit: Vec<Vec<f32>> = match calibration {
        Some(cal) => vectors.iter().map(|v| cal.center(v)).collect(),
        None => vectors.iter().map(|v| normalize(v)).collect(),
    };

    // 2 · kNN graph with LOCAL SCALING (Zelnik-Manor & Perona): in a space
    // whose similarities compress into a narrow band, absolute weights lose
    // contrast — scale each edge by the two endpoints' own neighbourhood
    // radii (distance to the kth neighbour), so "close" means close FOR
    // THAT REGION and the eigengap stays readable. Symmetrized by max.
    let k = params.k_nn.min(n - 1);
    let mut neighbours: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut sigma = vec![0.0f64; n];
    for i in 0..n {
        let mut ds: Vec<(usize, f64)> = Vec::with_capacity(n - 1);
        for j in 0..n {
            if j == i {
                continue;
            }
            // cosine distance
            ds.push((j, (1.0 - f64::from(dot(&unit[i], &unit[j]))).max(0.0)));
        }
        // deterministic ties
        ds.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then(a.0.cmp(&b.0)));
        ds.truncate(k);
        sigma[i] = ds.last().map_or(1.0, |t| t.1).max(1e-6);
        neighbours[i] = ds;
    }
    let mut w = vec![0.0f64; n * n];
    for i in 0..n {
        for &(j, d) in &neighbours[i] {
            let s = (-(d * d) / (sigma[i] * sigma[j])).exp();
            w[i * n + j] = w[i * n + j].max(s);
            w[j * n + i] = w[j * n + i].max(s);
        }
    }

    // 3 · normalized Laplacian L_sym = I − D^-1/2 W D^-1/2.
    let mut degree = vec![0.0f64; n];
    for i in 0..n {
        for j in 0..n {
            degree[i] += w[i * n + j];
        }
        if degree[i] <= 0.0 {
            degree[i] = 1.0; // isolated row: keep it well-posed
        }
    }
    let d_inv_sqrt: Vec<f64> = degree.iter().map(|d| 1.0 / d.sqrt()).collect();
    let mut lap = vec![0.0f64; n * n];
    for i in 0..n {
        for j in 0..n {
            let off = -d_inv_sqrt[i] * w[i * n + j] * d_inv_sqrt[j];
            lap[i * n + j] = (if i == j { 1.0 } else { 0.0 }) + off;
        }
    }

    // 4 · eigendecomposition (Jacobi) → ascending eigenvalues + vectors.
    let (values, evecs) = jacobi_eigen(&lap, n);

    // 5 · eigengap chooses k within [kMin, kMax].
    let k_max = params.k_max.min(n - 1);
    let k_min = params.k_min.min(k_max);
    let mut chosen = k_min;
    let mut best_gap = -1.0f64;
    for i in k_min..=k_max {
        let gap = values[i] - values[i - 1];
        if gap > best_gap {
            best_gap = gap;
            chosen = i;
        }
    }

    // 6 · spectral embedding: first `chosen` eigenvectors as rows, row-normalized.
    let mut embed: Vec<Vec<f32>> = vec![vec![0.0f32; chosen]; n];
    for r in 0..n {
        for c in 0..chosen {
            embed[r][c] = evecs[r * n + c] as f32;
        }
        embed[r] = normalize(&embed[r]);
    }

    // 7 · deterministic k-means++ in spectral space.
    let mut sorted_ids: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    sorted_ids.sort_unstable();
    let seed = fnv1a(&sorted_ids.join("\u{1F}"));
    let assignments = k_means(&embed, chosen, seed, params.max_iterations);

    SpectralResult { assignments, k: chosen, eigenvalues: values[..k_max + 1].to_vec() }
}

// MARK: - k-means++

pub fn k_means(points: &[Vec<f32>], k: usize, seed: u64, max_iterations: usize) -> Vec<usize> {
    let n = points.len();
    if !(k > 1 && n > k) {
        return vec![0; n];
    }
    let mut rng = SplitMix64::new(seed);

    // k-means++ seeding: spread the initial centroids by squared distance.
    let mut centroids: Vec<Vec<f32>> = vec![points[(rng.next() % n as u64) as usize].clone()];
    while centroids.len() < k {
        // Swift `.min()` semantics: first element, replaced when strictly smaller.
        let d2: Vec<f32> = points
            .iter()
            .map(|p| {
                let mut m: Option<f32> = None;
                for c in &centroids {
                    let d = sq_dist(p, c);
                    m = Some(match m {
                        None => d,
                        Some(cur) => {
                            if d < cur {
                                d
                            } else {
                                cur
                            }
                        }
                    });
                }
                m.unwrap_or(0.0)
            })
            .collect();
        let total: f32 = d2.iter().fold(0.0, |acc, d| acc + d); // in-order, as Swift reduce(0, +)
        if total <= 0.0 {
            centroids.push(points[(rng.next() % n as u64) as usize].clone());
            continue;
        }
        let mut pick = swift_unit_random(&mut rng) * total;
        let mut idx = 0;
        for (i, d) in d2.iter().enumerate() {
            pick -= d;
            if pick <= 0.0 {
                idx = i;
                break;
            }
            idx = i;
        }
        centroids.push(points[idx].clone());
    }

    let mut assign = vec![0usize; n];
    for _ in 0..max_iterations {
        let mut moved = false;
        for (i, p) in points.iter().enumerate() {
            let mut best = 0usize;
            let mut best_d = f32::MAX; // Float.greatestFiniteMagnitude
            for (c, centroid) in centroids.iter().enumerate() {
                let d = sq_dist(p, centroid);
                if d < best_d {
                    best_d = d;
                    best = c;
                }
            }
            if assign[i] != best {
                assign[i] = best;
                moved = true;
            }
        }
        if !moved {
            break;
        }
        let dim = points[0].len();
        let mut sums = vec![vec![0.0f32; dim]; k];
        let mut counts = vec![0usize; k];
        for (i, p) in points.iter().enumerate() {
            counts[assign[i]] += 1;
            for d in 0..dim {
                sums[assign[i]][d] += p[d];
            }
        }
        for c in 0..k {
            if counts[c] > 0 {
                centroids[c] = sums[c].iter().map(|s| s / counts[c] as f32).collect();
            }
        }
    }
    assign
}

// MARK: - Jacobi eigensolver (symmetric, cyclic sweeps)

/// Ascending eigenvalues + column eigenvectors of a symmetric n×n matrix
/// (row-major, `values` ascending, `vectors[r * n + c]` = component r of the
/// eigenvector for `values[c]`).
/// Plain cyclic Jacobi: bit-stable, dependency-free. Swap for LAPACK dsyevd
/// behind this signature if the window outgrows it.
pub fn jacobi_eigen(matrix: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut a = matrix.to_vec();
    let mut v = vec![0.0f64; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }

    for _ in 0..64 {
        // sweeps; converges long before this for our sizes
        let mut off = 0.0f64;
        for p in 0..n {
            for q in (p + 1)..n {
                off += a[p * n + q] * a[p * n + q];
            }
        }
        if off.sqrt() < 1e-10 {
            break;
        }
        for p in 0..n {
            for q in (p + 1)..n {
                let apq = a[p * n + q];
                if apq.abs() <= 1e-14 {
                    continue;
                }
                let app = a[p * n + p];
                let aqq = a[q * n + q];
                let theta = (aqq - app) / (2.0 * apq);
                let t = (if theta >= 0.0 { 1.0 } else { -1.0 })
                    / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for i in 0..n {
                    let aip = a[i * n + p];
                    let aiq = a[i * n + q];
                    a[i * n + p] = c * aip - s * aiq;
                    a[i * n + q] = s * aip + c * aiq;
                }
                for i in 0..n {
                    let api = a[p * n + i];
                    let aqi = a[q * n + i];
                    a[p * n + i] = c * api - s * aqi;
                    a[q * n + i] = s * api + c * aqi;
                }
                for i in 0..n {
                    let vip = v[i * n + p];
                    let viq = v[i * n + q];
                    v[i * n + p] = c * vip - s * viq;
                    v[i * n + q] = s * vip + c * viq;
                }
            }
        }
    }

    // Sort ascending, carrying columns. Stable, as Swift's `sorted` — equal
    // eigenvalues keep their pre-sort column order (stable identity).
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&x, &y| a[x * n + x].partial_cmp(&a[y * n + y]).unwrap());
    let mut values = vec![0.0f64; n];
    let mut sorted = vec![0.0f64; n * n];
    for (new_col, &old_col) in order.iter().enumerate() {
        values[new_col] = a[old_col * n + old_col];
        for r in 0..n {
            sorted[r * n + new_col] = v[r * n + old_col];
        }
    }
    (values, sorted)
}

// MARK: - tests (ported 1:1 from TransmissionTests.swift · SpectralClusteringTests)

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // Deterministic blob generator — SplitMix64, never SystemRandom.
    // RNG consumption order matters (row by row, dim by dim), exactly as Swift.
    fn blob(center: &[f32], n: usize, spread: f32, seed: u64) -> Vec<Vec<f32>> {
        let mut rng = SplitMix64::new(seed);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let mut row = Vec::with_capacity(center.len());
            for &c in center {
                let u = (rng.next() % 10_000) as f32 / 10_000.0 - 0.5;
                row.push(c + u * spread);
            }
            out.push(row);
        }
        out
    }

    fn three_blobs() -> (Vec<String>, Vec<Vec<f32>>) {
        // Three well-separated directions in 8-d, 12 points each.
        let mut centers = vec![vec![0.0f32; 8]; 3];
        centers[0][0] = 1.0;
        centers[1][3] = 1.0;
        centers[2][6] = 1.0;
        let mut vectors: Vec<Vec<f32>> = Vec::new();
        for (i, c) in centers.iter().enumerate() {
            vectors.extend(blob(c, 12, 0.15, 100 + i as u64));
        }
        let ids = (0..vectors.len()).map(|i| format!("e{i}")).collect();
        (ids, vectors)
    }

    #[test]
    fn eigengap_reads_k_off_the_geometry() {
        let (ids, vectors) = three_blobs();
        let result = cluster(&ids, &vectors, &SpectralParams::default(), None);
        assert_eq!(result.k, 3);
    }

    #[test]
    fn blobs_land_in_pure_clusters() {
        let (ids, vectors) = three_blobs();
        let result = cluster(&ids, &vectors, &SpectralParams::default(), None);
        // Every ground-truth blob maps to exactly one cluster label.
        for b in 0..3 {
            let labels: HashSet<usize> =
                result.assignments[b * 12..(b + 1) * 12].iter().copied().collect();
            assert_eq!(labels.len(), 1, "blob {b} split across {labels:?}");
        }
        let distinct: HashSet<usize> = result.assignments.iter().copied().collect();
        assert_eq!(distinct.len(), 3);
    }

    #[test]
    fn identical_input_identical_output() {
        let (ids, vectors) = three_blobs();
        let one = cluster(&ids, &vectors, &SpectralParams::default(), None);
        let two = cluster(&ids, &vectors, &SpectralParams::default(), None);
        assert_eq!(one, two);
    }

    #[test]
    fn tiny_input_collapses_honestly() {
        let result =
            cluster(&["a".to_string()], &[vec![1.0, 0.0]], &SpectralParams::default(), None);
        assert_eq!(result.k, 1);
        assert_eq!(result.assignments, vec![0]);
    }
}
