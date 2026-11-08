//  System.swift
//  Pacific object module: System — an interactable external system, API or
//  software package (Onshape, Gmail, a watched file …).
//
//  A System is a NORMAL Pacific object: it is stored in SQLite on device, its
//  membership list is an MLS group, and its edits sync to that group over MLS —
//  exactly like Project / Poll / Forum / Field / Topic. What is distinctive is
//  that a System is COMPOSED OF Fields and owns the *external* edge (HTTPS,
//  IMAP, a watched file) on top of the shared peer edge (E/MLS). Its definition
//  (declared fields + connector + operations) is authored by the user when they
//  create the System and is stored WITH the object — it is NOT registered as a
//  Dictionary schema. (A System may carry a `MatchingTag` so peers can recognise
//  the same kind of System across spaces — optional interop, not where the
//  object lives.) It authenticates via a scoped, revocable credential HANDLE
//  drawn from a Group; it never sees the raw secret.
//
//  This is an interface stub. Bodies are `fatalError("unimplemented")`.

import Foundation
import PacificCore

// MARK: - The object

/// A live System instance — a typed projection of its backing `.system` Matter.
/// The Matter stores the System's `definition` inline plus its binding (the
/// Group it draws credentials from and the granted handle). Stored in SQLite;
/// edits sync to the System's own MLS group.
public struct System: PacificObject {
    public static let kind: ObjectKind = .system

    public let id: MatterID
    public let space: SpaceID

    /// The user-authored definition (declared fields, connector, operations),
    /// stored with this object. Its optional `tag` is a cross-space interop key.
    public let definition: SystemDefinition

    /// Human label for this System (e.g. "Acme Onshape — prod").
    public let label: String

    /// The Group whose credential vault this System authenticates against.
    /// `nil` until the user binds one — honest-by-default, never faked.
    public let credentialSource: MatterID?

    /// The scoped, revocable handle the Group granted. Opaque: it references a
    /// secret in the Group vault; it is NOT the secret. `nil` until granted.
    public let credential: CredentialHandle?

    /// The Field objects (by Matter id) this System is composed of.
    public let fields: [MatterID]

    public init(from matter: Matter, in context: any PacificContext) throws {
        // TODO: decode definition / label / credentialSource / credential /
        // fields out of `matter.fields`; leave unknowns nil, never invent them.
        fatalError("unimplemented")
    }
}

// MARK: - The module descriptor

public struct SystemModule: PacificObjectModule {
    public static let kind: ObjectKind = .system
    public static let blurb =
        "An external system, API or tool — composed of Fields, invoked through scoped Group credentials."
}

// MARK: - The definition (authored by the user; stored with the System)

/// A user-authored System DEFINITION: the declared shape of the System — its
/// fields, connector, credential request and operations. It is created when the
/// user makes a System and stored WITH the object (in the `.system` Matter),
/// like any other object's content — NOT registered as a Dictionary schema.
///
/// The optional `tag` lets peers recognise the same *kind* of System across
/// spaces (shared vocabulary); it is interop, not identity or storage.
public struct SystemDefinition: Codable, Sendable {
    /// Display name chosen by the author (e.g. "Onshape").
    public var name: String
    /// Optional cross-space interop key (shared vocabulary). `nil` = local only.
    public var tag: MatchingTag?
    /// Who authored the definition (Identity B).
    public var author: SpaceID
    /// The declared fields the System is composed of.
    public var fields: [SystemField]
    /// How the System reaches the external world.
    public var connector: Connector
    /// The credential this definition requires a Group to satisfy.
    public var credentialRequest: CredentialRequest
    /// The operations the System exposes (read / write / call).
    public var operations: [Operation]

    public init(
        name: String,
        tag: MatchingTag? = nil,
        author: SpaceID,
        fields: [SystemField],
        connector: Connector,
        credentialRequest: CredentialRequest,
        operations: [Operation]
    ) {
        self.name = name; self.tag = tag; self.author = author
        self.fields = fields; self.connector = connector
        self.credentialRequest = credentialRequest; self.operations = operations
    }
}

/// One field the System is composed of — the declared half of a Field object.
/// `key`/`locator` deep-links into the external source via the connector.
public struct SystemField: Codable, Sendable {
    public var key: String                 // logical name within the System
    public var hint: FieldKindHint         // text / number / bool / ref / link / blob
    public var locator: String             // connector-relative path/selector/index
    public var writable: Bool              // can an Operation write back through it?
    public init(key: String, hint: FieldKindHint, locator: String, writable: Bool) {
        self.key = key; self.hint = hint; self.locator = locator; self.writable = writable
    }
}

// MARK: - The connector (external-edge endpoint abstraction)

/// How a System reaches its external world. The connector is transport for the
/// *external* edge (HTTPS to Onshape, IMAP for Gmail, a watched file) — distinct
/// from Pacific's E/MLS, which is the *peer* edge.
public struct Connector: Codable, Sendable {
    public enum Kind: String, Codable, Sendable {
        case http          // REST / GraphQL over HTTPS
        case imap          // mailbox-style (Gmail)
        case file          // a watched file / document on disk or in a vault
    }
    public var kind: Kind
    public var endpoint: URL?              // base URL; nil for `file`
    public var auth: AuthScheme           // how the credential is presented
    public init(kind: Kind, endpoint: URL?, auth: AuthScheme) {
        self.kind = kind; self.endpoint = endpoint; self.auth = auth
    }
}

/// How the connector presents the Group-held credential to the external system.
/// The System knows the *scheme*; the Group performs the actual signing/binding
/// so the raw secret never crosses the System boundary.
public enum AuthScheme: String, Codable, Sendable {
    case none
    case apiKeyHeader      // key injected into a header by the Group, not the System
    case oauthBearer       // bearer token minted/refreshed by the Group
    case basic             // user:pass held by the Group
    case clientCert        // mutual-TLS cert held by the Group
}

// MARK: - Operations / actions

/// An operation the System exposes — the unit the right rail and a Project's
/// CLA invoke. Read / write / call against the connector and the System's Fields.
public struct Operation: Codable, Sendable, Identifiable {
    public enum Mode: String, Codable, Sendable {
        case read          // pull current values into the System's Fields
        case write         // push values out through writable Fields
        case call          // invoke a side-effecting action (e.g. "regenerate")
    }
    public var id: String                  // stable name within the definition
    public var mode: Mode
    public var summary: String             // one line for the right rail
    /// Fields (by `SystemField.key`) this operation reads from / writes to.
    public var touches: [String]
    /// Minimum disclosure the caller must hold on the System to invoke it.
    /// TODO(membership=access): this becomes a membership/role check when the
    /// three-level `Disclosure` model is removed repo-wide — access is group
    /// membership, not a disclosure tier.
    public var requires: Disclosure
    public init(id: String, mode: Mode, summary: String, touches: [String], requires: Disclosure) {
        self.id = id; self.mode = mode; self.summary = summary
        self.touches = touches; self.requires = requires
    }
}

/// A right-rail / CLA invocation of an `Operation` against a live `System`.
public struct SystemAction: Sendable {
    public var system: MatterID            // the System instance
    public var operationID: String         // Operation.id to run
    public var caller: SpaceID             // who is invoking
    public var arguments: [String: FieldValue]
    public init(system: MatterID, operationID: String, caller: SpaceID, arguments: [String: FieldValue] = [:]) {
        self.system = system; self.operationID = operationID
        self.caller = caller; self.arguments = arguments
    }
}

/// The result of running a `SystemAction`. Honest-by-default: a value the
/// external system did not return is left out, never fabricated.
public struct ActionResult: Sendable {
    public var values: [String: FieldValue]   // updated Field values, by key
    public var provenance: Provenance         // who/when, appended to the Matter story
    public init(values: [String: FieldValue], provenance: Provenance) {
        self.values = values; self.provenance = provenance
    }
}

// MARK: - Credentials (requested from a Group; never the raw secret)

/// What a `SystemDefinition` asks a Group to satisfy. The Group's vault (B)
/// decides whether it holds a matching secret and at what scope.
public struct CredentialRequest: Codable, Sendable {
    public var purpose: String             // human reason, shown at grant time
    public var scheme: AuthScheme          // the shape of secret required
    public var scopes: [String]            // least-privilege scopes requested
    public init(purpose: String, scheme: AuthScheme, scopes: [String]) {
        self.purpose = purpose; self.scheme = scheme; self.scopes = scopes
    }
}

/// An opaque, scoped, revocable reference a Group granted in answer to a
/// `CredentialRequest`. It points at a secret in the Group vault; it is NOT the
/// secret. The Group performs auth on the System's behalf and can revoke
/// instantly. The System can present and use it but can never read it back.
public struct CredentialHandle: Codable, Sendable {
    public let id: UUID                    // vault reference, not the secret
    public let group: MatterID             // the granting Group
    public let scopes: [String]            // scopes actually granted (⊆ requested)
    public let expires: Date?              // nil = until revoked
    public init(id: UUID = UUID(), group: MatterID, scopes: [String], expires: Date?) {
        self.id = id; self.group = group; self.scopes = scopes; self.expires = expires
    }
}

// MARK: - The interface surface

/// Everything the right rail, a Project's CLA, and sibling objects invoke on a
/// System: authoring a definition, binding a Group credential, and running
/// operations — without ever surfacing the secret. Creating the System object
/// itself is the generic object-creation path (a new `.system` Matter synced to
/// its MLS group); there is no Dictionary registration step.
public protocol SystemInterface: Sendable {

    /// Author/validate a System definition (pure): no duplicate field keys,
    /// operations only touch declared fields, writes only through writable ones.
    func define(_ definition: SystemDefinition) throws -> SystemDefinition

    /// Ask `group` to satisfy the definition's `CredentialRequest`. Returns an
    /// opaque, scoped `CredentialHandle` — the System never sees the secret.
    func requestCredential(
        from group: MatterID,
        in context: any PacificContext
    ) async throws -> CredentialHandle

    /// Invoke an exposed `Operation` against this live System, authenticated via
    /// the Group-held handle, gated by the caller's access to the System's group.
    func invoke(_ action: SystemAction, in context: any PacificContext) async throws -> ActionResult
}
