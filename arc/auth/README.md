# auth: the account store

Accounts for WallFlowers, served at arc.wallflowers.io/auth (the passkey RP ID is still
`kenjin.cc`, so that name stays in code and tests). An account is an **Ed25519 identity public key** — the
one a person owns, derived from a seed they hold and can write down as 24
words. This service stores three sealed blobs for that key and reserves slugs
for it. It cannot read any of them, and it never sees the passkey.

## The model, in four sentences

The seed is the root. The passkey is a **local unlock** that opens a wrapped
copy of the seed on a device, and it never reaches this service — which is why
there is no WebAuthn here, no ceremony, no credential table and no session
cookie. The Arc knows an account by its public key and challenges it on that: a
nonce we issue, an Ed25519 signature over it, single use. Everything else is
three blobs and a slug table.

```
GET  /auth/challenge                 a nonce, single use, 120s
PUT  /auth/users/<pk>                store the wrap        signed
GET  /auth/users/<pk>                serve the wrap        UNAUTHENTICATED
PUT  /auth/users/<pk>/history        store the history     signed
GET  /auth/users/<pk>/history        serve the history     signed
GET  /auth/users/<pk>/history/meta   sizes and timestamps  signed
PUT  /auth/users/<pk>/head           move the anchor       signed + CAS
GET  /auth/users/<pk>/head           serve the head        UNAUTHENTICATED
GET  /auth/users/<pk>/head/meta      position and digest   UNAUTHENTICATED
GET  /auth/users/<pk>/account        what we hold for you  signed
PUT  /auth/users/<pk>/seals/<node>   move a seal counter   signed + CAS (D-34 (c))
GET  /auth/users/<pk>/seals/<node>   the counter held      signed
POST /auth/users/<pk>/sites          reserve a slug        signed
PUT  /auth/users/<pk>/sites/<s>/host bind it to a Host     signed
GET  /auth/site/check?slug=          is it free?           free
POST /auth/site/hold                 hold it during signup free
POST /auth/site/release              drop your own hold    free
GET  /auth/site/<slug>/host          its Host              UNAUTHENTICATED (the Arc asks)
GET  /auth/health
GET  /.well-known/passkey-endpoints  where passkeys are    public
                                     made and managed
```

## The passkey endpoints document

`/.well-known/passkey-endpoints` is the W3C discovery document a credential
manager fetches to find out where an RP makes and manages passkeys. Our RP ID is
`kenjin.cc` for every account on every Arc (D2), so there is one of these, and
the URLs in it are built from `PUBLIC_ORIGIN` rather than frozen, so a staging
deployment names its own pages.

It is published for its **optional third member**. Deriving an encryption key
from a passkey's PRF turns a replaceable credential into a non-replaceable
encryption root — delete the passkey, lose the data — and the credential manager
offering the delete button cannot warn anyone, because it has no idea the site
uses PRF at all. `prfUsageDetails` is the field that closes that gap, and it
points at `/passkey`: the seed is the root, the PRF only wraps it, the 24 words
*are* the seed, so deleting a passkey here costs a local unlock and never an
account. The slug `passkey` is reserved so no site can shadow that page.

**Not reachable at the edge yet.** nginx serves `/.well-known/` out of the site
image as static files (`site/default.conf.template`, `location ^~
/.well-known/`), so this route answers on the service and 404s on kenjin.cc
until that location forwards this one path to the `/auth/` upstream. That file
belongs to the site, not to this service.

## The three blobs, and why two of them are public

| | holds | sealed under | served |
|---|---|---|---|
| **the wrap** | the 32-byte seed | the passkey's PRF output | to anyone, by public key |
| **the history** | the archive + MLS state | a key derived from the **seed** | only against a signature |
| **the head** | the chain's tail hash + its position | a key derived from the **seed** | to anyone, by public key |

Reading the wrap **cannot** be gated. It is how a new device obtains the key it
would authenticate with: the address is the public key, and the public key is
downstream of the seed being fetched, so demanding a signature first would
demand the very thing being fetched. What that gives up is that holding
someone's public key is enough to download their encrypted seed — ciphertext
under a key only their authenticator can produce. What it does **not** give up
is their history, which is a different blob under a different key behind a
signature.

Reading the **head** is ungated for a different reason: it is the anchor an
archive chain is checked against, and a chain cannot prove its own tail was not
cut — a prefix of a valid chain is itself a valid chain. Served to anyone, the
head is *witnessable*: a mirroring arc, a monitor, or the person's own second
device sees the ciphertext revert without being able to read it. An anchor
nobody can watch is an anchor an arc can move. The cost is that the chain
position becomes a public per-key activity counter, and that is the price of the
two properties the head exists for — a witness has to see it move, and this
service has to refuse a write that moves it backwards.

Writing it is signed *and* compare-and-set on `X-Chain-Position`. A position is
a count, not a clock: position *n* names exactly one tail, forever. That makes
the ordering enforceable rather than advisory, which `PUT /history`'s
`X-History-Exported-At` guard is not — and it is the guard that eventually
replaces it. A position that goes backwards is refused as `behind`; the same
position with different bytes is refused as a `fork`; the same position with
identical bytes is a retry and succeeds as a no-op. The blob itself is sealed by
`pacific_core::head` (`PHD1`, domain `pacific/head/v1`), and the position is
sealed inside it too, so a client can catch this service reporting a different
one.

The position is also **bounded above**, at `CHAIN_POSITION_MAX` — a trillion,
reported by `/auth/health`. Because the position only ever goes up, one write at
an absurd number is not an odd row: it is the end of the account's anchor, and
there is no reset. The ceiling is set against the accident that happens — a
client sending a clock where a count belongs — so every unix reading is above it
and no chain arrives there. It also keeps the top half of a u64 out of the
database: a head carries its position as a u64, SQLite's INTEGER is signed
64-bit, and a position at or above 2^63 would otherwise raise `OverflowError`
and 500.

That split is why this service has no second factor. The previous design sealed
the seed, the archive and the live MLS state into one blob under the passkey,
so releasing it needed an emailed code to stop a compromised platform account
being the whole story — and that is why this service used to hold email
addresses, phone numbers and one-time codes. It holds none of them now.

## What signing looks like

The bytes are built by `pacific_core::identity::auth_payload` on the device and
by `app/auth.py` here — one definition in two languages, because a signer and a
verifier one byte apart fail in a way neither side can see:

```
pacific-auth:v1\n<audience>\n<identity_key>\n<nonce>
```

`identity_key` is the rendered form (`ed25519:` + 64 hex), not the bare hex the
URL carries. The audience is inside the signed bytes, so a signature captured by
one Arc cannot be replayed at another; the nonce is ours and single use, so
there is no replay window at all.

## Running it

```sh
DB_PATH=:memory: PUBLIC_ORIGIN=https://kenjin.cc uvicorn app.main:app --reload
.venv/bin/python -m pytest tests -q
```

`entrypoint.sh` restores the database from R2 if the volume is empty, then runs
the service under Litestream. **The volume matters**: lose the file and every
returning user is a new user, with their history gone.

## Not built

- **A way back from a bad anchor.** `CHAIN_POSITION_MAX` bounds the accident;
  it does not undo one. A device that declares a position ahead of its real
  chain — 999,999,999,999, or simply 500 when the chain is at 5 — has stranded
  its own anchor, and there is no DELETE, no reset, and re-registering the wrap
  does not clear the row. A signed reset (the account asking for its own head to
  be dropped) is the missing route; every other shape hands an arc a way to move
  an anchor, which is the thing the head exists to prevent.
- **Rate limiting on the wrap and head reads.** Serving by public key makes
  `identity_pk` enumerable: anyone can check whether a given key has an account,
  and now also how long its chain is. Bound it, or accept it deliberately.
- **The chain the head anchors.** `E1` in *The Build*. Until it exists the head
  is an anchor for nothing — the shape is settled and enforced here, and the
  `histories` table and its three routes stay until the chain replaces them.
- **A signup gate.** Anyone with a keypair can register. The site password gate
  covers the pre-launch window; an invite or attestation check would hang off
  `PUT /auth/users/<pk>`.
- **Arc attestation.** A client has no way to verify the code this Arc runs.
  The prototype at `~/pseudo` makes that a blocking gate, and it is the piece
  most worth porting next.

## TODO — prove the rollback before it is needed

`entrypoint.sh` restores from the Litestream replica when `$DB_PATH` is missing,
so **"delete the database and restart" is the undo** for a bad migration. That has
never been exercised. Before the locator re-key touches a schema:

1. Take a manual snapshot of the live database.
2. On the gatetest container, delete `/data/auth.db*` and restart it.
3. Confirm the accounts come back — not that the container starts, which it will
   either way: `-if-replica-exists` makes a missing replica a **no-op**, so an
   unconfigured replica and a healthy one both look like a clean boot.

That last point is the whole reason this is written down. The failure mode is a
restore that silently restores nothing, discovered at the moment it is the only
thing standing between you and the accounts.

Raised by `docs/pre-mortem-private-lane.html`. Not done.
