//  EventsContract.swift
//  The EVENTS consumer's data contract — what the ribbon requires, defined at the
//  consumer and handed UP to the resolver. Nothing here reads the graph; this is the
//  shape the graph must be able to answer in.
//
//  An Event is an n-ary commitment: a reified hyperedge. In the graph that is an
//  Entity(type: "Event") with one binary fact per participant, so a filter term is
//  always exactly ONE hop from the event.

import SwiftUI

// MARK: - Filter terms

/// A locked filter dimension. `category` alone is existential ("there must be a place");
/// binding `entityID` pins it ("…and it must be Bristol").
struct FilterTerm: Hashable, Identifiable {
    let category: EventCategory
    var entityID: String? = nil

    var id: String { entityID.map { "\(category.rawValue):\($0)" } ?? category.rawValue }
    var isBound: Bool { entityID != nil }
}

/// The categories a participant (or a filter dimension) can be. Mirrors the jewels.
///
/// `people` and `groups` are one *grammatical* category (each can be the Agent of a
/// predicate — "Hope is presenting" / "the choir is presenting"), and Pacific's core models
/// both as GroupObjects differing only in `GroupShape`. They are nonetheless two SELECTORS
/// (ruling of 1 Aug): who-you-know and what-you-belong-to are different questions to stand
/// in front of, so the grammar stays shared while the jewels split.
///
/// `things` exists because the filter demands it: "Hope * Car * Bristol" cannot be expressed
/// without a Things dimension. Plant and Animal fold into it. It no longer has a jewel — the
/// market moved to the LIFE screen's second face — so it survives here as a participant
/// category only.
enum EventCategory: String, Hashable, CaseIterable {
    case events, people, groups, places, projects, things
    // The single-page rail (FEED-TRANSMISSION 0-ANSWER-C) promotes the market's two
    // dimensions to first-class jewels; projects/things remain participant grammar only.
    case trade, job
    // The Profile pages' fifth jewel (place.posts + forum activity). NOT in the
    // default rail order — it exists for the profile jewels row only, for now.
    case posts
    // A Space's CHANNELS — its Forum GroupObjects. Split out of `posts`: the two
    // were sharing one lane on the group page ("the group's posts ARE its
    // channels' activity"), and a channel list is not a post feed. Profile
    // jewels row only, like `posts`.
    case channels

    /// The selector's display name — the six-jewel rail reads singular
    /// (GROUP · EVENT · PLACE · PERSON · TRADE · JOB).
    var title: String {
        switch self {
        case .events: return "EVENT"
        case .people: return "PERSON"
        case .groups: return "GROUP"
        case .places: return "PLACE"
        case .trade:  return "TRADE"
        case .job:    return "JOB"
        case .posts:  return "POSTS"
        case .channels: return "CHANNEL"
        case .projects, .things: return rawValue.uppercased()
        }
    }
}

// MARK: - Participation

/// How an entity takes part in a commitment. The filter ignores this; the ribbon shows
/// it only when wide enough, and it is what makes an event *mean* something rather than
/// just co-occur.
enum ParticipantRole: String, Hashable {
    /// Who is committed — the entity the commitment is ON. At least one is required.
    case subject
    /// Where it happens.
    case at
    /// Things or people involved without being the committed party.
    case with
    /// What it concerns (a project, a topic).
    case about
}

struct ParticipantVM: Hashable, Identifiable {
    let id: String                 // the participant entity's id
    let name: String
    let category: EventCategory
    let role: ParticipantRole
    /// The participant's category tint — the dot the contracted ribbon shows.
    var tint: Color
}

// MARK: - Recurrence (the STORED shape)

/// A repeating commitment, stored on the Event entity. RFC 5545: a rule is meaningless
/// without an anchor, so `dtstart` is required and `rrule` is the pattern from it.
///
/// TZID is NOT optional in practice. "The gallery opens 1000–1800h" recurs in LOCAL time;
/// anchoring it in UTC makes it drift an hour at every DST boundary. The zone travels with
/// the rule so expansion is DST-correct.
///
/// This never reaches the ribbon. `EventsSource` expands it against the query window to
/// produce `EventVM.startMs` — next-occurrence is relative to NOW and would be stale the
/// moment it was written, so it is derived per read, never stored.
struct EventRecurrence: Hashable {
    /// Epoch ms of the FIRST occurrence — RFC 5545 DTSTART.
    let dtstartMs: Int64
    /// IANA zone the rule expands in, e.g. "Europe/London".
    let tzid: String
    /// The pattern, e.g. "FREQ=WEEKLY;BYDAY=TU,TH".
    let rrule: String
    /// Length of each occurrence. Absent = a point in time, not a span.
    var durationMs: Int64? = nil
}

// MARK: - The event the ribbon renders

/// One row of the EVENTS ribbon.
///
/// `startMs` is the single sort key and is REQUIRED — a commitment with no resolvable
/// time cannot be sequenced, so it does not enter the ribbon. A recurring commitment
/// resolves to its NEXT OCCURRENCE upstream; the ribbon never interprets a recurrence
/// rule, it only ever sorts on this one number.
struct EventVM: Hashable, Identifiable {
    let id: String
    /// Epoch milliseconds of the occurrence this row represents. The sort key.
    let startMs: Int64
    /// Epoch milliseconds the occurrence ends, when the commitment is a span
    /// (the gallery's 1000–1800h) rather than a point.
    var endMs: Int64? = nil
    /// The commitment itself, terse — "Gallery opening", not a sentence.
    let title: String
    /// Every entity this commitment binds. NEVER empty: a disconnected event is
    /// unreachable by an intersection filter and must be rejected at extraction.
    let participants: [ParticipantVM]
    /// Present when this row is one occurrence of a repeating commitment. Carried for
    /// display only ("Tue & Thu"), never parsed here.
    var recurrenceLabel: String? = nil
    /// A remote flyer for the NEAR card's image slot — carried by reference rows
    /// (the Music System's "ra:" listings). Local rows use their own media facet.
    var flyerURL: URL? = nil
    /// Where the commitment lives at its SOURCE, when it has a home besides Pacific —
    /// a reference row's own listing page (ra.co / artrabbit.com). Absent on local rows.
    var accessURL: URL? = nil

    var isRecurring: Bool { recurrenceLabel != nil }
    var isSpan: Bool { endMs != nil }
    /// The entity the commitment is ON — the first `subject`, else the first participant.
    var subject: ParticipantVM? {
        participants.first { $0.role == .subject } ?? participants.first
    }
}

// MARK: - Ticketing (the capsule's contract)

/// What the ticket capsule needs to know about ONE listed event — projected, so the page
/// stays a skin: no FFI types, no stores, just the answer to "what does the capsule say".
struct TicketVM: Hashable {
    /// Minor units in the listing currency. 0 = a free event.
    let priceCents: Int64
    /// "$12" · "FREE" — the capsule's value half and the flyer chip.
    let priceLabel: String
    /// The wallet holds a credential for this listing → "TICKET ✓".
    let held: Bool
    /// A checkout/request is in flight → the capsule disables (9A).
    let pending: Bool
    /// sold ≥ capacity → "SOLD OUT".
    let soldOut: Bool
    /// THIS identity owns the event — the capsule is a value, and the door affordance shows.
    let owned: Bool
    /// Whether a tap can actually buy (free path, someone else's event).
    let buyable: Bool
}

/// The page's ticketing context, handed down like the sources: state in `byId`
/// (keyed by the VM id, "obj:<objectId>"), intents out through the closures.
struct EventTicketing {
    var byId: [String: TicketVM] = [:]
    var buy: (String) -> Void = { _ in }
    var door: (String) -> Void = { _ in }
}

/// Money display + entry for ticket listings. The supported set mirrors the fee table's
/// committed rates (usd/eur/gbp/krw/jpy); krw/jpy are zero-decimal.
enum TicketMoney {
    static let currencies = ["usd", "eur", "gbp", "krw", "jpy"]
    static let zeroDecimal: Set<String> = ["krw", "jpy"]

    static func symbol(_ currency: String) -> String {
        switch currency.lowercased() {
        case "eur": return "€"
        case "gbp": return "£"
        case "krw": return "₩"
        case "jpy": return "¥"
        default:    return "$"
        }
    }

    /// "$12" · "£8.50" · "₩15,000" — minor units to the label the capsule and chip show.
    static func label(cents: Int64, currency: String) -> String {
        let sym = symbol(currency)
        if zeroDecimal.contains(currency.lowercased()) {
            let f = NumberFormatter()
            f.numberStyle = .decimal
            return sym + (f.string(from: NSNumber(value: cents)) ?? String(cents))
        }
        if cents % 100 == 0 { return "\(sym)\(cents / 100)" }
        return String(format: "%@%.2f", sym, Double(cents) / 100)
    }

    /// A typed decimal amount to minor units (half-up), respecting zero-decimal currencies.
    static func minorUnits(_ amount: Decimal, currency: String) -> Int64 {
        let scale = zeroDecimal.contains(currency.lowercased()) ? Decimal(1) : Decimal(100)
        var scaled = amount * scale
        var rounded = Decimal()
        NSDecimalRound(&rounded, &scaled, 0, .plain)
        return NSDecimalNumber(decimal: rounded).int64Value
    }
}

// MARK: - The read seam

/// What the EVENTS ribbon asks of the world. One call: give me the commitments that
/// satisfy these locked terms, in chronological order.
///
/// The implementation resolves this as a 1-hop adjacency intersection over the graph —
/// an Event matches when it has a direct edge satisfying EVERY term. Ordering is by
/// `startMs` ascending; the source decides the window (see `EventsWindow`).
@MainActor
protocol EventsSource {
    func events(matching terms: [FilterTerm], window: EventsWindow) -> [EventVM]
}

/// Which slice of time the ribbon is showing. Chronological order is always ascending;
/// this only decides where the ribbon starts.
enum EventsWindow: Hashable {
    /// From now forward — commitments still outstanding.
    case upcoming
    /// Everything, past included. A past commitment remains true: it happened.
    case all
    /// An explicit span, for scrubbing back through history.
    case between(startMs: Int64, endMs: Int64)
}

// MARK: - The reference listing

/// A world listing projected for the REFERENCE page — the ONE skin every reference
/// provenance renders through (defined at the consumer, the EventsContract pattern).
/// The Music System's RA rows and the Art System's ArtRabbit rows both project here;
/// the page itself never learns a source's wire types.
struct ReferenceListing: Identifiable, Hashable {
    let id: String            // the feed row's id ("ra:…" / "art:…") — the mint's source key
    let title: String
    var lineup: String?       // RA's artists; absent for sources without one
    let whenLabel: String
    var venueName: String?
    var flyerURL: URL?
    /// The one capsule out — where this listing actually lives. Label + destination;
    /// absent (RA's unticketed rows) means no capsule at all.
    var linkLabel: String?
    var linkURL: URL?
    /// The listing's RAW times — what MINT authors onto the Event GroupObject
    /// (`whenLabel` is display only and cannot be parsed back).
    var startMs: Int64 = 0
    var endMs: Int64? = nil
}

extension ReferenceListing {
    /// An RA listing: lineup + when, and its ticketing lives on RA — the capsule shows
    /// only when the listing is ticketed there.
    init(music e: MusicEvent) {
        self.init(id: "ra:\(e.id)",
                  title: e.title,
                  lineup: e.artists.isEmpty ? nil : e.lineup,
                  whenLabel: MusicEnrichmentBridge.when(e),
                  venueName: e.venueName,
                  flyerURL: e.flyerURL,
                  linkLabel: e.isTicketed ? "TICKETS ON RA" : nil,
                  linkURL: e.isTicketed ? e.contentUrl : nil,
                  startMs: e.startMs,
                  endMs: e.endMs)
    }

    /// An ArtRabbit listing: the run as the when, and the capsule is the listing's own
    /// page — reference data's canonical home, ticketed or not.
    init(art e: ArtEvent) {
        self.init(id: "art:\(e.id)",
                  title: e.title,
                  lineup: nil,
                  whenLabel: ArtEnrichmentBridge.when(e),
                  venueName: e.venueName,
                  flyerURL: e.imageURL,
                  linkLabel: "ON ARTRABBIT",
                  linkURL: e.contentUrl,
                  startMs: e.startMs,
                  endMs: e.endMs)
    }
}
