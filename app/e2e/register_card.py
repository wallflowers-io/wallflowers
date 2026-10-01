"""A new owner's Register ends on Your card, with the name given on www (webapp/register-card 134ca736, Ralph
29 Sep: "patch in 2.2.2"). On a deployed Door, headless, a virtual PRF passkey:

  R1  www's draft (#register=, base64url JSON {kind, name, purpose, slug, pname, face}) rides the Door's window:
      a new account, its words, back to the webapp, the draft's sheet, Register. Then Your card, the first
      one: #youX hidden, Escape and the backdrop leave it up, #youName the draft's pname. Save: the card goes,
      /v2/me names them, and the Site Register made is theirs.
  R2  an account already named (layer 1's sign-up), the same: Register, the Site, and no card.

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  .venv/bin/python app/e2e/register_card.py
"""
from __future__ import annotations

import asyncio
import base64
import json
import sys
import time
import urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import ja_rehearsal as JR  # noqa: E402
import wallflowers_path as w  # noqa: E402

TAG = time.strftime("%m%d%H%M%S", time.gmtime())
PNAME = "Ana Kim"
say = JR.say
SHOWN = "(() => { const y = document.getElementById('you'); return !!y && !y.hidden && y.checkVisibility(); })()"
CARD = """(() => ({shown: (() => { const y = document.getElementById('you'); return !!y && !y.hidden && y.checkVisibility(); })(),
  x_hidden: (() => { const x = document.getElementById('youX'); return !x || x.hidden || !x.checkVisibility(); })(),
  name: (document.getElementById('youName') || {}).value ?? null}))()"""


def draft(site: str, slug: str) -> str:
    d = {"kind": "Community", "name": site, "purpose": "register_card.py", "slug": slug, "pname": PNAME, "face": None}
    return base64.urlsafe_b64encode(json.dumps(d).encode()).rstrip(b"=").decode()


async def register(p, work: Path, tag: str) -> str:
    await p.until("!document.getElementById('sheet').hidden && (document.getElementById('sheetGo') || {}).textContent === 'Register'",
                  "the draft's Register sheet", 60)
    title = await p.ev("document.getElementById('sheetTitle').textContent")
    await p.shot(work, f"R-{tag}-sheet")
    await p.click("sheetGo")
    await p.until("document.getElementById('sheet').hidden", "Register done", 60)
    return title


async def site_made(p, name: str) -> bool:
    return await p.ev(f"fetch('/v2/graph').then(r => r.json()).then(g => (g.objects || []).some(o => o.kind === 'group' && o.name === {json.dumps(name)}))")


async def key(p, k: str, code: str, vk: int) -> None:
    for t in ("keyDown", "keyUp"):
        await p.cdp("Input.dispatchKeyEvent", type=t, key=k, code=code, windowsVirtualKeyCode=vk)


async def tap(p, x: float, y: float) -> None:
    for t in ("mousePressed", "mouseReleased"):
        await p.cdp("Input.dispatchMouseEvent", type=t, x=x, y=y, button="left", clickCount=1)


async def run(s: w.Stack, results: list) -> None:
    D = s.door_url

    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail, ensure_ascii=False)[:900]))
        say(sid, "G" if ok else "NG", json.dumps(detail, ensure_ascii=False)[:900])

    # R1: a new, nameless owner by the Door's window.
    site1 = f"Register card {TAG}"
    b = await JR.window(s.work, "new-owner")
    try:
        await b.p.go(D + "/signin?new&return=" + urllib.parse.quote("/#register=" + draft(site1, f"regcard-{TAG}"), safe=""))
        words = await b.sign_up()
        title = await register(b.p, s.work, "new")
        t0 = time.monotonic()
        while time.monotonic() - t0 < 15 and not await b.p.ev(SHOWN):
            await asyncio.sleep(0.3)
        first = await b.p.ev(CARD)
        await b.p.shot(s.work, "R-new-card")
        kept = {}
        if first["shown"]:
            await key(b.p, "Escape", "Escape", 27)
            await asyncio.sleep(0.8)
            kept["escape"] = await b.p.ev(SHOWN)
            await tap(b.p, 4, 4)
            await asyncio.sleep(0.8)
            kept["backdrop"] = await b.p.ev(SHOWN)
            await b.p.click("youGo")
            t0 = time.monotonic()
            while time.monotonic() - t0 < 20 and await b.p.ev(SHOWN):
                await asyncio.sleep(0.3)
            kept["closed_on_save"] = not await b.p.ev(SHOWN)
        me = await b.me()
        made = await site_made(b.p, site1)
        result("R1", words == 24 and title == site1 and first["shown"] and first["x_hidden"] and first["name"] == PNAME
               and kept.get("escape") and kept.get("backdrop") and kept.get("closed_on_save") and me.get("display_name") == PNAME and made,
               "a new owner's Register ends on Your card, first, with the name given on www; saved, they are named",
               words=words, sheet=title, card=first, **kept, display_name=me.get("display_name"), site_made=made)
    finally:
        # Signed out, not left to idle out: door-test's timing runs gate on its session count.
        await b.p.ev("fetch('/v2/signout', {method: 'POST'}).then(r => r.status, () => 0)")
        b.close()

    # R2: an account already named.
    site2 = f"Register named {TAG}"
    named = w.signup(s, f"Named owner {TAG}")
    o = await JR.window(s.work, "named-owner")
    try:
        await JR.carry_session(o, D, named["client"])
        await o.p.go(D + "/#register=" + draft(site2, f"regnamed-{TAG}"))
        before = await o.me()
        title = await register(o.p, s.work, "named")
        await asyncio.sleep(8)
        card = await o.p.ev(CARD)
        await o.p.shot(s.work, "R-named-after")
        made = await site_made(o.p, site2)
        result("R2", bool(before.get("display_name")) and title == site2 and not card["shown"] and made,
               "an owner already named: Register, and no card", display_name=before.get("display_name"), sheet=title, card=card, site_made=made)
    finally:
        await o.p.ev("fetch('/v2/signout', {method: 'POST'}).then(r => r.status, () => 0)")
        o.close()
        named["client"].post("/v2/signout")


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP):
        print(__doc__)
        return 2
    bp.map_name()
    s = w.Stack(host="localhost")
    results: list = []
    try:
        s.up()
        asyncio.run(run(s, results))
    except (w.Fail, TimeoutError, OSError) as e:
        results.append(("--", "NG", f"{str(e) or type(e).__name__}"))
    finally:
        kept = s.work
        s.down()
    for sid, st, text in results:
        print(f"  {sid:<4} {st:<3}  {text}")
    print(f"screenshots: {kept}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("register card: " + ("G" if not bad else f"NG at {', '.join(bad)}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
