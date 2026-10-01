//! coarsening — THE COARSENING PYRAMID (8 Aug directive): "we can classify and
//! deduplicate by recursively convolving an embedded matter through sequentially
//! coarsening kernels."
//!
//! The recursion, literally:
//!   matter⁰ = the embedded rows (calibrated frame, unit mass each)
//!   at level ℓ: fuse every pair the kernel of bandwidth σ_ℓ joins
//!               (components under cosine-distance ≤ σ_ℓ — the kernel's
//!               support radius); each fused cluster becomes ONE grain of
//!               matter: mass-weighted centroid, summed mass, unioned members
//!   σ_{ℓ+1} = γ·σ_ℓ  — the kernel coarsens; recurse on the COARSENED matter
//!               (not the originals: mass centres drift as matter fuses,
//!               which is the convolution doing its work)
//!   stop     when one grain remains or maxLevels is spent
//!
//! ONE operation, both answers:
//!   DEDUPLICATE  what fuses at the finest scales is near-identity — those
//!                multi-member grains are the resolution calculus's candidate
//!                sets (they feed GraphResolution's blocking/proposals, they
//!                never merge anything themselves).
//!   CLASSIFY     what fuses at coarser scales is family structure — read a
//!                grain's category orthogonally against the ontology anchors,
//!                per level. Category stays orthogonal to instance at every
//!                scale.
//!
//! PERSISTENCE is the reality test: a grain's lifespan in levels (born when
//! formed, dead when absorbed). Structure that persists across many kernel
//! widths is a standing wave — the vortex, made computable. Noise fuses late
//! and dies immediately. This is 0-dimensional persistence over a scale-space,
//! which is what "sequentially coarsening kernels" resolves to when the
//! operation must be deterministic and on-device.
//!
//! Grounding (8 Aug ruling): the kernel operates on CALIBRATED vectors — in
//! the raw anisotropic cone every pair sits within one kernel width of the
//! common direction and the pyramid collapses at level 0. Determinism: no
//! randomness anywhere; sorted iteration and id tie-breaks only. (Where the
//! Swift original iterated an unordered Dictionary, this port iterates a
//! BTreeMap — the sorted iteration the determinism contract demands.)
//!
//! TWO KERNELS, ONE RECURSION (Ralph, mid-build: "this is computable from
//! existing graph structure"): the semantic kernel above needs vectors; the
//! STRUCTURAL kernel needs only the graph the projector already writes.
//! Affinity = cosine between two grains' typed-incidence profiles — sharing
//! neighbours under the same verbs IS the kernel — and coarsening CONTRACTS
//! the graph (fused grains merge, their edges rewire and sum), so the next
//! level's kernel is computed on the coarsened structure. Same Grain, same
//! persistence, same readings; and it covers the rows embeddings never reach
//! (rosters, memberships, folds). `build_from_graph` below.

use std::collections::BTreeMap;

use crate::graph_resolution::GraphEdgeRow;
use crate::ontology_anchors::KindAnchor;
use crate::space_calibration::SpaceCalibration;
use crate::vec::{dot, normalize};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PyramidParams {
    /// Finest kernel bandwidth, in cosine distance on the calibrated frame.
    /// 0.05 ≈ "the same thing said twice".
    pub sigma0: f32,
    /// Kernel coarsening factor per level (dyadic).
    pub gamma: f32,
    pub max_levels: usize,
}

impl Default for PyramidParams {
    fn default() -> Self {
        PyramidParams {
            sigma0: 0.05,
            gamma: 2.0,
            max_levels: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StructuralParams {
    /// Finest structural kernel: incidence-profile cosine ≥ tau0 fuses.
    /// 0.8 ≈ "connected to the same things by the same verbs" — the
    /// graph-vouched duplicate, no vectors required.
    pub tau0: f32,
    /// Kernel coarsening: the threshold RELAXES by this factor per level.
    pub relax: f32,
    pub max_levels: usize,
}

impl Default for StructuralParams {
    fn default() -> Self {
        StructuralParams {
            tau0: 0.8,
            relax: 0.6,
            max_levels: 8,
        }
    }
}

/// One grain of matter at some level: a cluster with mass, centre and history.
#[derive(Clone, Debug, PartialEq)]
pub struct Grain {
    /// the lexicographically-least member id
    pub id: String,
    /// unit, mass-weighted
    pub centroid: Vec<f32>,
    /// episodes absorbed
    pub mass: f32,
    /// sorted
    pub members: Vec<String>,
    /// level at which this grain formed
    pub born: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PyramidLevel {
    pub sigma: f32,
    pub grains: Vec<Grain>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pyramid {
    pub levels: Vec<PyramidLevel>,
    /// grain id → number of levels it survived unabsorbed (its persistence).
    pub persistence: BTreeMap<String, usize>,
}

impl Pyramid {
    /// The dedup reading: multi-member grains at the finest level — candidate
    /// identity sets for the resolution calculus, never merges.
    pub fn duplicate_sets(&self) -> Vec<Vec<String>> {
        self.levels
            .first()
            .map(|l0| {
                l0.grains
                    .iter()
                    .filter(|g| g.members.len() > 1)
                    .map(|g| g.members.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The classification reading at one level: each grain's calibrated
    /// alignment to each anchor — the category axis, orthogonal to the grains.
    pub fn classes_at_level(
        &self,
        l: usize,
        anchors: &[KindAnchor],
        calibration: Option<&SpaceCalibration>,
    ) -> Vec<(Grain, BTreeMap<String, f32>)> {
        if l >= self.levels.len() {
            return Vec::new();
        }
        self.levels[l]
            .grains
            .iter()
            .map(|g| {
                let mut kind: BTreeMap<String, f32> = BTreeMap::new();
                for a in anchors {
                    let anchor = match calibration {
                        Some(cal) => cal.center(&a.vector),
                        None => normalize(&a.vector),
                    };
                    let c = dot(&g.centroid, &anchor);
                    kind.insert(
                        a.kind.name.clone(),
                        match calibration {
                            Some(cal) => cal.calibrated01(c),
                            None => c,
                        },
                    );
                }
                (g.clone(), kind)
            })
            .collect()
    }
}

/// Union-find with the Swift original's exact shape: root-chase, then a single
/// path-compression write for the queried index.
fn find(parent: &mut [usize], x: usize) -> usize {
    let mut r = x;
    while parent[r] != r {
        r = parent[r];
    }
    parent[x] = r;
    r
}

/// Recursively convolve the matter. Vectors are moved into the calibrated
/// frame first when a calibration is given — on real embeddings it must be.
pub fn build(
    ids: &[String],
    vectors: &[Vec<f32>],
    params: &PyramidParams,
    calibration: Option<&SpaceCalibration>,
) -> Pyramid {
    assert_eq!(ids.len(), vectors.len(), "ids and vectors must align");
    if ids.is_empty() {
        return Pyramid {
            levels: Vec::new(),
            persistence: BTreeMap::new(),
        };
    }

    let framed: Vec<Vec<f32>> = match calibration {
        Some(cal) => vectors.iter().map(|v| cal.center(v)).collect(),
        None => vectors.iter().map(|v| normalize(v)).collect(),
    };

    // matter⁰ — one grain per row, in id order for determinism.
    let mut pairs: Vec<(String, Vec<f32>)> = ids.iter().cloned().zip(framed).collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut matter: Vec<Grain> = pairs
        .into_iter()
        .map(|(id, centroid)| Grain {
            id: id.clone(),
            centroid,
            mass: 1.0,
            members: vec![id],
            born: 0,
        })
        .collect();

    let mut levels: Vec<PyramidLevel> = Vec::new();
    let mut died: BTreeMap<String, usize> = BTreeMap::new(); // grain id → level it was absorbed at
    let mut sigma = params.sigma0;

    for level in 0..params.max_levels {
        // Fuse: connected components under kernel support d ≤ σ.
        let mut parent: Vec<usize> = (0..matter.len()).collect();
        for i in 0..matter.len() {
            for j in (i + 1)..matter.len() {
                // Swift: `Float(1 - SpectralClustering.dot(…))` — the dot
                // accumulates in Float, widens to Double, and the subtraction
                // happens in f64 before narrowing to f32. Mirrored exactly.
                let d = (1.0f64 - f64::from(dot(&matter[i].centroid, &matter[j].centroid))) as f32;
                if d <= sigma {
                    let ri = find(&mut parent, i);
                    let rj = find(&mut parent, j);
                    if ri != rj {
                        parent[ri.max(rj)] = ri.min(rj);
                    }
                }
            }
        }

        let mut groups: BTreeMap<usize, Vec<Grain>> = BTreeMap::new();
        for (i, g) in matter.iter().enumerate() {
            let r = find(&mut parent, i);
            groups.entry(r).or_default().push(g.clone());
        }

        let mut next: Vec<Grain> = Vec::new();
        for (_, group) in groups {
            if group.len() == 1 {
                next.push(group.into_iter().next().unwrap());
                continue;
            }
            // The convolution step: fused matter becomes one grain at its
            // mass-weighted centre. Absorbed grains record their death.
            let dim = group[0].centroid.len();
            let mut centre = vec![0.0f32; dim];
            let mut mass: f32 = 0.0;
            for g in &group {
                for d in 0..dim {
                    centre[d] += g.mass * g.centroid[d];
                }
                mass += g.mass;
            }
            let mut members: Vec<String> = group
                .iter()
                .flat_map(|g| g.members.iter().cloned())
                .collect();
            members.sort();
            let survivor = group.iter().map(|g| g.id.clone()).min().unwrap();
            for g in &group {
                if g.id != survivor {
                    died.insert(g.id.clone(), level);
                }
            }
            let born = if group.iter().any(|g| g.members.len() > 1) || group.len() > 1 {
                level
            } else {
                group[0].born
            };
            let scaled: Vec<f32> = centre.iter().map(|c| c / mass).collect();
            next.push(Grain {
                id: survivor,
                centroid: normalize(&scaled),
                mass,
                members,
                born,
            });
        }

        next.sort_by(|a, b| a.id.cmp(&b.id));
        matter = next;
        levels.push(PyramidLevel {
            sigma,
            grains: matter.clone(),
        });
        if matter.len() == 1 {
            break;
        }
        sigma *= params.gamma;
    }

    Pyramid {
        persistence: persistence_of(&levels, &died),
        levels,
    }
}

// MARK: - The structural kernel (computable from existing graph structure)

/// Recursively convolve the GRAPH: affinity is the cosine between two
/// grains' typed-incidence profiles ("verb>peer" / "verb<peer", weights
/// summed), fusion contracts the graph (edges rewire to survivors and
/// merge), and the relaxing threshold is the coarsening kernel. No vectors
/// anywhere — this runs on rows embeddings never reach. Grains carry no
/// centroid (empty), so `classes_at_level` is a semantic-mode reading;
/// structural grains classify by their members' node kinds instead.
pub fn build_from_graph(
    node_ids: &[String],
    edges: &[GraphEdgeRow],
    params: &StructuralParams,
) -> Pyramid {
    let mut base: std::collections::BTreeSet<String> = node_ids.iter().cloned().collect();
    for e in edges {
        base.insert(e.src.clone());
        base.insert(e.dst.clone());
    }
    let base_ids: Vec<String> = base.into_iter().collect();
    if base_ids.is_empty() {
        return Pyramid {
            levels: Vec::new(),
            persistence: BTreeMap::new(),
        };
    }

    // The working multigraph: contracted endpoints, summed weights.
    struct Link {
        src: String,
        verb: String,
        dst: String,
        weight: f32,
    }
    let mut links: Vec<Link> = edges
        .iter()
        .map(|e| Link {
            src: e.src.clone(),
            verb: e.verb.clone(),
            dst: e.dst.clone(),
            weight: 1.0,
        })
        .collect();
    let mut matter: Vec<Grain> = base_ids
        .iter()
        .map(|id| Grain {
            id: id.clone(),
            centroid: Vec::new(),
            mass: 1.0,
            members: vec![id.clone()],
            born: 0,
        })
        .collect();

    let mut levels: Vec<PyramidLevel> = Vec::new();
    let mut died: BTreeMap<String, usize> = BTreeMap::new();
    let mut tau = params.tau0;

    for level in 0..params.max_levels {
        // Incidence profiles on the CURRENT (contracted) structure.
        let mut profile: BTreeMap<String, BTreeMap<String, f32>> = BTreeMap::new();
        for l in &links {
            if l.src == l.dst {
                continue;
            }
            *profile
                .entry(l.src.clone())
                .or_default()
                .entry(format!("{}>{}", l.verb, l.dst))
                .or_insert(0.0) += l.weight;
            *profile
                .entry(l.dst.clone())
                .or_default()
                .entry(format!("{}<{}", l.verb, l.src))
                .or_insert(0.0) += l.weight;
        }
        fn cosine(a: &BTreeMap<String, f32>, b: &BTreeMap<String, f32>) -> f32 {
            let mut dot: f32 = 0.0;
            for (k, va) in a {
                if let Some(vb) = b.get(k) {
                    dot += va * vb;
                }
            }
            let na = a.values().map(|v| v * v).sum::<f32>().sqrt();
            let nb = b.values().map(|v| v * v).sum::<f32>().sqrt();
            if !(na > 0.0 && nb > 0.0) {
                return 0.0;
            }
            dot / (na * nb)
        }

        let mut parent: Vec<usize> = (0..matter.len()).collect();
        for i in 0..matter.len() {
            let Some(pi) = profile.get(&matter[i].id) else {
                continue;
            };
            for j in (i + 1)..matter.len() {
                let Some(pj) = profile.get(&matter[j].id) else {
                    continue;
                };
                if cosine(pi, pj) >= tau {
                    let ri = find(&mut parent, i);
                    let rj = find(&mut parent, j);
                    if ri != rj {
                        parent[ri.max(rj)] = ri.min(rj);
                    }
                }
            }
        }

        let mut groups: BTreeMap<usize, Vec<Grain>> = BTreeMap::new();
        for (i, g) in matter.iter().enumerate() {
            let r = find(&mut parent, i);
            groups.entry(r).or_default().push(g.clone());
        }

        let mut canon: BTreeMap<String, String> = BTreeMap::new();
        let mut next: Vec<Grain> = Vec::new();
        for (_, group) in groups {
            let survivor = group.iter().map(|g| g.id.clone()).min().unwrap();
            for g in &group {
                canon.insert(g.id.clone(), survivor.clone());
            }
            if group.len() == 1 {
                next.push(group.into_iter().next().unwrap());
                continue;
            }
            for g in &group {
                if g.id != survivor {
                    died.insert(g.id.clone(), level);
                }
            }
            let mut members: Vec<String> = group
                .iter()
                .flat_map(|g| g.members.iter().cloned())
                .collect();
            members.sort();
            next.push(Grain {
                id: survivor,
                centroid: Vec::new(),
                mass: group.iter().map(|g| g.mass).sum(),
                members,
                born: level,
            });
        }

        // CONTRACT: rewire links through the fusion, merge parallels.
        let mut merged: BTreeMap<String, Link> = BTreeMap::new();
        for l in links {
            let s = canon.get(&l.src).cloned().unwrap_or_else(|| l.src.clone());
            let d = canon.get(&l.dst).cloned().unwrap_or_else(|| l.dst.clone());
            if s == d {
                continue; // interior structure is absorbed mass
            }
            let key = format!("{}|{}|{}", s, l.verb, d);
            match merged.get_mut(&key) {
                Some(existing) => existing.weight += l.weight,
                None => {
                    merged.insert(
                        key,
                        Link {
                            src: s,
                            verb: l.verb,
                            dst: d,
                            weight: l.weight,
                        },
                    );
                }
            }
        }
        // (Swift sorts merged by key here; BTreeMap yields exactly that order.)
        links = merged.into_values().collect();

        next.sort_by(|a, b| a.id.cmp(&b.id));
        matter = next;
        levels.push(PyramidLevel {
            sigma: tau,
            grains: matter.clone(),
        });
        if matter.len() == 1 || links.is_empty() {
            break;
        }
        tau *= params.relax;
    }

    Pyramid {
        persistence: persistence_of(&levels, &died),
        levels,
    }
}

/// Persistence: survivors live to the last level; the absorbed lived from
/// birth to death.
fn persistence_of(levels: &[PyramidLevel], died: &BTreeMap<String, usize>) -> BTreeMap<String, usize> {
    let mut births: BTreeMap<String, usize> = BTreeMap::new();
    for level in levels {
        for g in &level.grains {
            births.entry(g.id.clone()).or_insert(g.born);
        }
    }
    let mut persistence: BTreeMap<String, usize> = BTreeMap::new();
    for (id, b) in births {
        let death = died.get(&id).copied().unwrap_or(levels.len());
        persistence.insert(id, death - b);
    }
    persistence
}

// MARK: - Tests
//
// The recursive convolution proven on synthetic matter — one operation, both
// answers: near-identity fuses at fine kernels (dedup), family structure at
// coarse ones (classes), persistence separates standing waves from noise,
// mass is conserved through every convolution, and the whole pyramid is
// deterministic. Plus the grounding case: on cone geometry the pyramid only
// works in the calibrated frame.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_resolution::{BiTime, Provenance};
    use crate::ontology_anchors::OntologyKind;
    use crate::spectral::SplitMix64;

    fn unit(v: &[f32]) -> Vec<f32> {
        normalize(v)
    }

    fn s(x: &str) -> String {
        x.to_string()
    }

    /// Two near-duplicates, one distinct row, plus a far blob of three.
    fn matter() -> (Vec<String>, Vec<Vec<f32>>) {
        let vectors: Vec<Vec<f32>> = vec![
            unit(&[1.0, 0.00, 0.0, 0.0]), // a1 ─┐ near-identical
            unit(&[1.0, 0.02, 0.0, 0.0]), // a2 ─┘
            unit(&[0.8, 0.6, 0.0, 0.0]),  // b — same family, not a duplicate
            unit(&[0.0, 0.0, 1.0, 0.00]), // c1 ─┐
            unit(&[0.0, 0.0, 1.0, 0.03]), // c2 ─┼ far blob
            unit(&[0.0, 0.03, 1.0, 0.0]), // c3 ─┘
        ];
        (
            vec![s("a1"), s("a2"), s("b"), s("c1"), s("c2"), s("c3")],
            vectors,
        )
    }

    #[test]
    fn fine_kernels_fuse_only_near_identity() {
        let (ids, vectors) = matter();
        let p = build(&ids, &vectors, &PyramidParams::default(), None);
        assert!(p.duplicate_sets().contains(&vec![s("a1"), s("a2")]));
        // family ≠ duplicate
        assert!(!p
            .duplicate_sets()
            .iter()
            .any(|set| set.contains(&s("b"))));
    }

    #[test]
    fn coarse_kernels_fuse_families() {
        let (ids, vectors) = matter();
        let p = build(&ids, &vectors, &PyramidParams::default(), None);
        // Some mid level holds exactly the two families as two grains.
        let families_level = p.levels.iter().find(|l| {
            l.grains.len() == 2
                && l.grains
                    .iter()
                    .any(|g| g.members == vec![s("a1"), s("a2"), s("b")])
        });
        assert!(families_level.is_some());
        // And the final level is one grain of everything.
        assert_eq!(p.levels.last().map(|l| l.grains.len()), Some(1));
        assert_eq!(
            p.levels.last().and_then(|l| l.grains.first()).map(|g| g.members.len()),
            Some(6)
        );
    }

    #[test]
    fn mass_is_conserved_through_every_convolution() {
        let (ids, vectors) = matter();
        let p = build(&ids, &vectors, &PyramidParams::default(), None);
        for level in &p.levels {
            assert_eq!(level.grains.iter().map(|g| g.mass).sum::<f32>(), 6.0);
        }
    }

    #[test]
    fn persistence_separates_standing_waves_from_late_fusions() {
        let (ids, vectors) = matter();
        let p = build(&ids, &vectors, &PyramidParams::default(), None);
        // The far blob's surviving grain forms early and outlives the grain
        // that only exists in the final all-fused level.
        let blob = p.persistence.get("c1").copied().unwrap_or(0);
        let lone = p.persistence.get("b").copied().unwrap_or(0);
        assert!(blob > lone);
    }

    #[test]
    fn deterministic_run_to_run() {
        let (ids, vectors) = matter();
        let a = build(&ids, &vectors, &PyramidParams::default(), None);
        let b = build(&ids, &vectors, &PyramidParams::default(), None);
        assert_eq!(a.levels.len(), b.levels.len());
        for (la, lb) in a.levels.iter().zip(&b.levels) {
            assert_eq!(
                la.grains.iter().map(|g| &g.id).collect::<Vec<_>>(),
                lb.grains.iter().map(|g| &g.id).collect::<Vec<_>>()
            );
            assert_eq!(
                la.grains.iter().map(|g| &g.members).collect::<Vec<_>>(),
                lb.grains.iter().map(|g| &g.members).collect::<Vec<_>>()
            );
        }
        assert_eq!(a.persistence, b.persistence);
    }

    #[test]
    fn empty_and_singleton_are_honest() {
        assert!(build(&[], &[], &PyramidParams::default(), None).levels.is_empty());
        let one = build(&[s("x")], &[unit(&[1.0, 0.0])], &PyramidParams::default(), None);
        assert_eq!(one.levels.first().map(|l| l.grains.len()), Some(1));
        assert!(one.duplicate_sets().is_empty());
    }

    #[test]
    fn cone_pyramid_needs_the_calibrated_frame() {
        // A tight cone (residual 0.15): raw cross-group cosine distance ≈ 0.02
        // sits INSIDE the dedup kernel, so the finest kernel fuses everything —
        // unrelated texts read as duplicates, the anisotropy failure exactly.
        // Calibrated, the same matter holds two pure families.
        let mut rng = SplitMix64::new(41);
        let mut cone = |group: usize, n: usize| -> Vec<Vec<f32>> {
            (0..n)
                .map(|_| {
                    let mut v = vec![0.0f32; 8];
                    v[0] = 1.0;
                    v[1 + group] = 0.15;
                    for d in 0..8 {
                        v[d] += ((rng.next() % 1000) as f32 / 1000.0 - 0.5) * 0.02;
                    }
                    unit(&v)
                })
                .collect()
        };
        let mut vectors = cone(0, 10);
        vectors.extend(cone(1, 10));
        let ids: Vec<String> = (0..20).map(|i| format!("e{i:02}")).collect();
        let cal = SpaceCalibration::fit(&vectors).unwrap();

        let raw = build(&ids, &vectors, &PyramidParams::default(), None);
        // dedup destroyed by the cone
        assert_eq!(raw.levels.first().map(|l| l.grains.len()), Some(1));

        let grounded = build(&ids, &vectors, &PyramidParams::default(), Some(&cal));
        assert!(grounded.levels.first().map(|l| l.grains.len()).unwrap_or(0) >= 2);
        let pure_families = grounded.levels.iter().find(|level| {
            level.grains.len() == 2
                && level.grains.iter().all(|g| {
                    g.members.iter().all(|m| m.as_str() < "e10")
                        || g.members.iter().all(|m| m.as_str() >= "e10")
                })
        });
        assert!(pure_families.is_some());
    }

    // MARK: - The structural kernel (no vectors anywhere)

    fn edge(id: &str, src: &str, verb: &str, dst: &str) -> GraphEdgeRow {
        GraphEdgeRow {
            id: s(id),
            src: s(src),
            verb: s(verb),
            dst: s(dst),
            scope: String::new(),
            provenance: Provenance::Folded { object: s("o") },
            time: BiTime {
                valid_at: 0,
                invalid_at: None,
                created_at: 0,
                expired_at: None,
            },
            episode: None,
        }
    }

    #[test]
    fn structural_kernel_fuses_graph_vouched_duplicates() {
        // p1 and p2 attend the same events and belong to the same group —
        // identical incidence profiles, no vectors involved. p3 is elsewhere.
        let edges = vec![
            edge("e1", "p1", "attends", "ev1"),
            edge("e2", "p2", "attends", "ev1"),
            edge("e3", "p1", "attends", "ev2"),
            edge("e4", "p2", "attends", "ev2"),
            edge("e5", "p1", "member_of", "g1"),
            edge("e6", "p2", "member_of", "g1"),
            edge("e7", "p3", "member_of", "g2"),
            edge("e8", "p3", "attends", "ev3"),
        ];
        let p = build_from_graph(&[], &edges, &StructuralParams::default());
        assert!(p
            .duplicate_sets()
            .iter()
            .any(|set| set.contains(&s("p1")) && set.contains(&s("p2"))));
        assert!(!p.duplicate_sets().iter().any(|set| set.contains(&s("p3"))));
    }

    #[test]
    fn structural_coarsening_finds_communities_then_everything() {
        // Two four-cliques joined by one bridge: interiors fuse before the whole.
        let mut edges: Vec<GraphEdgeRow> = Vec::new();
        let mut n = 0;
        let mut clique = |names: &[&str], edges: &mut Vec<GraphEdgeRow>| {
            for i in 0..names.len() {
                for j in (i + 1)..names.len() {
                    edges.push(edge(&format!("c{n}"), names[i], "knows", names[j]));
                    n += 1;
                }
            }
        };
        clique(&["a1", "a2", "a3", "a4"], &mut edges);
        clique(&["b1", "b2", "b3", "b4"], &mut edges);
        edges.push(edge("bridge", "a1", "knows", "b1"));
        let p = build_from_graph(&[], &edges, &StructuralParams::default());

        let community_level = p.levels.iter().find(|level| {
            level.grains.len() == 2
                && level.grains.iter().all(|g| {
                    g.members.iter().all(|m| m.starts_with('a'))
                        || g.members.iter().all(|m| m.starts_with('b'))
                })
        });
        assert!(community_level.is_some());
        for level in &p.levels {
            // mass conserved
            assert_eq!(level.grains.iter().map(|g| g.mass).sum::<f32>(), 8.0);
        }
    }

    #[test]
    fn structural_determinism() {
        let edges = vec![
            edge("e1", "x", "knows", "y"),
            edge("e2", "y", "knows", "z"),
            edge("e3", "x", "knows", "z"),
        ];
        let a = build_from_graph(&[], &edges, &StructuralParams::default());
        let b = build_from_graph(&[], &edges, &StructuralParams::default());
        assert_eq!(a.levels.len(), b.levels.len());
        for (la, lb) in a.levels.iter().zip(&b.levels) {
            assert_eq!(
                la.grains.iter().map(|g| &g.members).collect::<Vec<_>>(),
                lb.grains.iter().map(|g| &g.members).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn classes_read_orthogonally_per_level() {
        let (ids, vectors) = matter();
        let p = build(&ids, &vectors, &PyramidParams::default(), None);
        let anchors = vec![
            KindAnchor {
                kind: OntologyKind {
                    name: s("A"),
                    definition: s("x-ish"),
                },
                vector: unit(&[1.0, 0.2, 0.0, 0.0]),
            },
            KindAnchor {
                kind: OntologyKind {
                    name: s("C"),
                    definition: s("z-ish"),
                },
                vector: unit(&[0.0, 0.0, 1.0, 0.0]),
            },
        ];
        let families_idx = p
            .levels
            .iter()
            .position(|l| l.grains.len() == 2)
            .expect("no two-family level");
        let classes = p.classes_at_level(families_idx, &anchors, None);
        for (grain, kind) in classes {
            if grain.members.contains(&s("a1")) {
                assert!(kind["A"] > kind["C"]);
            }
            if grain.members.contains(&s("c1")) {
                assert!(kind["C"] > kind["A"]);
            }
        }
    }
}
