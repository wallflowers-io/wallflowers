//  VectorService.swift
//  The embedding seam for LocalStore/Vector.
//
//  ONE public facade, ONE hidden backend. Everything that needs vectors —
//  ingestion, LocalStore, note/chat search — depends on `VectorService` and never
//  learns how the vectors were produced. Today the only backend is Apple
//  NLEmbedding, on device: no network, no content leaves the device. The choice is
//  made exactly once, at construction; downstream it is invisible.
//
//  Honesty contract (mirrors LocalStore's): `embed` is fallible and THROWS on any
//  failure. It never returns a zero/placeholder vector, never silently substitutes
//  another backend, and validates that every vector it hands back
//  is non-empty and of the declared dimension. A starved backend fails loudly so
//  the caller can mark the work deferred — it does not fake freshness.
//
//  This file is the lower-level text -> vector seam. The Object.State /
//  Object.VectorMap-shaped wrapper described in the module README is a thin
//  adapter that sits ON TOP of this seam (deciding *which* of an object's fields
//  become text); it lands once those object types are real. The encapsulation
//  the rest of the app cares about — which embedder produced a vector being
//  invisible — lives entirely here.

import Foundation

// MARK: - Public surface

/// How a piece of text participates in retrieval. Some embedders produce better
/// results when a *query* and a stored *document* are embedded asymmetrically;
/// backends without the distinction (NLEmbedding) ignore it. This is a
/// backend-agnostic retrieval concept, so it can sit on the public surface
/// without leaking which backend is in use.
public enum InputKind: String, Sendable {
    case document
    case query
}

/// A fingerprint of EXACTLY how a set of vectors was produced. Two indexes are
/// comparable iff their epochs are equal; a mismatch must be surfaced loudly and
/// the corpus re-embedded — never silently mixed. LocalStore stores this beside
/// the vectors and refuses to compare across epochs.
public struct EmbeddingEpoch: Equatable, Codable, Sendable, CustomStringConvertible {
    /// Where the vectors come from, e.g. "apple.nlembedding".
    public let provider: String
    /// The specific model, e.g. "sentence.en".
    public let model: String
    /// Vector length. `0` signals a backend that failed to load (see `OnDeviceEmbedder`).
    public let dimension: Int
    /// The similarity metric these vectors are intended for.
    public let metric: String
    /// How/whether the raw vectors are normalised.
    public let normalization: String
    /// Bump to deliberately force a corpus re-embed when nothing else changed.
    public let revision: String

    public init(provider: String, model: String, dimension: Int,
                metric: String = "cosine", normalization: String, revision: String) {
        self.provider = provider
        self.model = model
        self.dimension = dimension
        self.metric = metric
        self.normalization = normalization
        self.revision = revision
    }

    public var description: String {
        "\(provider)/\(model)@\(revision) d=\(dimension) \(metric)/\(normalization)"
    }
}

/// Every failure mode is typed and loud. Nothing here degrades to an empty or
/// zero result.
public enum VectorServiceError: Error, CustomStringConvertible, Equatable {
    /// The chosen backend cannot produce vectors at all (e.g. no on-device model
    /// for the requested language on this OS). This never triggers a fallback.
    case backendUnavailable(String)
    /// A backend response could not be decoded.
    case decode(String)
    /// A backend produced no vector for an input (would have been a zero vector).
    case emptyEmbedding(index: Int)
    /// A backend produced a vector of the wrong dimension for its declared epoch.
    case dimensionMismatch(expected: Int, got: Int, index: Int)
    /// A backend returned a different number of vectors than inputs.
    case countMismatch(expected: Int, got: Int)

    public var description: String {
        switch self {
        case let .backendUnavailable(why): return "VectorService backend unavailable: \(why)"
        case let .decode(why): return "VectorService decode failure: \(why)"
        case let .emptyEmbedding(i): return "VectorService produced no vector for input #\(i)"
        case let .dimensionMismatch(e, g, i): return "VectorService dimension mismatch at #\(i): expected \(e), got \(g)"
        case let .countMismatch(e, g): return "VectorService returned \(g) vectors for \(e) inputs"
        }
    }
}

/// The embedding service. The ONLY embedding type the rest of the app touches.
public struct VectorService: Sendable {

    /// The one place the backend is chosen. After this, it is invisible.
    public enum Backend: Sendable {
        /// On-device Apple NLEmbedding — the ONLY backend. No network, no content
        /// leaves the device. `language` selects the sentence model.
        case onDevice(language: String = "en")
    }

    private let embedder: any Embedder

    /// Construct the service for a backend. Downstream callers receive the result
    /// and cannot tell which backend they got.
    public init(_ backend: Backend) {
        switch backend {
        case let .onDevice(language):
            self.embedder = OnDeviceEmbedder(language: language)
        }
    }

    /// Test / advanced seam: inject any backend (e.g. a stub in tests). Internal on
    /// purpose — the public world sees only `Backend`.
    init(embedder: any Embedder) {
        self.embedder = embedder
    }

    /// The fingerprint stamped on every vector this service produces. Callers
    /// persist it alongside the vectors so mismatches can be caught, not mixed.
    public var epoch: EmbeddingEpoch { embedder.epoch }

    /// Embed a batch. Returns one vector per input, IN INPUT ORDER (strict 1:1).
    /// Throws on any failure; never returns a placeholder.
    public func embed(_ texts: [String], as kind: InputKind = .document) async throws -> [[Float]] {
        guard !texts.isEmpty else { return [] }
        let vectors = try await embedder.embed(texts, as: kind)

        guard vectors.count == texts.count else {
            throw VectorServiceError.countMismatch(expected: texts.count, got: vectors.count)
        }
        let dimension = epoch.dimension
        for (i, v) in vectors.enumerated() {
            guard !v.isEmpty else { throw VectorServiceError.emptyEmbedding(index: i) }
            guard v.count == dimension else {
                throw VectorServiceError.dimensionMismatch(expected: dimension, got: v.count, index: i)
            }
        }
        return vectors
    }

    /// Embed a single string. Convenience over the batch path.
    public func embed(_ text: String, as kind: InputKind = .document) async throws -> [Float] {
        try await embed([text], as: kind)[0]
    }
}

// MARK: - The hidden seam

/// The private contract both backends implement. Never exposed publicly, so
/// adding/removing a backend cannot ripple out to callers.
protocol Embedder: Sendable {
    var epoch: EmbeddingEpoch { get }
    func embed(_ texts: [String], as kind: InputKind) async throws -> [[Float]]
}
