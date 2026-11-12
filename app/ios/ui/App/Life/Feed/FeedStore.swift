//  FeedStore.swift
//  Assembly + persistence for the Active Scroll Feed. One refresh per syncTick,
//  equality-gated publish (the 4 Aug storm rule), seen-ledger persisted as JSON.

import SwiftUI

@MainActor
final class FeedStore: ObservableObject {
    @Published private(set) var rows: [FeedItem] = []
    @Published private(set) var ledger = FeedSeenLedger()

    private static var ledgerURL: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("feed-seen.json")
    }

    init() {
        if let data = try? Data(contentsOf: Self.ledgerURL),
           let stored = try? JSONDecoder().decode(StoredLedger.self, from: data) {
            ledger = FeedSeenLedger(firstSeen: stored.firstSeen, lastVisitMs: stored.lastVisitMs)
        }
    }

    /// Snapshot the published reads, assemble + notch-filter, publish only on change.
    func refresh(core: CoreService, query: LifeQuery) {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let index = core.index
        let names = index.names
        let assembled = FeedAssembler.rows(events: core.events, listings: core.listings,
                                           things: core.things, posts: core.posts, exteriors: core.exteriors,
                                           outsides: core.outsides, nowMs: now,
                                           makerOf: index.maker(of:),
                                           createdOf: index.createdAt(of:),
                                           nameFor: { names[$0] ?? String($0.prefix(8)) })
        let filtered = FeedAssembler.filter(assembled, locked: query.locked)
        ledger.observe(filtered, nowMs: now)
        if filtered != rows { rows = filtered }
        persist()
    }

    /// Leaving the feed stamps the visit — the next arrival draws the band above it.
    func markVisited() {
        ledger.visit(nowMs: Int64(Date().timeIntervalSince1970 * 1000))
        persist()
    }

    private func persist() {
        let stored = StoredLedger(firstSeen: ledger.firstSeen, lastVisitMs: ledger.lastVisitMs)
        if let data = try? JSONEncoder().encode(stored) {
            try? data.write(to: Self.ledgerURL, options: .atomic)
        }
    }

    private struct StoredLedger: Codable {
        let firstSeen: [String: Int64]
        let lastVisitMs: Int64
    }
}
