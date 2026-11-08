//  BohrBloom.swift
//  The bloom the lock page and the one-pager wear, playing behind a compose canvas.
//
//  Imported from the one-pager's `bohrGpu` field, whose
//  WGSL is ported in Shell/Bloom.metal as `bohrBloom`. Six lobes on an orbit, a centre
//  that opens with motion, and a PAGE-SCALE BLEED (sigma 620 at 0.42 amplitude) that
//  carries the light well past the disc's own box.
//
//  That bleed is the point: it is what brightens the ground AROUND the newitem canvas
//  rather than only under it. `NoteAmbient`'s four drifting blobs cannot do it — they are
//  bounded shapes, and past their radius there is nothing.
//
//  WHY A SHADER AND NOT MORE BLURRED CIRCLES
//  The falloff is the identity. Two gaussians per lobe at a 6.5:1 sigma ratio, summed
//  additively and then premultiplied by their PEAK CHANNEL, is what makes black stay
//  black while colour still reaches the corners. Stacked `.blur()` circles composite
//  their alpha instead, and grey the ground.

import SwiftUI

/// The three OPAL hues, from the one-pager verbatim — pushed away from their own grey
/// point for emissive use on black (K = 2.15, "as on the live site").
///
/// The push is a chroma boost about each colour's OWN luma, not a saturation multiply:
/// the hue and the ring survive, only the distance from grey grows. Recomputed here
/// rather than hardcoding the results, so the source values stay readable as the
/// site's own.
enum Opal {
    static let hues: [Color] = [[124.0, 178.0, 214.0],
                                [214.0, 150.0, 186.0],
                                [150.0, 205.0, 170.0]].map { c in
        let n = c.map { $0 / 255.0 }
        let g = 0.2126 * n[0] + 0.7152 * n[1] + 0.0722 * n[2]
        let k = 2.15
        let p = n.map { min(max(g + ($0 - g) * k, 0), 1) }
        return Color(.sRGB, red: p[0], green: p[1], blue: p[2], opacity: 1)
    }
}

/// The bloom, drifting. Drawn as a full-bleed rectangle so the bleed term can reach the
/// screen edges, and `.plusLighter` so it ADDS to the ground instead of covering it —
/// the same compositing the shell's press-bloom uses.
struct BohrBloom: View {
    /// How bright the whole field sits. The newitem ground wants it present but not
    /// competing with the canvas in front of it.
    var intensity: Double = 1

    /// The CANVAS the field is anchored to — the card's LIVE frame, in the same space
    /// this view is laid out in.
    ///
    /// Measured rather than guessed, and measured every layout, because the card MOVES:
    /// the keyboard pushes it up and lets it back down, and a field centred on the view
    /// would slide out from under it. Anchoring to the card is what keeps the light
    /// where the light belongs.
    ///
    /// The ring is sized from it too, so every lobe orbits UNDER the canvas: what reaches
    /// the visible frame is the halo tails, the breathing bleed, and whatever shows
    /// through the card's own slight transparency. Light from behind, not blobs in the
    /// open.
    ///
    /// `.zero` = no canvas yet, so the field takes the view itself.
    var canvas: CGRect = .zero

    /// How far out the lobes ride, as a fraction of the canvas half-extents.
    ///
    /// 0.50, worked out rather than picked. On a 393-wide phone the card's half-width is
    /// 0.459 in the shader's space, and a lobe's halo has sigma 0.130. At this fraction a
    /// lobe centre sits at 0.230, so:
    ///   centre + 1 sigma = 0.360  — comfortably behind the card
    ///   centre + 2 sigma = 0.490  — just past its edge, at 4.6% of peak
    /// which is the intent: the LOBE is behind the canvas and only the far tail of its
    /// glow reaches the side strips. Light from behind the card, never a visible blob
    /// orbiting in the open.
    private static let ringFraction: CGFloat = 0.50

    /// Frozen under Reduce Motion, exactly as `NoteAmbient` freezes its drift: the field
    /// is still there and still coloured, it simply stops moving.
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        // The shader needs a CONCRETE size: it derives the lobe ring from the view's own
        // half-extents so the ring hugs the visible frame around the card.
        GeometryReader { geo in
            TimelineView(.animation(paused: reduceMotion)) { tl in
                let t = reduceMotion ? 0 : tl.date.timeIntervalSinceReferenceDate
                let a = Angles(at: t)

                let anchor = anchorRect(in: geo.size)
                let unit = max(min(geo.size.width, geo.size.height), 1)

                Rectangle()
                    .fill(.black)
                    .colorEffect(ShaderLibrary.bohrBloom(
                        .float2(Float(geo.size.width), Float(geo.size.height)),
                        .float2(Float(anchor.midX), Float(anchor.midY)),
                        .float2(Float(anchor.width * 0.5 / unit * Self.ringFraction),
                                Float(anchor.height * 0.5 / unit * Self.ringFraction)),
                        .float(a.spin),
                        .float(a.sweep),
                        .float(a.hueCh),
                        .float(a.motion),
                        .color(Opal.hues[0]),
                        .color(Opal.hues[1]),
                        .color(Opal.hues[2])
                    ))
                    // ADDS to the ground rather than covering it — the same compositing
                    // the shell's press-bloom uses, and what keeps black black.
                    .blendMode(.plusLighter)
                    .opacity(intensity)
            }
        }
        // The recentring rides the KEYBOARD'S OWN CLOCK — the same `KeyboardMotion`
        // curve the RGB disc row travels on (Shell.beginNew/endNew). So when the keyboard
        // rises, the card, the three circles and the light behind them all move as one
        // piece, instead of three things arriving at three different times.
        .animation(KeyboardMotion.shared.animation, value: canvas)
        .allowsHitTesting(false)   // the ground beneath still takes the tap-to-discard
        .ignoresSafeArea()
    }

    /// The rect the field is anchored to — the card when we have its frame, the view
    /// itself before the first layout has reported one.
    private func anchorRect(in size: CGSize) -> CGRect {
        canvas.isEmpty ? CGRect(origin: .zero, size: size) : canvas
    }

    /// The clock, reduced to four SMALL numbers.
    ///
    /// Every angle is wrapped in Swift, in Double, before it crosses into Metal — the
    /// pattern `NoteAmbient` has always used (it evaluates its own `cos`/`sin` here and
    /// hands the view a plain offset). `timeIntervalSinceReferenceDate` is ~8.1e8, and
    /// feeding a Float32 of that magnitude to a GPU `cos` is asking for trouble: at 8e4
    /// radians one 60fps step is barely two ULP, so the motion quantises before the
    /// shader even starts. Wrapping here costs nothing and removes the whole class.
    ///
    /// Three periods that do not divide into each other, so the field never reads as a
    /// spinning wheel:
    ///   spin   2π/0.95 ≈ 6.6 s   the lobes' orbit
    ///   sweep  2π/0.67 ≈ 9.4 s   which lobe leads
    ///   motion 2π/0.90 ≈ 7.0 s   the centre and the bleed breathing together
    struct Angles {
        let spin: Float
        let sweep: Float
        let hueCh: Float
        let motion: Float

        init(at t: TimeInterval) {
            let twoPi = 2 * Double.pi
            spin  = Float((t * 0.95).truncatingRemainder(dividingBy: twoPi))
            // Counter-rotating, so the bright lobe walks around the ring rather than
            // sitting on one of them.
            sweep = Float((-t * 0.67).truncatingRemainder(dividingBy: twoPi) + twoPi)
            hueCh = Float((t * 0.10).truncatingRemainder(dividingBy: 3))
            motion = Float(0.30 + 0.70 * (0.5 + 0.5 * sin(t * 0.90)))
        }
    }
}
