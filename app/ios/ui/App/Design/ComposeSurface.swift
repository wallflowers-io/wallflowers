//  ComposeSurface.swift
//  THE compose surface: the ground, the glass and the disc row that every "make a new
//  thing" modal wears. One implementation, three call sites — the NOTE compose card and
//  both LIFE mint wizards.
//
//  Before this each surface carried its own copy. They had already drifted:
//
//    • the ambient  — `NoteAmbient` and LifeMint's `AliveAmbient` were the same four
//      drifting blobs with the colours shuffled and the opacity a hair apart.
//    • the glass    — three separate 28pt cards. The NOTE one was deliberately built as
//      ONE clipped stack + ONE stroke, because filling a rounded rect and overlaying
//      more rounded rects puts FIVE independently antialiased curves on the same corner,
//      and with a material re-blurring the drifting ambient behind them every frame the
//      corners visibly crawl. The two wizards were still the crawling version.
//    • the buttons  — the shell's NOTE·CHAT·LIFE discs become the note card's ✕ · ↑ · →
//      (see InterfaceMatrix), while the wizards drew a small grey xmark and a capsule
//      that took the type's colour. Same job, different language.
//
//  So the fix is not "make the wizards look like the note" — it is one surface both use.
//  `DiscSlot` stays the home of the channel colours: a compose disc is RED cancel, GOLD
//  share, BLUE commit wherever it appears, and the surface's own tint lives on the glass
//  and the ambient instead of on the buttons.

import SwiftUI

// MARK: - Ground

/// The alive ground a compose card sits on: a deep-slate scrim with slow-drifting jewel
/// light, so the glass refracts moving colour even at rest.
///
/// `onTapOutside` is the empty-cancel gesture. It is deliberately the CALLER's decision
/// whether tapping out dismisses — a surface with something in it must not be binned by
/// a stray tap, and only the surface knows whether it has content.
struct ComposeGround: View {
    /// Replaces the ambient's indigo blob, so the ground carries the surface's identity.
    var tint: Color? = nil
    /// DEEP BLACK at zero transparency, and no drifting light. The LIFE newitem modals
    /// wear it (Ralph, 18 Aug) — "deep black, 0 opacity". The NOTE keeps the alive ground
    /// it was designed around.
    var opaque: Bool = false
    /// The CARD's live frame, so the bloom stays anchored to it — including while the
    /// keyboard pushes it up. Measured by the surface; `.zero` until the first layout.
    var canvas: CGRect = .zero
    var onTapOutside: (() -> Void)? = nil

    var body: some View {
        ZStack {
            if opaque {
                // Deep black — and then the BOHR BLOOM over it, which is what brightens
                // the ground around the canvas (Ralph, 18 Aug). Imported from the
                // one-pager; its page-scale bleed term is the reason it reaches the
                // corners where `NoteAmbient`'s bounded blobs cannot.
                Theme.bg
                // THE DIAL, if it wants turning — the shape, colour and motion are all
                // the imported field's; only how loud it sits is ours. 0.75 puts the
                // visible bands at roughly 0.12–0.5 luminance across an orbit, so the
                // sweep is plainly visible without the ground becoming a teal wash.
                BohrBloom(intensity: 0.75, canvas: canvas)
            } else {
                Theme.bg.opacity(0.72)
                NoteAmbient(tint: tint)
            }
        }
        .ignoresSafeArea()
        .contentShape(Rectangle())
        .onTapGesture { onTapOutside?() }
    }
}

// MARK: - Glass

extension View {
    /// The compose card, in the bedrock skin: a solid body with a faint cast of `tint` and
    /// the hard edge. No material: the card is paper, not glass. One clip and one stroke,
    /// so the corners do not crawl (see the note at the top of this file).
    ///
    /// `flash` replaces the resting edge with an outcome colour.
    /// `opaque` grounds the body in the screen's own colour instead of the card's — the
    /// LIFE newitem modals wear it (Ralph, 18 Aug): a mint is a surface you are working
    /// ON. The cast is unchanged, so the surface still carries its colour.
    func composeGlass(tint: Color, flash: Color? = nil, opaque: Bool = false) -> some View {
        modifier(ComposeGlass(tint: tint, flash: flash, opaque: opaque))
    }
}

struct ComposeGlass: ViewModifier {
    let tint: Color
    var flash: Color? = nil
    var opaque: Bool = false

    /// THE card skin — `PixelCard`'s, lit from the top-left like every other card. No hard
    /// shadow (Ralph, 8 Nov): the kind's colour spilled past the border, right and down,
    /// so the border is the card's edge. The coloured band across the top went before it
    /// (5 Nov).
    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: Sig.modalRadius, style: .continuous)
        return content
            .background(shape.fill(tint.opacity(0.05)))
            // Opaque: the ground at 0.94 (Ralph, 18 Aug), so the bloom behind is felt, not seen.
            .pixelCard(opaque ? Theme.bg.opacity(0.94) : Theme.surface, in: shape)
            .overlay {
                shape.strokeBorder(flash ?? .clear, lineWidth: DesignTokens.Skin.edge)
                    .animation(Motion.noteFlash, value: flash)
            }
    }
}

// MARK: - Discs

/// The compose disc row — ✕ cancel · ↑ share · → commit, in the shell's own channel
/// colours and with the shell's own press behaviour (`CircleButton` carries the bloom
/// report, the hold gesture and the disabled state).
///
/// The NOTE card does not use this: its buttons are the shell's ACTUAL discs, which
/// slide into this arrangement and back out again, so it gets the choreography as well
/// as the look. A modal presented ABOVE the shell (a sheet, a full-screen cover) covers
/// those discs and cannot borrow them, so it hosts this row instead — same colours, same
/// glyphs, same enable rules, same press animation.
///
/// `share` is optional. A surface with nothing to hand off renders TWO discs rather than
/// a permanently dead gold one — a button that can never fire is worse than an absent
/// one, and the colour language survives either way.
/// The three discs are addressable as `compose-cancel` / `compose-share` /
/// `compose-commit` on EVERY surface that wears them — the NOTE card, both mint
/// wizards and New Space. They had no identifiers at all, so a compose could be
/// filled in by a test but never committed by one: the only handle was the
/// commit's LABEL, which changes per surface ("Save Place", "Create Space").
struct ComposeDiscs: View {
    var canCommit: Bool
    var commitGlyph = "chevron.right"
    var commitLabel = "Save"
    /// GOLD's own gate. It defaults to `canCommit` — the note's rule, where a card with
    /// nothing in it can neither hand off nor keep — but the two genuinely come apart:
    /// New Space's gold opens the MEMBER list, which is answerable before the Space has
    /// a name and is exactly what you want to do first. Gating it on the commit rule
    /// would grey out the one disc that can be used yet.
    var canShare: Bool? = nil
    var shareGlyph = "chevron.up"
    var shareLabel = "Share"
    let cancel: () -> Void
    var share: (() -> Void)? = nil
    let commit: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            CircleButton(
                label: "Cancel",
                systemImage: "xmark",
                blockColor: DiscSlot.note.blockColor,
                showsGlyph: true,
                isActive: true,
                action: cancel
            )
            .accessibilityIdentifier("compose-cancel")
            Spacer(minLength: 0)
            if let share {
                let live = canShare ?? canCommit
                CircleButton(
                    label: shareLabel,
                    systemImage: shareGlyph,
                    blockColor: DiscSlot.chat.blockColor,
                    showsGlyph: true,
                    isActive: live,
                    enabled: live,
                    action: share
                )
                .accessibilityIdentifier("compose-share")
            }
            CircleButton(
                label: commitLabel,
                systemImage: commitGlyph,
                blockColor: DiscSlot.life.blockColor,
                showsGlyph: true,
                isActive: canCommit,
                enabled: canCommit,
                action: commit
            )
            .accessibilityIdentifier("compose-commit")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
    }
}

// MARK: - Field row

/// A labelled field on a compose card: a monospaced caption over a filled
/// well. Moved here from LifeMint when the New Space surface became the second
/// adopter — a shared atom belongs beside the surface it is part of, not
/// private to whichever screen happened to need it first.
extension View {
    /// THE row canvas — the filled, hairlined slot every row on a compose surface sits on,
    /// so the page reads as a stack of cards rather than one boxed field with loose
    /// controls under it (Ralph, 18 Aug).
    ///
    /// The values are the ones `New.swift`'s `entryBox` already draws for NAME and
    /// DESCRIBE; this is that canvas, named, so the rows that were bare can share it.
    /// `entryBox` still draws its own copy inline — deliberately untouched here, because
    /// changing it means re-verifying the fields it owns — and should adopt this the next
    /// time someone is in a position to check them.
    func composeCard(minHeight: CGFloat = CircleButton.discSize) -> some View {
        let slot = RoundedRectangle(cornerRadius: CircleButton.discRadius, style: .continuous)
        return padding(.horizontal, 20)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .center)
            .background(Theme.surface2, in: slot)
            .overlay(slot.strokeBorder(Theme.hairline, lineWidth: 1))
    }
}

/// A labelled field on ONE line — the caption, then its control at the trailing edge.
///
/// `FieldRow`'s stacked caption is right for anything that fills the width (a text well, a
/// segmented switch) and wrong for a COMPACT control: a date picker or a menu is narrower
/// than its own caption, so a row spent captioning it reads as a heading for something
/// bigger than what follows.
///
/// The control keeps its NATURAL width and the caption is the one that yields. That is what
/// keeps a long value ("One Time Thing") on one line at full size instead of wrapping — and
/// it is deliberately not solved by shrinking the type, which would make one value read
/// smaller than every other on the same card.
struct InlineFieldRow<Content: View>: View {
    let label: String
    @ViewBuilder var content: () -> Content

    /// Which side of the control the TITLE sits on. The caption is chrome, not content —
    /// it stays OUTSIDE the canvas either way, and this only says which end of the row it
    /// reads from.
    enum TitleSide { case leading, trailing }
    var titleSide: TitleSide = .leading

    var body: some View {
        HStack(spacing: 12) {
            if titleSide == .leading { caption }
            content().composeCard()          // the canvas is the CONTROL's, not the row's
            if titleSide == .trailing { caption }
        }
    }

    private var caption: some View {
        Text(label.uppercased())
            .font(.system(.caption2, design: .monospaced))
            .tracking(1.5)
            .foregroundStyle(Theme.textTertiary)
            .lineLimit(1)
    }
}

struct FieldRow<Content: View>: View {
    let label: String
    @ViewBuilder var content: () -> Content
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(label.uppercased()).font(.system(.caption2, design: .monospaced)).tracking(1.5)
                .foregroundStyle(Theme.textTertiary)
            content()
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Theme.fill.opacity(0.6)))
        }
        .padding(.vertical, 6)
    }
}

