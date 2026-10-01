"""fuzz — the property test: random sequences, driven twice, diffed every step.

The three F-cases are the sequences we thought of. This is the one that finds the
sequence we did not: a seeded random walk over publish / commit / subscribe /
evict, applied to a real `semaphore` process and to `RelayState` in lockstep, with
BOTH the client-visible frames and the relay's own four SQLite tables compared
after every single step. Any divergence stops the run and prints the step that
produced it, the frames, and the table diff.

Two walks, because eviction cannot be triggered on demand:

  * `walk` runs with retention OFF, so every step is deterministic and the diff is
    exact — append, the commit slot, replay cursors, fanout, seq monotonicity.
  * `rounds` runs with a per-tag cap and lets the relay's own timer sweep. Each
    round publishes, waits for the sweep to reach the state the model predicts,
    then diffs the tables and probes the Gap boundary from both sides of the floor.
"""
from __future__ import annotations

import asyncio
import base64
import random
from typing import Callable

from ..cases.framework import CaseResult, Run
from .. import address
from ..state import RelayHub, Retention
from .driver import Client
from .parity import _scratch
from .relay import Relay


def _tags(n: int) -> list[str]:
    # Real addresses: the relay refuses a Pub its tag did not sign (address.py).
    return [address.named(f"fuzz-{i}").tag_hex for i in range(1, n + 1)]


def _blob(rng: random.Random) -> str:
    return base64.b64encode(bytes(rng.getrandbits(8) for _ in
                                  range(rng.randint(1, 48)))).decode()


async def walk(steps: int = 120, seed: int = 20260914, n_tags: int = 4,
               n_clients: int = 4, on_step: Callable | None = None) -> CaseResult:
    """A seeded random walk with retention off. Every step diffed, both ways."""
    rng = random.Random(seed)
    tags = _tags(n_tags)
    r = Run("P1", f"Random walk, {steps} steps, seed {seed}",
            "REAL semaphore + model", on_step)
    with Relay(_scratch("fuzz"), label="fuzz") as relay:
        hub = RelayHub(retention=Retention())
        live = [await Client.connect(relay.url, f"c{i}") for i in range(n_clients)]
        model = [hub.connect(f"c{i}") for i in range(n_clients)]
        seen_seqs: list[int] = []
        divergence: str | None = None
        commits = rejects = subs = pubs = 0
        try:
            for n in range(1, steps + 1):
                i = rng.randrange(n_clients)
                if rng.random() < 0.62:
                    tag = rng.choice(tags)
                    blob = _blob(rng)
                    commit = rng.random() < 0.3
                    got = await live[i].pub(tag, blob, commit)
                    seq = got[-1]["seq"]
                    hub.publish(model[i], tag, blob, commit, seq=seq)
                    want = model[i].drain()
                    pubs += 1
                    commits += commit
                    rejects += got[-1]["ok"] is False
                    if seq:
                        seen_seqs.append(seq)
                    what = (f"step {n}: c{i} pub commit={commit} "
                            f"tag={tag[:4]}… -> ok={got[-1]['ok']}")
                else:
                    want_tags = rng.sample(tags, rng.randint(1, len(tags)))
                    since = rng.choice([0, 0, rng.choice(seen_seqs or [0]),
                                        rng.randint(0, 2 ** 50)])
                    v = rng.choice([0, 1, 1])
                    got = await live[i].sub(want_tags, since, v)
                    hub.subscribe(model[i], want_tags, since, v)
                    want = model[i].drain()
                    subs += 1
                    what = (f"step {n}: c{i} sub {len(want_tags)} tags "
                            f"since={since} v={v} -> {len(got)} frames")
                if got != want:
                    divergence = f"{what}\n  real  = {got}\n  model = {want}"
                    break
                # Other sockets may have been fanned out to; take those in too, so
                # the next step starts from an empty outbox on both sides.
                for j in range(n_clients):
                    if j == i:
                        continue
                    a, b = await live[j].quiet(0.05), model[j].drain()
                    if a != b:
                        divergence = f"{what}\n  fanout to c{j} real={a} model={b}"
                        break
                if divergence:
                    break
                if n % 10 == 0:
                    m, real = hub.tables(), relay.tables()
                    if m != real:
                        divergence = f"{what}\n  tables: " + "; ".join(m.diff(real))
                        break
            r.step(f"{pubs} publishes ({commits} commit-flagged, {rejects} refused), "
                   f"{subs} subscribes")
            r.check("no divergence in any frame, at any step", divergence is None,
                    divergence or "")
            m, real = hub.tables(), relay.tables()
            r.check("and the four tables are identical at the end", m == real,
                    "; ".join(m.diff(real))[:500] or
                    f"{len(real.blob)} blobs, {len(real.commit_slot)} slots, "
                    f"last_seq={real.meta.get('last_seq')}")
            r.check("the commit slot refused every commit after the first, per tag",
                    rejects == max(0, commits - len(real.commit_slot)),
                    f"{commits} commit publishes, {len(real.commit_slot)} slots taken, "
                    f"{rejects} refused")
            r.check("seq is globally monotonic across tags",
                    seen_seqs == sorted(seen_seqs) and len(set(seen_seqs)) == len(seen_seqs),
                    "so an observer on two tags recovers their exact interleaving (E11)")
        finally:
            for c in live:
                await c.close()
    return r.done()


async def rounds(n_rounds: int = 4, cap: int = 2, seed: int = 20260914,
                 n_tags: int = 3, on_step: Callable | None = None) -> CaseResult:
    """Publish, let the relay's own sweep fire, diff the floors it wrote."""
    rng = random.Random(seed)
    tags = _tags(n_tags)
    r = Run("P2", f"Eviction rounds, cap {cap}/tag, seed {seed}",
            "REAL semaphore + model", on_step)
    with Relay(_scratch("fuzzevict"), max_per_tag=cap, sweep_secs=1,
               label="fuzz-evict") as relay:
        hub = RelayHub(retention=Retention(max_per_tag=cap))
        pub = await Client.connect(relay.url, "author")
        m_pub = hub.connect("author")
        try:
            for round_no in range(1, n_rounds + 1):
                for _ in range(rng.randint(cap + 1, cap + 4)):
                    tag = rng.choice(tags)
                    blob = _blob(rng)
                    got = await pub.pub(tag, blob)
                    hub.publish(m_pub, tag, blob, seq=got[-1]["seq"])
                m_pub.drain()
                hub.evict()
                want = hub.tables()
                settled = await _settle(relay, want, timeout=12)
                r.step(f"round {round_no}: swept to {len(want.blob)} blobs, "
                       f"{len(want.retention_floor)} floors")
                r.check(f"round {round_no}: the relay's sweep landed where the model did",
                        settled, "; ".join(want.diff(relay.tables()))[:400])
                if not settled:
                    break

                # The Gap boundary, from both sides, on a tag that has evicted.
                floors = relay.tables().retention_floor
                if not floors:
                    continue
                tag = rng.choice(sorted(floors))
                floor = floors[tag]
                for since, expect_gap, note in ((floor - 1, True, "one below the floor"),
                                                (floor, False, "exactly at the floor"),
                                                (0, True, "from cold")):
                    c = await Client.connect(relay.url, "probe")
                    m = hub.connect("probe")
                    got = await c.sub([tag], since, 1)
                    hub.subscribe(m, [tag], since, 1)
                    want_frames = m.drain()
                    has_gap = any(f["t"] == "gap" for f in got)
                    r.check(f"round {round_no}: v=1 {note} -> "
                            f"{'Gap' if expect_gap else 'no Gap'}",
                            got == want_frames and has_gap == expect_gap,
                            f"real={got}" if got != want_frames else "")
                    await c.close()
                    # v = 0 must never be told, whatever the cursor.
                    c = await Client.connect(relay.url, "probe0")
                    m = hub.connect("probe0")
                    got0 = await c.sub([tag], since, 0)
                    hub.subscribe(m, [tag], since, 0)
                    r.check(f"round {round_no}: v=0 {note} -> silence",
                            got0 == m.drain() and not any(f["t"] == "gap" for f in got0),
                            str(got0) if any(f["t"] == "gap" for f in got0) else "")
                    await c.close()
        finally:
            await pub.close()
    return r.done()


async def _settle(relay: Relay, want, timeout: float = 12.0) -> bool:
    """Wait for the relay's timer sweep to reach the state the model predicted."""
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    while loop.time() < deadline:
        if relay.tables() == want:
            return True
        await asyncio.sleep(0.15)
    return relay.tables() == want


PROPERTY = [walk, rounds]


async def run_all(on_step: Callable | None = None) -> list[CaseResult]:
    return [await walk(on_step=on_step), await rounds(on_step=on_step)]
