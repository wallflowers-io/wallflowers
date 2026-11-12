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

/// How much of an Event's Place its outside shows (`base.setExterior.precision`, a geohash
/// length; Ralph, 12 Nov). The owner's choice; the venue's name is shown at every step.
enum PlaceShown: UInt8, CaseIterable, Identifiable {
    case name = 0
    case area = 5
    case street = 7
    case exact = 9

    var id: UInt8 { rawValue }

    var title: String {
        switch self {
        case .name:   return "Venue name only"
        case .area:   return "The area"
        case .street: return "The street"
        case .exact:  return "Exact place"
        }
    }
}
