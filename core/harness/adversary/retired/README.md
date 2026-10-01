# retired — the two files that made the E10 tripwire unable to detect its own fix

Kept, not deleted, so the review that found this has something to point at.

## `seal_intro_vector.rs` — the bespoke example (was `core/pacific-core/examples/`)

It built an `IntroPayload` by hand and called `seal::seal(&payload, &tag, &tag)` itself.
Real Rust, real crypto, real CBOR — and **useless as a tripwire**, for two separate
reasons:

1. **It could not detect the fix it gated.** D-E changes `node.rs:319` and
   `node.rs:632`. This file contains neither. Its blob would have stayed openable from
   its tag for exactly as long as somebody kept that `seal::seal` line in it, so the
   tripwire would have gone on reporting "still vulnerable" after the vulnerability was
   closed. The acceptance test for a change has to exercise the thing the change
   touches.

2. **It lived under `core/pacific-core/examples/`.** `cargo test` builds a package's
   examples, so one track's helper sat on every other track's build path: a compile
   error in the adversary's scaffolding would have turned the core suite red for people
   who have never heard of E10.

Replaced by `../intro_emit/` — a standalone crate (its own workspace, outside `core/`)
that drives `Node::pair_scan_why` and `Node::group_add_member` and lets node.rs publish.

## `rust_intro_vector.json` — the fixture fallback

`showcase.rust_intro_blob()` ran the example, and **silently fell back to this recording
when the example was absent**, then reported the source as though it were live.

That is precisely what a landed fix looks like from outside: `seal()` starts refusing
`tag == secret`, the emitter exits non-zero, the fallback fires, and the tripwire
reports "still vulnerable" — forever, and confidently. A pre-recorded blob within reach
of a tripwire is not a convenience, it is a way of laundering the signal.

Replaced by `../emitter.py`, which raises `EmitterUnavailable` and produces **no
verdict** rather than a false one. `test_the_tripwire_refuses_to_grade_without_a_live_blob`
asserts that neither of these files is back within reach.
