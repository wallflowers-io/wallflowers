//  FeedModel.swift
//  The Active Scroll Feed's row model + pure assembly (FEED-TRANSMISSION §10, SC-5/SC-6).
//
//  Everything here is a pure function so the notch semantics are unit-testable. The
//  assembler consumes the reads the app already publishes (events, listings); place
//  posts and hydrated System items join when their FFI reads land — same FeedItem,
//  new provenance. Ordering is soonest-first over `whenMs` as a stand-in for log
//  order: EventRow/ListingRow do not yet carry their delta timestamp (SC-6 gap,
//  recorded in the brief).
//
//  LOCK SEMANTICS (0-ANSWER-G refinement, Ralph's word): locking selectors FILTERS
//  THE DELTAS SHOWN IN THE FEED, by kind. Empty lock set = every delta passes;
//  otherwise only deltas of the locked categories penetrate. The finer per-category
//  constraint language (windows, bound entities) returns with the graph-query
//  composer, not here.

import Foundation
import PacificCore

enum FeedProvenance: Equatable {
    /// Residue of a fold this device is in.
    case member
    /// Reached this device through `hops` handshakes (listings, discovery).
    case hops(UInt32)
    /// A System's hydrated item (kind 21). Unused until the FFI regen exposes it.
    case hydrated
}

/// The Instagram anatomy on the one card (S34 refinement): AUTHOR + LOCATION ride
/// the header, TITLE is the caption, FOOTER is the timestamp slot (an event's date;
/// a standing listing's posture — its honest "when").
/// Where a tapped author name lands: a person's profile or a Space's (a Space
/// is a Group — GroupDetailView is its profile). Routed at the Life call site,
/// same discipline as the card tap itself.
enum AuthorRoute: Hashable {
    case person(String)
    case space(String)
}

struct FeedItem: Identifiable, Equatable {
    let id: String
    let category: EventCategory
    let title: String
    let author: String
    let location: String
    let footer: String
    let whenMs: Int64
    let provenance: FeedProvenance
    /// The underlying object id (event / group / place / peer) — what navigation needs.
    let objectID: String?
    /// The card's media, straight off the fold (EventRow.banner / ListingRow.photo).
    /// "" = dark ground. Decoded once per card by BannerCache.
    var bannerB64: String = ""
    /// The author's DOOR: who the header name routes to when tapped. nil = the
    /// row cannot honestly answer (no identity crosses the read model for it) —
    /// the name renders as plain text, never a dead button.
    var authorRoute: AuthorRoute? = nil
    /// The author's identity hex, for the header's avatar; nil when the card has no person.
    var authorID: String? = nil

    /// Equality is the per-tick republish gate — comparing full banners would run
    /// megabytes of string compare every 3s (the audit's 70MB landmine), so media
    /// equality is by LENGTH: a changed banner virtually never keeps its byte count.
    static func == (a: FeedItem, b: FeedItem) -> Bool {
        a.id == b.id && a.category == b.category && a.title == b.title
            && a.author == b.author && a.location == b.location && a.footer == b.footer
            && a.whenMs == b.whenMs && a.provenance == b.provenance
            && a.objectID == b.objectID && a.bannerB64.count == b.bannerB64.count
            && a.authorRoute == b.authorRoute && a.authorID == b.authorID
    }
}

enum FeedAssembler {

    /// Union the published reads into rows. Past events sit out unless something
    /// re-admits them later; standing listings ride at `nowMs` so they interleave
    /// with the near future instead of pinning to an epoch.
    // Module-qualified: a private `EventRow: View` in EventRibbon.swift shadows the FFI type.
    // `makerOf` names who made an object (ObjectIndex.maker: the group's first owner);
    // `nameFor` resolves a listing origin's identity hex to a display name.
    /// ONE EVENT AS A CARD — the feed's own projection of an Event, extracted so a
    /// second surface can render the same card without re-deriving its fields.
    ///
    /// `rows` applies the feed's UPCOMING rule (past events sit out) and then calls
    /// this; a caller that has already decided WHICH events it is showing — a Space
    /// profile listing the events that Space carries, past ones included — calls this
    /// directly. The date rule belongs to the feed, the projection belongs to the
    /// object, and they are not the same decision.
    static func item(for e: PacificCore.EventRow, maker: Maker?) -> FeedItem {
        FeedItem(
            id: "event:\(e.id)",
            category: .events,
            title: e.title,
            author: maker?.name ?? "",
            location: e.venuePlaceName.isEmpty ? e.venue : e.venuePlaceName,
            footer: Self.stamp(e.startMs),
            whenMs: e.startMs,
            provenance: .member,
            objectID: e.id,
            bannerB64: e.banner,
            authorRoute: maker?.id.map(AuthorRoute.person),
            authorID: maker?.id)
    }

    /// Who a card names as its author: a name, and the person's identity when there is one.
    struct Maker: Equatable {
        let name: String
        let id: String?
    }

    static func rows(events: [PacificCore.EventRow], listings: [ListingRow],
                     posts: [PostRow] = [], nowMs: Int64,
                     makerOf: (String) -> Maker? = { _ in nil },
                     nameFor: (String) -> String = { String($0.prefix(8)) }) -> [FeedItem] {
        var out: [FeedItem] = []
        out.reserveCapacity(events.count + listings.count + posts.count)

        // POSTS. A Post has no `when` of its own — it is not scheduled, it was written —
        // so it rides at `nowMs`, exactly as a standing listing does, and interleaves with
        // the near future instead of pinning to an epoch.
        //
        // This is the SC-6 gap showing: ordering here stands in for LOG ORDER, and no row
        // carries its delta timestamp yet. When they do, a Post sorts by when it was
        // written, which is the only ordering that is actually true of it.
        //
        // A RETRACTED post still appears. The author took it down, which is a statement,
        // and the row shows the tombstone — dropping it here would make "taken down"
        // indistinguishable from "never existed".
        for p in posts {
            let maker = makerOf(p.id)
            out.append(FeedItem(
                id: "post:\(p.id)",
                category: .posts,
                title: p.retracted ? "Taken down" : (p.title.isEmpty ? p.body : p.title),
                author: maker?.name ?? "",
                location: "",
                footer: "",
                whenMs: nowMs,
                provenance: .member,
                objectID: p.id,
                bannerB64: p.retracted ? "" : p.banner,
                authorRoute: maker?.id.map(AuthorRoute.person),
                authorID: maker?.id))
        }

        for e in events where e.endMs >= nowMs || e.startMs >= nowMs {
            out.append(item(for: e, maker: makerOf(e.id)))
        }
        for l in listings {
            out.append(FeedItem(
                id: "listing:\(l.origin):\(l.thingId)",
                category: .trade,
                title: l.title,
                author: nameFor(l.origin),
                location: l.area,
                footer: l.deadline == 0 ? l.posture.uppercased()
                                        : "\(l.posture.uppercased()) · \(Self.stamp(l.deadline))",
                whenMs: l.deadline == 0 ? nowMs : l.deadline,
                // ALWAYS over hops. `.member` means a fold this device is IN, and it
                // drew the filled mark for a direct connection's listing — which is
                // not my fold and never was. Inbound listings exclude my own, so
                // every row here arrived over at least one hop.
                provenance: .hops(l.hops),
                objectID: nil,
                bannerB64: l.photo,
                authorRoute: .person(l.origin),
                authorID: l.origin))
        }
        return out.sorted { $0.whenMs == $1.whenMs ? $0.id < $1.id : $0.whenMs < $1.whenMs }
    }

    /// The lock filter: locked categories are the ACTIVE kinds. Nothing locked = no
    /// filter; anything locked = only those kinds of deltas penetrate.
    static func filter(_ rows: [FeedItem], locked: Set<EventCategory>) -> [FeedItem] {
        guard !locked.isEmpty else { return rows }
        return rows.filter { locked.contains($0.category) }
    }

    private static func stamp(_ ms: Int64) -> String {
        guard ms > 0 else { return "" }
        let f = DateFormatter()
        f.dateFormat = "EEE d MMM · HH:mm"
        return f.string(from: Date(timeIntervalSince1970: Double(ms) / 1000))
    }
}

// MARK: - First-seen ledger (the unread band's memory)

/// "New since last look" must survive force-quit with no push (I-G). The ledger
/// records when each row id was first assembled on this device; the band divides
/// rows first seen after the last visit from the rest. Bounded, insertion-pruned.
struct FeedSeenLedger {
    private(set) var firstSeen: [String: Int64]
    private(set) var lastVisitMs: Int64
    static let cap = 600

    init(firstSeen: [String: Int64] = [:], lastVisitMs: Int64 = 0) {
        self.firstSeen = firstSeen
        self.lastVisitMs = lastVisitMs
    }

    mutating func observe(_ rows: [FeedItem], nowMs: Int64) {
        for r in rows where firstSeen[r.id] == nil { firstSeen[r.id] = nowMs }
        if firstSeen.count > Self.cap {
            let keep = Set(rows.map(\.id))
            let stale = firstSeen.keys.filter { !keep.contains($0) }
                .sorted { firstSeen[$0]! < firstSeen[$1]! }
            for k in stale.prefix(firstSeen.count - Self.cap) { firstSeen.removeValue(forKey: k) }
        }
    }

    mutating func visit(nowMs: Int64) { lastVisitMs = nowMs }

    func isNew(_ row: FeedItem) -> Bool {
        guard lastVisitMs > 0, let t = firstSeen[row.id] else { return false }
        return t > lastVisitMs
    }
}
