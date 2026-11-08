//  Media.swift
//  MEDIA — photos and videos as a first-class LIFE category, and the mechanism by
//  which several people's captures of the same subject become one 3D model.
//
//  The chain is: a capture carries METADATA → metadata binds it to a PLACE and a
//  SUBJECT → contributions to a subject accumulate ACROSS CONTRIBUTORS → past a
//  threshold the subject earns a reconstruction and becomes a Thing with geometry.
//
//  Why this is tractable rather than aspirational: "me + John + Hope each photograph
//  the car" is an UNPOSED, NON-SEQUENTIAL, MULTI-SOURCE image collection. That is the
//  literal problem statement of ACE0 (Scene Coordinate Reconstruction, ECCV 2024):
//  "we do not require pose priors nor sequential inputs, and we optimize efficiently
//  over thousands of images". VGGT (already running in arc/spatial) does the same for
//  a window of frames in one feed-forward pass. So the reconstructor for a federated
//  photo pile is a solved-shaped problem, not a new one.
//
//  What it is NOT: a promise of metric scale. Both VGGT and ACE0 recover geometry up
//  to a similarity transform. The kitchen scan that ships with this build measures
//  1.61 m across for a real kitchen for exactly that reason. So a reconstruction lands
//  with `scaleResolved = false` until a metric anchor (ARKit VIO on any one
//  contributor's capture) pins it. See Space.swift.

import Foundation
import simd
import CoreLocation

// MARK: - A single capture

struct MediaAsset: Identifiable, Codable, Hashable {
    enum Kind: String, Codable { case photo, video }

    var id: String = UUID().uuidString
    var kind: Kind
    /// Local filename / relative path of the pixels. Never uploaded by this model.
    var assetRef: String
    var capturedAt: Date

    // --- Capture metadata: the evidence that binds it to the graph ---

    /// EXIF GPS, when present. Absent is normal and handled honestly — an unplaced
    /// capture still contributes to a subject, it just cannot bind to a Place by
    /// location alone.
    var coordinate: GeoCoordinate?
    /// EXIF heading in degrees true, when present. Cheap view-diversity signal.
    var heading: Double?
    /// Frame count for a video; 1 for a photo. Drives how much a capture contributes.
    var frameCount: Int = 1

    // --- Federation ---

    /// WHO captured it. The whole point: contributions are per-author, and a subject
    /// reconstructs from the UNION across authors.
    var contributorID: String

    // --- Resolution (filled by binding, not by the capturer) ---

    /// The Place this capture was taken at, resolved from metadata.
    var placeID: String?
    /// The Thing this capture depicts, once the subject is resolved.
    var subjectID: String?

    /// Effective mapping frames this asset can contribute to a reconstruction.
    var mappingFrames: Int { kind == .video ? max(1, frameCount) : 1 }
}

// MARK: - Binding a capture to a Place

enum MediaBinding {
    /// Bind by proximity: the nearest Space whose *volume* contains the capture point,
    /// else the nearest anchored Space within `radius`. Containment beats proximity —
    /// a photo taken inside a bar belongs to the bar, not to the closer shopfront.
    ///
    /// Returns nil when the capture has no coordinate, which is honest rather than a
    /// guess: unplaced media is listed under MEDIA and bound later by other evidence.
    static func place(for asset: MediaAsset,
                      in graph: SpaceGraph,
                      origin: CLLocationCoordinate2D,
                      radius: Double = 60) -> String? {
        guard let c = asset.coordinate else { return nil }
        let world = ENU.offset(from: origin, to: c.clCoordinate)

        if let inside = graph.spacesContaining(world).first { return inside.id }

        var best: (id: String, d: Double)?
        for (id, _) in graph.spaces {
            guard let wp = graph.worldPose(of: id) else { continue }
            let d = simd_distance(wp.translation, world)
            if d <= radius, best == nil || d < best!.d { best = (id, d) }
        }
        return best?.id
    }
}

// MARK: - Accumulating contributions into a reconstruction

/// A subject (a car, a sculpture, a shopfront) accruing captures from several people.
/// This is the Thing-in-the-graph that photos and videos construct.
struct SubjectReconstruction: Identifiable, Codable, Hashable {
    enum State: String, Codable {
        /// Not enough material yet — the UI says what is still needed.
        case insufficient
        /// Threshold met; a reconstruction can be requested.
        case ready
        case reconstructing
        /// Has geometry. Scale may still be unresolved — see Space.scaleResolved.
        case reconstructed
    }

    var id: String
    var label: String
    /// Every capture bound to this subject, across all contributors.
    var contributions: [MediaAsset] = []
    var state: State = .insufficient
    /// The Space this subject became once it had geometry.
    var spaceID: String?

    /// Distinct people who have contributed. The federation dimension — three people
    /// with one photo each beats one person with three near-identical photos, because
    /// independent capture positions are what give a reconstruction its baseline.
    var contributors: Set<String> { Set(contributions.map(\.contributorID)) }
    var mappingFrames: Int { contributions.reduce(0) { $0 + $1.mappingFrames } }

    /// Rough view-diversity proxy from EXIF heading: the spread of directions the
    /// subject was shot from. Reconstruction needs baseline, not volume — twenty
    /// photos from one spot reconstruct nothing.
    var headingSpread: Double {
        let h = contributions.compactMap(\.heading).sorted()
        guard h.count >= 2 else { return 0 }
        // Largest angular gap subtracted from 360 = the arc actually covered.
        var gaps: [Double] = zip(h, h.dropFirst()).map { $1 - $0 }
        gaps.append(360 - (h.last! - h.first!))
        return 360 - (gaps.max() ?? 360)
    }

    /// The gate. Deliberately conservative: reconstruction that fails is worse than
    /// one that waits, because a failed model is what the user remembers.
    static let minContributors = 2
    static let minFrames = 8
    static let minHeadingSpread: Double = 45

    mutating func reassess() {
        guard state != .reconstructing, state != .reconstructed else { return }
        state = (contributors.count >= Self.minContributors
                 && mappingFrames >= Self.minFrames
                 && headingSpread >= Self.minHeadingSpread) ? .ready : .insufficient
    }

    /// What is still missing, for the UI to state plainly instead of showing a
    /// disabled button with no reason.
    var shortfall: String? {
        guard state == .insufficient else { return nil }
        var need: [String] = []
        if contributors.count < Self.minContributors {
            need.append("\(Self.minContributors - contributors.count) more contributor\(contributors.count == Self.minContributors - 1 ? "" : "s")")
        }
        if mappingFrames < Self.minFrames { need.append("\(Self.minFrames - mappingFrames) more frames") }
        if headingSpread < Self.minHeadingSpread { need.append("wider coverage") }
        return need.isEmpty ? nil : need.joined(separator: " · ")
    }
}

// MARK: - Store

@MainActor
final class MediaStore: ObservableObject {
    static let shared = MediaStore()

    @Published private(set) var assets: [MediaAsset] = []
    @Published private(set) var subjects: [String: SubjectReconstruction] = [:]

    private let url: URL

    init() {
        let base = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("pacific", isDirectory: true)
        try? FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        url = base.appendingPathComponent("media.json")
        load()
    }

    private struct Persisted: Codable {
        var assets: [MediaAsset]
        var subjects: [String: SubjectReconstruction]
    }

    private func load() {
        guard let d = try? Data(contentsOf: url),
              let p = try? JSONDecoder().decode(Persisted.self, from: d) else { return }
        assets = p.assets; subjects = p.subjects
    }

    private func persist() {
        guard let d = try? JSONEncoder().encode(Persisted(assets: assets, subjects: subjects)) else { return }
        try? d.write(to: url, options: .atomic)
    }

    /// Register a capture and fold it into its subject's reconstruction state.
    func add(_ asset: MediaAsset) {
        assets.insert(asset, at: 0)
        if let sid = asset.subjectID {
            var s = subjects[sid] ?? SubjectReconstruction(id: sid, label: sid)
            s.contributions.append(asset)
            s.reassess()
            subjects[sid] = s
        }
        persist()
    }

    func assets(forPlace placeID: String) -> [MediaAsset] {
        assets.filter { $0.placeID == placeID }
    }
    func assets(forSubject subjectID: String) -> [MediaAsset] {
        assets.filter { $0.subjectID == subjectID }
    }
}
