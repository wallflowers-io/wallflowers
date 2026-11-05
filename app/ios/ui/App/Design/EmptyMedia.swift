//  EmptyMedia.swift
//  The honest ground under an absent picture.
//
//  It lived in EventsPage.swift while events were the only surface with a picture
//  slot that could be empty. The universal Profile now has one too — a flyer, a
//  cover — and the component must not reach into a LIFE page to draw itself, so
//  the atom moved to where both can see it.
//
//  It is stripes rather than a grey box, and rather than a spinner, on purpose:
//  it says THERE IS NO PICTURE, which is a fact, instead of implying one is on
//  its way. Placebo-control doctrine, applied to media.

import SwiftUI

/// The empty flyer slot's ground — the design's diagonal stripes, in slate.
struct FlyerStripes: View {
    var body: some View {
        Canvas { context, size in
            context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Theme.surface2.opacity(0.5)))
            let step: CGFloat = 18
            var x: CGFloat = -size.height
            while x < size.width {
                var band = Path()
                band.move(to: CGPoint(x: x, y: size.height))
                band.addLine(to: CGPoint(x: x + size.height, y: 0))
                band.addLine(to: CGPoint(x: x + size.height + step / 2, y: 0))
                band.addLine(to: CGPoint(x: x + step / 2, y: size.height))
                band.closeSubpath()
                context.fill(band, with: .color(Theme.surface.opacity(0.85)))
                x += step
            }
        }
    }
}
