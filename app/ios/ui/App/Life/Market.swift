//  Market.swift
//  The market lens over LIFE — Has · Wants · Offers · Buying · Selling.
//
//  These are NOT a sixth entity category. The RibbonRail's jewels answer "what kind of
//  thing" (people, places, things…); a market intent answers "in what posture" — so it is
//  orthogonal to them and gets its own selector rather than a jewel.
//
//  The taxonomy is two axes, not five flat tabs (design of 25 Jul):
//
//                    │ Latent (standing) │ Active (on-market now)
//    ────────────────┼───────────────────┼───────────────────────
//    ACQUIRE         │      Wants        │        Buying
//    PROVIDE         │  Has → Offers     │       Selling
//
//  · Wants / Buying are DESIDERATIVE — a desire to acquire. Buying is a Want that has gone
//    active and time-bound.
//  · Offers / Selling are COMMISSIVE (Searle) — you committing yourself to provide. Selling
//    is an Offer with a specific item and a price, now.
//  · Has is the odd one out and the load-bearing one: not an intent but a STATE — your
//    inventory and capabilities. Offers and Selling are drawn FROM Has.
//
//  The asymmetry is deliberate: there is no demand-side "Has", because not-wanting is not
//  something anyone lists. Do not force symmetry onto it.
//
//  Graph fit is near-zero new primitives: the four intents are commitments (Event-shaped,
//  with participants and an optional price/deadline), and Has is entity state. Both ride the
//  Entity/Event/Fact model already there.

import SwiftUI
import PacificCore   // ThingRow — the folded Thing GroupObject

// MARK: - The intent

enum MarketIntent: String, CaseIterable, Identifiable, Hashable {
    case has     = "HAS"
    case wants   = "WANTS"
    case offers  = "OFFERS"
    case buying  = "BUYING"
    case selling = "SELLING"

    var id: String { rawValue }

    /// Selector order: Has first — it is the state the provide side is drawn from — then the
    /// two axes, each latent-before-active, so the row reads across in the same order as the
    /// table above.
    static let ordered: [MarketIntent] = [.has, .wants, .offers, .buying, .selling]

    /// Which side of the exchange. `nil` for Has, which is neither: it is what you hold.
    enum Side { case acquire, provide }
    var side: Side? {
        switch self {
        case .wants, .buying:   return .acquire
        case .offers, .selling: return .provide
        case .has:              return nil
        }
    }

    /// Standing intent vs on-market now. Has is a state, so it is neither.
    var isActive: Bool { self == .buying || self == .selling }

    /// Acquire reads cool, provide reads warm, Has is the neutral core they resolve around —
    /// the same colour semantics as the market cube on the Kenjin site, so the two surfaces
    /// describe one model rather than two.
    var color: Color {
        switch side {
        case .acquire?: return Color(hex: 0x5B8DEF)
        case .provide?: return Color(hex: 0xE0A83C)
        case nil:       return Theme.accent
        }
    }

    var glyph: String {
        switch self {
        case .has:     return "cube.fill"
        case .wants:   return "sparkle.magnifyingglass"
        case .offers:  return "hand.raised.fill"
        case .buying:  return "cart.fill"
        case .selling: return "tag.fill"
        }
    }
}

// MARK: - The read seam

/// One listed market item. Deliberately thin: an intent row is a commitment with a subject
/// and, when the intent is active, a price — nothing here presumes a fulfilment pipeline.
struct MarketItem: Identifiable, Hashable {
    let id: String
    let intent: MarketIntent
    /// What is wanted/offered, terse — "road bike", not a sentence.
    let title: String
    /// The market-face axis (artifact | skill | job) — which selector this item is under.
    var category: String = "artifact"
    /// Set only when the intent is active (Buying/Selling); a standing Want has no price.
    var price: String? = nil
    /// The entity this is about, when the graph resolved one.
    var subjectID: String? = nil

    // -- provenance: nil on MY OWN listings, set on everything discovered --
    //
    // A stranger's offer is only usable if you can see whose it is and who vouched for the
    // path it took to you. Both ride here so the row can say so without a second lookup.

    /// Identity (hex) of whoever created it; nil when the listing is mine.
    var origin: String? = nil
    /// Identity (hex) of whoever told me — equals `origin` at one hop.
    var via: String? = nil
    /// HOW FAR AWAY: 1 = straight from a connection, ≥2 = through the mesh. Never 0
    /// on a discovered row — 0 is my own fold, and my own things are not listings.
    var hops: UInt32 = 0
    /// Coarse area cell, when the listing carries one.
    var area: String? = nil
    /// The listing's face — decoded photo bytes; nil when it was listed without
    /// one (the row renders stripes, the honest fallback, never a stand-in).
    var photo: Data? = nil

    /// Someone else's, discovered — as opposed to my own posture.
    var isDiscovered: Bool { origin != nil }
}

/// What LIFE reads for the market lens. Mirrors `LifeSource`: the screen depends on this,
/// never on a concrete store, so the backing swaps without touching the view.
@MainActor
protocol MarketSource {
    func items(_ intent: MarketIntent) -> [MarketItem]
}

/// The market read as it actually stands: MINTED Thing GroupObjects for my own side, plus
/// the listings others have published or relayed to me for theirs.
///
/// It deliberately does NOT read the resolver graph. The graph holds what was *observed*; a
/// Thing GroupObject holds what you *accepted* at the swipe deck. Only the second is durable,
/// signed and shareable, so only the second belongs in a market. The graph's role upstream is
/// to propose — see `ProposalStore`.
///
/// BOTH SIDES IN ONE LIST, mine first. A market where you cannot see your own posture beside
/// the answering ones is a catalogue, not a market: the whole point of standing in WANTS is
/// to see who is in OFFERS.
@MainActor
struct CoreMarketSource: MarketSource {
    /// Folded Thing rows from pacific-core — what I own and posture.
    let things: [ThingRow]
    /// Listings discovered from connections and, multihop, from their connections.
    let listings: [ListingRow]

    func items(_ intent: MarketIntent) -> [MarketItem] {
        let posture = intent.rawValue.lowercased()

        let mine = things
            .filter { $0.posture == posture }
            .map { row in
                MarketItem(id: row.id,
                           intent: intent,
                           title: row.name.isEmpty ? row.descriptor : row.name,
                           category: row.category,
                           // Core only ever stores a price on an ACTIVE posture, so an empty
                           // string here is a genuine absence, not a dropped field.
                           price: row.price.isEmpty ? nil : row.price,
                           subjectID: row.id,
                           area: row.area.isEmpty ? nil : row.area,
                           photo: row.photo.isEmpty ? nil : Data(base64Encoded: row.photo))
            }
            .sorted { $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending }

        var theirs: [MarketItem] = []
        // The published listing carries no category yet, so a discovered row lands on
        // the artifact faces (INVENTORY · TRADE) until the wire schema grows one.
        for l in listings where l.posture == posture {
            let title = l.title.isEmpty ? l.descriptor : l.title
            theirs.append(MarketItem(id: "\(l.origin)/\(l.thingId)",  // network-wide identity
                                     intent: intent,
                                     title: title,
                                     price: l.price.isEmpty ? nil : l.price,
                                     subjectID: l.thingId,
                                     origin: l.origin,
                                     via: l.via,
                                     hops: l.hops,
                                     area: l.area.isEmpty ? nil : l.area,
                                     photo: l.photo.isEmpty ? nil : Data(base64Encoded: l.photo)))
        }
        // Nearest first: a direct connection's offer outranks one that came through the
        // mesh, then alphabetically so the order is stable between syncs.
        theirs.sort { a, b in
            if a.hops != b.hops { return a.hops < b.hops }
            return a.title.localizedCaseInsensitiveCompare(b.title) == .orderedAscending
        }

        return mine + theirs
    }
}
