//  PeopleRibbon.swift
//  The PEOPLE ribbon on LIFE. When the PEOPLE jewel is LOCKED, the faceted dots give way to a
//  vertical column of CONTACT JEWELS — each the SAME geometry as the top-left profile jewel
//  (`AccountBarButton`): a 34pt `Avatar` disc filled with the contact's photo, or a solid block
//  colour carrying their initials. Tapping a jewel binds that person as the PEOPLE dimension; a
//  white rim marks the bound contact.
//
//  The contacts are the real 1:1 connections (`core.peerPhoto`/`peerColor` per peer). Until any
//  exist, a seeded family stands in so the locked ribbon is never blank on a fresh device —
//  the same honest-fallback the top-down map uses (`PlaceAnnotation.demo`).

import SwiftUI

/// One contact rendered as a jewel — the profile-jewel geometry, per person.
struct ContactJewel: Identifiable, Hashable {
    let id: String
    let name: String
    var photo: Data? = nil
    var color: Color = LifeFilter.people.color

    /// Two-letter fallback shown on the block-colour disc when there is no photo.
    var initials: String { Avatar.initials(name) }
}

extension ContactJewel {
    /// Seeded contacts — the running-example family — shown until real connections exist, so the
    /// locked PEOPLE ribbon is never blank on a fresh device (mirrors `PlaceAnnotation.demo`).
    static let demo: [ContactJewel] = [
        .init(id: "demo.hope",  name: "Hope",  color: Color(hex: "E06C5A")),
        .init(id: "demo.john",  name: "John",  color: Color(hex: "4C8DBF")),
        .init(id: "demo.mum",   name: "Mum",   color: Color(hex: "C77DB0")),
        .init(id: "demo.dad",   name: "Dad",   color: Color(hex: "5FA46A")),
        .init(id: "demo.ralph", name: "Ralph", color: Color(hex: "D8A63C")),
    ]
}

/// The vertical column of contact jewels that extends below the PEOPLE jewel when it is locked.
/// Content-sized (not greedy): it is as tall as its jewels up to a cap, then scrolls — so when it
/// floats over a maximised selector's background it reads as a compact dock, not a full curtain.
struct ContactJewelColumn: View {
    let contacts: [ContactJewel]
    let width: CGFloat
    let boundID: String?
    let onSelect: (String) -> Void

    /// The profile jewel's diameter (`AccountBarButton.diameter`).
    private let jewelSize: CGFloat = 34
    private let gap: CGFloat = 12
    private let pad: CGFloat = 12
    /// How many jewels stand before the dock starts scrolling.
    private let cap = 8

    private var height: CGFloat {
        let n = min(contacts.count, cap)
        guard n > 0 else { return 0 }
        return CGFloat(n) * jewelSize + CGFloat(n - 1) * gap + pad * 2
    }

    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(spacing: gap) {
                ForEach(contacts) { c in
                    ContactJewelView(contact: c, size: jewelSize, bound: c.id == boundID)
                        .contentShape(Circle())
                        .onTapGesture { onSelect(c.id) }
                }
            }
            .frame(maxWidth: .infinity)   // centre the discs in the column
            .padding(.vertical, pad)
        }
        .frame(width: width, height: height)
    }
}

/// One contact jewel — identical geometry to the top-left profile jewel (a 34pt `Avatar` disc),
/// with a white rim while it is the bound PEOPLE dimension.
private struct ContactJewelView: View {
    let contact: ContactJewel
    let size: CGFloat
    let bound: Bool

    var body: some View {
        Avatar(initials: contact.initials, tint: contact.color,
               size: size, photo: contact.photo, color: contact.color)
            .overlay { if bound { Circle().strokeBorder(.white, lineWidth: 2) } }
            .opacity(bound ? 1 : 0.9)
    }
}

#Preview("Contact jewels — locked PEOPLE") {
    ZStack {
        Theme.bg.ignoresSafeArea()
        ContactJewelColumn(contacts: ContactJewel.demo, width: 56,
                           boundID: "demo.mum", onSelect: { _ in })
    }
    .preferredColorScheme(.dark)
}
