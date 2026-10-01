//! pacific-geometry — the SOLVER, in the substrate's language.
//!
//! A 1:1 port of PacificStore's pure engine modules (Swift → Rust), keeping every
//! constant, version tag, seed and determinism contract bit-for-bit. The Swift
//! originals remain the iOS callers' copy until the FFI swap; THIS crate is the
//! canonical implementation from 10 Aug 2026 (FEED-TRANSMISSION 0-ANSWER-I: the
//! CFD reading — these modules are the integrator, calibration rig, multigrid,
//! spectral pass and attitude control of one solver).
//!
//! House rules carried over: deterministic (seeded, versioned, no wall clocks),
//! no LLM anywhere (I-D), computed-never-stored lenses, refuse-don't-clamp.

pub mod vec;
pub mod space_calibration;
pub mod chromodynamics;
pub mod spectral;
pub mod ontology_anchors;
pub mod attitude;
pub mod coarsening;
pub mod transmission;
pub mod hydrated_kernel;
pub mod candidate_register;
pub mod graph_resolution;
pub mod causal_lens;

/// Hamiltonian CFD — the point-vortex solver (0-ANSWER-L).
pub mod vortex;

/// The spectral substrate — radix-2 FFT (turbulence-lanes M6, slice L2.0).
pub mod fft;

/// Pseudo-spectral CFD — 2-D vorticity on T² (turbulence-lanes M7, slice L2.1).
pub mod pseudo_spectral;
