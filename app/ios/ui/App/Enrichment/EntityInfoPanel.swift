//  EntityInfoPanel.swift
//  The panel presented when a user long-presses an enriched (matched) word. Shows
//  what Pacific knows about that entity — pulled from the cross-referenced
//  catalogue (the Notes outline, the Work projects, note titles, and Sysiphus SQL
//  records once connected). One panel, presented app-wide from the shell via
//  EnrichmentService.inspected.

import SwiftUI

struct EntityInfoPanel: View {
    let entity: EnrichedEntity
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var enrich: EnrichmentService
    @EnvironmentObject private var system: SystemService
    @State private var liveFields: [EntityField]?   // freshened from source (WS-D) for .sysiphus

    private var related: [EnrichedEntity] {
        // Other catalogue entities whose name appears in this entity's detail text.
        guard let d = entity.detail, !d.isEmpty else { return [] }
        let ids = Set(enrich.matches(in: d).map(\.entityId)).subtracting([entity.id])
        return ids.compactMap { enrich.entity($0) }.sorted { $0.name < $1.name }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 16) {
                    header
                        .accessibilityIdentifier("entity-info-panel")

                    if let s = entity.subtitle, !s.isEmpty {
                        Text(s)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }

                    if let d = entity.detail, !d.isEmpty {
                        card(title: "Notes", glyph: "text.alignleft") {
                            Text(d).font(.subheadline)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }

                    if !displayFields.isEmpty {
                        card(title: fieldsTitle, glyph: "list.bullet.rectangle") {
                            VStack(spacing: 8) {
                                ForEach(Array(displayFields.enumerated()), id: \.offset) { _, f in
                                    HStack(alignment: .top) {
                                        Text(f.label).foregroundStyle(.secondary)
                                        Spacer(minLength: 12)
                                        Text(f.value).multilineTextAlignment(.trailing)
                                    }
                                    .font(.subheadline)
                                }
                            }
                        }
                    }

                    if !related.isEmpty {
                        card(title: "Cross-referenced", glyph: "link") {
                            WrapLayout(spacing: 6) {
                                ForEach(related) { r in
                                    Button { enrich.inspected = r } label: {
                                        HStack(spacing: 4) {
                                            Image(systemName: r.kind.glyph).font(.caption2)
                                            Text(r.name).font(.caption.weight(.medium)).lineLimit(1)
                                        }
                                        .padding(.horizontal, 8).padding(.vertical, 5)
                                        .background(r.kind.tint.opacity(0.14), in: Capsule())
                                        .foregroundStyle(r.kind.tint)
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                        }
                    }

                    provenance
                }
                .padding()
            }
            .background(Brand.groupBG)
            .navigationTitle(entity.kind.word)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("Done") { dismiss() } } }
        }
        // LOOKUP live tier: for a Sysiphus design, re-read current stock / open orders from
        // source with the viewer's credential (WS-D). Cached fields render first; this updates.
        .task(id: entity.id) { await freshen() }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    /// Fields to render: the live source read if we have one, else the cached catalogue fields.
    private var displayFields: [EntityField] { liveFields ?? entity.fields }

    private func freshen() async {
        liveFields = nil
        let prefix = "sysiphus:"
        guard entity.origin == .sysiphus, entity.id.hasPrefix(prefix) else { return }
        let designId = String(entity.id.dropFirst(prefix.count))
        if let fresh = await system.lookupLive(designId) {
            liveFields = EnrichedEntity(sysiphus: fresh).fields
        }
    }

    private var header: some View {
        VStack(spacing: 10) {
            ZStack {
                Circle().fill(entity.kind.tint.gradient).frame(width: 64, height: 64)
                Image(systemName: entity.kind.glyph)
                    .font(.system(size: 26, weight: .semibold))
                    .foregroundStyle(.white)
            }
            Text(entity.name).font(.title2.bold()).multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 4)
    }

    private var fieldsTitle: String {
        let base: String
        switch entity.origin {
        case .sysiphus: base = "Sysiphus record"
        case .outline:  base = "Tags"
        case .timeline: base = "Project record"
        case .music:    base = "RA listing"
        case .art:      base = "ArtRabbit listing"
        default:        base = "Details"
        }
        return liveFields != nil ? base + " · live" : base   // freshened from source
    }

    private var provenance: some View {
        let (glyph, text): (String, String) = {
            switch entity.origin {
            case .outline:  return ("lock.fill", "From your outline — on this device")
            case .timeline: return ("square.grid.2x2.fill", "From your Work projects — reads from Sysiphus")
            case .sysiphus: return ("cylinder.split.1x2.fill", "From Sysiphus — linked SQL data")
            case .note:     return ("note.text", "From your Notes")
            case .chat:     return ("bubble.left.fill", "From a conversation")
            case .music:    return ("music.note", "From Resident Advisor — public listings")
            case .art:      return ("paintpalette", "From ArtRabbit — public listings")
            }
        }()
        return Label(text, systemImage: glyph)
            .font(.caption)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
            .padding(.top, 4)
    }

    @ViewBuilder
    private func card<Content: View>(title: String, glyph: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(title, systemImage: glyph)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Brand.accent)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding()
        .background(Brand.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}
