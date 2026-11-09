//  DoorChoice.swift
//  An object's door, as the creator offers it (#76, ICD 2.6.0 `base.setDoor`).

import Foundation
import PacificCore

/// A door, as the creator offers it.
enum DoorChoice: String, CaseIterable, Identifiable {
    case none = ""
    case review
    case auto

    var id: String { rawValue }

    var title: String {
        switch self {
        case .none:   return "No way in"
        case .review: return "Ask to join"
        case .auto:   return "Open"
        }
    }
}
