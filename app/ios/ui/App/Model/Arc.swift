//  Arc.swift
//  An Arc — a community's micro-datacentre. In the v1 API an Arc
//  is ONE versioned origin — `https://<host>/v1/…` — fronting its planes: identity, relay (blind
//  messaging), systems (connectors), and membership. The app learns ONE base URL and derives every
//  endpoint from it; a single discovery call (`GET /v1/arc`) reports which planes are live, so the
//  client never dials a dead endpoint or fakes a capability (ICD "honest capability").
//
//  Relay stays an explicit URL rather than `origin/v1/relay`, because today's live relay answers at
//  the host root; it becomes discovery-driven (planes.relay.url) once an Arc serves `/v1/arc`.
//
//  Selecting an Arc routes messaging (its relay) and records its origin and the audience that
//  membership credentials are signed for.

import Foundation
import PacificCore   // is_retired_relay_url: the core owns the dead-relay list

// MARK: - Arc

struct Arc: Identifiable, Hashable, Codable {
    let id: String            // stable code, e.g. "arc.global.01"
    let name: String          // "WallFlowers Global"
    let region: String        // "Global relay" / "United Kingdom"
    /// The ONE origin fronting every v1 plane, e.g. "https://arc.wallflowers.io". All HTTP
    /// endpoints derive from it: origin + "/v1/arc", "/v1/health", "/v1/systems".
    let origin: String
    /// The blind relay's wss URL. Directory-given to begin with, then REPLACED by the Arc's own
    /// `planes.relay.url` on the first successful discovery (`ArcStore.adoptAdvertisedRelays`) —
    /// the Arc is the authority on where its relay answers, not the catalog entry that found it.
    var relayURL: String
    /// A self-hosted / key-gated Arc's legacy `x-api-key`. Retired: membership (the MLS
    /// tether, ICD §10) is the per-request credential. nil for catalog Arcs.
    var serveKey: String? = nil
    /// True for an Arc the user added themselves (This Mac, a self-hosted box).
    var userAdded: Bool = false

    /// Join `origin` + a `/v1/...` path, tolerating a trailing slash on the origin.
    static func join(_ base: String, _ path: String) -> String {
        var b = base
        while b.hasSuffix("/") { b = String(b.dropLast()) }
        return b + path
    }

    // v1 endpoints — all derived from `origin`, never stored.
    var arcInfoURL: String  { Self.join(origin, "/v1/arc") }
    var healthURL: String   { Self.join(origin, "/v1/health") }
    var systemsBase: String { Self.join(origin, "/v1/systems") }

    var hasRelay: Bool { !relayURL.isEmpty }
    var hasOrigin: Bool { !origin.isEmpty }
    /// ADDRESSABLE: we know where to send a request. A non-empty string, nothing more — it
    /// says a URL exists, never that anything answers there.
    var isAddressable: Bool { hasRelay || (userAdded && hasOrigin) }

    /// Routable: addressable, AND — where discovery has actually run — reported at least one
    /// live plane. `capability` is nil when `GET /v1/arc` has not answered yet, and an unknown
    /// must not disqualify an Arc (that would make a slow probe look like a dead Arc), so nil
    /// falls back to addressability alone.
    ///
    /// Absence of evidence is not evidence of absence. What changed is that a POSITIVE
    /// "no planes live" now counts, where before only string-emptiness did.
    func isSelectable(_ capability: ArcCapability? = nil) -> Bool {
        guard isAddressable else { return false }
        guard let capability else { return true }
        return capability.relayLive || capability.systemsLive
    }

    /// Back-compat shorthand for call sites with no capability to hand.
    var isSelectable: Bool { isSelectable(nil) }
    /// The single identity shown in the picker — the origin (an Arc IS its origin), else the relay.
    var displayURL: String { origin.isEmpty ? relayURL : origin }
}

/// An Arc's discovered capability document (`GET /v1/arc`): its sovereign identity + which planes
/// are live. The SINGLE source of truth for what an Arc can do — replaces the old flat `/health`
/// probe. A plane's absence (or `live:false`) is reported, never masked.
struct ArcCapability: Equatable {
    var name: String? = nil
    var spaceId: String? = nil
    var fingerprint: String? = nil
    var relayLive: Bool = false
    var relayURL: String? = nil
    var systemsLive: Bool = false
    var providers: [String] = []
}

/// Discovery + liveness against an Arc's v1 origin. Honest by default: an unreachable Arc or one
/// not yet serving `/v1/arc` returns nil, never a fabricated "live".
enum ArcDiscovery {
    /// `GET origin/v1/arc` → the capability document, or nil (unreachable / discovery not up yet).
    static func capabilities(origin: String) async -> ArcCapability? {
        guard !origin.isEmpty, let url = URL(string: Arc.join(origin, "/v1/arc")) else { return nil }
        var req = URLRequest(url: url); req.timeoutInterval = 3
        guard let (data, resp) = try? await URLSession.shared.data(for: req),
              let http = resp as? HTTPURLResponse, (200..<300).contains(http.statusCode),
              let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else { return nil }
        return parse(obj)
    }

    /// Parse the ICD `/v1/arc` shape: { name, space_id, fingerprint, planes:{ relay{live,url},
    /// systems{live,providers} } }. Any other plane an Arc advertises is ignored: the app has no
    /// client for it, so reporting it live would promise a capability the app cannot use.
    static func parse(_ o: [String: Any]) -> ArcCapability {
        let planes = o["planes"] as? [String: Any] ?? [:]
        let relay = planes["relay"] as? [String: Any] ?? [:]
        let sys = planes["systems"] as? [String: Any] ?? [:]
        return ArcCapability(
            name: o["name"] as? String,
            spaceId: o["space_id"] as? String,
            fingerprint: o["fingerprint"] as? String,
            relayLive: relay["live"] as? Bool ?? false,
            relayURL: relay["url"] as? String,
            systemsLive: sys["live"] as? Bool ?? false,
            providers: (sys["providers"] as? [String]) ?? []
        )
    }

    /// Normalize a user-entered value into a canonical v1 ORIGIN (scheme://host[:port], no path).
    /// Accepts "127.0.0.1:8080", "http://host", or a full URL with a path; defaults a scheme-less
    /// private/loopback host to http, everything else to https.
    static func normalizeOrigin(_ raw: String) -> String? {
        var s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !s.isEmpty else { return nil }
        if !s.contains("://") {
            let hostPart = s.split(separator: "/").first.map(String.init) ?? s
            let local = ["127.", "localhost", "0.0.0.0", "192.168.", "10."].contains { hostPart.hasPrefix($0) }
            s = (local ? "http://" : "https://") + s
        }
        guard var comps = URLComponents(string: s), let scheme = comps.scheme, let host = comps.host else { return nil }
        var out = "\(scheme)://\(host)"
        if let port = comps.port { out += ":\(port)" }
        return out
    }

    /// Normalize a user-entered RELAY into a ws/wss URL — the same courtesy `normalizeOrigin`
    /// does for an origin, since someone typing a relay by hand will paste the https:// form of
    /// it as often as the wss:// one. The PATH is kept (a gateway serves the relay at
    /// `/v1/relay`, not the host root), which is the difference that matters here.
    static func normalizeRelayURL(_ raw: String) -> String? {
        var s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !s.isEmpty else { return nil }
        if !s.contains("://") {
            let hostPart = s.split(separator: "/").first.map(String.init) ?? s
            let local = ["127.", "localhost", "0.0.0.0", "192.168.", "10."].contains { hostPart.hasPrefix($0) }
            s = (local ? "ws://" : "wss://") + s
        }
        guard var comps = URLComponents(string: s), let scheme = comps.scheme else { return nil }
        switch scheme {
        case "http":     comps.scheme = "ws"
        case "https":    comps.scheme = "wss"
        case "ws", "wss": break
        default:         return nil
        }
        guard comps.host != nil else { return nil }
        return comps.string
    }
}

/// The Arc directory. The community mesh is fetched LIVE from the directory (`GET /api/arcs`,
/// see `ArcCatalog.swift`). The one constant is `globalDefault`: Pacific's always-on global Arc,
/// kept even when the directory is unreachable so there is always exactly one usable Arc.
enum ArcDirectory {
    /// The always-on default: WallFlowers' global Arc — the unified v1 deployment,
    /// ONE origin with the gateway fronting relay, membership, waker and box office. Each plane
    /// reports itself via `/v1/arc` and `/v1/health`, so the picker shows real liveness.
    ///
    /// ONE HOST FOR BOTH HALVES since 19 Sep 2026: the gateway serves `/v1/arc` and signup
    /// itself. Relay is `/v1/relay` on the gateway (NOT the host root — that was the
    /// pre-unification relay), matching `planes.relay.url` in the capability doc. Retired relays
    /// are listed in pacific-core paths.rs (RETIRED_RELAY_URLS). What the relay may KNOW is
    /// unchanged by sharing a front door.
    static let globalDefault = Arc(
        id: "arc.global.01", name: "WallFlowers Global", region: "Global",
        origin: "https://arc.wallflowers.io",
        relayURL: "wss://arc.wallflowers.io/v1/relay")

    /// Live Arcs first (the ones you can actually use), then the rest.
    static func nearestFirst(_ arcs: [Arc]) -> [Arc] {
        arcs.sorted { a, b in
            if a.hasRelay != b.hasRelay { return a.hasRelay }
            return a.name < b.name
        }
    }
}

/// Where the live directory stands: idle before the first load, then loading / loaded, or
/// failed with a human message the picker shows (never a silent empty list).
enum ArcLoadState: Equatable {
    case idle
    case loading
    case loaded
    case failed(String)
}

// MARK: - ArcStore

/// The single source of truth for Arc selection. Picking an Arc writes the routing keys — the relay
/// `CoreService.relayUrl` reads, with the origin and membership audience beside it — atomically,
/// all derived from ONE origin, so what the UI shows and what routes can never diverge. v1
/// collapse: an Arc is an origin + a relay URL; capability comes from `GET /v1/arc`.
@MainActor
final class ArcStore: ObservableObject {
    /// The Arc routing this device. On a fresh install this resolves to the live default for
    /// DISPLAY; routing keys are written only when the user actively picks an Arc.
    @Published private(set) var selected: Arc?

    /// The picker list: the user's own self-hosted Arcs first, then the always-on Global Arc +
    /// the community mesh. Never empty.
    @Published private(set) var directory: [Arc] = [ArcDirectory.globalDefault]

    /// The state of the live directory load, so the picker can show loading / error.
    @Published private(set) var loadState: ArcLoadState = .idle

    /// Discovered capability per Arc id (`GET /v1/arc`) — which planes are live.
    /// Never baked into the catalog: an Arc reads as live ONLY when its gateway answers.
    @Published private(set) var capability: [String: ArcCapability] = [:]

    /// Arc ids a discovery pass has actually COMPLETED for, whatever the outcome. Without this
    /// a missing capability is ambiguous — never probed and probed-and-dead look identical —
    /// and the UI would have to guess which. It must not guess.
    @Published private(set) var probed: Set<String> = []

    /// Bumped whenever the ROUTE actually changes (origin or audience). Membership is proven
    /// against a route, so a proof made against the old one has to be asked again when it
    /// moves. Selection alone is not enough: a discovery pass can repair a half-written route
    /// long after the Arc was selected.
    @Published private(set) var routeEpoch = 0

    /// The relay messaging ACTUALLY uses, republished whenever it changes. `CoreService.relayUrl`
    /// reads the same default, but statically — settings needs a value SwiftUI can observe, and
    /// a screen that shows a stale relay is worse than one that shows none.
    @Published private(set) var relayUrl: String = CoreService.relayUrl

    /// The user set the relay by hand, so selecting an Arc must not silently move it back. The
    /// on-device equivalent of the `PACIFIC_RELAY_URL` the harness scripts pass a simulator —
    /// without it a phone has no way to reach an Arc the directory hasn't heard of.
    @Published private(set) var relayPinned: Bool = false

    /// The core, so a change of Arc can HAND OVER the new transport set. Transport is core state
    /// now; writing the user default alone would leave messaging on the previous set.
    weak var core: CoreService?

    private let defaults = UserDefaults.standard
    private var community: [Arc] = []
    private var userArcs: [Arc] = []
    /// `planes.relay.url` per Arc id, learned from discovery and overlaid onto the directory.
    private var relayAdvertised: [String: String] = [:]

    init() {
        userArcs = loadUserArcs()
        rebuildDirectory()
        let savedId = defaults.string(forKey: Keys.arcId)
        selected = directory.first { $0.id == savedId } ?? directory.first { $0.isSelectable(capability[$0.id]) }
        // A harness/dev relay (PACIFIC_RELAY_URL) is an EXPLICIT choice, exactly like one set by
        // hand — pin it for this session so discovery cannot adopt over it. The two-sim and
        // phone-sim harnesses depend on the launch env holding for the whole run; quietly
        // re-pointing them at the Global Arc's advertised relay mid-run IS the "message
        // vanished because we were talking to the wrong relay" bug, not a fix for it.
        let envRelay = ProcessInfo.processInfo.environment["PACIFIC_RELAY_URL"] ?? ""
        relayPinned = defaults.bool(forKey: Keys.relayPinned) || !envRelay.isEmpty
    }

    /// Load the REAL Arc list from the directory + merge with the always-on Global Arc and the
    /// user's self-hosted Arcs. Loud on failure; then discover each Arc's live capabilities.
    func refresh() async {
        // Liveness FIRST, and independent of the directory. Every Arc we already know about —
        // which always includes the Global one — can be pinged now; sequencing that behind a
        // catalog fetch meant one unreachable host left the UI reporting "Checking…" forever.
        await refreshCapabilities()
        loadState = .loading
        do {
            community = try await ArcCatalogClient.fetch()
                .filter { $0.id != ArcDirectory.globalDefault.id }
            rebuildDirectory()
            if let sel = selected, let fresh = directory.first(where: { $0.id == sel.id }) {
                selected = fresh
            }
            loadState = .loaded
        } catch {
            community = []
            rebuildDirectory()
            let msg = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
            loadState = .failed(msg)
        }
        await refreshCapabilities()
    }

    /// Discover every Arc's `/v1/arc` capability document concurrently and publish the results.
    func refreshCapabilities() async {
        let targets = directory.filter { $0.hasOrigin }
        await withTaskGroup(of: (String, ArcCapability?).self) { group in
            for arc in targets {
                let origin = arc.origin
                group.addTask { (arc.id, await ArcDiscovery.capabilities(origin: origin)) }
            }
            for await (id, cap) in group where cap != nil {
                capability[id] = cap
            }
        }
        adoptAdvertisedRelays()
        // Re-apply routing for the SELECTED Arc so an audience discovery just learned is recorded
        // without the user re-picking.
        if let sel = selected { applyRoute(sel) }
    }

    /// Take each Arc's ADVERTISED relay (`planes.relay.url`) over the one it was listed with.
    /// This closes a gap that made a broken relay unrecoverable from the device: `relayURL` was
    /// fixed at construction — baked into the catalog entry, or captured once when a self-hosted
    /// Arc was added — so an Arc added while its relay was down could never acquire one, and an
    /// Arc that MOVED its relay could never be followed. The Arc is the authority on where its
    /// own relay answers; discovery already asked, and the answer was being thrown away.
    ///
    /// Only adopted when the Arc says the plane is live and names a URL. A `live:false`, or a
    /// discovery that timed out, leaves the listed value alone — a 3s probe is not enough to
    /// unroute someone's messaging, and the picker's Relay pill already reports the liveness.
    /// A RETIRED relay is never adopted, however loudly an Arc advertises it. Discovery
    /// asking the Arc where its relay lives is right, but an Arc we still use for
    /// membership can keep advertising a relay plane this build has decommissioned —
    /// and adoption would then overwrite the shipped default on every launch, silently
    /// undoing the migration in `CoreService.init` and routing messaging back to a dead
    /// host. The retired list is the core's (`is_retired_relay_url`), so the app, the
    /// engine, and the migration can never disagree about which relays are dead.
    private func adoptAdvertisedRelays() {
        var changed = false
        for (id, cap) in capability {
            guard cap.relayLive, let url = cap.relayURL, !url.isEmpty, relayAdvertised[id] != url,
                  !isRetiredRelayUrl(url: url)
            else { continue }
            relayAdvertised[id] = url
            changed = true
        }
        guard changed else { return }
        rebuildDirectory()
        guard let sel = selected, let fresh = directory.first(where: { $0.id == sel.id }) else { return }
        selected = fresh
        if !relayPinned, !fresh.relayURL.isEmpty { setRelay(fresh.relayURL) }
    }

    /// Point messaging at a relay by hand and KEEP it there. Returns false if it isn't a URL.
    @discardableResult
    func pinRelay(_ raw: String) -> Bool {
        guard let url = ArcDiscovery.normalizeRelayURL(raw) else { return false }
        defaults.set(true, forKey: Keys.relayPinned)
        relayPinned = true
        setRelay(url)
        return true
    }

    /// Hand the relay back to the selected Arc.
    func unpinRelay() {
        defaults.set(false, forKey: Keys.relayPinned)
        relayPinned = false
        if let arc = selected, !arc.relayURL.isEmpty { setRelay(arc.relayURL) }
    }

    /// The ONE place `pacific.relayUrl` is written, so the published value and the routed value
    /// cannot drift.
    private func setRelay(_ url: String) {
        defaults.set(url, forKey: Keys.relayUrl)
        relayUrl = url
        // The core owns transport now, so a new relay is not routed until it is HANDED OVER.
        // Writing the default alone would leave messaging on the previous transport set —
        // precisely the "messages went somewhere else" failure this refactor exists to end.
        core?.applyRoutes()
    }

    /// Add (or update) a self-hosted / on-device Arc from a user-entered ORIGIN + optional serve
    /// key. Discovers `/v1/arc` so the caller sees its real planes. Returns the created Arc and its
    /// capability (nil = unreachable / not serving discovery), or nil if the origin is malformed.
    @discardableResult
    func addArc(name: String, origin: String, serveKey: String) async -> (arc: Arc, capability: ArcCapability?)? {
        guard let canonical = ArcDiscovery.normalizeOrigin(origin) else { return nil }
        let comps = URLComponents(string: canonical)
        let host = comps?.host ?? "arc"
        let id = "user.\(host):\(comps?.port ?? 0)"
        let cap = await ArcDiscovery.capabilities(origin: canonical)
        let cleanName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let cleanKey = serveKey.trimmingCharacters(in: .whitespacesAndNewlines)
        let arc = Arc(
            id: id, name: cleanName.isEmpty ? (cap?.name ?? host) : cleanName, region: "Self-hosted",
            // Same retired-relay rule as `adoptAdvertisedRelays`: a user-added Arc that
            // advertises a decommissioned relay contributes no relay at all, rather than
            // one that routes messaging nowhere.
            origin: canonical,
            relayURL: {
                guard cap?.relayLive == true, let u = cap?.relayURL, !u.isEmpty,
                      !isRetiredRelayUrl(url: u) else { return "" }
                return u
            }(),
            serveKey: cleanKey.isEmpty ? nil : cleanKey, userAdded: true
        )
        userArcs.removeAll { $0.id == id }
        userArcs.append(arc)
        persistUserArcs()
        if let cap { capability[id] = cap }
        rebuildDirectory()
        return (arc, cap)
    }

    /// Forget a user-added Arc; if it was the selection, clear routing and fall back to a usable Arc.
    func removeArc(_ arc: Arc) {
        guard arc.userAdded else { return }
        userArcs.removeAll { $0.id == arc.id }
        persistUserArcs()
        capability[arc.id] = nil
        rebuildDirectory()
        if selected?.id == arc.id {
            for k in [Keys.arcOrigin, Keys.arcServeKey, Keys.arcId] { defaults.removeObject(forKey: k) }
            selected = directory.first { $0.isSelectable(capability[$0.id]) }
        }
    }

    /// Pick an Arc. Writes ONE origin (fronting systems + discovery + membership) + the relay URL
    /// (messaging). A user Arc carries its own transitional serve key; a catalog Arc's membership
    /// rides the tether (§10) — joining seeds the Arc's roster, and the app mints a per-request
    /// signed credential (ArcCredential minter → `arcMemberCredential`) that the Arc's membership
    /// plane verifies. Both halves ship today.
    func select(_ arc: Arc) {
        guard arc.isSelectable(capability[arc.id]) else { return }
        let switching = defaults.string(forKey: Keys.arcId) != arc.id
        defaults.set(arc.id, forKey: Keys.arcId)

        // Messaging follows a relay Arc — unless the user pinned a relay by hand, which outranks
        // the Arc's own answer until they hand it back (`unpinRelay`).
        if !arc.relayURL.isEmpty, !relayPinned {
            setRelay(arc.relayURL)
        }
        // NOTE: the origin is NOT written here. `applyRoute` owns the whole route
        // (origin + audience) and runs at the end of this function and on every
        // discovery pass, so there is exactly one writer and no way to write half of it.
        if let key = arc.serveKey, !key.isEmpty {
            defaults.set(key, forKey: Keys.arcServeKey)         // a self-hosted Arc's transitional key
        } else if switching {
            defaults.removeObject(forKey: Keys.arcServeKey)     // a key belongs to ONE Arc
        }
        if switching {
            defaults.removeObject(forKey: Keys.arcSpaceId)      // an audience belongs to ONE Arc
        }
        selected = arc                                          // publish AFTER the keys are written
        applyRoute(arc)
    }

    /// Write the WHOLE route for `arc`, atomically — the ONE owner. Splitting the writes across
    /// `select()` and here would let a discovery pass refresh half a route and leave the other
    /// half stale.
    private func applyRoute(_ arc: Arc) {
        let cap = capability[arc.id]
        // The origin rides with the route rather than being written once at selection — a
        // discovery pass must be able to REPAIR a half-written route, not just refresh half of it.
        let before = routeFingerprint()
        if !arc.origin.isEmpty {
            defaults.set(arc.origin, forKey: Keys.arcOrigin)
        } else {
            defaults.removeObject(forKey: Keys.arcOrigin)
        }
        // The Arc's own space id — the AUDIENCE we sign for when minting a membership
        // credential (ICD §10). It rides the same path as the origin deliberately: it comes
        // from the one authoritative `/v1/arc` document, and it needs re-applying on every
        // discovery pass, not just at selection. An Arc joined before this key existed
        // therefore acquires it on the next launch with no user action.
        if let space = cap?.spaceId, !space.isEmpty {
            defaults.set(space, forKey: Keys.arcSpaceId)
        }
        // Announce ONLY on a real change, so a discovery pass that confirms the status quo does
        // not ask the Arc to prove membership again.
        if routeFingerprint() != before { routeEpoch &+= 1 }
    }

    // MARK: Directory assembly + persistence

    private func rebuildDirectory() {
        let listed = userArcs + ArcDirectory.nearestFirst([ArcDirectory.globalDefault] + community)
        directory = listed.map { arc in
            guard let advertised = relayAdvertised[arc.id], advertised != arc.relayURL else { return arc }
            var a = arc
            a.relayURL = advertised
            return a
        }
    }

    private func loadUserArcs() -> [Arc] {
        guard let data = defaults.data(forKey: Keys.userArcs),
              let arcs = try? JSONDecoder().decode([Arc].self, from: data) else { return [] }
        return arcs
    }

    private func persistUserArcs() {
        if let data = try? JSONEncoder().encode(userArcs) { defaults.set(data, forKey: Keys.userArcs) }
    }

    /// The two values that make up the route, as one comparable string. Used to detect a
    /// genuine route change rather than trusting either key alone.
    private func routeFingerprint() -> String {
        [defaults.string(forKey: Keys.arcOrigin) ?? "",
         defaults.string(forKey: Keys.arcSpaceId) ?? ""].joined(separator: "|")
    }

    private enum Keys {
        static let arcId            = "pacific.arcId"
        static let relayUrl         = "pacific.relayUrl"
        static let arcOrigin        = "pacific.arcOrigin"
        static let arcServeKey      = "pacific.arcServeKey"
        static let arcSpaceId       = "pacific.arcSpaceId"
        static let userArcs         = "pacific.userArcs"
        static let relayPinned      = "pacific.relayPinned"
    }
}
