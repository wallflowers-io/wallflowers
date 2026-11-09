//  PlacesMap.swift
//  The TOP-DOWN view of the Map component — the Instagram-Map-shaped overview that
//  loads when the PLACES selector is maximised and no specific Place is chosen.
//
//  Design (from the Instagram Map case study):
//    • Full-bleed portrait map; controls float in the thumb zone.
//    • Content-as-pin: each pin wears the PLACES jewel colour + the entity's glyph,
//      never a generic dot.
//    • Tap a pin → the bottom detail sheet — the QUICK resolution. From it,
//      "Walk up →" hands off to the POV (the pin's old direct door), and the
//      identity row taps THROUGH to the Place's maximised detail (`PlaceDetail`,
//      the deep resolution).
//    • Honest-absent: places with no plottable coordinate are simply not pinned. An
//      empty map is an empty map — it carries no caption explaining itself.
//
//  iOS 17+ MapKit (SwiftUI `Map` + `Annotation`). Clustering at scale is a follow-on
//  (an `MKMapView` `UIViewRepresentable` with `clusteringIdentifier`, per the research);
//  for a personal graph the annotation count is small, so v1 draws pins directly.

import SwiftUI
import MapKit

struct PlacesMap: View {
    let places: [PlaceAnnotation]
    /// Commitments overlaid as pins — THE map is the one map (ruling of 1 Aug), so when
    /// the EVENTS dimension is locked its located commitments show up here rather than
    /// on a second map of their own. One pin per commitment, wearing the EVENTS tint;
    /// same-venue commitments stack, which is honest — they are in the same place.
    var events: [PlaceAnnotation] = []
    /// The POV hand-off — the sheet's "Walk up" calls this (Life presents
    /// PlacePOVView full-screen, exactly as the pin used to directly).
    var onOpen: (PlaceAnnotation) -> Void = { _ in }

    @EnvironmentObject private var location: LocationService

    @State private var camera: MapCameraPosition = .userLocation(followsHeading: true, fallback: .automatic)
    @State private var selectedID: String?
    /// The place whose detail sheet is up — set by a pin tap, cleared by ✕.
    @State private var sheetPlace: PlaceAnnotation?

    private var tint: Color { LifeFilter.places.color }

    var body: some View {
        ZStack(alignment: .bottom) {
            // No .pitch — this is the top-down map, and a tilted camera breaks the
            // rotate-with-device heading follow. Pan / zoom / rotate stay.
            Map(position: $camera, interactionModes: [.pan, .zoom, .rotate], selection: $selectedID) {
                ForEach(places) { place in
                    Annotation(place.name, coordinate: place.coordinate, anchor: .bottom) {
                        PlacePin(place: place, tint: tint, selected: place.id == selectedID)
                    }
                    .annotationTitles(.hidden)   // the pin draws its own label
                    .tag(place.id)
                }
                // Event pins are OVERLAY, not destination: no tag, so selecting one never
                // fires the POV path (a commitment has no POV — its venue does). MapKit
                // prints the commitment's title beneath the pin.
                ForEach(events) { ev in
                    Annotation(ev.name, coordinate: ev.coordinate, anchor: .bottom) {
                        PlacePin(place: ev, tint: EventsSeed.tint(.events), selected: false)
                    }
                }
                UserAnnotation()   // "you are here" — the blue dot, once location is authorised
            }
            .mapStyle(.standard(elevation: .flat,
                                pointsOfInterest: .excludingAll,
                                showsTraffic: false))
            .mapControlVisibility(.hidden)       // no default chrome; we float our own
            .padding(.bottom, -64)               // crop the Apple Maps logo off the bottom edge
            .ignoresSafeArea()
            .accessibilityIdentifier("places-map")

            // The bottom detail sheet — the pin's quick resolution (thumb zone).
            if let place = sheetPlace {
                PlaceDetailSheet(place: place, tint: tint,
                                 onClose: {
                                     withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                                         sheetPlace = nil
                                     }
                                     selectedID = nil
                                 },
                                 onWalkUp: { onOpen(place) })
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        // THE MAP'S PERMISSION, ASKED ON THE MAP. `UserAnnotation` above is the blue
        // dot this buys, so the alert lands with the thing it is for on screen —
        // rather than at launch, in front of a user who has not yet seen a map.
        // Repeat visits are a no-op: iOS shows the alert
        // only while the status is `.notDetermined`.
        .onAppear { location.request() }
        // The map's heading follows the device bearing; tapping the bearing readout locks it,
        // decoupling the map's rotation from the device (north-up).
        .onChange(of: location.headingLocked) { _, locked in
            camera = .userLocation(followsHeading: !locked, fallback: .automatic)
        }
        // A tap selects a pin and raises its detail sheet — the quick resolution.
        // The selection stays while the sheet is up (the pin reads as chosen);
        // ✕ or a tap on bare map clears both, so the same pin can be tapped again.
        .onChange(of: selectedID) { _, id in
            withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                sheetPlace = id.flatMap { id in places.first { $0.id == id } }
            }
        }
    }

    // MARK: Chrome

    // MARK: Camera

    /// Frame the camera to enclose every pin (with padding), or a sensible default when
    /// empty. Kept device-relative via `MapCameraPosition.region`.
    private func frameToPins() {
        guard !places.isEmpty else {
            camera = .automatic
            return
        }
        let lats = places.map(\.coordinate.latitude)
        let lngs = places.map(\.coordinate.longitude)
        let minLat = lats.min()!, maxLat = lats.max()!
        let minLng = lngs.min()!, maxLng = lngs.max()!
        let center = CLLocationCoordinate2D(latitude: (minLat + maxLat) / 2,
                                            longitude: (minLng + maxLng) / 2)
        // 1.6× padding, with a floor so a single pin isn't zoomed to the rooftop.
        let span = MKCoordinateSpan(latitudeDelta: max((maxLat - minLat) * 1.6, 0.02),
                                    longitudeDelta: max((maxLng - minLng) * 1.6, 0.02))
        withAnim { camera = .region(MKCoordinateRegion(center: center, span: span)) }
    }

    private func withAnim(_ body: () -> Void) {
        withAnimation(.spring(response: 0.4, dampingFraction: 0.85), body)
    }
}

// MARK: - The pin

/// A place pin: a teardrop disc in the PLACES colour carrying the entity's glyph, its
/// name on a chip beneath. A live (stream-backed) place gets a pulsing ring — "here
/// now" reads differently from "a place that is here" (the IG was-here/is-here lesson).
private struct PlacePin: View {
    let place: PlaceAnnotation
    let tint: Color
    let selected: Bool

    var body: some View {
        VStack(spacing: 3) {
            ZStack {
                if place.isLive {
                    Circle().stroke(tint.opacity(0.5), lineWidth: 2)
                        .frame(width: 44, height: 44)
                }
                Circle()
                    .fill(tint)
                    .frame(width: selected ? 40 : 30, height: selected ? 40 : 30)
                    .overlay(Circle().strokeBorder(.white, lineWidth: 2))
                    .shadow(color: .black.opacity(0.35), radius: 3, y: 1)
                Image(systemName: place.glyph)
                    .font(.system(size: selected ? 18 : 14, weight: .bold))
                    .foregroundStyle(.white)
            }
            if selected {
                Text(place.name)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 7).padding(.vertical, 3)
                    .background(.ultraThinMaterial, in: Capsule())
                    .fixedSize()
            }
        }
        // One element per pin, labelled by the place — an unselected pin is a
        // glyph disc with no text, so without this it is invisible to the
        // accessibility tree (and to the map UITest's coordinate tap).
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(place.name)
        .accessibilityIdentifier("map-pin:\(place.id)")
    }
}

// MARK: - The bottom detail sheet

/// A place's card, anchored to the bottom (thumb zone) — the pin's QUICK
/// resolution. The identity row taps THROUGH to the Place's maximised detail
/// (`.pacificOpenPlace`, the deep resolution); "Walk up" hands off to the POV,
/// the door the pin used to open directly.
private struct PlaceDetailSheet: View {
    let place: PlaceAnnotation
    let tint: Color
    var onClose: () -> Void
    var onWalkUp: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Button {
                    NotificationCenter.default.post(name: .pacificOpenPlace, object: nil,
                                                    userInfo: ["object": place.id])
                } label: {
                    HStack(spacing: 10) {
                        Circle().fill(tint).frame(width: 34, height: 34)
                            .overlay(Image(systemName: place.glyph).font(.system(size: 15, weight: .bold)).foregroundStyle(.white))
                        VStack(alignment: .leading, spacing: 1) {
                            Text(place.name).font(.system(size: 16, weight: .semibold)).foregroundStyle(.primary)
                            if let category = place.category, !category.isEmpty {
                                Text(category).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
                            }
                        }
                        Spacer(minLength: 0)
                        Image(systemName: "chevron.right")
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(.tertiary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("place-sheet-open")
                Button(action: onClose) {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 22)).foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("place-sheet-close")
            }
            Button(action: onWalkUp) {
                HStack(spacing: 8) {
                    Image(systemName: "figure.walk")
                        .font(.system(size: 13, weight: .semibold))
                    Text("Walk up")
                        .font(.system(size: 13, weight: .semibold))
                    Spacer()
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 12, weight: .semibold))
                }
                .foregroundStyle(.primary)
                .padding(.horizontal, 12).padding(.vertical, 9)
                .background(tint.opacity(0.22), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("place-sheet-walkup")
        }
        .padding(16)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 20).strokeBorder(Theme.hairline, lineWidth: 1))
        .padding(.horizontal, 12)
        .padding(.bottom, ShellMetrics.bottomBarHeight + 12)   // sit ABOVE the NOTE·CHAT·LIFE bar
        // Its own element, so its id does not overwrite the buttons' ids inside it.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("place-sheet")
    }
}

// MARK: - Preview (seeded — the family's world from the running example)

#Preview("Places map — seeded") {
    let seed: [PlaceAnnotation] = [
        .init(id: "bristol", name: "Bristol",  coordinate: .init(latitude: 51.4545, longitude: -2.5879), category: "John · Hope"),
        .init(id: "stroud",  name: "Stroud",   coordinate: .init(latitude: 51.7449, longitude: -2.2158), category: "Mum · Dad"),
        .init(id: "london",  name: "London",   coordinate: .init(latitude: 51.5074, longitude: -0.1278), category: "Ralph"),
        .init(id: "wales",   name: "Wales",    coordinate: .init(latitude: 51.9481, longitude: -3.3918), category: "the trip"),
        .init(id: "bluecar", name: "Blue car", coordinate: .init(latitude: 51.60,   longitude: -2.40),   category: "en route", isLive: true, glyph: "car.fill"),
    ]
    return PlacesMap(places: seed)
        .environmentObject(LocationService())
        .preferredColorScheme(.dark)
}

#Preview("Places map — empty") {
    PlacesMap(places: [])
        .environmentObject(LocationService())
        .preferredColorScheme(.dark)
}
