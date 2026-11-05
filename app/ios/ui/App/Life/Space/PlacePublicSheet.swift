//  PlacePublicSheet.swift
//  Opening a Place to the public, and the poster that lets people in.
//
//  This is the ONE surface for "is this place public", shared by every route that reaches
//  a Place. It exists as its own view because the control was originally only in
//  EntityDetailView — and LIFE ▸ PLACES renders a MAP whose pins open PlacePOVView, so
//  EntityDetail is never reached that way and the toggle was unreachable in practice.
//
//  See PublicPlace.swift for what public actually means: auto-accepted, admitted by the
//  host's device on its next sync, closing does not evict.

import SwiftUI
import PacificCore

struct PlacePublicSheet: View {
    let placeID: String
    let placeName: String
    @EnvironmentObject private var core: CoreService
    @Environment(\.dismiss) private var dismiss
    @State private var confirmRotate = false

    private var tint: Color { LifeFilter.places.color }

    /// Read off the folded GroupObject, not a passed-in row, so the toggle reflects what
    /// the core actually holds after a set — including a set that failed.
    private var isPublic: Bool {
        core.places.first { $0.id == placeID }?.access == "public"
    }

    private var visibility: String {
        core.places.first { $0.id == placeID }?.visibility ?? "private"
    }

    private var row: PlaceRow? { core.places.first { $0.id == placeID } }

    /// WHOSE LAND, and whether they were asked — deliberately two separate statements.
    ///
    /// The registry answers the first and can never answer the second. Showing them as one
    /// line would let "we know the council owns it" read as "the council is fine with it",
    /// which is exactly the confusion that gets caches archived and signs torn down.
    ///
    /// Nothing here performs a lookup: HMLR's free data is monthly bulk files, so
    /// resolution is an ingest job elsewhere. This surface reports what has been recorded,
    /// including the common case of nothing.
    @ViewBuilder private var landSection: some View {
        let r = row
        VStack(alignment: .leading, spacing: 8) {
            Text("The land").font(.subheadline.weight(.semibold))

            if let r, !r.landParcel.isEmpty {
                labelled("Parcel", r.landParcel)
                labelled("Owner", r.landProprietor.isEmpty
                            ? "not in the free data (a private owner isn’t published)"
                            : r.landProprietor)
                if !r.landSource.isEmpty {
                    labelled("Source", r.landSource.uppercased() + (r.landAsOf > 0
                        ? " · " + Date(timeIntervalSince1970: Double(r.landAsOf) / 1000)
                            .formatted(.dateTime.month().year())
                        : ""))
                }
                permissionRow(r.landPermission)
            } else {
                Text("Nobody has checked whose land this is.")
                    .font(.caption).foregroundStyle(.secondary)
                Text("Allotments, parks and benches are usually council-owned, which is published. A private owner is not.")
                    .font(.caption2).foregroundStyle(.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private func labelled(_ k: String, _ v: String) -> some View {
        HStack(alignment: .top) {
            Text(k).font(.caption).foregroundStyle(.secondary)
            Spacer(minLength: 12)
            Text(v).font(.caption.weight(.medium)).multilineTextAlignment(.trailing)
        }
    }

    /// Stated as its own claim, with `unknown` shown rather than hidden — "nobody asked"
    /// is the state most signs are actually in, and it should be visible.
    @ViewBuilder private func permissionRow(_ p: String) -> some View {
        let (text, tone): (String, Color) = switch p {
        case "granted": ("the owner agreed", .green)
        case "asked":   ("asked, no answer yet", .orange)
        case "refused": ("the owner said no", .red)
        default:        ("nobody has asked the owner", .orange)
        }
        HStack(spacing: 6) {
            Image(systemName: p == "granted" ? "checkmark.seal" : "exclamationmark.triangle")
                .font(.caption2)
            Text(text).font(.caption.weight(.medium))
        }
        .foregroundStyle(tone)
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 18) {
                    // WHO MAY DISCOVER IT — the visibility dial. Not the door
                    // below: this is how far the fact of the place travels
                    // (discovery answers, and the caches they land in), graded by
                    // distance in the graph.
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Visible to").font(.headline)
                        Picker("Visible to", selection: Binding(
                            get: { visibility },
                            set: { core.setPlaceVisibility(placeID, $0) }
                        )) {
                            Text("Members").tag("private")
                            Text("Connections").tag("connections")
                            Text("Network").tag("network")
                        }
                        .pickerStyle(.segmented)
                        .tint(tint)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)

                    Toggle(isOn: Binding(get: { isPublic }, set: { open in
                        core.setPlaceAccess(placeID, public: open)
                        // A sign needs a doorbell to be knocked at, so fit one the moment
                        // the place opens rather than making the user find a second
                        // control. Rotation stays explicit — it retires printed signs.
                        if open, core.places.first(where: { $0.id == placeID })?.doorbell.isEmpty != false {
                            core.fitDoorbell(placeID)
                        }
                    })) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Open to the public").font(.headline)
                            Text(isPublic ? "Anyone who scans the code joins."
                                          : "Joining is by invitation only.")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    .tint(tint)

                    if isPublic {
                        if let card = core.placeJoinCard(placeID) {
                            VStack(spacing: 12) {
                                QRView(payload: card, size: 220)
                                    .padding(14)
                                    .background(.white, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                                // The one thing about auto-accept a host can be genuinely
                                // surprised by: somebody's device has to do the letting-in,
                                // so with every member's phone off, nobody gets in.
                                Text("Print it and put it on the wall. Anyone who scans it joins automatically — while a member's phone is running.")
                                    .font(.caption).foregroundStyle(.secondary)
                                    .multilineTextAlignment(.center)
                                    .fixedSize(horizontal: false, vertical: true)
                                ShareLink(item: card) {
                                    Label("Share the code", systemImage: "square.and.arrow.up")
                                        .font(.subheadline.weight(.semibold))
                                }
                                // Rotation is the ONLY revocation there is. It retires
                                // every printed sign, and it is the only way to stop
                                // someone who has left from reading who arrives — leaving
                                // the group does not take the old doorbell off their
                                // device. Destructive, so it asks.
                                Button(role: .destructive) { confirmRotate = true } label: {
                                    Label("Replace the code", systemImage: "arrow.triangle.2.circlepath")
                                        .font(.caption.weight(.semibold))
                                }
                                .confirmationDialog("Replace this code?",
                                                    isPresented: $confirmRotate,
                                                    titleVisibility: .visible) {
                                    Button("Replace", role: .destructive) { core.fitDoorbell(placeID) }
                                    Button("Cancel", role: .cancel) {}
                                } message: {
                                    Text("Every printed sign stops working and will need reprinting. Do this if a sign ended up somewhere it shouldn’t, or to stop someone who has left from seeing who arrives.")
                                }
                            }
                        } else {
                            // The core refuses to vend a card for a place it will not admit
                            // to. Say so rather than showing an empty frame.
                            Text(core.error ?? "Couldn’t make a code for this place.")
                                .font(.caption).foregroundStyle(.orange)
                        }
                    }

                    landSection

                    Text("Closing it again stops new people joining. It does not remove anyone already in.")
                        .font(.caption2).foregroundStyle(.tertiary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding()
            }
            .background(Brand.groupBG)
            .navigationTitle(placeName.isEmpty ? "Place" : placeName)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}
