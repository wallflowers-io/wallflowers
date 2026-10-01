"""A late joiner's view of the Site, owner online and owner offline (door-test only; BUILD, 28 Sep).

A member admitted by claim gets nothing from before its epoch: the Site's parts (its Host, its
rooms) and a room's parent reach it only when the owner's live session restates the owner's
sequenced spine (restate_for_joiners, node.rs). Measured here, at layer 1, from the /v2/join
answer: until the member's own /v2/graph shows the Site's parts and its room's view.parent, and
until a site token (the Door's authorize, PKCE, DPoP) reaches its room. First with the owner
signed in and live, then with the owner signed out, over RESTATE_WAIT seconds.

  setup    an owner, a Site (Register's /v2/batch: the Site, its Host, the edge both ways),
           Healing Resistance and the three rooms, the Arc added and admitter on the Site and
           the three, the rehearsal claim key; signs out. RESTATE_STATE (scratch, 0600) keeps
           the owner's credentials and the ids. Then register RESTATE_CLIENT for the Site.
  measure  owner online, a member by claim (financial, a share); then owner offline, another.

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<door-test's edge root>
  E2E_AUTH=http://127.0.0.1:18021 RESTATE_STATE=<file> RESTATE_BUNDLES=<4 Arc bundles, a line each>
  [RESTATE_ORDER=offline-first: a member before the owner is ever back] [RESTATE_CLIENT=<client id> RESTATE_CALLBACK=<its callback>: the site token too] .venv/bin/python app/e2e/restate_timing.py setup|measure
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import wallflowers_path as w  # noqa: E402

STATE = Path(os.environ.get("RESTATE_STATE", ""))
BUNDLES = [b for b in Path(os.environ.get("RESTATE_BUNDLES", "/dev/null")).read_text().splitlines() if b.strip()]
CLIENT = os.environ.get("RESTATE_CLIENT", "")
CALLBACK = os.environ.get("RESTATE_CALLBACK", "")
WAIT = int(os.environ.get("RESTATE_WAIT", "180"))
THREE = ["Resources", "Skills, Time & Services", "Financial Support"]
TAG = time.strftime("%m%d%H%M", time.gmtime())


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def ok(r: httpx.Response) -> dict:
    if r.status_code != 200:
        raise w.Fail(f"{r.request.method} {r.request.url.path}: {r.status_code} {r.text[:200]}")
    return r.json() if r.content else {}


def apply(c, obj, op, **args):
    return ok(c.post("/v2/apply", json={"object": obj, "op": op, "args": args}))


def claim(site: str, choice: str, share: str | None) -> str:
    spec = {"site": site, "choice": choice, "ttlSeconds": 600, "now": int(time.time() * 1000), **({"share": share} if share else {})}
    js = "import(process.argv[1]).then(m => process.stdout.write(m.issueClaim(m.claimKey(process.argv[2]), JSON.parse(process.argv[3]))))"
    return subprocess.run(["node", "-e", js, w.CLAIM_MJS.as_uri(), w.KIOSK_SEED, json.dumps(spec)], capture_output=True, text=True,
                          check=True).stdout


def setup(s: w.Stack) -> None:
    owner = w.signup(s, f"restate owner {TAG}")
    c = owner["client"]
    at = int(time.time() * 1000)
    made = ok(c.post("/v2/batch", json={"steps": [
        {"do": "mint", "kind": "group", "draft": {"name": f"Restate {TAG}", "shape": "community"}},
        {"do": "mint", "kind": "host", "draft": {"name": f"Restate {TAG}"}},
        {"do": "apply", "object": {"$step": 0}, "op": "base.setPart", "args": {"part": {"$step": 1}, "role": "host", "at": at}},
        {"do": "apply", "object": {"$step": 1}, "op": "base.setParent", "args": {"parent": {"$step": 0}, "role": "host", "at": at}}]}))
    if made.get("refused"):
        raise w.Fail(f"/v2/batch: {made['refused']}")
    site, host = made["made"][0], made["made"][1]
    rooms = {}
    for name in ["Healing Resistance", *THREE]:
        at = int(time.time() * 1000)
        r = ok(c.post("/v2/batch", json={"steps": [
            {"do": "mint", "kind": "forum", "draft": {"name": name}},
            {"do": "apply", "object": site, "op": "base.setPart", "args": {"part": {"$step": 0}, "role": "room", "at": at}},
            {"do": "apply", "object": {"$step": 0}, "op": "base.setParent", "args": {"parent": site, "role": "room", "at": at}}]}))
        rooms[name] = r["made"][0]
    arc = None
    for obj, bundle in zip([site, *[rooms[n] for n in THREE]], BUNDLES):
        arc = ok(c.post("/v2/add", json={"object": obj, "bundle": bundle}))["member"]
        apply(c, obj, "base.setRole", member=arc, role="admitter")
    kid, key = next(iter(json.loads(w.CLAIM_KEYS.read_text()).items()))
    apply(c, site, "group.setClaimIssuer", kid=kid, key=key)
    ok(c.post("/v2/signout"))
    fd = os.open(STATE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump({"site": site, "host": host, "rooms": rooms, "arc": arc,
                   "owner": {"handle": owner["handle"], "pk": owner["pk"], "prf": owner["prf"].hex()}}, f)
    say("setup:", json.dumps({"site": site, "host": host[:16], "rooms": {n: r[:16] for n, r in rooms.items()}}))


def token(s: w.Stack, who: dict):
    """A site token for `who`: a sign-in carrying the client's PKCE, the code, /v2/token under DPoP."""
    verifier, state = w.b64u(os.urandom(32)), os.urandom(8).hex()
    cb = {"client": CLIENT, "redirect_uri": CALLBACK, "state": state, "code_challenge": w.b64u(hashlib.sha256(verifier.encode()).digest())}
    c = w.webapp(s)
    o = c.post("/v2/signin", json=w.work(c, "signin") or None).json()
    r = ok(c.post("/v2/signin/finish", json={"attempt": o["attempt"], "handle": who["handle"],
                                              "sealed": w.seal(o["key"], who["prf"], o["attempt"]), "client": cb}))
    code = parse_qs(urlsplit(r["redirect"]).query)["code"][0]
    k = w.dpop_key()
    t = ok(httpx.post(s.door_url + "/v2/token", verify=s.verify, timeout=30,
                      headers={"dpop": w.dpop(k, "POST", s.door_url + "/v2/token")},
                      json={"code": code, "code_verifier": verifier, "client": CLIENT, "redirect_uri": CALLBACK}))["access_token"]

    def graph() -> dict:
        return ok(httpx.get(s.door_url + "/v2/graph", verify=s.verify, timeout=30,
                            headers={"authorization": f"DPoP {t}", "dpop": w.dpop(k, "GET", s.door_url + "/v2/graph", t)}))
    return graph


def seen(g: dict, st: dict) -> dict:
    by = {o.get("id"): o for o in g.get("objects", [])}
    site = by.get(st["site"]) or {}
    parts = {p.get("part") for p in (site.get("view") or {}).get("parts") or []}
    room = st["rooms"]["Financial Support"]
    return {"parts": {st["host"], *(st["rooms"][n] for n in THREE)} <= parts,
            "parent": ((((by.get(room) or {}).get("view") or {}).get("parent")) or {}).get("parent") == st["site"],
            "room": room in by}


def join_and_watch(s: w.Stack, st: dict, label: str) -> dict:
    v = w.webapp(s)
    if v.get("/join", params={"claim": claim(st["site"], "financial", "zzzzzz" + "".join(__import__("secrets").choice("abcdefghjkmnpqrstuvwxyz23456789") for _ in range(14)))},
             follow_redirects=False).status_code != 303:
        raise w.Fail(f"{label}: /join refused the claim")
    member = w.signup(s, f"restate member {label} {TAG}", v)
    r, t0 = v.post("/v2/join"), time.monotonic()
    while r.status_code == 503 and time.monotonic() - t0 < 30:
        time.sleep(1)
        r = v.post("/v2/join")
    ok(r)
    t0 = time.monotonic()
    graph = token(s, member) if CLIENT else None
    got: dict = {"member": {}, **({"token": {}} if graph else {})}
    while time.monotonic() - t0 < WAIT:
        el = round(time.monotonic() - t0, 1)
        for where, g in (("member", ok(v.get("/v2/graph"))), *((("token", graph()),) if graph else ())):
            for k2, hit in seen(g, st).items():
                if hit and k2 not in got[where]:
                    got[where][k2] = el
        if all(k2 in got[where] for where in got for k2 in ("parts", "parent", "room")):
            break
        time.sleep(1)
    v.post("/v2/signout")
    say(f"measure: {label}", json.dumps({"seconds from /v2/join 200 (absent: not within the wait)": got, "wait": WAIT}))
    return got


def measure(s: w.Stack) -> int:
    st = json.loads(STATE.read_text())
    owner = {"handle": st["owner"]["handle"], "pk": st["owner"]["pk"], "prf": bytes.fromhex(st["owner"]["prf"])}
    if os.environ.get("RESTATE_ORDER") == "offline-first":
        # The owner not signed in since setup: nothing of theirs has run since the Arc was added.
        first = join_and_watch(s, st, "owner offline, never back since setup")
        say("measure:", "offline-first: the member's view", "whole" if all(k in first["member"] for k in ("parts", "parent")) else "NOT whole")
    home = w.signin(s, owner)["client"]
    live = threading.Event()

    def keep() -> None:   # the owner's page, open: a read every 15 s, as the webapp's poll would be
        while not live.wait(15):
            home.get("/v2/me")
    threading.Thread(target=keep, daemon=True).start()
    time.sleep(5)
    online = join_and_watch(s, st, "owner online")
    live.set()
    ok(home.post("/v2/signout"))
    time.sleep(10)
    offline = join_and_watch(s, st, "owner offline")
    good = all(k in online["member"] for k in ("parts", "parent"))
    say("measure:", "online the member's view whole" if good else "online NOT whole", ";",
        "offline empty as expected" if not offline["member"].get("parts") else "offline reached it too")
    return 0


def main() -> int:
    phase = sys.argv[1] if len(sys.argv) > 1 else ""
    if phase not in ("setup", "measure") or not (w.DEPLOYED and bp.DOOR_IP and STATE.name) or bool(CLIENT) != bool(CALLBACK):
        print(__doc__)
        return 2
    bp.map_name()
    s = w.Stack(host="localhost")
    s.up()
    if phase == "setup":
        setup(s)
        return 0
    return measure(s)


if __name__ == "__main__":
    sys.exit(main())
