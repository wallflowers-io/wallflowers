# Forum — PIE⊗CI

*A multi-user thread — like a Slack channel: a keyed set of topic_areas and a keyed set of member Groups, with shared multimedia.*

## Purpose

A Forum is where a fixed set of members hold an ongoing, many-to-many
conversation, split into named sub-threads. The user's job it does: "give these
people a shared, durable place to talk, organised into channels, with media."

Why this and not a neighbouring object:

- A **Poll** asks one question and derives one answer; a Forum is open-ended and
  derives nothing — its truth is the posts themselves. (A Poll may be *posted
  into* a Forum, but it is not a Forum.)
- A **Group** is the membership *unit*; a Forum *has* members (each a Group) but
  is itself a conversation, not an identity.
- A **Project** is a hub with a CLA that may *subscribe to* a Forum's activity;
  the Forum is the conversation it subscribes to, not the project itself.
- A **Topic** is a single-author, agent-managed research artefact reading Search;
  a Forum is multi-author, human-authored, and stores rather than synthesises.

What is true of a Forum and nothing else in the seven: it is the only object
that is a *stream of many independently-authored posts* fanned out as a standing
MLS group, with two keyed maps (topic_areas, members) governing routing and
visibility.

## Interface

The public surface is in `Forum.swift`:

- **`Forum: NovaObject`** (kind `.forum`) — projects the *hub* Matter: `title`,
  the keyed `topicAreas: [String: TopicArea]` map, the keyed
  `members: [SpaceID: Membership]` map, and `mlsGroup` (the MLS group id).
- **`ForumModule: NovaObjectModule`** — `kind` + `blurb` for the left rail.
- Core value types: **`TopicArea`** (a keyed sub-thread: stable `key` + display
  `name`), **`Membership`** (a member Group's `SpaceID` + `MemberRole`),
  **`Post`** (projected from its own post Matter: author, topic_area, body,
  attachments, timestamp), **`Attachment`** (a content-addressed `FieldValue.blob`
  with an optional MIME type).
- **`ForumInterface`** — `post(…)`, `posts(in:…)`, `addTopicArea(_:to:)`,
  `addMember(_:to:)`, `removeMember(_:from:)`.

What each rail sees and does:

- **Left rail (selection):** the Forum appears as one row (`ForumModule.blurb`,
  `bubble.left.and.bubble.right`). Selecting it loads the hub.
- **Centre (contextual detail):** the topic_area list and, within a chosen
  topic_area, its `posts(in:as:)` stream — only the topic_areas and posts the
  viewer is permitted to see (C).
- **Right rail (interaction):** compose-and-`post`, add a topic_area, and
  add/remove a member. Owners additionally adjust per-topic_area permissions.

How siblings reference it: by the hub's `MatterID`. A **Poll** posted into a
Forum is a post whose body references the Poll Matter (`.ref`). A **Project**
subscribes by watching post Matters whose `forum` field is this hub.

## Encapsulation

What the module hides and the invariants it guarantees:

- **The two keyed maps are the source of truth for routing and membership.**
  `topicAreas` keys are stable and unique within the Forum; the human `name` may
  change but the `key` never does, so historical posts never re-route.
- **`members` and the MLS group are kept in lock-step.** A Group in the
  `members` map but not the MLS group (or vice-versa) is an invariant violation;
  `addMember`/`removeMember` mutate both atomically, and removal forces an MLS
  key rotation so a removed member cannot decrypt subsequent posts.
- **Posts are append-only and immutable.** A Forum never rewrites a post Matter;
  edits/retractions are new Matters in the post's history, never destructive.
- **Blob bytes never leak inline.** A `Post` only ever carries an `Attachment`
  (a content address); the bytes live in the content-addressed blob store and
  are fetched separately, under the same permission as the post.
- **Nothing is derived.** Unlike a Poll, a Forum stores no computed output — the
  posts are the whole truth. There is no "latest message" cache held as an
  independent fact; it is always read from the post stream.
- Honest-by-default: an unknown attachment `mediaType` stays `nil`; a post with
  no body and only media has an empty body, never a fabricated placeholder.

## Connection

Pacific blocks bound, and how:

- **F Records:** a Forum is a *hub* Matter (kind `.forum`) plus one Matter per
  post (also `.forum`, with `forum: .ref(hub)`). The hub flattens its keyed maps
  into the flat `[String: FieldValue]` under `topic_area/<key>` and
  `member/<spaceID>` prefixes (see `ForumField`/`PostField`).
- **B Identity:** every post is authored by, and sealed with, its author's
  `SpaceID` credential. Members *are* Groups, addressed by their Identity space.
- **C Permissions:** visibility is gated per **(member, topic_area)**. A member
  may have `.full` disclosure on one topic_area and `.none` on another; revoking
  is instant. This is the mechanism by which "who sees which topic_area" works.
- **D Dictionary:** the `forum` shape registers `title`, `mls_group`, and the
  keyed `topic_area/*` and `member/*` fields; posts register `body`,
  `topic_area`, `forum`, and `attachment/*`.
- **E Network / MLS:** the Forum *is* a standing MLS group whose membership
  mirrors the `members` map. A new post seals to a Matter and fans out as an
  `Envelope` to the group's space; the blind relay never sees plaintext.
  Add/remove member = MLS add/remove proposal + commit (removal rotates keys).
- **A Search (`Indexable`):** `index_projections()` embeds every post body (the
  post stream *is* a Forum's searchable truth), federated under the same
  per-topic_area permissions. Attachments contribute any caption/transcript text
  (blob bytes aren't text); `topic_area` key + author ride as filter metadata.

Edges to other objects: **contains** TopicAreas and Posts (its own); **members
are** Groups (B); **may contain** a Poll (as a referenced post); is **subscribed
to** by Projects.

## Implementation

- **Matter projection.** Hub Matter (`.forum`): `title` (`.text`), `mls_group`
  (`.text`), `topic_area/<key>` → `.text(name)`, `member/<spaceID>` →
  `.text(role)`. Post Matter (`.forum`): `forum` (`.ref`), `topic_area`
  (`.text`), `body` (`.text`), `attachment/<n>` → `.blob(id:)`. A post's `at`
  comes from its Matter `history` provenance, not a duplicated field.
- **Storage.** Hub and posts go through `MatterStore.append`; the post stream
  for a topic_area is `project(kind: .forum, for: viewer)` filtered to posts
  whose `forum` ref matches and whose `topic_area` matches, ordered by
  provenance time.
- **Multimedia.** Bytes are written to the content-addressed blob store *before*
  the post Matter is appended; the Matter only stores the resulting blob id via
  `FieldValue.blob`. Same content yields the same id (dedup); fetch is lazy and
  permission-gated.
- **MLS group shape.** One MLS group per Forum. `members` map ↔ MLS roster is the
  binding invariant; `addMember`/`removeMember` issue MLS proposals + commits.
  Topic_areas are *not* separate MLS groups — they share the one group and rely
  on per-topic_area Permission gates (C) for partition. (Open question below.)
- **No agent hooks.** Unlike Project (CLA) or Topic (research loop), a Forum runs
  no agent; it is a passive store of human posts.
- **Honest-by-default.** Unknown stays `nil` (e.g. attachment `mediaType`); no
  derived/cached truth (no stored "last message", no computed output).

### Open questions

- **Topic_area isolation: permissions vs. crypto.** Today's design keeps one MLS
  group per Forum and partitions topic_areas with Permission gates (C). That
  means the *ciphertext* of every topic_area reaches every member's device, even
  ones they can't open — fine if C is trusted, weaker than per-topic_area MLS
  subgroups for true cryptographic compartmentalisation. Per-topic_area subgroups
  would be stronger but multiply key-rotation cost on every membership change.
  Unresolved.
- **Edit/retract semantics.** Posts are append-only; whether a retraction hides
  the prior post in the projection (and how that reconciles with peers who
  already synced it) is not yet specified.
- **Member removal & history.** A removed member keeps any plaintext their device
  already decrypted; key rotation only protects *future* posts. Whether the UI
  should make that limitation explicit is open.
