//  EventInterior.swift
//  The EVENT's page as a member opens it — its content, its sections, and the call that
//  raises it on KindPage.
//
//  EVENTS ARE NOT PROFILES. An event is a thing that HAPPENS — it has hosts rather than
//  an identity — so it gets `Design/Kind/`, the page every non-egoic GroupObject shares.
//
//  ONE CONTENT, TWO LAYERS. Everything on this page is `EventContent`, which an outsider
//  reads too (KindExterior, from the hosted copy). Joining adds only authorship: Edit for
//  the owner and co-hosts, and the comments. The order is Luma's (D17): the hero · title
//  · when · actions · location · hosts · lineup · about · tickets · going, then comments.

import SwiftUI
import MapKit
import EventKit
import EventKitUI
import PacificCore
import ArcResolverKit   // ArcFact — the happens_at edge the venue reads

// MARK: - Raising it

@MainActor
enum EventInterior {

    /// EVENTS' own palette slot — the same one the jewel and the `New Event`
    /// surface are cast in (`NewKind.colorIndex` = 0).
    static var tint: Color { PaletteStore.base(0) }

    /// Open a held Event. One call from every surface that lists events.
    static func open(_ objectId: String, core: CoreService, graph: GraphService,
                     shell: ShellState, origin: CGRect? = nil) {
        let source = EventInteriorSource(core: core, graph: graph, objectId: objectId)
        shell.beginNew(KindSession(noun: "event", source: source, origin: origin))
    }

    /// Open a WORLD LISTING — an RA or ArtRabbit row nobody has minted. It has no
    /// log to speak into, so BLUE is not "say something": it is MINT, the one
    /// WallFlowers act available on a row that is not yet an object. Minted from this
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
        let session = KindSession(
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

// MARK: - The content

/// Everything the Event's page shows, member or not. FILLED FROM THE FOLD, through the
/// rows the FFI has today (`EventRow`, `EventDetailRow`); the one place to swap for
/// core's `object_fields(id)` once it lands (BLUE EXTERIOR's `fields_of`).
struct EventContent {
    struct Act: Identifiable {
        let id: String
        let name: String
        let role: String
        let start: Int64
        let end: Int64
        var photo: Data? = nil
    }

    struct Host: Identifiable {
        let id: String
        let name: String
        let isSpace: Bool
        var photo: Data? = nil
        var color: Color? = nil
    }

    var title = ""
    var descriptor = ""
    var markdown = false
    var startMs: Int64 = 0
    var endMs: Int64 = 0
    var allDay = false
    var tz = ""
    var recurrence = ""
    var status = ""
    var online = ""
    var videoUrl = ""
    var ticketUrl = ""
    /// The free-text venue words, and the Place it is bound to (`event.setVenue`).
    var venue = ""
    var venuePlace = ""
    /// Where it is, when it says: the Event's point, else its Place's.
    var point: CLLocationCoordinate2D?
    var banner: Data?
    var photos: [KindPicture] = []
    var clip: KindClip?
    var acts: [Act] = []
    /// `setProfile.lineup` — old events only (`setLineup` replaces it).
    var legacyLineup: [String] = []
    var priceCents: Int64 = 0
    var currency = ""
    var capacity: Int64 = 0
    var hosts: [Host] = []
    /// Who is coming: the roster, less its hosts. A number; no names.
    var going = 0

    @MainActor
    static func fold(_ objectId: String, core: CoreService) -> EventContent? {
        guard let row = core.events.first(where: { $0.id == objectId }) else { return nil }
        let more = core.eventDetail(objectId)
        var c = EventContent()
        c.title = row.title
        c.descriptor = row.descriptor
        c.markdown = more?.descriptorFormat == "markdown"
        c.startMs = row.startMs
        c.endMs = row.endMs
        c.allDay = more?.allDay ?? false
        c.tz = more?.tz ?? ""
        c.recurrence = row.recurrence
        c.status = more?.status ?? ""
        c.online = more?.online ?? ""
        c.videoUrl = more?.videoUrl ?? ""
        c.ticketUrl = more?.ticketUrl ?? ""
        c.venue = row.venue
        c.venuePlace = row.venuePlace
        if row.placed {
            c.point = CLLocationCoordinate2D(latitude: row.lat, longitude: row.lng)
        } else if let place = core.places.first(where: { $0.id == row.venuePlace }), place.placed {
            c.point = CLLocationCoordinate2D(latitude: place.lat, longitude: place.lng)
        }
        c.banner = more?.banner.flatMap { Data(base64Encoded: $0.data) }
            ?? (row.banner.isEmpty ? nil : Data(base64Encoded: row.banner))
        c.photos = KindBind.pictures(more?.photos ?? [])
        c.clip = more?.clip.flatMap { m in Data(base64Encoded: m.data).map { KindClip(data: $0, mime: m.mime) } }
        c.acts = (more?.acts ?? []).map { a in
            let name = core.index.name(a.performer) ?? core.objectTitle(a.performer)
            return Act(id: a.performer, name: name.isEmpty ? String(a.performer.prefix(8)) : name,
                       role: a.role, start: a.start, end: a.end, photo: core.index.photo(a.performer))
        }
        c.legacyLineup = row.lineup
        c.priceCents = row.priceCents
        c.currency = row.currency
        c.capacity = row.capacity

        // HOSTS: the Space that made it, and its owner. Co-hosts (base.setRole admin)
        // join this list when their roles can be read here (PR B's object_roles).
        let owner = core.objectOwner(objectId)
        if let maker = core.creatorOfObject(objectId), maker.id != owner {
            let isSpace = core.spaces.contains { $0.id == maker.id }
            let name = maker.name.isEmpty ? (core.index.name(maker.id) ?? String(maker.id.prefix(8))) : maker.name
            c.hosts.append(Host(id: maker.id, name: name, isSpace: isSpace,
                                photo: isSpace ? nil : core.index.photo(maker.id), color: EventInterior.tint))
        }
        if !owner.isEmpty {
            let name = owner == core.myIdentityHex
                ? (core.displayName.isEmpty ? "You" : core.displayName)
                : (core.index.name(owner) ?? String(owner.prefix(8)))
            c.hosts.append(Host(id: owner, name: name, isSpace: false, photo: core.index.photo(owner),
                                color: core.index.color(owner)))
        }
        let hostIDs = Set(c.hosts.map(\.id))
        c.going = Set(core.objectMembers(objectId)).subtracting(hostIDs).count
        return c
    }
}

// MARK: - The binder

/// The binder for a held Event: its content as KindPage sections, and its writes.
@MainActor
struct EventInteriorSource: KindSource {
    let core: CoreService
    let graph: GraphService
    let objectId: String

    /// Every profile edit is `event.editProfile` (op 13): the owner's and a co-host's
    /// alike, one register, LWW by (gen, author).
    static let profile = "event.editProfile"

    func canEdit() async -> Bool { core.isGroupOwner(objectId) }

    func headline() async -> KindHeadline? {
        guard var c = EventContent.fold(objectId, core: core) else { return nil }
        if !c.venuePlace.isEmpty, let named = await venueName() { c.venue = named }
        let editor = await canEdit()
        let owner = core.isGroupOwner(objectId)
        let r = KindBind.rows(owner: editor, core: core)
        let p = Self.profile
        let content = c

        // WHEN: in View one composed line, its series, and any exception to "scheduled";
        // in Edit each field it is made of.
        var when: [KindRow] = [
            KindRow(label: "When", value: EventWhen.label(startMs: c.startMs, endMs: c.endMs, allDay: c.allDay)
                    + EventWhen.zone(c.tz)),
            KindRow(label: "Series", value: core.recurrenceLabel(c.recurrence)),
        ]
        var starts = KindRow(label: "Starts", value: "",
                             edit: editor ? KindEdit(op: p, arg: "startMs", input: .date, current: String(c.startMs)) : nil)
        var ends = KindRow(label: "Ends", value: "",
                           edit: editor ? KindEdit(op: p, arg: "endMs", input: .date,
                                                   current: c.endMs > 0 ? String(c.endMs) : "") : nil)
        var allDay = KindRow(label: "All day", value: "",
                             edit: editor ? KindEdit(op: p, arg: "allDay", input: .toggle, current: c.allDay ? "1" : "") : nil)
        var zone = r.text("Time zone", c.tz, op: p, arg: "tz")
        starts.editOnly = true; ends.editOnly = true; allDay.editOnly = true; zone.editOnly = true
        when += [starts, ends, allDay, zone]
        let statusEdit = r.choice("Status", c.status, op: p, arg: "status").edit
        let exceptional = !c.status.isEmpty && c.status != "scheduled"
        when.append(KindRow(label: "Status", value: exceptional ? c.status.uppercased() : "", edit: statusEdit))
        when.append(r.link("Online", c.online, op: p, arg: "online"))
        when.append(r.link("Video", c.videoUrl, op: p, arg: "videoUrl"))

        var venueText = r.text("Venue", c.venue, op: p, arg: "venue")
        venueText.editOnly = true

        let about = r.text("Describe", c.descriptor, op: p, arg: "descriptor", input: .multiline, markdown: c.markdown)
        var format = r.choice("Written in", c.markdown ? "markdown" : "", op: p, arg: "descriptorFormat")
        format.editOnly = true

        var tickets: [KindRow] = []
        if c.priceCents > 0 {
            tickets.append(KindRow(label: "Price", value: EventWhen.price(c.priceCents, c.currency)))
        }
        tickets.append(r.link("Tickets", c.ticketUrl, op: p, arg: "ticketUrl"))

        let blocks: [KindBlock] = [
            .rows("when", when),
            .view("actions") { ctx in AnyView(EventActions(content: content, core: core, ctx: ctx)) },
            .view("location") { ctx in
                AnyView(EventLocation(content: content, owner: owner, ctx: ctx) { place, name in
                    core.setEventVenue(objectId, placeId: place, name: name)
                })
            },
            .rows("venue", [venueText]),
            .view("hosts") { ctx in AnyView(EventHosts(hosts: content.hosts, ctx: ctx)) },
            .view("lineup") { ctx in
                AnyView(EventLineup(content: content, editor: editor, ctx: ctx, core: core) { acts in
                    await setLineup(acts)
                })
            },
            .rows("about", [about, format]),
            .rows("tickets", tickets),
            .view("going") { _ in AnyView(EventGoing(going: content.going, capacity: content.capacity)) },
        ]
        return KindHeadline(
            title: c.title,
            titleEdit: editor ? KindEdit(op: p, arg: "title", input: .line, current: c.title) : nil,
            photo: c.banner,
            gallery: c.photos,
            clip: c.clip,
            blocks: blocks,
            adds: editor ? [.cover, .picture] : [])
    }

    func author() async -> KindAuthor? { KindBind.author(of: objectId, core: core, tint: EventInterior.tint) }

    /// The event's comments section: the Forum minted with it in the comments role
    /// (ICD `kinds.event.parts`), read flat. None until the device holds it.
    func comments() async -> [KindComment] { KindBind.comments(of: objectId, core: core) }

    func commentsHeld() async -> Bool { core.index.commentsForum(of: objectId) != nil }

    func post(_ text: String) async -> Bool { await KindBind.post(text, on: objectId, core: core) }

    func edit(_ edit: KindEdit, to value: String) async -> Bool {
        await KindBind.edit(edit, to: value, on: objectId, core: core)
    }

    func addPicture(_ data: Data) async -> Bool {
        await core.addPicture(objectId, op: "event.addPhoto", arg: "photo", data, keyed: true)
    }

    func setCover(_ data: Data) async -> Bool {
        await core.addPicture(objectId, op: "event.setBanner", arg: "banner", data, keyed: false)
    }

    func removePicture(_ key: String) async -> Bool {
        await core.apply(objectId, op: "event.removePhoto", ["id": .text(v: key)])
    }

    /// `event.setLineup`: the whole list, in billing order. A person by `member`, an
    /// organisation or band by `object`. `setProfile.lineup` is no longer written.
    func setLineup(_ acts: [EventContent.Act]) async -> Bool {
        let people = Set(core.connectionRows().map(\.peerId)).union([core.myIdentityHex])
        let list: [[String: Any]] = acts.map { a in
            var o: [String: Any] = [people.contains(a.id) ? "member" : "object": a.id, "role": a.role]
            if a.start > 0 { o["start"] = a.start }
            if a.end > 0, a.end >= a.start { o["end"] = a.end }
            return o
        }
        guard let json = try? JSONSerialization.data(withJSONObject: list),
              let text = String(data: json, encoding: .utf8) else { return false }
        return await core.apply(objectId, op: "event.setLineup", ["acts": .text(v: text)])
    }

    /// The Spaces this can reach: groups whose roster I am in.
    func shareTargets() async -> [KindShareTarget] {
        core.spaces.map { KindShareTarget(id: $0.id, name: $0.name.isEmpty ? String($0.id.prefix(8)) : $0.name) }
    }

    /// The edge is authored on the SPACE's log, not the event's — `peer`, which the
    /// projector mints as `affiliated_with`. Two claims about the same object, each on
    /// its author's own log, neither able to overwrite the other.
    func share(to target: KindShareTarget) async -> Bool {
        guard let row = core.events.first(where: { $0.id == objectId }) else { return false }
        core.setAffiliation(target.id, peer: objectId, rel: "peer", name: row.title)
        return true
    }

    /// The live `happens_at` edge, named by the GRAPH; the folded row's name stands in
    /// until the projection lands.
    private func venueName() async -> String? {
        guard let row = core.events.first(where: { $0.id == objectId }) else { return nil }
        if let fact = await graph.happensAt(eventId: objectId) {
            return (await graph.entity(id: fact.dst))?.name ?? fact.dst
        }
        return row.venuePlaceName.isEmpty ? nil : row.venuePlaceName
    }
}

// MARK: - Sections

/// A section's caption: the one label a section carries.
private struct EventCaption: View {
    let text: String
    var body: some View {
        Text(text.uppercased())
            .font(.system(size: 11, weight: .semibold, design: .monospaced))
            .kerning(1.4)
            .foregroundStyle(Theme.textTertiary)
    }
}

/// ACTIONS: computed from the content, nothing written — into the calendar, directions
/// to its point, a message to its host where you are already connected.
private struct EventActions: View {
    let content: EventContent
    let core: CoreService
    let ctx: KindContext
    @State private var adding = false

    private var hostPeer: String? {
        guard let owner = content.hosts.first(where: { !$0.isSpace })?.id, owner != core.myIdentityHex else { return nil }
        return core.connectionRows().contains { $0.peerId == owner && $0.connected } ? owner : nil
    }

    var body: some View {
        if !ctx.editing {
            HStack(spacing: 10) {
                if content.startMs > 0 {
                    action("Calendar", "calendar.badge.plus", id: "detail-action-calendar") { adding = true }
                }
                if let point = content.point {
                    action("Directions", "arrow.triangle.turn.up.right.diamond", id: "detail-action-directions") {
                        let item = MKMapItem(placemark: MKPlacemark(coordinate: point))
                        item.name = content.venue.isEmpty ? content.title : content.venue
                        item.openInMaps(launchOptions: [MKLaunchOptionsDirectionsModeKey: MKLaunchOptionsDirectionsModeDefault])
                    }
                }
                if let peer = hostPeer {
                    action("Message", "bubble.left", id: "detail-action-contact") {
                        ctx.leave { NotificationCenter.default.post(name: .pacificOpenDM, object: nil, userInfo: ["peer": peer]) }
                    }
                }
            }
            .padding(.horizontal, 18)
            .padding(.top, 14)
            .sheet(isPresented: $adding) { EventCalendarSheet(content: content) { adding = false } }
        }
    }

    private func action(_ label: String, _ glyph: String, id: String, _ run: @escaping () -> Void) -> some View {
        Button(action: run) {
            Label(label, systemImage: glyph)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Theme.textPrimary)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Capsule().fill(Theme.fill.opacity(0.6)))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(id)
    }
}

/// EventKit's own editor, filled from the content: the person saves it into their
/// calendar, or not. WallFlowers writes nothing.
private struct EventCalendarSheet: UIViewControllerRepresentable {
    let content: EventContent
    var done: () -> Void

    func makeUIViewController(context: Context) -> EKEventEditViewController {
        let store = EKEventStore()
        let e = EKEvent(eventStore: store)
        e.title = content.title
        e.startDate = Date(timeIntervalSince1970: Double(content.startMs) / 1000)
        e.endDate = content.endMs > content.startMs
            ? Date(timeIntervalSince1970: Double(content.endMs) / 1000)
            : e.startDate
        e.isAllDay = content.allDay
        if !content.tz.isEmpty { e.timeZone = TimeZone(identifier: content.tz) }
        e.location = content.venue.isEmpty ? nil : content.venue
        e.url = URL(string: content.online).flatMap { $0.scheme == nil ? nil : $0 }
        let vc = EKEventEditViewController()
        vc.eventStore = store
        vc.event = e
        vc.editViewDelegate = context.coordinator
        return vc
    }

    func updateUIViewController(_ vc: EKEventEditViewController, context: Context) {}
    func makeCoordinator() -> Coordinator { Coordinator(done: done) }

    final class Coordinator: NSObject, EKEventEditViewDelegate {
        let done: () -> Void
        init(done: @escaping () -> Void) { self.done = done }
        func eventEditViewController(_ c: EKEventEditViewController, didCompleteWith action: EKEventEditViewAction) { done() }
    }
}

/// LOCATION: the Place, a door to its page, and a map with its pin. In Edit the owner
/// binds a Place (`event.setVenue`, owner-only in the ICD).
private struct EventLocation: View {
    let content: EventContent
    let owner: Bool
    let ctx: KindContext
    var bind: (String, String) -> Bool
    @State private var choosing = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if ctx.editing {
                if owner {
                    Button { choosing = true } label: {
                        FieldRow(label: "Place") {
                            Text(content.venuePlace.isEmpty ? "Choose" : content.venue)
                                .font(.system(size: 15))
                                .foregroundStyle(Theme.textPrimary)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("detail-edit-place")
                }
            } else {
                if !content.venuePlace.isEmpty {
                    Button {
                        let id = content.venuePlace
                        ctx.leave { NotificationCenter.default.post(name: .pacificOpenPlace, object: nil, userInfo: ["object": id]) }
                    } label: {
                        HStack(spacing: 6) {
                            Text(content.venue).font(.system(size: 15)).foregroundStyle(Theme.textPrimary)
                            Image(systemName: "chevron.right").font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(Theme.textTertiary)
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("detail-venue")
                } else if !content.venue.isEmpty {
                    Text(content.venue).font(.system(size: 15)).foregroundStyle(Theme.textSecondary)
                        .accessibilityIdentifier("detail-venue")
                }
                if let point = content.point {
                    Map(initialPosition: .region(MKCoordinateRegion(center: point, latitudinalMeters: 900,
                                                                    longitudinalMeters: 900)),
                        interactionModes: []) {
                        Marker(content.venue, coordinate: point).tint(EventInterior.tint)
                    }
                    .frame(height: 160)
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .accessibilityIdentifier("detail-map")
                }
            }
        }
        .padding(.horizontal, 18)
        .padding(.top, 14)
        .sheet(isPresented: $choosing) {
            EventPlacePicker(current: content.venuePlace) { place, name in
                if bind(place, name) { Task { await ctx.reload() } }
                choosing = false
            }
        }
    }
}

/// HOSTS (D16): who is behind it — the Space that made it and its owner — each a door
/// to their page. The card names the same author (`KindBind.author`).
private struct EventHosts: View {
    let hosts: [EventContent.Host]
    let ctx: KindContext

    var body: some View {
        if !hosts.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                EventCaption(text: hosts.count == 1 ? "Host" : "Hosts")
                    .padding(.horizontal, 18)
                    .padding(.bottom, 4)
                ForEach(hosts) { h in
                    Button {
                        let id = h.id, space = h.isSpace
                        ctx.leave {
                            NotificationCenter.default.post(name: space ? .pacificOpenGroup : .pacificOpenPerson,
                                                            object: nil, userInfo: ["object": id])
                        }
                    } label: {
                        ConversationRow(title: h.name, snippet: "", time: h.isSpace ? "SPACE" : "PERSON",
                                        tint: h.color ?? EventInterior.tint, initials: Avatar.initials(h.name),
                                        blockColor: h.color ?? EventInterior.tint, enrichSnippet: false,
                                        avatarSize: 36)
                            .padding(.horizontal, 18)
                            .frame(height: ProfileGeometry.bar)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("detail-host")
                }
            }
            .padding(.top, 18)
        }
    }
}

/// LINEUP (D13): a stack of faces that opens the bill — each act's name, role and set
/// times. In Edit (D14) the bill is written as `event.setLineup`; old events' text lineup
/// is read, never written.
private struct EventLineup: View {
    let content: EventContent
    let editor: Bool
    let ctx: KindContext
    let core: CoreService
    var write: ([EventContent.Act]) async -> Bool

    @State private var open = false
    @State private var picking = false
    @State private var saving = false

    var body: some View {
        if ctx.editing && editor {
            editing
        } else if !content.acts.isEmpty {
            Button { open = true } label: {
                HStack(spacing: 12) {
                    HStack(spacing: -10) {
                        ForEach(content.acts.prefix(5)) { a in
                            Avatar(initials: Avatar.initials(a.name), tint: Theme.fill, size: 34, photo: a.photo,
                                   color: EventInterior.tint)
                                .overlay(Circle().stroke(Theme.bg, lineWidth: 2))
                        }
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        EventCaption(text: "Lineup")
                        Text(content.acts.map(\.name).joined(separator: " · "))
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundStyle(Theme.textPrimary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Theme.textTertiary)
                }
                .padding(.horizontal, 18)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.top, 18)
            .accessibilityIdentifier("detail-lineup")
            .sheet(isPresented: $open) { bill.presentationDetents([.medium, .large]) }
        } else if !content.legacyLineup.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                EventCaption(text: "Lineup")
                Text(content.legacyLineup.joined(separator: " · "))
                    .font(.system(size: 15)).foregroundStyle(Theme.textSecondary)
            }
            .padding(.horizontal, 18)
            .padding(.top, 18)
            .accessibilityIdentifier("detail-lineup")
        }
    }

    private var bill: some View {
        NavigationStack {
            List(content.acts) { a in actRow(a) }
                .navigationTitle("Lineup")
                .navigationBarTitleDisplayMode(.inline)
        }
        .accessibilityIdentifier("detail-lineup-sheet")
    }

    private func actRow(_ a: EventContent.Act) -> some View {
        HStack(spacing: 12) {
            Avatar(initials: Avatar.initials(a.name), tint: Theme.fill, size: 36, photo: a.photo, color: EventInterior.tint)
            VStack(alignment: .leading, spacing: 2) {
                Text(a.name).font(.system(size: 15, weight: .semibold))
                Text([a.role.uppercased(), EventWhen.set(a.start, a.end)].filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(Theme.textTertiary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("detail-act")
    }

    /// The bill as its writer sees it: each act, its role, a removal; and an add.
    private var editing: some View {
        let roles = core.vocabulary("event.setLineup", "acts.role")
        return VStack(alignment: .leading, spacing: 8) {
            EventCaption(text: "Lineup")
            ForEach(Array(content.acts.enumerated()), id: \.element.id) { i, a in
                HStack(spacing: 10) {
                    Text(a.name).font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.textPrimary)
                    Spacer()
                    Menu {
                        ForEach(roles, id: \.self) { r in
                            Button(r) {
                                save { $0[i] = EventContent.Act(id: a.id, name: a.name, role: r, start: a.start, end: a.end) }
                            }
                        }
                    } label: {
                        Text(a.role.uppercased()).font(.system(size: 12, weight: .semibold, design: .monospaced))
                    }
                    .accessibilityIdentifier("detail-act-role")
                    Button { save { $0.remove(at: i) } } label: { Image(systemName: "minus.circle") }
                        .accessibilityIdentifier("detail-act-remove")
                }
                .disabled(saving)
            }
            Button { picking = true } label: {
                Label("Add act", systemImage: "plus").font(.system(size: 13, weight: .semibold))
            }
            .disabled(saving || roles.isEmpty)
            .accessibilityIdentifier("detail-act-add")
        }
        .padding(.horizontal, 18)
        .padding(.top, 18)
        .sheet(isPresented: $picking) {
            KindPicker(title: "Add act", sources: [.people, .spaces], exclude: Set(content.acts.map(\.id))) { pick in
                picking = false
                // The ICD's own word for an act with no billing yet, else its first.
                let role = roles.contains("performer") ? "performer" : roles[0]
                save { $0.append(EventContent.Act(id: pick.id, name: pick.name, role: role, start: 0, end: 0)) }
            }
        }
    }

    private func save(_ change: (inout [EventContent.Act]) -> Void) {
        var acts = content.acts
        change(&acts)
        saving = true
        Task {
            if await write(acts) { await ctx.reload() }
            saving = false
        }
    }
}

/// GOING: how many are coming, against the capacity when there is one. A number: who is
/// coming is not on the page.
private struct EventGoing: View {
    let going: Int
    let capacity: Int64

    var body: some View {
        if going > 0 || capacity > 0 {
            Text(capacity > 0 ? "\(going) going · of \(capacity)" : "\(going) going")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(Theme.textPrimary)
                .padding(.horizontal, 18)
                .padding(.top, 14)
                .accessibilityIdentifier("detail-going")
        }
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
struct ListingDetailSource: KindSource {
    let listing: ReferenceListing

    func headline() async -> KindHeadline? {
        let rows = [KindRow(label: "When", value: listing.whenLabel),
                    KindRow(label: "Venue", value: listing.venueName ?? ""),
                    KindRow(label: "Lineup", value: listing.lineup ?? "")]
        return KindHeadline(title: listing.title, photoURL: listing.flyerURL, rows: rows)
    }

    func author() async -> KindAuthor? { nil }
    func comments() async -> [KindComment] { [] }
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

    /// " (Europe/Paris)" when the event's zone is not this device's; "" otherwise.
    static func zone(_ tz: String) -> String {
        tz.isEmpty || tz == TimeZone.current.identifier ? "" : " (\(tz))"
    }

    /// An act's set: "21:00 – 22:30", or the start alone; "" when not said.
    static func set(_ start: Int64, _ end: Int64) -> String {
        guard start > 0 else { return "" }
        var s = clock.string(from: Date(timeIntervalSince1970: Double(start) / 1000))
        if end > start { s += " – " + clock.string(from: Date(timeIntervalSince1970: Double(end) / 1000)) }
        return s
    }

    /// A price from `event.setTickets`: minor units, in its currency when it names one.
    static func price(_ cents: Int64, _ currency: String) -> String {
        let amount = Double(cents) / 100
        guard !currency.isEmpty else { return String(format: "%.2f", amount) }
        let f = NumberFormatter()
        f.numberStyle = .currency
        f.currencyCode = currency.uppercased()
        return f.string(from: NSNumber(value: amount)) ?? String(format: "%.2f %@", amount, currency.uppercased())
    }

    private static let day: DateFormatter = make("EEE d MMM")
    private static let clock: DateFormatter = make("HH:mm")
    private static func make(_ format: String) -> DateFormatter {
        let f = DateFormatter()
        f.dateFormat = format
        return f
    }
}
