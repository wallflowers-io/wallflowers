//! Embedders behind the engine's `Embedder` trait. Two impls:
//!
//! - [`CallbackEmbedder`] (always available) — the platform supplies embeddings via a C
//!   callback. This is the MOBILE path: `ort` has no prebuilt ONNX Runtime for iOS/Android, so
//!   the app's own ONNX Runtime (the Swift/Kotlin embedder) is injected here. One ORT per app.
//! - [`OrtEmbedder`] (feature `ort-embedder`, default) — an in-Rust ort + HF-tokenizer BGE
//!   embedder for DESKTOP/dev builds.
//!
//! [`SharedEmbedder`] lets a caller that holds an `Arc<dyn Embedder>` hand the same instance to
//! the engine, which owns its embedder as a box.

use std::ffi::{c_char, c_void, CString};
use std::sync::Arc;

use lodedb_graph::{EmbedRole, Embedder, GraphError};

/// Wrap a shared `Arc<dyn Embedder>` as a `Box<dyn Embedder>` for the engine, so the caller can
/// keep its own handle on the instance the engine indexes and searches with.
pub struct SharedEmbedder(pub Arc<dyn Embedder>);

impl Embedder for SharedEmbedder {
    fn dimension(&self) -> usize {
        self.0.dimension()
    }
    fn embed(&self, texts: &[String], role: EmbedRole) -> Result<Vec<Vec<f32>>, GraphError> {
        self.0.embed(texts, role)
    }
}

/// The C callback the platform provides to embed a batch. It fills `count * dim` f32s into
/// `out`, row-major (text `i` -> `out[i*dim .. (i+1)*dim]`), and returns 0 on success (non-zero
/// = failure). `ctx` is the opaque platform handle passed at open.
pub type EmbedCallback = unsafe extern "C" fn(
    ctx: *mut c_void,
    texts: *const *const c_char,
    count: usize,
    dim: usize,
    out: *mut f32,
) -> i32;

/// An embedder whose vectors come from a platform-provided C callback (the mobile path). The
/// platform (Swift/Kotlin) owns `ctx` and keeps it + its ONNX runtime alive for the embedder's
/// lifetime.
pub struct CallbackEmbedder {
    ctx: *mut c_void,
    callback: EmbedCallback,
    dim: usize,
}

// SAFETY: the platform owns `ctx` and guarantees the callback is callable from the resolver's
// thread; the Swift/Kotlin side serializes access to the handle.
unsafe impl Send for CallbackEmbedder {}
unsafe impl Sync for CallbackEmbedder {}

impl CallbackEmbedder {
    /// # Safety
    /// `callback` and `ctx` must remain valid for the embedder's lifetime, and `ctx` must
    /// outlive it. The callback must honor the row-major `count * dim` output contract.
    pub unsafe fn new(callback: EmbedCallback, ctx: *mut c_void, dim: usize) -> Self {
        CallbackEmbedder { ctx, callback, dim }
    }
}

impl Embedder for CallbackEmbedder {
    fn dimension(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String], _role: EmbedRole) -> Result<Vec<Vec<f32>>, GraphError> {
        let count = texts.len();
        if count == 0 {
            return Ok(Vec::new());
        }
        let cstrings: Vec<CString> = texts
            .iter()
            .map(|t| CString::new(t.as_str()).unwrap_or_default())
            .collect();
        let ptrs: Vec<*const c_char> = cstrings.iter().map(|c| c.as_ptr()).collect();
        let mut out = vec![0f32; count * self.dim];
        // SAFETY: ptrs holds `count` valid C strings; out holds `count * dim` f32s.
        let rc = unsafe { (self.callback)(self.ctx, ptrs.as_ptr(), count, self.dim, out.as_mut_ptr()) };
        if rc != 0 {
            return Err(GraphError::Embedding(format!("platform embed callback returned {rc}")));
        }
        Ok(out.chunks(self.dim).map(<[f32]>::to_vec).collect())
    }
}

// ---- desktop/dev: in-Rust ort embedder --------------------------------------

#[cfg(feature = "ort-embedder")]
pub use ort_impl::OrtEmbedder;

#[cfg(feature = "ort-embedder")]
mod ort_impl {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use lodedb_graph::{EmbedRole, Embedder, GraphError};
    use ort::session::Session;
    use ort::value::Tensor;
    use tokenizers::Tokenizer;

    fn emb_err<E: std::fmt::Display>(e: E) -> GraphError {
        GraphError::Embedding(format!("ort embedder: {e}"))
    }

    /// A BGE ONNX embedder (desktop/dev). CLS-pooled + L2-normalized, matching the Swift `bge`
    /// preset. Cheap to clone (shared session + tokenizer).
    #[derive(Clone)]
    pub struct OrtEmbedder {
        session: Arc<Mutex<Session>>,
        tokenizer: Arc<Tokenizer>,
        dim: usize,
        max_length: usize,
    }

    impl OrtEmbedder {
        /// Load an ONNX model + its `tokenizer.json`. `dim` is the hidden size (768 for bge-base).
        pub fn load(
            model_path: impl AsRef<Path>,
            tokenizer_path: impl AsRef<Path>,
            dim: usize,
        ) -> Result<Self, GraphError> {
            let session = Session::builder()
                .map_err(emb_err)?
                .commit_from_file(model_path)
                .map_err(emb_err)?;
            let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(emb_err)?;
            Ok(OrtEmbedder {
                session: Arc::new(Mutex::new(session)),
                tokenizer: Arc::new(tokenizer),
                dim,
                max_length: 512,
            })
        }

        fn run(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, GraphError> {
            if texts.is_empty() {
                return Ok(vec![]);
            }
            let encs = self.tokenizer.encode_batch(texts.to_vec(), true).map_err(emb_err)?;
            let batch = encs.len();
            let seq = encs
                .iter()
                .map(|e| e.get_ids().len().min(self.max_length))
                .max()
                .unwrap_or(1)
                .max(1);

            let mut ids = vec![0i64; batch * seq];
            let mut mask = vec![0i64; batch * seq];
            let mut types = vec![0i64; batch * seq];
            for (b, e) in encs.iter().enumerate() {
                let (eids, emask, etypes) = (e.get_ids(), e.get_attention_mask(), e.get_type_ids());
                let n = eids.len().min(self.max_length);
                for t in 0..n {
                    ids[b * seq + t] = eids[t] as i64;
                    mask[b * seq + t] = emask[t] as i64;
                    types[b * seq + t] = etypes[t] as i64;
                }
            }

            let shape = [batch, seq];
            let mut sess = self.session.lock().map_err(|_| emb_err("session poisoned"))?;
            let outputs = sess
                .run(ort::inputs![
                    "input_ids" => Tensor::from_array((shape, ids)).map_err(emb_err)?,
                    "attention_mask" => Tensor::from_array((shape, mask)).map_err(emb_err)?,
                    "token_type_ids" => Tensor::from_array((shape, types)).map_err(emb_err)?,
                ])
                .map_err(emb_err)?;

            let (out_shape, data) = outputs
                .get("last_hidden_state")
                .ok_or_else(|| emb_err("model has no `last_hidden_state` output"))?
                .try_extract_tensor::<f32>()
                .map_err(emb_err)?;
            let hidden = *out_shape.last().ok_or_else(|| emb_err("empty output shape"))? as usize;
            let seq_out = out_shape[1] as usize;

            let mut out = Vec::with_capacity(batch);
            for b in 0..batch {
                let start = b * seq_out * hidden; // token 0 = CLS
                let mut v = data[start..start + hidden].to_vec();
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
                }
                out.push(v);
            }
            Ok(out)
        }
    }

    impl Embedder for OrtEmbedder {
        fn dimension(&self) -> usize {
            self.dim
        }
        fn embed(&self, texts: &[String], _role: EmbedRole) -> Result<Vec<Vec<f32>>, GraphError> {
            self.run(texts)
        }
    }
}
