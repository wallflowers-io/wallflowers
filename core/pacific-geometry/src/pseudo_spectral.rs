//! PSEUDO-SPECTRAL CFD — 2-D vorticity on the torus (turbulence-lanes M7,
//! slice L2.1): the field-form counterpart of the point-vortex solver.
//!
//! Incompressible 2-D flow on T² = [0, 2π)², vorticity–streamfunction form:
//!
//! ```text
//!     ∂ω/∂t + u·∇ω = −ν(−∇²)ω − ν_p(−∇²)^p ω
//!     ∇²ψ = ω ,   u = (−∂ψ/∂y, ∂ψ/∂x)
//! ```
//!
//! advanced entirely in spectral space: ψ̂ = −ω̂/k², velocities and vorticity
//! gradients by exact spectral differentiation, the ONE nonlinear product
//! formed pointwise in physical space and dealiased by Orszag's 2/3 rule
//! (modes with |kx| or |ky| above ⌊n/3⌋ are OUTSIDE the system — the resolved
//! set is declared at construction, a Galerkin projection, never a silent
//! clamp mid-run).
//!
//! INTEGRATOR PARITY WITH THE VORTEX SOLVER (A2): implicit midpoint, the same
//! fixed 12 fixed-point iterations, no early exit, no RNG, no clocks. The
//! exactness story transfers whole: Gauss methods conserve QUADRATIC first
//! integrals (Cooper; Hairer–Lubich–Wanner IV.2), and the truncated inviscid
//! system's two invariants — energy ½⟨|u|²⟩ = Σ|ω̂|²/2k² and enstrophy
//! ½⟨ω²⟩ = Σ|ω̂|²/2 — are exactly quadratic, because every dealiased triad
//! conserves them in detail. So the inviscid gate is machine precision, and
//! that is the L2.1 gate, not a wish.
//!
//! The k = 0 mode (mean vorticity) induces no velocity on the torus (there is
//! nothing to invert), and its advection tendency is the mean of a divergence
//! — identically zero. It is CARRIED, untouched, never zeroed: conservation
//! you can read, not an assumption.
//!
//! Physical-space products are taken on the real parts after the inverse
//! transforms — the reality constraint of a real vorticity field, enforced
//! where the representation makes it exact (imaginary residue is roundoff of
//! a Hermitian spectrum, not information).

use crate::fft::{Complex, Fft2};

/// Fixed-point iterations for the implicit midpoint solve — the SAME count as
/// the vortex solver, by construction (integrator parity is a stated design
/// fact, not a coincidence to drift apart).
const MIDPOINT_ITERATIONS: usize = 12;

/// The dissipation operator: ν on the Laplacian plus an optional
/// hyperviscosity ν_p(−∇²)^p. All zeros = inviscid, the conservation regime.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dissipation {
    pub nu: f64,
    pub hyper_nu: f64,
    /// p in (−∇²)^p; only read when `hyper_nu` ≠ 0. p = 1 duplicates `nu`.
    pub hyper_order: u32,
}

impl Dissipation {
    pub const INVISCID: Dissipation = Dissipation { nu: 0.0, hyper_nu: 0.0, hyper_order: 1 };

    pub fn viscous(nu: f64) -> Self {
        Dissipation { nu, hyper_nu: 0.0, hyper_order: 1 }
    }
}

/// The flow: ω̂ on an n×n grid (row-major, FFT wavenumber order), the 2-D
/// plan, the dealias mask, and the declared dissipation.
#[derive(Clone, Debug)]
pub struct TorusFlow {
    n: usize,
    omega_hat: Vec<Complex>,
    fft: Fft2,
    /// Signed wavenumber per index (FFT order: 0..n/2−1, −n/2..−1).
    wavenumber: Vec<f64>,
    /// 2/3-rule keep-mask: true where |kx| ≤ ⌊n/3⌋ and |ky| ≤ ⌊n/3⌋.
    keep: Vec<bool>,
    pub dissipation: Dissipation,
}

impl TorusFlow {
    /// Build from a real vorticity field on the n×n grid of [0, 2π)²
    /// (row r = y index, column c = x index, x = 2πc/n, y = 2πr/n).
    /// The field is projected onto the resolved 2/3 set — the declared
    /// Galerkin truncation of this solver, applied once, here, visibly.
    pub fn from_vorticity(omega: &[f64], n: usize, dissipation: Dissipation) -> Self {
        assert!(omega.len() == n * n, "field length {} does not match {n}×{n}", omega.len());
        let fft = Fft2::new(n);
        let wavenumber: Vec<f64> = (0..n)
            .map(|i| if i < n / 2 { i as f64 } else { i as f64 - n as f64 })
            .collect();
        let k_max = (n / 3) as f64;
        let keep: Vec<bool> = (0..n * n)
            .map(|idx| {
                wavenumber[idx / n].abs() <= k_max && wavenumber[idx % n].abs() <= k_max
            })
            .collect();
        let mut omega_hat: Vec<Complex> =
            omega.iter().map(|&w| Complex::new(w, 0.0)).collect();
        fft.forward(&mut omega_hat);
        for (v, &k) in omega_hat.iter_mut().zip(&keep) {
            if !k {
                *v = Complex::ZERO;
            }
        }
        TorusFlow { n, omega_hat, fft, wavenumber, keep, dissipation }
    }

    pub fn grid(&self) -> usize {
        self.n
    }

    /// The spectral state — the M8 diagnostics (E(k), Z(k), Π(k)) read here.
    pub fn omega_hat(&self) -> &[Complex] {
        &self.omega_hat
    }

    /// Real vorticity field, row-major, read back from the spectrum.
    pub fn vorticity(&self) -> Vec<f64> {
        let mut x = self.omega_hat.clone();
        self.fft.inverse(&mut x);
        x.into_iter().map(|v| v.re).collect()
    }

    /// Energy ½⟨|u|²⟩ = (1/2n⁴)·Σ_{k≠0} |ω̂|²/k² — quadratic invariant one.
    pub fn energy(&self) -> f64 {
        let n = self.n;
        let mut e = 0.0;
        for idx in 0..n * n {
            let k2 = self.k2(idx);
            if k2 > 0.0 {
                e += self.omega_hat[idx].abs2() / k2;
            }
        }
        e / (2.0 * (n as f64).powi(4))
    }

    /// Enstrophy ½⟨ω²⟩ = (1/2n⁴)·Σ |ω̂|² — quadratic invariant two.
    pub fn enstrophy(&self) -> f64 {
        let n = self.n;
        let z: f64 = self.omega_hat.iter().map(|v| v.abs2()).sum();
        z / (2.0 * (n as f64).powi(4))
    }

    /// Mean vorticity ⟨ω⟩ — the carried k = 0 mode, conserved exactly.
    pub fn mean_vorticity(&self) -> f64 {
        self.omega_hat[0].re / (self.n as f64 * self.n as f64)
    }

    fn k2(&self, idx: usize) -> f64 {
        let kx = self.wavenumber[idx % self.n];
        let ky = self.wavenumber[idx / self.n];
        kx * kx + ky * ky
    }

    /// dω̂/dt at the given spectral state: −dealias(FFT(u·∇ω)) − D(k)·ω̂.
    /// Four inverse transforms, one pointwise product, one forward transform —
    /// the pseudo-spectral core, in one fixed order.
    fn rhs(&self, omega_hat: &[Complex]) -> Vec<Complex> {
        let n = self.n;
        let nn = n * n;
        let mut u_hat = vec![Complex::ZERO; nn];
        let mut v_hat = vec![Complex::ZERO; nn];
        let mut wx_hat = vec![Complex::ZERO; nn];
        let mut wy_hat = vec![Complex::ZERO; nn];
        for idx in 0..nn {
            let w = omega_hat[idx];
            let kx = self.wavenumber[idx % n];
            let ky = self.wavenumber[idx / n];
            let k2 = kx * kx + ky * ky;
            // i·z = (−im, re); with ψ̂ = −ω̂/k²:  û = −i·ky·ψ̂ = i·ky·ω̂/k²,
            // v̂ = i·kx·ψ̂ = −i·kx·ω̂/k². The k = 0 mode induces nothing.
            if k2 > 0.0 {
                u_hat[idx] = Complex::new(-w.im, w.re).scale(ky / k2);
                v_hat[idx] = Complex::new(w.im, -w.re).scale(kx / k2);
            }
            wx_hat[idx] = Complex::new(-w.im, w.re).scale(kx);
            wy_hat[idx] = Complex::new(-w.im, w.re).scale(ky);
        }
        self.fft.inverse(&mut u_hat);
        self.fft.inverse(&mut v_hat);
        self.fft.inverse(&mut wx_hat);
        self.fft.inverse(&mut wy_hat);

        let mut advection: Vec<Complex> = (0..nn)
            .map(|i| {
                Complex::new(u_hat[i].re * wx_hat[i].re + v_hat[i].re * wy_hat[i].re, 0.0)
            })
            .collect();
        self.fft.forward(&mut advection);

        let d = self.dissipation;
        (0..nn)
            .map(|idx| {
                let k2 = self.k2(idx);
                let mut damp = d.nu * k2;
                if d.hyper_nu != 0.0 {
                    damp += d.hyper_nu * k2.powi(d.hyper_order as i32);
                }
                let adv = if self.keep[idx] { advection[idx] } else { Complex::ZERO };
                Complex::ZERO - adv - omega_hat[idx].scale(damp)
            })
            .collect()
    }

    /// One implicit-midpoint step: ω̂' = ω̂ + dt·F((ω̂ + ω̂')/2), fixed-point
    /// seeded with the explicit Euler guess — the A2 step, verbatim, on the
    /// field state.
    pub fn step(&mut self, dt: f64) {
        let nn = self.n * self.n;
        let z0 = self.omega_hat.clone();
        let f0 = self.rhs(&z0);
        let mut z1: Vec<Complex> = (0..nn).map(|i| z0[i] + f0[i].scale(dt)).collect();
        for _ in 0..MIDPOINT_ITERATIONS {
            let mid: Vec<Complex> = (0..nn).map(|i| (z0[i] + z1[i]).scale(0.5)).collect();
            let fm = self.rhs(&mid);
            for i in 0..nn {
                z1[i] = z0[i] + fm[i].scale(dt);
            }
        }
        self.omega_hat = z1;
    }

    /// Integrate `steps` fixed steps of `dt` — a pure function of
    /// (initial state, dt, steps), as everywhere in this crate.
    pub fn run(&mut self, dt: f64, steps: usize) {
        for _ in 0..steps {
            self.step(dt);
        }
    }
}

// MARK: - tests (the L2.1 gates + closed forms)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectral::SplitMix64;
    use std::f64::consts::TAU;

    /// Deterministic low-mode field: eight seeded harmonics, wavenumbers ≤ 5
    /// (inside every mask used here), amplitudes and phases from SplitMix64.
    fn seeded_field(n: usize, seed: u64) -> Vec<f64> {
        let mut rng = SplitMix64::new(seed);
        let mut unit = move || (rng.next() >> 11) as f64 / (1u64 << 53) as f64;
        let harmonics: Vec<(f64, f64, f64, f64)> = (0..8)
            .map(|_| {
                let kx = (1.0 + (unit() * 5.0).floor()).min(5.0);
                let ky = (1.0 + (unit() * 5.0).floor()).min(5.0);
                (kx, ky, unit() - 0.5, unit() * TAU)
            })
            .collect();
        (0..n * n)
            .map(|idx| {
                let x = TAU * (idx % n) as f64 / n as f64;
                let y = TAU * (idx / n) as f64 / n as f64;
                harmonics.iter().map(|&(kx, ky, a, p)| a * (kx * x + ky * y + p).cos()).sum()
            })
            .collect()
    }

    fn grid_xy(n: usize, idx: usize) -> (f64, f64) {
        (TAU * (idx % n) as f64 / n as f64, TAU * (idx / n) as f64 / n as f64)
    }

    /// The nonlinear term against a hand-derived closed form. For
    /// ω = cos x + cos 2y:  ψ = −cos x − cos(2y)/4,  u = −sin(2y)/2,
    /// v = sin x, and −u·∇ω = (3/2)·sin x·sin 2y. Every sign in the
    /// inversion, the velocities and the gradients is on trial here.
    #[test]
    fn closed_form_nonlinear_tendency() {
        let n = 32;
        let omega: Vec<f64> = (0..n * n)
            .map(|idx| {
                let (x, y) = grid_xy(n, idx);
                x.cos() + (2.0 * y).cos()
            })
            .collect();
        let flow = TorusFlow::from_vorticity(&omega, n, Dissipation::INVISCID);
        let mut tendency = flow.rhs(flow.omega_hat());
        flow.fft.inverse(&mut tendency);
        for (idx, t) in tendency.iter().enumerate() {
            let (x, y) = grid_xy(n, idx);
            let expected = 1.5 * x.sin() * (2.0 * y).sin();
            assert!(
                (t.re - expected).abs() < 1e-10,
                "tendency at {idx}: {} vs {expected}",
                t.re
            );
        }
    }

    /// Taylor–Green decays on its own modes at exactly e^(−2νt); the error
    /// left is the integrator's O(dt²), nothing spatial.
    #[test]
    fn taylor_green_decays_at_closed_form_rate() {
        let n = 32;
        let nu = 0.05;
        let omega: Vec<f64> = (0..n * n)
            .map(|idx| {
                let (x, y) = grid_xy(n, idx);
                2.0 * x.cos() * y.cos()
            })
            .collect();
        let mut flow = TorusFlow::from_vorticity(&omega, n, Dissipation::viscous(nu));
        let dt = 5e-3;
        let steps = 200;
        flow.run(dt, steps);
        let decay = (-2.0 * nu * dt * steps as f64).exp();
        let field = flow.vorticity();
        for idx in 0..n * n {
            let expected = omega[idx] * decay;
            assert!(
                (field[idx] - expected).abs() < 1e-8,
                "at {idx}: {} vs {expected}",
                field[idx]
            );
        }
    }

    /// GATE (L2.1): the inviscid truncated system holds BOTH quadratic
    /// invariants to machine precision across a genuinely nonlinear run —
    /// the field-form twin of A2's exact-invariant story. Measured on this
    /// field (release, 10 Aug 2026): relative drift ≤ 1.7e-15 out to 1600
    /// steps, flat — pure roundoff. The 1e-13 bound is that measurement with
    /// two decades of cross-platform headroom, not a softened claim.
    #[test]
    fn inviscid_conserves_energy_and_enstrophy() {
        let n = 32;
        let mut flow =
            TorusFlow::from_vorticity(&seeded_field(n, 0xF10A), n, Dissipation::INVISCID);
        let e0 = flow.energy();
        let z0 = flow.enstrophy();
        flow.run(5e-3, 400);
        let de = (flow.energy() - e0).abs() / e0;
        let dz = (flow.enstrophy() - z0).abs() / z0;
        assert!(de < 1e-13, "energy drifted by {de}");
        assert!(dz < 1e-13, "enstrophy drifted by {dz}");
    }

    /// The midpoint map is symmetric: forward then backward returns the field
    /// home — time-reversal on the torus, to fixed-point tolerance.
    #[test]
    fn time_reversal_returns_home() {
        let n = 32;
        let start = seeded_field(n, 0xB0A7);
        let mut flow = TorusFlow::from_vorticity(&start, n, Dissipation::INVISCID);
        let reference = flow.vorticity();
        flow.run(5e-3, 200);
        flow.run(-5e-3, 200);
        let back = flow.vorticity();
        for (a, b) in back.iter().zip(&reference) {
            assert!((a - b).abs() < 1e-9, "{a} vs {b}");
        }
    }

    /// GATE (L2.1): bit-identical runs — same field, same steps, equal to
    /// the last bit in spectral state.
    #[test]
    fn bit_identical_runs() {
        let n = 32;
        let field = seeded_field(n, 0x51DE);
        let mut a = TorusFlow::from_vorticity(&field, n, Dissipation::viscous(0.01));
        let mut b = TorusFlow::from_vorticity(&field, n, Dissipation::viscous(0.01));
        a.run(4e-3, 300);
        b.run(4e-3, 300);
        assert_eq!(a.omega_hat(), b.omega_hat());
    }

    /// The declared truncation: from full-spectrum noise, every mode outside
    /// the 2/3 set is exactly zero at construction and stays zero through a
    /// nonlinear step.
    #[test]
    fn dealiasing_truncates_to_two_thirds() {
        let n = 32;
        let mut rng = SplitMix64::new(0xA11A);
        let noise: Vec<f64> =
            (0..n * n).map(|_| (rng.next() >> 11) as f64 / (1u64 << 53) as f64 - 0.5).collect();
        let mut flow = TorusFlow::from_vorticity(&noise, n, Dissipation::INVISCID);
        flow.step(1e-3);
        let k_max = (n / 3) as f64;
        for idx in 0..n * n {
            let kx = flow.wavenumber[idx % n];
            let ky = flow.wavenumber[idx / n];
            if kx.abs() > k_max || ky.abs() > k_max {
                assert_eq!(flow.omega_hat()[idx], Complex::ZERO, "mode ({kx},{ky}) alive");
            }
        }
    }

    /// The k = 0 mode is carried and conserved — no velocity from it, no
    /// drift of it (its advection tendency is the mean of a divergence).
    #[test]
    fn mean_vorticity_is_carried_and_conserved() {
        let n = 32;
        let field: Vec<f64> = seeded_field(n, 0x3EA7).iter().map(|w| w + 0.3).collect();
        let mut flow = TorusFlow::from_vorticity(&field, n, Dissipation::INVISCID);
        let m0 = flow.mean_vorticity();
        assert!((m0 - 0.3).abs() < 1e-12, "mean lost at construction: {m0}");
        flow.run(5e-3, 200);
        assert!((flow.mean_vorticity() - m0).abs() < 1e-12, "mean drifted");
    }

    /// Hyperviscosity on a single mode decays at exactly e^(−ν_p·k^(2p)·t):
    /// mode (3,0), p = 2, rate ν_p·81 — the operator's closed form.
    #[test]
    fn hyperviscosity_decays_single_mode_exactly() {
        let n = 32;
        let hyper_nu = 1e-3;
        let omega: Vec<f64> =
            (0..n * n).map(|idx| (3.0 * grid_xy(n, idx).0).cos()).collect();
        let d = Dissipation { nu: 0.0, hyper_nu, hyper_order: 2 };
        let mut flow = TorusFlow::from_vorticity(&omega, n, d);
        let dt = 2e-3;
        let steps = 250;
        flow.run(dt, steps);
        let decay = (-hyper_nu * 81.0 * dt * steps as f64).exp();
        let field = flow.vorticity();
        for idx in 0..n * n {
            let expected = omega[idx] * decay;
            assert!((field[idx] - expected).abs() < 1e-8, "at {idx}");
        }
    }
}
