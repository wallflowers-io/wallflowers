//  BlockStore.swift
//  The device-local blocklist — who this device refuses to see.
//
//  Blocking is a LOCAL act with real, observable enforcement: the one seam every people
//  surface reads (`CoreService.connectionRows()`) drops blocked peers by default, so their
//  DM leaves the chat list, they leave PEOPLE and every picker, and their relay tag is no
//  longer watched for wakes. It is not a protocol op —
//  the MLS connection still exists and their deltas still fold — so ledger-reading paths
//  (the ticket wallet, invite folds, name maps) ask for `includeBlocked: true`: a block
//  hides a person, it does not rewrite history you were part of.
//
//  Mutations are main-actor (they come from taps); `contains` is readable from any thread
//  because `connectionRows()` runs inside detached FFI passes — hence the locked mirror.

import Foundation

final class BlockStore: ObservableObject, @unchecked Sendable {
    static let shared = BlockStore()
    private static let key = "pacific.blocked.peers"

    /// The blocked peer ids (space addresses), for SwiftUI. Main-thread reads only.
    @Published private(set) var blocked: Set<String>

    /// The any-thread copy `contains` reads — kept in step under the lock.
    private var mirror: Set<String>
    private let lock = NSLock()

    private init() {
        let set = Set(UserDefaults.standard.stringArray(forKey: Self.key) ?? [])
        blocked = set
        mirror = set
    }

    func contains(_ peer: String) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return mirror.contains(peer)
    }

    @MainActor func block(_ peer: String)   { update { $0.insert(peer) } }
    @MainActor func unblock(_ peer: String) { update { $0.remove(peer) } }

    @MainActor private func update(_ change: (inout Set<String>) -> Void) {
        var set = blocked
        change(&set)
        guard set != blocked else { return }
        blocked = set
        lock.lock(); mirror = set; lock.unlock()
        UserDefaults.standard.set(Array(set).sorted(), forKey: Self.key)
    }
}
