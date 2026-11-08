//  PlacePublishSheet.swift
//  Where a note goes when the gold disc says Share: published AT a Place, not handed
//  to the system sheet. The shelves — the Places you hold (nearest first) and the
//  Places your graph answered a discovery request with (1–2 handshakes out, cached
//  1-hop answers rendering instantly between sessions) — then the shared map-mint
//  (PlaceMintSection), which brings its own Nearby guard and stands the map on your
//  live fix.
//
//  Both shelves read ONE listing: `core.discovered` is the hops-aware merge (held
//  rows carry `holder == ""`), so held and network can never disagree about
//  ordering, search, or what "adopted" means. The sheet only CHOOSES a held place
//  id; the caller owns what publishing means (persist the note, author
//  `place.post`, leave on the blue edge). A NETWORK row is adopted here first —
//  the reference row becomes a local Place carrying the remote id as provenance —
//  so the caller always receives a place it can post into.

import SwiftUI
import MapKit
import PacificCore

struct PlacePublishSheet: View {
    /// Called with a HELD place object id once one is chosen/adopted/minted.
    let onPublish: (String) -> Void

    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var location: LocationService
    @Environment(\.dismiss) private var dismiss

    @State private var query = ""
    @State private var results: [MKMapItem] = []

    /// The query filter + nearest-first order, applied identically to both shelves.
    /// Decorate-sort-undecorate: one distance per row, not one per comparison.
    private func shelf(_ rows: [DiscoveredRow]) -> [DiscoveredRow] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        let hits = rows.filter { q.isEmpty || $0.name.lowercased().contains(q) }
        guard let here = location.coordinate else { return hits }
        return hits
            .map { ($0, $0.metres(from: here) ?? .greatestFiniteMagnitude) } // unplaced last
            .sorted { $0.1 < $1.1 }
            .map(\.0)
    }

    var body: some View {
        let held = shelf(core.discovered.filter { $0.holder.isEmpty })
        let network = shelf(core.discovered.filter { !$0.holder.isEmpty })
        NavigationStack {
            List {
                placeSection("Places", held, icon: nil) { pick($0.id) }
                placeSection("Network", network,
                             icon: "antenna.radiowaves.left.and.right") { adopt($0) }

                PlaceMintSection(results: results,
                                 onPick: { row in
                                     row.holder.isEmpty ? pick(row.id) : adopt(row)
                                 },
                                 onMinted: { id, _ in pick(id) })
            }
            .searchable(text: $query, prompt: "Search places")
            .onSubmit(of: .search) {
                Task { results = await PlaceSearch.nearby(query, near: location.coordinate) }
            }
            .navigationTitle("Publish")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
        .onAppear {
            Task { await core.discoverPlaces() }   // ask the graph; answers stream in below
        }
        // Fires once on subscribe (the @Published current value) — the initial read —
        // then again on every sync tick while the sheet is up.
        .onReceive(core.$syncTick) { _ in core.refreshDiscovered() }
    }

    /// One shelf: label-only rows, hidden entirely when empty.
    @ViewBuilder
    private func placeSection(_ title: String, _ rows: [DiscoveredRow],
                              icon: String?, action: @escaping (DiscoveredRow) -> Void) -> some View {
        if !rows.isEmpty {
            Section(title) {
                ForEach(rows, id: \.id) { row in
                    Button { action(row) } label: {
                        HStack {
                            Image(systemName: icon
                                ?? (row.placed ? "mappin.circle.fill" : "mappin.circle"))
                                .foregroundStyle(Color.secondary)
                            Text(row.name)
                                .foregroundStyle(Color.primary)
                        }
                    }
                }
            }
        }
    }

    // MARK: Actions

    private func pick(_ id: String) {
        onPublish(id)
        dismiss()
    }

    /// A NETWORK row publishes via adoption: the reference becomes a held Place
    /// (once — the remote id is the dedup key) and the note posts into that.
    private func adopt(_ row: DiscoveredRow) {
        guard let id = core.adoptPlace(row) else { return }
        pick(id)
    }
}
