//  ModeStore.swift
//  Modes Config — named filter configurations saved AS FILES (FEED-TRANSMISSION S14).
//  A mode is the notch state (locked + bound) plus the rail order. Files live in
//  Documents/Modes/<name>.json; "active" is the autosaved last configuration that
//  a rail drag-right restores.

import SwiftUI

struct FeedMode: Codable, Equatable {
    var locked: [String]
    var bound: [String: String]
    var order: [String]

    @MainActor
    static func capture(_ query: LifeQuery, order: [EventCategory]) -> FeedMode {
        FeedMode(locked: query.terms.map(\.category.rawValue),
                 bound: Dictionary(uniqueKeysWithValues:
                    query.terms.compactMap { t in t.entityID.map { (t.category.rawValue, $0) } }),
                 order: order.map(\.rawValue))
    }

    @MainActor
    func apply(to query: LifeQuery) {
        query.clear()
        for raw in locked {
            guard let c = EventCategory(rawValue: raw) else { continue }
            query.toggleLock(c)
            if let e = bound[raw] { query.bind(e, in: c) }
        }
    }

    var railOrder: [EventCategory] { order.compactMap(EventCategory.init(rawValue:)) }
}

@MainActor
final class ModeStore: ObservableObject {
    @Published private(set) var names: [String] = []

    private static var dir: URL {
        let d = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Modes", isDirectory: true)
        try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
        return d
    }

    init() { reload() }

    func reload() {
        names = ((try? FileManager.default.contentsOfDirectory(atPath: Self.dir.path)) ?? [])
            .filter { $0.hasSuffix(".json") }
            .map { String($0.dropLast(5)) }
            .sorted()
    }

    func save(_ mode: FeedMode, as name: String) {
        guard let data = try? JSONEncoder().encode(mode) else { return }
        try? data.write(to: Self.dir.appendingPathComponent("\(name).json"), options: .atomic)
        reload()
    }

    func load(_ name: String) -> FeedMode? {
        guard let data = try? Data(contentsOf: Self.dir.appendingPathComponent("\(name).json"))
        else { return nil }
        return try? JSONDecoder().decode(FeedMode.self, from: data)
    }

    func delete(_ name: String) {
        try? FileManager.default.removeItem(at: Self.dir.appendingPathComponent("\(name).json"))
        reload()
    }

    /// The autosaved last-active configuration (rail drag-right's target).
    func saveActive(_ mode: FeedMode) { save(mode, as: "active") }
    func active() -> FeedMode? { load("active") }
}
