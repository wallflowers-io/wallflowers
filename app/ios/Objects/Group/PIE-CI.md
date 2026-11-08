# Group — PIE⊗CI

*A person, a group of people, or an organisation — on or off the platform; the unit of membership everywhere in Nova, and the keeper of credentials.*

## Purpose

A Group is **who** — a single person, a team acting as one, or an organisation. Selecting a Group in the left rail shows detail on that person/group/organisation. It does two jobs nothing else in the seven does:

1. **It is the unit of membership.** Project, Forum, Poll and Field do not enrol raw `SpaceID`s — they enrol Groups. A Group is the one object that maps cleanly onto a Pacific **Identity** space: an `individual` to a person space, a `team`/`organisation` to a team space. Every "who can see / do / vote on this" question routes through a Group.
2. **It owns credentials.** Secrets (API keys, OAuth tokens) live at the Group level, in its vault. System and Field objects do not store their own keys; they ask the Group for *scoped, revocable access*. This is deliberate: credentials belong to a who, not to a what, and revoking a Group's access revokes it everywhere at once.

Why a Group and not a Project member list, or a bare `SpaceID`? Because a `SpaceID` is an Identity primitive with no roles, no off-platform story, and no vault; and a Project's member list is a *projection of Groups*, not a thing in itself. The Group is also the only object that can exist **off-platform** — a known contact we cannot yet sync with — so that membership and attribution work before the other party has joined.

## Interface

The public surface is the file `Sources/Group/Group.swift`. Doc and code agree.

- **`Group`** — value type, `NovaObject`, `kind == .group`, `ID == MatterID`. Fields: `space` (owning Identity space), `displayName`, `shape` (`GroupShape`), `presence` (`PlatformPresence`), and `credentialHandles` — content-free pointers, never secrets.
- **`GroupModule`** — `NovaObjectModule` descriptor (`kind`, `blurb`) the left rail iterates to list/create Groups.
- **`PlatformPresence`** — `.onPlatform(SpaceID)` for a live, syncable peer; `.offPlatform(inviteHint:)` for a known contact with no space yet. `.space` returns the `SpaceID` or `nil` — the honest answer to "can we sync with them?".
- **`GroupShape`** — `individual` | `team` | `organisation`; decides person-space vs team-space mapping.
- **`MembershipRole`** — `owner` | `admin` | `member` | `viewer` | `guest`. The capacity a Group plays *in a containing object*; held by that object, named with this shared vocabulary.
- **The credential surface** — `CredentialKind` (apiKey/oauthToken/…); `CredentialHandle` (opaque, `Codable`, content-free pointer to a vault entry); `CredentialScope` (grantee `MatterID`, allowed operation verbs, optional `expiry`); `ScopedCredentialGrant` (a live, revocable authorisation carrying an opaque capability `token`, **not** the secret); `GroupError` (loud failures — no stub grants).
- **`GroupInterface`** — the protocol siblings and the right rail invoke: `memberSpace(of:)`, `canAct(_:as:in:)`, `credentials(of:)`, `storeCredential(...)`, `issueGrant(for:scope:issuedBy:)`, `useGrant(_:performing:)`, `revoke(_:)`. Plus `ExternalRequest` / `ExternalResponse` — the vault-mediated edge through which a secret is injected at call time and never returned.

What each rail sees:

- **Left rail (selection)** — a row per Group via `GroupModule`; `displayName`, `shape` icon, and an on/off-platform indicator from `presence`.
- **Centre (contextual detail)** — the Group's profile, its memberships (the objects that enrol it), and its vault as a list of `CredentialHandle`s (labels and kinds only; secrets are never rendered).
- **Right rail (interaction)** — store a credential, issue/revoke a grant, edit roles, send an invite to an off-platform Group. The right rail acts purely through `GroupInterface`.

How siblings reference a Group: by `MatterID` (its identity) and, for sealing, by the `SpaceID` from `memberSpace(of:)`. A System/Field references a *credential* by `CredentialHandle` and obtains use through `ScopedCredentialGrant` — never by holding a key.

## Encapsulation

The module hides one thing above all: **the secret material**. Invariants that always hold:

- **A secret never crosses the boundary.** Not in `Group`, not in `CredentialHandle`, not in `ScopedCredentialGrant`, not in `ExternalRequest`/`ExternalResponse`. The only public artefacts are content-free handles and capability tokens. `useGrant` injects the real secret *inside* the vault at the network edge and returns only the external result; the secret is never an argument to, nor a field of, anything a caller can read.
- **Grants are scoped and revocable.** A `ScopedCredentialGrant` is bound to a single grantee `MatterID`, a set of operation verbs, and an optional expiry. `revoke` is instant and idempotent (mirrors Permissions C "slam the door"). A revoked or expired grant fails loudly with `grantRevokedOrExpired` — it never silently degrades to "no-op success".
- **Off-platform Groups have no vault.** `storeCredential`/`issueGrant` on an off-platform Group throw `offPlatformHasNoVault`. We do not pretend to hold a secret for a space that does not exist.
- **Roles live in the container, not the Group.** A Group carries no membership list of its own; `MembershipRole` is interpreted by whichever object enrolled it. This keeps a single Group consistent across many contexts without a second source of truth.
- **Honest by default.** Unknown is `nil`: `PlatformPresence.space`, `CredentialScope.expiry`, and `offPlatform(inviteHint:)` all admit "we don't know" rather than fabricating a value.

## Connection

Pacific blocks bound:

- **B · Identity** — core. A `Group` *is* a `SpaceID` projection: `individual` → person space, `team`/`organisation` → team space. `memberSpace(of:)` returns the `SpaceID` for sealing, or `nil` off-platform. Off-platform Groups hold a local placeholder space; `presence` is the truth.
- **C · Permissions** — credential grants are a Permissions-shaped capability. `issueGrant`/`revoke` express the same instantly-revocable disclosure model as `PermissionGate`; `canAct(_:as:in:)` derives a Group's allowed actions in a container, and maps `MembershipRole` onto `Disclosure` for read gating.
- **F · Records** — a Group projects from one `.group` `Matter`. `displayName`, `shape`, `presence` and the vault index (handle metadata only) live in `Matter.fields`; the raw secret does **not** (see Implementation).
- **E · Network** — only on-platform Groups can be `to:` targets for `Envelope`s; off-platform Groups cannot receive sealed messages, which is exactly why `presence`/`memberSpace` must be checked first. An invite to an off-platform Group rides this block once they join.
- **D · Dictionary** — `.group` is a base shape; an organisation may register a richer `TypeShape` (e.g. extra profile fields) without changing this contract.
- **A · Search** — Group implements `Indexable` **and** `Resolvable`. `index_projections()` embeds its public profile (`displayName` + `shape`) so a Group surfaces in semantic search ("who works on X"); `identifiers()` exposes its **email · domain · phone · handle · name** — the entity-resolution feedstock the connectors also fill, so an inbound email or website resolves to the right Group. The vault is the one deliberate exclusion, on **security** grounds (a search index is a queryable on-disk copy): handle labels/kinds may be surfaced, but raw secret bytes are recommended *out* — a risk call, not a ban.

Edges to other objects: **contained-by** Project/Forum/Poll/Field (as a member). **Provides-to** System and Field — the credential grants they use to reach external sources. Referenced by `MatterID`; sealed-to by `SpaceID`.

## Implementation

- **Matter projection.** `kind == .group`. `fields` carries: `displayName` (`.text`), `shape` (`.text`), `presence` (`.text` discriminator + optional `inviteHint`), and the **vault index** — one `.text`/`.ref` entry per `CredentialHandle` recording `id`, `kind` and `label` only. The secret bytes are **never** a `FieldValue`; Matters are replicated under Permissions and we will not replicate plaintext keys.
- **Where the secret actually lives.** Outside the Matter graph, in a device-local secure store (Keychain / Secure Enclave-backed), keyed by `CredentialHandle.id`. `ScopedCredentialGrant.token` is a capability minted against that store; `useGrant` is the only path that dereferences it, and it does so without surfacing the key. (Open question below: cross-device vault sync for a team space.)
- **MLS group shape.** A `team`/`organisation` Group corresponds to an MLS group whose members are the constituent person spaces; an `individual` is a single-credential space. Membership changes to a team Group are MLS add/remove operations.
- **Agent hooks.** None. Group is passive; it answers and authorises. (Projects run the CLA; Topics run the research loop — Group does neither.)
- **Honest-by-default.** Unknowns stay `nil` and failures throw `GroupError` loudly; there is no dev-mode path that returns a fake grant or a placeholder secret.

### Open design questions

- **Team vault sync.** A team Group's secrets must be usable from each member's device. The likely answer is wrapping the secret to each member's MLS credential so it syncs sealed (never as plaintext, never as a Matter field) — but the exact custody model (who can decrypt, what happens on member removal / key rotation) is not yet settled.
- **Grant token format.** Whether `ScopedCredentialGrant.token` is a signed macaroon-style capability (attenuable, offline-verifiable) or a server-side reference resolved per use is open; it changes how `revoke` propagates.
- **Off-platform → on-platform reconciliation.** When an off-platform contact joins, their placeholder space must merge into the real `SpaceID` without breaking existing memberships and attributions. The merge/identity-claim handshake is unspecified.
- **De-duplicating Groups.** Two members may each create a Group for the same external organisation; whether/how these reconcile (via `MatchingTag` in Dictionary D, or left as distinct) is undecided.
