//  ProfileStack.swift
//  The universal Profile page: headline → map → bio/info → JEWELS → pins,
//  driven entirely through a `ProfileSource`.
//
//  TWO KINDS, ONE STACK: a person and a Space — the EGOIC objects. Anything that
//  merely happened or was posted goes to `Design/Kind/` instead.
//
//  The jewels row moved above the pins (15 Aug): it FILTERS the content below
//  it, and a filter drawn after its content reads as a footer.
//
//  The stack owns the collapse state and the ONE spring
//  (.spring(response: 0.45, dampingFraction: 0.85)); every card beneath is a
//  pure function of that state. Tap the headline to collapse/expand — the
//  scroll-driven collapse (HideOnScroll/CoverScroll offset) is T2/T3's wiring.
//
//  The jewels row REUSES the rail's atoms rather than re-drawing them:
//  `JewelFace` (RibbonRail.swift) is the face, `LifeQuery` holds the lock
//  state, and the lock semantics are the feed's own — `FeedAssembler.filter`
//  decides what a locked set lets through (lock = kind filter, S27-S34).
//
//  ACTION FORWARDING (M1). `subject` names the id the person actions act on, and
//  turns the headline's circles on. Pass nil — a Space, or a person page that has
//  no id to act for — and the circles render inert and no status value is drawn,
//  which is the honest answer rather than a placebo. This is what
//  PersonProfilePage's ON RECORD deviation asked for: it composed these cards by
//  hand because the seam was not forwarded, and is now a bare host again.
//
//  NO HOST FOOTER. There was one for a day, so the event page could hang its
//  photo strip and ticket block under the pins. Events are not profiles, and a
//  component with an escape hatch is a component that will be asked to render
//  everything — the hatch went out with the page that needed it.

import SwiftUI

/// The profile jewels, in spec order: People · Spaces · Channels · Events ·
/// Listings · Posts (Spaces ≡ groups; Listings ≡ trade — the Profile spec's
/// vocabulary over the rail's categories).
///
/// CHANNELS was split from POSTS: a Space's channel list and its post feed were
/// sharing one lane, and they are not the same thing.
///
/// The same six on a PERSON: `#` there filters to the channels and forums THAT
/// person is in and you can see — the intersection, not their whole membership.
/// A person's DM is not in this row at all; it is the gold circle on the
/// headline, because it is an action rather than a filter.
///
/// A namespace rather than a static on `ProfileStack`: the stack is generic over
/// its footer now, and Swift has no static stored properties in a generic type.
/// No loss — the jewel ORDER was never a property of what a page hangs below its
/// pins, and the second reader of it (GroupDetailView) never wanted the stack.
enum ProfileJewels {
    static let order: [EventCategory] = [.people, .groups, .channels, .events, .trade, .posts]
}

struct ProfileStack: View {
    let source: any ProfileSource
    /// The subject of the person actions. nil = no actions (Space pages, and any
    /// host that cannot name an id) — the circles stay inert.
    let subject: String?
    /// Circles the HOST puts on the headline, over and above the person trio the
    /// stack builds for itself. An event's DISCUSS and INVITE are the event page's
    /// business, not the component's: the stack owns the corners, the host owns
    /// what they do.
    let actions: [ProfileAction]

    /// The jewels' lock state — the rail's own query object, scoped to this page.
    @StateObject private var query = LifeQuery()
    @State private var collapsed: Bool
    @State private var headline: ProfileHeadline?
    @State private var info: ProfileInfo?
    @State private var cells: [ProfileGeoCell] = []
    @State private var pins: [ProfilePin] = []
    @State private var status: ProfilePeerStatus?
    @State private var reportURL: URL?

    @Environment(\.openURL) private var openURL

    init(source: any ProfileSource,
         subject: String? = nil,
         actions: [ProfileAction] = [],
         initiallyCollapsed: Bool = false) {
        self.source = source
        self.subject = subject
        self.actions = actions
        _collapsed = State(initialValue: initiallyCollapsed)
    }

    var body: some View {
        GeometryReader { geo in
            let width = max(0, geo.size.width - 2 * DesignTokens.Life.gutter)
            ScrollView {
                // SPACING IS PER-ELEMENT, not per-stack: the bio band butts
                // straight onto the wall with no seam, and everything else keeps
                // the card gap by asking for it.
                VStack(spacing: 0) {
                    if let headline {
                        // The banner is a WALL: the full screen's width, flush to
                        // the top, no gutter and no corners. Every card below it
                        // keeps the gutter.
                        HeadlineCard(headline: headline, collapsed: collapsed,
                                     width: geo.size.width,
                                     actions: circles(for: headline))
                            .contentShape(Rectangle())
                            .onTapGesture {
                                withAnimation(ProfileGeometry.spring) { collapsed.toggle() }
                            }
                        // The pair state as a bare VALUE (terse UI) — drawn only
                        // where the binder can answer, beside the circle it explains.
                        if let status {
                            Text(status.rawValue.uppercased())
                                .font(.system(size: 10, weight: .semibold, design: .monospaced))
                                .kerning(1.2)
                                .foregroundStyle(Theme.textSecondary)
                                .frame(width: width, alignment: .trailing)
                                .padding(.top, DesignTokens.Profile.cardSpacing)
                                .accessibilityIdentifier("person-status")
                        }
                        if !cells.isEmpty {
                            MapCard(cells: cells, color: headline.color, width: width)
                                .padding(.top, DesignTokens.Profile.cardSpacing)
                        }
                        if let info {
                            // A person gets the BIO card (prose on the profile's own
                            // tone); a Space and an EVENT get the info card, which is
                            // the one that can state facts beside the words.
                            if headline.kind == .person {
                                BioCard(text: info.desc)
                            } else {
                                InfoCard(info: info)
                            }
                        }
                        /* The rail sits DIRECTLY UNDER the info card, because it
                           filters what follows it. Below the pins it read as a
                           footer — a control placed after the thing it controls
                           cannot say what it does. Selecting a jewel filters the
                           content beneath to that kind. */
                        // FLUSH to the card above it: the rail is a row of squares,
                        // and the strip it stands in has to measure exactly one of
                        // them — a gap here would make that strip taller than a slot.
                        jewelsRow(width: geo.size.width)
                        ForEach(visiblePins) { pin in
                            pinBar(pin, fallback: headline.color, width: width)
                                .padding(.top, DesignTokens.Profile.cardSpacing)
                        }
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.bottom, DesignTokens.Profile.verticalPad)
            }
            .background(Theme.bg)
        }
        .task {
            headline = await source.headline()
            info = await source.info()
            cells = await source.geoCells()
            pins = await source.pins()
            // Only where a subject is named. A binder that cannot answer returns
            // nil by protocol default, so this stays honest for Spaces too.
            if let subject {
                status = await source.peerStatus(id: subject)
                reportURL = await source.reportSurface(id: subject)
            }
        }
        .accessibilityIdentifier("profile-stack")
    }

    // MARK: The circles — the stack's person trio, plus the host's

    /// The person trio is the STACK's because it is driven by the stack's own
    /// state (the pair status it fetched); everything else is the host's.
    /// A person headline can therefore never collide with a host action, because
    /// no host that renders a person page passes any.
    private func circles(for headline: ProfileHeadline) -> [ProfileAction] {
        guard headline.kind == .person else { return actions }
        return [
            ProfileAction(slot: .bottomTrailing, symbol: "plus", run: connectAction),
            ProfileAction(slot: .topTrailing, symbol: "exclamationmark", run: issueAction),
            ProfileAction(slot: .bottomLeading, symbol: "bubble.left.fill",
                          tint: DesignTokens.Profile.dmCircle, run: messageAction),
        ] + actions
    }

    // MARK: The person circles — real actions or none

    /// Connect/Disconnect. Enabled only where the pair flow can TRULY advance
    /// from a page: an inbound `pending` awaits our accept; a known `contact`
    /// can be asked for. `connected` is inert until core wires Disconnect
    /// (Revoked); nil status (the binder can't answer) is inert too.
    private var connectAction: (() -> Void)? {
        guard let subject, status == .pending || status == .contact else { return nil }
        return {
            Task { @MainActor in
                if let after = await source.requestConnect(id: subject) { status = after }
            }
        }
    }

    /// The DM. Opens through the same `.pacificOpenDM` seam the chats list and
    /// the deep link use — one route to a conversation, never a second.
    /// A Space has no DM, and a page with no subject has nobody to message.
    private var messageAction: (() -> Void)? {
        guard let subject, headline?.kind == .person else { return nil }
        return {
            NotificationCenter.default.post(name: .pacificOpenDM, object: nil,
                                            userInfo: ["peer": subject])
        }
    }

    /// Issue/Report — a door to the governing-Arc surface, never a local action
    /// (a block hides a person, it does not rewrite history; BlockStore is not
    /// this circle's business).
    private var issueAction: (() -> Void)? {
        guard let reportURL else { return nil }
        return { openURL(reportURL) }
    }

    // MARK: Pins — the lock filter, by the feed's own rule

    /// Which pins the locked jewels let through. The decision is
    /// `FeedAssembler.filter`'s, verbatim — the pins are projected into the
    /// feed's row type, filtered by the one lock semantic, and mapped back — so
    /// the profile and the feed can never disagree about what a lock means.
    private var visiblePins: [ProfilePin] {
        let rows = pins.map {
            FeedItem(id: $0.id, category: $0.kind.category, title: $0.title,
                     author: "", location: "", footer: "", whenMs: 0,
                     provenance: .member, objectID: $0.destinationID)
        }
        let kept = Set(FeedAssembler.filter(rows, locked: query.locked).map(\.id))
        return pins.filter { kept.contains($0.id) }
    }

    /// A pin with a routable destination navigates by VALUE — the host's stack
    /// registers the destinations (Life.swift). Others render inert.
    @ViewBuilder
    private func pinBar(_ pin: ProfilePin, fallback: Color, width: CGFloat) -> some View {
        let bar = PinBar(pin: pin, fallback: fallback, width: width)
        if let id = pin.destinationID {
            switch pin.kind {
            case .person:       NavigationLink(value: PersonLink(id: id)) { bar }.buttonStyle(.plain)
            case .place:
                Button {
                    NotificationCenter.default.post(name: .pacificOpenPlace, object: nil, userInfo: ["object": id])
                } label: { bar }
                .buttonStyle(.plain)
            case .space:        NavigationLink(value: GroupLink(id: id)) { bar }.buttonStyle(.plain)
            // Event/listing/post pages route through sheet/feed surfaces the
            // stack doesn't own yet — inert until their tracks wire them.
            case .event, .listing, .post: bar
            }
        } else {
            bar
        }
    }

    // MARK: Jewels — the shared rail, the feed's lock

    private func jewelsRow(width: CGFloat) -> some View {
        ProfileJewelRail(isActive: { query.isLocked($0) },
                         onTap: { cat in
                             withAnimation(.spring(response: 0.36, dampingFraction: 0.78)) {
                                 query.toggleLock(cat)
                             }
                         },
                         idPrefix: "profile-jewel",
                         width: width)
    }
}

/// ProfilePinKind in the rail's vocabulary — one mapping, beside its one user.
extension ProfilePinKind {
    var category: EventCategory {
        switch self {
        case .person:  .people
        case .space:   .groups
        case .place:   .places
        case .event:   .events
        case .listing: .trade
        case .post:    .posts
        }
    }
}

// MARK: - Previews (fixture-driven; the four contract states)

#Preview("Person — expanded") {
    NavigationStack {
        ProfileStack(source: FixtureProfileSource.dare)
    }
    .preferredColorScheme(.dark)
}

#Preview("Person — collapsed") {
    NavigationStack {
        ProfileStack(source: FixtureProfileSource.dare, initiallyCollapsed: true)
    }
    .preferredColorScheme(.dark)
}

#Preview("Space — map card (Jimmy's)") {
    NavigationStack {
        ProfileStack(source: FixtureProfileSource.jimmys)
    }
    .preferredColorScheme(.dark)
}

#Preview("Space — live (NTS)") {
    NavigationStack {
        ProfileStack(source: FixtureProfileSource.nts)
    }
    .preferredColorScheme(.dark)
}
