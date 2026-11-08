//  Space.swift
//  A SPACE is a local frame with extent and geometry — the thing you can be INSIDE.
//  This is the primitive `PlaceAnnotation`/`ENUProjector` could not express: those
//  carry a 2-DoF ground position (east/north, Y pinned to 0) and no orientation at
//  all, which is half the problem when you are placing a *room* rather than a pin.
//
//  Three things a Place does not have and a Space must:
//
//    1. POSE — 6-DoF, plus SCALE. Sim(3), not SE(3). Not theory: the VGGT
//       reconstruction this ships with measures 1.61 × 0.94 × 1.70 m for a real
//       kitchen, because feed-forward reconstruction is correct only up to a
//       similarity transform. Scale is a
//       value that must be carried and resolved, so it is a field. In the capture
//       path ARKit VIO resolves it (metric, gravity-aligned); until then a Space
//       declares `scaleResolved = false` and says so rather than lying.
//
//    2. EXTENT — a volume, so "am I inside?" is answerable. A pin cannot answer it.
//
//    3. LEVEL — a DISCRETE ordinal, not a continuous altitude. A basement bar is
//       level -1. GPS altitude is ±10-20 m, which is worse than the floor spacing
//       it would be discriminating; Apple IMDF and OGC IndoorGML both use ordinals
//       for exactly this reason.
//
//  Frames NEST, and the anchor is what a Space hangs from:
//    • `.geodetic` — pinned to the real world at an ENU tangent origin.
//    • `.parent`   — inside another Space. A skin is just a child Space, so skins
//                    compose and can themselves be skinned. No separate type.
//    • `.stream`   — the frame's world pose is LIVE and arrives out-of-band. This
//                    is the tour bus, and it reuses geo.rs's LocationStream shape
//                    verbatim: carry the ACCESS, never the positions.
//
//  Mirrors pacific-core's `geo.rs` discipline — see the fixed-point note on Pose.

import Foundation
import simd
import CoreLocation

// MARK: - Pose (Sim(3))

/// A rigid transform plus uniform scale: the transform placing a Space's local frame
/// inside its parent's frame.
///
/// Stored in FIXED POINT, like `geo.rs` stores coordinates as e7 integers, and for the
/// same reason: these values fold into an append-only log that must replay identically
/// on every replica, and float non-determinism would break that. Translation is
/// micrometres (i64 → ±9.2e12 m of range at 1 µm resolution); rotation is a unit
/// quaternion and scale is a ratio, both at 1e9 (n9).
struct Pose: Codable, Hashable {
    var xUm: Int64 = 0, yUm: Int64 = 0, zUm: Int64 = 0
    var qxN9: Int32 = 0, qyN9: Int32 = 0, qzN9: Int32 = 0, qwN9: Int32 = 1_000_000_000
    var scaleN9: Int64 = 1_000_000_000

    static let identity = Pose()
    private static let um: Double = 1_000_000
    private static let n9: Double = 1_000_000_000

    init() {}

    init(translation t: SIMD3<Double>, rotation q: simd_quatd = simd_quatd(ix: 0, iy: 0, iz: 0, r: 1), scale s: Double = 1) {
        xUm = Int64((t.x * Pose.um).rounded())
        yUm = Int64((t.y * Pose.um).rounded())
        zUm = Int64((t.z * Pose.um).rounded())
        let n = simd_normalize(q)
        qxN9 = Int32((n.imag.x * Pose.n9).rounded())
        qyN9 = Int32((n.imag.y * Pose.n9).rounded())
        qzN9 = Int32((n.imag.z * Pose.n9).rounded())
        qwN9 = Int32((n.real   * Pose.n9).rounded())
        scaleN9 = Int64((s * Pose.n9).rounded())
    }

    var translation: SIMD3<Double> {
        SIMD3(Double(xUm) / Pose.um, Double(yUm) / Pose.um, Double(zUm) / Pose.um)
    }
    /// Re-normalised on read: fixed-point rounding perturbs the unit norm slightly.
    var rotation: simd_quatd {
        let q = simd_quatd(ix: Double(qxN9) / Pose.n9, iy: Double(qyN9) / Pose.n9,
                           iz: Double(qzN9) / Pose.n9, r: Double(qwN9) / Pose.n9)
        return simd_length(q.vector) > 0 ? simd_normalize(q) : simd_quatd(ix: 0, iy: 0, iz: 0, r: 1)
    }
    var scale: Double { Double(scaleN9) / Pose.n9 }

    /// This pose as a 4×4 transform (scale, then rotate, then translate).
    var matrix: simd_double4x4 {
        let r = simd_double3x3(rotation) * scale
        return simd_double4x4(
            SIMD4(r.columns.0, 0), SIMD4(r.columns.1, 0), SIMD4(r.columns.2, 0),
            SIMD4(translation, 1)
        )
    }

    /// Compose: `parent.compose(child)` gives the child expressed in the parent's parent.
    /// This is what walking an anchor chain does at every hop.
    func compose(_ child: Pose) -> Pose {
        let s = scale * child.scale
        let r = rotation * child.rotation
        let t = translation + rotation.act(child.translation * scale)
        return Pose(translation: t, rotation: r, scale: s)
    }

    /// Map a point from this frame's local space into the parent's space.
    func apply(_ p: SIMD3<Double>) -> SIMD3<Double> {
        rotation.act(p * scale) + translation
    }

    /// Map a point from the parent's space back into this frame's local space.
    func inverseApply(_ p: SIMD3<Double>) -> SIMD3<Double> {
        guard scale != 0 else { return .zero }
        return rotation.inverse.act(p - translation) / scale
    }
}

// MARK: - Extent

/// Axis-aligned half-extents in the Space's OWN frame, micrometres. Gives a Space a
/// volume, which is what makes "am I inside this bar?" a question with an answer.
struct Extent: Codable, Hashable {
    var hxUm: Int64, hyUm: Int64, hzUm: Int64

    init(halfExtents h: SIMD3<Double>) {
        hxUm = Int64((abs(h.x) * 1_000_000).rounded())
        hyUm = Int64((abs(h.y) * 1_000_000).rounded())
        hzUm = Int64((abs(h.z) * 1_000_000).rounded())
    }
    /// Build from a measured bounding box (min/max in local metres).
    init(min lo: SIMD3<Double>, max hi: SIMD3<Double>) {
        self.init(halfExtents: (hi - lo) / 2)
    }

    var halfExtents: SIMD3<Double> {
        SIMD3(Double(hxUm), Double(hyUm), Double(hzUm)) / 1_000_000
    }
    var size: SIMD3<Double> { halfExtents * 2 }

    /// Containment test in LOCAL coordinates — the caller brings the point into this
    /// frame first (via `Pose.inverseApply` up the chain).
    func contains(local p: SIMD3<Double>) -> Bool {
        let h = halfExtents
        return abs(p.x) <= h.x && abs(p.y) <= h.y && abs(p.z) <= h.z
    }
}

// MARK: - Anchor

/// What a Space's pose is expressed *relative to*.
enum SpaceAnchor: Codable, Hashable {
    /// Pinned to the real world: an ENU tangent plane at this geodetic origin, on this
    /// discrete floor. Level 0 = ground, -1 = basement.
    case geodetic(lat: Double, lng: Double, level: Int16)
    /// Nested inside another Space's frame (a room in a venue; a skin on a room).
    case parent(spaceID: String)
    /// A MOVING frame — the tour bus. Carries only how to REACH the pose stream,
    /// never poses, exactly as `geo.rs`'s LocationSource.Stream carries only access.
    case stream(kind: String, reference: String)

    var level: Int16 {
        if case .geodetic(_, _, let l) = self { return l }
        return 0
    }
    var isLive: Bool {
        if case .stream = self { return true }
        return false
    }
}

// MARK: - Attached assets

/// The renderable geometry for a Space — what `ScanStore.assetRef` has been pointing
/// at all along without anything ever reading it.
struct GeometryRef: Codable, Hashable {
    var format: String      // pspc | usdz | glb
    var asset: String       // bundle resource name / relative path
    var pointCount: Int
    /// Provenance from the reconstructor: overlap | arkit | raw (matches ScanStore).
    var fusion: String
}

/// The relocaliser map for a Space.
///
/// Sized from the ACE paper (Brachmann/Cavallari/Prisacariu, CVPR 2023): the map for a
/// scene is a scene-specific MLP head on a frozen scene-agnostic backbone — about 4 MB,
/// trained in under 5 minutes from RGB + poses, with no depth and no 3D model. At that
/// size the map is not a service you call, it is a payload the Space CARRIES: it fits
/// in a GroupObject and syncs like any other. That is what makes a private venue
/// relocalisable without shipping the scan to anyone.
///
/// Nothing trains one yet — this is the slot, and `state` says so honestly.
struct RelocMapRef: Codable, Hashable {
    enum State: String, Codable { case absent, training, ready }
    var state: State = .absent
    var asset: String = ""
    var sizeBytes: Int = 0
    /// Frames contributed to the mapping buffer.
    var mappingFrames: Int = 0
}

// MARK: - The Space

struct Space: Identifiable, Codable, Hashable {
    enum Role: String, Codable { case venue, room, skin }

    var id: String
    var label: String
    var role: Role = .room
    var anchor: SpaceAnchor
    /// Pose within `anchor`'s frame.
    var pose: Pose = .identity
    var extent: Extent?
    var geometry: GeometryRef?
    var reloc: RelocMapRef = .init()
    /// False while the geometry's scale is still only up to a similarity transform —
    /// i.e. no metric anchor has been applied. Surfaced in the UI, never hidden.
    var scaleResolved: Bool = false
    /// Child Spaces (rooms of a venue, skins of a room).
    var children: [String] = []
    /// Which child skin is currently worn, if any.
    var activeSkin: String?

    var level: Int16 { anchor.level }
    var isLive: Bool { anchor.isLive }
}

// MARK: - Frame resolution

/// Resolves a Space's pose up its anchor chain into the world (ENU) frame — the
/// transform tree. A `.stream` anchor takes its world pose from `livePoses`, which is
/// fed out-of-band; absent a live fix the Space is honestly unplaced (nil) rather than
/// silently drawn at the origin.
struct SpaceGraph {
    private(set) var spaces: [String: Space] = [:]
    /// Live world poses for `.stream`-anchored Spaces, keyed by stream reference.
    var livePoses: [String: Pose] = [:]

    init(_ list: [Space] = []) { for s in list { spaces[s.id] = s } }

    mutating func insert(_ s: Space) { spaces[s.id] = s }
    func space(_ id: String) -> Space? { spaces[id] }

    /// The Space's pose in the world frame, or nil when the chain cannot be resolved
    /// (missing parent, or a live frame with no fix yet).
    func worldPose(of id: String, depth: Int = 0) -> Pose? {
        guard depth < 32, let s = spaces[id] else { return nil }   // depth guard: cycles
        switch s.anchor {
        case .geodetic:
            return s.pose
        case .parent(let parentID):
            guard let base = worldPose(of: parentID, depth: depth + 1) else { return nil }
            return base.compose(s.pose)
        case .stream(_, let reference):
            guard let live = livePoses[reference] else { return nil }
            return live.compose(s.pose)
        }
    }

    /// Every Space whose volume contains `worldPoint`, innermost (deepest) first.
    /// This is "which space am I in", and it is why Extent exists.
    func spacesContaining(_ worldPoint: SIMD3<Double>) -> [Space] {
        spaces.values.compactMap { s -> (Space, Double)? in
            guard let ext = s.extent, let wp = worldPose(of: s.id) else { return nil }
            let local = wp.inverseApply(worldPoint)
            guard ext.contains(local: local) else { return nil }
            return (s, ext.size.x * ext.size.y * ext.size.z)
        }
        .sorted { $0.1 < $1.1 }      // smallest volume = innermost
        .map(\.0)
    }
}

// MARK: - ENU

/// Geodetic → local East-North-Up metres. Carried over from the old `ENUProjector`,
/// which was correct for what it did; a Space needs it only to place its ANCHOR, since
/// everything inside the Space is already in the Space's own frame.
enum ENU {
    static func offset(from origin: CLLocationCoordinate2D, to c: CLLocationCoordinate2D) -> SIMD3<Double> {
        let mPerLat = 111_320.0
        let mPerLng = 111_320.0 * cos(origin.latitude * .pi / 180)
        return SIMD3((c.longitude - origin.longitude) * mPerLng, 0,
                     -(c.latitude - origin.latitude) * mPerLat)   // North → −Z
    }
}
