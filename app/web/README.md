# Pacific — an interior for any site

The web implementation of pacific-core. A Pacific site is two things wearing one
address. The **public face** is whatever
the community already has: their HTML, their CSS, their domain, their words. The
**interior** is Pacific's — forum, events, members, federation, and the
conversations along the bottom.

`pacific.js` is the interior, packaged so that adding it to a site is one line
and costs that site nothing.

## Embed

```html
<script src="https://kenjin.cc/pacific.js" data-site="cambridge-dd"></script>
```

That is the whole integration. No build step, no framework, no stylesheet to
include, no element to position, no class names to avoid. Put the tag anywhere
in the document — inside a card, inside a footer, inside a grid — and nothing
about the page moves.

The equivalent element form, if a page would rather be explicit:

```html
<pacific-site site="cambridge-dd"></pacific-site>
```

## Free-standing

The same file, on a page of its own, is the site with no host behind it — the
shape the iOS app wears. No door, no page to go back to; the window is the page.

```html
<pacific-site site="cambridge-dd" mode="standalone"></pacific-site>
<script src="https://kenjin.cc/pacific.js"></script>
```

## What it touches

One `<div>` appended to `<body>`, holding a shadow root. Nothing else.

- **It cannot style the host, and the host cannot style it.** Every rule lives
  inside the shadow root, and `:host { all: initial }` closes the only channel a
  host page's CSS has into a shadow tree — inheritance. So there is no reset to
  include, no specificity war, and no prefix on any class name.
- **It takes no space.** The layer is fixed to the viewport and transparent to
  clicks; only the mark, the scrim and the window take a click.
- **It is mounted on `<body>`, not where the tag sits.** `position: fixed` is
  measured against the nearest ancestor with a transform, a filter or a
  containment, and a host page has every right to have one anywhere.
- **One line reaches outside the shadow root**, and it is written down in the
  source: while the window is open the host page holds its scroll, the way it
  would under any modal, and gets it back on close.

## The keyholder

The device key does not live on the member's origin. `pacific.js` runs there, and
a shadow root isolates CSS but not scripts — same origin is same origin, so any
script on that page could reach the same IndexedDB and use the same
non-extractable `CryptoKey` handles. Non-extractable stops exfiltration; it does
not stop use.

So `keyholder/` is a second origin (`:8103` locally, its own document). It holds
the device's Ed25519 key and the device's replica of the graph. The host page
gets a hidden iframe and a `MessagePort`, handed over once, and typed requests
over it. The port is the authorisation: unforgeable, and unobtainable by a script
that was not given one.

```html
<script src="pacific.js" data-site="cambridge-dd"
        data-keyholder="https://keys.wallflowers.io/"></script>
```

`docs/pacific-keyholder.html` is the test, not an illustration of one: it runs on
the member's origin beside the interior and tries every route a script there
would actually have to the key — read into the frame, open the same-named
database, ask for the key outright, use the key. All five results are printed on
the page.

Ed25519 and X25519 are native in WebCrypto and non-extractable, so the browser's
device key is the same kind of key `identity.rs` mints, not a translation of one.

**Open on the keyholder:** the origin allowlist is a constant in
`keyholder/keyholder.js`. In production it is the site's registered origins served
from Pacific's own directory — without that, any page anywhere embeds the script
with a real site handle and gets a live session. Also:
`navigator.storage.persist()` currently returns **false** in Safari here, which
means the device is evictable and will need re-pairing. Both are marked in the
source.

## One reducer, not three

The op reducer used to exist three times: in `pacific.js`, in `keyholder.js`, and
in Rust. `pacific.js`'s copy is **gone**, and the file says so where it was, so it
does not grow back. The reducer belongs to the *store*, and the store lives on the
keyholder's origin — a component embedded in a member's page cannot be the
authority on what a commit means, because the page could change it.

Consequences, all deliberate:

- **A keyholder is now required.** `Pacific.mount` throws a named error without
  one; there is no local fallback, because a local fallback needs a reducer.
- **The demos need the server.** `file://` no longer works for
  `pacific-site-desktop.html` or `pacific-standalone.html` — a keyholder needs an
  origin, and `file://` has none. Use `./run.sh`.
- **The keyholder holds many sites.** The log and the checkpoint are keyed by
  site handle; the device key is not, because one device carries many sites. Before
  this, two sites on one keyholder origin silently shared a world — whichever
  loaded first won. `pacific-log-test.html` asserts the isolation.

The remaining JS reducer can only go when the web client's world moves onto the
core's object model — see `docs/multi-device-plan.md`. The two do not fold
the same thing: the core folds real `Delta`s into `GroupState`; the browser folds
six invented ops into a demo world.

## Signing in, and which Arc is yours

The profile icon in the title bar is the door. Signed out it is a dashed outline
with no initial in it — a face that is not yours is worse than no face — and
clicking it opens `kenjin.cc/signin`, which already implements the three doors: a
Kenjin passkey (WebAuthn), Apple, and AI Passport. **None of that is
reimplemented here.**

Two things are forced rather than chosen:

- **The keyholder owns the session.** It is an httpOnly cookie on Pacific's
  origin; the interior runs on the member's origin and cannot read it, and must
  not be able to. The keyholder is already on that origin for the device key, so
  it asks `GET /auth/session` and hands back an *answer* —
  `{signedIn, uid, providers, arc, store}` — never the cookie. There is no method
  that would return one.
- **The door is a popup.** A WebAuthn ceremony must run on the RP's own origin,
  and the two OAuth doors are top-level redirects that mint a signed cookie.
  Neither survives a hidden iframe. So it opens a window and this side asks again
  when it closes; the deployed sign-in flow needs no change to support that.

**The Arc comes from the session, not from a constant.** Which Arc holds a
member's mailboxes is a property of the account, so `sync.start` takes
`session.arc` first (`wss://arc.kenjin.cc/v1/relay`), an explicit override
second, and the local harness last. A device that silently synced from the wrong
relay would look exactly like one that had nothing to sync.

`docs/pacific-signin-test.html` exercises both paths.
`keyholder/auth/session` is a **static fixture**, not the service — in production
nginx proxies `^~ /auth/` to `kenjin-auth`. Delete the file for the signed-out
path.

## The delta log

The keyholder stores a **feed**, not a fold. A commit appends
`{id, author, seq, at, op}` to an append-only store keyed `(author, seq)`; the
world is derived over a checkpoint that collapses every 200 deltas. It used to
mutate one `world` blob and discard the op — which is unmergeable, because two
replicas of one graph cannot converge from snapshots.

Measured here: 51k appends/s, 54k replays/s, 9.5 MB per 20k deltas. A million
deltas would be ~477 MB and ~19 s to fold cold, which is what the checkpoint is
for. `docs/pacific-log-test.html` asserts the lot.

`store.deltas({since})` returns the tail for a given author→seq vector. Nothing
consumes it yet; it exists because a log whose contents cannot be enumerated is a
log that cannot converge, and building it without the reader is how that gets
found out late.

**Two things marked in the source.** Fold order is `(at, author, seq)` — 
deterministic, so replicas holding the same set agree, but *not* the core's rule
(owner-sequenced ops belong on a spine; commutative ops carry their own LWW key).
And `DeltaId` is sha256 over key-sorted JSON rather than canonical CBOR, so ids
must not cross the wire until that is reconciled.

## Two devices, one person

`docs/pacific-two-devices.html` — device A on `:8103`, device B on `:8104`. Two
origins are two IndexedDB stores, so these are two real devices with their own
keys and their own logs, not a simulation of two.

They pair over X25519 (the offer string is what a QR carries), HKDF-split the
shared secret into a **tag** and a **seal key**, and talk through the real
semaphore relay on `:8787` speaking `pacific-wire`'s Frame vocabulary. The tag is
derived and never transmitted; the relay sees 32 opaque bytes and a sealed blob.

A delta authored on A lands in B's log **under A's authorship**, folds into B's
world, and pushes to B's interior without B asking — and back the other way. That
is what the log being keyed `(author, seq)` was for.

The relay's own telemetry after a run: `total_publishes=2 total_deliveries=4
store_tags=1 store_blobs=2 bad_frames=0` — counts and a tag count, never a tag
value and never content.

Needs the relay up: `RELAY_BIND=127.0.0.1:8787 ~/pacific/arc/target/debug/semaphore`.

**What this is not.** It is not MLS and it is not a group — it is a pairwise
channel between two devices that trust each other because one person holds both.
That is deliberately the smaller primitive: `docs/multi-device-plan.md` calls the
same shape "hand an encrypted archive to a device that cannot decrypt the past"
and builds history transfer on it.

**A browser finding, measured 9 Sep.** An X25519 `CryptoKeyPair` does **not**
survive IndexedDB in WebKit — the put completes, the transaction fires
`oncomplete`, and the record reads back undefined. Silent, no error. Ed25519,
ECDH P-256, ECDSA and AES-GCM all round-trip, and Ed25519 still signs afterwards.
The exchange key is now ephemeral, which it should have been anyway; only the
derived AES-GCM channel key persists.

## The core, in the browser

`./build-wasm.sh` builds `pacific-core` for `wasm32-unknown-unknown` and drops
`core_wasm.wasm` beside the pages. 251 KB, **no imports** — the script asserts
that on every build, because a module that grows one fails to instantiate with a
LinkError in the page rather than here.

It exports the three things JavaScript must not compute for itself:

| | why a JS version is dangerous, not just duplicative |
|---|---|
| `delta_canon` | the integer-keyed CBOR a delta is hashed and signed over. The web client had been using key-sorted JSON — different bytes, different id. |
| `delta_id` | `sha256(canonical)`, the content address *and* the chain link. An id computed two ways is a chain that silently forks. |
| `sas_words` | the three words two people read aloud to confirm a key. Drift here does not fail loudly; it tells two people they match when they do not. |

`docs/pacific-wasm-test.html` asserts all of it against vectors pinned by a
native Rust test (`core-wasm`, `vectors::fixture_is_stable`). Its last line
recomputes the old JS id and shows it differs — the drift was real.

Requires the core's local-dev `.cargo/config.toml` (see
`core/.cargo/config.toml.example`): `pacific-core`'s dev-dep on arc's `relay`
pins a rev no longer on the remote.

## The case studies

`docs/pacific-site.html?site=stoma` and `?site=example` put the interior over
a real captured site, served from its own mirror on its own port (`./run.sh`
starts all three). A captured site cannot be asked to add a script tag, so its
page is loaded whole into a cross-origin sandboxed iframe and the interior goes
over it. Cross-origin is the architecture rather than a workaround: the embed
case gets its guarantee from a shadow root, this one gets it from the browser's
origin rules.

`docs/derive-pacific-site.py` is obsolete. It existed to rewrite a 47KB
single-file prototype into a second 47KB copy per world; there is one component
now and it takes its world as an argument. It fails loudly on its missing
anchors rather than producing a wrong page, which is what its own docstring
promises — but it should be deleted.

`docs/pacific-site-desktop.html` is a host page whose CSS is deliberately
hostile — a content-box reset, `button { all: unset }` in a script face, dashed
tables, `.row` tilted two degrees, `h2` at 60px, letter-spacing on the body.
None of it reaches the interior. That page is the test, not an illustration of
one.

## Attributes

| attribute | | |
|---|---|---|
| `site` | required | the site's handle |
| `mode` | `embed` (default) or `standalone` | |
| `endpoint` | orecloud base URL | omit and it runs on the local store |
| `token` | bearer token for orecloud | |

On a `<script>` tag the same four are `data-site`, `data-mode`, `data-endpoint`,
`data-token`.

## The world, and the wire

Everything on screen comes from one object, and every change to it is one of
six ops:

```
{t:'post',   thread, by, s}     a reply in a thread
{t:'conv',   id, with}          the first word to someone new
{t:'say',    conv, s}           a message
{t:'read',   conv}              a conversation opened
{t:'rsvp',   event, going}      going, or not
{t:'invite', email}             an invitation
```

That is not tidiness for its own sake: **ops are the unit orecloud syncs**, so
the same six that redraw a pane are the six that travel. A store is three
methods, and swapping the local one for the orecloud one is the only difference
between a demo and a member's real site on four devices.

```js
{
  load()        -> Promise<world>
  commit(op)    -> Promise<world>
  subscribe(fn) -> unsubscribe        // fn(world) when a change arrives
}
```

Pass your own with `Pacific.mount({ site, store })`.

## orecloud — open

**The wire format is not settled.** The adapter in `pacific.js` states the three
routes it assumes, so the real shape can be matched against them:

```
GET  {endpoint}/sites/{site}        -> the world
POST {endpoint}/sites/{site}/ops    -> the world      (body: one op)
GET  {endpoint}/sites/{site}/stream -> text/event-stream of worlds
```

Ops are applied locally first and reconciled with what comes back, so a member
on a slow connection is never typing into a frozen pane. With no `endpoint` the
adapter refuses to guess quietly: it falls back to the local store and says so
once in the console.

Three things still need answering before this is real: how orecloud identifies
the member (the component currently trusts `world.me`), whether it wants whole
worlds or patches on the stream, and where end-to-end encryption sits relative
to it — the keys are the member's, so either the ops are sealed before they
leave and orecloud stores ciphertext, or orecloud is inside the trust boundary,
and those are different products.
