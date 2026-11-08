//  ONNXBGEEmbedder.swift
//  WS-D — the ONNX/BGE embedder wiring for the union store.
//
//  The union store is BGE-embedded (bge-base-en-v1.5, 768-d, CLS pooling, asymmetric
//  query prefix). LodeDBCore already ships the parity-critical half of that pipeline:
//  `ONNXTextEmbedder` owns pooling / L2-normalization / role prefix / model identity,
//  byte-faithful to the Python `embedding_backends` pipeline, so an index built here
//  stays comparable with one the Rust route or a server builds (§1, §9 parity risk).
//  What it does NOT own — and what an integrator must supply — is the pair it composes:
//    • a `TextTokenizer` (the BGE WordPiece tokenizer, over the model's tokenizer.json)
//    • an `EmbeddingModelSession` (the ONNX graph, run on ONNX Runtime + the CoreML EP)
//
//  Those two conformances need `onnxruntime-swift` and `swift-transformers`, which are
//  NOT yet SwiftPM dependencies of this package (§9: "ORT provisioning is a heavy step
//  separate from the code scaffolding … structure the embedding behind a trait so the
//  append mechanics land first"). So this file lands the *seam and the factory* without
//  those packages:
//
//    - `ONNXInferenceBackend` / `BGETokenizerBackend` are the two protocol boundaries
//      the real ONNX-Runtime session and swift-transformers tokenizer will sit behind.
//    - `ONNXRuntimeEmbeddingSession` (EmbeddingModelSession) and `SwiftTransformersBGE-
//      Tokenizer` (TextTokenizer) are the concrete LodeDBCore conformances; each just
//      adapts its backend, so they compile today and do not change when ORT lands.
//    - `BGEEmbedderFactory.makeEmbedder(...)` wires the pair into
//      `ONNXTextEmbedder(preset:tokenizer:session:)`.
//    - The DEFAULT provisioning is `UnprovisionedONNXRuntime`, whose backends THROW a
//      clear "ONNX runtime not provisioned" error. No stub vectors, no zero fallback:
//      until the runtime is wired the factory fails loud (house rule, §9).
//
//  ─────────────────────────────────────────────────────────────────────────────────
//  PROVISIONING (turning the throwing factory into a working BGE embedder)
//  ─────────────────────────────────────────────────────────────────────────────────
//  1. Bundle the model artifacts into the app's resources (see `BGEModelArtifacts`):
//       • bge-base-en-v1.5 exported to ONNX — `model.onnx` (opset ≥ 14; the graph that
//         emits `last_hidden_state`, so CLS pooling stays OURS for cross-runtime parity;
//         a pre-pooled `sentence_embedding` output also works via the single-row path).
//       • the model's `tokenizer.json` (the exact HuggingFace tokenizer BGE was trained
//         with — a mismatched tokenizer silently wrecks recall).
//  2. Add the SwiftPM dependencies (to this package's Package.swift OR the app target):
//       • onnxruntime-swift  — https://github.com/microsoft/onnxruntime-swift-package-manager
//       • swift-transformers — https://github.com/huggingface/swift-transformers
//  3. Implement the two backend protocols against those packages:
//       • an `ONNXInferenceBackend` over `ORTSession` — build the session with the
//         CoreML execution provider appended (CPU fallback), feed `input_ids`,
//         `attention_mask` and an all-zero `token_type_ids` (BERT/BGE needs it), read
//         `last_hidden_state` back as `[sequenceLength][hiddenSize]`. Do NOT pool.
//       • a `BGETokenizerBackend` over swift-transformers `AutoTokenizer` /
//         `Tokenizer(tokenizerConfig:)` — encode to input ids + attention mask, adding
//         `[CLS]`/`[SEP]` and truncating to `maxLength` exactly as tokenizer.json says.
//  4. Provide an `ONNXRuntimeProvisioning` that returns those backends and pass it to
//     `BGEEmbedderFactory.makeEmbedder(artifacts:provisioning:)`. The conformances and
//     factory below do not change — only the two backends and the provisioning do.
//  ─────────────────────────────────────────────────────────────────────────────────

import Foundation
import LodeDBCore

// MARK: - Model artifacts

/// The on-disk locations of the two bundled BGE model artifacts. Provisioning reads the
/// ONNX graph and the tokenizer from these URLs; both are REQUIRED (a missing artifact
/// is a loud `.notFound`, never a silent no-op).
public struct BGEModelArtifacts: Sendable, Equatable {
    /// The exported `bge-base-en-v1.5` ONNX graph (emits `last_hidden_state`).
    public let onnxModelURL: URL
    /// The model's HuggingFace `tokenizer.json` (the exact tokenizer BGE was trained with).
    public let tokenizerJSONURL: URL

    public init(onnxModelURL: URL, tokenizerJSONURL: URL) {
        self.onnxModelURL = onnxModelURL
        self.tokenizerJSONURL = tokenizerJSONURL
    }

    /// The default resource names looked up inside a bundle.
    public enum ResourceName {
        public static let onnxModel = "bge-base-en-v1.5"
        public static let onnxModelExtension = "onnx"
        public static let tokenizerJSON = "tokenizer"
        public static let tokenizerJSONExtension = "json"
    }

    /// Resolves the artifacts from `bundle` by their default resource names, throwing
    /// `.notFound` (fail loud) if either is absent — a missing model must surface as an
    /// error, never as a degraded/empty embedder.
    public static func inBundle(_ bundle: Bundle) throws -> BGEModelArtifacts {
        guard let onnx = bundle.url(
            forResource: ResourceName.onnxModel, withExtension: ResourceName.onnxModelExtension)
        else {
            throw LodeDBError.notFound(
                "BGE ONNX model '\(ResourceName.onnxModel).\(ResourceName.onnxModelExtension)' "
                + "is not bundled in \(bundle.bundlePath) — bundle the exported bge-base-en-v1.5 graph")
        }
        guard let tokenizer = bundle.url(
            forResource: ResourceName.tokenizerJSON, withExtension: ResourceName.tokenizerJSONExtension)
        else {
            throw LodeDBError.notFound(
                "BGE '\(ResourceName.tokenizerJSON).\(ResourceName.tokenizerJSONExtension)' "
                + "is not bundled in \(bundle.bundlePath) — bundle the model's tokenizer.json")
        }
        return BGEModelArtifacts(onnxModelURL: onnx, tokenizerJSONURL: tokenizer)
    }
}

// MARK: - Backend seams (behind these sit onnxruntime-swift / swift-transformers)

/// The boundary the real ONNX Runtime session sits behind. A conformer wraps an
/// `ORTSession` (built with the CoreML execution provider, CPU fallback) and runs the
/// BGE graph for ONE tokenized input.
///
/// It returns `last_hidden_state` as `[sequenceLength][hiddenSize]` (BGE hiddenSize is
/// 768) and MUST NOT pool: CLS pooling, L2-normalization and the query prefix are owned
/// by `ONNXTextEmbedder`/`EmbeddingMath` so both runtimes (Swift + Rust) produce
/// identical vectors (§9 parity). A backend that already pools may return a single
/// `[1][hiddenSize]` row — `EmbeddingMath.pool` passes that through unchanged.
public protocol ONNXInferenceBackend: Sendable {
    /// Runs the graph. `inputIDs` and `attentionMask` come from the tokenizer and share
    /// a length; the backend synthesizes any all-zero `token_type_ids` the graph needs.
    func run(inputIDs: [Int32], attentionMask: [Int32]) throws -> [[Float]]
}

/// The boundary the real swift-transformers tokenizer sits behind. A conformer wraps a
/// `Tokenizer` loaded from `tokenizer.json` and encodes ONE text — adding the model's
/// special tokens (`[CLS]`/`[SEP]`) and truncating to `maxLength` — into the input ids
/// and attention mask the ONNX graph expects.
public protocol BGETokenizerBackend: Sendable {
    func encode(_ text: String, maxLength: Int) throws -> TokenizedText
}

// MARK: - LodeDBCore conformances (the WS-D deliverables)

/// An `EmbeddingModelSession` for the BGE ONNX graph, intended for ONNX Runtime with the
/// CoreML execution provider. It delegates inference to an injected `ONNXInferenceBackend`
/// (the ORT seam) and adds only a loud emptiness guard — it never fabricates output.
public struct ONNXRuntimeEmbeddingSession: EmbeddingModelSession {
    private let backend: ONNXInferenceBackend

    public init(backend: ONNXInferenceBackend) {
        self.backend = backend
    }

    public func run(_ input: TokenizedText) throws -> [[Float]] {
        let rows = try backend.run(inputIDs: input.inputIDs, attentionMask: input.attentionMask)
        // Fail loud on an empty forward pass rather than letting an empty result flow into
        // pooling as a silent zero/degenerate vector.
        guard !rows.isEmpty else {
            throw LodeDBError.internalError("ONNX inference returned no rows for a non-empty input")
        }
        return rows
    }
}

/// A `TextTokenizer` for BGE, intended for swift-transformers over the model's
/// `tokenizer.json`. It delegates encoding to an injected `BGETokenizerBackend` and
/// validates the shape (non-empty, matched lengths) so a tokenizer/graph mismatch fails
/// loud instead of producing a mis-pooled vector.
public struct SwiftTransformersBGETokenizer: TextTokenizer {
    private let backend: BGETokenizerBackend

    public init(backend: BGETokenizerBackend) {
        self.backend = backend
    }

    public func encode(_ text: String, maxLength: Int) throws -> TokenizedText {
        let tokenized = try backend.encode(text, maxLength: maxLength)
        guard !tokenized.inputIDs.isEmpty else {
            throw LodeDBError.invalidArgument("tokenizer produced no tokens for the given text")
        }
        guard tokenized.inputIDs.count == tokenized.attentionMask.count else {
            throw LodeDBError.invalidArgument(
                "tokenizer input id count \(tokenized.inputIDs.count) does not match attention mask "
                + "count \(tokenized.attentionMask.count)")
        }
        return tokenized
    }
}

// MARK: - Provisioning seam

/// Builds the two backends from the bundled artifacts. This is the single point the ORT
/// provisioning plugs in: implement it against onnxruntime-swift + swift-transformers and
/// hand it to `BGEEmbedderFactory`. Until then the default (`UnprovisionedONNXRuntime`)
/// throws.
public protocol ONNXRuntimeProvisioning: Sendable {
    /// Opens the ONNX graph at `modelURL` as an inference session. `expectedHiddenSize`
    /// (768 for BGE) is passed so the implementation can assert the graph's output width
    /// up front rather than discovering a mismatch mid-ingest. `maxSequenceLength` is the
    /// window the session is prepared for (BGE: 512).
    func makeInferenceBackend(
        modelURL: URL, expectedHiddenSize: Int, maxSequenceLength: Int
    ) throws -> ONNXInferenceBackend

    /// Loads the tokenizer at `tokenizerJSONURL`. `maxSequenceLength` is the truncation
    /// window (BGE: 512) the tokenizer should be configured for; the per-call `maxLength`
    /// passed to `encode` agrees with it (it is `preset.maxSequenceLength`).
    func makeTokenizerBackend(
        tokenizerJSONURL: URL, maxSequenceLength: Int
    ) throws -> BGETokenizerBackend
}

/// The message every unprovisioned path surfaces. Kept in one place so callers can match
/// on it and the wording stays consistent.
public enum ONNXProvisioning {
    public static let notProvisionedMessage =
        "ONNX runtime not provisioned: bundle bge-base-en-v1.5 .onnx + tokenizer.json, add the "
        + "onnxruntime-swift and swift-transformers SwiftPM dependencies, implement "
        + "ONNXRuntimeProvisioning against them, and pass it to BGEEmbedderFactory.makeEmbedder "
        + "(see ONNXBGEEmbedder.swift provisioning notes)."
}

/// The default provisioning: the ONNX Runtime and tokenizer packages are not wired, so
/// every backend request throws `.unsupported(ONNXProvisioning.notProvisionedMessage)`.
/// This is the deliberate fail-loud gate — no stub session, no zero vectors — that keeps
/// the append/search machinery honest until the model is actually provisioned (§9).
public struct UnprovisionedONNXRuntime: ONNXRuntimeProvisioning {
    public init() {}

    public func makeInferenceBackend(
        modelURL: URL, expectedHiddenSize: Int, maxSequenceLength: Int
    ) throws -> ONNXInferenceBackend {
        throw LodeDBError.unsupported(ONNXProvisioning.notProvisionedMessage)
    }

    public func makeTokenizerBackend(
        tokenizerJSONURL: URL, maxSequenceLength: Int
    ) throws -> BGETokenizerBackend {
        throw LodeDBError.unsupported(ONNXProvisioning.notProvisionedMessage)
    }
}

// MARK: - Factory

/// Builds the `.bge` `ONNXTextEmbedder` the union store uses. It composes the two WS-D
/// conformances (`SwiftTransformersBGETokenizer` + `ONNXRuntimeEmbeddingSession`) over the
/// backends `provisioning` vends. With the default `UnprovisionedONNXRuntime` this THROWS
/// `ONNXProvisioning.notProvisionedMessage` — the caller gets a real embedder or a loud
/// error, never a silent placeholder.
public enum BGEEmbedderFactory {

    /// The BGE preset the union store is built with. This is `UnionConfig.embeddingPreset`,
    /// NOT LodeDBCore's `EmbeddingPreset.bge`: the shipped `.bge` preset caps the window at
    /// 256 tokens, but §2 mandates 512 for the union store, and the two MUST match or the
    /// tokenizer truncates ingest at half the intended window (a silent recall loss). Every
    /// other field (768-d, CLS pooling, the query prefix, the `BAAI/bge-base-en-v1.5`
    /// identity) is identical to `.bge`.
    public static var preset: EmbeddingPreset { UnionConfig.embeddingPreset }

    /// Builds the BGE `ONNXTextEmbedder` from the bundled `artifacts`, using `provisioning`
    /// to construct the ONNX session and tokenizer. Throws if provisioning is not wired
    /// (the default) or if the runtime fails to open the model — never returns a
    /// non-functional embedder.
    public static func makeEmbedder(
        artifacts: BGEModelArtifacts,
        provisioning: ONNXRuntimeProvisioning = UnprovisionedONNXRuntime()
    ) throws -> ONNXTextEmbedder {
        let preset = self.preset
        let tokenizerBackend = try provisioning.makeTokenizerBackend(
            tokenizerJSONURL: artifacts.tokenizerJSONURL,
            maxSequenceLength: preset.maxSequenceLength)
        let inferenceBackend = try provisioning.makeInferenceBackend(
            modelURL: artifacts.onnxModelURL,
            expectedHiddenSize: preset.dimension,
            maxSequenceLength: preset.maxSequenceLength)
        return ONNXTextEmbedder(
            preset: preset,
            tokenizer: SwiftTransformersBGETokenizer(backend: tokenizerBackend),
            session: ONNXRuntimeEmbeddingSession(backend: inferenceBackend))
    }

    /// Convenience: resolve the artifacts from `bundle` (default `.main`) then build. Throws
    /// `.notFound` if an artifact is missing, or the provisioning error if ORT is not wired.
    public static func makeEmbedder(
        bundle: Bundle = .main,
        provisioning: ONNXRuntimeProvisioning = UnprovisionedONNXRuntime()
    ) throws -> ONNXTextEmbedder {
        try makeEmbedder(
            artifacts: BGEModelArtifacts.inBundle(bundle),
            provisioning: provisioning)
    }
}
