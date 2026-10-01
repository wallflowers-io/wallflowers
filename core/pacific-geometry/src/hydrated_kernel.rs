//! hydrated_kernel — THE HYDRATED COARSENING KERNEL (8 Aug directive): "the
//! critical step is the coarsening kernel, representing contextual scenarios
//! for the data. this can be hydrated via available structural metadata and
//! available GroupObject data."
//!
//! The reframe: a coarsening level is not a bandwidth, it is a SCENARIO — a
//! conjunction of conditions under which two things count as the same. "Same
//! words within the same conversation between the same people" is a different
//! claim than "same words anywhere", and the ladder of scenarios IS the
//! sequentially coarsening kernel: each level relaxes the context.
//!
//! HYDRATION: the ladder is built from what the data actually carries —
//! structural metadata (the union store's §3 keys: authors, spans, tags,
//! sources; a dataset's fields: manufacturer, price, venue) and GroupObject
//! data (shared rosters, conversations, venues — the folds). A channel whose
//! key the corpus barely has never enters the ladder: a context you cannot
//! verify is not a context, so ABSENT DATA FAILS a channel rather than
//! passing it (precision-honest; hydration's coverage gate keeps this from
//! starving the ladder).
//!
//! This is the same Grain/Pyramid/persistence machinery — scenarios simply
//! replace the geometric kernel as the fusion test, and grains carry their
//! hydration (meta unions, group unions) up the tree so coarse scenarios read
//! coarsened context, exactly as the semantic kernel reads drifted centroids.
//!
//! 1:1 port of PacificStore's `HydratedKernel.swift` (Swift `Float` → `f32`,
//! `Double` → `f64`). The Swift `enum KernelHydration` namespace maps to this
//! module's `ladder` / `ladder_with` (default-argument wrapper, as
//! `SpaceCalibration::fit` / `fit_with_components`); the Swift
//! `extension CoarseningPyramid` build path maps to `build_hydrated`.

use std::collections::{BTreeMap, BTreeSet};

use crate::coarsening::{Grain, Pyramid, PyramidLevel};
use crate::space_calibration::SpaceCalibration;
use crate::vec::dot;

// MARK: - Hydrated matter

/// (Swift `[String: Set<String>]` / `[String]` → ordered collections: Swift
/// Dictionary/Set iteration is per-instance random; every walk here must be
/// deterministic.)
#[derive(Clone, Debug, PartialEq)]
pub struct HydratedRow {
    pub id: String,
    /// semantic channel, optional
    pub vector: Option<Vec<f32>>,
    /// scalar keys are singleton sets
    pub meta: BTreeMap<String, BTreeSet<String>>,
    /// GroupObject memberships
    pub groups: Vec<String>,
}

impl HydratedRow {
    /// The Swift memberwise init defaults (`vector: nil, meta: [:],
    /// groups: []`): construct bare, then set the fields the row carries.
    pub fn new(id: impl Into<String>) -> Self {
        HydratedRow {
            id: id.into(),
            vector: None,
            meta: BTreeMap::new(),
            groups: Vec::new(),
        }
    }
}

// MARK: - Scenarios

/// One contextual condition. Absent data FAILS the channel.
#[derive(Clone, Debug, PartialEq)]
pub enum KernelChannel {
    /// Calibrated cosine distance ≤ maxDistance.
    Semantic { max_distance: f32 },
    /// The key's value sets intersect.
    Agree { key: String },
    /// Set-valued key overlaps by at least this Jaccard.
    Overlap { key: String, at_least: f32 },
    /// Numeric key within a relative tolerance: |a−b| ≤ fraction·max(|a|,|b|).
    Within { key: String, fraction: f64 },
    /// The rows share at least one GroupObject.
    SharedGroup,
}

/// A scenario: the conjunction that defines "the same, in this context".
#[derive(Clone, Debug, PartialEq)]
pub struct Scenario {
    pub name: String,
    pub all: Vec<KernelChannel>,
}

impl Scenario {
    pub fn new(name: impl Into<String>, all: Vec<KernelChannel>) -> Self {
        Scenario { name: name.into(), all }
    }
}

// MARK: - Hydration

/// `KernelHydration.ladder` with the Swift default arguments
/// (`sigmas: [0.05, 0.1, 0.2, 0.4], priceTolerance: 0.05, minJaccard: 0.3,
/// coverage: 0.8`).
pub fn ladder(rows: &[HydratedRow]) -> Vec<Scenario> {
    ladder_with(rows, &[0.05, 0.1, 0.2, 0.4], 0.05, 0.3, 0.8)
}

/// Build a ladder from what the corpus actually offers. Channels enter only
/// with ≥ `coverage` presence across rows; numeric-parsing keys become
/// `within`, multi-valued keys become `overlap`, the rest `agree`; a groups
/// channel enters when memberships exist; semantic channels enter when
/// vectors do. The ladder then relaxes: the strictest scenario first, one
/// context channel dropped per level (deterministic reverse-alphabetical),
/// widening semantics, ending at pure geometry.
pub fn ladder_with(
    rows: &[HydratedRow],
    sigmas: &[f32],
    price_tolerance: f64,
    min_jaccard: f32,
    coverage: f32,
) -> Vec<Scenario> {
    let n = rows.len() as f32;
    if rows.is_empty() {
        return Vec::new();
    }

    let mut key_count: BTreeMap<&str, usize> = BTreeMap::new();
    let mut key_multi: BTreeSet<&str> = BTreeSet::new();
    let mut key_numeric: BTreeSet<&str> = BTreeSet::new();
    for row in rows {
        for (k, v) in &row.meta {
            if v.is_empty() {
                continue;
            }
            *key_count.entry(k.as_str()).or_insert(0) += 1;
            if v.len() > 1 {
                key_multi.insert(k);
            }
            if v.iter().all(|s| parse_number(s).is_some()) {
                key_numeric.insert(k);
            }
        }
    }
    // Swift: `keyCount.filter { … }.keys.sorted()` — the BTreeMap walk is the
    // sorted key order already.
    let covered: Vec<&str> = key_count
        .iter()
        .filter(|(_, &count)| count as f32 / n >= coverage)
        .map(|(&k, _)| k)
        .collect();
    let mut context: Vec<KernelChannel> = covered
        .iter()
        .map(|&key| {
            if key_numeric.contains(key) {
                KernelChannel::Within { key: key.to_owned(), fraction: price_tolerance }
            } else if key_multi.contains(key) {
                KernelChannel::Overlap { key: key.to_owned(), at_least: min_jaccard }
            } else {
                KernelChannel::Agree { key: key.to_owned() }
            }
        })
        .collect();
    let has_groups = rows.iter().any(|r| !r.groups.is_empty());
    if has_groups {
        context.push(KernelChannel::SharedGroup);
    }
    let has_vectors = rows.iter().any(|r| r.vector.is_some());

    let mut out: Vec<Scenario> = Vec::new();
    let mut sigma_idx: usize = 0;
    fn sigma(sigmas: &[f32], idx: &mut usize) -> f32 {
        let s = sigmas[(*idx).min(sigmas.len() - 1)];
        *idx += 1;
        s
    }
    // Strictest first: full context + tight geometry.
    let mut remaining = context;
    loop {
        let mut all = remaining.clone();
        if has_vectors {
            all.insert(0, KernelChannel::Semantic { max_distance: sigma(sigmas, &mut sigma_idx) });
        }
        out.push(Scenario::new(format!("ctx{}", remaining.len()), all));
        if remaining.is_empty() {
            break;
        }
        remaining.pop(); // covered keys are sorted; drop reverse-alphabetical
    }
    // Tail: widest pure geometry, if any sigmas remain unused.
    while has_vectors && sigma_idx < sigmas.len() {
        // Swift evaluates `name: "geo\(sigmaIdx)"` before `sigma()` increments.
        let name = format!("geo{sigma_idx}");
        let max_distance = sigma(sigmas, &mut sigma_idx);
        out.push(Scenario::new(name, vec![KernelChannel::Semantic { max_distance }]));
    }
    out
}

/// Swift: `Double($0.filter { !" $€£,".contains($0) })` — strip spaces,
/// currency marks and thousands commas, then parse. (Swift's `Double(String)`
/// also admits hex-float literals, which Rust's parser does not — noted, not
/// mirrored: no metadata value plausibly carries one.)
fn parse_number(s: &str) -> Option<f64> {
    let cleaned: String = s.chars().filter(|c| !" $€£,".contains(*c)).collect();
    cleaned.parse::<f64>().ok()
}

// MARK: - Scenario coarsening (the pyramid's third build path)

#[derive(Clone)]
struct HydratedGrainState {
    meta: BTreeMap<String, BTreeSet<String>>,
    groups: BTreeSet<String>,
}

/// Recursively convolve hydrated matter through the scenario ladder. Same
/// Grain/Pyramid outputs; `PyramidLevel.sigma` carries the level index.
///
/// (Swift `CoarseningPyramid.build(hydrated:ladder:calibration:)`.)
pub fn build_hydrated(
    rows: &[HydratedRow],
    ladder: &[Scenario],
    calibration: Option<&SpaceCalibration>,
) -> Pyramid {
    if rows.is_empty() || ladder.is_empty() {
        return Pyramid { levels: Vec::new(), persistence: BTreeMap::new() };
    }
    let mut sorted: Vec<&HydratedRow> = rows.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));

    let mut matter: Vec<Grain> = sorted
        .iter()
        .map(|r| {
            let framed = r.vector.as_ref().map(|v| match calibration {
                Some(cal) => cal.center(v),
                None => sc_normalize(v),
            });
            Grain {
                id: r.id.clone(),
                centroid: framed.unwrap_or_default(),
                mass: 1.0,
                members: vec![r.id.clone()],
                born: 0,
            }
        })
        .collect();
    let mut state: BTreeMap<String, HydratedGrainState> = BTreeMap::new();
    for r in &sorted {
        let previous = state.insert(
            r.id.clone(),
            HydratedGrainState {
                meta: r.meta.clone(),
                groups: r.groups.iter().cloned().collect(),
            },
        );
        // Swift `Dictionary(uniqueKeysWithValues:)` traps on a duplicate id.
        assert!(previous.is_none(), "duplicate HydratedRow id");
    }

    fn passes(
        a: &Grain,
        b: &Grain,
        scenario: &Scenario,
        state: &BTreeMap<String, HydratedGrainState>,
    ) -> bool {
        let sa = &state[&a.id];
        let sb = &state[&b.id];
        for channel in &scenario.all {
            match channel {
                KernelChannel::Semantic { max_distance } => {
                    // Swift: `Float(1 - SpectralClustering.dot(a, b)) <= maxD`
                    // — dot accumulates Float, RETURNS Double; the subtraction
                    // happens in f64 and narrows to f32. Mirrored exactly
                    // (the double rounding is part of the contract). NaN fails
                    // the guard in both languages.
                    if a.centroid.is_empty() || b.centroid.is_empty() {
                        return false;
                    }
                    let d = (1.0f64 - f64::from(dot(&a.centroid, &b.centroid))) as f32;
                    if !(d <= *max_distance) {
                        return false;
                    }
                }
                KernelChannel::Agree { key } => {
                    let (Some(va), Some(vb)) = (sa.meta.get(key), sb.meta.get(key)) else {
                        return false;
                    };
                    if va.intersection(vb).next().is_none() {
                        return false;
                    }
                }
                KernelChannel::Overlap { key, at_least } => {
                    let (Some(va), Some(vb)) = (sa.meta.get(key), sb.meta.get(key)) else {
                        return false;
                    };
                    if va.is_empty() || vb.is_empty() {
                        return false;
                    }
                    let jaccard =
                        va.intersection(vb).count() as f32 / va.union(vb).count() as f32;
                    if !(jaccard >= *at_least) {
                        return false;
                    }
                }
                KernelChannel::Within { key, fraction } => {
                    let (Some(x), Some(y)) =
                        (min_number(sa.meta.get(key)), min_number(sb.meta.get(key)))
                    else {
                        return false;
                    };
                    // Swift: |x−y| ≤ fraction·max(|x|, |y|, .ulpOfOne);
                    // Double.ulpOfOne == f64::EPSILON.
                    if !((x - y).abs() <= fraction * x.abs().max(y.abs()).max(f64::EPSILON)) {
                        return false;
                    }
                }
                KernelChannel::SharedGroup => {
                    if sa.groups.intersection(&sb.groups).next().is_none() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Swift `num(_:)`: `s?.compactMap { Double($0.filter …) }.min()` — the
    /// least parsed number, or nil when nothing parses. (min is
    /// order-independent, so the unordered Swift Set walk and this sorted one
    /// agree; the running-minimum keeps Swift's `min()` comparison form.)
    fn min_number(s: Option<&BTreeSet<String>>) -> Option<f64> {
        let mut parsed = s?.iter().filter_map(|v| parse_number(v));
        let mut result = parsed.next()?;
        for x in parsed {
            if x < result {
                result = x;
            }
        }
        Some(result)
    }

    let mut levels: Vec<PyramidLevel> = Vec::new();
    let mut died: BTreeMap<String, usize> = BTreeMap::new();

    for (level, scenario) in ladder.iter().enumerate() {
        // Fuse: connected components under the scenario's conjunction.
        let mut parent: Vec<usize> = (0..matter.len()).collect();
        fn find(parent: &mut [usize], x: usize) -> usize {
            let mut r = x;
            while parent[r] != r {
                r = parent[r];
            }
            parent[x] = r;
            r
        }
        for i in 0..matter.len() {
            for j in (i + 1)..matter.len() {
                if passes(&matter[i], &matter[j], scenario, &state) {
                    let ri = find(&mut parent, i);
                    let rj = find(&mut parent, j);
                    if ri != rj {
                        parent[ri.max(rj)] = ri.min(rj);
                    }
                }
            }
        }
        // Swift: `groupsOf.sorted(by: { $0.key < $1.key })` — the BTreeMap
        // walk is that sorted order; members append in matter (id) order.
        let mut groups_of: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in 0..matter.len() {
            let root = find(&mut parent, i);
            groups_of.entry(root).or_default().push(i);
        }

        let mut next: Vec<Grain> = Vec::new();
        for group in groups_of.values() {
            if group.len() == 1 {
                next.push(matter[group[0]].clone());
                continue;
            }
            let survivor = group.iter().map(|&i| matter[i].id.clone()).min().unwrap();
            let mut merged = state[&survivor].clone();
            let mut centre: Vec<f32> = Vec::new();
            let mut mass: f32 = 0.0;
            let mut with_vec: f32 = 0.0;
            for &gi in group {
                let g = &matter[gi];
                if g.id != survivor {
                    died.insert(g.id.clone(), level);
                    let s = state[&g.id].clone();
                    for (k, v) in s.meta {
                        merged.meta.entry(k).or_default().extend(v);
                    }
                    merged.groups.extend(s.groups);
                }
                mass += g.mass;
                if !g.centroid.is_empty() {
                    if centre.is_empty() {
                        centre = vec![0.0; g.centroid.len()];
                    }
                    for d in 0..g.centroid.len() {
                        centre[d] += g.mass * g.centroid[d];
                    }
                    with_vec += g.mass;
                }
            }
            state.insert(survivor.clone(), merged);
            let centroid = if with_vec > 0.0 {
                sc_normalize(&centre.iter().map(|c| c / with_vec).collect::<Vec<f32>>())
            } else {
                Vec::new()
            };
            let mut members: Vec<String> = group
                .iter()
                .flat_map(|&i| matter[i].members.iter().cloned())
                .collect();
            members.sort();
            next.push(Grain { id: survivor, centroid, mass, members, born: level });
        }
        next.sort_by(|a, b| a.id.cmp(&b.id));
        matter = next;
        levels.push(PyramidLevel { sigma: level as f32, grains: matter.clone() });
        if matter.len() == 1 {
            break;
        }
    }

    let mut births: BTreeMap<String, usize> = BTreeMap::new();
    for level in &levels {
        for g in &level.grains {
            births.entry(g.id.clone()).or_insert(g.born);
        }
    }
    let mut persistence: BTreeMap<String, usize> = BTreeMap::new();
    for (id, b) in births {
        let death = died.get(&id).copied().unwrap_or(levels.len());
        persistence.insert(id, death - b);
    }
    Pyramid { levels, persistence }
}

// MARK: - small vector helpers
//
// The verbatim Swift `SpectralClustering.normalize` (multiply by the
// reciprocal), NOT `vec::normalize` (divide) — the two round differently in
// f32 and the FFI swap pins Swift-bit-identical centroids. Same private copy
// as `spectral.rs` keeps, for the same reason.
fn sc_normalize(v: &[f32]) -> Vec<f32> {
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
// Scenarios as kernels, proven: context separates what geometry collides
// (the boilerplate collision), context fuses what geometry cannot reach (the
// paraphrase), GroupObject co-incidence is a channel, absent data fails a
// channel, and hydration shapes the ladder to the corpus's actual coverage.
#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        sc_normalize(v)
    }

    fn meta_of(pairs: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
        pairs
            .iter()
            .map(|(k, vs)| {
                ((*k).to_owned(), vs.iter().map(|v| (*v).to_owned()).collect())
            })
            .collect()
    }

    #[test]
    fn context_separates_the_boilerplate_collision() {
        // Two rows with NEAR-IDENTICAL vectors (shared boilerplate) but
        // different manufacturers: the identity scenario requires agreement,
        // so they stay apart at fine levels — geometry alone would fuse them.
        let rows = vec![
            HydratedRow {
                vector: Some(unit(&[1.0, 0.01, 0.0, 0.0])),
                meta: meta_of(&[("manufacturer", &["sony"])]),
                ..HydratedRow::new("a")
            },
            HydratedRow {
                vector: Some(unit(&[1.0, 0.02, 0.0, 0.0])),
                meta: meta_of(&[("manufacturer", &["philips"])]),
                ..HydratedRow::new("b")
            },
        ];
        let ladder = vec![
            Scenario::new(
                "identity",
                vec![
                    KernelChannel::Semantic { max_distance: 0.05 },
                    KernelChannel::Agree { key: "manufacturer".into() },
                ],
            ),
            Scenario::new("geometry", vec![KernelChannel::Semantic { max_distance: 0.05 }]),
        ];
        let p = build_hydrated(&rows, &ladder, None);
        assert_eq!(p.levels[0].grains.len(), 2); // context kept them apart
        assert_eq!(p.levels[1].grains.len(), 1); // pure geometry fuses — the contrast
    }

    #[test]
    fn context_fuses_the_paraphrase_geometry_cannot_reach() {
        // Distant vectors (cos ≈ 0.7 — outside any fine kernel) but the same
        // manufacturer and a price within 3%: the contextual scenario fuses.
        let rows = vec![
            HydratedRow {
                vector: Some(unit(&[1.0, 0.0, 0.0, 0.0])),
                meta: meta_of(&[("manufacturer", &["sony"]), ("price", &["199.99"])]),
                ..HydratedRow::new("a")
            },
            HydratedRow {
                vector: Some(unit(&[0.7, 0.714, 0.0, 0.0])),
                meta: meta_of(&[("manufacturer", &["sony"]), ("price", &["195.00"])]),
                ..HydratedRow::new("b")
            },
        ];
        let ladder = vec![
            Scenario::new("identity", vec![KernelChannel::Semantic { max_distance: 0.05 }]),
            Scenario::new(
                "context",
                vec![
                    KernelChannel::Semantic { max_distance: 0.4 },
                    KernelChannel::Agree { key: "manufacturer".into() },
                    KernelChannel::Within { key: "price".into(), fraction: 0.05 },
                ],
            ),
        ];
        let p = build_hydrated(&rows, &ladder, None);
        assert_eq!(p.levels[0].grains.len(), 2);
        assert_eq!(p.levels[1].grains.len(), 1); // hydration reached the paraphrase
        assert_eq!(p.levels[1].grains[0].members, ["a", "b"]);
    }

    #[test]
    fn group_object_coincidence_is_a_channel() {
        // No vectors at all: two rows fuse purely because they share a
        // GroupObject — the folds hydrating the kernel.
        let rows = vec![
            HydratedRow { groups: vec!["grp:harbour".into()], ..HydratedRow::new("a") },
            HydratedRow {
                groups: vec!["grp:harbour".into(), "grp:other".into()],
                ..HydratedRow::new("b")
            },
            HydratedRow { groups: vec!["grp:elsewhere".into()], ..HydratedRow::new("c") },
        ];
        let ladder = vec![Scenario::new("sameRoom", vec![KernelChannel::SharedGroup])];
        let p = build_hydrated(&rows, &ladder, None);
        let members: Vec<Vec<String>> =
            p.levels[0].grains.iter().map(|g| g.members.clone()).collect();
        assert!(members.contains(&vec!["a".to_owned(), "b".to_owned()]));
        assert!(members.contains(&vec!["c".to_owned()]));
    }

    #[test]
    fn absent_data_fails_the_channel() {
        // b has no price: the price channel cannot be verified, so it FAILS —
        // a context you cannot verify is not a context.
        let rows = vec![
            HydratedRow { meta: meta_of(&[("price", &["10.00"])]), ..HydratedRow::new("a") },
            HydratedRow::new("b"),
        ];
        let ladder = vec![Scenario::new(
            "price",
            vec![KernelChannel::Within { key: "price".into(), fraction: 0.5 }],
        )];
        let p = build_hydrated(&rows, &ladder, None);
        assert_eq!(p.levels[0].grains.len(), 2);
    }

    #[test]
    fn hydration_shapes_the_ladder_to_coverage() {
        // manufacturer on every row; colour on one of four (below coverage);
        // price numeric everywhere; no groups, no vectors.
        let rows: Vec<HydratedRow> = (0..4)
            .map(|i| {
                let mut pairs: Vec<(String, Vec<String>)> = vec![
                    ("manufacturer".to_owned(), vec![format!("m{i}")]),
                    ("price".to_owned(), vec![format!("{}", i * 10 + 5)]),
                ];
                if i == 0 {
                    pairs.push(("colour".to_owned(), vec!["red".to_owned()]));
                }
                HydratedRow {
                    meta: pairs
                        .into_iter()
                        .map(|(k, vs)| (k, vs.into_iter().collect()))
                        .collect(),
                    ..HydratedRow::new(format!("r{i}"))
                }
            })
            .collect();
        let ladder = ladder(&rows);
        let channels: Vec<&KernelChannel> = ladder.iter().flat_map(|s| &s.all).collect();
        assert!(channels
            .iter()
            .any(|c| matches!(c, KernelChannel::Within { key, .. } if key == "price")));
        assert!(channels
            .iter()
            .any(|c| matches!(c, KernelChannel::Agree { key } if key == "manufacturer")));
        assert!(!channels
            .iter()
            .any(|c| matches!(c, KernelChannel::Agree { key } if key == "colour")));
        assert!(!channels
            .iter()
            .any(|c| matches!(c, KernelChannel::Semantic { .. }))); // no vectors
        // The ladder relaxes: strictly fewer channels as levels coarsen.
        let sizes: Vec<usize> = ladder.iter().map(|s| s.all.len()).collect();
        let mut descending = sizes.clone();
        descending.sort_by(|a, b| b.cmp(a));
        assert_eq!(sizes, descending);
    }

    #[test]
    fn coarsened_grains_carry_their_hydration_up() {
        // After a fuse, the grain's meta is the union — a coarse scenario reads
        // coarsened context, exactly like drifted centroids.
        let rows = vec![
            HydratedRow {
                meta: meta_of(&[("tag", &["x"])]),
                groups: vec!["g1".into()],
                ..HydratedRow::new("a")
            },
            HydratedRow {
                meta: meta_of(&[("tag", &["y"])]),
                groups: vec!["g1".into()],
                ..HydratedRow::new("b")
            },
            HydratedRow {
                meta: meta_of(&[("tag", &["y"])]),
                groups: vec!["g2".into()],
                ..HydratedRow::new("c")
            },
        ];
        let ladder = vec![
            Scenario::new("room", vec![KernelChannel::SharedGroup]), // a+b fuse
            Scenario::new("tag", vec![KernelChannel::Agree { key: "tag".into() }]), // (ab) meets c via y
        ];
        let p = build_hydrated(&rows, &ladder, None);
        assert_eq!(p.levels[0].grains.len(), 2);
        assert_eq!(p.levels[1].grains.len(), 1); // the union carried "y" upward
    }

    #[test]
    fn deterministic_run_to_run() {
        let rows = vec![
            HydratedRow {
                vector: Some(unit(&[1.0, 0.0, 0.0, 0.0])),
                meta: meta_of(&[("m", &["s"])]),
                ..HydratedRow::new("a")
            },
            HydratedRow {
                vector: Some(unit(&[1.0, 0.01, 0.0, 0.0])),
                meta: meta_of(&[("m", &["s"])]),
                ..HydratedRow::new("b")
            },
            HydratedRow {
                vector: Some(unit(&[0.0, 1.0, 0.0, 0.0])),
                meta: meta_of(&[("m", &["t"])]),
                ..HydratedRow::new("c")
            },
        ];
        let ladder = ladder(&rows);
        let one = build_hydrated(&rows, &ladder, None);
        let two = build_hydrated(&rows, &ladder, None);
        let members = |p: &Pyramid| -> Vec<Vec<Vec<String>>> {
            p.levels
                .iter()
                .map(|l| l.grains.iter().map(|g| g.members.clone()).collect())
                .collect()
        };
        assert_eq!(members(&one), members(&two));
        assert_eq!(one.persistence, two.persistence);
    }
}
