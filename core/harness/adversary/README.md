# harness/adversary — the adversaries (Track 3, build step 03)

Real sockets, real crypto, no simulation. These three demonstrate capabilities that
are **true today** because the legs they exercise are real: the semaphore relay, its
global sequence, R2's content addressing, and the `seal.rs` construction. They are
the acceptance tests the protocol fixes are meant to flip.

| Case | Gate | What it demonstrates | Real leg |
|------|------|----------------------|----------|
| **E10** | G0 | A relay/eavesdropper opens a pairing **intro blob** using only the routing tag it was published under. `node.rs:319/:632` seal it with `conn_secret == dest_tag == the tag`, and `tag_hex` is `hex::encode` (the raw tag), so the seal key is a pure function of the routing address. | real relay + real crypto + a blob **the real pairing path published** |
| **E11** | G4 | An observer on **two unrelated tags** recovers their exact interleaving, because `seq` is one global monotonic counter (`Hub::next_seq`, `meta['last_seq']`). A correlation channel for "these two tags are one conversation". | real relay |
| **E12** | G5 | R2 keys are `sha256(content)`, so whoever holds the bucket can **confirm** whether specific known bytes are present by hashing them — a membership oracle. Also shows sealed content dedups **only byte-identically** (same plaintext, two epochs → two keys). | real boto3 client vs. a modelled content-addressed bucket |

## Run

```sh
PY=.venv/bin/python
cd harness/adversary

export PATH="$HOME/.cargo/bin:$PATH"  # the emitter is a cargo build; cargo is not on PATH

$PY run_all.py                      # all three, with a rich render of what each LEARNED
$PY test_e10_intro_tripwire.py      # E10 standalone (spins up the relay + the emitter)
$PY -m pytest -v                    # E10 + E11 + E12 as tests
```

Point at an already-running relay with `RELAY_URL=ws://host:port`; otherwise the
harness spawns `arc/target/debug/semaphore` on a free localhost port (in-memory
store) and tears it down after.

## The E10 tripwire — green because the bug is present

`test_e10_intro_tripwire.py` is **expected to pass today, and a pass means the
vulnerability is still open**. Most of its tests are *inverted*: they assert that a
live hole is still live. They are the acceptance test for The Build's **D-E** (seal
the intro to a real recipient key) and **A5** (`seal()` rejecting `tag == secret`),
so the day those land the file goes **red — and that red is the deliverable**.

A failure here is not a regression. The message says which of two things happened, in
words nobody can mistake for a bug report:

| banner | meaning | what to do |
|---|---|---|
| `G0 TRIPWIRE CLEARED — THE INTRO FIX HAS LANDED. UPDATE THIS TEST.` | an intro blob no longer opens under a key derived from its own routing tag | delete/xfail the inverted tests, keep the three that are not inverted, add the positive gate, flip G0 in `execution.html` §02 |
| `NO VERDICT` | the harness could not obtain a **live** intro blob, so it has no answer to give | fix the cause it prints. **Do not add a fixture fallback** |

Three tests are **not** inverted and stay green either way: the narrowing
(`test_opening_an_intro_does_not_confer_group_access`), the negative control
(`test_the_tripwire_itself_can_fail`), and the no-fallback rule
(`test_the_tripwire_refuses_to_grade_without_a_live_blob`).

## What makes the verdict honest

A tripwire is only worth its runtime if it can detect the fix it gates. Two defects
destroyed that here; `retired/README.md` keeps the originals and the diagnosis.

**1. It must attack the CALL SITES THE FIX WILL CHANGE.** The old emitter built an
`IntroPayload` by hand and called `seal::seal(&p, &tag, &tag)` itself. That blob stays
openable from its tag for as long as anyone keeps writing that line — so the tripwire
would have gone on reporting "still vulnerable" after the vulnerability was closed.
`intro_emit` now drives the real verbs and lets **node.rs** publish:

| scenario | verb | call site |
|---|---|---|
| `pair` | `Node::pair_scan_why` | `node.rs:319` `seal::seal(&payload, &dest, &dest)` |
| `groupjoin` | `Node::group_add_member` | `node.rs:632` `seal::seal(&payload, intro_tag, intro_tag)` |

Both are covered, because the `IntroPayload` type is shared and the fix therefore has
**two** call sites. A fix applied to only one of them leaves the other test green, and
that is exactly what the second test is for.

**2. There is NO recorded-blob fallback.** The old runner silently fell back to
`fixtures/rust_intro_vector.json` when the Rust example was absent, and reported the
source as though it were live. A landed fix looks exactly like a failed emitter run
from out here — `seal()` starts refusing `tag == secret`, so the emitter exits
non-zero — and the fallback would have reported "still vulnerable" forever, and
confidently. Now `emitter.py` raises `EmitterUnavailable` and the tripwire produces
**no verdict**. The exception carries a `phase` (`build` / `run`), so a *run* failure
inside the seal is reported as the fix landing while a broken cargo build is reported
as a broken cargo build.

Two further things keep the green meaningful: `why` and `arc` are **per-run nonces**,
so recovering them out of relay ciphertext cannot be a restatement of something the
adversary was handed; and the operator-side sweep must **discriminate** — a pairing
puts an intro blob and a stack of prekey blobs on the relay, and exactly one of them
is keyed by its own address (measured: 1 of 10).

Verified both ways. Against the tree as it stands, 7/7 pass. Against a copy of
`pacific-core` with `node.rs:319/:632` sealed under a non-tag secret — D-E's shape —
the three inverted tests fail with the `G0 TRIPWIRE CLEARED` banner and the other four
still pass.

## The emitter is NOT a cargo example

`intro_emit/` is a **standalone crate** — its own `[workspace]`, outside `core/` —
that path-depends on `pacific-core`. It compiles the same library through the same
public API and enters nobody else's build. It is deliberately not
`core/pacific-core/examples/`, because `cargo test` builds a package's examples, so a
helper put there sits on **every other track's build path**: a compile error in the
adversary's scaffolding would turn the core suite red for people who have never heard
of E10. The retired `seal_intro_vector.rs` did exactly that.

It builds itself on first use (about a minute cold, incremental after). `emitter.py`
mirrors `core/.cargo/config.toml`'s machine-local `[patch]` paths into the crate,
because patches apply at a workspace root and this crate is its own root — without
that it would compile a *different* `mls-rs` than the tree does, and then its bytes
would not be the tree's bytes.

## Interface with Track 2

Track 2 owns the harness state objects (`RelayState`, `R2State`) and the render, which
did not exist when this was written. Track 3 codes against a thin local interface
(`knowledge.py`: a growing `Knowledge` set per adversary) and reaches into none of
Track 2's files. When Track 2's modules land, an adversary can be handed one in place
of a live socket; the `Knowledge` set is the shape the render consumes.

## Files

- `intro_emit/` — the Rust emitter: a standalone crate that drives the REAL pairing path
- `emitter.py` — build + run it. Raises `EmitterUnavailable`; **never** falls back
- `pacific_seal.py` — HKDF + XChaCha20-Poly1305 seal/open + IntroPayload CBOR decode
- `relay_client.py` — minimal real WebSocket client (Pub/Sub/drain)
- `relay_harness.py` — spawn/attach the real semaphore relay (`store=True` for the operator view)
- `knowledge.py` — the thin "what it learned" set the render consumes
- `eavesdropper.py` — E10 adversary · `ordering.py` — E11 · `r2_oracle.py` — E12
- `showcase.py` — end-to-end runners · `run_all.py` — render all three
- `test_e10_intro_tripwire.py`, `test_adversaries.py` — the tests
- `retired/` — the bespoke example and the fixture fallback, kept with the note on why they were wrong
