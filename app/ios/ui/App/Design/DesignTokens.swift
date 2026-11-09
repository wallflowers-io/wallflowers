//  DesignTokens.swift
//  THE single source for every design constant the app shares with anything that
//  depicts it — today the one-pager's UI diagram, tomorrow
//  anything else that has to agree with the app rather than resemble it.
//
//  WHY THIS FILE EXISTS. These numbers used to live only as literals inside the
//  views, and the diagram carried hand-transcribed copies of them. That is a
//  silent-drift machine: `ObjectCard` insets itself by 14pt, the diagram had been
//  given 19.6pt, and nothing anywhere could notice. A comment citing a source
//  ("token: Note-modal geometry = ComposeGlass cornerRadius 28") cannot fail a
//  build; this file can.
//
//  THE RULE. Swift is authoritative — it is the app. `scripts/design-tokens.py`
//  parses THIS file and regenerates the diagram's CSS from it, and its --check
//  mode fails when the committed CSS no longer matches. So a token moves here,
//  and everything downstream follows or breaks loudly.
//
//  WHAT BELONGS HERE. A constant TWO implementations must agree on: geometry the
//  diagram draws, and the springs it animates with. Not derived arithmetic (the
//  diagram's cqw scale falls out of `Screen.designWidth`), and not a constant only
//  one place uses.

import SwiftUI

enum DesignTokens {

    /// The reference screen the diagram scales against. iPhone logical width —
    /// the 480:1043 frame the page draws is this device's aspect.
    enum Screen {
        static let designWidth: CGFloat = 393
        /// Full screen height. The detail hero is a full-bleed wall (CoverScroll
        /// ignores the safe area), so it is measured against this, not the body.
        static let designHeight: CGFloat = 852
    }

    /// LIFE's one horizontal gutter. Ruled 15 Aug: the tab had FOUR — cards 14,
    /// the feed band 18, ProfileStack and GroupDetailView 16, EventDetailPage 20
    /// across ten sites — so tapping a card and landing on its event shifted the
    /// margin by 6pt. One number now, and the card's was the one that won.
    enum Life {
        static let gutter: CGFloat = 14
    }

    /// FeedView — the Active Scroll Feed.
    enum Feed {
        /// LazyVStack spacing: cards breathe, and the gap lives in the owning
        /// stack rather than in the card (ObjectCard's own header says so).
        static let stackSpacing: CGFloat = 26
        /// Clearance under the rail, added to the jewel diameter.
        static let topGap: CGFloat = 40
    }

    /// ObjectCard — THE card, inherited by every GroupObject kind.
    enum Card {
        /// Header and footer lines sit 2pt in from the square's edge.
        static let lineInset: CGFloat = 2
        /// VStack spacing between header, square and footer.
        static let vstackSpacing: CGFloat = 9
        /// Header HStack spacing.
        static let headerGap: CGFloat = 10
        /// The author's avatar; its ring is the provenance mark: solid = member,
        /// dashed = over hops.
        static let avatar: CGFloat = 30
        static let ring: CGFloat = 1.5
        /// The kind's glyph, above the card's top right.
        static let kindSize: CGFloat = 15
        static let authorSize: CGFloat = 13
        static let locationSize: CGFloat = 12
        static let titleSize: CGFloat = 22
        /// The title's inset inside the square.
        static let titleInset: CGFloat = 16
        static let footerSize: CGFloat = 12.5
        /// The card's edge — the skin's hard outline (Skin.edge).
        static let hairline: CGFloat = 3
    }

    /// ProfileStack — the universal Profile (Person and Space are one component).
    /// The ratios are of the CARD's width, which is the screen less two gutters.
    enum Profile {
        /// Expanded headline: person 2.5:1, space 2.5:0.6.
        static let headlinePerson: CGFloat = 0.4      // 1 / 2.5
        static let headlineSpace: CGFloat = 0.24      // 0.6 / 2.5
        /// Info and map cards, 2.5:0.8. The BIO has no ratio any more — it is a
        /// full-width band that takes the height of its own words (26 Aug).
        static let info: CGFloat = 0.32
        /// The bio band's inset from its own shaded edges.
        static let bandPad: CGFloat = 16
        /// The collapsed bar — also the directory row and the pin bar. One bar,
        /// three uses, so this is a single number by design.
        static let bar: CGFloat = 64
        /// VStack spacing between cards, and the stack's vertical padding.
        static let cardSpacing: CGFloat = 12
        static let verticalPad: CGFloat = 16
        /// The selector rail (ProfileJewelRail): the glyph alone in an equal
        /// slot, with a rule under the active one. No circle — the circled jewel
        /// belongs to the LIFE rail, which is a different instrument.
        static let jewelGlyph: CGFloat = 17
        /// A slot is a SQUARE — its side is the rail's width over the number of
        /// jewels (`ProfileJewels.order`), so the strip between the card above and
        /// the rule measures one tile. That leaves the rail with no vertical number
        /// of its own: the gap under the glyph and the pad around it are both what
        /// centring the glyph in that square produces, which is why `jewelRuleGap`
        /// and `jewelPad` are gone rather than retuned. `jewelRule` went with the
        /// underline itself: the lit tile's tint is the whole selection mark now
        /// (ProfileJewelRail.hue), and a rule under it said so a second time.
        /// The DM circle on a person headline — CHAT's gold, so the control that
        /// crosses to the CHAT tab is the one coloured thing on the card.
        static let dmCircle = Color(red: 0.949, green: 0.780, blue: 0.267)   // F2C744
        /// Sig.modalRadius — every profile card wears the Note-modal geometry.
        /// The one token the profile component spec already names.
        static let radius: CGFloat = 28
    }

    /// EventDetailPage — the hero wall a tapped feed card opens into.
    enum Event {
        /// With a flyer. Note how close this is to a feed card's own height
        /// (365 square + chrome): the app's expansion is mostly the card going
        /// FULL-BLEED and losing its frame, not growing much taller.
        static let heroHeight: CGFloat = 440
        /// Without one.
        static let heroHeightPlain: CGFloat = 260
    }

    /// ThreadView — a Channel rendered as a nested thread, its Rooms as tabs.
    enum Room {
        /// The room tab: a capsule, selected = accent fill.
        static let tabPadH: CGFloat = 12
        static let tabPadV: CGFloat = 6
        /// One nesting step. A reply is indented by this much per depth.
        static let indentUnit: CGFloat = 16
        /// A post row's own insets, and the gap between its stacked parts.
        static let postPadH: CGFloat = 12
        static let postPadV: CGFloat = 7
        static let postSpacing: CGFloat = 5
        /// The rail-to-body gap in a post row.
        static let railGap: CGFloat = 8
    }

    /// ChatView + Sig.ChatBubble — one conversation, Signal geometry.
    enum Chat {
        /// Sig.bubbleRadius, and the tightened tail on the sender's edge. The
        /// tail is what says WHOSE bubble it is, before colour does.
        static let bubbleRadius: CGFloat = 18
        static let bubbleTail: CGFloat = 4
        static let bubblePadH: CGFloat = 12
        static let bubblePadV: CGFloat = 8
        /// The opposite margin a bubble never crosses — Spacer(minLength: 44).
        /// It is what keeps a thread readable as two columns.
        static let bubbleInset: CGFloat = 44
        /// Sig.gapDistinct / gapGrouped: consecutive messages from one author
        /// close up to 2, a change of speaker opens to 12.
        static let gapDistinct: CGFloat = 12
        static let gapGrouped: CGFloat = 2
    }

    /// The shell's disc row. The three discs NEVER come and go — they move and
    /// change identity (Shell.buttonRow), and the GAPS carry the whole slide.
    enum Discs {
        static let size: CGFloat = 54.5
        /// Both gaps at rest; the row is gathered and centred.
        static let restGap: CGFloat = 30
        /// The row's inset from the band's edges.
        static let rowEdge: CGFloat = 32
        /// Composing, the trailing gap closes to this and the leading one takes
        /// whatever is left — so NOTE goes to the left edge and CHAT+LIFE gather
        /// at the right. One number produces the entire spread.
        static let composeGap: CGFloat = 12
    }

    /// RibbonRail + JewelFace — the selector rail.
    enum Rail {
        /// RibbonWidth.contracted. The jewel at rest, a circle.
        static let jewel: CGFloat = 46
        /// Minimum gap; the real rail uses flexible spacers that absorb slack.
        static let gap: CGFloat = 4
        static let inset: CGFloat = 10
        static let topPad: CGFloat = 8
        /// The glyph, as a fraction of the jewel's diameter.
        static let glyphRatio: CGFloat = 0.36
        static let stroke: CGFloat = 1.4
        /// JewelFace.opal — the rail is monochrome; only the orb carries colour.
        /// A dynamic pair: pale opal on slate, ink-opal on paper — same hue family,
        /// so the rail keeps its cast in both keys and never vanishes into the ground.
        static let opal = Theme.dyn(light: 0x3D4D47, dark: 0xD9E3E0)
    }

    /// The bedrock skin (Design/Pixel.swift):
    /// rounded surfaces, a hard edge, a bevel inside it, an unblurred offset shadow.
    enum Skin {
        /// A card's corner, where nothing more specific (Profile.radius, a capsule) applies.
        static let radius: CGFloat = 14
        static let edge: CGFloat = 3
        static let bevel: CGFloat = 3
        static let shadow: CGFloat = 3
        /// The darker band under a button's face.
        static let lip: CGFloat = 4
    }

    /// The shell's own motion. A spring here is as much a contract as a length:
    /// the diagram animates with these numbers or it is depicting something else.
    enum Motion {
        /// Shell.perchContent — the top-circle precession.
        static let perchResponse: Double = 0.35
        static let perchDamping: Double = 0.8
        /// ProfileGeometry.spring — the headline collapse.
        static let profileResponse: Double = 0.45
        static let profileDamping: Double = 0.85
    }
}
