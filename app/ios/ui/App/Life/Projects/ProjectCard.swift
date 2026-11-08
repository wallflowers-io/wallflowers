//  ProjectCard.swift
//  The folded header for a Project — the PROJECTS category's column row.
//
//  Extracted from the retired WORK tab. `WorkView` itself is gone: Shell records that
//  "Projects now live under LIFE (it absorbed Work)", and the only remaining mention
//  was a stale comment elsewhere. The card outlived the tab because LIFE always
//  reused it; it belongs to the category, not to a screen that no longer exists.

import SwiftUI
import PacificCore


struct RealProjectCard: View {
    @EnvironmentObject private var core: CoreService
    let objectId: String
    @State private var header: ProjectHeaderRow?
    @State private var itemCount = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 12) {
                LogoBadge(initials: initials, tint: Brand.accent)
                VStack(alignment: .leading, spacing: 2) {
                    Text(displayTitle).font(.headline)
                    Text(statusLine).font(.caption.weight(.semibold)).foregroundStyle(Brand.accent)
                }
                Spacer(minLength: 0)
                Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary)
            }
            HStack(spacing: 12) {
                Label("\(itemCount) items", systemImage: "list.bullet")
                if let pc = header?.pieceCount, pc > 0 {
                    Label("\(pc)-piece body", systemImage: "square.grid.3x3.fill")
                }
                Spacer()
            }
            .font(.caption).foregroundStyle(.secondary)
        }
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .onAppear(perform: load)
        .onChange(of: core.syncTick) { _, _ in load() }
    }

    private var displayTitle: String {
        let t = header?.title ?? ""
        return t.isEmpty ? "Untitled project" : t
    }
    private var initials: String {
        let t = header?.title ?? ""
        return t.isEmpty ? "P" : String(t.prefix(2)).uppercased()
    }
    private var statusLine: String {
        switch header?.status {
        case "archived": return "Archived"
        case "paused":   return "Paused"
        default:         return "Active"
        }
    }
    private func load() {
        header = core.projectHeader(objectId)
        itemCount = core.projectItems(objectId).count
    }
}
