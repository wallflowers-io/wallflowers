//  Forum.swift
//  A multi-user thread — like a Slack channel. The conversational GroupObject:
//  it hosts post / poll / question as delta op-groups (the former standalone
//  Poll #16, Message #17 and Question #24 folded in — see pacific-core/src/object.rs).
//
//  A Forum is a projection of a *hub* Matter (kind `.forum`) plus a stream of
//  *post* Matters (also kind `.forum`) that reference it. The hub carries two
//  keyed maps — `topic_areas` (sub-threads) and `members` (Groups) — and the
//  identity of the MLS group the Forum syncs across.
//
//    F Records      — hub Matter + one Matter per post; the honest source of truth.
//    B Identity      — each post is authored & sealed by its author's SpaceID.
//    C Permissions   — per-(member, topic_area) Disclosure gates visibility.
//    D Dictionary    — the `forum` shape; topic_areas/members are keyed fields.
//    E Network/MLS   — the Forum *is* an MLS group; posts fan out as Envelopes.
//    A Search        — every post Matter is indexed for on-device vector search.
//
//  Interface stub: types and the operation surface the right rail and sibling
//  objects invoke. Bodies are intentionally unimplemented.

import Foundation
import PacificCore

// MARK: - Keys used in the hub Matter's `fields`

/// The well-known field keys the Forum hub projects from. Keeping them in one
/// place is the single source of truth for the on-disk shape.
public enum ForumField {
    public static let title         = "title"          // .text
    public static let mlsGroup      = "mls_group"      // .text — the MLS group id (E)
    /// Keyed maps are flattened into the Matter's flat `[String: FieldValue]`
    /// under namespaced prefixes, e.g. `topic_area/<key>` and `member/<spaceID>`.
    public static let topicAreaPrefix = "topic_area/"  // value: .text (display name)
    public static let memberPrefix    = "member/"      // value: .text (Membership role)
}

/// The well-known field keys a *post* Matter projects from.
public enum PostField {
    public static let forum     = "forum"        // .ref(hub MatterID)
    public static let topicArea = "topic_area"   // .text (the key within the forum)
    public static let body      = "body"         // .text
    /// Attachments are flattened as `attachment/<n>` → .blob(id:).
    public static let attachmentPrefix = "attachment/"
}

// MARK: - Core value types

/// A namespaced sub-thread (channel) within a Forum. The `key` is stable and
/// used in routing & permissions; the `name` is the human label.
///
/// DESIGN DECISION (2026-06-27, see Objects/PRIMITIVE-DECISIONS.txt): RENAME this
/// to `Channel` — and `ForumField.topicAreaPrefix` "topic_area/" → "channel/",
/// `PostField.topicArea` → `channel`. "Topic area" is being reserved for a DIFFERENT
/// concept (a recursive sub-Project in the Work outline), so the two must not share a
/// name. NOT yet applied (the rename spans this file + the Rust ForumType reducer).
public struct TopicArea: Identifiable, Hashable, Sendable {
    public let key: String          // stable key, unique within the Forum
    public var name: String         // display name
    public var id: String { key }
    public init(key: String, name: String) {
        self.key = key; self.name = name
    }
}

/// A member's role in the Forum. Governs default disclosure, not a separate ACL.
public enum MemberRole: String, Codable, Sendable {
    case owner      // can add/remove members & topic_areas, set permissions
    case poster     // can post and read all permitted topic_areas
    case reader     // read-only
}

/// One entry in the Forum's keyed `members` map: a Group, by its SpaceID.
public struct Membership: Identifiable, Hashable, Sendable {
    public let group: SpaceID       // the member Group's Identity space (B)
    public var role: MemberRole
    public var id: SpaceID { group }
    public init(group: SpaceID, role: MemberRole) {
        self.group = group; self.role = role
    }
}

/// A content-addressed multimedia object referenced by a Post. Backed by
/// `FieldValue.blob(id:)`; the bytes live in the content-addressed blob store,
/// never inline in the Matter.
public struct Attachment: Identifiable, Hashable, Sendable {
    public let blobID: String       // content address (FieldValue.blob id)
    public var mediaType: String?   // MIME, honest-by-default: unknown stays nil
    public var id: String { blobID }
    public init(blobID: String, mediaType: String? = nil) {
        self.blobID = blobID; self.mediaType = mediaType
    }

    /// The FieldValue this attachment serialises to inside a post Matter.
    public var fieldValue: FieldValue { .blob(id: blobID) }
}

/// A single message in a Forum, projected from its own post Matter.
public struct Post: Identifiable, Sendable {
    public let id: MatterID         // the post Matter's id
    public let forum: MatterID      // the hub Matter this post belongs to
    public let author: SpaceID      // sealed via Identity (B)
    public var topicArea: String    // the TopicArea key this post lives under
    public var body: String
    public var attachments: [Attachment]
    public var at: Date

    public init(
        id: MatterID,
        forum: MatterID,
        author: SpaceID,
        topicArea: String,
        body: String,
        attachments: [Attachment] = [],
        at: Date
    ) {
        self.id = id; self.forum = forum; self.author = author
        self.topicArea = topicArea; self.body = body
        self.attachments = attachments; self.at = at
    }

    /// Project a Post from its backing Matter. Honest-by-default: a missing field
    /// is left empty/nil rather than fabricated.
    public init(from matter: Matter) throws {
        fatalError("unimplemented")
    }
}

// MARK: - Folded op-groups: poll · question (former Poll #16 / Question #24)
//
// Poll and Question are no longer standalone objects — they are delta op-groups a
// Forum hosts (alongside post / retractPost). The value types below were lifted from
// the former Poll module so the capability lives on its host. The op REDUCERS are the
// next, separate PR; today they are milestone-NotImplemented, as before.

/// The shape of a poll's question, fixing how responses are modelled and tallied.
public enum PollKind: String, Codable, Sendable, CaseIterable {
    case select
    case multiSelect = "multi_select"
    case freeText = "free_text"
    /// True when responses reference a fixed choice set (and thus tally).
    public var isChoiceBased: Bool { self != .freeText }
}

/// One selectable option in a choice-based poll. Stable `id` so a response can
/// reference a choice even if its label is later re-worded.
public struct PollChoice: Identifiable, Codable, Sendable, Equatable, Hashable {
    public let id: UUID
    public var label: String
    public var media: FieldValue?   // expected `.blob(id:)` when present
    public init(id: UUID = UUID(), label: String, media: FieldValue? = nil) {
        self.id = id; self.label = label; self.media = media
    }
}

/// The poll prompt plus its fixed choice set (empty for `.freeText`).
public struct PollPrompt: Codable, Sendable, Equatable {
    public var prompt: String
    public var media: [FieldValue]
    public var options: [PollChoice]
    public init(prompt: String, media: [FieldValue] = [], options: [PollChoice] = []) {
        self.prompt = prompt; self.media = media; self.options = options
    }
}

/// One member's poll response — at most one current response per member (a re-cast
/// replaces, it does not append a second truth).
public struct PollResponse: Identifiable, Codable, Sendable, Equatable {
    public var id: SpaceID { member }
    public let member: SpaceID
    public var value: PollResponseValue
    public var at: Date
    public init(member: SpaceID, value: PollResponseValue, at: Date = Date()) {
        self.member = member; self.value = value; self.at = at
    }
}

/// The typed payload of a poll response; which case is valid is fixed by `PollKind`.
public enum PollResponseValue: Codable, Sendable, Equatable {
    case choice(PollChoice.ID)
    case choices(Set<PollChoice.ID>)
    case text(String, media: [FieldValue] = [])
}

/// The DERIVED poll outcome — a projection produced on demand, never stored. There
/// is intentionally no public initialiser: only a derivation may mint an outcome.
public struct PollOutcome: Sendable, Equatable {
    public let memberCount: Int
    public let responseCount: Int
    public let tally: [PollChoice.ID: Int]
    public let summary: FreeTextSummary?
    init(memberCount: Int, responseCount: Int,
         tally: [PollChoice.ID: Int] = [:], summary: FreeTextSummary? = nil) {
        self.memberCount = memberCount; self.responseCount = responseCount
        self.tally = tally; self.summary = summary
    }
}

/// The aggregation of a `.freeText` poll: a synthesised summary plus the source count.
public struct FreeTextSummary: Sendable, Equatable {
    public let text: String
    public let sourcedFrom: Int
    public init(text: String, sourcedFrom: Int) {
        self.text = text; self.sourcedFrom = sourcedFrom
    }
}

/// A one-shot question posed in a Forum: any member may `ask`; the single-slot
/// answer is set by a `respond` (last-writer-wins). Honest-by-default: an
/// unanswered question's `answer` stays nil, never invented. (Former Question #24.)
public struct ForumQuestion: Identifiable, Sendable, Equatable {
    public let id: MatterID
    public let asker: SpaceID
    public var prompt: String
    public var answer: String?
    public var asked: Date
    public init(id: MatterID, asker: SpaceID, prompt: String, answer: String? = nil, asked: Date) {
        self.id = id; self.asker = asker; self.prompt = prompt
        self.answer = answer; self.asked = asked
    }
}

// MARK: - The Forum object (projects from the hub Matter)

/// A multi-user thread. A projection of the hub Matter plus its post stream.
public struct Forum: PacificObject {
    public static let kind: ObjectKind = .forum

    public let id: MatterID                  // the hub Matter id
    public let space: SpaceID                // owning space (Identity B)
    public var title: String
    /// The keyed set of sub-threads, keyed by `TopicArea.key`.
    public var topicAreas: [String: TopicArea]
    /// The keyed set of members, keyed by the member Group's SpaceID.
    public var members: [SpaceID: Membership]
    /// The MLS group this Forum syncs across (E). Its membership mirrors `members`.
    public var mlsGroup: String

    public init(
        id: MatterID,
        space: SpaceID,
        title: String,
        topicAreas: [String: TopicArea] = [:],
        members: [SpaceID: Membership] = [:],
        mlsGroup: String
    ) {
        self.id = id; self.space = space; self.title = title
        self.topicAreas = topicAreas; self.members = members
        self.mlsGroup = mlsGroup
    }

    /// Re-project the Forum hub from its honest record. Posts are loaded lazily
    /// through `ForumInterface.posts(in:)`, not eagerly here.
    public init(from matter: Matter, in context: any PacificContext) throws {
        fatalError("unimplemented")
    }
}

// MARK: - Module descriptor (the left rail iterates these)

public struct ForumModule: PacificObjectModule {
    public static let kind: ObjectKind = .forum
    public static let blurb = "A multi-user thread — channels, polls, questions, members and shared media."
}

// MARK: - The interface surface (what the right rail & siblings invoke)

/// Operations a Forum exposes. Every mutation writes a Matter (F), seals it via
/// Identity (B) and fans it out over the MLS group (E); reads are gated by
/// Permissions (C) per (member, topic_area).
public protocol ForumInterface: Sendable {

    /// Post a message into a topic_area. Mints a post Matter authored by `author`,
    /// uploads any attachment bytes to the content-addressed blob store first,
    /// seals the Matter and broadcasts it to the Forum's MLS group.
    func post(
        body: String,
        to topicArea: String,
        attachments: [Attachment],
        as author: SpaceID,
        in forum: Forum
    ) async throws -> Post

    /// The posts a `viewer` is permitted to see in a topic_area, newest last.
    /// Scoped by the per-(member, topic_area) Permission gate (C).
    func posts(
        in topicArea: String,
        of forum: Forum,
        as viewer: SpaceID
    ) async throws -> [Post]

    /// Add a sub-thread to the keyed `topic_areas` map. Updates the hub Matter.
    func addTopicArea(_ area: TopicArea, to forum: Forum) async throws -> Forum

    /// Add a Group to the keyed `members` map AND to the MLS group (E). The two
    /// are kept in lock-step — membership in the map without MLS membership (or
    /// vice-versa) is an invariant violation.
    func addMember(_ membership: Membership, to forum: Forum) async throws -> Forum

    /// Remove a Group from the keyed `members` map AND from the MLS group (E),
    /// triggering an MLS key rotation so the removed member cannot read new posts.
    func removeMember(_ group: SpaceID, from forum: Forum) async throws -> Forum

    // — Folded poll op-group (former Poll #16). The outcome is always DERIVED. —

    /// Open a poll in a `topicArea`. Authors the poll's `configure` Delta; returns
    /// the poll's Matter id (a post-like Delta on this Forum).
    func openPoll(
        _ prompt: PollPrompt,
        kind: PollKind,
        in topicArea: String,
        of forum: Forum,
        as author: SpaceID
    ) async throws -> MatterID

    /// Cast or REPLACE the caller's vote (one response per member; commutative LWW).
    func vote(
        _ value: PollResponseValue,
        on poll: MatterID,
        of forum: Forum,
        as member: SpaceID
    ) async throws

    /// DERIVE the outcome for `viewer` — tally for choice polls, summary for free-text.
    /// Always recomputed from the responses the viewer may see; never read from storage.
    func pollOutcome(
        _ poll: MatterID,
        for viewer: SpaceID,
        of forum: Forum,
        summarise: (([PollResponse]) async throws -> FreeTextSummary)?
    ) async throws -> PollOutcome

    /// Freeze a poll (owner-sequenced) — no further votes are counted.
    func closePoll(_ poll: MatterID, of forum: Forum) async throws

    // — Folded question op-group (former Question #24). One-shot ask + single answer. —

    /// Pose a one-shot question in a `topicArea`. Any member may ask.
    func ask(
        _ prompt: String,
        in topicArea: String,
        of forum: Forum,
        as asker: SpaceID
    ) async throws -> ForumQuestion

    /// Set the single-slot answer (commutative, last-writer-wins).
    func respond(
        to question: MatterID,
        answer: String,
        of forum: Forum,
        as member: SpaceID
    ) async throws -> ForumQuestion
}
