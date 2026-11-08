//  Proposal.swift
//  The Convergence CONTRACT — the one datastructure shared across its four
//  consumers: the graph and the listing Systems EMIT proposals, the notification
//  tray BATCHES them, the swipe deck RENDERS + decides them, and the classifier
//  TRAINS on (features → decision). Deliberately transport-neutral `Codable`, so a
//  proposal persists and decodes with no adapter.
//
//  A proposal asks ONE useful binary question about the graph:
//    • MINT  — "is this a real thing worth tracking?"  (a new node)
//    • MERGE — "is this the same one you already know?" (bind into an existing node)
//  Right = accept, Left = reject. A merge-reject falls through to a mint (the
//  subject is fully specified on a merge too), so one swipe always resolves it and
//  the entity is never lost. The optional `metadata` string carries the nuance a
//  binary can't — and is richer supervision beside the label.

import Foundation

/// The eight LIFE node types. Groups is split from Person (peer-identity Groups →
/// `person`, standalone → `group`); the other seven are the resolver's set. Raw,
/// UI-free on purpose — glyph/tint live in the view layer.
enum LifeType: String, Codable, CaseIterable, Hashable {
    case person, place, event, project, plant, animal, thing, group
    // The market face's 2×2 (1 Aug retarget) — {artifact, capability} × {held, moving}.
    // `thing`/`plant`/`animal`/`project` stay for legacy graph entities minted before it.
    case inventory, skill, trade, job
}

/// One binary question about the graph.
struct Proposal: Identifiable, Codable, Hashable {
    enum Kind: String, Codable { case mint, merge }

    let id: String
    let kind: Kind
    let subject: Subject
    /// The node a MERGE would bind into; `nil` for a mint.
    let target: Target?
    /// The model's reason — shown small, the "why" behind the question.
    let rationale: String
    /// 0…1. Gates auto-apply UPSTREAM (only the uncertain band reaches the deck),
    /// and is the classifier's regression/label target over time.
    let confidence: Double
    let provenance: Provenance
    /// The classifier's feature vector, FROZEN at emit time. Opaque to the UI —
    /// carried through untouched and echoed back with the decision for training.
    let features: [String: Double]

    /// The entity the card is about. Fully specified even on a MERGE, so
    /// a merge-reject can mint it with no second round-trip.
    struct Subject: Codable, Hashable {
        let name: String
        let type: LifeType
        /// One-line who/what; may be "".
        let summary: String
        /// The source span the mention came from — the context that makes the
        /// binary question answerable ("…pull the coral pair first…").
        let snippet: String
        /// UTF-16 offset + length of `name` within `snippet` for highlighting; when
        /// nil the view finds the first case-insensitive occurrence.
        let highlightStart: Int?
        let highlightLength: Int?
    }

    /// The existing node a MERGE would bind the subject into.
    struct Target: Codable, Hashable {
        /// hard = a GroupObject-backed node (authoritative; the tag hydrates live);
        /// soft = another extracted mention.
        enum Tier: String, Codable { case hard, soft }
        let nodeId: String
        let name: String
        let type: LifeType
        let summary: String
        /// Cosine similarity to the subject, for display.
        let similarity: Double
        let tier: Tier
    }

    struct Provenance: Codable, Hashable {
        /// `music` and `art` are the Systems' sweeps (VenueProposer) — SOURCED proposals,
        /// not derived from the graph.
        enum Source: String, Codable { case note, message, chat, music, art }
        let episodeId: String
        let source: Source
        /// Unix ms the observation happened — the moment a merge hydrates AS-OF.
        let occurredAt: Int
        /// Human provenance line: "your note · Jun 1", "chat with Sophia".
        let label: String
    }
}

/// A batch delivered together — one notification opens one deck.
struct ProposalBatch: Identifiable, Codable, Hashable {
    let id: String
    let createdAt: Int
    let proposals: [Proposal]
}

/// The user's answer: the apply instruction AND the classifier's training row —
/// join `features` on `proposalId` for X, `decision` for y.
struct ProposalDecision: Codable, Hashable {
    enum Decision: String, Codable { case accept, reject }
    let proposalId: String
    let decision: Decision
    /// Optional free-text from the bottom field ("" if none). Parsed lightly at
    /// APPLY time (`→ Type` retypes, `aka X` adds an alias, else a note attribute);
    /// carried raw here.
    let metadata: String
    let decidedAt: Int
}

// MARK: - Sample batch (the "something solid" the deck is built against)

extension Proposal {
    /// A representative spread — a mint, a soft↔hard merge (the GroupObject bridge),
    /// a soft↔soft merge, and a couple of the newer types — for previews and the
    /// deck's first build, before real ones exist.
    static let sample: [Proposal] = [
        Proposal(
            id: "p1", kind: .mint,
            subject: .init(name: "Alice", type: .person,
                           summary: "Started at Acme Corp as a software engineer",
                           snippet: "Alice started a new job at Acme Corp as a software engineer, based in San Francisco.",
                           highlightStart: 0, highlightLength: 5),
            target: nil,
            rationale: "New named individual, not seen before.",
            confidence: 0.44,
            provenance: .init(episodeId: "e1", source: .note, occurredAt: 1_559_347_200_000,
                              label: "your note · Jun 1 2019"),
            features: ["name_len": 5, "type_prior": 0.9, "is_known": 0]),

        Proposal(
            id: "p2", kind: .merge,
            subject: .init(name: "Paris 27", type: .project,
                           summary: "Lookbook sprint mentioned in chat",
                           snippet: "paris 27 lookbook sprint starts now — coral pair first, print deadline the 27th",
                           highlightStart: 0, highlightLength: 8),
            target: .init(nodeId: "grp_9f3a", name: "Paris 27", type: .project,
                          summary: "3 deliverables · Sophia +2 · active",
                          similarity: 0.93, tier: .hard),
            rationale: "Same name as your live Paris 27 project; high similarity.",
            confidence: 0.71,
            provenance: .init(episodeId: "e2", source: .chat, occurredAt: 1_784_000_000_000,
                              label: "chat with Sophia · Thu"),
            features: ["similarity": 0.93, "type_match": 1, "target_tier_hard": 1]),

        Proposal(
            id: "p3", kind: .mint,
            subject: .init(name: "Rex", type: .animal,
                           summary: "Alice's golden retriever",
                           snippet: "Alice adopted a golden retriever named Rex.",
                           highlightStart: 38, highlightLength: 3),
            target: nil,
            rationale: "A named pet — Animal, owned by Alice.",
            confidence: 0.52,
            provenance: .init(episodeId: "e3", source: .note, occurredAt: 1_631_232_000_000,
                              label: "your note · Sep 10 2021"),
            features: ["name_len": 3, "type_prior": 0.8, "has_owner": 1]),

        Proposal(
            id: "p4", kind: .merge,
            subject: .init(name: "the fig", type: .plant,
                           summary: "Fiddle-leaf fig for the apartment",
                           snippet: "Alice bought a fiddle-leaf fig for her apartment; the fig needs more light.",
                           highlightStart: 62, highlightLength: 7),
            target: .init(nodeId: "ent_fig1", name: "fiddle-leaf fig", type: .plant,
                          summary: "Alice's apartment plant",
                          similarity: 0.81, tier: .soft),
            rationale: "Likely the same fiddle-leaf fig mentioned earlier.",
            confidence: 0.66,
            provenance: .init(episodeId: "e4", source: .note, occurredAt: 1_684_540_800_000,
                              label: "your note · May 20 2023"),
            features: ["similarity": 0.81, "type_match": 1, "target_tier_hard": 0]),
    ]
}
