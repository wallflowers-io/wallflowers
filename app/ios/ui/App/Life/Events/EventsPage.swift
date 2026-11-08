//  EventsPage.swift
//  The EVENTS skin over the LIFE tab — the "Three Channels" design's blue channel,
//  ported. Two lenses on the same commitments: NEAR (flyer cards, soonest first) and
//  DATES (day-grouped list). The MAP lens was consolidated away (1 Aug): THE map is the
//  fullscreen one under the MAP jewel, and locked EVENTS overlays its pins there — see
//  `EventsPage.venuePins` + `PlacesMap.events`. A presentation SKIN, not a parallel
//  store: it renders the EventVMs the ribbon reads (EventsSource × the LifeQuery
//  terms), resolves venues against the placed PLACES, and holds no event data of its
//  own. The page wears the blue channel's identity: its accent IS the LIFE disc's
//  block colour (see DiscSlot — the interface matrix is the home of channel colour),
//  so the page reads as the interior of the disc you arrived through.
//
//  GOING is a DELTA, not a device flag. It is a signed `contact.inviteReply` on the
//  pairwise channel with whoever invited you, so the organiser actually learns it and
//  it follows you to another device. `CoreService.goingEvents` derives the set from
//  those replies; nothing is stored here.
//
//  That splits what the old going-mark conflated. Saying you are coming is a
//  COMMITMENT and needs a counterparty; being reminded is a PERSONAL alarm and needs
//  nobody. A row you were invited to offers the commitment; a row from a world feed
//  (RA, ArtRabbit) has no organiser to tell, so it offers only the reminder — which
//  is honest, where the old dot claimed an RSVP nobody would ever receive.
//
//  The reminder is held by the OS (a pending UNNotificationRequest IS the record),
//  so retiring the going-set removed the persistence rather than moving it.
//  No fabricated counts either — a row shows YOUR state, never a "184 going".

import SwiftUI
import CoreLocation
import UserNotifications

// MARK: - Reminders (the OS holds them; nothing is persisted here)

/// Which events have a local reminder pending. The store of truth is
/// `UNUserNotificationCenter` — this is a published mirror so SwiftUI can render it,
/// rehydrated from the notification centre rather than from any file of our own.
@MainActor
final class EventReminders: ObservableObject {
    static let shared = EventReminders()
    @Published private(set) var armed: Set<String> = []

    private init() {}

    func isArmed(_ id: String) -> Bool { armed.contains(id) }

    /// Re-read the OS. Called on appear, so a reminder set on another launch (or one
    /// the system already delivered and dropped) is reflected rather than remembered.
    func refresh() async {
        let pending = await UNUserNotificationCenter.current().pendingNotificationRequests()
        armed = Set(pending.compactMap { $0.identifier.hasPrefix("rsvp-")
            ? String($0.identifier.dropFirst(5)) : nil })
    }

    /// Arm or cancel. Best-effort: with notifications denied there is simply no
    /// wake-up, and nothing else about the row changes.
    ///
    /// ARMING IS THE ASK. This is one of the two moments a wake is genuinely
    /// wanted (the other is accepting a connection), so the permission is
    /// requested HERE and awaited before the request is added — an alert raised
    /// over the event the user just marked, rather than at launch over nothing.
    /// Adding before the answer would silently drop the reminder.
    func toggle(_ id: String, title: String, startMs: Int64, venue: String?) {
        let centre = UNUserNotificationCenter.current()
        if armed.contains(id) {
            armed.remove(id)
            centre.removePendingNotificationRequests(withIdentifiers: ["rsvp-\(id)"])
            return
        }
        guard let fire = RsvpReminder.fireDate(startMs: startMs) else { return }
        armed.insert(id)
        let c = UNMutableNotificationContent()
        c.title = title
        if let venue { c.body = venue }
        c.sound = .default
        c.userInfo = ["rsvpEvent": id]
        Task {
            guard await WakePermission.request() else {
                armed.remove(id)   // no wake is possible; do not claim one
                return
            }
            // Re-measure AFTER the answer: the user may have sat on the alert, and
            // a non-positive interval is a trigger iOS refuses.
            let delay = fire.timeIntervalSinceNow
            guard delay > 0 else { armed.remove(id); return }
            try? await centre.add(UNNotificationRequest(
                identifier: "rsvp-\(id)", content: c,
                trigger: UNTimeIntervalNotificationTrigger(
                    timeInterval: delay, repeats: false)))
        }
    }
}

/// The reminder a going-mark earns — pure, so the clamp rules are testable: 2h before
/// the start, never in the past, and none at all for something already begun or about
/// to begin (a reminder that fires after the doors open is noise, not help). Multi-day
/// runs whose start is behind us get no reminder either — there is nothing to be late for.
enum RsvpReminder {
    static let lead: TimeInterval = 2 * 3600
    static let minNotice: TimeInterval = 120

    static func fireDate(startMs: Int64, now: Date = Date()) -> Date? {
        let start = Date(timeIntervalSince1970: Double(startMs) / 1000)
        guard start.timeIntervalSince(now) > minNotice else { return nil }
        return max(start.addingTimeInterval(-lead), now.addingTimeInterval(60))
    }
}

// MARK: - The page

struct EventsPage: View {
    let source: any EventsSource
    let terms: [FilterTerm]
    /// The placed PLACES — venue resolution (a commitment's `.at` hop) and the MAP's pins.
    let places: [PlaceAnnotation]
    /// Ticket state for GroupObject-backed rows, keyed by VM id — the capsule's contract.
    /// State in, intents out; the page holds none of it (see EventTicketing).
    var ticketing = EventTicketing()
    /// Present ⇒ the page offers composing an event (the ＋ and the empty state's capsule).
    var onCompose: (() -> Void)? = nil
    /// Present ⇒ tapping a minted (GroupObject-backed) card or row opens the event's
    /// page. Handed the VM id ("obj:<objectId>"); the capsule/RSVP taps stay their own.
    var onOpen: ((String) -> Void)? = nil
    /// Present ⇒ a row with no GroupObject and no feed page (graph / market / calendar)
    /// still opens, as a REFERENCE built from the row itself. Without this those rows
    /// would have nowhere to go, which is how the RSVP fallback got there in the first
    /// place.
    var onOpenReference: ((ReferenceListing) -> Void)? = nil

    @EnvironmentObject private var location: LocationService
    @StateObject private var reminders = EventReminders.shared
    @EnvironmentObject private var core: CoreService
    /// The lens at rest. DEBUG: a headless drive lands on DATES directly via the
    /// `-pacificEventsLens dates` launch default — simctl cannot tap the tab.
    @State private var mode: Mode = {
        #if DEBUG
        if let lens = UserDefaults.standard.string(forKey: "pacificEventsLens"),
           let m = Mode(rawValue: lens.uppercased()) { return m }
        #endif
        return .near
    }()
    @State private var query = ""

    enum Mode: String, CaseIterable {
        case near = "NEAR", dates = "DATES", tickets = "TICKETS"
    }

    /// The blue channel's identity — the LIFE disc's own colour, via the matrix.
    static let accent = DiscSlot.life.blockColor
    /// The soft blue the design uses for kickers, date headers and distances.
    static let accentSoft = DiscSlot.life.blockColor.lightened(0.35)

    var body: some View {
        ZStack(alignment: .top) {
            // The channel glow — the wash the prototype pours from the top of the screen.
            // Unframed: the gradient reaches clear on its own well inside the screen, so
            // there is no frame edge to print a seam across the ground.
            RadialGradient(colors: [Self.accent.opacity(0.16), .clear],
                           center: .top, startRadius: 0, endRadius: 360)
                .ignoresSafeArea()
                .allowsHitTesting(false)

            VStack(spacing: 0) {
                searchPill
                    .padding(.horizontal, 20)
                    .padding(.bottom, 6)
                tabs
                    .padding(.horizontal, 20)

                if mode == .tickets {
                    // The wallet scrolls itself (it is a List) — the same view the tray's
                    // VIEW presents, embedded rather than re-implemented.
                    TicketWalletView()
                } else {
                    ScrollView {
                        Group {
                            switch mode {
                            case .near:    nearList
                            case .dates:   datesList
                            case .tickets: EmptyView()
                            }
                        }
                        .padding(.bottom, 24)
                        .animation(.easeOut(duration: 0.28), value: mode)
                    }
                    .scrollIndicators(.hidden)
                }
            }
        }
        // Warm every card's poster the moment the rows exist — before anything
        // scrolls — so images land once and never pop in (see PosterStore).
        .onAppear { PosterStore.shared.prefetch(cards.map(\.flyerURL)) }
        .task { await reminders.refresh() }
        .onChange(of: cards.map(\.flyerURL)) { _, urls in
            PosterStore.shared.prefetch(urls)
        }
    }

    // MARK: Chrome — the search pill and the three lens tabs

    private var searchPill: some View {
        HStack(spacing: 12) {
            HStack(spacing: 12) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(Theme.textTertiary)
                TextField("Search events, venues", text: $query)
                    .font(.system(size: 17))
                    .foregroundStyle(Theme.textPrimary)
                    .autocorrectionDisabled()
            }
            .padding(.horizontal, 18)
            .frame(height: 52)
            .background(Theme.fill, in: Capsule())

            if let onCompose {
                Button(action: onCompose) {
                    Image(systemName: "plus")
                        .font(.system(size: 18, weight: .semibold))
                        .foregroundStyle(Self.accentSoft)
                        .frame(width: 52, height: 52)
                        .background(Theme.fill, in: Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("New event")
                .accessibilityIdentifier("events-new")
            }
        }
    }

    /// NEAR and DATES sit left; the hairline runs the gap; TICKETS holds the right edge.
    private var tabs: some View {
        HStack(alignment: .bottom, spacing: 22) {
            tab(.near)
            tab(.dates)
            Rectangle()
                .fill(Theme.hairline.opacity(0.5))
                .frame(height: 2)
                .frame(maxWidth: .infinity, alignment: .bottom)
            tab(.tickets)
        }
    }

    private func tab(_ m: Mode) -> some View {
        VStack(spacing: 10) {
            Text(m.rawValue)
                .font(.system(size: 15, weight: .bold))
                .tracking(1.2)
                .foregroundStyle(mode == m ? Theme.textPrimary : Theme.textTertiary)
            Rectangle()
                .fill(mode == m ? Self.accent : Theme.hairline.opacity(0.5))
                .frame(height: 2)
        }
        .fixedSize()
        .contentShape(Rectangle())
        .onTapGesture { withAnimation(.easeOut(duration: 0.18)) { mode = m } }
    }

    // MARK: NEAR — flyer cards, soonest first

    private var nearList: some View {
        VStack(spacing: 14) {
            if cards.isEmpty { emptyWord }
            ForEach(cards) { ev in
                NearCard(event: ev, going: isGoing(ev), committable: canCommit(ev),
                         toggle: { mark(ev) },
                         buy: { ticketing.buy(ev.id) },
                         door: { ticketing.door(ev.id) },
                         open: openAction(ev))
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 18)
    }

    // MARK: DATES — day-grouped, past included (a past commitment remains true)

    private var datesList: some View {
        VStack(spacing: 0) {
            if dateGroups.isEmpty { emptyWord.padding(.top, 18) }
            ForEach(dateGroups, id: \.label) { group in
                Text(group.label)
                    .font(.system(size: 12, weight: .heavy))
                    .tracking(1.7)
                    .foregroundStyle(Self.accentSoft)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 20)
                    .padding(.top, 18)
                    .padding(.bottom, 8)
                ForEach(group.items) { ev in
                    DateRow(event: ev, going: isGoing(ev), committable: canCommit(ev),
                            toggle: { mark(ev) },
                            open: openAction(ev))
                }
            }
        }
        .padding(.top, 8)
    }

    /// A blank lens shows only the category's NAME — the same empty-screen word LIFE's
    /// other surfaces use — plus the compose capsule when the page can create one.
    private var emptyWord: some View {
        VStack(spacing: 24) {
            EmptyWord("EVENTS", scale: .lane, placed: false)
            if let onCompose {
                Button(action: onCompose) {
                    Text("NEW EVENT")
                        .font(.system(size: 15, weight: .bold))
                        .tracking(0.6)
                        .foregroundStyle(Self.accentSoft)
                        .padding(.horizontal, 20)
                        .frame(minHeight: 44)
                        .overlay(Capsule().strokeBorder(Self.accentSoft.opacity(0.45), lineWidth: 1.5))
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("events-compose")
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 120)
    }

    // MARK: The projection — EventVM × places × location → what the lenses draw

    /// NEAR and MAP look forward (commitments still outstanding); DATES keeps the past.
    private var cards: [EventCardVM] {
        let window: EventsWindow = mode == .dates ? .all : .upcoming
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        return source.events(matching: terms, window: window)
            .map { vm(for: $0) }
            .filter { q.isEmpty || $0.matches(q) }
    }

    /// Chronological groups under day headings, in the order the events already have.
    private var dateGroups: [(label: String, items: [EventCardVM])] {
        var out: [(label: String, items: [EventCardVM])] = []
        for ev in cards {
            if let last = out.indices.last, out[last].label == ev.dayLabel {
                out[last].items.append(ev)
            } else {
                out.append((ev.dayLabel, [ev]))
            }
        }
        return out
    }

    /// The raw object id behind a VM id, or nil for a row with no GroupObject.
    private func objectId(_ ev: EventCardVM) -> String? {
        ev.id.hasPrefix("obj:") ? String(ev.id.dropFirst(4)) : nil
    }

    /// Going is the FOLD's answer — a signed reply exists, or it does not.
    private func isGoing(_ ev: EventCardVM) -> Bool {
        objectId(ev).map { core.goingEvents.contains($0) } ?? false
    }

    /// True when a real commitment is possible: somebody invited you, so there is a
    /// channel to answer on. False elsewhere, and the dot then arms a reminder only —
    /// rather than pretending to RSVP to a listing with no organiser behind it.
    private func canCommit(_ ev: EventCardVM) -> Bool {
        objectId(ev).map { core.canCommit(event: $0) } ?? false
    }

    /// One gesture, two honest meanings. Where there is someone to tell, this authors
    /// the reply. Where there is not, it arms the personal reminder and says nothing
    /// to anybody.
    private func mark(_ ev: EventCardVM) {
        if let oid = objectId(ev), core.canCommit(event: oid) {
            let going = !core.goingEvents.contains(oid)
            Task {
                await core.setGoing(event: oid, going: going)
                if going {
                    reminders.toggle(ev.id, title: ev.title,
                                     startMs: ev.startMs, venue: ev.venueName)
                }
            }
            return
        }
        reminders.toggle(ev.id, title: ev.title, startMs: ev.startMs, venue: ev.venueName)
    }

    /// The card/row tap. EVERY row opens something: a minted GroupObject ("obj:") opens
    /// the full event page, a world listing ("ra:" / "art:") opens its REFERENCE page,
    /// and everything else (graph / market / calendar rows) opens a reference page
    /// built from the row itself — it already carries title, when and venue, which is
    /// exactly what a reference page renders.
    ///
    /// Nothing falls through to the RSVP any more. A tap that sometimes reads and
    /// sometimes commits, depending on which feed the row came from, is not a tap
    /// anyone can learn.
    private func openAction(_ ev: EventCardVM) -> (() -> Void)? {
        if ev.id.hasPrefix("obj:") || ev.id.hasPrefix("ra:") || ev.id.hasPrefix("art:") {
            guard let onOpen else { return nil }
            return { onOpen(ev.id) }
        }
        guard let onOpenReference else { return nil }
        // The row already carries what a reference page renders — title, when, where.
        let ref = ReferenceListing(id: ev.id,
                                   title: ev.title,
                                   whenLabel: ev.whenLabel,
                                   venueName: ev.venueName,
                                   startMs: ev.startMs)
        return { onOpenReference(ref) }
    }

    private func vm(for e: EventVM) -> EventCardVM {
        let at = e.participants.first { $0.role == .at }
        let venue = at.flatMap { Self.venue(named: $0, in: places) }
        let distance = venue.flatMap { v in
            location.coordinate.map { here in
                CLLocation(latitude: here.latitude, longitude: here.longitude)
                    .distance(from: CLLocation(latitude: v.coordinate.latitude,
                                               longitude: v.coordinate.longitude)) / 1609.344
            }
        }
        // The kicker chip carries what the commitment already knows about itself — its
        // rhythm ("TUE & THU") or what it concerns — and is absent otherwise.
        let kicker = e.recurrenceLabel?.uppercased()
            ?? e.participants.first { $0.role == .about }.map { $0.name.uppercased() }
        return EventCardVM(id: e.id,
                           startMs: e.startMs,
                           title: e.title,
                           venue: venue,
                           venueName: venue?.name ?? at?.name,
                           whenLabel: Self.whenLabel(e),
                           dayLabel: Self.day.string(from: Self.date(e.startMs)).uppercased(),
                           // A multi-day row's right column is when the run CLOSES —
                           // its start is a day, so a wall clock would be an invention.
                           clockLabel: e.endMs.map { end in
                               Self.multiDay(e.startMs, end)
                                   ? "– " + Self.day.string(from: Self.date(end))
                                   : Self.clock.string(from: Self.date(e.startMs))
                           } ?? Self.clock.string(from: Self.date(e.startMs)),
                           kicker: kicker,
                           distanceMi: distance,
                           ticket: ticketing.byId[e.id],
                           flyerURL: e.flyerURL)
    }

    // MARK: Clock text

    private static func date(_ ms: Int64) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }

    private static let day: DateFormatter = make("EEE d MMM")
    private static let clock: DateFormatter = make("HH:mm")
    private static func make(_ format: String) -> DateFormatter {
        let f = DateFormatter()
        f.dateFormat = format
        return f
    }

    /// "Fri 7 Aug · 23:00 – 04:00" — the span's end rides along when there is one.
    /// A MULTI-DAY span (an exhibition's run) reads as dates, never clocks: its halves
    /// are days, and "00:00 – 00:00" would be an invented time.
    private static func whenLabel(_ e: EventVM) -> String {
        if let end = e.endMs, multiDay(e.startMs, end) {
            return day.string(from: date(e.startMs)) + " – " + day.string(from: date(end))
        }
        var s = day.string(from: date(e.startMs)) + " · " + clock.string(from: date(e.startMs))
        if let end = e.endMs { s += " – " + clock.string(from: date(end)) }
        return s
    }

    static func multiDay(_ startMs: Int64, _ endMs: Int64) -> Bool {
        !Calendar.current.isDate(date(startMs), inSameDayAs: date(endMs))
    }

    // MARK: Venue resolution — ONE place, shared with the fullscreen map's overlay

    /// The `.at` hop names the venue; a placed PLACE of the same identity (or, for
    /// resolver-lifted names, the same name) pins it.
    static func venue(named at: ParticipantVM, in places: [PlaceAnnotation]) -> PlaceAnnotation? {
        places.first { $0.id == at.id }
            ?? places.first { $0.name.caseInsensitiveCompare(at.name) == .orderedSame }
    }

    /// The located commitments as map pins — the fullscreen Map's EVENTS overlay, shown
    /// there while the EVENTS dimension is locked (see `PlacesMap.events`). One pin per
    /// commitment; an unplaced venue contributes none rather than a guess.
    static func venuePins(for events: [EventVM], places: [PlaceAnnotation]) -> [PlaceAnnotation] {
        events.compactMap { e in
            guard let at = e.participants.first(where: { $0.role == .at }),
                  let v = venue(named: at, in: places) else { return nil }
            return PlaceAnnotation(id: "event-\(e.id)", name: e.title,
                                   coordinate: v.coordinate,
                                   category: v.name, glyph: "calendar")
        }
    }
}

/// One commitment, projected for the page: venue resolved, clocks formatted, distance
/// measured. Pure value — the lenses only read it.
private struct EventCardVM: Identifiable {
    let id: String
    let startMs: Int64
    let title: String
    let venue: PlaceAnnotation?
    let venueName: String?
    let whenLabel: String
    let dayLabel: String
    let clockLabel: String
    let kicker: String?
    let distanceMi: Double?
    /// Present ⇒ the underlying GroupObject carries a ticket listing (drives the capsule).
    var ticket: TicketVM? = nil
    /// Present ⇒ a reference row's remote flyer fills the card's image slot ("ra:" rows).
    var flyerURL: URL? = nil

    var whereLabel: String? {
        guard let venueName else { return nil }
        return distanceLabel.map { venueName + " · " + $0 } ?? venueName
    }
    var distanceLabel: String? { distanceMi.map { String(format: "%.1f mi", $0) } }

    func matches(_ q: String) -> Bool {
        title.lowercased().contains(q)
            || (venueName?.lowercased().contains(q) ?? false)
            || (kicker?.lowercased().contains(q) ?? false)
    }
}

// MARK: - NEAR's flyer card

private struct NearCard: View {
    let event: EventCardVM
    let going: Bool
    /// True ⇒ the mark is a real commitment (there is an organiser to tell). False ⇒
    /// it arms a personal reminder only, and the row must not imply otherwise.
    var committable: Bool = false
    let toggle: () -> Void
    var buy: () -> Void = {}
    var door: () -> Void = {}
    /// Present ⇒ the card body opens the event's page. The capsule and door keep their
    /// own taps (buttons win over the container gesture), so acting ≠ navigating.
    var open: (() -> Void)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            flyerSlot
            VStack(alignment: .leading, spacing: 6) {
                Text(event.title)
                    .font(.system(size: 21, weight: .heavy))
                    .tracking(-0.3)
                    .foregroundStyle(Theme.textPrimary)
                    .lineSpacing(2)
                if let whereLabel = event.whereLabel {
                    Text(whereLabel)
                        .font(.system(size: 15))
                        .foregroundStyle(Theme.textSecondary)
                }
                Text(event.whenLabel)
                    .font(.system(size: 15))
                    .foregroundStyle(Theme.textSecondary)
                HStack(spacing: 12) {
                    if let ticket = event.ticket {
                        ticketButton(ticket)
                    } else {
                        rsvpButton
                    }
                    Spacer(minLength: 0)
                    if event.ticket?.owned == true {
                        doorButton
                    }
                }
                .padding(.top, 10)
            }
            .padding(EdgeInsets(top: 16, leading: 18, bottom: 18, trailing: 18))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .contentShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
        .onTapGesture { open?() }
    }

    /// The flyer's slot — the row's remote flyer when it carries one (an RA listing),
    /// striped otherwise (and while the flyer loads or fails: stripes stay the honest
    /// fallback). The kicker chip rides its top corner; a listed event's price chip
    /// rides the opposite one.
    private var flyerSlot: some View {
        // The image rides an OVERLAY on a fixed-size ground: a filled image that IS the
        // layout reports its own width, and one wide flyer would push the card sideways.
        Color.clear
            .frame(height: 150)
            .frame(maxWidth: .infinity)
            .overlay {
                if let url = event.flyerURL {
                    PosterImage(url: url) { FlyerStripes() }
                } else {
                    FlyerStripes()
                }
            }
            .clipped()
            .overlay(alignment: .topTrailing) {
                if let kicker = event.kicker {
                    chip(kicker).padding(14)
                }
            }
            .overlay(alignment: .topLeading) {
                if let ticket = event.ticket {
                    chip(ticket.priceLabel).padding(14)
                }
            }
            .clipShape(UnevenRoundedRectangle(topLeadingRadius: 22, topTrailingRadius: 22))
    }

    private func chip(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 11, weight: .bold))
            .tracking(1.1)
            .foregroundStyle(EventsPage.accentSoft)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Color.black.opacity(0.55), in: Capsule())
    }

    private var rsvpButton: some View {
        Button(action: toggle) {
            Text(going ? "GOING ✓" : "RSVP")
                .font(.system(size: 15, weight: .bold))
                .tracking(0.6)
                .foregroundStyle(going ? .white : EventsPage.accentSoft)
                .padding(.horizontal, 20)
                .frame(minHeight: 44)
                .background(Capsule().fill(going ? EventsPage.accent : .clear))
                .overlay(Capsule().strokeBorder(
                    going ? EventsPage.accent : EventsPage.accentSoft.opacity(0.45),
                    lineWidth: 1.5))
        }
        .buttonStyle(.plain)
        .animation(.easeOut(duration: 0.2), value: going)
        .accessibilityLabel(going ? "Going — tap to withdraw" : "RSVP")
    }

    /// The ticket capsule — same geometry as RSVP, its words from the listing's state.
    private func ticketButton(_ t: TicketVM) -> some View {
        TicketCapsule(ticket: t, buy: buy)
    }

    /// The owner's door — scan wallets at the entrance.
    private var doorButton: some View {
        Button(action: door) {
            Image(systemName: "qrcode.viewfinder")
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(EventsPage.accentSoft)
                .frame(width: 44, height: 44)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Door")
        .accessibilityIdentifier("event-door")
    }
}

/// The one ticket capsule, shared by the NEAR card and the event page — the words come
/// from the listing's state. Held fills like GOING ✓; the rest stay outlined. Disabled
/// while a checkout is in flight (9A), when sold out, and on your own event (the capsule
/// is a value there).
struct TicketCapsule: View {
    let ticket: TicketVM
    var identifier = "event-ticket-capsule"
    var buy: () -> Void = {}

    var body: some View {
        Button(action: buy) {
            Text(label)
                .font(.system(size: 15, weight: .bold))
                .tracking(0.6)
                .foregroundStyle(ticket.held ? .white : EventsPage.accentSoft)
                .padding(.horizontal, 20)
                .frame(minHeight: 44)
                .background(Capsule().fill(ticket.held ? EventsPage.accent : .clear))
                .overlay(Capsule().strokeBorder(
                    ticket.held ? EventsPage.accent : EventsPage.accentSoft.opacity(0.45),
                    lineWidth: 1.5))
                .opacity(ticket.pending ? 0.45 : 1)
        }
        .buttonStyle(.plain)
        .disabled(ticket.pending || ticket.soldOut || ticket.held || !ticket.buyable)
        .accessibilityIdentifier(identifier)
        .accessibilityLabel(label)
    }

    private var label: String {
        if ticket.held { return "TICKET ✓" }
        if ticket.soldOut { return "SOLD OUT" }
        if ticket.priceCents == 0 { return "FREE" }
        return "GET TICKET · \(ticket.priceLabel)"
    }
}

// MARK: - DATES' row

private struct DateRow: View {
    let event: EventCardVM
    let going: Bool
    /// True ⇒ the mark is a real commitment (there is an organiser to tell). False ⇒
    /// it arms a personal reminder only, and the row must not imply otherwise.
    var committable: Bool = false
    let toggle: () -> Void
    /// Opens the event's detail. Every row has one — see `EventsPage.openAction`.
    var open: (() -> Void)? = nil

    var body: some View {
        // Tapping a row OPENS it. It used to fall back to `toggle` whenever there was
        // no page to open, which made the whole row an "I'm attending" button on
        // exactly the rows you were most likely to be browsing — you looked at an
        // event and were silently marked as going. Going is now its own control (the
        // dot), so reading and committing can never be the same gesture.
        Button(action: { open?() }) {
            HStack(spacing: 16) {
                goingDot
                    .accessibilityIdentifier("event-going-\(event.id.prefix(12))")
                VStack(alignment: .leading, spacing: 2) {
                    Text(event.title)
                        .font(.system(size: 17, weight: .bold))
                        .foregroundStyle(Theme.textPrimary)
                    if let venueName = event.venueName {
                        Text(venueName)
                            .font(.system(size: 14))
                            .foregroundStyle(Theme.textSecondary)
                    }
                }
                Spacer(minLength: 12)
                VStack(alignment: .trailing, spacing: 2) {
                    Text(event.clockLabel)
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(Theme.textPrimary)
                    if event.ticket?.held == true {
                        Text("ticket")
                            .font(.system(size: 12))
                            .foregroundStyle(EventsPage.accentSoft)
                    } else if going {
                        Text("going")
                            .font(.system(size: 12))
                            .foregroundStyle(EventsPage.accentSoft)
                    }
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 14)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .overlay(alignment: .top) {
            Rectangle().fill(Theme.hairline.opacity(0.6)).frame(height: 0.5).padding(.horizontal, 20)
        }
    }

    /// The going-mark, and the ONLY thing on a row that sets it. An 8pt dot inside a
    /// 44pt target: the mark is small because it is information, the target is large
    /// because it is a commitment. Reading a row and committing to it must never be
    /// the same gesture.
    private var goingDot: some View {
        Button(action: toggle) {
            Circle()
                .fill(going ? EventsPage.accent : Theme.hairline)
                .frame(width: 8, height: 8)
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(going ? "Going — tap to withdraw" : "Mark as going")
    }
}

// MARK: - Previews

#Preview("Events page — NEAR (seeded)") {
    EventsPage(source: SeedEventsSource(), terms: [], places: PlaceAnnotation.demo)
        .background(Theme.bg.ignoresSafeArea())
        .environmentObject(LocationService())
        .preferredColorScheme(.dark)
}

#Preview("Events page — empty") {
    EventsPage(source: SeedEventsSource(all: []), terms: [], places: [])
        .background(Theme.bg.ignoresSafeArea())
        .environmentObject(LocationService())
        .preferredColorScheme(.dark)
}
