//  ArcResolver.swift
//  The Swift face of the on-device arc-resolver: one handle owns the graph and serves BOTH
//  the deterministic object projection (projectObjects, rehome) and every LIFE-tab read.
//  Embeddings come from the platform (the app's UnionEmbedder) via a C callback; no language
//  model runs and nothing here reaches the network. All FFI payloads are JSON.

import Foundation
import ArcResolverFFI

// MARK: - Public model

/// A resolved graph entity — a GroupObject candidate the LIFE tab renders. `entity_type` is one
/// of the SEVEN_TYPES (Person/Place/Event/Project/Plant/Animal/Thing) plus Group.
public struct ArcEntity: Identifiable, Hashable {
    public let id: String
    public let entityType: String
    public let label: String
    /// Human name: `properties.name` if present, else the label.
    public let name: String
    public let summary: String?
    /// Where the node came from: `"object"` = projected from a minted GroupObject
    /// (durable truth, id = the object id); nil = a node lifted out of note or chat text by a
    /// build from before text extraction was removed, which an existing graph may still hold.
    public let source: String?
    public let validAt: Int64?
    public let invalidAt: Int64?
    public let createdAt: Int64?

    /// A node backed by a real minted GroupObject (vs a name lifted from text).
    public var isObject: Bool { source == "object" }

    public static func == (a: ArcEntity, b: ArcEntity) -> Bool { a.id == b.id }
    public func hash(into h: inout Hasher) { h.combine(id) }
}

/// One typed, directed, bi-temporal fact. The edge an Event hangs its participants on, and
/// what the LIFE filter intersects over — a participant is always exactly one hop away.
public struct ArcFact: Identifiable, Hashable {
    public let id: String
    public let src: String
    public let relation: String
    public let dst: String
    public let fact: String
    public let validAt: Int64?
    public let invalidAt: Int64?
    public let createdAt: Int64?

    /// Live on both axes — not superseded, not ended.
    public var isLive: Bool { invalidAt == nil }

    public static func == (a: ArcFact, b: ArcFact) -> Bool { a.id == b.id }
    public func hash(into h: inout Hasher) { h.combine(id) }
}

public struct ArcStats { public let entities: Int; public let facts: Int; public let indexedDocuments: Int }

/// One minted GroupObject, as CoreService reads it after a fold — the input row of the
/// object projection. `id` is the object id (hex); `members` are roster space ids;
/// `affiliations` mirror the folded `group.setAffiliation` records.
public struct ArcObjectSnapshot {
    public let id: String
    public let kind: String
    public let name: String
    public let shape: String?
    public let members: [(space: String, name: String?)]
    public let affiliations: [(peer: String, rel: String, name: String, tether: String, at: Int64)]
    /// An Event's venue reference — projected as the `happens_at` edge onto the Place.
    public let venue: (place: String, name: String, at: Int64)?
    public let at: Int64?

    public init(id: String, kind: String, name: String, shape: String? = nil,
                members: [(space: String, name: String?)] = [],
                affiliations: [(peer: String, rel: String, name: String, tether: String, at: Int64)] = [],
                venue: (place: String, name: String, at: Int64)? = nil,
                at: Int64? = nil) {
        self.id = id
        self.kind = kind
        self.name = name
        self.shape = shape
        self.members = members
        self.affiliations = affiliations
        self.venue = venue
        self.at = at
    }
}

/// What one projection run did. Steady state is all zeros except the `unchanged` pair.
public struct ArcProjectStats {
    public let entitiesWritten: Int
    public let entitiesUnchanged: Int
    public let factsAdded: Int
    public let factsUnchanged: Int
    public let factsInvalidated: Int
    public let skippedObjects: Int

    /// Did this run move the graph at all? (The epoch only needs bumping when it did.)
    public var changedAnything: Bool { entitiesWritten + factsAdded + factsInvalidated > 0 }
}

/// One episode's provenance: which entities it produced facts about. `episodeId` is the
/// caller-stable id the episode was written under, so for notes it IS the note's UUID.
public struct ArcEpisodeProvenance: Hashable, Sendable {
    public let episodeId: String
    public let source: String
    public let occurredAt: Int64
    public let entities: [String]
}

public enum ArcResolverError: Error, CustomStringConvertible {
    case open(String)
    case call(String)
    case badResponse(String)
    public var description: String {
        switch self {
        case .open(let m): return "arc-resolver open failed: \(m)"
        case .call(let m): return "arc-resolver: \(m)"
        case .badResponse(let m): return "arc-resolver bad response: \(m)"
        }
    }
}

/// The platform embedder the resolver calls into (the app supplies its UnionEmbedder here).
public protocol ArcEmbedder: AnyObject {
    var dimension: Int { get }
    /// Row-per-text embeddings. Return nil on failure; the resolver treats that as an embed error.
    func embed(_ texts: [String]) -> [[Float]]?
}

// MARK: - Config

public struct ArcResolverConfig {
    public var graphPath: String
    public var indexFacts: Bool

    public init(graphPath: String, indexFacts: Bool = true) {
        self.graphPath = graphPath
        self.indexFacts = indexFacts
    }
}

// MARK: - Embed bridge

/// Retained context handed to the C callback. Owns the embedder so it outlives the handle.
private final class EmbedBridge {
    let embedder: ArcEmbedder
    init(_ e: ArcEmbedder) { embedder = e }
}

/// The `@convention(c)` thunk (no captures): unpack `ctx` -> bridge -> embedder, fill `out`
/// row-major with `count * dim` floats. Non-zero return signals an embedding failure to Rust.
private func arcEmbedThunk(_ ctx: UnsafeMutableRawPointer?,
                           _ texts: UnsafePointer<UnsafePointer<CChar>?>?,
                           _ count: Int,
                           _ dim: Int,
                           _ out: UnsafeMutablePointer<Float>?) -> Int32 {
    guard let ctx = ctx, let out = out else { return 1 }
    let bridge = Unmanaged<EmbedBridge>.fromOpaque(ctx).takeUnretainedValue()
    if count == 0 { return 0 }
    var strings: [String] = []
    strings.reserveCapacity(count)
    if let texts = texts {
        for i in 0..<count { strings.append(texts[i].map { String(cString: $0) } ?? "") }
    } else {
        return 1
    }
    guard let vecs = bridge.embedder.embed(strings), vecs.count == count else { return 2 }
    for (i, v) in vecs.enumerated() {
        guard v.count == dim else { return 3 }
        v.withUnsafeBufferPointer { buf in
            guard let base = buf.baseAddress else { return }
            (out + i * dim).update(from: base, count: dim)
        }
    }
    return 0
}

// MARK: - ArcResolver

/// Thread-safe to share across tasks: every FFI call serializes on `queue` (concurrent reads,
/// barrier writes), the handle + bridge pointers are stable for the object's life, and the
/// injected embedder must itself be safe to call from the resolver's thread.
public final class ArcResolver: @unchecked Sendable {
    private let handle: OpaquePointer
    private let bridge: Unmanaged<EmbedBridge>
    private let dim: Int
    // Concurrent reads; projectObjects and rehome are barriers (they mutate). Matches the C-ABI
    // contract: "writes mutate; reads are shared" — serialize writes, allow concurrent reads.
    private let queue = DispatchQueue(label: "io.wallflowers.arc-resolver", attributes: .concurrent)

    public init(config: ArcResolverConfig, embedder: ArcEmbedder) throws {
        let dim = embedder.dimension
        precondition(dim > 0, "ArcResolver: embedder dimension must be > 0")
        self.dim = dim

        let bridge = Unmanaged.passRetained(EmbedBridge(embedder))
        self.bridge = bridge

        let cfg: [String: Any] = [
            "graph_path": config.graphPath,
            "vector_dim": dim,
            "index_facts": config.indexFacts,
        ]
        guard let cfgData = try? JSONSerialization.data(withJSONObject: cfg),
              let cfgStr = String(data: cfgData, encoding: .utf8) else {
            bridge.release()
            throw ArcResolverError.open("could not encode config")
        }

        var errPtr: UnsafeMutablePointer<CChar>? = nil
        let h = cfgStr.withCString { cfgC -> OpaquePointer? in
            arc_resolver_open(cfgC, arcEmbedThunk, bridge.toOpaque(), dim, &errPtr)
        }
        guard let h = h else {
            let msg = ArcResolver.takeString(errPtr) ?? "unknown error"
            bridge.release()
            throw ArcResolverError.open(msg)
        }
        self.handle = h
    }

    deinit {
        arc_resolver_free(handle)
        bridge.release()
    }

    // MARK: write

    /// The ratified-delta projection: reconcile the graph against the COMPLETE snapshot of
    /// the app's minted GroupObjects. Deterministic: entities keyed by
    /// object id, `member_of` / `affiliated_with` facts keyed by (src, relation, dst), and a
    /// projected relationship absent from the snapshot is invalidated at `nowMs` — history
    /// stays queryable. Re-running an unchanged snapshot writes nothing.
    @discardableResult
    public func projectObjects(_ objects: [ArcObjectSnapshot], nowMs: Int64) throws -> ArcProjectStats {
        let rows: [[String: Any]] = objects.map { o in
            var row: [String: Any] = ["id": o.id, "kind": o.kind, "name": o.name]
            if let shape = o.shape { row["shape"] = shape }
            if let at = o.at { row["at"] = at }
            if !o.members.isEmpty {
                row["members"] = o.members.map { m -> [String: Any] in
                    var r: [String: Any] = ["space": m.space]
                    if let n = m.name { r["name"] = n }
                    return r
                }
            }
            if !o.affiliations.isEmpty {
                row["affiliations"] = o.affiliations.map { a -> [String: Any] in
                    ["peer": a.peer, "rel": a.rel, "name": a.name, "tether": a.tether, "at": a.at]
                }
            }
            if let v = o.venue {
                row["venue"] = ["place": v.place, "name": v.name, "at": v.at]
            }
            return row
        }
        let req: [String: Any] = ["now_ms": nowMs, "objects": rows]
        let obj = try callBarrier("project_objects", req) { arc_resolver_project_objects(self.handle, $0) }
        guard let d = obj as? [String: Any] else { throw ArcResolverError.badResponse("project_objects") }
        return ArcProjectStats(
            entitiesWritten: d["entities_written"] as? Int ?? 0,
            entitiesUnchanged: d["entities_unchanged"] as? Int ?? 0,
            factsAdded: d["facts_added"] as? Int ?? 0,
            factsUnchanged: d["facts_unchanged"] as? Int ?? 0,
            factsInvalidated: d["facts_invalidated"] as? Int ?? 0,
            skippedObjects: d["skipped_objects"] as? Int ?? 0
        )
    }

    /// PROMOTION: a ghost node lifted from text (by a build from before text extraction was
    /// removed) has just been minted as a real GroupObject — move the ghost's node onto the
    /// object's id. Facts, note-provenance, and the summary follow; the ghost is removed.
    /// Returns how many facts moved.
    @discardableResult
    public func rehome(ghostId: String, objectId: String, objectType: String,
                       name: String) throws -> Int {
        let req: [String: Any] = ["ghost_id": ghostId, "object_id": objectId,
                                  "object_type": objectType, "name": name]
        let obj = try callBarrier("rehome_entity", req) { arc_resolver_rehome_entity(self.handle, $0) }
        guard let d = obj as? [String: Any] else { throw ArcResolverError.badResponse("rehome_entity") }
        return d["facts_moved"] as? Int ?? 0
    }

    // MARK: reads (LIFE tab)

    /// Entities, optionally filtered by type + as-of ("now" or epoch ms). The LIFE tab's source.
    public func entities(type: String? = nil, asOf: Int64? = nil) throws -> [ArcEntity] {
        var req: [String: Any] = [:]
        if let type = type { req["type"] = type }
        if let asOf = asOf { req["as_of"] = asOf }
        let obj = try callRead("entities", req) { arc_resolver_entities(self.handle, $0) }
        guard let arr = obj as? [[String: Any]] else { throw ArcResolverError.badResponse("entities") }
        return arr.map(ArcResolver.entity(from:))
    }

    public func getEntity(id: String) throws -> ArcEntity? {
        let obj = try callRead("get_entity", ["id": id]) { arc_resolver_get_entity(self.handle, $0) }
        guard let dict = obj as? [String: Any] else { return nil }
        return ArcResolver.entity(from: dict)
    }

    /// Facts incident to `id` — the 1-hop neighbourhood. This is what makes an Event's
    /// participants readable, and therefore what the LIFE multimodal filter intersects over.
    /// The C entry point already ships in the binary; this only surfaces it.
    ///
    /// `direction` is "out" | "in" | "both"; `asOf` nil = the current frame.
    public func neighbors(id: String, direction: String = "out",
                          relation: String? = nil, asOf: Int64? = nil) throws -> [ArcFact] {
        var req: [String: Any] = ["id": id, "direction": direction]
        if let relation = relation { req["relation"] = relation }
        if let asOf = asOf { req["as_of"] = asOf }
        let obj = try callRead("neighbors", req) { arc_resolver_neighbors(self.handle, $0) }
        guard let arr = obj as? [[String: Any]] else { throw ArcResolverError.badResponse("neighbors") }
        return arr.map(ArcResolver.fact(from:))
    }

    /// Semantic entity search — the graph's entry point: query text is embedded and matched
    /// against entity labels, returning `(score, entity)` best-first. This is what turns a
    /// free-text search box into a graph query. `asOf` nil = the current frame.
    /// The C entry point already ships in the binary; this only surfaces it.
    public func semanticEntities(query: String, k: Int = 10, type: String? = nil,
                                 asOf: Int64? = nil) throws -> [(score: Float, entity: ArcEntity)] {
        var req: [String: Any] = ["query": query, "k": k]
        if let type = type { req["type"] = type }
        if let asOf = asOf { req["as_of"] = asOf }
        let obj = try callRead("semantic_entities", req) { arc_resolver_semantic_entities(self.handle, $0) }
        // The engine returns [[score, Entity], ...] — a positional pair, not an object.
        guard let arr = obj as? [[Any]] else { throw ArcResolverError.badResponse("semantic_entities") }
        return arr.compactMap { row in
            guard row.count >= 2,
                  let score = row[0] as? NSNumber,
                  let dict = row[1] as? [String: Any] else { return nil }
            return (score.floatValue, ArcResolver.entity(from: dict))
        }
    }

    /// Provenance: one row per episode, carrying the entities that episode produced facts
    /// about. `episodeId` IS the source object's id (a note's UUID), so inverting this gives
    /// `entity -> [note]`. Nothing writes new episodes: this reads the rows a graph already
    /// holds from before text extraction was removed.
    ///
    /// One query per episode server-side — build the index once and rebuild it when the graph
    /// epoch moves, never per keystroke.
    public func episodeIndex() throws -> [ArcEpisodeProvenance] {
        let s = withResponse("episode_index") { arc_resolver_episode_index(self.handle) }
        let obj = try ArcResolver.parse(s, "episode_index")
        guard let arr = obj as? [[String: Any]] else { throw ArcResolverError.badResponse("episode_index") }
        return arr.map {
            ArcEpisodeProvenance(
                episodeId: $0["episode_id"] as? String ?? "",
                source: $0["source"] as? String ?? "",
                occurredAt: $0["occurred_at"] as? Int64 ?? 0,
                entities: $0["entities"] as? [String] ?? []
            )
        }
    }

    static func fact(from d: [String: Any]) -> ArcFact {
        ArcFact(id: d["id"] as? String ?? "",
                src: d["src"] as? String ?? "",
                relation: d["relation"] as? String ?? "",
                dst: d["dst"] as? String ?? "",
                fact: d["fact"] as? String ?? "",
                validAt: d["valid_at"] as? Int64,
                invalidAt: d["invalid_at"] as? Int64,
                createdAt: d["created_at"] as? Int64)
    }

    public func stats() throws -> ArcStats {
        let s = withResponse("stats") { arc_resolver_stats(self.handle) }
        let obj = try ArcResolver.parse(s, "stats")
        guard let d = obj as? [String: Any] else { throw ArcResolverError.badResponse("stats") }
        return ArcStats(entities: d["entities"] as? Int ?? 0,
                        facts: d["facts"] as? Int ?? 0,
                        indexedDocuments: d["indexed_documents"] as? Int ?? 0)
    }

    // MARK: - plumbing

    private func callRead(_ what: String, _ req: [String: Any],
                          _ fn: @escaping (UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?) throws -> Any {
        try queue.sync { try invoke(what, req, fn) }
    }

    private func callBarrier(_ what: String, _ req: [String: Any],
                             _ fn: @escaping (UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?) throws -> Any {
        var result: Result<Any, Error>!
        queue.sync(flags: .barrier) { result = Result { try self.invoke(what, req, fn) } }
        return try result.get()
    }

    private func invoke(_ what: String, _ req: [String: Any],
                        _ fn: (UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?) throws -> Any {
        guard let data = try? JSONSerialization.data(withJSONObject: req),
              let json = String(data: data, encoding: .utf8) else {
            throw ArcResolverError.call("\(what): could not encode request")
        }
        let raw = json.withCString { fn($0) }
        let s = ArcResolver.takeString(raw) ?? "null"
        return try ArcResolver.parse(s, what)
    }

    private func withResponse(_ what: String, _ fn: () -> UnsafeMutablePointer<CChar>?) -> String {
        queue.sync { ArcResolver.takeString(fn()) ?? "null" }
    }

    /// Copy a returned C string into Swift and free the Rust allocation.
    private static func takeString(_ ptr: UnsafeMutablePointer<CChar>?) -> String? {
        guard let ptr = ptr else { return nil }
        defer { arc_resolver_string_free(ptr) }
        return String(cString: ptr)
    }

    /// Parse a JSON response, surfacing `{"error": ...}` as a thrown error.
    private static func parse(_ s: String, _ what: String) throws -> Any {
        guard let data = s.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]) else {
            throw ArcResolverError.badResponse("\(what): not JSON")
        }
        if let dict = obj as? [String: Any], let err = dict["error"] as? String {
            throw ArcResolverError.call("\(what): \(err)")
        }
        return obj
    }

    private static func entity(from dict: [String: Any]) -> ArcEntity {
        let props = dict["properties"] as? [String: Any]
        let label = dict["label"] as? String ?? ""
        let name = (props?["name"] as? String) ?? label
        let summary = props?["summary"] as? String
        return ArcEntity(
            id: dict["id"] as? String ?? "",
            entityType: dict["entity_type"] as? String ?? "",
            label: label,
            name: name,
            summary: summary,
            source: props?["source"] as? String,
            validAt: (dict["valid_at"] as? NSNumber)?.int64Value,
            invalidAt: (dict["invalid_at"] as? NSNumber)?.int64Value,
            createdAt: (dict["created_at"] as? NSNumber)?.int64Value
        )
    }
}
