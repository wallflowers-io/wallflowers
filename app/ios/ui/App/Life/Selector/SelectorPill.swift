//  SelectorPill.swift
//  The jewel's face, identity-parameterised — ONE pill for every selector band in the
//  app. The LIFE rail wraps it per EventCategory (RibbonRail's `JewelFace`), and the
//  screen's second face reuses it for the market band (INVENTORY · SKILLS · TRADE ·
//  JOBS), so the two bands are the same component wearing different identities rather
//  than two drawings that have to be kept in step.
//
//  Type size is FIXED — scaling it to fit read as untidy — so when the word cannot sit
//  beside the icon, the ICON is dropped instead. The word is what expansion exists to
//  show.
//
//  LABEL BUDGET (system Heavy 9.5, kern 0.6 ≈ 7.4pt/char; zone = screenWidth / n):
//
//      rail | zone  | word alone | word + icon
//      -----|-------|------------|------------
//        3  | 134.0 |  15 chars  |   9 chars
//        4  | 100.5 |  11 chars  |   5 chars
//        5  |  80.4 |   8 chars  |   2 chars   ← the binding case
//
//  **A label must be ≤ 8 characters on a five-pill band** (EVENTS · PEOPLE · GROUPS ·
//  MAP · PROJECTS all clear it) and ≤ 11 on a four-pill band (INVENTORY = 9 clears it).

import SwiftUI

struct SelectorPill: View {
    let title: String
    let glyph: String
    let tint: Color
    /// The column's width — a CAP, not a hint. Ignoring it let an expanded pill take its
    /// intrinsic size and draw straight over the neighbouring column.
    var width: CGFloat = RibbonWidth.contracted
    var expanded = false
    var muted = false
    var rim: Color = .clear

    /// 11, up from 9.5 — four-across rows at the shared maximised width leave room
    /// for a label that reads at a glance.
    private static let labelSize: CGFloat = 11
    private static let kern: CGFloat = 0.6
    private static let gap: CGFloat = 8
    /// Puts the icon's centre on Ø/2, so it does not shift when the pill expands.
    private static let padLead: CGFloat = 18
    private static let padTrail: CGFloat = 14

    private var w: CGFloat {
        expanded ? max(RibbonWidth.contracted, width) : RibbonWidth.contracted
    }

    var body: some View {
        let shape = Capsule(style: .continuous)
        // ONE grammar for every maximised pill: icon + label, always. A long label
        // scales down a hair rather than evicting the icon — the old measured
        // drop-the-icon rule is what made the eight LIFE headings disagree.
        HStack(spacing: expanded ? Self.gap : 0) {
            Image(systemName: glyph)
                .font(.system(size: 19, weight: .bold))
                .foregroundStyle(.white)
            if expanded {
                Text(title)
                    .font(.system(size: Self.labelSize, weight: .heavy))
                    .tracking(Self.kern)
                    .foregroundStyle(.white)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
        }
        .padding(.leading, expanded ? Self.padLead : 0)
        .padding(.trailing, expanded ? Self.padTrail : 0)
        // Contracted centres the lone icon; expanded sits leading so the icon holds
        // its place through the morph.
        .frame(width: w, height: RibbonWidth.contracted,
               alignment: expanded ? .leading : .center)
        .background(shape.fill(tint))
        .opacity(muted ? 0.68 : 1)
        .overlay(shape.strokeBorder(rim, lineWidth: 1.5))
        .clipShape(shape)
    }
}
