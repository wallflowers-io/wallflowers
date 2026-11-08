//  GraphResolution.swift
//  THE RESOLUTION CALCULUS (8 Aug directive): filter and identify resolved and
//  deduplicated nodes and edges in a bitemporal knowledge graph — constructed
//  from fundamental logical units, composed in four layers:
//
//    L0 · TEMPORAL/IDENTITY PREDICATES   alive · known · current · overlap ·
//        compatible (kind lattice) · sourceDisjoint. The bitemporal atoms:
//        event time (valid_at/invalid_at — when true in the world) is NEVER
//        conflated with transaction time (created_at/expired_at — when this
//        store learned/superseded it). Every question is asked AS-OF (t, τ).
//    L1 · EVIDENCE GAUGES   lex (normalized labels, token overlap) · sem
//        (CALIBRATED cosine — SpaceCalibration required on real vectors; a raw
//        cosine in an anisotropic space is not evidence) · geo (located kinds)
//        · neigh (shared typed neighbours — the graph vouching for itself).
//    L2 · IDENTITY   candidate blocking → per-kind MATCH RULES (named
//        conjunctions of L1 evidence — a match always knows which rule earned
//        it) → union-find closure WITH VETO (transitive chaining is the classic
//        ER failure: two records the same source lists separately may never
//        fuse, so any union that would put source-disjoint rows in one class
//        is refused) → deterministic canonicalization (minted > folded >
//        reference, then earliest created, then id — "minted wins",
//        generalized).
//    L3 · EDGE RESOLUTION   rewrite endpoints to canonicals → group by
//        (src, verb, dst, scope) → COALESCE same-assertion valid intervals
//        that touch (bitemporal normalization) → CONFLICTS are surfaced, never
//        averaged: one provenance closing an interval another holds open is a
//        finding for a human, not a tie for a tiebreaker.
//
//  DOCTRINE. The output is a VIEW: a canon map, justified identifications,
//  coalesced edges, flagged conflicts, and PROPOSALS (near-misses for the mint
//  surface). Nothing here rewrites the graph — the fold is truth, bitemporal
//  stores never destroy, and a durable merge (same_as) is a user act. Every
//  step is deterministic: fixed thresholds, sorted iteration, id tie-breaks.

import Foundation

// MARK: - Rows (the abstract shape of lodedb-graph records)

public struct BiTime: Sendable, Equatable {
    public var validAt: UInt64          // event time: true in the world from…
    public var invalidAt: UInt64?       // …until (nil = still true)
    public var createdAt: UInt64        // transaction time: this store learned it at…
    public var expiredAt: UInt64?       // …and superseded it at (nil = current record)

    public init(validAt: UInt64, invalidAt: UInt64? = nil,
                createdAt: UInt64, expiredAt: UInt64? = nil) {
        self.validAt = validAt
        self.invalidAt = invalidAt
        self.createdAt = createdAt
        self.expiredAt = expiredAt
    }
}

/// Where a row came from — the rank that decides canonicals and the identity
/// of a source for the disjointness veto.
public enum Provenance: Sendable, Equatable {
    case minted(author: String)                  // a user attested it — rank 0
    case folded(object: String)                  // projected from a GroupObject — rank 1
    case reference(source: String, key: String)  // an external feed row — rank 2

    var rank: Int {
        switch self {
        case .minted: return 0
        case .folded: return 1
        case .reference: return 2
        }
    }

    /// The source identity for the veto: two rows the SAME source deliberately
    /// lists apart must never fuse. Minted rows are never source-disjoint —
    /// a person may genuinely mint the same thing twice.
    var disjointKey: String? {
        switch self {
        case .minted: return nil
        case .folded(let object): return "folded:\(object)"
        case .reference(let source, _): return "ref:\(source)"
        }
    }
}

public struct GraphNodeRow: Sendable {
    public let id: String
    public let kind: String
    public let label: String
    public let scope: String
    public let provenance: Provenance
    public let time: BiTime
    public let vector: [Float]?
    public let coordinate: (lat: Double, lon: Double)?

    public init(id: String, kind: String, label: String, scope: String = "",
                provenance: Provenance, time: BiTime,
                vector: [Float]? = nil, coordinate: (lat: Double, lon: Double)? = nil) {
        self.id = id
        self.kind = kind
        self.label = label
        self.scope = scope
        self.provenance = provenance
        self.time = time
        self.vector = vector
        self.coordinate = coordinate
    }
}

public struct GraphEdgeRow: Sendable {
    public let id: String
    public let src: String
    public let verb: String
    public let dst: String
    public let scope: String
    public let provenance: Provenance
    public let time: BiTime
    public let episode: String?

    public init(id: String, src: String, verb: String, dst: String, scope: String = "",
                provenance: Provenance, time: BiTime, episode: String? = nil) {
        self.id = id
        self.src = src
        self.verb = verb
        self.dst = dst
        self.scope = scope
        self.provenance = provenance
        self.time = time
        self.episode = episode
    }
}

// MARK: - L0 · temporal + identity predicates

public enum L0 {
    /// Event-time currency: true in the world at t.
    public static func alive(_ b: BiTime, at t: UInt64) -> Bool {
        b.validAt <= t && (b.invalidAt.map { t < $0 } ?? true)
    }

    /// Transaction-time currency: this store believed it at τ.
    public static func known(_ b: BiTime, at tau: UInt64) -> Bool {
        b.createdAt <= tau && (b.expiredAt.map { tau < $0 } ?? true)
    }

    /// THE bitemporal filter: true in the world at t, believed at τ.
    public static func current(_ b: BiTime, t: UInt64, tau: UInt64) -> Bool {
        alive(b, at: t) && known(b, at: tau)
    }

    /// Event-time co-existence of two records.
    public static func overlap(_ a: BiTime, _ b: BiTime) -> Bool {
        let aEnd = a.invalidAt ?? .max
        let bEnd = b.invalidAt ?? .max
        return a.validAt < bEnd && b.validAt < aEnd
    }

    /// Kind compatibility under the ontology's subclass lattice (Land ⊑ Place):
    /// equal, or one subsumes the other by parent-chain.
    public static func compatible(_ x: String, _ y: String, lattice: [String: String]) -> Bool {
        if x == y { return true }
        func chain(_ k: String) -> Set<String> {
            var out: Set<String> = [k]
            var cur = k
            while let p = lattice[cur], !out.contains(p) { out.insert(p); cur = p }
            return out
        }
        return chain(x).contains(y) || chain(y).contains(x)
    }

    /// The disjointness axiom: one source listing two rows separately is that
    /// source ASSERTING they differ.
    public static func sourceDisjoint(_ a: Provenance, _ b: Provenance) -> Bool {
        guard let ka = a.disjointKey, let kb = b.disjointKey, ka == kb else { return false }
        if case .reference(_, let x) = a, case .reference(_, let y) = b { return x != y }
        if case .folded(let x) = a, case .folded(let y) = b { return x != y }
        return false
    }
}

// MARK: - L1 · evidence gauges

public struct Evidence: Sendable, Equatable {
    public var lexExact: Bool      // normalized labels identical
    public var lexOverlap: Float   // token Jaccard ∈ [0,1]
    public var sem: Float?         // CALIBRATED alignment (0.5 = unrelated); nil = no vectors
    public var geoNear: Bool?      // within the kind's radius; nil = no coordinates
    public var neigh: Float        // typed-neighbour Jaccard ∈ [0,1]
}

public enum L1 {
    /// Label normalization: lowercase, alphanumerics, collapsed whitespace.
    public static func normal(_ s: String) -> String {
        s.lowercased()
            .components(separatedBy: CharacterSet.alphanumerics.inverted)
            .filter { !$0.isEmpty }
            .joined(separator: " ")
    }

    public static func lexOverlap(_ a: String, _ b: String) -> Float {
        let ta = Set(normal(a).split(separator: " "))
        let tb = Set(normal(b).split(separator: " "))
        guard !ta.isEmpty, !tb.isEmpty else { return 0 }
        return Float(ta.intersection(tb).count) / Float(ta.union(tb).count)
    }

    /// Calibrated semantic evidence. RAW COSINE IS NOT EVIDENCE in an
    /// anisotropic space — this gauge exists only through the calibration.
    public static func sem(_ a: [Float]?, _ b: [Float]?, calibration: SpaceCalibration?) -> Float? {
        guard let a, let b, let cal = calibration else { return nil }
        return cal.calibrated01(Float(SpectralClustering.dot(cal.center(a), cal.center(b))))
    }

    /// Equirectangular metres — exact enough at venue radii.
    public static func geoNear(_ a: (lat: Double, lon: Double)?, _ b: (lat: Double, lon: Double)?,
                               radiusM: Double) -> Bool? {
        guard let a, let b else { return nil }
        let dLat = (a.lat - b.lat) * 111_320.0
        let dLon = (a.lon - b.lon) * 111_320.0 * cos((a.lat + b.lat) * .pi / 360)
        return (dLat * dLat + dLon * dLon).squareRoot() <= radiusM
    }

    /// The graph vouching for itself: Jaccard over (verb, neighbour) sets.
    public static func neigh(_ x: String, _ y: String, edges: [GraphEdgeRow]) -> Float {
        func around(_ id: String) -> Set<String> {
            var out: Set<String> = []
            for e in edges {
                if e.src == id { out.insert("\(e.verb)>\(e.dst)") }
                if e.dst == id { out.insert("\(e.verb)<\(e.src)") }
            }
            return out
        }
        let a = around(x), b = around(y)
        guard !a.isEmpty, !b.isEmpty else { return 0 }
        return Float(a.intersection(b).count) / Float(a.union(b).count)
    }
}

// MARK: - L2 · identity

/// A named conjunction of evidence conditions — a match always knows which
/// rule earned it. Rules are per-kind; SEM alone never suffices for Person
/// (namesakes are the canonical trap).
public struct MatchRule: Sendable {
    public enum Condition: Sendable, Equatable {
        case lexExact
        case lexOverlapAtLeast(Float)
        case semAtLeast(Float)       // calibrated scale: 0.5 = unrelated
        case geoNear
        case neighAtLeast(Float)
    }
    public let name: String
    public let requires: [Condition]

    public init(name: String, requires: [Condition]) {
        self.name = name
        self.requires = requires
    }

    func satisfied(by e: Evidence) -> Bool {
        requires.allSatisfy { c in
            switch c {
            case .lexExact:                 return e.lexExact
            case .lexOverlapAtLeast(let x): return e.lexOverlap >= x
            case .semAtLeast(let x):        return (e.sem ?? 0) >= x
            case .geoNear:                  return e.geoNear == true
            case .neighAtLeast(let x):      return e.neigh >= x
            }
        }
    }
}

public struct ResolutionParams: Sendable {
    /// Per-kind rules; a kind with no entry never auto-identifies.
    public var rules: [String: [MatchRule]] = ResolutionParams.defaultRules
    /// Blocking gate: pairs must clear at least one of these cheaply.
    public var blockLexOverlap: Float = 0.3
    public var blockSem: Float = 0.8
    /// Near-miss band → proposals for the mint surface.
    public var proposeSem: Float = 0.85
    /// Subclass lattice (child → parent), read from the ontology.
    public var lattice: [String: String] = ["Land": "Place", "Mediation": "Project"]
    public var geoRadiusM: Double = 150
    public init() {}

    public static let defaultRules: [String: [MatchRule]] = [
        "Place": [
            MatchRule(name: "lex+geo", requires: [.lexExact, .geoNear]),
            MatchRule(name: "sem+geo", requires: [.semAtLeast(0.9), .geoNear]),
            MatchRule(name: "lex+neigh", requires: [.lexExact, .neighAtLeast(0.3)]),
        ],
        "Event": [
            MatchRule(name: "lex+overlap", requires: [.lexExact]),   // + L0.overlap gate below
            MatchRule(name: "sem+neigh", requires: [.semAtLeast(0.9), .neighAtLeast(0.3)]),
        ],
        "Person": [
            // Never SEM alone: namesakes. The graph must vouch.
            MatchRule(name: "lex+neigh", requires: [.lexExact, .neighAtLeast(0.25)]),
            MatchRule(name: "lex+sem+neigh",
                      requires: [.lexOverlapAtLeast(0.6), .semAtLeast(0.9), .neighAtLeast(0.2)]),
        ],
        "Group": [
            MatchRule(name: "lex", requires: [.lexExact]),
            MatchRule(name: "sem+neigh", requires: [.semAtLeast(0.9), .neighAtLeast(0.3)]),
        ],
    ]
}

// MARK: - Output

public struct Identification: Sendable {
    public let a: String
    public let b: String
    public let rule: String        // which rule earned it — the justification
    public let evidence: Evidence
}

public struct EdgeConflict: Sendable {
    public let edgeA: String
    public let edgeB: String
    public let reason: String
}

/// The resolved, deduplicated VIEW — computed, never written back.
public struct ResolvedView: Sendable {
    public let canon: [String: String]          // every node id → its canonical id
    public let nodes: [GraphNodeRow]            // canonical nodes, current at (t, τ)
    public let edges: [GraphEdgeRow]            // rewritten + coalesced, current at (t, τ)
    public let identifications: [Identification]
    public let conflicts: [EdgeConflict]
    /// Near-misses for the mint surface: pairs the calculus will not fuse but
    /// a human might — the same_as proposal seam.
    public let proposals: [Identification]
}

// MARK: - The operation

public enum GraphResolution {

    /// The complex operation, composed: FILTER (bitemporal, at t·τ) → BLOCK →
    /// MATCH (per-kind rules) → CLOSE (vetoed union-find) → CANONICALIZE →
    /// REWRITE + COALESCE edges → surface conflicts and proposals.
    public static func resolve(nodes allNodes: [GraphNodeRow],
                               edges allEdges: [GraphEdgeRow],
                               t: UInt64, tau: UInt64,
                               params: ResolutionParams = ResolutionParams(),
                               calibration: SpaceCalibration? = nil) -> ResolvedView {
        // L0 · FILTER — only rows true at t and believed at τ take part.
        let nodes = allNodes.filter { L0.current($0.time, t: t, tau: tau) }.sorted { $0.id < $1.id }
        let edges = allEdges.filter { L0.current($0.time, t: t, tau: tau) }.sorted { $0.id < $1.id }
        let byId = Dictionary(uniqueKeysWithValues: nodes.map { ($0.id, $0) })

        // L2 · BLOCK + MATCH.
        var identifications: [Identification] = []
        var proposals: [Identification] = []
        for i in 0..<nodes.count {
            for j in (i + 1)..<nodes.count {
                let a = nodes[i], b = nodes[j]
                guard a.scope == b.scope,
                      L0.compatible(a.kind, b.kind, lattice: params.lattice),
                      !L0.sourceDisjoint(a.provenance, b.provenance) else { continue }
                if a.kind == "Event" || b.kind == "Event" {
                    guard L0.overlap(a.time, b.time) else { continue }
                }

                let sem = L1.sem(a.vector, b.vector, calibration: calibration)
                let lexOv = L1.lexOverlap(a.label, b.label)
                guard lexOv >= params.blockLexOverlap || (sem ?? 0) >= params.blockSem else { continue }

                let e = Evidence(lexExact: L1.normal(a.label) == L1.normal(b.label)
                                            && !L1.normal(a.label).isEmpty,
                                 lexOverlap: lexOv,
                                 sem: sem,
                                 geoNear: L1.geoNear(a.coordinate, b.coordinate,
                                                     radiusM: params.geoRadiusM),
                                 neigh: L1.neigh(a.id, b.id, edges: edges))

                let kind = a.kind == b.kind ? a.kind
                    : (params.lattice[a.kind] == b.kind ? b.kind : a.kind)
                if let rule = (params.rules[kind] ?? []).first(where: { $0.satisfied(by: e) }) {
                    identifications.append(Identification(a: a.id, b: b.id,
                                                          rule: rule.name, evidence: e))
                } else if (e.sem ?? 0) >= params.proposeSem || e.lexExact {
                    proposals.append(Identification(a: a.id, b: b.id,
                                                    rule: "proposal", evidence: e))
                }
            }
        }

        // L2 · CLOSE — union-find with the source-disjointness veto. Unions run
        // in DESCENDING evidence order (rule priority, then semantic strength,
        // then lexical overlap): a weak early match must never poison a class
        // and get a strong later match vetoed — the bench caught exactly that.
        let ruleIndex: [String: Int] = {
            var out: [String: Int] = [:]
            for (_, rules) in params.rules {
                for (i, r) in rules.enumerated() where out[r.name] == nil { out[r.name] = i }
            }
            return out
        }()
        identifications.sort {
            let ra = ruleIndex[$0.rule] ?? .max, rb = ruleIndex[$1.rule] ?? .max
            if ra != rb { return ra < rb }
            let sa = $0.evidence.sem ?? 0, sb = $1.evidence.sem ?? 0
            if sa != sb { return sa > sb }
            if $0.evidence.lexOverlap != $1.evidence.lexOverlap {
                return $0.evidence.lexOverlap > $1.evidence.lexOverlap
            }
            return ($0.a, $0.b) < ($1.a, $1.b)
        }
        var parent: [String: String] = [:]
        func find(_ x: String) -> String {
            var r = x
            while let p = parent[r], p != r { r = p }
            parent[x] = r
            return r
        }
        for n in nodes { parent[n.id] = n.id }
        func classMembers(_ root: String) -> [GraphNodeRow] {
            nodes.filter { find($0.id) == root }
        }
        var accepted: [Identification] = []
        for ident in identifications {
            let ra = find(ident.a), rb = find(ident.b)
            guard ra != rb else { accepted.append(ident); continue }
            let merged = classMembers(ra) + classMembers(rb)
            var vetoed = false
            outer: for x in 0..<merged.count {
                for y in (x + 1)..<merged.count
                where L0.sourceDisjoint(merged[x].provenance, merged[y].provenance) {
                    vetoed = true
                    break outer
                }
            }
            if vetoed {
                proposals.append(ident)   // the calculus refuses; a human may not
            } else {
                parent[max(ra, rb)] = min(ra, rb)   // deterministic direction
                accepted.append(ident)
            }
        }

        // L2 · CANONICALIZE — minted > folded > reference, then earliest, then id.
        var canon: [String: String] = [:]
        var classes: [String: [GraphNodeRow]] = [:]
        for n in nodes { classes[find(n.id), default: []].append(n) }
        var canonicalNodes: [GraphNodeRow] = []
        for (_, members) in classes.sorted(by: { $0.key < $1.key }) {
            let winner = members.min {
                ($0.provenance.rank, $0.time.createdAt, $0.id)
                    < ($1.provenance.rank, $1.time.createdAt, $1.id)
            }!
            canonicalNodes.append(winner)
            for m in members { canon[m.id] = winner.id }
        }

        // L3 · REWRITE + COALESCE + CONFLICTS.
        var grouped: [String: [GraphEdgeRow]] = [:]
        for e in edges {
            // Endpoints outside the input slice pass through UNRESOLVED — an
            // edge to a node this call wasn't given is not an edge to nothing.
            let s = canon[e.src] ?? e.src
            let d = canon[e.dst] ?? e.dst
            grouped["\(s)|\(e.verb)|\(d)|\(e.scope)", default: []].append(
                GraphEdgeRow(id: e.id, src: s, verb: e.verb, dst: d, scope: e.scope,
                             provenance: e.provenance, time: e.time, episode: e.episode))
        }
        var resolvedEdges: [GraphEdgeRow] = []
        var conflicts: [EdgeConflict] = []
        for (_, group) in grouped.sorted(by: { $0.key < $1.key }) {
            let sorted = group.sorted { ($0.time.validAt, $0.id) < ($1.time.validAt, $1.id) }
            var open: GraphEdgeRow? = nil
            for e in sorted {
                guard let cur = open else { open = e; continue }
                let curEnd = cur.time.invalidAt
                if curEnd == nil || e.time.validAt <= curEnd! {
                    // Same assertion, touching intervals. A closed interval
                    // overlapped by one held open is a CONFLICT, not a merge.
                    if let end = curEnd, e.time.invalidAt == nil, e.time.validAt < end {
                        conflicts.append(EdgeConflict(edgeA: cur.id, edgeB: e.id,
                                                      reason: "open assertion overlaps a closed one"))
                    }
                    let mergedEnd: UInt64? = (curEnd == nil || e.time.invalidAt == nil)
                        ? nil : max(curEnd!, e.time.invalidAt!)
                    let keep = cur.provenance.rank <= e.provenance.rank ? cur : e
                    open = GraphEdgeRow(id: keep.id, src: cur.src, verb: cur.verb, dst: cur.dst,
                                        scope: cur.scope, provenance: keep.provenance,
                                        time: BiTime(validAt: min(cur.time.validAt, e.time.validAt),
                                                     invalidAt: mergedEnd,
                                                     createdAt: min(cur.time.createdAt, e.time.createdAt),
                                                     expiredAt: nil),
                                        episode: cur.episode ?? e.episode)
                } else {
                    resolvedEdges.append(cur)
                    open = e
                }
            }
            if let cur = open { resolvedEdges.append(cur) }
        }

        return ResolvedView(canon: canon,
                            nodes: canonicalNodes,
                            edges: resolvedEdges,
                            identifications: accepted,
                            conflicts: conflicts,
                            proposals: proposals)
    }
}
