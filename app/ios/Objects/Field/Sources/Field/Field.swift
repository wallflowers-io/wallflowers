//  Field.swift
//  The Field object module — the public interface stub.
//
//  A Field is a set of key:index pairs, and the BASE UNIT of Pacific's parametric
//  modelling. It is the smallest addressable thing in a model: a `key` names a
//  slot; the `.index` addresses into a structured source (a row, a cell, a JSON
//  path, a sheet coordinate). Systems are COMPOSED OF Fields; a parametric model
//  is a directed graph of Fields where a Field's value may be a function of
//  other Fields.
//
//  A Field's value comes from one of three places (the `FieldSource`):
//
//    (1) LITERAL    — a value set by hand, stored honestly in the Matter.
//    (2) DEEP-LINK  — a live value pulled/pushed through a System using a scoped,
//                     revocable credential GRANT drawn from the Group level. The
//                     Field never holds the secret; it holds a handle/grant
//                     reference and asks the vault to make the call. An
//                     unresolved live value stays `nil` — never fabricated.
//    (3) PARAMETRIC — a function of other Fields. The expression names its inputs
//                     by Field MatterID; those names are the edges of the
//                     dependency graph, and the value is recomputed when any
//                     input changes. The output is NEVER stored as an independent
//                     truth: it is always re-derivable from inputs + expression.
//
//  Each Field carries its OWN members with per-member visibility/edit Roles —
//  permissioning at field granularity (Permissions C), finer than the object as a
//  whole. This is the interface stub: real, well-named Swift; bodies unimplemented.

import Foundation
import PacificCore

// MARK: - key:index — the addressable cell

/// How a Field addresses INTO a structured source. The `.index` is deliberately a
/// small algebra rather than a bare integer, because real sources are addressed in
/// different ways (a spreadsheet cell, a JSON path, a database row+column, an
/// ordinal position). A Field is identified to its source by `(key, index)`.
public enum FieldIndex: Hashable, Codable, Sendable {
    /// Ordinal position within an ordered source (0-based).
    case ordinal(Int)
    /// A named coordinate within a structured source ("B7", "temperature").
    case named(String)
    /// A path into a tree/document source ("results", "0", "value").
    case path([String])
    /// A 2-D coordinate (row, column) for tabular sources.
    case cell(row: Int, column: Int)
    /// No sub-addressing: the source IS the value (a scalar endpoint).
    case whole
}

/// One key:index pair — the atomic addressing unit a Field is built from. `key`
/// is the human/semantic name of the slot; `index` says where in the source the
/// slot's value actually lives.
public struct KeyIndex: Hashable, Codable, Sendable {
    /// The slot name ("mass", "yield_strength", "row.subtotal").
    public var key: String
    /// Where the slot's value is addressed in the structured source.
    public var index: FieldIndex

    public init(key: String, index: FieldIndex = .whole) {
        self.key = key; self.index = index
    }
}

// MARK: - The deep-link to a networked data source

/// Whether the live link reads from, writes to, or does both with its source.
public enum LinkDirection: String, Codable, Sendable {
    case pull            // read the live value out of the source
    case push           // write this Field's value into the source
    case bidirectional   // both — last-writer semantics resolved by the source
}

/// A deep-link from a Field to a live networked data source, reached THROUGH a
/// System using a scoped credential GRANT from the Group level. This struct holds
/// only the *addressing* and *authorisation reference* — never a secret. The
/// destination value, when fetched, is carried by `FieldValue.link` (the Matter
/// field) for the address and a resolved `FieldValue` for the value; an
/// unresolved value is `nil`.
public struct DataSourceLink: Codable, Sendable, Equatable {
    /// The System object (its `.system` Matter) this link traverses. The System is
    /// what knows how to talk to the external source; the Field rides through it.
    public let system: MatterID
    /// The endpoint within the System the (key, index) addresses into. This is the
    /// `FieldValue.link` URL — the deep link itself.
    public let endpoint: URL
    /// The (key, index) being addressed at that endpoint.
    public let address: KeyIndex
    /// Which Group's vault holds the credential, and which entry. We carry the
    /// vault entry id, NOT the secret — the Group issues a scoped grant on demand.
    public let credentialOwner: SpaceID
    public let credentialEntry: UUID
    /// Read / write / both.
    public let direction: LinkDirection

    public init(
        system: MatterID,
        endpoint: URL,
        address: KeyIndex,
        credentialOwner: SpaceID,
        credentialEntry: UUID,
        direction: LinkDirection = .pull
    ) {
        self.system = system; self.endpoint = endpoint; self.address = address
        self.credentialOwner = credentialOwner; self.credentialEntry = credentialEntry
        self.direction = direction
    }
}

/// The freshness of a live (deep-linked) value. Honest by default: until a fetch
/// succeeds the value is `.unresolved`, never a placeholder dressed up as data.
public enum Liveness: Sendable, Equatable {
    /// Never fetched, or invalidated — the value is genuinely unknown (`nil`).
    case unresolved
    /// Fetched successfully; carries when, so the UI can show staleness honestly.
    case resolved(at: Date)
    /// A fetch was attempted and failed. We surface the failure rather than hide it.
    case failed(at: Date, reason: String)
}

// MARK: - Per-member visibility / edit roles (Permissions C, field granularity)

/// A member's capability ON THIS FIELD specifically. Distinct from a Group's
/// object-wide MembershipRole: the same person may edit one Field in a System and
/// only see the existence of its neighbour. Maps onto Pacific `Disclosure`.
public enum FieldRole: String, Codable, Sendable, CaseIterable, Comparable {
    case none      // not even existence (Disclosure .none)
    case existence  // "this Field exists" — no value (Disclosure .existence)
    case viewer    // may read the resolved value (Disclosure .summary/.full)
    case editor    // may read AND set the value / change the source
    case owner     // editor + may change this Field's membership and links

    public var disclosure: Disclosure {
        switch self {
        case .none:                       return .none
        case .existence:                  return .existence
        case .viewer:                     return .summary
        case .editor, .owner:             return .full
        }
    }
    public var canEdit: Bool { self == .editor || self == .owner }

    public static func < (l: FieldRole, r: FieldRole) -> Bool {
        let order: [FieldRole] = [.none, .existence, .viewer, .editor, .owner]
        return order.firstIndex(of: l)! < order.firstIndex(of: r)!
    }
}

/// One member's standing on this Field. The member is a Group, named by its space.
public struct FieldMember: Codable, Sendable, Equatable {
    /// The member Group's space (Identity B).
    public let space: SpaceID
    /// What that member may see/do on THIS Field.
    public var role: FieldRole

    public init(space: SpaceID, role: FieldRole) {
        self.space = space; self.role = role
    }
}

// MARK: - Parametric: a Field value as a function of other Fields

/// A reference to another Field by its Matter id, optionally addressing a single
/// key:index within it. Each `FieldRef` in an expression is one EDGE of the
/// dependency graph.
public struct FieldRef: Hashable, Codable, Sendable {
    public let field: MatterID
    /// Which key:index of that Field to read; `nil` means the Field's whole value.
    public let address: KeyIndex?
    public init(field: MatterID, address: KeyIndex? = nil) {
        self.field = field; self.address = address
    }
}

/// A parametric expression: the function that derives this Field's value from
/// other Fields. Kept as an explicit tree (not an opaque string) so the
/// dependency graph is statically inspectable — we can extract every input
/// without evaluating, detect cycles, and recompute the minimal cone on change.
public indirect enum Parametric: Codable, Sendable, Equatable {
    /// A constant operand inside an expression.
    case literal(FieldValue)
    /// Read another Field's (resolved) value — an input edge of the graph.
    case ref(FieldRef)
    /// A named n-ary operation over sub-expressions ("add", "mul", "lookup",
    /// "interp", "if"). The operator vocabulary is resolved by the evaluator.
    case op(String, [Parametric])

    /// Every Field this expression depends on, directly — the out-edges of this
    /// node in the dependency graph. Computed without evaluating.
    public var inputs: Set<FieldRef> {
        switch self {
        case .literal:            return []
        case .ref(let r):         return [r]
        case .op(_, let args):    return args.reduce(into: Set<FieldRef>()) { $0.formUnion($1.inputs) }
        }
    }
}

/// Where a Field's value comes from. Exactly one of three sources — a Field is
/// never simultaneously a literal and a live link and a formula.
public enum FieldSource: Sendable {
    /// A value set by hand and stored honestly in the Matter.
    case literal(FieldValue)
    /// A live value reached through a System with a Group-scoped grant. The
    /// resolved value is held separately (and is `nil` until a fetch succeeds).
    case deepLink(DataSourceLink)
    /// A function of other Fields. The output is always re-derived, never stored
    /// as an independent truth.
    case parametric(Parametric)
}

// MARK: - Errors (raised loudly; we never return a fake value)

public enum FieldError: Error, Sendable {
    case notAuthorised(SpaceID)              // member's FieldRole too low for the op
    case sourceUnreachable(MatterID)         // the System could not be reached
    case grantUnavailable(owner: SpaceID)    // Group would not issue a usable grant
    case cycleDetected([MatterID])           // a parametric loop in the graph
    case unresolvedInput(FieldRef)           // an input has no value yet (honest stop)
    case unknownOperator(String)             // expression names an op we cannot evaluate
    case notWritable                         // tried to set a pull-only / formula Field
}

// MARK: - The Field object — a projection of its backing Matter

/// A live Field instance, projected from one `.field` Matter. Value type.
///
/// Invariants the type promises (enforced in the implementation):
///   • exactly one `FieldSource`;
///   • a `.parametric` Field's value is ALWAYS re-derivable — never persisted as
///     an independent truth;
///   • a `.deepLink` Field's `value` is `nil` whenever `liveness` is `.unresolved`
///     or `.failed` — an unresolved networked value is never fabricated;
///   • the dependency graph is acyclic.
public struct Field: PacificObject {
    public static let kind: ObjectKind = .field

    public let id: MatterID
    /// The owning space (Identity B) — typically the System/Project that holds it.
    public let space: SpaceID

    /// The semantic key:index this Field occupies. The `key` is its name in the
    /// model; the `index` is how it addresses into its source (if structured).
    public var keyIndex: KeyIndex

    /// Where this Field's value comes from: literal, deep-link, or parametric.
    public var source: FieldSource

    /// The last-known resolved value. For a literal it is the literal; for a
    /// deep-link it is the last successful fetch (or `nil`); for a parametric it
    /// is the last computed result (a cache only — always re-derivable). NEVER a
    /// fabricated stand-in: `nil` means genuinely unknown.
    public private(set) var value: FieldValue?

    /// Freshness of a deep-linked value. `.unresolved` while `value == nil`.
    public private(set) var liveness: Liveness

    /// Per-member visibility/edit standing on THIS Field (Permissions C at field
    /// granularity). Absent members fall back to the containing object's gate.
    public private(set) var members: [FieldMember]

    public init(from matter: Matter, in context: any PacificContext) throws {
        // TODO: decode keyIndex / source / members from matter.fields; for a
        // deep-link, read FieldValue.link as the endpoint; leave `value` nil and
        // `liveness` .unresolved until a resolve is performed.
        fatalError("unimplemented")
    }

    /// Direct memberwise init (used by tests and by the projection above once built).
    public init(
        id: MatterID,
        space: SpaceID,
        keyIndex: KeyIndex,
        source: FieldSource,
        value: FieldValue? = nil,
        liveness: Liveness = .unresolved,
        members: [FieldMember] = []
    ) {
        self.id = id; self.space = space; self.keyIndex = keyIndex
        self.source = source; self.value = value; self.liveness = liveness
        self.members = members
    }
}

// MARK: - The interface surface siblings and the right rail invoke

/// Everything other modules (System, Project, Forum, Poll) and the three UI rails
/// call on a Field. The defining operations are: resolve a live value, set/edit a
/// value under field-level permissions, and declare/inspect a parametric
/// dependency so a model graph can be built and recomputed.
public protocol FieldInterface: Sendable {

    // — Read / resolve the value —

    /// The current value WITHOUT touching the network: the literal, the last
    /// successful fetch, or the last computed result. `nil` if genuinely unknown.
    /// Gated by `viewer`'s FieldRole — returns `nil` below `.viewer`.
    func currentValue(of field: Field, as viewer: SpaceID) async -> FieldValue?

    /// Resolve a deep-linked Field NOW: ask its System (through a Group-scoped
    /// grant) for the live value and update `value`/`liveness`. On failure the
    /// value stays `nil` and `liveness` becomes `.failed` — never fabricated.
    /// Throws `sourceUnreachable` / `grantUnavailable` rather than returning junk.
    func resolve(_ field: Field, as viewer: SpaceID) async throws -> Field

    // — Set / edit the value —

    /// Set a literal value, or PUSH a value out through a deep-link's System.
    /// Requires the editor's FieldRole to permit editing; throws `notAuthorised`
    /// otherwise, and `notWritable` for a pull-only or parametric Field.
    func set(_ value: FieldValue, on field: Field, as editor: SpaceID) async throws -> Field

    // — Per-member permissioning (field granularity) —

    /// Set/raise/lower a member's visibility/edit role on THIS Field. Only an
    /// `.owner` may call; mirrors the change into Permissions C (Disclosure).
    func setRole(_ role: FieldRole, for member: SpaceID, on field: Field, by owner: SpaceID) async throws -> Field

    // — Parametric composition (the dependency graph) —

    /// Declare this Field's value to be a function of other Fields. Validates the
    /// graph stays acyclic (throws `cycleDetected`) before committing. Switches the
    /// Field's source to `.parametric`.
    func defineExpression(_ expr: Parametric, on field: Field, as editor: SpaceID) async throws -> Field

    /// The direct input Fields this Field depends on — the out-edges in the graph.
    /// Empty for literal and deep-link Fields.
    func dependencies(of field: Field) -> Set<FieldRef>

    /// Recompute a parametric Field from its inputs' current values. If any input
    /// is unresolved it throws `unresolvedInput` (an honest stop) rather than
    /// substituting a default. Returns the Field with a freshly-derived `value`.
    func recompute(_ field: Field, as viewer: SpaceID) async throws -> Field

    /// The Fields whose values would change if `field` changed — the downstream
    /// recomputation cone (the in-edges, transitively). Lets a model push minimal
    /// updates when one Field moves.
    func dependents(of field: Field, in context: any PacificContext) async -> Set<MatterID>
}

// MARK: - Module descriptor (the left rail iterates these)

public struct FieldModule: PacificObjectModule {
    public static let kind: ObjectKind = .field
    public static let blurb: String =
        "A key:index cell — the base unit of parametric modelling; links live data and depends on other Fields."
}
