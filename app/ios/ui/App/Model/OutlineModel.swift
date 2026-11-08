//  OutlineModel.swift
//  The Notes tab's OUTLINE model + seed (relocated from the old Work tab —
//  Projects now live in Work as season timelines). Primitive model:
//   • PLACE and EVENT are first-class entities; an EVENT is always a window.
//   • An ACTION is just an UNRESOLVED future Event with a Group ASSIGNEE; "~"
//     (waiting) is DERIVED (assignee ≠ you). To-dos are authored inline, then
//     projected into first-class Action entities.
//  CRUCIALLY (per the founder): the seed inferences are deliberately rough — the
//  user EDITS / REFINES / ENRICHES them, and that act is how Pacific helps them
//  understand their world. So the store is OBSERVABLE and mutable in-session.
//
//  Seed: Nye's world at Aries — two sites (Sheep Lane, Great Pulteney Street),
//  suppliers, wholesalers, Sysiphus, and the July 2026 event windows.
//
//  WS-F: the OutlineModel stays the in-session
//  source of truth for the entity graph, but every entity mutation now PROJECTS into the
//  ONE union store (§3): each entity is a node (kind: node, its EntityKind → EntityType),
//  each of its links is an edge (kind: edge, src/dst). That projection is what makes
//  entities show up in LIFE's LocalStore reads. The store is
//  attached at the composition root; before then (previews / tests) mutations are
//  in-memory only — the absence of a store, not a masked failure.

import SwiftUI
import PacificStore

enum EntityKind: String, Hashable, CaseIterable, Codable {
    case person, org, place, event
    var glyph: String {
        switch self {
        case .person:  return "person.fill"
        case .org:     return "building.2.fill"
        case .place:   return "mappin.circle.fill"
        case .event:   return "calendar"
        }
    }
    var word: String {
        switch self {
        case .person: return "Person"; case .org: return "Organisation"
        case .place: return "Place"; case .event: return "Event"
        }
    }
}

struct TimeChip: Hashable, Codable {
    enum Urgency: String, Codable { case overdue, soon, dated, loose }
    var label: String
    var urgency: Urgency
    var tint: Color {
        switch urgency {
        case .overdue: return .red
        case .soon:    return .orange
        case .dated:   return Brand.accent
        case .loose:   return .secondary
        }
    }
}

struct EventWindow: Hashable, Codable {
    var start: String
    var end: String
    var urgency: TimeChip.Urgency = .dated
    var display: String { end.isEmpty ? start : "\(start) → \(end)" }
}

struct ToDoVM: Identifiable, Hashable, Codable {
    // `var` (not `let`) so the synthesized decoder restores the SAME id across a
    // relaunch — a `let` with a default is skipped by Codable and would re-mint.
    var id = UUID()
    var verb: String
    var time: TimeChip? = nil
    var waiting: Bool = false
    var linkIDs: [String] = []
    static func == (l: ToDoVM, r: ToDoVM) -> Bool { l.id == r.id }
    func hash(into h: inout Hasher) { h.combine(id) }
}

struct EntityVM: Identifiable, Hashable, Codable {
    /// The persisted shape (the union-store snapshot). `tint` is deliberately absent:
    /// it is DERIVED from the id (`tintFor`), so storing it would just be a second
    /// source of truth. Its default keeps the synthesized decoder valid.
    enum CodingKeys: String, CodingKey {
        case id, name, kind, descriptor, containerID, attributes, notes, todos
        case childIDs, actionIDs, linkIDs, window, due, assignee, resolved
        case offPlatform, waiting, availability, unminted, location
    }

    let id: String
    var name: String
    var kind: EntityKind
    var descriptor: String? = nil
    var containerID: String? = nil
    var attributes: [String] = []
    var notes: String? = nil
    var todos: [ToDoVM] = []
    var childIDs: [String] = []
    var actionIDs: [String] = []
    var linkIDs: [String] = []
    var window: EventWindow? = nil
    var due: TimeChip? = nil
    var assignee: String? = nil
    var resolved: Bool = false
    var offPlatform: Bool = true
    var waiting: Bool = false
    var availability: TimeChip? = nil
    var unminted: Bool = false
    /// The object's location facet — COMMON to all kinds (places, orgs, things,
    /// optionally people): a fixed coordinate or a live stream reference. nil =
    /// unplaced. Mirrors pacific-core `geo.rs` LocationSource; see LifeGeo.
    var location: EntityLocation? = nil
    var tint: Color = .gray

    var actionWaiting: Bool { assignee != nil && assignee != "me" }
    var isAction: Bool { kind == .event && assignee != nil }

    var initials: String {
        let words = name.split(separator: " ")
        guard let f = words.first else { return "?" }
        if words.count >= 2 { return (f.prefix(1) + words[1].prefix(1)).uppercased() }
        return f.prefix(2).uppercased()
    }
    static func == (l: EntityVM, r: EntityVM) -> Bool { l.id == r.id }
    func hash(into h: inout Hasher) { h.combine(id) }
}

// MARK: - The observable, editable store

final class OutlineModel: ObservableObject {
    static let shared = OutlineModel()

    @Published private(set) var byID: [String: EntityVM] = [:]
    @Published private(set) var order: [String] = []

    /// The union-store DAL (WS-E). Injected at the composition root via `attach(store:)`;
    /// nil in previews / tests, where the graph lives only in memory.
    private var store: LocalStore?

    /// The tail of the projection write chain. `mirror` is fire-and-forget from
    /// synchronous UI mutations, so two writes to the SAME document id would otherwise
    /// race for the store actor's mailbox with no ordering guarantee — and the engine
    /// replaces a record wholesale on re-append, so the LOSER's payload survives.
    ///
    /// That is not hypothetical: the LIFE mint calls `addEntity` (which projects a stub
    /// carrying "added by you · refine me") and then `save` (the enriched entity) for the
    /// same id, back to back. Unordered, the stub can land last and the user's descriptor,
    /// notes and links are gone on the next launch — silently, because the in-memory copy
    /// still looks right all session. Chaining every write keeps them FIFO.
    private var writeTail: Task<Void, Never>?

    init() {
        // Real-ops only: the Outline starts EMPTY and is populated exclusively by the
        // user's own refine/enrich loop. (The old PACIFIC_ENABLE_MOCK_SEED fixture is
        // gone — fake entities must never light up in anyone's chats or notes, and
        // fixtures couple development to fake data instead of the real flows.)
        byID = [:]
        order = []
    }

    // — Queries —
    func entity(_ id: String) -> EntityVM? { byID[id] }
    func topLevel() -> [EntityVM] { order.compactMap { byID[$0] } }
    func children(of id: String) -> [EntityVM] { (byID[id]?.childIDs ?? []).compactMap { byID[$0] } }
    func actions(of id: String) -> [EntityVM] { (byID[id]?.actionIDs ?? []).compactMap { byID[$0] } }

    func inbound(to id: String) -> [EntityVM] {
        byID.values.filter { e in
            e.linkIDs.contains(id) || e.todos.contains { $0.linkIDs.contains(id) }
        }
        .filter { !$0.isAction }
        .sorted { $0.name < $1.name }
    }

    func search(_ query: String, filter: String) -> [EntityVM] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        func typeOK(_ e: EntityVM) -> Bool {
            switch filter {
            case "People":   return e.kind == .person
            case "Orgs":     return e.kind == .org
            case "Places":   return e.kind == .place
            case "Events":   return e.kind == .event && !e.isAction
            case "Actions":  return e.isAction
            default:         return true
            }
        }
        return byID.values.filter { e in
            typeOK(e) && (q.isEmpty
                || e.name.lowercased().contains(q)
                || (e.descriptor ?? "").lowercased().contains(q)
                || (e.notes ?? "").lowercased().contains(q)
                || e.attributes.contains { $0.lowercased().contains(q) })
        }.sorted { $0.name < $1.name }
    }

    // — Composition root —
    /// Attach the shared union store: back-fill whatever is already in memory, then
    /// rebuild the graph from what the store durably holds.
    ///
    /// ORDER MATTERS. The back-fill runs FIRST, over entities that existed before the
    /// store arrived (at launch: none, since `init` starts empty). Hydrate then loads the
    /// durable rows. The two must never cross: re-mirroring a freshly-hydrated entity
    /// would rewrite its row from the in-memory copy, so `hydrate` assigns `byID`
    /// directly and never routes through `save`/`addEntity`/`mirror`.
    func attach(store: LocalStore) {
        self.store = store
        for e in byID.values where !e.isAction { mirror(e) }
        Task { await hydrate(from: store) }
    }

    // — Rehydrate (the store is the durable home; `byID` is its live view) —

    /// Rebuild the entity graph from the union store, once, at attach. The published
    /// mutation lands on the main actor so observers re-render. An empty store is a
    /// genuinely-empty world, not a masked error; a failure is logged loudly.
    private func hydrate(from store: LocalStore) async {
        do {
            var loaded: [EntityVM] = []
            for kind in EntityKind.allCases {
                let docs = try await store.listDocuments(type: kind.entityType)
                loaded += docs.compactMap(OutlineModel.entity(from:))
            }
            let rebuilt = loaded
            await MainActor.run { self.apply(rebuilt) }
        } catch {
            await Trace.shared.log("outline", "hydrate the outline from the store — it will read as empty until this succeeds: \(error)", ok: false)
        }
    }

    /// Land a hydrated graph. Anything created in-memory while hydrate was in flight WINS
    /// (it is newer than the durable snapshot), so a fast first mint is never clobbered.
    private func apply(_ loaded: [EntityVM]) {
        guard !loaded.isEmpty else { return }
        for e in loaded where byID[e.id] == nil { byID[e.id] = e }
        // Top level is everything with no container; children hang off their parent.
        order = byID.values
            .filter { $0.containerID == nil && !$0.isAction }
            .sorted { $0.name < $1.name }
            .map(\.id)
    }

    // — Mutations (the refine/enrich loop) —
    func save(_ e: EntityVM) {
        byID[e.id] = e
        mirror(e)
    }

    func addAction(to ownerID: String, verb: String, waiting: Bool, due: TimeChip?) {
        // Actions are to-do verbs (unresolved events with an assignee), NOT entity nodes —
        // LIFE excludes them and §3 has no assignee field — so they are not projected.
        guard let owner = byID[ownerID] else { return }
        let aid = "act-\(ownerID)-\(UUID().uuidString.prefix(4))"
        byID[aid] = OutlineModel.makeAction(id: aid, owner: owner, verb: verb, waiting: waiting, due: due, links: [])
        byID[ownerID]?.actionIDs.append(aid)
    }

    @discardableResult
    func addEntity(to containerID: String?, name: String, kind: EntityKind) -> String {
        let id = "u-\(UUID().uuidString.prefix(6))"
        let entity = EntityVM(id: id, name: name, kind: kind, descriptor: "added by you · refine me",
                              containerID: containerID, tint: tintFor(id))
        byID[id] = entity
        if let c = containerID {
            byID[c]?.childIDs.append(id)
        } else {
            order.append(id)
        }
        mirror(entity)
        // The PARENT changed too — its `childIDs` now names this child. Persisting only
        // the child would orphan it: on relaunch the child hydrates with a `containerID`
        // (so `apply` keeps it out of `order`) while the parent's stored snapshot still
        // has an empty `childIDs` (so `children(of:)` never returns it), and the entity
        // drops out of the outline hierarchy entirely.
        if let c = containerID, let parent = byID[c] { mirror(parent) }
        return id
    }

    // — Union-store projection (§3): entity → node, links → edges —

    /// Project one entity into the union store: a node row (kind: node, its EntityType),
    /// plus an edge row (kind: edge, src/dst) for each entity it links to. Idempotent by
    /// id (node id = entity id; edge id = "src->dst"), so a re-save replaces rather than
    /// duplicates. In-memory only when no store is attached (previews / tests).
    ///
    /// KNOWN LIMIT: a link REMOVED from an entity leaves its prior edge row behind (we
    /// upsert the current edges but do not diff-and-remove the gone ones). Edge cleanup on
    /// unlink is a follow-up; it does not affect newly-linked relatedness.
    private func mirror(_ e: EntityVM) {
        guard let store, !e.isAction else { return }
        let type = e.kind.entityType
        let nodeText = OutlineModel.searchText(e)
        var nodeMeta = Metadata.node(source: .entity, objectId: e.id, type: type).build()
        // The LOSSLESS half. §3 metadata carries only id + type, and `searchText` flattens
        // name/descriptor/attributes/notes into one embeddable blob — enough to SEARCH a
        // row, nowhere near enough to REBUILD it. Carry the whole entity as a snapshot so
        // a relaunch restores it exactly instead of a name-only husk.
        if let snapshot = OutlineModel.encodeSnapshot(e) {
            nodeMeta[EntityMeta.snapshot] = snapshot
        }
        // Resolve link display names on this (UI) thread before the detached write.
        let links = Set(e.linkIDs + e.todos.flatMap(\.linkIDs)).subtracting([e.id])
        let edges: [(id: String, text: String, meta: [String: String])] = links.map { dst in
            let id = "\(e.id)->\(dst)"
            let text = "\(e.name) → \(byID[dst]?.name ?? dst)"
            return (id, text, Metadata.edge(source: .entity, objectId: id, src: e.id, dst: dst).build())
        }
        let nodeID = e.id
        enqueueWrite { [store] in
            do {
                try await store.upsert(id: nodeID, text: nodeText, metadata: nodeMeta)
                for edge in edges {
                    try await store.upsert(id: edge.id, text: edge.text, metadata: edge.meta)
                }
            } catch {
                await Trace.shared.log("outline", object: nodeID, "project the node into the store — the edit is in memory only: \(error)", ok: false)
            }
        }
    }

    /// Append one projection write to the serial chain, so writes reach the store in the
    /// order the user made them. Called only from the main thread (every mutation is a UI
    /// action), so `writeTail` needs no further synchronisation.
    private func enqueueWrite(_ write: @escaping @Sendable () async -> Void) {
        let prior = writeTail
        writeTail = Task {
            await prior?.value
            await write()
        }
    }

    /// Metadata keys the entity producer layers on top of the §3 node row. Namespaced
    /// with an `entity.` prefix so they never collide with the shared §3 schema keys
    /// (same discipline as `NotesStore.NoteMeta`).
    private enum EntityMeta {
        static let snapshot = "entity.snapshot"
    }

    /// The entity as a JSON snapshot, for a lossless round-trip. Returns nil (loudly) on
    /// an encode failure — the row still persists and stays searchable, it just falls
    /// back to the degraded rebuild below.
    private static func encodeSnapshot(_ e: EntityVM) -> String? {
        do {
            return String(data: try JSONEncoder().encode(e), encoding: .utf8)
        } catch {
            Task { @MainActor in Trace.shared.log("outline", object: e.id, "encode the node snapshot — it will not persist: \(error)", ok: false) }
            return nil
        }
    }

    /// Rebuild an entity from a stored node row. Prefers the lossless snapshot; falls back
    /// to §3 metadata + the first line of the embedded text for any row written before
    /// snapshots existed (that fallback recovers the name only — all the projection ever
    /// kept — and is never treated as a faithful entity).
    private static func entity(from doc: Document) -> EntityVM? {
        guard Metadata.source(doc.metadata) == .entity,
              Metadata.kind(doc.metadata) == .node,
              let type = Metadata.type(doc.metadata),
              let kind = EntityKind(entityType: type) else { return nil }

        if let raw = doc.metadata[EntityMeta.snapshot], let data = raw.data(using: .utf8) {
            do {
                var e = try JSONDecoder().decode(EntityVM.self, from: data)
                e.tint = tintFor(e.id)          // derived, never persisted
                return e
            } catch {
                Task { @MainActor in Trace.shared.log("outline", object: doc.id, "decode the stored node — it is skipped on load: \(error)", ok: false) }
            }
        }

        let name = (doc.text ?? "").split(separator: "\n").first.map(String.init) ?? doc.id
        return EntityVM(id: doc.id, name: name, kind: kind, tint: tintFor(doc.id))
    }

    /// The searchable/embeddable text for an entity node — its name plus whatever refined
    /// context the user has given it. Never empty (name is always present).
    private static func searchText(_ e: EntityVM) -> String {
        var parts = [e.name]
        if let d = e.descriptor, !d.isEmpty { parts.append(d) }
        if !e.attributes.isEmpty { parts.append(e.attributes.joined(separator: ", ")) }
        if let n = e.notes, !n.isEmpty { parts.append(n) }
        return parts.joined(separator: "\n")
    }

    private static func makeAction(id: String, owner: EntityVM, verb: String,
                                   waiting: Bool, due: TimeChip?, links: [String]) -> EntityVM {
        EntityVM(id: id, name: verb, kind: .event,
                 descriptor: waiting ? "Action · waiting on \(owner.name)" : "Action · your move",
                 containerID: owner.id, linkIDs: links, due: due,
                 assignee: waiting ? owner.id : "me", resolved: false, waiting: waiting, tint: owner.tint)
    }
}


private let palette: [Color] = [.teal, .indigo, .blue, .green, .orange, .purple, .pink, .mint, .cyan, .red, .brown]
func tintFor(_ s: String) -> Color {
    let n = s.unicodeScalars.reduce(0) { $0 + Int($1.value) }
    return palette[n % palette.count]
}

extension EntityKind {
    /// The §3 graph-node `type` this UI kind projects to in the union store. (LIFE's
    /// PROJECTS filter — `EntityType.project` — has no `EntityKind`; projects are synced
    /// GroupObjects, not local entity-graph entities.)
    var entityType: EntityType {
        switch self {
        case .person: return .person
        case .org:    return .org
        case .place:  return .place
        case .event:  return .event
        }
    }

    /// The inverse of ``entityType``, for rebuilding entities off stored node rows. Nil
    /// for `.project`: projects are synced GroupObjects, not local entity-graph entities,
    /// so they have no `EntityKind` and are never hydrated into the outline.
    init?(entityType: EntityType) {
        switch entityType {
        case .person:  self = .person
        case .org:     self = .org
        case .place:   self = .place
        case .event:   self = .event
        case .project: return nil
        }
    }
}
