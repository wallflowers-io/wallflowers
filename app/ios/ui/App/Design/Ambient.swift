//  Ambient.swift
//  The "idle · alive" ground (from MotionLab): slow-drifting jewel blobs on deep
//  slate, heavily blurred — the subtle lighting-on-black the NOTE compose card
//  refracts, now shared with the forum room's ground. One ambient, one place;
//  frozen to a single static frame when Reduce Motion is on.

import SwiftUI

struct NoteAmbient: View {
    /// Replaces the indigo blob, so a compose surface can carry its own identity in the
    /// ground without a second copy of this view. `nil` keeps the note's indigo.
    var tint: Color? = nil

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Group {
            if reduceMotion {
                blobs(at: 0)
            } else {
                TimelineView(.animation) { ctx in
                    blobs(at: ctx.date.timeIntervalSinceReferenceDate)
                }
            }
        }
        .blur(radius: 56)
    }

    @ViewBuilder private func blobs(at s: Double) -> some View {
        ZStack {
            drift(tint ?? Color(hex: 0x7A6FF0), speed: 0.13, s, phase: 0)   // indigo, or the surface's tint
            drift(Color(hex: 0x33C08C), speed: 0.10, s, phase: 2)   // emerald
            drift(Color(hex: 0x49A6F2), speed: 0.16, s, phase: 4)   // azure
            drift(Color(hex: 0xF0669E), speed: 0.09, s, phase: 1)   // rose
        }
    }

    private func drift(_ c: Color, speed sp: Double, _ s: Double, phase ph: Double) -> some View {
        Circle().fill(c)
            .frame(width: 240, height: 240)
            .offset(x: CGFloat(cos(s * sp + ph)) * 150,
                    y: CGFloat(sin(s * sp * 1.3 + ph)) * 300)
            .opacity(0.45)
    }
}
