//  FeedView.swift
//  The Active Scroll Feed — LIFE's ground state (FEED-TRANSMISSION 0-ANSWER-C).
//  Monochrome; the orb is the one colour carve-out on this surface. Rows are
//  label-or-value. Provenance: filled = member fold, dotted ring = arrived over hops.

import SwiftUI
import UIKit

struct FeedView: View {
    @ObservedObject var store: FeedStore
    /// TAP = INSTANCE DETAIL (the object's own page), routed by kind at the call
    /// site — the uniform card contract (see ObjectCard). Kind-browsing is the rail's.
    var onOpen: (FeedItem) -> Void
    var onAuthor: ((AuthorRoute) -> Void)? = nil
    /// The LIFE search text. Empty = the feed is itself; matching is over what a card
    /// SHOWS — title, author, place — because that is what a person is scanning for
    /// when they start typing at a list they can already see.
    var query: String = ""

    private var shown: [FeedItem] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return store.rows }
        return store.rows.filter {
            $0.title.lowercased().contains(q) || $0.author.lowercased().contains(q)
                || $0.location.lowercased().contains(q)
        }
    }

    var body: some View {
        ScrollView {
            // CHRONOLOGICAL (0-ANSWER-G refinement 2): soonest first, scrolling down
            // moves forward through time. Square cards, edge to edge — the
            // Instagram/Vine grammar on the one card component.
            LazyVStack(spacing: DesignTokens.Feed.stackSpacing) {
                ForEach(Array(shown.enumerated()), id: \.element.id) { index, row in
                    if bandIndex == index {
                        unreadBand
                    }
                    ObjectCard(row: row, onAuthor: onAuthor)
                        .contentShape(Rectangle())
                        .onTapGesture { onOpen(row) }
                        // One addressable element per card, or the row identifier
                        // is invisible to XCUITest — the RibbonRail jewels learned
                        // the same lesson (a row nothing can query is a row nothing
                        // can regression-test). `.combine` keeps the texts readable.
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("feed-row-\(row.id)")
                }
                if shown.isEmpty {
                    Text("—")
                        .foregroundStyle(.secondary)
                        .padding(.top, 120)
                        .accessibilityIdentifier("feed-empty")
                }
            }
            .padding(.top, RibbonWidth.contracted + DesignTokens.Feed.topGap)
        }
        .accessibilityIdentifier("life-feed")
        .onDisappear { store.markVisited() }
    }

    /// Index of the first NEW row — the band sits above the fresh stretch.
    private var bandIndex: Int? {
        store.rows.firstIndex(where: { store.ledger.isNew($0) })
    }

    private var unreadBand: some View {
        HStack(spacing: 8) {
            Rectangle().frame(height: 1).opacity(0.15)
            Text("NEW").font(.system(size: 9, weight: .semibold)).tracking(2)
                .foregroundStyle(.secondary)
            Rectangle().frame(height: 1).opacity(0.15)
        }
        .padding(.horizontal, DesignTokens.Life.gutter)
        .padding(.vertical, 4)
        .accessibilityIdentifier("feed-unread-band")
    }
}

// MARK: - The white orb

/// The reset jewel: FLAT white circle (bloom removed, 10 Aug ruling). Press = show
/// the feed. `currentID` is retained for the eventual vector-derived treatment
/// (FEED-TRANSMISSION §9-F) — unused while the face is flat.
struct FeedOrb: View {
    let currentID: String?
    var size: CGFloat = 30
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            Circle()
                .fill(Theme.accent)
                .frame(width: size, height: size)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("feed-orb")
    }
}

// MARK: - Modes Config (rail drag-left)

struct ModesConfigSheet: View {
    @ObservedObject var modes: ModeStore
    var capture: () -> FeedMode
    var apply: (FeedMode) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""

    var body: some View {
        NavigationStack {
            List {
                Section {
                    HStack {
                        TextField("Name", text: $name)
                            .accessibilityIdentifier("mode-name-field")
                        Button("Save") {
                            let trimmed = name.trimmingCharacters(in: .whitespaces)
                            guard !trimmed.isEmpty else { return }
                            modes.save(capture(), as: trimmed)
                            name = ""
                        }
                        .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                        .accessibilityIdentifier("mode-save")
                    }
                }
                Section {
                    ForEach(modes.names.filter { $0 != "active" }, id: \.self) { n in
                        Button(n) {
                            if let m = modes.load(n) { apply(m); dismiss() }
                        }
                        .accessibilityIdentifier("mode-\(n)")
                    }
                    .onDelete { idx in
                        let visible = modes.names.filter { $0 != "active" }
                        for i in idx { modes.delete(visible[i]) }
                    }
                }
            }
            .navigationTitle("Modes")
            .navigationBarTitleDisplayMode(.inline)
        }
        .presentationDetents([.medium])
    }
}
