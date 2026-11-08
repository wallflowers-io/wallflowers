//  CandidateRegister.swift
//  REGISTRATION (8 Aug directive): identify entities which could PLAUSIBLY be
//  registered as nodes and edges — "this is an intrinsically quantum operation."
//
//  Quantum, structurally (an analogy made load-bearing, and no more than that —
//  no Hilbert spaces, no complex amplitudes, no overclaim):
//    SUPERPOSITION   a candidate before minting is not one thing. It is a
//                    distribution over KINDS (the affinity vector, never
//                    argmaxed), over IDENTITIES (new vs same-as-existing, with
//                    amplitudes), and over EXTENTS (the same surviving grain at
//                    different pyramid scales is the same entity with different
//                    boundaries — all of them carried, linked by lineage).
//    MEASUREMENT     minting IS the collapse. The register never collapses
//                    anything itself; `collapsing(_:toExisting:)` models what
//                    an attestation does to the state, so the UI can show the
//                    consequences of a measurement before a human makes it.
//    ENTANGLEMENT    candidates are not independent: every edge candidate
//                    references node candidates, so collapsing one identity
//                    re-targets the edges that touch it and dissolves its
//                    same-as alternatives. Registration order matters.
//    COMPLEMENTARITY the pyramid's scale ↔ specificity trade-off: sharpen the
//                    instance (fine kernel) and the kind spreads; sharpen the
//                    family (coarse kernel) and the instance blurs. The
//                    register holds readings at every persistent scale rather
//                    than pretending one is true.
//
//  Everything upstream feeds it: the pyramid supplies grains + persistence
//  (only standing waves register — noise never does), the anchors supply the
//  kind amplitudes (calibrated), the resolution calculus's evidence supplies
//  identity amplitudes against the existing graph, chromodynamic charge +
//  strict stream-time precedence supply led_to edge candidates (the causal
//  lens's rule, applied grain-to-grain). Output is EVIDENCE for the mint
//  surface. Deterministic throughout.

import Foundation

// MARK: - The superposed states

/// Identity as a distribution, never a verdict: amplitude that this candidate
/// is NEW, and amplitudes that it is each existing node. Amplitudes are
/// clamped [0,1] and independent — this is evidence weight, not probability
/// theatre; they deliberately do not sum to 1.
public struct IdentitySuperposition: Sendable {
    public let newAmplitude: Float
    public let sameAs: [(id: String, amplitude: Float)]
}

public struct CandidateNode: Sendable {
    public let id: String                 // "<grain>@L<level>" — one EXTENT of an entity
    public let grain: String              // the surviving grain id — lineage key across scales
    public let level: Int
    public let members: [String]          // episode ids, sorted
    public let mass: Float
    public let persistence: Int           // levels survived — why it registered at all
    /// Kind amplitudes over the anchors, normalized over the above-null part —
    /// the superposition the mint surface renders. Empty = kindless, honestly.
    public let kinds: [String: Float]
    public let identity: IdentitySuperposition
    public let charge: Charge
    public let spanMs: (start: UInt64, end: UInt64)
    public let centroid: [Float]
}

public struct CandidateEdge: Sendable {
    public let verb: String               // "led_to" | "same_as"
    public let from: String               // candidate id, or existing node id after collapse
    public let to: String
    public let amplitude: Float
    public let evidence: String           // itemized — the lens shows its working
}

/// The register: an uncollapsed state over possible nodes and edges.
public struct Register: Sendable {
    public let nodes: [CandidateNode]
    public let edges: [CandidateEdge]

    /// MEASUREMENT, modeled: collapse one candidate's identity onto an existing
    /// node. The candidate leaves the register, its same-as alternatives
    /// dissolve, and every edge touching it re-targets — the entangled update.
    public func collapsing(_ candidateId: String, toExisting existingId: String) -> Register {
        let nodes = self.nodes.filter { $0.id != candidateId }
        let edges = self.edges.compactMap { e -> CandidateEdge? in
            if e.verb == "same_as", e.from == candidateId || e.to == candidateId { return nil }
            let from = e.from == candidateId ? existingId : e.from
            let to = e.to == candidateId ? existingId : e.to
            return CandidateEdge(verb: e.verb, from: from, to: to,
                                 amplitude: e.amplitude, evidence: e.evidence)
        }
        return Register(nodes: nodes, edges: edges)
    }
}

// MARK: - The operation

public enum CandidateRegister {

    public struct Params: Sendable {
        /// A grain must survive this many levels to register — the vortex bar.
        public var minPersistence: Int = 2
        public var minMass: Float = 2
        /// Kind amplitudes keep only the above-null part (calibrated 0.5 = unrelated).
        public var kindFloor: Float = 0.5
        /// Identity: calibrated alignment to an existing node above this
        /// contributes a same-as amplitude.
        public var identityFloor: Float = 0.75
        /// led_to: alignment floor + gap half-life (the causal lens's rule).
        public var causalFloor: Float = 0.6
        public var gapHalfLifeMs: UInt64 = 14 * 24 * 3_600_000
        public var pyramid = PyramidParams()
        /// Which Chromodynamics instrument charges candidate nodes (lc-v1 /
        /// lc-kr-v1) — pass the union language authority's value (F-2).
        public var language: NLLanguageIdentifier = .english
        public init() {}
    }

    public static func identify(episodes: [TransmissionEpisode],
                                anchors: [KindAnchor],
                                existing: [GraphNodeRow] = [],
                                calibration: SpaceCalibration? = nil,
                                params: Params = Params()) -> Register {
        guard !episodes.isEmpty else { return Register(nodes: [], edges: []) }
        let byId = Dictionary(uniqueKeysWithValues: episodes.map { ($0.id, $0) })

        func center(_ v: [Float]) -> [Float] {
            calibration.map { $0.center(v) } ?? SpectralClustering.normalize(v)
        }
        func report(_ cos: Float) -> Float {
            calibration.map { $0.calibrated01(cos) } ?? cos
        }
        let framedAnchors = anchors.map { ($0.kind.name, center($0.vector)) }
        let framedExisting = existing.compactMap { n in
            n.vector.map { (n.id, center($0)) }
        }

        // The matter, convolved: grains + persistence from the pyramid.
        let pyramid = CoarseningPyramid.build(ids: episodes.map(\.id),
                                              vectors: episodes.map(\.vector),
                                              params: params.pyramid,
                                              calibration: calibration)

        // Every distinct EXTENT of a persistent grain registers once.
        var seenExtents = Set<String>()
        var nodes: [CandidateNode] = []
        for (level, layer) in pyramid.levels.enumerated() {
            for g in layer.grains where g.members.count >= Int(params.minMass) {
                let extent = "\(g.id)|\(g.members.joined(separator: ","))"
                guard !seenExtents.contains(extent),
                      (pyramid.persistence[g.id] ?? 0) >= params.minPersistence else { continue }
                seenExtents.insert(extent)

                // KIND superposition — the above-null part, normalized, never argmaxed.
                var kinds: [String: Float] = [:]
                for (name, anchor) in framedAnchors {
                    let a = report(Float(SpectralClustering.dot(g.centroid, anchor)))
                    if a > params.kindFloor { kinds[name] = a - params.kindFloor }
                }
                let total = kinds.values.reduce(0, +)
                if total > 0 { kinds = kinds.mapValues { $0 / total } }

                // IDENTITY superposition against the existing graph.
                var sameAs: [(String, Float)] = []
                for (id, vec) in framedExisting {
                    let a = report(Float(SpectralClustering.dot(g.centroid, vec)))
                    if a >= params.identityFloor {
                        sameAs.append((id, min(1, (a - 0.5) * 2)))
                    }
                }
                sameAs.sort { $0.1 != $1.1 ? $0.1 > $1.1 : $0.0 < $1.0 }
                let newAmp = max(0, 1 - (sameAs.first?.1 ?? 0))

                // Charge + span, composed over member episodes.
                let members = g.members.compactMap { byId[$0] }
                let charge = Chromodynamics.compose(
                    members.map { Chromodynamics.charge($0.text, language: params.language) })
                let span = (start: members.map(\.ts).min() ?? 0,
                            end: members.map(\.ts).max() ?? 0)

                nodes.append(CandidateNode(id: "\(g.id)@L\(level)", grain: g.id, level: level,
                                           members: g.members, mass: g.mass,
                                           persistence: pyramid.persistence[g.id] ?? 0,
                                           kinds: kinds,
                                           identity: IdentitySuperposition(newAmplitude: newAmp,
                                                                           sameAs: sameAs),
                                           charge: charge, spanMs: span, centroid: g.centroid))
            }
        }
        nodes.sort { $0.id < $1.id }

        // EDGES. same_as: the identity superposition, in edge form (entangled
        // with its node). led_to: strict span precedence ∧ alignment ∧ the
        // towards-lean of the lead-up — grain-to-grain, finest extents only.
        var edges: [CandidateEdge] = []
        for n in nodes {
            for (id, amp) in n.identity.sameAs {
                edges.append(CandidateEdge(verb: "same_as", from: n.id, to: id, amplitude: amp,
                                           evidence: "aligned \(String(format: "%.2f", amp)) with existing"))
            }
        }
        let finest = Dictionary(grouping: nodes, by: \.grain)
            .compactMap { $0.value.min { $0.level < $1.level } }
            .sorted { $0.id < $1.id }
        for a in finest {
            for b in finest where a.grain != b.grain {
                guard a.spanMs.end < b.spanMs.start else { continue }   // strict precedence
                let align = report(Float(SpectralClustering.dot(a.centroid, b.centroid)))
                guard align >= params.causalFloor else { continue }
                let gap = b.spanMs.start - a.spanMs.end
                let recency = pow(0.5, Float(gap) / Float(params.gapHalfLifeMs))
                let amp = align * (0.5 + 0.5 * a.charge.blue) * recency
                edges.append(CandidateEdge(verb: "led_to", from: a.id, to: b.id, amplitude: amp,
                                           evidence: "precedes by \(gap)ms · aligned \(String(format: "%.2f", align)) · towards-lean \(String(format: "%.2f", a.charge.blue))"))
            }
        }
        edges.sort { $0.amplitude != $1.amplitude ? $0.amplitude > $1.amplitude
                                                  : ($0.from, $0.to) < ($1.from, $1.to) }

        return Register(nodes: nodes, edges: edges)
    }
}
