# harness/devices — the browser, the embed, and the phone

The device agents of `docs/multi-platform-harness.html` §03 for the three
platforms that are not Rust: a **browser device** (keyholder.js in Chromium),
the **embedded site** (pacific.js mounted in a member's own page), and the
**iOS app** (Pacific.app in a simulator). The Rust agent is `docs/the-harness.html`
§06 step 05 and is not here.

Nothing here touches `core/`, `arc/`, `app/` or `site/`, or anything in
`harness/` outside this directory. Every run starts its own relay, its own
keyholder servers and its own simulators. It does not touch what is already
running on :8100–8104 or :8787, or the `E2E-*` and `Pacific Demo` simulators.

```
harness/devices/
  protocol.py      Agent, Answer, the verb envelope; Sidecar (JSON lines over stdin/stdout)
  runner.py        Runner — drives devices, judges answers; until() and settle(), never a sleep
  web/agent.mjs    one browser device: a bun sidecar driving headless Chromium (playwright-core)
  web/agent.py     WebAgent (a keyholder, no UI) · EmbedAgent (pacific.js in a page) · KeyholderServer
  ios/agent.py     IOSAgent · Simulator — pacific:// lines in, dev-*.txt files out
  cases.py         W1–W4, I1
  __main__.py      python -m harness.devices …
```

## Running it

```sh
cd <repo>
.venv/bin/python -m harness.devices browser    # W1 W2 W3 — do the browser drivers work
.venv/bin/python -m harness.devices findings   # W4 — red until keyholder.js is fixed
.venv/bin/python -m harness.devices ios        # I1 — two simulators, one MLS room
.venv/bin/python -m harness.devices all        # everything; browser and phones concurrently
.venv/bin/python -m harness.devices w3         # any one case by name
```

Nothing to install. The browser agent is `bun web/agent.mjs`, which resolves
`playwright-core` 1.58.2 from bun's cache and launches the Chromium headless
shell in `~/Library/Caches/ms-playwright`. The relay is `arc/target/debug/semaphore`,
started through `harness.live.relay.Relay` rather than a third copy of that code. The
phone needs Xcode and a **Debug** simulator build of Pacific.app, because Release
compiles DevLinks out. Transcripts, logs and screenshots go to `--out`, a temp
directory by default, whose path is printed first. Exit status is 0 only if every check is green.

## Where it stands — 14 Sep

| case | what it proves | result |
|---|---|---|
| **W1** | two browser devices pair over X25519 and converge through a real relay; the relay's blobs carry no plaintext | green, 10/10 |
| **W2** | pacific.js in a hostile page: one shadow root, the key's database invisible from the member's origin; mark → chat disc → type → the keyholder folded it as this device's delta | green, 12/12 |
| **W3** | a person types in the embed and a second browser device holds it; the other way, the embed redraws unprompted | green, 6/6 |
| **W4** | two commits issued at once both survive in the log | **red, 1/4 — a keyholder.js bug, below** |
| **I1** | a room converges across two phones through real MLS | **red — the app dies at launch on this Mac, below** |

## The protocol

One request, one answer, one process per device:

```
→ {"id": 7, "verb": "rpc", "args": {"m": "store.commit", "a": {"op": {…}}}}
← {"id": 7, "ok": true, "v": {…}}
```

The value rides under `v`, the shape the keyholder already replies in. `Agent.do(verb, **args)`
returns an `Answer`. A verb the platform cannot perform comes back
`unsupported`, carrying the reason from that agent's `CANNOT` table. That is a
different outcome from trying and failing, and the transcript marks it `SKIP`, not `FAIL`.
Assertions live in `cases.py` and nowhere in an agent.

| verb | web | embed | ios |
|---|---|---|---|
| `identity` | `device.identity` | same | `dev-contact.txt` (space, bundle) |
| `sync` | `sync.start {relay}` | same | `pacific://sync` |
| `device_offer` · `device_accept` | X25519 pairing of one person's devices | same | CANNOT |
| `bundle` · `pair_scan` · `pair_accept` | CANNOT | CANNOT | MLS contact pairing of two people |
| `commit` · `deltas` · `world` | `store.commit` · `store.deltas` · `store.load` | same, `world` is the interior's own | CANNOT (`commit`) |
| `room_new` · `room_add` · `obj_post` · `obj_reply` · `obj_view` | CANNOT | CANNOT | `new-room` · `room-add-member` · `room-post` · `room-reply` · `dump-thread` |
| `dm_post` · `dm_reply` | CANNOT | CANNOT | `dm-send` · `dm-reply` (no `dm_view`: DevLinks writes no DM transcript) |
| `ui_open` · `ui_view` · `ui_chat_say` · `ui_text` | — | through the shadow root | — |
| `rpc` · `rpc_many` · `link` | any keyholder method · several at once | same | any `pacific://` line |
| `screenshot` · `host` · `pushes` · `console` | yes | yes | `screenshot` |

`rpc`, `rpc_many` and `link` are escape hatches. They are named so that a scenario
reaching for one is visibly off the shared vocabulary.

**No case crosses platforms, and that is a finding.** A browser pairs devices over
X25519 and folds the interior's own ops into a per-site log. A phone pairs people
by MLS contact bundle and folds core deltas into GroupObjects. No verb gets the same
answer from both, and the `CANNOT` tables say so in place. "Browser and core on one
relay" waits on GroupObject conformity (multi-platform-harness §06).

## How each platform is driven

### Browser and embed

- **The member's origin is routed, not served.** keyholder.js only answers pages on
  an origin registered for the site, which for `cambridge-dd` is `http://localhost:8100`.
  Another track's docs server may be on that port, and `app/web/run.sh stop` kills
  whatever holds it. So the agent answers `:8100` itself, from `app/web/docs` on
  disk and read-only. The keyholder is a real `serve.py` on a port of its own,
  because that origin is the device boundary.
- **Local Network Access is granted to `:8100` alone.** A routed document has no
  address, so Chromium treats it as public, and a public page framing loopback fails
  with `ERR_BLOCKED_BY_LOCAL_NETWORK_ACCESS_CHECKS`. Measured: a real loopback page
  loads the frame, and the routed page loads it only with the grant or with the check
  disabled. The agent grants the permission, the way a person clicking Allow would,
  and the check stays on for every other origin. Neither run.sh nor production meets this.
- **Chromium is pinned** to the headless shell whose revision playwright-core wants.
  Its default path points at a full Chrome build that is not installed.
- **The embed is driven through the shadow root:** the mark, the discs, the composer.
  Its world is read back from `Pacific.mount`'s api. A conversation click commits a
  `read`, so the composer waits for that commit to show up in the keyholder's log
  before it types. Typing sooner triggers W4.

### Phone

- **Commands go through the file, not the URL.** `simctl openurl` needs a tap to
  confirm on iOS 26. DevLinks' `DevCommands` drains `dev-commands.txt` in the app's
  container every second, through the same `DevLinks.handle`.
- **Delivery is acknowledged.** The drain records how many lines it has consumed in
  `pacific.dev.commandCursor`. `send()` reads that value back through
  `simctl spawn … defaults read` and returns once the app has taken its line. What the
  line *did* is read from a `dev-*.txt` file, which is deleted before the verb and
  awaited afterwards, so an answer cannot be stale.
- **Simulators are the agent's own.** Each is `Harness-<device>`, created if missing.
  Every start clean-installs the app and resets the simulator's keychain, which is
  why the agent never touches a simulator it did not name.

**Verified so far:** simulator create, boot, clean install and launch with
`PACIFIC_*` env; resolving the container; and the cursor read path (a value written
into `network.pacific` reads back through `Simulator.cursor()`, and `None` once
deleted). **Not yet verified:** the app's own cursor write, any DevLinks verb, and
I1. The app never lives long enough to run them.

## What the drivers found

**W4: two commits at once lose one, silently.** `app/web/keyholder/keyholder.js:248`,
`append()`, reads the log to pick the next `seq` and writes afterwards. Two appends that
both read before either writes get the same `(author, seq)`. The log is keyed
`['site','a','seq']`, so on the device the second write replaces the first. On a
paired device, the `have` check in `sync.start` drops an `(author, seq)` it already
holds as a re-delivery, so it keeps the first. W3 lost a message this way before its
driver waited for the click to land. W4's latest run: both commits accepted, one delta
in the log (`#0 'second · …'`). A person clicking and typing is usually slower than
the race, but a quick double send, or one site open in two tabs, hits the same
unserialised `append()`. That trigger is plausible and has not been demonstrated.

**`serve.py` does not start without `AUTH_UPSTREAM`.** `app/web/keyholder/serve.py:201`
reads `FIXTURE`, which no longer exists, and dies with `NameError` before serving.
`KeyholderServer` sets `AUTH_UPSTREAM` to a closed loopback port. No agent verb needs
`/auth`, and anything that did would fail loudly with `upstream_unreachable`.

**Pacific.app dies at launch on this Mac, in every build.** `PacificApp.swift:64`
calls `fatalError("the union embedder could not be built — unsupported(no on-device
sentence embedding is available …)")`. English uses Apple's on-device sentence embedding.
The LinguisticData asset it needs is missing: this Mac's catalog for it is marked
`.purged`, and no simulator on it has a copy. All nine builds on the machine, 62 on
12 Aug through 109 on 13 Sep, carry both the drain and this fatal path. The app
deliberately allows no silent fallback, and
`PACIFIC_RESOLVER_ALLOW_NLEMBEDDER_FALLBACK` covers the resolver, not this path.
Until the asset is back, or the app handles its absence, no simulator on this machine
can run I1, and neither can the scripts in `app/ios/scripts`.

## Conventions worth keeping

- A red case attaches its evidence: each browser device's console, `sync.status`
  and held deltas, plus the relay's blob counts. It should say what happened, not only that a wait timed out.
- `until` waits for the thing a case is about; `settle` then waits until nothing
  is still changing. Quiescence alone can be satisfied before anything has started arriving.
- The footer names what is real on every device, every run. `BACKEND:` lines are
  attached to each case.
