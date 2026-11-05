//  CausalLens.swift
//  THE CAUSAL EMBEDDING MODEL's query operator (8 Aug naming): the composite of
//  the six instruments is an embedding model whose geometry carries the
//  PRECONDITIONS of causal claims — and this lens is where they compose into a
//  causal question: WHAT LED HERE, and WHAT FOLLOWED FROM HERE.
//
//  Why the name is earned, precisely:
//    · a frozen encoder is correlational — cosine is symmetric and timeless;
//    · this stack breaks the symmetry three ways: TIME (fold-anchored spans,
//      the stream's own clock, bitemporal as-of), DIRECTION (the chromodynamic
//      arrow — from vs towards is an orientation cosine never had), and
//      PERSISTENCE (structure that survives coarsening — the vortex test);
//    · and it closes with ATTESTATION: the model assembles evidence, a person
//      establishes the cause (a minted same_as / led_to).
//
//  And the boundary, stated so it cannot be overclaimed: this is OBSERVATIONAL
//  + TESTIMONIAL machinery. No interventions, no counterfactuals, no
//  do-calculus — the lens NEVER asserts causation. An antecedent is a
//  candidate: aligned (calibrated), strictly prior (stream time), and leaning
//  the right way (charge). The output is ranked evidence with its working
//  shown, feeding the mint surface where causes are actually established.
//
//  ANTECEDENTS(x): episodes strictly BEFORE x, aligned with x, weighted toward
//  those that lean TOWARDS (blue — plans, intents: the shape of a lead-up).
//  CONSEQUENTS(x): episodes strictly AFTER x, aligned with x, weighted toward
//  those that lean FROM (red — recollection, reference-back: the shape of a
//  wake). Deterministic throughout; calibration mandatory on real vectors.

import Foundation

/// One candidate in a causal query: the evidence, itemized — never a verdict.
public struct CausalTrace: Sendable {
    public let id: String
    /// Calibrated alignment to the target (0.5 = unrelated) — raw cosine when
    /// no calibration was given (synthetic geometry only).
    public let alignment: Float
    /// Stream-time gap to the target, milliseconds. Always > 0: precedence is
    /// strict, simultaneity proves nothing.
    public let gapMs: UInt64
    /// The episode's chromodynamic charge — the lean that earned its weight.
    public let charge: Charge
    /// alignment · directional lean · recency of the gap. The ranking value,
    /// derived from the three shown ingredients and nothing else.
    public let score: Float
}

public enum CausalLens {

    public struct Params: Sendable {
        /// Alignment floor: below this the episode is unrelated, whatever its
        /// timing (on the calibrated scale 0.5 = null).
        public var minAlignment: Float = 0.6
        /// Gap half-life: evidence decays with distance in stream time.
        public var gapHalfLifeMs: UInt64 = 14 * 24 * 3_600_000
        public var k: Int = 10
        /// Which Chromodynamics instrument charges the traces (lc-v1 / lc-kr-v1)
        /// — pass the union language authority's value (F-2, one authority).
        public var language: NLLanguageIdentifier = .english
        public init() {}
    }

    /// What led here: strictly-prior, aligned episodes, weighted toward the
    /// TOWARDS lean — a lead-up talks about what it is heading into.
    public static func antecedents(of target: (vector: [Float], ts: UInt64),
                                   in episodes: [TransmissionEpisode],
                                   calibration: SpaceCalibration? = nil,
                                   params: Params = Params()) -> [CausalTrace] {
        trace(target: target, episodes: episodes.filter { $0.ts < target.ts },
              gap: { target.ts - $0 },
              lean: { $0.blue },
              calibration: calibration, params: params)
    }

    /// What followed from here: strictly-later, aligned episodes, weighted
    /// toward the FROM lean — a wake talks about where it came from.
    public static func consequents(of target: (vector: [Float], ts: UInt64),
                                   in episodes: [TransmissionEpisode],
                                   calibration: SpaceCalibration? = nil,
                                   params: Params = Params()) -> [CausalTrace] {
        trace(target: target, episodes: episodes.filter { $0.ts > target.ts },
              gap: { $0 - target.ts },
              lean: { $0.red },
              calibration: calibration, params: params)
    }

    private static func trace(target: (vector: [Float], ts: UInt64),
                              episodes: [TransmissionEpisode],
                              gap: (UInt64) -> UInt64,
                              lean: (Charge) -> Float,
                              calibration: SpaceCalibration?,
                              params: Params) -> [CausalTrace] {
        let t = calibration.map { $0.center(target.vector) }
            ?? SpectralClustering.normalize(target.vector)

        var out: [CausalTrace] = []
        for e in episodes where !e.vector.isEmpty {
            let v = calibration.map { $0.center(e.vector) }
                ?? SpectralClustering.normalize(e.vector)
            let cos = Float(SpectralClustering.dot(t, v))
            let alignment = calibration.map { $0.calibrated01(cos) } ?? cos
            guard alignment >= params.minAlignment else { continue }

            let g = gap(e.ts)
            let recency = pow(0.5, Float(g) / Float(params.gapHalfLifeMs))
            let charge = Chromodynamics.charge(e.text, language: params.language)
            // The lean opens the gate wider, it never closes it: an aligned,
            // prior episode with no directional markers is still evidence.
            let score = alignment * (0.5 + 0.5 * lean(charge)) * recency

            out.append(CausalTrace(id: e.id, alignment: alignment, gapMs: g,
                                   charge: charge, score: score))
        }
        return Array(out
            .sorted { $0.score != $1.score ? $0.score > $1.score : $0.id < $1.id }
            .prefix(params.k))
    }
}
