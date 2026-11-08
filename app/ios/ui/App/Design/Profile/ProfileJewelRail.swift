//  ProfileJewelRail.swift
//  The profile's selector rail — ONE component, worn by every profile page: the
//  universal `ProfileStack` (person · place) and `GroupDetailView` (Space). It
//  was two hand-copied HStacks until 26 Aug, and a selector that must read the
//  same on every profile is one component or it is a drift waiting to happen.
//
//  NOT `JewelFace`. That is the LIFE rail's instrument — a draggable, lockable,
//  circled jewel that widens into a capsule when it is chosen. A profile's rail
//  only SELECTS the lane below it, so it wears the glyph alone: no circle, six
//  equal slots spanning the full screen width, and a rule under the active one
//  as wide as its slot — a sixth of the screen. Same glyphs, same opal,
//  different instrument.

import SwiftUI

struct ProfileJewelRail: View {
    /// Which lane is showing. The two hosts hold that state differently (a
    /// single `lane`, or the feed query's lock set), so the rail ASKS rather
    /// than owns — it draws a selection, it does not decide one.
    let isActive: (EventCategory) -> Bool
    let onTap: (EventCategory) -> Void
    /// Accessibility id prefix (`group-jewel-…` / `profile-jewel-…`). The two
    /// hosts are addressed separately by the UI tests, so the prefix comes in.
    let idPrefix: String
    /// The rail's full width — the strip the six slots divide.
    ///
    /// IT COMES IN because a slot is a SQUARE: its height is its own width, and a
    /// view cannot read the width it is about to be given. The host measures the
    /// glass once (both do already, for the wall) and hands it down — the same
    /// arithmetic-free rule CoverScroll states for the wall.
    let width: CGFloat

    /// MAY THE VIEWER MINT this kind on behalf of the profile? The rail ASKS, the
    /// same way it asks which lane is active: it draws the affordance, it does not
    /// decide who has it. A host that does not answer offers no ＋ at all, which is
    /// the honest default for a profile that is not yours.
    var canMint: (EventCategory) -> Bool = { _ in false }
    /// Raise the mint surface for a kind. Called ONLY where `canMint` said yes.
    var onMint: (EventCategory) -> Void = { _ in }

    private static let opal = DesignTokens.Rail.opal

    /// THE RAIL'S SPECTRUM — one hue per SLOT, in degrees on the colour wheel:
    /// red at the left running to indigo at the right (Ralph, 26 Aug). Degrees and
    /// not hex because a walk along the wheel is what this is — the six numbers say
    /// so, where six sampled swatches would not.
    ///
    /// NOT `Palettes.swift`: those five are the entity colours a person CHOOSES,
    /// re-tinting the whole accent system live. This is the rail's own fixed
    /// spectrum, and it is positional — the third tile is yellow because it is
    /// third, whatever jewel sits there.
    private static let hue: [Double] = [0, 32, 52, 132, 218, 272]

    /// The lit tile's ground: its own hue, laid on thin. Subtle is the whole point
    /// — it says WHICH tile is lit without becoming a block of colour, and it has
    /// to sit as quietly on paper as it does on slate, which an opacity does and a
    /// fixed swatch does not.
    private static func ground(_ slot: Int) -> Color {
        Color(hue: hue[slot] / 360, saturation: 0.85, brightness: 0.95).opacity(0.16)
    }

    /// The spin. One curve for the mark's whole quarter-turn, here beside it.
    private static let spin = Animation.spring(response: 0.34, dampingFraction: 0.72)

    /// One slot's side: a sixth of the rail, in both directions.
    private var side: CGFloat { width / CGFloat(ProfileJewels.order.count) }

    var body: some View {
        HStack(spacing: 0) {
            ForEach(ProfileJewels.order, id: \.self) { cat in
                slot(cat)
            }
        }
        .frame(width: width)
    }

    /// One slot: a SQUARE with the glyph at its centre. Six of them tile the width
    /// exactly, so the strip between the card above and the rail's bottom edge
    /// measures one square — the rail reads as a row of tiles rather than a band of
    /// arbitrary height.
    ///
    /// SELECTION IS THE TINT, and only the tint (Ralph, 26 Aug). The underline that
    /// used to mark the active slot is gone — a lit tile with a rule beneath it said
    /// the same thing twice — and the glyphs stay UNIFORM opal, so the row reads as
    /// one instrument with one tile lit, not six differently-dressed marks.
    private func slot(_ cat: EventCategory) -> some View {
        let on = isActive(cat)
        let slot = ProfileJewels.order.firstIndex(of: cat) ?? 0
        // A LIT TILE BECOMES ITS OWN ＋ — but only where the viewer may actually
        // author that kind for this profile. Where they may not, the lit tile keeps
        // its jewel: an affordance that would be refused at the fold is worse than
        // no affordance, and this rail is read by people who are not owners.
        let mint = on && canMint(cat)
        return ZStack {
            mark(cat.glyph, showing: !mint, from: 90)
            mark("plus", showing: mint, from: -90)
        }
        .font(.system(size: DesignTokens.Profile.jewelGlyph, weight: .regular))
        .foregroundStyle(Self.opal)
        .frame(width: side, height: side)
        .background(on ? Self.ground(slot) : .clear)
        .animation(Self.spin, value: mint)
        // The whole slot takes the tap, not just the glyph: without a circle to
        // aim at, the target has to be the column.
        .contentShape(Rectangle())
        // A ＋ is a different verb from a jewel. Tapping the lit tile mints;
        // tapping any other selects, exactly as before.
        .onTapGesture { mint ? onMint(cat) : onTap(cat) }
        .accessibilityElement(children: .ignore)
        .accessibilityIdentifier("\(idPrefix)-\(cat.rawValue)")
        .accessibilityLabel(mint ? "New \(cat.title)" : cat.title)
        .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
    }

    /// One face of a slot's mark. The two are stacked and turn the SAME way — the
    /// jewel a quarter out as the ＋ a quarter in — so the change reads as one mark
    /// spinning rather than two swapping places.
    private func mark(_ symbol: String, showing: Bool, from: Double) -> some View {
        Image(systemName: symbol)
            .opacity(showing ? 1 : 0)
            .rotationEffect(.degrees(showing ? 0 : from))
            .scaleEffect(showing ? 1 : 0.5)
    }
}
