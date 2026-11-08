//  Lookup.swift
//  Workstream D — the LOOKUP seam. RESOLVE gives an entity id; LOOKUP turns it into a
//  record in two tiers: (a) the on-device cached Field (instant), then (b) a live,
//  credential-scoped read of the source (Sysiphus) so volatile facets (stock, open
//  orders) are current — the "context from source" step. This view wraps the design
//  card: it renders the cached Field immediately, then freshens from source.

import SwiftUI

struct LiveDesignCard: View {
    /// The cached Field (from resolution/index).
    let cached: SysiphusField
    var primary: Bool = true
    var score: Float? = nil

    @EnvironmentObject private var system: SystemService
    @State private var live: SysiphusField?

    var body: some View {
        DesignEntryCard(field: live ?? cached, primary: primary, score: score, isLive: live != nil)
            // Live tier: re-read this design from source with the viewer's credential.
            .task(id: cached.id) { live = await system.lookupLive(cached.id) }
    }
}
