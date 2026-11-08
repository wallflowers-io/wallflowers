//  Pixel.swift
//  The bedrock skin's two surfaces, worn round:
//  the CARD sunk in bedrock — a hard edge, a bevel lit top-left — and the BUTTON with
//  its lip. Every bedrock surface wears one of these; the numbers are DesignTokens.Skin.

import SwiftUI

extension View {
    /// A bedrock card behind this view, in `shape` (a `Skin.radius` rounded rect unless
    /// told otherwise). The edge sits INSIDE the frame, so the caller's own padding clears
    /// it. `bevel` off for small chips, where a lit rim would eat the whole face. `shadow`
    /// drops a hard, unblurred copy of the shape under it.
    func pixelCard<S: InsettableShape>(_ fill: Color = Theme.surface,
                                       in shape: S = RoundedRectangle(cornerRadius: DesignTokens.Skin.radius, style: .continuous),
                                       bevel: Bool = true, shadow: Bool = false,
                                       shadowColor: Color = Theme.edge) -> some View {
        modifier(PixelCard(fill: fill, shape: shape, bevel: bevel, shadow: shadow, shadowColor: shadowColor))
    }
}

struct PixelCard<S: InsettableShape>: ViewModifier {
    let fill: Color
    let shape: S
    var bevel = true
    var shadow = false
    /// The hard shadow's colour. A compose card casts its kind's.
    var shadowColor = Theme.edge

    func body(content: Content) -> some View {
        let e = DesignTokens.Skin.edge, b = DesignTokens.Skin.bevel, s = DesignTokens.Skin.shadow
        content
            .background {
                shape.fill(fill)
                    .overlay {
                        if bevel {
                            shape.inset(by: e)
                                .strokeBorder(LinearGradient(colors: [Theme.bevelHi, Theme.bevelLo],
                                                             startPoint: .topLeading, endPoint: .bottomTrailing),
                                              lineWidth: b)
                        }
                    }
                    .overlay(shape.strokeBorder(Theme.edge, lineWidth: e))
            }
            .background(shape.fill(shadowColor.opacity(shadow ? 1 : 0)).offset(x: s, y: s))
    }
}

/// The skin's button: primary is gold over its darker lip, secondary is a raised slab.
/// Pressing sinks it onto its own shadow. The label keeps its font.
struct PixelButtonStyle: ButtonStyle {
    var primary = true
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        let lip = DesignTokens.Skin.lip, s = DesignTokens.Skin.shadow
        let shape = RoundedRectangle(cornerRadius: DesignTokens.Skin.radius, style: .continuous)
        let down = configuration.isPressed
        configuration.label
            .foregroundStyle(primary ? Theme.onAccent : Theme.textPrimary)
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 10 + lip)
            .background {
                shape.fill(primary ? Theme.accent : Theme.surface2)
                    .overlay(alignment: .bottom) {
                        Rectangle().fill(primary ? Theme.accentLip : Theme.bevelLo).frame(height: lip)
                    }
                    .clipShape(shape)
                    .overlay(shape.strokeBorder(Theme.edge, lineWidth: DesignTokens.Skin.edge))
            }
            .offset(x: down ? s : 0, y: down ? s : 0)
            .background(shape.fill(Theme.edge).offset(x: s, y: s))
            .opacity(enabled ? 1 : 0.4)
    }
}

extension ButtonStyle where Self == PixelButtonStyle {
    static var pixel: PixelButtonStyle { PixelButtonStyle() }
    static var pixelSecondary: PixelButtonStyle { PixelButtonStyle(primary: false) }
}
