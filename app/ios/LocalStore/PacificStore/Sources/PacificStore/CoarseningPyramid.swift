//  CoarseningPyramid.swift
//  THE COARSENING PYRAMID (8 Aug directive): "we can classify and deduplicate by
//  recursively convolving an embedded matter through sequentially coarsening
//  kernels."
//
//  The recursion, literally:
//    matter⁰ = the embedded rows (calibrated frame, unit mass each)
//    at level ℓ: fuse every pair the kernel of bandwidth σ_ℓ joins
//                (components under cosine-distance ≤ σ_ℓ — the kernel's
//                support radius); each fused cluster becomes ONE grain of
//                matter: mass-weighted centroid, summed mass, unioned members
//    σ_{ℓ+1} = γ·σ_ℓ  — the kernel coarsens; recurse on the COARSENED matter
//                (not the originals: mass centres drift as matter fuses,
//                which is the convolution doing its work)
//    stop     when one grain remains or maxLevels is spent
//
//  ONE operation, both answers:
//    DEDUPLICATE  what fuses at the finest scales is near-identity — those
//                 multi-member grains are the resolution calculus's candidate
//                 sets (they feed GraphResolution's blocking/proposals, they
//                 never merge anything themselves).
//    CLASSIFY     what fuses at coarser scales is family structure — read a
//                 grain's category orthogonally against the ontology anchors,
//                 per level. Category stays orthogonal to instance at every
//                 scale.
//
//  PERSISTENCE is the reality test: a grain's lifespan in levels (born when
//  formed, dead when absorbed). Structure that persists across many kernel
//  widths is a standing wave — the vortex, made computable. Noise fuses late
//  and dies immediately. This is 0-dimensional persistence over a scale-space,
//  which is what "sequentially coarsening kernels" resolves to when the
//  operation must be deterministic and on-device.
//
//  Grounding (8 Aug ruling): the kernel operates on CALIBRATED vectors — in
//  the raw anisotropic cone every pair sits within one kernel width of the
//  common direction and the pyramid collapses at level 0. Determinism: no
//  randomness anywhere; sorted iteration and id tie-breaks only.
//
//  TWO KERNELS, ONE RECURSION (Ralph, mid-build: "this is computable from
//  existing graph structure"): the semantic kernel above needs vectors; the
//  STRUCTURAL kernel needs only the graph the projector already writes.
//  Affinity = cosine between two grains' typed-incidence profiles — sharing
//  neighbours under the same verbs IS the kernel — and coarsening CONTRACTS
//  the graph (fused grains merge, their edges rewire and sum), so the next
//  level's kernel is computed on the coarsened structure. Same Grain, same
//  persistence, same readings; and it covers the rows embeddings never reach
//  (rosters, memberships, folds). `buildFromGraph` below.

import Foundation

public struct PyramidParams: Sendable {
    /// Finest kernel bandwidth, in cosine distance on the calibrated frame.
    /// 0.05 ≈ "the same thing said twice".
    public var sigma0: Float = 0.05
    /// Kernel coarsening factor per level (dyadic).
    public var gamma: Float = 2
    public var maxLevels: Int = 8
    public init() {}
}

public struct StructuralParams: Sendable {
    /// Finest structural kernel: incidence-profile cosine ≥ tau0 fuses.
    /// 0.8 ≈ "connected to the same things by the same verbs" — the
    /// graph-vouched duplicate, no vectors required.
    public var tau0: Float = 0.8
    /// Kernel coarsening: the threshold RELAXES by this factor per level.
    public var relax: Float = 0.6
    public var maxLevels: Int = 8
    public init() {}
}

/// One grain of matter at some level: a cluster with mass, centre and history.
public struct Grain: Sendable {
    public let id: String            // the lexicographically-least member id
    public let centroid: [Float]     // unit, mass-weighted
    public let mass: Float           // episodes absorbed
    public let members: [String]     // sorted
    public let born: Int             // level at which this grain formed
}

public struct PyramidLevel: Sendable {
    public let sigma: Float
    public let grains: [Grain]
}

public struct Pyramid: Sendable {
    public let levels: [PyramidLevel]
    /// grain id → number of levels it survived unabsorbed (its persistence).
    public let persistence: [String: Int]

    /// The dedup reading: multi-member grains at the finest level — candidate
    /// identity sets for the resolution calculus, never merges.
    public var duplicateSets: [[String]] {
        levels.first?.grains.filter { $0.members.count > 1 }.map(\.members) ?? []
    }

    /// The classification reading at one level: each grain's calibrated
    /// alignment to each anchor — the category axis, orthogonal to the grains.
    public func classes(atLevel l: Int, anchors: [KindAnchor],
                        calibration: SpaceCalibration?) -> [(grain: Grain, kind: [String: Float])] {
        guard l < levels.count else { return [] }
        return levels[l].grains.map { g in
            var kind: [String: Float] = [:]
            for a in anchors {
                let anchor = calibration.map { $0.center(a.vector) }
                    ?? SpectralClustering.normalize(a.vector)
                let c = Float(SpectralClustering.dot(g.centroid, anchor))
                kind[a.kind.name] = calibration.map { $0.calibrated01(c) } ?? c
            }
            return (g, kind)
        }
    }
}

public enum CoarseningPyramid {

    /// Recursively convolve the matter. Vectors are moved into the calibrated
    /// frame first when a calibration is given — on real embeddings it must be.
    public static func build(ids: [String], vectors: [[Float]],
                             params: PyramidParams = PyramidParams(),
                             calibration: SpaceCalibration? = nil) -> Pyramid {
        precondition(ids.count == vectors.count, "ids and vectors must align")
        guard !ids.isEmpty else { return Pyramid(levels: [], persistence: [:]) }

        let framed = calibration.map { cal in vectors.map(cal.center) }
            ?? vectors.map(SpectralClustering.normalize)

        // matter⁰ — one grain per row, in id order for determinism.
        var matter: [Grain] = zip(ids, framed)
            .sorted { $0.0 < $1.0 }
            .map { Grain(id: $0.0, centroid: $0.1, mass: 1, members: [$0.0], born: 0) }

        var levels: [PyramidLevel] = []
        var died: [String: Int] = [:]     // grain id → level it was absorbed at
        var sigma = params.sigma0

        for level in 0..<params.maxLevels {
            // Fuse: connected components under kernel support d ≤ σ.
            var parent = Array(0..<matter.count)
            func find(_ x: Int) -> Int {
                var r = x
                while parent[r] != r { r = parent[r] }
                parent[x] = r
                return r
            }
            for i in 0..<matter.count {
                for j in (i + 1)..<matter.count {
                    let d = Float(1 - SpectralClustering.dot(matter[i].centroid, matter[j].centroid))
                    if d <= sigma {
                        let ri = find(i), rj = find(j)
                        if ri != rj { parent[max(ri, rj)] = min(ri, rj) }
                    }
                }
            }

            var groups: [Int: [Grain]] = [:]
            for (i, g) in matter.enumerated() { groups[find(i), default: []].append(g) }

            var next: [Grain] = []
            for (_, group) in groups.sorted(by: { $0.key < $1.key }) {
                if group.count == 1 {
                    next.append(group[0])
                    continue
                }
                // The convolution step: fused matter becomes one grain at its
                // mass-weighted centre. Absorbed grains record their death.
                let dim = group[0].centroid.count
                var centre = [Float](repeating: 0, count: dim)
                var mass: Float = 0
                for g in group {
                    for d in 0..<dim { centre[d] += g.mass * g.centroid[d] }
                    mass += g.mass
                }
                let members = group.flatMap(\.members).sorted()
                let survivor = group.map(\.id).min()!
                for g in group where g.id != survivor { died[g.id] = level }
                next.append(Grain(id: survivor,
                                  centroid: SpectralClustering.normalize(centre.map { $0 / mass }),
                                  mass: mass,
                                  members: members,
                                  born: group.contains { $0.members.count > 1 } || group.count > 1
                                      ? level : group[0].born))
            }

            matter = next.sorted { $0.id < $1.id }
            levels.append(PyramidLevel(sigma: sigma, grains: matter))
            if matter.count == 1 { break }
            sigma *= params.gamma
        }

        // Persistence: survivors live to the last level; the absorbed lived
        // from birth to death.
        var births: [String: Int] = [:]
        for level in levels.indices {
            for g in levels[level].grains where births[g.id] == nil { births[g.id] = g.born }
        }
        var persistence: [String: Int] = [:]
        for (id, b) in births {
            persistence[id] = (died[id] ?? levels.count) - b
        }

        return Pyramid(levels: levels, persistence: persistence)
    }

    // MARK: - The structural kernel (computable from existing graph structure)

    /// Recursively convolve the GRAPH: affinity is the cosine between two
    /// grains' typed-incidence profiles ("verb>peer" / "verb<peer", weights
    /// summed), fusion contracts the graph (edges rewire to survivors and
    /// merge), and the relaxing threshold is the coarsening kernel. No vectors
    /// anywhere — this runs on rows embeddings never reach. Grains carry no
    /// centroid (empty), so `classes(atLevel:)` is a semantic-mode reading;
    /// structural grains classify by their members' node kinds instead.
    public static func buildFromGraph(nodeIds: [String], edges: [GraphEdgeRow],
                                      params: StructuralParams = StructuralParams()) -> Pyramid {
        let baseIds = Set(nodeIds).union(edges.flatMap { [$0.src, $0.dst] }).sorted()
        guard !baseIds.isEmpty else { return Pyramid(levels: [], persistence: [:]) }

        // The working multigraph: contracted endpoints, summed weights.
        struct Link { var src: String; var verb: String; var dst: String; var weight: Float }
        var links = edges.map { Link(src: $0.src, verb: $0.verb, dst: $0.dst, weight: 1) }
        var matter: [Grain] = baseIds.map {
            Grain(id: $0, centroid: [], mass: 1, members: [$0], born: 0)
        }

        var levels: [PyramidLevel] = []
        var died: [String: Int] = [:]
        var tau = params.tau0

        for level in 0..<params.maxLevels {
            // Incidence profiles on the CURRENT (contracted) structure.
            var profile: [String: [String: Float]] = [:]
            for l in links where l.src != l.dst {
                profile[l.src, default: [:]]["\(l.verb)>\(l.dst)", default: 0] += l.weight
                profile[l.dst, default: [:]]["\(l.verb)<\(l.src)", default: 0] += l.weight
            }
            func cosine(_ a: [String: Float], _ b: [String: Float]) -> Float {
                var dot: Float = 0
                for (k, va) in a { if let vb = b[k] { dot += va * vb } }
                let na = a.values.map { $0 * $0 }.reduce(0, +).squareRoot()
                let nb = b.values.map { $0 * $0 }.reduce(0, +).squareRoot()
                guard na > 0, nb > 0 else { return 0 }
                return dot / (na * nb)
            }

            var parent = Array(0..<matter.count)
            func find(_ x: Int) -> Int {
                var r = x
                while parent[r] != r { r = parent[r] }
                parent[x] = r
                return r
            }
            for i in 0..<matter.count {
                guard let pi = profile[matter[i].id] else { continue }
                for j in (i + 1)..<matter.count {
                    guard let pj = profile[matter[j].id] else { continue }
                    if cosine(pi, pj) >= tau {
                        let ri = find(i), rj = find(j)
                        if ri != rj { parent[max(ri, rj)] = min(ri, rj) }
                    }
                }
            }

            var groups: [Int: [Grain]] = [:]
            for (i, g) in matter.enumerated() { groups[find(i), default: []].append(g) }

            var canon: [String: String] = [:]
            var next: [Grain] = []
            for (_, group) in groups.sorted(by: { $0.key < $1.key }) {
                let survivor = group.map(\.id).min()!
                for g in group { canon[g.id] = survivor }
                if group.count == 1 {
                    next.append(group[0])
                    continue
                }
                for g in group where g.id != survivor { died[g.id] = level }
                next.append(Grain(id: survivor, centroid: [],
                                  mass: group.map(\.mass).reduce(0, +),
                                  members: group.flatMap(\.members).sorted(),
                                  born: level))
            }

            // CONTRACT: rewire links through the fusion, merge parallels.
            var merged: [String: Link] = [:]
            for l in links {
                let s = canon[l.src] ?? l.src
                let d = canon[l.dst] ?? l.dst
                guard s != d else { continue }   // interior structure is absorbed mass
                let key = "\(s)|\(l.verb)|\(d)"
                if var existing = merged[key] {
                    existing.weight += l.weight
                    merged[key] = existing
                } else {
                    merged[key] = Link(src: s, verb: l.verb, dst: d, weight: l.weight)
                }
            }
            links = merged.sorted { $0.key < $1.key }.map(\.value)

            matter = next.sorted { $0.id < $1.id }
            levels.append(PyramidLevel(sigma: tau, grains: matter))
            if matter.count == 1 || links.isEmpty { break }
            tau *= params.relax
        }

        var births: [String: Int] = [:]
        for level in levels.indices {
            for g in levels[level].grains where births[g.id] == nil { births[g.id] = g.born }
        }
        var persistence: [String: Int] = [:]
        for (id, b) in births { persistence[id] = (died[id] ?? levels.count) - b }

        return Pyramid(levels: levels, persistence: persistence)
    }
}
