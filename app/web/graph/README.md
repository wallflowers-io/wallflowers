# graph — the WallFlowers graph, driven by hand

WallFlowers is a **graph**. Each node is an MLS GroupObject; each edge is a relationship its
objects declare; a person's WallFlowers is the subgraph they are a member of. This page draws
that graph, and mints into it.

```
app/door/run.sh         http://127.0.0.1:8233 — the door image, and its relay
./serve.sh              http://127.0.0.1:8232/graph/
```

The document root is `app/web`, not `graph/`, so the page reads the account module, the shared
op bindings and the staged wasm by relative path. Run `app/web/build-wasm.sh` first if
`docs/core` is empty.

## Where the keys are

**Not here.** The browser holds a session id and nothing else. Every write goes to the door,
which runs the native `Node` in a process per persona: the seed, the MLS state and the signing
key never enter JavaScript. `core-wasm` is still loaded, for `ops_on()` — the catalogue, which
is not a secret.

## The four rails

| | |
|---|---|
| **left** | the personas the door's store holds. Clicking one acts as them |
| **canvas** | every object, coloured by kind, with the edges between them |
| **bottom left** | what the Ego can create |
| **bottom right** | what the Ego may legally do to the Target, by ICD channel |
| **right** | the fold of whatever is selected, and every crossing into the core |

**Ego and Target.** The Ego is the persona whose door the page is acting through. The Target is
a person, an object, or an edge. A person Target resolves to the pairwise log the two of them
hold, so the Actions panel answers "what may these two do to each other".

## Create

Product words on the left, kinds in the core. Which kinds mint is `mint::classify`, served at
`/v1/kinds`; which fields a mint carries is `profile_args`, served at `/v1/draft/:kind`; which
of those the op requires is the ICD, through `wallflowers-ops.js`.

| button | kind |
|---|---|
| Site | `group` |
| ChatRoom | `forum` — name-only; its name rides the MLS context |
| Article | `post` |
| Listing | `thing` |
| Event | `event` |
| Project | `project` |
| Connection | `contact` — **paired, not minted** |

`mint::classify(Contact)` answers `Paired`: a connection is formed by two people, so the button
takes a person rather than a sheet. `POST /v1/connect` drives both doors — the peer's bundle,
the Ego's scan (group, Welcome, vertebra), the peer's drain and accept — and the connection
lands on both spines.

## Actions

Only what is legal, grouped by the ICD channel that declares it. Four gates, each the core's:
the kind declares the op (`authoring::catalogue`, via `ops_on()`), a door writes it
(`reachable`), the Ego is on the roster, and an owner-sequenced op needs the owner. A refusal is
not a button, so it is not drawn — the empty state says which gate closed.

**They list; they do not author.** Authoring from this panel needs an arg sheet per op and a
door route that takes one.

## Not built here

- **Authentication.** `POST /v1/session` opens a persona by key with no proof at all. The door
  binds loopback and says so at boot.
- **Joining an object.** A second member is an MLS Add and a Welcome; only pairing does that
  round trip today.
