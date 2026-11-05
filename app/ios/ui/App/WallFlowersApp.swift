//  WallFlowersApp.swift
//  Pacific for Aries Arise — the entry point and 3-tab root (Notes / Chats / Work).
//  Me (profile) and the Live core live behind the top-left account avatar. No language
//  model runs anywhere, on the device or on an Arc. The differentiator is behind-the-scenes
//  indexing + cross-referencing (the Notes outline + the Work projects + the Sysiphus SQL
//  interface), surfaced as inline enrichment. The interface is the "Slate" design
//  language: a deep slate ground with colour reserved for enriched entities,
//  status and rich media (see Theme).

import SwiftUI
import UIKit
import UserNotifications
import PacificStore
import LodeDBCore   // LodeEmbedder — the union embedder resolved at the composition root

@main
struct WallFlowersApp: App {
    /// SwiftUI does not surface the APNs callbacks, so a minimal delegate receives the token.
    @UIApplicationDelegateAdaptor(PushAppDelegate.self) private var pushDelegate
    @StateObject private var core: CoreService
    @StateObject private var graph: GraphService      // the on-device KG (derived read-index behind LIFE + note search)
    @StateObject private var outputs = OutputStore()  // the notification tray (ticket arrivals)
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var arcs: ArcStore           // the selected Arc — routes MLS
    @StateObject private var membership = ArcMembership()  // joining an Arc — the device's Tether
    @StateObject private var liveSync = LiveSync()    // foreground relay SUB — push-triggered sync
    @StateObject private var inAppNotify = InAppNotify()  // foreground message banner (in-app, not iOS)
    @StateObject private var enrich = EnrichmentService()
    @StateObject private var notes: NotesStore        // WS-F: backed by the union store
    @StateObject private var proposals: ProposalStore       // graph entities awaiting a swipe
    @StateObject private var convos = ConversationStore()
    @StateObject private var chatIndex: ChatIndex          // chats/forums vectorised into the union store
    @StateObject private var resolution: ResolutionService
    @StateObject private var system: SystemService
    @StateObject private var spans: ResolvedSpansStore
    @StateObject private var orchestrator: ResolutionOrchestrator
    @StateObject private var location: LocationService      // CoreLocation: map "you are here" + compass + the listing Systems' "near me"
    @StateObject private var google = GoogleService()       // Google Calendar: on-device OAuth System → EVENTS
    @StateObject private var music: MusicService            // the Music System (RA, scope-scoped) → EVENTS
    @StateObject private var art: ArtService                // the Art System (ArtRabbit, scope-scoped) → EVENTS
    @StateObject private var venues = VenueProposer()       // NEW RA venues → Place mint suggestions
    /// The launch sequence (Wake → Orbit-CMY) plays once over black, then fades to
    /// reveal the app. True until the gate finishes.
    @State private var showLaunch = true

    init() {
        Self.applySlateChrome()

        // ONE notification delegate, set before launch finishes so a tap that launched the app
        // is still delivered to it (the center holds its delegate weakly, hence `shared`). The
        // mint category registers alongside it: a banner raised before its category exists
        // shows no actions.
        UNUserNotificationCenter.current().delegate = NotifPresenter.shared
        ProposalStore.registerNotificationCategory()

        // THE SEOUL LANGUAGE LANE: the ONE
        // persisted authority (`UnionLanguageSetting`) selects the union embedder's
        // language. Resolved FIRST — before anything constructs an embedder — so that a
        // configured language whose model cannot come up (the ko contextual assets are
        // an on-demand OS download) is RESET to English here, out loud, and every later
        // `UnionLanguageSetting.load()` in this process sees the resolved value. A reset
        // is visible (logged + persisted), never a silent per-callsite fallback.
        // NOTHING HERE MAY KILL THE LAUNCH. This block used to `fatalError` on a
        // missing embedder, and the assets it needs — Apple's on-device NL models
        // (LinguisticData for en, NLContextualEmbedding for the CJK lane) — are OS
        // downloads that a FRESHLY CREATED SIMULATOR does not have. A harness
        // provisions exactly that, so the app could not start at all on the device
        // most likely to be asked to make an account: no screen, no first boot, no
        // identity. An unavailable index is a degraded app; an app that will not
        // launch is not an app. The degrade is loud and it fails every call it
        // cannot honour (DegradedUnionStore) — it does not quietly run in memory.
        var unionLanguage = UnionLanguageSetting.load()
        var unionEmbedder: LodeEmbedder?
        /// The first fault, in the words of whatever threw it — carried all the way
        /// into the degraded store so a log read later still names the real cause.
        var unionFault: String?
        do {
            unionEmbedder = try UnionEmbedder.makeUnion(language: unionLanguage)
        } catch {
            if unionLanguage != .english {
                NSLog("[union] '%@' embedder unavailable (%@) — resetting the union language to en",
                      unionLanguage.rawValue, "\(error)")
                unionLanguage = .english
                UnionLanguageSetting.save(unionLanguage)
                do { unionEmbedder = try UnionEmbedder.makeUnion(language: unionLanguage) }
                catch { unionFault = "the en union embedder could not be built — \(error)" }
            } else {
                unionFault = "the en union embedder could not be built — \(error)"
            }
        }
        if let unionFault {
            NSLog("[union] EMBEDDER UNAVAILABLE — %@", unionFault)
            NSLog("[union] the app runs WITHOUT the union index. On a simulator this "
                + "is usually the on-device NL asset: it downloads on first use and "
                + "a fresh device has never had one.")
        }

        _graph = StateObject(wrappedValue: GraphService())
        // Built here (not inline) because the Arc store needs THIS instance: reading a
        // @StateObject's wrapped value inside init is not valid, so the concrete object is
        // made once and handed to both.
        let coreService = CoreService()
        _core = StateObject(wrappedValue: coreService)
        // The Arc store decides WHICH relay; the core owns the transport set. Wire them so a
        // change of Arc reaches the router instead of stopping at a UserDefault.
        let arcStore = ArcStore()
        arcStore.core = coreService
        _arcs = StateObject(wrappedValue: arcStore)

        // The ONE union store (WS-E) every local producer writes through and LIFE / note
        // search read from: the REAL, LodeDB-backed store on disk at
        // `<AppSupport>/pacific/union.lode`. The backend is chosen HERE, in one place,
        // explicitly — and there is NO silent fallback. If the store or its embedder
        // cannot come up, that is a real fault (a missing engine, an unavailable model,
        // a corrupt store) and we surface it rather than quietly running in-memory and
        // losing every write on relaunch.
        //
        // The embedder follows the union language authority resolved above (en = Apple's
        // NL sentence model, the shipped default; ko = the CJK contextual model); BGE
        // lands once the ONNX runtime is provisioned (see UnionEmbedder / UnionConfig).
        let unionStore: LocalStore
        if let unionEmbedder {
            do {
                // The LANGUAGE-CHANGE reindex: when the on-disk store was built under a
                // different language's model, rebuild it from retained text before opening
                // (no-op otherwise). This is the launch half of the `union-language` dev
                // verb — a live store handle cannot swap models under its producers.
                if try LodeLocalStore.migrateStore(to: unionLanguage, embedder: unionEmbedder) {
                    NSLog("[union] store reindexed under %@",
                          UnionConfig.modelIdentity(for: unionLanguage))
                }
                unionStore = try LodeLocalStore(embedder: unionEmbedder, language: unionLanguage)
            } catch {
                // Same rule as the embedder above: a store that will not open is a
                // real fault and is reported as one on every call, but it is not a
                // reason to refuse to run. A corrupt store must not cost someone the
                // ability to reach their own identity.
                unionStore = DegradedUnionStore(cause: "the union store could not be opened — \(error)")
            }
        } else {
            unionStore = DegradedUnionStore(cause: unionFault ?? "no union embedder")
        }
        let notesStore = NotesStore(store: unionStore)
        _notes = StateObject(wrappedValue: notesStore)
        // Chats/forums vectorise into the SAME store — one index, one embedder;
        // the CHAT search bar's cosine lane reads it back (ChatIndex).
        _chatIndex = StateObject(wrappedValue: ChatIndex(store: unionStore))
        _proposals = StateObject(wrappedValue: ProposalStore())
        // The entity graph (in-session truth) projects its nodes/edges into the same store.
        OutlineModel.shared.attach(store: unionStore)

        // The System is scoped to a credential; it feeds resolved Fields into the resolver.
        let r = ResolutionService()
        _resolution = StateObject(wrappedValue: r)
        _system = StateObject(wrappedValue: SystemService(resolution: r))
        // The Music System is scoped to an AREA (RA is public — no credential); its
        // listings index into the SAME resolver, the neutral ResolvableEntity tier.
        // Built here (the @StateObject-in-init pattern above) because the listing
        // Systems follow THIS instance's fix: an unpinned scope tracks the device.
        let loc = LocationService()
        _location = StateObject(wrappedValue: loc)
        _music = StateObject(wrappedValue: MusicService(resolution: r, location: loc))
        // The Art System mirrors it: scoped to a PLACE (ArtRabbit is public too), its
        // listings index into the same resolver.
        _art = StateObject(wrappedValue: ArtService(resolution: r, location: loc))
        // The resolved-span cache + orchestrator (buildContext -> candidates -> heuristic
        // decide), all on device. The layer that augments the exact-alias catalogue; consumed
        // by the chat-receive resolver once EnrichedText reads cached spans (next step).
        let store = ResolvedSpansStore()
        _spans = StateObject(wrappedValue: store)
        _orchestrator = StateObject(wrappedValue: ResolutionOrchestrator(resolution: r, store: store))
    }

    /// THEME — "pacific" (light) or "kenjin" (dark). The switch lives in
    /// Settings; the scheme is pinned here so the whole tree re-themes live.
    @AppStorage("themeMode") private var themeMode = "kenjin"

    var body: some Scene {
        WindowGroup {
            ZStack {
            Group {
                // FIRST LOAD: the boot sequence owns the screen until it has been
                // walked to its end (BootFlow — door, name+mint, face, bio,
                // recovery key, fingerprint). The launch task below opens the graph and
                // then waits on the same gate: everything else it starts (tags, live tap,
                // capabilities, drains) needs the identity to exist.
                if core.needsOnboarding { BootFlow() } else { RootShell() }
            }
                .environmentObject(core)
                .environmentObject(graph)
                .environmentObject(outputs)
                .environmentObject(arcs)
                .environmentObject(membership)
                .environmentObject(enrich)
                .environmentObject(notes)
                .environmentObject(proposals)
                .environmentObject(convos)
                .environmentObject(chatIndex)
                .environmentObject(resolution)
                .environmentObject(location)
                .environmentObject(system)
                .environmentObject(google)
                .environmentObject(music)
                .environmentObject(art)
                .environmentObject(spans)
                .environmentObject(orchestrator)
                .tint(Brand.accent)
                // THEME: the Settings switch decides the key — Kenjin (Dark)
                // is home; Pacific (Light) is the same ramp on paper.
                .preferredColorScheme(themeMode == "pacific" ? .light : .dark)
                // The in-app message banner, over EVERYTHING (including an open
                // conversation — where a banner for another chat is the whole point).
                .overlay(alignment: .top) { InAppBannerHost(notify: inAppNotify).padding(.top, 4) }
                .task {
                    // Open the on-device graph FIRST, ahead of the onboarding gate and every
                    // network step below. Everything it holds is projected from objects this
                    // device has folded, so it needs no identity, no Arc and no network, and
                    // LIFE, event detail and note search read it from the first frame.
                    graph.open()
                    // The projections the first render fired may have found the graph still
                    // closed and been dropped, so offer the current snapshot now rather than
                    // leaving it to wait for a sync to succeed, which offline may be never.
                    Task { await ObjectProjection.project(core: core, graph: graph) }
                    // ONBOARDING GATE: everything below needs the identity to exist
                    // (tags, the live tap, capabilities, the drains). On a first load
                    // this waits for Welcome's mint; normal launches pass straight through.
                    NSLog("[launch] task start, needsOnboarding=%d", core.needsOnboarding ? 1 : 0)
                    if core.needsOnboarding {
                        for await needs in core.$needsOnboarding.values { if !needs { break } }
                    }
                    NSLog("[launch] gate passed")
                    // NO PERMISSION PROMPT HERE. First boot asks for nothing the OS
                    // has to arbitrate: notifications ride
                    // the first moment a wake is wanted (`WakePermission`, from a
                    // connection accepted or a reminder armed), location belongs to
                    // the map, and photos are implied by the picker.
                    //
                    // Ask APNs for a token. Without one this device cannot be woken at all, so
                    // it is requested every launch — the token is not stable across reinstall,
                    // restore or some OS updates.
                    PushRegistrar.shared.register()
                    // THE WAKE SEAM. The push delegate has no route to the core (it is a
                    // @StateObject built in this App's init), so the capability is handed DOWN
                    // rather than reached up for: a content-available push runs exactly this.
                    PushRegistrar.shared.onWake = { await InboundWake.shared.wake(core: core) }
                    // Assert the current tag set at launch, then on every sync tick below
                    // (tags rotate per epoch). Debounced — see WakeDebounce.
                    PushRegistrar.shared.syncRegistration(tags: core.wakeTags())
                    // The live tap: while foregrounded, one relay socket SUBbed to our tags
                    // turns delivery from poll-bound (≤3s) into push-triggered (~sub-second).
                    liveSync.start(core: core)
                    // NOT a health check here. `checkHealth` reaches a host that may not
                    // resolve, and awaiting it in the launch task would hold every later step
                    // (Arc capabilities, the listing Systems) behind a DNS
                    // timeout. The WAKE screen probes on appear, which is the only place the
                    // answer is actually looked at.
                    // Note titles join the catalogue at launch (not on first visit
                    // to the Notes tab — the app opens on Chats).
                    enrich.ingestNotes(notes.notes)
                    // Location updates START here but the PERMISSION does not: `resume`
                    // only begins updates if the user has already granted When-In-Use
                    // (on the map, where the ask means something). Without it the
                    // listing Systems keep their persisted or default scope and upgrade
                    // the moment a fix arrives — `followLocation` is built for exactly
                    // that, and a scope was never worth a cold-launch permission alert.
                    location.resume()
                    // Discover the selected Arc's capabilities AT LAUNCH, not on a visit to some
                    // screen. The capability document is where the Arc advertises its relay
                    // (adopted over the listed one) and its space id — the audience membership
                    // credentials are signed for — so a cold launch must not wait on navigation.
                    await arcs.refreshCapabilities()
                    // The Music and Art Systems pull their scopes' listings AT LAUNCH,
                    // so the EVENTS page comes populated without a Systems-tab visit.
                    // Gated by the -pacificNoMusic / -pacificNoArt opt-outs inside
                    // refresh() (the UI tests' seams); a source down ⇒ a loud status in
                    // its System view, EVENTS just shows local rows.
                    await music.refresh()
                    await art.refresh()
                    #if DEBUG
                    // Headless drive: one fixture venue proposal through the real sweep
                    // (-pacificSeedVenueProposal <name>) — the venue-deck UI test's seam.
                    await venues.seedIfRequested(core: core, proposals: proposals)
                    #endif
                }
                .onReceive(core.$objects) { objs in
                    // ChatRooms (Forum GroupObjects) join the catalogue so a mention of a room's
                    // name resolves and the type-router opens the real thread. Re-runs on every
                    // object change, so it self-heals as the sync loop lands seeded titles.
                    // Keyed off the SYNCED GroupContext name, not this device's local mirror —
                    // a member who joined (rather than created) the room has no local title.
                    // Reads the EMITTED list: a @Published publisher fires before the property
                    // is set, so `core.objects` here is still the previous list.
                    let rooms = objs
                        .filter { $0.kind == "forum" && !$0.name.isEmpty }
                        .map { (id: $0.id, title: $0.name) }
                    enrich.ingestConversations(rooms)
                    // DELTA FOLD → GRAPH, immediately on a list change: a local mint
                    // (OrgWizard/EntityWizard) publishes `objects` the moment it lands.
                    // `refresh()` equality-gates this publisher, so a steady-state sync
                    // tick no longer re-fires it — roster/venue changes that leave the
                    // list itself identical are covered by the syncTick trigger below,
                    // where the projection's own fingerprint makes an unchanged pass free.
                    Task { await ObjectProjection.project(core: core, graph: graph, objects: objs) }
                }
                .onReceive(core.$syncTick) { _ in
                    // DELTA FOLD → GRAPH, the every-sync half: rosters, orgViews and venues
                    // can change while the object LIST stays identical, so each tick offers
                    // the projection a fresh snapshot. Inputs are read off-main and the
                    // fingerprint skips the reconcile when nothing the projector reads moved.
                    Task { await ObjectProjection.project(core: core, graph: graph) }
                    // Relay tags are PER-EPOCH, so a registration goes stale the moment a group
                    // advances. Re-assert the CURRENT set every tick — idempotent and cheap,
                    // because WakeDebounce sends only when the set (or the token) actually
                    // changed, when the assertion went stale, or when a backoff expired.
                    // The live tap re-SUBs off the same set (it compares before sending).
                    let tags = core.wakeTags()
                    PushRegistrar.shared.syncRegistration(tags: tags)
                    liveSync.assertTags(tags)
                    // Foreground arrivals announce IN-APP (skipping the open thread);
                    // the same seen-set keeps background wakes from re-announcing them.
                    InboundWake.shared.foregroundTick(core: core, notify: inAppNotify)
                }
                .onChange(of: scenePhase) { _, phase in
                    // Returning to the foreground is the moment connectivity most often came
                    // back, so the live tap reopens here — while foregrounded the phone is its
                    // own waker (see LiveSync.swift).
                    if phase == .active, !core.needsOnboarding {
                        liveSync.start(core: core)
                        Task { await google.refreshEvents() }
                    }
                    if phase == .background {
                        // The live socket dies with the scene anyway — close it deliberately
                        // and hand delivery back to the waker (APNs).
                        liveSync.stop()
                        // Everything on screen this session counts as SEEN. Without this the
                        // first background wake would announce messages the user already read,
                        // because the fold re-presents them identically on every sync.
                        InboundWake.shared.absorbCurrent(core: core)
                        // Commit a profile edit that is still inside its debounce
                        // window, so a change made a moment before backgrounding is
                        // published rather than lost with the process.
                        Task { await core.flushProfile() }
                    }
                }
                .onReceive(arcs.$routeEpoch.dropFirst()) { _ in
                    // The route was repaired or moved by a DISCOVERY pass (not a selection) — a
                    // new origin or a new audience — so membership proven against the old one
                    // proves nothing now. Ask the Arc again.
                    if let arc = arcs.selected { Task { await membership.verify(arc) } }
                }
                .onReceive(graph.$cachedEntities) { entities in
                    // Proposals follow the GRAPH, not the view lifecycle: whenever the read cache
                    // refills, rebuild the deck from it and announce what has never been
                    // announced. Shell's own rebuilds stay (idempotent); this one does not depend
                    // on the shell or the tray being mounted. It reads the EMITTED list, because a
                    // @Published publisher fires before the property is set.
                    let mintedNames = Set(core.objects
                        .filter { ["group", "thing", "place", "project"].contains($0.kind) }
                        .map { $0.name.lowercased() })
                    proposals.rebuild(resolved: entities, mintedNames: mintedNames)
                }
                .onReceive(system.$fields) { fields in
                    // A signed-in credentialed System feeds its REAL Sysiphus designs into the
                    // ONE catalogue -> they light up in chat/notes/work + long-press panel via the
                    // mature enrichment path. Guarded: only ingest when signed in and non-empty, so
                    // this never clears the fixture .sysiphus origin on launch / sign-out. (Retiring
                    // the SQLite fixture service leaves this the sole .sysiphus feeder — the unify.)
                    guard system.isSignedIn, !fields.isEmpty else { return }
                    enrich.ingest(fields.map(EnrichedEntity.init(sysiphus:)), from: .sysiphus)
                }
                .onReceive(music.$events) { events in
                    // The Music System's listings join the ONE catalogue — events, artists
                    // and venues light up in chat/notes at REFERENCE priority (.music sits
                    // below every local origin). Guarded like the Sysiphus ingest: a failed
                    // or empty fetch never clears what a previous pull registered.
                    guard !events.isEmpty else { return }
                    enrich.ingest(MusicEnrichmentBridge.entities(from: events), from: .music)
                    // Every NEW venue in the refreshed feed becomes a Place SUGGESTION:
                    // an actionable MINT banner (capped per refresh) + a Convergence
                    // card whose accept mints the real Place GroupObject. The proposer
                    // dedups durably (seen-set + minted-Place names), so the steady
                    // state proposes nothing. -pacificNoMusic never reaches here — the
                    // opt-out stops refresh() publishing at all.
                    Task { await venues.sweep(events: events, core: core, proposals: proposals) }
                }
                .onReceive(art.$events) { events in
                    // The Art System's listings join the same catalogue at the same
                    // REFERENCE priority, under the same guard — and its venues take
                    // the SAME notification path as RA's: every new venue with an
                    // actual street address (from its event page) becomes a Place mint
                    // suggestion; accept mints the GroupObject, address as descriptor.
                    guard !events.isEmpty else { return }
                    enrich.ingest(ArtEnrichmentBridge.entities(from: events), from: .art)
                    Task { await venues.sweepArt(events: events, core: core, proposals: proposals) }
                }
                .onReceive(location.$coordinate) { _ in
                    // The fix arrived or moved. Each listing System re-scopes ONLY when
                    // its scope is unpinned and the device actually left it (the cheap
                    // distance guard inside), so ordinary jitter is a no-op — but the
                    // first fix after a fallback launch upgrades SF to where you ARE.
                    Task { await music.followLocation(); await art.followLocation() }
                }

            if showLaunch {
                // No cross-fade here: LaunchView opens its own aperture and only calls back
                // once it is fully transparent, so fading it again would re-darken the app
                // through the reveal — the flash-to-black we just removed.
                LaunchView { showLaunch = false }
                    .transition(.identity)
                    .zIndex(1)
            }
            }
        }
    }

    /// Slate-tint the UIKit bars so nav + tab chrome matches the deep slate ground.
    private static func applySlateChrome() {
        let slate = UIColor(Theme.bg)

        let tab = UITabBarAppearance()
        tab.configureWithOpaqueBackground()
        tab.backgroundColor = slate
        tab.shadowColor = .clear
        UITabBar.appearance().standardAppearance = tab
        UITabBar.appearance().scrollEdgeAppearance = tab

        let nav = UINavigationBarAppearance()
        nav.configureWithOpaqueBackground()
        nav.backgroundColor = slate
        nav.shadowColor = .clear
        nav.titleTextAttributes = [.foregroundColor: UIColor(Theme.textPrimary)]
        nav.largeTitleTextAttributes = [.foregroundColor: UIColor(Theme.textPrimary)]
        UINavigationBar.appearance().standardAppearance = nav
        UINavigationBar.appearance().scrollEdgeAppearance = nav
        UINavigationBar.appearance().compactAppearance = nav
    }


}
