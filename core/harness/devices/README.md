# harness/devices — the phone

The device agent of `docs/multi-platform-harness.html` §03 for the **iOS app**
(WallFlowers.app in a simulator). The Rust agent is `docs/the-harness.html` §06 step
05 and is not here. The browser and embed agents (W1–W4) drove the keyholder, an
origin that held device keys in the browser, and were retired with it: the web signs
in through the Door, and no browser holds a key.

Nothing here touches `core/`, `arc/`, `app/` or `site/`, or anything in `harness/`
outside this directory. Every run starts its own relay and its own simulators. It
does not touch what is already running on :8787, or the `E2E-*` simulators.

```
harness/devices/
  protocol.py      Agent, Answer, the verb envelope; Sidecar (JSON lines over stdin/stdout)
  runner.py        Runner — drives devices, judges answers; until() and settle(), never a sleep
  ios/agent.py     IOSAgent · Simulator — pacific:// lines in, dev-*.txt files out
  cases.py         I1
  __main__.py      python -m harness.devices …
```

## Running it

```sh
cd <repo>
.venv/bin/python -m harness.devices ios        # I1 — two simulators, one MLS room
.venv/bin/python -m harness.devices all        # everything
```

The relay is `target/debug/semaphore`, started through `harness.live.relay.Relay`.
The phone needs Xcode and a **Debug** simulator build of the app, because Release
compiles DevLinks out. Transcripts, logs and screenshots go to `--out`, a temp
directory by default, whose path is printed first. Exit status is 0 only if every
check is green.

## Where it stands — 14 Sep

| case | what it proves | result |
|---|---|---|
| **I1** | a room converges across two phones through real MLS | **red — the app dies at launch on this Mac, below** |

## The protocol

One request, one answer, one process per device:

```
→ {"id": 7, "verb": "rpc", "args": {"m": "store.commit", "a": {"op": {…}}}}
← {"id": 7, "ok": true, "v": {…}}
```

The value rides under `v`. `Agent.do(verb, **args)`
returns an `Answer`. A verb the platform cannot perform comes back
`unsupported`, carrying the reason from that agent's `CANNOT` table. That is a
different outcome from trying and failing, and the transcript marks it `SKIP`, not `FAIL`.
Assertions live in `cases.py` and nowhere in an agent.

| verb | ios |
|---|---|
| `identity` | `dev-contact.txt` (space, bundle) |
| `sync` | `pacific://sync` |
| `bundle` · `pair_scan` · `pair_accept` | MLS contact pairing of two people |
| `room_new` · `room_add` · `obj_post` · `obj_reply` · `obj_view` | `new-room` · `room-add-member` · … |
| `dm_post` · `dm_reply` | `dm-send` · `dm-reply` (no `dm_view`: DevLinks writes no DM transcript) |
| `rpc` · `link` | any `pacific://` line |
| `screenshot` | `screenshot` |

`rpc` and `link` are escape hatches. They are named so that a scenario reaching for
one is visibly off the shared vocabulary.

## How the phone is driven


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

## What the driver found

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

- A red case attaches its evidence: each device's logs and the relay's blob counts.
  It should say what happened, not only that a wait timed out.
- `until` waits for the thing a case is about; `settle` then waits until nothing
  is still changing. Quiescence alone can be satisfied before anything has started arriving.
- The footer names what is real on every device, every run. `BACKEND:` lines are
  attached to each case.
