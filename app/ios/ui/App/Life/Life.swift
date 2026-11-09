//  Life.swift
//  The LIFE screen — replaces the old WORK tab and absorbs the Notes OUTLINE into
//  one filtered surface over your world: PEOPLE · PLACES · GROUPS · PROJECTS ·
//  EVENTS. Four of those are entity lists over your (deliberately rough, endlessly
//  refinable) entity graph; PROJECTS is the live pacific-core project GroupObjects —
//  the same RealProjectCard the retired Work tab drew. Long-press any entity row to
//  refine it: that refine/enrich loop is how Pacific learns your world.
//
//  The old Outline's "All" (nested tree) and "Actions" filters are dropped — LIFE is
//  a flat, per-type surface. GROUPS is the org kind, relabelled.
//
//  DATA SEAM: everything this screen
//  reads flows through `LifeSource`. TODAY that is `OutlineLifeSource`, backed by the
//  in-session OutlineModel (entities) + CoreService (project GroupObjects), so LIFE
//  compiles and renders now. SWAP POINT (WS-E/WS-F): drop in a PacificStore.LocalStore-
//  backed adapter that reads the union store directly — listDocuments(filter:{type})
//  for entities, the project GroupObjects for projects(), edge enumeration for
//  related(to:) — projecting union rows into these same EntityVM / ObjectRow shapes,
//  so the views below don't change. NO silent fallbacks: an empty list is a genuinely
//  empty world, never a swallowed error.

import SwiftUI
import UIKit            // haptics for the selector's Lock / Hold gestures
import PacificCore
import ArcResolverKit   // ArcEntity — the resolved GroupObjects the LIFE tab now reads

// MARK: - The LIFE read seam (§7)

/// The projection the LIFE screen reads. App-native shapes (EntityVM for the entity
/// graph, ObjectRow for project GroupObjects) so the reused Outline / EntityDetail
/// views and RealProjectCard render unchanged. The concrete backing swaps from
/// `OutlineLifeSource` (today) to a LocalStore adapter (WS-E/WS-F) without touching UI.
@MainActor
protocol LifeSource {
    /// Entities of one kind — the PEOPLE / PLACES / GROUPS / EVENTS filters. Excludes
    /// Actions (LIFE has no Actions filter; actions are to-do verbs, not entities).
    func entities(ofKind kind: EntityKind) -> [EntityVM]
    /// The live project GroupObjects — the PROJECTS filter.
    func projects() -> [ObjectRow]
    /// Entities related to `id` — the relatedness primitive (edge enumeration in the
    /// real impl). Part of the frozen §7 contract; EntityDetail reads relatedness from
    /// the model directly today and routes through here once the store swap lands.
    func related(to id: String) -> [EntityVM]
}

/// Today's `LifeSource`: the in-session entity graph (OutlineModel) + the live project
/// GroupObjects (CoreService). This is the ONE place that knows the current backing —
/// replacing it with a LocalStore adapter is the whole swap.
@MainActor
struct OutlineLifeSource: LifeSource {
    let core: CoreService
    private var model: OutlineModel { .shared }

    func entities(ofKind kind: EntityKind) -> [EntityVM] {
        // PLACES come from the core, not the outline. A Place is a GroupObject
        // (ObjectKind::Place) carrying the location facet — durable, signed, shareable —
        // exactly as PROJECTS are. An OutlineModel row is device-local and could never be
        // sent to anyone, so it is not a legitimate home for one.
        if kind == .place {
            return core.places
                .map(\.entityVM)
                .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        }
        return model.byID.values
            .filter { $0.kind == kind && !$0.isAction }
            .sorted { $0.name < $1.name }
    }

    func projects() -> [ObjectRow] { core.projectObjects }

    func related(to id: String) -> [EntityVM] { model.inbound(to: id) }
}

/// The LIFE source the objective delivers: entities RESOLVED from notes + chats by the
/// on-device arc-resolver (via `GraphService.cachedEntities`), mapped into the same EntityVM
/// shape so the rows/detail views render unchanged. PROJECTS still come from live core
/// GroupObjects. Entity-graph relatedness (resolver neighbors) folds in next.
@MainActor
struct GraphLifeSource: LifeSource {
    let core: CoreService
    let resolved: [ArcEntity]
    /// The real 1:1 connections. Fetched OUTSIDE the render path (`connectionRows()` is an
    /// FFI call that sets `core.error` on failure, which must not happen mid view-update).
    var connections: [ConnectionRow] = []

    func entities(ofKind kind: EntityKind) -> [EntityVM] {
        // PLACES come from the GRAPH — every minted Place GroupObject is projected there
        // (entity type "Place", id == the object id, stamped source:"object"), so the
        // graph is the complete set. `core.places` is a second list that can lag a sync,
        // and reading only it is why Places could go missing from the map.
        //
        // Resolver GHOSTS are still excluded (`isObject`): a "Place" lifted out of a note
        // is a proposal about somewhere, not somewhere you went, and carries no location.
        //
        // The join is for COORDINATES only: the projection records identity, not lat/lng,
        // so a placed Place takes its point from the matching PlaceRow. A Place with no
        // fix yet is listed with `location == nil` — honest, and the map declines to pin
        // what it cannot place rather than inventing a point.
        if kind == .place {
            let rows = Dictionary(core.places.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
            var byID: [String: EntityVM] = [:]
            for e in resolved where e.entityType == "Place" && e.isObject {
                var vm = rows[e.id]?.entityVM
                    ?? EntityVM(id: e.id, name: e.name.isEmpty ? e.label : e.name, kind: .place)
                if vm.descriptor == nil { vm.descriptor = e.summary }
                byID[e.id] = vm
            }
            // Anything minted but not yet projected (the projection runs on the next
            // objects change) still shows — the tab must never be emptier than the truth.
            for row in core.places where byID[row.id] == nil {
                byID[row.id] = row.entityVM
            }
            return byID.values
                .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        }

        var out = resolved
            .compactMap(GraphLifeSource.vm(from:))
            .filter { $0.kind == kind }

        // PEOPLE lists every real connection — the people you are actually connected to —
        // ahead of names the resolver merely lifted out of notes/chats. A resolved name
        // that matches a connection is dropped so nobody appears twice.
        if kind == .person {
            let people = connections.map(GraphLifeSource.vm(from:))
            let known = Set(people.map { $0.name.lowercased() })
            out = people + out.filter { !known.contains($0.name.lowercased()) }
        }

        return out.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    func projects() -> [ObjectRow] { core.projectObjects }

    func related(to id: String) -> [EntityVM] { [] }

    /// Resolver SEVEN_TYPES (+ Group) → the four LIFE entity kinds. Project is the PROJECTS
    /// filter (live GroupObjects, not here); Plant/Animal/Thing have no filter yet → dropped.
    static func kind(for type: String) -> EntityKind? {
        switch type {
        case "Person":         return .person
        case "Place":          return .place
        case "Group":          return .org
        case "Event":          return .event
        default:               return nil
        }
    }

    /// A real 1:1 connection as a PEOPLE row. `offPlatform = false` marks it as someone
    /// actually on Pacific, unlike a name the resolver lifted out of text. An unnamed peer
    /// falls back to its id stub — the hex is an address, never shown as a whole.
    static func vm(from c: ConnectionRow) -> EntityVM {
        let name = c.displayName.isEmpty ? String(c.peerId.prefix(12)) + "…" : c.displayName
        var vm = EntityVM(id: c.peerId, name: name, kind: .person)
        vm.descriptor = c.connected ? nil : "Pending"   // only say something when it matters
        vm.offPlatform = false
        vm.tint = LifeFilter.people.color
        return vm
    }

    static func vm(from e: ArcEntity) -> EntityVM? {
        guard let kind = kind(for: e.entityType) else { return nil }
        let name = e.name.isEmpty ? e.label : e.name
        var vm = EntityVM(id: e.id, name: name, kind: kind)
        vm.descriptor = e.summary
        // A node projected from a minted GroupObject is durable truth (id = the object
        // id); only names lifted out of text render as off-platform ghosts.
        vm.offPlatform = !e.isObject
        vm.tint = LifeFilter.allCases.first(where: { $0.kind == kind })?.color ?? .gray
        return vm
    }
}

// MARK: - The filter

/// The five LIFE filters, in bar order. GROUPS is the org kind, relabelled; PROJECTS
/// is GroupObjects, not entity-graph entities (hence `kind == nil`).
enum LifeFilter: String, CaseIterable {
    case events   = "EVENTS"
    case people   = "PEOPLE"
    case places   = "PLACES"
    case groups   = "GROUPS"
    case projects = "PROJECTS"
    // OVERFLOW — more categories than fit one row. They live in the vertical carousel
    // that opens under the rightmost selector in jiggle mode.
    case things   = "THINGS"
    case plants   = "PLANTS"
    case animals  = "ANIMALS"
    /// A published thing with its own spine — `ObjectKind::Post` (30). Its own case
    /// since 18 Aug: POSTS used to collapse into `.media`, the photo-library import,
    /// which is why its `+` was hidden. `.media` mints nothing, and before the Post
    /// primitive existed there was nothing for it to mint.
    case posts    = "POSTS"
    // MEDIA is photos + videos. It is not a leaf category like the others: a capture
    // carries metadata that binds it to a PLACE and accrues to a subject, and once
    // enough contributors have shot the same subject it reconstructs into a THING with
    // geometry. So MEDIA is the raw material the graph is built FROM. See Media.swift.
    case media    = "MEDIA"

    /// The rail speaks EventCategory; LIFE speaks LifeFilter. One mapping, in one
    /// place, so the two can never drift apart again.
    init(_ category: EventCategory) {
        switch category {
        case .events:   self = .events
        case .people:   self = .people
        case .groups:   self = .groups
        case .places:   self = .places
        case .projects: self = .projects
        case .things:   self = .things
        // The rail's market jewels wear the nearest existing filter slots — colour
        // and mint routing only; their grounds are the Feed config pages.
        case .trade:    self = .things
        case .job:      self = .projects
        // Profile-only jewel (never in the rail order). MEDIA is the nearest slot
        // that honestly mints nothing and lists no entity kind.
        // POSTS is its OWN filter now that Post is a real kind (30). It used to collapse
        // into `.media` — the photo-library import — which is why the jewel's `+` was
        // hidden: `.media` mints nothing, and a Post had no object to mint.
        case .posts:    self = .posts
        case .channels: self = .media
        }
    }

    /// The categories the row itself carries, in default order (PROJECTS anchored last).
    static let primary: [LifeFilter] = [.events, .people, .places, .groups, .projects]
    /// The rest — the jiggle-mode carousel.
    static let overflow: [LifeFilter] = [.things, .plants, .animals, .media]

    /// The entity kind this filter lists, or nil where there is none.
    var kind: EntityKind? {
        switch self {
        case .people:   return .person
        case .places:   return .place
        case .groups:   return .org
        case .events:   return .event
        // POSTS is GroupObjects too — a Post(30) is its own spine, never a resolver row.
        case .posts:    return nil
        // PROJECTS is GroupObjects; Thing/Plant/Animal have no EntityKind at all (the
        // resolver drops them), so they list nothing rather than pretending otherwise.
        case .projects, .things, .plants, .animals: return nil
        // MEDIA lists MediaAssets, not resolved entities — it has no EntityKind.
        case .media: return nil
        }
    }

    /// This category's stable colour slot in the active palette — by category, not enum
    /// order, so LIFE and the mint menu agree on which colour a category wears.
    var colorIndex: Int {
        switch self {
        case .events:   return 0
        case .people:   return 1
        case .places:   return 2
        case .groups:   return 3
        case .projects: return 4
        case .posts:    return 6
        case .things:   return 5
        case .plants:   return 6
        case .animals:  return 7
        case .media:    return 8
        }
    }

    /// The category's PURE palette colour — the flat fill for the LIFE selector. No
    /// gradient, no sheen: the raw Van Gogh hex for this category's slot. The palette is
    /// a deliberate FIVE, so the overflow categories take lightened variants of the first
    /// three rather than diluting it with invented hues.
    var color: Color {
        colorIndex < 5 ? PaletteStore.base(colorIndex)
                       : PaletteStore.base(colorIndex - 5).lightened(0.34)
    }

    /// The section's icon — one glyph per thing-type (matches the mint menu).
    var glyph: String {
        switch self {
        case .people:   return "person.fill"
        case .places:   return "mappin.circle.fill"
        case .groups:   return "person.3.fill"
        case .projects: return "square.grid.2x2.fill"
        case .events:   return "calendar"
        case .things:   return "shippingbox.fill"
        case .posts:    return "text.bubble.fill"
        case .plants:   return "leaf.fill"
        case .animals:  return "pawprint.fill"
        case .media:    return "photo.stack.fill"
        }
    }
}

// MARK: - The screen

struct LifeView: View {
    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var graph: GraphService   // the resolver-backed KG (LIFE reads it)
    @EnvironmentObject private var shell: ShellState     // to freeze the page-swipe while arranging
    @EnvironmentObject private var google: GoogleService // Google Calendar → the EVENTS ribbon (3rd provenance)
    @EnvironmentObject private var music: MusicService   // the Music System (RA) → reference rows in EVENTS
    @EnvironmentObject private var art: ArtService       // the Art System (ArtRabbit) → reference rows in EVENTS
    @ObservedObject private var model = OutlineModel.shared

    /// DERIVED from the rail's selection — never stored: a second copy drifted from the
    /// rail. On conflict, re-derivation wins.
    private var filter: LifeFilter { LifeFilter(lifeQuery.selected) }
    /// Heterogeneous value-based routes: a String pushes a Project (the
    /// `pacificOpenProject` deep link), a `GroupLink` pushes a group's page, an
    /// `EntityVM` an entity's. The DISCOVER face's stack — typed destinations.
    @State private var path = NavigationPath()
    @State private var sheet: OutlineSheet?
    /// The Active Scroll Feed — LIFE's ground state when every jewel is minimised
    /// (FEED-TRANSMISSION 0-ANSWER-C), and the Modes Config it saves/loads.
    @StateObject private var feedStore = FeedStore()
    @StateObject private var modeStore = ModeStore()
    @State private var modesSheet = false
    /// The place whose POV is open — reached by tapping a pin. The POV is a property of the
    /// place (any location/POV-enabled GroupObject).
    @State private var openedPlace: PlaceAnnotation?
    @State private var scanningPlace = false

    /// Scan a public place's poster. Lands you in the DM with the host — membership
    /// arrives when their device admits you, which is not instant, so this deliberately
    /// does not pretend to open the place.
    private var joinPlaceButton: some View {
        Button { scanningPlace = true } label: {
            Image(systemName: "qrcode.viewfinder")
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 52, height: 52)
                .background(LifeFilter.places.color, in: Circle())
                .shadow(color: .black.opacity(0.35), radius: 10, y: 4)
        }
        .buttonStyle(.plain)
        .padding(.trailing, 20)
        .padding(.bottom, 120)   // clear of the selector rail
        .accessibilityLabel("Scan a place code")
    }
    /// The real 1:1 connections, refreshed on entry and on every sync — PEOPLE lists them.
    @State private var connections: [ConnectionRow] = []
    /// Selector jiggle/re-order mode. Owned here (not in the bar) so a tap anywhere
    /// outside the selector row can leave it.
    @State private var jiggle = false

    // The two-faced screen (market face + axial spin) was removed 10 Aug — LIFE is
    // one page; TRADE/JOB are rail jewels.

    /// The multimodal filter: which ribbon is expanded, which dimensions are locked, and
    /// which entity each locked dimension is pinned to.
    /// LIFE's search text. Lives here rather than in `LifeQuery`: that object is the
    /// jewels' LOCK state (a kind filter), and a text query is a different question
    /// asked of the same feed — conflating them would make clearing one clear the other.
    @State private var lifeSearch = ""
    /// The top bar's search is open — the only time the jewel rail is up (9 Nov).
    @State private var searchOpen = false

    @StateObject private var lifeQuery = LifeQuery()
    /// Commitments + participants read out of the resolver graph. Rebuilt off-main whenever
    /// `graph.epoch` bumps (i.e. after every projection that changed the graph), so the rail
    /// re-lists live.
    @State private var eventsSnapshot = EventsSnapshot()

    /// EVENTS = what the resolver OBSERVED + your own dated market commitments + your own
    /// MINTED events, in one chronological feed. A posture with a deadline is Event-shaped by
    /// design, so it belongs in the feed rather than in a parallel surface of its own; a
    /// minted Event GroupObject is the feed's durable row and WINS over a graph row sharing
    /// its (title, day) — see GroupObjectEventsSource.
    private var eventsSource: any EventsSource {
        MergedEventsSource(graph: GraphEventsSource(snapshot: eventsSnapshot),
                           market: MarketCommitments.events(from: core.things,
                                                            me: core.displayName),
                           calendar: CalendarEventsSource.from(google.events, tint: EventsSeed.tint).events,
                           groupObjects: GroupObjectEventsSource.events(from: core.events,
                                                                        me: core.displayName),
                           music: MusicEventsSource.events(from: music.events),
                           art: ArtEventsSource.events(from: art.events))
    }

    /// The event whose door is open, and the wallet's held listings — the ticket
    /// capsule's inputs (see `eventTicketing`). Composing an event is a `New` session
    /// now (Design/New.swift), so it needs no flag here.
    @State private var doorEvent: DoorLink?
    /// The event whose PAGE is open — a card/row tap navigates; acting stays on the capsule.
    /// The world listing whose REFERENCE page is open ("ra:" / "art:" rows — values +
    /// the source's own capsule), already projected for the one reference skin.
    @State private var heldListings: Set<String> = []
    @StateObject private var pendingTickets = PendingTicketStore.shared

    /// The capsule contract for every LISTED minted event, keyed by VM id ("obj:<id>").
    /// State in, intents out — EventsPage stays a skin.
    private var eventTicketing: EventTicketing {
        var byId: [String: TicketVM] = [:]
        for row in core.events where row.listed {
            let owned = core.isGroupOwner(row.id)
            let soldOut = row.capacity > 0 && row.sold >= row.capacity
            byId["obj:\(row.id)"] = TicketVM(
                priceCents: row.priceCents,
                priceLabel: row.priceCents == 0 ? "FREE"
                    : TicketMoney.label(cents: row.priceCents, currency: row.currency),
                held: heldListings.contains(row.listing),
                pending: pendingTickets.pending.contains(row.listing),
                soldOut: soldOut,
                owned: owned,
                // The buyable path in this slice is the FREE one (phone-fulfilled); a priced
                // buy needs the box-office checkout (Slice 2/4), so its capsule is a value.
                buyable: !owned && !soldOut && row.open && row.priceCents == 0)
        }
        return EventTicketing(byId: byId,
                              buy: { buyTicket($0) },
                              door: { doorEvent = DoorLink(id: String($0.dropFirst(4))) })
    }

    /// GET TICKET on a free event: request-first — author the request Delta into the
    /// organizer's connection and mark the listing pending (9A). The organizer's device
    /// answers on its sweep; the credential's arrival clears the pending mark.
    private func buyTicket(_ vmId: String) {
        let objectId = String(vmId.dropFirst(4))
        guard let row = core.events.first(where: { $0.id == objectId }),
              row.listed, row.priceCents == 0 else { return }
        let owner = core.objectOwner(objectId)
        guard !owner.isEmpty, owner != core.myIdentityHex else { return }
        let pi = "free-" + UUID().uuidString.lowercased()
        pendingTickets.add(row.listing)
        Task { _ = await core.requestTicket(peer: owner, listing: row.listing, qty: 1, pi: pi) }
    }

    /// PEOPLE's real people, and the listings the wallet holds credentials for across
    /// them — read off the main thread, applied when they land.
    private func reloadPeople() {
        Task {
            let read = await core.peopleAndHeldListings()
            connections = read.people
            heldListings = read.held
        }
    }

    /// Entry work — run at launch (the preload warm-up) and again on every real
    /// arrival at this page. Cheap enough to repeat; keeping it in one place is
    /// what keeps the parked screen's data as fresh as the visited one's.
    private func enterLife() {
        // Every read here lands off the main thread: the page shows what it has at once.
        core.refresh()
        reloadPeople()                           // PEOPLE, and the ticket capsule's TICKET ✓ state
        core.refreshThings()                     // the market face's minted objects
        core.refreshEvents()                     // the minted Event GroupObjects
        core.refreshOrgs()                       // the Group identity records (GROUPS)
        Task { await graph.refreshEntities() }   // pull the latest resolved entities on entry
        shell.suppressPageSwipe = jiggle || showingMap
        shell.showCompass = showingMap
    }

    /// The real 1:1 connections as contact jewels — the locked PEOPLE ribbon. Photo + block
    /// colour come from the peer's published profile; the seeded family stands in until any real
    /// connections exist, so the ribbon is never blank on a fresh device.
    private var contactJewels: [ContactJewel] {
        let real = connections.map { c -> ContactJewel in
            let name = c.displayName.isEmpty ? String(c.peerId.prefix(12)) + "…" : c.displayName
            return ContactJewel(id: c.peerId, name: name,
                                photo: core.index.photo(c.peerId),
                                color: core.index.color(c.peerId) ?? LifeFilter.people.color)
        }
        return real.isEmpty ? ContactJewel.demo : real
    }

    // LIFE reads the on-device graph: the objects projected into it. Rebuilt on each render,
    // so a graph.cachedEntities change re-lists live.
    private var source: LifeSource {
        GraphLifeSource(core: core, resolved: graph.cachedEntities, connections: connections)
    }


    /// Connection display names keyed by identity hex — built once per render, not per row.
    private var peerNames: [String: String] { core.index.names }

    /// DEBUG-only headless drives of the pages simctl cannot reach by tapping
    /// (`-pacificOpenEvent` / `-pacificOpenListing`). Lifted OUT of `body`'s
    /// modifier chain: inline, it pushed the whole chain past the type-checker's
    /// budget. Launch-arg defaults are ephemeral, never persisted.
    private func openFromLaunchArgs() {
        #if DEBUG
        // A page nobody can screenshot is a page nobody can verify. Deferred: a
        // cover presented while the screen switch is still animating in is
        // silently dropped by SwiftUI.
        if let wanted = UserDefaults.standard.string(forKey: "pacificOpenEvent") {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
                if let row = core.events.first(where: { $0.title.localizedCaseInsensitiveContains(wanted) }) {
                    EventDetail.open(row.id, core: core, graph: graph, shell: shell)
                }
            }
        }
        // Same seam for a reference row's page (`-pacificOpenListing <title
        // fragment>`, empty = the first listing) — both Systems' feeds, music
        // first. Polled, because the feeds are live network fetches that land
        // seconds after launch.
        if let fragment = UserDefaults.standard.string(forKey: "pacificOpenListing") {
            Task { @MainActor in
                for _ in 0..<20 {
                    try? await Task.sleep(for: .seconds(1))
                    let matches: (String) -> Bool = {
                        fragment.isEmpty || $0.localizedCaseInsensitiveContains(fragment)
                    }
                    if let hit = music.events.first(where: { matches($0.title) }) {
                        EventDetail.open(listing: ReferenceListing(music: hit), core: core,
                                         graph: graph, shell: shell)
                        break
                    }
                    if let hit = art.events.first(where: { matches($0.title) }) {
                        EventDetail.open(listing: ReferenceListing(art: hit), core: core,
                                         graph: graph, shell: shell)
                        break
                    }
                }
            }
        }
        #endif
    }

    var body: some View {
        // The LIFE screen is TWO-FACED (the blue channel's second screen, 1 Aug): the
        // discover face — the multimodal filter — and the market face. The blue disc's
        // second tap turns it over (AxialFlip's slab renders the turn; this tree just
        // swaps faces instantly under it). EACH FACE OWNS ITS OWN NavigationStack:
        // the flip is a turn between the two faces' LAST screens — flip away from
        // inside a group view and back, and you are back inside the group view. One
        // shared stack put every pushed screen ABOVE the face swap, so flipping
        // under it visibly changed nothing.
        ZStack(alignment: .top) {
                NavigationStack(path: $path) {
                    discoverFace
                        // Pushed inside the DISCOVER stack — reserve the floating disc
                        // row's height so their bottom content (e.g. a project ChatRoom's
                        // composer) isn't hidden behind it. The shell's own inset only
                        // reaches the root.
                        .navigationDestination(for: String.self) { ProjectDetailView(objectId: $0).shellBottomBarClearance() }
                        .navigationDestination(for: EntityVM.self) { EntityDetailView(entity: $0).shellBottomBarClearance() }
                        .navigationDestination(for: GroupLink.self) { GroupDetailView(objectId: $0.id).shellBottomBarClearance() }
                        // Profile pages on the REAL binder (M1): the universal
                        // component reads the current fold through ProfileSource;
                        // fixtures survive only in Previews.
                        .navigationDestination(for: PersonLink.self) { link in
                            PersonProfilePage(id: link.id,
                                              source: RealProfileSource(core: core, subject: .peer(link.id)))
                                .shellBottomBarClearance()
                        }
                        .navigationDestination(for: PlaceLink.self) { link in
                            PlaceProfilePage(placeID: link.id,
                                             source: RealProfileSource(core: core, subject: .place(link.id)))
                                .shellBottomBarClearance()
                        }
                        // No `+` (9 Nov): RED's creator mints every kind it did.
                        .tabChrome("Life", showsTitle: false, showsNotifications: true,
                                   search: TabSearch(text: $lifeSearch, open: $searchOpen))
                }
        }
        // The feed assembles once per sync tick, once per filter change, and whenever one
        // of its inputs changes: a mint lands with no sync, and a device that cannot reach
        // the relay never ticks. (`feedInputsChanged` delivers on the next main-queue turn,
        // after the @Published property is set.) The equality gate inside the store makes
        // repeats free.
        .onReceive(core.$syncTick) { _ in feedStore.refresh(core: core, query: lifeQuery) }
        .onReceive(core.feedInputsChanged) { feedStore.refresh(core: core, query: lifeQuery) }
        .onChange(of: lifeQuery.locked) { _, _ in feedStore.refresh(core: core, query: lifeQuery) }
        .onChange(of: lifeQuery.bound) { _, _ in feedStore.refresh(core: core, query: lifeQuery) }
        .task {
            feedStore.refresh(core: core, query: lifeQuery)
            // RED's creator mints through these same sessions.
            shell.makeMintSession = { session(for: $0) }
        }
        .sheet(isPresented: $modesSheet) {
            ModesConfigSheet(modes: modeStore,
                             capture: { FeedMode.capture(lifeQuery, order: RibbonRail.storedOrder()) },
                             apply: { m in m.apply(to: lifeQuery); modeStore.saveActive(m) })
        }
        // The map (and arranging) own horizontal drags — the shell page-swipe must not
        // steal them, or a map pan flips to Notes/Chats instead of panning the map.
        .onChange(of: jiggle) { _, on in
            shell.suppressPageSwipe = on || showingMap
            shell.showCompass = showingMap
        }
        .onChange(of: lifeQuery.maximised) { _, m in
            shell.suppressPageSwipe = jiggle || showingMap
            shell.showCompass = showingMap
            // A jewel picked while searching is browsing, not typing: the keyboard gives
            // the ground back (it covered the map's lower half, where its sheet rises).
            if m != nil {
                UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder),
                                                to: nil, from: nil, for: nil)
            }
        }
        // Opening or closing search starts from the feed, every jewel minimised.
        .onChange(of: searchOpen) { _, _ in
            jiggle = false
            lifeQuery.minimiseAll()
            shell.suppressPageSwipe = showingMap
            shell.showCompass = showingMap
        }
        // BLUE pressed while LIFE is up (10 Aug order): home to the Feed — search
        // closed, every jewel minimised, same spring as the rail's own tap grammar.
        .onReceive(NotificationCenter.default.publisher(for: .pacificLifeHome)) { _ in
            withAnimation(.spring(response: 0.36, dampingFraction: 0.78)) {
                lifeSearch = ""
                searchOpen = false
                lifeQuery.minimiseAll()
            }
        }
        .sheet(item: $sheet) { outlineSheet($0) }
        // The organizer's door — full screen, like the verdicts it shows.
        .fullScreenCover(item: $doorEvent) { link in
            DoorScanSheet(objectId: link.id)
        }
        // The event's page — full screen, like the flyer it is. Same ticketing
        // contract as the cards, so the capsule acts identically on both surfaces.
        .sheet(isPresented: $scanningPlace) {
            ScanCodeSheet(onOpen: { _ in scanningPlace = false })
                .environmentObject(core)
        }
        // The POV cover: reached from the map sheet's WALK UP and from the
        // Place Profile page (T4) — the place's interior is a property of the place.
        .fullScreenCover(item: $openedPlace) { place in
            PlacePOVView(focus: place,
                         neighbours: placeAnnotations,
                         avatarColor: core.avatarColor,
                         onExit: { openedPlace = nil })
        }
        // PRELOADED: this screen mounts once, at launch, and stays mounted — so
        // `onAppear` is the data warm-up, and ARRIVAL is `shell.screen` becoming
        // .life (see ShellState.screen). Parking must also put back what LIFE
        // borrowed from the shell (the swipe lock, the compass).
        .onAppear { enterLife() }
        .onChange(of: shell.screen) { was, now in
            if now == .life {
                enterLife()
            } else if was == .life {
                shell.suppressPageSwipe = false
                shell.showCompass = false
            }
        }
        .onAppear { openFromLaunchArgs() }
        .onDisappear { shell.suppressPageSwipe = false; shell.showCompass = false }
        // Rebuild the commitment snapshot off-main on entry and after every graph change. Keyed
        // on `graph.epoch` (bumped when a projection changes the graph), so an event minted
        // while LIFE is open re-lists without a manual refresh. Mirrors GraphService's own
        // detached reads.
        .task(id: graph.epoch) {
            guard let resolver = graph.resolver else { return }
            eventsSnapshot = await Task.detached(priority: .userInitiated) {
                EventsSnapshot.load(from: resolver, tint: EventsSeed.tint)
            }.value
        }
        // A landed sync can add a connection — re-read it so PEOPLE self-heals. It can also
        // deliver a ticket or fold an event, so the capsule's inputs re-read with it.
        .onReceive(core.$syncTick) { _ in
            reloadPeople()
            // A landed sync can add a group, fold a profile edit, or land a
            // forum edge — the GROUPS page re-lists from the same tick.
            core.refreshOrgs()
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificOpenProject)) { note in
            // Absorbed from the old Work tab: agents/dev-links open a project by id.
            lifeQuery.select(.projects)
            if let obj = note.userInfo?["object"] as? String { path = NavigationPath([obj]) }
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificOpenPerson)) { note in
            // The detail modal's author bar, when the author is a person. Mirrors
            // the Space route below: land on the lane, push the typed page.
            lifeQuery.select(.people)
            if let obj = note.userInfo?["object"] as? String {
                path = NavigationPath([PersonLink(id: obj)])
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificOpenGroup)) { note in
            // A room title's deep link: land on GROUPS with the hosting group's
            // page pushed — the typed route the rows themselves take.
            lifeQuery.select(.groups)
            if let obj = note.userInfo?["object"] as? String {
                path = NavigationPath([GroupLink(id: obj)])
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .pacificOpenEventRow)) { note in
            // An RSVP reminder's tap: land on EVENTS and reopen the row it reminded
            // about — the minted page for "obj:", the reference page for "ra:"/"art:"
            // (looked up in its System's live feed; a listing gone from the feed opens
            // nothing rather than a stale ghost).
            guard let id = note.userInfo?["id"] as? String else { return }
            lifeQuery.select(.events)
            if id.hasPrefix("ra:") {
                if let hit = music.events.first(where: { $0.id == String(id.dropFirst(3)) }) {
                    EventDetail.open(listing: ReferenceListing(music: hit),
                                     core: core, graph: graph, shell: shell)
                }
            } else if id.hasPrefix("art:") {
                if let hit = art.events.first(where: { $0.id == String(id.dropFirst(4)) }) {
                    EventDetail.open(listing: ReferenceListing(art: hit),
                                     core: core, graph: graph, shell: shell)
                }
            } else if id.hasPrefix("obj:") {
                EventDetail.open(String(id.dropFirst(4)), core: core, graph: graph, shell: shell)
            }
        }
    }

    /// The DISCOVER face — the multimodal filter: each jewel owns the column beneath it
    /// and the column is EXACTLY the jewel's width. Locking activates a dimension;
    /// binding an entity pins it.
    private var discoverFace: some View {
        ZStack(alignment: .top) {
            // BASE, interactive. SINGLE-PAGE (FEED-TRANSMISSION 0-ANSWER-C): the ground
            // is keyed on the MAXIMISED jewel — nil means every jewel is minimised and
            // the ground is the Active Scroll Feed. Each config page owns its layer as
            // the PRIMARY view so its touches actually arrive (the map precedent).
            // Searching is the only way to a jewel (9 Nov): closed, the ground is the feed.
            if showingMap {
                // THE map returns as PLACE's maximised state (10 Aug word) — fullscreen,
                // the one map, with the join door.
                PlacesMap(places: placeAnnotations,
                          events: eventPinsOnMap,
                          onOpen: { openedPlace = $0 })
                    .ignoresSafeArea()
                    .overlay(alignment: .bottomTrailing) { joinPlaceButton }
            } else if searchOpen, let m = lifeQuery.maximised {
                // Every other maximised jewel is a flat LIST of that type's
                // GroupObjects (0-ANSWER-G S27), on the same card the feed uses.
                Theme.bg.ignoresSafeArea()
                TypedListPage(category: m, onOpen: openObject, onAuthor: openAuthor,
                              query: lifeSearch)
            } else {
                // Every jewel minimised: the Active Scroll Feed IS the LIFE page. Its
                // search is the top bar's (TabChrome's search layout), not a pull-down.
                Theme.bg.ignoresSafeArea()
                FeedView(store: feedStore, onOpen: openObject,
                         onAuthor: openAuthor, query: lifeSearch, railShown: searchOpen)
            }

            if searchOpen { selectors }
        }
    }

    /// The rail and the mode strip — up only while searching (9 Nov).
    @ViewBuilder private var selectors: some View {
        // The mode strip — the selector ROW's own gestures (S14): a decisive drag
        // RIGHT restores the last saved configuration; LEFT opens Modes Config.
        // Behind the rail, so jewels keep their taps and the strip reads the gaps.
        Color.clear
            .frame(height: RibbonWidth.contracted + 16)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 45)
                    .onEnded { v in
                        guard abs(v.translation.width) > 70,
                              abs(v.translation.width) > abs(v.translation.height) * 2
                        else { return }
                        if v.translation.width > 0 {
                            if let m = modeStore.active() { m.apply(to: lifeQuery) }
                            else { lifeQuery.clear() }
                        } else {
                            modesSheet = true
                        }
                    }
            )
            .zIndex(0.5)

        // The selector rail — LIFE's ONE selector mechanic. When a maximised selector
        // owns the background (the map, the events page), the rail collapses to the
        // jewel band plus each LOCKED selector's summary overlay and passes touches
        // through everywhere else, so the background stays interactive. It fills the
        // canvas otherwise (and while arranging, so the drag owns the whole surface).
        RibbonRail(query: lifeQuery,
                   orbID: feedStore.rows.last?.id,
                   onOrb: {
                       withAnimation(.spring(response: 0.34, dampingFraction: 0.72)) {
                           lifeQuery.minimiseAll()
                       }
                   },
                   jiggle: $jiggle)
            .frame(maxWidth: .infinity, alignment: .leading)
            .zIndex(1)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .background {
                if jiggle {
                    Color.clear
                        .contentShape(Rectangle())
                        .onTapGesture {
                            withAnimation(.spring(response: 0.34, dampingFraction: 0.72)) {
                                jiggle = false
                            }
                        }
                }
            }
    }

    /// Locked EVENTS overlays its located, still-outstanding commitments on THE map —
    /// the whole of the map consolidation. Unlocked, the map is places alone.
    private var eventPinsOnMap: [PlaceAnnotation] {
        guard lifeQuery.isLocked(.events) else { return [] }
        return EventsPage.venuePins(
            for: eventsSource.events(matching: lifeQuery.terms, window: .upcoming),
            places: placeAnnotations)
    }

    /// The top-down map is up only while searching with MAP maximised — the map's
    /// gestures and compass go with it.
    private var showingMap: Bool { searchOpen && lifeQuery.maximised == .places }

    /// THE tap contract for ObjectCard, everywhere it appears (feed + typed lists):
    /// The author DOOR (11 Aug order): tapping a card's header name opens the
    /// author's profile — a person's page, or the Space's (GroupDetailView).
    private func openAuthor(_ route: AuthorRoute) {
        switch route {
        case .person(let id): path.append(PersonLink(id: id))
        case .space(let id):  path.append(GroupLink(id: id))
        }
    }

    /// TAP = INSTANCE DETAIL, routed by the item's kind. Kinds without a detail page
    /// yet fall through silently — a card never navigates somewhere that isn't its
    /// object. Browsing a KIND stays on the rail (10 Aug order).
    private func openObject(_ item: FeedItem) {
        guard let oid = item.objectID else { return }
        switch item.category {
        case .groups: path.append(GroupLink(id: oid))
        case .events: EventDetail.open(oid, core: core, graph: graph, shell: shell)
        // TRADE used to open the EVENT page — a listing rendered as a night, mostly
        // empty. POSTS fell through `default:` to nothing at all, so a post card was
        // a card you could tap forever. Both open their own object now.
        case .trade:  ListingDetail.open(oid, core: core, shell: shell)
        case .posts:  PostDetail.open(oid, core: core, shell: shell)
        case .people: path.append(PersonLink(id: oid))
        case .places: path.append(PlaceLink(id: oid))
        default: break
        }
    }

    // MARK: Mint sessions — RED's creator mints through these

    /// One session per kind. The commit closure is the ONLY thing that differs, and each
    /// one is a single `core.mint` — the arg vocabularies live in `pacific_core::mint`,
    /// not here.
    private func session(for kind: NewKind) -> NewSession {
        // No per-kind seeding here: `NewSession` seeds every declared field that has an
        // opening answer (a Choice takes its first value, a Date takes now), from the
        // type's own declaration. That is what closes the 1970 bug — a required field is
        // never unanswered behind a control that looks filled in.
        return NewSession(kind: kind) { draft in
            guard let id = core.mint(kind: kind.rawValue, draft: draft.mint(for: kind))
            else { return nil }          // core surfaced the error; the surface stays up
            await afterMint(kind, id: id, draft: draft)
            return .saved
        }
    }

    /// What each kind needs AFTER its object exists. Deliberately separate from the mint:
    /// these are further ops (or pure refreshes), and folding them into one call would
    /// make a partial failure unreportable.
    @MainActor
    private func afterMint(_ kind: NewKind, id: String, draft: NewDraft) async {
        switch kind {
        case .space:
            // The founder's tenure is written by core at mint (membership-through-mls.md
            // §10.3) — the app no longer records it.
            core.refresh()
            core.refreshOrgs()
        case .listing:
            // A listing with no posture is invisible to the market, so seed the one the
            // editor seeds. Posture is op 1 — a separate delta by design.
            if !draft.posture.isEmpty {
                core.setPosture(id, posture: draft.posture, reach: draft.reach)
            }
            core.refreshThings()
        case .event:
            core.refreshEvents()
            // The event ALSO lands on the user's real Google Calendar, as it did before
            // the `+` was repointed at the real Event(29) mint. That mirror is orthogonal
            // to which object gets minted, so repointing the mint must not silently take
            // it away — see ruling R2, which is about whether
            // it should survive at all, not about whether to drop it by accident.
            //
            // Fire-and-forget with loud logging, exactly as before: a calendar that did
            // not accept the event must never fail the mint that did.
            mirrorToGoogleCalendar(draft)
        case .place:
            // A new Place has to reach `placeAnnotations` before the map can pin it.
            await graph.refreshEntities()
        case .project:
            lifeQuery.select(.projects)
            path.append(id)
        case .post:
            // A Post has its own spine and nothing to attach it to — it IS the object.
            // The refresh is what puts it in front of you.
            core.refresh()
        case .note, .channel:
            break                        // not reachable from LIFE's `+`
        }
    }

    /// The Google Calendar mirror the EVENTS `+` has always written (`events.insert`).
    /// OAuth is assumed handled — the client vends the token; a failure is logged
    /// loudly, never silently dropped.
    private func mirrorToGoogleCalendar(_ draft: NewDraft) {
        let title = draft.name.trimmingCharacters(in: .whitespaces)
        let start = draft.date ?? Date()
        let end = draft.endDate ?? start.addingTimeInterval(3600)   // default 1-hour block
        let desc = draft.descriptor
        Task {
            do {
                let ev = try await GoogleCalendarService().insertEvent(
                    summary: title, description: desc.isEmpty ? nil : desc, start: start, end: end)
                NSLog("[Events] Google Calendar event created: \(ev.id) — \(ev.htmlLink)")
            } catch {
                NSLog("[Events] Google Calendar insert failed: \(error.localizedDescription)")
            }
        }
    }

    // MARK: PROJECTS — the live project GroupObjects (reuses RealProjectCard)

    private var projects: [ObjectRow] { source.projects() }

    /// True when the selected category has nothing to list — drives the centred name.
    private var currentEmpty: Bool {
        filter == .projects ? projects.isEmpty : entityResults.isEmpty
    }

    private var projectList: some View {
        LazyVStack(spacing: 14) {
            ForEach(projects, id: \.id) { obj in
                NavigationLink(value: obj.id) { RealProjectCard(objectId: obj.id) }
                    .buttonStyle(.plain)
            }
        }
        .padding()
    }

    // MARK: Entity filters — flat lists over the graph (reuse FlatRow / EntityDetail)

    private var entityResults: [EntityVM] {
        guard let kind = filter.kind else { return [] }
        return source.entities(ofKind: kind)
    }

    // PLACES is minted from a COORDINATE, as it has been since the scan path was
    // orphaned (v1 ships no spatial processing): a Place is somewhere you have been,
    // and the fix carries that claim. See Life/Space/SpatialCaptureView.swift and the
    // excludes in ui/project.yml.

    /// The placed PLACES entities, for the top-down Map.
    private var placeAnnotations: [PlaceAnnotation] {
        // No demo fallback: showing Bristol/Stroud/London when you have scanned your
        // own room is worse than showing nothing, and it hid exactly this bug.
        return source.entities(ofKind: .place).compactMap(PlaceAnnotation.init(entity:))
    }

    private var entityList: some View {
        LazyVStack(spacing: 0) {
            VStack(spacing: 0) {
                ForEach(entityResults) { e in
                    NavigationLink(value: e) { FlatRow(entity: e) }
                        .buttonStyle(.plain)
                        .contextMenu { editMenu(e, present: { sheet = $0 }) }
                }
            }
            .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .padding(.horizontal)
        }
        .padding(.bottom)
    }

    // MARK: Chrome

    /// A blank category shows only its NAME — reusing NOTE/CHAT's empty-screen text
    /// (26pt semibold, tracked, secondary, sat above centre) so LIFE reads as the same
    /// surface. e.g. an empty PLACES puts "PLACES" where CHAT/NOTE put their word.
    private var emptyState: some View {
        GeometryReader { geo in
            Text(filter.rawValue)
                .font(.system(size: 18, weight: .semibold))   // NOTE/CHAT's 26 − 30%
                .tracking(1.4)
                .foregroundStyle(.secondary)
                .frame(width: geo.size.width, height: geo.size.height)
                .offset(y: -geo.size.height * 0.2)   // sit above centre (device-relative)
        }
    }
}
