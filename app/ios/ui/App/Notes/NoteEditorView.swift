//  NoteEditorView.swift
//  A single note: a plain editor (title + TextEditor) that toggles to an ENRICHED
//  read view (EnrichedText) where matched entities bolden + colour and long-press
//  opens the info panel. New/empty notes open straight into edit mode.

import SwiftUI

struct NoteEditorView: View {
    @EnvironmentObject private var store: NotesStore
    let noteID: UUID

    @State private var title = ""
    @State private var bodyText = ""
    @State private var editing = true
    @State private var didLoad = false
    @FocusState private var focus: Field?
    private enum Field { case title, body }

    private var note: Note? { store.notes.first { $0.id == noteID } }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if editing {
                TextField("Title", text: $title)
                    .font(.title2.bold())
                    .focused($focus, equals: .title)
                    .padding([.horizontal, .top])
                TextEditor(text: $bodyText)
                    .focused($focus, equals: .body)
                    .font(.body)
                    .scrollContentBackground(.hidden)
                    .padding(.horizontal, 12)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        if !title.trimmingCharacters(in: .whitespaces).isEmpty {
                            Text(title).font(.title2.bold())
                        }
                        if bodyText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            Text("Empty note").foregroundStyle(.secondary)
                        } else {
                            EnrichedText(bodyText, font: .preferredFont(forTextStyle: .body), selectable: true)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
                }
            }
        }
        .background(Brand.groupBG)
        .navigationTitle(editing ? "Edit" : "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(editing ? "Done" : "Edit") {
                    withAnimation { editing.toggle() }
                    if !editing { focus = nil; save() }
                }
            }
        }
        .onAppear {
            guard !didLoad, let n = note else { return }
            title = n.title
            bodyText = n.body
            editing = n.title.isEmpty && n.body.isEmpty
            didLoad = true
        }
        .onChange(of: title) { _, _ in if didLoad { save() } }
        .onChange(of: bodyText) { _, _ in if didLoad { save() } }
        .onDisappear {
            save()
            if let n = note { store.discardIfEmpty(n) }
        }
    }

    private func save() {
        guard var n = note else { return }
        n.title = title
        n.body = bodyText
        store.update(n)
    }
}
