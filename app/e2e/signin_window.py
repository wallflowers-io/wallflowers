"""The Door's window for a first-timer (UX, 29 Sep: signin-new-primary 3d69c93a, words-hide-start c2f8961c).

On a deployed Door, headless, a virtual PRF passkey; every /v2/signup the page starts counted by a trap:
  W1  ?new: Create an account first and .primary, Sign in .quiet; one tap to the 24 words
  W2  the words shown: #start hidden, and a tap where Create an account stood starts no second /v2/signup
  W3  signin.js held 3 s (Fetch interception, this window): the buttons drawn disabled, a tap then starts
      nothing; once bound, the first tap goes on to the words

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  .venv/bin/python app/e2e/signin_window.py
"""
from __future__ import annotations

import asyncio
import json
import sys
from pathlib import Path

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import journeys as J  # noqa: E402
import wallflowers_path as w  # noqa: E402

say = J.say
TRAP = r"""(() => { const f = window.fetch; window.__signups = 0;
  window.fetch = (...a) => { if (/\/v2\/signup$/.test(String(a[0]?.url ?? a[0]))) window.__signups++; return f(...a); }; })();"""
LAYOUT = """(() => { const s = document.getElementById('start'), b = [...s.querySelectorAll('button')].filter(x => !x.hidden);
  return {order: b.map(x => x.id), new_cls: document.getElementById('new').className, in_cls: document.getElementById('in').className,
          disabled: {in: document.getElementById('in').disabled, new: document.getElementById('new').disabled}}; })()"""
RECT = "(() => { const r = document.getElementById('new').getBoundingClientRect(); return {x: r.left + r.width / 2, y: r.top + r.height / 2}; })()"


async def tap(p, x: float, y: float) -> None:
    for t in ("mousePressed", "mouseReleased"):
        await p.cdp("Input.dispatchMouseEvent", type=t, x=x, y=y, button="left", clickCount=1)


async def words_shown(p, secs: float = 60) -> int:
    await p.until("!document.getElementById('shown').hidden", "the recovery words", secs)
    return await p.ev("document.querySelectorAll('#list li').length")


async def hold_script(b, secs: float) -> asyncio.Task:
    """This window only: /door/signin.js held `secs` before it goes on (a second socket takes the events)."""
    import websockets
    port = int((b.prof / "DevToolsActivePort").read_text().split()[0])
    t = next(t for t in httpx.get(f"http://127.0.0.1:{port}/json/list", timeout=5).json() if t["type"] == "page")

    async def serve():
        async with websockets.connect(t["webSocketDebuggerUrl"], max_size=None) as ws:
            await ws.send(json.dumps({"id": 1, "method": "Fetch.enable",
                                      "params": {"patterns": [{"urlPattern": "*/door/signin.js*", "requestStage": "Request"}]}}))
            n = 1
            async for raw in ws:
                d = json.loads(raw)
                if d.get("method") == "Fetch.requestPaused":
                    await asyncio.sleep(secs)
                    n += 1
                    await ws.send(json.dumps({"id": n, "method": "Fetch.continueRequest", "params": {"requestId": d["params"]["requestId"]}}))
    task = asyncio.create_task(serve())
    await asyncio.sleep(1)
    return task


async def run(s: w.Stack, results: list) -> None:
    D = s.door_url

    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail)[:700]))
        say(sid, "G" if ok else "NG", json.dumps(detail)[:700])

    b = await J.Browser().open(s.work, "window-new")
    try:
        await b.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=TRAP)
        await b.p.go(D + "/signin?new")
        await b.p.until("!document.getElementById('new').disabled", "the window bound", 20)
        lay = await b.p.ev(LAYOUT)
        at = await b.p.ev(RECT)
        await b.p.shot(s.work, "W-new")
        await tap(b.p, at["x"], at["y"])
        await b.p.until("!document.getElementById('shown').hidden || !document.getElementById('words').hidden", "the words or the passkey step", 60)
        if await b.p.ev("document.getElementById('shown').hidden"):
            await b.p.click("saved")
        n = await words_shown(b.p)
        result("W1", lay["order"][:2] == ["new", "in"] and "primary" in lay["new_cls"] and "quiet" in lay["in_cls"] and n == 24,
               "?new: Create an account first and primary, Sign in quiet; one tap to the words", layout=lay, words=n)
        before = await b.p.ev("window.__signups")
        start_hidden = await b.p.ev("document.getElementById('start').hidden")
        await tap(b.p, at["x"], at["y"])
        await asyncio.sleep(2)
        after = await b.p.ev("window.__signups")
        await b.p.shot(s.work, "W-words")
        result("W2", start_hidden and after == before, "the words shown: #start hidden, and a tap where Create an account stood starts nothing",
               start_hidden=start_hidden, signups=[before, after])
    finally:
        b.close()

    b = await J.Browser().open(s.work, "window-held")
    held = None
    try:
        await b.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=TRAP)
        held = await hold_script(b, 3)
        await b.p.cdp("Page.navigate", url=D + "/signin?new")
        await b.p.until("!!document.getElementById('new')", "the window drawn", 10)
        early = await b.p.ev(LAYOUT)
        at = await b.p.ev(RECT)
        await tap(b.p, at["x"], at["y"])
        await asyncio.sleep(0.5)
        early_signups = await b.p.ev("window.__signups")
        await b.p.until("!document.getElementById('new').disabled", "the buttons bound", 20)
        # Bound, ?new puts Create an account first: it moved, so it is measured again.
        at = await b.p.ev(RECT)
        await tap(b.p, at["x"], at["y"])
        # As journeys' sign_up: the words, or the passkey step first where the one tap was not ready.
        await b.p.until("!document.getElementById('shown').hidden || !document.getElementById('words').hidden", "the words or the passkey step", 60)
        if await b.p.ev("document.getElementById('shown').hidden"):
            await b.p.click("saved")
        n = await words_shown(b.p)
        result("W3", early["disabled"] == {"in": True, "new": True} and early_signups == 0 and n == 24,
               "signin.js held 3 s: the buttons drawn disabled, a tap then starts nothing; bound, the first tap goes on",
               early=early["disabled"], early_signups=early_signups, words=n)
    finally:
        if held is not None:
            held.cancel()
        b.close()


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP):
        print(__doc__)
        return 2
    bp.map_name()
    s = w.Stack(host="localhost")
    results: list = []
    try:
        s.up()
        asyncio.run(asyncio.wait_for(run(s, results), 300))
    except (w.Fail, TimeoutError, OSError) as e:
        results.append(("--", "NG", f"{str(e) or type(e).__name__}"))
    finally:
        kept = s.work
        s.down()
    for sid, st, text in results:
        print(f"  {sid:<3} {st:<3}  {text}")
    print(f"screenshots: {kept}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("signin window: " + ("G" if not bad else f"NG at {', '.join(bad)}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
