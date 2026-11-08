//! project — the ratified-delta projection: minted GroupObjects -> the LIFE graph.
//!
//! This is the write path into the on-device graph. `project_objects` folds the app's
//! own GroupObjects (groups/projects/things/places/events, their rosters, their
//! Group↔Group AFFILIATIONS, and an Event's VENUE reference) into entities and
//! facts DETERMINISTICALLY — no model, no episodes, no guessing. The per-object delta logs remain the source of truth;
//! the graph is the derived read-index, and this is the fold that derives it.
//!
//! Reconciliation contract (idempotent; the caller re-runs it after every sync):
//!   - an entity per object, id = the object's stable id (a group id hex, a
//!     member's space1<hex>) — NEVER a name slug, so projected nodes and the
//!     slug-keyed nodes older builds lifted from text live in disjoint id spaces;
//!   - a fact per relationship, identified by (src, relation, dst) among the LIVE
//!     facts stamped `properties.source == "object"`;
//!   - a projected fact that stops being desired (member left, affiliation
//!     cleared) is INVALIDATED at `now_ms`, never deleted — the bi-temporal
//!     history ("which chapters were federated last spring") is the point;
//!   - entities and facts without the `source:"object"` stamp (text-lifted by
//!     older builds, still present in existing graphs) are never touched, and a
//!     projected entity keeps any summary it already carries.
//!
//! Federation normalization: each group's log records its own HALF of an edge
//! (rel = parent | child | peer). Both halves project to ONE canonical fact —
//!   parent : (self)  -[member_of]->        (peer)
//!   child  : (peer)  -[member_of]->        (self)
//!   peer   : (lo)    -[affiliated_with]->  (hi)     (lexicographic endpoints)
//! — so when both sides of a federation fold on one device, the second half
//! reconciles as "already present" instead of minting a mirror edge.

use std::collections::BTreeMap;

use lodedb_graph::{AsOf, Direction, GraphError};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::resolver::ArcResolver;

/// The relation a projected roster/federation/venue/authorship fact carries.
pub const REL_MEMBER_OF: &str = "member_of";
pub const REL_AFFILIATED_WITH: &str = "affiliated_with";
pub const REL_HAPPENS_AT: &str = "happens_at";
pub const REL_CREATED: &str = "created";
/// The provenance stamp distinguishing projected truth from text-lifted ghosts.
const SOURCE_OBJECT: &str = "object";

/// One minted GroupObject, as the app's core sees it after a fold.
#[derive(Debug, Clone, Deserialize)]
pub struct ObjectSnapshot {
    /// The object's stable id (group id hex) — becomes the entity id.
    pub id: String,
    /// The directory kind string: "group" | "project" | "thing" | "place" | ...
    pub kind: String,
    /// Folded display name.
    pub name: String,
    /// Group shape ("individual" | "team" | "organisation"), groups only.
    #[serde(default)]
    pub shape: Option<String>,
    /// MLS roster space ids (space1<hex>) to project as member_of facts.
    #[serde(default)]
    pub members: Vec<MemberRef>,
    /// This group's half of each Group↔Group edge (from group.setAffiliation).
    #[serde(default)]
    pub affiliations: Vec<AffiliationSnap>,
    /// The Event's venue reference (from event.setVenue) — the happens_at edge.
    #[serde(default)]
    pub venue: Option<VenueSnap>,
    /// When this object became true (mint time, unix ms) — the entity's valid_at.
    #[serde(default)]
    pub at: Option<i64>,
}

/// Mirror of the folded `event.setVenue` record: which Place the Event happens at.
#[derive(Debug, Clone, Deserialize)]
pub struct VenueSnap {
    /// The Place's object id (hex).
    pub place: String,
    /// The Place's display name as known at authoring.
    #[serde(default)]
    pub name: String,
    /// Event time (unix ms) the edge became true — the fact's valid_at.
    #[serde(default)]
    pub at: i64,
}

/// A roster member: the space id is the identity, the name is a label courtesy.
#[derive(Debug, Clone, Deserialize)]
pub struct MemberRef {
    pub space: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// Mirror of the folded `group.setAffiliation` record.
#[derive(Debug, Clone, Deserialize)]
pub struct AffiliationSnap {
    /// The peer group's object id (hex).
    pub peer: String,
    /// "parent" | "child" | "peer" — how the peer relates to the recording group.
    pub rel: String,
    /// The peer's display name as known at authoring.
    #[serde(default)]
    pub name: String,
    /// The group-tether object id carrying the bilateral channel ("" when none).
    #[serde(default)]
    pub tether: String,
    /// Event time (unix ms) the affiliation became effective — the fact's valid_at.
    #[serde(default)]
    pub at: i64,
}

/// What one reconcile run did — every count is work actually performed, so a
/// steady-state run reports zeros everywhere except `unchanged`.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ProjectStats {
    pub entities_written: usize,
    pub entities_unchanged: usize,
    pub facts_added: usize,
    pub facts_unchanged: usize,
    pub facts_invalidated: usize,
    /// Objects whose kind has no LIFE projection (forums, connections, tethers).
    pub skipped_objects: usize,
}

/// The LIFE entity type for a directory kind. `None` = the kind has no LIFE node
/// (chat forums, connections, tethers — they carry conversation, not LIFE entities).
/// An individual-shaped group IS a person (a Group is a "who"; a group-of-one's
/// "who" is the person), so it lands in PEOPLE, not GROUPS.
fn entity_type_for(kind: &str, shape: Option<&str>) -> Option<&'static str> {
    match kind {
        "group" => Some(match shape {
            Some("individual") | None => "Person",
            _ => "Group",
        }),
        "project" => Some("Project"),
        "thing" => Some("Thing"),
        "place" => Some("Place"),
        "event" => Some("Event"),
        _ => None,
    }
}

/// One desired fact, canonical form.
#[derive(Debug, Clone, PartialEq)]
struct DesiredFact {
    src: String,
    relation: &'static str,
    dst: String,
    text: String,
    valid_at: Option<i64>,
    tether: String,
}

impl DesiredFact {
    fn key(&self) -> (String, String, String) {
        (self.src.clone(), self.relation.to_string(), self.dst.clone())
    }
}

/// A short human label for a bare space id (space1<hex> is 71 chars of noise).
fn space_label(space: &str) -> String {
    let tail = space.strip_prefix("space1").unwrap_or(space);
    format!("space·{}", &tail[..tail.len().min(8)])
}

fn is_projected(props: &Value) -> bool {
    props.get("source").and_then(Value::as_str) == Some(SOURCE_OBJECT)
}

impl ArcResolver {
    /// Reconcile the graph against a full snapshot of the app's minted objects.
    ///
    /// `objects` is the complete list (this is a sync-to-target, not an append):
    /// a projected fact whose relationship is absent from the snapshot is
    /// invalidated at `now_ms`. Objects only referenced by others (an affiliation
    /// peer whose group we don't hold locally) get a stub entity that a later
    /// snapshot upgrades in place.
    pub fn project_objects(
        &mut self,
        objects: &[ObjectSnapshot],
        now_ms: i64,
    ) -> Result<ProjectStats, GraphError> {
        let mut stats = ProjectStats::default();
        let kg = self.graph_mut();

        // ---- pass 1: desired entities (objects, members, affiliation stubs) ----
        // BTreeMap so the write order is deterministic run-to-run.
        // (id -> (type, label, extra props, valid_at, is_stub))
        let mut desired: BTreeMap<String, (String, String, Value, Option<i64>, bool)> =
            BTreeMap::new();
        for o in objects {
            let Some(etype) = entity_type_for(&o.kind, o.shape.as_deref()) else {
                stats.skipped_objects += 1;
                continue;
            };
            let mut props = json!({ "name": o.name, "kind": o.kind, "source": SOURCE_OBJECT });
            if let Some(shape) = &o.shape {
                props["shape"] = json!(shape);
            }
            desired.insert(o.id.clone(), (etype.to_string(), o.name.clone(), props, o.at, false));

            for m in &o.members {
                let label = m.name.clone().unwrap_or_else(|| space_label(&m.space));
                let props = json!({ "name": label, "space": m.space, "source": SOURCE_OBJECT });
                // A member row never downgrades a full object entity.
                desired
                    .entry(m.space.clone())
                    .or_insert(("Person".to_string(), label, props, None, false));
            }
            for a in &o.affiliations {
                let label = if a.name.is_empty() { a.peer.clone() } else { a.name.clone() };
                let props = json!({ "name": label, "kind": "group", "source": SOURCE_OBJECT, "stub": true });
                // A stub never overwrites the peer's own snapshot row (stub=true tail).
                desired
                    .entry(a.peer.clone())
                    .or_insert(("Group".to_string(), label, props, None, true));
            }
            if let Some(v) = &o.venue {
                // A venue we don't hold locally (someone else's shared Place) still
                // needs a node for the happens_at edge to land on.
                let label = if v.name.is_empty() { v.place.clone() } else { v.name.clone() };
                let props = json!({ "name": label, "kind": "place", "source": SOURCE_OBJECT, "stub": true });
                desired
                    .entry(v.place.clone())
                    .or_insert(("Place".to_string(), label, props, None, true));
            }
        }

        // ---- pass 2: write entities (merge-preserving, embed only on change) ----
        for (id, (etype, name, props, valid_at, is_stub)) in &desired {
            let existing = kg.get_entity(id)?;
            match existing {
                None => {
                    kg.upsert_entity(id, etype, name, props.clone(), *valid_at, None)?;
                    stats.entities_written += 1;
                }
                Some(e) => {
                    // A stub only ever fills a hole; never touch a real node with it.
                    if *is_stub {
                        stats.entities_unchanged += 1;
                        continue;
                    }
                    // Merge OUR keys over the stored properties; keep every key we did
                    // not write (a summary a promoted ghost carried in, etc.). Label keeps
                    // the "{name}. {summary}" convention when a summary exists.
                    let mut merged = e.properties.as_object().cloned().unwrap_or_default();
                    for (k, v) in props.as_object().into_iter().flatten() {
                        merged.insert(k.clone(), v.clone());
                    }
                    merged.remove("stub");
                    let label = match merged.get("summary").and_then(Value::as_str) {
                        Some(s) if !s.is_empty() => format!("{name}. {s}"),
                        _ => name.clone(),
                    };
                    let merged = Value::Object(merged);
                    if e.entity_type == *etype && e.label == label && e.properties == merged {
                        stats.entities_unchanged += 1;
                        continue; // nothing moved: skip the write AND the re-embed
                    }
                    // Preserve the stored validity window unless the snapshot knows better.
                    let valid_at = e.valid_at.or(*valid_at);
                    kg.upsert_entity(id, etype, &label, merged, valid_at, e.invalid_at)?;
                    stats.entities_written += 1;
                }
            }
        }

        // ---- pass 3: desired facts (roster membership + federation edges) ----
        let name_of = |id: &str, desired: &BTreeMap<String, (String, String, Value, Option<i64>, bool)>| {
            desired.get(id).map(|(_, n, ..)| n.clone()).unwrap_or_else(|| id.to_string())
        };
        let mut facts: Vec<DesiredFact> = Vec::new();
        for o in objects {
            if entity_type_for(&o.kind, o.shape.as_deref()).is_none() {
                continue;
            }
            for m in &o.members {
                facts.push(DesiredFact {
                    src: m.space.clone(),
                    relation: REL_MEMBER_OF,
                    dst: o.id.clone(),
                    text: format!("{} is a member of {}", name_of(&m.space, &desired), o.name),
                    valid_at: None, // the roster carries no join time; open start is honest
                    tether: String::new(),
                });
            }
            if let Some(v) = &o.venue {
                facts.push(DesiredFact {
                    src: o.id.clone(),
                    relation: REL_HAPPENS_AT,
                    dst: v.place.clone(),
                    text: format!("{} happens at {}", o.name, name_of(&v.place, &desired)),
                    valid_at: (v.at > 0).then_some(v.at),
                    tether: String::new(),
                });
            }
            for a in &o.affiliations {
                let peer_name = name_of(&a.peer, &desired);
                let f = match a.rel.as_str() {
                    "parent" => DesiredFact {
                        src: o.id.clone(),
                        relation: REL_MEMBER_OF,
                        dst: a.peer.clone(),
                        text: format!("{} is a member group of {}", o.name, peer_name),
                        valid_at: Some(a.at),
                        tether: a.tether.clone(),
                    },
                    "child" => DesiredFact {
                        src: a.peer.clone(),
                        relation: REL_MEMBER_OF,
                        dst: o.id.clone(),
                        text: format!("{} is a member group of {}", peer_name, o.name),
                        valid_at: Some(a.at),
                        tether: a.tether.clone(),
                    },
                    "peer" => {
                        let (lo, hi, lo_n, hi_n) = if o.id <= a.peer {
                            (o.id.clone(), a.peer.clone(), o.name.clone(), peer_name)
                        } else {
                            (a.peer.clone(), o.id.clone(), peer_name, o.name.clone())
                        };
                        DesiredFact {
                            src: lo,
                            relation: REL_AFFILIATED_WITH,
                            dst: hi,
                            text: format!("{lo_n} is affiliated with {hi_n}"),
                            valid_at: Some(a.at),
                            tether: a.tether.clone(),
                        }
                    }
                    // Authorship: the group published this Event/listing as its own.
                    "created" => DesiredFact {
                        src: o.id.clone(),
                        relation: REL_CREATED,
                        dst: a.peer.clone(),
                        text: format!("{} created {}", o.name, peer_name),
                        valid_at: Some(a.at),
                        tether: a.tether.clone(),
                    },
                    other => {
                        log::warn!("unknown affiliation rel {other:?} on {} -> {}; skipped", o.id, a.peer);
                        continue;
                    }
                };
                facts.push(f);
            }
        }
        // Both halves of one federation edge normalize to the same key: fold them.
        let mut by_key: BTreeMap<(String, String, String), DesiredFact> = BTreeMap::new();
        for f in facts {
            by_key.entry(f.key()).or_insert(f);
        }

        // ---- pass 4: reconcile facts per desired-entity neighborhood ----
        // Live projected facts among the desired entities' neighborhoods, keyed the
        // same way, so present-but-undesired ones can be invalidated.
        let mut live: BTreeMap<(String, String, String), String> = BTreeMap::new(); // key -> fact id
        for id in desired.keys() {
            for f in kg.neighbors(id, Direction::Both, None, AsOf::Now)? {
                if is_projected(&f.properties) {
                    live.insert((f.src.clone(), f.relation.clone(), f.dst.clone()), f.id.clone());
                }
            }
        }

        for (key, f) in &by_key {
            if live.contains_key(key) {
                stats.facts_unchanged += 1;
                continue;
            }
            let mut props = json!({ "source": SOURCE_OBJECT });
            if !f.tether.is_empty() {
                props["tether"] = json!(f.tether);
            }
            kg.add_fact(&f.src, f.relation, &f.dst, &f.text, props, Vec::new(), f.valid_at, &[])?;
            stats.facts_added += 1;
        }
        for (key, fact_id) in &live {
            if !by_key.contains_key(key) {
                if kg.invalidate_fact(fact_id, Some(now_ms))? {
                    stats.facts_invalidated += 1;
                }
            }
        }

        Ok(stats)
    }
}

impl ArcResolver {
    /// PROMOTION: a text-lifted ghost (a node an older build extracted from a note, which
    /// existing graphs still hold) becomes a minted GroupObject, and the graph
    /// follows. The ghost's node (slug id) is replaced by the object's node (stable
    /// object id): every fact is re-pointed onto the object — same relation, text,
    /// properties, EPISODES (so note-provenance survives), and validity window —
    /// then the ghost and its old facts are removed. The object node keeps the
    /// ghost's summary and gains the `source:"object"` stamp; the next
    /// `project_objects` run reconciles it like any minted object.
    ///
    /// Returns the number of facts moved. Idempotent-ish: a second call finds no
    /// ghost and moves nothing.
    pub fn rehome_entity(
        &mut self,
        ghost_id: &str,
        object_id: &str,
        object_type: &str,
        name: &str,
    ) -> Result<usize, GraphError> {
        let kg = self.graph_mut();
        let Some(ghost) = kg.get_entity(ghost_id)? else {
            return Ok(0); // already rehomed (or never existed): nothing to move
        };

        // The object node: ghost properties (summary!) under ours, object stamp on top.
        let mut props = ghost.properties.as_object().cloned().unwrap_or_default();
        props.insert("name".into(), json!(name));
        props.insert("source".into(), json!(SOURCE_OBJECT));
        let label = match props.get("summary").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => format!("{name}. {s}"),
            _ => name.to_string(),
        };
        kg.upsert_entity(object_id, object_type, &label, Value::Object(props), ghost.valid_at, None)?;

        // Re-point every non-superseded fact. Expired facts are corrections' leftovers;
        // their successors are in this same set, so history is not thinned by skipping them.
        let mut moved = 0usize;
        for f in kg.neighbors(ghost_id, Direction::Both, None, AsOf::All)? {
            if f.expired_at.is_some() {
                continue;
            }
            let src = if f.src == ghost_id { object_id } else { &f.src };
            let dst = if f.dst == ghost_id { object_id } else { &f.dst };
            if src == dst {
                continue; // a ghost↔object self-loop says nothing
            }
            let new_id =
                kg.add_fact(src, &f.relation, dst, &f.fact, f.properties.clone(), f.episodes.clone(), f.valid_at, &[])?;
            if let Some(iv) = f.invalid_at {
                kg.invalidate_fact(&new_id, Some(iv))?;
            }
            moved += 1;
        }

        // Drop the ghost — cascades its OLD facts out of topology + index. The copies
        // above carry the provenance forward under the object's id.
        kg.remove_entity(ghost_id)?;
        Ok(moved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lodedb_graph::{Embedder, EmbedRole, GraphConfig, TemporalGraph};

    /// A deterministic no-model embedder: hash chars into a fixed-dim vector. The
    /// projector's writes must embed (the engine indexes labels + fact text), and
    /// tests must not fake that away — they just don't need a real model to do it.
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

    fn scratch(dir: &std::path::Path) -> ArcResolver {
        let cfg = GraphConfig { vector_dim: 64, index_text: true, index_facts: true };
        let kg = TemporalGraph::open(&dir.join("g.lode"), cfg, Some(Box::new(HashEmbedder(64))))
            .expect("open scratch graph");
        ArcResolver::new(kg)
    }

    fn chapter_and_federation() -> Vec<ObjectSnapshot> {
        vec![
            ObjectSnapshot {
                id: "aa".repeat(32),
                kind: "group".into(),
                name: "Berkeley Students for Space".into(),
                shape: Some("team".into()),
                members: vec![
                    MemberRef { space: format!("space1{}", "11".repeat(32)), name: Some("Ada".into()) },
                    MemberRef { space: format!("space1{}", "22".repeat(32)), name: None },
                ],
                affiliations: vec![AffiliationSnap {
                    peer: "bb".repeat(32),
                    rel: "parent".into(),
                    name: "National Students for Space".into(),
                    tether: "cc".repeat(32),
                    at: 1_700_000_000_000,
                }],
                venue: None,
                at: Some(1_690_000_000_000),
            },
            ObjectSnapshot {
                id: "bb".repeat(32),
                kind: "group".into(),
                name: "National Students for Space".into(),
                shape: Some("organisation".into()),
                members: vec![],
                // The federation's own half of the SAME edge — must NOT double-write.
                affiliations: vec![AffiliationSnap {
                    peer: "aa".repeat(32),
                    rel: "child".into(),
                    name: "Berkeley Students for Space".into(),
                    tether: "cc".repeat(32),
                    at: 1_700_000_000_000,
                }],
                venue: None,
                at: None,
            },
        ]
    }

    /// The Berkeley shape end-to-end: chapter + federation project to two Group
    /// nodes, two Person members, roster facts, and ONE canonical member_of edge
    /// between the groups even though both halves were recorded.
    #[test]
    fn projects_federation_as_one_canonical_edge() {
        let dir = std::env::temp_dir().join(format!("arc-proj-fed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = scratch(&dir);
        let objs = chapter_and_federation();

        let stats = r.project_objects(&objs, 2_000_000_000_000).unwrap();
        assert_eq!(stats.entities_written, 4, "chapter + federation + 2 members");
        assert_eq!(stats.facts_added, 3, "2 roster edges + ONE federation edge");
        assert_eq!(stats.facts_invalidated, 0);

        let chapter = r.graph().get_entity(&"aa".repeat(32)).unwrap().unwrap();
        assert_eq!(chapter.entity_type, "Group");
        assert_eq!(chapter.properties["source"], "object");
        assert_eq!(chapter.valid_at, Some(1_690_000_000_000));

        // The federation edge: exactly one, canonical direction, tether stamped,
        // valid from the authored event time.
        let up = r
            .graph()
            .neighbors(&"aa".repeat(32), Direction::Out, Some(REL_MEMBER_OF), AsOf::Now)
            .unwrap();
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].dst, "bb".repeat(32));
        assert_eq!(up[0].valid_at, Some(1_700_000_000_000));
        assert_eq!(up[0].properties["tether"], "cc".repeat(32));

        // Members land as Person nodes with member_of facts into the chapter.
        let members = r
            .graph()
            .neighbors(&"aa".repeat(32), Direction::In, Some(REL_MEMBER_OF), AsOf::Now)
            .unwrap();
        assert_eq!(members.len(), 2);

        // Re-running the identical snapshot is a no-op (no re-embeds, no new facts).
        let again = r.project_objects(&objs, 2_000_000_000_001).unwrap();
        assert_eq!(again.entities_written, 0, "steady state writes nothing");
        assert_eq!(again.facts_added, 0);
        assert_eq!(again.entities_unchanged, 4);
        assert_eq!(again.facts_unchanged, 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Leaving the federation: the chapter's half disappears from the snapshot and
    /// the edge is INVALIDATED at now — history keeps the interval, Now() drops it.
    #[test]
    fn a_cleared_affiliation_is_invalidated_not_deleted() {
        let dir = std::env::temp_dir().join(format!("arc-proj-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = scratch(&dir);
        let mut objs = chapter_and_federation();
        r.project_objects(&objs, 1_800_000_000_000).unwrap();

        // Both sides clear the edge (group.clearAffiliation folded on each log).
        objs[0].affiliations.clear();
        objs[1].affiliations.clear();
        let stats = r.project_objects(&objs, 1_900_000_000_000).unwrap();
        assert_eq!(stats.facts_invalidated, 1, "one canonical edge, one invalidation");

        let now = r
            .graph()
            .neighbors(&"aa".repeat(32), Direction::Out, Some(REL_MEMBER_OF), AsOf::Now)
            .unwrap();
        assert!(now.is_empty(), "the edge is gone from Now");

        // ...but the interval survives under AsOf::At (bi-temporal, the point).
        let then = r
            .graph()
            .neighbors(
                &"aa".repeat(32),
                Direction::Out,
                Some(REL_MEMBER_OF),
                AsOf::At(1_850_000_000_000),
            )
            .unwrap();
        assert_eq!(then.len(), 1, "history answers 'were we federated then?'");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Roster churn: a member leaving invalidates exactly their member_of fact;
    /// an unstamped fact on the same entity (the kind older builds lifted from text)
    /// is never touched.
    #[test]
    fn roster_and_unstamped_facts_reconcile_independently() {
        let dir = std::env::temp_dir().join(format!("arc-proj-roster-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = scratch(&dir);
        let mut objs = chapter_and_federation();
        r.project_objects(&objs, 1_800_000_000_000).unwrap();

        // An unstamped fact about the chapter, as older builds wrote them from text.
        r.graph_mut()
            .add_fact(
                &format!("space1{}", "11".repeat(32)),
                "LEADS",
                &"aa".repeat(32),
                "Ada leads Berkeley Students for Space",
                Value::Null,
                Vec::new(),
                Some(1_810_000_000_000),
                &[],
            )
            .unwrap();

        objs[0].members.remove(1); // the second member leaves
        let stats = r.project_objects(&objs, 1_900_000_000_000).unwrap();
        assert_eq!(stats.facts_invalidated, 1, "exactly the departed member's edge");

        let living = r
            .graph()
            .neighbors(&"aa".repeat(32), Direction::Both, None, AsOf::Now)
            .unwrap();
        assert!(
            living.iter().any(|f| f.relation == "LEADS"),
            "an unstamped fact is not the projector's to reconcile"
        );
        assert_eq!(
            living.iter().filter(|f| f.relation == REL_MEMBER_OF && f.dst == "aa".repeat(32)).count(),
            1,
            "one member remains"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Promotion: minting a GroupObject from a text-lifted ghost re-points the
    /// ghost's facts (and their episode provenance) onto the object's node, keeps
    /// the summary, and removes the ghost — one entity, one id, history intact.
    #[test]
    fn rehome_moves_facts_and_provenance_onto_the_object_node() {
        let dir = std::env::temp_dir().join(format!("arc-rehome-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = scratch(&dir);

        // A graph an older build wrote: a note episode produced a ghost "toyota" with a fact to Marco.
        r.graph_mut()
            .add_episode_with_id(Some("note-1"), "note", "Marco fixed the Toyota", 1_000, Value::Null, &[])
            .unwrap();
        r.graph_mut()
            .upsert_entity("toyota", "Thing", "Toyota. An old grey car.",
                           json!({"name": "Toyota", "summary": "An old grey car."}), None, None)
            .unwrap();
        r.graph_mut()
            .upsert_entity("marco", "Person", "Marco", json!({"name": "Marco"}), None, None)
            .unwrap();
        r.graph_mut()
            .add_fact("marco", "FIXED", "toyota", "Marco fixed the Toyota", Value::Null,
                      vec!["note-1".into()], Some(1_000), &[])
            .unwrap();

        // Accept: a Thing GroupObject is minted; the graph follows.
        let obj = "ab".repeat(32);
        let moved = r.rehome_entity("toyota", &obj, "Thing", "Toyota").unwrap();
        assert_eq!(moved, 1);

        // One node: the object's, stamped + summary kept; the ghost is gone.
        assert!(r.graph().get_entity("toyota").unwrap().is_none());
        let node = r.graph().get_entity(&obj).unwrap().unwrap();
        assert_eq!(node.properties["source"], "object");
        assert_eq!(node.properties["summary"], "An old grey car.");

        // The fact survived re-pointed, and the note still vouches for the OBJECT.
        let facts = r.graph().neighbors(&obj, Direction::In, Some("FIXED"), AsOf::Now).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].src, "marco");
        assert_eq!(facts[0].episodes, vec!["note-1".to_string()]);
        let idx = r.episode_index().unwrap();
        let row = idx.iter().find(|e| e.episode_id == "note-1").unwrap();
        assert!(row.entities.contains(&obj), "provenance follows the object: {:?}", row.entities);

        // A second rehome is a no-op, and a later projection reconciles the same id.
        assert_eq!(r.rehome_entity("toyota", &obj, "Thing", "Toyota").unwrap(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A group's `created` affiliation projects an authorship edge onto the object —
    /// how a group's Events and market listings hang off its node.
    #[test]
    fn a_created_affiliation_projects_authorship() {
        let dir = std::env::temp_dir().join(format!("arc-proj-created-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = scratch(&dir);

        let group_id = "4d".repeat(32);
        let event_id = "5e".repeat(32);
        let objs = vec![
            ObjectSnapshot {
                id: group_id.clone(),
                kind: "group".into(),
                name: "Berkeley Students".into(),
                shape: Some("team".into()),
                members: vec![],
                affiliations: vec![AffiliationSnap {
                    peer: event_id.clone(),
                    rel: "created".into(),
                    name: "Launch party".into(),
                    tether: String::new(),
                    at: 1_750_000_000_000,
                }],
                venue: None,
                at: None,
            },
            ObjectSnapshot {
                id: event_id.clone(),
                kind: "event".into(),
                name: "Launch party".into(),
                shape: None,
                members: vec![],
                affiliations: vec![],
                venue: None,
                at: None,
            },
        ];
        r.project_objects(&objs, 1_760_000_000_000).unwrap();

        let made = r
            .graph()
            .neighbors(&group_id, Direction::Out, Some(REL_CREATED), AsOf::Now)
            .unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].dst, event_id);
        assert_eq!(made[0].fact, "Berkeley Students created Launch party");
        assert_eq!(made[0].valid_at, Some(1_750_000_000_000));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An Event with a venue reference projects an Event node and ONE happens_at
    /// edge onto the Place's node; re-pointing the venue invalidates the old edge
    /// and adds the new one; a venue we don't hold locally lands on a Place stub.
    #[test]
    fn an_event_projects_and_repoints_its_happens_at_edge() {
        let dir = std::env::temp_dir().join(format!("arc-proj-venue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = scratch(&dir);

        let event_id = "1a".repeat(32);
        let place_id = "2b".repeat(32);
        let mut objs = vec![
            ObjectSnapshot {
                id: event_id.clone(),
                kind: "event".into(),
                name: "Launch party".into(),
                shape: None,
                members: vec![],
                affiliations: vec![],
                venue: Some(VenueSnap {
                    place: place_id.clone(),
                    name: "ARC Gangnam".into(),
                    at: 1_750_000_000_000,
                }),
                at: Some(1_740_000_000_000),
            },
            ObjectSnapshot {
                id: place_id.clone(),
                kind: "place".into(),
                name: "ARC Gangnam".into(),
                shape: None,
                members: vec![],
                affiliations: vec![],
                venue: None,
                at: None,
            },
        ];

        let stats = r.project_objects(&objs, 1_760_000_000_000).unwrap();
        assert_eq!(stats.entities_written, 2, "the Event and the Place");
        assert_eq!(stats.facts_added, 1, "one happens_at edge");

        let ev = r.graph().get_entity(&event_id).unwrap().unwrap();
        assert_eq!(ev.entity_type, "Event");
        assert_eq!(ev.valid_at, Some(1_740_000_000_000));
        assert_eq!(r.graph().get_entity(&place_id).unwrap().unwrap().entity_type, "Place");

        let at = r
            .graph()
            .neighbors(&event_id, Direction::Out, Some(REL_HAPPENS_AT), AsOf::Now)
            .unwrap();
        assert_eq!(at.len(), 1);
        assert_eq!(at[0].dst, place_id);
        assert_eq!(at[0].valid_at, Some(1_750_000_000_000));
        assert_eq!(at[0].fact, "Launch party happens at ARC Gangnam");

        // The event moves to a venue we do NOT hold: old edge invalidated, new edge
        // onto a Place stub carrying the courtesy name.
        let other_place = "3c".repeat(32);
        objs[0].venue = Some(VenueSnap {
            place: other_place.clone(),
            name: "Monroe".into(),
            at: 1_770_000_000_000,
        });
        let stats = r.project_objects(&objs, 1_780_000_000_000).unwrap();
        assert_eq!(stats.facts_invalidated, 1);
        assert_eq!(stats.facts_added, 1);

        let at = r
            .graph()
            .neighbors(&event_id, Direction::Out, Some(REL_HAPPENS_AT), AsOf::Now)
            .unwrap();
        assert_eq!(at.len(), 1);
        assert_eq!(at[0].dst, other_place);
        let stub = r.graph().get_entity(&other_place).unwrap().unwrap();
        assert_eq!(stub.entity_type, "Place");
        assert_eq!(stub.properties["stub"], true);
        assert_eq!(stub.label, "Monroe");

        // ...and history still answers "where was it before?" (bi-temporal).
        let then = r
            .graph()
            .neighbors(&event_id, Direction::Out, Some(REL_HAPPENS_AT), AsOf::At(1_760_000_000_500))
            .unwrap();
        assert_eq!(then.len(), 1);
        assert_eq!(then[0].dst, place_id);

        // Clearing the venue invalidates the edge; the entities stay.
        objs[0].venue = None;
        let stats = r.project_objects(&objs, 1_790_000_000_000).unwrap();
        assert_eq!(stats.facts_invalidated, 1);
        assert!(r
            .graph()
            .neighbors(&event_id, Direction::Out, Some(REL_HAPPENS_AT), AsOf::Now)
            .unwrap()
            .is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Kind/shape mapping: an individual-shaped group is a Person; kinds with no
    /// LIFE node are skipped and counted; a stub peer upgrades in place when its
    /// real object arrives, keeping the SAME node id.
    #[test]
    fn shapes_stubs_and_skips_map_honestly() {
        let dir = std::env::temp_dir().join(format!("arc-proj-shape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = scratch(&dir);

        let objs = vec![
            ObjectSnapshot {
                id: "dd".repeat(32),
                kind: "group".into(),
                name: "Marco".into(),
                shape: Some("individual".into()),
                members: vec![],
                affiliations: vec![AffiliationSnap {
                    peer: "ee".repeat(32),
                    rel: "peer".into(),
                    name: "Sister Circle".into(),
                    tether: String::new(),
                    at: 5,
                }],
                venue: None,
                at: None,
            },
            ObjectSnapshot {
                id: "ff".repeat(32),
                kind: "connection".into(), // a chat tether: no LIFE node
                name: "chat".into(),
                shape: None,
                members: vec![],
                affiliations: vec![],
                venue: None,
                at: None,
            },
        ];
        let stats = r.project_objects(&objs, 1_000).unwrap();
        assert_eq!(stats.skipped_objects, 1);
        assert_eq!(r.graph().get_entity(&"dd".repeat(32)).unwrap().unwrap().entity_type, "Person");
        let stub = r.graph().get_entity(&"ee".repeat(32)).unwrap().unwrap();
        assert_eq!(stub.properties["stub"], true);
        assert_eq!(stub.label, "Sister Circle");

        // The peer's real object arrives: same id, stub flag dropped, label real.
        let upgraded = vec![ObjectSnapshot {
            id: "ee".repeat(32),
            kind: "group".into(),
            name: "Sister Circle".into(),
            shape: Some("team".into()),
            members: vec![],
            affiliations: vec![],
            venue: None,
            at: Some(3),
        }];
        r.project_objects(&upgraded, 2_000).unwrap();
        let real = r.graph().get_entity(&"ee".repeat(32)).unwrap().unwrap();
        assert!(real.properties.get("stub").is_none(), "stub tail dropped");
        assert_eq!(real.properties["shape"], "team");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
