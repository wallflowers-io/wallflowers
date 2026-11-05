//  SelectorState.swift
//  The three states a Selector can be in — declared once, for every category.
//
//  Today each category re-derives its own posture from scattered booleans
//  (`jiggle`, `lifeQuery.selected == .places`, `backgroundOwned`, `market != nil`),
//  which is why behaviour diverges between categories and why fixes have to be made
//  five times. One enum, read everywhere, is the point of the primitive.
//
//  It is also the fix for a specific rendering bug. `lifeQuery.selected == .places`
//  was evaluated in TWO places — once to choose the base layer (PlacesMap vs
//  Theme.bg) and once to size the rail (`backgroundOwned`). Leaving PLACES flipped
//  both, but a rail frame change is a cheap layout pass while tearing down MapKit's
//  UIViewRepresentable is asynchronous and slow, so the rail's black arrived first and
//  the map lingered beneath it. Two views, one condition, no shared transition.
//
//  With a single state driving a single background slot, that desync is not
//  expressible: there is one view identity to swap and one transition to run.

import SwiftUI

/// What a selector is doing right now. Exactly one category may be `.maximised`;
/// any number may be `.locked`.
enum SelectorState: Equatable {
    /// Collapsed to its jewel. The resting state.
    case contracted
    /// Tapped — wider, and it OWNS THE BACKGROUND (the Places map, the Things market
    /// list). At most one selector is maximised at a time.
    case maximised
    /// Swiped down past the vertical extent and released. Overlays its summary on
    /// whatever owns the background, and stays there while other categories change.
    case locked
    /// The rail is being rearranged; every jewel wobbles and gestures belong to the
    /// rail rather than to any one jewel.
    case jiggling

    var ownsBackground: Bool { self == .maximised }
    var showsSummary: Bool { self == .locked }
    var isInteractive: Bool { self != .jiggling }
}

/// Which categories are in which state. The single source of truth the rail, the
/// canvas and the `+` all read — so they cannot disagree, which is what produced both
/// the mismatched-tint regression and the map teardown race.
struct SelectorArrangement: Equatable {
    /// Left-to-right order. USER-REORDERABLE, so nothing may key off position —
    /// identity is the category, never its index.
    private(set) var order: [SelectorCategory]
    private(set) var maximised: SelectorCategory?
    private(set) var locked: Set<SelectorCategory>
    private(set) var jiggling: Bool

    init(order: [SelectorCategory] = SelectorCategory.defaultOrder,
         maximised: SelectorCategory? = .events,
         locked: Set<SelectorCategory> = [],
         jiggling: Bool = false) {
        self.order = order
        self.maximised = maximised
        self.locked = locked
        self.jiggling = jiggling
    }

    func state(of category: SelectorCategory) -> SelectorState {
        if jiggling { return .jiggling }
        if locked.contains(category) { return .locked }
        return maximised == category ? .maximised : .contracted
    }

    /// The category currently owning the background, if any. The canvas asks for this
    /// ONCE — one slot, one identity, one transition.
    var backgroundOwner: SelectorCategory? { jiggling ? nil : maximised }

    // MARK: Transitions — the only ways the arrangement may change

    mutating func maximise(_ category: SelectorCategory) {
        // Maximising is exclusive; locked selectors are unaffected and keep overlaying.
        maximised = category
    }

    mutating func toggleLock(_ category: SelectorCategory) {
        if locked.contains(category) { locked.remove(category) } else { locked.insert(category) }
    }

    mutating func setJiggling(_ on: Bool) { jiggling = on }

    mutating func move(_ category: SelectorCategory, to index: Int) {
        guard let from = order.firstIndex(of: category), index >= 0, index < order.count else { return }
        order.remove(at: from)
        order.insert(category, at: min(index, order.count))
    }
}
