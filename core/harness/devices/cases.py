"""cases — what each platform can prove today, run against the real thing.

None of these carries a Rust milestone name, because none of them is a Rust
case: they are the platform legs multi-platform-harness.html §03 asks for, and
each exists to show one agent really drives its platform. I1 has a headless twin
(`m4_three_user_thread`, via app/ios/scripts/three-sim-thread-demo.sh).

    I1  a room converges across two phones, through real MLS

The browser leg (W1–W4) drove the keyholder, an origin that held device keys in the
browser. It was retired with the keyholder: the web signs in through the Door, and no
browser holds a key.
"""
from __future__ import annotations

import asyncio
import secrets
from pathlib import Path

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
    "i1": [i1],
    "ios": [i1],
    "all": [i1],
}
