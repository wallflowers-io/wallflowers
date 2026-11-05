//  CircleButton.swift
//  The Shell's round action buttons (NOTE · CHAT · LIFE) — a filled colour disc (or a
//  slate disc + glyph); the whole disc is the tap target. On TOUCH-DOWN it reports its
//  press and its LIVE centre to the shell, which renders the Metal press-bloom
//  un-clipped and keeps it on the disc as the compose row slides (see Shell.fireBloom,
//  Bloom.metal). The bloom is driven by the press, not the tap action.

import SwiftUI
import UIKit

/// Publishes each participating disc's centre (keyed by `bloomID`) in the "shell"
/// coordinate space, so the shell can place the press-bloom at the disc's live
/// position — it follows the disc as the row slides into/out of compose.
struct DiscCenterKey: PreferenceKey {
    static var defaultValue: [Int: CGPoint] = [:]
    static func reduce(value: inout [Int: CGPoint], nextValue: () -> [Int: CGPoint]) {
        value.merge(nextValue()) { _, new in new }
    }
}

struct CircleButton: View {
    /// The disc's diameter. The shell's action discs AND the message composer's slot
    /// key off this one number, so the "CHAT disc stretches into the text field" morph
    /// stays coupled — the slot's corner radius is exactly this disc's radius.
    /// 54.5: halfway between the original 64 and the over-condensed 45 — small enough
    /// to read as the condensed row, big enough to stay an easy thumb target. The
    /// composer slot, search field, and bottom-bar clearance follow via derivation.
    static let discSize: CGFloat = DesignTokens.Discs.size
    static let discRadius: CGFloat = discSize / 2

    /// Accessibility name (spoken, not shown).
    let label: String
    /// The SF Symbol displayed in the disc (when `blockColor` is nil, or as the white
    /// compose glyph when `showsGlyph`).
    let systemImage: String
    /// When set, the disc is a solid BLOCK OF COLOUR (the tab identity). Constant across
    /// rest and compose so the disc slides rather than cross-fading.
    var blockColor: Color? = nil
    /// On a colour block, overlay `systemImage` as a WHITE glyph (the compose X · ↑ · →).
    var showsGlyph: Bool = false
    /// Colours for the press-bloom (core → outer). nil ⇒ `[blockColor]`.
    var bloomColors: [Color]? = nil
    /// Identity used to report this disc's centre + press to the shell. < 0 ⇒ no bloom.
    var bloomID: Int = -1
    /// Fired on touch-down (true) and release (false) with this disc's id + bloom colours.
    var onPress: ((Int, Bool, [Color]) -> Void)? = nil
    var size: CGFloat = CircleButton.discSize
    var isActive: Bool = false
    var enabled: Bool = true
    var longPressAction: (() -> Void)? = nil
    var longPressLabel: String = "More"
    let action: () -> Void

    /// True from the moment the hold fires until the finger lifts — so the tap that
    /// UIKit delivers on release is swallowed rather than running `action` as well.
    @State private var handledByHold = false

    var body: some View {
        Button {
            guard !handledByHold else { handledByHold = false; return }
            action()
        } label: {
            discContent.background(centerReporter)
        }
        // The style reports touch-down/up (isPressed) — that is what fires the bloom.
        .buttonStyle(CircleButtonStyle(onPressChange: { pressed in
            onPress?(bloomID, pressed, effectiveBloom ?? [])
        }))
        .disabled(!enabled)
        .simultaneousGesture(holdGesture, including: longPressAction == nil ? .subviews : .all)
        .accessibilityLabel(label)
        .accessibilityAddTraits(isActive ? .isSelected : [])
        .accessibilityActions {
            if let longPressAction {
                Button(longPressLabel, action: longPressAction)
            }
        }
    }

    /// The disc's face: a solid colour block (NOTE·CHAT·LIFE) with an optional white
    /// compose glyph, otherwise the SF Symbol on a slate/accent fill.
    @ViewBuilder private var discContent: some View {
        if let blockColor {
            // The DISC is the owner; the glyph is its OVERLAY, and `.compositingGroup()` flattens
            // the two into ONE layer BEFORE any ancestor transform/animation touches them — so the
            // glyph is carried by the disc atomically and cannot lag or float off during the
            // keyboard/compose slide. As a `.background` the two were still separate layers, and
            // under SwiftUI's keyboard-avoidance layout animation a primary and its background can
            // interpolate their origins on marginally different paths — which is what let the icon
            // drift off its circle. The glyph also carries no `.animation` of its own (a
            // child-scoped clock is the other way they fly apart).
            Circle()
                .fill(blockColor)                          // constant colour → the disc slides
                .overlay(Circle().strokeBorder(Theme.edge, lineWidth: DesignTokens.Skin.edge))
                .background(Circle().fill(Theme.edge)
                    .offset(x: DesignTokens.Skin.shadow, y: DesignTokens.Skin.shadow))
                .frame(width: size, height: size)
                .overlay {
                    Image(systemName: systemImage)         // compose glyph — X · ↑ · → in white
                        .font(.system(size: size * 0.36, weight: .semibold))
                        .foregroundStyle(.white)
                        .opacity(showsGlyph ? 1 : 0)       // fades in place, inside the disc
                }
                .compositingGroup()                        // flatten disc+glyph → one atomic layer
                .opacity(blockOpacity)
                .contentShape(Circle())
        } else {
            Image(systemName: systemImage)
                .font(.system(size: size * 0.375, weight: .semibold))
                .contentTransition(.symbolEffect(.replace))
                .animation(Motion.glyphSwap, value: systemImage)
                .foregroundStyle(glyphColour)
                .frame(width: size, height: size)
                .background(Circle().fill(isActive ? Brand.accent : Theme.surface2))
                .overlay(Circle().strokeBorder(Theme.edge, lineWidth: DesignTokens.Skin.edge))
                .contentShape(Circle())
        }
    }

    /// Colour-block opacity: selected/active pops; disabled or unselected discs recede.
    private var blockOpacity: Double {
        if !enabled { return 0.3 }
        return isActive ? 1.0 : 0.3
    }

    private var glyphColour: Color {
        if !enabled { return Theme.textTertiary.opacity(0.6) }
        return isActive ? Theme.onAccent : Theme.textPrimary
    }

    /// Publishes the disc's live centre (in "shell" space) every layout, so the shell can
    /// keep the bloom on it while the row slides.
    private var centerReporter: some View {
        GeometryReader { p in
            let f = p.frame(in: .named("shell"))
            Color.clear.preference(
                key: DiscCenterKey.self,
                value: bloomID >= 0 ? [bloomID: CGPoint(x: f.midX, y: f.midY)] : [:]
            )
        }
    }

    private var effectiveBloom: [Color]? {
        if let bloomColors { return bloomColors }
        if let blockColor { return [blockColor] }
        return nil
    }

    /// Runs alongside the Button's own tap. `onChanged` fires at touch-down, clearing the
    /// swallow flag so a press whose tap never lands can't eat the NEXT tap.
    private var holdGesture: some Gesture {
        LongPressGesture(minimumDuration: 0.4)
            .onChanged { _ in handledByHold = false }
            .onEnded { _ in
                guard let longPressAction else { return }
                handledByHold = true
                UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                longPressAction()
            }
    }
}

/// A quiet press: the disc dips a touch. It also surfaces the press state (isPressed),
/// which is what drives the shell's press-bloom (fire on touch-down, decay on release).
private struct CircleButtonStyle: ButtonStyle {
    var onPressChange: (Bool) -> Void = { _ in }
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.92 : 1)
            .animation(.spring(response: 0.28, dampingFraction: 0.7), value: configuration.isPressed)
            .onChange(of: configuration.isPressed) { _, pressed in onPressChange(pressed) }
    }
}

#Preview("Circle buttons — rest tabs + compose buttons") {
    VStack(spacing: 30) {
        HStack(spacing: 24) {   // rest: colour blocks (Chat selected)
            CircleButton(label: "Note", systemImage: "square.and.pencil", blockColor: .red) {}
            CircleButton(label: "Chat", systemImage: "bubble.left.and.bubble.right", blockColor: .yellow, isActive: true) {}
            CircleButton(label: "Life", systemImage: "tree", blockColor: .blue) {}
        }
        HStack(spacing: 24) {   // compose: white X · ↑ · → over the same colours
            CircleButton(label: "Cancel", systemImage: "xmark", blockColor: .red, showsGlyph: true, isActive: true) {}
            CircleButton(label: "Share", systemImage: "chevron.up", blockColor: .yellow, showsGlyph: true, isActive: true) {}
            CircleButton(label: "Send", systemImage: "chevron.right", blockColor: .blue, showsGlyph: true, isActive: true) {}
        }
    }
    .padding(40)
    .frame(maxWidth: .infinity)
    .background(Theme.bg)
}
