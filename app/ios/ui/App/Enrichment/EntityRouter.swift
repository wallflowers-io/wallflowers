//  EntityRouter.swift
//  The type-router. A resolved entity opens its REAL context, dispatched by type, instead of
//  the one generic info panel: discuss a Project -> the Project screen; a ChatRoom -> its
//  thread; an outline Person/Org/Event -> its outline detail; a System Field -> its live
//  record. Seed / note / unknown entities fall back to the info panel.
//
//  The destination is read from the catalogue id namespace (minted in EnrichmentService ingest
//  + the Sysiphus bridge), so routing needs no new state — it dispatches what already resolves:
//    outline:<id>    -> the live outline node            EntityDetailView
//    thread:<objId>  -> a ChatRoom conversation          ThreadView          (lights up once
//    project:<objId> -> a real Project GroupObject        ProjectDetailView    conversations /
//    else            -> Field (live) / seed / note        EntityInfoPanel      projects ingest)

import SwiftUI

enum EntityRoute {
    case outline(EntityVM)                              // the outline graph node
    case thread(objectId: String, title: String)       // a real ChatRoom (Forum/Group) object
    case project(objectId: String)                     // a real Project GroupObject
    case panel(EnrichedEntity)                          // EntityInfoPanel: Field (live) / seed / note

    /// Decide the destination for a resolved catalogue entity from its id namespace.
    static func route(_ e: EnrichedEntity) -> EntityRoute {
        if e.id.hasPrefix("outline:"),
           let vm = OutlineModel.shared.entity(String(e.id.dropFirst("outline:".count))) {
            return .outline(vm)
        }
        if e.id.hasPrefix("thread:") {
            return .thread(objectId: String(e.id.dropFirst("thread:".count)), title: e.name)
        }
        if e.id.hasPrefix("project:") {
            return .project(objectId: String(e.id.dropFirst("project:".count)))
        }
        return .panel(e)   // Sysiphus Fields render here too — EntityInfoPanel freshens them live.
    }
}

/// Presents the destination for the long-pressed entity, dispatched by type. The real detail
/// views normally live in a NavigationStack (pushed), so they're wrapped + given a Done button
/// for sheet presentation; the info panel is already self-contained.
struct EntityRouter: View {
    let entity: EnrichedEntity
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        switch EntityRoute.route(entity) {
        case .outline(let vm):
            routed { EntityDetailView(entity: vm) }
        case .thread(let objectId, let title):
            routed { ThreadView(objectId: objectId, title: title) }
        case .project(let objectId):
            routed { ProjectDetailView(objectId: objectId) }
        case .panel(let e):
            EntityInfoPanel(entity: e)   // own NavigationStack + detents + Done
        }
    }

    @ViewBuilder
    private func routed<V: View>(@ViewBuilder _ content: () -> V) -> some View {
        NavigationStack {
            content()
                .toolbar { ToolbarItem(placement: .topBarLeading) { Button("Done") { dismiss() } } }
        }
    }
}
