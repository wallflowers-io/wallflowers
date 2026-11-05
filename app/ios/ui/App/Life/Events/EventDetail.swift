//  EventDetail.swift
//  The EVENT's detail modal — its binder, and the one call that raises it.
//
//  WHAT REPLACED WHAT. This is the third shape the event page has taken, and the
//  two before it were both wrong in the same way. It was 687 hand-rolled lines
//  that drew their own hero, profile block, capsule row and scroll shell; then it
//  was the universal PROFILE component, which was worse in a subtler way — a
//  profile answers "who is this", and a night is not a who. An event borrowed the
//  egoic IA, wore a name-plate geometry meant for a Space, and grew a capsule row
//  (PLACE · PEOPLE · SHARE TO) that exists nowhere else in the app.
//
//  So: EVENTS ARE NOT PROFILES. People and Spaces are egoic and get
//  `Design/Profile`. An event is a thing that HAPPENS — it has an author rather
//  than an identity — so it gets `Design/Detail`, the modal every non-egoic
//  GroupObject shares with listings and posts.
//
//  AND IT IS A MODAL, hosted under the shell's disc row (`ShellSurface`), not a
//  fullScreenCover over it. That is what keeps RED · GOLD · BLUE visible and makes
//  them this surface's own buttons: close · share · say something. A cover would
//  hide the three circles that are the app's constant.
//
//  WHAT THE BINDER ANSWERS is only the three things the contract asks for — the
//  flyer, whose night it is, and what people said. Everything the old page carried
//  beyond that either moved to a fact line (the when, the series, the venue) or
//  went (ticketing is hidden by ruling; the photo strip and the RSVP circle were
//  page furniture that a modal has no room for and no need of).

import SwiftUI
import PacificCore
import ArcResolverKit   // ArcFact — the happens_at edge the venue fact reads

// MARK: - Raising it

@MainActor
enum EventDetail {

    /// EVENTS' own palette slot — the same one the jewel and the `New Event`
    /// surface are cast in (`NewKind.colorIndex` = 0).
    static var tint: Color { PaletteStore.base(0) }

    /// Open a minted Event. One call from every surface that lists events.
    static func open(_ objectId: String, core: CoreService, graph: GraphService,
                     shell: ShellState) {
        let source = EventDetailSource(core: core, graph: graph, objectId: objectId)
        shell.beginNew(DetailSession(noun: "event", source: source))
    }

    /// Open a WORLD LISTING — an RA or ArtRabbit row nobody has minted. It has no
    /// log to speak into, so BLUE is not "say something": it is MINT, the one
    /// Pacific act available on a row that is not yet an object. Minted from this
    /// listing before ⇒ OPEN that event, never a twin.
    static func open(listing: ReferenceListing, core: CoreService, graph: GraphService,
                     shell: ShellState) {
        let existing = MintedFrom.objectId(for: listing.id)
            .flatMap { id in core.events.contains { $0.id == id } ? id : nil }
        if let existing {
            open(existing, core: core, graph: graph, shell: shell)
            return
        }
        let source = ListingDetailSource(listing: listing)
        let session = DetailSession(
            noun: "listing", source: source,
            primary: .action(label: "Mint") {
                guard listing.startMs > 0,
                      let id = core.mintEvent(title: listing.title,
                                              startMs: listing.startMs,
                                              endMs: listing.endMs ?? 0,
                                              venue: listing.venueName ?? "",
                                              lineup: (listing.lineup ?? "")
                                                  .replacingOccurrences(of: " · ", with: "\n"))
                else { return nil }         // nothing minted ⇒ the surface stays up
                MintedFrom.record(id, from: listing.id)
                // The minted night opens in place of the listing, after this
                // surface's departure has settled — a surface raised into one that
                // is still leaving is silently dropped.
                DispatchQueue.main.asyncAfter(deadline: .now() + Motion.noteExit + 0.1) {
                    open(id, core: core, graph: graph, shell: shell)
                }
                return .saved
            })
        shell.beginNew(session)
    }
}

// MARK: - The minted event

/// The binder for a minted Event GroupObject.
@MainActor
struct EventDetailSource: DetailSource {
    let core: CoreService
    let graph: GraphService
    let objectId: String

    private var row: EventRow? { core.events.first { $0.id == objectId } }

    /// The flyer, and the night's facts as terse lines: when · the series · where ·
    /// who is playing · how many said yes. Each one is dropped where the world
    /// cannot answer, so the block is short on a bare event and long on a full one
    /// — rather than a fixed set of rows half of which read "—".
    func headline() async -> DetailHeadline? {
        guard let row else { return nil }
        // The full media facet, not the list row's still: this is where the flyer
        // is looked AT. The row's own banner stands in while that read is cold.
        let b64 = core.eventMedia(objectId).map(\.banner).flatMap { $0.isEmpty ? nil : $0 }
            ?? (row.banner.isEmpty ? nil : row.banner)

        var facts = [EventWhen.label(startMs: row.startMs, endMs: row.endMs)]
        let series = core.recurrenceLabel(row.recurrence)
        if !series.isEmpty { facts.append(series) }
        if let venue = await venueName() { facts.append(venue) }
        if !row.lineup.isEmpty { facts.append(row.lineup.joined(separator: " · ")) }
        // NOT the MLS roster: an event's members are its organiser and door staff,
        // and calling those attendance would be a confident falsehood. This counts
        // signed `contact.inviteReply` legs, and a night nobody has answered says
        // nothing rather than "0 going".
        let going = core.attendees(of: objectId).count
        if going > 0 { facts.append("\(going) going") }
        if !row.descriptor.isEmpty { facts.append(row.descriptor) }

        return DetailHeadline(title: row.title,
                              photo: b64.flatMap { Data(base64Encoded: $0) },
                              facts: facts)
    }

    /// The Space that published the night. The `created` edge already existed on
    /// the host's own log; nothing on the event read it back, so an event used to
    /// be a dead end for its own publisher.
    func author() async -> DetailAuthor? {
        guard let host = core.creatorOfObject(objectId) else { return nil }
        let name = host.name.isEmpty ? "Host" : host.name
        return DetailAuthor(id: host.id, name: name, initials: Avatar.initials(name),
                            isSpace: true, color: EventDetail.tint)
    }

    /// The event's OWN log is the room — `forum.post` deltas fold beside the
    /// event's, so there is no second object and no link to lose.
    func comments() async -> [DetailComment] {
        core.objectThread(objectId).map { row in
            DetailComment(id: "\(row.post.author):\(row.post.gen)",
                          author: name(of: row.post),
                          initials: Avatar.initials(name(of: row.post)),
                          body: row.post.text,
                          whenMs: Int64(row.post.ts),
                          mine: row.post.mine,
                          color: EventDetail.tint)
        }
    }

    func post(_ text: String) async -> Bool {
        await core.sendToGroup(objectId, text)
    }

    /// The Spaces this can reach: groups whose roster I am in. A Space I merely
    /// know of is not a place I can speak for.
    func shareTargets() async -> [DetailShareTarget] {
        core.spaces
            .map { DetailShareTarget(id: $0.id, name: $0.name.isEmpty ? String($0.id.prefix(8)) : $0.name) }
    }

    /// The edge is authored on the SPACE's log, not the event's — `peer`, which the
    /// projector mints as `affiliated_with`. The promoter's Space keeps the
    /// `created` edge it wrote; ours says only that we are affiliated. Two claims
    /// about the same object, each on its author's own log, neither able to
    /// overwrite the other.
    func share(to target: DetailShareTarget) async -> Bool {
        guard let row else { return false }
        core.setAffiliation(target.id, peer: objectId, rel: "peer", name: row.title)
        return true
    }

    // MARK: Naming things

    /// The live `happens_at` edge, named by the GRAPH; the folded row's own venue
    /// fields stand in until the projection lands.
    private func venueName() async -> String? {
        guard let row else { return nil }
        if let fact = await graph.happensAt(eventId: objectId) {
            return (await graph.entity(id: fact.dst))?.name ?? fact.dst
        }
        if !row.venuePlaceName.isEmpty { return row.venuePlaceName }
        return row.venue.isEmpty ? nil : row.venue
    }

    private func name(of post: ChatMsg) -> String {
        if post.mine { return core.displayName.isEmpty ? "You" : core.displayName }
        let names = core.peerNamesByIdentity()
        return names[post.author] ?? String(post.author.prefix(8))
    }
}

// MARK: - The world listing

/// The binder for a REFERENCE listing — an RA or ArtRabbit row nobody has minted.
///
/// It answers ONE of the three questions and nil to the rest, and that is the
/// honest shape of a listing: there is no fold, so there is no author to credit
/// and no log to comment on. What it has is a picture at a URL and the values
/// printed on it.
@MainActor
struct ListingDetailSource: DetailSource {
    let listing: ReferenceListing

    func headline() async -> DetailHeadline? {
        var facts = [listing.whenLabel]
        if let venue = listing.venueName, !venue.isEmpty { facts.append(venue) }
        if let lineup = listing.lineup, !lineup.isEmpty { facts.append(lineup) }
        return DetailHeadline(title: listing.title, photoURL: listing.flyerURL, facts: facts)
    }

    func author() async -> DetailAuthor? { nil }
    func comments() async -> [DetailComment] { [] }
    func post(_ text: String) async -> Bool { false }
}

// MARK: - When

/// The one place an event's WHEN becomes words — so the flyer, the card and the
/// invite can never word the same instant differently.
enum EventWhen {
    /// "Sat 2 Aug · 13:00 – 18:00", or just the start when there is no stated end.
    static func label(startMs: Int64, endMs: Int64) -> String {
        let start = Date(timeIntervalSince1970: Double(startMs) / 1000)
        var s = day.string(from: start) + " · " + clock.string(from: start)
        if endMs > 0 {
            s += " – " + clock.string(from: Date(timeIntervalSince1970: Double(endMs) / 1000))
        }
        return s
    }

    private static let day: DateFormatter = make("EEE d MMM")
    private static let clock: DateFormatter = make("HH:mm")
    private static func make(_ format: String) -> DateFormatter {
        let f = DateFormatter()
        f.dateFormat = format
        return f
    }
}
