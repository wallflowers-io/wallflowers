//  KeyboardMotion.swift
//  Mirrors the iOS keyboard's own show/hide animation so the compose disc-slide
//  moves in lockstep with the keyboard rather than on a guessed spring. It reads
//  the REAL duration + curve straight from the keyboard notification
//  (UIResponder.keyboard* userInfo), per Apple's "Managing the Keyboard" docs.

import SwiftUI
import UIKit

final class KeyboardMotion {
    static let shared = KeyboardMotion()

    // Defaults match the system keyboard (0.25s, its private curve 7) so the very
    // first compose — before any notification has been seen — is already in step.
    private(set) var duration: Double = 0.25
    private(set) var curveRaw: Int = 7

    private init() {
        let nc = NotificationCenter.default
        nc.addObserver(self, selector: #selector(capture(_:)),
                       name: UIResponder.keyboardWillShowNotification, object: nil)
        nc.addObserver(self, selector: #selector(capture(_:)),
                       name: UIResponder.keyboardWillHideNotification, object: nil)
    }

    @objc private func capture(_ note: Notification) {
        let info = note.userInfo
        if let d = info?[UIResponder.keyboardAnimationDurationUserInfoKey] as? Double, d > 0 {
            duration = d
        }
        if let c = info?[UIResponder.keyboardAnimationCurveUserInfoKey] as? Int {
            curveRaw = c
        }
    }

    /// The keyboard's timing as a SwiftUI animation. Curves 0–3 are the public
    /// `UIView.AnimationCurve` cases; the keyboard's own curve (7) is matched by a
    /// cubic Bézier approximation.
    var animation: Animation {
        switch curveRaw {
        case 0:  return .easeInOut(duration: duration)
        case 1:  return .easeIn(duration: duration)
        case 2:  return .easeOut(duration: duration)
        case 3:  return .linear(duration: duration)
        default: return .timingCurve(0.17, 0.59, 0.25, 1.0, duration: duration)  // keyboard curve 7
        }
    }
}
