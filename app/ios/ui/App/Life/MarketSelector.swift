//  MarketSelector.swift
//  The market ribbon on LIFE — the column that extends below the THINGS jewel.
//
//  Market is NOT its own row. It rides THINGS, because a thing is precisely what you have,
//  want, offer, buy, or sell: the jewel says "things", the column says in what posture. That
//  keeps LIFE's one selector mechanic intact — every dimension is a jewel with a ribbon under
//  it — instead of stacking a second control above the rail.
//
//  Geometry mirrors `ContactJewelColumn` (the locked-PEOPLE ribbon): a vertical column of
//  discs at the jewel's width, tap to select, a white rim on the active one. Same mechanic,
//  same shape, so the two ribbons read as one language.
//
//  It replaces nothing that was working: `.things` has no `EntityKind` at all — the resolver
//  drops Thing/Plant/Animal — so the entity ribbon under THINGS was already empty.

import SwiftUI
import UIKit   // UIImage — the listing thumbnail decodes from folded base64

/// The vertical column of market postures that extends below the THINGS jewel.
struct MarketIntentColumn: View {
    let width: CGFloat
    let selected: MarketIntent?
    let onSelect: (MarketIntent) -> Void

    /// The profile jewel's diameter, as `ContactJewelColumn` uses.
    private let jewelSize: CGFloat = 34

    var body: some View {
        ScrollView(showsIndicators: false) {
            LazyVStack(spacing: 12) {
                ForEach(MarketIntent.ordered) { intent in
                    MarketIntentJewel(intent: intent,
                                      size: jewelSize,
                                      bound: intent == selected)
                        .contentShape(Circle())
                        .onTapGesture { onSelect(intent) }
                        // Tap-gesture discs, not Buttons — so the traits have to say so, or
                        // the postures are invisible to XCUITest and nothing can regression-
                        // test the market. Same treatment the rail's jewels needed.
                        .accessibilityElement(children: .ignore)
                        .accessibilityIdentifier("market-\(intent.rawValue.lowercased())")
                        .accessibilityLabel(intent.rawValue)
                        .accessibilityAddTraits(intent == selected ? [.isButton, .isSelected] : .isButton)
                }
            }
            .frame(maxWidth: .infinity)   // centre the discs in the column
            .padding(.vertical, 12)
        }
        .frame(width: width)
    }
}

/// One posture as a disc — the intent's glyph on its side's colour. Acquire reads cool,
/// provide reads warm, Has is the neutral core they resolve around: the same semantics as
/// the market cube, so the two surfaces describe one model.
private struct MarketIntentJewel: View {
    let intent: MarketIntent
    let size: CGFloat
    let bound: Bool

    var body: some View {
        Circle()
            .fill(intent.color.opacity(bound ? 1 : 0.22))
            .frame(width: size, height: size)
            .overlay {
                Image(systemName: intent.glyph)
                    .font(.system(size: size * 0.4, weight: .bold))
                    .foregroundStyle(bound ? Color.black : intent.color)
            }
            .overlay { if bound { Circle().strokeBorder(Theme.accent, lineWidth: 2) } }
            .opacity(bound ? 1 : 0.9)
    }
}

// MARK: - The listed surface

/// The market list for one posture. Empty until something supplies market intents, at
/// which point it lists real commitments — so it shows the posture's NAME over empty space,
/// exactly as every other LIFE category does when it has nothing, rather than demo rows.
struct MarketList: View {
    let intent: MarketIntent
    let items: [MarketItem]
    /// Tapping a listing opens it for editing. The market is a set of things you can change,
    /// not a read-only report. Discovered listings are not mine to edit, so they do not call it.
    var onOpen: (MarketItem) -> Void = { _ in }
    /// Identity → display name, for attributing discovered listings.
    var nameFor: (String) -> String = { String($0.prefix(8)) }

    var body: some View {
        if items.isEmpty {
            GeometryReader { geo in
                Text(intent.rawValue)
                    .font(.system(size: 18, weight: .semibold))
                    .tracking(1.4)
                    .foregroundStyle(.secondary)
                    .frame(width: geo.size.width, height: geo.size.height)
                    .offset(y: -geo.size.height * 0.2)
            }
        } else {
            LazyVStack(spacing: 0) {
                VStack(spacing: 0) {
                    ForEach(items) { item in
                        MarketRow(item: item, nameFor: nameFor)
                            .contentShape(Rectangle())
                            // Only my own listings are editable; a discovered one is
                            // someone else's object and this device cannot author it.
                            .onTapGesture { if !item.isDiscovered { onOpen(item) } }
                    }
                }
                .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                .padding(.horizontal)
            }
            .padding(.bottom)
        }
    }
}

private struct MarketRow: View {
    let item: MarketItem
    /// Resolves an identity to a name — a discovered listing is only usable if you can see
    /// whose it is.
    var nameFor: (String) -> String = { String($0.prefix(8)) }

    var body: some View {
        HStack(spacing: 12) {
            // The listing's face — every row carries the slot, so the list reads as
            // a market. A photo fills it; without one it's the diagonal stripes,
            // the same honest fallback the event flyer uses, never a stand-in image.
            thumb
            HStack(alignment: .firstTextBaseline) {
                // A discovered listing is marked by a dot in the intent's colour: mine are
                // the plain rows, theirs carry the mark. One glyph, no caption.
                if item.isDiscovered {
                    Circle().fill(item.intent.color).frame(width: 5, height: 5)
                        .alignmentGuide(.firstTextBaseline) { $0[.bottom] + 1 }
                }
                Text(item.title)
                    .font(.subheadline)
                    .lineLimit(1)
                Spacer(minLength: 8)
                if let origin = item.origin {
                    // Whose it is, and — when it came through the mesh — who passed it on.
                    // `via` is the trust: a stranger's offer is only worth reading because
                    // someone you know forwarded it.
                    Text(item.hops <= 1
                         ? nameFor(origin)
                         : "\(nameFor(origin)) · via \(nameFor(item.via ?? origin))")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
                if let price = item.price {
                    Text(price)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }

    @ViewBuilder private var thumb: some View {
        Group {
            if let data = item.photo, let ui = UIImage(data: data) {
                Image(uiImage: ui).resizable().scaledToFill()
            } else {
                FlyerStripes()
            }
        }
        .frame(width: 40, height: 40)
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

#Preview("Market postures — the THINGS ribbon") {
    ZStack {
        Theme.bg.ignoresSafeArea()
        MarketIntentColumn(width: 56, selected: .wants, onSelect: { _ in })
    }
    .preferredColorScheme(.dark)
}
