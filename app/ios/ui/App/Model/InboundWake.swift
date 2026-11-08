//  InboundWake.swift
//  What happens AFTER the push — turning a content-free wake into a real notification.
//
//  THE DIVISION OF LABOUR (RINGFENCE.txt §6). The waker cannot decrypt, so the push it sends
//  says NOTHING: a generic "New message" alert with `content-available` beside it. That is not
//  a limitation to work around, it is the property that keeps the wake tier blind. The APP does
//  the real work: woken via the `remote-notification` background mode it SYNCS, decrypts in the
//  one process that owns the MLS ratchet, and raises a LOCAL notification carrying the real
//  sender and preview.
//
//  WHY NOT THE NOTIFICATION SERVICE EXTENSION. Two processes touching the MLS ratchet risk
//  PERMANENT key divergence, and core state lives in the app sandbox rather than an App Group
//  (NotificationService.swift:19-32). That blocker stands; nothing here weakens it.
//
//  WHAT THIS CANNOT DO, said plainly because the alternative is a bug report nobody can explain:
//  a FORCE-QUIT app is not launched by a `content-available` push. iOS delivers only the generic
//  alert the waker sent, and this code never runs. That is an iOS rule, not a defect here.
//
//  DEDUPE IS THE WHOLE CORRECTNESS PROBLEM. The core's fold is idempotent, so every sync
//  re-presents the SAME messages; without a durable seen-set the user would be notified again
//  on every wake, every foreground, every tick. So the seen-set is persisted (the VenueProposer
//  pattern) and — critically — PRIMED on first use, or a device that installs this build would
//  be notified once for every message it has ever received.

import Foundation
import SwiftUI
import UIKit
import UserNotifications
import PacificCore

// MARK: - The seen-set

/// Message keys this device has already raised a notification for. Bounded and insertion-
/// ordered: an unbounded set grows forever on a chatty account, and a plain `Set` gives no
/// answer to "which one do I drop".
struct InboundSeen: Equatable {
    private(set) var order: [String] = []
    private(set) var keys: Set<String> = []
    /// False until the first pass has absorbed the existing backlog. The distinction between
    /// "never looked" and "looked, nothing new" is the difference between a silent install and
    /// a hundred banners on first launch.
    private(set) var primed = false

    var cap = 2000

    init(order: [String] = [], primed: Bool = false) {
        self.primed = primed
        for k in order { insert(k) }
    }

    /// Take these as already-handled WITHOUT reporting them as new. The first-run path.
    mutating func absorb(_ incoming: [String]) {
        for k in incoming { insert(k) }
        primed = true
    }

    /// The subset of `incoming` never seen before, in the order given — and marks them seen.
    /// Before priming this returns nothing and absorbs everything, by design.
    mutating func fresh(_ incoming: [String]) -> [String] {
        guard primed else { absorb(incoming); return [] }
        var new: [String] = []
        for k in incoming where !keys.contains(k) {
            insert(k)
            new.append(k)
        }
        return new
    }

    private mutating func insert(_ k: String) {
        guard !keys.contains(k) else { return }
        keys.insert(k)
        order.append(k)
        while order.count > cap {
            keys.remove(order.removeFirst())
        }
    }
}

/// The durable half — same shape as the VenueProposer's seen-set, so there is one persistence
/// idiom for "a question already answered on this device".
struct InboundSeenStore {
    private let defaults: UserDefaults
    private static let orderKey = "pacific.wake.seen"
    private static let primedKey = "pacific.wake.seen.primed"

    init(defaults: UserDefaults = .standard) { self.defaults = defaults }

    func load() -> InboundSeen {
        InboundSeen(order: defaults.stringArray(forKey: Self.orderKey) ?? [],
                    primed: defaults.bool(forKey: Self.primedKey))
    }

    func save(_ seen: InboundSeen) {
        defaults.set(seen.order, forKey: Self.orderKey)
        defaults.set(seen.primed, forKey: Self.primedKey)
    }
}

// MARK: - What a wake found

/// One inbound message worth telling the user about, already resolved to the sender and the
/// route a tap must take. Built as a value so the harvest is testable without a live core.
struct InboundHit: Equatable {
    /// The conversation this belongs to — the aggregation key AND the notification identifier,
    /// so a second message in the same chat REPLACES the banner rather than stacking.
    let conversation: String
    /// Display title: the peer's name for a DM, the room's name for a group.
    let title: String
    let body: String
    let ts: UInt64
    /// The `pacificOpenDM` / `pacificOpenRoom` userInfo pair that routes a tap.
    let routeKey: String
    let routeValue: String
    /// Stable per-message identity for the seen-set: conversation + author + generation. The
    /// core's log is append-only, so (author, gen) means the same message forever.
    let key: String
}

/// One conversation's transcript as harvested from the core, in the shape the pure function
/// below consumes.
struct InboundScan {
    let conversation: String
    let title: String
    let routeKey: String
    let routeValue: String
    /// (author, gen, text, ts, mine) — the fields of `ChatMsg` this path actually uses. Taken
    /// as a tuple rather than `ChatMsg` so tests need no FFI type.
    let messages: [(author: String, gen: UInt64, text: String, ts: UInt64, mine: Bool)]
}

enum InboundWakeHarvest {
    /// Every INBOUND message across the scanned conversations, newest last, as candidate hits.
    /// Own messages are excluded — a device notifying itself about what it just sent is the
    /// classic multi-device bug.
    static func hits(_ scans: [InboundScan]) -> [InboundHit] {
        scans.flatMap { scan in
            scan.messages.filter { !$0.mine }.map { m in
                InboundHit(conversation: scan.conversation,
                           title: scan.title,
                           body: preview(m.text),
                           ts: m.ts,
                           routeKey: scan.routeKey,
                           routeValue: scan.routeValue,
                           key: "\(scan.conversation):\(m.author):\(m.gen)")
            }
        }
    }

    /// One banner per conversation — the newest message, with a count when more than one landed
    /// in the same wake. A wake after a long offline stretch must not fire fifty banners.
    static func banners(_ new: [InboundHit]) -> [(hit: InboundHit, more: Int)] {
        var byConversation: [String: [InboundHit]] = [:]
        var order: [String] = []
        for h in new {
            if byConversation[h.conversation] == nil { order.append(h.conversation) }
            byConversation[h.conversation, default: []].append(h)
        }
        return order.compactMap { id in
            guard let group = byConversation[id], let newest = group.max(by: { $0.ts < $1.ts })
            else { return nil }
            return (newest, group.count - 1)
        }
    }

    /// A preview, never the whole message: a notification is a summons, not a reader.
    static func preview(_ text: String, limit: Int = 140) -> String {
        let flat = text.replacingOccurrences(of: "\n", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard flat.count > limit else { return flat }
        return String(flat.prefix(limit)).trimmingCharacters(in: .whitespaces) + "…"
    }
}

// MARK: - The runtime

/// Runs the wake: sync, harvest, dedupe, notify. One instance for the app — the push delegate
/// and the foreground path both come through here so the seen-set has exactly one owner.
@MainActor
final class InboundWake: ObservableObject {
    static let shared = InboundWake()

    /// The category every wake-raised notification carries, so the presenter can suppress its
    /// banner in the foreground without guessing which notifications are ours.
    static let category = "pacific.wake.message"

    /// Test opt-OUT (the `-pacificNoMusic` idiom): UI suites that drive precise
    /// taps launch with this so a wake banner can never cover the surface
    /// mid-flow — XCTest's interruption handling taps a banner it meets, which
    /// opens a room cover over whatever the test was driving.
    static let optOutArg = "-pacificNoWake"

    /// How many wakes have raised something — a diagnosable fact for the Settings row rather
    /// than an invisible one.
    @Published private(set) var lastWokeAt: Date?
    @Published private(set) var lastRaised = 0

    private let store: InboundSeenStore
    private var seen: InboundSeen
    private var running = false

    init(store: InboundSeenStore = InboundSeenStore()) {
        self.store = store
        self.seen = store.load()
    }

    /// The whole path, from a push (or a foreground refresh) to banners. Returns how many
    /// notifications were raised, which is what the background-fetch completion handler needs
    /// to report honestly to iOS.
    @discardableResult
    func wake(core: CoreService) async -> Int {
        if running { return 0 }
        running = true
        defer { running = false }

        await core.syncNow(silent: true)
        lastWokeAt = Date()

        let hits = InboundWakeHarvest.hits(scan(core: core))
        let new = seen.fresh(hits.map(\.key))
        store.save(seen)
        guard !new.isEmpty else { lastRaised = 0; return 0 }

        let newSet = Set(new)
        let raised = InboundWakeHarvest.banners(hits.filter { newSet.contains($0.key) })

        // FOREGROUND: no banner. The chat is already on screen and the sync above has already
        // refreshed it (`syncTick`), so a notification would be telling the user about
        // something they are looking at.
        guard UIApplication.shared.applicationState != .active else {
            lastRaised = 0
            return 0
        }

        for (hit, more) in raised { present(hit, more: more) }
        lastRaised = raised.count
        return raised.count
    }

    /// The FOREGROUND half: announce new inbound IN-APP after a sync landed. Same
    /// harvest, same seen-set — but the surface is the InAppNotify banner, never an
    /// iOS notification (NotifPresenter suppresses those while active). Skips the
    /// conversation currently on screen (ChatFocus) — its messages are visible where
    /// they belong — while still marking them seen so no later wake re-announces them.
    /// Called on every sync tick; the fold cost matches the CHAT list's own rebuild.
    func foregroundTick(core: CoreService, notify: InAppNotify) {
        guard UIApplication.shared.applicationState == .active else { return }
        let hits = InboundWakeHarvest.hits(scan(core: core))
        let new = seen.fresh(hits.map(\.key))
        store.save(seen)
        guard !new.isEmpty else { return }
        let newSet = Set(new)
        let raised = InboundWakeHarvest.banners(hits.filter { newSet.contains($0.key) })
        for (hit, more) in raised where hit.conversation != ChatFocus.shared.current {
            notify.show(title: hit.title,
                        body: more > 0 ? "\(hit.body)  +\(more)" : hit.body,
                        routeKey: hit.routeKey, routeValue: hit.routeValue)
        }
    }

    /// Mark everything currently folded as already handled, WITHOUT notifying.
    ///
    /// This is what stops a foreground session's messages from being announced hours later by
    /// the first background wake. The user read them on screen; a banner for them afterwards
    /// would be a bug, not a feature. Called when the app leaves the foreground, so it runs
    /// twice a session rather than on every three-second sync tick.
    func absorbCurrent(core: CoreService) {
        seen.absorb(InboundWakeHarvest.hits(scan(core: core)).map(\.key))
        store.save(seen)
    }

    /// Read every conversation the core holds. Not cheap, but a wake is rare — and the
    /// alternative (trusting the push to name a conversation) is exactly the un-blinding the
    /// wake tier exists to avoid: the push carries no identity at all.
    private func scan(core: CoreService) -> [InboundScan] {
        var scans: [InboundScan] = []
        for peer in core.connectionRows() where peer.connected {
            let convo = Conversation(id: peer.peerId, kind: .dm, title: peer.displayName, tint: .clear)
            let name = peer.displayName.isEmpty ? String(peer.peerId.prefix(12)) + "…" : peer.displayName
            scans.append(InboundScan(conversation: "dm:" + peer.peerId,
                                     title: name,
                                     routeKey: "peer", routeValue: peer.peerId,
                                     messages: core.messages(convo).map(Self.fields)))
        }
        for room in core.groupObjects {
            let convo = Conversation(id: room.id, kind: .group, title: room.name, tint: .clear)
            let name = room.name.isEmpty ? "Room" : room.name
            scans.append(InboundScan(conversation: "grp:" + room.id,
                                     title: name,
                                     routeKey: "object", routeValue: room.id,
                                     messages: core.messages(convo).map(Self.fields)))
        }
        return scans
    }

    private static func fields(_ m: ChatMsg) -> (author: String, gen: UInt64, text: String, ts: UInt64, mine: Bool) {
        (m.author, m.gen, m.text, m.ts, m.mine)
    }

    private func present(_ hit: InboundHit, more: Int) {
        guard !ProcessInfo.processInfo.arguments.contains(Self.optOutArg) else { return }
        let c = UNMutableNotificationContent()
        c.title = hit.title
        c.body = more > 0 ? "\(hit.body)\n+\(more) more" : hit.body
        c.sound = .default
        c.threadIdentifier = hit.conversation
        c.categoryIdentifier = Self.category
        c.userInfo = [hit.routeKey: hit.routeValue, "wake": true]
        // The conversation IS the identifier, so a second arrival replaces the first banner
        // instead of stacking one per message.
        UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: "wake-" + hit.conversation, content: c, trigger: nil))
    }

    /// Route a tap on a wake notification to the conversation it came from — the existing
    /// `pacificOpenDM` / `pacificOpenRoom` seam, not a second navigation mechanism.
    /// Returns false when the notification was not one of ours.
    @discardableResult
    static func route(_ userInfo: [AnyHashable: Any]) -> Bool {
        if let peer = userInfo["peer"] as? String, !peer.isEmpty {
            NotificationCenter.default.post(name: .pacificOpenDM, object: nil, userInfo: ["peer": peer])
            return true
        }
        if let object = userInfo["object"] as? String, !object.isEmpty {
            NotificationCenter.default.post(name: .pacificOpenRoom, object: nil, userInfo: ["object": object])
            return true
        }
        return false
    }
}
