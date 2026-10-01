//! CandidateRegister — REGISTRATION (8 Aug directive): identify entities which
//! could PLAUSIBLY be registered as nodes and edges — "this is an intrinsically
//! quantum operation."
//!
//! Quantum, structurally (an analogy made load-bearing, and no more than that —
//! no Hilbert spaces, no complex amplitudes, no overclaim):
//!
//! ```text
//! SUPERPOSITION   a candidate before minting is not one thing. It is a
//!                 distribution over KINDS (the affinity vector, never
//!                 argmaxed), over IDENTITIES (new vs same-as-existing, with
//!                 amplitudes), and over EXTENTS (the same surviving grain at
//!                 different pyramid scales is the same entity with different
//!                 boundaries — all of them carried, linked by lineage).
//! MEASUREMENT     minting IS the collapse. The register never collapses
//!                 anything itself; `collapsing(_, to_existing)` models what
//!                 an attestation does to the state, so the UI can show the
//!                 consequences of a measurement before a human makes it.
//! ENTANGLEMENT    candidates are not independent: every edge candidate
//!                 references node candidates, so collapsing one identity
//!                 re-targets the edges that touch it and dissolves its
//!                 same-as alternatives. Registration order matters.
//! COMPLEMENTARITY the pyramid's scale ↔ specificity trade-off: sharpen the
//!                 instance (fine kernel) and the kind spreads; sharpen the
//!                 family (coarse kernel) and the instance blurs. The
//!                 register holds readings at every persistent scale rather
//!                 than pretending one is true.
//! ```
//!
//! Everything upstream feeds it: the pyramid supplies grains + persistence
//! (only standing waves register — noise never does), the anchors supply the
//! kind amplitudes (calibrated), the resolution calculus's evidence supplies
//! identity amplitudes against the existing graph, chromodynamic charge +
//! strict stream-time precedence supply led_to edge candidates (the causal
//! lens's rule, applied grain-to-grain). Output is EVIDENCE for the mint
//! surface. Deterministic throughout.
//!
//! 1:1 port of PacificStore's `CandidateRegister.swift` (Swift `Float` → `f32`,
//! `UInt64` → `u64`, `Int` → `usize`). The Swift `enum CandidateRegister`
//! namespace maps to this module's items.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::chromodynamics::{self, Charge};
use crate::coarsening::{self, PyramidParams};
use crate::ontology_anchors::KindAnchor;
use crate::space_calibration::SpaceCalibration;
use crate::transmission::TransmissionEpisode;
use crate::vec::{dot, normalize};

// MARK: - Inputs

/// Existing-graph row, as this operation consumes it — a plain-data mirror of
/// the store's `GraphNodeRow` carrying only the fields the identity reading
/// touches (the Swift row also carries kind/label/scope/provenance/time/
/// coordinate; none of them feed the register).
#[derive(Debug, Clone)]
pub struct GraphNodeRow {
    pub id: String,
    pub vector: Option<Vec<f32>>,
}

// MARK: - The superposed states

/// Identity as a distribution, never a verdict: amplitude that this candidate
/// is NEW, and amplitudes that it is each existing node. Amplitudes are
/// clamped [0,1] and independent — this is evidence weight, not probability
/// theatre; they deliberately do not sum to 1.
#[derive(Debug, Clone)]
pub struct IdentitySuperposition {
    pub new_amplitude: f32,
    /// `(id, amplitude)` — Swift's labeled tuple, kept as a plain pair.
    pub same_as: Vec<(String, f32)>,
}

#[derive(Debug, Clone)]
pub struct CandidateNode {
    /// "<grain>@L<level>" — one EXTENT of an entity
    pub id: String,
    /// the surviving grain id — lineage key across scales
    pub grain: String,
    pub level: usize,
    /// episode ids, sorted
    pub members: Vec<String>,
    pub mass: f32,
    /// levels survived — why it registered at all
    pub persistence: usize,
    /// Kind amplitudes over the anchors, normalized over the above-null part —
    /// the superposition the mint surface renders. Empty = kindless, honestly.
    /// (Swift `[String: Float]`; a `BTreeMap` so every walk over it — the
    /// normalizing sum included — runs in sorted-key order, which Swift's
    /// unordered `Dictionary` cannot promise.)
    pub kinds: BTreeMap<String, f32>,
    pub identity: IdentitySuperposition,
    pub charge: Charge,
    /// `(start, end)` in stream-time milliseconds.
    pub span_ms: (u64, u64),
    pub centroid: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct CandidateEdge {
    /// "led_to" | "same_as"
    pub verb: String,
    /// candidate id, or existing node id after collapse
    pub from: String,
    pub to: String,
    pub amplitude: f32,
    /// itemized — the lens shows its working
    pub evidence: String,
}

/// The register: an uncollapsed state over possible nodes and edges.
#[derive(Debug, Clone)]
pub struct Register {
    pub nodes: Vec<CandidateNode>,
    pub edges: Vec<CandidateEdge>,
}

impl Register {
    /// MEASUREMENT, modeled: collapse one candidate's identity onto an existing
    /// node. The candidate leaves the register, its same-as alternatives
    /// dissolve, and every edge touching it re-targets — the entangled update.
    pub fn collapsing(&self, candidate_id: &str, existing_id: &str) -> Register {
        let nodes: Vec<CandidateNode> = self
            .nodes
            .iter()
            .filter(|n| n.id != candidate_id)
            .cloned()
            .collect();
        let edges: Vec<CandidateEdge> = self
            .edges
            .iter()
            .filter_map(|e| {
                if e.verb == "same_as" && (e.from == candidate_id || e.to == candidate_id) {
                    return None;
                }
                let from = if e.from == candidate_id { existing_id.to_string() } else { e.from.clone() };
                let to = if e.to == candidate_id { existing_id.to_string() } else { e.to.clone() };
                Some(CandidateEdge {
                    verb: e.verb.clone(),
                    from,
                    to,
                    amplitude: e.amplitude,
                    evidence: e.evidence.clone(),
                })
            })
            .collect();
        Register { nodes, edges }
    }
}

// MARK: - The operation

#[derive(Debug, Clone)]
pub struct Params {
    /// A grain must survive this many levels to register — the vortex bar.
    pub min_persistence: usize,
    pub min_mass: f32,
    /// Kind amplitudes keep only the above-null part (calibrated 0.5 = unrelated).
    pub kind_floor: f32,
    /// Identity: calibrated alignment to an existing node above this
    /// contributes a same-as amplitude.
    pub identity_floor: f32,
    /// led_to: alignment floor + gap half-life (the causal lens's rule).
    pub causal_floor: f32,
    pub gap_half_life_ms: u64,
    pub pyramid: PyramidParams,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            min_persistence: 2,
            min_mass: 2.0,
            kind_floor: 0.5,
            identity_floor: 0.75,
            causal_floor: 0.6,
            gap_half_life_ms: 14 * 24 * 3_600_000,
            pyramid: PyramidParams::default(),
        }
    }
}

/// Identify the register from the episodes' matter. (Swift defaults —
/// `existing: []`, `calibration: nil`, `params: Params()` — are expressed by
/// passing `&[]`, `None`, `&Params::default()`.)
pub fn identify(
    episodes: &[TransmissionEpisode],
    anchors: &[KindAnchor],
    existing: &[GraphNodeRow],
    calibration: Option<&SpaceCalibration>,
    params: &Params,
) -> Register {
    if episodes.is_empty() {
        return Register { nodes: Vec::new(), edges: Vec::new() };
    }
    // Swift `Dictionary(uniqueKeysWithValues:)` traps on a duplicate id — keep
    // that loudness rather than silently keeping one of the pair.
    let mut by_id: HashMap<&str, &TransmissionEpisode> = HashMap::new();
    for e in episodes {
        assert!(
            by_id.insert(e.id.as_str(), e).is_none(),
            "duplicate episode id: {}",
            e.id
        );
    }

    let center = |v: &[f32]| -> Vec<f32> {
        match calibration {
            Some(cal) => cal.center(v),
            None => normalize(v),
        }
    };
    let report = |cos: f32| -> f32 {
        match calibration {
            Some(cal) => cal.calibrated01(cos),
            None => cos,
        }
    };
    let framed_anchors: Vec<(String, Vec<f32>)> = anchors
        .iter()
        .map(|a| (a.kind.name.clone(), center(&a.vector)))
        .collect();
    let framed_existing: Vec<(String, Vec<f32>)> = existing
        .iter()
        .filter_map(|n| n.vector.as_ref().map(|v| (n.id.clone(), center(v))))
        .collect();

    // The matter, convolved: grains + persistence from the pyramid.
    let ids: Vec<String> = episodes.iter().map(|e| e.id.clone()).collect();
    let vectors: Vec<Vec<f32>> = episodes.iter().map(|e| e.vector.clone()).collect();
    let pyramid = coarsening::build(&ids, &vectors, &params.pyramid, calibration);

    // Every distinct EXTENT of a persistent grain registers once.
    let mut seen_extents: HashSet<String> = HashSet::new();
    let mut nodes: Vec<CandidateNode> = Vec::new();
    for (level, layer) in pyramid.levels.iter().enumerate() {
        // (Swift `Int(params.minMass)` truncates toward zero; `as usize` does
        // the same on any value this dial is honestly set to.)
        for g in layer.grains.iter().filter(|g| g.members.len() >= params.min_mass as usize) {
            let extent = format!("{}|{}", g.id, g.members.join(","));
            if seen_extents.contains(&extent)
                || pyramid.persistence.get(&g.id).copied().unwrap_or(0) < params.min_persistence
            {
                continue;
            }
            seen_extents.insert(extent);

            // KIND superposition — the above-null part, normalized, never argmaxed.
            let mut kinds: BTreeMap<String, f32> = BTreeMap::new();
            for (name, anchor) in &framed_anchors {
                let a = report(dot(&g.centroid, anchor));
                if a > params.kind_floor {
                    kinds.insert(name.clone(), a - params.kind_floor);
                }
            }
            let total: f32 = kinds.values().fold(0.0, |acc, v| acc + v);
            if total > 0.0 {
                for v in kinds.values_mut() {
                    *v /= total;
                }
            }

            // IDENTITY superposition against the existing graph.
            let mut same_as: Vec<(String, f32)> = Vec::new();
            for (id, vec) in &framed_existing {
                let a = report(dot(&g.centroid, vec));
                if a >= params.identity_floor {
                    same_as.push((id.clone(), ((a - 0.5) * 2.0).min(1.0)));
                }
            }
            same_as.sort_by(|x, y| {
                if x.1 != y.1 {
                    y.1.partial_cmp(&x.1).unwrap_or(Ordering::Equal)
                } else {
                    x.0.cmp(&y.0)
                }
            });
            let new_amp = (1.0 - same_as.first().map_or(0.0, |p| p.1)).max(0.0);

            // Charge + span, composed over member episodes.
            let members: Vec<&TransmissionEpisode> = g
                .members
                .iter()
                .filter_map(|m| by_id.get(m.as_str()).copied())
                .collect();
            let charges: Vec<Charge> = members
                .iter()
                .map(|e| chromodynamics::charge(&e.text))
                .collect();
            let charge = chromodynamics::compose(&charges);
            let span = (
                members.iter().map(|e| e.ts).min().unwrap_or(0),
                members.iter().map(|e| e.ts).max().unwrap_or(0),
            );

            nodes.push(CandidateNode {
                id: format!("{}@L{}", g.id, level),
                grain: g.id.clone(),
                level,
                members: g.members.clone(),
                mass: g.mass,
                persistence: pyramid.persistence.get(&g.id).copied().unwrap_or(0),
                kinds,
                identity: IdentitySuperposition { new_amplitude: new_amp, same_as },
                charge,
                span_ms: span,
                centroid: g.centroid.clone(),
            });
        }
    }
    nodes.sort_by(|a, b| a.id.cmp(&b.id));

    // EDGES. same_as: the identity superposition, in edge form (entangled
    // with its node). led_to: strict span precedence ∧ alignment ∧ the
    // towards-lean of the lead-up — grain-to-grain, finest extents only.
    let mut edges: Vec<CandidateEdge> = Vec::new();
    for n in &nodes {
        for (id, amp) in &n.identity.same_as {
            edges.push(CandidateEdge {
                verb: "same_as".to_string(),
                from: n.id.clone(),
                to: id.clone(),
                amplitude: *amp,
                evidence: format!("aligned {amp:.2} with existing"),
            });
        }
    }
    // (Swift groups via unordered Dictionary then sorts by id; a BTreeMap walk
    // plus the same sort lands identically. `min(by:)` keeps the FIRST minimal
    // element — levels are distinct within a grain, but mirror it anyway.)
    let mut by_grain: BTreeMap<&str, Vec<&CandidateNode>> = BTreeMap::new();
    for n in &nodes {
        by_grain.entry(n.grain.as_str()).or_default().push(n);
    }
    let mut finest: Vec<&CandidateNode> = by_grain
        .values()
        .filter_map(|group| {
            let mut best: Option<&CandidateNode> = None;
            for n in group {
                if best.is_none_or(|b| n.level < b.level) {
                    best = Some(n);
                }
            }
            best
        })
        .collect();
    finest.sort_by(|a, b| a.id.cmp(&b.id));
    for a in &finest {
        for b in &finest {
            if a.grain == b.grain {
                continue;
            }
            if !(a.span_ms.1 < b.span_ms.0) {
                continue; // strict precedence
            }
            let align = report(dot(&a.centroid, &b.centroid));
            if !(align >= params.causal_floor) {
                continue;
            }
            let gap = b.span_ms.0 - a.span_ms.1;
            let recency = 0.5f32.powf(gap as f32 / params.gap_half_life_ms as f32);
            let amp = align * (0.5 + 0.5 * a.charge.blue) * recency;
            edges.push(CandidateEdge {
                verb: "led_to".to_string(),
                from: a.id.clone(),
                to: b.id.clone(),
                amplitude: amp,
                evidence: format!(
                    "precedes by {gap}ms · aligned {align:.2} · towards-lean {:.2}",
                    a.charge.blue
                ),
            });
        }
    }
    edges.sort_by(|x, y| {
        if x.amplitude != y.amplitude {
            y.amplitude.partial_cmp(&x.amplitude).unwrap_or(Ordering::Equal)
        } else {
            (x.from.as_str(), x.to.as_str()).cmp(&(y.from.as_str(), y.to.as_str()))
        }
    });

    Register { nodes, edges }
}

// MARK: - Tests
//
// The quantum register, proven: superpositions are held (never argmaxed),
// noise never registers, identity amplitudes read against the existing graph,
// led_to needs strict precedence AND alignment AND lean, and collapse is a
// measurement whose consequences propagate through the entangled edges.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology_anchors::OntologyKind;

    const DAY: u64 = 24 * 3_600_000;

    fn unit(v: &[f32]) -> Vec<f32> {
        normalize(v)
    }

    /// Two standing clusters in stream time: a planning burst, then a doing
    /// burst in the same direction; plus one singleton of noise elsewhere.
    fn world() -> (Vec<TransmissionEpisode>, Vec<KindAnchor>) {
        let mut episodes: Vec<TransmissionEpisode> = Vec::new();
        for i in 0..5u64 {
            episodes.push(TransmissionEpisode {
                id: format!("plan{i}"),
                vector: unit(&[1.0, i as f32 * 0.005, 0.0, 0.0]),
                text: "let's plan the pier gathering, we will book it soon".to_string(),
                ts: 90 * DAY + i * DAY / 2,
            });
        }
        // Distinct from the planning burst (cosine ≈ 0.8 — aligned, not a
        // duplicate: outside every fine kernel, inside the causal floor).
        for i in 0..5u64 {
            episodes.push(TransmissionEpisode {
                id: format!("held{i}"),
                vector: unit(&[0.8, 0.6, i as f32 * 0.005, 0.0]),
                text: "the pier gathering".to_string(),
                ts: 100 * DAY + i * DAY / 2,
            });
        }
        episodes.push(TransmissionEpisode {
            id: "noise".to_string(),
            vector: unit(&[0.0, 0.0, 0.0, 1.0]),
            text: "stray".to_string(),
            ts: 95 * DAY,
        });
        let anchors = vec![
            KindAnchor {
                kind: OntologyKind {
                    name: "Event".to_string(),
                    definition: "a dated happening".to_string(),
                },
                vector: unit(&[1.0, 0.1, 0.0, 0.0]),
            },
            KindAnchor {
                kind: OntologyKind {
                    name: "Place".to_string(),
                    definition: "a located where".to_string(),
                },
                vector: unit(&[0.9, 0.0, 0.3, 0.0]),
            },
        ];
        (episodes, anchors)
    }

    #[test]
    fn standing_waves_register_noise_does_not() {
        let (episodes, anchors) = world();
        let r = identify(&episodes, &anchors, &[], None, &Params::default());
        assert!(!r.nodes.is_empty());
        // singletons never register
        assert!(!r.nodes.iter().any(|n| n.members == ["noise"]));
    }

    #[test]
    fn kind_superposition_is_held_not_collapsed() {
        let (episodes, anchors) = world();
        let r = identify(&episodes, &anchors, &[], None, &Params::default());
        // The clusters sit between the two anchors: both kinds must survive in
        // the distribution — the register never argmaxes.
        let ambiguous: Vec<&CandidateNode> =
            r.nodes.iter().filter(|n| n.kinds.len() >= 2).collect();
        assert!(!ambiguous.is_empty());
        for n in r.nodes.iter().filter(|n| !n.kinds.is_empty()) {
            let total: f32 = n.kinds.values().sum();
            // normalized distribution, all of it kept
            assert!((total - 1.0).abs() < 0.001);
        }
    }

    #[test]
    fn identity_amplitudes_read_against_the_existing_graph() {
        let (episodes, anchors) = world();
        let near = GraphNodeRow {
            id: "existing-pier".to_string(),
            vector: Some(unit(&[1.0, 0.02, 0.02, 0.0])),
        };
        let r = identify(&episodes, &anchors, &[near], None, &Params::default());
        let with_same: Vec<&CandidateNode> = r
            .nodes
            .iter()
            .filter(|n| !n.identity.same_as.is_empty())
            .collect();
        assert!(!with_same.is_empty());
        for n in &with_same {
            assert_eq!(
                n.identity.same_as.first().map(|p| p.0.as_str()),
                Some("existing-pier")
            );
            // "new" lost amplitude to "same"
            assert!(n.identity.new_amplitude < 1.0);
        }
        assert!(r
            .edges
            .iter()
            .any(|e| e.verb == "same_as" && e.to == "existing-pier"));
    }

    #[test]
    fn led_to_needs_precedence_alignment_and_lean() {
        let (episodes, anchors) = world();
        let r = identify(&episodes, &anchors, &[], None, &Params::default());
        let led: Vec<&CandidateEdge> =
            r.edges.iter().filter(|e| e.verb == "led_to").collect();
        // The planning burst (earlier, towards-leaning, aligned) leads to the
        // held burst — and nothing leads to or from the unregistered noise.
        assert!(led
            .iter()
            .any(|e| e.from.starts_with("plan") && e.to.starts_with("held")));
        assert!(!led
            .iter()
            .any(|e| e.from.contains("noise") || e.to.contains("noise")));
        // Never the reverse: precedence is strict.
        assert!(!led
            .iter()
            .any(|e| e.from.starts_with("held") && e.to.starts_with("plan")));
    }

    #[test]
    fn collapse_is_a_measurement_and_edges_are_entangled() {
        let (episodes, anchors) = world();
        let near = GraphNodeRow {
            id: "existing-pier".to_string(),
            vector: Some(unit(&[1.0, 0.02, 0.05, 0.0])),
        };
        let r = identify(&episodes, &anchors, &[near], None, &Params::default());
        let led = r
            .edges
            .iter()
            .find(|e| e.verb == "led_to")
            .expect("no led_to edge to collapse through");
        let target = led.to.clone();
        let collapsed = r.collapsing(&target, "existing-pier");
        // The candidate left the register; its same_as alternatives dissolved;
        // the led_to edge re-targeted to the existing node.
        assert!(!collapsed.nodes.iter().any(|n| n.id == target));
        assert!(!collapsed
            .edges
            .iter()
            .any(|e| e.verb == "same_as" && e.from == target));
        assert!(collapsed
            .edges
            .iter()
            .any(|e| e.verb == "led_to" && e.to == "existing-pier"));
    }

    #[test]
    fn extents_superpose_across_scales_with_shared_lineage() {
        let (episodes, anchors) = world();
        let r = identify(&episodes, &anchors, &[], None, &Params::default());
        // The same surviving grain appears at more than one scale (its extent
        // grows as the kernel coarsens) — all extents held, linked by grain id.
        let mut by_grain: BTreeMap<&str, usize> = BTreeMap::new();
        for n in &r.nodes {
            *by_grain.entry(n.grain.as_str()).or_insert(0) += 1;
        }
        assert!(by_grain.values().any(|&count| count > 1));
    }

    #[test]
    fn deterministic_and_honest_when_empty() {
        let (episodes, anchors) = world();
        assert!(identify(&[], &anchors, &[], None, &Params::default())
            .nodes
            .is_empty());
        let a = identify(&episodes, &anchors, &[], None, &Params::default());
        let b = identify(&episodes, &anchors, &[], None, &Params::default());
        assert_eq!(
            a.nodes.iter().map(|n| &n.id).collect::<Vec<_>>(),
            b.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
        assert_eq!(
            a.edges.iter().map(|e| &e.evidence).collect::<Vec<_>>(),
            b.edges.iter().map(|e| &e.evidence).collect::<Vec<_>>()
        );
    }
}
