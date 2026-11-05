//  PlaygroundApp.swift
//  The design bench: a blank iPad app compiled from the WallFlowers sources (minus the
//  app's own @main), so every element on the page is the app's real view. No core, no
//  identity — it opens straight onto the page.

import SwiftUI

@main
struct PlaygroundApp: App {
    /// The one environment object the primitives read (EnrichedText).
    @StateObject private var enrich = EnrichmentService()
    @State private var key = "kenjin"

    var body: some Scene {
        WindowGroup {
            PlaygroundPage(key: $key)
                .environmentObject(enrich)
                .tint(Brand.accent)
                .preferredColorScheme(key == "pacific" ? .light : .dark)
        }
    }
}
