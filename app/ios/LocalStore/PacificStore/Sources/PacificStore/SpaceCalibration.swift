//  SpaceCalibration.swift
//  The actual mathematics of the embedding space, made explicit (8 Aug ruling:
//  "ground yourself in the actual mathematics of embedding models").
//
//  A trained sentence-embedding space is ANISOTROPIC: vectors live in a narrow
//  cone around a common direction, and the cosine between two UNRELATED texts is
//  not 0 — it is whatever the model's geometry makes it (typically 0.4–0.7).
//  Consequences, if ignored:
//    · a mean of unit vectors points mostly at the corpus common direction, so
//      every stream's "heading" collapses toward the same place;
//    · resultant length (coherence) saturates high for ANY set — buffeting and
//      a beam become indistinguishable;
//    · raw cosines to anchors compress into a narrow band whose absolute values
//      mean nothing; only their differences carry signal.
//
//  The corrections here are the standard ones:
//    CENTER      subtract the corpus mean of unit vectors (removes the cone's
//                axis), optionally deflate the top principal component(s) —
//                "all-but-the-top" (Mu & Viswanath 2018). Deflation is powerful
//                and dangerous in equal measure: on a small, structured corpus
//                the top PC can BE the signal, so `components` defaults to 0
//                (mean-only) and is a deliberate dial, not a reflex.
//    CALIBRATE   estimate the null: the mean and spread of centered cosines
//                between random pairs of THIS corpus. Every reported alignment
//                is then a z-score against unrelatedness, squashed to [0,1] by
//                a logistic — 0.5 literally means "as similar as two unrelated
//                texts here", which raw cosine never meant.
//
//  Everything is deterministic: the PC power iteration and the null-pair sample
//  are SplitMix64-seeded from the corpus shape, so the same corpus fits the same
//  calibration on any device.
//
//  Fit it from the union store's own vectors (a few hundred suffice); refit when
//  the corpus grows materially or the embedder changes. Quantization noise from
//  vector reconstruction is absorbed into the null spread — that is where it
//  belongs.

import Foundation

public struct SpaceCalibration: Sendable {
    /// Mean of the corpus's unit vectors — the cone's axis.
    public let mean: [Float]
    /// Deflated principal directions (possibly empty).
    public let removed: [[Float]]
    /// Null statistics: centered cosine between random unrelated pairs.
    public let nullMean: Float
    public let nullSpread: Float

    // MARK: - Fit

    /// Estimate the space from a corpus sample. Nil below 8 vectors — a null
    /// estimated from almost nothing would calibrate lies.
    public static func fit(_ vectors: [[Float]], components: Int = 0) -> SpaceCalibration? {
        let units = vectors.filter { !$0.isEmpty }.map(SpectralClustering.normalize)
        guard units.count >= 8, let dim = units.first?.count else { return nil }

        var mean = [Float](repeating: 0, count: dim)
        for u in units { for d in 0..<dim { mean[d] += u[d] } }
        for d in 0..<dim { mean[d] /= Float(units.count) }

        var centered = units.map { u in zip(u, mean).map { $0 - $1 } }
        var removed: [[Float]] = []
        for c in 0..<max(0, components) {
            guard let pc = principal(centered, dim: dim, seed: UInt64(c) &+ 1) else { break }
            removed.append(pc)
            for i in centered.indices {
                let proj = dotf(centered[i], pc)
                for d in 0..<dim { centered[i][d] -= proj * pc[d] }
            }
        }

        // The null: deterministic random pairs of centered unit vectors.
        let cu = centered.map(SpectralClustering.normalize)
        var rng = SplitMix64(seed: fnv1a("\(units.count)|\(dim)|\(removed.count)"))
        let sample = min(2000, units.count * (units.count - 1) / 2)
        var cosines: [Float] = []
        cosines.reserveCapacity(sample)
        for _ in 0..<sample {
            let i = Int(rng.next() % UInt64(cu.count))
            var j = Int(rng.next() % UInt64(cu.count))
            if j == i { j = (j + 1) % cu.count }
            cosines.append(Float(SpectralClustering.dot(cu[i], cu[j])))
        }
        let m = cosines.reduce(0, +) / Float(cosines.count)
        let variance = cosines.map { ($0 - m) * ($0 - m) }.reduce(0, +) / Float(cosines.count)

        return SpaceCalibration(mean: mean, removed: removed,
                                nullMean: m, nullSpread: max(1e-4, variance.squareRoot()))
    }

    // MARK: - Apply

    /// A vector moved into the calibrated frame: unit → mean-subtracted →
    /// deflated → re-normalized. A vector that centering annihilates (it WAS
    /// the common direction) falls back to its raw unit self rather than NaN.
    public func center(_ v: [Float]) -> [Float] {
        let u = SpectralClustering.normalize(v)
        var c = zip(u, mean).map { $0 - $1 }
        for pc in removed {
            let proj = dotf(c, pc)
            for d in 0..<c.count { c[d] -= proj * pc[d] }
        }
        let n = SpectralClustering.normalize(c)
        return n.contains(where: { $0 != 0 }) ? n : u
    }

    /// A centered cosine as a z-score against this corpus's unrelated-pair null.
    public func z(_ cosine: Float) -> Float { (cosine - nullMean) / nullSpread }

    /// The z squashed to [0,1]: 0.5 = as similar as unrelated texts get HERE.
    public func calibrated01(_ cosine: Float) -> Float { 1 / (1 + exp(-z(cosine))) }

    // MARK: - internals

    /// Top principal direction by deterministic power iteration.
    static func principal(_ rows: [[Float]], dim: Int, seed: UInt64) -> [Float]? {
        var rng = SplitMix64(seed: seed)
        var p = SpectralClustering.normalize((0..<dim).map { _ in
            Float(rng.next() % 1000) / 1000 - 0.5
        })
        for _ in 0..<60 {
            var next = [Float](repeating: 0, count: dim)
            for r in rows {
                let proj = dotf(r, p)
                for d in 0..<dim { next[d] += proj * r[d] }
            }
            let n = SpectralClustering.normalize(next)
            guard n.contains(where: { $0 != 0 }) else { return nil }
            p = n
        }
        return p
    }
}

func dotf(_ a: [Float], _ b: [Float]) -> Float {
    var s: Float = 0
    for i in 0..<min(a.count, b.count) { s += a[i] * b[i] }
    return s
}
