//! Performance measurement for the vortex solver — release-mode, single core.
//! The LIBRARY is clock-free; measurement lives out here. Run:
//!   cargo run -p pacific-geometry --example bench_vortex --release
//!
//! Cost model: one step = (1 + MIDPOINT_ITERATIONS) Biot–Savart evaluations,
//! each O(N²) with ~10 flops per ordered pair — so pair-interactions/second is
//! the honest throughput unit.

use pacific_geometry::vortex::{Vortex, VortexField};
use std::time::Instant;

fn field(n: usize) -> VortexField {
    fn splitmix(state: &mut u64) -> f64 {
        *state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
    let mut s = 0xBEEF_00D5_u64 ^ n as u64;
    let vortices = (0..n)
        .map(|i| Vortex {
            x: splitmix(&mut s) * 4.0 - 2.0,
            y: splitmix(&mut s) * 4.0 - 2.0,
            circulation: if i % 2 == 0 { 0.9 } else { -0.7 },
        })
        .collect();
    VortexField::new(vortices, 0.05)
}

fn main() {
    println!("N       steps    total_ms   µs/step    steps/s     pair-int/s   H-drift");
    for &n in &[6usize, 16, 64, 128, 256, 512, 1024, 2048] {
        // Calibrate step counts so each row runs ~0.3–1s.
        let steps = (30_000_000 / (n * n)).clamp(20, 200_000);
        let mut f = field(n);
        let h0 = f.hamiltonian();
        // Warm-up (page in, branch-train), untimed.
        f.step(1e-4);
        let t0 = Instant::now();
        f.run(1e-4, steps);
        let dt = t0.elapsed();
        let h1 = f.hamiltonian();
        let per_step = dt.as_secs_f64() / steps as f64;
        // 13 velocity evaluations per step, N(N−1) ordered pairs each.
        let pairs = 13.0 * (n * (n - 1)) as f64 / per_step;
        println!(
            "{:<7} {:<8} {:>8.1}  {:>9.2} {:>10.0}  {:>12.3e}   {:+.2e}",
            n,
            steps,
            dt.as_secs_f64() * 1e3,
            per_step * 1e6,
            1.0 / per_step,
            pairs,
            (h1 - h0) / h0.abs().max(1.0),
        );
    }

    // The product regime: 100 grains, one solver pass per 3s sync tick.
    let n = 100;
    let mut f = field(n);
    let t0 = Instant::now();
    f.run(1e-3, 1000); // a full 1000-step trajectory, not one step
    let traj = t0.elapsed();
    println!(
        "\nproduct regime: N=100 grains, 1000-step trajectory = {:.2} ms \
         ({}x inside a 3s sync tick)",
        traj.as_secs_f64() * 1e3,
        (3.0 / traj.as_secs_f64()) as u64
    );

    // The V&V suite's heaviest study, timed end to end.
    let t0 = Instant::now();
    let mut g = field(6);
    g.run(1.0 / 8000.0, 8000);
    println!(
        "V&V finest run: N=6, 8000 steps = {:.2} ms",
        t0.elapsed().as_secs_f64() * 1e3
    );
}
