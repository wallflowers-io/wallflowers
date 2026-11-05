//  EventPlacePicker.swift
//  The venue chooser behind an Event's place row: pick one of the Places you already
//  hold, take a NEARBY known/cached place (yours, or one the graph answered a
//  discovery request with — the shared mint section surfaces both within a
//  kilometre), or mint a NEW Place from Apple's map through the same
//  `PlaceMintSection` the publish sheet uses (durable, signed, shareable — never a
//  device-local row).
//
//  The picker only CHOOSES; what the choice means belongs to the caller. The detail
//  page authors `event.setVenue` (the happens_at edge) with it; the composer holds it
//  until the Event exists. One search field drives both halves: it filters the held
//  Places and, on submit, asks MapKit for the rest of the world.

import SwiftUI
import MapKit
import PacificCore

struct EventPlacePicker: View {
    /// The currently referenced Place object id ("" / nil = none) — shows Remove.
    var current: String? = nil
    /// (placeId, name). ("", "") = remove the reference.
    let onSelect: (String, String) -> Void

    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var location: LocationService
    @Environment(\.dismiss) private var dismiss

    @State private var query = ""
    @State private var results: [MKMapItem] = []

    private var held: [PlaceRow] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        return core.places.filter { q.isEmpty || $0.name.lowercased().contains(q) }
    }

    var body: some View {
        NavigationStack {
            List {
                if let current, !current.isEmpty {
                    Button(role: .destructive) { pick("", "") } label: {
                        Label("Remove", systemImage: "mappin.slash")
                    }
                }

                if !held.isEmpty {
                    Section("Places") {
                        ForEach(held, id: \.id) { row in
                            Button { pick(row.id, row.name) } label: {
                                HStack {
                                    Image(systemName: row.placed ? "mappin.circle.fill" : "mappin.circle")
                                        .foregroundStyle(row.id == current ? Color.accentColor : Color.secondary)
                                    Text(row.name)
                                        .foregroundStyle(Color.primary)
                                }
                            }
                        }
                    }
                }

                PlaceMintSection(results: results,
                                 onPick: { row in pickDiscovered(row) },
                                 onMinted: { id, name in pick(id, name) })
            }
            .searchable(text: $query, prompt: "Search places")
            .onSubmit(of: .search) {
                Task { results = await PlaceSearch.nearby(query, near: location.coordinate) }
            }
            .navigationTitle("Venue")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
        .onAppear {
            core.refreshPlaces()
            // Render the discovery CACHE (a pure fold read — the venue picker reads
            // what is known; it does not put a fresh question on the wire).
            core.refreshDiscovered()
        }
    }

    // MARK: Actions

    private func pick(_ id: String, _ name: String) {
        onSelect(id, name)
        dismiss()
    }

    /// A nearby row: yours is picked directly; a peer's is adopted first (the
    /// reference becomes a held Place, once), then picked like any other.
    private func pickDiscovered(_ row: DiscoveredRow) {
        if row.holder.isEmpty {
            pick(row.id, row.name)
        } else if let id = core.adoptPlace(row) {
            pick(id, row.name)
        }
    }
}
