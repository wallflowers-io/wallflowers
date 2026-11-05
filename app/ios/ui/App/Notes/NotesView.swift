//  NotesView.swift
//  The NOTES page — every note you have kept, newest first, over a search field. It is
//  reached by HOLDING the shell's NOTE disc; a tap on that disc composes, so this page
//  creates nothing and shows no `+`. Row snippets render enriched, so linked entities
//  light up in your notes; tapping a row opens it in the editor.
//
//  This page is deliberately ONE thing: find a note, open a note. The entity Outline it
//  used to carry as a second segment now lives in LIFE.

import SwiftUI

struct NotesView: View {
    @EnvironmentObject private var store: NotesStore
    @EnvironmentObject private var enrich: EnrichmentService
    @EnvironmentObject private var shell: ShellState

    @EnvironmentObject private var graph: GraphService

    @State private var query = ""
    @State private var path: [Note] = []
    @State private var outcome = NoteSearchOutcome()
    @State private var searchError: String?
    @State private var searchTask: Task<Void, Never>?
    /// The selected tag chip (nil = All). Browse-by-tag, per the Three Channels design:
    /// the chip narrows the journal AND the searched ranking — the two compose.
    @State private var selectedTag: String?

    /// The count line under the list. With a tag selected it names the filter, so a
    /// short list reads as "filtered", never "lost".
    private var countLine: String {
        let n = rows.count
        let noun = n == 1 ? "note" : "notes"
        guard let tag = activeTag else { return "\(n) \(noun)" }
        return "\(n) \(noun) tagged #\(tag)"
    }

    /// Every tag currently in use, frequency-first — the chips row is built from the
    /// notes themselves, so it can never offer a filter with nothing behind it.
    private var tagVocabulary: [String] {
        var counts: [String: (display: String, n: Int)] = [:]
        for note in store.notes {
            for t in note.tags {
                let k = t.name.lowercased()
                counts[k] = (counts[k]?.display ?? t.name, (counts[k]?.n ?? 0) + 1)
            }
        }
        return counts.values
            .sorted { $0.n == $1.n ? $0.display < $1.display : $0.n > $1.n }
            .map(\.display)
    }

    /// The selection, validated against the live vocabulary — a tag that vanished
    /// (notes retagged or deleted) silently reads as All rather than an empty screen.
    private var activeTag: String? {
        guard let t = selectedTag,
              tagVocabulary.contains(where: { $0.caseInsensitiveCompare(t) == .orderedSame })
        else { return nil }
        return t
    }

    /// The rows to show. An empty query is the plain journal (newest first, no search run).
    /// A non-empty query shows the FUSED ranking, in the engine's order — not the store's —
    /// so a note that ranked first because both lanes found it appears first. The tag chip
    /// then narrows either list.
    private var rows: [Note] {
        let base: [Note]
        if query.trimmingCharacters(in: .whitespaces).isEmpty {
            base = store.notes
        } else {
            let byID = Dictionary(uniqueKeysWithValues: store.notes.map { ($0.id.uuidString, $0) })
            base = outcome.results.compactMap { byID[$0.id] }
        }
        guard let tag = activeTag else { return base }
        return base.filter { n in n.tags.contains { $0.name.caseInsensitiveCompare(tag) == .orderedSame } }
    }

    /// The searchable journal list. Extracted from `body` because as one inline
    /// expression the List builder — chips, entity chips, an error row, the rows and
    /// the count line — exceeded the type-checker's budget.
    @ViewBuilder private var journal: some View {
        SearchableList(text: $query) {
            List {
                if !tagVocabulary.isEmpty {
                    TagFilterChips(vocabulary: tagVocabulary, selected: activeTag) {
                        selectedTag = $0
                    }
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
                    .blocksPageSwipe()
                }
                if !outcome.entities.isEmpty {
                    EntityChips(entities: outcome.entities)
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                        .blocksPageSwipe()
                }
                if let searchError {
                    Text(searchError)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                }
                ForEach(rows) { note in
                    NavigationLink(value: note) { NoteRow(note: note) }
                        // ON THE ROW, not on the list. A drag that starts on a note
                        // is a swipe-to-delete and must not page; a drag that starts
                        // anywhere else — the search field, the empty space below the
                        // last note — is still the shell's page-swipe back to CHAT.
                        // Marking the whole list ate both, and the page-swipe off
                        // this screen died the moment it had a single note in it.
                        // The same swipe a chat and a room wear — mask, gesture and
                        // confirm in one definition (`swipeToRemove`). It replaces
                        // `.onDelete`, which gave the gesture but asked nothing: a note
                        // went the instant the swipe completed.
                        .swipeToRemove(title: deleteTitle(note)) { store.delete(note) }
                }
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
                // The count line under the list — with a tag selected it names
                // the filter, so a short list reads as "filtered", never "lost".
                if !tagVocabulary.isEmpty {
                    // Built as a plain String first: as one inline expression
                    // the concatenation of an interpolation with an
                    // Optional.map defeated the type-checker's budget.
                    Text(countLine)
                        .font(.caption)
                        .foregroundStyle(Theme.textTertiary)
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
        }
    }

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if store.notes.isEmpty {
                    emptyState
                } else {
                    journal
                }
            }
            .background(Theme.bg)
            .navigationDestination(for: Note.self) { note in
                NoteEditorView(noteID: note.id)
            }
            .accountToolbar()
            // The same top bar CHAT and LIFE carry: the universal `+` and the
            // notification bell. `+` composes — literally the same `New` session the NOTE
            // disc raises (the shell builds it), so there is one way a note gets written,
            // reached from two places.
            .tabChrome("Notes", newLabel: "New note", showsTitle: false, showsNotifications: true) {
                shell.beginNote()
            }
        }
        .onAppear { enrich.ingestNotes(store.notes) }
        // Preloaded screen: onAppear fires once at launch; a real arrival is the
        // shell's page landing here (see ShellState.screen).
        .onChange(of: shell.screen) { _, s in if s == .notes { enrich.ingestNotes(store.notes) } }
        .onChange(of: store.notes) { _, new in enrich.ingestNotes(new) }
        // Both lanes are async and one embeds the query, so search is DEBOUNCED and the
        // in-flight task is cancelled on every keystroke — typing never queues a backlog of
        // embeddings, and a slow result can never land after a newer one.
        .onChange(of: query) { _, q in runSearch(q) }
    }

    private func runSearch(_ q: String) {
        searchTask?.cancel()
        guard !q.trimmingCharacters(in: .whitespaces).isEmpty else {
            outcome = NoteSearchOutcome()
            searchError = nil
            return
        }
        let engine = NoteSearch(store: store.store, graph: graph)
        searchTask = Task {
            try? await Task.sleep(for: .milliseconds(180))
            guard !Task.isCancelled else { return }
            do {
                let result = try await engine.run(query: q)
                guard !Task.isCancelled else { return }
                outcome = result
                searchError = nil
            } catch {
                guard !Task.isCancelled else { return }
                // Loud, not silent: a failed search shows nothing rather than showing the
                // unfiltered journal, which would read as "no matches" when it is a fault.
                outcome = NoteSearchOutcome()
                searchError = "\(error.localizedDescription)"
            }
        }
    }

    /// Names the note being deleted — its title, or its first words when it has none,
    /// so the dialog is about something recognisable rather than "this item".
    private func deleteTitle(_ note: Note) -> String {
        let name = note.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let fallback = note.body.trimmingCharacters(in: .whitespacesAndNewlines).prefix(28)
        let shown = name.isEmpty ? String(fallback) : name
        return shown.isEmpty ? "Delete this note?" : "Delete \u{201c}\(shown)\u{201d}?"
    }

    private var emptyState: some View { EmptyWord("NOTE") }
}

/// The browse-by-tag chip row (Three Channels design): All first, then every tag in use,
/// frequency-first. Selection is single and explicit — the accent chip is the filter.
private struct TagFilterChips: View {
    var vocabulary: [String]
    var selected: String?
    var choose: (String?) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip("All", isOn: selected == nil) { choose(nil) }
                ForEach(vocabulary, id: \.self) { tag in
                    chip("#" + tag,
                         isOn: selected?.caseInsensitiveCompare(tag) == .orderedSame) { choose(tag) }
                }
            }
            .padding(.vertical, 2)
        }
    }

    private func chip(_ label: String, isOn: Bool, tap: @escaping () -> Void) -> some View {
        Button(action: tap) {
            Text(label)
                .font(.caption.weight(.semibold))
                .foregroundStyle(isOn ? Theme.onAccent : Theme.textSecondary)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(Capsule().fill(isOn ? Theme.accent : Theme.fill))
                .lineLimit(1)
        }
        .buttonStyle(.plain)
    }
}

/// The entities the query itself matched in the graph — the things, not the words. Tapping
/// one is not wired here; this row is the evidence that the graph lane contributed.
private struct EntityChips: View {
    var entities: [ArcEntityRef]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(entities) { e in
                    Text(e.name)
                        .font(.caption)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(Capsule().fill(Color.primary.opacity(0.08)))
                }
            }
            .padding(.vertical, 2)
        }
    }
}

struct NoteRow: View {
    var note: Note
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline) {
                Text(note.derivedTitle).font(.headline).lineLimit(1)
                Spacer(minLength: 8)
                Text(note.updated.formatted(.relative(presentation: .named)))
                    .font(.caption).foregroundStyle(.secondary)
            }
            if !note.snippet.isEmpty {
                EnrichedLabel(note.snippet, font: .subheadline, color: .secondary)
                    .lineLimit(2)
            }
            // Embedding-identified tags (NoteTagger), as hashtags per the Three Channels
            // design — text, not capsules, so a row stays quiet; capped so it stays a row.
            if !note.tags.isEmpty {
                HStack(spacing: 10) {
                    ForEach(note.tags.prefix(4), id: \.self) { tag in
                        Text("#" + tag.name)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(Theme.textSecondary)
                            .lineLimit(1)
                    }
                }
                .padding(.top, 2)
            }
        }
        .padding(.vertical, 4)
    }
}
