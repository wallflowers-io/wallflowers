# Events — the extraction contract

Derived from the consumer, not the other way round. The EVENTS ribbon
(`ui/App/Life/Events/`) is a chronological column inside a 1-hop intersection filter;
everything below exists because the ribbon or the filter cannot work without it.

Nothing lifts commitments out of text any more: model extraction from notes and chats has been
removed, along with the Python prototype this contract was first written against. The shape
below still binds anything that writes Event nodes. Today the only writer is the object
projection in `arc-resolver` (`src/project.rs`), and it implements only part of this shape —
`coordination/delta-graph.icd.json` records which edges are implemented.

## What a producer emits

**Temporal commitments** implied by the input — not "events mentioned". A commitment is
something that binds entities at a time:

- "Hope is driving to Bristol on Saturday" → one commitment, Saturday, participants
  Hope (subject) · Bristol (at) · the car (with)
- "the gallery opens Tuesdays and Thursdays 1000–1800" → one *recurring* commitment

## Output per commitment

| Field | Required | Notes |
|---|---|---|
| `title` | ✅ | terse — "Gallery opening", not a sentence |
| `dtstart_ms` | ✅ | absolute epoch ms; relative refs resolved against the episode's `reference_time` |
| `tzid` | ✅ | IANA zone. A local-time commitment anchored in UTC drifts at every DST boundary |
| `rrule` | — | RFC 5545 pattern, e.g. `FREQ=WEEKLY;BYDAY=TU,TH`. Absent = one-off |
| `duration_ms` | — | length of an occurrence. Absent = a point in time, **not** a zero-length span |
| `participants[]` | ✅ | non-empty; each `{name, category, role}` |

`participants[].category` ∈ `people · places · groups · projects · events`
`participants[].role` ∈ `subject · at · with · about` — **at least one `subject`**.

## Two hard rejection rules

Both exist because of the consumer. Enforce where the event is produced; do not emit and hope.

1. **No resolvable time → drop the commitment.** `dtstart_ms` is the ribbon's only sort
   key. "Sometime next month" is not a point and cannot be sequenced. Emitting it with a
   guessed date is worse than dropping it — it puts a fiction in the timeline.
2. **No participants → drop the commitment.** The filter matches events by 1-hop adjacency
   to locked terms. An event with no edges is unreachable under *every* filter, so it can
   never be seen. A disconnected event is not a degraded event; it is invisible.

## Why this shape: Davidsonian event semantics

The Event-as-node design is not a workaround for `Fact` being binary. It is the standard
formal treatment of action sentences, and naming it explains most of the decisions below.

Donald Davidson's insight (1967) was that action sentences quantify over an **event
variable**. "Hope drove to Bristol on Saturday" is not a three-place relation — it is:

> ∃e. drive(e) ∧ agent(e, Hope) ∧ goal(e, Bristol) ∧ time(e, Saturday)

The event `e` is a thing in its own right, and every participant attaches to it by a
separate binary predicate. The **neo-Davidsonian** refinement takes this further: the verb
contributes only the event type, and *every* argument — including the agent — is severed
onto its own role predicate. That is exactly our schema:

| Davidson | Ours |
|---|---|
| the event variable `e` | `Entity(type: "Event")` |
| `drive(e)` | the Event's label |
| `agent(e, Hope)` | `Event —subject→ Hope` |
| `goal(e, Bristol)` | `Event —at→ Bristol` |
| `time(e, Saturday)` | `valid_at` on the fact |

Three practical consequences fall straight out of it:

1. **Adjuncts are unbounded, so participants is a list.** English lets you stack arbitrarily
   many prepositional phrases onto one clause; each is just another conjunct on `e`. There
   is no fixed arity to model, which is why an Event has N participant facts rather than a
   fixed set of columns.
2. **Roles are thematic, not syntactic.** `subject` means **Agent** — the entity that
   performs or is committed — *not* the grammatical subject. Passive voice separates them:
   in "the car was picked up by Hope", the syntactic subject is the car but the agent is
   Hope. Extraction must follow the thematic role or passives invert the commitment.
3. **The event node is a nominalisation.** "Drive to Bristol", "Northwind review" — English
   already nominalises predicates, which is what lets an Event have a name at all. It also
   creates a known failure: a nominalisation absorbs its modifiers, so "the Northwind
   review" reads as one noun phrase and the Project gets swallowed into the Event's name
   instead of being extracted as a participant.

### Role vocabulary vs. thematic roles

Our four roles are a deliberately coarse projection of the thematic-role inventory:

| English form | Thematic role | Our role |
|---|---|---|
| subject (active) | Agent | `subject` |
| direct object | Patient / Theme | `with` |
| *to X* | **Goal** | `at` |
| *from X* | **Source** | `at` — *conflated* |
| *at / in X* | Location | `at` |
| *with X* | Instrument / Comitative | `with` |
| *about X* | Topic | `about` |
| *on Saturday* | Time | `valid_at` |

`at` currently collapses Goal, Source, and Location. That is a known lossy edge: in "drive
to Bristol from the garage" both places take `at`, so direction is not recoverable. Split
`at` into `to`/`from`/`at` when direction starts to matter.

## How it lands in the graph

An event is an **n-ary commitment** — a reified hyperedge, because `Fact` is binary:

```
Entity(type: "Event", label: title)
  properties: { dtstart_ms, tzid, rrule?, duration_ms? }
  valid_at / invalid_at  ← when the COMMITMENT held, NOT when it recurs

  ─[subject]→  Entity(Person "Hope")
  ─[at]→       Entity(Place  "Bristol")
  ─[with]→     Entity(Thing  "the car")
```

Each participant is exactly one fact from the event, which is what makes the filter's
1-hop intersection cheap.

### Keep recurrence off the bi-temporal axis

`valid_at`/`invalid_at` mean *when this commitment was true* — the period the gallery's
opening-hours policy stood. They are **not** the occurrence times. Collapsing the two
would make a weekly commitment look like it stopped being true every Wednesday and break
as-of queries.

### Next occurrence is never stored

It is relative to *now*, so it is stale the moment it is written. `EventsSource` expands
`DTSTART + RRULE` in `tzid` at read time to produce the ribbon's `startMs`. The graph
stores only the pattern.

## Triggers

One input:

- **GroupObject delta** — a delta arrives, subject to debounce / rate limit.

Notes and chats were inputs only to model extraction, which has been removed.
