//! NIGHTWATER — the verified engine on screen, tonight.
//!
//! Composition (the multiphysics, honestly labelled):
//!   1. VORTEX LAYER — pacific_geometry::vortex, unmodified: the NASA-verified
//!      Hamiltonian solver. Invariants (P, Q, L, H) exported live; injections
//!      are user-applied impulses, so the HUD re-baselines on injection.
//!   2. INK LAYER — passive tracers advected by the engine's OWN Biot–Savart
//!      operator (`induced_velocities`), RK2 midpoint. Passive: no back-force.
//!   3. SURFACE LAYER — a 1-D damped wave equation h_tt = c²h_xx − γh_t + βv_y,
//!      forced by the flow's vertical velocity sampled along y = 0.
//!      DECLARED demo-grade and one-way coupled.
//!   4. SPRAY LAYER — ballistic droplets shed where the surface runs steep:
//!      gravity + drag toward the local flow velocity. DECLARED demo-grade.
//!
//! Determinism: fixed seed, fixed dt, fixed substeps, SplitMix64 only
//! (REUSED from pacific_geometry::spectral — never copied). No clocks in here;
//! the page measures µs/step around the call. wasm is single-threaded; state
//! lives in one Box behind `init`.

use pacific_geometry::spectral::SplitMix64;
use pacific_geometry::vortex::{Vortex, VortexField};

const TRACERS: usize = 12_000;
const SURF_N: usize = 384;
const SURF_X0: f64 = -1.6;
const SURF_X1: f64 = 1.6;
const SPRAY_MAX: usize = 1_500;
const DT: f64 = 1.0 / 120.0; // physics step; 2 substeps per 60 fps frame
const SUBSTEPS: usize = 2;

struct Spray {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    life: f64,
}

struct Sim {
    field: VortexField,
    rng: SplitMix64,
    tracers: Vec<(f64, f64)>,
    tracer_out: Vec<f32>, // x,y interleaved for the page
    surf_h: Vec<f64>,
    surf_v: Vec<f64>,
    surf_out: Vec<f32>,
    spray: Vec<Spray>,
    spray_out: Vec<f32>,
    vort_out: Vec<f32>, // x,y,Γ interleaved
    // HUD baselines — re-zeroed by inject() because an injection IS an impulse.
    p0: f64,
    q0: f64,
    h0: f64,
}

fn unit(rng: &mut SplitMix64) -> f64 {
    (rng.next() >> 11) as f64 / (1u64 << 53) as f64
}

impl Sim {
    fn new(seed: u64) -> Sim {
        let mut rng = SplitMix64::new(seed);
        // McWilliams-flavoured start: a band of mixed-sign blobs in the "wind"
        // (above the surface) and a gentler band in the "water" (below).
        let mut vortices = Vec::new();
        for i in 0..96 {
            vortices.push(Vortex {
                x: unit(&mut rng) * 2.4 - 1.2,
                y: 0.12 + unit(&mut rng) * 0.75,
                circulation: (if i % 2 == 0 { 1.0 } else { -1.0 }) * (0.25 + 0.55 * unit(&mut rng)),
            });
        }
        for i in 0..64 {
            vortices.push(Vortex {
                x: unit(&mut rng) * 2.4 - 1.2,
                y: -0.12 - unit(&mut rng) * 0.75,
                circulation: (if i % 2 == 0 { 1.0 } else { -1.0 }) * (0.10 + 0.25 * unit(&mut rng)),
            });
        }
        let field = VortexField::new(vortices, 0.03);

        let mut tracers = Vec::with_capacity(TRACERS);
        for _ in 0..TRACERS {
            tracers.push((unit(&mut rng) * 3.2 - 1.6, unit(&mut rng) * 2.2 - 1.1));
        }

        let (p0, q0) = field.impulse();
        let h0 = field.hamiltonian();
        Sim {
            field,
            rng,
            tracers,
            tracer_out: vec![0.0; TRACERS * 2],
            surf_h: vec![0.0; SURF_N],
            surf_v: vec![0.0; SURF_N],
            surf_out: vec![0.0; SURF_N],
            spray: Vec::with_capacity(SPRAY_MAX),
            spray_out: vec![0.0; SPRAY_MAX * 2],
            vort_out: Vec::new(),
            p0,
            q0,
            h0,
        }
    }

    fn step_frame(&mut self) {
        // 1 · the verified solver, fixed substeps.
        for _ in 0..SUBSTEPS {
            self.field.step(DT);
        }
        let dt = DT * SUBSTEPS as f64;

        // 2 · ink: RK2 midpoint through the engine's Biot–Savart operator.
        let u1 = self.field.induced_velocities(&self.tracers);
        let mid: Vec<(f64, f64)> = self
            .tracers
            .iter()
            .zip(&u1)
            .map(|(&(x, y), &(u, v))| (x + 0.5 * dt * u, y + 0.5 * dt * v))
            .collect();
        let u2 = self.field.induced_velocities(&mid);
        for (i, t) in self.tracers.iter_mut().enumerate() {
            t.0 += dt * u2[i].0;
            t.1 += dt * u2[i].1;
            // Ink that leaves the stage is respawned upstream, deterministically.
            if t.0 < -1.7 || t.0 > 1.7 || t.1 < -1.15 || t.1 > 1.15 {
                *t = (unit(&mut self.rng) * 3.2 - 1.6, unit(&mut self.rng) * 2.2 - 1.1);
            }
        }

        // 3 · surface: damped wave equation forced by v_y along y = 0.
        let dx = (SURF_X1 - SURF_X0) / (SURF_N - 1) as f64;
        let cols: Vec<(f64, f64)> = (0..SURF_N)
            .map(|i| (SURF_X0 + i as f64 * dx, 0.0))
            .collect();
        let flow = self.field.induced_velocities(&cols);
        let c2 = 0.36; // c = 0.6 domain-units/s
        let gamma = 1.6;
        let beta = 0.55;
        let mut acc = vec![0.0f64; SURF_N];
        for i in 1..SURF_N - 1 {
            let lap = (self.surf_h[i - 1] - 2.0 * self.surf_h[i] + self.surf_h[i + 1]) / (dx * dx);
            acc[i] = c2 * lap - gamma * self.surf_v[i] + beta * flow[i].1;
        }
        for i in 0..SURF_N {
            self.surf_v[i] += dt * acc[i];
            self.surf_h[i] += dt * self.surf_v[i];
        }
        self.surf_h[0] = 0.0;
        self.surf_h[SURF_N - 1] = 0.0;

        // 4 · spray: shed on steep, fast crests; gravity + drag to local flow.
        for i in 1..SURF_N - 1 {
            if self.spray.len() >= SPRAY_MAX {
                break;
            }
            let slope = (self.surf_h[i + 1] - self.surf_h[i - 1]) / (2.0 * dx);
            if slope.abs() > 0.55 && self.surf_v[i] > 0.35 && unit(&mut self.rng) < 0.08 {
                self.spray.push(Spray {
                    x: SURF_X0 + i as f64 * dx,
                    y: self.surf_h[i] * 0.12,
                    vx: 0.4 * slope.signum() * unit(&mut self.rng),
                    vy: 0.5 + 0.6 * self.surf_v[i],
                    life: 1.8,
                });
            }
        }
        if !self.spray.is_empty() {
            let pos: Vec<(f64, f64)> = self.spray.iter().map(|s| (s.x, s.y)).collect();
            let wind = self.field.induced_velocities(&pos);
            let g = 1.2;
            let inv_tau = 1.0 / 0.35;
            for (s, w) in self.spray.iter_mut().zip(&wind) {
                s.vx += dt * ((w.0 - s.vx) * inv_tau);
                s.vy += dt * ((w.1 - s.vy) * inv_tau - g);
                s.x += dt * s.vx;
                s.y += dt * s.vy;
                s.life -= dt;
            }
            self.spray.retain(|s| s.life > 0.0 && s.y > -0.05);
        }

        // Pack the render buffers (f32 for the page).
        for (i, &(x, y)) in self.tracers.iter().enumerate() {
            self.tracer_out[i * 2] = x as f32;
            self.tracer_out[i * 2 + 1] = y as f32;
        }
        for i in 0..SURF_N {
            self.surf_out[i] = (self.surf_h[i] * 0.12) as f32;
        }
        for (i, s) in self.spray.iter().enumerate() {
            self.spray_out[i * 2] = s.x as f32;
            self.spray_out[i * 2 + 1] = s.y as f32;
        }
        self.vort_out.clear();
        for v in &self.field.vortices {
            self.vort_out.push(v.x as f32);
            self.vort_out.push(v.y as f32);
            self.vort_out.push(v.circulation as f32);
        }
    }

    fn inject_dipole(&mut self, x: f64, y: f64, ux: f64, uy: f64) {
        // A drag is an impulse: a counter-rotating pair whose self-induced
        // velocity points along the drag. Perpendicular offset, ±Γ.
        let speed = (ux * ux + uy * uy).sqrt();
        if speed < 1e-6 {
            return;
        }
        let (nx, ny) = (-uy / speed, ux / speed);
        let d = 0.05;
        let gamma = (speed * 2.0).clamp(0.05, 0.9) * std::f64::consts::TAU * d;
        self.field.vortices.push(Vortex { x: x + nx * d / 2.0, y: y + ny * d / 2.0, circulation: -gamma });
        self.field.vortices.push(Vortex { x: x - nx * d / 2.0, y: y - ny * d / 2.0, circulation: gamma });
        // The user applied an impulse; conservation restarts from here.
        let (p, q) = self.field.impulse();
        self.p0 = p;
        self.q0 = q;
        self.h0 = self.field.hamiltonian();
    }

    fn ink_drop(&mut self, x: f64, y: f64) {
        // Re-seat the oldest 600 tracers as a tight drop — pure relabelling,
        // no dynamics touched.
        let n = 600.min(self.tracers.len());
        for i in 0..n {
            let a = unit(&mut self.rng) * std::f64::consts::TAU;
            let r = 0.035 * unit(&mut self.rng).sqrt();
            self.tracers[i] = (x + r * a.cos(), y + r * a.sin());
        }
    }
}

static mut SIM: Option<Box<Sim>> = None;

fn sim() -> &'static mut Sim {
    unsafe {
        #[allow(static_mut_refs)]
        SIM.as_mut().expect("init first")
    }
}

#[no_mangle]
pub extern "C" fn init(seed: u64) {
    unsafe {
        SIM = Some(Box::new(Sim::new(seed)));
    }
}

#[no_mangle]
pub extern "C" fn step_frame() {
    sim().step_frame();
}

#[no_mangle]
pub extern "C" fn tracers_ptr() -> *const f32 {
    sim().tracer_out.as_ptr()
}
#[no_mangle]
pub extern "C" fn tracers_count() -> u32 {
    TRACERS as u32
}
#[no_mangle]
pub extern "C" fn surface_ptr() -> *const f32 {
    sim().surf_out.as_ptr()
}
#[no_mangle]
pub extern "C" fn surface_count() -> u32 {
    SURF_N as u32
}
#[no_mangle]
pub extern "C" fn surface_x0() -> f64 {
    SURF_X0
}
#[no_mangle]
pub extern "C" fn surface_x1() -> f64 {
    SURF_X1
}
#[no_mangle]
pub extern "C" fn spray_ptr() -> *const f32 {
    sim().spray_out.as_ptr()
}
#[no_mangle]
pub extern "C" fn spray_count() -> u32 {
    sim().spray.len() as u32
}
#[no_mangle]
pub extern "C" fn vortices_ptr() -> *const f32 {
    sim().vort_out.as_ptr()
}
#[no_mangle]
pub extern "C" fn vortices_count() -> u32 {
    sim().field.vortices.len() as u32
}
#[no_mangle]
pub extern "C" fn impulse_p_drift() -> f64 {
    let s = sim();
    s.field.impulse().0 - s.p0
}
#[no_mangle]
pub extern "C" fn impulse_q_drift() -> f64 {
    let s = sim();
    s.field.impulse().1 - s.q0
}
#[no_mangle]
pub extern "C" fn h_rel_drift() -> f64 {
    let s = sim();
    (s.field.hamiltonian() - s.h0) / s.h0.abs().max(1.0)
}
#[no_mangle]
pub extern "C" fn inject(x: f64, y: f64, ux: f64, uy: f64) {
    sim().inject_dipole(x, y, ux, uy);
}
#[no_mangle]
pub extern "C" fn ink(x: f64, y: f64) {
    sim().ink_drop(x, y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_frames() {
        let mut a = Sim::new(7);
        let mut b = Sim::new(7);
        for _ in 0..30 {
            a.step_frame();
            b.step_frame();
        }
        assert_eq!(a.tracer_out, b.tracer_out);
        assert_eq!(a.surf_out, b.surf_out);
        assert_eq!(a.spray.len(), b.spray.len());
    }

    #[test]
    fn engine_invariants_hold_inside_the_show() {
        let mut s = Sim::new(11);
        let (p0, q0) = s.field.impulse();
        for _ in 0..240 {
            s.step_frame();
        }
        let (p1, q1) = s.field.impulse();
        assert!((p1 - p0).abs() < 1e-12, "P drifted under the demo layers");
        assert!((q1 - q0).abs() < 1e-12, "Q drifted under the demo layers");
    }

    #[test]
    fn injection_rebaselines() {
        let mut s = Sim::new(3);
        s.step_frame();
        s.inject_dipole(0.0, 0.4, 0.3, 0.0);
        assert!(s.field.impulse().0 - s.p0 == 0.0);
        for _ in 0..60 {
            s.step_frame();
        }
        assert!((s.field.impulse().0 - s.p0).abs() < 1e-12);
    }
}
