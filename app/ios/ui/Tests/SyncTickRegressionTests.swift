//  SyncTickRegressionTests.swift
//  Guards for the 4 Aug sync-tick investigation: the 3-second sync loop republished
//  unchanged @Published state, which re-ran the graph projection (~2N main-thread FFI
//  calls) and a full-transcript hashtag rescan on every tick. These pin the pieces of
//  the fix that are pure logic: the projection fingerprint (what makes an unchanged
//  tick free), the snapshot reshape (now a pure function over ProjectionInputs), and
//  ChatIndex's accumulated tag sets (incremental at ingest, one-time v1 backfill).

import Foundation
import Testing
import PacificCore
import PacificStore
import ArcResolverKit
@testable import WallFlowers

// MARK: - helpers

private func msg(_ text: String, gen: UInt64) -> ChatMsg {
    ChatMsg(author: "aa", gen: gen, text: text, ts: 1_000 + gen, mine: false,
            replyToAuthor: "", replyToGen: 0, replyPreview: "", reactions: [],
            receipt: 0, up: 0, down: 0, myVote: 0)
}

private func objectRow(id: String, kind: String, name: String, members: [String] = []) -> ObjectRow {
    ObjectRow(id: id, kind: kind, name: name, owner: "", creator: "", createdAt: nil, members: members,
              parent: "", parentRole: "", peer: "", link: "")
}

private func eventRow(id: String, title: String,
                      venuePlace: String = "", venueName: String = "") -> EventRow {
    EventRow(id: id, title: title, descriptor: "", startMs: 0, endMs: 0, venue: "",
             venuePlace: venuePlace, venuePlaceName: venueName,
             venueAt: venuePlace.isEmpty ? 0 : 42,
             placed: false, lat: 0, lng: 0, listed: false, priceCents: 0, currency: "",
             capacity: 0, open: false, sold: 0, overCapacity: false, listing: "",
             banner: "", bannerMime: "", photoCount: 0, hasClip: false,
             lineup: [], recurrence: "")
}

// MARK: - projection fingerprint (the steady-state gate)

struct ProjectionFingerprintTests {
    @Test func equalSnapshotsShareAFingerprint() {
        let snap = [ArcObjectSnapshot(id: "g1", kind: "group", name: "Crew",
                                      members: [(space: "space1a", name: nil)])]
        #expect(GraphService.fingerprint(snap) == GraphService.fingerprint(snap))
    }

    @Test func rosterChangeChangesTheFingerprint() {
        let before = [ArcObjectSnapshot(id: "g1", kind: "group", name: "Crew",
                                        members: [(space: "space1a", name: nil)])]
        let after = [ArcObjectSnapshot(id: "g1", kind: "group", name: "Crew",
                                       members: [(space: "space1a", name: nil),
                                                 (space: "space1b", name: "Bo")])]
        #expect(GraphService.fingerprint(before) != GraphService.fingerprint(after))
    }

    @Test func venueChangeChangesTheFingerprint() {
        let unplaced = [ArcObjectSnapshot(id: "e1", kind: "event", name: "Launch")]
        let placed = [ArcObjectSnapshot(id: "e1", kind: "event", name: "Launch",
                                        venue: (place: "p1", name: "ARC", at: 42))]
        #expect(GraphService.fingerprint(unplaced) != GraphService.fingerprint(placed))
    }

    @Test func fieldsAreSeparatedNotConcatenated() {
        // "ab"+"c" and "a"+"bc" must not collide.
        let x = [ArcObjectSnapshot(id: "ab", kind: "c", name: "")]
        let y = [ArcObjectSnapshot(id: "a", kind: "bc", name: "")]
        #expect(GraphService.fingerprint(x) != GraphService.fingerprint(y))
    }
}

// MARK: - snapshot reshape (pure over ProjectionInputs)

struct ObjectProjectionTests {
    @Test func snapshotsMapKindsRostersAndVenues() {
        let inputs = CoreService.ProjectionInputs(names: ["space1beef": "Ada"], groups: [:])
        let objects = [objectRow(id: "g1", kind: "group", name: "Crew", members: ["beef"]),
                       objectRow(id: "e1", kind: "event", name: "raw-name"),
                       objectRow(id: "f1", kind: "forum", name: "Room")]
        let events = [eventRow(id: "e1", title: "Launch", venuePlace: "p1", venueName: "ARC")]

        let snaps = ObjectProjection.snapshots(objects: objects, events: events, inputs: inputs)

        #expect(snaps.count == 2)   // the forum is episodes, not an entity
        let g = snaps[0]
        #expect(g.id == "g1" && g.kind == "group" && g.name == "Crew")
        #expect(g.members.count == 1)
        #expect(g.members[0].space == "ed25519:beef")
        // Labelled via the names map — and its key arrives in a
        // DIFFERENT rendering to the roster's, which is the point: the lookup
        // normalises, so the label survives the mismatch.
        #expect(g.members[0].name == "Ada")
        let e = snaps[1]
        #expect(e.name == "Launch")           // the folded title wins over the object row's name
        #expect(e.venue?.place == "p1")
        #expect(e.venue?.name == "ARC")
    }
}

// MARK: - ChatIndex tag sets (incremental, persisted, v1 backfill)

@MainActor
struct ChatIndexTagTests {
    private func throwaway() throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("chat-index-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeStore(_ dir: URL) throws -> LocalStore {
        try LodeLocalStore(embedder: UnionEmbedder.makeUnion(),
                           storeURL: dir.appendingPathComponent("union.lode"),
                           startCheckpointer: false)
    }

    @Test func tagsAccumulateAcrossIngests() throws {
        let dir = try throwaway()
        let idx = ChatIndex(store: try makeStore(dir),
                            stateURL: dir.appendingPathComponent("chat-index.json"))
        let first = [msg("hello #alpha", gen: 0), msg("plain", gen: 1)]
        idx.ingest(conversation: "c1", kind: .chat, msgs: first)
        #expect(idx.tags(conversation: "c1", msgs: first) == ["alpha"])

        let more = first + [msg("now #beta too", gen: 2)]
        idx.ingest(conversation: "c1", kind: .chat, msgs: more)
        #expect(idx.tags(conversation: "c1", msgs: more) == ["alpha", "beta"])
    }

    @Test func legacyCountsFileBackfillsOnceAndPersistsV2() throws {
        let dir = try throwaway()
        let stateURL = dir.appendingPathComponent("chat-index.json")
        // The v1 file: bare counts, no tags.
        try JSONEncoder().encode(["c1": 2]).write(to: stateURL)
        let store = try makeStore(dir)

        let rows = [msg("#old history", gen: 0), msg("a #tale", gen: 1)]
        let idx = ChatIndex(store: store, stateURL: stateURL)
        #expect(idx.tags(conversation: "c1", msgs: rows) == ["old", "tale"])

        // The backfill persisted as v2: a fresh index reads the set with NO transcript.
        let reloaded = ChatIndex(store: store, stateURL: stateURL)
        #expect(reloaded.tags(conversation: "c1", msgs: []) == ["old", "tale"])
    }
}
