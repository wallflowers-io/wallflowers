//  KoreanUnionEmbedderTests.swift
//  The Korean union profile, proven
//  on the REAL stack — the contextual embedder Apple actually ships for Korean,
//  the persisted language authority, and the language-change store migration.
//
//  The measured failure these exist to prevent: Apple's en sentence model embeds
//  Korean rows into a degenerate ~0.97-cosine band where an UNRELATED pair can
//  outscore a related one (measured 0.968 vs 0.966 on this fixture set), sinking
//  search, families and the orb together.
//
//  The contextual model's assets are an on-demand OS download, so the tests that
//  need the live model are `.enabled(if:)`-gated on their presence — a machine
//  without them skips them VISIBLY rather than faking a pass with a stub.

import Testing
import Foundation
import NaturalLanguage
@testable import PacificStore

/// Whether this machine holds the CJK contextual assets (an OS download).
private var koreanAssetsAvailable: Bool {
    if #available(iOS 17.0, macOS 14.0, *) {
        return ContextualUnionEmbedder.assetsAvailable(for: .korean)
    }
    return false
}

struct KoreanUnionEmbedderTests {

    // MARK: - Contract

    @Test func modelIdentityFollowsTheLanguage() {
        #expect(UnionConfig.modelIdentity(for: .english) == UnionConfig.modelIdentity)
        #expect(UnionConfig.modelIdentity(for: .english) == "apple.NLEmbedding.sentence.en")
        #expect(UnionConfig.modelIdentity(for: .korean) == "apple.NLContextualEmbedding.mean.ko")
    }

    @Test(.enabled(if: koreanAssetsAvailable))
    func koreanEmbedderMatchesTheUnionContract() throws {
        let embedder = try UnionEmbedder.makeUnion(language: .korean)
        #expect(embedder.dimension == UnionConfig.vectorDimension)
        #expect(embedder.modelIdentity == UnionConfig.modelIdentity(for: .korean))
    }

    // MARK: - Non-degeneracy (the F-3 acceptance test)

    @Test(.enabled(if: koreanAssetsAvailable))
    func koreanSentencesEmbedNonDegenerately() throws {
        let embedder = try UnionEmbedder.makeUnion(language: .korean)
        // Two related sentences (going to a music festival / planning to see a
        // concert) and one unrelated (a mackerel-braising recipe).
        let vectors = try embedder.embed(texts: [
            "내일 친구들과 함께 서울에서 열리는 음악 축제에 갈 예정이야",
            "다음 주에 콘서트 공연을 보러 가려고 계획하고 있어",
            "고등어 조림 요리법은 무와 간장이 필요하다",
        ])

        // Non-degenerate: real unit vectors, not zeros and not one collapsed ray.
        for v in vectors {
            let norm = v.map { $0 * $0 }.reduce(0, +).squareRoot()
            #expect(abs(norm - 1) < 1e-3)
        }

        func cosine(_ a: [Float], _ b: [Float]) -> Float { zip(a, b).map(*).reduce(0, +) }
        let related = cosine(vectors[0], vectors[1])
        let unrelated = cosine(vectors[0], vectors[2])
        // The ordering the en model measurably got WRONG on these very texts —
        // and with real separation, not the en model's collapsed ~0.97 band.
        #expect(related > unrelated)
        #expect(related - unrelated > 0.05)
    }

    // MARK: - Language-change migration (the reindexAll path)

    @Test(.enabled(if: koreanAssetsAvailable))
    func migrateStoreRebuildsUnderTheKoreanModelWithEverythingRetained() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("pacific-union-migrate-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let url = root.appendingPathComponent("union.lode", isDirectory: true)

        // A store born ENGLISH, holding mixed rows (the Seoul reality).
        let en = try LodeLocalStore(
            embedder: UnionEmbedder.makeUnion(), storeURL: url, startCheckpointer: false)
        try await en.upsert(
            id: "ko-festival", text: "내일 서울에서 열리는 음악 축제에 갈 예정이야",
            metadata: Metadata.document(source: .note, objectId: "ko-festival").build())
        try await en.upsert(
            id: "ko-recipe", text: "고등어 조림 요리법은 무와 간장이 필요하다",
            metadata: Metadata.document(source: .note, objectId: "ko-recipe").build())
        try await en.upsert(
            id: "en-ada", text: "Ada Lovelace, mathematician",
            metadata: Metadata.document(source: .note, objectId: "en-ada").build())
        _ = try await en.checkpointNow()
        await en.close()

        // The language change: rebuild aside + swap, from retained text.
        let koEmbedder = try UnionEmbedder.makeUnion(language: .korean)
        let rebuilt = try LodeLocalStore.migrateStore(
            to: .korean, embedder: koEmbedder, storeURL: url)
        #expect(rebuilt)
        // Idempotent: a second reconcile is a no-op.
        let again = try LodeLocalStore.migrateStore(
            to: .korean, embedder: koEmbedder, storeURL: url)
        #expect(!again)

        // The migrated store fails CLOSED against the old embedder…
        #expect(throws: (any Error).self) {
            _ = try LodeLocalStore(
                embedder: UnionEmbedder.makeUnion(), storeURL: url, startCheckpointer: false)
        }

        // …and opens under the Korean profile with nothing lost: every document,
        // every retained text, every metadata row.
        let ko = try LodeLocalStore(
            embedder: koEmbedder, language: .korean, storeURL: url, startCheckpointer: false)
        let docs = try await ko.listDocuments(type: nil)
        #expect(docs.map(\.id).sorted() == ["en-ada", "ko-festival", "ko-recipe"])
        let note = try await ko.get(id: "ko-festival")
        #expect(note?.text == "내일 서울에서 열리는 음악 축제에 갈 예정이야")
        #expect(Metadata.source(note?.metadata ?? [:]) == .note)

        // And Korean search now ranks the related row first — the on-language
        // vectors doing the work the en model measurably could not.
        let hits = try await ko.search("음악 공연 콘서트", k: 3)
        #expect(hits.first?.id == "ko-festival")

        await ko.close()
    }
}

// MARK: - The ONE language authority

struct UnionLanguageSettingTests {

    private func scratchDefaults() throws -> (UserDefaults, String) {
        let suite = "pacific-union-language-tests-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        return (defaults, suite)
    }

    @Test func defaultsToEnglish() throws {
        let (defaults, suite) = try scratchDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        #expect(UnionLanguageSetting.load(from: defaults) == .english)
    }

    @Test func roundTripsKorean() throws {
        let (defaults, suite) = try scratchDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        UnionLanguageSetting.save(.korean, to: defaults)
        #expect(UnionLanguageSetting.load(from: defaults) == .korean)
        #expect(defaults.string(forKey: UnionLanguageSetting.defaultsKey) == "ko")
    }

    @Test func unrecognizedValueReadsAsTheShippedDefault() throws {
        let (defaults, suite) = try scratchDefaults()
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set("klingon", forKey: UnionLanguageSetting.defaultsKey)
        #expect(UnionLanguageSetting.load(from: defaults) == .english)
    }
}
