//  ResolutionContracts.swift
//  Foundation (workstream F) for the resolution pipeline — the seams every other
//  workstream builds against.
//
//  Two operations: RESOLVE (text -> entity ids, fuzzy, cacheable) and LOOKUP
//  (id -> record). What is sent to LocalStore is a CONTEXT BUNDLE (ResolutionQuery),
//  not a bare span — context is the input to the candidate generators. Candidate-gen
//  is engine-agnostic (DesignResolver.candidates); the buildContext()/decide() seams
//  are deterministic impls that run on the device.

import Foundation

// MARK: - The resolver-facing entity projection

/// The projection a resolver INDEXES. Both the current Sysiphus facade and the real
/// System/Field object produce this, so the resolver is decoupled from either — the
/// key seam that lets WS-A (real object) and the resolver tracks proceed in parallel.
struct ResolvableEntity: Hashable, Identifiable {
    let id: String              // canonical entity id (Sysiphus design id today)
    var kind: String            // "design" / "person" / "org" … (mapped to a marker colour at render)
    var name: String
    var aliases: [String]        // human phrasings — the recall surface
    var text: String            // resolution_text: name + aliases + category + material + …
    var metadata: [String: String]  // season / category / sku / … (for scope filter + k-hop)
    var sourceLocator: String   // how LOOKUP re-reads this from source (e.g. the design id)
}

// MARK: - The context bundle (what is sent to LocalStore)

/// Not a bare span. Context is the INPUT to candidate generation (mention -> lexical,
/// window -> in-context embedding, anchors -> k-hop, scope -> metadata filter).
struct ResolutionQuery: Hashable {
    var mention: String                 // the span, e.g. "cargo shorts"
    var window: String                  // surrounding text (sentence / message)
    var anchors: [String] = []           // entity ids already resolved in this thread
    var conversation: String? = nil      // for coreference / recency
    var asOf: Date = Date()
    var scope: String? = nil             // credential scope key (whose index we may hit)

    /// The bare-span convenience: mention == window, no anchors.
    static func bare(_ text: String) -> ResolutionQuery {
        ResolutionQuery(mention: text, window: text)
    }
}

// MARK: - Results

/// A candidate entity from generation, before the decider picks.
struct Candidate: Hashable {
    let entityId: String
    let score: Float
    var generator: String = "lexical"   // which generator produced it (lexical/vector/hybrid/graph)
}

/// The decider's verdict over a candidate set.
struct Decision: Hashable {
    var entityId: String?
    var confidence: Float
    var abstained: Bool
    static let abstain = Decision(entityId: nil, confidence: 0, abstained: true)
}

/// A resolved occurrence inside a specific string — the cached unit the render layer reads.
struct ResolvedSpan: Hashable {
    var range: Range<String.Index>
    var entityId: String
    var kind: String
    var confidence: Float
}

// MARK: - The shaping and deciding seams (deterministic, on device)

/// FRONT: shape a raw span + thread into a context bundle (coreference / mention expansion).
/// Default = identity.
protocol MentionShaper {
    func build(mention: String, window: String, anchors: [String]) -> ResolutionQuery
}

struct IdentityShaper: MentionShaper {
    func build(mention: String, window: String, anchors: [String]) -> ResolutionQuery {
        ResolutionQuery(mention: mention, window: window, anchors: anchors)
    }
}

/// BACK: pick from a candidate set (select-from-set + calibrated abstain). Default = argmax
/// above a confidence floor; an ambiguous set abstains rather than guessing. `async` so an
/// implementation may do real work off the caller's actor; the heuristic default returns
/// without suspending.
protocol Decider {
    func decide(_ query: ResolutionQuery, _ candidates: [Candidate]) async -> Decision
}

struct HeuristicDecider: Decider {
    /// Accept the top candidate when it clears the floor AND leads the runner-up; else abstain.
    var floor: Float = 0.5
    var margin: Float = 1.2   // top must beat 2nd by this factor when a 2nd exists

    func decide(_ query: ResolutionQuery, _ candidates: [Candidate]) async -> Decision {
        let sorted = candidates.sorted { $0.score > $1.score }
        guard let top = sorted.first else { return .abstain }
        // Normalise the top score to a rough 0…1 confidence for the floor test.
        let conf = min(1, top.score / max(1, (sorted.dropFirst().first?.score ?? 0) + top.score))
        if top.score >= floor,
           (sorted.count == 1 || top.score >= (sorted[1].score * margin)) {
            return Decision(entityId: top.entityId, confidence: conf, abstained: false)
        }
        return .abstain
    }
}
