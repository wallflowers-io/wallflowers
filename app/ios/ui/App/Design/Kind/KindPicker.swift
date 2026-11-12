//  KindPicker.swift
//  Choose a profile or object to point at: the people you are connected to, the Spaces
//  you are in. Only what this device holds, so whatever is picked can be named and
//  opened. One picker for every surface that points at one (a lineup's acts, a tag's
//  object): a new source is one more case.

import SwiftUI
import PacificCore

struct KindPicker: View {
    enum Source: Hashable { case people, spaces }

    struct Pick: Identifiable, Hashable {
        let id: String
        let name: String
        let source: Source
    }

    let title: String
    let sources: [Source]
    /// Already chosen: not offered again.
    var exclude: Set<String> = []
    let onPick: (Pick) -> Void

    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""

    private var picks: [Pick] {
        var out: [Pick] = []
        for s in sources {
            switch s {
            case .people:
                out += core.connectionRows().filter(\.connected).map {
                    Pick(id: $0.peerId, name: $0.displayName.isEmpty ? String($0.peerId.prefix(8)) : $0.displayName,
                         source: .people)
                }
            case .spaces:
                out += core.spaces.map {
                    Pick(id: $0.id, name: $0.name.isEmpty ? String($0.id.prefix(8)) : $0.name, source: .spaces)
                }
            }
        }
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        return out.filter { !exclude.contains($0.id) && (q.isEmpty || $0.name.lowercased().contains(q)) }
    }

    var body: some View {
        NavigationStack {
            List(picks) { p in
                Button {
                    onPick(p)
                    dismiss()
                } label: {
                    HStack(spacing: 12) {
                        Avatar(initials: Avatar.initials(p.name), tint: Theme.fill, size: 32,
                               photo: core.index.photo(p.id), color: core.index.color(p.id))
                        Text(p.name).foregroundStyle(Theme.textPrimary)
                        Spacer()
                        Text(p.source == .people ? "PERSON" : "SPACE")
                            .font(.system(size: 11, weight: .semibold, design: .monospaced))
                            .foregroundStyle(Theme.textTertiary)
                    }
                }
                .accessibilityIdentifier("kind-pick")
            }
            .searchable(text: $query)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }
        .accessibilityIdentifier("kind-picker")
    }
}
