//  PointCloudAsset.swift
//  Loads a `.pspc` point cloud — the real output of arc/spatial (VGGT → fused voxel
//  cloud → GLB → this) — and builds a RealityKit mesh from it.
//
//  Two RealityKit facts shape this file:
//
//  1. RealityKit cannot load GLB. It reads USD and .reality. So the pipeline's GLB is
//     converted offline to a flat `.pspc` blob (header + float32 XYZ + uint8 RGB) and
//     the mesh is built at runtime. 22,309 points is 335 KB — small enough that runtime
//     construction is cheaper than shipping a baked mesh.
//
//  2. RealityKit's MeshDescriptor has NO vertex-colour channel. So per-point colour is
//     carried in a texture atlas: one texel per point, and ALL vertices of a given
//     splat are given the SAME uv. A constant uv across a triangle interpolates to a
//     constant value, so each splat samples exactly one texel — no bleeding, no
//     filtering artefacts, full colour. Mipmaps are disabled for the same reason.
//
//  A point has no area, so each point is drawn as a 3-quad axis cross: visible from any
//  direction, 12 vertices / 6 triangles each. `stride` subsamples if a device needs it.

import Foundation
import RealityKit
import CoreGraphics
import simd

struct PointCloud {
    var positions: [SIMD3<Float>]
    var colors: [SIMD3<UInt8>]
    var boundsMin: SIMD3<Float>
    var boundsMax: SIMD3<Float>

    var count: Int { positions.count }
    var size: SIMD3<Float> { boundsMax - boundsMin }

    enum LoadError: Error, LocalizedError {
        case missing(String), badMagic, truncated
        var errorDescription: String? {
            switch self {
            case .missing(let n): return "point cloud '\(n)' not found in bundle"
            case .badMagic:       return "not a PSPC point cloud"
            case .truncated:      return "point cloud data truncated"
            }
        }
    }

    /// Load a cloud from disk, by extension. `.glb` is what the reconstruction
    /// service streams back; `.pspc` is the bundled fixture format.
    static func load(contentsOf url: URL) throws -> PointCloud {
        let data = try Data(contentsOf: url)
        switch url.pathExtension.lowercased() {
        case "glb":  return try glb(data)
        case "pspc": return try pspc(data)
        default:     throw LoadError.badMagic
        }
    }

    /// Parse a glTF-binary point cloud — the arc/spatial wire format.
    ///
    /// GLB is: `"glTF"` | version u32 | length u32, then chunks of
    /// `len u32 | type u32 | bytes`, where type 0x4E4F534A is JSON and 0x004E4942 is
    /// binary. The mesh is POINTS mode with POSITION and (optionally) COLOR_0
    /// accessors, both VEC3 float32. Only what this pipeline emits is handled — a
    /// general glTF reader is a much larger thing and would be dead weight here.
    static func glb(_ data: Data) throws -> PointCloud {
        guard data.count >= 12 else { throw LoadError.truncated }
        func u32(_ o: Int) -> UInt32 {
            data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: o, as: UInt32.self) }
        }
        guard u32(0) == 0x46546C67 else { throw LoadError.badMagic }   // "glTF"
        let total = Int(u32(8))

        var json: [String: Any]?
        var bin: Data?
        var off = 12
        while off + 8 <= min(total, data.count) {
            let len = Int(u32(off)), kind = u32(off + 4)
            let body = data.subdata(in: (off + 8)..<min(off + 8 + len, data.count))
            if kind == 0x4E4F534A { json = try? JSONSerialization.jsonObject(with: body) as? [String: Any] }
            if kind == 0x004E4942 { bin = body }
            off += 8 + len
        }
        guard let g = json, let blob = bin,
              let accessors = g["accessors"] as? [[String: Any]],
              let views = g["bufferViews"] as? [[String: Any]],
              let meshes = g["meshes"] as? [[String: Any]],
              let prims = meshes.first?["primitives"] as? [[String: Any]],
              let attrs = prims.first?["attributes"] as? [String: Any],
              let posIdx = attrs["POSITION"] as? Int
        else { throw LoadError.truncated }

        func vec3(_ index: Int) -> [SIMD3<Float>] {
            let a = accessors[index]
            guard let vi = a["bufferView"] as? Int, let n = a["count"] as? Int else { return [] }
            let base = (views[vi]["byteOffset"] as? Int ?? 0) + (a["byteOffset"] as? Int ?? 0)
            guard base + n * 12 <= blob.count else { return [] }
            return blob.withUnsafeBytes { raw in
                (0..<n).map { i in
                    let o = base + i * 12
                    return SIMD3<Float>(raw.loadUnaligned(fromByteOffset: o, as: Float.self),
                                        raw.loadUnaligned(fromByteOffset: o + 4, as: Float.self),
                                        raw.loadUnaligned(fromByteOffset: o + 8, as: Float.self))
                }
            }
        }

        let positions = vec3(posIdx)
        guard !positions.isEmpty else { throw LoadError.truncated }
        // COLOR_0 is float32 [0,1] here; absent means an uncoloured cloud, drawn white.
        let colors: [SIMD3<UInt8>] = (attrs["COLOR_0"] as? Int).map { idx in
            vec3(idx).map { c in
                SIMD3<UInt8>(UInt8(max(0, min(255, c.x * 255))),
                             UInt8(max(0, min(255, c.y * 255))),
                             UInt8(max(0, min(255, c.z * 255))))
            }
        } ?? Array(repeating: SIMD3<UInt8>(235, 235, 235), count: positions.count)

        var lo = positions[0], hi = positions[0]
        for p in positions { lo = simd_min(lo, p); hi = simd_max(hi, p) }
        return PointCloud(positions: positions, colors: colors, boundsMin: lo, boundsMax: hi)
    }

    /// Parse a `.pspc` from the app bundle.
    ///
    /// Layout, little-endian: `"PSPC"` | version u32 | count u32 | bboxMin 3×f32 |
    /// bboxMax 3×f32 | count×(3×f32 position) | count×(3×u8 colour).
    static func load(resource name: String, bundle: Bundle = .main) throws -> PointCloud {
        guard let url = bundle.url(forResource: name, withExtension: "pspc") else {
            throw LoadError.missing(name)
        }
        return try pspc(try Data(contentsOf: url))
    }

    static func pspc(_ data: Data) throws -> PointCloud {
        guard data.count >= 12 else { throw LoadError.truncated }
        guard data[0] == 0x50, data[1] == 0x53, data[2] == 0x50, data[3] == 0x43 else {   // "PSPC"
            throw LoadError.badMagic
        }

        func u32(_ o: Int) -> UInt32 {
            data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: o, as: UInt32.self) }
        }
        func f32(_ o: Int) -> Float {
            data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: o, as: Float.self) }
        }

        let count = Int(u32(8))
        let header = 12 + 24
        let posBytes = count * 12
        let colBytes = count * 3
        guard data.count >= header + posBytes + colBytes else { throw LoadError.truncated }

        let lo = SIMD3<Float>(f32(12), f32(16), f32(20))
        let hi = SIMD3<Float>(f32(24), f32(28), f32(32))

        var positions = [SIMD3<Float>](); positions.reserveCapacity(count)
        var colors = [SIMD3<UInt8>](); colors.reserveCapacity(count)
        data.withUnsafeBytes { raw in
            for i in 0..<count {
                let o = header + i * 12
                positions.append(SIMD3(raw.loadUnaligned(fromByteOffset: o,     as: Float.self),
                                       raw.loadUnaligned(fromByteOffset: o + 4, as: Float.self),
                                       raw.loadUnaligned(fromByteOffset: o + 8, as: Float.self)))
            }
            let cBase = header + posBytes
            for i in 0..<count {
                let o = cBase + i * 3
                colors.append(SIMD3(raw.load(fromByteOffset: o,     as: UInt8.self),
                                    raw.load(fromByteOffset: o + 1, as: UInt8.self),
                                    raw.load(fromByteOffset: o + 2, as: UInt8.self)))
            }
        }
        return PointCloud(positions: positions, colors: colors, boundsMin: lo, boundsMax: hi)
    }

    /// Measured extent, for the Space's `Extent` — the volume comes from the scan
    /// itself rather than being declared by hand.
    var measuredExtent: Extent {
        Extent(min: SIMD3<Double>(boundsMin), max: SIMD3<Double>(boundsMax))
    }
}

// MARK: - Mesh construction

enum PointCloudMesh {
    /// Build a splat mesh + colour-atlas material.
    ///
    /// - Parameters:
    ///   - splat: half-size of each point's cross, in the cloud's own units.
    ///   - stride: take every Nth point (1 = all).
    static func build(_ cloud: PointCloud,
                      splat: Float = 0.006,
                      stride pointStride: Int = 1) throws -> ModelEntity {
        let idx = Swift.stride(from: 0, to: cloud.count, by: max(1, pointStride)).map { $0 }
        let n = idx.count

        // Colour atlas: one texel per drawn point, square, nearest-sampled by construction.
        let side = max(1, Int(ceil(sqrt(Double(n)))))
        var pixels = [UInt8](repeating: 0, count: side * side * 4)
        for (j, i) in idx.enumerated() {
            let c = cloud.colors[i]
            let p = j * 4
            pixels[p] = c.x; pixels[p + 1] = c.y; pixels[p + 2] = c.z; pixels[p + 3] = 255
        }

        var positions = [SIMD3<Float>](); positions.reserveCapacity(n * 12)
        var uvs = [SIMD2<Float>](); uvs.reserveCapacity(n * 12)
        var indices = [UInt32](); indices.reserveCapacity(n * 18)

        let s = splat
        // Three axis-aligned quads (XY, YZ, XZ) so a point reads from any direction.
        let quads: [(SIMD3<Float>, SIMD3<Float>)] = [
            (SIMD3(s, 0, 0), SIMD3(0, s, 0)),
            (SIMD3(0, s, 0), SIMD3(0, 0, s)),
            (SIMD3(s, 0, 0), SIMD3(0, 0, s)),
        ]

        for (j, i) in idx.enumerated() {
            let c = cloud.positions[i]
            // All 12 verts share this uv → constant interpolation → exactly one texel.
            let uv = SIMD2<Float>((Float(j % side) + 0.5) / Float(side),
                                  (Float(j / side) + 0.5) / Float(side))
            for (u, v) in quads {
                let base = UInt32(positions.count)
                positions.append(c - u - v)
                positions.append(c + u - v)
                positions.append(c + u + v)
                positions.append(c - u + v)
                uvs.append(contentsOf: [uv, uv, uv, uv])
                indices.append(contentsOf: [base, base + 1, base + 2,
                                            base, base + 2, base + 3])
            }
        }

        var desc = MeshDescriptor(name: "pointcloud")
        desc.positions = MeshBuffer(positions)
        desc.textureCoordinates = MeshBuffer(uvs)
        desc.primitives = .triangles(indices)

        let mesh = try MeshResource.generate(from: [desc])
        let material = try atlasMaterial(pixels: pixels, side: side)
        return ModelEntity(mesh: mesh, materials: [material])
    }

    /// Unlit so the scan reads as captured colour rather than being relit — and unlit
    /// costs nothing, which matters at ~130k triangles on a phone.
    private static func atlasMaterial(pixels: [UInt8], side: Int) throws -> RealityKit.Material {
        var mutable = pixels
        let cg = mutable.withUnsafeMutableBytes { buf -> CGImage? in
            guard let ctx = CGContext(data: buf.baseAddress,
                                      width: side, height: side,
                                      bitsPerComponent: 8, bytesPerRow: side * 4,
                                      space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { return nil }
            return ctx.makeImage()
        }
        guard let image = cg else { return UnlitMaterial(color: .white) }

        // Mipmaps OFF: a mipped atlas averages neighbouring points' colours together.
        let options = TextureResource.CreateOptions(semantic: .color, mipmapsMode: .none)
        let texture = try TextureResource(image: image, options: options)
        var material = UnlitMaterial()
        material.color = .init(tint: .white, texture: .init(texture))
        return material
    }
}
