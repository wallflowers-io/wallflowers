"""The Egregore journeys 1 and 2, rehearsed in the real window against a deployed Door
(pdr/e2e-journeys.md, P7). door-test only, with its rehearsal claim key, never the kiosk's.

An owner Registers a Site in the window, mints a room, and runs the one-sign-in snippet twice (the
second must say "already" and add or publish nothing); signs out. Then, per journey, a visitor
comes by a kiosk-format claim for that Site, signs up in the window, and lands joined, holding the
Site and its room. Journey 2's claim carries a share and a choice.

The snippet reads what a run makes from `window.__J`: SITE, SLUG, BUNDLES (the Site's, the Host's,
the room's, fetched from door-test's Arc just before: six, three a run), KID and KEY, and MARK (a
data URL: headless cannot answer the file picker, so the picker is not rehearsed). Its console
lines are kept. The Register draft carries production's face defect (a picture described as "data
URL, …"), so that the Face read afterwards shows whether the snippet cleared it. Headless Chromes pinned to the Door (browser_path.pinned), each its own profile and a
virtual PRF authenticator, killed at the end.

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<door-test's edge root>
  E2E_AUTH=http://127.0.0.1:18021 JOURNEY_SNIPPET=<the snippet> JOURNEY_BUNDLES=<6 bundles, a line each>
  JOURNEY_MARK=<a data URL's file>
  .venv/bin/python app/e2e/journeys.py
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.parse
from pathlib import Path

import httpx
import websockets

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import wallflowers_path as w  # noqa: E402

SNIPPET = Path(os.environ.get("JOURNEY_SNIPPET", ""))
BUNDLES = [b for b in Path(os.environ.get("JOURNEY_BUNDLES", "/dev/null")).read_text().splitlines() if b.strip()]
TAG = time.strftime("%m%d%H%M", time.gmtime())
NAME, SLUG = f"Journeys {TAG}", f"journeys-{TAG}"
SHARE = "rehearsa2share3xyzab"
MARK = Path(os.environ.get("JOURNEY_MARK", "/dev/null")).read_text().strip()
# Run 2 must say "already" for these; the room's admitter it cannot see (a forum's view carries
# no roles), so re-applying it is named, not failed.
ALREADY = ["the Arc added to the Site", "the Arc the Site's admitter", "the Arc added to room", "the Arc added to the Host", "published at"]
# JOURNEY_ROOMS=choice: the rooms by choice (Ralph, 28 Sep, P1): one room per claim choice, and
# Healing Resistance, which the Arc also admits to, and which no claim's visitor may join.
BY_CHOICE = os.environ.get("JOURNEY_ROOMS", "") == "choice"
ROOMS = ({"resources": "Resources", "skills": "Skills, Time & Services", "financial": "Financial Support", None: "Healing Resistance"}
         if BY_CHOICE else {None: "Journeys room"})


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def b64u(s: str) -> str:
    return base64.urlsafe_b64encode(s.encode()).rstrip(b"=").decode()


def claim(site: str, choice: str, share: str | None = None) -> str:
    spec = {"site": site, "choice": choice, "ttlSeconds": 600, "now": int(time.time() * 1000)}
    if share:
        spec["share"] = share
    js = ("import(process.argv[1]).then(m => process.stdout.write(m.issueClaim(m.claimKey(process.argv[2]), "
          "JSON.parse(process.argv[3]))))")
    r = subprocess.run(["node", "-e", js, w.CLAIM_MJS.as_uri(), w.KIOSK_SEED, json.dumps(spec)], capture_output=True, text=True)
    if r.returncode:
        raise w.Fail(f"claim.mjs: {r.stderr[-300:]}")
    return r.stdout


class Browser:
    """One headless Chrome, pinned to the Door, its own profile and a virtual PRF passkey."""

    async def open(self, work: Path, tag: str) -> "Browser":
        self.prof = work / f"chrome-{tag}"
        self.proc = subprocess.Popen([bp.chrome(), *bp.pinned(), "--remote-debugging-port=0", f"--user-data-dir={self.prof}",
                                      "--no-first-run", "--no-default-browser-check", "--window-size=430,932", "about:blank"],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(600):
            try:
                port = int((self.prof / "DevToolsActivePort").read_text().split()[0])
                t = next(t for t in httpx.get(f"http://127.0.0.1:{port}/json/list", timeout=1).json() if t["type"] == "page")
                break
            except Exception:
                await asyncio.sleep(0.2)
        else:
            raise w.Fail("Chrome for Testing did not open its DevTools port")
        self.p = bp.Page(await websockets.connect(t["webSocketDebuggerUrl"], max_size=None))
        for m in ("Page.enable", "Runtime.enable", "Inspector.enable"):
            await self.p.cdp(m)
        await self.p.cdp("Emulation.setFocusEmulationEnabled", enabled=True)
        await self.p.cdp("WebAuthn.enable", enableUI=False)
        await self.p.cdp("WebAuthn.addVirtualAuthenticator", options={
            "protocol": "ctap2", "ctap2Version": "ctap2_1", "transport": "internal", "hasResidentKey": True,
            "hasUserVerification": True, "isUserVerified": True, "automaticPresenceSimulation": True, "hasPrf": True})
        return self

    def close(self) -> None:
        self.proc.kill()
        self.proc.wait(timeout=10)

    async def sign_up(self) -> int:
        """From Create an account to the words, and Continue: the number of words shown. One tap
        (2f3285ae): the passkey is asked at once; where it was not, its own step's button."""
        p = self.p
        await p.click("new")
        await p.until("!document.getElementById('shown').hidden || !document.getElementById('words').hidden",
                      "the words, or the passkey step", 120)
        if await p.ev("document.getElementById('shown').hidden"):
            await p.click("saved")
            await p.until("!document.getElementById('shown').hidden", "the recovery words", 120)
        n = await p.ev("document.querySelectorAll('#list li').length")
        await p.click("done")
        return n

    async def me(self) -> dict:
        return await self.p.ev("fetch('/v2/me').then(r => r.status === 200 ? r.json() : {status: r.status})")

    async def graph(self) -> dict:
        return await self.p.ev("fetch('/v2/graph').then(r => r.json())")

    async def post(self, path: str, body: dict) -> dict:
        return await self.p.ev(f"fetch({json.dumps(path)}, {{method: 'POST', headers: {{'content-type': 'application/json'}}, "
                               f"body: {json.dumps(json.dumps(body))}}}).then(async r => ({{status: r.status, body: await r.text()}}))")

    async def snippet(self, j: dict) -> list[str]:
        """The snippet in this page, its console lines kept; a throw is kept as a line too."""
        await self.p.ev("window.__log = []; window.__J = " + json.dumps(j) + "; for (const k of ['log', 'warn', 'error']) {"
                        " const o = console[k].bind(console); console[k] = (...a) => { __log.push(k + ': ' + a.map(x => typeof x === 'string' ? x : JSON.stringify(x)).join(' ')); o(...a); }; } true")
        await self.p.ev("(async () => {\n" + SNIPPET.read_text() + "\n})().catch(e => __log.push('threw: ' + (e && e.message || e))).then(() => true)")
        return await self.p.ev("__log")


async def run(s: w.Stack, results: list) -> None:
    D, owner, j = s.door_url, None, {}
    opened: list[Browser] = []

    def result(sid, ok, what, **detail):
        results.append((sid, "PASS" if ok else "FAIL", what + ": " + json.dumps(detail, ensure_ascii=False)[:1600]))
        say(sid, "G" if ok else "R", json.dumps(detail, ensure_ascii=False)[:1600])
        return ok

    try:
        owner = await Browser().open(s.work, "owner")
        opened.append(owner)
        draft = {"kind": "Community", "name": NAME, "purpose": "Journeys rehearsal on door-test", "slug": SLUG, "pname": NAME,
                 "face": {"mark": "data URL, 46303 chars", "header": {"logo": "data URL, 1200 chars", "banner": ""}}}
        await owner.p.go(D + "/signin?new&return=" + urllib.parse.quote("/#register=" + b64u(json.dumps(draft, separators=(",", ":"))), safe=""))
        words = await owner.sign_up()
        await owner.p.until(f"location.origin === {json.dumps(D)} && !document.getElementById('sheet').hidden", "the Register sheet", 90)
        await owner.p.click("sheetGo")
        await owner.p.until("document.getElementById('sheet').hidden && location.hash === ''", "the Site to open", 120)
        g = await owner.graph()
        site = next((o["id"] for o in g.get("objects", []) if o.get("kind") == "group" and o.get("name") == NAME), None)
        host = next((o["id"] for o in g.get("objects", []) if o.get("kind") == "host"), None)
        rooms, made = {}, []
        for choice, name in ROOMS.items():
            at = int(time.time() * 1000)
            m = await owner.post("/v2/mint", {"kind": "forum", "draft": {"name": name}})
            rid = json.loads(m["body"]).get("object_id") if m["status"] == 200 else None
            a = await owner.post("/v2/apply", {"object": site, "op": "base.setPart", "args": {"part": rid, "role": "room", "at": at}})
            b = await owner.post("/v2/apply", {"object": rid, "op": "base.setParent", "args": {"parent": site, "role": "room", "at": at}})
            rooms[choice] = rid
            made.append([name, (rid or "")[:16], a["status"], b["status"]])
        room = rooms.get("skills") or rooms[None]
        await owner.p.shot(s.work, "J-owner-site")
        if not result("J0", bool(site and host and all(rooms.values())) and words == 24 and all(x[2] == x[3] == 200 for x in made),
                      "the owner Registers a Site in the window and mints its rooms",
                      site=(site or "")[:16], host=(host or "")[:16], rooms=made, words=words):
            return
        kid, key = next(iter(json.loads(w.CLAIM_KEYS.read_text()).items()))
        per = 2 + len(ROOMS)
        j = {"SITE": site, "SLUG": SLUG, "HOST": host, "ROOM": room, "BUNDLES": BUNDLES[:per], "KID": kid, "KEY": key, "MARK": MARK}
        first = await owner.snippet(j)
        bad = [x for x in first if x.startswith(("error:", "threw:")) or "✗" in x]
        result("J0s", not bad and len(BUNDLES) == 2 * (2 + len(ROOMS)) and bool(MARK), "the one-sign-in snippet, first run",
               lines=[x[:110] for x in first], bad=bad)
        await owner.p.go(D + "/")
        await owner.p.until("fetch('/v2/me').then(r => r.status === 200)", "the owner's page again", 30)
        second = await owner.snippet({**j, "BUNDLES": BUNDLES[per:2 * per]})
        missing = [a for a in ALREADY if not any(x.startswith("log: already") and a in x for x in second)]
        bad2 = [x for x in second if x.startswith(("error:", "threw:")) or "✗" in x]
        again = [x for x in second if "room" in x and "admitter" in x and not x.startswith("log: already")]
        result("J0a", not bad2 and not missing, "the snippet again, a fresh page and fresh bundles: \"already\" for every add, "
               "the Site's admitter and the publish", lines=[x[:110] for x in second], missing_already=missing, bad=bad2,
               named_not_failed={"the room's admitter re-applied": again})
        out = await owner.post("/v2/signout", {})
        await owner.p.go("about:blank")
        result("J0o", out["status"] == 200, "the owner signs out: the Site sealed", signout=out["status"])

        visits = ((1, ("resources", None)), (2, ("skills", None)), (3, ("financial", SHARE))) if BY_CHOICE else \
            ((1, ("skills", None)), (2, ("financial", SHARE)))
        for n, (choice, share) in visits:
            v = await Browser().open(s.work, f"visitor{n}")
            opened.append(v)
            c = claim(site, choice, share)
            await v.p.go(D + "/join?claim=" + urllib.parse.quote(c, safe=""))
            landed = await v.p.ev("location.pathname + location.hash")
            await v.p.go(D + "/signin?new&return=" + urllib.parse.quote("/#join", safe=""))
            vw = await v.sign_up()
            await v.p.until("location.hash.startsWith('#joined') || (document.getElementById('refused') && "
                            "!document.getElementById('refused').hidden)", "#joined, or a refusal", 90)
            hashed = await v.p.ev("location.hash")
            refused = await v.p.ev("(document.getElementById('refused') || {}).textContent || ''")
            await asyncio.sleep(2)
            me, g = await v.me(), await v.graph()
            ids = {o.get("id") for o in g.get("objects", [])}
            await v.p.shot(s.work, f"J{n}-joined")
            own = rooms.get(choice) if BY_CHOICE else room
            others = {ROOMS[c]: r in ids for c, r in rooms.items() if r != own}
            result(f"J{n}", hashed.startswith("#joined") and site in ids and own in ids and not any(others.values())
                   and me.get("noncompliant") == [] and vw == 24,
                   f"journey {n}: the claim ({choice}{', a share' if share else ''}), /join, sign-up in the window, "
                   "admitted by the Arc, the Site and its own room held" + (", no other room" if BY_CHOICE else ""),
                   join_landed=landed, hash=hashed, refused=refused[:200], holds_site=site in ids, holds_own_room=own in ids,
                   holds_other_rooms=others, noncompliant=me.get("noncompliant"), words=vw)
            await v.post("/v2/signout", {})
    finally:
        for b in opened:
            b.close()
        say("every Chrome closed")
        results.append(("--", "INFO", "run: " + json.dumps({"site": j.get("SITE"), "slug": SLUG, "host": j.get("HOST"), "room": j.get("ROOM")})))


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP and SNIPPET.is_file()):
        print("journeys.py: a deployed Door only: set E2E_DOOR, E2E_DOOR_IP, E2E_DOOR_CA, E2E_AUTH and JOURNEY_SNIPPET")
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
        print(f"  {sid:<4} {st:<6}  {text}")
    print(f"screenshots: {kept}")
    bad = [t for sid, st, t in results if st == "FAIL"]
    print("journeys: " + ("G" if not bad else f"R; first: {bad[0][:300]}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
