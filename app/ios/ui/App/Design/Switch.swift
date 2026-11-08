//  Switch.swift
//  THE switch — every two-or-more-state control in the app, one definition.
//
//  There was no origin before this. `ThemeSwitch`, the mint surface's KIND and STANCE, the
//  listing category, the visibility face — all of them were a bare
//  `Picker(...).pickerStyle(.segmented)` styled at the call site, which means they wore
//  UIKit's animation and there was nowhere to change it. The booleans were worse: nine
//  `Toggle(...)` sites wearing the system switch, a different control language again for
//  the same act of flipping something.
//
//  So the animation lives HERE, in `Switch.motion`, and nowhere else. Swapping it is one
//  line, and every switch in the app moves together.

import SwiftUI

struct Switch<Value: Hashable>: View {
    let options: [Value]
    /// What each option READS as. The wire value and the word shown are not the same
    /// thing — `offers` reads as OFFER — so the caller supplies the words.
    let word: (Value) -> String
    @Binding var selection: Value

    /// THE ANIMATION. One place, every switch.
    static var motion: Animation { .spring(response: 0.34, dampingFraction: 0.78) }

    @Namespace private var indicator

    var body: some View {
        HStack(spacing: 4) {
            ForEach(options, id: \.self) { option in
                let on = option == selection
                Text(word(option))
                    .font(.system(.footnote, design: .monospaced).weight(on ? .semibold : .regular))
                    .tracking(1)
                    .foregroundStyle(on ? Theme.onAccent : Theme.textSecondary)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                    .background {
                        // ONE indicator that MOVES between the options rather than a fill
                        // per option that cross-fades — which is what makes it read as a
                        // thing sliding rather than two things blinking.
                        if on {
                            Capsule()
                                .fill(Brand.accent)
                                .overlay(Capsule().strokeBorder(Theme.edge, lineWidth: DesignTokens.Skin.edge))
                                .matchedGeometryEffect(id: "switch", in: indicator)
                        }
                    }
                    .contentShape(Capsule())
                    .onTapGesture {
                        guard !on else { return }
                        withAnimation(Self.motion) { selection = option }
                    }
                    .accessibilityIdentifier("switch-\(word(option).lowercased())")
                    .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
            }
        }
        .frame(maxWidth: .infinity)
        .pixelCard(Theme.fill, in: Capsule(), bevel: false, shadow: true)
    }
}

extension Switch where Value == Bool {
    /// The boolean case — the same control, so a yes/no reads in the same language as a
    /// three-way. `on`/`off` are the words, because "Free"/"Not free" is a better switch
    /// than a nameless system toggle floating at the end of a row.
    init(_ on: String, _ off: String, isOn: Binding<Bool>) {
        self.init(options: [true, false], word: { $0 ? on : off }, selection: isOn)
    }
}
