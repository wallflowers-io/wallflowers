//  CompassBand.swift
//  The compass, pared to its essence: the bearing NUMBER (000–360) printed above the yellow
//  CHAT disc (positioned in the shell), plus a turn-ratchet HAPTIC. No bar, no arrow, no
//  degree symbol, no letters — just the digits.
//
//  Haptic — the answer to "is there a cooldown?": it fires once per 5° (subtle) and 45°
//  (firm) ACTUALLY TURNED, not when the heading lands on an absolute multiple. The angular
//  threshold IS the cooldown, so a fast turn can't skip a mark and boundary jitter can't
//  double-fire. It reads heading from the environment, so heading updates re-render only this.

import SwiftUI
import UIKit

struct CompassReadout: View {
    @EnvironmentObject private var location: LocationService

    @State private var anchor5: Double?
    @State private var anchor45: Double?

    private var heading: Double? { location.heading }

    var body: some View {
        Text(readout)
            .font(.system(size: 15, weight: .semibold, design: .rounded))
            .monospacedDigit()
            .foregroundStyle(location.headingLocked ? LifeFilter.places.color : .white)
            .shadow(color: .black.opacity(0.45), radius: 2)
            .contentShape(Rectangle())
            .onTapGesture {
                location.headingLocked.toggle()   // lock ⇒ map rotation decouples from the device
                UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
            }
            .onChange(of: heading) { _, h in ratchet(h) }
    }

    private var readout: String {
        guard let h = heading else { return "000" }
        let deg = ((Int(h.rounded()) % 360) + 360) % 360
        return String(format: "%03d", deg)
    }

    /// A subtle tick every 5° TURNED, a firmer one every 45° turned — threshold-based, so it
    /// can neither skip marks on a fast turn nor double-fire on jitter.
    private func ratchet(_ h: Double?) {
        guard let h else { return }
        if anchor5 == nil { anchor5 = h }
        if anchor45 == nil { anchor45 = h }
        if delta(h, anchor45!) >= 45 {
            anchor45 = h; anchor5 = h
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        } else if delta(h, anchor5!) >= 5 {
            anchor5 = h
            UISelectionFeedbackGenerator().selectionChanged()
        }
    }

    /// Shortest angular distance between two bearings, in degrees [0, 180].
    private func delta(_ a: Double, _ b: Double) -> Double {
        let d = abs(a - b).truncatingRemainder(dividingBy: 360)
        return min(d, 360 - d)
    }
}
