"""parity — drive the real relay and the model with the SAME sequence, diff both.

Two implementations agreeing is the only thing that can catch them drifting apart.
So every step here happens twice: once over a websocket to a real `semaphore`
process, once against `RelayState`/`RelayHub`, and then

  * the FRAMES are compared (the client's view: Ack, Msg, Gap, Eose), and
  * the TABLES are compared (the relay's view: blob, commit_slot,
    retention_floor, meta — read out of the relay's own SQLite file).

The model follows the relay's seq numbers, because the relay mints them from its
clock and nothing else could agree with that. Everything else — whether a commit
is accepted, which blob survives, what the floor is, who is told about a Gap — is
the model PREDICTING, and a disagreement is a finding.
"""
from __future__ import annotations

import asyncio
import base64
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable
from urllib.parse import parse_qs, urlparse

from .. import address, wire
from ..cases.framework import CaseResult, Run
from ..sigv4 import presign
from ..state import Budget, MediaError, R2Relay, RelayHub, Retention, Verdict
from .driver import Client
from .relay import Relay

# Real addresses: the relay refuses a Pub its tag did not sign (address.py).
TAG_A = address.named("A").tag_hex
TAG_B = address.named("B").tag_hex
KEY = "cd" * 32


def _blob(text: str) -> str:
    return base64.b64encode(text.encode()).decode()


def _scratch(name: str) -> Path:
    return Path(tempfile.mkdtemp(prefix=f"harness-{name}-"))


def _diff_tables(r: Run, relay: Relay, hub: RelayHub, label: str) -> None:
    model, real = hub.tables(), relay.tables()
    r.check(f"tables agree {label}", model == real,
            "; ".join(model.diff(real))[:400] or "blob/commit_slot/retention_floor/meta")


# ── F13 ─────────────────────────────────────────────────────────────────────


async def f13_live(on_step: Callable | None = None) -> CaseResult:
    r = Run("F13", "Relay refuses a second commit", "REAL semaphore + model", on_step)
    with Relay(_scratch("f13"), label="f13") as relay:
        hub = RelayHub(retention=Retention())
        ada, bo, eve = [await Client.connect(relay.url, n) for n in ("ada", "bo", "eve")]
        m_ada, m_bo, m_eve = [hub.connect(n) for n in ("ada", "bo", "eve")]
        try:
            r.step("eve subscribes to the group-epoch tag she must be able to route")
            live = await eve.sub([TAG_A], since=0, v=1)
            hub.subscribe(m_eve, [TAG_A], since=0, v=1)
            r.check("an empty tag replays nothing and says Eose",
                    live == m_eve.drain() == [wire.eose()], str(live))

            r.step("ada commits")
            live = await ada.pub(TAG_A, _blob("ada's commit"), commit=True)
            seq_win = live[-1]["seq"]
            hub.publish(m_ada, TAG_A, _blob("ada's commit"), commit=True, seq=seq_win)
            r.check("the winner's Ack matches the model's frame for frame",
                    live == m_ada.drain(), str(live))
            r.check("the relay minted a microsecond timestamp, not a counter",
                    seq_win > 1_700_000_000_000_000, f"seq={seq_win}")
            live_eve = await eve.quiet()
            r.check("eve is fanned out the sealed blob and its seq",
                    live_eve == m_eve.drain() == [wire.msg(TAG_A, seq_win, _blob("ada's commit"))])

            r.step("bo commits to the same tag")
            live = await bo.pub(TAG_A, _blob("bo's commit"), commit=True)
            model_seq, _, model_ok = hub.publish(m_bo, TAG_A, _blob("bo's commit"),
                                                 commit=True, seq=live[-1]["seq"])
            r.check("the loser gets Ack{seq:0, ok:false}", live == [wire.ack(0, ok=False)],
                    str(live))
            r.check("the model predicted the rejection independently",
                    (model_seq, model_ok) == (0, False) and m_bo.drain() == live)
            r.check("and nothing was fanned out to eve",
                    await eve.quiet() == m_eve.drain() == [])
            _diff_tables(r, relay, hub, "after the lost race")

            r.step("ordinary traffic on that tag continues")
            live = await ada.pub(TAG_A, _blob("chatter"))
            hub.publish(m_ada, TAG_A, _blob("chatter"), seq=live[-1]["seq"])
            r.check("the slot gates COMMITS, not the tag",
                    live[-1]["ok"] is True and m_ada.drain()[-1] == live[-1])
            await eve.quiet(); m_eve.drain()

            r.step("a different tag has its own slot")
            live = await bo.pub(TAG_B, _blob("bo's own epoch"), commit=True)
            seq_b = live[-1]["seq"]
            hub.publish(m_bo, TAG_B, _blob("bo's own epoch"), commit=True, seq=seq_b)
            r.check("bo's own group-epoch is accepted", live[-1]["ok"] is True)
            m_bo.drain()

            r.step("a third commit attempt, after a restart of the connection")
            await ada.close()
            ada = await Client.connect(relay.url, "ada")
            live = await ada.pub(TAG_A, _blob("try again"), commit=True)
            hub.publish(m_ada, TAG_A, _blob("try again"), commit=True, seq=live[-1]["seq"])
            r.check("the slot is durable — still refused", live == [wire.ack(0, ok=False)])
            _diff_tables(r, relay, hub, "at the end")
            r.check("the slot table holds exactly the two winners' seqs",
                    relay.tables().commit_slot == {TAG_A: seq_win, TAG_B: seq_b},
                    str(relay.tables().commit_slot))
        finally:
            for c in (ada, bo, eve):
                await c.close()
    return r.done()


# ── F14 ─────────────────────────────────────────────────────────────────────


async def f14_live(on_step: Callable | None = None) -> CaseResult:
    r = Run("F14", "Eviction announces a Gap", "REAL semaphore + model", on_step)
    with Relay(_scratch("f14"), max_per_tag=1, sweep_secs=1, label="f14") as relay:
        hub = RelayHub(retention=Retention(max_per_tag=1))
        ada = await Client.connect(relay.url, "ada")
        m_ada = hub.connect("ada")
        try:
            r.step("three blobs on one tag, one on a tag that stays under the cap")
            seqs = []
            for text in ("one", "two", "three"):
                live = await ada.pub(TAG_A, _blob(text))
                seqs.append(live[-1]["seq"])
                hub.publish(m_ada, TAG_A, _blob(text), seq=seqs[-1])
            live = await ada.pub(TAG_B, _blob("untouched"))
            seq_b = live[-1]["seq"]
            hub.publish(m_ada, TAG_B, _blob("untouched"), seq=seq_b)
            m_ada.drain()
            _diff_tables(r, relay, hub, "before the sweep")

            r.step("the relay's own sweep runs (RELAY_MAX_BLOBS_PER_TAG=1, every 1s)")
            floor = relay.wait_for_floor(TAG_A, timeout=10)
            hub.evict()
            r.check("the relay recorded a floor", floor is not None, f"floor={floor}")
            r.check("and it is the highest seq REMOVED, exactly as the model said",
                    floor == seqs[1] == hub.tables().retention_floor.get(TAG_A),
                    f"real={floor} model={hub.tables().retention_floor.get(TAG_A)} "
                    f"removed={seqs[:2]} kept={seqs[2]}")
            _diff_tables(r, relay, hub, "after the sweep")
            r.check("the under-cap tag has no floor row",
                    TAG_B not in relay.tables().retention_floor)

            r.step("a v>=1 subscriber arrives from cold")
            modern = await Client.connect(relay.url, "laptop v1")
            m_modern = hub.connect("laptop v1")
            live = await modern.sub([TAG_A, TAG_B], since=0, v=1)
            hub.subscribe(m_modern, [TAG_A, TAG_B], since=0, v=1)
            model = m_modern.drain()
            r.check("Gap first, then the short backlog, then Eose — model and relay agree",
                    live == model, f"real={live}\nmodel={model}")
            r.check("the Gap names the floor", live[0] == wire.gap(TAG_A, seqs[1]))
            r.check("only what survived is replayed",
                    [f for f in live if f["t"] == "msg"] ==
                    [wire.msg(TAG_A, seqs[2], _blob("three")),
                     wire.msg(TAG_B, seq_b, _blob("untouched"))])

            r.step("a v=0 subscriber arrives from cold")
            old = await Client.connect(relay.url, "phone v0")
            m_old = hub.connect("phone v0")
            live = await old.sub([TAG_A, TAG_B], since=0, v=0)
            hub.subscribe(m_old, [TAG_A, TAG_B], since=0, v=0)
            r.check("it is told NOTHING about the hole", live == m_old.drain()
                    and [f for f in live if f["t"] == "gap"] == [], str(live))
            r.check("and cannot tell this replay from a whole one",
                    [f["t"] for f in live] == ["msg", "msg", "eose"])

            r.step("a v>=1 subscriber whose cursor sits exactly at the floor")
            at = await Client.connect(relay.url, "at floor")
            m_at = hub.connect("at floor")
            live = await at.sub([TAG_A], since=seqs[1], v=1)
            hub.subscribe(m_at, [TAG_A], since=seqs[1], v=1)
            r.check("has missed nothing and is not told otherwise",
                    live == m_at.drain() and [f for f in live if f["t"] == "gap"] == [],
                    str(live))
            for c in (modern, old, at):
                await c.close()
        finally:
            await ada.close()
    return r.done()


# ── F15 ─────────────────────────────────────────────────────────────────────

R2_ENV = {
    "RELAY_R2_ENDPOINT": "acct.r2.cloudflarestorage.com",
    "RELAY_R2_BUCKET": "pacific-media",
    "RELAY_R2_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE",
    "RELAY_R2_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
}


def _reproduce(url: str, *, method: str, key: str | None, length: int | None) -> str:
    """Re-sign the relay's URL in Python, at the instant the relay signed it."""
    q = parse_qs(urlparse(url).query)
    now = datetime.strptime(q["X-Amz-Date"][0], "%Y%m%dT%H%M%SZ").replace(tzinfo=timezone.utc)
    segs = ["pacific-media"] + ([key] if key else [])
    return presign(method=method, host=R2_ENV["RELAY_R2_ENDPOINT"], path_segments=segs,
                   region="auto", expires_in=int(q["X-Amz-Expires"][0]),
                   signed_headers=([("content-length", str(length))] if length is not None else []),
                   query=[], now=now,
                   access_key_id=R2_ENV["RELAY_R2_ACCESS_KEY_ID"],
                   secret_access_key=R2_ENV["RELAY_R2_SECRET_ACCESS_KEY"])


async def f15_live(on_step: Callable | None = None) -> CaseResult:
    r = Run("F15", "Budget refuses before the free tier", "REAL semaphore + model", on_step)

    r.step("a relay with no object storage configured")
    with Relay(_scratch("f15a"), label="f15-unconfigured") as relay:
        c = await Client.connect(relay.url, "ada")
        await c.declare(2)   # media is level two — see Client.declare
        got = await c.media_put(KEY, 1024)
        r.check("MediaPut is refused loudly, not dropped",
                got == wire.media_err(KEY, "unconfigured"), str(got))
        got = await c.media_get(KEY)
        r.check("and so is MediaGet", got == wire.media_err(KEY, "unconfigured"), str(got))
        await c.close()

    r.step("a relay with a budget that has never heard back from R2")
    with Relay(_scratch("f15b"), label="f15-stale",
               env={**R2_ENV, "RELAY_R2_BUDGET_BYTES": "9000000000",
                    "RELAY_MEDIA_MAX_BYTES": "1024"}) as relay:
        c = await Client.connect(relay.url, "ada")
        await c.declare(2)   # media is level two — see Client.declare
        got = await c.media_put(KEY, 100)
        r.check("STALE — it fails closed before its first measurement",
                got == wire.media_err(KEY, "budget_stale"), str(got))
        model = R2Relay(budget=Budget(9_000_000_000, 2700), max_bytes=1024)
        r.check("the model refuses the same request the same way",
                model.presign_put(KEY, 100) is MediaError.BUDGET_STALE)
        got = await c.media_put(KEY, 4096)
        r.check("an oversized object is refused FIRST, before the budget is consulted",
                got == wire.media_err(KEY, "too_large"), str(got))
        r.check("the model orders the ladder identically",
                model.presign_put(KEY, 4096) is MediaError.TOO_LARGE)
        got = await c.media_put("nothex" * 4, 100)
        r.check("and a bad key before that",
                got == wire.media_err("nothex" * 4, "bad_key"), str(got))
        r.check("the model again", model.presign_put("nothex" * 4, 100) is MediaError.BAD_KEY)
        await c.close()

    r.step("a relay explicitly uncapped (RELAY_R2_BUDGET_BYTES=0) — the Ok leg")
    with Relay(_scratch("f15c"), label="f15-ok",
               env={**R2_ENV, "RELAY_R2_BUDGET_BYTES": "0"}) as relay:
        c = await Client.connect(relay.url, "ada")
        await c.declare(2)   # media is level two — see Client.declare
        got = await c.media_put(KEY, 1024)
        r.check("Ok — the relay mints a capability", got["t"] == "media_url", str(got)[:160])
        r.check("bound to exactly that length",
                "X-Amz-SignedHeaders=content-length%3Bhost" in got["url"])
        r.check("and TTL'd", got["expires_in"] == 300 and got["method"] == "PUT")
        r.check("the Python presigner reproduces the URL BYTE FOR BYTE",
                got["url"] == _reproduce(got["url"], method="PUT", key=KEY, length=1024),
                "botocore, sigv4.rs and harness.sigv4 now agree on a hand-rolled signature")
        got_g = await c.media_get(KEY)
        r.check("a GET capability carries no content-length",
                got_g["t"] == "media_url" and got_g["method"] == "GET"
                and "content-length" not in got_g["url"])
        r.check("and reproduces too",
                got_g["url"] == _reproduce(got_g["url"], method="GET", key=KEY, length=None))
        await c.close()

    r.step("OverBudget, the one leg the binary cannot be driven to")
    b = Budget(1000, 900)
    b.record(0, 1_700_000_000)
    ladder = [b.reserve(600, 1_700_000_000), b.reserve(500, 1_700_000_000),
              b.reserve(1, 1_700_000_000 + 901)]
    r.check("the model walks Ok -> OverBudget -> Stale",
            ladder == [Verdict.OK, Verdict.OVER_BUDGET, Verdict.STALE], str(ladder))
    r.note("OverBudget needs a SUCCESSFUL ListObjectsV2, and `measure_usage` only "
           "accepts https from the real endpoint — so against this binary the leg is "
           "model-only. Everything either side of it (unconfigured, too_large, "
           "bad_key, budget_stale, and a minted, byte-identical URL) is live.")
    return r.done()


# ── runner ──────────────────────────────────────────────────────────────────

LIVE = [f13_live, f14_live, f15_live]


async def run_all(on_step: Callable | None = None) -> list[CaseResult]:
    return [await case(on_step) for case in LIVE]


def main() -> int:
    from ..cases.framework import report
    results = asyncio.run(run_all())
    text, green = report(results)
    print(text)
    return 0 if green else 1
