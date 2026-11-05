//  LifeQuery.swift
//  The multimodal filter — LIFE's core mechanic.
//
//  Each jewel owns a ribbon. Three independent pieces of state, and the whole surface is
//  a function of them:
//
//    SELECT  which ribbon is expanded. Width only — never changes what matches.
//    LOCK    which categories are ACTIVE filter dimensions (existential: "a place").
//    BIND    an entity chosen inside a locked ribbon, pinning that dimension ("Bristol").
//
//  Locked+unbound reads "a gallery, an event, a group"; binding narrows it to
//  "Hope * Car * Bristol". The composition is a conjunction — every locked dimension must
//  be satisfied, one hop from the event that joins them.

import SwiftUI

@MainActor
final class LifeQuery: ObservableObject {
    /// Which ribbon is expanded. Presentation only — it never affects `terms`, so widening
    /// a ribbon to browse it can never silently change the result set.
    @Published var selected: EventCategory = .events

    /// Which jewel's CONFIG PAGE owns the ground (FEED-TRANSMISSION 0-ANSWER-C).
    /// nil = every jewel minimised = the Active Scroll Feed. `selected` tracks the
    /// maximised jewel so width and ground never disagree.
    @Published private(set) var maximised: EventCategory? = nil

    /// Tap grammar: maximise the jewel; a second tap on the maximised jewel — or the
    /// white orb — minimises back to the feed.
    func maximise(_ c: EventCategory) {
        selected = c
        maximised = (maximised == c) ? nil : c
    }

    func minimiseAll() { maximised = nil }

    /// Active filter dimensions. Empty = no filter, every ribbon shows its category whole.
    @Published private(set) var locked: Set<EventCategory> = []

    /// Locked dimensions narrowed to one entity. A binding without its lock is meaningless,
    /// so unlocking always clears the bind.
    @Published private(set) var bound: [EventCategory: String] = [:]

    // The market posture used to ride here (on the THINGS jewel). It moved to the LIFE
    // screen's second face (MarketFace, 1 Aug) — a posture is a lens on the MARKET, not a
    // dimension of the commitment filter, and the two-faces split makes that literal.

    // MARK: The query

    /// The conjunction the graph is asked to satisfy. Bound dimensions carry their entity;
    /// locked-but-unbound stay existential.
    var terms: [FilterTerm] {
        locked
            .sorted { $0.rawValue < $1.rawValue }          // stable order = stable identity
            .map { FilterTerm(category: $0, entityID: bound[$0]) }
    }

    var isFiltering: Bool { !locked.isEmpty }

    // MARK: Gestures

    func select(_ c: EventCategory) { selected = c }

    /// Lock toggles the dimension. Unlocking drops any binding with it — a pinned entity in
    /// an inactive dimension would be invisible state that still filtered.
    func toggleLock(_ c: EventCategory) {
        if locked.contains(c) {
            locked.remove(c)
            bound[c] = nil
        } else {
            locked.insert(c)
        }
    }

    /// Choosing an entity inside a ribbon pins that dimension to it. Selecting the same
    /// entity again releases the pin back to existential — the dimension stays active.
    /// Binding implies the lock, so tapping an entity in an unlocked ribbon activates it.
    func bind(_ entityID: String, in c: EventCategory) {
        locked.insert(c)
        bound[c] = (bound[c] == entityID) ? nil : entityID
    }

    func isLocked(_ c: EventCategory) -> Bool { locked.contains(c) }
    func boundEntity(in c: EventCategory) -> String? { bound[c] }

    func clear() {
        locked.removeAll()
        bound.removeAll()
    }
}

// MARK: - Ribbon widths

/// A category gets EXACTLY the horizontal width of its own selector — no more. The jewel is
/// the width control, so the column beneath it inherits the jewel's width and nothing else.
///
/// Width is therefore a function of SELECTION ALONE. Locking marks a dimension active and
/// changes what matches; it must never change how wide the column is, or lock and select
/// would both be competing to own the same dimension of the layout.
enum RibbonWidth {
    /// The jewel at rest — a circle. 46: SEVEN equal circles (orb + six selectors,
    /// 0-ANSWER-G S31) must fit a 390pt screen with even gaps.
    static let contracted: CGFloat = DesignTokens.Rail.jewel
    /// The jewel selected — a pill. Its length is the column's full width.
    /// 134, not 168: the maximised pill gives back ~20% so a rail reads as a row of
    /// selectors with one lit, not one banner crowding the rest off the edge.
    static let expanded: CGFloat = 134

    @MainActor
    static func of(_ c: EventCategory, in q: LifeQuery) -> CGFloat {
        // Expansion tracks the MAXIMISED jewel: on the feed (nothing maximised)
        // the rail reads as six equal selectors, none lit.
        q.maximised == c ? expanded : contracted
    }
}
