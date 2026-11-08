//  Project.swift
//  The Project object module — a project-management hub with a Core Loop Agent.
//
//  A Project is itself a Matter (kind `.project`). It owns task Matters and
//  references them by `FieldValue.ref`. It carries optional additional members,
//  each a Group bound to a ROLE. Its defining feature is the Core Loop Agent
//  (CLA): an agent hook that fires whenever any of the Project's *subscriptions*
//  registers a change. A subscription targets a Topic (deep-research object) or a
//  System (connector). A change event arrives either inbound over Network E
//  (an Envelope from the subscribed space) or as a local Matter/Search update.
//
//  On a change the CLA produces RECOMMENDATIONS (always safe; never act on their
//  own) and, within an explicit autonomy boundary, may take AUTONOMOUS ACTIONS.
//  The boundary is honest and narrow: every autonomous action is gated by
//  Permissions (C) and signed under the Project's own Identity (B). Anything the
//  CLA may not do unattended degrades to a recommendation — it never silently
//  escalates. See PIE-CI.md for the full strategy.
//
//  DESIGN DECISIONS — Pacific v1 UI work, 2026-06-27 (see Objects/PRIMITIVE-DECISIONS.txt):
//   • ACTION = an UNRESOLVED Event + a Group ASSIGNEE. The "task" is NOT its own
//     primitive — it folds into a first-class EVENT (always a window). A `taskRefs`
//     entry is one such Event; "~/waiting" is DERIVED (assignee ≠ you).
//   • TASKS ARE HOST-AGNOSTIC: an action/task may belong to ANY Group or Place, not
//     only a Project. `taskRefs` here is the Project-scoped view of that edge.
//   • CONTAINMENT: "Systems/Forums/Polls/Fields belong to a Project OR a Group, and
//     Projects contain sub-Projects (topic areas) recursively" needs ONE card-to-card
//     containment edge — a new structural primitive, still UNSPECIFIED.
//   • A PROJECT IS A GROUP-CARD THAT CARRIES A CLA: a "light" entity card can be
//     promoted to a managed Project; the CLA + autonomy below are what differ.

import Foundation
import PacificCore

// MARK: - Member roles

/// A member's role within a Project. Members are Groups (a `SpaceID`). The role
/// fixes what that member may see and, crucially, how far the CLA may act *on
/// that member's behalf* without asking. Roles are advisory to the UI but
/// authoritative for the CLA's autonomy check (see `AutonomyPolicy`).
public enum Role: String, Codable, Sendable, CaseIterable, Comparable {
    /// Read the Project and its task Matters; receive recommendations.
    case viewer
    /// Act on task Matters; receive and accept/decline recommendations.
    case contributor
    /// As contributor, plus may change subscriptions, member roles, and the
    /// CLA's autonomy policy.
    case maintainer
    /// The owning space. May do anything a maintainer may, and is the Identity
    /// (B) under which the Project's own autonomous actions are signed.
    case owner

    public static func < (l: Role, r: Role) -> Bool { l.rank < r.rank }
    private var rank: Int {
        switch self {
        case .viewer: return 0
        case .contributor: return 1
        case .maintainer: return 2
        case .owner: return 3
        }
    }
}

/// A single member binding: a Group, in a Role.
public struct Membership: Codable, Sendable, Hashable {
    public var group: SpaceID      // the member Group's Identity space (B)
    public var role: Role
    public init(group: SpaceID, role: Role) { self.group = group; self.role = role }
}

// MARK: - Subscriptions

/// What a Project may subscribe to. Exactly the two sources the CLA watches.
public enum SubscriptionTarget: Codable, Sendable, Hashable {
    /// A Topic object (agent-managed deep-research). Its Matter id.
    case topic(MatterID)
    /// A System connector. Its Matter id.
    case system(MatterID)
}

/// A standing interest in a target. When the target changes, a `ChangeEvent`
/// is delivered to the Project and the CLA is triggered.
///
/// A subscription is *not* a copy of the target's data — it is an edge plus the
/// disclosure level we hold on the target, recorded so the CLA can reason about
/// how much it actually saw. Honest-by-default: if we only hold `.existence`,
/// the event carries no contents and the CLA may only recommend "go look".
public struct Subscription: Codable, Sendable, Hashable, Identifiable {
    public var id: UUID
    public var target: SubscriptionTarget
    /// The disclosure (C) the Project currently holds on the target's Matter.
    /// Determines how much of a change event the CLA can see.
    public var heldDisclosure: Disclosure
    /// Last change we processed, for de-duplication / "what's new since".
    public var lastSeen: Date?
    public init(
        id: UUID = UUID(),
        target: SubscriptionTarget,
        heldDisclosure: Disclosure = .existence,
        lastSeen: Date? = nil
    ) {
        self.id = id; self.target = target
        self.heldDisclosure = heldDisclosure; self.lastSeen = lastSeen
    }
}

/// How a change reached us. The CLA trusts neither path more than its
/// disclosure — `via` is provenance, not authorisation.
public enum ChangeOrigin: Codable, Sendable, Hashable {
    /// An authenticated Envelope arrived over Network E from the target's space.
    case network(envelope: UUID)
    /// A local Matter (F) was appended/updated for the subscribed target.
    case matter(MatterID)
    /// The on-device Search index (A) surfaced new relevance for the target.
    case search
}

/// The event handed to the CLA when a subscription fires. Carries only what our
/// `heldDisclosure` permits — never more.
public struct ChangeEvent: Sendable {
    public let subscription: Subscription
    public let origin: ChangeOrigin
    public let at: Date
    /// The disclosure actually applied to this event's contents.
    public let disclosure: Disclosure
    /// A short, permission-clamped summary of what changed (nil when we hold
    /// only `.existence` — honest: we genuinely do not know the contents).
    public let summary: String?
    public init(
        subscription: Subscription,
        origin: ChangeOrigin,
        at: Date,
        disclosure: Disclosure,
        summary: String?
    ) {
        self.subscription = subscription; self.origin = origin
        self.at = at; self.disclosure = disclosure; self.summary = summary
    }
}

// MARK: - The CLA's two output surfaces

/// A proposed action the CLA surfaces for a human to accept or decline. Always
/// safe to ignore; never executed on its own.
public struct Recommendation: Identifiable, Sendable {
    public let id: UUID
    /// The event that prompted it.
    public let promptedBy: ChangeEvent
    /// One-line, human-readable proposal shown in the right rail.
    public let proposal: String
    /// The action it would take if accepted. Executing it runs the same
    /// `AutonomyPolicy` gate — acceptance is a human's authorisation, not a
    /// bypass.
    public let proposed: Action
    /// Why the CLA could not take this unattended (nil if it merely chose to ask).
    public let blockedReason: AutonomyDecision.Denial?
    public init(
        id: UUID = UUID(),
        promptedBy: ChangeEvent,
        proposal: String,
        proposed: Action,
        blockedReason: AutonomyDecision.Denial? = nil
    ) {
        self.id = id; self.promptedBy = promptedBy
        self.proposal = proposal; self.proposed = proposed
        self.blockedReason = blockedReason
    }
}

/// A concrete, executable effect. The CLA may *take* an action autonomously only
/// when `AutonomyPolicy` permits it for the action's kind; otherwise it must be
/// wrapped in a `Recommendation`.
public struct Action: Sendable, Identifiable {
    public let id: UUID
    public var kind: Kind
    public init(id: UUID = UUID(), kind: Kind) { self.id = id; self.kind = kind }

    /// The closed set of effects a Project's CLA can have. Deliberately small
    /// and explicit so the autonomy boundary is auditable.
    public enum Kind: Sendable {
        /// Create a task Matter under this Project (appended to `taskRefs`).
        case createTask(title: String, status: MatterStatus)
        /// Move an owned task Matter's status.
        case setTaskStatus(MatterID, MatterStatus)
        /// Set a field on an owned task Matter.
        case setTaskField(MatterID, key: String, value: FieldValue)
        /// Adjust a subscription (e.g. raise/lower interest, mark seen).
        case touchSubscription(Subscription.ID)
        /// Send a sealed Envelope to a member or peer (e.g. notify, ask a System).
        /// The most powerful, outward-facing effect — gated hardest.
        case dispatch(to: SpaceID, note: String)
    }
}

// MARK: - The autonomy boundary (honest, explicit, gated by C and B)

/// The policy that decides, per action kind, whether the CLA may act unattended.
/// This is the single source of truth for the autonomy boundary. It does not
/// itself touch Permissions/Identity — it states *intent*; the runtime then
/// additionally enforces C and B (see `AutonomyDecision`).
public struct AutonomyPolicy: Codable, Sendable {
    /// Action kinds the CLA may take without asking, by tag. A kind absent here
    /// is recommend-only. Default policy below is deliberately conservative.
    public enum ActionTag: String, Codable, Sendable, CaseIterable {
        case createTask, setTaskStatus, setTaskField, touchSubscription, dispatch
    }
    /// The unattended-allowed set. Anything not in it degrades to a recommendation.
    public var unattended: Set<ActionTag>
    /// Hard ceiling: outward-facing `dispatch` may never be unattended unless this
    /// is explicitly true AND the runtime gate also passes. Two locks, not one.
    public var allowOutboundDispatch: Bool

    public init(unattended: Set<ActionTag> = [.touchSubscription],
                allowOutboundDispatch: Bool = false) {
        self.unattended = unattended
        self.allowOutboundDispatch = allowOutboundDispatch
    }

    /// Conservative default: the CLA may keep its own subscriptions tidy
    /// unattended, and nothing else. Everything substantive is recommend-only
    /// until a maintainer widens the policy.
    public static let conservative = AutonomyPolicy()
}

/// The outcome of asking "may the CLA take this action unattended, right now?".
/// Combines the static `AutonomyPolicy` with the live Permission (C) and
/// Identity (B) checks. Honest: a denial says exactly why.
public enum AutonomyDecision: Sendable {
    case allow
    case deny(Denial)

    public enum Denial: String, Sendable, Codable {
        case policyRecommendOnly      // AutonomyPolicy does not allow this kind unattended
        case outboundDispatchLocked   // dispatch attempted but allowOutboundDispatch == false
        case insufficientDisclosure   // we don't hold enough C on the target/Matter to act safely
        case notSignableIdentity      // the Project has no owner Identity to sign the effect under
    }
}

// MARK: - The Core Loop Agent

/// The agent hook that defines a Project. The runtime drives it: it subscribes,
/// receives `ChangeEvent`s, and lets the CLA respond. Two response surfaces —
/// recommend (always) and act (only within the autonomy boundary).
///
/// Implementations are out of scope for this stub; this is the contract the
/// right rail and the runtime invoke.
public protocol CoreLoopAgent: Sendable {
    /// The trigger. Called once per change on a subscription. The CLA returns its
    /// recommendations; any actions it took autonomously are reported via the
    /// returned outcome (already gated and executed by the runtime through `act`).
    func onChange(_ event: ChangeEvent, in project: Project) async -> CoreLoopOutcome

    /// Decide, for a candidate action, whether it may run unattended now —
    /// folding the static `AutonomyPolicy` together with live C and B checks.
    /// The runtime calls this before executing anything the CLA proposes to do
    /// itself, and again when a human accepts a `Recommendation`.
    func authorise(_ action: Action, in project: Project) async -> AutonomyDecision
}

/// What one CLA invocation produced.
public struct CoreLoopOutcome: Sendable {
    /// Proposals for a human (right rail). Safe; inert until accepted.
    public var recommendations: [Recommendation]
    /// Actions the CLA actually took unattended (each already passed `authorise`).
    public var actionsTaken: [Action]
    public init(recommendations: [Recommendation] = [], actionsTaken: [Action] = []) {
        self.recommendations = recommendations
        self.actionsTaken = actionsTaken
    }
}

// MARK: - The Project object

/// A Project: a projection of its backing Matter (kind `.project`) plus its
/// members, subscriptions, task references, and autonomy policy. The CLA is held
/// by the runtime, not the value — the value is the honest record's projection.
public struct Project: PacificObject {
    public static let kind: ObjectKind = .project

    public let id: MatterID
    public let space: SpaceID                 // owning space (Identity B)
    public var title: String
    public var status: MatterStatus

    /// Optional additional members, each a Group in a Role. The owner is the
    /// space itself; this list is *additional* members.
    public var members: [Membership]

    /// Standing subscriptions whose changes trigger the CLA.
    public var subscriptions: [Subscription]

    /// The task Matters this Project owns/manages, referenced (not embedded) by
    /// `FieldValue.ref`. The Project is the honest source for *which* tasks it
    /// owns; each task Matter is the honest source for its own contents.
    public var taskRefs: [MatterID]

    /// The autonomy boundary for this Project's CLA.
    public var autonomy: AutonomyPolicy

    public init(
        id: MatterID,
        space: SpaceID,
        title: String,
        status: MatterStatus = .doing,
        members: [Membership] = [],
        subscriptions: [Subscription] = [],
        taskRefs: [MatterID] = [],
        autonomy: AutonomyPolicy = .conservative
    ) {
        self.id = id; self.space = space; self.title = title; self.status = status
        self.members = members; self.subscriptions = subscriptions
        self.taskRefs = taskRefs; self.autonomy = autonomy
    }

    /// Re-project a Project from its honest Matter. Keys are documented in
    /// PIE-CI.md (§ Implementation). Honest-by-default: an absent field stays at
    /// its empty/default value, never fabricated.
    public init(from matter: Matter, in context: any PacificContext) throws {
        // TODO: decode title/status/members/subscriptions/taskRefs/autonomy from
        // matter.fields; resolve `taskRefs` as the FieldValue.ref edges and the
        // subscription targets as topic/system refs.
        fatalError("unimplemented")
    }
}

// MARK: - The interface surface (right rail + CLA + siblings)

/// The operations the centre/right rail and sibling objects invoke on a Project.
/// This is the public surface PIE-CI.md (§ Interface) describes.
public protocol ProjectInterface: Sendable {
    // — Membership (maintainer+) —
    func addMember(_ member: Membership) async throws
    func setRole(of group: SpaceID, to role: Role) async throws
    func removeMember(_ group: SpaceID) async throws

    // — Subscriptions (maintainer+); a Topic/System edge plus its held disclosure —
    func subscribe(to target: SubscriptionTarget) async throws -> Subscription
    func unsubscribe(_ id: Subscription.ID) async throws

    // — Task ownership; tasks are Matters referenced by FieldValue.ref —
    func createTask(title: String, status: MatterStatus) async throws -> MatterID
    func tasks(for viewer: SpaceID) async throws -> [Matter]

    // — The CLA surface the right rail drives —
    /// Currently outstanding recommendations awaiting a human decision.
    func pendingRecommendations() async -> [Recommendation]
    /// A human accepts a recommendation; the runtime re-runs `authorise` then
    /// executes. Acceptance authorises *this* action, not a policy change.
    func accept(_ recommendation: Recommendation.ID) async throws
    /// A human declines; the recommendation is dropped (recorded in provenance).
    func decline(_ recommendation: Recommendation.ID) async throws
    /// Adjust the autonomy boundary (maintainer+). Widening it is itself an event
    /// worth recording in the Matter's history.
    func setAutonomy(_ policy: AutonomyPolicy) async throws
}

// MARK: - Module descriptor (the left rail iterates these)

public struct ProjectModule: PacificObjectModule {
    public static let kind: ObjectKind = .project
    public static let blurb =
        "A project hub with a Core Loop Agent that watches your Topics and Systems, recommends, and acts within bounds."
    public init() {}
}
