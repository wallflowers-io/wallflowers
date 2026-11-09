//  EventDetail.swift
//  The EVENT's maximised detail — its binder, and the one call that raises it.
//
//  EVENTS ARE NOT PROFILES. People and Spaces are egoic and get `Design/Profile`.
//  An event is a thing that HAPPENS — it has an author rather than an identity — so
//  it gets `Design/Detail`, the page every non-egoic GroupObject shares.
//
//  WHAT THE BINDER ANSWERS: the flyer and its photos, when · whether it is still on
//  · where · how to join online · who is playing · the words, then whose night it
//  is and what people said. Ticketing stays hidden (ruling). The OWNER writes each
//  of those rows in place, through `event.setProfile`, which restates the whole
//  profile with the one value changed.

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
                     shell: ShellState, origin: CGRect? = nil) {
        let source = EventDetailSource(core: core, graph: graph, objectId: objectId)
        shell.beginNew(DetailSession(noun: "event", source: source, origin: origin))
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

    func headline() async -> DetailHeadline? {
        guard let row else { return nil }
        let more = core.eventDetail(objectId)
        let owner = core.isGroupOwner(objectId)
        let profile = "event.setProfile"
        let r = DetailBind.rows(owner: owner, core: core)
        let allDay = more?.allDay ?? false

        var rows: [DetailRow] = []
        if owner {
            rows.append(DetailRow(label: "Starts", value: "",
                                  edit: DetailEdit(op: profile, arg: "startMs", input: .date,
                                                   current: String(row.startMs))))
            rows.append(DetailRow(label: "Ends", value: "",
                                  edit: DetailEdit(op: profile, arg: "endMs", input: .date,
                                                   current: row.endMs > 0 ? String(row.endMs) : "")))
            rows.append(DetailRow(label: "All day", value: "",
                                  edit: DetailEdit(op: profile, arg: "allDay", input: .toggle,
                                                   current: allDay ? "1" : "")))
            rows.append(r.text("Time zone", more?.tz ?? "", op: profile, arg: "tz"))
            rows.append(r.choice("Status", more?.status ?? "", op: profile, arg: "status"))
        } else {
            rows.append(DetailRow(label: "When", value: EventWhen.label(startMs: row.startMs, endMs: row.endMs,
                                                                        allDay: allDay)))
            // Scheduled is what an event is unless it says otherwise; only the
            // exceptions are news.
            if let s = more?.status, !s.isEmpty, s != "scheduled" {
                rows.append(DetailRow(label: "Status", value: s.uppercased()))
            }
        }
        let series = core.recurrenceLabel(row.recurrence)
        rows.append(DetailRow(label: "Series", value: series))
        if !row.venuePlace.isEmpty, !owner {
            rows.append(DetailRow(label: "Venue", value: await venueName() ?? row.venuePlaceName,
                                  place: row.venuePlace))
        } else {
            rows.append(r.text("Venue", row.venue, op: profile, arg: "venue"))
        }
        rows.append(r.link("Online", more?.online ?? "", op: profile, arg: "online"))
        rows.append(r.link("Video", more?.videoUrl ?? "", op: profile, arg: "videoUrl"))
        rows.append(r.link("Tickets", more?.ticketUrl ?? "", op: profile, arg: "ticketUrl"))
        let acts = (more?.acts ?? []).map { core.index.name($0.performer) ?? core.objectTitle($0.performer) }
            .filter { !$0.isEmpty }
        if owner || acts.isEmpty {
            // The profile's lineup, one name a line on the wire.
            rows.append(owner
                ? r.text("Lineup", row.lineup.joined(separator: "\n"), op: profile, arg: "lineup", input: .multiline)
                : DetailRow(label: "Lineup", value: row.lineup.joined(separator: " · ")))
        } else {
            rows.append(DetailRow(label: "Lineup", value: acts.joined(separator: " · ")))
        }
        // NOT the MLS roster: an event's members are its organiser and door staff,
        // and calling those attendance would be a confident falsehood. This counts
        // signed `contact.inviteReply` legs, and a night nobody has answered says
        // nothing rather than "0 going".
        let going = core.attendees(of: objectId).count
        if going > 0 { rows.append(DetailRow(label: "Going", value: "\(going) going")) }
        rows.append(r.text("Describe", row.descriptor, op: profile, arg: "descriptor", input: .multiline,
                           markdown: more?.descriptorFormat == "markdown"))
        if owner {
            rows.append(r.choice("Written in", more?.descriptorFormat ?? "", op: profile, arg: "descriptorFormat"))
        }

        // The flyer: `event.setBanner`, else the list row's still.
        let banner = more?.banner.flatMap { Data(base64Encoded: $0.data) }
            ?? (row.banner.isEmpty ? nil : Data(base64Encoded: row.banner))
        return DetailHeadline(
            title: row.title,
            titleEdit: owner ? DetailEdit(op: profile, arg: "title", input: .line, current: row.title) : nil,
            photo: banner,
            gallery: DetailBind.pictures(more?.photos ?? []),
            rows: rows,
            adds: owner ? [.picture] : [])
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

    /// The event's comments section: the Forum minted with it in the comments role
    /// (ICD `kinds.event.parts`), read flat. None until the device holds it.
    func comments() async -> [DetailComment] {
        guard let forum = core.index.commentsForum(of: objectId) else { return [] }
        return core.objectThread(forum).map { row in
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
        await DetailBind.post(text, on: objectId, core: core)
    }

    func edit(_ edit: DetailEdit, to value: String) async -> Bool {
        await DetailBind.edit(edit, to: value, on: objectId, core: core)
    }

    func addPicture(_ data: Data) async -> Bool {
        await core.addPicture(objectId, op: "event.addPhoto", arg: "photo", data, keyed: true)
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
        let names = core.index.names
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
        let rows = [DetailRow(label: "When", value: listing.whenLabel),
                    DetailRow(label: "Venue", value: listing.venueName ?? ""),
                    DetailRow(label: "Lineup", value: listing.lineup ?? "")]
        return DetailHeadline(title: listing.title, photoURL: listing.flyerURL, rows: rows)
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
    static func label(startMs: Int64, endMs: Int64, allDay: Bool = false) -> String {
        let start = Date(timeIntervalSince1970: Double(startMs) / 1000)
        if allDay { return day.string(from: start) }
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
