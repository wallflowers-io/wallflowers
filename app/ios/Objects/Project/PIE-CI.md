# Project — PIE⊗CI

*A project-management hub whose defining organ is a Core Loop Agent (CLA) that watches the Project's subscriptions, recommends actions, and — within an explicit, gated boundary — takes them.*

## Purpose

A Project is where a piece of work is run: it owns and manages the task Matters that make up that work, carries optional additional members (each a Group, in a role), and runs a **Core Loop Agent**. The CLA is the thing that is true of a Project and of nothing else in the seven: an agent hook that fires whenever any of the Project's **subscriptions** registers a change, and that responds along two surfaces — it **recommends** action to a human, and, inside a narrow boundary, it **acts** autonomously.

Why this and not a neighbour: a Topic is also agent-managed, but its agent runs *inward* — a research loop reading the personal vector DB to keep one subject current. A Project's agent runs *outward and operational* — it reacts to change across many subscribed sources (Topics *and* Systems) and turns that change into task movement. A System is a connector the Project consumes; a Forum/Poll are conversational; a Group is the membership unit a Project's members are drawn from. The Project is the only object that *owns task Matters* and the only one whose agent is permitted to *act* on the world rather than merely describe it.

## Interface

Every type named here lives in `Sources/Project/Project.swift`.

**The value.** `Project` conforms to `NovaObject` with `kind == .project`. It projects from its backing Matter and carries: `members: [Membership]`, `subscriptions: [Subscription]`, `taskRefs: [MatterID]`, and an `autonomy: AutonomyPolicy`. `init(from:in:)` re-projects it from the honest record (currently a stub).

**Roles and members.** `Role` is an ordered enum — `viewer < contributor < maintainer < owner`. A `Membership` binds a member Group's `SpaceID` to a `Role`. The role is advisory to the UI but *authoritative for the CLA*: it bounds how far the agent may act on a given member's behalf.

**Subscriptions and events.** A `Subscription` is a standing interest in a `SubscriptionTarget` — exactly `.topic(MatterID)` or `.system(MatterID)`. It is an *edge plus the `heldDisclosure` we hold on the target*, never a copy of the target's data. When the target changes, the runtime delivers a `ChangeEvent` (carrying a `ChangeOrigin` — `.network(envelope:)`, `.matter`, or `.search` — the clamped `disclosure`, and a permission-clamped `summary?`) and triggers the CLA.

**The CLA.** `CoreLoopAgent` is the agent-hook protocol the runtime drives. `onChange(_:in:)` is the **trigger** — one call per change — returning a `CoreLoopOutcome` (`recommendations` for humans, `actionsTaken` already executed). `authorise(_:in:)` returns an `AutonomyDecision` (`.allow` / `.deny(Denial)`), folding the static policy with live C and B checks; the runtime calls it before the CLA acts and again when a human accepts a recommendation.

**The two output types.** A `Recommendation` is a safe, inert proposal (`proposal`, the `proposed: Action`, and a `blockedReason: AutonomyDecision.Denial?` explaining why it could not run unattended). An `Action` is a concrete effect with a closed `Kind` set: `createTask`, `setTaskStatus`, `setTaskField`, `touchSubscription`, and the outward-facing `dispatch`.

**The rails.** `ProjectInterface` is the operation surface:
- **Left rail (selection):** `ProjectModule: NovaObjectModule` supplies `kind` + `blurb` so the rail can list and create Projects generically.
- **Centre (contextual detail):** reads the projected `Project` — title/status, members, subscriptions, and the owned task Matters via `tasks(for:)`.
- **Right rail (interaction):** drives the CLA surface — `pendingRecommendations()`, `accept(_:)` / `decline(_:)`, subscription edits (`subscribe`/`unsubscribe`), membership edits, and `setAutonomy(_:)`.

**How siblings reference it.** Topics and Systems are referenced *by `MatterID`* inside a `Subscription.target`; conversely a Topic knows Projects subscribe to it. Task Matters are referenced *by `FieldValue.ref`* in `taskRefs`. Members are referenced *by `SpaceID`* (Identity).

## Encapsulation

What the module hides and the invariants it always holds:

- **The CLA is not part of the value.** `Project` (the projection) holds the *policy* (`AutonomyPolicy`) but not the running agent. The runtime owns the `CoreLoopAgent`. The value can be serialised, synced and re-projected without dragging agent state across the boundary.
- **A subscription is an edge, never a cache.** It stores the target's id and the disclosure we hold — not the target's contents. There is one source of truth for a Topic/System (its own Matter); the Project never shadows it. (This is the no-dual-source rule applied: the Project does not "retain a projection" of the subscribed object.)
- **An event never carries more than its disclosure.** `ChangeEvent.summary` is clamped to `heldDisclosure`; at `.existence` it is `nil`. The CLA cannot reason about contents it was never permitted to see — and it says so honestly rather than guessing.
- **Recommendations are inert.** A `Recommendation` has *no* effect until a human accepts it, and acceptance re-runs the same `authorise` gate. There is no path by which surfacing a recommendation mutates the world.
- **The autonomy boundary is single-sourced and never silently escalates.** `AutonomyPolicy` is the only place the unattended-allowed set is defined. Any action not permitted unattended **degrades to a recommendation** — it is never quietly performed. Outward `dispatch` carries a *second* lock (`allowOutboundDispatch`) on top of the policy set, so it can never go out unattended by accident.
- **Task contents are not owned here.** The Project owns *which* tasks exist (the `taskRefs` list) but each task Matter is the honest source of its own fields/status. The Project edits tasks only through `Action`s, all of which run the gate.

## Connection

Pacific blocks bound, and how:

- **F · Records.** The Project *is* a Matter (`.project`). Task Matters are separate records, owned by reference (`FieldValue.ref` in `taskRefs`). Every CLA action that mutates a record appends to that Matter's `history` (Provenance), including who/what authorised it — the CLA's autonomous acts are signed under the Project's owner Identity, accepted recommendations under the accepting human.
- **C · Permissions.** Three ways. (1) `Subscription.heldDisclosure` records the level we hold on each target and clamps every `ChangeEvent`. (2) `authorise` consults the live `PermissionGate` before any autonomous action; `insufficientDisclosure` is a first-class denial. (3) Member `Role` bounds what each member sees and how far the CLA may act for them. Revocation is instant: drop the disclosure and the next event arrives empty and the next action is denied.
- **B · Identity.** Members are `SpaceID`s. Autonomous actions are signed under the Project's **owner** space — if there is no owner Identity to sign under, the decision is `notSignableIdentity` and the action degrades to a recommendation. This is the honest tie between "the agent did it" and "a real, accountable identity stands behind it".
- **E · Network/MLS.** Inbound change from a subscribed peer space arrives as a verified, de-duplicated `Envelope` (`ChangeOrigin.network`). Outward `dispatch` actions send a sealed `Envelope` to a member or System space. The relay stays blind; the CLA never sees plaintext it is not entitled to.
- **A · Search.** A subscription can also fire from the on-device vector index surfacing new relevance for a target (`ChangeOrigin.search`) — e.g. a Topic the Project follows gains material the index now ranks highly. Search results are already permission-scoped, so the resulting event is honest by construction.
- **D · Dictionary.** `.project` is a base shape; its field layout (below) registers as a `TypeShape` so differently-named spaces still recognise a Project and its `taskRefs`/subscription edges.

Edges to other objects: **subscribes to** Topics and Systems (its trigger sources); **owns/manages** task Matters (by ref); **draws members from** Groups; **consumes credentials** indirectly — a `dispatch` to a System uses that System's own Group-scoped credentials, not the Project's.

## Implementation

- **Matter projection.** A Project Matter (`kind: .project`) carries fields: `title` (`.text`), `status` on the Matter itself, `members` (encoded list of `SpaceID`+`Role`), `subscriptions` (encoded list of target id + kind + held disclosure), `taskRefs` (a set of `FieldValue.ref` edges), and `autonomy` (encoded `AutonomyPolicy`). `init(from:in:)` decodes these; **honest-by-default** — an absent field stays empty/default, never fabricated.
- **Storage.** Records live in the `MatterStore`; the projection is rebuilt on read under the viewer's permissions. No separate Project database.
- **The agent hook (CLA).** The runtime subscribes to: the `Transport.inbound()` stream (filtered to subscribed target spaces → `.network` events), `MatterStore` updates for owned/subscribed Matters (→ `.matter`), and `SearchEngine` relevance changes (→ `.search`). Each produces a clamped `ChangeEvent` and calls `CoreLoopAgent.onChange`. The agent returns recommendations and may call back through `authorise` to take actions; the runtime executes only `.allow`ed ones and records both takings and proposals in provenance.
- **The autonomy boundary, concretely.** Default policy is `.conservative`: the CLA may only `touchSubscription` unattended (keep its own edges tidy) — everything substantive is recommend-only until a maintainer widens it. `dispatch` requires *both* membership in the unattended set *and* `allowOutboundDispatch == true`, *and* a passing live C+B gate. A widened policy is itself recorded in the Matter's history.
- **Embeddings / vector use (`Indexable`).** `index_projections()` embeds the Project's title + description so it is searchable and the CLA's `.search`-origin trigger can fire. Owned task Matters are indexed **under their own ids** (not a Project-level digest) to avoid churn and a second source of truth. Volatile facets — `status`, `subscriptions` — travel as filter *metadata* rather than embedded text: a churn tradeoff (re-embedding on every change buys little semantic value), tunable if value-search proves worth it.
- **MLS group shape.** The Project's members form (or join) an MLS group keyed on the owner space; `dispatch` and inbound subscription envelopes ride that group / the targets' groups over the blind relay.

**Open questions (genuinely undecided):**
1. **Action granularity vs. plans.** The `Action.Kind` set is intentionally closed and flat. Should a multi-step plan be a first-class `Action` (a transaction the human accepts/declines as a unit) or always decomposed into per-step recommendations? Flat is more auditable; plans are more useful.
2. **`dispatch` and a System's credentials.** A `dispatch` to a System should use *that System's* Group-scoped credential, not the Project's. The exact handoff (does the CLA ever hold the credential, or only ask the System object to act?) is unresolved — leaning toward "ask the System, never hold".
3. **Per-member autonomy.** `Role` bounds the CLA today at the Project level. Whether the autonomy policy should be expressible *per member* (act unattended for the owner, recommend-only for contributors) is open; it adds power but multiplies the surface a human must reason about.
4. **De-duplication window.** `Subscription.lastSeen` plus `Envelope.id` dedup network events, but a `.matter`+`.search` pair describing the *same* underlying change could double-fire the CLA. A cross-origin coalescing key is not yet designed.
