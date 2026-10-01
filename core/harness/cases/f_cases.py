"""F13, F14, F15 — the three cases that are real today against the state objects.

Each one is the design's rule, stated as an assertion over the tables. The same
sequences are driven against the live relay in `harness/live/parity.py`, and the
tables are diffed; when the two disagree, one of them is wrong and the
disagreement is the finding.
"""
from __future__ import annotations

import base64
from typing import Callable

from .. import wire
from ..state import (Budget, DEFAULT_BUDGET_BYTES, MAX_LIST_PAGES, MediaError,
                     R2Relay, R2State, RelayHub, RelayState, Retention, SeqSource,
                     Verdict)
from .framework import CaseResult, Run

TAG_A = "aa" * 32
TAG_B = "bb" * 32


def _blob(text: str) -> str:
    return base64.b64encode(text.encode()).decode()


def _clock(start: int = 1_700_000_000_000_000):
    """A clock the case drives, so seq is an assertion rather than a sleep."""
    box = {"t": start}

    def tick(step: int = 1_000_000) -> int:
        box["t"] += step
        return box["t"]
    return box, tick


# ── F13 ─────────────────────────────────────────────────────────────────────


def f13_relay_refuses_a_second_commit(on_step: Callable | None = None,
                                      backend: str = "model") -> CaseResult:
    """The commit_slot PRIMARY KEY rejects; the loser gets Ack{ok:false}.

    The relay's first-writer-wins arbitration is the only thing standing between
    two devices and a GROUP FORK. What makes it safe is that it is one
    transaction: no crash can leave a slot claimed for a blob that was never
    stored, which would wedge that group-epoch permanently.
    """
    r = Run("F13", "Relay refuses a second commit", backend, on_step)
    box, tick = _clock()
    hub = RelayHub(retention=Retention(), seq=SeqSource(clock=lambda: box["t"]))
    r.expose(hub=hub)

    r.step("two devices author into the same group-epoch; a third is listening")
    ada = hub.connect("ada/phone")
    bo = hub.connect("bo/sim")
    eve = hub.connect("eve")
    hub.subscribe(eve, [TAG_A], since=0, v=1)
    eve.drain()

    r.step("ada commits first")
    tick()
    seq_win, fanout, ok = hub.publish(ada, TAG_A, _blob("ada's commit"), commit=True)
    r.check("the winner is acked ok", ada.drain() == [wire.ack(seq_win, ok=True)])
    r.check("and fanned out to the subscriber", [f for f in eve.drain()
                                                 if f["t"] == "msg"] ==
            [wire.msg(TAG_A, seq_win, _blob("ada's commit"))], f"fanout={fanout}")

    r.step("bo commits to the same tag a microsecond later")
    tick()
    seq_lose, fanout2, ok2 = hub.publish(bo, TAG_A, _blob("bo's commit"), commit=True)
    r.check("the loser is acked ok:false", bo.drain() == [wire.ack(0, ok=False)])
    r.check("with seq 0 — there is no sequence number to give it",
            (seq_lose, ok2) == (0, False))
    r.check("and nothing was fanned out", eve.drain() == [], f"fanout={fanout2}")

    r.step("the store is asked what it kept")
    t = hub.tables()
    r.check("the loser's blob is NOT in the mailbox",
            list(t.blob) == [(TAG_A, seq_win)],
            "a rejected commit that was still stored would be replayed as sequenced")
    r.check("the slot holds exactly one seq, the winner's",
            t.commit_slot == {TAG_A: seq_win})
    r.check("the lost race did not advance the seq high-water",
            t.meta.get("last_seq") == seq_win,
            "the whole append is one transaction, so a rejection leaves no trace")

    r.step("ordinary traffic on that tag continues after the commit")
    tick()
    seq_chat, _, ok3 = hub.publish(ada, TAG_A, _blob("chatter"), commit=False)
    r.check("the slot gates COMMITS, not the tag", ok3 and seq_chat > seq_win)
    r.check("a second commit is still refused, forever",
            hub.publish(ada, TAG_A, _blob("again"), commit=True)[2] is False)
    r.check("a DIFFERENT tag has its own slot",
            hub.publish(bo, TAG_B, _blob("bo's own epoch"), commit=True)[2] is True)
    r.check("rejected_commits counted twice", hub.metrics.rejected_commits == 2)
    r.note("the loser MUST rebase onto the winning commit — the relay has told it "
           "only that it lost, which is all a blind relay can say")
    return r.done()


# ── F14 ─────────────────────────────────────────────────────────────────────


def f14_eviction_announces_a_gap(on_step: Callable | None = None,
                                 backend: str = "model") -> CaseResult:
    """The floor is recorded before the delete; Sub{v>=1} is told, v=0 is not.

    A blind relay cannot know whether anyone drained, so retention can only ever
    be time-based — but a hole in someone's history can at least be ANNOUNCED
    rather than inferred. Without the Gap a client below the floor simply receives
    less than it should and cannot tell: `Sub` replays `seq > since` either way.
    """
    r = Run("F14", "Eviction announces a Gap", backend, on_step)
    box, tick = _clock()
    hub = RelayHub(retention=Retention(max_per_tag=1),
                   seq=SeqSource(clock=lambda: box["t"]))
    r.expose(hub=hub)
    ada = hub.connect("ada/phone")

    r.step("three blobs land on one tag, and one on a tag that never evicts")
    seqs = []
    for text in ("one", "two", "three"):
        tick()
        seqs.append(hub.publish(ada, TAG_A, _blob(text))[0])
    tick()
    seq_b = hub.publish(ada, TAG_B, _blob("untouched"))[0]
    ada.drain()

    r.step("the sweep runs with a cap of one blob per tag")
    removed = hub.evict()
    t = hub.tables()
    r.check("two blobs removed", removed == 2)
    r.check("the newest survives", sorted(s for _, s in t.blob if _ == TAG_A) == [seqs[2]])
    r.check("the floor is the highest seq REMOVED, not the lowest surviving one",
            t.retention_floor.get(TAG_A) == seqs[1],
            f"floor={t.retention_floor.get(TAG_A)} removed={seqs[:2]} kept={seqs[2]}")
    r.check("a tag that never evicted has NO floor row",
            TAG_B not in t.retention_floor,
            "absent is a different claim from zero, and only absent suppresses a Gap")

    r.step("a v>=1 subscriber arrives from cold")
    modern = hub.connect("laptop v1")
    hub.subscribe(modern, [TAG_A, TAG_B], since=0, v=1)
    got = modern.drain()
    r.check("it is told the backlog is incomplete, BEFORE the backlog",
            got[0] == wire.gap(TAG_A, seqs[1]), str(got[:1]))
    r.check("exactly one Gap — the untouched tag is not announced",
            [f for f in got if f["t"] == "gap"] == [wire.gap(TAG_A, seqs[1])])
    r.check("then what survived, in seq order, then Eose",
            [f for f in got if f["t"] != "gap"] ==
            [wire.msg(TAG_A, seqs[2], _blob("three")),
             wire.msg(TAG_B, seq_b, _blob("untouched")), wire.eose()])

    r.step("a v=0 subscriber arrives from cold")
    old = hub.connect("phone v0")
    hub.subscribe(old, [TAG_A, TAG_B], since=0, v=0)
    got0 = old.drain()
    r.check("it is told NOTHING about the hole",
            [f for f in got0 if f["t"] == "gap"] == [],
            "an unknown frame is fatal on that build, so silence is the only safe thing")
    r.check("it gets the same short replay it cannot tell from a whole one",
            got0 == [wire.msg(TAG_A, seqs[2], _blob("three")),
                     wire.msg(TAG_B, seq_b, _blob("untouched")), wire.eose()])

    r.step("a v>=1 subscriber whose cursor sits exactly at the floor")
    at_floor = hub.connect("laptop at floor")
    hub.subscribe(at_floor, [TAG_A], since=seqs[1], v=1)
    r.check("is NOT told it missed anything, because it did not",
            [f for f in at_floor.drain() if f["t"] == "gap"] == [])

    r.step("a later sweep that removes nothing")
    hub.evict()
    r.check("does not lower or clear the floor — the hole it describes is permanent",
            hub.tables().retention_floor.get(TAG_A) == seqs[1])

    r.step("the age policy, on a second store")
    aged = RelayState()
    for q, b in ((100, "a"), (200, "b"), (300, "c")):
        aged.append(TAG_A, q, b)
    r.check("cutoff = now - window sweeps ACROSS TAGS by seq alone",
            aged.evict(now_us=350, window_us=100, max_per_tag=0) == 2)
    r.check("and the floor is again the highest removed", aged.floors([TAG_A]) == {TAG_A: 200})
    r.note("seq is globally monotonic, so the age sweep cuts every tag at one "
           "cutoff — the floors it writes are a fact about the whole relay's clock")
    return r.done()


# ── F15 ─────────────────────────────────────────────────────────────────────


def f15_budget_refuses(on_step: Callable | None = None,
                       backend: str = "model") -> CaseResult:
    """Ok -> OverBudget -> Stale, and the relay refusing before the free tier.

    The relay is the only place a cap can be ENFORCED rather than observed after
    the fact: nothing else holds a key, and a client cannot write without a
    presigned URL. A dashboard alert tells you the bill already happened.
    """
    r = Run("F15", "Budget refuses before the free tier", backend, on_step)
    t0 = 1_700_000_000

    r.step("the default cap sits under R2's free tier")
    r.check("DEFAULT_BUDGET_BYTES is 9 GB against a 10 GB tier",
            DEFAULT_BUDGET_BYTES == 9_000_000_000,
            "the GB of headroom absorbs measurement lag and R2's own metadata")

    r.step("a relay that has never heard from R2")
    budget = Budget(max_bytes=1000, stale_after_secs=900)
    relay = R2Relay(budget=budget)
    r.expose(budget=budget)
    key = "ab" * 32
    r.check("refuses before its first measurement", budget.reserve(1, t0) is Verdict.STALE)
    r.check("and the presign refuses with budget_stale",
            relay.presign_put(key, 1, now_s=t0) is MediaError.BUDGET_STALE,
            "a meter that cannot see is not permission to keep spending")

    r.step("R2 answers: the bucket holds nothing")
    budget.record(0, t0)
    r.check("Ok — 600 of 1000 bytes", budget.reserve(600, t0) is Verdict.OK)

    r.step("a second upload is authorised against measured + in-flight")
    r.check("OverBudget — 600 reserved leaves 400",
            budget.reserve(500, t0) is Verdict.OVER_BUDGET,
            "counting authorised bytes as stored errs towards refusing early")
    r.check("but 400 exactly still fits", budget.reserve(400, t0) is Verdict.OK)
    r.check("committed() == the cap", budget.committed() == 1000)
    r.check("and the presign says over_budget",
            relay.presign_put(key, 1, now_s=t0) is MediaError.OVER_BUDGET)

    r.step("most of those presigns were never redeemed; R2 says 100")
    budget.record(100, t0)
    r.check("a measurement CLEARS in-flight reservations", budget.committed() == 100)
    r.check("so the gate reopens", budget.reserve(200, t0) is Verdict.OK)

    r.step("then the meter goes quiet for three poll intervals")
    r.check("Stale — it fails CLOSED", budget.reserve(1, t0 + 2701) is Verdict.STALE)
    r.check("and so does the presign",
            relay.presign_put(key, 1, now_s=t0 + 2701) is MediaError.BUDGET_STALE)

    r.step("the refusal ladder is ordered: key shape, then size, then budget")
    fresh = Budget(max_bytes=10 ** 12, stale_after_secs=900)
    fresh.record(0, t0)
    r.expose(budget=fresh)
    live = R2Relay(budget=fresh, max_bytes=1024)
    r.check("a bad key is refused before anything is reserved",
            live.presign_put("nothex", 1, now_s=t0) is MediaError.BAD_KEY)
    r.check("an oversized object is refused before the budget is touched",
            live.presign_put(key, 2048, now_s=t0) is MediaError.TOO_LARGE)
    r.check("nothing was reserved by either refusal", fresh.committed() == 0)
    url = live.presign_put(key, 1024, now_s=t0)
    r.check("a good request mints a URL bound to exactly that length",
            isinstance(url, str) and "X-Amz-SignedHeaders=content-length%3Bhost" in url)
    r.check("and the reservation happened BEFORE signing", fresh.committed() == 1024,
            "a URL that exists is a URL that may be redeemed")

    r.step("measurement itself is bounded")
    bucket = R2State()
    r.expose(bucket=bucket)
    for i in range(5):
        bucket.put_bytes(f"object {i}".encode() * 10)
    measured, pages, capped = bucket.measure(budget_max=10 ** 9)
    r.check("a small bucket measures in one page",
            (measured, pages, capped) == (bucket.total_bytes(), 1, False))
    r.check("MAX_LIST_PAGES is 64 and hitting it reads as FULL, not as a number",
            MAX_LIST_PAGES == 64,
            "a measurement we declined to finish is not evidence of headroom")
    r.note("Ok and Stale are reachable against the live binary; OverBudget needs a "
           "successful ListObjectsV2, which needs real R2 — see live/parity.py")
    return r.done()


ALL = [f13_relay_refuses_a_second_commit,
       f14_eviction_announces_a_gap,
       f15_budget_refuses]
