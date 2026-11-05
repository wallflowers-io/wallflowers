//  Attitude.swift
//  THE VECTORISED ORIENTATION (8 Aug): the process by which the incident stream —
//  episodes, their charges, the fixed points the user has minted — is transformed
//  into a stable orientation. Attitude FIRST: without it the free stream impacts
//  quasi-randomly and everything downstream diverges; flux control (FEED ranking,
//  retrieval bias, the notification trim) comes after, and reads this.
//
//  THE PROCESS, five stages, all deterministic:
//    WINDOW     two spans over the stream, cut at the STREAM'S OWN CLOCK (the
//               newest episode's ts — never wall time; the vehicle's clock is its
//               stream, and the same episodes must yield the same orientation on
//               any device at any hour).
//    WEIGHT     exponential decay by age within each window (half-life params).
//    INTEGRATE  weighted mean of unit vectors → a heading per window. The slow
//               window is the gyro's long memory (disposition); the fast window
//               is the instantaneous reading (attention).
//    FILTER     complementary blend — slerp(slow, fast, α) — the classic attitude
//               filter: the stable low-frequency reference rejects quasi-random
//               impulses, the high-frequency reading tracks a real manoeuvre.
//    REFERENCE  lock the free-floating heading to fixed points: cosine against
//               GUIDE STARS (minted entities — the star tracker; the graph the
//               user built is the catalogue) and against the ontology's kind
//               anchors (the inertial frame's axes).
//
//  The output carries its own honesty: COHERENCE is the resultant length of the
//  fast window (1 = a beam, 0 = buffeting — "admitting they don't know" as a
//  number), EVIDENCE counts what was integrated, RATE is the turn between this
//  fast heading and the previous fast-span's, and TRIM is the composed charge
//  (red/from · blue/towards · gold/about) — the temporal lean of the vehicle.
//
//  Like every lens: COMPUTED, never stored. An Orientation is valid at its
//  stream-clock and recomputed from the fold whenever asked.

import Foundation

// MARK: - Inputs

/// A fixed point to lock against — a minted entity's id + its resolution vector.
/// The star tracker's catalogue: only what the user has established by hand.
public struct GuideStar: Sendable {
    public let id: String
    public let vector: [Float]
    public init(id: String, vector: [Float]) {
        self.id = id
        self.vector = vector
    }
}

public struct OrientationParams: Sendable {
    /// Slow window — disposition. Span and half-life in stream milliseconds.
    public var slowSpanMs: UInt64 = 45 * 24 * 3_600_000
    public var slowHalfLifeMs: UInt64 = 14 * 24 * 3_600_000
    /// Fast window — attention.
    public var fastSpanMs: UInt64 = 7 * 24 * 3_600_000
    public var fastHalfLifeMs: UInt64 = 36 * 3_600_000
    /// Complementary-filter weight of the FAST heading (α). 0 = pure disposition,
    /// 1 = pure attention.
    public var blend: Float = 0.35
    /// Which Chromodynamics instrument charges the trim (lc-v1 / lc-kr-v1) —
    /// pass the union language authority's value (F-2, one authority).
    public var language: NLLanguageIdentifier = .english
    /// How many guide-star locks the orientation reports.
    public var guideStars: Int = 5
    public init() {}
}

// MARK: - Output

public struct OrientationLock: Sendable, Equatable {
    public let id: String
    public let alignment: Float   // cosine of the heading to this star
}

/// The vectorised orientation — the attitude state at one stream-clock instant.
public struct Orientation: Sendable {
    /// Unit heading in the shared embedding space — where the vehicle points.
    public let heading: [Float]
    /// Turn between this fast heading and the previous fast-span's, in radians.
    /// 0 when the prior span holds no evidence — no claim without a reading.
    public let rateRadians: Float
    /// Composed charge over the fast window — the momentary temporal lean.
    public let trim: Charge
    /// Composed charge over the slow window — the standing disposition.
    public let slowTrim: Charge
    /// Resultant length of the fast window ∈ [0,1]: 1 = a beam, 0 = buffeting.
    /// The orientation's own confession of how much attitude it actually has.
    public let coherence: Float
    /// Guide-star locks, best first (deterministic tie-break by id).
    public let locks: [OrientationLock]
    /// Kind name → cosine of the heading to that kind's anchor.
    public let kindAlignment: [String: Float]
    /// Episodes integrated across the slow window.
    public let evidence: Int
    /// The stream clock this orientation is valid at (newest episode ts).
    public let clock: UInt64
}

// MARK: - The process

public enum Attitude {

    /// Determine the orientation from the stream. Nil for an empty stream —
    /// there is no attitude without input, and pretending otherwise is exactly
    /// the fabricated row the house rules ban.
    ///
    /// Pass `calibration` for REAL embedding vectors. Uncentered, an
    /// anisotropic space collapses every heading toward the corpus common
    /// direction, saturates coherence for any stream, and hides rate inside a
    /// ~0.95-cosine band; centered, those quantities mean what they claim.
    /// Locks and kind alignments are then reported CALIBRATED (0.5 = as
    /// aligned as unrelated texts get in this corpus), raw cosine otherwise.
    public static func orientation(episodes: [TransmissionEpisode],
                                   guideStars: [GuideStar] = [],
                                   anchors: [KindAnchor] = [],
                                   params: OrientationParams = OrientationParams(),
                                   calibration: SpaceCalibration? = nil) -> Orientation? {
        var stream = episodes.filter { !$0.vector.isEmpty }
        if let cal = calibration {
            stream = stream.map {
                TransmissionEpisode(id: $0.id, vector: cal.center($0.vector),
                                    text: $0.text, ts: $0.ts)
            }
        }
        guard let clock = stream.map(\.ts).max() else { return nil }

        // WINDOW + WEIGHT + INTEGRATE — one reading per window.
        let slow = integrate(stream, endingAt: clock,
                             spanMs: params.slowSpanMs, halfLifeMs: params.slowHalfLifeMs,
                             language: params.language)
        let fast = integrate(stream, endingAt: clock,
                             spanMs: params.fastSpanMs, halfLifeMs: params.fastHalfLifeMs,
                             language: params.language)
        guard let fastReading = fast, let slowReading = slow else { return nil }

        // FILTER — the complementary blend.
        let heading = slerp(slowReading.heading, fastReading.heading, t: params.blend)

        // RATE — this fast heading against the previous fast-span's, when that
        // span holds any evidence at all.
        var rate: Float = 0
        if clock > params.fastSpanMs,
           let prior = integrate(stream, endingAt: clock - params.fastSpanMs,
                                 spanMs: params.fastSpanMs, halfLifeMs: params.fastHalfLifeMs,
                                 language: params.language) {
            let d = clamp(Float(SpectralClustering.dot(fastReading.heading, prior.heading)))
            rate = acos(d)
        }

        // REFERENCE — guide-star locks + kind alignment, read off the heading.
        // Reference vectors enter the SAME frame as the stream; with a
        // calibration the reported number is the null-calibrated alignment.
        func frame(_ v: [Float]) -> [Float] {
            calibration.map { $0.center(v) } ?? SpectralClustering.normalize(v)
        }
        func report(_ cosine: Float) -> Float {
            calibration.map { $0.calibrated01(cosine) } ?? cosine
        }
        let locks = guideStars
            .map { OrientationLock(id: $0.id,
                                   alignment: report(clamp(Float(SpectralClustering.dot(
                                       heading, frame($0.vector)))))) }
            .sorted { $0.alignment != $1.alignment ? $0.alignment > $1.alignment : $0.id < $1.id }
            .prefix(params.guideStars)

        var kindAlignment: [String: Float] = [:]
        for anchor in anchors {
            kindAlignment[anchor.kind.name] =
                report(clamp(Float(SpectralClustering.dot(heading, frame(anchor.vector)))))
        }

        return Orientation(heading: heading,
                           rateRadians: rate,
                           trim: fastReading.charge,
                           slowTrim: slowReading.charge,
                           coherence: fastReading.resultant,
                           locks: Array(locks),
                           kindAlignment: kindAlignment,
                           evidence: slowReading.count,
                           clock: clock)
    }

    // MARK: - one window's reading

    struct Reading {
        let heading: [Float]
        let resultant: Float   // |Σ wᵢuᵢ| / Σ wᵢ — the coherence of this window
        let charge: Charge     // weight-composed chromodynamic charge
        let count: Int
    }

    /// Integrate one window ending at `end`: exponential decay by age, unit
    /// vectors summed, charge composed under the same weights.
    static func integrate(_ stream: [TransmissionEpisode], endingAt end: UInt64,
                          spanMs: UInt64, halfLifeMs: UInt64,
                          language: NLLanguageIdentifier = .english) -> Reading? {
        let start = end > spanMs ? end - spanMs : 0
        let rows = stream.filter { $0.ts > start && $0.ts <= end }
        guard !rows.isEmpty else { return nil }

        let dim = rows[0].vector.count
        var sum = [Float](repeating: 0, count: dim)
        var totalW: Float = 0
        var charge = Charge.neutral
        for row in rows {
            let age = Float(end - row.ts)
            let w = pow(0.5, age / Float(halfLifeMs))
            let unit = SpectralClustering.normalize(row.vector)
            for d in 0..<dim { sum[d] += w * unit[d] }
            let c = Chromodynamics.charge(row.text, language: language)
            charge.red += w * c.red
            charge.blue += w * c.blue
            charge.gold += w * c.gold
            totalW += w
        }
        guard totalW > 0 else { return nil }

        var mag: Float = 0
        for x in sum { mag += x * x }
        let resultant = mag.squareRoot() / totalW

        return Reading(heading: SpectralClustering.normalize(sum),
                       resultant: min(1, resultant),
                       charge: Charge(red: charge.red / totalW,
                                      blue: charge.blue / totalW,
                                      gold: charge.gold / totalW),
                       count: rows.count)
    }

    // MARK: - spherical blend

    /// Slerp between unit headings. Near-parallel falls back to a normalized
    /// lerp; near-antipodal (pathological for one stream) snaps to the nearer
    /// endpoint rather than inventing an axis.
    static func slerp(_ a: [Float], _ b: [Float], t: Float) -> [Float] {
        let d = clamp(Float(SpectralClustering.dot(a, b)))
        if d > 0.9999 {
            return SpectralClustering.normalize(zip(a, b).map { (1 - t) * $0 + t * $1 })
        }
        if d < -0.9999 { return t < 0.5 ? a : b }
        let theta = acos(d)
        let sa = sin((1 - t) * theta) / sin(theta)
        let sb = sin(t * theta) / sin(theta)
        return SpectralClustering.normalize(zip(a, b).map { sa * $0 + sb * $1 })
    }

    static func clamp(_ x: Float) -> Float { max(-1, min(1, x)) }
}
