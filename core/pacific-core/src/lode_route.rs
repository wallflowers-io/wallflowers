//! lode_route — the rust-direct (rust->rust) GroupObject route into the union
//! LodeDB store.
//!
//! Decision (see coordination/parallel-workstreams.txt §0/§4): a GroupObject's
//! reduced content lands in LodeDB via a rust-direct append at the delta_log.
//! pacific-core depends on `lodedb-core` directly (no C ABI) and drives
//! `engine::CoreAppender`. A single running `LodeCheckpointer` (Swift, in-app)
//! folds the WAL — this side only appends.
//!
//! WS-G (this file): the config contract, the open path, the §3 metadata builder,
//! and the real append surface are wired here. A reduced content Delta (a
//! `forum.post` text, a project item, a topic finding) is chunked through the
//! appender's own planner (`prepare_documents`), embedded through a
//! [`DeltaEmbedder`] (document-role, BGE/CLS/L2 — parity with LodeDB
//! `EmbeddingMath`), and durably appended through `append_embedded_documents`.
//! The append mechanics are REAL; only the ONNX/ort embedding SESSION is deferred
//! (ort + tokenizers on iOS is a heavy provisioning step). That deferral lives
//! entirely behind [`DeltaEmbedder`]: the ort-backed impl FAILS LOUD when
//! unprovisioned. We never append zero vectors (coordination §9 house rule).
//!
//! Config MUST match the Swift `UnionConfig` enum and the checkpointer exactly.
//! A retention/chunk mismatch REWRITES the store (coordination §2).

// Several seams (the metadata field vocabulary, the parity math, the ort embedder,
// the projector) are declared ahead of their first in-crate caller (the Swift side
// and the WS-G ort provisioning consume them later), so silence dead-code here.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub use crate::projection::{ContentProjector, DeltaSource, ProjectedContent};
use lodedb_core::engine::CoreAppender;
use lodedb_core::{CoreDocument, CoreError, CoreErrorCode, CoreOpenOptions};

fn invalid(message: &str) -> CoreError {
    CoreError::new(CoreErrorCode::InvalidArgument, message)
}

/// The §2 union-store config contract, as a Rust const module that MUST match
/// the Swift `UnionConfig` enum and the checkpointer value-for-value. These seed
/// a freshly created store and are enforced on reopen; a mismatch rewrites it.
pub mod union_config {
    /// The store directory under `<AppSupport>/pacific/` (a LodeDB store is a
    /// directory, not a file). The app resolves the platform AppSupport base and
    /// joins [`super::union_store_path`] onto it — Rust cannot resolve AppSupport.
    pub const STORE_DIR_NAME: &str = "union.lode";
    /// The pacific-owned subdirectory of AppSupport that holds the store.
    pub const PACIFIC_SUBDIR: &str = "pacific";
    /// Mandatory: the appender and checkpointer both require WAL commit mode
    /// (generation mode never replays the WAL, so an append would be a no-op).
    pub const COMMIT_MODE: &str = "wal";
    /// Per-append durability: fsync each record before acknowledging.
    pub const DURABILITY: &str = "fsync";
    /// BGE (bge-base-en-v1.5) vector width. Set at index creation; the appender
    /// validates every appended vector against the store's persisted dimension.
    pub const VECTOR_DIMENSION: usize = 768;
    /// The embedder identity guard the store persists and re-checks on reopen.
    pub const MODEL_IDENTITY: &str = "BAAI/bge-base-en-v1.5";
    /// Retain the raw document text (durable payload for `get`/`getDocument`).
    pub const STORE_TEXT: bool = true;
    /// Retain a durable lexical index (BM25 half of the hybrid RRF search).
    pub const INDEX_TEXT: bool = true;
    /// BGE max sequence length (informs the chunk limit; avoids silent truncation).
    pub const MAX_SEQUENCE_LENGTH: usize = 512;
    /// ~512 BGE tokens. The appender's planner MUST chunk at the same limit as the
    /// store writer, or a fold cannot reuse the writer's resident chunks.
    pub const CHUNK_CHARACTER_LIMIT: usize = 2000;
    /// BGE pooling strategy (mirrored byte-for-byte by the Swift/Rust embedders).
    pub const POOLING: &str = "cls";
    /// BGE query-side role prefix (documents are embedded without it).
    pub const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
}

/// The §3 metadata schema — one Rust builder (the §7 seam pairs it with one Swift
/// builder). Every row carries `source`/`objectId`/`chunkId`/`kind`; graph rows
/// (`node`/`edge`) add `type`/`src`/`dst`; scoped/synced rows add `groupObjectId`
/// (the membership filter key). Keys are the shared on-row vocabulary — the Swift
/// builder writes the exact same strings so a filter grammar matches both writers.
pub mod metadata {
    use super::DeltaSource;
    use lodedb_core::CoreMetadata;

    pub const SOURCE: &str = "source";
    pub const OBJECT_ID: &str = "objectId";
    pub const CHUNK_ID: &str = "chunkId";
    pub const KIND: &str = "kind";
    pub const TYPE: &str = "type";
    pub const SRC: &str = "src";
    pub const DST: &str = "dst";
    pub const GROUP_OBJECT_ID: &str = "groupObjectId";

    pub const KIND_DOC: &str = "doc";
    pub const KIND_NODE: &str = "node";
    pub const KIND_EDGE: &str = "edge";

    /// The deterministic §3 chunk id: `"<objectId>#<n>"`. Deterministic in
    /// `(objectId, n)`, so a re-ingest of the same content maps to the same id and
    /// the fold replaces it in place (idempotent re-ingest). It is also the LodeDB
    /// `document_id` used for a row, so idempotency is keyed at the store.
    pub fn chunk_id(object_id: &str, n: usize) -> String {
        format!("{object_id}#{n}")
    }

    /// A §3 metadata row under construction. Build via [`RowMetadata::doc`] (the
    /// GroupObject-content route) / [`RowMetadata::node`] / [`RowMetadata::edge`]
    /// (the entity-graph projection), then [`RowMetadata::into_metadata`].
    pub struct RowMetadata {
        inner: CoreMetadata,
    }

    impl RowMetadata {
        /// A `doc` row: reduced object content (a forum post, project item, topic
        /// finding, note). `chunk_id` is the deterministic §3 id ([`chunk_id`]).
        pub fn doc(
            source: DeltaSource,
            object_id: &str,
            group_object_id: &str,
            chunk_id: &str,
        ) -> Self {
            let mut inner = CoreMetadata::new();
            inner.insert(SOURCE.to_string(), source.as_str().to_string());
            inner.insert(OBJECT_ID.to_string(), object_id.to_string());
            inner.insert(CHUNK_ID.to_string(), chunk_id.to_string());
            inner.insert(KIND.to_string(), KIND_DOC.to_string());
            inner.insert(GROUP_OBJECT_ID.to_string(), group_object_id.to_string());
            Self { inner }
        }

        /// A `node` row: one entity in the graph (`entity_type` = person|org|
        /// place|event|project). No `groupObjectId` for a local-only entity.
        pub fn node(
            source: DeltaSource,
            object_id: &str,
            entity_type: &str,
            chunk_id: &str,
        ) -> Self {
            let mut inner = CoreMetadata::new();
            inner.insert(SOURCE.to_string(), source.as_str().to_string());
            inner.insert(OBJECT_ID.to_string(), object_id.to_string());
            inner.insert(CHUNK_ID.to_string(), chunk_id.to_string());
            inner.insert(KIND.to_string(), KIND_NODE.to_string());
            inner.insert(TYPE.to_string(), entity_type.to_string());
            Self { inner }
        }

        /// An `edge` row: a relatedness edge `src -> dst` in the graph.
        pub fn edge(
            source: DeltaSource,
            object_id: &str,
            src: &str,
            dst: &str,
            chunk_id: &str,
        ) -> Self {
            let mut inner = CoreMetadata::new();
            inner.insert(SOURCE.to_string(), source.as_str().to_string());
            inner.insert(OBJECT_ID.to_string(), object_id.to_string());
            inner.insert(CHUNK_ID.to_string(), chunk_id.to_string());
            inner.insert(KIND.to_string(), KIND_EDGE.to_string());
            inner.insert(SRC.to_string(), src.to_string());
            inner.insert(DST.to_string(), dst.to_string());
            Self { inner }
        }

        /// Sets the `groupObjectId` scope key (membership = access). Chainable.
        pub fn with_group_object(mut self, group_object_id: &str) -> Self {
            self.inner
                .insert(GROUP_OBJECT_ID.to_string(), group_object_id.to_string());
            self
        }

        /// Consumes the builder into the LodeDB `CoreMetadata` map.
        pub fn into_metadata(self) -> CoreMetadata {
            self.inner
        }
    }
}

/// The parity-critical embedding tail: CLS pooling, L2 normalization, and the BGE
/// role prefixes. This mirrors LodeDB `EmbeddingMath` (Swift) EXACTLY — an index
/// built with one runtime MUST stay searchable by the other, so any divergence
/// here breaks recall (coordination §9). The ort/ONNX SESSION output flows through
/// [`pool_and_normalize`]; the pure math is faithful even before ort is wired.
pub mod embedding_math {
    use lodedb_core::CoreError;

    /// BGE (bge-base-en-v1.5). Documents are embedded WITHOUT a prefix; only the
    /// query side gets [`union_config::QUERY_PREFIX`](super::union_config::QUERY_PREFIX).
    pub const DOCUMENT_PREFIX: &str = "";

    /// The role-appropriate prefix to prepend before tokenizing (BGE asymmetry):
    /// documents get none, queries get the search-passage prefix.
    pub fn document_prefix() -> &'static str {
        DOCUMENT_PREFIX
    }
    pub fn query_prefix() -> &'static str {
        super::union_config::QUERY_PREFIX
    }

    /// CLS pooling: the first ([CLS]) token's row. Mirrors
    /// `EmbeddingMath.pool(_, pooling: .cls)` — a one-row input (an already-pooled
    /// ONNX output) returns that row; otherwise token 0.
    pub fn cls_pool(token_embeddings: &[Vec<f32>]) -> Result<Vec<f32>, CoreError> {
        token_embeddings
            .first()
            .cloned()
            .ok_or_else(|| super::invalid("token embeddings must not be empty"))
    }

    /// Row-wise L2 normalization, preserving an all-zero vector (the Python
    /// `safe_norms` guard). Mirrors `EmbeddingMath.l2Normalize` byte-faithfully.
    pub fn l2_normalize(vector: &[f32]) -> Vec<f32> {
        let norm = vector
            .iter()
            .map(|component| component * component)
            .sum::<f32>()
            .sqrt();
        let safe = if norm == 0.0 { 1.0 } else { norm };
        vector.iter().map(|component| component / safe).collect()
    }

    /// The full document-embedding tail applied to one ONNX per-token output:
    /// CLS-pool then L2-normalize. This is the exact sequence
    /// `ONNXTextEmbedder.embed` runs after the session, so a Rust-embedded row and
    /// a Swift-embedded row are byte-identical for the same tokens.
    pub fn pool_and_normalize(token_embeddings: &[Vec<f32>]) -> Result<Vec<f32>, CoreError> {
        Ok(l2_normalize(&cls_pool(token_embeddings)?))
    }
}

/// Embeds reduced GroupObject content into document-role BGE vectors. The one seam
/// the ONNX/ort provisioning plugs into: an impl tokenizes with the BGE tokenizer,
/// runs the bge-base-en-v1.5 ONNX graph, then [`embedding_math::pool_and_normalize`]
/// (CLS + L2). Every impl MUST return exactly one [`union_config::VECTOR_DIMENSION`]
/// unit vector per input, in order — and MUST fail loud when unprovisioned rather
/// than return zero or fabricated vectors (coordination §9 house rule).
pub trait DeltaEmbedder: Send + Sync {
    /// Embeds `texts` as documents (no query prefix). One vector per input, in
    /// order. Fails loud rather than degrading recall or emitting zeros.
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, CoreError>;
}

/// The ort/ONNX-backed [`DeltaEmbedder`] — TODO: the BGE tokenizer + the
/// bge-base-en-v1.5 ONNX Runtime session are a heavy iOS provisioning step (ort +
/// tokenizers on Rust-on-iOS is the hardest artifact in the plan; coordination §9).
/// Until they are wired, this FAILS LOUD on every call: no silent fallback, no zero
/// vectors. When provisioned, its session output flows through
/// [`embedding_math::pool_and_normalize`] for Swift/Rust vector parity.
#[derive(Default)]
pub struct OrtBgeEmbedder {
    // TODO(WS-G): hold the ort::Session (CoreML EP) + the BGE `tokenizer.json`
    // tokenizer once the iOS artifacts are provisioned. The parity tail already
    // lives in `embedding_math::pool_and_normalize`.
    _unprovisioned: (),
}

impl OrtBgeEmbedder {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DeltaEmbedder for OrtBgeEmbedder {
    fn embed_documents(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, CoreError> {
        Err(CoreError::new(
            CoreErrorCode::Unsupported,
            "OrtBgeEmbedder: the BGE/ONNX embedding session is not provisioned in \
             this build (ort + tokenizers on iOS is a separate artifact step) — \
             refusing to embed rather than return zero vectors (no silent fallback)",
        ))
    }
}

/// Resolves the union-store directory under a platform AppSupport base:
/// `<app_support>/pacific/union.lode`. The caller (app/FFI) supplies the
/// resolved AppSupport path; this encodes the §2 layout in one place.
pub fn union_store_path(app_support_base: impl AsRef<Path>) -> PathBuf {
    app_support_base
        .as_ref()
        .join(union_config::PACIFIC_SUBDIR)
        .join(union_config::STORE_DIR_NAME)
}

/// The rust-direct route: a shared-lock `CoreAppender` over the one union store,
/// paired with a [`DeltaEmbedder`].
///
/// Multi-producer by design (coordination §2): this holds NO exclusive writer.
/// It takes the shared `.lodedb.lock` only for the duration of each append, so it
/// contends with the Swift `LodeAppender` and lets the running `LodeCheckpointer`
/// fold the WAL while this stays open.
pub struct LodeRoute {
    appender: CoreAppender,
    embedder: Box<dyn DeltaEmbedder>,
}

impl LodeRoute {
    /// Opens the union store at `store_dir` for shared appending, driving
    /// `embedder` for the document-role BGE vectors. The directory must already
    /// hold exactly one index (the store is created by the Swift writer / a create
    /// step); opening here neither creates nor rewrites it.
    pub fn open(
        store_dir: impl AsRef<Path>,
        embedder: Box<dyn DeltaEmbedder>,
    ) -> Result<Self, CoreError> {
        let appender = CoreAppender::open(Self::open_options(store_dir))?;
        Ok(Self { appender, embedder })
    }

    /// The §2 open options as a `CoreOpenOptions`. Shared with the Swift writer's
    /// config so an appended record is byte-identical to a writer-authored one.
    fn open_options(store_dir: impl AsRef<Path>) -> CoreOpenOptions {
        CoreOpenOptions {
            path: store_dir.as_ref().to_string_lossy().into_owned(),
            read_only: false,
            durability: union_config::DURABILITY.to_string(),
            commit_mode: union_config::COMMIT_MODE.to_string(),
            store_text: union_config::STORE_TEXT,
            index_text: union_config::INDEX_TEXT,
            // Let the store's persisted text-store manifest win on reopen.
            compress_text: true,
            chunk_character_limit: union_config::CHUNK_CHARACTER_LIMIT,
            // Multi-producer: contend for the shared writer lock so a concurrent
            // Swift appender / checkpointer serialises correctly against us.
            acquire_writer_lock: true,
        }
    }

    /// Appends (creates or replaces) a source object's reduced content into the
    /// union store under the §3 metadata schema, returning the assigned LSN.
    ///
    /// The document id is the deterministic §3 chunk id `"<objectId>#0"`, so a
    /// re-ingest of the same object replaces it in place (idempotent). The text is
    /// chunked through the appender's own planner, every planned chunk is embedded
    /// document-role (BGE/CLS/L2 via the [`DeltaEmbedder`]), and the record is
    /// applied through `append_embedded_documents` — the REAL append mechanics. If
    /// the embedder is unprovisioned it fails loud here; we never append zeros.
    pub fn append_object(
        &self,
        source: DeltaSource,
        object_id: &str,
        group_object_id: &str,
        text: &str,
    ) -> Result<u64, CoreError> {
        if object_id.trim().is_empty() {
            return Err(invalid("object_id is required"));
        }
        if text.trim().is_empty() {
            return Err(invalid("content text is required"));
        }
        let chunk_id = metadata::chunk_id(object_id, 0);
        let row = metadata::RowMetadata::doc(source, object_id, group_object_id, &chunk_id)
            .into_metadata();
        let document = CoreDocument {
            document_id: chunk_id,
            text: text.to_string(),
            metadata: row,
        };

        // The chunk plan is deterministic and lock-free; each entry of
        // `plan.chunks_to_embed` becomes one document-role BGE vector.
        let plan = self
            .appender
            .prepare_documents(std::slice::from_ref(&document))?;
        let chunk_texts: Vec<String> = plan
            .chunks_to_embed
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect();

        // Embed. Fails loud if unprovisioned — never a zero-vector fallback.
        let embeddings = self.embedder.embed_documents(&chunk_texts)?;
        if embeddings.len() != plan.chunks_to_embed.len() {
            return Err(CoreError::new(
                CoreErrorCode::Internal,
                "embedder returned the wrong number of vectors for the plan",
            ));
        }
        // House rule (coordination §9): never append a zero vector. The appender
        // already rejects non-finite/dimension-mismatched vectors; add the
        // all-zero guard so a broken embedder fails loud instead of poisoning recall.
        for vector in &embeddings {
            if vector.len() != union_config::VECTOR_DIMENSION {
                return Err(invalid(
                    "embedded vector dimension does not match the union store",
                ));
            }
            if vector.iter().all(|component| *component == 0.0) {
                return Err(CoreError::new(
                    CoreErrorCode::Internal,
                    "embedder returned a zero vector — refusing to append (no silent fallback)",
                ));
            }
        }

        self.appender.append_embedded_documents(&plan, &embeddings)
    }

    /// Removes a source object from the union store by its object id, appending one
    /// durable `delete_documents` record (keyed by the deterministic §3 chunk id)
    /// and returning its LSN. Needs no embedder, so it is always live.
    pub fn remove(&self, object_id: &str) -> Result<u64, CoreError> {
        if object_id.trim().is_empty() {
            return Err(invalid("object_id is required"));
        }
        let chunk_id = metadata::chunk_id(object_id, 0);
        self.appender
            .append_deletes(std::slice::from_ref(&chunk_id))
    }
}

impl ContentProjector for LodeRoute {
    fn project(&self, content: ProjectedContent<'_>) -> Result<u64, crate::CoreError> {
        self.append_object(
            content.source,
            content.object_id,
            content.group_object_id,
            content.text,
        )
        .map_err(crate::CoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lodedb_core::engine::CoreEngine;
    use lodedb_core::CoreIndexCreateOptions;
    use std::sync::atomic::{AtomicU64, Ordering};

    const INDEX_ID: &str = "union";

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_store_dir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "pacific-lode-route-{}-{nanos}-{counter}",
            std::process::id()
        ))
    }

    /// Opens a writer at the §2 union config to create the store's single index.
    fn writer_options(store_dir: &Path) -> CoreOpenOptions {
        CoreOpenOptions {
            path: store_dir.to_string_lossy().into_owned(),
            read_only: false,
            durability: union_config::DURABILITY.to_string(),
            commit_mode: union_config::COMMIT_MODE.to_string(),
            store_text: union_config::STORE_TEXT,
            index_text: union_config::INDEX_TEXT,
            compress_text: true,
            chunk_character_limit: union_config::CHUNK_CHARACTER_LIMIT,
            acquire_writer_lock: true,
        }
    }

    fn create_union_index(store_dir: &Path) {
        let mut engine = CoreEngine::open(writer_options(store_dir)).expect("open writer");
        engine
            .create_index_with_options(CoreIndexCreateOptions {
                index_id: INDEX_ID.to_string(),
                index_key: INDEX_ID.to_string(),
                client_id_hash: INDEX_ID.to_string(),
                name: "pacific-union".to_string(),
                model: union_config::MODEL_IDENTITY.to_string(),
                provider: "native".to_string(),
                task: "text".to_string(),
                route_profile: "native-core".to_string(),
                storage_profile: "turbovec_direct".to_string(),
                vector_dim: union_config::VECTOR_DIMENSION,
                bit_width: 4,
                ann: None,
                rescore: None,
            })
            .expect("create union index");
        engine.persist().expect("persist empty base");
    }

    /// A deterministic stub embedder: a stable non-zero unit vector per input text,
    /// derived from the text bytes. It drives the REAL `append_embedded_documents`
    /// mechanics (dimension + finiteness enforced by the appender) without ort. It
    /// stands in for `OrtBgeEmbedder`; it never returns zeros.
    struct StubEmbedder;

    impl DeltaEmbedder for StubEmbedder {
        fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, CoreError> {
            Ok(texts
                .iter()
                .map(|text| {
                    let mut vector = vec![0.0_f32; union_config::VECTOR_DIMENSION];
                    for (i, byte) in text.bytes().enumerate() {
                        vector[i % union_config::VECTOR_DIMENSION] += f32::from(byte) + 1.0;
                    }
                    // Guarantee a non-zero vector even for whitespace-only text
                    // (the append path also guards this).
                    vector[0] += 1.0;
                    embedding_math::l2_normalize(&vector)
                })
                .collect())
        }
    }

    #[test]
    fn append_object_folds_into_the_next_writer() {
        let store_dir = unique_store_dir();
        create_union_index(&store_dir);

        // The rust-direct route drives the real prepare -> embed -> apply path.
        let route = LodeRoute::open(&store_dir, Box::new(StubEmbedder)).expect("open route");
        let lsn = route
            .append_object(
                DeltaSource::Forum,
                "post-abc123",
                "groupobj-xyz",
                "hello from the rust-direct GroupObject route",
            )
            .expect("append object");
        assert!(lsn >= 1, "append must return the assigned LSN");

        // Reopen a writer: WAL replay folds the appended record. The document is
        // present under its deterministic §3 chunk id, proving REAL append mechanics.
        let engine = CoreEngine::open(writer_options(&store_dir)).expect("reopen writer");
        let stats = engine.stats(INDEX_ID).expect("stats");
        assert_eq!(
            stats.document_count, 1,
            "the appended content did not fold into the writer"
        );

        std::fs::remove_dir_all(&store_dir).ok();
    }

    #[test]
    fn append_object_fails_loud_when_embedder_unprovisioned() {
        let store_dir = unique_store_dir();
        create_union_index(&store_dir);

        // The ort embedder is unprovisioned: the route must fail loud, never append
        // a zero vector (coordination §9 house rule).
        let route =
            LodeRoute::open(&store_dir, Box::new(OrtBgeEmbedder::new())).expect("open route");
        let error = route
            .append_object(
                DeltaSource::Topic,
                "finding-1",
                "topic-9",
                "a topic finding",
            )
            .expect_err("must refuse without a provisioned embedder");
        assert_eq!(error.code(), CoreErrorCode::Unsupported);

        // Nothing was appended: a fresh writer sees the empty base.
        let engine = CoreEngine::open(writer_options(&store_dir)).expect("reopen writer");
        assert_eq!(engine.stats(INDEX_ID).expect("stats").document_count, 0);

        std::fs::remove_dir_all(&store_dir).ok();
    }

    #[test]
    fn chunk_id_is_deterministic_and_metadata_matches_schema() {
        assert_eq!(metadata::chunk_id("obj-1", 0), "obj-1#0");
        assert_eq!(metadata::chunk_id("obj-1", 3), "obj-1#3");

        let row = metadata::RowMetadata::doc(
            DeltaSource::Project,
            "item-7",
            "proj-42",
            &metadata::chunk_id("item-7", 0),
        )
        .into_metadata();
        assert_eq!(
            row.get(metadata::SOURCE).map(String::as_str),
            Some("project")
        );
        assert_eq!(
            row.get(metadata::OBJECT_ID).map(String::as_str),
            Some("item-7")
        );
        assert_eq!(
            row.get(metadata::CHUNK_ID).map(String::as_str),
            Some("item-7#0")
        );
        assert_eq!(row.get(metadata::KIND).map(String::as_str), Some("doc"));
        assert_eq!(
            row.get(metadata::GROUP_OBJECT_ID).map(String::as_str),
            Some("proj-42")
        );
    }

    #[test]
    fn embedding_math_mirrors_lodedb_cls_and_l2() {
        // CLS pooling returns token 0.
        let tokens = vec![vec![3.0_f32, 4.0], vec![9.0, 9.0]];
        assert_eq!(embedding_math::cls_pool(&tokens).unwrap(), vec![3.0, 4.0]);

        // L2 normalize yields a unit vector; [3,4] -> [0.6, 0.8].
        let unit = embedding_math::l2_normalize(&[3.0, 4.0]);
        assert!((unit[0] - 0.6).abs() < 1e-6 && (unit[1] - 0.8).abs() < 1e-6);

        // All-zero is preserved (the safe_norms guard), not turned into NaN.
        assert_eq!(embedding_math::l2_normalize(&[0.0, 0.0]), vec![0.0, 0.0]);
    }
}
