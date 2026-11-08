//  SysiphusField.swift
//  A Field on a System GroupObject: the on-device projection of one Sysiphus
//  entry (here, a DESIGN — the human unit of reference, above SKUs). A System is
//  scoped to a shared credential; because each Aries employee has their own login,
//  each has their own System (group-of-1) and pulls their own Fields. Resolution
//  targets these Fields, so "talk in human → the design's Sysiphus entry".

import Foundation

/// The employee a System is scoped to (their credential = the System's membership).
struct Employee: Codable, Hashable, Identifiable {
    let id: String
    let username: String
    let name: String
    let role: String
}

/// One design Field, rolled up by Sysiphus across Shopify / BrightPearl / Warewolf.
struct SysiphusField: Codable, Hashable, Identifiable {
    let id: String                 // Sysiphus design id, e.g. "DSGN-RAM-CARGO"
    var name: String
    var season: String?
    var collection: String?
    var category: String?
    var material: String?
    var fit: String?
    var description: String?
    var wholesalePrice: Double?
    var status: String?
    var systems: [String: String]  // shopify / brightpearl / warewolf ids
    var aliases: [String]
    var variants: [Variant]
    var stock: Stock
    var openOrders: [OpenOrder]
    var resolutionText: String     // every human phrasing — the recall surface

    struct Variant: Codable, Hashable {
        var sku: String
        var colourway: String
        var size: String
        var onHand: Int
        var allocated: Int
    }
    struct Stock: Codable, Hashable {
        var onHand: Int
        var allocated: Int
        var available: Int
        var location: String
    }
    struct OpenOrder: Codable, Hashable {
        var order: String
        var stockist: String
        var status: String
        var qty: Int
    }

    /// Colourways available (deduped), for the panel.
    var colourways: [String] { Array(Set(variants.map(\.colourway))).sorted() }
    var sizes: [String] { Array(Set(variants.map(\.size))).sorted() }
}
