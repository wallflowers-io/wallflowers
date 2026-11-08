//  PlaceMintSection.swift
//  THE map-mint: the "New place" experience both choosers share (PlacePublishSheet,
//  EventPlacePicker) — extracted so the two can never diverge on what minting knows.
//
//  What it knows, beyond the pin-and-name of the original section:
//    • WHERE YOU STAND — the map opens on your live fix and draws it
//      (`UserAnnotation`), so a dropped pin is a choice relative to you, not to a
//      continent-level default camera;
//    • WHAT ALREADY EXISTS — a NEARBY group appears above the mint whenever
//      known/cached Places (held, or a peer's answer from the discovery cache) sit
//      within a kilometre of you. Minting a duplicate of a place the graph already
//      holds should take deliberate effort, not be the path of least resistance.
//
//  The section only CHOOSES or MINTS; what that means belongs to the caller
//  (`onPick` an existing row — adopt-then-use for a network row — or `onMinted` a
//  fresh id). Search stays with the parent: `.searchable` owns the field, the
//  parent resolves it through `PlaceSearch.nearby` and hands the results in.

import SwiftUI
import MapKit
import CoreLocation
import PacificCore

extension DiscoveredRow {
    /// Metres from `here`, or nil for an unplaced row (no point, no distance).
    func metres(from here: CLLocationCoordinate2D) -> Double? {
        guard placed else { return nil }
        return CLLocation(latitude: here.latitude, longitude: here.longitude)
            .distance(from: CLLocation(latitude: lat, longitude: lng))
    }
}

struct PlaceMintSection: View {
    /// How far "right here" reaches: a kilometre — the walk you'd actually make.
    static let nearbyMetres: Double = 1_000

    /// MapKit results the parent's search field resolved (may be empty).
    let results: [MKMapItem]
    /// An existing row was chosen instead of minting.
    let onPick: (DiscoveredRow) -> Void
    /// A fresh Place was minted: (object id, name).
    let onMinted: (String, String) -> Void

    @EnvironmentObject private var core: CoreService
    @EnvironmentObject private var location: LocationService

    @State private var name = ""
    @State private var pin: CLLocationCoordinate2D?
    @State private var pinAddress = ""
    @State private var camera: MapCameraPosition = .automatic

    /// Known/cached Places within a kilometre of where you stand, nearest first —
    /// held rows and the discovery cache together (`core.discovered` is the one
    /// hops-aware merge). Empty when there is no fix: without knowing where you
    /// are, "nearby" would be a guess dressed as a fact.
    private var nearby: [(row: DiscoveredRow, metres: Double)] {
        guard let here = location.coordinate else { return [] }
        return core.discovered
            .compactMap { row in row.metres(from: here).map { (row, $0) } }
            .filter { $0.1 <= Self.nearbyMetres }
            .sorted { $0.1 < $1.1 }
    }

    private var mintable: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty && pin != nil
    }

    var body: some View {
        if !nearby.isEmpty {
            Section("Nearby") {
                ForEach(nearby, id: \.row.id) { entry in
                    Button { onPick(entry.row) } label: {
                        HStack {
                            Image(systemName: entry.row.holder.isEmpty
                                ? "mappin.circle.fill"
                                : "antenna.radiowaves.left.and.right")
                                .foregroundStyle(Color.secondary)
                            Text(entry.row.name)
                                .foregroundStyle(Color.primary)
                            Spacer()
                            Text(Self.distanceLabel(entry.metres))
                                .foregroundStyle(Color.secondary)
                                .monospacedDigit()
                        }
                    }
                }
            }
        }

        Section("New place") {
            ForEach(Array(results.enumerated()), id: \.offset) { _, item in
                Button { adopt(item) } label: {
                    Label(item.name ?? "—", systemImage: "magnifyingglass")
                        .foregroundStyle(Color.primary)
                }
            }
            MapReader { proxy in
                Map(position: $camera) {
                    UserAnnotation()   // where you stand, live
                    if let pin {
                        Marker(name.isEmpty ? "Here" : name, coordinate: pin)
                    }
                }
                .frame(height: 220)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .onTapGesture { tap in
                    guard let coord = proxy.convert(tap, from: .local) else { return }
                    pin = coord
                    pinAddress = ""
                }
            }
            .listRowInsets(EdgeInsets())
            TextField("Name", text: $name)
            Button { mint() } label: {
                Label("Mint", systemImage: "plus.circle.fill")
            }
            .disabled(!mintable)
        }
        .onAppear {
            // Open on where you STAND, not on the map's idea of the world. Only
            // until a pin exists — a chosen spot must never be yanked back.
            if pin == nil, let here = location.coordinate {
                camera = .region(MKCoordinateRegion(center: here,
                                                    latitudinalMeters: 1_200,
                                                    longitudinalMeters: 1_200))
            }
        }
    }

    /// "80 m" · "450 m" · "1.0 km" — one significant step per band.
    static func distanceLabel(_ metres: Double) -> String {
        if metres < 100 { return "\(Int(metres.rounded())) m" }
        if metres < 1_000 { return "\(Int((metres / 10).rounded()) * 10) m" }
        return String(format: "%.1f km", metres / 1_000)
    }

    // MARK: Actions

    /// A search result becomes the pending pin: MapKit names it, the map shows it.
    private func adopt(_ item: MKMapItem) {
        let coord = item.placemark.coordinate
        pin = coord
        if name.trimmingCharacters(in: .whitespaces).isEmpty { name = item.name ?? "" }
        pinAddress = item.placemark.title ?? ""
        withAnimation {
            camera = .region(MKCoordinateRegion(center: coord,
                                                latitudinalMeters: 800, longitudinalMeters: 800))
        }
    }

    /// Mint the Place GroupObject at the pin. The descriptor carries the address
    /// MapKit gave us — the honest provenance of a map-picked pin (contrast
    /// PlaceMint's stood-here fix).
    private func mint() {
        guard let pin else { return }
        let title = name.trimmingCharacters(in: .whitespaces)
        guard let id = core.mintPlace(name: title,
                                      descriptor: pinAddress,
                                      placed: true,
                                      lat: pin.latitude,
                                      lng: pin.longitude) else { return }
        onMinted(id, title)
    }
}
