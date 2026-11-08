//! icd_graph — conformance between the ICD's `x-graph` clauses and what the
//! projector ACTUALLY mints.
//!
//! `pacific-core/src/icd.rs` pins the op INVENTORY: every op a reducer declares
//! appears in `coordination/delta-graph.icd.json`, and every message in the ICD
//! is a real op. It deliberately stops there, and says so:
//!
//!     "What is deliberately NOT asserted: the `x-graph` clauses themselves.
//!      Whether `event.setVenue` really mints `happens_at` is a claim about the
//!      projector, and the projector lives in another crate (arc-resolver);
//!      asserting it here would only compare the document to itself."
//!
//! That reasoning is right, and this module is its other half — the assertion
//! made from the side that can actually keep it. Here the projector is a direct
//! dependency, so the check is BEHAVIOURAL: drive `project_objects` with the
//! folded residue of an op and read back the relations it wrote. Nothing is
//! compared to a hand-kept table of intentions; the graph is asked.
//!
//! Two checks, both table-driven so a run reports EVERY drift at once rather
//! than dying on the first:
//!
//!   vocabulary   every relation constant the projector can emit appears in
//!                `info.x-vocabulary.relations`. A relation the code mints and
//!                the document has never heard of fails here.
//!   projection   for each message marked `x-graph.status: "implemented"` that
//!                declares edges, the set of relations the projector emits from
//!                that op's residue equals the set the message declares.
//!
//! An op whose status is `specified` is exempt by design: the ICD's whole point
//! is that relations are specified before they are built, and `implemented` is
//! the honest line between the two. Promoting a status without building the
//! projection is what fails.

#![cfg(test)]

use std::collections::BTreeSet;

use lodedb_graph::{AsOf, Direction, EmbedRole, Embedder, GraphConfig, GraphError, TemporalGraph};
use serde_json::Value;

use crate::project::{
    AffiliationSnap, ObjectSnapshot, VenueSnap, REL_AFFILIATED_WITH, REL_CREATED, REL_HAPPENS_AT,
    REL_MEMBER_OF,
};
use crate::resolver::ArcResolver;

/// The same deterministic no-model embedder `project.rs`'s tests use: the
/// projector's writes must embed, and a conformance test has no business
/// faking that away just because it does not need a real model.
struct HashEmbedder(usize);
impl Embedder for HashEmbedder {
    fn embed(&self, texts: &[String], _role: EmbedRole) -> Result<Vec<Vec<f32>>, GraphError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0_f32; self.0];
                for (i, b) in t.bytes().enumerate() {
                    v[(i * 31 + b as usize) % self.0] += 1.0;
                }
                v
            })
            .collect())
    }
    fn dimension(&self) -> usize {
        self.0
    }
}

/// Every relation constant the projector is capable of writing. Adding a
/// `REL_*` to `project.rs` without adding it here leaves it unchecked, so the
/// list is deliberately exhaustive and deliberately hand-kept: it is the ONE
/// place a new relation has to be declared, and the vocabulary check below is
/// what makes forgetting it expensive.
const PROJECTOR_RELATIONS: &[&str] = &[
    REL_MEMBER_OF,
    REL_AFFILIATED_WITH,
    REL_HAPPENS_AT,
    REL_CREATED,
];

fn doc() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../../core/coordination/delta-graph.icd.json"
    );
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("the ICD must be readable at {path}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("the ICD must be valid JSON: {e}"))
}

/// The relations one message's `x-graph` declares, and whether it is implemented.
fn declared(doc: &Value, msg: &str) -> (String, BTreeSet<String>) {
    let g = &doc["components"]["messages"][msg]["x-graph"];
    let status = g["status"].as_str().unwrap_or("<none>").to_string();
    let rels = g["edges"]
        .as_array()
        .map(|es| {
            es.iter()
                .filter_map(|e| e["rel"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    (status, rels)
}

fn scratch(tag: &str) -> (ArcResolver, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("arc-icd-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = GraphConfig { vector_dim: 64, index_text: true, index_facts: true };
    let kg = TemporalGraph::open(&dir.join("g.lode"), cfg, Some(Box::new(HashEmbedder(64))))
        .expect("open scratch graph");
    let r = ArcResolver::new(kg);
    (r, dir)
}

/// Drive the projector with one snapshot and return every relation it wrote.
fn relations_from(tag: &str, objects: &[ObjectSnapshot]) -> BTreeSet<String> {
    let (mut r, dir) = scratch(tag);
    r.project_objects(objects, 2_000_000_000_000)
        .expect("projection must succeed");
    // Every fact incident to the object, in either direction — the projector's
    // canonical form flips some edges (a `child` half is recorded here but
    // minted pointing the other way), so an Out-only read would miss them.
    let mut out = BTreeSet::new();
    for o in objects {
        for f in r
            .graph()
            .neighbors(&o.id, Direction::Both, None, AsOf::Now)
            .expect("the graph must answer for a projected object")
        {
            out.insert(f.relation.clone());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn group(id: &str, affs: Vec<AffiliationSnap>) -> ObjectSnapshot {
    ObjectSnapshot {
        id: id.repeat(32),
        kind: "group".into(),
        name: "Fixture".into(),
        shape: Some("organisation".into()),
        members: vec![],
        affiliations: affs,
        venue: None,
        at: Some(1_690_000_000_000),
    }
}

fn aff(peer: &str, rel: &str) -> AffiliationSnap {
    AffiliationSnap {
        peer: peer.repeat(32),
        rel: rel.into(),
        name: "Peer".into(),
        tether: String::new(),
        at: 1_700_000_000_000,
    }
}

// ---------------------------------------------------------------------------

/// A relation the projector can write must be one the document names. This is
/// the cheap half — no projection is run — and it is the half that catches a
/// constant added to `project.rs` in passing.
#[test]
fn every_projector_relation_is_in_the_vocabulary() {
    let d = doc();
    let vocab: BTreeSet<&str> = d["info"]["x-vocabulary"]["relations"]
        .as_array()
        .expect("x-vocabulary.relations is an array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();

    let missing: Vec<&str> = PROJECTOR_RELATIONS
        .iter()
        .copied()
        .filter(|r| !vocab.contains(r))
        .collect();

    assert!(
        missing.is_empty(),
        "the projector mints relations the ICD's vocabulary does not name: {missing:?}\n\
         add them to info.x-vocabulary.relations, or stop minting them"
    );
}

/// `info.x-vocabulary.implemented` must be exactly the set the projector can
/// write — no more (a promise the code does not keep) and no less (a relation
/// shipped without being declared implemented).
#[test]
fn implemented_vocabulary_matches_the_projector() {
    let d = doc();
    let claimed: BTreeSet<String> = d["info"]["x-vocabulary"]["implemented"]
        .as_array()
        .expect("x-vocabulary.implemented is an array")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let real: BTreeSet<String> = PROJECTOR_RELATIONS.iter().map(|s| s.to_string()).collect();

    assert_eq!(
        claimed, real,
        "\nx-vocabulary.implemented disagrees with the projector.\n\
           claimed but not minted: {:?}\n\
           minted but not claimed: {:?}",
        claimed.difference(&real).collect::<Vec<_>>(),
        real.difference(&claimed).collect::<Vec<_>>()
    );
}

/// A message filed as `implemented` may only declare relations the projector is
/// actually able to mint. This catches the other direction of drift from the
/// behavioural test below: an edge promoted to implemented on a relation that
/// has no code path at all — where there is nothing to drive, so nothing to
/// observe, and a purely behavioural check would stay silent.
#[test]
fn implemented_messages_only_declare_mintable_relations() {
    let d = doc();
    let can: BTreeSet<&str> = PROJECTOR_RELATIONS.iter().copied().collect();
    let mut bad: Vec<String> = Vec::new();

    for (name, msg) in d["components"]["messages"]
        .as_object()
        .expect("components.messages is a map")
    {
        let g = &msg["x-graph"];
        if g["status"].as_str() != Some("implemented") {
            continue;
        }
        for e in g["edges"].as_array().into_iter().flatten() {
            let rel = e["rel"].as_str().unwrap_or("<none>");
            if !can.contains(rel) {
                bad.push(format!("  {name} declares `{rel}`, which no REL_* constant mints"));
            }
        }
    }

    assert!(
        bad.is_empty(),
        "\nmessages filed as implemented declare relations the projector cannot write:\n{}\n\
         drop them to \"specified\" until the projection exists.\n",
        bad.join("\n")
    );
}

/// THE ratchet. For every implemented message that declares edges, run the op's
/// folded residue through the projector and compare what came out with what the
/// document promised.
#[test]
fn implemented_x_graph_matches_what_the_projector_writes() {
    let d = doc();
    let mut failures: Vec<String> = Vec::new();

    // group.setAffiliation — one message, but the projector's relation depends
    // on the affiliation's `rel`, so the fixture exercises every arm of it.
    // Whatever the document declares has to cover all of them.
    let observed = relations_from(
        "aff",
        &[group(
            "aa",
            vec![
                aff("bb", "parent"),
                aff("cc", "child"),
                aff("dd", "peer"),
                aff("ee", "created"),
            ],
        )],
    );
    check(&d, "group.setAffiliation", &observed, &mut failures);

    // event.setVenue — the Event's happens_at edge.
    let venue_obj = ObjectSnapshot {
        id: "11".repeat(32),
        kind: "event".into(),
        name: "Fixture Event".into(),
        shape: None,
        members: vec![],
        affiliations: vec![],
        venue: Some(VenueSnap {
            place: "22".repeat(32),
            name: "Fixture Place".into(),
            at: 1_700_000_000_000,
        }),
        at: Some(1_690_000_000_000),
    };
    let observed = relations_from("venue", &[venue_obj]);
    check(&d, "event.setVenue", &observed, &mut failures);

    assert!(
        failures.is_empty(),
        "\nthe ICD's x-graph disagrees with the projector:\n\n{}\n\
         Either correct the message's x-graph to describe what the projector does,\n\
         or drop its status back to \"specified\" until the projection exists.\n",
        failures.join("\n\n")
    );
}

fn check(d: &Value, msg: &str, observed: &BTreeSet<String>, failures: &mut Vec<String>) {
    let (status, declared) = declared(d, msg);
    if status != "implemented" {
        failures.push(format!(
            "  {msg}\n    status is \"{status}\", but the projector writes {observed:?}\n\
             \x20   — a projection that exists should not be filed as unbuilt"
        ));
        return;
    }
    if &declared != observed {
        failures.push(format!(
            "  {msg}\n    declared: {declared:?}\n    minted:   {observed:?}\n\
             \x20   missing from the ICD: {:?}",
            observed.difference(&declared).collect::<Vec<_>>()
        ));
    }
}
