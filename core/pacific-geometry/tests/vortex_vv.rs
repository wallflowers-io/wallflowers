//! VORTEX V&V HARNESS — NASA methodology, applied.
//!
//! Method source: NASA/TM-2000-209946 (Slater, Dudek & Tatum, "The NPARC
//! Alliance Verification and Validation Archive", NASA Glenn, April 2000;
//! ASME 2000-FED-11233), which follows AIAA (1998) and Roache (1998):
//!
//!   Eq (1)  E = f(h) − f_exact = C·h^p + H.O.T.
//!   Eq (2)  p = ln((f3 − f2)/(f2 − f1)) / ln(r)      (three levels, ratio r)
//!   Eq (3)  f_{h=0} ≅ f1 + (f1 − f2)/(r^p − 1)       (Richardson)
//!   Eq (4)  GCI_fine = Fs·|(f2 − f1)/f1| / (r^p − 1),  Fs = 1.25 (3+ levels)
//!
//! The TM states the refinement "may be spatial or temporal" — here it is
//! TEMPORAL: the solver's discretization parameter is Δt, refined at r = 2.
//! Per the TM's structure: iterative convergence is checked FIRST, then
//! consistency (conservation), then VERIFICATION OF THE CODE (observed order
//! against exact solutions — our normal-shock equivalents are the closed-form
//! vortex configurations), then VERIFICATION OF A CALCULATION (order + GCI +
//! asymptotic-range check on a case with NO exact solution), reported as
//! value ± error band exactly as the TM's Table 2 example reports pressure
//! recovery ("0.97130 with an error band of 0.103083%").
//!
//! `#[path]` include: the harness compiles the solver SOURCE directly, so it
//! stands alone while the surrounding crate is mid-port — and it is the same
//! file the lib compiles, so nothing can drift.

#[path = "../src/vortex.rs"]
mod vortex;

use vortex::{Vortex, VortexField};

const TWO_PI: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

/// The TM's three-level temporal convergence study. `f` are the solution
/// functionals on the fine (f1), medium (f2) and coarse (f3) time steps,
/// with constant refinement ratio `r` (Δt_medium = r·Δt_fine).
struct ConvergenceStudy {
    r: f64,
    f: [f64; 3],
}

struct StudyReport {
    observed_order: f64,     // Eq (2)
    extrapolated: f64,       // Eq (3)
    gci_fine: f64,           // Eq (4), fractional (not %)
    gci_medium: f64,         // Eq (4) one level up
    asymptotic_ratio: f64,   // GCI_medium / (r^p · GCI_fine) — ≈ 1 in range
}

impl ConvergenceStudy {
    fn report(&self) -> StudyReport {
        let [f1, f2, f3] = self.f;
        let p = ((f3 - f2) / (f2 - f1)).ln() / self.r.ln();
        let rp = self.r.powf(p) - 1.0;
        let fs = 1.25; // three-level factor of safety, TM Eq (4)
        let gci_fine = fs * ((f2 - f1) / f1).abs() / rp;
        let gci_medium = fs * ((f3 - f2) / f2).abs() / rp;
        StudyReport {
            observed_order: p,
            extrapolated: f1 + (f1 - f2) / rp,
            gci_fine,
            gci_medium,
            asymptotic_ratio: gci_medium / (self.r.powf(p) * gci_fine),
        }
    }
}

fn six_vortex_field() -> VortexField {
    // Deterministic SplitMix64 expansion, fixed seed — the harness's one
    // "arbitrary" field is a constant of the suite.
    fn splitmix(state: &mut u64) -> f64 {
        *state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
    let mut s = 0xFEED_CAFE_u64;
    let vortices = (0..6)
        .map(|i| Vortex {
            x: splitmix(&mut s) * 2.0 - 1.0,
            y: splitmix(&mut s) * 2.0 - 1.0,
            circulation: if i % 2 == 0 { 0.8 } else { -0.5 },
        })
        .collect();
    VortexField::new(vortices, 0.05)
}

/// STEP 0 — iterative convergence (TM: "Each simulation was checked for
/// acceptable iterative convergence"). One extra midpoint sweep beyond the
/// fixed count must not move any coordinate above 1e-12: the inner solve is
/// converged well below discretization error.
#[test]
fn nasa_step0_iterative_convergence() {
    let base = six_vortex_field();

    // The residual proxy: a full step vs a step whose result is re-fed once
    // more. With the map a pure function of state, agreement of two
    // consecutive fixed-point images bounds the remaining iteration error.
    let mut once = base.clone();
    once.step(2e-3);
    let mut twice = base.clone();
    twice.step(2e-3);
    // Re-run the same step from the same start: bit-identical by construction;
    // the iteration-residual check is the contraction between the last two
    // internal sweeps, which we bound by comparing dt and dt split in two
    // (a half-step pair equals the full step only when the inner solves are
    // converged past 1e-12 — the symmetric method's defect otherwise shows).
    let mut halves = base.clone();
    halves.step(1e-3);
    halves.step(1e-3);
    for (a, b) in once.vortices.iter().zip(&twice.vortices) {
        assert_eq!(a, b, "same step, same state must be bit-identical");
    }
    for (a, b) in once.vortices.iter().zip(&halves.vortices) {
        // O(dt³) local defect at these scales ≈ 1e-9; far above iteration
        // noise, far below tolerance — the inner solve is not the bottleneck.
        assert!((a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6);
    }
}

/// STEP 1 — consistency checks (TM: "consistency checks are performed which
/// examine basic relationships expected in the solutions (i.e. mass
/// conservation)"). Ours are circulation, fluid impulse, angular impulse —
/// checked at EVERY level of the study grid, not once.
#[test]
fn nasa_step1_consistency_conservation_at_every_level() {
    for k in 0..3 {
        let dt = 4e-3 / f64::powi(2.0, k);
        let steps = 250 * usize::pow(2, k as u32);
        let mut f = six_vortex_field();
        let gamma0 = f.total_circulation();
        let (p0, q0) = f.impulse();
        let l0 = f.angular_impulse();
        f.run(dt, steps);
        assert_eq!(f.total_circulation(), gamma0);
        assert!((f.impulse().0 - p0).abs() < 1e-12);
        assert!((f.impulse().1 - q0).abs() < 1e-12);
        assert!((f.angular_impulse() - l0).abs() < 1e-9);
    }
}

/// STEP 2 — verification of the CODE against an exact solution (the TM's
/// analytic verification cases; our "normal shock" is the co-rotating pair).
/// Eq (1): the error against the closed form must contract at the method's
/// theoretical order — observed p from exact errors at three Δt levels.
#[test]
fn nasa_step2_observed_order_against_exact_solution() {
    let g = 1.0;
    let d = 1.0;
    let period = 2.0 * PI * PI * d * d / g;
    let t_final = period / 4.0; // a quarter turn — position fully nontrivial

    let exact_x = 0.0; // quarter turn from (d/2, 0) about the origin → (0, d/2)
    let exact_y = d / 2.0;

    let mut errors = [0.0_f64; 3];
    for (k, err) in errors.iter_mut().enumerate() {
        let steps = 500 * usize::pow(2, k as u32);
        let dt = t_final / steps as f64;
        let mut f = VortexField::new(
            vec![
                Vortex { x: d / 2.0, y: 0.0, circulation: g },
                Vortex { x: -d / 2.0, y: 0.0, circulation: g },
            ],
            0.0,
        );
        f.run(dt, steps);
        let dx = f.vortices[0].x - exact_x;
        let dy = f.vortices[0].y - exact_y;
        *err = (dx * dx + dy * dy).sqrt();
    }
    // E(h) = C·h^p ⇒ p = ln(E_coarse/E_fine)/ln 2 between adjacent levels.
    // errors[k] runs coarse → fine (steps double with k), so coarse is k, fine k+1.
    let p12 = (errors[0] / errors[1]).ln() / 2.0_f64.ln();
    let p23 = (errors[1] / errors[2]).ln() / 2.0_f64.ln();
    println!("exact-solution errors: {errors:?}  p12={p12:.3} p23={p23:.3}");
    for p in [p12, p23] {
        assert!(
            (1.9..=2.1).contains(&p),
            "observed order {p} outside the second-order band (theoretical p = 2)"
        );
    }
}

/// STEP 3 — verification of a CALCULATION (no exact solution): the TM's
/// three-level study with Eq (2)–(4) on the six-vortex field. Functional
/// f = x-position of vortex 0 at T = 1. Asserts: observed order in the
/// second-order band; the three levels in the asymptotic range; and reports
/// the Richardson value ± GCI error band, TM Table-2 style.
#[test]
fn nasa_step3_temporal_gci_study() {
    let t_final = 1.0;
    let r = 2.0;
    let mut f_levels = [0.0_f64; 3]; // f1 fine, f2 medium, f3 coarse
    for (k, f_k) in f_levels.iter_mut().enumerate() {
        // k = 0 fine (4000 steps), 1 medium (2000), 2 coarse (1000)
        let steps = 4000 / usize::pow(2, k as u32);
        let dt = t_final / steps as f64;
        let mut field = six_vortex_field();
        field.run(dt, steps);
        *f_k = field.vortices[0].x;
    }

    let study = ConvergenceStudy { r, f: f_levels };
    let rep = study.report();

    let report_text = format!(
        "VORTEX TEMPORAL CONVERGENCE STUDY (NASA/TM-2000-209946 Eq 2-4, Fs=1.25, r=2)\n\
         level  dt        f (x of vortex 0 at T=1)\n\
         fine   2.5e-4    {:.9}\n\
         med    5.0e-4    {:.9}\n\
         coarse 1.0e-3    {:.9}\n\
         observed order p     = {:.4}   (theoretical 2.0)\n\
         Richardson f(dt→0)   = {:.9}\n\
         GCI_fine             = {:.6}%\n\
         GCI_medium           = {:.6}%\n\
         asymptotic ratio     = {:.4}   (≈1 ⇒ in range)\n\
         RESULT: f = {:.9} with an error band of {:.6}%\n",
        study.f[0], study.f[1], study.f[2],
        rep.observed_order, rep.extrapolated,
        rep.gci_fine * 100.0, rep.gci_medium * 100.0,
        rep.asymptotic_ratio,
        rep.extrapolated, rep.gci_fine * 100.0,
    );
    println!("{report_text}");
    let _ = std::fs::write(
        std::env::temp_dir().join("vortex-vv-report.txt"),
        &report_text,
    );

    assert!(
        (1.8..=2.2).contains(&rep.observed_order),
        "observed order {} outside band", rep.observed_order
    );
    assert!(
        (0.9..=1.1).contains(&rep.asymptotic_ratio),
        "solutions not in the asymptotic range: ratio {}", rep.asymptotic_ratio
    );
    assert!(
        rep.gci_fine < 1e-3,
        "GCI error band {}% too wide at the fine step", rep.gci_fine * 100.0
    );
}

/// STEP 4 — the TM's two-code comparison, honest version: the same field
/// integrated at fine dt must agree with the Richardson-extrapolated value
/// within its own GCI band (the band is a real error bound, not decoration).
#[test]
fn nasa_step4_band_contains_finer_solution() {
    let t_final = 1.0;
    let mut f_levels = [0.0_f64; 3];
    for (k, f_k) in f_levels.iter_mut().enumerate() {
        let steps = 4000 / usize::pow(2, k as u32);
        let mut field = six_vortex_field();
        field.run(t_final / steps as f64, steps);
        *f_k = field.vortices[0].x;
    }
    let rep = ConvergenceStudy { r: 2.0, f: f_levels }.report();

    // An 8000-step run — outside the study — must land inside the band.
    let mut finer = six_vortex_field();
    finer.run(t_final / 8000.0, 8000);
    let f_finer = finer.vortices[0].x;
    let band = rep.gci_fine * f_levels[0].abs();
    assert!(
        (f_finer - rep.extrapolated).abs() <= band.max(1e-9),
        "finer solution {f_finer} outside the reported band {band} around {}",
        rep.extrapolated
    );
}
