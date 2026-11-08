//  SysiphusEnrichmentBridge.swift
//  Bridges a credentialed System's Fields (SysiphusField) into the ONE enrichment
//  catalogue as .product entities, so a signed-in employee's REAL Sysiphus designs
//  light up wherever EnrichedText / EnrichedLabel render (chat bubbles, note bodies,
//  work notes) and long-press opens the info panel — through the mature enrichment
//  path, not a second renderer.
//
//  This is the credentialed feeder for EntityOrigin.sysiphus. The SQLite dev fixture
//  (Sysiphus/SysiphusService) writes the same origin; the clean end-state retires that
//  fixture and leaves this as the sole .sysiphus source (the unify — coordinated).

import Foundation

extension EnrichedEntity {
    /// Project one design Field into a catalogue entity. `aliases` become surface forms
    /// (the recall surface the trie scans); the rolled-up Shopify/BrightPearl/Warewolf ids,
    /// stock and open orders become the panel's Sysiphus record.
    init(sysiphus f: SysiphusField) {
        var fields: [EntityField] = []
        if let c = f.category { fields.append(EntityField(label: "Category", value: c)) }
        if let m = f.material { fields.append(EntityField(label: "Material", value: m)) }
        if let fit = f.fit { fields.append(EntityField(label: "Fit", value: fit)) }
        if let p = f.wholesalePrice { fields.append(EntityField(label: "Wholesale", value: "£\(Int(p))")) }
        fields.append(EntityField(label: "Stock",
                                  value: "\(f.stock.available) avail / \(f.stock.onHand) on hand · \(f.stock.location)"))
        if !f.openOrders.isEmpty {
            let orders = Set(f.openOrders.map { "\($0.stockist) (\($0.status))" }).sorted().joined(separator: ", ")
            fields.append(EntityField(label: "Open orders", value: orders))
        }
        for (sys, id) in f.systems.sorted(by: { $0.key < $1.key }) {
            fields.append(EntityField(label: sys.capitalized, value: id))
        }

        self.init(
            id: "sysiphus:\(f.id)",
            name: f.name,
            kind: .product,
            origin: .sysiphus,
            subtitle: [f.season, f.category].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "),
            detail: f.description,
            fields: fields,
            surfaceForms: f.aliases
        )
    }
}
