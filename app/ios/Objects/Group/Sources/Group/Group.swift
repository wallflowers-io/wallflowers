//  Group.swift
//  The Group object module — the public interface stub.
//
//  A Group is a user, a group of users, a person or a group of people, ON or
//  OFF the platform. Selecting a Group in the left rail shows detail on that
//  person / group / organisation. Two defining responsibilities:
//
//    (1) MEMBERSHIP. A Group is the unit of membership everywhere else in Pacific
//        — Project / Forum / Poll / Field members are all Groups — and it maps
//        to a Pacific Identity SpaceID (an individual = a person space; a group
//        acting as one = a team space).
//
//    (2) CREDENTIALS. Credential management lives at the Group level. A Group
//        holds the secrets (API keys, OAuth tokens) that System and Field
//        objects use to reach external data sources. The secret NEVER leaves
//        the vault: siblings receive a scoped, revocable *grant*, not the raw
//        material (Permissions C).
//
//  This is the interface stub: real, well-named Swift; bodies are unimplemented.

import Foundation
import PacificCore

// MARK: - On-/off-platform state

/// Whether a Group is reachable as a live peer space, or is only a contact we
/// know of but cannot yet sync with. Honest by default: an off-platform Group
/// still exists as a first-class member, it simply cannot receive MLS envelopes.
public enum PlatformPresence: Sendable, Equatable {
    /// A live peer: backed by a real Pacific SpaceID we can seal envelopes to.
    case onPlatform(SpaceID)
    /// A known contact with no syncable space yet (a name, maybe an invite hint).
    /// `inviteHint` is whatever we have to reach them out-of-band; `nil` if unknown.
    case offPlatform(inviteHint: String?)

    /// The peer space, when one exists. `nil` for off-platform contacts —
    /// never fabricated.
    public var space: SpaceID? {
        if case let .onPlatform(s) = self { return s }
        return nil
    }
}

// MARK: - Whether the Group is one person or a team acting as one

// DESIGN DECISIONS — Pacific v1 UI work, 2026-06-27 (see Objects/PRIMITIVE-DECISIONS.txt):
//  • PLACE is its OWN first-class entity, NOT a GroupShape case. A place (a venue,
//    an address, a city) has no peer identity / no vault — do not model it as a Group.
//  • PERSON stays `GroupShape.individual`: there is deliberately NO separate
//    "Person" primitive (a separate kind would fork the person representation).
//  • CANONICAL RESOLUTION: the deep-link / outline model assumes a recurring entity
//    resolves to ONE Group ("referenced from" backlinks). The de-dup + off-platform
//    → on-platform MERGE mechanism is still OPEN — see this package's PIE-CI.md
//    ("De-duplicating Groups" / "off-platform → on-platform reconciliation").
public enum GroupShape: String, Codable, Sendable {
    /// An individual person — maps to a person space.
    case individual
    /// A group of people acting as a single member — maps to a team space.
    case team
    /// An organisation (a team with an external/legal identity).
    case organisation
}

// MARK: - The role a Group plays in a context it is a member of

/// The capacity in which a Group participates in a sibling object. The same
/// Group plays different roles in different Projects / Forums / Fields; the role
/// is held by the *containing* object, not by the Group — this is the shared
/// vocabulary both sides name it with.
public enum MembershipRole: String, Codable, Sendable, CaseIterable {
    case owner       // can change membership and credentials
    case admin       // can change membership
    case member      // ordinary participant
    case viewer      // read-only (maps onto Disclosure .summary / .full)
    case guest       // off-platform or provisional; minimal disclosure
}

// MARK: - The credential vault (encapsulated; secrets never cross the boundary)

/// The kind of secret a credential carries. The shape is public; the secret
/// material behind it is NOT — it lives only inside the vault.
public enum CredentialKind: String, Codable, Sendable {
    case apiKey
    case oauthToken
    case bearerToken
    case basicAuth
    case sshKey
    case custom
}

/// An opaque, content-free *handle* to a secret in a Group's vault. This is the
/// only thing that ever crosses the module boundary: callers pass it around,
/// store it, and reference it — but cannot read the secret from it. Resolving a
/// handle to usable access goes through `ScopedCredentialGrant` (below), and
/// even then the plaintext is never returned to the caller.
public struct CredentialHandle: Hashable, Codable, Sendable {
    /// Stable identifier of the vault entry. Not the secret; a pointer to it.
    public let id: UUID
    /// Which Group's vault this handle belongs to (its owning space).
    public let owner: SpaceID
    /// Declared kind, so a System/Field can check shape before requesting use.
    public let kind: CredentialKind
    /// Human label ("Onshape prod key"). Never the secret value.
    public let label: String

    public init(id: UUID = UUID(), owner: SpaceID, kind: CredentialKind, label: String) {
        self.id = id; self.owner = owner; self.kind = kind; self.label = label
    }
}

/// The bounds a grant is constrained to: which sibling may use it, for what, and
/// until when. Authorisation, not the secret.
public struct CredentialScope: Codable, Sendable, Equatable {
    /// The sibling object the grant is issued *to* (a System's or Field's Matter).
    public let grantee: MatterID
    /// Coarse permission verbs the holder may exercise (e.g. "read", "write").
    public let allowedOperations: Set<String>
    /// Hard expiry. `nil` means "until explicitly revoked" — never silently forever
    /// in practice, but we do not fabricate an expiry we were not given.
    public let expiry: Date?

    public init(grantee: MatterID, allowedOperations: Set<String>, expiry: Date? = nil) {
        self.grantee = grantee; self.allowedOperations = allowedOperations; self.expiry = expiry
    }
}

/// A live, revocable authorisation to *use* a credential — without ever seeing
/// it. The grantee performs an external call by handing the `token` back to the
/// vault, which injects the real secret at the network edge and returns the
/// result. The grantee holds proof-of-authorisation, not the secret.
public struct ScopedCredentialGrant: Identifiable, Sendable {
    public let id: UUID
    public let handle: CredentialHandle
    public let scope: CredentialScope
    /// Opaque capability token. Presenting it lets the vault perform an action
    /// *on the grantee's behalf*; it is not the secret and cannot be reversed
    /// into one. Revoked instantly via `GroupInterface.revoke`.
    public let token: Data

    public init(id: UUID = UUID(), handle: CredentialHandle, scope: CredentialScope, token: Data) {
        self.id = id; self.handle = handle; self.scope = scope; self.token = token
    }
}

/// Errors the credential surface raises loudly. We never silently return a stub
/// grant or a fake secret.
public enum GroupError: Error, Sendable {
    case offPlatformHasNoVault           // can't hold credentials without a space
    case unknownCredential(CredentialHandle)
    case notAuthorised(by: SpaceID)      // caller's space may not issue this grant
    case grantRevokedOrExpired
}

// MARK: - The Group object — a projection of its backing Matter

/// A live Group instance, projected from one `.group` Matter. Value type.
public struct Group: PacificObject {
    public static let kind: ObjectKind = .group

    public let id: MatterID
    /// The owning space (Identity B). For an off-platform Group this is a local
    /// placeholder space we minted; `presence` tells the truth about reachability.
    public let space: SpaceID

    /// Display name of the person / group / organisation.
    public var displayName: String
    /// One person, a team, or an organisation.
    public var shape: GroupShape
    /// On- or off-platform reachability. Source of truth for "can we sync?".
    public var presence: PlatformPresence

    /// The Arc that GOVERNS this object — its moderation / reports / safety authority,
    /// and the Semaphore relay its traffic routes through (Arcs have static IPs, so the
    /// endpoint doubles as the relay). Cultural federation in action: whoever creates
    /// the object stamps their Arc; the roster moves it elsewhere by SUPERMAJORITY.
    /// Set and enforced in pacific-core (`groups.arc_url`, `node::swap_group_arc`); this
    /// is the read-side projection. `nil` before we have learned it (routing then falls
    /// back to the device's default Arc — never fabricated).
    public var configuredArc: URL?

    /// Public, content-free handles to this Group's vault entries. The handles
    /// are listable; the secrets behind them are not exposed here or anywhere.
    public private(set) var credentialHandles: [CredentialHandle]

    public init(from matter: Matter, in context: any PacificContext) throws {
        // TODO: decode displayName / shape / presence / configuredArc / credentialHandles
        // from matter.fields; mint or resolve the SpaceID via context.identity.
        fatalError("unimplemented")
    }

    /// Direct memberwise init (used by tests and by the projection above once built).
    public init(
        id: MatterID,
        space: SpaceID,
        displayName: String,
        shape: GroupShape,
        presence: PlatformPresence,
        configuredArc: URL? = nil,
        credentialHandles: [CredentialHandle] = []
    ) {
        self.id = id; self.space = space; self.displayName = displayName
        self.shape = shape; self.presence = presence
        self.configuredArc = configuredArc
        self.credentialHandles = credentialHandles
    }
}

// MARK: - The interface surface siblings and the right rail invoke

/// Everything other modules (System, Field, Project, Forum, Poll) and the right
/// rail call on a Group. The defining surface is credential issuance: a System
/// or Field asks the Group for a *scoped, revocable grant*, never for a secret.
public protocol GroupInterface: Sendable {

    // — Identity / membership —

    /// The space this Group maps to, or `nil` if it is an off-platform contact
    /// with no syncable space. Callers must handle `nil` (cannot seal to them).
    func memberSpace(of group: Group) -> SpaceID?

    /// Whether `group` may act in `role` within the object identified by `context`.
    /// Used by siblings to gate membership-derived permissions.
    func canAct(_ group: Group, as role: MembershipRole, in context: MatterID) async -> Bool

    // — Credential vault: shapes out, secrets never out —

    /// List the content-free handles in this Group's vault (e.g. to populate a
    /// System's "which credential?" picker). Returns handles, never secrets.
    func credentials(of group: Group) -> [CredentialHandle]

    /// Store a new secret. The plaintext is consumed into the vault and a handle
    /// is returned; the caller is expected to discard its copy. Throws
    /// `offPlatformHasNoVault` for an off-platform Group.
    func storeCredential(
        _ secret: Data,
        kind: CredentialKind,
        label: String,
        in group: Group
    ) async throws -> CredentialHandle

    /// Issue a scoped, revocable grant to a System/Field Matter. The returned
    /// grant authorises use of the credential without revealing it. Throws if the
    /// caller's space is not permitted to issue (`notAuthorised`).
    func issueGrant(
        for handle: CredentialHandle,
        scope: CredentialScope,
        issuedBy issuer: SpaceID
    ) async throws -> ScopedCredentialGrant

    /// Perform an external action on the grantee's behalf: the vault injects the
    /// real secret at the edge and returns ONLY the result. The secret is never
    /// part of the return value. Throws `grantRevokedOrExpired` if the grant is
    /// no longer live.
    func useGrant(
        _ grant: ScopedCredentialGrant,
        performing request: ExternalRequest
    ) async throws -> ExternalResponse

    /// Revoke a grant instantly (Permissions C: slam the door). Idempotent.
    func revoke(_ grant: ScopedCredentialGrant) async throws
}

// MARK: - The edge through which the vault speaks to external systems

/// A request a sibling wants performed against an external system, expressed
/// WITHOUT the secret — the vault supplies authentication itself.
public struct ExternalRequest: Sendable {
    public var method: String
    public var url: URL
    public var headers: [String: String]   // never contains the secret
    public var body: Data?
    public init(method: String, url: URL, headers: [String: String] = [:], body: Data? = nil) {
        self.method = method; self.url = url; self.headers = headers; self.body = body
    }
}

/// The result of a vault-mediated external call. Carries the response only —
/// no echo of the injected secret.
public struct ExternalResponse: Sendable {
    public var status: Int
    public var headers: [String: String]
    public var body: Data
    public init(status: Int, headers: [String: String], body: Data) {
        self.status = status; self.headers = headers; self.body = body
    }
}

// MARK: - Module descriptor (the left rail iterates these)

public struct GroupModule: PacificObjectModule {
    public static let kind: ObjectKind = .group
    public static let blurb: String =
        "A person, group or organisation — the unit of membership, and the keeper of credentials."
}
