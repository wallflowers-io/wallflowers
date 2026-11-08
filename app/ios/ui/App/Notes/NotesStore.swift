//  NotesStore.swift
//  The Notes producer (WS-F). LodeDB IS the
//  notes home now: every note lives in the ONE union store via the `LocalStore` DAL
//  (WS-E). notes.json is retired — there is no second source of truth. The PUBLIC
//  interface (`create` / `update` / `delete` / `notes`) is unchanged, so NoteComposeModal,
//  NotesView and NoteEditorView keep working verbatim; only the backing swapped.
//
//  The @Published `notes` array is the OBSERVABLE cache the UI renders and edits
//  synchronously (create must return a Note immediately for navigation). Durable writes
//  MIRROR into the union store asynchronously (source = note, §3), and the cache is
//  hydrated from the store once at launch. LodeDB is therefore the durable + searchable
//  home; the cache is just its live view.
//
//  Note <-> Document round-trip (§3 doc row, source: note):
//    • `text`      the searchable/embeddable payload: "title\n\n body" when there is a
//                  title, else the body. Never empty when the note has content, so the
//                  embedder is never handed an empty string and the row is enumerable.
//    • title       carried in metadata (short; also fed to the enrichment catalogue).
//    • created /   epoch-seconds strings in metadata, so ordering and the "updated N ago"
//      updated     line survive a relaunch.
//  The body is reconstructed by stripping the known "title\n\n" prefix we wrote — a
//  format we own on both sides, so the title/body split is preserved exactly for the
//  editor (no lossy first-line-is-title guessing).
//
//  Honesty contract (house rule): a store write/read that fails is logged loudly, never
//  silently swallowed. An empty `notes` after a clean load is a genuinely-empty world,
//  not a masked error.

import Foundation
import PacificStore

struct Note: Identifiable, Codable, Equatable, Hashable {
    var id = UUID()
    var title: String = ""
    var body: String = ""
    var created: Date = Date()
    var updated: Date = Date()
    /// The note's #tags — parsed from its own text by the ONE extractor CHAT
    /// shares (`Hashtags`, 2 Aug ruling), re-derived on every commit. Carried on
    /// the note so rows render them synchronously; never independently edited.
    var tags: [NoteTag] = []

    /// First non-empty line, used as a fallback title / snippet.
    var derivedTitle: String {
        let t = title.trimmingCharacters(in: .whitespacesAndNewlines)
        if !t.isEmpty { return t }
        let firstLine = body.split(separator: "\n", maxSplits: 1).first.map(String.init) ?? ""
        let f = firstLine.trimmingCharacters(in: .whitespacesAndNewlines)
        return f.isEmpty ? "New note" : f
    }

    var snippet: String {
        let t = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let b = body.trimmingCharacters(in: .whitespacesAndNewlines)
        if !t.isEmpty { return b }
        // Title empty ⇒ first line was the title; snippet is the remainder.
        let parts = b.split(separator: "\n", maxSplits: 1)
        return parts.count > 1 ? parts[1].trimmingCharacters(in: .whitespacesAndNewlines) : ""
    }
}

@MainActor
final class NotesStore: ObservableObject {
    @Published private(set) var notes: [Note] = []

    /// The union-store DAL (WS-E). The composition root injects the shared instance so a
    /// note written here is the same row the LIFE search reads back.
    ///
    /// Readable (not private) because NOTE search reads the very rows this store writes —
    /// `NoteSearch`'s text lane runs `search` over them. Exposing the one instance NotesStore
    /// already owns is what keeps search and persistence on a single store; a second handle
    /// would be a second source of truth.
    let store: LocalStore

    /// Metadata keys the Notes producer layers on top of the §3 doc row. Namespaced with
    /// a `note.` prefix so they never collide with the shared §3 schema keys.
    private enum NoteMeta {
        static let title = "note.title"
        static let created = "note.created"
        static let updated = "note.updated"
        /// JSON-encoded [NoteTag]. One string value because LodeDB metadata is
        /// [String: String]; exact round-trip beats a delimiter convention. Not used for
        /// store-side filtering (that would want one key per tag) — display + search read
        /// the hydrated cache.
        static let tags = "note.tags"
    }

    init(store: LocalStore) {
        self.store = store
        // Hydrate the cache from the durable store (LodeDB is the home). Async because the
        // store is actor-backed; the UI shows an empty list until this returns, then the
        // published mutation lands the loaded notes and observers re-render.
        Task { await hydrate() }
    }

    // MARK: - Load (LodeDB is the notes home)

    private func hydrate() async {
        do {
            let docs = try await store.listDocuments()
            let loaded: [Note] = docs.compactMap { doc in
                // Only OUR note rows — the union also holds entity nodes/edges and other
                // producers' documents.
                guard Metadata.source(doc.metadata) == .note,
                      Metadata.kind(doc.metadata) == .doc,
                      let uuid = UUID(uuidString: doc.id) else { return nil }
                let title = doc.metadata[NoteMeta.title] ?? ""
                let text = doc.text ?? ""
                return Note(
                    id: uuid,
                    title: title,
                    body: Self.body(fromStored: text, title: title),
                    created: Self.date(doc.metadata[NoteMeta.created]),
                    updated: Self.date(doc.metadata[NoteMeta.updated]),
                    tags: Self.tags(doc.metadata[NoteMeta.tags])
                )
            }
            notes = loaded.sorted { $0.updated > $1.updated }
        } catch {
            Trace.shared.log("notes", "hydrate the notes from the store — the list will read as empty until this succeeds: \(error)", ok: false)
        }
    }

    // MARK: - Mutations (the stable public interface)

    @discardableResult
    func create() -> Note {
        // A blank note is NOT persisted — nothing to embed, no empty row. It becomes
        // durable on the first `update` that gives it content (or is pruned by
        // `discardIfEmpty`).
        let n = Note()
        notes.insert(n, at: 0)
        return n
    }

    func update(_ note: Note) {
        guard let i = notes.firstIndex(where: { $0.id == note.id }) else { return }
        var n = note
        n.updated = Date()
        notes[i] = n
        notes.sort { $0.updated > $1.updated }
        mirror(n)
    }

    func delete(_ note: Note) {
        notes.removeAll { $0.id == note.id }
        Task { [store] in
            do { try await store.remove(id: note.id.uuidString) }
            catch { Trace.shared.log("notes", object: note.id.uuidString, "delete the note from the store — it will come back on next launch: \(error)", ok: false) }
        }
    }

    /// Prune a note that was created but left completely empty (avoids blank rows).
    func discardIfEmpty(_ note: Note) {
        guard let n = notes.first(where: { $0.id == note.id }) else { return }
        if n.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
           n.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            delete(n)
        }
    }

    // MARK: - Durable mirror (write straight to the union store)

    /// Upsert a note with content into the union store (source: note, §3); a note whose
    /// title and body are both blank is removed instead — so an emptied note never lingers
    /// as a durable ghost. Fire-and-forget from the synchronous mutations; failures are
    /// logged, never masked.
    private func mirror(_ note: Note) {
        var note = note
        // #tags are THE tags (2 Aug ruling): parsed from the note's own words by
        // the one extractor CHAT shares (`Hashtags`) — never minted by a model.
        // Derived INSIDE the commit, so the chips can never disagree with the
        // text and no second, re-announcing write exists. `type` is empty — a
        // hashtag carries no entity type, and we don't fabricate one.
        note.tags = Hashtags.extract(Self.storedText(for: note)).map { NoteTag(name: $0, type: "") }
        if let i = notes.firstIndex(where: { $0.id == note.id }), notes[i].tags != note.tags {
            notes[i].tags = note.tags
        }
        let hasContent = !note.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !note.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let id = note.id.uuidString
        Task { [store] in
            do {
                guard hasContent else { try await store.remove(id: id); return }
                var metadata = Metadata.document(source: .note, objectId: id).build()
                metadata[NoteMeta.title] = note.title
                metadata[NoteMeta.created] = Self.epoch(note.created)
                metadata[NoteMeta.updated] = Self.epoch(note.updated)
                if let encoded = Self.tagsJSON(note.tags) { metadata[NoteMeta.tags] = encoded }
                try await store.upsert(id: id, text: Self.storedText(for: note), metadata: metadata)
            } catch {
                Trace.shared.log("notes", object: id, "persist the note — LodeDB is its only home, so it is lost at next launch: \(error)", ok: false)
            }
        }
    }

    // MARK: - Round-trip helpers

    /// The searchable/embeddable payload. Title is prefixed so it is part of the embedded
    /// text; the body follows after a blank line. Empty only when the note is truly empty
    /// (which never reaches here — `mirror` removes instead).
    private static func storedText(for note: Note) -> String {
        note.title.isEmpty ? note.body : note.title + "\n\n" + note.body
    }

    /// Recover the exact body from the stored text by removing the `title\n\n` prefix we
    /// wrote. We own the format on both sides, so this is exact — not a first-line guess.
    private static func body(fromStored text: String, title: String) -> String {
        guard !title.isEmpty else { return text }
        let prefix = title + "\n\n"
        return text.hasPrefix(prefix) ? String(text.dropFirst(prefix.count)) : text
    }

    /// Epoch seconds as a string — full precision, formatter-free, stable ordering.
    private static func epoch(_ date: Date) -> String {
        String(date.timeIntervalSince1970)
    }

    /// Parse an epoch-seconds metadata value. A missing/garbled presentational timestamp
    /// falls back to now (it only affects display order, not correctness of the note).
    private static func date(_ raw: String?) -> Date {
        guard let raw, let seconds = Double(raw) else { return Date() }
        return Date(timeIntervalSince1970: seconds)
    }

    /// Tags round-trip. Encode: nil when empty, so untagged notes carry no key at all.
    /// Decode: a missing/garbled value is an untagged note — logged, never a crash.
    private static func tagsJSON(_ tags: [NoteTag]) -> String? {
        guard !tags.isEmpty, let data = try? JSONEncoder().encode(tags) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    private static func tags(_ raw: String?) -> [NoteTag] {
        guard let raw, let data = raw.data(using: .utf8) else { return [] }
        do { return try JSONDecoder().decode([NoteTag].self, from: data) }
        catch {
            Trace.shared.log("notes", "decode the note\u{2019}s tags — they will read as none: \(error)", ok: false)
            return []
        }
    }
}
