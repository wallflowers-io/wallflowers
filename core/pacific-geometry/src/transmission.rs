//! Transmission — THE CORE TRANSMISSION (7 Aug directive): episodes → instances →
//! categories → ranked candidates, all on device, no model in the loop.
//!
//! ```text
//!     LodeDB is required to categorise via spectral analysis of embedding
//!     space / ingested episodes. K-means clustering is used to resolve People,
//!     Places, Events, and Groups. All are ranked via sentiment transduced via
//!     pacific-ontology.ttl. Category may be orthogonal to instance.
//! ```
//!
//! What that means as machinery, layer by layer:
//!   INSTANCE   SpectralClustering partitions the episode vectors by the space's
//!              own geometry — a cluster is a CANDIDATE INSTANCE (this person,
//!              this place), not a category.
//!   CATEGORY   an orthogonal read: each cluster's original-space centroid is
//!              projected against the ontology's kind anchors (OntologyAnchors,
//!              from the .ttl at load time). The affinity VECTOR is the output —
//!              never an argmax. Category may be orthogonal to instance.
//!   RANK       sentiment, scored per episode on device (Apple NL — a gauge, not
//!              a generator), aggregated per cluster, and TRANSDUCED through the
//!              kind affinities: a cluster's charge distributes across the kinds
//!              it resembles, so each kind ranks its own candidates by how much
//!              felt life points at them.
//!
//! Doctrine (ontology v0.4, verbatim): clustering lenses are "never stored,
//! exactly as Themes are centroids and not assertions." A TransmissionFrame is
//! COMPUTED and shown; minting stays a user act on LIFE (4 Aug ruling — this
//! lens is the no-LLM resolution path, not a return of extraction). Everything
//! is deterministic given the same episodes, except sentiment's model version —
//! which is why the frame records it.
//!
//! Port of PacificStore/Transmission.swift (1:1; Swift Float → f32, Double →
//! f64, Int → usize, UInt64 → u64). The Apple NL gauge stays Swift-side; here
//! the injectable `Gauge` carries the same neutral fallback ("none", score 0).
//! Swift's `[String: _]` dictionaries are BTreeMaps: Swift Dictionary iteration
//! is unordered, and the determinism contract wants every walk over these maps
//! reproducible.

use std::collections::BTreeMap;

use crate::ontology_anchors::KindAnchor;
use crate::space_calibration::SpaceCalibration;
use crate::spectral::{cluster, SpectralParams};
use crate::vec::dot;

// MARK: - small vector helper
//
// Swift Transmission reads through `SpectralClustering.normalize` — the
// reciprocal-multiply form (`x * (1/√mag)`), which rounds DIFFERENTLY from
// `vec::normalize`'s divide for ~80% of vectors. spectral.rs keeps its verbatim
// copy private, so this module carries its own bit-identical copy; the FFI swap
// pins Swift-bit-identical output. (Swift's `SpectralClustering.dot` is an
// f32-accumulating min-prefix dot returned through Double; `Float(dot(…))`
// round-trips that f32 exactly, so `vec::dot` is bit-identical here.)
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

// MARK: - Input

/// One ingested episode as the lens reads it: the union-store row's stable id,
/// its vector (LodeDBCore.getVectors — quantization-reconstructed, same space),
/// its retained text (sentiment), and its event time (recency windows are the
/// caller's concern; the lens takes what it's given).
#[derive(Clone, Debug)]
pub struct TransmissionEpisode {
    pub id: String,
    pub vector: Vec<f32>,
    pub text: String,
    pub ts: u64,
}

impl TransmissionEpisode {
    pub fn new(id: impl Into<String>, vector: Vec<f32>, text: impl Into<String>, ts: u64) -> Self {
        TransmissionEpisode { id: id.into(), vector, text: text.into(), ts }
    }
}

// MARK: - Output

/// One resolved candidate instance: a cluster, its members, its orthogonal
/// category read, and its sentiment charge.
#[derive(Clone, Debug)]
pub struct TransmissionCandidate {
    /// members, input order preserved
    pub episode_ids: Vec<String>,
    /// original-space, unit length
    pub centroid: Vec<f32>,
    /// kind name → cosine affinity of the centroid to that kind's anchor.
    /// THE VECTOR IS THE ANSWER — collapsing it to a winner is the caller's
    /// mistake to refuse.
    pub kind_affinity: BTreeMap<String, f32>,
    /// Mean episode sentiment in [-1, +1] (0 when no gauge is available).
    pub sentiment: f32,
    /// kind name → this candidate's transduced rank score for that kind's list:
    /// |sentiment| · affinity · log2(1 + members). Charge times resemblance
    /// times evidence mass.
    pub rank_score: BTreeMap<String, f32>,
}

/// The computed lens frame — shown, never stored (Themes-are-centroids rule).
#[derive(Clone, Debug)]
pub struct TransmissionFrame {
    pub candidates: Vec<TransmissionCandidate>,
    /// kind name → candidate indices, best first — the per-kind shortlists the
    /// mint surfaces read.
    pub ranked: BTreeMap<String, Vec<usize>>,
    /// The eigenvalues that justified k — the lens shows its working.
    pub eigenvalues: Vec<f64>,
    /// What produced sentiment ("apple-nl" | "none" | an injected gauge's name):
    /// the one non-deterministic ingredient, named so a frame is reproducible
    /// in context.
    pub sentiment_gauge: String,
}

// MARK: - The lens

/// Sentiment gauge: text → [-1, +1]. Injectable for tests and for platforms
/// without Apple NL; `name` labels the frame.
pub struct Gauge {
    pub name: String,
    pub score: Box<dyn Fn(&str) -> f32>,
}

impl Gauge {
    pub fn new(name: impl Into<String>, score: impl Fn(&str) -> f32 + 'static) -> Gauge {
        Gauge { name: name.into(), score: Box::new(score) }
    }

    /// Apple's on-device sentiment — a gauge, not a generator; no LLM — stays
    /// Swift-side (`Gauge.appleNL()`). This is the Swift `#else` branch
    /// verbatim: the neutral gauge, name "none", score 0. Swift's
    /// `gauge: = .appleNL()` default is expressed here by passing
    /// `&Gauge::none()`.
    pub fn none() -> Gauge {
        Gauge::new("none", |_| 0.0)
    }
}

/// Run the lens: cluster the episodes, read each cluster's category
/// orthogonally against `anchors`, rank per kind by transduced sentiment.
///
/// Pass `calibration` for REAL embedding vectors: instance structure and
/// category affinities are then read in the centered frame, where 0 means
/// unrelated — which raw cosine never meant in an anisotropic space.
///
/// (Swift defaults `params: SpectralParams()`, `gauge: .appleNL()`,
/// `calibration: nil` are expressed by passing `&SpectralParams::default()`,
/// `&Gauge::none()`, `None`.)
pub fn frame(
    episodes: &[TransmissionEpisode],
    anchors: &[KindAnchor],
    params: &SpectralParams,
    gauge: &Gauge,
    calibration: Option<&SpaceCalibration>,
) -> TransmissionFrame {
    if episodes.is_empty() {
        return TransmissionFrame {
            candidates: Vec::new(),
            ranked: BTreeMap::new(),
            eigenvalues: Vec::new(),
            sentiment_gauge: gauge.name.clone(),
        };
    }

    // Into the calibrated frame once — instance and category both read here.
    let framed: Vec<Vec<f32>> = match calibration {
        Some(cal) => episodes.iter().map(|e| cal.center(&e.vector)).collect(),
        None => episodes.iter().map(|e| normalize(&e.vector)).collect(),
    };

    // INSTANCE — spectral structure over the framed episode vectors.
    let ids: Vec<String> = episodes.iter().map(|e| e.id.clone()).collect();
    let result = cluster(&ids, &framed, params, None);

    let mut members: Vec<Vec<usize>> = vec![Vec::new(); result.k.max(1)];
    for (i, &c) in result.assignments.iter().enumerate() {
        members[c].push(i);
    }

    let unit_anchors: Vec<(String, Vec<f32>)> = anchors
        .iter()
        .map(|anchor| {
            (
                anchor.kind.name.clone(),
                match calibration {
                    Some(cal) => cal.center(&anchor.vector),
                    None => normalize(&anchor.vector),
                },
            )
        })
        .collect();

    let mut candidates: Vec<TransmissionCandidate> = Vec::new();
    for rows in members.iter().filter(|rows| !rows.is_empty()) {
        // Framed-space centroid: the spectral embedding clusters, the framed
        // (centered-or-unit) space carries meaning — anchors are read HERE,
        // in the same frame they were moved into above.
        let dim = framed[rows[0]].len();
        let mut centroid = vec![0.0f32; dim];
        for &r in rows {
            let v = &framed[r];
            for d in 0..dim {
                centroid[d] += v[d];
            }
        }
        let scaled: Vec<f32> = centroid.iter().map(|x| x / rows.len() as f32).collect();
        let centroid = normalize(&scaled);

        // CATEGORY — the orthogonal read. The whole vector survives.
        let mut affinity: BTreeMap<String, f32> = BTreeMap::new();
        for (name, anchor) in &unit_anchors {
            affinity.insert(name.clone(), dot(&centroid, anchor));
        }

        // RANK — sentiment charge, transduced through the affinities.
        let charge = rows
            .iter()
            .map(|&r| (gauge.score)(&episodes[r].text))
            .fold(0.0f32, |acc, s| acc + s) // in-order, as Swift reduce(0, +)
            / rows.len() as f32;
        let mass = ((1 + rows.len()) as f32).log2();
        let mut score: BTreeMap<String, f32> = BTreeMap::new();
        for (name, &a) in &affinity {
            score.insert(name.clone(), charge.abs() * a.max(0.0) * mass);
        }

        candidates.push(TransmissionCandidate {
            episode_ids: rows.iter().map(|&r| episodes[r].id.clone()).collect(),
            centroid,
            kind_affinity: affinity,
            sentiment: charge,
            rank_score: score,
        });
    }

    // Per-kind shortlists, best first; ties broken by first episode id so the
    // frame is stable end to end. (Rust `sort_by` is stable, as Swift's
    // `sorted`; the id tiebreak is byte-lexicographic — identical to Swift's
    // `<` for the ASCII ids the store mints.)
    let mut ranked: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (name, _) in &unit_anchors {
        let mut order: Vec<usize> = (0..candidates.len()).collect();
        order.sort_by(|&x, &y| {
            let a = candidates[x].rank_score.get(name).copied().unwrap_or(0.0);
            let b = candidates[y].rank_score.get(name).copied().unwrap_or(0.0);
            if a != b {
                // Swift `a > b` as the areInIncreasingOrder verdict.
                if a > b {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            } else {
                first_id(&candidates[x]).cmp(first_id(&candidates[y]))
            }
        });
        ranked.insert(name.clone(), order);
    }

    TransmissionFrame {
        candidates,
        ranked,
        eigenvalues: result.eigenvalues,
        sentiment_gauge: gauge.name.clone(),
    }
}

fn first_id(c: &TransmissionCandidate) -> &str {
    c.episode_ids.first().map_or("", |s| s.as_str())
}

// MARK: - tests (ported 1:1 from TransmissionTests.swift · TransmissionFrameTests)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ontology_anchors::OntologyKind;
    use crate::spectral::SplitMix64;

    // Deterministic blob generator — SplitMix64, never SystemRandom.
    // RNG consumption order matters (row by row, dim by dim), exactly as Swift.
    fn blob(center: &[f32], n: usize, spread: f32, seed: u64) -> Vec<Vec<f32>> {
        let mut rng = SplitMix64::new(seed);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let mut row = Vec::with_capacity(center.len());
            for &c in center {
                let u = (rng.next() % 10_000) as f32 / 10_000.0 - 0.5;
                row.push(c + u * spread);
            }
            out.push(row);
        }
        out
    }

    /// A fake embedder/gauge world: anchors on axes, episodes near them.
    fn world() -> (Vec<TransmissionEpisode>, Vec<KindAnchor>) {
        let person_anchor = KindAnchor {
            kind: OntologyKind { name: "Person".to_string(), definition: "a human".to_string() },
            vector: vec![1.0, 0.0, 0.0, 0.0],
        };
        let place_anchor = KindAnchor {
            kind: OntologyKind { name: "Place".to_string(), definition: "a where".to_string() },
            vector: vec![0.0, 1.0, 0.0, 0.0],
        };
        let mut episodes: Vec<TransmissionEpisode> = Vec::new();
        // A charged person-shaped cluster…
        for (i, v) in blob(&[1.0, 0.1, 0.0, 0.0], 8, 0.1, 7).into_iter().enumerate() {
            episodes.push(TransmissionEpisode::new(format!("p{i}"), v, "love this", i as u64));
        }
        // …and a neutral place-shaped one.
        for (i, v) in blob(&[0.1, 1.0, 0.0, 0.0], 8, 0.1, 8).into_iter().enumerate() {
            episodes.push(TransmissionEpisode::new(format!("q{i}"), v, "the corner", i as u64));
        }
        (episodes, vec![person_anchor, place_anchor])
    }

    fn gauge() -> Gauge {
        Gauge::new("fake", |t: &str| if t.contains("love") { 0.9 } else { 0.0 })
    }

    #[test]
    fn category_stays_orthogonal_to_instance() {
        let (episodes, anchors) = world();
        let f = frame(&episodes, &anchors, &SpectralParams::default(), &gauge(), None);
        assert_eq!(f.candidates.len(), 2);
        // Every candidate carries the FULL affinity vector — both kinds present,
        // whatever it most resembles.
        for c in &f.candidates {
            assert_eq!(c.kind_affinity.len(), 2);
        }
    }

    #[test]
    fn sentiment_transduces_into_the_ranking() {
        let (episodes, anchors) = world();
        let f = frame(&episodes, &anchors, &SpectralParams::default(), &gauge(), None);
        // The charged cluster is the person-shaped one; it must top the Person
        // list and carry its charge.
        let person_best = f.ranked["Person"][0];
        let best = &f.candidates[person_best];
        assert!(best.episode_ids.first().is_some_and(|id| id.starts_with('p')));
        assert!(best.sentiment > 0.5);
        // The neutral cluster's rank score is zero everywhere: no charge, no rank.
        let neutral = f
            .candidates
            .iter()
            .find(|c| c.episode_ids.first().is_some_and(|id| id.starts_with('q')))
            .unwrap();
        assert!(neutral.rank_score.values().all(|&s| s == 0.0));
    }

    #[test]
    fn frame_names_its_gauge() {
        let (episodes, anchors) = world();
        let f = frame(&episodes, &anchors, &SpectralParams::default(), &gauge(), None);
        assert_eq!(f.sentiment_gauge, "fake");
    }

    #[test]
    fn empty_episodes_empty_frame() {
        let f = frame(&[], &[], &SpectralParams::default(), &gauge(), None);
        assert!(f.candidates.is_empty());
        assert!(f.ranked.is_empty());
    }
}
