//! GraphResolution — THE RESOLUTION CALCULUS (8 Aug directive): filter and
//! identify resolved and deduplicated nodes and edges in a bitemporal knowledge
//! graph — constructed from fundamental logical units, composed in four layers:
//!
//!   L0 · TEMPORAL/IDENTITY PREDICATES   alive · known · current · overlap ·
//!       compatible (kind lattice) · source_disjoint. The bitemporal atoms:
//!       event time (valid_at/invalid_at — when true in the world) is NEVER
//!       conflated with transaction time (created_at/expired_at — when this
//!       store learned/superseded it). Every question is asked AS-OF (t, τ).
//!   L1 · EVIDENCE GAUGES   lex (normalized labels, token overlap) · sem
//!       (CALIBRATED cosine — SpaceCalibration required on real vectors; a raw
//!       cosine in an anisotropic space is not evidence) · geo (located kinds)
//!       · neigh (shared typed neighbours — the graph vouching for itself).
//!   L2 · IDENTITY   candidate blocking → per-kind MATCH RULES (named
//!       conjunctions of L1 evidence — a match always knows which rule earned
//!       it) → union-find closure WITH VETO (transitive chaining is the classic
//!       ER failure: two records the same source lists separately may never
//!       fuse, so any union that would put source-disjoint rows in one class
//!       is refused) → deterministic canonicalization (minted > folded >
//!       reference, then earliest created, then id — "minted wins",
//!       generalized).
//!   L3 · EDGE RESOLUTION   rewrite endpoints to canonicals → group by
//!       (src, verb, dst, scope) → COALESCE same-assertion valid intervals
//!       that touch (bitemporal normalization) → CONFLICTS are surfaced, never
//!       averaged: one provenance closing an interval another holds open is a
//!       finding for a human, not a tie for a tiebreaker.
//!
//! DOCTRINE. The output is a VIEW: a canon map, justified identifications,
//! coalesced edges, flagged conflicts, and PROPOSALS (near-misses for the mint
//! surface). Nothing here rewrites the graph — the fold is truth, bitemporal
//! stores never destroy, and a durable merge (same_as) is a user act. Every
//! step is deterministic: fixed thresholds, sorted iteration, id tie-breaks.
//!
//! Port of PacificStore/GraphResolution.swift (1:1; Swift Float → f32,
//! Double → f64, UInt64 → u64). Where Swift iterates a Dictionary (unordered),
//! this port iterates a BTreeMap in sorted key order — the determinism the
//! Swift comments claim, made structural.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::space_calibration::SpaceCalibration;
use crate::vec::dot;

// MARK: - Rows (the abstract shape of lodedb-graph records)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BiTime {
    /// Event time: true in the world from…
    pub valid_at: u64,
    /// …until (None = still true)
    pub invalid_at: Option<u64>,
    /// Transaction time: this store learned it at…
    pub created_at: u64,
    /// …and superseded it at (None = current record)
    pub expired_at: Option<u64>,
}

impl BiTime {
    pub fn new(valid_at: u64, invalid_at: Option<u64>, created_at: u64, expired_at: Option<u64>) -> Self {
        BiTime { valid_at, invalid_at, created_at, expired_at }
    }
}

/// Where a row came from — the rank that decides canonicals and the identity
/// of a source for the disjointness veto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// A user attested it — rank 0
    Minted { author: String },
    /// Projected from a GroupObject — rank 1
    Folded { object: String },
    /// An external feed row — rank 2
    Reference { source: String, key: String },
}

impl Provenance {
    pub(crate) fn rank(&self) -> usize {
        match self {
            Provenance::Minted { .. } => 0,
            Provenance::Folded { .. } => 1,
            Provenance::Reference { .. } => 2,
        }
    }

    /// The source identity for the veto: two rows the SAME source deliberately
    /// lists apart must never fuse. Minted rows are never source-disjoint —
    /// a person may genuinely mint the same thing twice.
    pub(crate) fn disjoint_key(&self) -> Option<String> {
        match self {
            Provenance::Minted { .. } => None,
            Provenance::Folded { object } => Some(format!("folded:{object}")),
            Provenance::Reference { source, .. } => Some(format!("ref:{source}")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GraphNodeRow {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub scope: String,
    pub provenance: Provenance,
    pub time: BiTime,
    pub vector: Option<Vec<f32>>,
    /// (lat, lon)
    pub coordinate: Option<(f64, f64)>,
}

impl GraphNodeRow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        label: impl Into<String>,
        scope: impl Into<String>,
        provenance: Provenance,
        time: BiTime,
        vector: Option<Vec<f32>>,
        coordinate: Option<(f64, f64)>,
    ) -> Self {
        GraphNodeRow {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
            scope: scope.into(),
            provenance,
            time,
            vector,
            coordinate,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GraphEdgeRow {
    pub id: String,
    pub src: String,
    pub verb: String,
    pub dst: String,
    pub scope: String,
    pub provenance: Provenance,
    pub time: BiTime,
    pub episode: Option<String>,
}

impl GraphEdgeRow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        src: impl Into<String>,
        verb: impl Into<String>,
        dst: impl Into<String>,
        scope: impl Into<String>,
        provenance: Provenance,
        time: BiTime,
        episode: Option<String>,
    ) -> Self {
        GraphEdgeRow {
            id: id.into(),
            src: src.into(),
            verb: verb.into(),
            dst: dst.into(),
            scope: scope.into(),
            provenance,
            time,
            episode,
        }
    }
}

// MARK: - L0 · temporal + identity predicates

pub enum L0 {}

impl L0 {
    /// Event-time currency: true in the world at t.
    pub fn alive(b: &BiTime, t: u64) -> bool {
        b.valid_at <= t && b.invalid_at.map(|inv| t < inv).unwrap_or(true)
    }

    /// Transaction-time currency: this store believed it at τ.
    pub fn known(b: &BiTime, tau: u64) -> bool {
        b.created_at <= tau && b.expired_at.map(|x| tau < x).unwrap_or(true)
    }

    /// THE bitemporal filter: true in the world at t, believed at τ.
    pub fn current(b: &BiTime, t: u64, tau: u64) -> bool {
        Self::alive(b, t) && Self::known(b, tau)
    }

    /// Event-time co-existence of two records.
    pub fn overlap(a: &BiTime, b: &BiTime) -> bool {
        let a_end = a.invalid_at.unwrap_or(u64::MAX);
        let b_end = b.invalid_at.unwrap_or(u64::MAX);
        a.valid_at < b_end && b.valid_at < a_end
    }

    /// Kind compatibility under the ontology's subclass lattice (Land ⊑ Place):
    /// equal, or one subsumes the other by parent-chain.
    pub fn compatible(x: &str, y: &str, lattice: &BTreeMap<String, String>) -> bool {
        if x == y {
            return true;
        }
        fn chain(k: &str, lattice: &BTreeMap<String, String>) -> BTreeSet<String> {
            let mut out: BTreeSet<String> = BTreeSet::new();
            out.insert(k.to_string());
            let mut cur = k.to_string();
            while let Some(p) = lattice.get(&cur) {
                if out.contains(p) {
                    break;
                }
                out.insert(p.clone());
                cur = p.clone();
            }
            out
        }
        chain(x, lattice).contains(y) || chain(y, lattice).contains(x)
    }

    /// The disjointness axiom: one source listing two rows separately is that
    /// source ASSERTING they differ.
    pub fn source_disjoint(a: &Provenance, b: &Provenance) -> bool {
        let (Some(ka), Some(kb)) = (a.disjoint_key(), b.disjoint_key()) else {
            return false;
        };
        if ka != kb {
            return false;
        }
        if let (Provenance::Reference { key: x, .. }, Provenance::Reference { key: y, .. }) = (a, b) {
            return x != y;
        }
        if let (Provenance::Folded { object: x }, Provenance::Folded { object: y }) = (a, b) {
            return x != y;
        }
        false
    }
}

// MARK: - L1 · evidence gauges

#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    /// Normalized labels identical
    pub lex_exact: bool,
    /// Token Jaccard ∈ [0,1]
    pub lex_overlap: f32,
    /// CALIBRATED alignment (0.5 = unrelated); None = no vectors
    pub sem: Option<f32>,
    /// Within the kind's radius; None = no coordinates
    pub geo_near: Option<bool>,
    /// Typed-neighbour Jaccard ∈ [0,1]
    pub neigh: f32,
}

pub enum L1 {}

impl L1 {
    /// Label normalization: lowercase, alphanumerics, collapsed whitespace.
    pub fn normal(s: &str) -> String {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .collect::<Vec<&str>>()
            .join(" ")
    }

    pub fn lex_overlap(a: &str, b: &str) -> f32 {
        let na = Self::normal(a);
        let nb = Self::normal(b);
        let ta: BTreeSet<&str> = na.split(' ').filter(|t| !t.is_empty()).collect();
        let tb: BTreeSet<&str> = nb.split(' ').filter(|t| !t.is_empty()).collect();
        if ta.is_empty() || tb.is_empty() {
            return 0.0;
        }
        ta.intersection(&tb).count() as f32 / ta.union(&tb).count() as f32
    }

    /// Calibrated semantic evidence. RAW COSINE IS NOT EVIDENCE in an
    /// anisotropic space — this gauge exists only through the calibration.
    pub fn sem(a: Option<&[f32]>, b: Option<&[f32]>, calibration: Option<&SpaceCalibration>) -> Option<f32> {
        let (Some(a), Some(b), Some(cal)) = (a, b, calibration) else {
            return None;
        };
        Some(cal.calibrated01(dot(&cal.center(a), &cal.center(b))))
    }

    /// Equirectangular metres — exact enough at venue radii.
    pub fn geo_near(a: Option<(f64, f64)>, b: Option<(f64, f64)>, radius_m: f64) -> Option<bool> {
        let (Some(a), Some(b)) = (a, b) else {
            return None;
        };
        let d_lat = (a.0 - b.0) * 111_320.0;
        let d_lon = (a.1 - b.1) * 111_320.0 * ((a.0 + b.0) * std::f64::consts::PI / 360.0).cos();
        Some((d_lat * d_lat + d_lon * d_lon).sqrt() <= radius_m)
    }

    /// The graph vouching for itself: Jaccard over (verb, neighbour) sets.
    pub fn neigh(x: &str, y: &str, edges: &[GraphEdgeRow]) -> f32 {
        fn around(id: &str, edges: &[GraphEdgeRow]) -> BTreeSet<String> {
            let mut out: BTreeSet<String> = BTreeSet::new();
            for e in edges {
                if e.src == id {
                    out.insert(format!("{}>{}", e.verb, e.dst));
                }
                if e.dst == id {
                    out.insert(format!("{}<{}", e.verb, e.src));
                }
            }
            out
        }
        let a = around(x, edges);
        let b = around(y, edges);
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }
        a.intersection(&b).count() as f32 / a.union(&b).count() as f32
    }
}

// MARK: - L2 · identity

/// A condition a `MatchRule` may require. (Swift: `MatchRule.Condition` —
/// hoisted to module scope, Rust enums do not nest.)
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    LexExact,
    LexOverlapAtLeast(f32),
    /// Calibrated scale: 0.5 = unrelated
    SemAtLeast(f32),
    GeoNear,
    NeighAtLeast(f32),
}

/// A named conjunction of evidence conditions — a match always knows which
/// rule earned it. Rules are per-kind; SEM alone never suffices for Person
/// (namesakes are the canonical trap).
#[derive(Clone, Debug)]
pub struct MatchRule {
    pub name: String,
    pub requires: Vec<Condition>,
}

impl MatchRule {
    pub fn new(name: impl Into<String>, requires: Vec<Condition>) -> Self {
        MatchRule { name: name.into(), requires }
    }

    fn satisfied(&self, e: &Evidence) -> bool {
        self.requires.iter().all(|c| match c {
            Condition::LexExact => e.lex_exact,
            Condition::LexOverlapAtLeast(x) => e.lex_overlap >= *x,
            Condition::SemAtLeast(x) => e.sem.unwrap_or(0.0) >= *x,
            Condition::GeoNear => e.geo_near == Some(true),
            Condition::NeighAtLeast(x) => e.neigh >= *x,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ResolutionParams {
    /// Per-kind rules; a kind with no entry never auto-identifies.
    pub rules: BTreeMap<String, Vec<MatchRule>>,
    /// Blocking gate: pairs must clear at least one of these cheaply.
    pub block_lex_overlap: f32,
    pub block_sem: f32,
    /// Near-miss band → proposals for the mint surface.
    pub propose_sem: f32,
    /// Subclass lattice (child → parent), read from the ontology.
    pub lattice: BTreeMap<String, String>,
    pub geo_radius_m: f64,
}

impl Default for ResolutionParams {
    fn default() -> Self {
        ResolutionParams {
            rules: Self::default_rules(),
            block_lex_overlap: 0.3,
            block_sem: 0.8,
            propose_sem: 0.85,
            lattice: [("Land", "Place"), ("Mediation", "Project")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            geo_radius_m: 150.0,
        }
    }
}

impl ResolutionParams {
    /// (Swift: `static let defaultRules` — a function here, std has no
    /// dependency-free lazy statics worth the ceremony.)
    pub fn default_rules() -> BTreeMap<String, Vec<MatchRule>> {
        let mut out: BTreeMap<String, Vec<MatchRule>> = BTreeMap::new();
        out.insert(
            "Place".to_string(),
            vec![
                MatchRule::new("lex+geo", vec![Condition::LexExact, Condition::GeoNear]),
                MatchRule::new("sem+geo", vec![Condition::SemAtLeast(0.9), Condition::GeoNear]),
                MatchRule::new("lex+neigh", vec![Condition::LexExact, Condition::NeighAtLeast(0.3)]),
            ],
        );
        out.insert(
            "Event".to_string(),
            vec![
                // + L0.overlap gate below
                MatchRule::new("lex+overlap", vec![Condition::LexExact]),
                MatchRule::new("sem+neigh", vec![Condition::SemAtLeast(0.9), Condition::NeighAtLeast(0.3)]),
            ],
        );
        out.insert(
            "Person".to_string(),
            vec![
                // Never SEM alone: namesakes. The graph must vouch.
                MatchRule::new("lex+neigh", vec![Condition::LexExact, Condition::NeighAtLeast(0.25)]),
                MatchRule::new(
                    "lex+sem+neigh",
                    vec![
                        Condition::LexOverlapAtLeast(0.6),
                        Condition::SemAtLeast(0.9),
                        Condition::NeighAtLeast(0.2),
                    ],
                ),
            ],
        );
        out.insert(
            "Group".to_string(),
            vec![
                MatchRule::new("lex", vec![Condition::LexExact]),
                MatchRule::new("sem+neigh", vec![Condition::SemAtLeast(0.9), Condition::NeighAtLeast(0.3)]),
            ],
        );
        out
    }
}

// MARK: - Output

#[derive(Clone, Debug)]
pub struct Identification {
    pub a: String,
    pub b: String,
    /// Which rule earned it — the justification
    pub rule: String,
    pub evidence: Evidence,
}

#[derive(Clone, Debug)]
pub struct EdgeConflict {
    pub edge_a: String,
    pub edge_b: String,
    pub reason: String,
}

/// The resolved, deduplicated VIEW — computed, never written back.
#[derive(Clone, Debug)]
pub struct ResolvedView {
    /// Every node id → its canonical id
    pub canon: BTreeMap<String, String>,
    /// Canonical nodes, current at (t, τ)
    pub nodes: Vec<GraphNodeRow>,
    /// Rewritten + coalesced, current at (t, τ)
    pub edges: Vec<GraphEdgeRow>,
    pub identifications: Vec<Identification>,
    pub conflicts: Vec<EdgeConflict>,
    /// Near-misses for the mint surface: pairs the calculus will not fuse but
    /// a human might — the same_as proposal seam.
    pub proposals: Vec<Identification>,
}

// MARK: - The operation

pub enum GraphResolution {}

impl GraphResolution {
    /// The complex operation, composed: FILTER (bitemporal, at t·τ) → BLOCK →
    /// MATCH (per-kind rules) → CLOSE (vetoed union-find) → CANONICALIZE →
    /// REWRITE + COALESCE edges → surface conflicts and proposals.
    ///
    /// (Swift defaults `params: ResolutionParams()` and `calibration: nil` —
    /// pass `&ResolutionParams::default()` and `None`.)
    pub fn resolve(
        all_nodes: &[GraphNodeRow],
        all_edges: &[GraphEdgeRow],
        t: u64,
        tau: u64,
        params: &ResolutionParams,
        calibration: Option<&SpaceCalibration>,
    ) -> ResolvedView {
        // L0 · FILTER — only rows true at t and believed at τ take part.
        let mut nodes: Vec<GraphNodeRow> = all_nodes
            .iter()
            .filter(|n| L0::current(&n.time, t, tau))
            .cloned()
            .collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut edges: Vec<GraphEdgeRow> = all_edges
            .iter()
            .filter(|e| L0::current(&e.time, t, tau))
            .cloned()
            .collect();
        edges.sort_by(|a, b| a.id.cmp(&b.id));

        // L2 · BLOCK + MATCH.
        let mut identifications: Vec<Identification> = Vec::new();
        let mut proposals: Vec<Identification> = Vec::new();
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                let a = &nodes[i];
                let b = &nodes[j];
                if a.scope != b.scope
                    || !L0::compatible(&a.kind, &b.kind, &params.lattice)
                    || L0::source_disjoint(&a.provenance, &b.provenance)
                {
                    continue;
                }
                if (a.kind == "Event" || b.kind == "Event") && !L0::overlap(&a.time, &b.time) {
                    continue;
                }

                let sem = L1::sem(a.vector.as_deref(), b.vector.as_deref(), calibration);
                let lex_ov = L1::lex_overlap(&a.label, &b.label);
                if !(lex_ov >= params.block_lex_overlap || sem.unwrap_or(0.0) >= params.block_sem) {
                    continue;
                }

                let na = L1::normal(&a.label);
                let e = Evidence {
                    lex_exact: na == L1::normal(&b.label) && !na.is_empty(),
                    lex_overlap: lex_ov,
                    sem,
                    geo_near: L1::geo_near(a.coordinate, b.coordinate, params.geo_radius_m),
                    neigh: L1::neigh(&a.id, &b.id, &edges),
                };

                let kind = if a.kind == b.kind {
                    a.kind.clone()
                } else if params.lattice.get(&a.kind) == Some(&b.kind) {
                    b.kind.clone()
                } else {
                    a.kind.clone()
                };
                let matched = params
                    .rules
                    .get(&kind)
                    .map(|rs| rs.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .find(|r| r.satisfied(&e));
                if let Some(rule) = matched {
                    identifications.push(Identification {
                        a: a.id.clone(),
                        b: b.id.clone(),
                        rule: rule.name.clone(),
                        evidence: e,
                    });
                } else if e.sem.unwrap_or(0.0) >= params.propose_sem || e.lex_exact {
                    proposals.push(Identification {
                        a: a.id.clone(),
                        b: b.id.clone(),
                        rule: "proposal".to_string(),
                        evidence: e,
                    });
                }
            }
        }

        // L2 · CLOSE — union-find with the source-disjointness veto. Unions run
        // in DESCENDING evidence order (rule priority, then semantic strength,
        // then lexical overlap): a weak early match must never poison a class
        // and get a strong later match vetoed — the bench caught exactly that.
        let rule_index: BTreeMap<String, usize> = {
            let mut out: BTreeMap<String, usize> = BTreeMap::new();
            // Swift iterates its rules Dictionary unordered here; sorted-by-kind
            // iteration pins first-seen indices deterministically.
            for rules in params.rules.values() {
                for (i, r) in rules.iter().enumerate() {
                    out.entry(r.name.clone()).or_insert(i);
                }
            }
            out
        };
        identifications.sort_by(|x, y| {
            let rx = rule_index.get(&x.rule).copied().unwrap_or(usize::MAX);
            let ry = rule_index.get(&y.rule).copied().unwrap_or(usize::MAX);
            if rx != ry {
                return rx.cmp(&ry);
            }
            let sx = x.evidence.sem.unwrap_or(0.0);
            let sy = y.evidence.sem.unwrap_or(0.0);
            if sx != sy {
                return sy.partial_cmp(&sx).unwrap_or(Ordering::Equal);
            }
            if x.evidence.lex_overlap != y.evidence.lex_overlap {
                return y
                    .evidence
                    .lex_overlap
                    .partial_cmp(&x.evidence.lex_overlap)
                    .unwrap_or(Ordering::Equal);
            }
            (x.a.as_str(), x.b.as_str()).cmp(&(y.a.as_str(), y.b.as_str()))
        });

        let mut parent: HashMap<String, String> = HashMap::new();
        fn find(parent: &mut HashMap<String, String>, x: &str) -> String {
            let mut r = x.to_string();
            while let Some(p) = parent.get(&r) {
                if *p == r {
                    break;
                }
                r = p.clone();
            }
            parent.insert(x.to_string(), r.clone());
            r
        }
        for n in &nodes {
            parent.insert(n.id.clone(), n.id.clone());
        }
        fn class_members<'a>(
            nodes: &'a [GraphNodeRow],
            parent: &mut HashMap<String, String>,
            root: &str,
        ) -> Vec<&'a GraphNodeRow> {
            nodes.iter().filter(|n| find(parent, &n.id) == root).collect()
        }
        let mut accepted: Vec<Identification> = Vec::new();
        for ident in identifications {
            let ra = find(&mut parent, &ident.a);
            let rb = find(&mut parent, &ident.b);
            if ra == rb {
                accepted.push(ident);
                continue;
            }
            let mut merged = class_members(&nodes, &mut parent, &ra);
            merged.extend(class_members(&nodes, &mut parent, &rb));
            let mut vetoed = false;
            'outer: for x in 0..merged.len() {
                for y in (x + 1)..merged.len() {
                    if L0::source_disjoint(&merged[x].provenance, &merged[y].provenance) {
                        vetoed = true;
                        break 'outer;
                    }
                }
            }
            if vetoed {
                proposals.push(ident); // the calculus refuses; a human may not
            } else {
                let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                parent.insert(hi, lo); // deterministic direction
                accepted.push(ident);
            }
        }

        // L2 · CANONICALIZE — minted > folded > reference, then earliest, then id.
        let mut canon: BTreeMap<String, String> = BTreeMap::new();
        let mut classes: BTreeMap<String, Vec<GraphNodeRow>> = BTreeMap::new();
        for n in &nodes {
            let root = find(&mut parent, &n.id);
            classes.entry(root).or_default().push(n.clone());
        }
        let mut canonical_nodes: Vec<GraphNodeRow> = Vec::new();
        for members in classes.values() {
            let winner = members
                .iter()
                .min_by(|a, b| {
                    (a.provenance.rank(), a.time.created_at, a.id.as_str())
                        .cmp(&(b.provenance.rank(), b.time.created_at, b.id.as_str()))
                })
                .expect("class is never empty")
                .clone();
            for m in members {
                canon.insert(m.id.clone(), winner.id.clone());
            }
            canonical_nodes.push(winner);
        }

        // L3 · REWRITE + COALESCE + CONFLICTS.
        let mut grouped: BTreeMap<String, Vec<GraphEdgeRow>> = BTreeMap::new();
        for e in &edges {
            // Endpoints outside the input slice pass through UNRESOLVED — an
            // edge to a node this call wasn't given is not an edge to nothing.
            let s = canon.get(&e.src).cloned().unwrap_or_else(|| e.src.clone());
            let d = canon.get(&e.dst).cloned().unwrap_or_else(|| e.dst.clone());
            grouped
                .entry(format!("{}|{}|{}|{}", s, e.verb, d, e.scope))
                .or_default()
                .push(GraphEdgeRow {
                    id: e.id.clone(),
                    src: s,
                    verb: e.verb.clone(),
                    dst: d,
                    scope: e.scope.clone(),
                    provenance: e.provenance.clone(),
                    time: e.time,
                    episode: e.episode.clone(),
                });
        }
        let mut resolved_edges: Vec<GraphEdgeRow> = Vec::new();
        let mut conflicts: Vec<EdgeConflict> = Vec::new();
        for (_, group) in grouped {
            let mut sorted = group;
            sorted.sort_by(|a, b| {
                (a.time.valid_at, a.id.as_str()).cmp(&(b.time.valid_at, b.id.as_str()))
            });
            let mut open: Option<GraphEdgeRow> = None;
            for e in sorted {
                open = match open {
                    None => Some(e),
                    Some(cur) => {
                        let cur_end = cur.time.invalid_at;
                        if cur_end.is_none() || e.time.valid_at <= cur_end.unwrap() {
                            // Same assertion, touching intervals. A closed interval
                            // overlapped by one held open is a CONFLICT, not a merge.
                            if let Some(end) = cur_end {
                                if e.time.invalid_at.is_none() && e.time.valid_at < end {
                                    conflicts.push(EdgeConflict {
                                        edge_a: cur.id.clone(),
                                        edge_b: e.id.clone(),
                                        reason: "open assertion overlaps a closed one".to_string(),
                                    });
                                }
                            }
                            let merged_end: Option<u64> = match (cur_end, e.time.invalid_at) {
                                (Some(ce), Some(ee)) => Some(ce.max(ee)),
                                _ => None,
                            };
                            let keep = if cur.provenance.rank() <= e.provenance.rank() {
                                &cur
                            } else {
                                &e
                            };
                            Some(GraphEdgeRow {
                                id: keep.id.clone(),
                                src: cur.src.clone(),
                                verb: cur.verb.clone(),
                                dst: cur.dst.clone(),
                                scope: cur.scope.clone(),
                                provenance: keep.provenance.clone(),
                                time: BiTime {
                                    valid_at: cur.time.valid_at.min(e.time.valid_at),
                                    invalid_at: merged_end,
                                    created_at: cur.time.created_at.min(e.time.created_at),
                                    expired_at: None,
                                },
                                episode: cur.episode.clone().or_else(|| e.episode.clone()),
                            })
                        } else {
                            resolved_edges.push(cur);
                            Some(e)
                        }
                    }
                };
            }
            if let Some(cur) = open {
                resolved_edges.push(cur);
            }
        }

        ResolvedView {
            canon,
            nodes: canonical_nodes,
            edges: resolved_edges,
            identifications: accepted,
            conflicts,
            proposals,
        }
    }
}

// MARK: - Tests
//
// The resolution calculus, unit by unit and composed — no store, no app.
//
// What must hold: the L0 atoms answer bitemporal questions exactly (including
// time travel — an old τ sees the superseded world); the source-disjointness
// veto refuses the classic chain-merge; namesake Persons survive semantic
// similarity; minted beats reference at canonicalization; duplicate edges
// coalesce their valid intervals; a closed-vs-open assertion is surfaced as a
// conflict, never averaged; and the whole operation is deterministic.

#[cfg(test)]
mod tests {
    use super::*;

    fn bt(v: u64, inv: Option<u64>, c: u64, x: Option<u64>) -> BiTime {
        BiTime::new(v, inv, c, x)
    }

    fn mint(author: &str) -> Provenance {
        Provenance::Minted { author: author.to_string() }
    }

    fn fold(object: &str) -> Provenance {
        Provenance::Folded { object: object.to_string() }
    }

    fn refr(source: &str, key: &str) -> Provenance {
        Provenance::Reference { source: source.to_string(), key: key.to_string() }
    }

    // MARK: L0Tests

    #[test]
    fn bitemporal_atoms_answer_exactly() {
        let b = bt(100, Some(200), 150, Some(400));
        assert!(L0::alive(&b, 100) && L0::alive(&b, 199));
        assert!(!L0::alive(&b, 99) && !L0::alive(&b, 200));
        assert!(L0::known(&b, 150) && L0::known(&b, 399));
        assert!(!L0::known(&b, 149) && !L0::known(&b, 400));
        assert!(L0::current(&b, 150, 200));
        assert!(!L0::current(&b, 250, 200)); // no longer true in the world
        assert!(!L0::current(&b, 150, 450)); // superseded record
    }

    #[test]
    fn time_travel_sees_the_superseded_world() {
        // The record was corrected at τ=400: old τ sees v1, new τ sees v2.
        let v1 = bt(100, None, 100, Some(400));
        let v2 = bt(100, Some(300), 400, None);
        assert!(L0::known(&v1, 399) && !L0::known(&v2, 399));
        assert!(!L0::known(&v1, 400) && L0::known(&v2, 400));
    }

    #[test]
    fn lattice_subsumes() {
        let lat: BTreeMap<String, String> =
            [("Land".to_string(), "Place".to_string())].into_iter().collect();
        assert!(L0::compatible("Land", "Place", &lat));
        assert!(L0::compatible("Place", "Land", &lat));
        assert!(!L0::compatible("Person", "Place", &lat));
    }

    #[test]
    fn source_disjointness_axiom() {
        let ra1 = refr("ra", "1");
        let ra2 = refr("ra", "2");
        let art = refr("artrabbit", "9");
        assert!(L0::source_disjoint(&ra1, &ra2)); // same source, listed apart
        assert!(!L0::source_disjoint(&ra1, &art)); // different sources may co-refer
        assert!(!L0::source_disjoint(&mint("a"), &mint("a")));
    }

    // MARK: ResolutionTests

    fn place(id: &str, label: &str, prov: Provenance, time: BiTime) -> GraphNodeRow {
        GraphNodeRow::new(id, "Place", label, "", prov, time, None, Some((51.53, -0.08)))
    }

    #[test]
    fn lex_geo_deduplicates_a_venue_and_minted_wins() {
        let nodes = vec![
            place("ref1", "Corsica Studios", refr("ra", "r1"), bt(0, None, 5, None)),
            place("mint1", "Corsica Studios", mint("ralph"), bt(0, None, 9, None)),
        ];
        let view = GraphResolution::resolve(&nodes, &[], 10, 10, &ResolutionParams::default(), None);
        assert_eq!(view.canon.get("ref1").map(String::as_str), Some("mint1")); // minted wins despite later created
        assert_eq!(view.nodes.len(), 1);
        assert_eq!(view.identifications.first().map(|i| i.rule.as_str()), Some("lex+geo"));
    }

    #[test]
    fn veto_refuses_the_chain_merge() {
        // Two rows the SAME source lists apart, both matching a third by name+geo:
        // the closure must refuse to fuse all three; the refused link becomes a proposal.
        let nodes = vec![
            place("ra1", "The Yard", refr("ra", "1"), bt(0, None, 1, None)),
            place("ra2", "The Yard", refr("ra", "2"), bt(0, None, 1, None)),
            place("m1", "The Yard", mint("ralph"), bt(0, None, 1, None)),
        ];
        let view = GraphResolution::resolve(&nodes, &[], 10, 10, &ResolutionParams::default(), None);
        let distinct: BTreeSet<&String> = view.canon.values().collect();
        assert_eq!(distinct.len(), 2); // never one class of three
        assert!(!view.proposals.is_empty()); // the refused union is surfaced
    }

    #[test]
    fn namesake_persons_survive_semantic_similarity() {
        // Same name, high calibrated sem, ZERO shared neighbours → no rule fires;
        // the pair lands in proposals for a human, not in the canon map.
        let corpus: Vec<Vec<f32>> = (0..10)
            .map(|i| {
                let mut v = vec![0.0f32; 8];
                v[0] = 1.0;
                v[i % 4 + 1] = 0.3;
                v
            })
            .collect();
        let cal = SpaceCalibration::fit(&corpus).unwrap();
        let mut va = vec![0.0f32; 8];
        va[0] = 1.0;
        va[1] = 0.3;
        let a = GraphNodeRow::new(
            "p1", "Person", "Sam Park", "", fold("grpA"), bt(0, None, 1, None),
            Some(va.clone()), None,
        );
        let b = GraphNodeRow::new(
            "p2", "Person", "Sam Park", "", fold("grpB"), bt(0, None, 1, None),
            Some(va), None,
        );
        let view = GraphResolution::resolve(
            &[a, b], &[], 10, 10, &ResolutionParams::default(), Some(&cal),
        );
        assert_eq!(view.canon.get("p1").map(String::as_str), Some("p1"));
        assert_eq!(view.canon.get("p2").map(String::as_str), Some("p2"));
        assert!(view.proposals.iter().any(|p| p.a == "p1" && p.b == "p2"));
    }

    #[test]
    fn shared_neighbours_resolve_the_person_the_graph_vouches_for() {
        let a = GraphNodeRow::new(
            "p1", "Person", "Sam Park", "", fold("grpA"), bt(0, None, 1, None), None, None,
        );
        let b = GraphNodeRow::new(
            "p2", "Person", "Sam Park", "", fold("grpB"), bt(0, None, 1, None), None, None,
        );
        let edges = vec![
            GraphEdgeRow::new("e1", "p1", "member_of", "g9", "", fold("grpA"), bt(0, None, 1, None), None),
            GraphEdgeRow::new("e2", "p2", "member_of", "g9", "", fold("grpB"), bt(0, None, 1, None), None),
        ];
        let view = GraphResolution::resolve(&[a, b], &edges, 10, 10, &ResolutionParams::default(), None);
        assert_eq!(view.canon.get("p2").map(String::as_str), Some("p1"));
        assert_eq!(view.identifications.first().map(|i| i.rule.as_str()), Some("lex+neigh"));
        // The two member_of edges collapse onto the canonical pair.
        assert_eq!(view.edges.len(), 1);
    }

    #[test]
    fn edge_intervals_coalesce() {
        let n = vec![place("v1", "Corsica Studios", mint("r"), bt(0, None, 1, None))];
        let edges = vec![
            GraphEdgeRow::new("e1", "v1", "hosts", "v1", "", mint("r"), bt(0, Some(100), 1, None), None),
            GraphEdgeRow::new("e2", "v1", "hosts", "v1", "", mint("r"), bt(100, Some(200), 1, None), None),
            GraphEdgeRow::new("e3", "v1", "hosts", "v1", "", mint("r"), bt(300, Some(400), 1, None), None),
        ];
        // FILTER precedes COALESCE — that is the contract: only the edge alive
        // at t takes part, so disjoint intervals never falsely fuse across t.
        let at50 = GraphResolution::resolve(&n, &edges, 50, 10, &ResolutionParams::default(), None);
        assert_eq!(at50.edges.len(), 1);
        assert_eq!(at50.edges.first().map(|e| e.id.as_str()), Some("e1"));
        let at150 = GraphResolution::resolve(&n, &edges, 150, 10, &ResolutionParams::default(), None);
        assert_eq!(at150.edges.len(), 1);
        assert_eq!(at150.edges.first().map(|e| e.id.as_str()), Some("e2"));
    }

    #[test]
    fn open_versus_closed_assertion_is_a_conflict_not_a_merge() {
        let n = vec![place("v1", "Corsica Studios", mint("r"), bt(0, None, 1, None))];
        let edges = vec![
            GraphEdgeRow::new("closed", "v1", "hosts", "v1", "", fold("o1"), bt(0, Some(100), 1, None), None),
            GraphEdgeRow::new("open", "v1", "hosts", "v1", "", fold("o2"), bt(50, None, 1, None), None),
        ];
        let view = GraphResolution::resolve(&n, &edges, 60, 10, &ResolutionParams::default(), None);
        assert_eq!(view.conflicts.len(), 1);
        assert!(view.conflicts.first().map(|c| c.reason.contains("open")) == Some(true));
    }

    #[test]
    fn deterministic_end_to_end() {
        let nodes = vec![
            place("ref1", "Corsica Studios", refr("ra", "r1"), bt(0, None, 1, None)),
            place("mint1", "Corsica Studios", mint("ralph"), bt(0, None, 1, None)),
            place("ra1", "The Yard", refr("ra", "1"), bt(0, None, 1, None)),
            place("ra2", "The Yard", refr("ra", "2"), bt(0, None, 1, None)),
        ];
        let one = GraphResolution::resolve(&nodes, &[], 10, 10, &ResolutionParams::default(), None);
        let two = GraphResolution::resolve(&nodes, &[], 10, 10, &ResolutionParams::default(), None);
        assert_eq!(one.canon, two.canon);
        assert_eq!(
            one.identifications.iter().map(|i| i.rule.clone()).collect::<Vec<_>>(),
            two.identifications.iter().map(|i| i.rule.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            one.proposals.iter().map(|p| p.a.clone()).collect::<Vec<_>>(),
            two.proposals.iter().map(|p| p.a.clone()).collect::<Vec<_>>()
        );
    }
}
