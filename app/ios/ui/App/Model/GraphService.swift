//  GraphService.swift
//  The one on-device bi-temporal knowledge graph, live in the app, backed by the arc-resolver
//  crate's graph handle. Nodes (Person/Place/Event/Project/Plant/Animal/Thing/Group) + their
//  typed bi-temporal facts, with as-of queries + semantic retrieval, serving every LIFE-tab read.
//
//  Per the Ratify design note (§2) the graph is a DERIVED READ-INDEX, not a second source of
//  truth: the per-object delta logs remain authoritative, and the graph is their deterministic
//  projection (`projectObjects`). The file lives on the device and the only embedder it uses is
//  the on-device union embedder, so nothing here leaves the phone. Instantiated ONCE at the
//  composition root, like `CoreService` / `OutputStore`.

import Foundation
import ArcResolverKit
import LodeDBCore     // LodeEmbedder / NLEmbedder
import PacificStore   // UnionEmbedder.makeUnion() — the same on-device embedder the union store uses

@MainActor
final class GraphService: ObservableObject {
    /// Flips true once the graph is open — the LIFE tab / note search gate on it.
    @Published private(set) var isReady = false
    /// Bumped after every write that changed the graph, so views (LIFE) can refresh off one signal.
    @Published private(set) var epoch = 0
    /// The last open error, surfaced rather than silently swallowed (no-fallback contract).
    @Published private(set) var lastError: String?
    /// The LIFE tab's read cache — every resolved entity, refreshed off `epoch`. Views read
    /// this synchronously (via `GraphLifeSource`); the async resolver reads fill it.
    @Published private(set) var cachedEntities: [ArcEntity] = []

    /// The live engine. Thread-safe (writes off-main, reads concurrent). nil until `open()`.
    private(set) var resolver: ArcResolver?
    /// OPTIONAL, and it is the launch that made it so — see `init`. Nil means the
    /// on-device model is not installed; the resolver then never opens, rather than
    /// opening around a vector nobody produced.
    private let embedder: ArcEmbedder?
    private let graphPath: String

    init() {
        let dir = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        // The resolver-owned graph. Distinct file from the legacy bare-LodeGraph `graph.lode`;
        // one writer owns it (the object projection) and serves reads.
        graphPath = dir.appendingPathComponent("resolver.lode").path

        // Same explicit-no-fallback contract as the union store: the on-device embedder must
        // come up (Apple NL today; BGE once provisioned). A missing embedder is a real fault
        // — it is NOT, however, a reason to kill the process. This ran inside
        // `WallFlowersApp.init()` (GraphService is built there), and Apple's NL assets are an OS
        // download a freshly created simulator has never made, so a `fatalError` here meant
        // the app would not launch at all on a fresh device. The resolver stays CLOSED and
        // says so in `lastError`; nothing fabricates a vector.
        do {
            embedder = try LodeArcEmbedder(UnionEmbedder.makeUnion())
            embedderFault = nil
        } catch {
            embedder = nil
            embedderFault = "\(error)"
            NSLog("[graph] EMBEDDER UNAVAILABLE — %@. The on-device graph stays closed: "
                + "no LIFE projection. Everything else runs.", "\(error)")
        }
    }

    /// Why there is no embedder, when there is none. Surfaced through `lastError`
    /// on every `open`, so the reason is visible where the resolver's state is.
    private let embedderFault: String?

    /// Open the graph file. Called from the launch task; a second call while the graph is open
    /// is a no-op rather than a second handle on the same file, and a call after a failed open
    /// tries again.
    func open() {
        guard resolver == nil else { return }
        let cfg = ArcResolverConfig(graphPath: graphPath, indexFacts: true)
        guard let embedder else {
            resolver = nil
            isReady = false
            lastError = "no on-device embedder — \(embedderFault ?? "unknown"). "
                      + "The graph resolver cannot open without one."
            NSLog("GraphService: resolver NOT opened — there is no embedder on this device")
            return
        }
        do {
            resolver = try ArcResolver(config: cfg, embedder: embedder)
            isReady = true
            lastError = nil
            lastProjected = nil   // a freshly opened graph always gets one full projection pass
            Task { await refreshEntities() }   // surface anything already in the graph
        } catch {
            isReady = false
            lastError = "\(error)"
            NSLog("GraphService: graph open failed — \(error)")
        }
    }

    /// Reload the LIFE read cache from the graph (all entities). Off-main; publishes on main.
    func refreshEntities() async {
        let all = await entities(ofType: nil)
        cachedEntities = all
    }

    // MARK: - write (delta fold → graph): the ratified-delta projection

    /// A projection pass is in flight — triggers fire on the sync cadence, and two
    /// passes interleaving would race the reconcile for no benefit.
    private var projecting = false
    /// Fingerprint of the last snapshot that REACHED the graph. A trigger whose
    /// snapshot matches it is a steady-state tick and skips the reconcile entirely;
    /// nil until the graph opens, so a freshly opened graph always gets one full pass.
    private var lastProjected: String?

    /// Reconcile the graph against the COMPLETE snapshot of minted GroupObjects
    /// (groups + rosters + affiliations, projects, things, places). Deterministic and
    /// idempotent — safe to call on every sync tick; a steady-state run writes nothing
    /// and, via the fingerprint, no longer even reaches the reconcile.
    /// This is the projection the header promised: the delta logs stay authoritative,
    /// the graph derives from them.
    func projectObjects(_ snapshots: [ArcObjectSnapshot]) async {
        guard let r = resolver else { return }
        if projecting { return }
        projecting = true
        defer { projecting = false }
        let fp = Self.fingerprint(snapshots)
        if fp == lastProjected { return }
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let stats: ArcProjectStats? = await Task.detached(priority: .utility) {
            do { return try r.projectObjects(snapshots, nowMs: now) }
            catch { NSLog("GraphService.projectObjects failed: \(error)"); return nil }
        }.value
        guard let s = stats else { return }   // a failed write retries on the next trigger
        lastProjected = fp
        if s.changedAnything {
            NSLog("GraphService.project → \(s.entitiesWritten) entities, +\(s.factsAdded)/−\(s.factsInvalidated) facts")
            epoch += 1
            await refreshEntities()
        }
    }

    /// Stable, order-sensitive digest of a snapshot — equal strings ⇔ nothing the
    /// projector reads has changed. Field order mirrors `ArcObjectSnapshot`.
    nonisolated static func fingerprint(_ snaps: [ArcObjectSnapshot]) -> String {
        var s = ""
        for o in snaps {
            s += o.id; s += "\u{1F}"; s += o.kind; s += "\u{1F}"; s += o.name
            s += "\u{1F}"; s += o.shape ?? ""
            for m in o.members { s += "\u{1E}"; s += m.space; s += "\u{1F}"; s += m.name ?? "" }
            for a in o.affiliations {
                s += "\u{1E}"; s += a.peer; s += "\u{1F}"; s += a.rel; s += "\u{1F}"
                s += a.name; s += "\u{1F}"; s += a.tether; s += "\u{1F}"; s += String(a.at)
            }
            if let v = o.venue {
                s += "\u{1E}"; s += v.place; s += "\u{1F}"; s += v.name; s += "\u{1F}"; s += String(v.at)
            }
            s += "\u{1E}"; s += String(o.at ?? 0); s += "\n"
        }
        return s
    }

    /// PROMOTION: a node that is not an object yet — a ghost an earlier build lifted from
    /// text — was just minted as a real GroupObject. The graph follows: facts and
    /// note-provenance re-point onto the object's node, the ghost is removed, one identity
    /// remains.
    func promote(ghostId: String, toObject objectId: String, type: String, name: String) async {
        guard let r = resolver else { return }
        let moved: Int? = await Task.detached(priority: .utility) {
            do { return try r.rehome(ghostId: ghostId, objectId: objectId, objectType: type, name: name) }
            catch { NSLog("GraphService.promote failed: \(error)"); return nil }
        }.value
        if let moved {
            NSLog("GraphService.promote \(name) → \(objectId.prefix(8))…, \(moved) facts moved")
            epoch += 1
            await refreshEntities()
        }
    }

    // MARK: - reads (LIFE tab)

    /// Entities of a type (nil = all), off-main. The LIFE tab's source.
    func entities(ofType type: String? = nil, asOf: Int64? = nil) async -> [ArcEntity] {
        guard let r = resolver else { return [] }
        return await Task.detached(priority: .userInitiated) {
            (try? r.entities(type: type, asOf: asOf)) ?? []
        }.value
    }

    /// One graph entity by id — the node a projected edge points at.
    func entity(id: String) async -> ArcEntity? {
        guard let r = resolver else { return nil }
        return await Task.detached(priority: .userInitiated) { try? r.getEntity(id: id) }.value
    }

    /// The Place an Event happens at — the live `happens_at` edge off the event's node.
    /// Nil when no venue is set (or the graph is still opening).
    func happensAt(eventId: String) async -> ArcFact? {
        guard let r = resolver else { return nil }
        return await Task.detached(priority: .userInitiated) {
            do { return try r.neighbors(id: eventId, direction: "out", relation: "happens_at").first }
            catch { NSLog("GraphService.happensAt failed: \(error)"); return nil }
        }.value
    }

    /// Semantic entity search — the graph's entry point for a free-text query. Off-main
    /// (embeds the query on device). Returns `(score, entity)` best-first.
    func semanticEntities(query: String, k: Int = 10, asOf: Int64? = nil)
        async -> [(score: Float, entity: ArcEntity)] {
        guard let r = resolver, !query.trimmingCharacters(in: .whitespaces).isEmpty else { return [] }
        return await Task.detached(priority: .userInitiated) {
            do { return try r.semanticEntities(query: query, k: k, asOf: asOf) }
            catch { NSLog("GraphService.semanticEntities failed: \(error)"); return [] }
        }.value
    }

    // MARK: - provenance (entity -> the notes that produced it)

    /// `entityId -> [noteId]`, inverted from the graph's episode index. An episode's id is the
    /// UUID of the note it was read from, so these ids ARE note ids. Nothing on the device
    /// writes episodes, so the index holds only what an earlier build recorded, and for a
    /// graph created since it is empty.
    ///
    /// Cached: rebuilt only when `epoch` moves, never per keystroke — the underlying call is
    /// one query per episode.
    private var provenance: [String: [String]] = [:]
    private var provenanceEpoch = -1

    /// The notes whose episodes carry facts about `entityID`, newest episode first.
    func noteIDs(forEntity entityID: String) async -> [String] {
        await refreshProvenanceIfStale()
        return provenance[entityID] ?? []
    }

    /// Rebuild the entity -> notes map if the graph has changed since the last build.
    private func refreshProvenanceIfStale() async {
        guard let r = resolver, provenanceEpoch != epoch else { return }
        let rows: [ArcEpisodeProvenance] = await Task.detached(priority: .userInitiated) {
            do { return try r.episodeIndex() }
            catch { NSLog("GraphService.episodeIndex failed: \(error)"); return [] }
        }.value
        // Newest episode first, so a note that mentions an entity most recently ranks first.
        var map: [String: [String]] = [:]
        for row in rows.sorted(by: { $0.occurredAt > $1.occurredAt }) where row.source == "note" {
            for entity in row.entities {
                map[entity, default: []].append(row.episodeId)
            }
        }
        provenance = map
        provenanceEpoch = epoch
    }

    func stats() async -> ArcStats? {
        guard let r = resolver else { return nil }
        return await Task.detached { try? r.stats() }.value
    }
}

/// Adapts a LodeDBCore `LodeEmbedder` (the app's UnionEmbedder) to the resolver's `ArcEmbedder`.
/// Dimension is probed once at build so the graph's vector width matches the model.
private final class LodeArcEmbedder: ArcEmbedder {
    private let inner: LodeEmbedder
    let dimension: Int

    init(_ inner: LodeEmbedder) throws {
        self.inner = inner
        let probe = try inner.embed(texts: ["dimension probe"])
        guard let dim = probe.first?.count, dim > 0 else {
            throw NSError(domain: "GraphService", code: 1,
                          userInfo: [NSLocalizedDescriptionKey: "embedder produced no dimension"])
        }
        dimension = dim
    }

    func embed(_ texts: [String]) -> [[Float]]? {
        try? inner.embed(texts: texts)
    }
}
