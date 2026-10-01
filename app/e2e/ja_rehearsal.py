"""J-A's dress rehearsal on door-test (Ralph, 28 Sep: Antoine, a fresh phone, a live donation, a
WallFlowers profile, landed in the egregore site), each step G or NG against named commits. door-test
only; the rehearsal claim key; demo codes, not Stripe (Software Management, 28 Sep).

  J0  the Site's owner, a layer-1 account (RESTATE_STATE, restate_timing.py's), signed in and held live
      for the whole run: a late joiner's view of the Site needs the owner's session (run 73)
  J1  the owner's window (the layer-1 session's cookie carried in) runs Ralph's 2.1.0 snippet
      (one-signin-2.1.0.js, door-test's rehearsal of it): the three rooms marked with their choice
  J2  the kiosk (egregore egg/kiosk at JA_KIOSK, built, KIOSK_DEMO) signing for the Site: FINANCIAL
      SUPPORT by the paid route; its QR decoded, no QR before the payment is numbered
  J3  the visitor by the QR in an iPhone-sized headless window (the simulator's Safari cannot finish a
      passkey ceremony under automation: run 75): /join, the passkey, the words, joined, a share
  J4  the profile card: the webapp's sheet, a name and a picture, saved
  J5  the handoff to `home` (the join's answer), the site's sign-in: consent, the passkey again,
      landed in the community, named; on the one-QR site, by the language choice (the rail's CLOSE,
      the Profile button), then ENGLISH and FORUMS
  J6  his room only: the Financial Support room, drawn by the site and held in his own graph; not
      Resources, not Skills, never Healing Resistance
  J7  a post in his room shows there
  J8  his card seen by a second member (a fresh member by claim, layer 1): the Site's view's
      profiles[his key] carries his name, and his picture

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  RESTATE_STATE=<the owner's file> [JA_A11_MEMBER=<a door.2 member of all three>] [JA_OWNER=offline] JA_KIOSK=<egregore checkout, built> JA_SNIPPET=<door-test's 2.1.0 snippet>
  JA_BUNDLES=<5 Arc bundles> JOURNEY_MARK=<a data URL's file> JA_SITE=http://127.0.0.1:3471 (EGREGORE's build)
  [JA_CARD_SHEET, JA_CARD_NAME, JA_CARD_PHOTO, JA_CARD_SAVE: the card sheet's selectors, WEBAPP_HUMAN's by default]
  .venv/bin/python app/e2e/ja_rehearsal.py
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import re
import subprocess
import sys
import threading
import time
import urllib.parse
from pathlib import Path

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import entry_qr as E  # noqa: E402
import journeys as J  # noqa: E402
import wallflowers_path as w  # noqa: E402

STATE = Path(os.environ.get("RESTATE_STATE", ""))
A11_MEMBER = Path(os.environ.get("JA_A11_MEMBER", "/nonexistent"))
# JA_OWNER=offline (O-75, 2.2.0): the owner is not signed in during the run; the rooms were marked before
# (run 77). A newcomer must then hold, from the Arc's welcome alone, the Site's rooms, face and name,
# earlier members' cards and earlier posts (J9); the owner signs in only at the end, for J8.
OFFLINE = os.environ.get("JA_OWNER", "live") == "offline"
EARLIER = os.environ.get("JA_EARLIER_POST", "J-A rehearsal 0928")
UI = os.environ.get("JA_UI") == "1"   # EGREGORE's frozen client UI, U1–U14, after J7
KIOSK = Path(os.environ.get("JA_KIOSK", ""))
SNIPPET = Path(os.environ.get("JA_SNIPPET", ""))
BUNDLES = [b for b in Path(os.environ.get("JA_BUNDLES", "/dev/null")).read_text().splitlines() if b.strip()]
SITE = os.environ.get("JA_SITE", "http://127.0.0.1:3471").rstrip("/")
# The card, by the webapp's release: 2.2.1's one sheet (WEBAPP_HUMAN, 5cd130da) or 2.2.2's Your card dialog
# (239c6f21, #you); JA_CARD_<KEY> overrides one selector.
_CARDS = {"2.2.1": {"sheet": "#sheet", "name": "#cardName", "photo": "#sheetFields input[type=file]", "save": "#sheetGo",
                    "why": "#sheetWhy", "title": "#sheetTitle", "close": "#sheetX"},
          "2.2.2": {"sheet": "#you", "name": "#youName", "photo": "#youBody input[type=file]", "save": "#youGo",
                    "why": "#youWhy", "title": "#youBody h3", "close": "#youX"}}
CARD = {k: os.environ.get(f"JA_CARD_{k.upper()}", d) for k, d in _CARDS[os.environ.get("JA_CARD", "2.2.1")].items()}
CARD_OPEN = ("(() => { const e = document.querySelector(" + json.dumps(CARD["sheet"]) + "); return !!e && !e.hidden && "
             "(document.querySelector(" + json.dumps(CARD["title"]) + ") || {}).textContent === 'Your card'; })()")
TAG = time.strftime("%m%d%H%M", time.gmtime())
NAME = f"Antoine rehearsal {TAG}"
# EGREGORE's community: the redesign's (a dialog's section, named by its aria-label, rooms as [data-row]),
# or the earlier panel (its h1, rooms as buttons in a list).
LANDED = ("(document.querySelector('dialog[open] section[lang]')?.getAttribute('aria-label') || '').includes('/forums') || "
          "!!document.querySelector('section[lang] h1')")
ROWS = "[...document.querySelectorAll('dialog[open] section[lang] [data-row], section[lang] ul li button')]"
# The one-QR site (egregore b40e572, Ralph's ask): /community opens the egg past its door on the language
# choice, signed in; a language, then FORUMS, holds the member's room (EGREGORE's A3w).
DIALBTN = "[...document.querySelectorAll('dialog[open] [role=menu] button')]"
WELCOME = f"{DIALBTN}.some(x => /ENGLISH/i.test(x.textContent))"
# The ledger EGREGORE's site reads (its LEDGER_PATH), for U10; unset, a fresh site's empty one.
LEDGER = os.environ.get("JA_LEDGER", "")
THREE = {"resources": "Resources", "skills": "Skills, Time & Services", "financial": "Financial Support"}
HR = "Healing Resistance"
say = J.say
# A 1x1 PNG: the card's picture, given to the file input as a file.
PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")


def pinned_with(*extra: str):
    """The Door's pin, plus this run's needs: EGREGORE's site on the loopback address, and autoplay."""
    base = bp.pinned

    def f():
        flags = base()
        flags = [x.replace("EXCLUDE localhost", "EXCLUDE localhost, EXCLUDE 127.0.0.1") if x.startswith("--host-resolver-rules") else x
                 for x in flags]
        return [*flags, *extra]
    return base, f


async def window(work: Path, tag: str, *extra: str) -> J.Browser:
    base, f = pinned_with(*extra)
    bp.pinned = f
    try:
        return await J.Browser().open(work, tag)
    finally:
        bp.pinned = base


async def carry_session(b: J.Browser, door: str, client: httpx.Client) -> None:
    """The layer-1 session's cookies into this window, so its page is the owner's."""
    await b.p.cdp("Network.enable")
    for c in client.cookies.jar:
        await b.p.cdp("Network.setCookie", name=c.name, value=c.value, url=door, path=c.path or "/", secure=True,
                      httpOnly=bool(c.has_nonstandard_attr("HttpOnly") or c.name.startswith("__Host-")), sameSite="Strict")


# ---- EGREGORE's frozen client UI (egregore 12d982d; EGREGORE's map, 29 Sep), on the site, signed in ----
SEC = "document.querySelector('dialog[open] section[lang]')"
PATHJS = f"(({SEC} || {{}}).getAttribute ? {SEC}.getAttribute('aria-label') : '') || ''"
ROWSJS = "[...document.querySelectorAll('dialog[open] section[lang] [data-row]')]"
DIALJS = "[...document.querySelectorAll('dialog[open] [role=menu] button')]"
RAILJS = "document.querySelector('dialog[open] [class*=bar]')"
# The dial, shown: its menu stays mounted under a community page, so "shown" is no section and a visible button
# (checkVisibility: the egg is position: fixed, so offsetParent is null throughout), and it is the dial's own
# (FORUMS): the language choice is a menu too (b40e572).
AT_DIAL = ("(!document.querySelector('dialog[open] section[lang]') && "
           "[...document.querySelectorAll('dialog[open] [role=menu] button')].some(b => b.checkVisibility() && b.textContent.trim() === 'FORUMS'))")


async def ui_round(v, work: Path, result, door: str, site_origin: str, hits: list) -> None:
    p = v.p

    async def path() -> str:
        return await p.ev(PATHJS)

    async def to(sid: str, expr: str, what: str, secs: float = 20) -> bool:
        try:
            await p.until(expr, what, secs)
            return True
        except w.Fail:
            await p.shot(work, f"U-{sid}-stuck")
            return False

    async def ensure_open() -> None:
        """The egg open: its screen, then ENTER (signed in), where a step left it closed."""
        if await p.ev("!!document.querySelector('dialog[open]')"):
            return
        await p.ev("(document.querySelector('[class*=screenBox] button') || {click() {}}).click()")
        await asyncio.sleep(1)
        await p.ev("([...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'ENTER') || {click() {}}).click()")
        await asyncio.sleep(1.5)

    async def at_dial() -> list:
        await ensure_open()
        # An egg reopened after a sign-in boots, then shows the language choice (b40e572) or the dial.
        try:
            await p.until(f"({AT_DIAL}) || ({WELCOME}) || !!document.querySelector('dialog[open] section[lang]')", "the egg, booted", 20)
        except w.Fail:
            pass
        if await p.ev(f"({WELCOME}) && !({AT_DIAL})"):
            await p.ev(f"{DIALBTN}.find(x => /ENGLISH/i.test(x.textContent)).click()")
            await asyncio.sleep(1)
        for _ in range(4):   # up to the dial by the rail's BACK; never its CLOSE, which shuts the egg
            if await p.ev(AT_DIAL):
                break
            await p.ev(f"(() => {{ const r = {RAILJS}; const b = r && [...r.querySelectorAll('button')].find(x => /^BACK$/i.test(x.textContent.trim())); if (b) b.click(); }})()")
            await asyncio.sleep(0.8)
        return await p.ev(f"{DIALJS}.map(b => b.textContent.trim())")

    async def dial(label: str) -> bool:
        await at_dial()
        await p.ev(f"({DIALJS}.find(b => b.textContent.trim() === {json.dumps(label)}) || {{click() {{}}}}).click()")
        return await to(label, f"({PATHJS}).includes({json.dumps('/' + label.lower())})", label, 20)

    async def escape() -> None:
        await p.cdp("Input.dispatchKeyEvent", type="keyDown", key="Escape", code="Escape", windowsVirtualKeyCode=27)
        await p.cdp("Input.dispatchKeyEvent", type="keyUp", key="Escape", code="Escape", windowsVirtualKeyCode=27)
        await asyncio.sleep(0.8)

    async def row(rx: str) -> None:
        await p.ev(f"({ROWSJS}.find(r => new RegExp({json.dumps(rx)}).test(r.textContent)) || {{click() {{}}}}).click()")

    # U1–U3: out of a room by the rail's BACK, by Escape, then history.back() to the dial.
    start = await path()
    await p.ev(f"({RAILJS}.querySelector('button') || {{click() {{}}}}).click()")
    await asyncio.sleep(0.8)
    after_back = await path()
    result("U1", "/forums/" in start and after_back.endswith("/forums"), "the rail's BACK steps out of a room", before=start, after=after_back)
    await row("Financial Support")
    in_room = await to("U2", f"({PATHJS}).includes('/forums/')", "the room again", 15)
    await escape()
    after_esc = await path()
    egg_open = await p.ev("!!document.querySelector('dialog[open]')")
    await p.shot(work, "U-after-escape")
    result("U2", in_room and after_esc.endswith("/forums"), "Escape steps out of a room", in_room=in_room, after=after_esc, egg_open=egg_open)
    await ensure_open()
    if not (await path()).endswith("/forums"):
        await dial("FORUMS")
    at_forums = await path()
    await p.ev("history.back()")
    await asyncio.sleep(1)
    dial_now = await p.ev(AT_DIAL)
    await p.shot(work, "U-after-back")
    result("U3", bool(dial_now), "history.back() from the rooms reaches the dial", from_=at_forums, dial_shown=dial_now,
           egg_open=await p.ev("!!document.querySelector('dialog[open]')"))
    await ensure_open()
    # U4: the dial.
    labels = set(await at_dial() or [])
    await p.shot(work, "U-dial")
    places = await p.ev(f"(() => {{ const b = {DIALJS}.find(b => b.textContent.trim() === 'PLACES'); "
                        "return b ? (b.disabled || b.getAttribute('aria-disabled') === 'true') : null; })()")
    result("U4", {"RESOURCES", "EVENTS", "FORUMS", "MEMBERS", "FUND"} <= labels and "TREASURY" not in labels and places is True,
           "the dial: FUND not TREASURY, PLACES locked", labels=sorted(labels), places_locked=places)
    # U5: the language cycle, and the answer room's line by language (FR falls back to EN).
    langs, lines = [], {}
    for _ in range(3):
        await dial("FORUMS")
        lang = await p.ev(f"(({RAILJS} || document).querySelector('button[aria-label^=\"Language:\"]') || {{}}).textContent || ''")
        lines[lang.strip()] = await p.ev(f"({ROWSJS}.find(r => /Financial Support|재정|Soutien/.test(r.textContent)) || {{}}).textContent || ''")
        langs.append(lang.strip())
        await p.ev(f"(({RAILJS} || document).querySelector('button[aria-label^=\"Language:\"]') || {{click() {{}}}}).click()")
        await asyncio.sleep(0.8)
    back_to = await p.ev(f"(({RAILJS} || document).querySelector('button[aria-label^=\"Language:\"]') || {{}}).textContent || ''")
    en, fr, ko = lines.get("EN", ""), lines.get("FR", ""), lines.get("KO", "")
    result("U5", langs == ["EN", "FR", "KO"] and back_to.strip() == "EN" and "put something in motion" in en
           and "put something in motion" in fr and any("가" <= ch <= "힣" for ch in ko),
           "the language cycles EN, FR, KO, EN; the answer room's line in EN, FR as EN, KO in Korean",
           cycle=langs + [back_to.strip()], en=en[:90], fr=fr[:90], ko=ko[:90])
    # U6: the profile icon opens the Door's #you in a new tab.
    await p.ev("window.__opened = []; window.open = (...a) => { __opened.push(a); return null; }; true")
    await p.ev(f"(({RAILJS} || document).querySelector('button[aria-label=Profile]') || {{click() {{}}}}).click()")
    opened = await p.ev("window.__opened")
    result("U6", opened == [[door + "/#you", "_blank", "noopener"]], "the profile icon opens the Door's #you in a new tab", opened=opened)
    # U7: EVENTS.
    await dial("EVENTS")
    await asyncio.sleep(1.5)
    ev_text = await p.ev(f"({SEC} || {{}}).innerText || ''")
    await p.shot(work, "U-events")
    day = re.search(r"\b(MON|TUE|WED|THU|FRI|SAT|SUN) \d{1,2} [A-Z]{3}\b", ev_text.upper())
    result("U7", "HEALING CIRCLE" in ev_text.upper() and bool(day), "EVENTS: the Face's event under its whole day",
           day=day.group(0) if day else None, text=ev_text[:160])
    # U8: RESOURCES: the pdf with CORS read in the egg, the one without as a link out.
    await dial("RESOURCES")
    await asyncio.sleep(1.5)
    res = await p.ev(f"{ROWSJS}.map(r => ({{tag: r.tagName, text: r.textContent.trim(), href: r.getAttribute('href') || ''}}))")
    await p.shot(work, "U-resources")
    before = len(hits)
    await row("Stand-in PDF")
    drawn = await to("U8", "!!document.querySelector('dialog[open] canvas[data-page]')", "the pdf drawn in the egg", 30)
    label = await p.ev("(document.querySelector('dialog[open] canvas[data-page]') || {getAttribute() { return ''; }}).getAttribute('aria-label')")
    await p.shot(work, "U-reader")
    fetched = [h for h in hits[before:] if h[0] == "GET" and h[1] == site_origin]
    await escape()
    out_of = await path()
    # The one without CORS: opened, it falls back to its title as a link out (EGREGORE: a[data-row], same tab).
    await row("Community resources")
    fell = await to("U8", "!!document.querySelector('dialog[open] section[lang] a[data-row][href*=\"example.org\"]')", "the link out", 30)
    out = await p.ev("[...document.querySelectorAll('dialog[open] section[lang] a[data-row]')].map(a => a.getAttribute('href'))")
    await p.shot(work, "U-reader-nocors")
    await escape()
    result("U8", drawn and label == "1 / 1" and bool(fetched) and fell and out_of.endswith("/resources"),
           "RESOURCES: the pdf served with CORS drawn in the egg (1 / 1), fetched from the site's origin; the one without "
           "CORS a link out; Escape back to the list", rows=[(r["tag"], r["text"][:50]) for r in res], page=label,
           fetched=len(fetched), after_escape=out_of, no_cors_link=out)
    # U9: MEMBERS.
    await dial("MEMBERS")
    await asyncio.sleep(1.5)
    cards = await p.ev(f"{ROWSJS}.map(r => r.textContent.trim())")
    await p.shot(work, "U-members")
    hexlike = [c for c in cards if re.fullmatch(r"[0-9a-f]{16,}.*", c)]
    role = next((c for c in cards if re.search(r"FACILITATOR|ADMIN", c)), None)
    ok_prof = False
    if role:
        await row(re.escape(role[:20]))
        ok_prof = await to("U9", f"({PATHJS}).includes('/members/')", "a member's card", 15)
        await p.shot(work, "U-member")
    result("U9", len(cards) >= 3 and not hexlike and bool(role) and ok_prof,
           "MEMBERS: cards named from profiles, a FACILITATOR or ADMIN, a card opens", cards=len(cards), role=(role or "")[:60],
           hexlike=len(hexlike), opened=ok_prof)
    # U10: FUND on door-test: the ledger the site reads (JA_LEDGER, its gestures in won: the one-QR runs' stand-ins
    # land there), its total, its count and every gift's number; with none named, the empty ledger.
    await dial("FUND")
    await asyncio.sleep(1.5)
    fund = await p.ev(f"({SEC} || {{}}).innerText || ''")
    await p.shot(work, "U-fund")
    ledger = [json.loads(l) for l in Path(LEDGER).read_text().splitlines() if l.strip()] if LEDGER else []
    total, count = sum(g["amountCents"] for g in ledger), len(ledger)
    missing = [g["n"] for g in ledger if f"#{g['n']}" not in fund]
    result("U10", f"₩{total:,}" in fund and f"GIFTS {count}" in fund and not missing,
           f"FUND: the ledger, ₩{total:,} and GIFTS {count}" + (", every gift's number" if ledger else " (empty)"),
           text=fund[:200], missing=missing[:5])
    # U11–U12: in his room, a reaction on and off, and a reply.
    await dial("FORUMS")
    await row("Financial Support")
    await to("U11", "!!document.querySelector('dialog[open] section[lang] ol > li')", "the room's messages", 20)
    last = "document.querySelector('dialog[open] section[lang] ol > li:last-of-type')"
    await p.ev(f"({last}.querySelector('[aria-label=React]') || {{click() {{}}}}).click()")
    await asyncio.sleep(0.8)
    picked = await p.ev(f"(() => {{ const b = [...{last}.querySelectorAll('button')].find(x => /\\p{{Extended_Pictographic}}/u.test(x.textContent) && !/\\d/.test(x.textContent)); "
                        "if (!b) return ''; b.click(); return b.textContent.trim(); })()")
    await asyncio.sleep(2)
    chip = await p.ev(f"(() => {{ const c = [...{last}.querySelectorAll('[data-on]')][0]; if (!c) return null; const s = getComputedStyle(c); "
                      "return {text: c.textContent.trim(), color: s.color, glow: s.textShadow || s.boxShadow}; })()")
    await p.shot(work, "U-reaction")
    if chip:
        await p.ev(f"[...{last}.querySelectorAll('[data-on]')][0].click()")
        await asyncio.sleep(2)
    off = await p.ev(f"[...{last}.querySelectorAll('[data-on]')].length === 0")
    result("U11", bool(picked) and bool(chip) and chip["text"].startswith(picked) and off,
           "a reaction: on, as its own chip in its colour, then off", emoji=picked, chip=chip, toggled_off=off)
    await p.ev(f"([...{last}.querySelectorAll('button')].find(b => b.textContent.trim() === 'REPLY') || {{click() {{}}}}).click()")
    replying = await to("U12", "/REPLYING TO /.test(document.querySelector('dialog[open] section[lang]').innerText)", "REPLYING TO", 10)
    reply = f"a reply {TAG}"
    await p.ev(f"""(() => {{ const i = document.querySelector('dialog[open] section[lang] form input');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, {json.dumps(reply)});
      i.dispatchEvent(new Event('input', {{bubbles: true}})); i.form.requestSubmit(); }})()""")
    shown = await to("U12", f"[...document.querySelectorAll('dialog[open] section[lang] ol li')].some(li => li.textContent.includes({json.dumps(reply)}))",
                     "the reply", 30)
    await p.shot(work, "U-reply")
    result("U12", replying and shown, "REPLY: REPLYING TO its author, the reply shown in the room", replying=replying, shown=shown)
    # U13: close (the backdrop) and reopen, then reload: the same page.
    here = await path()
    # A real tap outside the card: the viewport's corner, where the backdrop shows (the card sits inset).
    for kind in ("mousePressed", "mouseReleased"):
        await p.cdp("Input.dispatchMouseEvent", type=kind, x=4, y=4, button="left", clickCount=1)
    await asyncio.sleep(1)
    closed = await p.ev("!document.querySelector('dialog[open] section[lang]')")
    await p.ev("(document.querySelector('[class*=screenBox] button') || {click() {}}).click()")
    await asyncio.sleep(1)
    await p.ev("([...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'ENTER') || {click() {}}).click()")
    reopened = await to("U13", f"({PATHJS}) === {json.dumps(here)}", "the same page after reopening", 15)
    await p.cdp("Page.reload")
    await asyncio.sleep(3)
    await p.ev("(document.querySelector('[class*=screenBox] button') || {click() {}}).click()")
    await asyncio.sleep(1)
    await p.ev("([...document.querySelectorAll('button')].find(b => b.textContent.trim() === 'ENTER') || {click() {}}).click()")
    reloaded = await to("U13", f"({PATHJS}) === {json.dumps(here)}", "the same page after a reload", 20)
    result("U13", closed and reopened and reloaded, "closed by the backdrop and reopened, then reloaded: the same page",
           page=here, closed=closed, reopened=reopened, reloaded=reloaded, after=await path())
    # U14: the search at the foot: 16 px, so iOS does not zoom.
    await at_dial()   # the search lives on the dial (and KNOWLEDGE's tree), its text drawn by the canvas
    size = await p.ev("(() => { const i = document.querySelector('dialog[open] input[type=search]'); "
                      "return i ? parseFloat(getComputedStyle(i).fontSize) : null; })()")
    result("U14", size is not None and size >= 16, "the search's font is 16 px or more", font_px=size)


async def run(s: w.Stack, results: list) -> None:
    D = s.door_url
    opened: list[J.Browser] = []
    procs: list[subprocess.Popen] = []
    live = threading.Event()

    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail, ensure_ascii=False)[:1400]))
        say(sid, "G" if ok else "NG", json.dumps(detail, ensure_ascii=False)[:1400])
        return ok

    st = json.loads(STATE.read_text())
    site, rooms = st["site"], st["rooms"]
    owner = {"handle": st["owner"]["handle"], "pk": st["owner"]["pk"], "prf": bytes.fromhex(st["owner"]["prf"])}
    try:
        # J0: the owner, live; or, OFFLINE, not signed in by this run.
        home = None
        if OFFLINE:
            result("J0", True, "the Site's owner offline: not signed in during the run", site=site[:16])
        else:
            home = w.signin(s, owner)["client"]

            def keep() -> None:
                nonlocal home
                while not live.wait(15):
                    if home.get("/v2/me").status_code != 200:
                        home = w.signin(s, owner)["client"]
            threading.Thread(target=keep, daemon=True).start()
            result("J0", w.me(home).get("pk") is not None, "the Site's owner signed in and held live", site=site[:16])

        if not OFFLINE:  # J1 is the owner's; OFFLINE, the marks of run 77 are read back by a newcomer in J9
            # J1: the 2.1.0 snippet in the owner's window.
            o = await window(s.work, "owner")
            opened.append(o)
            await carry_session(o, D, home)
            await o.p.go(D + "/")
            await o.p.until("fetch('/v2/me').then(r => r.status === 200)", "the owner's page", 30)
            kid, key = next(iter(json.loads(w.CLAIM_KEYS.read_text()).items()))
            J.SNIPPET = SNIPPET
            lines = await o.snippet({"SITE": site, "SLUG": f"ja-{TAG}", "BUNDLES": BUNDLES[:5], "KID": kid, "KEY": key, "MARK": J.MARK})
            g = await o.graph()
            parts = next(x for x in g["objects"] if x["id"] == site)["view"].get("parts", [])
            marked = {p.get("choice"): p.get("part") for p in parts if p.get("role") == "room" and p.get("choice")}
            want = {c: rooms[n] for c, n in THREE.items()}
            bad = [x for x in lines if x.startswith(("error:", "threw:")) or "✗" in x]
            result("J1", marked == want and not [x for x in bad if "publish" not in x], "the owner's 2.1.0 snippet: the three rooms marked by choice",
                   marked={c: (r or "")[:16] for c, r in marked.items()}, lines=[x[:100] for x in lines], bad=bad)
            o.close()
            opened.remove(o)

        # A11, per choice (Software Assurance; 2.1.0 row 3): a claim's visitor lands in the room of its choice
        # only; a member admitted before 2.1.0 to all three rooms stays in all three.
        def rooms_of(c: httpx.Client) -> dict:
            ids = {x.get("id") for x in c.get("/v2/graph").json().get("objects", [])}
            return {n: rooms[n] in ids for n in (*THREE.values(), HR)}
        for c, name in THREE.items():
            m = w.webapp(s)
            j = m.get("/join", params={"claim": J.claim(site, c, "zzzzzzabcdefghjkmnpq" if c == "financial" else None)},
                      follow_redirects=False).status_code
            w.signup(s, f"ja a11 {c} {TAG}", m)
            r, t0 = m.post("/v2/join"), time.monotonic()
            while r.status_code == 503 and time.monotonic() - t0 < 30:
                time.sleep(1)
                r = m.post("/v2/join")
            time.sleep(3)
            held = rooms_of(m)
            result(f"A11{c[0]}", r.status_code == 200 and held == {n: n == name for n in held},
                   f"A11, 2.1.0: a {c} claim lands in {name} only", join=j, v2_join=r.status_code, held=held)
            m.post("/v2/signout")
        if A11_MEMBER.is_file():
            k2 = json.loads(A11_MEMBER.read_text())
            back = w.signin(s, {"handle": k2["member"]["handle"], "pk": k2["member"]["pk"], "prf": bytes.fromhex(k2["member"]["prf"])})["client"]
            time.sleep(3)
            held = rooms_of(back)
            result("A11x", held == {n: n != HR for n in held}, "A11, 2.1.0: a member of all three rooms before 2.1.0 stays in all three",
                   admitted_at=k2["build"], held=held, before=k2["held_at_door2"])
            back.post("/v2/signout")

        # J2: the kiosk, FINANCIAL SUPPORT, paid.
        kp, lp, sp = w.free_port(), w.free_port(), w.free_port()
        log = open(s.work / "kiosk.log", "w")
        env = {**os.environ, "KIOSK_DEMO": "1", "KIOSK_DEMO_PAYS_AFTER": "6", "KIOSK_PORT": str(kp), "KIOSK_SITE": f"http://127.0.0.1:{lp}",
               "KIOSK_PAY_BASE": f"http://127.0.0.1:{lp}", "KIOSK_CLAIM_KEY": w.KIOSK_SEED, "KIOSK_DOOR_URL": D, "KIOSK_CLAIM_SITE": site,
               "KIOSK_CLOCK_FILE": "", "SHOW_URL": "", "SOUND_PORT": str(sp)}
        procs.append(subprocess.Popen(["node", "egg/kiosk/stand-in-ledger.mjs", str(lp)], cwd=KIOSK, stdout=log, stderr=subprocess.STDOUT))
        procs.append(subprocess.Popen(["node", "egg/kiosk/dist/kiosk.mjs"], cwd=KIOSK, env=env, stdout=log, stderr=subprocess.STDOUT))
        w.wait_for(f"http://127.0.0.1:{kp}/emergence", "the kiosk", 60, proc=procs[1])
        k = await window(s.work, "kiosk", "--autoplay-policy=no-user-gesture-required")
        opened.append(k)
        await k.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=E.CAPTURE)
        await k.p.go(f"http://localhost:{kp}/emergence?at=profile&mute=1&bulb=0&timecode=0")
        steps, early = [], None
        for text in ("FINANCIAL SUPPORT", "I'M READY TO GIVE", "₩20,000", "GIVE ₩20,000", "Let's join our hands", "DOWNLOAD & JOIN THE COMMUNITY"):
            try:
                await E.click_text(k.p, text, 90)
                steps.append(text)
            except w.Fail as e:
                await k.p.shot(s.work, "K-stuck")
                steps.append(f"{text}: {str(e)[:100]}")
                break
            if text == "GIVE ₩20,000":
                early = await k.p.ev(E.QR_SRC.replace(".take img", ".join .take img"))
            await asyncio.sleep(1)
        q = await E.read_qr(k, s.work, "financial") if len(steps) == 6 and ":" not in steps[-1] else {"url": "", "decoded": None}
        result("J2", bool(q["url"]) and q["decoded"] == [q["url"]] and not early,
               "the kiosk: FINANCIAL SUPPORT, paid (demo), DOWNLOAD & JOIN, its QR decoded", steps=steps, qr=q["url"][:60],
               decoded=q["decoded"] == [q["url"]] if q["decoded"] is not None else "no BarcodeDetector", qr_before_payment=bool(early))
        k.close()
        opened.remove(k)
        for p in procs:
            p.terminate()
        if not q["url"]:
            return

        # J3: the visitor, by the QR.
        v = await window(s.work, "visitor")
        opened.append(v)
        await v.p.go(q["url"])
        landed = await v.p.ev("location.pathname + location.hash")
        await v.p.go(D + "/signin?new&return=" + urllib.parse.quote("/#join", safe=""))
        words = await v.sign_up()
        await v.p.until("location.hash.startsWith('#joined') || (document.getElementById('refused') && "
                        "!document.getElementById('refused').hidden) || " + CARD_OPEN, "#joined, the card, or a refusal", 90)
        hashed = await v.p.ev("location.hash")
        me = await v.me()
        share = urllib.parse.parse_qs(hashed.lstrip("#").replace("joined&", "", 1)).get("a", [""])[0]
        await v.p.shot(s.work, "V-joined")
        result("J3", words == 24 and (hashed.startswith("#joined") or bool(await v.p.ev(CARD_OPEN))),
               "the visitor by the QR: /join, the passkey, the words, joined", landed=landed, hash=hashed, share=share or None,
               pk=(me.get("pk") or "")[:20])
        mine = (me.get("pk") or "").removeprefix("ed25519:")

        # J4: the card.
        try:
            await v.p.until(CARD_OPEN, "the card sheet", 30)
            await v.p.until(f"(() => {{ const b = document.querySelector({json.dumps(CARD['save'])}); return !!b && !b.disabled; }})()", "Save enabled", 40)
            await v.p.ev(f"(() => {{ const i = document.querySelector({json.dumps(CARD['name'])}); i.value = {json.dumps(NAME)}; "
                         "i.dispatchEvent(new Event('input', {bubbles: true})); return true; })()")
            if await v.p.ev(f"!!document.querySelector({json.dumps(CARD['photo'])})"):
                f = s.work / "card.png"
                f.write_bytes(PNG)
                doc = await v.p.cdp("DOM.getDocument")
                node = await v.p.cdp("DOM.querySelector", nodeId=doc["root"]["nodeId"], selector=CARD["photo"])
                await v.p.cdp("DOM.setFileInputFiles", files=[str(f)], nodeId=node["nodeId"])
                await asyncio.sleep(2)
            await v.p.shot(s.work, "V-card")
            await v.p.ev(f"document.querySelector({json.dumps(CARD['save'])}).click()")
            # The webapp hands off to `home` the moment the card is saved: the name is kept from the read that
            # saw it, on the Door's page, not read again after the page has gone to the site.
            me, t0 = {}, time.monotonic()
            while time.monotonic() - t0 < 30 and not me.get("display_name"):
                try:
                    me = await v.p.ev(f"location.origin === {json.dumps(D)} ? fetch('/v2/me').then(r => r.json()) : null") or me
                except w.Fail:
                    pass
                await asyncio.sleep(0.2)
            why = await v.p.ev(f"location.origin === {json.dumps(D)} ? (document.querySelector({json.dumps(CARD['why'])}) || {{}}).textContent || '' : ''")
            result("J4", me.get("display_name") == NAME and not why, "the profile card: name and picture, saved",
                   display_name=me.get("display_name"), card_note=me.get("card_note"), sheet_why=why or None)
        except w.Fail as e:
            await v.p.shot(s.work, "V-card-stuck")
            result("J4", False, "the profile card", why=str(e)[:300])

        # J5w: the Site's sign-in window wears its Face (2f3285ae, 8ec5c7fe): a fresh window, no credential,
        # so it stays put: its name, mark and look from the Face, the WallFlowers mark at the foot, no wordmark.
        f = await window(s.work, "face-window")
        try:
            await f.p.go(D + "/signin?" + urllib.parse.urlencode({"client": "egregore-local", "redirect_uri": SITE + "/signin/callback",
                                                                  "code_challenge": "A" * 43, "code_challenge_method": "S256", "state": "x"}))
            await asyncio.sleep(2)
            await f.p.shot(s.work, "W-face-window")
            face = await f.p.ev("""(() => ({face: document.documentElement.hasAttribute('data-face'),
              name: (document.querySelector('h1.site-name') || {}).textContent || '', mark: !!document.querySelector('img.site-mark'),
              look: !![...document.querySelectorAll('link[rel=stylesheet]')].find(l => l.href.includes('/door/face.css')),
              foot: !!document.querySelector('footer.wf img[src="/door/wallflowers.png"]'), wordmark: !!document.querySelector('img.mark'),
              consent: !!document.getElementById('consent'), title: document.title}))()""")
            brand = httpx.get(f"http://127.0.0.1:8090/v1/face/ja-rehearsal/brand", timeout=10)
            bname = brand.json().get("name") if brand.status_code == 200 else None
            result("J5w", face["face"] and face["name"] == bname and face["look"] and face["foot"] and not face["wordmark"] and not face["consent"],
                   "the Site's sign-in window wears its Face: name, look, the WallFlowers mark at the foot, no wordmark, no consent",
                   brand=[brand.status_code, bname], **face)
        finally:
            f.close()

        # J5: the handoff and the site's sign-in.
        try:
            # The site, reached, starts the Door's sign-in at once: its page, or the Door's window for its client.
            await v.p.until(f"location.origin === {json.dumps(SITE)} || (location.pathname === '/signin' && "
                            "new URLSearchParams(location.search).get('client') === 'egregore-local')", "the handoff to the site", 60)
            handoff = await v.p.ev("location.href")
            # One step (2f3285ae): no consent; the passkey asked at once in the Site's window. Its own
            # button only where the platform would not ask without a tap.
            t0 = time.monotonic()
            while time.monotonic() - t0 < 90:
                if await v.p.ev("location.origin === " + json.dumps(SITE) + " && (" + LANDED + " || " + WELCOME + ")"):
                    break
                if await v.p.ev(f"location.origin === {json.dumps(D)} && !!document.getElementById('in') && "
                                "!document.getElementById('start').hidden && getComputedStyle(document.getElementById('in')).display !== 'none' "
                                "&& !document.getElementById('in').disabled && performance.now() > 8000"):
                    await v.p.shot(s.work, "V-window-button")
                    await v.p.click("in")
                await asyncio.sleep(0.5)
            welcome = None
            if await v.p.ev("location.origin === " + json.dumps(SITE) + " && " + WELCOME):
                await v.p.shot(s.work, "V-welcome")
                welcome = await v.p.ev("({rail: (document.querySelector('dialog[open] [class*=bar]') || {}).innerText?.replace(/\\s+/g, ' ') ?? null, "
                                       "profile: !!document.querySelector('dialog[open] button[aria-label=Profile]')})")
                await v.p.ev(f"{DIALBTN}.find(x => /ENGLISH/i.test(x.textContent)).click()")
                await v.p.until(f"{DIALBTN}.some(x => x.textContent === 'FORUMS')", "the dial, past the language", 30)
                await v.p.ev(f"{DIALBTN}.find(x => x.textContent === 'FORUMS').click()")
            await v.p.until("location.origin === " + json.dumps(SITE) + " && (" + LANDED + ")", "the community", 20)
            await v.p.until(f"{ROWS}.length > 0", "its rooms", 45)
            await asyncio.sleep(3)
            await v.p.shot(s.work, "V-landed")
            got = await v.p.ev("""(() => { const d = document.querySelector('dialog[open] section[lang]'), s = d || document.querySelector('section[lang]');
              return {url: location.href, name: d ? d.getAttribute('aria-label') : (s.querySelector('h1') || {}).textContent || '',
                      rooms: """ + ROWS + """.map(x => x.textContent.trim()),
                      not_reached: /not reached/i.test(document.body.innerText)}; })()""")
            result("J5", bool(got["name"]) and not got["not_reached"] and (welcome is None or ("CLOSE" in (welcome["rail"] or "") and welcome["profile"])),
                   "the handoff to home, the Site's window, the passkey again, landed in the community"
                   + (" by the language choice, signed in" if welcome else ""),
                   handoff=handoff.split("&code_challenge")[0][:120], welcome=welcome, **got)
            # J6: his room only, as the site draws it.
            drawn = [r for r in got["rooms"] if any(n in r for n in (*THREE.values(), HR))]
            result("J6a", drawn == [x for x in drawn if "Financial Support" in x] and any("Financial Support" in r for r in drawn),
                   "the site draws his room only", rooms=got["rooms"])
        except w.Fail as e:
            await v.p.shot(s.work, "V-site-stuck")
            result("J5", False, "the handoff and the site", why=str(e)[:300], at=await v.p.ev("location.href"))
        # J7: a post in his room: the room's own input, wherever the layout puts it at this width.
        INPUT = "[...document.querySelectorAll('form input:not([type=hidden]), form textarea')].find(e => e.offsetParent)"
        try:
            text = f"J-A rehearsal {TAG}"
            await v.p.ev("(() => { const all = " + ROWS + "; (all.find(x => /Financial Support/.test(x.textContent)) || all[0]).click(); })()")
            await v.p.until(f"!!{INPUT}", "the room's input", 20)
            # Posts from before he joined (earlier runs' J7): on screen before his own.
            earlier = await v.p.ev(f"[...document.querySelectorAll('li, p, article')].filter(e => e.textContent.includes({json.dumps(EARLIER)}) "
                                   f"&& !e.textContent.includes({json.dumps(TAG)})).length")
            result("J7a", earlier > 0, "posts from before he joined, shown in his room", earlier=earlier, looked_for=EARLIER)
            await v.p.ev(f"""(() => {{ const i = {INPUT};
              Object.getOwnPropertyDescriptor(Object.getPrototypeOf(i), 'value').set.call(i, {json.dumps(text)});
              i.dispatchEvent(new Event('input', {{bubbles: true}})); i.form.requestSubmit(); }})()""")
            await v.p.until(f"[...document.querySelectorAll('li, p, article')].some(e => e.textContent.includes({json.dumps(text)}))", "the post", 30)
            await v.p.shot(s.work, "V-posted")
            result("J7", True, "a post in his room shows there", text=text)
        except w.Fail as e:
            await v.p.shot(s.work, "V-post-stuck")
            result("J7", False, "a post in his room shows there", why=str(e)[:300])

        # U1–U14: EGREGORE's frozen client UI, on the site, signed in (JA_UI=1; the stand-in pdf served again).
        if UI:
            import pdf_viewer as PV
            PV.HITS.clear()
            srv, _ = PV.stand_in()
            try:
                if await v.p.ev("location.origin") != SITE:
                    await v.p.go(SITE + "/#community")
                    await asyncio.sleep(3)
                await ui_round(v, s.work, result, D, SITE, PV.HITS)
            except w.Fail as e:
                await v.p.shot(s.work, "U-stuck")
                result("U--", False, "the UI round stopped", why=str(e)[:300])
            finally:
                srv.shutdown()

        # J10: /#you opens his card sheet, his name in it; Escape dismisses it (WEBAPP_HUMAN, 2.2.1 final).
        try:
            await v.p.go(D + "/#you")
            await v.p.until(CARD_OPEN, "the card sheet by #you", 20)
            named = await v.p.ev(f"(document.querySelector({json.dumps(CARD['name'])}) || {{}}).value || ''")
            await v.p.shot(s.work, "V-you")
            if os.environ.get("JA_CARD") == "2.2.2":   # its close button (WEBAPP_HUMAN, 239c6f21)
                await v.p.ev(f"document.querySelector({json.dumps(CARD['close'])}).click()")
            else:                                        # 2.2.1: Escape (Cancel, the backdrop or Escape)
                await v.p.cdp("Input.dispatchKeyEvent", type="keyDown", key="Escape", code="Escape", windowsVirtualKeyCode=27)
                await v.p.cdp("Input.dispatchKeyEvent", type="keyUp", key="Escape", code="Escape", windowsVirtualKeyCode=27)
            await asyncio.sleep(1)
            gone = not await v.p.ev(CARD_OPEN)
            result("J10", named == NAME and gone, "#you opens his card, his name in it; it closes", name=named, dismissed=gone)
        except w.Fail as e:
            await v.p.shot(s.work, "V-you-stuck")
            result("J10", False, "#you opens his card sheet", why=str(e)[:300])

        # J6b: his own graph at the Door.
        await v.p.go(D + "/")
        g = await v.graph()
        ids = {x.get("id") for x in g.get("objects", [])}
        held = {n: rooms[n] in ids for n in (*THREE.values(), HR)}
        result("J6b", held == {"Resources": False, "Skills, Time & Services": False, "Financial Support": True, HR: False},
               "his own graph holds the Financial Support room only", held=held)
        # J9: what a newcomer holds of the Site and his room, from his own graph at the Door.
        by = {x.get("id"): x for x in g.get("objects", [])}
        sv = (by.get(site) or {}).get("view") or {}
        rv = (by.get(rooms["Financial Support"]) or {}).get("view") or {}
        partset = {p.get("part") for p in sv.get("parts") or []}
        others = {k: p.get("name") for k, p in (sv.get("profiles") or {}).items() if k != mine}
        posts = [m for m in rv.get("messages") or [] if EARLIER in json.dumps(m, ensure_ascii=False) and TAG not in json.dumps(m, ensure_ascii=False)]
        nine = {"parts": {st["host"], *(rooms[n] for n in THREE.values())} <= partset,
                "choices": sorted(p.get("choice") for p in sv.get("parts") or [] if p.get("choice")) == ["financial", "resources", "skills"],
                "face": bool(sv.get("face")), "name": bool((by.get(site) or {}).get("name")),
                "earlier cards": len([n for n in others.values() if n]) >= 3, "room's parent": (rv.get("parent") or {}).get("parent") == site,
                "earlier posts": len(posts) > 0}
        result("J9", all(nine.values()), "a newcomer holds the Site's rooms, face and name, earlier cards and earlier posts"
               + (" with the owner offline" if OFFLINE else ""), **nine, cards_seen=len(others), posts_seen=len(posts))
        v.close()
        opened.remove(v)

        # J8: a second member sees his card: the owner, a member before him (OFFLINE: signed in only now).
        if home is None:
            home = w.signin(s, owner)["client"]
        card, t0 = None, time.monotonic()
        while time.monotonic() - t0 < 60 and not card:
            view = next((x.get("view") or {} for x in home.get("/v2/graph").json().get("objects", []) if x.get("id") == site), {})
            card = (view.get("profiles") or {}).get(mine)
            if not card:
                time.sleep(2)
        result("J8", bool(card) and card.get("name") == NAME and bool(card.get("icon")), "a second member (the owner, a member before him) sees his card",
               card={k: (v2 if k != "icon" else bool(v2)) for k, v2 in (card or {}).items()}, after_s=round(time.monotonic() - t0, 1))
        # J8b: a member who joins after him.
        m = w.webapp(s)
        m.get("/join", params={"claim": w.claim(site, choice="resources")}, follow_redirects=False)
        w.signup(s, f"ja second member {TAG}", m)
        r, t0 = m.post("/v2/join"), time.monotonic()
        while r.status_code == 503 and time.monotonic() - t0 < 30:
            time.sleep(1)
            r = m.post("/v2/join")
        card, t0 = None, time.monotonic()
        while time.monotonic() - t0 < 60 and not card:
            g = m.get("/v2/graph").json()
            view = next((x.get("view") or {} for x in g.get("objects", []) if x.get("id") == site), {})
            card = (view.get("profiles") or {}).get(mine)
            time.sleep(2)
        m.post("/v2/signout")
        result("J8b", bool(card) and card.get("name") == NAME, "a member who joins after him sees his card",
               join=r.status_code, card={k: (v2 if k != "icon" else bool(v2)) for k, v2 in (card or {}).items()}, after_s=round(time.monotonic() - t0, 1))
    finally:
        live.set()
        for b in opened:
            b.close()
        for p in procs:
            p.terminate()
            p.wait(timeout=10)
        try:
            if home is not None:
                home.post("/v2/signout")
        except Exception:
            pass
        say("every Chrome closed; the kiosk stopped; the owner signed out")


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP and STATE.is_file() and SNIPPET.is_file() and (KIOSK / "egg/kiosk/dist/kiosk.mjs").is_file()):
        print(__doc__)
        return 2
    w.CLAIM_MJS = KIOSK / "egg" / "kiosk" / "claim.mjs"
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
        print(f"  {sid:<5} {st:<3}  {text}")
    print(f"screenshots and the kiosk's log: {kept}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("J-A: " + ("G" if not bad else f"NG at {', '.join(bad)}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
