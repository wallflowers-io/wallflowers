//! CausalLens — THE CAUSAL EMBEDDING MODEL's query operator (8 Aug naming): the
//! composite of the six instruments is an embedding model whose geometry carries
//! the PRECONDITIONS of causal claims — and this lens is where they compose into
//! a causal question: WHAT LED HERE, and WHAT FOLLOWED FROM HERE.
//!
//! Why the name is earned, precisely:
//!   · a frozen encoder is correlational — cosine is symmetric and timeless;
//!   · this stack breaks the symmetry three ways: TIME (fold-anchored spans,
//!     the stream's own clock, bitemporal as-of), DIRECTION (the chromodynamic
//!     arrow — from vs towards is an orientation cosine never had), and
//!     PERSISTENCE (structure that survives coarsening — the vortex test);
//!   · and it closes with ATTESTATION: the model assembles evidence, a person
//!     establishes the cause (a minted same_as / led_to).
//!
//! And the boundary, stated so it cannot be overclaimed: this is OBSERVATIONAL +
//! TESTIMONIAL machinery. No interventions, no counterfactuals, no
//! do-calculus — the lens NEVER asserts causation. An antecedent is a
//! candidate: aligned (calibrated), strictly prior (stream time), and leaning
//! the right way (charge). The output is ranked evidence with its working
//! shown, feeding the mint surface where causes are actually established.
//!
//! ANTECEDENTS(x): episodes strictly BEFORE x, aligned with x, weighted toward
//! those that lean TOWARDS (blue — plans, intents: the shape of a lead-up).
//! CONSEQUENTS(x): episodes strictly AFTER x, aligned with x, weighted toward
//! those that lean FROM (red — recollection, reference-back: the shape of a
//! wake). Deterministic throughout; calibration mandatory on real vectors.
//!
//! 1:1 port of PacificStore's `CausalLens.swift` (Swift `Float` → `f32`,
//! `UInt64` → `u64`, `Int` → `usize`). The Swift `enum CausalLens` namespace
//! maps to this module's items — the chromodynamics precedent.

use crate::chromodynamics::{self, Charge};
use crate::space_calibration::SpaceCalibration;
use crate::transmission::TransmissionEpisode;
use crate::vec::dot;

/// One candidate in a causal query: the evidence, itemized — never a verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct CausalTrace {
    pub id: String,
    /// Calibrated alignment to the target (0.5 = unrelated) — raw cosine when
    /// no calibration was given (synthetic geometry only).
    pub alignment: f32,
    /// Stream-time gap to the target, milliseconds. Always > 0: precedence is
    /// strict, simultaneity proves nothing.
    pub gap_ms: u64,
    /// The episode's chromodynamic charge — the lean that earned its weight.
    pub charge: Charge,
    /// alignment · directional lean · recency of the gap. The ranking value,
    /// derived from the three shown ingredients and nothing else.
    pub score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// Alignment floor: below this the episode is unrelated, whatever its
    /// timing (on the calibrated scale 0.5 = null).
    pub min_alignment: f32,
    /// Gap half-life: evidence decays with distance in stream time.
    pub gap_half_life_ms: u64,
    pub k: usize,
}

impl Params {
    /// The Swift member defaults, verbatim (`public init() {}`).
    pub fn new() -> Self {
        Params {
            min_alignment: 0.6,
            gap_half_life_ms: 14 * 24 * 3_600_000,
            k: 10,
        }
    }
}

impl Default for Params {
    fn default() -> Self {
        Params::new()
    }
}

/// What led here: strictly-prior, aligned episodes, weighted toward the
/// TOWARDS lean — a lead-up talks about what it is heading into.
///
/// `target` is the Swift labelled tuple `(vector, ts)`; Swift's defaulted
/// arguments (`calibration: nil`, `params: Params()`) become explicit
/// `None` / `Params::default()` at the call site.
pub fn antecedents(
    target: (&[f32], u64),
    episodes: &[TransmissionEpisode],
    calibration: Option<&SpaceCalibration>,
    params: Params,
) -> Vec<CausalTrace> {
    let (_, target_ts) = target;
    trace(
        target,
        episodes.iter().filter(|e| e.ts < target_ts).collect(),
        |ts| target_ts - ts,
        |c| c.blue,
        calibration,
        params,
    )
}

/// What followed from here: strictly-later, aligned episodes, weighted
/// toward the FROM lean — a wake talks about where it came from.
pub fn consequents(
    target: (&[f32], u64),
    episodes: &[TransmissionEpisode],
    calibration: Option<&SpaceCalibration>,
    params: Params,
) -> Vec<CausalTrace> {
    let (_, target_ts) = target;
    trace(
        target,
        episodes.iter().filter(|e| e.ts > target_ts).collect(),
        |ts| ts - target_ts,
        |c| c.red,
        calibration,
        params,
    )
}

fn trace(
    target: (&[f32], u64),
    episodes: Vec<&TransmissionEpisode>,
    gap: impl Fn(u64) -> u64,
    lean: impl Fn(Charge) -> f32,
    calibration: Option<&SpaceCalibration>,
    params: Params,
) -> Vec<CausalTrace> {
    let (target_vector, _) = target;
    let t = match calibration {
        Some(cal) => cal.center(target_vector),
        None => normalize(target_vector),
    };

    let mut out: Vec<CausalTrace> = Vec::new();
    for e in episodes {
        if e.vector.is_empty() {
            continue;
        }
        let v = match calibration {
            Some(cal) => cal.center(&e.vector),
            None => normalize(&e.vector),
        };
        // Swift: `Float(SpectralClustering.dot(t, v))` — an f32 accumulation
        // widened to Double and immediately narrowed back; the round-trip is
        // the identity, so the f32 dot IS the value.
        let cos = dot(&t, &v);
        let alignment = match calibration {
            Some(cal) => cal.calibrated01(cos),
            None => cos,
        };
        // Swift `guard alignment >= minAlignment else { continue }` — the
        // negated form keeps the same NaN behaviour (NaN fails the guard).
        if !(alignment >= params.min_alignment) {
            continue;
        }

        let g = gap(e.ts);
        let recency = 0.5f32.powf(g as f32 / params.gap_half_life_ms as f32);
        let charge = chromodynamics::charge(&e.text);
        // The lean opens the gate wider, it never closes it: an aligned,
        // prior episode with no directional markers is still evidence.
        let score = alignment * (0.5 + 0.5 * lean(charge)) * recency;

        out.push(CausalTrace {
            id: e.id.clone(),
            alignment,
            gap_ms: g,
            charge,
            score,
        });
    }
    // Descending score, ascending id on ties — the Swift comparator verbatim
    // (equal evidence resolves by stable identity, never by input order).
    out.sort_by(|a, b| {
        if a.score != b.score {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.id.cmp(&b.id)
        }
    });
    out.truncate(params.k);
    out
}

// Swift calls `SpectralClustering.normalize`, whose reciprocal-multiply form
// (`x * (1/√mag)`) rounds differently from `vec::normalize`'s divide for ~80%
// of vectors. spectral.rs keeps that form private, so this module carries its
// own verbatim copy — the FFI swap pins Swift-bit-identical output.
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

// MARK: - Tests
//
// The causal query operator, proven — precedence is strict, alignment gates,
// the chromodynamic lean widens but never closes, and nothing here ever
// asserts a cause: the lens returns ranked evidence, deterministically.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectral::SplitMix64;

    const DAY: u64 = 24 * 3_600_000;

    fn ep(id: &str, v: &[f32], ts: u64, text: &str) -> TransmissionEpisode {
        TransmissionEpisode::new(id.to_string(), normalize(v), text.to_string(), ts)
    }

    fn ids(traces: &[CausalTrace]) -> Vec<String> {
        traces.iter().map(|t| t.id.clone()).collect()
    }

    const X: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const Y: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

    #[test]
    fn precedence_is_strict() {
        let target = (&X[..], 100 * DAY);
        let episodes = vec![
            ep("before", &X, 99 * DAY, "walked the pier"),
            ep("same", &X, 100 * DAY, "walked the pier"),
            ep("after", &X, 101 * DAY, "walked the pier"),
        ];
        let ante = antecedents(target, &episodes, None, Params::default());
        let cons = consequents(target, &episodes, None, Params::default());
        assert_eq!(ids(&ante), vec!["before"]);
        assert_eq!(ids(&cons), vec!["after"]); // simultaneity proves nothing, both ways
    }

    #[test]
    fn alignment_gates_the_unrelated() {
        let target = (&X[..], 100 * DAY);
        let episodes = vec![
            ep("related", &X, 99 * DAY, "walked the pier"),
            ep("unrelated", &Y, 99 * DAY, "walked the pier"),
        ];
        let ante = antecedents(target, &episodes, None, Params::default());
        assert!(ante.iter().any(|t| t.id == "related"));
        assert!(!ante.iter().any(|t| t.id == "unrelated"));
    }

    #[test]
    fn towards_lean_ranks_the_lead_up() {
        let target = (&X[..], 100 * DAY);
        let episodes = vec![
            ep("plan", &X, 99 * DAY, "let's plan the launch, we will book the venue soon"),
            ep("flat", &X, 99 * DAY, "the pier was quiet"),
        ];
        let ante = antecedents(target, &episodes, None, Params::default());
        assert_eq!(ante.first().map(|t| t.id.as_str()), Some("plan"));
        // The lean widened the gate — it did not close it on the flat episode.
        assert_eq!(ante.len(), 2);
    }

    #[test]
    fn from_lean_ranks_the_wake() {
        let target = (&X[..], 100 * DAY);
        let episodes = vec![
            ep("recollection", &X, 101 * DAY, "remember how it began back before the launch"),
            ep("flat", &X, 101 * DAY, "the pier is quiet"),
        ];
        let cons = consequents(target, &episodes, None, Params::default());
        assert_eq!(cons.first().map(|t| t.id.as_str()), Some("recollection"));
    }

    #[test]
    fn nearer_evidence_outranks_distant_equal_evidence() {
        let target = (&X[..], 100 * DAY);
        let episodes = vec![
            ep("near", &X, 99 * DAY, "walked the pier"),
            ep("far", &X, 40 * DAY, "walked the pier"),
        ];
        let ante = antecedents(target, &episodes, None, Params::default());
        assert_eq!(ante.first().map(|t| t.id.as_str()), Some("near"));
    }

    #[test]
    fn deterministic_and_honest_when_empty() {
        let target = (&X[..], 100 * DAY);
        assert!(antecedents(target, &[], None, Params::default()).is_empty());
        let episodes = vec![
            ep("a", &X, 99 * DAY, "walked the pier"),
            ep("b", &X, 99 * DAY, "walked the pier"),
        ];
        let one = antecedents(target, &episodes, None, Params::default());
        let two = antecedents(target, &episodes, None, Params::default());
        assert_eq!(ids(&one), ids(&two));
        assert_eq!(ids(&one), vec!["a", "b"]); // equal evidence: id tie-break
    }

    #[test]
    fn calibrated_cone_still_finds_the_true_antecedent() {
        // Cone geometry: raw alignment cannot separate related from unrelated;
        // calibrated, the lens gates correctly.
        let mut rng = SplitMix64::new(77);
        let mut cone = |g: usize| -> Vec<f32> {
            let mut v = vec![0.0f32; 8];
            v[0] = 1.0;
            v[1 + g] = 0.3;
            for d in 0..8 {
                v[d] += ((rng.next() % 1000) as f32 / 1000.0 - 0.5) * 0.04;
            }
            normalize(&v)
        };
        let mut corpus: Vec<Vec<f32>> = (0..10).map(|_| cone(0)).collect();
        corpus.extend((0..10).map(|_| cone(1)));
        let cal = SpaceCalibration::fit(&corpus).unwrap();
        let target = (&corpus[0][..], 100 * DAY);
        let episodes = vec![
            ep("kin", &corpus[1], 99 * DAY, "walked the pier"),
            ep("stranger", &corpus[15], 99 * DAY, "walked the pier"),
        ];
        let ante = antecedents(target, &episodes, Some(&cal), Params::default());
        assert!(ante.iter().any(|t| t.id == "kin"));
        assert!(!ante.iter().any(|t| t.id == "stranger"));
    }
}
