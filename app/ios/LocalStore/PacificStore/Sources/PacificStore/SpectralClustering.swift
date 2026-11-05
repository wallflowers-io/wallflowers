//  SpectralClustering.swift
//  The spectral half of the CORE TRANSMISSION (7 Aug directive): categorise the
//  embedding space by its own geometry, no model in the loop.
//
//  The textbook pipeline, chosen deliberately over anything cleverer:
//    cosine kNN graph  →  normalized Laplacian L_sym = I − D^-1/2 W D^-1/2
//    →  eigendecomposition  →  EIGENGAP picks k  →  row-normalized spectral
//    embedding  →  deterministic k-means++.
//  The eigengap is the point: k is READ OFF the space's own structure, never
//  guessed and never asked of a model. Zero eigenvalues count connected
//  components, the largest gap after them bounds the natural cluster count.
//
//  DETERMINISM IS THE CONTRACT (house rule, same as ChatChunker): identical
//  vectors in, identical clusters out, on any device, any launch. The k-means
//  seed derives from the input ids (FNV-1a over the sorted list through a
//  SplitMix64 stream) — never SystemRandomNumberGenerator, never Date.
//
//  The eigensolver is cyclic JACOBI, hand-rolled: ~60 lines, dependency-free,
//  bit-stable across platforms, O(N³) — fine for the lens's windowed N (≤ ~768
//  episodes; the Transmission caps its window). If the window ever grows past
//  that, swap this routine for Accelerate/LAPACK dsyevd behind the same
//  signature; nothing above it changes.

import Foundation

// MARK: - Deterministic randomness

/// SplitMix64 — tiny, seedable, stable. The ONLY randomness source in the lens.
struct SplitMix64: RandomNumberGenerator {
    private var state: UInt64
    init(seed: UInt64) { state = seed }
    mutating func next() -> UInt64 {
        state &+= 0x9E3779B97F4A7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58476D1CE4E5B9
        z = (z ^ (z >> 27)) &* 0x94D049BB133111EB
        return z ^ (z >> 31)
    }
}

/// FNV-1a over a string — the stable seed derivation (String.hashValue is
/// process-seeded and would break run-to-run determinism).
func fnv1a(_ s: String) -> UInt64 {
    var h: UInt64 = 0xcbf29ce484222325
    for b in s.utf8 { h = (h ^ UInt64(b)) &* 0x100000001b3 }
    return h
}

// MARK: - The clustering

public struct SpectralParams: Sendable {
    /// Neighbours per row in the similarity graph.
    public var kNN: Int = 10
    /// The k the eigengap may choose from (a lens should propose neither one
    /// mega-cluster nor confetti).
    public var kMin: Int = 2
    public var kMax: Int = 12
    /// k-means iterations cap; assignment stability usually converges in < 20.
    public var maxIterations: Int = 50
    public init() {}
}

public struct SpectralResult: Sendable, Equatable {
    /// Cluster index per input row (aligned with the input order).
    public let assignments: [Int]
    /// The k the eigengap chose.
    public let k: Int
    /// Ascending eigenvalues of L_sym — kept so callers can SEE the gap that
    /// justified k (a lens shows its working; it never just asserts).
    public let eigenvalues: [Double]
}

public enum SpectralClustering {

    /// Cluster `vectors` (any dimension, cosine geometry) into a structure the
    /// space itself justifies. `ids` seed the deterministic RNG and must be the
    /// rows' stable identities. N < kMin collapses to one cluster honestly.
    ///
    /// Pass `calibration` for REAL embedding vectors: trained spaces are
    /// anisotropic (a narrow cone), and clustering them raw hands the geometry
    /// to the corpus common direction. Centering first is the grounded move.
    public static func cluster(ids: [String], vectors: [[Float]],
                               params: SpectralParams = SpectralParams(),
                               calibration: SpaceCalibration? = nil) -> SpectralResult {
        precondition(ids.count == vectors.count, "ids and vectors must align")
        let n = vectors.count
        guard n >= max(2, params.kMin) else {
            return SpectralResult(assignments: Array(repeating: 0, count: n), k: n == 0 ? 0 : 1,
                                  eigenvalues: [])
        }

        // 1 · into the calibrated frame (or plain unit vectors when raw).
        let unit = calibration.map { cal in vectors.map(cal.center) } ?? vectors.map(normalize)

        // 2 · kNN graph with LOCAL SCALING (Zelnik-Manor & Perona): in a space
        // whose similarities compress into a narrow band, absolute weights lose
        // contrast — scale each edge by the two endpoints' own neighbourhood
        // radii (distance to the kth neighbour), so "close" means close FOR
        // THAT REGION and the eigengap stays readable. Symmetrized by max.
        let k = min(params.kNN, n - 1)
        var neighbours = [[(j: Int, d: Double)]](repeating: [], count: n)
        var sigma = [Double](repeating: 0, count: n)
        for i in 0..<n {
            var ds: [(j: Int, d: Double)] = []
            ds.reserveCapacity(n - 1)
            for j in 0..<n where j != i {
                ds.append((j, max(0, 1 - dot(unit[i], unit[j]))))   // cosine distance
            }
            ds.sort { $0.d < $1.d || ($0.d == $1.d && $0.j < $1.j) }   // deterministic ties
            neighbours[i] = Array(ds.prefix(k))
            sigma[i] = max(1e-6, neighbours[i].last?.d ?? 1)
        }
        var w = [Double](repeating: 0, count: n * n)
        for i in 0..<n {
            for (j, d) in neighbours[i] {
                let s = exp(-(d * d) / (sigma[i] * sigma[j]))
                w[i * n + j] = max(w[i * n + j], s)
                w[j * n + i] = max(w[j * n + i], s)
            }
        }

        // 3 · normalized Laplacian L_sym = I − D^-1/2 W D^-1/2.
        var degree = [Double](repeating: 0, count: n)
        for i in 0..<n {
            for j in 0..<n { degree[i] += w[i * n + j] }
            if degree[i] <= 0 { degree[i] = 1 }   // isolated row: keep it well-posed
        }
        let dInvSqrt = degree.map { 1.0 / $0.squareRoot() }
        var lap = [Double](repeating: 0, count: n * n)
        for i in 0..<n {
            for j in 0..<n {
                let off = -dInvSqrt[i] * w[i * n + j] * dInvSqrt[j]
                lap[i * n + j] = (i == j ? 1.0 : 0.0) + off
            }
        }

        // 4 · eigendecomposition (Jacobi) → ascending eigenvalues + vectors.
        let (values, vectors: evecs) = jacobiEigen(lap, n: n)

        // 5 · eigengap chooses k within [kMin, kMax].
        let kMax = min(params.kMax, n - 1)
        let kMin = min(params.kMin, kMax)
        var chosen = kMin
        var bestGap = -1.0
        for i in kMin...kMax {
            let gap = values[i] - values[i - 1]
            if gap > bestGap { bestGap = gap; chosen = i }
        }

        // 6 · spectral embedding: first `chosen` eigenvectors as rows, row-normalized.
        var embed = [[Float]](repeating: [Float](repeating: 0, count: chosen), count: n)
        for r in 0..<n {
            for c in 0..<chosen { embed[r][c] = Float(evecs[r * n + c]) }
            embed[r] = normalize(embed[r])
        }

        // 7 · deterministic k-means++ in spectral space.
        let seed = fnv1a(ids.sorted().joined(separator: "\u{1F}"))
        let assignments = kMeans(embed, k: chosen, seed: seed, maxIterations: params.maxIterations)

        return SpectralResult(assignments: assignments, k: chosen,
                              eigenvalues: Array(values.prefix(kMax + 1)))
    }

    // MARK: - k-means++

    static func kMeans(_ points: [[Float]], k: Int, seed: UInt64, maxIterations: Int) -> [Int] {
        let n = points.count
        guard k > 1, n > k else { return Array(repeating: 0, count: n) }
        var rng = SplitMix64(seed: seed)

        // k-means++ seeding: spread the initial centroids by squared distance.
        var centroids: [[Float]] = [points[Int(rng.next() % UInt64(n))]]
        while centroids.count < k {
            let d2 = points.map { p in centroids.map { sqDist(p, $0) }.min() ?? 0 }
            let total = d2.reduce(0, +)
            if total <= 0 { centroids.append(points[Int(rng.next() % UInt64(n))]); continue }
            var pick = Float.random(in: 0..<1, using: &rng) * total
            var idx = 0
            for (i, d) in d2.enumerated() { pick -= d; if pick <= 0 { idx = i; break }; idx = i }
            centroids.append(points[idx])
        }

        var assign = [Int](repeating: 0, count: n)
        for _ in 0..<maxIterations {
            var moved = false
            for (i, p) in points.enumerated() {
                var best = 0
                var bestD = Float.greatestFiniteMagnitude
                for (c, centroid) in centroids.enumerated() {
                    let d = sqDist(p, centroid)
                    if d < bestD { bestD = d; best = c }
                }
                if assign[i] != best { assign[i] = best; moved = true }
            }
            if !moved { break }
            let dim = points[0].count
            var sums = [[Float]](repeating: [Float](repeating: 0, count: dim), count: k)
            var counts = [Int](repeating: 0, count: k)
            for (i, p) in points.enumerated() {
                counts[assign[i]] += 1
                for d in 0..<dim { sums[assign[i]][d] += p[d] }
            }
            for c in 0..<k where counts[c] > 0 {
                centroids[c] = sums[c].map { $0 / Float(counts[c]) }
            }
        }
        return assign
    }

    // MARK: - Jacobi eigensolver (symmetric, cyclic sweeps)

    /// Ascending eigenvalues + column eigenvectors of a symmetric n×n matrix.
    /// Plain cyclic Jacobi: bit-stable, dependency-free. Swap for LAPACK dsyevd
    /// behind this signature if the window outgrows it.
    static func jacobiEigen(_ matrix: [Double], n: Int) -> (values: [Double], vectors: [Double]) {
        var a = matrix
        var v = [Double](repeating: 0, count: n * n)
        for i in 0..<n { v[i * n + i] = 1 }

        for _ in 0..<64 {   // sweeps; converges long before this for our sizes
            var off = 0.0
            for p in 0..<n { for q in (p + 1)..<n { off += a[p * n + q] * a[p * n + q] } }
            if off.squareRoot() < 1e-10 { break }
            for p in 0..<n {
                for q in (p + 1)..<n {
                    let apq = a[p * n + q]
                    guard abs(apq) > 1e-14 else { continue }
                    let app = a[p * n + p], aqq = a[q * n + q]
                    let theta = (aqq - app) / (2 * apq)
                    let t = (theta >= 0 ? 1.0 : -1.0) / (abs(theta) + (theta * theta + 1).squareRoot())
                    let c = 1 / (t * t + 1).squareRoot()
                    let s = t * c
                    for i in 0..<n {
                        let aip = a[i * n + p], aiq = a[i * n + q]
                        a[i * n + p] = c * aip - s * aiq
                        a[i * n + q] = s * aip + c * aiq
                    }
                    for i in 0..<n {
                        let api = a[p * n + i], aqi = a[q * n + i]
                        a[p * n + i] = c * api - s * aqi
                        a[q * n + i] = s * api + c * aqi
                    }
                    for i in 0..<n {
                        let vip = v[i * n + p], viq = v[i * n + q]
                        v[i * n + p] = c * vip - s * viq
                        v[i * n + q] = s * vip + c * viq
                    }
                }
            }
        }

        // Sort ascending, carrying columns.
        let order = (0..<n).sorted { a[$0 * n + $0] < a[$1 * n + $1] }
        var values = [Double](repeating: 0, count: n)
        var sorted = [Double](repeating: 0, count: n * n)
        for (newCol, oldCol) in order.enumerated() {
            values[newCol] = a[oldCol * n + oldCol]
            for r in 0..<n { sorted[r * n + newCol] = v[r * n + oldCol] }
        }
        return (values, sorted)
    }

    // MARK: - small vector helpers

    static func normalize(_ v: [Float]) -> [Float] {
        var mag: Float = 0
        for x in v { mag += x * x }
        guard mag > 0 else { return v }
        let inv = 1 / mag.squareRoot()
        return v.map { $0 * inv }
    }

    static func dot(_ a: [Float], _ b: [Float]) -> Double {
        var s: Float = 0
        for i in 0..<min(a.count, b.count) { s += a[i] * b[i] }
        return Double(s)
    }

    static func sqDist(_ a: [Float], _ b: [Float]) -> Float {
        var s: Float = 0
        for i in 0..<min(a.count, b.count) { let d = a[i] - b[i]; s += d * d }
        return s
    }
}
