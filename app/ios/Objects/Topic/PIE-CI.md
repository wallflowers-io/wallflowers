# Topic — PIE⊗CI

*An agent-managed deep-research dossier into one topic — grounded in your own corpus, kept live by a research loop, and honest by default: every finding is cited or it does not exist.*

## Purpose

A Topic does the user's job of *staying current and deeply informed about one
particular subject* without doing the reading themselves. You charter it with a
brief; thereafter its research agent keeps a living dossier up to date for you —
pulling first from your own corpus, then from active external sources — and
notifies anything that depends on it the moment its understanding moves.

What is true of a Topic and of nothing else in the seven:

- It is the only **agent-managed** object whose content is *produced by a
  background research loop*, not authored by people. A Forum accumulates human
  posts; a Topic accumulates **agent-established, cited Findings**.
- It is the only object whose first move is to **read the on-device vector index
  (Search A)** — it is anchored in the user's own Matters before it ever reaches
  outward. It is a consumer of Search A, not (primarily) a producer of records.
- It is the **upstream** of the Project relationship: a Project *subscribes* to a
  Topic. The Topic is the sensor; the Project's Core Loop Agent is the actuator.

Neighbour check. A **System** is a live connection to one external source; a
Topic may *use* several Systems as Connections but its job is synthesis across
them, not access to any one. A **Field** deep-links a single datum; a Topic
curates a whole evolving dossier. A **Forum** is human conversation; a Topic is
machine research. Nothing else both grounds in Search A and emits change events
that a Project consumes.

## Interface

The public surface is the research loop, the dossier, and the change-event
subscription. Core value types (see `Topic.swift`):

- `Topic: NovaObject` (kind `.topic`) — `brief`, `cadence`, `connections`,
  `digest`. Projects from one `.topic` Matter.
- `TopicModule: NovaObjectModule` — `kind` + `blurb` for the left rail.
- `ResearchAgent` — the loop protocol: `ground(topic:using:as:)` (Search A
  first), `tick(topic:connections:in:)` (the periodic pass), `ingest(_:topic:in:)`
  (reaction to an active push).
- `Finding` — one cited claim; `source: FindingSource` is **required**, so an
  uncited claim is unrepresentable. Carries `confidence` and `establishedAt`.
- `FindingSource` — `.corpus(MatterID, score:)`, `.connection(id:, locator:)`,
  `.peer(SpaceID, locator:)`. The provenance gate.
- `Connection` — an active source (`.system` / `.peerFeed` / `.web`), polled on
  cadence and able to push between ticks.
- `TopicDigest` — the living projection: `summary`, `findings`, `openQuestions`,
  monotonic `revision`, `revisedAt`.
- `TopicChange` — the event subscribers receive: `toRevision`, `added`,
  `removed`, `summaryChanged`.
- `TopicInterface` — the operations the rails and siblings invoke.

The three rails:

- **Left rail (selection)** — lists Topics via `TopicModule`; shows the brief and
  a freshness/revision badge from the digest.
- **Centre rail (contextual detail)** — renders the `TopicDigest`: the generated
  summary, the cited Findings (each with its auditable `locator`), and the open
  questions. `digest(of:)` / `finding(_:in:)`.
- **Right rail (interaction)** — `refresh` (run a pass now), `setCadence`,
  `connect` / `disconnect` Connections.

How siblings reference it. A **Project** references a Topic by `MatterID` and
**subscribes** via `subscribe(to:)`, consuming `TopicChange` in its CLA; it can
catch up after downtime via `changes(of:since:)`. A **System** is referenced *by*
a Topic as the `backing` of a `.system` Connection.

## Encapsulation

What the module hides, and the invariants it always upholds:

- **The dossier is a projection, never an independent truth.** `TopicDigest` is
  always re-derivable from its `findings`, and each Finding from its `source`. The
  generated `summary` is convenience, not record — it never outlives the evidence
  it traces to. (Mirrors the Poll rule: output derivable from inputs, never
  stored as standalone truth.)
- **No Finding without provenance.** `Finding.source` is non-optional; the only
  ways in carry a citable origin. An attempt to admit an uncited claim is
  `TopicError.uncitedFinding`, not a silently accepted guess. This is the
  no-fabrication rule enforced in the type system.
- **Unknown stays unknown.** When the agent cannot establish something, it emits
  an `openQuestion`, never an invented answer. An unreachable Connection becomes
  a recorded gap (`connectionUnreachable`), not fabricated findings.
- **Raw secrets never enter the boundary.** Connections authenticate through a
  Group-issued scoped credential *grant*; the Topic holds authorisation, never
  the secret (Permissions C; Group's vault).
- **Peer records are never copied.** A `.peer` Finding cites the contributing
  space and a locator; the federated raw record is not duplicated into the digest
  (Search A federation under C).
- **`revision` is monotonic.** Subscribers rely on it to detect missed changes;
  it only ever increases, and `changes(of:since:)` fails loudly with
  `revisionTooOld` rather than replaying a fabricated history.

## Connection

Pacific blocks this module binds, and how:

- **A · Search (central).** The agent's *first* act each pass is
  `ground(topic:using:as:)` → `SearchEngine.query(brief, as: space, limit:)`,
  scoped to what the owning space may see and federated to permitted peers. The
  permitted hits become `.corpus` Findings (carrying the index's relevance
  score). The Topic also *writes back*: persisted Findings are themselves Matters,
  indexed via `SearchEngine.index(_:)` / `VectorIndex.upsert(_:_:)`, so the
  dossier in turn enriches the user's corpus. If Search A is unavailable the pass
  refuses to proceed (`searchUnavailable`) — it never "researches" ungrounded.
- **F · Records.** The Topic projects from one `.topic` Matter (`brief`,
  `cadence`, serialised `connections`); each `Finding` persists as its own Matter
  whose `history` (`Provenance`) records the agent pass that established it.
- **B · Identity.** All research runs *as* the owning `space`; outbound change
  events are authored and sealed under that identity.
- **C · Permissions.** Search grounding and peer-feed Connections are gated by
  disclosure; a subscribing Project sees `TopicChange` only at its granted level.
- **E · Network.** `TopicChange` is sealed in an `Envelope` and delivered over
  Transport to a remote subscriber; a same-space subscriber gets it on a local
  event channel. Active `.peerFeed` Connections also arrive over E.
- **D · Dictionary.** The `.topic` shape and the `Finding`/`Connection` field
  shapes resolve by `MatchingTag` so differently-named peer spaces interoperate.

Edges to other objects:

- **Subscribed-to by** Project (its CLA consumes `TopicChange`). This is the
  defining edge.
- **References** System (as the backing of `.system` Connections) and peer spaces
  (as `.peerFeed` Connections / `.peer` Findings).
- **Draws on** the user's own Matters of every kind, via Search A.

## Implementation

- **Matter projection.** Kind `.topic`. Fields: `brief: text`, `cadence`
  (encoded), and `connections` (encoded list of `Connection`). Findings are *not*
  inlined — each is its own Matter (kind reused with a finding shape) referenced
  by the digest, so provenance lives in `Matter.history` and the dossier stays
  re-derivable.
- **The research agent loop.** Two halves, one contract:
  1. *Periodic* — `tick` fires on `ResearchCadence` (`.manual` /
     `.every(seconds:)` / `.adaptive(floor:ceiling:)`; adaptive widens when
     findings stop moving and narrows when a Connection churns, bounded so it is
     never silently disabled nor hammers a source).
  2. *Active* — `ingest` reacts to a `ConnectionUpdate` a Connection pushed
     between ticks.
     Both: ground in Search A first, pull/parse Connections into **cited**
     Findings, reconcile against the current digest (add new, drop retracted /
     failed-re-verification), bump `revision`, regenerate the `summary`.
- **Embeddings / vector use (`Indexable`).** Read path is Search A grounding
  (corpus hits → cited Findings). Write path: `index_projections()` embeds the
  brief, each Finding's claim, the open questions — and the generated **summary**
  too, as a re-derivable projection, so a Topic is findable by its synthesis
  (consistent with "the digest is a projection, never independent truth"). Confidence
  and revision stay as metadata (churn); peer raw records are never copied
  (federation). The vector DB is the spine, not an add-on.
- **Change emission.** On any digest move, build a minimal `TopicChange` (delta,
  not the whole dossier), seal it per subscriber, and deliver over E (remote) or
  the local channel (same-space). `subscribe(to:)` is the live stream;
  `changes(of:since:)` is the catch-up, failing loudly past retention.
- **Honest-by-default rule.** Unknown stays `nil` / empty, never fabricated:
  - a claim with no citable source is rejected (`uncitedFinding`);
  - an unreachable Connection yields an open question, not invented findings;
  - low confidence is surfaced as-is, never rounded up to certainty;
  - the generated summary is empty when there are no findings.

**Open questions** (genuinely undecided):

- *Finding identity across passes* — when a re-verification confirms an existing
  claim from a new source, do we mutate the Finding's `source`, keep both, or
  supersede? Affects the `added`/`removed` semantics of `TopicChange`.
- *Adaptive-cadence policy* — the floor/ceiling envelope is specified, but the
  signal that widens/narrows it (finding-churn rate? Connection volatility?) is
  not yet pinned.
- *Cross-space Finding provenance* — for a `.peer` Finding, how much of the
  contributing space's citation may we surface under each disclosure level
  without leaking more than C permits?
- *Digest retention* — how many historic `revision`s we retain bounds how far a
  long-offline Project may catch up before `revisionTooOld`; not yet sized.
