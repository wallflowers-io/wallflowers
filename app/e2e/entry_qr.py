"""The entry QR end to end on door-test (pdr/e2e-journeys.md P1; Software Assurance's A11), at the
build to be deployed, with the rehearsal claim key only, never the kiosk's.

A Site as production's is: an owner Registers it in the window, its room Healing Resistance, then
Ralph's entry snippet (one-signin-today.js, door-test's rehearsal of it: SITE, SLUG, ROOMS,
BUNDLES, KID, KEY and MARK from window.__J) with ENTRY_ROOMS (Ralph, 29 Sep: 'three'). Then the
kiosk (egregore egg/kiosk at ENTRY_KIOSK, built; KIOSK_DEMO, its stand-in ledger) on loopback,
signing for that Site, and in its own window each answer: RESOURCES, SKILLS / TIME / SERVICES, and
FINANCIAL SUPPORT by the paid route (a demo code that pays itself). Each QR the page draws is read
from the <img> (decoded by the page's BarcodeDetector, where Chrome has one, and matched to the
/api/claim answer the page drew it from), and a visitor follows its URL in the real window, signs
up and must land #joined: the Site held, with 'three' the three rooms, never Healing Resistance,
nothing noncompliant; the paid visitor's hash carrying its share. A11 at layer 1: a good claim
admits (the Site, the rooms); the same claim again, an expired one, one tampered (its choice
changed under the old signature), and one for a Site that never named this key, each refused.

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<door-test's edge root>
  E2E_AUTH=http://127.0.0.1:18021 ENTRY_KIOSK=<egregore checkout> ENTRY_SNIPPET=<the snippet>
  ENTRY_BUNDLES=<6 bundles from door-test's Arc, a line each> JOURNEY_MARK=<a data URL's file>
  [ENTRY_ROOMS=three|none] .venv/bin/python app/e2e/entry_qr.py
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import subprocess
import sys
import time
import urllib.parse
from pathlib import Path

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import journeys as J  # noqa: E402
import wallflowers_path as w  # noqa: E402

KIOSK = Path(os.environ.get("ENTRY_KIOSK", ""))
SNIPPET = Path(os.environ.get("ENTRY_SNIPPET", ""))
ROOMS = os.environ.get("ENTRY_ROOMS", "three")
BUNDLES = [b for b in Path(os.environ.get("ENTRY_BUNDLES", "/dev/null")).read_text().splitlines() if b.strip()]
THREE = ["Resources", "Skills, Time & Services", "Financial Support"]
HR = "Healing Resistance"
TAG = time.strftime("%m%d%H%M", time.gmtime())
NAME, SLUG = f"Entry {TAG}", f"entry-{TAG}"
# The kiosk's claims are its own (egg/kiosk/claim.mjs at ENTRY_KIOSK), A11's too.
w.CLAIM_MJS = KIOSK / "egg" / "kiosk" / "claim.mjs"
say = J.say

# Every /api/claim answer the kiosk page is given, kept before its app loads.
CAPTURE = """(() => { const f = window.fetch; window.__claims = [];
  window.fetch = async (...a) => { const r = await f(...a); const u = String((a[0] && a[0].url) || a[0]);
    if (u.includes('/api/claim?')) r.clone().json().then(b => __claims.push({u, status: r.status, body: b})).catch(() => {});
    return r; }; })();"""
# The deepest element whose text, or label, is this one's, its case and its apostrophes aside.
CLICK = """((t) => { const n = s => (s || '').trim().replace(/[\u2018\u2019]/g, "'").toLowerCase(), all = [...document.querySelectorAll('body *')];
  const el = all.find(e => n(e.getAttribute('aria-label')) === n(t)) || all.filter(e => n(e.textContent) === n(t)).pop();
  if (!el) return false; el.scrollIntoView({block: 'center'}); el.click(); return true; })"""
QR_SRC = "(() => { const i = document.querySelector('.take img'); return i && i.complete && i.src.startsWith('data:image/svg') ? i.src : ''; })()"
DECODE = """(async () => { if (typeof BarcodeDetector === 'undefined') return null;
  const i = document.querySelector('.take img'), c = document.createElement('canvas'); c.width = c.height = 800;
  const x = c.getContext('2d'); x.fillStyle = '#fff'; x.fillRect(0, 0, 800, 800); x.drawImage(i, 40, 40, 720, 720);
  return (await new BarcodeDetector({formats: ['qr_code']}).detect(c)).map(r => r.rawValue); })()"""


async def click_text(p: bp.Page, text: str, secs: float = 30) -> None:
    await p.until(f"{CLICK}({json.dumps(text)})", f'"{text}"', secs)


async def read_qr(k: J.Browser, work: Path, tag: str) -> dict:
    """The QR the kiosk page draws: its URL by the answer it was drawn from, and by decoding it."""
    await k.p.until(QR_SRC, f"the {tag} QR", 90)
    await k.p.shot(work, f"K-{tag}")
    src = await k.p.ev(QR_SRC)
    got = [c for c in (await k.p.ev("window.__claims") or []) if c.get("status") == 200 and (c.get("body") or {}).get("qr") == src]
    url = got[-1]["body"]["url"] if got else ""
    decoded = await k.p.ev(DECODE)
    return {"url": url, "decoded": decoded, "asks": len(await k.p.ev("window.__claims") or [])}


async def visit(s: w.Stack, work: Path, tag: str, url: str, site: str, rooms: dict, share: bool) -> tuple[bool, dict]:
    """A visitor follows the QR's URL in the real window, signs up, lands."""
    v = await J.Browser().open(work, f"visitor-{tag}")
    try:
        await v.p.go(url)
        landed = await v.p.ev("location.pathname + location.hash")
        await v.p.go(s.door_url + "/signin?new&return=" + urllib.parse.quote("/#join", safe=""))
        words = await v.sign_up()
        await v.p.until("location.hash.startsWith('#joined') || (document.getElementById('refused') && "
                        "!document.getElementById('refused').hidden)", "#joined, or a refusal", 90)
        hashed = await v.p.ev("location.hash")
        refused = await v.p.ev("(document.getElementById('refused') || {}).textContent || ''")
        await asyncio.sleep(2)
        me, g = await v.me(), await v.graph()
        ids = {o.get("id") for o in g.get("objects", [])}
        await v.p.shot(work, f"V-{tag}")
        await v.post("/v2/signout", {})
    finally:
        v.close()
    a = urllib.parse.parse_qs(hashed.lstrip("#").replace("joined&", "", 1)).get("a", [""])[0]
    held = {name: rid in ids for name, rid in rooms.items()}
    want = {name: ROOMS == "three" and name != HR for name in rooms}
    ok = (hashed.startswith("#joined") and site in ids and held == want and me.get("noncompliant") == [] and words == 24
          and (a.startswith("zzzzzz") if share else not a))
    return ok, {"landed": landed, "hash": hashed, "refused": refused[:160], "holds_site": site in ids, "rooms_held": held,
                "noncompliant": me.get("noncompliant"), "share": a or None}


def join(v: httpx.Client) -> httpx.Response:
    r, t0 = v.post("/v2/join"), time.monotonic()
    while r.status_code == 503 and time.monotonic() - t0 < 30:
        time.sleep(2)
        r = v.post("/v2/join")
    return r


def holds(v: httpx.Client) -> set:
    return {o.get("id") for o in v.get("/v2/graph").json().get("objects", [])}


def a11(s: w.Stack, site: str, rooms: dict, wrong: str) -> list:
    """A11 at layer 1, each case its own fresh account."""
    out = []
    good = w.claim(site, choice="resources")
    v = w.webapp(s)
    j = v.get("/join", params={"claim": good}, follow_redirects=False).status_code
    w.signup(s, f"entry a11 good {TAG}", v)
    r = join(v)
    time.sleep(3)
    ids = holds(v)
    held = {name: rid in ids for name, rid in rooms.items()}
    want = {name: ROOMS == "three" and name != HR for name in rooms}
    out.append(("A11a", j == 303 and r.status_code == 200 and site in ids and held == want and w.me(v).get("noncompliant") == [],
                "a good claim admits: the Site" + (" and the three rooms" if ROOMS == "three" else ""),
                {"join": j, "v2_join": r.status_code, "holds_site": site in ids, "rooms_held": held}))
    v.post("/v2/signout")
    again = w.webapp(s)
    j = again.get("/join", params={"claim": good}, follow_redirects=False).status_code
    w.signup(s, f"entry a11 reused {TAG}", again)
    r = join(again)
    time.sleep(3)
    out.append(("A11b", r.status_code == 409 and site not in holds(again), "the same claim, a second account: refused",
                {"join": j, "v2_join": [r.status_code, r.text[:120]], "holds_site": site in holds(again)}))
    again.post("/v2/signout")
    r = w.webapp(s).get("/join", params={"claim": w.claim(site, ago=7200, choice="resources")}, follow_redirects=False)
    out.append(("A11c", r.status_code == 400 and not r.headers.get("set-cookie"), "an expired claim: refused at /join",
                {"join": [r.status_code, r.text[:120]]}))
    head, payload, sig = w.claim(site, choice="resources").split(".")
    body = json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))
    body["c"] = "financial"
    forged = f"{head}.{base64.urlsafe_b64encode(json.dumps(body, separators=(',', ':')).encode()).rstrip(b'=').decode()}.{sig}"
    r = w.webapp(s).get("/join", params={"claim": forged}, follow_redirects=False)
    out.append(("A11d", r.status_code == 400 and not r.headers.get("set-cookie"),
                "a tampered claim (its choice changed under the old signature): refused at /join", {"join": [r.status_code, r.text[:120]]}))
    for sid, other, what in (("A11e", wrong, "a Site that never named this key, the Arc its admitter"),
                             ("A11f", os.urandom(32).hex(), "a Site that does not exist")):
        v = w.webapp(s)
        j = v.get("/join", params={"claim": w.claim(other, choice="resources")}, follow_redirects=False)
        r = None
        if j.status_code == 303:
            w.signup(s, f"entry a11 {sid} {TAG}", v)
            r = join(v)
            time.sleep(3)
        ids = holds(v) if r is not None else set()
        out.append((sid, (j.status_code == 400 or (r is not None and r.status_code != 200)) and other not in ids and site not in ids,
                    f"a claim for {what}: refused", {"join": j.status_code, "v2_join": [r.status_code, r.text[:120]] if r is not None else None,
                                                     "holds_it": other in ids, "holds_the_site": site in ids}))
        if r is not None:
            v.post("/v2/signout")
    return out


async def run(s: w.Stack, results: list) -> None:
    D = s.door_url
    opened: list[J.Browser] = []
    procs: list[subprocess.Popen] = []

    def result(sid, ok, what, **detail):
        results.append((sid, "PASS" if ok else "FAIL", what + ": " + json.dumps(detail, ensure_ascii=False)[:1600]))
        say(sid, "G" if ok else "R", json.dumps(detail, ensure_ascii=False)[:1600])
        return ok

    try:
        # The Site, as production's: Registered in the window, Healing Resistance its room, then the snippet.
        owner = await J.Browser().open(s.work, "owner")
        opened.append(owner)
        draft = {"kind": "Community", "name": NAME, "purpose": "Entry QR rehearsal on door-test", "slug": SLUG, "pname": NAME,
                 "face": {"mark": "data URL, 46303 chars", "header": {"logo": "data URL, 1200 chars", "banner": ""}}}
        await owner.p.go(D + "/signin?new&return=" + urllib.parse.quote("/#register=" + J.b64u(json.dumps(draft, separators=(",", ":"))), safe=""))
        await owner.sign_up()
        await owner.p.until(f"location.origin === {json.dumps(D)} && !document.getElementById('sheet').hidden", "the Register sheet", 90)
        await owner.p.click("sheetGo")
        await owner.p.until("document.getElementById('sheet').hidden && location.hash === ''", "the Site to open", 120)
        g = await owner.graph()
        site = next((o["id"] for o in g.get("objects", []) if o.get("kind") == "group" and o.get("name") == NAME), None)
        at = int(time.time() * 1000)
        m = await owner.post("/v2/mint", {"kind": "forum", "draft": {"name": HR}})
        hr = json.loads(m["body"]).get("object_id") if m["status"] == 200 else None
        a = await owner.post("/v2/apply", {"object": site, "op": "base.setPart", "args": {"part": hr, "role": "room", "at": at}})
        b = await owner.post("/v2/apply", {"object": hr, "op": "base.setParent", "args": {"parent": site, "role": "room", "at": at}})
        kid, key = next(iter(json.loads(w.CLAIM_KEYS.read_text()).items()))
        per = 2 + (3 if ROOMS == "three" else 0)
        J.SNIPPET = SNIPPET
        lines = await owner.snippet({"SITE": site, "SLUG": SLUG, "ROOMS": ROOMS, "BUNDLES": BUNDLES[:per], "KID": kid, "KEY": key, "MARK": J.MARK})
        bad = [x for x in lines if x.startswith(("error:", "threw:")) or "✗" in x]
        g = await owner.graph()
        names = {o.get("id"): o.get("name") for o in g.get("objects", [])}
        parts = next(o for o in g["objects"] if o["id"] == site)["view"].get("parts", [])
        rooms = {names.get(p["part"]): p["part"] for p in parts if p.get("role") == "room"}
        # A11's other Site: the Arc its admitter, no claim key named.
        m = await owner.post("/v2/mint", {"kind": "group", "draft": {"name": f"Entry other {TAG}", "shape": "community"}})
        wrong = json.loads(m["body"]).get("object_id") if m["status"] == 200 else None
        add = await owner.post("/v2/add", {"object": wrong, "bundle": BUNDLES[per]})
        node = json.loads(add["body"]).get("member") if add["status"] == 200 else None
        grant = await owner.post("/v2/apply", {"object": wrong, "op": "base.setRole", "args": {"member": node, "role": "admitter"}})
        out = await owner.post("/v2/signout", {})
        owner.close()
        opened.remove(owner)
        want_rooms = {HR, *THREE} if ROOMS == "three" else {HR}
        if not result("Q0", bool(site and hr) and a["status"] == b["status"] == 200 and not bad and set(rooms) == want_rooms
                      and len(BUNDLES) >= per + 1 and grant["status"] == 200 and out["status"] == 200,
                      f"the Site as production's: Registered, {HR} its room, the entry snippet (ROOMS {ROOMS!r}); A11's other Site",
                      site=(site or "")[:16], rooms={n: (r or "")[:16] for n, r in rooms.items()}, snippet=[x[:100] for x in lines],
                      bad=bad, other=(wrong or "")[:16], other_admitter=grant["status"]):
            return

        for sid, ok, what, detail in a11(s, site, rooms, wrong):
            result(sid, ok, what, **detail)

        # The kiosk, signing for that Site with the rehearsal seed, on ports of its own (another session's
        # kiosk may hold the ones COMMUNITY's recipe names), reached as localhost: the pinned Chrome maps
        # every other name, an address included, to nothing.
        KPORT, LPORT, SPORT = w.free_port(), w.free_port(), w.free_port()
        log = open(s.work / "kiosk.log", "w")
        env = {**os.environ, "KIOSK_DEMO": "1", "KIOSK_DEMO_PAYS_AFTER": "6", "KIOSK_PORT": str(KPORT),
               "KIOSK_SITE": f"http://127.0.0.1:{LPORT}", "KIOSK_PAY_BASE": f"http://127.0.0.1:{LPORT}", "KIOSK_CLAIM_KEY": w.KIOSK_SEED,
               "KIOSK_DOOR_URL": D, "KIOSK_CLAIM_SITE": site, "KIOSK_CLOCK_FILE": "", "SHOW_URL": "", "SOUND_PORT": str(SPORT)}
        procs.append(subprocess.Popen(["node", "egg/kiosk/stand-in-ledger.mjs", str(LPORT)], cwd=KIOSK, stdout=log, stderr=subprocess.STDOUT))
        procs.append(subprocess.Popen(["node", "egg/kiosk/dist/kiosk.mjs"], cwd=KIOSK, env=env, stdout=log, stderr=subprocess.STDOUT))
        w.wait_for(f"http://127.0.0.1:{KPORT}/emergence", "the kiosk", 60, proc=procs[1])
        if any(p.poll() is not None for p in procs):
            raise w.Fail(f"the kiosk or its ledger stopped: {(s.work / 'kiosk.log').read_text()[-400:]}")
        kiosk_said = [x for x in (s.work / "kiosk.log").read_text().splitlines() if "claim" in x][:4]
        page = f"http://localhost:{KPORT}/emergence?at=profile&mute=1&bulb=0&timecode=0"

        # The kiosk's window only: its steps follow the voice's clock, which a headless page will not start
        # without a gesture, so its answers would never show.
        pinned = bp.pinned
        bp.pinned = lambda: [*pinned(), "--autoplay-policy=no-user-gesture-required"]
        try:
            k = await J.Browser().open(s.work, "kiosk")
        finally:
            bp.pinned = pinned
        opened.append(k)
        await k.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=CAPTURE)
        for n, (answer, tag) in enumerate((("RESOURCES", "resources"), ("SKILLS / TIME / SERVICES", "skills"))):
            if n:
                await asyncio.sleep(21)   # the free answers' bound: one claim per 20 s across the kiosk
            await k.p.go(page)
            await click_text(k.p, answer, 60)
            q = await read_qr(k, s.work, tag)
            decoded_ok = q["decoded"] is None or q["decoded"] == [q["url"]]
            ok, d = await visit(s, s.work, tag, q["url"], site, rooms, share=False) if q["url"] else (False, {})
            result(f"Q{n + 1}", bool(q["url"]) and q["url"].startswith(D + "/join?claim=v1.") and decoded_ok and ok,
                   f"{answer}: the kiosk's QR, a visitor by it in the window, admitted",
                   qr={"url": q["url"][:60], "decoded": "no BarcodeDetector" if q["decoded"] is None else q["decoded"] == [q["url"]],
                       "asks": q["asks"]}, kiosk=kiosk_said if n == 0 else None, **d)

        # FINANCIAL SUPPORT: the paid route (egg/emergence/graph.ts: to-pay, where, pay, paid-artifact,
        # sign-after-pay, received, artifact); the QR only once the demo code has paid and been numbered.
        await k.p.go(page)
        steps = []
        route = ("FINANCIAL SUPPORT", "I'M READY TO GIVE", "₩20,000", "GIVE ₩20,000", "Let's join our hands", "DOWNLOAD & JOIN THE COMMUNITY")
        for text in route:
            try:
                await click_text(k.p, text, 90)
                steps.append(text)
            except w.Fail as e:
                await k.p.shot(s.work, "K-financial-stuck")
                steps.append(f"{text}: {str(e)[:120]}")
                break
            await asyncio.sleep(1)
        q = await read_qr(k, s.work, "financial") if len(steps) == len(route) and ":" not in steps[-1] else {"url": "", "decoded": None, "asks": 0}
        decoded_ok = q["decoded"] is None or q["decoded"] == [q["url"]]
        ok, d = await visit(s, s.work, "financial", q["url"], site, rooms, share=True) if q["url"] else (False, {})
        result("Q3", bool(q["url"]) and decoded_ok and ok, "FINANCIAL SUPPORT, paid: the kiosk's QR after the payment, a visitor, its share",
               steps=steps, qr={"url": q["url"][:60], "decoded": "no BarcodeDetector" if q["decoded"] is None else q["decoded"] == [q["url"]],
                                "asks": q["asks"]}, **d)
        k.close()
        opened.remove(k)

    finally:
        for b in opened:
            b.close()
        for p in procs:
            p.terminate()
            p.wait(timeout=10)
        say("every Chrome closed; the kiosk and its ledger stopped")


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP and SNIPPET.is_file() and (KIOSK / "egg" / "kiosk" / "dist" / "kiosk.mjs").is_file()):
        print(__doc__)
        return 2
    bp.map_name()
    s = w.Stack(host="localhost")
    results: list = []
    try:
        s.up()
        asyncio.run(run(s, results))
    except (w.Fail, TimeoutError, OSError) as e:
        results.append(("--", "FAIL", f"{str(e) or type(e).__name__}"))
    finally:
        kept = s.work
        s.down()
    for sid, st, text in results:
        print(f"  {sid:<5} {st:<6}  {text}")
    print(f"screenshots and the kiosk's log: {kept}")
    bad = [t for sid, st, t in results if st == "FAIL"]
    print("entry_qr: " + ("G" if not bad else f"R; first: {bad[0][:300]}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
