// SPDX-License-Identifier: AGPL-3.0-or-later
//! arc-resolver — the on-device, cross-platform bi-temporal graph core.
//!
//! One handle over a [`lodedb_graph`] graph. The app's minted GroupObjects are projected into it
//! deterministically (`project_objects`, with promotion via `rehome_entity`), and the LIFE tab
//! reads it back: entities, neighbours, history, and semantic search over embeddings the device
//! computes itself. There is no language model in this crate and nothing in it calls the
//! network; the one learned component is the embedder the platform injects. Written once in Rust
//! so iOS (Swift) and Android (Kotlin) share it — the platform layer is only UI + config.
//!
//! ```ignore
//! let embedder = OrtEmbedder::load("bge.onnx", "tokenizer.json", 768)?;
//! let mut r = ArcResolver::open("graph.db", cfg, Some(Arc::new(embedder)))?;
//! let stats = r.project_objects(&objects, now_ms)?;
//! let hits = r.graph().semantic_entities(Some("gallery"), None, 10, None, AsOf::Now)?;
//! ```

mod embedder;
pub mod ffi;
mod icd_graph;
mod project;
mod resolver;
mod types;

pub use embedder::{CallbackEmbedder, EmbedCallback, SharedEmbedder};
#[cfg(feature = "ort-embedder")]
pub use embedder::OrtEmbedder;
pub use project::{
    AffiliationSnap, MemberRef, ObjectSnapshot, ProjectStats, REL_AFFILIATED_WITH, REL_MEMBER_OF,
};
pub use resolver::ArcResolver;
pub use types::EpisodeProvenance;

// Re-export the engine surface callers need to open a graph + run LIFE-tab queries.
pub use lodedb_graph::{
    self, AsOf, Direction, Embedder, Entity, Fact, GraphConfig, GraphError, TemporalGraph,
};
