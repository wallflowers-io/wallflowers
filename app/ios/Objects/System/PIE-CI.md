# System — PIE⊗CI

*An interactable external system, API or software package — a normal Nova object (SQLite-backed, its membership an MLS group, edits synced over MLS) that is composed of Fields, owns the external edge, and is driven through scoped Group credentials.*

## Purpose

A **System** brings a real-world tool — Onshape, SolidWorks, Thermal Desktop, Gmail — *inside* the model, so a Project can subscribe to it and a person can drive it from the right rail. Nova reads from, writes to and calls it.

What is true of System and nothing else in the seven:

- It is the only object **composed of Fields** — a System *is* a bundle of declared fields, each deep-linking into an external source, plus the operations over them. (A Field alone is one `key:index` link; a System is the assembly that gives those links a connector, credentials and actions.)
- It is the only object that **reaches outside Pacific** — it owns the *external* edge (HTTPS, a desktop bridge, IMAP), distinct from the *peer* edge every object shares (E/MLS).

A System is **created and authored by the user the way every object is**: you make one and fill in its declared fields, connector and operations. That definition is stored *with* the System object — it is **not** a special "Dictionary schema" and there is no register-at-runtime step. (Optionally a System carries a `MatchingTag` so peers can recognise the same *kind* of System across spaces — shared vocabulary, not storage or identity.)

Not a Group (which holds credentials + membership — the System *borrows* a Group's credential), not a Project (the hub that *subscribes to* Systems), not a Field (one link — the System composes many).

## Interface

The public surface is `Sources/System/System.swift`.

- **Object** — `System: NovaObject` (kind `.system`). A projection of one `.system` Matter carrying the `definition` (inline), `label`, `credentialSource` (the Group), the granted `credential` handle, and `fields` (the composed Field Matter ids).
- **Module** — `SystemModule: NovaObjectModule` (`kind`, `blurb`) so the left rail can list/create it.
- **Core value types** — `SystemDefinition` (the user-authored shape: `name`, optional interop `tag`, `author`, `fields`, `connector`, `credentialRequest`, `operations`); `SystemField` (one declared field — `key`, `hint`, `locator`, `writable`); `Connector` (`Kind` http / localBridge / imap / file, `endpoint`, `auth`) with `AuthScheme`; `Operation` (`Mode` read / write / call, `touches`, an access gate) with `SystemAction` / `ActionResult`; `CredentialRequest` and the opaque `CredentialHandle`.
- **Interface protocol** — `SystemInterface`: `define(_:)` (author/validate a definition — creating the System object itself is the generic object-creation path, no Dictionary step); `requestCredential(from:in:)` (ask a Group, get a handle); `invoke(_:in:)` (run an Operation).

The three rails:

- **Left rail (selection)** — `SystemModule.blurb`; lists each System by `label`.
- **Centre (contextual detail)** — the definition: declared `SystemField`s, the `Connector`, credential status (which Group, scopes, expiry), and the `Operation`s.
- **Right rail (interaction)** — one button per `Operation` (builds a `SystemAction`, calls `invoke`); the bind-credential affordance; and the author/edit-definition flow.

Sibling references: a **Project** references a System by `MatterID` (subscribes to it as a connector, drives it from the CLA). A **Field** is referenced as a composed member (`fields: [MatterID]`). A System references a **Group** by `MatterID` as its `credentialSource`. Where used, peers recognise the same *kind* of System by its optional `MatchingTag`.

## Encapsulation

- **The raw secret never enters the System.** A System holds only a `CredentialHandle` — an opaque, scoped, revocable reference into a Group's vault. The Group performs the actual signing/binding (`AuthScheme`); the System presents the handle and can never read the secret back. The load-bearing boundary.
- **The definition is stored with the object.** `SystemDefinition` (no secret, no credential state) and the binding (Group + handle) both live in the `.system` Matter — one object, one source of truth, synced over the System's MLS group. There is no separate registry to fall out of step.
- **Operations are the only way out.** External read/write/call happens *only* through a declared `Operation` touching declared `SystemField`s; no operation touches an undeclared field; no write goes through a field whose `writable` is false.
- **Access is membership.** Invoking a System requires membership in its group (owner/member); revoking membership or the credential handle stops it. *(The stub still carries the old `Disclosure` gate, pending the repo-wide membership=access sweep.)*
- **Honest-by-default.** A value the external system did not return is omitted from `ActionResult.values`, never fabricated; an unbound credential is `nil`, never a stub.

## Connection

- **F · Records** — stored as one `.system` Matter (definition + binding); every edit is a delta, every `invoke` appends a `Provenance` entry.
- **E · Network/MLS** — the System *is* an MLS group; its membership list governs access and its edits sync to that group over MLS. `requestCredential` and credential use are mediated by the credential Group over MLS (the peer edge). The **external** edge — the `Connector` — is separate and not MLS.
- **B · Identity** — `SystemDefinition.author` and the owning `space` are `SpaceID`s; the granting Group is an Identity-backed space.
- **A · Search** — System implements `Indexable`. `index_projections()` embeds the definition (name, field keys + hints, per-operation summaries, the credential-request *purpose*) so a System is findable by capability. Live external values stay pulled-on-demand rather than mirrored — a **churn** tradeoff (they'd re-embed every fetch for thin semantic gain), not a prohibition; index them if value-search is worth the cost. The credential handle is held *out* on security grounds.
- **D · Dictionary** — *optional and secondary.* A System may publish its `MatchingTag` as shared vocabulary so differently-named peers recognise the same kind of System. The Dictionary is **not** where the System lives and registration is not required.

Object edges: **composes** Fields; **referenced by / subscribed to by** Projects; **draws credentials from** a Group; **contained by** the owning space.

## Implementation

- **Matter projection** — `kind: .system`. The Matter stores the `definition` (its fields/connector/operations, inline), `label`, `credentialSource` (ref → Group Matter), `credential` (a serialised opaque `CredentialHandle`), and the composed Field ids (refs). It lives in the on-device SQLite tables; the delta log is the source of truth.
- **Authoring path** — the user fills a `SystemDefinition` (name, fields, connector, credential request, operations). `define` validates it (no duplicate field keys; operations only touch declared fields; writes only through `writable` fields). Creating the System is the ordinary object-creation path — a new `.system` Matter appended and synced to its MLS group. No Dictionary lowering.
- **Credential path** — `requestCredential(from: group)` sends the `CredentialRequest` (purpose, scheme, least-privilege scopes) to the Group over MLS (E). The Group's vault answers with a `CredentialHandle` (vault id, granted scopes ⊆ requested, optional expiry). At `invoke` time the connector asks the Group to *apply* the handle for the declared `AuthScheme` (inject header / mint bearer / present cert) — the System orchestrates, the Group signs. Revoking at the Group slams the door.
- **Invocation** — `invoke` checks the caller's access, resolves the connector, applies the credential via the Group, performs the read/write/call, returns an `ActionResult` of updated Field values + a `Provenance` entry. Unreturned values omitted, not faked.
- **MLS groups** — the System has its own group (its membership = access). The credential conversation is a separate two-party MLS exchange between the owning space and the credential Group.

### Open questions

- **Where the connector code lives.** A `localBridge` (SolidWorks, Thermal Desktop) is native and out-of-process — sibling module, app plugin, or shelled out? Undecided.
- **Credential application protocol.** "The Group signs on the System's behalf" needs a concrete request/response over E per `AuthScheme` (especially `oauthBearer` refresh and `clientCert` mTLS). Sketched, not specified.
- **Operation arguments & schema.** `SystemAction.arguments` is an untyped `[String: FieldValue]` bag; whether `Operation` should declare a typed parameter schema is open.
- **Field ownership.** A System composes Field objects; whether those are owned by the System's Matter or are independent, reusable Field Matters it merely references needs settling with the Field module.
