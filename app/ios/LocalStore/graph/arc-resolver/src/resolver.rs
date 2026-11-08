//! `ArcResolver` — the one handle over the on-device bi-temporal graph.
//!
//! It owns the `lodedb-graph` engine, which indexes and searches with the platform's embedder.
//! Every write is the deterministic object projection or its promotion step (`project.rs`), and
//! every read is a LIFE-tab query the FFI answers from the engine. No language model is involved
//! and nothing here touches the network. The name predates the removal of the text-extraction
//! pipeline; it stays because it is baked into the FFI symbols and the Swift package.

use std::path::Path;
use std::sync::Arc;

use lodedb_graph::{Embedder, GraphConfig, GraphError, TemporalGraph};

use crate::embedder::SharedEmbedder;
use crate::types::EpisodeProvenance;

/// The on-device graph handle.
pub struct ArcResolver {
    kg: TemporalGraph,
}

impl ArcResolver {
    /// Wrap an already-open graph.
    pub fn new(kg: TemporalGraph) -> Self {
        ArcResolver { kg }
    }

    /// Open the graph at `path`, injecting `embedder` for on-write indexing and semantic reads.
    pub fn open(
        path: impl AsRef<Path>,
        config: GraphConfig,
        embedder: Option<Arc<dyn Embedder>>,
    ) -> Result<Self, GraphError> {
        let engine_embedder: Option<Box<dyn Embedder>> =
            embedder.map(|e| Box::new(SharedEmbedder(e)) as Box<dyn Embedder>);
        let kg = TemporalGraph::open(path.as_ref(), config, engine_embedder)?;
        Ok(Self::new(kg))
    }

    /// Read-only access to the underlying graph (for LIFE-tab queries).
    pub fn graph(&self) -> &TemporalGraph {
        &self.kg
    }

    /// Mutable graph access for the object projection and promotion (`project.rs`) — the
    /// crate's only writers, which share this one handle because the index takes a writer lock.
    pub(crate) fn graph_mut(&mut self) -> &mut TemporalGraph {
        &mut self.kg
    }

    /// Provenance, one episode per row: `(episode_id, source, occurred_at, entity_ids)`.
    ///
    /// This is the entity -> source-object join that makes graph-ranked note search possible.
    /// `episode_id` is the caller-stable id the episode was written under (a note's UUID), so
    /// the reverse map `entity -> [note]` is built by the caller in one pass with no id
    /// translation. Nothing in this crate writes new episodes: the rows are the ones a graph
    /// already holds from before text extraction was removed, and promotion keeps them attached
    /// to the object node. Entities are the endpoints of the facts each episode produced,
    /// deduped, in first-seen order; `fact_episodes` is the underlying provenance table.
    ///
    /// Cost is one query per episode, so this is a build-an-index-once call, not a per-
    /// keystroke one. Callers cache it and rebuild when the graph epoch moves.
    pub fn episode_index(&self) -> Result<Vec<EpisodeProvenance>, GraphError> {
        let mut out = Vec::new();
        for ep in self.kg.episodes()? {
            let mut entities: Vec<String> = Vec::new();
            for fact in self.kg.facts_by_episode(&ep.id)? {
                for endpoint in [fact.src, fact.dst] {
                    if !entities.contains(&endpoint) {
                        entities.push(endpoint);
                    }
                }
            }
            out.push(EpisodeProvenance {
                episode_id: ep.id,
                source: ep.source,
                occurred_at: ep.occurred_at,
                entities,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    use lodedb_graph::GraphConfig;
    use serde_json::{json, Value};

    /// A resolver over a scratch graph with no embedder: `episode_index` is pure graph work,
    /// so the fixtures write their vectors directly.
    fn scratch(dir: &std::path::Path) -> ArcResolver {
        let kg = TemporalGraph::open(&dir.join("g.lode"), GraphConfig::default(), None)
            .expect("open scratch graph");
        ArcResolver::new(kg)
    }

    /// The whole point of the caller-stable id: the episode a note produced is keyed BY the
    /// note, so `episode_index` yields note ids directly and a repeated write is not a second row.
    #[test]
    fn episode_index_maps_entities_back_to_the_note_that_produced_them() {
        let dir = std::env::temp_dir().join(format!("arc-prov-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = scratch(&dir);
        let note_id = "6E5B0E64-0000-0000-0000-00000000AAAA";

        // Stand in for a graph that already holds a note's episode: an episode keyed by the
        // note, and a fact between two entities carrying that episode as provenance.
        r.kg.add_episode_with_id(Some(note_id), "note", "took the jacket to Marco", 1_000,
                                 Value::Null, &[]).unwrap();
        // Vector-in: the scratch graph has no embedder, and the engine refuses the text-in
        // verbs rather than inventing a vector. The values are irrelevant to provenance.
        let v = vec![0.1_f32; GraphConfig::default().vector_dim];
        r.kg.upsert_entity_vec("marco", "Person", "Marco", json!({"name": "Marco"}), &v, None, None).unwrap();
        r.kg.upsert_entity_vec("jacket", "Thing", "jacket", json!({"name": "jacket"}), &v, None, None).unwrap();
        r.kg.add_fact_vec("jacket", "TAILORED_BY", "marco", "the jacket is tailored by Marco",
                          Value::Null, vec![note_id.to_string()], Some(1_000), &[], &v).unwrap();

        let index = r.episode_index().unwrap();
        let row = index.iter().find(|e| e.episode_id == note_id).expect("episode keyed by note id");
        assert_eq!(row.source, "note");
        assert!(row.entities.contains(&"marco".to_string()), "entities: {:?}", row.entities);
        assert!(row.entities.contains(&"jacket".to_string()), "entities: {:?}", row.entities);

        // Writing the same note's episode again must not mint a second row — the id is the
        // join key, and a duplicate would split one note's provenance across two episodes.
        r.kg.add_episode_with_id(Some(note_id), "note", "took the jacket to Marco", 1_000,
                                 Value::Null, &[]).unwrap();
        let again = r.episode_index().unwrap();
        assert_eq!(again.iter().filter(|e| e.episode_id == note_id).count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
