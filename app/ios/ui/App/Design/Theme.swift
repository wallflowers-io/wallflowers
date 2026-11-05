//  Theme.swift
//  Bedrock — the WallFlowers language. A
//  purple-grey ground carries the interface; COLOUR is reserved and deliberate,
//  appearing in exactly three roles:
//    1. Rich multimedia — real images/photos render full-colour on the bedrock.
//    2. Contextual markers — the enrichment entity palette (see EnrichmentKind.tint),
//       status (ok / warn / danger), unread, and the single gold action accent.
//    3. Nothing else. Chrome (backgrounds, rows, bubbles, bars, cards) is bedrock.
//
//  The theme has TWO KEYS, chosen by the THEME switch in Settings (never by the
//  OS): Kenjin (Dark) is bedrock; Pacific (Light) is the older slate ramp printed
//  on paper — white ground, ink text, black as the one accent. Colours are
//  explicit pairs so both casts stay deliberate.

import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

enum Theme {
    /// A dynamic pair: resolved per frame by the effective colour scheme, which
    /// WallFlowersApp pins from the THEME switch (`themeMode`) — so a mode flip
    /// re-renders live, and even a `static let` that captured one of these
    /// (Brand.accent) stays correct: the provider travels inside the Color.
    static func dyn(light: UInt, dark: UInt) -> Color {
        #if canImport(UIKit)
        Color(UIColor { trait in
            trait.userInterfaceStyle == .light ? UIColor(Color(hex: light)) : UIColor(Color(hex: dark))
        })
        #else
        Color(hex: dark)
        #endif
    }

    // Ground — bedrock under Kenjin, paper under Pacific.
    static let bg       = dyn(light: 0xFFFFFF, dark: 0x1F1D23)   // app background
    static let surface  = dyn(light: 0xF2F4F7, dark: 0x34313A)   // rows, cards
    static let surface2 = dyn(light: 0xE9EDF2, dark: 0x38333F)   // raised: composer, sheets, selected, avatars
    static let hairline = dyn(light: 0xD9DFE7, dark: 0x15131A)   // separators, borders
    static let fill     = dyn(light: 0xEDF0F4, dark: 0x26232C)   // input fields, unselected chips

    // Text — warm off-whites on bedrock; ink on paper.
    static let textPrimary   = dyn(light: 0x14181E, dark: 0xF3EFE6)
    static let textSecondary = dyn(light: 0x5A6470, dark: 0xD6D0C4)
    static let textTertiary  = dyn(light: 0x9AA4B0, dark: 0x9D96A8)

    // The single ACTION accent — primary affordances, active state, unread. Gold on
    // bedrock; black on paper. Anything filled with `accent` puts `onAccent` on top.
    static let accent = dyn(light: 0x14181E, dark: 0xF2C744)
    static let onAccent = dyn(light: 0xFFFFFF, dark: 0x1F1D23)   // text/glyphs on an accent fill
    /// The darker band under an accent button's face (Pixel).
    static let accentLip = dyn(light: 0x000000, dark: 0xB8871E)

    // The skin's edges (Pixel): the hard outline and shadow, and the bevel inside it,
    // lit top-left and shaded bottom-right.
    static let edge    = dyn(light: 0x14181E, dark: 0x15131A)
    static let bevelHi = dyn(light: 0xFFFFFF, dark: 0x5C5568)
    static let bevelLo = dyn(light: 0xD9DFE7, dark: 0x38333F)

    // Chat bubbles — monochrome. "Mine" is a step lighter (a marker of authorship),
    // not an accent flood; colour inside a bubble comes from enriched words.
    static let bubbleIncoming = surface
    static let bubbleOutgoing  = dyn(light: 0xDCE9EF, dark: 0x5C5568)

    // Status markers (contextual) — darkened on paper so they keep their contrast.
    static let ok     = dyn(light: 0x2E8F5B, dark: 0x67A33A)
    static let warn   = dyn(light: 0x9C7215, dark: 0xC9773F)
    static let danger = dyn(light: 0xC2455C, dark: 0xD8434A)
}

/// The ONE choreography for opening and leaving a conversation: the CHAT disc stretching
/// into the composer, the shell's disc row leaving and returning, and the transcript
/// fading under them. Every part reads from here, so the pieces move together instead of
/// each running its own curve.
///
/// They all start TOGETHER, on the same frame — the morph runs alongside the screen
/// transition, not after it. Only the durations differ, so the quick parts (the row, the
/// fade) settle while the stretch is still travelling.
///
/// Asymmetric on purpose — the standard entering/exiting rule. Arriving is a spring with
/// a whisper of overshoot (it lands, it doesn't stop dead); leaving is faster and fully
/// damped, because a collapse that bounces reads as indecision.
enum Motion {
    /// The disc → full-width field stretch.
    static let extend = Animation.spring(response: 0.42, dampingFraction: 0.80)
    /// The field → disc collapse: faster than the stretch and fully damped.
    static let retract = Animation.spring(response: 0.26, dampingFraction: 1.0)

    /// NOTE and LIFE fading out where they stand, and fading back. They never move: the
    /// conversation is drawn over them, so the widening composer covers them as it goes.
    static let rowOut = Animation.easeOut(duration: 0.20)
    static let rowIn  = Animation.easeIn(duration: 0.20)

    /// The transcript arriving over the row and leaving with the collapse.
    static let screenIn  = Animation.easeOut(duration: 0.24)
    static let screenOut = Animation.easeIn(duration: 0.22)

    /// The three discs sliding between their two arrangements — NOTE·CHAT·LIFE evenly
    /// spaced, and the compose card's X · Share · Send pushed to the edges. The same
    /// three views travelling, so this is the one motion that carries the identity swap.
    static let discSlide = Animation.spring(response: 0.38, dampingFraction: 0.84)

    /// The glyph swapping inside a disc (the symbol's line-draw). Rides the slide, but
    /// 30% quicker than it — the draw was lagging the disc it travels in.
    static let glyphSwap = discSlide.speed(1 / 0.7)

    /// The list search field leaving upward and dropping back.
    static let searchHide = Animation.easeOut(duration: 0.22)

    /// The compose card's edge lighting up as it leaves — quick, so the colour has
    /// arrived while the card is still solid enough to carry it.
    static let noteFlash = Animation.easeOut(duration: 0.10)
    /// The compose card leaves in LAYERS, back to front — the dimmed ground and its
    /// drifting colour go first and quickly, and the card itself follows a beat later,
    /// so the screen you were on is already back before the card clears it. Everything
    /// leaving at once read as a single flat sheet being switched off.
    static let noteGroundOut = Animation.easeOut(duration: 0.14)
    /// How far behind the ground the card sets off.
    static let noteCardDelay: TimeInterval = 0.08
    /// Cancelled: the card simply fades, as it always did.
    static let noteFade = Animation.easeIn(duration: 0.22).delay(noteCardDelay)
    /// Saved: the canvas opens OUT to fill the screen as it fades — the note going into
    /// the app rather than being taken off it. easeOut, not easeIn: this is a release.
    static let noteExpand = Animation.easeOut(duration: 0.28).delay(noteCardDelay)
    /// Shared: the card leaves upward, the way the thing it handed off went.
    static let noteSlideOut = Animation.easeIn(duration: 0.26).delay(noteCardDelay)
    /// When the last of those has finished and the card can be torn down.
    static let noteExit: TimeInterval = 0.40

    /// When the LAST of the leaving motions has finished — the point at which the
    /// conversation can be torn down. Not a stagger: everything above already ran from
    /// t=0 and the screen is transparent by now, so the removal is invisible bookkeeping.
    static let exit: TimeInterval = 0.32

    /// The field's contents (text field + send) — they fade on their own schedule so you
    /// never see them crushed inside a disc: in only once the slot has begun to open, out
    /// immediately when it starts to close.
    static let contentIn  = Animation.easeOut(duration: 0.16).delay(0.10)
    static let contentOut = Animation.easeIn(duration: 0.08)
}

extension Color {
    /// 0xRRGGBB literal → sRGB Color.
    init(hex: UInt) {
        self.init(
            .sRGB,
            red:   Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue:  Double(hex & 0xFF) / 255,
            opacity: 1
        )
    }

    /// "RRGGBB" text (optional "#"/"0x" prefix) → sRGB Color; falls back to the
    /// slate avatar surface when the string doesn't parse. The text twin of the
    /// `UInt` literal init, for hex persisted as a String (the self avatar colour).
    init(hex string: String) {
        let cleaned = string
            .trimmingCharacters(in: .whitespaces)
            .replacingOccurrences(of: "#", with: "")
            .replacingOccurrences(of: "0x", with: "", options: .caseInsensitive)
        self.init(hex: UInt(cleaned, radix: 16) ?? 0x1F2731)
    }

    /// This colour as an opaque "RRGGBB" hex string — the persisted form of the
    /// self avatar block colour (round-trips through `init(hex: String)`).
    var hexString: String {
        #if canImport(UIKit)
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        UIColor(self).getRed(&r, green: &g, blue: &b, alpha: &a)
        return String(format: "%02X%02X%02X",
                      Int((r * 255).rounded()), Int((g * 255).rounded()), Int((b * 255).rounded()))
        #else
        return "1F2731"
        #endif
    }
}
