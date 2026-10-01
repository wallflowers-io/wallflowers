//! FFT — the spectral substrate of lane L2 (turbulence-lanes M6, slice L2.0).
//!
//! Radix-2 decimation-in-time Cooley–Tukey, f64, zero-dep, in-place. This is
//! the first FFT anywhere in the workspace (checked 10 Aug 2026): A7's Jacobi
//! eigensolver is the dense small-n instrument and explicitly NOT an FFT; this
//! module is the O(N log N) one, built for the pseudo-spectral T² vorticity
//! solver (M7: ω̂ advance, ψ̂ = −ω̂/k²) and the shell diagnostics (M8: E(k),
//! Z(k), Π(k)) that consume it.
//!
//! CONVENTION (pinned):
//! ```text
//!     forward   X_k = Σ_j x_j · e^(−i·2π·jk/n)         (unscaled)
//!     inverse   x_j = (1/n) Σ_k X_k · e^(+i·2π·jk/n)   (scaled by 1/n)
//! ```
//! so inverse∘forward is the identity and Parseval reads
//! Σ|x|² = (1/n)·Σ|X|².
//!
//! House rules, as everywhere in this crate: deterministic — fixed stage,
//! block and butterfly order, no RNG, no clocks; bit-identical runs on a
//! given platform (same precedent as the vortex solver, whose ln/exp also
//! come from the system libm; the Swift-bit-parity pin covers the f32 social
//! atoms, not this f64 solver lane). Twiddles are computed DIRECTLY per
//! entry from sin/cos — never by the multiplicative recurrence, which
//! accumulates rounding drift and is what "deterministic but wrong" looks
//! like. Non-power-of-two lengths are REFUSED, never silently zero-padded:
//! padding changes the transform's meaning, and refuse-don't-clamp is the
//! doctrine.
//!
//! Verification gates (pre-declared in turbulence-lanes §6, L2.0): round-trip
//! to 1e-12, Parseval, bit-identity, naive-DFT cross-check at small n. All
//! four live in the tests below, alongside the closed forms (impulse, DC,
//! single-bin cosine, Hermitian symmetry) and the convolution theorem against
//! the direct O(N²) circular sum.

/// A complex number in f64 — the lane-L2 scalar. Deliberately minimal; the
/// arithmetic keeps one fixed expression order so results are bit-stable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const ZERO: Complex = Complex { re: 0.0, im: 0.0 };

    pub fn new(re: f64, im: f64) -> Self {
        Complex { re, im }
    }

    pub fn conj(self) -> Self {
        Complex { re: self.re, im: -self.im }
    }

    pub fn scale(self, s: f64) -> Self {
        Complex { re: self.re * s, im: self.im * s }
    }

    /// |z|² without the square root — the spectral energy reading.
    pub fn abs2(self) -> f64 {
        self.re * self.re + self.im * self.im
    }
}

impl std::ops::Add for Complex {
    type Output = Complex;
    fn add(self, rhs: Complex) -> Complex {
        Complex { re: self.re + rhs.re, im: self.im + rhs.im }
    }
}

impl std::ops::Sub for Complex {
    type Output = Complex;
    fn sub(self, rhs: Complex) -> Complex {
        Complex { re: self.re - rhs.re, im: self.im - rhs.im }
    }
}

impl std::ops::Mul for Complex {
    type Output = Complex;
    fn mul(self, rhs: Complex) -> Complex {
        Complex {
            re: self.re * rhs.re - self.im * rhs.im,
            im: self.re * rhs.im + self.im * rhs.re,
        }
    }
}

/// The plan: twiddle table + bit-reversal permutation for one size n,
/// precomputed once so a solver stepping thousands of frames pays sin/cos
/// exactly once. A plan is immutable after construction — sharing one across
/// calls cannot change any result.
#[derive(Clone, Debug)]
pub struct Fft {
    n: usize,
    /// w[k] = e^(−i·2π·k/n) for k < n/2 — the forward table; the inverse
    /// conjugates on the fly (an exact sign flip, so equally deterministic).
    twiddle: Vec<Complex>,
    /// Bit-reversal permutation of 0..n.
    perm: Vec<usize>,
}

impl Fft {
    /// Build a plan for length `n`. REFUSES a non-power-of-two (including 0):
    /// zero-padding or truncating would silently change what the transform
    /// means, and the caller owns that decision.
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two(), "FFT length must be a power of two, got {n}");
        let log2n = n.trailing_zeros();
        let twiddle: Vec<Complex> = (0..n / 2)
            .map(|k| {
                let (s, c) = (-std::f64::consts::TAU * k as f64 / n as f64).sin_cos();
                Complex { re: c, im: s }
            })
            .collect();
        let perm: Vec<usize> = (0..n)
            .map(|i| {
                if log2n == 0 {
                    i
                } else {
                    i.reverse_bits() >> (usize::BITS - log2n)
                }
            })
            .collect();
        Fft { n, twiddle, perm }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        false // a plan always has n ≥ 1
    }

    /// Forward transform, in place, unscaled. `x.len()` must equal the plan's.
    pub fn forward(&self, x: &mut [Complex]) {
        self.transform(x, false);
    }

    /// Inverse transform, in place, scaled by 1/n — inverse∘forward = identity.
    pub fn inverse(&self, x: &mut [Complex]) {
        self.transform(x, true);
        let inv = 1.0 / self.n as f64;
        for v in x.iter_mut() {
            *v = v.scale(inv);
        }
    }

    /// The shared butterfly pass. Stage → block → butterfly order is fixed;
    /// the only difference between directions is the twiddle conjugation.
    fn transform(&self, x: &mut [Complex], conjugate: bool) {
        let n = self.n;
        assert!(x.len() == n, "buffer length {} does not match plan length {n}", x.len());

        for i in 0..n {
            let j = self.perm[i];
            if i < j {
                x.swap(i, j);
            }
        }

        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let stride = n / len;
            for base in (0..n).step_by(len) {
                for k in 0..half {
                    let mut w = self.twiddle[k * stride];
                    if conjugate {
                        w = w.conj();
                    }
                    let a = x[base + k];
                    let b = x[base + k + half] * w;
                    x[base + k] = a + b;
                    x[base + k + half] = a - b;
                }
            }
            len *= 2;
        }
    }
}

/// The 2-D plan for an n×n row-major grid — rows then columns through one
/// shared 1-D plan (the M7 construction layer; separability is the theorem,
/// the fixed row→column order is the determinism). Inverse runs columns then
/// rows so the two passes unwind in reverse.
#[derive(Clone, Debug)]
pub struct Fft2 {
    n: usize,
    fft: Fft,
}

impl Fft2 {
    /// Square power-of-two grids only — same refusal contract as `Fft`.
    pub fn new(n: usize) -> Self {
        Fft2 { n, fft: Fft::new(n) }
    }

    /// Grid side length; buffers are n·n row-major.
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Forward 2-D transform, in place, unscaled:
    /// X_{kx,ky} = Σ_{j,l} x_{j,l} · e^(−i·2π·(j·kx + l·ky)/n).
    pub fn forward(&self, x: &mut [Complex]) {
        let n = self.n;
        assert!(x.len() == n * n, "grid length {} does not match {n}×{n}", x.len());
        for r in 0..n {
            self.fft.forward(&mut x[r * n..(r + 1) * n]);
        }
        let mut col = vec![Complex::ZERO; n];
        for c in 0..n {
            for r in 0..n {
                col[r] = x[r * n + c];
            }
            self.fft.forward(&mut col);
            for r in 0..n {
                x[r * n + c] = col[r];
            }
        }
    }

    /// Inverse 2-D transform, in place, scaled by 1/n² — inverse∘forward is
    /// the identity.
    pub fn inverse(&self, x: &mut [Complex]) {
        let n = self.n;
        assert!(x.len() == n * n, "grid length {} does not match {n}×{n}", x.len());
        let mut col = vec![Complex::ZERO; n];
        for c in 0..n {
            for r in 0..n {
                col[r] = x[r * n + c];
            }
            self.fft.inverse(&mut col);
            for r in 0..n {
                x[r * n + c] = col[r];
            }
        }
        for r in 0..n {
            self.fft.inverse(&mut x[r * n..(r + 1) * n]);
        }
    }
}

// MARK: - tests (the L2.0 gates + closed forms)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectral::SplitMix64;

    /// Deterministic test signal in [−1, 1)² — SplitMix64 (the crate's one
    /// randomness atom, per the F2 dedupe finding), 53-bit expansion.
    fn seeded_signal(n: usize, seed: u64) -> Vec<Complex> {
        let mut rng = SplitMix64::new(seed);
        let mut unit = move || (rng.next() >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
        (0..n).map(|_| Complex::new(unit(), unit())).collect()
    }

    /// The O(N²) oracle — the transform's definition, summed in index order.
    fn naive_dft(x: &[Complex]) -> Vec<Complex> {
        let n = x.len();
        (0..n)
            .map(|k| {
                let mut acc = Complex::ZERO;
                for (j, &v) in x.iter().enumerate() {
                    let angle = -std::f64::consts::TAU * (j * k) as f64 / n as f64;
                    let (s, c) = angle.sin_cos();
                    acc = acc + v * Complex::new(c, s);
                }
                acc
            })
            .collect()
    }

    fn max_err(a: &[Complex], b: &[Complex]) -> f64 {
        a.iter().zip(b).map(|(p, q)| (*p - *q).abs2().sqrt()).fold(0.0, f64::max)
    }

    /// δ at 0 → a flat spectrum of exact ones (no arithmetic can miss: every
    /// butterfly adds zeros to the impulse).
    #[test]
    fn impulse_transforms_flat() {
        let mut x = vec![Complex::ZERO; 16];
        x[0] = Complex::new(1.0, 0.0);
        Fft::new(16).forward(&mut x);
        for v in &x {
            assert_eq!(*v, Complex::new(1.0, 0.0));
        }
    }

    /// A constant signal concentrates at bin 0 with weight n·c.
    #[test]
    fn dc_concentrates_at_bin_zero() {
        let n = 64;
        let mut x = vec![Complex::new(0.75, 0.0); n];
        Fft::new(n).forward(&mut x);
        assert!((x[0].re - 0.75 * n as f64).abs() < 1e-12);
        assert!(x[0].im.abs() < 1e-12);
        for v in &x[1..] {
            assert!(v.abs2().sqrt() < 1e-12);
        }
    }

    /// cos(2π·m·j/n) lands n/2 on bins m and n−m and nothing anywhere else.
    #[test]
    fn cosine_lands_on_its_bin_pair() {
        let n = 64;
        let m = 5;
        let mut x: Vec<Complex> = (0..n)
            .map(|j| {
                Complex::new((std::f64::consts::TAU * (m * j) as f64 / n as f64).cos(), 0.0)
            })
            .collect();
        Fft::new(n).forward(&mut x);
        for (k, v) in x.iter().enumerate() {
            let expected = if k == m || k == n - m { n as f64 / 2.0 } else { 0.0 };
            assert!((v.re - expected).abs() < 1e-10, "bin {k}: {v:?}");
            assert!(v.im.abs() < 1e-10, "bin {k}: {v:?}");
        }
    }

    /// GATE (L2.0): inverse∘forward returns the signal to 1e-12, across sizes
    /// including the degenerate n = 1.
    #[test]
    fn round_trip_hits_1e12() {
        for (n, seed) in [(1, 7u64), (2, 11), (8, 13), (256, 17), (1024, 19)] {
            let original = seeded_signal(n, seed);
            let mut x = original.clone();
            let plan = Fft::new(n);
            plan.forward(&mut x);
            plan.inverse(&mut x);
            let err = max_err(&x, &original);
            assert!(err < 1e-12, "n = {n}: round-trip error {err}");
        }
    }

    /// GATE (L2.0): Parseval — Σ|x|² = (1/n)·Σ|X|², relative 1e-12.
    #[test]
    fn parseval_holds() {
        let n = 512;
        let x = seeded_signal(n, 23);
        let physical: f64 = x.iter().map(|v| v.abs2()).sum();
        let mut spectrum = x;
        Fft::new(n).forward(&mut spectrum);
        let spectral: f64 = spectrum.iter().map(|v| v.abs2()).sum::<f64>() / n as f64;
        assert!(
            (physical - spectral).abs() / physical < 1e-12,
            "Parseval broke: {physical} vs {spectral}"
        );
    }

    /// GATE (L2.0): the fast path agrees with the O(N²) definition at every
    /// small n where the naive sum is trustworthy.
    #[test]
    fn matches_naive_dft_at_small_n() {
        for n in [1usize, 2, 4, 8, 16, 32] {
            let x = seeded_signal(n, 100 + n as u64);
            let expected = naive_dft(&x);
            let mut fast = x;
            Fft::new(n).forward(&mut fast);
            let err = max_err(&fast, &expected);
            assert!(err < 1e-10, "n = {n}: deviates from the DFT by {err}");
        }
    }

    /// GATE (L2.0): bit-identical runs — same input, same plan, equal to the
    /// last bit; and a fresh plan changes nothing (the plan is pure).
    #[test]
    fn bit_identical_runs() {
        let x = seeded_signal(256, 31);
        let plan = Fft::new(256);
        let mut a = x.clone();
        let mut b = x.clone();
        plan.forward(&mut a);
        plan.forward(&mut b);
        assert_eq!(a, b);
        let mut c = x;
        Fft::new(256).forward(&mut c);
        assert_eq!(a, c);
    }

    /// Real input → Hermitian spectrum: X[n−k] = conj(X[k]). M7's real
    /// vorticity fields lean on this.
    #[test]
    fn real_input_is_hermitian() {
        let n = 128;
        let mut x: Vec<Complex> =
            seeded_signal(n, 41).into_iter().map(|v| Complex::new(v.re, 0.0)).collect();
        Fft::new(n).forward(&mut x);
        for k in 1..n {
            let d = (x[n - k] - x[k].conj()).abs2().sqrt();
            assert!(d < 1e-10, "bin {k}: symmetry off by {d}");
        }
    }

    /// The convolution theorem against the direct circular sum — the identity
    /// M7's nonlinear term stands on (and this crate is the convolution
    /// engine; the theorem is the point of owning an FFT).
    #[test]
    fn convolution_theorem_matches_direct_sum() {
        let n = 64;
        let a = seeded_signal(n, 51);
        let b = seeded_signal(n, 53);

        let mut direct = vec![Complex::ZERO; n];
        for (k, slot) in direct.iter_mut().enumerate() {
            for j in 0..n {
                *slot = *slot + a[j] * b[(n + k - j) % n];
            }
        }

        let plan = Fft::new(n);
        let mut fa = a;
        let mut fb = b;
        plan.forward(&mut fa);
        plan.forward(&mut fb);
        let mut prod: Vec<Complex> = fa.iter().zip(&fb).map(|(p, q)| *p * *q).collect();
        plan.inverse(&mut prod);

        let err = max_err(&prod, &direct);
        assert!(err < 1e-10, "convolution theorem off by {err}");
    }

    /// Refuse-don't-clamp: a non-power-of-two length is a caller error,
    /// surfaced loudly — never padded into a different transform.
    #[test]
    #[should_panic(expected = "power of two")]
    fn refuses_non_power_of_two() {
        let _ = Fft::new(48);
    }

    /// 2-D round-trip at the same 1e-12 gate as the 1-D core.
    #[test]
    fn fft2_round_trip_hits_1e12() {
        let n = 32;
        let original = seeded_signal(n * n, 61);
        let mut x = original.clone();
        let plan = Fft2::new(n);
        plan.forward(&mut x);
        plan.inverse(&mut x);
        let err = max_err(&x, &original);
        assert!(err < 1e-12, "2-D round-trip error {err}");
    }

    /// Separability: the 2-D transform of an outer product f(x)·g(y) is the
    /// outer product of the 1-D transforms — the theorem the row→column
    /// construction stands on, checked end to end.
    #[test]
    fn fft2_separable_matches_outer_product() {
        let n = 16;
        let f = seeded_signal(n, 71);
        let g = seeded_signal(n, 73);
        let mut grid: Vec<Complex> = (0..n * n).map(|i| f[i / n] * g[i % n]).collect();
        Fft2::new(n).forward(&mut grid);
        let plan = Fft::new(n);
        let mut fhat = f;
        let mut ghat = g;
        plan.forward(&mut fhat);
        plan.forward(&mut ghat);
        let expected: Vec<Complex> = (0..n * n).map(|i| fhat[i / n] * ghat[i % n]).collect();
        let err = max_err(&grid, &expected);
        assert!(err < 1e-10, "separability off by {err}");
    }
}
