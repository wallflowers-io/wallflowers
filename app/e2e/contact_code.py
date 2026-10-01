"""A contact code adds its person to a Site and its rooms (W-96: door/contact-code, webapp/contact-code; BUILD, 30 Sep).

The loopback system (the Door built from this checkout), headless Chromes each with its own profile and a
virtual PRF passkey; A's /v2/add calls recorded by a fetch trap:

  K1  a site's token (DPoP, a registered client) asks POST /v2/bundle: 403, "a contact code is the webapp's"
  K2  B signs up, and Your card's Copy a new code gives wf1.<six distinct key packages>
  K3  A signs up, Registers a Site, mints six rooms, and adds B by the code in Site settings: the Site and
      five rooms each take a package (six /v2/add, 2xx), in the order A's room list shows them; the sixth, the last
      shown, is named "needs another code"
  K4  B, still signed in, holds the Site and those five rooms, not the one named; B's page shows the Site
  K5  B signs out and in again in the window: still holds them
  K6  the same code pasted again in A's browser: refused there ("Used already"), no /v2/add sent
  K7  B, not the owner: no Add by contact code in Site settings, and B's POST /v2/add of C's package to the Site
      is refused in core's words; C does not hold the Site
  K8  alone, under DOOR_CODE_SECS=5 (door/contact-code-life): B's code used by A after 10 s is refused by its kind (410,
      "the contact code has expired"), said on A's sheet without the MLS layer's text, and the Site's roster gains no one

  .venv/bin/python app/e2e/contact_code.py        (E2E_KEEP=1 keeps the screenshots)
  On a deployed Door, K1-K7: E2E_DOOR, E2E_DOOR_IP, E2E_DOOR_CA, E2E_AUTH as the other drivers take them
"""
from __future__ import annotations

import asyncio
import base64
import hashlib
import json
import os
import sys
import time
import urllib.parse
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent))
os.environ.setdefault("E2E_IDLE_SECS", "900")         # A waits minutes between its steps: 60 s would sign A out
import browser_path as bp  # noqa: E402
import journeys as J  # noqa: E402
import wallflowers_path as w  # noqa: E402

TAG = time.strftime("%m%d%H%M%S", time.gmtime())
NAME = f"Contact code {TAG}"
ROOMS = [f"Room {i}" for i in range(1, 7)]
say = J.say
TRAP = r"""(() => {
  const f = window.fetch; window.__adds = [];
  window.fetch = async (...a) => {
    const r = await f(...a);
    if (String(a[0]?.url ?? a[0]).endsWith('/v2/add')) window.__adds.push({status: r.status, body: (await r.clone().text().catch(() => '')).slice(0, 200)});
    return r;
  };
})();"""
OPT = "[...document.querySelectorAll('#manageBody .opt')].map(b => b.querySelector('b').textContent)"


async def headless(work: Path, tag: str) -> J.Browser:
    """journeys' Browser, headless (Ralph, 26 Sep): pinned to a deployed Door; on the loopback one, nothing to pin."""
    if w.DEPLOYED:
        return await J.Browser().open(work, tag)
    pinned, bp.pinned = bp.pinned, lambda to="": ["--headless=new"]
    try:
        return await J.Browser().open(work, tag)
    finally:
        bp.pinned = pinned


async def held(b: J.Browser, ids: list[str], secs: float = 60) -> set[str]:
    """Which of `ids` b's graph holds, once all are in or `secs` pass."""
    end, got = time.monotonic() + secs, set()
    while time.monotonic() < end:
        g = await b.graph()
        got = {o.get("id") for o in g.get("objects", [])} & set(ids)
        if got == set(ids):
            break
        await asyncio.sleep(1)
    return got


def site_token(s: w.Stack, who: dict, client: str = w.CLIENT, callback: str = w.CALLBACK) -> tuple[str, object]:
    """`who`'s token for a registered client, as a site holds one."""
    verifier, state = w.b64u(os.urandom(32)), os.urandom(8).hex()
    cb = {"client": client, "redirect_uri": callback, "state": state, "code_challenge": w.b64u(hashlib.sha256(verifier.encode()).digest())}
    c = w.webapp(s)
    o = c.post("/v2/signin", json=w.work(c, "signin") or None).json()
    f = c.post("/v2/signin/finish", json={"attempt": o["attempt"], "handle": who["handle"], "sealed": w.seal(o["key"], who["prf"], o["attempt"]),
                                          "client": cb}).json()
    k = w.dpop_key()
    r = c.post("/v2/token", json={"code": parse_qs(urlsplit(f["redirect"]).query)["code"][0], "code_verifier": verifier,
                                  "client": client, "redirect_uri": callback}, headers={"dpop": w.dpop(k, "POST", f"{s.door_url}/v2/token")})
    if r.status_code != 200:
        raise w.Fail(f"/v2/token: {r.status_code} {r.text[:200]}")
    return r.json()["access_token"], k


async def copy_code(b: J.Browser, s: w.Stack, name: str) -> tuple[str, int]:
    """A new person through the window, their first card saved, and Your card's Copy a new code: (the code, words)."""
    await b.p.go(s.door_url + "/signin?new")
    words = await b.sign_up()
    await save_first_card(b, name)
    await b.p.click("meBtn")
    await b.p.until("[...document.querySelectorAll('#meMenu .mi')].some(e => e.textContent.trim() === 'Your card')", "the menu", 10)
    await b.p.ev("[...document.querySelectorAll('#meMenu .mi')].find(e => e.textContent.trim() === 'Your card').click()")
    await b.p.until("!!document.getElementById('codeGo')", "Your contact code", 10)
    await b.p.click("codeGo")
    await b.p.until("(document.getElementById('codeOut') || {}).value?.startsWith('wf1.')", "the code", 60)
    code = await b.p.ev("document.getElementById('codeOut').value")
    await b.p.shot(s.work, f"code-{name.split()[0]}")
    await b.p.ev("document.getElementById('youX').click()")
    return code, words


async def save_first_card(b: J.Browser, name: str) -> None:
    await b.p.until("!document.getElementById('you').hidden && !!document.getElementById('youName')", "the first card", 90)
    await b.p.ev("(() => { const i = document.getElementById('youName'); if (!i.value) { i.value = " + json.dumps(name) + "; "
                 "i.dispatchEvent(new Event('input', {bubbles: true})); } })()")
    await b.p.until("!document.getElementById('youGo').disabled", "Save ready", 30)
    await b.p.click("youGo")
    await b.p.until("document.getElementById('you').hidden", "the first card saved", 60)


async def register(a: J.Browser, s: w.Stack) -> tuple[str, int]:
    """A new owner Registers a Site through the window and saves their first card: (the Site's id, words)."""
    draft = {"kind": "Community", "name": NAME, "purpose": "contact_code.py", "slug": f"cc-{TAG}", "pname": "Ada " + TAG, "face": None}
    await a.p.go(s.door_url + "/signin?new&return=" + urllib.parse.quote("/#register=" + J.b64u(json.dumps(draft)), safe=""))
    words = await a.sign_up()
    await a.p.until("!document.getElementById('sheet').hidden && document.getElementById('sheetGo').textContent === 'Register'", "the Register sheet", 90)
    await a.p.click("sheetGo")
    await a.p.until("document.getElementById('sheet').hidden && location.hash === ''", "the Site to open", 120)
    await save_first_card(a, "Ada " + TAG)
    g = await a.graph()
    site = next((o["id"] for o in g["objects"] if o.get("kind") == "group" and o.get("name") == NAME), None)
    if not site:
        raise w.Fail(f"no Site named {NAME} in A's graph")
    return site, words


async def add_by_code(a: J.Browser, code: str) -> None:
    """Site settings, Add by contact code, the code pasted, Add."""
    await a.p.click("siteRole")
    await a.p.until(f"{OPT}.includes('Add by contact code')", "Site settings", 10)
    await a.p.ev("[...document.querySelectorAll('#manageBody .opt')].find(b => b.querySelector('b').textContent === 'Add by contact code').click()")
    await a.p.until("!!document.getElementById('codeIn') && !document.getElementById('sheet').hidden", "the code sheet", 10)
    await a.p.ev("(() => { const i = document.getElementById('codeIn'); i.value = " + json.dumps(code) + "; i.dispatchEvent(new Event('input', {bubbles: true})); })()")
    await a.p.click("sheetGo")


def resulter(results: list):
    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail, ensure_ascii=False)[:900]))
        say(sid, "G" if ok else "NG", json.dumps(detail, ensure_ascii=False)[:900])
    return result


async def run(s: w.Stack, results: list) -> None:
    D, result = s.door_url, resulter(results)
    # K1 first: the client's registration restarts the Door, ending every session.
    C = w.signup(s, "contact C")
    c_site = C["client"].post("/v2/mint", json={"kind": "group", "draft": {"name": "C's"}}).json()["object_id"]
    if w.DEPLOYED:                                 # a deployed Door's own registered client (door-test: the quickstart's)
        tok, k = site_token(s, C, os.environ.get("CC_CLIENT", "starter-5173"), os.environ.get("CC_CALLBACK", "http://localhost:5173/"))
    else:
        s.register({w.CLIENT: {"callbacks": [w.CALLBACK], "origins": ["http://localhost:3100"], "site": c_site}})
        tok, k = site_token(s, C)
    r = w.webapp(s).post("/v2/bundle", headers={"authorization": f"DPoP {tok}", "dpop": w.dpop(k, "POST", f"{D}/v2/bundle", tok)})
    result("K1", r.status_code == 403 and "a contact code is the webapp's" in r.text, "a site's token asks for a contact code: refused",
           status=r.status_code, said=r.text[:200])
    c2 = w.signin(s, C)["client"]
    r = c2.post("/v2/bundle")
    c_code = r.json().get("bundles", []) if r.status_code == 200 else []

    opened: list[J.Browser] = []
    try:
        B = await headless(s.work, "B")
        opened.append(B)
        code, wb = await copy_code(B, s, "Bo " + TAG)
        body = code[4:] + "=" * (-len(code[4:]) % 4)
        packs = json.loads(base64.urlsafe_b64decode(body))
        result("K2", wb == 24 and len(packs) == 6 and len(set(packs)) == 6, "B's card gives a contact code: six distinct key packages",
               words=wb, packages=len(packs), distinct=len(set(packs)), chars=len(code))

        A = await headless(s.work, "A")
        opened.append(A)
        await A.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=TRAP)
        site, wa = await register(A, s)
        rooms = []
        for name in ROOMS:
            at = int(time.time() * 1000)
            rid = json.loads((await A.post("/v2/mint", {"kind": "forum", "draft": {"name": name}}))["body"])["object_id"]
            await A.post("/v2/apply", {"object": site, "op": "base.setPart", "args": {"part": rid, "role": "room", "at": at}})
            await A.post("/v2/apply", {"object": rid, "op": "base.setParent", "args": {"parent": site, "role": "room", "at": at}})
            rooms.append(rid)
        await A.p.until(f"document.body.innerText.includes({json.dumps(ROOMS[-1])})", "the six rooms shown", 60)
        await add_by_code(A, code)
        await A.p.until("document.getElementById('sheet').hidden || !!document.getElementById('sheetWhy').textContent", "the add", 120)
        toast = await A.p.ev("document.getElementById('toast').textContent")
        why = await A.p.ev("document.getElementById('sheetWhy').textContent")
        adds = await A.p.ev("window.__adds")
        # The room left out is said at the top of the feed: "<room>: needs another code".
        await A.p.until("document.body.innerText.includes('needs another code')", "the room named", 20)
        said = [l for l in (await A.p.ev("document.body.innerText")).splitlines() if "needs another code" in l]
        left = next((n for n in ROOMS if any(l.strip().startswith(n + ":") for l in said)), None)
        # The code walks the rooms as A's own list shows them (roomsOf): the one left out is the last shown.
        await A.p.ev("[...document.querySelectorAll('#tabs .tab')].find(b => b.textContent.trim() === 'Rooms').click()")
        await A.p.until(f"document.querySelectorAll('#lbody .it .t').length >= {len(ROOMS)}", "A's room list", 10)
        shown = await A.p.ev("[...document.querySelectorAll('#lbody .it .t')].map(e => e.textContent)")
        await A.p.shot(s.work, "K3-added")
        result("K3", wa == 24 and len(adds) == 6 and all(200 <= x["status"] < 300 for x in adds)
               and toast == f"Added to {NAME}, not to 1 room" and not why and left is not None
               and sorted(shown) == sorted(ROOMS) and shown[-1] == left,
               "A adds B by the code: the Site and five rooms, one package each; the sixth, the last A's list shows, named",
               adds=[x["status"] for x in adds], toast=toast, why=why, named=said, list_order=shown)

        want = [site, *(r for n, r in zip(ROOMS, rooms) if n != left)]
        out_room = rooms[ROOMS.index(left)] if left else None
        got = await held(B, want)
        extra = out_room in {o.get("id") for o in (await B.graph()).get("objects", [])}
        await B.p.until("document.body.innerText.includes(" + json.dumps(NAME) + ")", "the Site on B's page", 30)
        await B.p.shot(s.work, "K4-B")
        result("K4", got == set(want) and not extra, "B, signed in throughout, holds the Site and the five rooms, not the one named; "
               "B's page shows the Site", held=f"{len(got)} of {len(want)}", named_room_held=extra, left=left)

        # Signed out from a page of the Door's that does not move: the webapp leaves for the window mid-call.
        await B.p.go(D + "/v2/icd")
        out = await B.post("/v2/signout", {})
        gone = (await B.me()).get("status")
        await B.p.go(D + "/signin")
        await B.p.until("!!document.getElementById('in') && !document.getElementById('in').disabled", "the window", 30)
        await B.p.click("in")
        await B.p.until(f"location.origin === {json.dumps(D)} && location.pathname === '/' && !!document.getElementById('meBtn')", "B back in", 90)
        got2 = await held(B, want)
        me = await B.me()
        result("K5", out["status"] == 200 and gone == 401 and got2 == set(want) and me.get("noncompliant") == [],
               "B signs out and in again: still holds the Site and the five rooms", signout=out["status"], me_after=gone,
               held=f"{len(got2)} of {len(want)}", noncompliant=me.get("noncompliant"))

        n = len(await A.p.ev("window.__adds"))
        await add_by_code(A, code)
        await asyncio.sleep(2)
        why2 = await A.p.ev("document.getElementById('sheetWhy').textContent")
        n2 = len(await A.p.ev("window.__adds"))
        await A.p.shot(s.work, "K6-again")
        result("K6", why2 == "Used already. Ask them for a new one." and n2 == n, "the same code again in A's browser: refused there, nothing sent",
               why=why2, adds_sent=n2 - n)

        await B.p.until("document.body.innerText.includes(" + json.dumps(NAME) + ")", "B's Site shown", 30)
        await B.p.click("siteBtn")
        await B.p.until("[...document.querySelectorAll('#siteMenu .mi')].some(e => e.textContent.includes(" + json.dumps(NAME) + "))", "B's Sites", 10)
        await B.p.ev("[...document.querySelectorAll('#siteMenu .mi')].find(e => e.textContent.includes(" + json.dumps(NAME) + ")).click()")
        await B.p.until("!document.getElementById('siteRole').hidden", "B in the Site", 30)
        await B.p.click("siteRole")
        await B.p.until("!document.getElementById('manage').hidden", "B's Site settings", 10)
        opts = await B.p.ev(OPT)
        await B.p.shot(s.work, "K7-B-settings")
        r7 = await B.post("/v2/add", {"object": site, "bundle": c_code[0]}) if c_code else {"status": None, "body": "no code for C"}
        await asyncio.sleep(3)
        c_holds = site in {o.get("id") for o in c2.get("/v2/graph").json().get("objects", [])}
        result("K7", "Add by contact code" not in opts and r7["status"] is not None and 400 <= r7["status"] < 500 and bool(r7["body"]) and not c_holds,
               "B, a member not the owner: no Add by contact code, and B's add of C's package refused; C holds nothing",
               options=opts, add=r7["status"], said=r7["body"][:200], c_holds_site=c_holds)
    finally:
        for b in opened:
            b.close()
        say("every Chrome closed")


async def run_expiry(s: w.Stack, results: list) -> None:
    """K8, under DOOR_CODE_SECS (the code-with-life build): B's code used by A past its life."""
    result, life = resulter(results), int(os.environ["DOOR_CODE_SECS"])
    opened: list[J.Browser] = []
    try:
        B = await headless(s.work, "B")
        opened.append(B)
        code, _ = await copy_code(B, s, "Bo " + TAG)
        at = time.monotonic()
        pk = w.key((await B.me()).get("pk", ""))
        A = await headless(s.work, "A")
        opened.append(A)
        await A.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=TRAP)
        site, _ = await register(A, s)
        await asyncio.sleep(max(0.0, 2 * life - (time.monotonic() - at)))
        aged = round(time.monotonic() - at, 1)
        await add_by_code(A, code)
        await A.p.until("document.getElementById('sheet').hidden || !!document.getElementById('sheetWhy').textContent", "the add", 60)
        why = await A.p.ev("document.getElementById('sheetWhy').textContent")
        adds = await A.p.ev("window.__adds")
        await A.p.shot(s.work, "K8-expired")
        await asyncio.sleep(3)
        g = await A.graph()
        members = next((o.get("members") or [] for o in g.get("objects", []) if o.get("id") == site), [])
        gained = pk in {w.key(m) for m in members}
        held_b = site in {o.get("id") for o in (await B.graph()).get("objects", [])}
        result("K8", len(adds) == 1 and adds[0]["status"] == 410 and "the contact code has expired" in adds[0]["body"]
               and bool(why) and "mls:" not in why and len(members) >= 1 and not gained and not held_b,
               f"a code used {aged} s after it was made (life {life} s): refused, and the roster gains no one",
               add=adds[:1], why=why[:200], members=len(members), roster_gained_b=gained, b_holds_site=held_b)
    finally:
        for b in opened:
            b.close()
        say("every Chrome closed")


def main() -> int:
    expiry = int(os.environ.get("DOOR_CODE_SECS", "1800")) < 60
    if w.DEPLOYED:
        # A deployed Door (door-test): pinned as browser_path does it; K8 needs the Door's own DOOR_CODE_SECS, not this one's
        if not bp.DOOR_IP or expiry:
            print(__doc__)
            return 2
        bp.map_name()
    s = w.Stack(host="localhost")
    results: list = []
    try:
        if not w.DEPLOYED:
            s.build()
        s.up()
        asyncio.run(asyncio.wait_for((run_expiry if expiry else run)(s, results), 900))
    except Exception as e:                            # any step that could not go on is an NG, named
        results.append(("--", "NG", f"{type(e).__name__}: {e}"))
    finally:
        kept = s.work
        if not w.DEPLOYED:
            s.down()
    for sid, st, text in results:
        print(f"  {sid:<3} {st:<3}  {text}")
    if os.environ.get("E2E_KEEP"):
        print(f"screenshots: {kept}")
    bad, want = [sid for sid, st, _ in results if st == "NG"], 1 if expiry else 7
    print("contact code: " + ("G" if not bad and len(results) == want else f"NG at {', '.join(bad) or 'a step not reached'}"))
    return 0 if not bad and len(results) == want else 1


if __name__ == "__main__":
    sys.exit(main())
