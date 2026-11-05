//  PlaceProfilePage.swift
//  The Place Profile — the deep page behind a map pin (T4 of the Profile build).
//
//  A Place's "profile" used to be the map's bottom sheet and nothing else. This
//  page is the located-Space profile from the component spec:
//  headline 2.5:0.6 (icon inline-left; block-colour banner — HONEST: Places have
//  no banner op, §5 pending ruling) → location card 2.5:0.8 (map texture, area
//  cells derived CLIENT-SIDE from the Place's own GeoPoint) → info/access/land
//  cards (PlacePublicSheet's statements re-expressed in the card grammar, terse)
//  → the walk-up bar (PlacePOVView — the existing pin hand-off, from the page)
//  → the posts lane.
//
//  Everything identity-shaped reuses Design/Profile's atoms (HeadlineCard,
//  MapCard, InfoCard, PinBar, ProfileGeometry) — never a re-implementation. The
//  page reads the REAL folded PlaceRow off CoreService (the PlacePublicSheet
//  pattern); only the posts lane rides the injected `ProfileSource` (the fixture
//  binder until T5b swaps the real place.post read in behind the same seam —
//  `placePosts` exists at the FFI but CoreService, owned by T5, does not surface
//  it yet, and `ProfileSource` has no posts() method: the lane rides `pins()`).
//
//  AREA CELLS — the precision ruling. Cells are derived at geohash precision 6
//  (~1.22 × 0.61 km) from Geohash.encode + neighbours, never persisted:
//    • the 3×3 patch (~3.7 × 1.8 km) reads as an AREA — the card is texture,
//      not cartography; the venue-encode precision (9, ~4.8 m) would draw a
//      rooftop and claim survey accuracy a card render cannot honour;
//    • p6 cells are 2:1, so the patch sits naturally in the 2.5:0.8 card and
//      the 6-char labels fit their cells;
//    • 6 is PlaceFix's own coarsest honest bucket (accuracy > 150 m → p6): the
//      card never claims tighter than the loosest fix the mint pipe accepts;
//    • the anti-triangulation doctrine floors PERSISTED cells at 3..=5 — these
//      cells are client-side only, from the Place's own declared point, which
//      is its own disclosure.

import SwiftUI
import CoreLocation
import PacificCore   // PlaceRow — the folded Place GroupObject off the FFI

struct PlaceProfilePage: View {
    let placeID: String
    /// The posts lane's binder. Fixture by default (dies at M1 with the fixture
    /// delete); T5b injects the real binder behind the same protocol.
    private let source: any ProfileSource

    init(placeID: String, source: (any ProfileSource)? = nil) {
        self.placeID = placeID
        self.source = source ?? FixtureProfileSource.place(placeID)
    }

    @EnvironmentObject private var core: CoreService
    @State private var collapsed = false
    @State private var walkingUp = false
    @State private var lane: [ProfilePin] = []

    /// The folded truth, read live so a set reflects immediately (the
    /// PlacePublicSheet pattern). nil = gone/unreadable → the page shows
    /// nothing rather than a fabricated shell (the ProfileSource contract).
    private var row: PlaceRow? { core.places.first { $0.id == placeID } }

    private var tint: Color { LifeFilter.places.color }

    var body: some View {
        GeometryReader { geo in
            let width = max(0, geo.size.width - 32)   // near-full-width cards, 16pt gutters
            ScrollView {
                VStack(spacing: 12) {
                    if let row {
                        HeadlineCard(headline: headline(row), collapsed: collapsed, width: width)
                            .contentShape(ProfileGeometry.shape)
                            .onTapGesture {
                                withAnimation(ProfileGeometry.spring) { collapsed.toggle() }
                            }

                        // The location card, directly below the headline — cells
                        // from the Place's own point. Unplaced → no card (honest).
                        let cells = Self.areaCells(lat: row.lat, lng: row.lng, placed: row.placed)
                        if !cells.isEmpty {
                            MapCard(cells: cells, color: tint, width: width)
                        }

                        if !row.descriptor.isEmpty {
                            InfoCard(info: ProfileInfo(desc: row.descriptor))
                        }

                        accessCard(row, width: width)

                        if !row.landParcel.isEmpty {
                            landCard(row, width: width)
                        }

                        if row.placed {
                            walkUpBar(width: width)
                        }

                        // The posts lane — place.post rows, fixture-fed until T5b.
                        ForEach(lane) { pin in
                            PinBar(pin: pin, fallback: tint, width: width)
                        }
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
            }
            .background(Theme.bg)
        }
        .task { lane = await source.pins() }
        .onAppear { core.refreshPlaces() }
        .fullScreenCover(isPresented: $walkingUp) {
            if let row, row.placed {
                PlacePOVView(focus: annotation(row),
                             neighbours: neighbours(of: row),
                             avatarColor: core.avatarColor,
                             onExit: { walkingUp = false })
            }
        }
        .accessibilityIdentifier("place-profile")
    }

    // MARK: Headline — the Space shape, honest banner

    private func headline(_ row: PlaceRow) -> ProfileHeadline {
        ProfileHeadline(kind: .space,
                        name: row.name,
                        initials: Avatar.initials(row.name),
                        banner: .block,   // Places have no banner op — block colour, honestly
                        color: tint,
                        color2: tint.darkened(0.45))
    }

    // MARK: Access — the doorbell/access/visibility statements as one card

    /// PlacePublicSheet's dials, re-expressed as terse stats. This card REPORTS;
    /// the controls stay on the sheet (reached from the POV chrome).
    private func accessCard(_ row: PlaceRow, width: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            if !row.address.isEmpty {
                labelled("address", row.address)
            } else if !row.label.isEmpty {
                labelled("address", row.label)
            }
            HStack(spacing: 18) {
                ProfileStat(dot: row.access == "public" ? Theme.ok : nil,
                            value: row.access == "public" ? "PUBLIC" : "PRIVATE",
                            label: "access")
                ProfileStat(dot: nil,
                            value: row.doorbell.isEmpty ? "NONE" : "FITTED",
                            label: "doorbell")
                ProfileStat(dot: nil,
                            value: visibilityValue(row.visibility),
                            label: "visible to")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(18)
        .frame(width: width)
        .background(Theme.surface)
        .clipShape(ProfileGeometry.shape)
        .overlay(ProfileGeometry.shape.strokeBorder(Theme.hairline))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("place-access")
    }

    private func visibilityValue(_ v: String) -> String {
        switch v {
        case "connections": return "CONNECTIONS"
        case "network":     return "NETWORK"
        default:            return "MEMBERS"
        }
    }

    // MARK: Land — whose land, and whether they were asked (two separate claims)

    /// Rendered only when somebody has actually looked (`landParcel` non-empty) —
    /// an absent card is an absent claim, never a "—".
    private func landCard(_ row: PlaceRow, width: CGFloat) -> some View {
        let (permission, tone): (String, Color) = switch row.landPermission {
        case "granted": ("OWNER AGREED", Theme.ok)
        case "asked":   ("ASKED", .orange)
        case "refused": ("REFUSED", .red)
        default:        ("NOBODY ASKED", .orange)
        }
        return VStack(alignment: .leading, spacing: 8) {
            labelled("parcel", row.landParcel)
            if !row.landProprietor.isEmpty {
                labelled("owner", row.landProprietor)
            }
            if !row.landSource.isEmpty {
                labelled("source", row.landSource.uppercased() + (row.landAsOf > 0
                    ? " · " + Date(timeIntervalSince1970: Double(row.landAsOf) / 1000)
                        .formatted(.dateTime.month().year())
                    : ""))
            }
            HStack(spacing: 5) {
                Circle().fill(tone.opacity(0.8)).frame(width: 7, height: 7)
                Text(permission)
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .foregroundStyle(tone)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(18)
        .frame(width: width)
        .background(Theme.surface)
        .clipShape(ProfileGeometry.shape)
        .overlay(ProfileGeometry.shape.strokeBorder(Theme.hairline))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("place-land")
    }

    private func labelled(_ label: String, _ value: String) -> some View {
        HStack(alignment: .top) {
            Text(label)
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(Theme.textSecondary)
            Spacer(minLength: 12)
            Text(value)
                .font(.system(size: 12, weight: .semibold, design: .monospaced))
                .foregroundStyle(Theme.textPrimary)
                .multilineTextAlignment(.trailing)
        }
    }

    // MARK: Walk up — the POV, from the page (the pin hand-off's second door)

    private func walkUpBar(width: CGFloat) -> some View {
        Button { walkingUp = true } label: {
            HStack(spacing: 12) {
                Image(systemName: "figure.walk")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(tint)
                Text("WALK UP")
                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                    .kerning(1.2)
                    .foregroundStyle(Theme.textPrimary)
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Theme.textTertiary)
            }
            .padding(.horizontal, 18)
            .frame(width: width, height: ProfileGeometry.bar)
            .background(Theme.surface)
            .clipShape(ProfileGeometry.shape)
            .overlay(ProfileGeometry.shape.strokeBorder(Theme.hairline))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("place-walk-up")
    }

    private func annotation(_ row: PlaceRow) -> PlaceAnnotation {
        PlaceAnnotation(id: row.id, name: row.name,
                        coordinate: .init(latitude: row.lat, longitude: row.lng),
                        category: row.descriptor.isEmpty ? nil : row.descriptor)
    }

    private func neighbours(of row: PlaceRow) -> [PlaceAnnotation] {
        core.places
            .filter { $0.placed && $0.id != row.id }
            .map { PlaceAnnotation(id: $0.id, name: $0.name,
                                   coordinate: .init(latitude: $0.lat, longitude: $0.lng),
                                   category: $0.descriptor.isEmpty ? nil : $0.descriptor) }
    }

    // MARK: Area cells — geohash 3×3 from the Place's own point, client-side

    /// The precision the location card draws at — see the file header for the
    /// ruling. Distinct from `Geohash.venuePrecision` (9), which is an ENCODE
    /// precision for proximity queries, not a display statement.
    static let areaPrecision = 6

    /// The Place's cell plus its eight neighbours, projected into the unit card
    /// (north up, geographic aspect preserved, letterboxed). Empty when the
    /// Place is unplaced — the card simply isn't drawn.
    static func areaCells(lat: Double, lng: Double, placed: Bool,
                          precision: Int = areaPrecision) -> [ProfileGeoCell] {
        guard placed else { return [] }
        // The projection is `Geohash.paint` now — the same maths this function used to
        // hold privately, at the card's aspect. A second copy of a projection is how
        // two surfaces end up disagreeing about where north is.
        let patch = Geohash.patch(around: .init(latitude: lat, longitude: lng),
                                  precision: precision)
        return Geohash.paint(patch, aspect: 2.5 / 0.8)
            .map { ProfileGeoCell(hash: $0.hash, x: $0.x, y: $0.y, w: $0.w, h: $0.h) }
    }
}
