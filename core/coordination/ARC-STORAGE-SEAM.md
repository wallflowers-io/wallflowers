# The arc storage seam

The line login meets on. Three routes and one signing payload; core builds one
side, the arc builds the middle, the edge builds the other side, and none of us
has to wait for the others to start.

Written by the edge agent, 18 Sep 2026. Verified claims are marked; the two open
questions are at the bottom with a recommendation each.

## Why this exists

`tests/p1_resume_from_words.rs` proves the whole recovery claim end to end with
real cryptography — a device holding only the twenty-four words resumes a leaf
already in an MLS group and speaks, without publishing a commit. It proves it
against two in-process stand-ins, and says so: *"They prove the protocol closes,
not that the services exist. Neither service has these routes today."*

Those stand-ins are the specification. This document is them, written out.

## What already exists — please do not rebuild it

**In the wasm** (verified: 55 exports in `core-wasm/src/lib.rs`) — `recovery_words`,
`recovery_seed`, `locators`, `wrap_seal`/`wrap_open`, `head_seal`/`head_open`/
`head_check`, `history_build`/`history_peek`/`history_unpack`, `arc_write_sign`/
`arc_write_verify`, `chain_next_tag`, `mls_rekey*`. The browser can already do
every piece of a login except talk to a store.

**The relay half.** `Relay` in p1 is append-and-replay by opaque tag — the relay
never learns what a tag means and never opens a blob. The deployed relay already
does exactly that: `wss://arc.wallflowers.io/v1/relay`, `relay: live` on
`GET /v1/health` (verified 18 Sep). **Open question for the arc author: do chain
entries already have a home there?** If they do, half this seam is built and
nobody should write a second one.

**The gateway's shape.** `public` / `console_surface` / `gated` are already three
separate routers, and `/v1/box/*` already proves the pattern of a plane whose
routes are signature-authorised with the exact public path hashed into the
canonical string.

So what is missing is one thing: **a locator-addressed row store with
first-write-claims.**

## The surface

```
GET  /v1/store/nonce           -> { nonce }             single use; the arc spends it
GET  /v1/store/{locator}       -> { body, position }    ungated, by design
PUT  /v1/store/{locator}       body + acct pk + position + signature
                               -> 204 | 401 bad signature | 409 claimed | 409 stale
```

**GET is ungated on purpose.** An anchor nobody can watch is an anchor an arc can
move. The wrap's own unauthenticated read already concedes this deliberately, and
the reason it is safe is in `locator.rs`: a locator is an ADDRESS, not a
capability. Learning one lets you fetch ciphertext; nothing at a locator opens
without the seed or the PRF, and nothing at a locator can be overwritten without
`acct`.

**PUT is first-write-claims with a compare-and-set.** The arc holds no seed, so it
cannot verify that an opaque locator belongs to anybody — recording the first
claimant is the most it can do.

- the first PUT records the `acct` verifying key;
- a later write under a different key is refused — *locator is claimed by another key*;
- `position` must be strictly greater than the stored one, else *stale write*;
- the signature is checked with `locator::verify_write`, which core holds
  deliberately **so that the two sides cannot drift** — the arc should call core's
  rule, not reimplement it.

### The bytes that are signed

`locator.rs`'s `ArcWrite`, unchanged:

```
ACCT_SIG_DOMAIN \n audience \n <locator hex> \n nonce \n <body>
```

A newline in `audience` or `nonce` is refused, and that is load-bearing rather
than tidy: without it the encoding is not injective and one signature authorises
two different writes. The nonce comes from the arc, which is precisely the party
this defends against.

**`position` is deliberately NOT in the signature, and that is correct.** The
position is sealed inside the head; the arc declares it on GET; and
`head::open_declared` (head.rs:300) fails loudly if the two disagree —
*"the arc declared head position N and served position M"*. The seal binds it, so
the signature does not have to.

## Why these routes must be public

A device holding only the words has no membership credential to present. Joining
is what it is trying to do. The gateway's public router currently says:

```rust
// Public: identity/discovery + the membership ENTRANCE (how you join). Nothing else.
```

This is a widening of that sentence and should be written into it rather than
slipped past it. The existing public routes are credential-free **because they are
pre-account**. These are credential-free for a different reason: **authorisation
rides in the payload.** Same door, different justification.

## Two things to settle first, and which way each goes

> **Both settled, 17 Sep 2026, as recommended below.** `locator.rs` derives from the §06
> families — `pacific/identity/root/v1`, and `pacific/storage/{root,acct,wrap,head,history,
> index,chain}/v1` off the storage root — and carries `ArcWrite` with audience and nonce.
> `p1` imports `pacific_core::locator` for both and its stand-in signs `ArcWrite`. What
> follows is kept as the reasoning, not as an open question.

Neither file is simply right — each is ahead of the other in one dimension, which
is why this needs a meeting rather than a decision.

**1 · Label families — `p1` is ahead.** `docs/five-things.html` §06 (16 Sep) ruled
`pacific/identity/` and `pacific/storage/`, disjoint, and named the cost of mixing
them: another derivation version on every existing account when BYO lands. p1
follows the ruling; `locator.rs` uses `pacific/locator/v1` and
`pacific/arc-write/v1` and derives straight from the seed with no root fork.
**Recommend: adopt §06 in `locator.rs`.**

**2 · The signing payload — `locator.rs` is ahead.** Its `ArcWrite` doc states
plainly that binding only locator and body *"looked sufficient… It was not"*, and
names what each missing field costs: no audience → a signature captured at one arc
verifies at another; no nonce → rollback. p1's stand-in signs
`pacific/storage/put/v1 || loc || position || sha256(blob)`, which is the shape
that analysis superseded. **Recommend: adopt `ArcWrite`, and update the stand-in.**

Then `p1` imports `pacific_core::locator` for both and stops carrying a second
definition — which is the failure `locator.rs`'s own doc says nothing currently
catches.

**This is free until the first PUT.** No account has a locator-addressed record
anywhere (`pacific-account.js` still addresses `/auth/users/{pk}`, five call
sites). First-write-claims means the first write fixes an address for that account
permanently.

## Who builds what

| | |
|---|---|
| **core** | settle the labels; fold p1 onto the module; keep `verify_write` as the one rule both sides call |
| **arc** | the three routes, the nonce issue-and-spend, the row store; confirm whether the chain already lives on the relay |
| **edge** | the client, and a local stand-in implementing exactly these three routes so the browser path is testable before the arc ships — announced as a stand-in, per the house rule that a fixture must say so |

Order: **labels → routes → client.** The first blocks everything and costs no
code.

## What this does not cover

Reading back past the join epoch (needs the retained `pacific/archive/v1` content
key, which has no writer), leasing (does not exist anywhere in the tree), and
rotation ordering. p1 §5 pins the forward-secrecy boundary that read-back would
have to cross.

---

If `core/coordination/` is the right home for this, move it there — it is an
interface contract and those travel with the core. I left it at the root rather
than put a file in your tree uninvited.
