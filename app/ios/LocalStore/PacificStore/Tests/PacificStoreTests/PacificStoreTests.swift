import XCTest
@testable import PacificStore

final class PacificStoreTests: XCTestCase {

    // MARK: - §2 union config contract

    func testUnionConfigMatchesContract() {
        // The ACTIVE profile: Apple's on-device NL sentence embedder. BGE remains the
        // §2 target (asserted below) and lands once the ONNX runtime is provisioned.
        XCTAssertEqual(UnionConfig.vectorDimension, 512)
        XCTAssertEqual(UnionConfig.modelIdentity, "apple.NLEmbedding.sentence.en")
        // The BGE target the store migrates to (via `reindexAll`) after provisioning.
        XCTAssertEqual(UnionConfig.bgeVectorDimension, 768)
        XCTAssertEqual(UnionConfig.bgeModelIdentity, "BAAI/bge-base-en-v1.5")
        XCTAssertTrue(UnionConfig.storeText)
        XCTAssertTrue(UnionConfig.indexText)
        XCTAssertEqual(UnionConfig.maxSequenceLength, 512)
        XCTAssertEqual(UnionConfig.chunkCharacterLimit, 2000)
        XCTAssertEqual(UnionConfig.commitMode, .wal)
        XCTAssertEqual(UnionConfig.durability, .fsync)
        XCTAssertEqual(UnionConfig.pooling, .cls)
        XCTAssertEqual(
            UnionConfig.queryPrefix,
            "Represent this sentence for searching relevant passages: ")
    }

    func testEmbeddingPresetHonoursSequenceLength() {
        // The whole point of building the preset by hand: 512, not LodeDBCore's 256.
        XCTAssertEqual(UnionConfig.embeddingPreset.maxSequenceLength, 512)
        // The preset describes the BGE TARGET pipeline, not the active NL profile.
        XCTAssertEqual(UnionConfig.embeddingPreset.dimension, UnionConfig.bgeVectorDimension)
        XCTAssertEqual(UnionConfig.embeddingPreset.modelIdentity, UnionConfig.bgeModelIdentity)
        XCTAssertEqual(UnionConfig.embeddingPreset.pooling, .cls)
    }

    /// The load-bearing agreement: `LodeLocalStore.init` fails closed when the embedder's
    /// width or identity disagrees with the contract, and the index is CREATED at
    /// `vectorDimension`. So the active constants must match the embedder we actually
    /// ship — this catches a platform change to Apple's NL model before it becomes a
    /// launch-time crash.
    func testUnionEmbedderMatchesActiveContract() throws {
        let embedder = try UnionEmbedder.makeUnion()
        XCTAssertEqual(embedder.dimension, UnionConfig.vectorDimension)
        XCTAssertEqual(embedder.modelIdentity, UnionConfig.modelIdentity)
    }

    func testLodeStoreOptionsBridge() {
        let opts = UnionConfig.lodeStoreOptions()
        XCTAssertEqual(opts.commitMode, .wal)
        XCTAssertEqual(opts.durability, .fsync)
        XCTAssertEqual(opts.chunkCharacterLimit, 2000)
        XCTAssertTrue(opts.storeText)
        XCTAssertTrue(opts.indexText)
        XCTAssertFalse(opts.acquireWriterLock) // multi-producer union default
    }

    func testStorePathEndsWithUnionLode() throws {
        let path = try UnionConfig.storePath()
        XCTAssertTrue(path.hasSuffix("pacific/union.lode"), path)
    }

    // MARK: - §3 metadata schema

    func testDeterministicChunkId() {
        XCTAssertEqual(Metadata.chunkId(objectId: "note-1", chunkIndex: 3), "note-1#3")
    }

    func testNodeRoundTrips() {
        let row = Metadata.node(
            source: .entity, objectId: "e42", type: .person, groupObjectId: "g1"
        ).build()

        XCTAssertEqual(Metadata.source(row), .entity)
        XCTAssertEqual(Metadata.objectId(row), "e42")
        XCTAssertEqual(Metadata.chunkId(row), "e42#0")
        XCTAssertEqual(Metadata.kind(row), .node)
        XCTAssertEqual(Metadata.type(row), .person)
        XCTAssertEqual(Metadata.groupObjectId(row), "g1")
        XCTAssertNil(Metadata.src(row))
        XCTAssertNil(Metadata.dst(row))
    }

    func testEdgeRoundTrips() {
        let row = Metadata.edge(source: .project, objectId: "edge-1", src: "a", dst: "b").build()
        XCTAssertEqual(Metadata.kind(row), .edge)
        XCTAssertEqual(Metadata.src(row), "a")
        XCTAssertEqual(Metadata.dst(row), "b")
        XCTAssertNil(Metadata.type(row))
    }

    func testDocumentOmitsOptionalKeys() {
        let row = Metadata.document(source: .note, objectId: "n1").build()
        XCTAssertNil(row[Metadata.Key.type])
        XCTAssertNil(row[Metadata.Key.groupObjectId])
        XCTAssertEqual(Metadata.kind(row), .doc)
    }

    // MARK: - LodeLocalStore (the real engine, on a throwaway store)

    /// Opens the REAL store on a fresh temp directory, with the same options, contract and
    /// embedder the app ships — so these tests exercise production behaviour instead of a
    /// hand-rolled double that can drift from it. The background fold loop is off for
    /// determinism; reads still see every write because `LodeLocalStore` refreshes the WAL
    /// tail before each one. Returns the store plus the root dir to delete afterwards.
    private func makeTempStore() throws -> (store: LodeLocalStore, root: URL) {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("pacific-store-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let store = try LodeLocalStore(
            embedder: UnionEmbedder.makeUnion(),
            storeURL: root.appendingPathComponent("union.lode", isDirectory: true),
            startCheckpointer: false
        )
        return (store, root)
    }

    func testLodeLocalStoreUpsertSearchRemove() async throws {
        let (store, root) = try makeTempStore()
        defer { try? FileManager.default.removeItem(at: root) }

        let meta = Metadata.node(source: .entity, objectId: "p1", type: .person).build()
        try await store.upsert(id: "p1", text: "Ada Lovelace, mathematician", metadata: meta)

        let byText = try await store.search("lovelace", k: 5)
        XCTAssertEqual(byText.map(\.id), ["p1"])

        let people = try await store.listDocuments(type: .person)
        XCTAssertEqual(people.map(\.id), ["p1"])
        let orgs = try await store.listDocuments(type: .org)
        XCTAssertTrue(orgs.isEmpty)

        let fetched = try await store.get(id: "p1")
        XCTAssertEqual(fetched?.text, "Ada Lovelace, mathematician")

        try await store.remove(id: "p1")
        let afterRemove = try await store.get(id: "p1")
        XCTAssertNil(afterRemove)

        await store.close()
    }

    func testLodeLocalStoreScopeFilter() async throws {
        let (store, root) = try makeTempStore()
        defer { try? FileManager.default.removeItem(at: root) }

        try await store.upsert(
            id: "s1", text: "scoped",
            metadata: Metadata.document(source: .forum, objectId: "s1", groupObjectId: "g1").build())
        try await store.upsert(
            id: "s2", text: "scoped",
            metadata: Metadata.document(source: .forum, objectId: "s2", groupObjectId: "g2").build())
        try await store.upsert(
            id: "local", text: "scoped",
            metadata: Metadata.document(source: .note, objectId: "local").build())

        // Membership IS access (§3): the engine-side filter admits the authorised group
        // plus un-scoped local rows, and excludes everything else — before top-k.
        let scoped = try await store.search("scoped", k: 10, scope: .groups(["g1"]))
        XCTAssertEqual(scoped.map(\.id).sorted(), ["local", "s1"])

        await store.close()
    }

    /// The durability guarantee the whole swap exists for: a row written through one
    /// store handle is still there after that handle is closed and the store reopened —
    /// i.e. it genuinely hit disk, not just the in-memory overlay.
    func testLodeLocalStoreSurvivesReopen() async throws {
        let (store, root) = try makeTempStore()
        defer { try? FileManager.default.removeItem(at: root) }
        let url = root.appendingPathComponent("union.lode", isDirectory: true)

        try await store.upsert(
            id: "p1", text: "Ada Lovelace, mathematician",
            metadata: Metadata.node(source: .entity, objectId: "p1", type: .person).build())
        _ = try await store.checkpointNow()   // fold the WAL into a committed generation
        await store.close()

        let reopened = try LodeLocalStore(
            embedder: UnionEmbedder.makeUnion(), storeURL: url, startCheckpointer: false)
        let people = try await reopened.listDocuments(type: .person)
        XCTAssertEqual(people.map(\.id), ["p1"])
        let fetched = try await reopened.get(id: "p1")
        XCTAssertEqual(fetched?.text, "Ada Lovelace, mathematician")

        await reopened.close()
    }

    // MARK: - LifeSource stub

    func testStubLifeSource() async throws {
        let person = Document(id: "p1", metadata: Metadata.node(source: .entity, objectId: "p1", type: .person).build())
        let life = StubLifeSource(
            entities: [.person: [person]],
            edges: ["p1": [Edge(id: "e1", src: "p1", dst: "o1")]]
        )
        let persons = try await life.entities(ofType: .person)
        XCTAssertEqual(persons.map(\.id), ["p1"])
        let projects = try await life.projects()
        XCTAssertTrue(projects.isEmpty)
        let related = try await life.related(to: "p1")
        XCTAssertEqual(related.map(\.dst), ["o1"])
    }

    // MARK: - LocalStore-backed LifeSource (WS-F)

    func testLocalStoreLifeSourceProjectsNodesAndEdges() async throws {
        // Mirror exactly what the entity producer writes: an entity is a node row, each
        // of its links is an edge row (src/dst), all in the ONE union store.
        let (store, root) = try makeTempStore()
        defer { try? FileManager.default.removeItem(at: root) }
        try await store.upsert(
            id: "p1", text: "Ada",
            metadata: Metadata.node(source: .entity, objectId: "p1", type: .person).build())
        try await store.upsert(
            id: "o1", text: "Analytical Engine Co",
            metadata: Metadata.node(source: .entity, objectId: "o1", type: .org).build())
        try await store.upsert(
            id: "p1->o1", text: "Ada → Analytical Engine Co",
            metadata: Metadata.edge(source: .entity, objectId: "p1->o1", src: "p1", dst: "o1").build())

        let life = LocalStoreLifeSource(store: store)

        // entities(ofType:) enumerates by §3 `type` — nodes only, no edges leak in.
        let people = try await life.entities(ofType: .person)
        XCTAssertEqual(people.map(\.id), ["p1"])
        let orgs = try await life.entities(ofType: .org)
        XCTAssertEqual(orgs.map(\.id), ["o1"])

        // projects() is the project-typed enumeration — empty here.
        let projects = try await life.projects()
        XCTAssertTrue(projects.isEmpty)

        // related(to:) selects the edges incident from an id and reads back src/dst.
        let related = try await life.related(to: "p1")
        XCTAssertEqual(related.map(\.dst), ["o1"])
        XCTAssertEqual(related.first?.src, "p1")
        // A node id with no outgoing edges is a genuinely-empty relation, not an error.
        let unrelated = try await life.related(to: "o1")
        XCTAssertTrue(unrelated.isEmpty)

        await store.close()
    }
}

// MARK: - CLIP (real model, no mocks)

import CoreGraphics
import CoreML

/// Exercises the ACTUAL MobileCLIP towers through `CLIPEmbedder`. Skipped (not failed) when
/// the weights are unprovisioned, since they are deliberately not committed — but when they
/// are present these run the real model, because a mocked embedder proves nothing about
/// whether text and images actually land in the same space.
final class CLIPEmbedderTests: XCTestCase {

    private func solid(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat) -> CGImage {
        let ctx = CGContext(data: nil, width: 256, height: 256, bitsPerComponent: 8, bytesPerRow: 0,
                            space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue)!
        ctx.setFillColor(red: r, green: g, blue: b, alpha: 1)
        ctx.fill(CGRect(x: 0, y: 0, width: 256, height: 256))
        return ctx.makeImage()!
    }

    private func makeEmbedder() throws -> CLIPEmbedder? {
        do { return try CLIPEmbedder() }
        catch CLIPEmbedderError.notProvisioned(let what) {
            throw XCTSkip("CLIP weights not provisioned (\(what)) — see CLIPModels/README.md")
        }
    }

    /// Both towers emit unit vectors of the shared width. If this drifts, every similarity
    /// downstream is silently mis-scaled.
    func testBothTowersEmitUnitVectorsOfTheSharedDimension() throws {
        guard let clip = try makeEmbedder() else { return }
        for v in [try clip.embed(image: solid(1, 0, 0)), try clip.embed(query: "a red square")] {
            XCTAssertEqual(v.count, CLIPEmbedder.dimension)
            XCTAssertEqual(sqrt(v.reduce(0) { $0 + $1 * $1 }), 1.0, accuracy: 1e-3,
                           "vectors must be L2-normalised so a dot product is cosine similarity")
        }
    }

    /// The whole point: a TEXT query must rank the matching IMAGE higher. This is what fails
    /// if the BPE tokenizer, the 77-token packing, or the image preprocessing is wrong — none
    /// of which throw on their own, they just quietly return a vector for the wrong thing.
    func testATextQueryRanksTheMatchingImageHigher() throws {
        guard let clip = try makeEmbedder() else { return }
        let red = try clip.embed(image: solid(0.9, 0.05, 0.05))
        let blue = try clip.embed(image: solid(0.05, 0.05, 0.9))
        func cosine(_ a: [Float], _ b: [Float]) -> Float { zip(a, b).reduce(0) { $0 + $1.0 * $1.1 } }

        let redQuery = try clip.embed(query: "a solid red image")
        XCTAssertGreaterThan(cosine(redQuery, red), cosine(redQuery, blue),
                             "\"red\" must prefer the red image — cross-modal alignment is broken")

        let blueQuery = try clip.embed(query: "a solid blue image")
        XCTAssertGreaterThan(cosine(blueQuery, blue), cosine(blueQuery, red),
                             "\"blue\" must prefer the blue image — cross-modal alignment is broken")
    }
}
