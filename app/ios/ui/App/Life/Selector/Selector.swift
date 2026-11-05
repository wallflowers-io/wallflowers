//  Selector.swift
//  The primitive every LIFE category conforms to.
//
//  A category contributes four things and inherits everything else. Jiggle, maximise
//  and lock are implemented ONCE, in Selector/, so a fix to any of them is a fix
//  everywhere — which is the whole reason for the boundary. A category cannot opt out
//  of a behaviour, and cannot implement it differently by accident.
//
//  This also collapses two duplications the module map turned up:
//
//    * TWO category enums — `EventCategory` (5 cases, drives the rail) and
//      `LifeFilter` (9 cases, drives `+` and the lists). They had to be kept in step
//      by hand, and were not: `+` sat on its initial `.people` while the rail moved,
//      so it minted a Person whatever was maximised.
//    * TWO colour systems — `EventsSeed.tint` (hardcoded orange/blue/green/pink/teal,
//      what the jewel fills with) and `LifeFilter.color` (PaletteStore, the
//      user-selectable palette, what everything else read). They could never agree.
//      `tint` below is the single source; whether it stays fixed or follows the
//      palette is now ONE decision in ONE place rather than a divergence.

import SwiftUI

/// The categories the rail carries. Identity only — appearance and behaviour come
/// from the `Selector` conformance, so adding a category is one new folder.
enum SelectorCategory: String, CaseIterable, Hashable, Identifiable {
    case events, people, groups, places, projects, things

    var id: String { rawValue }

    /// Default left-to-right order (mirrors the rail, 1 Aug): PEOPLE and GROUPS are two
    /// selectors; THINGS has no jewel — the market it carried moved to the LIFE screen's
    /// second face. The user may reorder, so this is a STARTING arrangement and never a
    /// lookup key — see `SelectorArrangement.order`.
    static let defaultOrder: [SelectorCategory] = [.events, .people, .groups, .places, .projects]

    /// PLACES reads MAP: the jewel names the surface it maximises.
    var title: String { self == .places ? "MAP" : rawValue.uppercased() }
}

/// What a category must supply. Everything absent from this protocol is inherited.
@MainActor
protocol Selector {
    var category: SelectorCategory { get }

    /// SF Symbol for the jewel.
    var glyph: String { get }

    /// The one colour this category wears — jewel, `+`, ribbon accents, all of it.
    var tint: Color { get }

    /// The full-canvas surface shown while `.maximised`, or nil for categories that
    /// do not own a background (the plain ground is then shown for them).
    ///
    /// Returned as a single view so the canvas has ONE slot to swap with ONE
    /// transition. The map-teardown race came from having two.
    @ViewBuilder func background() -> AnyView?

    /// The compact overlay a `.locked` selector keeps on screen over whatever owns
    /// the background.
    @ViewBuilder func summary() -> AnyView?

    /// The column beneath the jewel — the ribbon content at the given width.
    @ViewBuilder func column(width: CGFloat) -> AnyView
}

extension Selector {
    // Defaults, so a category only implements what it actually has. Most categories
    // own no background and have no locked summary.
    func background() -> AnyView? { nil }
    func summary() -> AnyView? { nil }
}

/// Resolves a category to its implementation.
///
/// The `selector(for:)` lookup lands with the five conformances — the rail will then
/// iterate `arrangement.order` and ask here, so it never switches on category itself.
/// Until they exist this holds only what is answerable without one.
@MainActor
enum SelectorRegistry {
    /// THE colour source. Must be answerable without building a category, because the
    /// `+` needs it before any surface is constructed — which is precisely why it
    /// previously read a second palette and could never match the jewel.
    static func tint(_ category: SelectorCategory) -> Color {
        // Colour must be answerable WITHOUT a context — the `+` needs it before any
        // category has been built. Kept beside the conformances it feeds.
        switch category {
        case .events:   return .orange
        case .people:   return .blue
        case .groups:   return .indigo
        case .places:   return .green
        case .projects: return .pink
        case .things:   return .teal
        }
    }
}

/// What a Selector needs from the host to build its surfaces. The category depends on
/// this protocol, never on `LifeView` or `CoreService` — the host injects the
/// concretes, so `Selector/` stays free of app types.
@MainActor
protocol SelectorContext {
    var query: LifeQuery { get }
    var arrangement: SelectorArrangement { get }
}
