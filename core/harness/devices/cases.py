"""cases — what each platform can prove today, run against the real thing.

None of these carries a Rust milestone name, because none of them is a Rust
case: they are the platform legs multi-platform-harness.html §03 asks for, and
each exists to show one agent really drives its platform. I1 has a headless twin
(`m4_three_user_thread`, via app/ios/scripts/three-sim-thread-demo.sh); the W
cases have none, because the browser does not fold what the core folds.

    W1  two browser devices pair and converge over a real relay
    W2  the embed: pacific.js in a hostile page, driven through its shadow root
    W3  a person types in the embed, and a second browser device holds it
    W4  two commits issued at once both survive — RED until keyholder.js is fixed
    I1  a room converges across two phones, through real MLS

WHAT NONE OF THEM CROSSES. No case puts a phone and a browser in one scenario,
and that is a finding rather than an omission: a browser pairs two devices over
X25519 and folds the interior's own ops into a per-site log, while a phone pairs
by MLS contact bundle and folds core deltas into GroupObjects. There is no verb
both sides can answer the same way, and each agent's CANNOT table says so in
place. "Browser and core on one relay" waits on GroupObject conformity (§06).
"""
from __future__ import annotations

import asyncio
import secrets
from pathlib import Path
from typing import Any

from ..cases.framework import CaseResult, Run
from ..live.relay import Relay
from .runner import Runner


def _token() -> str:
    return secrets.token_hex(3)


async def _relay(workdir: Path, label: str) -> Relay:
    relay = Relay(workdir, label=label)
    await asyncio.to_thread(relay.start)
    return relay


def _finish(run: Run, runner: Runner | None, workdir: Path) -> None:
    if runner is None:
        return
    for line in runner.footer():
        run.note(line)
    path = workdir / f"{run.result.name}.transcript.txt"
    path.write_text("\n".join(a.line(400) for a in runner.transcript) + "\n")
    run.note(f"transcript: {path}")


def _msgs(world: Any, conv: str) -> list[str]:
    for c in (world or {}).get("convs", []):
        if c.get("id") == conv:
            return [m.get("s") for m in c.get("msgs", [])]
    return []


async def _blobs(relay: Relay | None, tag: str) -> int:
    if relay is None:
        return 0
    tables = await asyncio.to_thread(relay.tables)
    return sum(1 for t, _ in tables.blob if t == tag)


async def _evidence(run: Run, runner: Runner, relay: Relay | None, who: list[str]) -> None:
    """What each browser device logged, what it thinks its sync is, and what it
    holds — attached to a case the moment a wait fails, so a red case carries its
    own evidence instead of a timeout."""
    for name in who:
        ag = runner.devices[name]
        try:
            got = await ag.call("console")
            lines = [f"{x['kind']}: {x['text'][:200]}" for x in got["lines"]
                     if x["kind"] in ("pageerror", "console.error", "console.warning")]
            run.note(f"{name} console: " + (" | ".join(lines[-8:]) or "no warnings or errors"))
            run.note(f"{name} sync.status: {await ag.rpc('sync.status')}")
            ds = await ag.rpc("store.deltas")
            run.note(f"{name} holds {len(ds)} deltas: " + ", ".join(
                f"{d['a'][:8]}#{d['seq']}:{d['op'].get('t')}" for d in ds))
        except Exception as ex:       # noqa: BLE001 — evidence is best-effort
            run.note(f"{name} evidence unavailable: {type(ex).__name__}: {ex}")
    if relay is not None:
        tables = await asyncio.to_thread(relay.tables)
        per_tag: dict[str, int] = {}
        for t, _ in tables.blob:
            per_tag[t[:12]] = per_tag.get(t[:12], 0) + 1
        run.note(f"relay holds {len(tables.blob)} blobs: {per_tag}")


# ── W1 ─────────────────────────────────────────────────────────────────────────

async def w1(workdir: Path, on_step=None) -> CaseResult:
    from .web import TWO_DEVICE_SEED, WebAgent

    run = Run("W1", "Two browser devices pair and converge over a real relay",
              backend="web", on_step=on_step)
    workdir = workdir / "W1"
    relay = runner = None
    A, B = "ada/browser-a", "ada/browser-b"
    said_a, said_b = f"ballot papers are printed · {_token()}", f"bring them Thursday · {_token()}"
    try:
        relay = await _relay(workdir, "relay")
        runner = Runner()
        runner.add(WebAgent(A, workdir=workdir, relay_url=relay.url),
                   WebAgent(B, workdir=workdir, relay_url=relay.url))

        run.step("two Chromiums, each framing a keyholder on a port of its own")
        await runner.start()
        ia, ib = await runner.must(A, "identity"), await runner.must(B, "identity")
        run.check("two origins minted two different device keys",
                  bool(ia["pk"]) and bool(ib["pk"]) and ia["pk"] != ib["pk"],
                  f"{ia['pk'][:12]}… / {ib['pk'][:12]}…")
        for d in (A, B):
            await runner.must(d, "seed_world", seed=TWO_DEVICE_SEED)

        run.step("pair: each accepts the other's offer — the offer is what a QR carries")
        offer_a, offer_b = await runner.must(A, "device_offer"), await runner.must(B, "device_offer")
        tag_a = await runner.must(A, "device_accept", offer=offer_b)
        tag_b = await runner.must(B, "device_accept", offer=offer_a)
        run.check("both derived the same tag and neither sent it", tag_a == tag_b, tag_a[:16] + "…")

        run.step("both subscribe on the harness's own semaphore")
        sa, sb = await runner.must(A, "sync"), await runner.must(B, "sync")
        run.check("both are on that relay, on that tag",
                  sa.get("relay") == relay.url == sb.get("relay") and sa["tag"] == sb["tag"] == tag_a,
                  f"{sa} / {sb}")

        run.step("A says something; B is waited on until it holds it")
        pushed_before = await runner.must(B, "pushes")
        await runner.must(A, "commit", op={"t": "say", "conv": "c1", "s": said_a})
        got = await runner.until(lambda ans: said_a in _msgs(ans[B], "c1"),
                                 lambda ag: ag.call("world"), [B],
                                 what="B to fold A's message", timeout=30)
        if not run.check("B folded the message A authored", got.ok, got.detail()):
            await _evidence(run, runner, relay, [A, B])
        run.check("and was pushed it by its keyholder rather than asking",
                  (await runner.must(B, "pushes")) > pushed_before)
        mine = [d for d in await runner.must(B, "deltas") if d.get("a") == ia["pk"]]
        run.check("B's log holds A's delta under A's authorship, not its own",
                  len(mine) == 1 and mine[0]["op"].get("s") == said_a, str(mine)[:300])

        run.step("and back the other way, then both settle")
        await runner.must(B, "commit", op={"t": "say", "conv": "c1", "s": said_b})
        back = await runner.until(lambda ans: said_b in _msgs(ans[A], "c1"),
                                  lambda ag: ag.call("world"), [A],
                                  what="A to fold B's message", timeout=30)
        run.check("A folded the message B authored", back.ok, back.detail())
        settled = await runner.settle(lambda ag: ag.call("world"), [A, B], timeout=20)
        run.check("both devices are quiet and fold the same conversation",
                  settled.ok and _msgs(settled.answers[A], "c1") == _msgs(settled.answers[B], "c1")
                  and set(_msgs(settled.answers[A], "c1")) == {said_a, said_b},
                  settled.detail())

        run.step("what the relay actually stored")
        tables = await asyncio.to_thread(relay.tables)
        bodies = [str(b) for b in tables.blob.values()]
        run.check("the relay holds blobs on exactly one tag",
                  bool(bodies) and {t for t, _ in tables.blob} == {tag_a}, f"{len(bodies)} blobs")
        run.check("and none of them carries either message in the clear",
                  not any(said_a in b or said_b in b for b in bodies))
    except Exception as ex:           # noqa: BLE001 — a case that threw is a red case
        run.check("the case ran to the end", False, f"{type(ex).__name__}: {ex}")
    finally:
        _finish(run, runner, workdir)
        if runner:
            await runner.close()
        if relay:
            await asyncio.to_thread(relay.stop)
    return run.done()


# ── W2 ─────────────────────────────────────────────────────────────────────────

async def w2(workdir: Path, on_step=None) -> CaseResult:
    from .web import EmbedAgent

    run = Run("W2", "The embed: pacific.js in a hostile host page, driven through its shadow root",
              backend="embed", on_step=on_step)
    workdir = workdir / "W2"
    runner = None
    E = "bo/embed"
    said = f"I can bring the urn · {_token()}"
    try:
        runner = Runner()
        embed = EmbedAgent(E, workdir=workdir)
        runner.add(embed)

        run.step("a member's page with hostile CSS, and pacific.js mounted into it")
        await runner.start()
        run.check("the interior mounted against its keyholder", bool(embed.opened.get("pk")),
                  str(embed.opened))
        host = await runner.must(E, "host")
        run.check("the page is on the member's registered origin", host["origin"] == "http://localhost:8100",
                  host["origin"])
        run.check("the interior is one host element with a shadow root",
                  len(host["hosts"]) == 1 and host["hosts"][0]["shadow"], str(host["hosts"]))
        run.check("and none of it is in the host's light DOM", not host["interiorInLightDom"])
        run.check("the device key's database is not visible from the member's origin",
                  host["databases"] is not None and "pacific.keyholder" not in host["databases"],
                  f"host sees {host['databases']}")
        run.check("because it lives on the keyholder's origin, framed",
                  embed.keyholder.origin in host["frames"], str(host["frames"]))

        run.step("a person clicks the mark, then the chat disc")
        opened = await runner.must(E, "ui_open")
        run.check("the mark opens the window, not the full page", opened["win"] == "modal", str(opened))
        screen = await runner.must(E, "ui_view", view="chat")
        run.check("the chat disc lands on CHAT", screen["screen"] == "chat", str(screen))

        run.step("types a message into the composer and sends it")
        sent = await runner.must(E, "ui_chat_say", text=said)
        conv = sent["conv"]
        run.check("the message is on screen in the conversation", said in sent["rendered"],
                  sent["rendered"][-200:])
        run.check("in the world the interior draws", said in _msgs(await runner.must(E, "world"), conv))

        run.step("and asks the keyholder directly what it holds")
        held = await runner.must(E, "rpc", m="store.load")
        run.check("the keyholder folded it — the page did not just draw it", said in _msgs(held, conv))
        deltas = await runner.must(E, "deltas")
        authored = [d for d in deltas if d.get("op", {}).get("s") == said]
        run.check("as one delta authored by this device's key",
                  len(authored) == 1 and authored[0]["a"] == embed.opened["pk"], str(authored)[:300])
        shot = await runner.must(E, "screenshot", path=str(workdir / "W2-embed.png"))
        run.note(f"screenshot: {shot}")
    except Exception as ex:           # noqa: BLE001
        run.check("the case ran to the end", False, f"{type(ex).__name__}: {ex}")
    finally:
        _finish(run, runner, workdir)
        if runner:
            await runner.close()
    return run.done()


# ── W3 ─────────────────────────────────────────────────────────────────────────

async def w3(workdir: Path, on_step=None) -> CaseResult:
    from .web import EmbedAgent, WebAgent

    run = Run("W3", "A person types in the embed, and a second browser device holds it",
              backend="embed+web", on_step=on_step)
    workdir = workdir / "W3"
    relay = runner = None
    E, D = "bo/embed", "bo/browser"
    said_e, said_d = f"see you at the hall · {_token()}", f"doors at seven · {_token()}"
    try:
        relay = await _relay(workdir, "relay")
        runner = Runner()
        runner.add(EmbedAgent(E, workdir=workdir, relay_url=relay.url),
                   WebAgent(D, workdir=workdir, relay_url=relay.url))

        run.step("the embed mounts; the second device folds the same site's genesis")
        await runner.start()
        seed = await runner.must(E, "seed")
        await runner.must(D, "seed_world", seed=seed)
        convs = seed.get("convs") or []
        if not run.check("the site's world has a conversation to talk in", bool(convs)):
            return run.done()
        conv = convs[0]["id"]

        run.step("pair the embed's keyholder with the second device, and sync both")
        offer_e, offer_d = await runner.must(E, "device_offer"), await runner.must(D, "device_offer")
        tag_e = await runner.must(E, "device_accept", offer=offer_d)
        tag_d = await runner.must(D, "device_accept", offer=offer_e)
        run.check("same tag on both", tag_e == tag_d, tag_e[:16] + "…")
        await runner.must(E, "sync")
        await runner.must(D, "sync")

        run.step("a person types in the embed's CHAT")
        before = await _blobs(relay, tag_e)
        await runner.must(E, "ui_chat_say", text=said_e, conv=conv)
        reached = await runner.until(lambda ans: ans[E] > before,
                                     lambda ag: _blobs(relay, tag_e), [E],
                                     what="the embed's commit to reach the relay", timeout=15)
        run.check("what was typed reached the relay as a sealed blob", reached.ok,
                  f"blobs on the tag: {before} before, {reached.answers.get(E)} after" if reached.ok
                  else reached.detail())
        got = await runner.until(lambda ans: said_e in _msgs(ans[D], conv),
                                 lambda ag: ag.call("world"), [D],
                                 what="the second device to fold the embed's message", timeout=30)
        if not run.check("the second device folded what was typed in the embed", got.ok, got.detail()):
            await _evidence(run, runner, relay, [E, D])

        run.step("the second device writes; the embed's screen shows it without being touched")
        await runner.must(D, "commit", op={"t": "say", "conv": conv, "s": said_d})
        shown = await runner.until(lambda ans: said_d in ans[E],
                                   lambda ag: ag.call("ui_text", selector="#chlog"), [E],
                                   what="the embed to redraw with the other device's message",
                                   timeout=30)
        run.check("the embed redrew CHAT with the other device's message", shown.ok, shown.detail())
        settled = await runner.settle(lambda ag: ag.call("world"), [E, D], timeout=20)
        run.check("both settle on the same conversation",
                  settled.ok and _msgs(settled.answers[E], conv) == _msgs(settled.answers[D], conv),
                  settled.detail())
        shot = await runner.must(E, "screenshot", path=str(workdir / "W3-embed.png"))
        run.note(f"screenshot: {shot}")
    except Exception as ex:           # noqa: BLE001
        run.check("the case ran to the end", False, f"{type(ex).__name__}: {ex}")
    finally:
        _finish(run, runner, workdir)
        if runner:
            await runner.close()
        if relay:
            await asyncio.to_thread(relay.stop)
    return run.done()


# ── W4 ─────────────────────────────────────────────────────────────────────────

async def w4(workdir: Path, on_step=None) -> CaseResult:
    """Two commits at once. Red until keyholder.js serialises append().

    Found by W3, whose first runs lost a message: the embed's conversation click
    committed a `read` and the typed `say` followed within milliseconds. Both
    were published, both under the embed's key AND seq 0. The embed's own log
    kept only the `say` (it overwrote the `read`); the paired device kept only the
    `read` (it dropped the `say` as a re-delivery). Nothing logged an error.

    This pins that mechanism directly, on one device with nothing else moving, so
    the day it goes green is the day the fix landed.
    """
    from .web import TWO_DEVICE_SEED, WebAgent

    run = Run("W4", "Two commits issued at once both survive in the device's log",
              backend="web", on_step=on_step)
    workdir = workdir / "W4"
    runner = None
    D = "ada/browser"
    first, second = f"first · {_token()}", f"second · {_token()}"
    try:
        runner = Runner()
        runner.add(WebAgent(D, workdir=workdir))

        run.step("one browser device, one conversation, nothing paired")
        await runner.start()
        await runner.must(D, "seed_world", seed=TWO_DEVICE_SEED)

        run.step("two sends in the same instant — a quick double send, or one site in two tabs")
        answers = await runner.must(D, "rpc_many", calls=[
            {"m": "store.commit", "a": {"op": {"t": "say", "conv": "c1", "s": first}}},
            {"m": "store.commit", "a": {"op": {"t": "say", "conv": "c1", "s": second}}}])
        run.check("the keyholder accepted both commits", all(a["ok"] for a in answers),
                  str(answers)[:300])

        deltas = await runner.must(D, "deltas")
        said = [d["op"].get("s") for d in deltas]
        seqs = [d["seq"] for d in deltas]
        run.check("its log holds both messages", first in said and second in said,
                  f"the log holds {len(deltas)} delta(s): "
                  + ", ".join(f"#{d['seq']} {d['op'].get('s')!r}" for d in deltas))
        run.check("under two different seqs", len(seqs) == 2 and len(set(seqs)) == 2, f"seqs {seqs}")
        world = await runner.must(D, "world")
        run.check("and the world it folds shows both",
                  first in _msgs(world, "c1") and second in _msgs(world, "c1"),
                  str(_msgs(world, "c1")))
        run.note("RED UNTIL FIXED — the failure is the finding. keyholder.js append() picks the "
                 "next seq from a read of the log and writes afterwards, so two appends that both "
                 "read before either writes mint the same (author, seq). The log is keyed "
                 "['site','a','seq']: here the second write replaces the first, and on a paired "
                 "device the sync handler drops an (author, seq) it already holds as a re-delivery, "
                 "keeping the first. Either way one op is gone and nothing says so.")
    except Exception as ex:           # noqa: BLE001
        run.check("the case ran to the end", False, f"{type(ex).__name__}: {ex}")
    finally:
        _finish(run, runner, workdir)
        if runner:
            await runner.close()
    return run.done()


# ── I1 ─────────────────────────────────────────────────────────────────────────

async def i1(workdir: Path, on_step=None) -> CaseResult:
    from .ios import IOSAgent

    run = Run("I1", "A room converges across two phones, through real MLS",
              backend="ios", on_step=on_step)
    workdir = workdir / "I1"
    workdir.mkdir(parents=True, exist_ok=True)
    relay = runner = None
    A, B = "ada/phone", "bo/phone"
    post, reply = f"kickoff · {_token()}", f"I'll lock the studio · {_token()}"
    try:
        relay = await _relay(workdir, "relay")
        runner = Runner()
        runner.add(IOSAgent(A, relay.url, display_name="Ada"),
                   IOSAgent(B, relay.url, display_name="Bo"))

        run.step("boot two simulators of the harness's own and clean-install the app on both")
        await runner.start()
        ia, ib = await runner.must(A, "identity"), await runner.must(B, "identity")
        run.check("two phones minted two identities",
                  bool(ia["space"]) and bool(ib["space"]) and ia["space"] != ib["space"],
                  f"{ia['space'][:16]}… / {ib['space'][:16]}…")

        run.step("ada mints a room and adds bo by a fresh contact bundle — a real MLS Add")
        room = await runner.must(A, "room_new")
        run.check("the room's id came back out of the app", bool(room), room)
        bundle = await runner.must(B, "bundle")
        run.check("bo exported a fresh bundle, not the one from launch", bundle != ib["bundle"])
        await runner.must(A, "room_add", object=room, bundle=bundle)

        async def view(ag):
            (await ag.do("sync")).unwrap()
            return (await ag.do("obj_view", object=room)).unwrap()

        run.step("ada posts; bo syncs until he holds the post")
        await runner.must(A, "obj_post", object=room, text=post)
        got = await runner.until(lambda ans: any(r["text"] == post for r in ans[B]), view, [B],
                                 what="bo to fold ada's post", timeout=180, interval=1.0)
        run.check("bo joined from the Welcome and folded ada's post", got.ok, got.detail())

        run.step("bo replies; both sync until the reply sits under the post")
        await runner.must(B, "obj_reply", object=room, text=reply)
        both = await runner.until(
            lambda ans: all(any(r["text"] == reply and r["depth"] == 1 for r in ans[d]) for d in (A, B)),
            view, [A, B], what="the reply at depth 1 on both phones", timeout=180, interval=1.0)
        run.check("the reply is nested under the post on both phones", both.ok, both.detail())

        settled = await runner.settle(view, [A, B], timeout=90, interval=1.0)
        run.check("both phones settle on the identical thread tree",
                  settled.ok and settled.answers[A] == settled.answers[B]
                  and [r["text"] for r in settled.answers[A]] == [post, reply],
                  settled.detail())

        drained = [a for a in runner.transcript if a.ok and isinstance(a.v, dict) and "line" in a.v]
        run.check("every command was acknowledged by the app's drain, none slept on",
                  bool(drained) and all(a.v["drained_ms"] >= 0 for a in drained),
                  f"{len(drained)} lines, slowest {max((a.v['drained_ms'] for a in drained), default=0)}ms")
        for d in (A, B):
            shot = await runner.must(d, "screenshot", path=str(workdir / f"I1-{d.replace('/', '-')}.png"))
            run.note(f"screenshot: {shot}")
    except Exception as ex:           # noqa: BLE001
        run.check("the case ran to the end", False, f"{type(ex).__name__}: {ex}")
    finally:
        _finish(run, runner, workdir)
        if runner:
            await runner.close()
        if relay:
            await asyncio.to_thread(relay.stop)
    return run.done()


GROUPS = {
    "w1": [w1], "w2": [w2], "w3": [w3], "w4": [w4], "i1": [i1],
    "web": [w1],
    "embed": [w2, w3],
    #: Do the drivers work? Green means every browser leg really drove its platform.
    "browser": [w1, w2, w3],
    #: What the drivers found. Red until the product is fixed, by design.
    "findings": [w4],
    "ios": [i1],
    "all": [w1, w2, w3, w4, i1],
}
