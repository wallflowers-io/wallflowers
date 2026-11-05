//  Topic.swift
//  The Topic object module — the public interface stub.
//
//  A Topic is an AGENT-MANAGED object for deep research into one particular
//  topic. It is not a document you write; it is a dossier kept alive for you.
//  Three defining responsibilities, true of nothing else in the seven:
//
//    (1) THE RESEARCH AGENT LOOP. Every Topic owns a `ResearchAgent` that runs on
//        a PERIODIC schedule (a cadence) and reacts to ACTIVE connections pushing
//        new material. Each pass produces Findings and folds them into the digest.
//        The user does not curate by hand — the agent does, continuously.
//
//    (2) GROUNDED IN SEARCH A. Before reaching outward, the agent reads the user's
//        OWN corpus through the on-device vector index (Search A:
//        `SearchEngine` / `VectorIndex`). The user's Matters are the first-class
//        ground truth the research is anchored to; external sources extend it.
//
//    (3) HONEST-BY-DEFAULT PROVENANCE. Every Finding carries where it came from.
//        A Finding with no citable source is left blank / dropped — it is NEVER
//        fabricated. Unknown stays unknown (mirrors Matter's `nil`, never faked).
//
//  And because a Project subscribes to a Topic, a Topic EMITS a change event when
//  its digest moves, which the subscribing Project's Core Loop Agent consumes.
//
//  This is the interface stub: real, well-named Swift; bodies are unimplemented.

import Foundation
import PacificCore

// MARK: - Provenance: where a Finding came from (honest-by-default)

/// The origin of a Finding. The whole point of the type is that a Finding
/// without a citable origin cannot be represented as "sourced" — provenance is
/// not optional decoration, it is the gate a Finding must pass to be admitted.
public enum FindingSource: Codable, Sendable, Equatable {
    /// Grounded in the user's own corpus: a Matter surfaced by Search A.
    /// Carries the Matter and the relevance score the index returned.
    case corpus(MatterID, score: Float)
    /// Drawn from an active Connection (a connector / System or a networked feed).
    /// `connection` is the Connection's id; `locator` is the in-source reference
    /// (a URL, a record id) — enough to re-fetch and audit, never a paraphrase.
    case connection(id: UUID, locator: URL)
    /// A peer space contributed this under Permissions (C), via Search A federation.
    /// We cite the contributing space, never copy its raw record.
    case peer(SpaceID, locator: URL?)

    /// The auditable locator, when the source is an external one. `nil` for a
    /// corpus finding (whose locator is the MatterID itself) — never fabricated.
    public var locator: URL? {
        switch self {
        case .corpus: return nil
        case let .connection(_, locator): return locator
        case let .peer(_, locator): return locator
        }
    }
}

// MARK: - Finding: one cited claim the agent has established

/// A single established claim in the dossier. A Finding ALWAYS has a source —
/// the initialiser requires one — so an uncited claim is unrepresentable. This
/// is the no-fabrication rule expressed in the type system, not a convention.
public struct Finding: Identifiable, Codable, Sendable, Equatable {
    public let id: UUID
    /// The claim, in the agent's words — but every claim is backed by `source`.
    public var claim: String
    /// Where it came from. Required. No source ⇒ no Finding.
    public var source: FindingSource
    /// The agent's calibrated confidence in [0, 1]. Honest: low confidence is
    /// surfaced, not rounded up; we never present a guess as certainty.
    public var confidence: Float
    /// When this Finding was established (or last re-verified) by a research pass.
    public var establishedAt: Date

    public init(
        id: UUID = UUID(),
        claim: String,
        source: FindingSource,
        confidence: Float,
        establishedAt: Date
    ) {
        self.id = id; self.claim = claim; self.source = source
        self.confidence = confidence; self.establishedAt = establishedAt
    }
}

// MARK: - Connection: an active source the agent pulls from over time

/// What kind of live source a Connection draws on.
public enum ConnectionKind: String, Codable, Sendable {
    /// A registered System object (Onshape, Gmail …) — accessed via its operations.
    case system
    /// A networked feed reached over the Network (E): another space's permitted view.
    case peerFeed
    /// A plain web/HTTP source the agent polls.
    case web
}

/// An active, named source feeding the Topic. A Connection is "active" in two
/// senses: the agent polls it on the Topic's cadence, AND it may push to the
/// agent out of band (e.g. a System raising that an upstream record changed).
/// A Connection authenticates via a Group-issued scoped credential GRANT — it
/// never holds a raw secret (Permissions C; Group's vault).
public struct Connection: Identifiable, Codable, Sendable {
    public let id: UUID
    public var kind: ConnectionKind
    /// Human label ("Onshape assembly tree", "arXiv cs.RO").
    public var label: String
    /// The sibling that backs this connection, when there is one: a System's
    /// Matter for `.system`, the peer's space for `.peerFeed`. `nil` for `.web`.
    public var backing: MatterID?
    /// Where the agent reaches it. For `.system`, this is the System's locator;
    /// for `.web`, the feed URL. Re-fetchable, so any Finding from it is auditable.
    public var endpoint: URL?
    /// The last successful pull. `nil` if never yet reached — not faked as "now".
    public var lastPulledAt: Date?

    public init(
        id: UUID = UUID(),
        kind: ConnectionKind,
        label: String,
        backing: MatterID? = nil,
        endpoint: URL? = nil,
        lastPulledAt: Date? = nil
    ) {
        self.id = id; self.kind = kind; self.label = label
        self.backing = backing; self.endpoint = endpoint; self.lastPulledAt = lastPulledAt
    }
}

// MARK: - The research cadence (the periodic half of the loop)

/// How often the agent runs an unprompted research pass. The active half of the
/// loop (Connections pushing) is independent of this and can fire between ticks.
public enum ResearchCadence: Codable, Sendable, Equatable {
    /// Only when the user or a Connection asks. No background ticking.
    case manual
    /// On a fixed interval (seconds). The simplest periodic schedule.
    case every(seconds: TimeInterval)
    /// Adaptive: the agent widens the interval when findings stop changing and
    /// narrows it when a Connection is churning. `floor`/`ceiling` bound it so it
    /// is never silently disabled and never hammers a source.
    case adaptive(floor: TimeInterval, ceiling: TimeInterval)
}

// MARK: - TopicDigest: the living projection (the dossier)

/// The curated, living projection over Findings — the research dossier itself.
/// It is a PROJECTION, never an independent truth: it is always re-derivable from
/// its `findings` (and they from their sources). The summary is generated, the
/// findings are the record; we never store a conclusion that outlives its evidence.
public struct TopicDigest: Codable, Sendable, Equatable {
    /// A short generated synthesis. Every sentence traces to `findings`; if there
    /// are no findings, the summary is empty — we do not invent one.
    public var summary: String
    /// The cited claims, most-relevant first. The honest core of the dossier.
    public var findings: [Finding]
    /// Open questions the agent could not yet answer from any source. Surfaced
    /// honestly as gaps rather than papered over with a fabricated answer.
    public var openQuestions: [String]
    /// Monotonic version, bumped on every change. Subscribers compare it to know
    /// they have the latest; it is the identity carried in `TopicChange`.
    public var revision: Int
    /// When this revision was produced.
    public var revisedAt: Date

    public init(
        summary: String = "",
        findings: [Finding] = [],
        openQuestions: [String] = [],
        revision: Int = 0,
        revisedAt: Date
    ) {
        self.summary = summary; self.findings = findings
        self.openQuestions = openQuestions; self.revision = revision; self.revisedAt = revisedAt
    }
}

// MARK: - TopicChange: the event a subscribing Project consumes

/// What moved in the digest. A subscribing Project's Core Loop Agent (CLA) reads
/// this to decide whether to recommend or act. It carries the new revision and a
/// minimal delta — not the whole dossier — so it is cheap to seal and ship over E.
public struct TopicChange: Codable, Sendable, Equatable {
    /// The Topic whose digest moved (its backing Matter).
    public let topic: MatterID
    /// The digest revision this change brings the subscriber up to.
    public let toRevision: Int
    /// Findings added since the subscriber's last seen revision.
    public let added: [Finding]
    /// Finding ids removed (e.g. a source retracted or failed re-verification).
    public let removed: [UUID]
    /// Whether the generated summary text changed (subscribers may re-read it).
    public let summaryChanged: Bool
    /// When the change was emitted.
    public let at: Date

    public init(
        topic: MatterID,
        toRevision: Int,
        added: [Finding] = [],
        removed: [UUID] = [],
        summaryChanged: Bool = false,
        at: Date
    ) {
        self.topic = topic; self.toRevision = toRevision
        self.added = added; self.removed = removed
        self.summaryChanged = summaryChanged; self.at = at
    }
}

// MARK: - Errors raised loudly (no silent fallbacks)

/// Failures the Topic surface raises rather than papering over. We never return a
/// stub digest or a fabricated Finding when a source is unreachable.
public enum TopicError: Error, Sendable {
    /// Search A is unavailable — we refuse to "research" ungrounded. Fail loudly.
    case searchUnavailable
    /// A Connection could not be reached on this pass. The pass records the gap as
    /// an open question; it does NOT invent findings to fill it.
    case connectionUnreachable(Connection)
    /// An attempt to admit a claim with no citable source. Rejected by design.
    case uncitedFinding
    /// A subscriber asked for a delta from a revision we no longer retain.
    case revisionTooOld(have: Int, asked: Int)
}

// MARK: - The research agent (the loop itself)

/// The agent that keeps a Topic alive. Conformers implement the two-sided loop:
/// a PERIODIC pass (`tick`) on the Topic's cadence, and reaction to ACTIVE
/// Connections (`ingest`). Both produce Findings that are folded into the digest.
///
/// The agent's contract is honesty: it grounds in Search A first (`ground`), it
/// cites every Finding, and where it cannot establish something it emits an open
/// question rather than a fabricated answer.
public protocol ResearchAgent: Sendable {
    /// Ground the research in the user's own corpus: query Search A (federated
    /// under Permissions C) for Matters relevant to the Topic, and turn the
    /// permitted hits into corpus-sourced candidate Findings. This is run FIRST on
    /// every pass — external work extends this base, it does not replace it.
    /// Throws `searchUnavailable` rather than proceeding ungrounded.
    func ground(
        topic: Topic,
        using search: SearchEngine,
        as viewer: SpaceID
    ) async throws -> [Finding]

    /// One periodic research pass (the scheduled half of the loop). Grounds via
    /// Search A, pulls each active Connection, reconciles against the current
    /// digest, and returns the next digest. Unreachable Connections become open
    /// questions, never invented findings.
    func tick(
        topic: Topic,
        connections: [Connection],
        in context: any PacificContext
    ) async throws -> TopicDigest

    /// React to an active Connection that pushed new material between ticks (the
    /// active half of the loop). Same honesty contract as `tick`.
    func ingest(
        _ update: ConnectionUpdate,
        topic: Topic,
        in context: any PacificContext
    ) async throws -> TopicDigest
}

/// New material a Connection pushed to the agent out of cadence.
public struct ConnectionUpdate: Sendable {
    public let connection: Connection
    /// The raw payload the connection delivered, to be parsed into cited Findings.
    public let payload: Data
    public let at: Date
    public init(connection: Connection, payload: Data, at: Date) {
        self.connection = connection; self.payload = payload; self.at = at
    }
}

// MARK: - The Topic object — a projection of its backing Matter

/// A live Topic instance, projected from one `.topic` Matter. Value type.
public struct Topic: PacificObject {
    public static let kind: ObjectKind = .topic

    public let id: MatterID
    /// The owning space (Identity B) — whose corpus and Connections this researches.
    public let space: SpaceID

    /// The research brief: the question / area the agent is chartered to pursue.
    public var brief: String
    /// How often the agent runs an unprompted pass (the periodic half of the loop).
    public var cadence: ResearchCadence
    /// The active sources feeding this Topic. Listable; their secrets are not here.
    public private(set) var connections: [Connection]
    /// The current living projection. Always re-derivable from its findings;
    /// never an independent source of truth.
    public private(set) var digest: TopicDigest

    public init(from matter: Matter, in context: any PacificContext) throws {
        // TODO: decode brief / cadence / connections from matter.fields; rebuild
        // (or lazily re-derive) the digest from the Topic's Finding Matters.
        // Honest-by-default: a field we cannot decode stays nil/empty, not faked.
        fatalError("unimplemented")
    }

    /// Direct memberwise init (used by tests and by the projection above once built).
    public init(
        id: MatterID,
        space: SpaceID,
        brief: String,
        cadence: ResearchCadence,
        connections: [Connection] = [],
        digest: TopicDigest
    ) {
        self.id = id; self.space = space; self.brief = brief
        self.cadence = cadence; self.connections = connections; self.digest = digest
    }
}

// MARK: - The interface surface siblings and the right rail invoke

/// Everything other modules (chiefly Project, which subscribes) and the three UI
/// rails call on a Topic. The defining surface is the research loop plus the
/// change-event subscription a Project's CLA consumes.
public protocol TopicInterface: Sendable {

    // — Run the loop —

    /// Run one research pass NOW (the right rail's "refresh", or a manual cadence).
    /// Drives the agent's `tick`, persists the new findings as Matters, and emits a
    /// `TopicChange` to subscribers if the digest moved. Returns the new digest.
    func refresh(_ topic: Topic, using agent: any ResearchAgent) async throws -> TopicDigest

    /// Set the periodic cadence (the left/right rail's schedule control). Does not
    /// fire a pass itself; it changes when the background loop next ticks.
    func setCadence(_ cadence: ResearchCadence, on topic: Topic) async throws

    // — Manage active Connections —

    /// Attach an active source (a System, a peer feed, a web feed). The agent will
    /// poll it on cadence and accept its pushes. Authenticates via a Group grant.
    func connect(_ connection: Connection, to topic: Topic) async throws
    /// Detach a source. Findings already cited to it are retained (they remain
    /// auditable by `locator`); no new pulls occur.
    func disconnect(_ connection: Connection, from topic: Topic) async throws

    // — Read the dossier (the centre rail) —

    /// The current living projection — summary, cited findings, open questions.
    func digest(of topic: Topic) -> TopicDigest
    /// The single Finding by id, for the centre rail to show its full citation.
    func finding(_ id: UUID, in topic: Topic) -> Finding?

    // — Subscribe to changes (what a Project's CLA consumes) —

    /// Subscribe to digest changes. The returned stream yields a `TopicChange` on
    /// every revision. A subscribing Project's Core Loop Agent reads it to decide
    /// whether to recommend or act. Delivered over the Network (E) for a remote
    /// subscriber, or a local event channel for a same-space one.
    func subscribe(to topic: Topic) -> AsyncStream<TopicChange>

    /// Catch a subscriber up: the delta from `theirRevision` to current. Throws
    /// `revisionTooOld` rather than silently replaying a fabricated history.
    func changes(of topic: Topic, since theirRevision: Int) async throws -> [TopicChange]
}

// MARK: - Module descriptor (the left rail iterates these)

public struct TopicModule: PacificObjectModule {
    public static let kind: ObjectKind = .topic
    public static let blurb: String =
        "An agent-managed deep-research dossier — grounded in your own corpus, kept live, every finding cited."
}
