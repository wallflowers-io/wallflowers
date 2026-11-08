//  HydratedKernel.swift
//  THE HYDRATED COARSENING KERNEL (8 Aug directive): "the critical step is the
//  coarsening kernel, representing contextual scenarios for the data. this can
//  be hydrated via available structural metadata and available GroupObject
//  data."
//
//  The reframe: a coarsening level is not a bandwidth, it is a SCENARIO — a
//  conjunction of conditions under which two things count as the same. "Same
//  words within the same conversation between the same people" is a different
//  claim than "same words anywhere", and the ladder of scenarios IS the
//  sequentially coarsening kernel: each level relaxes the context.
//
//  HYDRATION: the ladder is built from what the data actually carries —
//  structural metadata (the union store's §3 keys: authors, spans, tags,
//  sources; a dataset's fields: manufacturer, price, venue) and GroupObject
//  data (shared rosters, conversations, venues — the folds). A channel whose
//  key the corpus barely has never enters the ladder: a context you cannot
//  verify is not a context, so ABSENT DATA FAILS a channel rather than
//  passing it (precision-honest; hydration's coverage gate keeps this from
//  starving the ladder).
//
//  This is the same Grain/Pyramid/persistence machinery — scenarios simply
//  replace the geometric kernel as the fusion test, and grains carry their
//  hydration (meta unions, group unions) up the tree so coarse scenarios read
//  coarsened context, exactly as the semantic kernel reads drifted centroids.

import Foundation

// MARK: - Hydrated matter

public struct HydratedRow: Sendable {
    public let id: String
    public let vector: [Float]?                 // semantic channel, optional
    public let meta: [String: Set<String>]      // scalar keys are singleton sets
    public let groups: [String]                 // GroupObject memberships

    public init(id: String, vector: [Float]? = nil,
                meta: [String: Set<String>] = [:], groups: [String] = []) {
        self.id = id
        self.vector = vector
        self.meta = meta
        self.groups = groups
    }
}

// MARK: - Scenarios

/// One contextual condition. Absent data FAILS the channel.
public enum KernelChannel: Sendable, Equatable {
    /// Calibrated cosine distance ≤ maxDistance.
    case semantic(maxDistance: Float)
    /// The key's value sets intersect.
    case agree(key: String)
    /// Set-valued key overlaps by at least this Jaccard.
    case overlap(key: String, atLeast: Float)
    /// Numeric key within a relative tolerance: |a−b| ≤ fraction·max(|a|,|b|).
    case within(key: String, fraction: Double)
    /// The rows share at least one GroupObject.
    case sharedGroup
}

/// A scenario: the conjunction that defines "the same, in this context".
public struct Scenario: Sendable {
    public let name: String
    public let all: [KernelChannel]
    public init(name: String, all: [KernelChannel]) {
        self.name = name
        self.all = all
    }
}

// MARK: - Hydration

public enum KernelHydration {

    /// Build a ladder from what the corpus actually offers. Channels enter only
    /// with ≥ `coverage` presence across rows; numeric-parsing keys become
    /// `within`, multi-valued keys become `overlap`, the rest `agree`; a groups
    /// channel enters when memberships exist; semantic channels enter when
    /// vectors do. The ladder then relaxes: the strictest scenario first, one
    /// context channel dropped per level (deterministic reverse-alphabetical),
    /// widening semantics, ending at pure geometry.
    public static func ladder(rows: [HydratedRow],
                              sigmas: [Float] = [0.05, 0.1, 0.2, 0.4],
                              priceTolerance: Double = 0.05,
                              minJaccard: Float = 0.3,
                              coverage: Float = 0.8) -> [Scenario] {
        let n = Float(rows.count)
        guard n > 0 else { return [] }

        var keyCount: [String: Int] = [:]
        var keyMulti: Set<String> = []
        var keyNumeric: Set<String> = []
        for row in rows {
            for (k, v) in row.meta where !v.isEmpty {
                keyCount[k, default: 0] += 1
                if v.count > 1 { keyMulti.insert(k) }
                if v.allSatisfy({ Double($0.filter { !" $€£,".contains($0) }) != nil }) {
                    keyNumeric.insert(k)
                }
            }
        }
        let covered = keyCount.filter { Float($0.value) / n >= coverage }.keys.sorted()
        var context: [KernelChannel] = covered.map { key in
            if keyNumeric.contains(key) { return .within(key: key, fraction: priceTolerance) }
            if keyMulti.contains(key) { return .overlap(key: key, atLeast: minJaccard) }
            return .agree(key: key)
        }
        let hasGroups = rows.contains { !$0.groups.isEmpty }
        if hasGroups { context.append(.sharedGroup) }
        let hasVectors = rows.contains { $0.vector != nil }

        var out: [Scenario] = []
        var sigmaIdx = 0
        func sigma() -> Float {
            let s = sigmas[min(sigmaIdx, sigmas.count - 1)]
            sigmaIdx += 1
            return s
        }
        // Strictest first: full context + tight geometry.
        var remaining = context
        repeat {
            var all = remaining
            if hasVectors { all.insert(.semantic(maxDistance: sigma()), at: 0) }
            out.append(Scenario(name: "ctx\(remaining.count)", all: all))
            if remaining.isEmpty { break }
            remaining.removeLast()   // covered keys are sorted; drop reverse-alphabetical
        } while true
        // Tail: widest pure geometry, if any sigmas remain unused.
        while hasVectors && sigmaIdx < sigmas.count {
            out.append(Scenario(name: "geo\(sigmaIdx)", all: [.semantic(maxDistance: sigma())]))
        }
        return out
    }
}

// MARK: - Scenario coarsening (the pyramid's third build path)

extension CoarseningPyramid {

    struct HydratedGrainState {
        var meta: [String: Set<String>]
        var groups: Set<String>
    }

    /// Recursively convolve hydrated matter through the scenario ladder. Same
    /// Grain/Pyramid outputs; `PyramidLevel.sigma` carries the level index.
    public static func build(hydrated rows: [HydratedRow], ladder: [Scenario],
                             calibration: SpaceCalibration? = nil) -> Pyramid {
        guard !rows.isEmpty, !ladder.isEmpty else { return Pyramid(levels: [], persistence: [:]) }
        let sorted = rows.sorted { $0.id < $1.id }

        var matter: [Grain] = sorted.map { r in
            let framed = r.vector.map { v in
                calibration.map { $0.center(v) } ?? SpectralClustering.normalize(v)
            }
            return Grain(id: r.id, centroid: framed ?? [], mass: 1, members: [r.id], born: 0)
        }
        var state: [String: HydratedGrainState] = Dictionary(uniqueKeysWithValues: sorted.map {
            ($0.id, HydratedGrainState(meta: $0.meta, groups: Set($0.groups)))
        })

        func passes(_ a: Grain, _ b: Grain, _ scenario: Scenario) -> Bool {
            let sa = state[a.id]!, sb = state[b.id]!
            for channel in scenario.all {
                switch channel {
                case .semantic(let maxD):
                    guard !a.centroid.isEmpty, !b.centroid.isEmpty,
                          Float(1 - SpectralClustering.dot(a.centroid, b.centroid)) <= maxD
                    else { return false }
                case .agree(let key):
                    guard let va = sa.meta[key], let vb = sb.meta[key],
                          !va.intersection(vb).isEmpty else { return false }
                case .overlap(let key, let minJ):
                    guard let va = sa.meta[key], let vb = sb.meta[key], !va.isEmpty, !vb.isEmpty,
                          Float(va.intersection(vb).count) / Float(va.union(vb).count) >= minJ
                    else { return false }
                case .within(let key, let fraction):
                    func num(_ s: Set<String>?) -> Double? {
                        s?.compactMap { Double($0.filter { !" $€£,".contains($0) }) }.min()
                    }
                    guard let x = num(sa.meta[key]), let y = num(sb.meta[key]),
                          abs(x - y) <= fraction * max(abs(x), abs(y), .ulpOfOne)
                    else { return false }
                case .sharedGroup:
                    guard !sa.groups.intersection(sb.groups).isEmpty else { return false }
                }
            }
            return true
        }

        var levels: [PyramidLevel] = []
        var died: [String: Int] = [:]

        for (level, scenario) in ladder.enumerated() {
            var parent = Array(0..<matter.count)
            func find(_ x: Int) -> Int {
                var r = x
                while parent[r] != r { r = parent[r] }
                parent[x] = r
                return r
            }
            for i in 0..<matter.count {
                for j in (i + 1)..<matter.count where passes(matter[i], matter[j], scenario) {
                    let ri = find(i), rj = find(j)
                    if ri != rj { parent[max(ri, rj)] = min(ri, rj) }
                }
            }
            var groupsOf: [Int: [Grain]] = [:]
            for (i, g) in matter.enumerated() { groupsOf[find(i), default: []].append(g) }

            var next: [Grain] = []
            for (_, group) in groupsOf.sorted(by: { $0.key < $1.key }) {
                if group.count == 1 { next.append(group[0]); continue }
                let survivor = group.map(\.id).min()!
                var merged = state[survivor]!
                var centre = [Float]()
                var mass: Float = 0
                var withVec: Float = 0
                for g in group {
                    if g.id != survivor {
                        died[g.id] = level
                        let s = state[g.id]!
                        for (k, v) in s.meta { merged.meta[k, default: []].formUnion(v) }
                        merged.groups.formUnion(s.groups)
                    }
                    mass += g.mass
                    if !g.centroid.isEmpty {
                        if centre.isEmpty { centre = [Float](repeating: 0, count: g.centroid.count) }
                        for d in 0..<g.centroid.count { centre[d] += g.mass * g.centroid[d] }
                        withVec += g.mass
                    }
                }
                state[survivor] = merged
                let centroid = withVec > 0
                    ? SpectralClustering.normalize(centre.map { $0 / withVec }) : []
                next.append(Grain(id: survivor, centroid: centroid, mass: mass,
                                  members: group.flatMap(\.members).sorted(), born: level))
            }
            matter = next.sorted { $0.id < $1.id }
            levels.append(PyramidLevel(sigma: Float(level), grains: matter))
            if matter.count == 1 { break }
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
