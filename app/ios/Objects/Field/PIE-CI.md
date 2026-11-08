# Field — PIE⊗CI

*A set of key:index pairs — the base unit of Nova's parametric modelling: it links live data and depends on other Fields.*

## Purpose

A Field is the smallest addressable thing a model is built from: a `key` names a slot, and an `.index` says where that slot's value lives in a structured source (a row, a cell, a JSON path, an ordinal position). A Field's value can be a literal, a **live value deep-linked into a networked data source**, or a **function of other Fields**.

What is true of Field and nothing else in the seven: it is the **base unit of parametric modelling**. A System is *composed of* Fields; a parametric model is a directed graph whose **nodes are Fields and whose edges are the references inside their expressions**. Group keeps credentials, System knows how to talk to an external source, Project is the hub with a CLA — but only a Field is an individually-addressable cell that can (a) carry a live value pulled/pushed through a System and (b) be the input or output of a formula. If the other six are the furniture of a workspace, Field is the variable.

## Interface

The public surface is in `Sources/Field/Field.swift`.

**Value types.**
- `KeyIndex` — the atomic `(key: String, index: FieldIndex)` pair. `FieldIndex` is a small algebra (`.ordinal`, `.named`, `.path`, `.cell(row:column:)`, `.whole`) so a Field can address into genuinely different source shapes rather than assuming an integer.
- `DataSourceLink` — the deep-link: the `system` (a `.system` Matter) the link rides through, the `endpoint` URL (this is the `FieldValue.link` value), the `address` (`KeyIndex`), a `credentialOwner`/`credentialEntry` pointing at the Group vault entry (never the secret), and a `LinkDirection` (`pull`/`push`/`bidirectional`).
- `Liveness` — `.unresolved` / `.resolved(at:)` / `.failed(at:reason:)`, so the centre rail can show staleness and failure *honestly* rather than dressing a stale or absent value as fresh.
- `FieldRole` (`none`/`existence`/`viewer`/`editor`/`owner`) and `FieldMember` — per-member standing on **this Field**, with a `disclosure` mapping onto Pacific `Disclosure`.
- `FieldRef`, `Parametric`, `FieldSource` — the parametric machinery (see Encapsulation).
- `Field` — the object value type, `NovaObject` with `kind == .field`.
- `FieldModule` — the `NovaObjectModule` descriptor the left rail iterates.

**Protocol `FieldInterface`** — the operations the right rail and sibling objects invoke:
- read/resolve — `currentValue(of:as:)` (no network, permission-gated) and `resolve(_:as:)` (fetch the live value through the System + Group grant);
- set/edit — `set(_:on:as:)` (literal set, or push out through the link);
- per-field permissioning — `setRole(_:for:on:by:)`;
- parametric — `defineExpression(_:on:as:)`, `dependencies(of:)`, `recompute(_:as:)`, `dependents(of:in:)`.

**The three rails.**
- **Left rail (selection):** Field appears via `FieldModule.blurb`; selecting one routes to its `.field` Matter.
- **Centre (contextual detail):** the resolved `value`, its `KeyIndex`, its `Liveness` (fresh / stale / failed), its source (literal vs link vs formula), and — for a parametric Field — its inputs and the live dependency sub-graph.
- **Right rail (interaction):** resolve/refresh, edit the literal or push a value, define or rewire the expression, and manage per-member roles.

**How siblings reference it.** A System references its Fields by `MatterID` (a System *is* a set of Field Matters). A parametric Field references other Fields by `FieldRef` (a `MatterID` plus an optional `KeyIndex`). Nothing references a Field's *value* directly — only its identity — so the value can always be re-resolved or re-derived.

## Encapsulation

**One source, three flavours.** `FieldSource` is `.literal | .deepLink | .parametric` — a Field is never two at once. The public `value` is `private(set)`; callers change it only through `set` / `resolve` / `recompute`, never by assignment. This is the boundary that keeps the honest-by-default rule enforceable.

**Honest-by-default, structurally.** For a `.deepLink` Field, `value == nil` whenever `liveness` is `.unresolved` or `.failed`. An unresolved networked value is **never fabricated** — no zero, no empty string, no last-good-pretending-to-be-fresh. A failed fetch is surfaced as `.failed(at:reason:)`, and `resolve` throws (`sourceUnreachable` / `grantUnavailable`) rather than returning a plausible-looking stand-in.

**The parametric dependency graph (the heart of the module).** A `.parametric` Field's value is **always re-derivable from its inputs and its expression, and is never stored as an independent truth** — the persisted `value` is a cache, exactly as a Poll's tally is derived from its responses. The expression is an explicit tree (`Parametric` = `.literal | .ref | .op`), not an opaque string, precisely so the graph is *inspectable without evaluation*:

- `Parametric.inputs` walks the tree and returns the set of `FieldRef`s it reads — the **out-edges** of this node. `FieldInterface.dependencies(of:)` exposes them.
- The invariant the module guarantees is **acyclicity**. `defineExpression` validates that adding the edges keeps the graph a DAG and throws `cycleDetected([MatterID])` otherwise — a parametric loop is rejected at declaration time, never discovered mid-evaluation.
- `recompute` evaluates over inputs' *current* values. If any input is unresolved it throws `unresolvedInput(FieldRef)` — an **honest stop**, never a substituted default. Honesty is transitive: a formula over an unresolved live Field is itself unresolved, not zero.
- `dependents(of:in:)` returns the transitive **in-edges** — the downstream recomputation cone — so when one Field moves, the model recomputes only the minimal affected set rather than the whole graph.

**Per-field permissioning (Permissions C at field granularity).** `members: [FieldMember]` is `private(set)` and carries each member's `FieldRole` on **this Field specifically** — finer than the containing object's membership. The same person may be `editor` on one Field of a System and only `existence` on its neighbour. `FieldRole.disclosure` maps to Pacific `Disclosure`, so the field-level role is the same truth the gate enforces, not a parallel one. `currentValue` returns `nil` below `.viewer`; `set`/`defineExpression` require `canEdit`; `setRole` requires `.owner`. A member not listed on a Field falls back to the containing object's gate (no silent widening).

**What never leaks.** The credential secret (only a vault-entry reference crosses the boundary); a value that has not actually been resolved or derived; and the internal evaluation order of the graph.

## Connection

**Pacific blocks.**
- **F · Records.** A Field projects from one `.field` Matter. The deep-link endpoint is stored as `FieldValue.link(URL)`; literal values as the matching `FieldValue` case; the expression and members as encoded fields. Edits append `Provenance`.
- **C · Permissions.** Field-level `FieldRole`s map onto `Disclosure` via `PermissionGate`; `setRole` writes through the gate. Revocation is instant and per-Field.
- **B · Identity.** Members are Groups, named by `SpaceID`; the owning `space` is the System/Project that holds the Field.
- **E · Network / MLS.** Resolving/pushing a deep-link is a vault-mediated external call; the credential grant and any cross-space sync travel sealed over MLS. The blind relay never sees plaintext or secrets.
- **A · Search (`Indexable`).** `index_projections()` embeds a Field's `key`, its resolved value **in context** (e.g. `yield_strength: 250 MPa`), and a source description, so a Field is findable by what it measures. Live values re-embed on each resolve — a **churn** tradeoff worth taking if value-search matters; a bare number or date on its own embeds weakly and is better left as filter metadata. Credential references stay out (security). Federated under Permissions; peer raw records are never copied (a federation rule, not an embed exclusion).
- **D · Dictionary.** Field shares the `.field` base `TypeShape`; a System that defines named Fields registers/extends shapes under a `MatchingTag` so differently-named peers line up.

**Edges to other objects.**
- **Contained by System** — a System is composed of Fields (by `MatterID`).
- **Deep-links through System** — `DataSourceLink.system` names the System the live value is reached through.
- **Draws credentials from Group** — `credentialOwner`/`credentialEntry` reference a Group vault entry; the Group issues a **scoped, revocable grant** (never the secret) for each fetch/push.
- **References other Fields** — `FieldRef` edges form the parametric graph; a model is this graph of Fields.
- **Members are Groups** — `FieldMember.space` is a Group's space.

> *Note on module boundaries:* Field depends only on `PacificCore` (mirroring its siblings; packages do not import each other). Cross-module concepts — the System it rides through, the Group vault it draws from — are referenced by `MatterID` / `SpaceID` / vault-entry `UUID`, and the actual grant issuance is performed by the Group module's `useGrant` at the network edge. Field holds the *reference and the authorisation request*, never the secret.

## Implementation

**Matter projection.** `ObjectKind.field`. From `matter.fields`: `keyIndex` (key + encoded index); a `source` discriminator with either the literal `FieldValue`, the `DataSourceLink` (endpoint as `FieldValue.link`, plus system/credential references and direction), or the encoded `Parametric` tree; the `[FieldMember]` roster; and the last-known `value` + `Liveness`. `init(from:in:)` leaves `value == nil` / `liveness == .unresolved` for a deep-link until a resolve succeeds — the projection never invents a value.

**Resolution & push.** `resolve` builds an `ExternalRequest`-shaped call addressed at the link's `endpoint`/`address`, asks the owning Group to `useGrant` (the vault injects the secret at the edge and returns only the result), decodes the value, and sets `value` + `liveness = .resolved(at:)`. Failure sets `.failed(at:reason:)` and leaves `value == nil`. `push`/`set` on a writable link reverses the direction through the same grant; `set` on a `.pull` or `.parametric` Field throws `notWritable`.

**Parametric evaluation.** `defineExpression` extracts `expr.inputs`, checks the resulting graph is acyclic against the surrounding model (`cycleDetected` otherwise), and commits the `.parametric` source. `recompute` resolves each input's `currentValue`, evaluates the `op` tree, and writes the derived `value` — throwing `unresolvedInput` if any input is unknown and `unknownOperator` for an op the evaluator does not implement. The derived value is cached on the Field but is authoritative only as a *projection*; on conflict, re-derivation wins. Change propagation uses `dependents` to recompute the minimal downstream cone.

**Embeddings / vector use.** The `key` and any resolved textual value are embedded for on-device search so Fields are semantically findable; numeric live values are not embedded (they would churn the index on every refresh).

**MLS group shape.** A Field has no MLS group of its own; it syncs within the group of its containing System/Project. Per-member field roles are carried in the Matter and enforced by `PermissionGate`, sealed over the containing object's MLS group.

**Agent hooks.** None of its own. A Project's CLA may *read* resolved Field values and *write* literal Fields through `FieldInterface` under the same permission gate as any member — there is no privileged agent path.

**Honest-by-default rule (restated, because it is load-bearing here).** Three places a value could be faked, and is not: an unresolved deep-link stays `nil` with explicit `Liveness`; a parametric Field over an unresolved input refuses to compute rather than defaulting; and a value below the viewer's `FieldRole` is withheld, not approximated.

### Open questions

- **Cross-model cycle detection.** `defineExpression` can keep a *single* model acyclic, but a dependency graph can span Fields owned by different spaces reachable only over MLS. Detecting a cycle that closes through a peer's private Field — without leaking that the Field exists — is unresolved. A conservative local-only check that errs toward rejecting unverifiable edges is the likely first cut.
- **Recomputation trigger.** Whether downstream recompute is push (a resolved Field notifies dependents) or pull (a dependent recomputes lazily on read) is not yet decided; push gives freshness, pull gives cost control. Likely pull-with-invalidation, but unproven against a large parametric model.
- **Live-value caching vs honesty.** How long a `.resolved(at:)` value may be shown before it must be re-fetched or downgraded to "stale" is a policy not yet fixed — it trades network cost against the honest-by-default principle and probably belongs to the System, not the Field.
- **Expression operator vocabulary.** `Parametric.op` is an open `String`-named n-ary operator resolved by the evaluator; the canonical set (arithmetic, lookup/interpolation, conditional, unit-aware ops) and how user-defined operators register is deferred.
- **Index semantics across source kinds.** `FieldIndex` covers ordinal/named/path/cell/whole, but reconciling these with each System's native addressing (and validating an `address` against a source's actual shape) is left to the System adapter and not yet specified.
