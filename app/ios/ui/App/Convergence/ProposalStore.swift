//  ProposalStore.swift
//  The bridge from the on-device graph to the swipe deck — GAP-2 of the hammer journey.
//
//  The graph holds entities; minting puts GroupObjects in core. A PROPOSAL is
//  simply the gap between the two: "the graph holds a thing you have not accepted yet".
//
//  DERIVED, NOT STORED. Proposals are recomputed from (graph entities ⊖ minted things) rather
//  than persisted as their own queue. That is not laziness — it is the only version that
//  cannot go wrong:
//    • nothing to lose on relaunch, so a missed swipe is never a lost entity;
//    • no second source of truth to drift from the graph;
//    • a thing minted on another device simply stops being proposed here;
//    • rejecting is stateless by design — see `dismissed`.
//
//  Honesty contract (house rule): a proposal is a real question about real graph data.
//  Nothing is invented to fill an empty deck; an empty graph proposes nothing.

import Foundation
import PacificCore
import ArcResolverKit
import UserNotifications

@MainActor
final class ProposalStore: ObservableObject {

    /// The notification category carrying Accept / Dismiss. Registered once at launch.
    static let mintCategory = "MINT"
    static let acceptAction = "MINT_ACCEPT"
    static let dismissAction = "MINT_DISMISS"

    /// Register the actionable category. Call once, before any mint banner is raised.
    static func registerNotificationCategory() {
        let accept = UNNotificationAction(identifier: acceptAction, title: "Track it",
                                          options: [])
        let dismiss = UNNotificationAction(identifier: dismissAction, title: "Not now",
                                           options: [])
        UNUserNotificationCenter.current().setNotificationCategories([
            UNNotificationCategory(identifier: mintCategory,
                                   actions: [accept, dismiss],
                                   intentIdentifiers: [],
                                   options: [])
        ])
    }

    /// Apply a decision that arrived from a notification action rather than a swipe.
    ///
    /// SAFETY: this can run while the app is backgrounded or freshly launched into the
    /// background, and `.accept` performs a REAL MLS write (`mintThing`). So it resolves the
    /// id against the CURRENT pending set and no-ops if the proposal is gone (already decided
    /// on another surface) — an action must never mint twice, and must never mint something
    /// the graph no longer proposes.
    func decide(proposalId: String, accept: Bool, core: CoreService, graph: GraphService? = nil) {
        guard let p = pending.first(where: { $0.id == proposalId }) else {
            NSLog("[proposal] notification action for \(proposalId) — no longer pending, ignoring")
            return
        }
        decide(p, accept ? .accept : .reject, core: core, graph: graph)
    }

    /// Questions awaiting a swipe, newest observation first.
    @Published private(set) var pending: [Proposal] = []

    /// Entity ids rejected THIS SESSION. Held in memory on purpose: a reject says "not worth
    /// tracking right now", not "never show me this again". A durable blocklist would need
    /// its own truth and its own way to be wrong; re-proposing after a relaunch is the
    /// cheaper mistake.
    private var dismissed: Set<String> = []

    /// Proposal ids we've already raised a notification for. DURABLE, because the in-memory
    /// version had a hole that silenced the product — every relaunch re-"seeded" the pending
    /// set as old news, so a proposal that arrived while the app was away (on a later launch
    /// or in a background window) was never announced at all. Durability makes the rule
    /// honest: announced means ANNOUNCED once, ever — not "existed before the most recent launch".
    private var announced: Set<String> = []
    private let announcedURL: URL

    /// Proposals a SYSTEM sweep injected (`propose` — today the Music System's venues)
    /// rather than the graph deriving. The ONE exception to DERIVED-NOT-STORED: their
    /// source is a rolling feed window (a venue drops out when its listings pass), so
    /// the proposal itself is the durable record until it is DECIDED — persisted beside
    /// `announced`, reloaded at init, retired on decide or when its name gets minted.
    private var sourced: [Proposal] = []
    private let sourcedURL: URL

    /// `directory` is injectable so unit tests get hermetic persistence (the default is
    /// the app's one Application Support/pacific).
    init(directory: URL? = nil) {
        let dir = directory ?? FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        announcedURL = dir.appendingPathComponent("proposals-announced.json")
        sourcedURL = dir.appendingPathComponent("proposals-sourced.json")
        if let data = try? Data(contentsOf: announcedURL),
           let ids = try? JSONDecoder().decode(Set<String>.self, from: data) {
            announced = ids
        }
        if let data = try? Data(contentsOf: sourcedURL),
           let kept = try? JSONDecoder().decode([Proposal].self, from: data) {
            sourced = kept
            // Undecided sourced questions survive relaunch — straight back on the deck
            // (a rebuild will fold the graph's own in around them).
            pending = kept.sorted { $0.provenance.occurredAt > $1.provenance.occurredAt }
        }
    }

    /// The graph types a swipe can promote into a GroupObject, and the LIFE type the
    /// card renders — the 8-category retarget (1 Aug): every graph type proposes except
    /// EVENT, which needs no mint — a commitment in the graph lands directly in the
    /// EVENTS feed (GraphEventsSource), so a card asking to "track" it would author
    /// nothing the feed doesn't already show. `Thing` stays for entities written under
    /// the pre-retarget roster.
    ///
    /// A graph Place mints UNPLACED (`mintPlace(placed: false)` exists for exactly
    /// this) — a graph node carries no coordinate; `placeSetLocation` fills it in later.
    private static let proposable: [String: LifeType] = [
        "Person": .person, "Group": .group, "Place": .place,
        "Inventory": .inventory, "Skill": .skill, "Trade": .trade, "Job": .job,
        "Thing": .thing,
    ]

    /// Recompute from the graph. Cheap and idempotent — call it after every graph change.
    ///
    /// `mintedNames` are the display names of objects that already exist as GroupObjects
    /// (any kind). Matching is by NAME, case-insensitively, because that is the identity
    /// the user actually sees; projected nodes are excluded exactly (`isObject` — their
    /// id IS an object id, nothing to propose).
    func rebuild(resolved: [ArcEntity], mintedNames: Set<String>) {
        // A sourced proposal whose name now exists as an object (minted here, from the
        // deck, or on another device) is ANSWERED — retire the durable record so it can
        // never resurrect.
        let kept = sourced.count
        sourced.removeAll { mintedNames.contains($0.subject.name.lowercased()) }
        if sourced.count != kept { persistSourced() }

        // Newest observation first, so the freshest thing is the first card.
        let derived = resolved
            .filter { !$0.isObject }
            .filter { Self.proposable[$0.entityType] != nil }
            .filter { !dismissed.contains($0.id) }
            .filter { !mintedNames.contains(Self.displayName($0).lowercased()) }
            .map { entity in
                let name = Self.displayName(entity)
                let occurred = Int(entity.createdAt ?? 0)
                return Proposal(
                    id: entity.id,
                    kind: .mint,
                    subject: .init(name: name,
                                   type: Self.proposable[entity.entityType] ?? .thing,
                                   summary: entity.summary ?? "",
                                   snippet: entity.summary ?? name,
                                   highlightStart: nil,
                                   highlightLength: nil),
                    target: nil,
                    rationale: "In your graph; not tracked yet.",
                    // Every untracked graph entity reaches the deck today. Once the
                    // classifier is trained, this is the gate that auto-applies the confident
                    // band and leaves only the uncertain middle to swipe.
                    confidence: 0.5,
                    provenance: .init(episodeId: "",
                                      source: .chat,
                                      occurredAt: occurred,
                                      label: "from your graph"),
                    features: ["name_len": Double(name.count),
                               "has_summary": (entity.summary?.isEmpty == false) ? 1 : 0])
            }
        // The graph's questions plus the sourced ones still open, one deck, newest first.
        pending = (derived + sourced.filter { !dismissed.contains($0.id) })
            .sorted { ($0.provenance.occurredAt) > ($1.provenance.occurredAt) }
        Trace.shared.log("proposals",
                         "rebuilt: \(pending.count) pending from \(resolved.count) graph entities + \(sourced.count) sourced (\(mintedNames.count) already minted)",
                         ok: !resolved.isEmpty || !sourced.isEmpty)
        announceNewMints()
    }

    /// Inject proposals from a SYSTEM sweep (the Music System's venues). Deduped by id
    /// against everything already sourced or pending; each NEW one is announced as its
    /// OWN actionable MINT banner — accept mints without opening the app — but capped at
    /// `announceLimit` per call: a first sync of an area surfaces dozens of venues, and
    /// the overflow lands in the deck silently, marked announced so no later rebuild
    /// banners it either.
    func propose(_ new: [Proposal], announceLimit: Int = 5) {
        let known = Set(sourced.map(\.id)).union(pending.map(\.id))
        let fresh = new.filter { !known.contains($0.id) && !dismissed.contains($0.id) }
        guard !fresh.isEmpty else { return }
        sourced.append(contentsOf: fresh)
        persistSourced()
        pending = (pending + fresh)
            .sorted { $0.provenance.occurredAt > $1.provenance.occurredAt }

        let banners = fresh.filter { !announced.contains($0.id) }.prefix(announceLimit)
        announced.formUnion(fresh.map(\.id))
        if let data = try? JSONEncoder().encode(announced) {
            try? data.write(to: announcedURL, options: .atomic)
        }
        Trace.shared.log("notify",
                         "sourced \(fresh.count) proposals, \(banners.count) banners: \(fresh.prefix(3).map(\.subject.name).joined(separator: ", "))")
        for p in banners {
            let c = UNMutableNotificationContent()
            c.title = p.subject.name
            c.body = p.subject.summary.isEmpty ? p.rationale : p.subject.summary
            c.sound = .default
            c.categoryIdentifier = Self.mintCategory
            c.userInfo = ["proposal": p.id]
            UNUserNotificationCenter.current().add(
                UNNotificationRequest(identifier: "mint-\(p.id)", content: c, trigger: nil))
        }
    }

    private func persistSourced() {
        if let data = try? JSONEncoder().encode(sourced) {
            try? data.write(to: sourcedURL, options: .atomic)
        }
    }

    #if DEBUG
    /// Launch-arg state reset for the seeded venue-proposal UI test: drop every sourced
    /// proposal (a stale one from a prior run would sit on top of the deck and take the
    /// test's ✓ instead of the seed). Never a host-side `rm` — the launch owns its state.
    func resetSourced() {
        sourced = []
        pending = []
        persistSourced()
    }
    #endif

    /// Raise ONE batched local notification for proposals that have never been announced —
    /// regardless of which launch or background window produced them. The old launch-backlog
    /// seeding silenced everything discovered while the app was away; the durable
    /// `announced` set is what keeps that from happening again.
    private func announceNewMints() {
        let fresh = pending.filter { !announced.contains($0.id) }
        // Nothing new ⇒ `formUnion` is a no-op by definition, so the encode+write
        // would be pure churn on the steady path (rebuild runs on every cache refresh,
        // every epoch bump and every tray open).
        guard !fresh.isEmpty else { return }
        announced.formUnion(pending.map(\.id))
        if let data = try? JSONEncoder().encode(announced) {
            try? data.write(to: announcedURL, options: .atomic)
        }
        Trace.shared.log("notify", "banner for \(fresh.count): \(fresh.prefix(3).map(\.subject.name).joined(separator: ", "))")

        let c = UNMutableNotificationContent()
        c.title = fresh.count == 1 ? "Something to track"
                                   : "\(fresh.count) things to track"
        c.body = fresh.prefix(3).map { $0.subject.name }.joined(separator: ", ")
        c.sound = .default
        // ACTIONABLE. Without a category the banner was a dead end: it carried no userInfo,
        // so `NotifPresenter` had nothing to route on and a tap opened the launch tab and did
        // nothing. A suggestion you cannot accept from the place you are told about it
        // is not a suggestion, it is an interruption.
        c.categoryIdentifier = Self.mintCategory
        c.userInfo = ["proposal": fresh[0].id]
        UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: "mint-\(fresh[0].id)", content: c, trigger: nil))
    }

    /// Record a swipe. ACCEPT mints the kind the card proposed — a person becomes an
    /// individual Group, an org an organisation Group, a car a Thing — and the graph
    /// FOLLOWS: the ghost's facts and note-provenance re-point onto the minted object
    /// (`GraphService.promote`), so one identity remains where two would drift. The
    /// minted object is an MLS group, so sharing it is `AddPeopleSheet` — no new
    /// machinery. REJECT only stops it being asked again this session; the entity
    /// stays in the graph either way, because the graph records what was OBSERVED
    /// and a reject is a statement about tracking, not about truth.
    func decide(_ proposal: Proposal, _ decision: ProposalDecision.Decision,
                core: CoreService, graph: GraphService? = nil) {
        switch decision {
        case .accept:
            let s = proposal.subject
            let minted: (id: String?, type: String)
            switch s.type {
            case .person:
                minted = (core.mintOrg(name: s.name, shape: "individual", purpose: s.summary), "Person")
            case .group:
                minted = (core.mintOrg(name: s.name, shape: "organisation", purpose: s.summary), "Group")
            case .place:
                // Unplaced by design — text carries no coordinate; the fix arrives later.
                // Except a SOURCED venue proposal (VenueProposer), which can carry the
                // venue's real fix in its frozen features: the mint is the same, then the
                // location lands as the separate owner-sequenced op it always is.
                minted = (core.mintPlace(name: s.name, descriptor: s.summary,
                                         placed: false, lat: 0, lng: 0), "Place")
                if let id = minted.id,
                   let lat = proposal.features["lat"], let lng = proposal.features["lng"],
                   !core.setPlaceLocation(id, lat: lat, lng: lng) {
                    NSLog("[proposal] placeSetLocation FAILED for %@: %@",
                          s.name, core.error ?? "no error set")
                    Trace.shared.log("proposals", "placeSetLocation FAILED for \(s.name)", ok: false)
                }
            case .inventory, .skill:
                // Held — the HAS posture is what the text stated (holding a thing, having
                // a capability), so accepting records it.
                let id = core.mintThing(name: s.name, descriptor: s.summary)
                if let id { core.setPosture(id, posture: "has") }
                minted = (id, s.type == .skill ? "Skill" : "Inventory")
            case .trade, .job:
                // Moving — but the DIRECTION (wants vs offers) lives in the facts, not the
                // entity, so accepting mints the thing and leaves the posture to the one
                // surface that authors postures deliberately: the ListingEditor. Never
                // guess a direction the user didn't state to us.
                minted = (core.mintThing(name: s.name, descriptor: s.summary),
                          s.type == .job ? "Job" : "Trade")
            default:
                minted = (core.mintThing(name: s.name, descriptor: s.summary), "Thing")
            }
            if minted.id == nil {
                // LOUD — an accepted suggestion that authored nothing is a fault, not a shrug.
                NSLog("[proposal] ACCEPT of %@ minted NOTHING: %@", s.name, core.error ?? "no error set")
                Trace.shared.log("proposals", "mint FAILED for \(s.name): \(core.error ?? "?")", ok: false)
            }
            // A graph-derived proposal's id IS its graph ghost — promotion re-points the
            // ghost's facts onto the minted object. A .music/.art (sourced) proposal has
            // no ghost to rehome; its Place reaches the graph through the object
            // projection (projectObjects on the next objects change), like every other mint.
            if let objectId = minted.id, let graph,
               ![.music, .art].contains(proposal.provenance.source) {
                let (ghost, type, name) = (proposal.id, minted.type, s.name)
                Task { await graph.promote(ghostId: ghost, toObject: objectId, type: type, name: name) }
            }
        case .reject:
            dismissed.insert(proposal.id)
        }
        pending.removeAll { $0.id == proposal.id }
        // A decided sourced proposal is DONE either way — its durable record retires so
        // it cannot come back at the next launch (its System's own seen-set stops the
        // next sweep re-proposing it).
        if sourced.contains(where: { $0.id == proposal.id }) {
            sourced.removeAll { $0.id == proposal.id }
            persistSourced()
        }
    }

    /// `properties.name` when the resolver set one, else the embedded label.
    private static func displayName(_ e: ArcEntity) -> String {
        e.name.isEmpty ? e.label : e.name
    }
}
