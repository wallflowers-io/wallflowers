//  ScanRow.swift
//  A row in a place's "Scans" section — one reconstructed 3D capture of the place.

import SwiftUI

struct ScanRow: View {
    let scan: Scan
    var onDelete: (() -> Void)? = nil

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "cube.transparent.fill")
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(Brand.accent)
                .frame(width: 30, height: 30)
                .background(Brand.accent.opacity(0.12),
                            in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            VStack(alignment: .leading, spacing: 1) {
                Text(scan.derivedName).font(.subheadline.weight(.medium))
                Text(scan.summary).font(.caption2).foregroundStyle(.secondary)
            }
            Spacer()
            if let f = scan.fusion {
                Text(f).font(.caption2.weight(.medium)).foregroundStyle(.secondary)
                    .padding(.horizontal, 6).padding(.vertical, 2)
                    .background(Brand.groupBG, in: Capsule())
            }
            Text(scan.format.uppercased()).font(.caption2).foregroundStyle(.tertiary)
        }
        .contentShape(Rectangle())
        .contextMenu {
            if let onDelete {
                Button(role: .destructive) { onDelete() } label: {
                    Label("Delete scan", systemImage: "trash")
                }
            }
        }
    }
}
