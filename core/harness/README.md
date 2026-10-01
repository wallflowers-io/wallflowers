# harness — the two state objects, the object catalogue, and the render

Track 2 of `docs/execution.html`: build steps 01 and 02 of `docs/the-harness.html`
§06. The relay and the bucket modelled as what they are — databases with insert
rules — property-tested against the real thing, and rendered live.

`harness/objects/` adds §01's object model, cut over to the REAL catalogue: the
kinds and their wire type ids are parsed from `core/pacific-core/src/object.rs`
and the 88 ops from `core/coordination/delta-graph.icd.json`. It READS those two
files; it does not link against core, and it does not run a reducer.

```
harness/
  wire.py              the frame vocabulary, mirrored from arc/planes/relay/src/wire.rs
  sigv4.py             a Python port of sigv4.rs — a second opinion on a hand-rolled signature
  state/relay.py       RelayState (store.rs), SeqSource + RelayHub (lib.rs)
  state/r2.py          R2State, Budget, R2Relay (budget.rs + media.rs)
  objects/catalogue.py the 12 kinds and the 88 ops, parsed from core/ — never transcribed
  objects/model.py     GroupObject: the two folds, the authority gates, the refusals
  objects/cases.py     O1–O10, each mirroring a Rust test by name
  objects/test_objects.py  the same rules as pytest, plus the unit-level edges
  render.py            rich.live.Live, the §04 panels, the BACKEND + CATALOGUE footer
  cases/f_cases.py     F13, F14, F15 against the model
  cases/rust_parity.py every Rust unit test we mirror, re-run in Python
  live/relay.py        a real `semaphore` process + a read-only window on its SQLite
  live/driver.py       a websocket client that speaks the same frame dicts
  live/parity.py       F13/F14/F15 driven against the real relay and diffed
  live/fuzz.py         the property test: seeded random walks, diffed every step
```

## Running it

```sh
cd <repo>
.venv/bin/python -m harness units       # 29 Rust unit tests, re-run in Python
.venv/bin/python -m harness cases       # F13 F14 F15 against the model, no I/O
.venv/bin/python -m harness objects     # O1–O10 over the real object catalogue, no I/O
.venv/bin/python -m harness live        # the same three against a real semaphore
.venv/bin/python -m harness property    # random walk + eviction rounds, diffed
.venv/bin/python -m harness render      # the F- and O-cases under rich.live.Live
.venv/bin/python -m harness all         # all of it; exit 0 only if everything is green

.venv/bin/python -m pytest harness/     # the same rules as pytest: adversaries + objects
```

The live commands start their own `semaphore` on a free port with a scratch
`RELAY_STORE`, and reap it. Nothing needs to be running first, and nothing is
left behind but a temp directory.

## What is real here

| leg | real? |
|---|---|
| the relay, its store, the wire, the eviction sweep | **yes** — a real process, diffed row by row |
| the presigner | **yes** — the relay's own URL, reproduced byte for byte |
| R2 itself (ListObjectsV2, OverBudget) | no — needs a live bucket; the model carries it |
| the object CATALOGUE — kinds, type ids, op ids, authority, fold, status | **yes** — parsed from `core/`, and a disagreement between the two source files fails the parse |
| the object model — folds, gates, refusals | no — the RULES are real, the reducers are not. No reducer runs in `objects/` |
| people, devices | no — step 04 (`ModelDevice`); the PEOPLE panel says so |

The footer says which, every run. `BACKEND: model — proves the design, not the
code`. A run that drives the live relay says that instead.

## The interface Track 3 codes against

Import from `harness.state`; nothing else in here is a stable surface.

```python
from harness.state import RelayState, RelayHub, Conn, Retention, SeqSource, Tables
from harness.state import R2State, R2Relay, Budget, Verdict, MediaError
from harness import wire
```

### RelayState — everything the relay knows

```python
s = RelayState()
s.append(tag, seq, body, commit=False) -> bool   # False = the slot was taken; NOTHING changed
s.replay(tag, since)   -> [(seq, body)]          # seq > since, EXCLUSIVE of the cursor
s.floors(tags)         -> {tag: floor}           # tags that never evicted are ABSENT
s.evict(now_us, window_us, max_per_tag) -> int   # floors written BEFORE the deletes
s.counts()             -> (tags, blobs)
s.resume_seq()         -> int                    # meta['last_seq'], the GLOBAL high-water
```

Adversary reads, which is why this class exists:

```python
s.tables()        -> Tables(blob, commit_slot, retention_floor, meta)  # the four tables entire
s.sizes()         -> [(tag, seq, size)]        # the eavesdropper's metadata, no bodies
s.interleaving()  -> [(tag, seq), …]           # E11: a TOTAL order across every tag
```

`Tables` compares with `==` and explains itself with `.diff(other) -> [str]`.

### RelayHub — the relay as a process

```python
hub  = RelayHub(state=None, retention=Retention(), seq=SeqSource())
conn = hub.connect("eve")                       # a socket; conn.out is the frames it received
hub.subscribe(conn, tags, since=0, v=0) -> (replayed, gaps)
hub.publish(conn, tag, blob, commit=False) -> (seq, fanout, ok)
hub.evict(now_us=None) -> removed
conn.drain() -> [frame, …]                      # take everything delivered since last drain
hub.taps.append(lambda conn_id, frame: ...)     # every frame the hub emits, as it emits it
```

Frames are plain dicts (`harness.wire`), identical in shape to what comes off a
real socket — so an adversary written against `conn.out` also works against
`harness.live.driver.Client`, which returns the same dicts from a real websocket.

Three behaviours a passive observer depends on, all mirrored exactly:

* a rejected commit is `Ack{seq: 0, ok: false}`, is **not** stored and is **not**
  fanned out;
* fanout happens *before* the publisher's Ack, so a publisher subscribed to its
  own tag sees `Msg` then `Ack`;
* `Gap` is sent before that tag's backlog, only when `v >= 1` **and**
  `since < floor`.

### R2State / Budget / R2Relay — the bucket and the meter

```python
r2 = R2State()
r2.put_bytes(ciphertext)          -> Object      # key = sha256 hex; identical bytes collapse
r2.list_objects_v2(continuation)  -> ListPage    # LIST_PAGE_SIZE 1000
r2.measure(budget_max)            -> (bytes, pages, capped)   # MAX_LIST_PAGES 64 reads as FULL
r2.operator_view()                -> [(key, size, last_modified)]   # all an R2 operator sees
r2.confirms(candidate_bytes)      -> bool        # E12: the bucket is a confirmation oracle
r2.dedup_ratio([b1, b2, …])       -> (distinct, offered)      # sealed content dedups only
                                                 # byte-identically — not across epochs

b = Budget(max_bytes=DEFAULT_BUDGET_BYTES, stale_after_secs=2700)
b.record(measured_bytes, now_secs)               # a measurement CLEARS `reserved`
b.reserve(length, now_secs) -> Verdict.OK | OVER_BUDGET | STALE

relay = R2Relay(budget=b)
relay.presign_put(key, length) -> url | MediaError   # bad_key → too_large → budget, in that order
relay.presign_get(key)         -> url | MediaError
relay.presign_list(token)      -> url
```

`MediaError`'s *value* is the wire token (`"budget_stale"`, `"too_large"`, …), so
`wire.media_err(key, MediaError.TOO_LARGE.value)` is the frame the relay sends.

### The object catalogue — `harness.objects`

Import from `harness.objects`. It reads two files in `core/` and nothing else.

```python
from harness.objects import GroupObject, Leaf, Rejection, catalogue

cat = catalogue()                  # parsed once per process
cat.line()                         # '9 channels · 88 ops · 45 implemented / 43 specified · icd 1acf7999…'
cat.kind("project").implemented    # 7 of 20 — read from x-graph.status, not typed here
cat.kind_for_type_id(16)           # raises ReservedTypeId: 16 is Poll, folded away
cat.check()                        # [] — the Python half of icd.rs
```

```python
notes = GroupObject("notes", "group", owner="ada",
                    roster=["ada/phone", "ada/laptop", "bo/sim"])

notes.type_id          # 18                  the wire id, from object.rs
notes.leaves           # 3                   membership is per LEAF
notes.people           # ('ada', 'bo')       authority is per PERSON (A2)
notes.kind.implemented # 9 of 26

d = notes.author("ada/laptop", "setCover", {"media": "blob:x"})   # the owner's 2nd device
notes.deliver(d, "ada/laptop")                       # Verdict(accepted=True)

notes.deliver(notes.author("bo/sim", "setCover", {}), "bo/sim").rejection
# Rejection.UNAUTHORIZED — `group.setCover` is owner-authority and bo is not the owner

notes.deliver(notes.author("ada/phone", "setMemberRole", {}), "ada/phone").rejection
# Rejection.NOT_IMPLEMENTED — status=specified in the ICD: declared, not built
```

An op that the kind does not declare fails at the point of AUTHORING, which is
how a Conversation refuses the room vocabulary:

```python
chat = GroupObject("chat", "conversation", owner="ada", roster=["ada/phone", "bo/sim"])
chat.author("ada/phone", "post", {"body": "hi"})     # fine — forum.post, $ref'd
chat.author("ada/phone", "setRoom", {})              # UnknownOp: conversation declares
                                                     # forum.post/react/receipt/vote
```

The two folds, and the one question worth asking of each:

```python
from harness.objects import converges, replica

converges(chat,  commutative_deltas)   # (True,  {one digest})       — any order
converges(notes, sequenced_deltas)     # (False, {three digests})    — order is the state
```

`converges` returns `(all_agree, {digest: the first order that reached it})` over
every permutation. A commutative set must converge; a sequenced spine must not,
and a model that made both converge would have modelled neither.

### What `objects/` will not do

* It will not guess. `x-graph` says which graph fields an op writes; it does not
  say what value they take (that is the projector, in another repo), so the fold
  records **which op last wrote each field** and stops there.
* It will not invent a slot. The commutative LWW slot is supplied by the
  scenario, because the ICD's clause for a commutative op is almost always an
  exclusion — `reactions carry no extractable fact`.
* Its DeltaIds are SHA-256 over canonical JSON, not the Rust's canonical CBOR.
  They are stable and comparable to each other, and to nothing in `core/`.
* No reducer runs. `objects/` enforces what the catalogue DECLARES. The footer
  says so on every frame, under the BACKEND line.

### The render

```python
from harness.render import Scene, Dashboard, AdversaryView, PersonRow, ObjectRow

class Eve:                      # Track 3's adversary, rendered without this
    name = "EVE"                # module knowing anything else about it
    def rows(self): return [("tags seen", "6"), ("plaintext opened", "1  ← intro")]

with Dashboard(Scene(backend="model", hub=hub, bucket=r2, adversary=Eve())) as d:
    d.update()
```

Inside a case, `Run.expose(hub=…, bucket=…, budget=…, adversary=…, objects=…,
catalogue=…)` hands those objects to whatever is rendering, and
`Dashboard.on_step` is the callback every case accepts as `on_step=`.

The OBJECTS panel (`ObjectRow`, or `ObjectRow.of(group_object)`) is the §04
GROUPS panel with the catalogue behind it — kind and wire type id, the roster
folded to people, the epoch, and the implemented/specified split of that kind:

```
┌─ OBJECTS ─────────────────────────────────────────────────────────────────┐
│ notes group·18        1 person · 1 leaf · e0   9/26 implemented           │
│ build project·22      1 person · 1 leaf · e0   7/20 implemented           │
│ chat  conversation·26 2 people · 2 leaves · e0 4/4 implemented            │
│ atlas field·20        1 person · 1 leaf · e0   0 ops — declared, not built │
└───────────────────────────────────────────────────────────────────────────┘
BACKEND: model — proves the design, not the code        7/7 checks
CATALOGUE: 9 channels · 88 ops · 45 implemented / 43 specified · icd 1acf79991817
           declarations enforced; no reducer runs here
```

Two footer lines, not one. The richer object model is exactly where a harness
starts to look more real than it is, so the catalogue line states where the ops
came from and that nothing behind them ran.

## What the tables made visible

`seq` is **globally monotonic, not per-tag**. There is one `meta['last_seq']`, and
the age sweep cuts `WHERE seq <= cutoff` across every tag at once. So anyone who
sees seqs can totally order writes across the whole system, and an observer on two
tags learns their exact interleaving — a correlation channel for deciding that two
tags belong to one conversation. Per-tag sequences would leak strictly less. That
is E11, and `RelayState.interleaving()` is it in one call.

## What the catalogue made visible

Four things that were prose, or nothing at all, before the ops were read in:

* **`project` is 7/20, not 6/20.** Every number in this README was parsed. That
  is the argument for parsing them.
* **A non-owner member of a Group can author nothing that is built.** All nine
  implemented Group ops are owner+sequenced; every any-member op on the kind —
  `memberJoined`/`memberLeft`, the wallet attestations, the whole notebook — is
  `status: specified`. Worth knowing before writing a scenario in which a member
  acts.
* **Only `conversation`, `event` and `post` are built whole.** `group` is 9/26,
  `project` 7/20, `place` 4/9, `thing` 2/4.
* **RATIFY is undocumented.** `RATIFY_PROPOSE/VOTE/CLOSE` (`0xF000`–`0xF002`) are
  recognised by the Coordinator on **every** object, ahead of the type's own op
  table, and appear in no `ops()` table and no ICD channel. `icd.rs` anchors on
  `ObjectType::ops()`, so it cannot see them — three ops on the wire, one of them
  owner-sequenced, that the conformance test structurally cannot check. This
  harness cannot see them either, for the same reason, and says so in O10.

## Conventions worth keeping

* Cases carry the **Rust case names** (F13, F14, F15; O1–O10 carry the name of
  the Rust test each mirrors) so drift between the two implementations is visible
  rather than comfortable.
* Live runs follow the relay's seq numbers (`hub.publish(..., seq=observed)`)
  because only the relay's clock can mint them. Everything else — accept or
  refuse, what survives, what the floor is, who hears a Gap — the model predicts,
  and a disagreement is a finding.
* `harness/` is a sibling of `core/` and `arc/`: it drives both and belongs to
  neither.

## Overlap with Track 3, worth collapsing later

`harness/adversary/` (Track 3) carries its own `relay_harness.py` and
`relay_client.py`, which do the same two jobs as `live/relay.py` and
`live/driver.py` here. Two copies of "start a semaphore, talk to it" will drift.
`live/relay.py` additionally reads the relay's SQLite store, which is what makes
the table diff possible, so that is the one to keep — but the choice is the
executive's, not this track's, and nothing has been changed on Track 3's side.
