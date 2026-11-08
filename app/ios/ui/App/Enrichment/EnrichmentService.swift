//  EnrichmentService.swift
//  The app-level owner of the enrichment catalogue. Views read this from the
//  environment to (a) resolve matches for inline styling and (b) drive the single
//  long-press info panel (`inspected`). Sources register their entities here —
//  ALL of them real: the Notes OUTLINE graph (at launch + on every outline edit),
//  NOTE titles (at launch + on change), conversation titles, and SYSIPHUS when
//  its SQL interface is connected. No source seeds fake matches — an unconnected
//  Sysiphus contributes nothing, and no fixture ever enters the catalogue.

import SwiftUI
import Combine

@MainActor
final class EnrichmentService: ObservableObject {
    /// Bumped whenever the catalogue changes, so EnrichedText/Label recompute.
    @Published private(set) var version = 0
    /// The entity currently being inspected via long-press — drives one app-level sheet.
    @Published var inspected: EnrichedEntity?

    private var catalogue = EnrichmentCatalogue()
    private var outlineSub: AnyCancellable?

    init() {
        ingestOutline()
        // The outline is a live, editable store (the refine/enrich loop). Re-ingest
        // after every mutation — objectWillChange fires pre-mutation, so hop the
        // main queue once and read the post-change state.
        outlineSub = OutlineModel.shared.objectWillChange
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.ingestOutline() }
    }

    var isEmpty: Bool { catalogue.isEmpty }
    var count: Int { catalogue.count }

    func matches(in text: String) -> [EnrichmentMatch] { catalogue.matches(in: text) }
    func entity(_ id: String) -> EnrichedEntity? { catalogue.entity(id) }

    /// What enriched views render: the exact-alias trie's matches.
    func allMatches(in text: String) -> [EnrichmentMatch] { catalogue.matches(in: text) }

    /// Replace a source's entities. Only republishes (bumps `version`, forcing
    /// enriched views to rematch) when the match index actually changed — editing
    /// a note body, which never alters surface forms, is a no-op here.
    func ingest(_ entities: [EnrichedEntity], from origin: EntityOrigin) {
        if catalogue.replace(origin: origin, with: entities) {
            version += 1
        }
    }

    /// Pull the Notes tab's outline graph into the catalogue (excludes Actions —
    /// those are to-do verbs, not nameable entities).
    func ingestOutline() {
        let ents = OutlineModel.shared.byID.values
            .filter { !$0.isAction }
            .map { EnrichedEntity(outline: $0) }
        ingest(Array(ents), from: .outline)
    }

    /// Register note titles as lightweight entities so notes cross-reference too.
    /// Em-dash segments of a title register as aliases ("AW26 flat shoot — run
    /// sheet v2" also matches "run sheet v2"), so chat copy that points at a note
    /// by its short name lights up.
    func ingestNotes(_ notes: [Note]) {
        let ents = notes.compactMap { note -> EnrichedEntity? in
            let title = note.title.trimmingCharacters(in: .whitespacesAndNewlines)
            guard title.count >= 3 else { return nil }
            let segments = title.components(separatedBy: "—")
                .map { $0.trimmingCharacters(in: .whitespaces) }
                .filter { $0.count >= 4 && $0 != title }
            return EnrichedEntity(
                id: "note:\(note.id.uuidString)",
                name: title,
                kind: .topic,
                origin: .note,
                subtitle: "Note",
                detail: String(note.body.prefix(200)),
                surfaceForms: segments
            )
        }
        ingest(ents, from: .note)
    }

    /// Register ChatRoom conversations (Forum GroupObjects) so a mention of a room's name
    /// resolves to it and the type-router opens the REAL thread (ThreadView). Only titled rooms
    /// are registered — an untitled "Group ab12" fallback name would not resolve meaningfully.
    /// This is the intended use of EntityOrigin.chat (built for conversation/member names).
    func ingestConversations(_ rooms: [(id: String, title: String)]) {
        let ents = rooms.compactMap { room -> EnrichedEntity? in
            let title = room.title.trimmingCharacters(in: .whitespacesAndNewlines)
            guard title.count >= 3 else { return nil }
            return EnrichedEntity(id: "thread:\(room.id)", name: title, kind: .topic,
                                  origin: .chat, subtitle: "Channel")
        }
        ingest(ents, from: .chat)
    }
}

// MARK: - Mapping the outline graph into catalogue entities

extension EnrichedEntity {
    init(outline e: EntityVM) {
        let kind: EnrichmentKind
        switch e.kind {
        case .person:  kind = .person
        case .org:     kind = .org
        case .place:   kind = .place
        case .event:   kind = .event
        }
        self.init(
            id: "outline:\(e.id)",
            name: e.name,
            kind: kind,
            origin: .outline,
            subtitle: e.descriptor ?? e.kind.word,
            detail: e.notes,
            fields: e.attributes.map { EntityField(label: "tag", value: $0) },
            surfaceForms: []
        )
    }
}
