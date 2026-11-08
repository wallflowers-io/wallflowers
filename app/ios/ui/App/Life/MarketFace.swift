//  MarketFace.swift
//  The market FACE (the LIFE screen's axial second face) was REMOVED 10 Aug —
//  TRADE and JOB are jewels on the one-page rail (FEED-TRANSMISSION 0-ANSWER-G).
//  What survives is the category vocabulary: GroupDetailView's rail borrows its
//  glyphs and tints.

import SwiftUI

enum MarketFaceCategory: String, CaseIterable, Identifiable {
    case inventory, skills, trade, jobs
    var id: String { rawValue }

    var title: String { rawValue.uppercased() }

    var glyph: String {
        switch self {
        case .inventory: return "shippingbox.fill"      // continuity: the retired THINGS jewel
        case .skills:    return "wrench.and.screwdriver.fill"
        case .trade:     return "arrow.left.arrow.right"
        case .jobs:      return "hammer.fill"
        }
    }

    /// Fixed tints, like the first face's seed tints. TRADE wears the exchange blue the
    /// design's trade face carried; INVENTORY keeps the retired THINGS teal.
    var tint: Color {
        switch self {
        case .inventory: return .teal
        case .skills:    return .purple
        case .trade:     return Color(red: 0.18, green: 0.44, blue: 0.72)
        case .jobs:      return .brown
        }
    }
}
