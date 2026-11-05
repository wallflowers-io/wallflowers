//  ContextualUnionEmbedder.swift
//  The KOREAN union embedder.
//
//  Apple ships a SENTENCE embedding model for English only — on current OS builds
//  `NLEmbedding.sentenceEmbedding(for: .korean)` (and even `wordEmbedding`) is nil,
//  so Korean rows embedded through the en model land in a degenerate ~0.97-cosine
//  band where an unrelated pair can outscore a related one. What the OS DOES ship
//  for Korean is `NLContextualEmbedding` — the multilingual transformer whose CJK
//  variant covers ko/ja/zh — producing 512-d token vectors (the SAME width as the
//  union contract, so no index-width change). This type mean-pools those token
//  vectors into one document vector and L2-normalizes it, exactly the pooling BGE's
//  `.mean` preset uses.
//
//  Contract notes, mirroring `NLEmbedder`:
//    • REAL embedder or throw — no zero vectors, no stub. The contextual model's
//      assets are an on-demand OS download; if they are not on device, `init`
//      throws rather than vending a broken gauge (`requestAssets` fetches them).
//    • Its vectors are NOT comparable with the en sentence model's (or BGE's):
//      the store binds to `modelIdentity` and a language change goes through
//      `LodeLocalStore.migrateStore` — never a silent mix.
//    • The identity string is OURS ("apple.NLContextualEmbedding.mean.ko"), not
//      the model's asset UUID, which changes across OS revisions. Weight drift
//      inside an identity is the same accepted risk the en NL sentence model
//      already carries — an Apple-model gauge, versioned by name.

import Foundation
import NaturalLanguage
import LodeDBCore

/// Apple's on-device contextual (transformer) embedding, pooled to one vector per
/// document. The union's Korean profile; usable for any `NLContextualEmbedding`
/// language, chosen explicitly at the composition root.
@available(iOS 17.0, macOS 14.0, *)
public final class ContextualUnionEmbedder: LodeEmbedder {
    private let embedding: NLContextualEmbedding
    private let language: NLLanguage

    public let dimension: Int
    /// A stable identity for this embedder, suitable as the persisted model name.
    public let modelIdentity: String?

    /// Creates a mean-pooled contextual embedder for `language`. Throws `.unsupported`
    /// if no contextual model covers the language, or if the model's assets are not on
    /// device yet (request them with `requestAssets(for:)` — an OS download, not ours).
    public init(language: NLLanguage) throws {
        guard let embedding = NLContextualEmbedding(language: language) else {
            throw LodeDBError.unsupported(
                "no on-device contextual embedding model covers language '\(language.rawValue)'")
        }
        guard embedding.hasAvailableAssets else {
            throw LodeDBError.unsupported(
                "contextual embedding assets for '\(language.rawValue)' are not on this device — "
                + "request them first (ContextualUnionEmbedder.requestAssets)")
        }
        try embedding.load()
        self.embedding = embedding
        self.language = language
        self.dimension = embedding.dimension
        self.modelIdentity = "apple.NLContextualEmbedding.mean.\(language.rawValue)"
    }

    public func embed(texts: [String]) throws -> [[Float]] {
        try texts.map { text in
            let result = try embedding.embeddingResult(for: text, language: language)
            var sum = [Double](repeating: 0, count: dimension)
            var tokens = 0
            result.enumerateTokenVectors(in: text.startIndex..<text.endIndex) { vector, _ in
                for (i, v) in vector.enumerated() where i < sum.count { sum[i] += v }
                tokens += 1
                return true
            }
            guard tokens > 0 else {
                // Same honesty rule as NLEmbedder's nil-vector case: an unembeddable
                // text is an error, never a zero vector that would poison the index.
                throw LodeDBError.invalidArgument("no on-device embedding for the given text")
            }
            let mean = sum.map { Float($0 / Double(tokens)) }
            return EmbeddingMath.l2Normalize(mean)
        }
    }

    /// Whether the OS already holds the contextual model's assets for `language`
    /// (they are an on-demand download; absent assets make `init` throw).
    public static func assetsAvailable(for language: NLLanguage) -> Bool {
        NLContextualEmbedding(language: language)?.hasAvailableAssets ?? false
    }

    /// Ask the OS to fetch the contextual model's assets for `language`. Returns the
    /// resulting availability; a failed or unsupported request surfaces as `false`
    /// plus the thrown error when the OS reports one. Needs network the first time.
    @discardableResult
    public static func requestAssets(for language: NLLanguage) async throws -> Bool {
        guard let embedding = NLContextualEmbedding(language: language) else {
            throw LodeDBError.unsupported(
                "no on-device contextual embedding model covers language '\(language.rawValue)'")
        }
        if embedding.hasAvailableAssets { return true }
        return try await withCheckedThrowingContinuation { continuation in
            embedding.requestAssets { result, error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: result == .available)
                }
            }
        }
    }
}
