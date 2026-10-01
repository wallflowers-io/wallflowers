//! attitude — THE VECTORISED ORIENTATION (8 Aug): the process by which the
//! incident stream — episodes, their charges, the fixed points the user has
//! minted — is transformed into a stable orientation. Attitude FIRST: without it
//! the free stream impacts quasi-randomly and everything downstream diverges;
//! flux control (FEED ranking, retrieval bias, the notification trim) comes
//! after, and reads this.
//!
//! THE PROCESS, five stages, all deterministic:
//! ```text
//!   WINDOW     two spans over the stream, cut at the STREAM'S OWN CLOCK (the
//!              newest episode's ts — never wall time; the vehicle's clock is its
//!              stream, and the same episodes must yield the same orientation on
//!              any device at any hour).
//!   WEIGHT     exponential decay by age within each window (half-life params).
//!   INTEGRATE  weighted mean of unit vectors → a heading per window. The slow
//!              window is the gyro's long memory (disposition); the fast window
//!              is the instantaneous reading (attention).
//!   FILTER     complementary blend — slerp(slow, fast, α) — the classic attitude
//!              filter: the stable low-frequency reference rejects quasi-random
//!              impulses, the high-frequency reading tracks a real manoeuvre.
//!   REFERENCE  lock the free-floating heading to fixed points: cosine against
//!              GUIDE STARS (minted entities — the star tracker; the graph the
//!              user built is the catalogue) and against the ontology's kind
//!              anchors (the inertial frame's axes).
//! ```
//!
//! The output carries its own honesty: COHERENCE is the resultant length of the
//! fast window (1 = a beam, 0 = buffeting — "admitting they don't know" as a
//! number), EVIDENCE counts what was integrated, RATE is the turn between this
//! fast heading and the previous fast-span's, and TRIM is the composed charge
//! (red/from · blue/towards · gold/about) — the temporal lean of the vehicle.
//!
//! Like every lens: COMPUTED, never stored. An Orientation is valid at its
//! stream-clock and recomputed from the fold whenever asked.
//!
//! 1:1 port of PacificStore's `Attitude.swift` (Swift `Float` → `f32`). The
//! Swift `enum Attitude` namespace maps to this module's items.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::chromodynamics::{self, Charge};
use crate::ontology_anchors::KindAnchor;
use crate::space_calibration::SpaceCalibration;
use crate::transmission::TransmissionEpisode;
use crate::vec::{dot, normalize};

// MARK: - Inputs

/// A fixed point to lock against — a minted entity's id + its resolution vector.
/// The star tracker's catalogue: only what the user has established by hand.
#[derive(Debug, Clone)]
pub struct GuideStar {
    pub id: String,
    pub vector: Vec<f32>,
}

impl GuideStar {
    pub fn new(id: impl Into<String>, vector: Vec<f32>) -> Self {
        GuideStar { id: id.into(), vector }
    }
}

#[derive(Debug, Clone)]
pub struct OrientationParams {
    /// Slow window — disposition. Span and half-life in stream milliseconds.
    pub slow_span_ms: u64,
    pub slow_half_life_ms: u64,
    /// Fast window — attention.
    pub fast_span_ms: u64,
    pub fast_half_life_ms: u64,
    /// Complementary-filter weight of the FAST heading (α). 0 = pure disposition,
    /// 1 = pure attention.
    pub blend: f32,
    /// How many guide-star locks the orientation reports.
    pub guide_stars: usize,
}

impl Default for OrientationParams {
    fn default() -> Self {
        OrientationParams {
            slow_span_ms: 45 * 24 * 3_600_000,
            slow_half_life_ms: 14 * 24 * 3_600_000,
            fast_span_ms: 7 * 24 * 3_600_000,
            fast_half_life_ms: 36 * 3_600_000,
            blend: 0.35,
            guide_stars: 5,
        }
    }
}

impl OrientationParams {
    pub fn new() -> Self {
        Self::default()
    }
}

// MARK: - Output

#[derive(Debug, Clone, PartialEq)]
pub struct OrientationLock {
    pub id: String,
    /// cosine of the heading to this star
    pub alignment: f32,
}

/// The vectorised orientation — the attitude state at one stream-clock instant.
#[derive(Debug, Clone)]
pub struct Orientation {
    /// Unit heading in the shared embedding space — where the vehicle points.
    pub heading: Vec<f32>,
    /// Turn between this fast heading and the previous fast-span's, in radians.
    /// 0 when the prior span holds no evidence — no claim without a reading.
    pub rate_radians: f32,
    /// Composed charge over the fast window — the momentary temporal lean.
    pub trim: Charge,
    /// Composed charge over the slow window — the standing disposition.
    pub slow_trim: Charge,
    /// Resultant length of the fast window ∈ [0,1]: 1 = a beam, 0 = buffeting.
    /// The orientation's own confession of how much attitude it actually has.
    pub coherence: f32,
    /// Guide-star locks, best first (deterministic tie-break by id).
    pub locks: Vec<OrientationLock>,
    /// Kind name → cosine of the heading to that kind's anchor.
    /// (Swift `[String: Float]`; a BTreeMap keeps iteration deterministic.)
    pub kind_alignment: BTreeMap<String, f32>,
    /// Episodes integrated across the slow window.
    pub evidence: usize,
    /// The stream clock this orientation is valid at (newest episode ts).
    pub clock: u64,
}

// MARK: - The process

/// Determine the orientation from the stream. None for an empty stream —
/// there is no attitude without input, and pretending otherwise is exactly
/// the fabricated row the house rules ban.
///
/// Pass `calibration` for REAL embedding vectors. Uncentered, an
/// anisotropic space collapses every heading toward the corpus common
/// direction, saturates coherence for any stream, and hides rate inside a
/// ~0.95-cosine band; centered, those quantities mean what they claim.
/// Locks and kind alignments are then reported CALIBRATED (0.5 = as
/// aligned as unrelated texts get in this corpus), raw cosine otherwise.
///
/// (Swift default arguments — `guideStars: []`, `anchors: []`,
/// `params: OrientationParams()`, `calibration: nil` — are expressed here by
/// passing `&[]`, `&[]`, `&OrientationParams::default()`, `None`.)
pub fn orientation(
    episodes: &[TransmissionEpisode],
    guide_stars: &[GuideStar],
    anchors: &[KindAnchor],
    params: &OrientationParams,
    calibration: Option<&SpaceCalibration>,
) -> Option<Orientation> {
    let mut stream: Vec<TransmissionEpisode> = episodes
        .iter()
        .filter(|e| !e.vector.is_empty())
        .cloned()
        .collect();
    if let Some(cal) = calibration {
        stream = stream
            .into_iter()
            .map(|e| TransmissionEpisode {
                id: e.id,
                vector: cal.center(&e.vector),
                text: e.text,
                ts: e.ts,
            })
            .collect();
    }
    let clock = stream.iter().map(|e| e.ts).max()?;

    // WINDOW + WEIGHT + INTEGRATE — one reading per window.
    let slow = integrate(&stream, clock, params.slow_span_ms, params.slow_half_life_ms);
    let fast = integrate(&stream, clock, params.fast_span_ms, params.fast_half_life_ms);
    let (fast_reading, slow_reading) = match (fast, slow) {
        (Some(f), Some(s)) => (f, s),
        _ => return None,
    };

    // FILTER — the complementary blend.
    let heading = slerp(&slow_reading.heading, &fast_reading.heading, params.blend);

    // RATE — this fast heading against the previous fast-span's, when that
    // span holds any evidence at all.
    let mut rate: f32 = 0.0;
    if clock > params.fast_span_ms {
        if let Some(prior) = integrate(
            &stream,
            clock - params.fast_span_ms,
            params.fast_span_ms,
            params.fast_half_life_ms,
        ) {
            let d = clamp(dot(&fast_reading.heading, &prior.heading));
            rate = d.acos();
        }
    }

    // REFERENCE — guide-star locks + kind alignment, read off the heading.
    // Reference vectors enter the SAME frame as the stream; with a
    // calibration the reported number is the null-calibrated alignment.
    let frame = |v: &[f32]| -> Vec<f32> {
        match calibration {
            Some(cal) => cal.center(v),
            None => normalize(v),
        }
    };
    let report = |cosine: f32| -> f32 {
        match calibration {
            Some(cal) => cal.calibrated01(cosine),
            None => cosine,
        }
    };
    let mut locks: Vec<OrientationLock> = guide_stars
        .iter()
        .map(|star| OrientationLock {
            id: star.id.clone(),
            alignment: report(clamp(dot(&heading, &frame(&star.vector)))),
        })
        .collect();
    // Swift: `.sorted { $0.alignment != $1.alignment ? $0.alignment > $1.alignment
    //                                                : $0.id < $1.id }`
    locks.sort_by(|a, b| {
        if a.alignment != b.alignment {
            if a.alignment > b.alignment { Ordering::Less } else { Ordering::Greater }
        } else {
            a.id.cmp(&b.id)
        }
    });
    locks.truncate(params.guide_stars);

    let mut kind_alignment: BTreeMap<String, f32> = BTreeMap::new();
    for anchor in anchors {
        kind_alignment.insert(
            anchor.kind.name.clone(),
            report(clamp(dot(&heading, &frame(&anchor.vector)))),
        );
    }

    Some(Orientation {
        heading,
        rate_radians: rate,
        trim: fast_reading.charge,
        slow_trim: slow_reading.charge,
        coherence: fast_reading.resultant,
        locks,
        kind_alignment,
        evidence: slow_reading.count,
        clock,
    })
}

// MARK: - one window's reading

pub(crate) struct Reading {
    pub(crate) heading: Vec<f32>,
    /// |Σ wᵢuᵢ| / Σ wᵢ — the coherence of this window
    pub(crate) resultant: f32,
    /// weight-composed chromodynamic charge
    pub(crate) charge: Charge,
    pub(crate) count: usize,
}

/// Integrate one window ending at `end`: exponential decay by age, unit
/// vectors summed, charge composed under the same weights.
pub(crate) fn integrate(
    stream: &[TransmissionEpisode],
    end: u64,
    span_ms: u64,
    half_life_ms: u64,
) -> Option<Reading> {
    let start = end.saturating_sub(span_ms);
    let rows: Vec<&TransmissionEpisode> = stream
        .iter()
        .filter(|e| e.ts > start && e.ts <= end)
        .collect();
    if rows.is_empty() {
        return None;
    }

    let dim = rows[0].vector.len();
    let mut sum = vec![0.0f32; dim];
    let mut total_w: f32 = 0.0;
    let mut charge = Charge::NEUTRAL;
    for row in &rows {
        let age = (end - row.ts) as f32;
        let w = 0.5f32.powf(age / half_life_ms as f32);
        let unit = normalize(&row.vector);
        for d in 0..dim {
            sum[d] += w * unit[d];
        }
        let c = chromodynamics::charge(&row.text);
        charge.red += w * c.red;
        charge.blue += w * c.blue;
        charge.gold += w * c.gold;
        total_w += w;
    }
    if !(total_w > 0.0) {
        return None;
    }

    let mut mag: f32 = 0.0;
    for x in &sum {
        mag += x * x;
    }
    let resultant = mag.sqrt() / total_w;

    Some(Reading {
        heading: normalize(&sum),
        resultant: resultant.min(1.0),
        charge: Charge {
            red: charge.red / total_w,
            blue: charge.blue / total_w,
            gold: charge.gold / total_w,
        },
        count: rows.len(),
    })
}

// MARK: - spherical blend

/// Slerp between unit headings. Near-parallel falls back to a normalized
/// lerp; near-antipodal (pathological for one stream) snaps to the nearer
/// endpoint rather than inventing an axis.
pub(crate) fn slerp(a: &[f32], b: &[f32], t: f32) -> Vec<f32> {
    let d = clamp(dot(a, b));
    if d > 0.9999 {
        let lerp: Vec<f32> = a
            .iter()
            .zip(b)
            .map(|(x, y)| (1.0 - t) * x + t * y)
            .collect();
        return normalize(&lerp);
    }
    if d < -0.9999 {
        return if t < 0.5 { a.to_vec() } else { b.to_vec() };
    }
    let theta = d.acos();
    let sa = ((1.0 - t) * theta).sin() / theta.sin();
    let sb = (t * theta).sin() / theta.sin();
    let mix: Vec<f32> = a.iter().zip(b).map(|(x, y)| sa * x + sb * y).collect();
    normalize(&mix)
}

pub(crate) fn clamp(x: f32) -> f32 {
    (-1.0f32).max(1.0f32.min(x))
}

// The orientation process proven on synthetic streams — pure logic, no store.
//
// What must hold: the heading integrates the stream's dominant direction; decay
// makes the recent outweigh the old while the slow window still remembers; a
// turning stream registers rate and a steady one doesn't; coherence confesses
// buffeting; trim carries the chromodynamic lean; guide-star locks rank the
// fixed point the heading actually points at; the stream's own clock (not wall
// time) drives everything; and the whole process is deterministic.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology_anchors::OntologyKind;

    const HOUR: u64 = 3_600_000;
    const DAY: u64 = 24 * HOUR;

    fn ep(id: &str, v: &[f32], ts: u64) -> TransmissionEpisode {
        ep_text(id, v, ts, "walked the pier")
    }

    fn ep_text(id: &str, v: &[f32], ts: u64, text: &str) -> TransmissionEpisode {
        TransmissionEpisode {
            id: id.to_string(),
            vector: v.to_vec(),
            text: text.to_string(),
            ts,
        }
    }

    fn orient(episodes: &[TransmissionEpisode]) -> Option<Orientation> {
        orientation(episodes, &[], &[], &OrientationParams::default(), None)
    }

    const X: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const Y: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

    #[test]
    fn heading_integrates_the_dominant_direction() {
        let episodes: Vec<_> = (0..6)
            .map(|i| ep(&format!("e{i}"), &X, 100 * DAY + i as u64 * HOUR))
            .collect();
        let o = orient(&episodes).unwrap();
        assert!(o.heading[0] > 0.99);
        assert!(o.coherence > 0.99);
        assert_eq!(o.evidence, 6);
        assert_eq!(o.clock, 100 * DAY + 5 * HOUR);
    }

    #[test]
    fn recent_outweighs_old_but_the_slow_window_remembers() {
        // Ten days pointed at x, then the last two days pointed at y.
        let mut episodes: Vec<_> = (0..10)
            .map(|i| ep(&format!("old{i}"), &X, 90 * DAY + i as u64 * DAY))
            .collect();
        episodes.extend(
            (0..8).map(|i| ep(&format!("new{i}"), &Y, 100 * DAY + i as u64 * 6 * HOUR)),
        );
        let o = orient(&episodes).unwrap();
        // The blended heading leans toward y (the manoeuvre) but keeps x in it
        // (the disposition) — the complementary filter's whole point.
        assert!(o.heading[1] > o.heading[0]);
        assert!(o.heading[0] > 0.05);
    }

    #[test]
    fn turning_stream_registers_rate_steady_stream_does_not() {
        let steady: Vec<_> = (0..20)
            .map(|i| ep(&format!("s{i}"), &X, 100 * DAY + i as u64 * 8 * HOUR))
            .collect();
        let so = orient(&steady).unwrap();
        assert!(so.rate_radians < 0.05);

        // Previous fast-span pointed at x; current fast-span points at y.
        let mut turning: Vec<_> = (0..8)
            .map(|i| ep(&format!("a{i}"), &X, 90 * DAY + i as u64 * 12 * HOUR))
            .collect();
        turning.extend(
            (0..8).map(|i| ep(&format!("b{i}"), &Y, 98 * DAY + i as u64 * 12 * HOUR)),
        );
        let to = orient(&turning).unwrap();
        assert!(to.rate_radians > 1.0); // ~π/2 turn
    }

    #[test]
    fn coherence_confesses_buffeting() {
        // A scattered stream: orthogonal directions in equal measure.
        let dirs: [[f32; 4]; 4] = [
            [1.0, 0.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0, 0.0],
        ];
        let episodes: Vec<_> = (0..12)
            .map(|i| ep(&format!("e{i}"), &dirs[i % 4], 100 * DAY + i as u64 * HOUR))
            .collect();
        let o = orient(&episodes).unwrap();
        assert!(o.coherence < 0.3);
    }

    #[test]
    fn trim_carries_the_chromodynamic_lean() {
        let episodes: Vec<_> = (0..5)
            .map(|i| {
                ep_text(
                    &format!("e{i}"),
                    &X,
                    100 * DAY + i as u64 * HOUR,
                    "let's plan the next launch, we will meet soon",
                )
            })
            .collect();
        let o = orient(&episodes).unwrap();
        assert!(o.trim.blue > o.trim.red);
    }

    #[test]
    fn guide_star_locks_rank_the_fixed_point_the_heading_points_at() {
        let episodes: Vec<_> = (0..6)
            .map(|i| ep(&format!("e{i}"), &X, 100 * DAY + i as u64 * HOUR))
            .collect();
        let stars = [
            GuideStar::new("away", Y.to_vec()),
            GuideStar::new("here", X.to_vec()),
        ];
        let o = orientation(&episodes, &stars, &[], &OrientationParams::default(), None).unwrap();
        assert_eq!(o.locks.first().map(|l| l.id.as_str()), Some("here"));
        assert!(o.locks.first().unwrap().alignment > 0.99);
    }

    #[test]
    fn kind_alignment_reads_off_the_heading() {
        let episodes: Vec<_> = (0..4)
            .map(|i| ep(&format!("e{i}"), &X, 100 * DAY + i as u64 * HOUR))
            .collect();
        let anchors = [
            KindAnchor {
                kind: OntologyKind {
                    name: "Place".to_string(),
                    definition: "a where".to_string(),
                },
                vector: X.to_vec(),
            },
            KindAnchor {
                kind: OntologyKind {
                    name: "Person".to_string(),
                    definition: "a who".to_string(),
                },
                vector: Y.to_vec(),
            },
        ];
        let o = orientation(&episodes, &[], &anchors, &OrientationParams::default(), None).unwrap();
        assert!(o.kind_alignment["Place"] > o.kind_alignment["Person"]);
    }

    #[test]
    fn stream_clock_not_wall_clock() {
        // An old stream must orient exactly as it did when it was current: the
        // windows cut at the newest EPISODE, so wall time cannot appear anywhere.
        let old: Vec<_> = (0..6)
            .map(|i| ep(&format!("e{i}"), &X, 10 * DAY + i as u64 * HOUR))
            .collect();
        let o = orient(&old).unwrap();
        assert_eq!(o.clock, 10 * DAY + 5 * HOUR);
        assert_eq!(o.evidence, 6);
        assert!(o.coherence > 0.99);
    }

    #[test]
    fn deterministic_run_to_run() {
        let mut episodes: Vec<_> = (0..9)
            .map(|i| {
                ep(
                    &format!("e{i}"),
                    &[0.6, 0.3, 0.1, 0.0],
                    100 * DAY + i as u64 * 7 * HOUR,
                )
            })
            .collect();
        episodes.extend((0..4).map(|i| ep(&format!("f{i}"), &Y, 101 * DAY + i as u64 * HOUR)));
        let one = orient(&episodes).unwrap();
        let two = orient(&episodes).unwrap();
        assert_eq!(one.heading, two.heading);
        assert_eq!(one.rate_radians, two.rate_radians);
        assert_eq!(one.locks, two.locks);
    }

    #[test]
    fn empty_stream_has_no_attitude() {
        assert!(orient(&[]).is_none());
    }
}
