//  UnionEmbedder.swift
//  The ACTIVE on-device embedder for the union store.
//
//  The §2 target is BGE (`bge-base-en-v1.5`, 768-d) for cross-runtime parity with the
//  Rust/server route — but BGE needs a provisioned ONNX runtime (a bundled graph +
//  tokenizer.json + onnxruntime-swift + swift-transformers), which is NOT wired yet:
//  `BGEEmbedderFactory` deliberately throws until it is (see ONNXBGEEmbedder.swift).
//
//  So the union runs TODAY on Apple's on-device NaturalLanguage sentence embedder,
//  which needs no bundled model and no extra dependencies. That is a deliberate,
//  documented contract choice, not a silent fallback:
//
//    • `UnionConfig.vectorDimension` / `.modelIdentity` describe THIS embedder, so the
//      store is created at the right width and a mismatched model still fails closed.
//    • Vectors are NOT comparable with a BGE-built index. When BGE is provisioned,
//      point `makeUnion()` at `BGEEmbedderFactory`, restore the §2 BGE constants, and
//      run `LodeLocalStore.reindexAll()` — retained text makes that migration exact.
//
//  Honesty contract: this vends a REAL embedder or throws. No zero vectors, no stub.

import Foundation
import NaturalLanguage
import LodeDBCore

/// Builds the embedder the union store is opened with. One place, chosen explicitly.
public enum UnionEmbedder {

    /// The active on-device embedder for `language`. Throws if the platform cannot
    /// vend the model (fail loud — the caller gets a working embedder or a real error).
    ///
    ///   .english  Apple's NL SENTENCE embedder — the shipped default, unchanged.
    ///   .korean   Apple ships NO Korean sentence model, so this is the CJK
    ///             NLContextualEmbedding, mean-pooled + L2-normalized
    ///             (`ContextualUnionEmbedder`) — same 512-d width, its own model
    ///             identity, vectors NOT comparable across languages. Its assets
    ///             are an on-demand OS download; absent assets throw (see
    ///             `ContextualUnionEmbedder.requestAssets`).
    public static func makeUnion(language: NLLanguageIdentifier = .english) throws -> LodeEmbedder {
        switch language {
        case .english:
            return try NLEmbedder(language: language.nlLanguage)
        case .korean:
            guard #available(iOS 17.0, macOS 14.0, *) else {
                throw LodeDBError.unsupported(
                    "the Korean union embedder needs iOS 17 / macOS 14 (NLContextualEmbedding)")
            }
            return try ContextualUnionEmbedder(language: language.nlLanguage)
        }
    }
}

/// A tiny, dependency-free spelling of the languages the union embedder supports, so
/// callers (the composition root) never have to `import NaturalLanguage` themselves.
/// The raw value is the persisted spelling (`UnionLanguageSetting`) and the suffix of
/// the store's model identity — frozen.
public enum NLLanguageIdentifier: String, CaseIterable, Sendable {
    case english = "en"
    case korean = "ko"

    var nlLanguage: NLLanguage {
        switch self {
        case .english: return .english
        case .korean: return .korean
        }
    }
}
