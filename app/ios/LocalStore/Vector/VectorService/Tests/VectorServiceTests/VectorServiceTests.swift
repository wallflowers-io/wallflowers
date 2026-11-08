//  VectorServiceTests.swift
//  Exercises the encapsulation: callers use only `VectorService`; the on-device
//  backend is validated through it.
//
//  - The on-device path is tested for real against NLEmbedding on the host Mac.
//    If the OS lacks the sentence model it FAILS LOUDLY (no skip) — the honest
//    behaviour. Run it on your Mac to see the production backend actually embed.
//
//  There is no remote backend to test: the Voyage embedder (which POSTed text to
//  api.voyageai.com) was deleted with the Anthropic-route cut, 26 Sep 2026. No
//  path here sends content off the device.

import XCTest
import Foundation
@testable import VectorService

final class VectorServiceTests: XCTestCase {

    // MARK: On-device backend — real NLEmbedding on the host

    func testOnDeviceEmbeddingIsSemantic() async throws {
        let service = VectorService(.onDevice())
        let texts = [
            "The cat sat on the mat.",
            "A feline rested on the rug.",
            "Quarterly revenue grew twelve percent."
        ]
        let vectors = try await service.embed(texts)

        XCTAssertEqual(vectors.count, 3)
        XCTAssertGreaterThan(service.epoch.dimension, 0, "on-device model failed to load on this host")
        for v in vectors {
            XCTAssertEqual(v.count, service.epoch.dimension)
            XCTAssertFalse(v.allSatisfy { $0 == 0 }, "a real vector must not be all zeros")
        }

        let related = Self.cosine(vectors[0], vectors[1])    // cat ~ feline
        let unrelated = Self.cosine(vectors[0], vectors[2])  // cat ~ revenue
        XCTAssertGreaterThan(related, unrelated,
                             "cat/feline should be closer than cat/revenue")
    }

    // MARK: Seam invariants

    func testEpochNamesTheOnDeviceBackend() {
        let onDevice = VectorService(.onDevice()).epoch
        XCTAssertEqual(onDevice.provider, "apple.nlembedding")
    }

    func testEmptyInputReturnsEmptyWithoutTouchingBackend() async throws {
        let service = VectorService(.onDevice())
        let vectors = try await service.embed([])
        XCTAssertTrue(vectors.isEmpty)
    }

    /// A backend that lies about its dimension is caught by the facade, not passed on.
    func testWrongDimensionIsRejected() async throws {
        let service = VectorService(embedder: FixedEmbedder(dimension: 2, vectors: [[1.0, 2.0, 3.0]]))
        do {
            _ = try await service.embed(["x"])
            XCTFail("expected a dimension mismatch")
        } catch let VectorServiceError.dimensionMismatch(expected, got, _) {
            XCTAssertEqual(expected, 2)
            XCTAssertEqual(got, 3)
        }
    }

    /// A backend that returns the wrong number of vectors is caught too.
    func testCountMismatchIsRejected() async throws {
        let service = VectorService(embedder: FixedEmbedder(dimension: 2, vectors: [[1.0, 0.0]]))
        do {
            _ = try await service.embed(["x", "y"])
            XCTFail("expected a count mismatch")
        } catch let VectorServiceError.countMismatch(expected, got) {
            XCTAssertEqual(expected, 2)
            XCTAssertEqual(got, 1)
        }
    }

    // MARK: Helpers

    private static func cosine(_ a: [Float], _ b: [Float]) -> Float {
        let dot = zip(a, b).reduce(Float(0)) { $0 + $1.0 * $1.1 }
        let na = sqrt(a.reduce(Float(0)) { $0 + $1 * $1 })
        let nb = sqrt(b.reduce(Float(0)) { $0 + $1 * $1 })
        return (na == 0 || nb == 0) ? 0 : dot / (na * nb)
    }
}

/// An in-process stub backend — the only way a test gets a non-NLEmbedding
/// embedder now that the remote backend is gone. It touches no network.
private struct FixedEmbedder: Embedder, Sendable {
    let dimension: Int
    let vectors: [[Float]]

    var epoch: EmbeddingEpoch {
        EmbeddingEpoch(provider: "test.fixed", model: "fixed", dimension: dimension,
                       metric: "cosine", normalization: "none", revision: "1")
    }

    func embed(_ texts: [String], as kind: InputKind) async throws -> [[Float]] { vectors }
}
