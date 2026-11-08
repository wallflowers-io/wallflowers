//  ProposalPipelineTests.swift
//  The priority path of the 8-category retarget: what the graph holds lands as
//  DEDUPED mint suggestions (ProposalStore.rebuild is what the notification tray and
//  the swipe deck both read). Asserted per category, plus the three dedup gates:
//  projected objects never propose, minted names never re-propose, EVENT never
//  proposes at all — a commitment in the graph lands directly in the EVENTS feed, so
//  there is nothing to mint.

import XCTest
@testable import WallFlowers
@testable import ArcResolverKit

@MainActor
final class ProposalPipelineTests: XCTestCase {

    private func entity(_ id: String, _ type: String, _ name: String,
                        object: Bool = false) -> ArcEntity {
        ArcEntity(id: id, entityType: type, label: name, name: name, summary: nil,
                  source: object ? "object" : nil,
                  validAt: nil, invalidAt: nil, createdAt: 1)
    }

    /// A store persisting into a fresh temp dir — the tests run inside the app's own
    /// container (TEST_HOST), so the default directory can hold REAL sourced venue
    /// proposals from an earlier live run, and these counts must not read them.
    private func makeStore() -> ProposalStore {
        ProposalStore(directory: FileManager.default.temporaryDirectory
            .appendingPathComponent("proposal-tests-\(UUID().uuidString)", isDirectory: true))
    }

    func test_allEightCategories_landAsDedupedMintSuggestions() {
        let store = makeStore()
        store.rebuild(resolved: [
            entity("e1", "Event", "Basement Set"),
            entity("p1", "Person", "Mira"),
            entity("g1", "Group", "Regen Collective"),
            entity("l1", "Place", "Gallery 4"),
            entity("i1", "Inventory", "Spare sub"),
            entity("s1", "Skill", "Rigging"),
            entity("t1", "Trade", "Plinths"),
            entity("j1", "Job", "Two riggers, Sat night"),
            entity("x1", "Thing", "Toyota"),                          // pre-retarget roster
            entity("d1", "Person", "Warehouse 12 Ops", object: true), // projected GroupObject
            entity("m1", "Person", "Hope"),                           // already minted, by name
        ], mintedNames: ["hope"])

        let byId = Dictionary(uniqueKeysWithValues: store.pending.map { ($0.id, $0) })

        // Seven of the eight categories propose, each rendered as its own LifeType.
        XCTAssertEqual(byId["p1"]?.subject.type, .person)
        XCTAssertEqual(byId["g1"]?.subject.type, .group)
        XCTAssertEqual(byId["l1"]?.subject.type, .place)
        XCTAssertEqual(byId["i1"]?.subject.type, .inventory)
        XCTAssertEqual(byId["s1"]?.subject.type, .skill)
        XCTAssertEqual(byId["t1"]?.subject.type, .trade)
        XCTAssertEqual(byId["j1"]?.subject.type, .job)
        // Legacy graphs keep proposing under the old roster's Thing.
        XCTAssertEqual(byId["x1"]?.subject.type, .thing)

        // The gates.
        XCTAssertNil(byId["e1"], "an extracted Event needs no mint — the feed lists it directly")
        XCTAssertNil(byId["d1"], "a projected object IS minted — nothing to propose")
        XCTAssertNil(byId["m1"], "a name already minted (any kind) must not be re-proposed")
        XCTAssertEqual(store.pending.count, 8)
    }

    func test_rejectSilences_thisSessionOnly_andMintedNamesWin() {
        let store = makeStore()
        let rigging = entity("s1", "Skill", "Rigging")
        store.rebuild(resolved: [rigging], mintedNames: [])
        XCTAssertEqual(store.pending.count, 1)

        // A reject drops it from pending and keeps it out of the NEXT rebuild.
        store.decide(store.pending[0], .reject, core: CoreService())
        XCTAssertTrue(store.pending.isEmpty)
        store.rebuild(resolved: [rigging], mintedNames: [])
        XCTAssertTrue(store.pending.isEmpty, "a rejected id must not be re-asked this session")

        // Once the name exists as a minted object, it never proposes again — the
        // durable half of dedup, independent of the session's dismissals.
        store.rebuild(resolved: [entity("s2", "Skill", "Rigging")], mintedNames: ["rigging"])
        XCTAssertTrue(store.pending.isEmpty)
    }
}
