//  OnDeviceEmbedder.swift
//  The on-device (production) backend behind VectorService.
//
//  Apple's NaturalLanguage `NLEmbedding` sentence model. No network, no content
//  leaves the device — this is the production embedder once the app ships on a
//  device with a Neural Engine. It also runs in macOS host tests, so this path is
//  exercisable on the build Mac today (only a later swap to a bespoke CoreML/ONNX
//  model would be genuinely iPhone-only).
//
//  `kind` (document/query) is ignored: NLEmbedding has no asymmetric mode.

import Foundation
import NaturalLanguage

final class OnDeviceEmbedder: Embedder, @unchecked Sendable {
    // @unchecked Sendable: NLEmbedding is a reference type with no documented
    // thread-safety, so all access to it is serialised behind `lock`.
    private let language: NLLanguage
    private let model: NLEmbedding?
    private let dim: Int
    private let lock = NSLock()

    init(language code: String) {
        let language = NLLanguage(code)
        self.language = language
        self.model = NLEmbedding.sentenceEmbedding(for: language)
        self.dim = model?.dimension ?? 0
    }

    var epoch: EmbeddingEpoch {
        // dimension 0 truthfully signals "model failed to load"; `embed` throws
        // `backendUnavailable` before any caller could act on a zero-dim epoch.
        EmbeddingEpoch(provider: "apple.nlembedding",
                       model: "sentence.\(language.rawValue)",
                       dimension: dim,
                       metric: "cosine",
                       normalization: "provider-default",
                       revision: "1")
    }

    func embed(_ texts: [String], as kind: InputKind) async throws -> [[Float]] {
        // The work is synchronous and CPU-bound; run it in a sync helper so the
        // lock is never held across an async suspension point (Swift 6-clean).
        try embedLocked(texts)
    }

    private func embedLocked(_ texts: [String]) throws -> [[Float]] {
        guard let model else {
            throw VectorServiceError.backendUnavailable(
                "NLEmbedding has no sentence model for language '\(language.rawValue)' on this OS")
        }
        lock.lock()
        defer { lock.unlock() }

        var out: [[Float]] = []
        out.reserveCapacity(texts.count)
        for (i, text) in texts.enumerated() {
            guard let vector = model.vector(for: text) else {
                throw VectorServiceError.emptyEmbedding(index: i)
            }
            out.append(vector.map(Float.init))
        }
        return out
    }
}
