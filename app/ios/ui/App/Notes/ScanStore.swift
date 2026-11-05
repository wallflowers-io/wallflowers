//  ScanStore.swift
//  On-device store for spatial SCANS — reconstructed 3D captures (point clouds) of a
//  PLACE. Mirrors NotesStore exactly: plain local JSON in Application Support, fully
//  on-device, real-ops only (starts EMPTY; a scan appears only when a real capture or
//  import registers one — no fixtures, matching NotesStore / OutlineModel).
//
//  A scan belongs to a PLACE entity (Notes → Outline → Places) via `placeID`
//  (the EntityVM.id). The scan surface lives in that place's EntityDetailView.

import Foundation

struct Scan: Identifiable, Codable, Equatable, Hashable {
    var id = UUID()
    var placeID: String                 // EntityVM.id of the place this scan captures
    var name: String = ""
    var capturedAt: Date = Date()
    var pointCount: Int = 0
    var format: String = "glb"          // glb | ply | usdz
    var assetRef: String = ""           // filename / relative path of the geometry asset
    var device: String? = nil           // provenance: where it was reconstructed
    var fusion: String? = nil           // provenance: fusion mode (overlap / arkit / raw)

    var derivedName: String {
        let t = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return t.isEmpty ? "Scan" : t
    }

    var summary: String {
        let pts = pointCount > 0 ? "\(pointCount.formatted()) pts" : "—"
        return capturedAt.formatted(date: .abbreviated, time: .omitted) + " · " + pts
    }
}

@MainActor
final class ScanStore: ObservableObject {
    static let shared = ScanStore()

    @Published private(set) var scans: [Scan] = []

    private let url: URL

    init() {
        let base = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        url = base.appendingPathComponent("scans.json")
        load()
    }

    private func load() {
        guard let data = try? Data(contentsOf: url),
              let decoded = try? JSONDecoder().decode([Scan].self, from: data) else {
            return  // no decodable scans.json ⇒ the real empty state (never clobbered)
        }
        scans = decoded.sorted { $0.capturedAt > $1.capturedAt }
    }

    private func persist() {
        guard let data = try? JSONEncoder().encode(scans) else { return }
        try? data.write(to: url, options: .atomic)
    }

    /// Scans captured for a given place, newest first.
    func scans(forPlace placeID: String) -> [Scan] {
        scans.filter { $0.placeID == placeID }.sorted { $0.capturedAt > $1.capturedAt }
    }

    /// Register a real scan — from an on-device capture or an import of a pipeline asset.
    @discardableResult
    func register(_ scan: Scan) -> Scan {
        scans.insert(scan, at: 0)
        scans.sort { $0.capturedAt > $1.capturedAt }
        persist()
        return scan
    }

    func delete(_ scan: Scan) {
        scans.removeAll { $0.id == scan.id }
        persist()
    }
}
