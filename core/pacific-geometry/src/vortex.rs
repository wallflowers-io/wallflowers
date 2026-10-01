//! HAMILTONIAN CFD — the point-vortex system (Helmholtz/Kirchhoff), the solver
//! of 0-ANSWER-I in the space 0-ANSWER-L names.
//!
//! Ideal 2-D flow, reduced to its low-order Hamiltonian skeleton: N vortices at
//! z_i with circulations Γ_i, evolving under
//!
//! ```text
//!     H = −(1/4π) Σ_{i<j} Γ_i Γ_j ln(|z_i − z_j|² + ε²)
//! ```
//!
//! with the weighted symplectic structure Γ_i dx_i ∧ dy_i (position IS the
//! phase space — x is y's conjugate momentum; a vortex has no independent
//! velocity state). ε = 0 is the classical singular kernel; ε > 0 is the vortex
//! blob (Chorin/Krasny) — a DECLARED core, never a silent clamp.
//!
//! The product reading (FEED-TRANSMISSION S35/S38): a vortex is a CONVERGED
//! MASS — a CoarseningPyramid grain placed on a 2-D reduction of the calibrated
//! space (the principal plane, or the charge chart), with circulation =
//! mass × coherence. This solver is the instrument for how causes advect one
//! another: same-sign convergences co-rotate (scenes), opposite-sign pairs
//! translate (a dialogue moving through the space). It predicts FAMILIES of
//! motion — S9 forbids reading single trajectories as verdicts.
//!
//! INTEGRATOR: implicit midpoint — symplectic for the general (non-separable)
//! H, symmetric (time-reversible), and it conserves LINEAR and QUADRATIC
//! invariants EXACTLY (Cooper): the fluid impulse (ΣΓx, ΣΓy) — the very
//! quantity 0-ANSWER-H is about — and the angular impulse ΣΓ|z|², to machine
//! precision, while H itself oscillates boundedly with NO drift. Fixed
//! iteration count, no early exit, no RNG, no clocks: bit-identical runs.

/// One vortex: a position on the plane and a signed circulation.
/// Γ > 0 turns counter-clockwise. Γ carries the physical weight — for a
/// converged grain, circulation = mass × coherence (signed by charge polarity
/// if the caller has one; sign is orientation, not valence).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vortex {
    pub x: f64,
    pub y: f64,
    pub circulation: f64,
}

/// The field: the vortex set plus the declared core radius ε.
#[derive(Clone, Debug, PartialEq)]
pub struct VortexField {
    pub vortices: Vec<Vortex>,
    /// Blob core (ε). 0 = classical point vortices. The regularisation keeps
    /// the SAME Hamiltonian structure (a smoothed kernel is still a kernel —
    /// S38), so every conservation statement below holds for any ε.
    pub core: f64,
}

/// Fixed-point iterations for the implicit midpoint solve. Fixed — never
/// adaptive — so the map is a pure function of (state, dt).
const MIDPOINT_ITERATIONS: usize = 12;

const TWO_PI: f64 = std::f64::consts::TAU;

impl VortexField {
    pub fn new(vortices: Vec<Vortex>, core: f64) -> Self {
        Self { vortices, core }
    }

    /// Place converged grains on a 2-D reduction: positions from the caller's
    /// chart (principal plane / charge chart), circulation = mass × coherence.
    /// Pairs are zipped positionally; length mismatch is a caller error and
    /// truncates to the shorter (refuse-don't-invent).
    pub fn from_grains(positions: &[(f64, f64)], circulations: &[f64], core: f64) -> Self {
        let vortices = positions
            .iter()
            .zip(circulations)
            .map(|(&(x, y), &circulation)| Vortex { x, y, circulation })
            .collect();
        Self { vortices, core }
    }

    /// The Hamiltonian. Coincident vortices at ε = 0 would be −∞; the pair is
    /// SKIPPED (its interaction is undefined, not infinite) so the instrument
    /// stays finite and the caller can see the degeneracy in the invariant
    /// drift instead of a NaN.
    pub fn hamiltonian(&self) -> f64 {
        let eps2 = self.core * self.core;
        let n = self.vortices.len();
        let mut h = 0.0;
        for i in 0..n {
            for j in (i + 1)..n {
                let a = self.vortices[i];
                let b = self.vortices[j];
                let dx = a.x - b.x;
                let dy = a.y - b.y;
                let r2 = dx * dx + dy * dy + eps2;
                if r2 > 0.0 {
                    h -= a.circulation * b.circulation * r2.ln() / (2.0 * TWO_PI);
                }
            }
        }
        h
    }

    /// The fluid impulse (ΣΓx, ΣΓy) — conserved EXACTLY by the integrator.
    pub fn impulse(&self) -> (f64, f64) {
        self.vortices.iter().fold((0.0, 0.0), |(p, q), v| {
            (p + v.circulation * v.x, q + v.circulation * v.y)
        })
    }

    /// The angular impulse ΣΓ|z|² — quadratic, conserved exactly by midpoint.
    pub fn angular_impulse(&self) -> f64 {
        self.vortices
            .iter()
            .map(|v| v.circulation * (v.x * v.x + v.y * v.y))
            .sum()
    }

    /// Total circulation ΣΓ — trivially conserved (nothing writes Γ).
    pub fn total_circulation(&self) -> f64 {
        self.vortices.iter().map(|v| v.circulation).sum()
    }

    /// The induced velocity field sampled at arbitrary positions — the
    /// Biot–Savart operator as a public instrument (passive tracers, probes,
    /// couplings). Sampling positions are NOT vortices: every vortex
    /// contributes at every sample point (no self-exclusion by index).
    pub fn induced_velocities(&self, pos: &[(f64, f64)]) -> Vec<(f64, f64)> {
        let eps2 = self.core * self.core;
        pos.iter()
            .map(|&(x, y)| {
                let mut u = 0.0;
                let mut v = 0.0;
                for w in &self.vortices {
                    let dx = x - w.x;
                    let dy = y - w.y;
                    let r2 = dx * dx + dy * dy + eps2;
                    if r2 > 0.0 {
                        u -= w.circulation * dy / (TWO_PI * r2);
                        v += w.circulation * dx / (TWO_PI * r2);
                    }
                }
                (u, v)
            })
            .collect()
    }

    /// Biot–Savart velocities at the given positions (usually the midpoint
    /// state). Self-induction is zero (a point vortex does not advect itself).
    fn velocities_at(&self, pos: &[(f64, f64)]) -> Vec<(f64, f64)> {
        let eps2 = self.core * self.core;
        let n = pos.len();
        let mut out = vec![(0.0, 0.0); n];
        for i in 0..n {
            let (xi, yi) = pos[i];
            let mut u = 0.0;
            let mut v = 0.0;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let (xj, yj) = pos[j];
                let g = self.vortices[j].circulation;
                let dx = xi - xj;
                let dy = yi - yj;
                let r2 = dx * dx + dy * dy + eps2;
                if r2 > 0.0 {
                    u -= g * dy / (TWO_PI * r2);
                    v += g * dx / (TWO_PI * r2);
                }
            }
            out[i] = (u, v);
        }
        out
    }

    /// One implicit-midpoint step: z' = z + dt · U((z + z')/2), solved by a
    /// FIXED number of fixed-point iterations seeded with the explicit Euler
    /// guess. Symplectic, symmetric, deterministic.
    pub fn step(&mut self, dt: f64) {
        let n = self.vortices.len();
        if n == 0 {
            return;
        }
        let z0: Vec<(f64, f64)> = self.vortices.iter().map(|v| (v.x, v.y)).collect();

        // Explicit guess.
        let u0 = self.velocities_at(&z0);
        let mut z1: Vec<(f64, f64)> = (0..n)
            .map(|i| (z0[i].0 + dt * u0[i].0, z0[i].1 + dt * u0[i].1))
            .collect();

        for _ in 0..MIDPOINT_ITERATIONS {
            let mid: Vec<(f64, f64)> = (0..n)
                .map(|i| ((z0[i].0 + z1[i].0) * 0.5, (z0[i].1 + z1[i].1) * 0.5))
                .collect();
            let um = self.velocities_at(&mid);
            for i in 0..n {
                z1[i] = (z0[i].0 + dt * um[i].0, z0[i].1 + dt * um[i].1);
            }
        }

        for i in 0..n {
            self.vortices[i].x = z1[i].0;
            self.vortices[i].y = z1[i].1;
        }
    }

    /// Integrate `steps` fixed steps of `dt`. The trajectory is a pure
    /// function of (initial state, dt, steps).
    pub fn run(&mut self, dt: f64, steps: usize) {
        for _ in 0..steps {
            self.step(dt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// A vortex pair (Γ, −Γ) at distance d translates at Γ/(2πd),
    /// perpendicular to the separation — the closed form, hit to 1e-9.
    #[test]
    fn pair_translates_at_closed_form_speed() {
        let g = 1.7;
        let d = 0.9;
        let mut f = VortexField::new(
            vec![
                Vortex { x: 0.0, y: d / 2.0, circulation: g },
                Vortex { x: 0.0, y: -d / 2.0, circulation: -g },
            ],
            0.0,
        );
        let speed = g / (TWO_PI * d);
        let dt = 1e-3;
        let steps = 2000;
        f.run(dt, steps);
        let travelled = dt * steps as f64 * speed;
        assert!(close(f.vortices[0].x, travelled, 1e-9), "x0 = {}", f.vortices[0].x);
        assert!(close(f.vortices[1].x, travelled, 1e-9));
        assert!(close(f.vortices[0].y, d / 2.0, 1e-9));   // separation carried rigidly
        assert!(close(f.vortices[1].y, -d / 2.0, 1e-9));
    }

    /// Two equal vortices co-rotate about their midpoint with period
    /// T = 2π²d²/Γ; after one full period both return home.
    #[test]
    fn corotating_pair_returns_after_one_period() {
        let g = 1.0;
        let d = 1.0;
        let mut f = VortexField::new(
            vec![
                Vortex { x: d / 2.0, y: 0.0, circulation: g },
                Vortex { x: -d / 2.0, y: 0.0, circulation: g },
            ],
            0.0,
        );
        let period = 2.0 * std::f64::consts::PI * std::f64::consts::PI * d * d / g;
        let steps = 20_000;
        let dt = period / steps as f64;
        f.run(dt, steps);
        assert!(close(f.vortices[0].x, d / 2.0, 1e-4));
        assert!(close(f.vortices[0].y, 0.0, 1e-4));
        assert!(close(f.vortices[1].x, -d / 2.0, 1e-4));
    }

    /// Deterministic 6-vortex field (SplitMix64 expansion, seed fixed):
    /// linear and angular impulse conserved to machine precision; H bounded
    /// with no drift.
    #[test]
    fn invariants_survive_a_thousand_steps() {
        fn splitmix(state: &mut u64) -> f64 {
            *state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = *state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            let bits = z ^ (z >> 31);
            (bits >> 11) as f64 / (1u64 << 53) as f64
        }
        let mut s = 0xFEED_CAFE_u64;
        let vortices: Vec<Vortex> = (0..6)
            .map(|i| Vortex {
                x: splitmix(&mut s) * 2.0 - 1.0,
                y: splitmix(&mut s) * 2.0 - 1.0,
                circulation: if i % 2 == 0 { 0.8 } else { -0.5 },
            })
            .collect();
        let mut f = VortexField::new(vortices, 0.05);
        let (p0, q0) = f.impulse();
        let l0 = f.angular_impulse();
        let h0 = f.hamiltonian();
        let mut h_min = h0;
        let mut h_max = h0;
        for _ in 0..1000 {
            f.step(2e-3);
            let h = f.hamiltonian();
            h_min = h_min.min(h);
            h_max = h_max.max(h);
        }
        let (p1, q1) = f.impulse();
        assert!(close(p1, p0, 1e-12), "impulse P drifted: {p0} → {p1}");
        assert!(close(q1, q0, 1e-12), "impulse Q drifted: {q0} → {q1}");
        assert!(close(f.angular_impulse(), l0, 1e-9), "angular impulse drifted");
        let h_scale = h0.abs().max(1.0);
        assert!((h_max - h_min) / h_scale < 1e-4, "H drifted: [{h_min}, {h_max}]");
    }

    /// The map is symmetric: forward n steps then backward n steps returns to
    /// the start — time-reversal to fixed-point tolerance (the ℤ₂ duality,
    /// as an integrator property).
    #[test]
    fn time_reversal_returns_home() {
        let mut f = VortexField::new(
            vec![
                Vortex { x: 0.3, y: 0.1, circulation: 1.0 },
                Vortex { x: -0.4, y: 0.2, circulation: 0.7 },
                Vortex { x: 0.1, y: -0.5, circulation: -0.9 },
            ],
            0.02,
        );
        let start = f.clone();
        f.run(1e-3, 500);
        f.run(-1e-3, 500);
        for (a, b) in f.vortices.iter().zip(&start.vortices) {
            assert!(close(a.x, b.x, 1e-9));
            assert!(close(a.y, b.y, 1e-9));
        }
    }

    /// Bit-identical determinism: two runs from the same state agree exactly.
    #[test]
    fn bit_identical_runs() {
        let init = VortexField::new(
            vec![
                Vortex { x: 0.2, y: 0.0, circulation: 1.1 },
                Vortex { x: -0.2, y: 0.1, circulation: -0.6 },
                Vortex { x: 0.0, y: 0.4, circulation: 0.3 },
            ],
            0.01,
        );
        let mut a = init.clone();
        let mut b = init.clone();
        a.run(5e-3, 700);
        b.run(5e-3, 700);
        assert_eq!(a, b);
    }

    /// from_grains zips positionally and truncates to the shorter input.
    #[test]
    fn grains_place_faithfully() {
        let f = VortexField::from_grains(&[(1.0, 2.0), (3.0, 4.0)], &[0.5, -0.5, 9.9], 0.0);
        assert_eq!(f.vortices.len(), 2);
        assert_eq!(f.vortices[0], Vortex { x: 1.0, y: 2.0, circulation: 0.5 });
        assert_eq!(f.total_circulation(), 0.0);
    }
}
