"""Every route's latency (W-96; Ralph, 30 Sep: "All the routes need to be tested for latency").

THE ROUTES are read at run time from app/door/routes.json, which the Door's routes test holds to its three
routers (the Door's, the person's process's, the Arc gateway's); never listed here. Each is measured by what it is:
  public   GET, no credentials (the Door's pages and scripts, /v2/icd, /join with a claim; the Arc's faces)
  account  sign-up, sign-in, the token, Continue, sign-out: each step timed inside a whole flow, its proof of
           work solved before the clock starts
  dpop     a site's call, as the Community API makes it: a token for egregore-local, scoped to the ja-rehearsal
           Site, held by a member of its Financial Support room (a claim, then the site's sign-in)
  cookie   the webapp's own writes (/v2/add, /v2/batch, /v2/site/address, /v2/join)
WARM: one keep-alive connection and a person-process already open; the first call not counted. COLD: a new
connection (TCP and TLS) each time. Each figure: from the request's start to its first byte (ttfb) and to its
last (the headline). A stream (/v2/events) is timed to its headers. Each route's batch is bracketed by the
guest's uptime against the wall clock; one that lost more than a second (this Mac asleep) is measured again.
A route with no recipe is written with n 0 and says so. Writes go to door-test only.

  LAT_LINK=loopback (this Mac to door-test's Door, direct) | venue (through venue_link's hop, the venue's
  measured distance to the Door; the Arc has none measured, so its routes are loopback only)
  LAT_PROD=1: production's unauthenticated reads only (/v2/icd, /v2/signin.js), from this Mac
  LAT_WARM=30 LAT_COLD=10 (account and cookie writes: a third of each) LAT_OUT=<the JSON's path>
  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  RESTATE_STATE=<the Site's owner file> JA_KIOSK=<egregore checkout> .venv/bin/python app/e2e/route_latency.py
"""
from __future__ import annotations

import hashlib
import json
import math
import os
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import journeys as J  # noqa: E402
import wallflowers_path as w  # noqa: E402

PRODUCT = Path(__file__).resolve().parents[2]
LINK = os.environ.get("LAT_LINK", "loopback")
PROD = os.environ.get("LAT_PROD") == "1"
WARM, COLD = int(os.environ.get("LAT_WARM", "30")), int(os.environ.get("LAT_COLD", "10"))
OUT = Path(os.environ.get("LAT_OUT", ""))
STATE = Path(os.environ.get("RESTATE_STATE", ""))
ARC = os.environ.get("E2E_ARC", "http://127.0.0.1:8090").rstrip("/")
CLIENT, CALLBACK = "egregore-local", "http://127.0.0.1:3471/signin/callback"
SLUG = os.environ.get("LAT_SLUG", "ja-rehearsal")


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def routes() -> dict[tuple[str, str], dict]:
    """Every route in app/door/routes.json. A /v2 route the Door forwards is also the process's: the Door's row is kept."""
    out: dict[tuple[str, str], dict] = {}
    for r in json.loads((PRODUCT / "app" / "door" / "routes.json").read_text()):
        k = (r["method"], r["path"])
        if k not in out or out[k]["audience"] == "internal":
            out[k] = r
    return out


def stats(xs: list[float]) -> dict:
    if not xs:
        return {"n": 0, "p50_ms": None, "p95_ms": None}
    s = sorted(xs)
    return {"n": len(s), "p50_ms": round(s[(len(s) - 1) // 2], 1), "p95_ms": round(s[max(0, math.ceil(0.95 * len(s)) - 1)], 1)}


def uptime() -> tuple[float, float]:
    out = subprocess.run(["limactl", "shell", "door-test", "cat", "/proc/uptime"], capture_output=True, text=True, timeout=30).stdout
    return time.time(), float(out.split()[0])


def lost(a: tuple[float, float], b: tuple[float, float]) -> float:
    return round((b[0] - a[0]) - (b[1] - a[1]), 2)


class Timed:
    """One measured call: its status, ms to first byte, ms to the end."""

    def __init__(self, client: httpx.Client, method: str, url: str, stream: bool = False, **kw) -> None:
        t0 = time.perf_counter()
        with client.stream(method, url, **kw) as r:
            self.ttfb = (time.perf_counter() - t0) * 1000
            self.status = r.status_code
            if not stream:
                r.read()
                self.body = r.content
            else:
                self.body = b""
        self.total = (time.perf_counter() - t0) * 1000 if not stream else self.ttfb

    def json(self):
        return json.loads(self.body or b"null")


class Run:
    link = LINK

    def __init__(self, s: w.Stack) -> None:
        self.s, self.D = s, s.door_url
        self.rows: dict[tuple[str, str], dict] = {}

    def client(self, base: str | None = None, **kw) -> httpx.Client:
        base = base or self.D
        verify = self.s.verify if base == self.D else True
        return httpx.Client(base_url=base, timeout=60, verify=verify, headers={"origin": self.D} if base == self.D else {}, **kw)

    def record(self, method: str, path: str, auth: str, warm: list, cold: list, statuses: list, note: str = "", frozen: float = 0.0):
        tot = lambda xs: [x.total for x in xs]
        ttfb = [x.ttfb for x in warm]
        st = max(set(statuses), key=statuses.count) if statuses else None
        row = {"method": method, "path": path, **stats(tot(warm)), "status": st, "note": note, "auth": auth, "link": self.link,
               "cold": stats(tot(cold)), "ttfb": {k: v for k, v in stats(ttfb).items() if k != "n"},
               "statuses": {str(k): statuses.count(k) for k in sorted(set(statuses))}}
        if frozen:
            row["note"] = (note + "; " if note else "") + f"measured again: the first batch lost {frozen} s to a freeze"
        self.rows[(method, path)] = row
        say(f"{method} {path}", json.dumps({k: row[k] for k in ("n", "p50_ms", "p95_ms", "status")}), "cold", json.dumps(row["cold"]))

    def batch(self, method: str, path: str, auth: str, make, n_warm: int = WARM, n_cold: int = COLD, base: str | None = None,
              stream: bool = False, note: str = ""):
        """`make(client)` gives (url, kwargs) for one call, fresh each time (a proof, a body)."""
        frozen = 0.0
        for attempt in (1, 2):
            a = uptime() if not PROD else (0, 0)
            warm, cold, statuses = [], [], []
            c = self.client(base)
            url, kw = make(c)
            Timed(c, method, url, stream=stream, **kw)          # the connection and the process, warmed
            for _ in range(n_warm):
                url, kw = make(c)
                t = Timed(c, method, url, stream=stream, **kw)
                warm.append(t)
                statuses.append(t.status)
            c.close()
            for _ in range(n_cold):
                c = self.client(base)
                url, kw = make(c)
                t = Timed(c, method, url, stream=stream, **kw)
                cold.append(t)
                statuses.append(t.status)
                c.close()
            b = uptime() if not PROD else (0, 0)
            gone = lost(a, b) if not PROD else 0.0
            if gone <= 1.0:
                break
            frozen = gone
            say(f"{method} {path}: {gone} s lost to a freeze; measured again")
        self.record(method, path, auth, warm, cold, statuses, note, frozen)


def run_prod(out: dict) -> None:
    """Production's unauthenticated reads, from this Mac: no pinned name, the system's trust."""
    r = Run.__new__(Run)
    r.rows, r.D, r.link = {}, "https://app.wallflowers.io", out["link"]
    r.s = type("S", (), {"verify": True})()
    for path in ("/v2/icd", "/v2/signin.js"):
        r.batch("GET", path, "none", lambda c, p=path: (p, {}))
    out["routes"] = list(r.rows.values())


def run_door_test(s: w.Stack, out: dict) -> None:
    run = Run(s)
    D = s.door_url
    st = json.loads(STATE.read_text())
    site, fs = st["site"], st["rooms"]["Financial Support"]
    inv = routes()
    say("routes from app/door/routes.json:", len(inv))

    # PUBLIC
    page = httpx.get(D + "/", verify=s.verify, timeout=30).text
    # The page names the model's hash (icd.rs, 0f4d33e3): <meta id="wallflowers-icd" content="<sha256>">.
    icd_hash = (re.search(r'id="wallflowers-icd" content="([0-9a-f]{16,128})"', page) or [None, None])[1]
    public = {"/v2/icd": "/v2/icd", "/v2/signin.js": "/v2/signin.js", "/v2/work": "/v2/work?for=signin", "/signin": "/signin",
              "/": "/", "/index.html": "/index.html", "/door/face.css": f"/door/face.css?client={CLIENT}",
              "/door/face-mark": f"/door/face-mark?client={CLIENT}"}
    if icd_hash:
        public["/v2/icd/:hash"] = f"/v2/icd/{icd_hash}"
    for _, p in inv:
        if p.startswith("/door/") and p not in public:
            public[p] = p
    for p, url in public.items():
        run.batch("GET", p, "none", lambda c, u=url: (u, {}))
    claim = J.claim(site, "financial", None)
    run.batch("GET", "/join", "none", lambda c: ("/join", {"params": {"claim": claim}}), note="a claim, not spent: its 303")

    def bye(token: str, key) -> None:
        """A token's session signed out, untimed: the Door holds 64 at most, and a run makes many."""
        httpx.post(f"{D}/v2/signout", headers={"authorization": f"DPoP {token}", "dpop": w.dpop(key, "POST", f"{D}/v2/signout", token)},
                   verify=s.verify, timeout=30)

    # ACCOUNT: each step inside whole flows, warm on one connection, cold each on a new one.
    steps: dict[str, dict[str, list]] = {}

    class Refused(Exception):
        pass

    def keep(path: str, t: Timed, temp: str) -> None:
        """A step's time counts only when it answered 2xx; a refusal is counted in its statuses, and ends that flow."""
        d = steps.setdefault(path, {"warm": [], "cold": [], "st": []})
        d["st"].append(t.status)
        if not 200 <= t.status < 300:
            raise Refused(f"{path} {t.status} {t.body[:120]!r}")
        d[temp].append(t)

    def flow(c: httpx.Client, temp: str) -> None:
        body = {"name": "latency", **w.work(c, "signup")}
        t = Timed(c, "POST", "/v2/signup", json=body); keep("/v2/signup", t, temp)
        o, prf = t.json(), os.urandom(32)
        t = Timed(c, "POST", "/v2/signup/finish", json={"attempt": o["attempt"], "sealed": w.seal(o["key"], prf, o["attempt"])})
        keep("/v2/signup/finish", t, temp)
        t = Timed(c, "POST", "/v2/signout", json={}); keep("/v2/signout", t, temp)
        body = w.work(c, "signin") or None
        t = Timed(c, "POST", "/v2/signin", json=body); keep("/v2/signin", t, temp)
        a = t.json()
        verifier, state = w.b64u(os.urandom(32)), os.urandom(8).hex()
        cb = {"client": CLIENT, "redirect_uri": CALLBACK, "state": state, "code_challenge": w.b64u(hashlib.sha256(verifier.encode()).digest())}
        t = Timed(c, "POST", "/v2/signin/finish", json={"attempt": a["attempt"], "handle": o["handle"],
                                                         "sealed": w.seal(a["key"], prf, a["attempt"]), "client": cb})
        keep("/v2/signin/finish", t, temp)
        code = parse_qs(urlsplit(t.json()["redirect"]).query)["code"][0]
        k, url = w.dpop_key(), f"{D}/v2/token"
        t = Timed(c, "POST", "/v2/token", json={"code": code, "code_verifier": verifier, "client": CLIENT, "redirect_uri": CALLBACK},
                  headers={"dpop": w.dpop(k, "POST", url)})
        keep("/v2/token", t, temp)
        bye(t.json()["access_token"], k)
        # A site's sign-up held over its words: Continue.
        body = {"name": "latency site", **w.work(c, "signup")}
        t = Timed(c, "POST", "/v2/signup", json=body); keep("/v2/signup", t, temp)
        o2 = t.json()
        v2, s2 = w.b64u(os.urandom(32)), os.urandom(8).hex()
        cb2 = {"client": CLIENT, "redirect_uri": CALLBACK, "state": s2, "code_challenge": w.b64u(hashlib.sha256(v2.encode()).digest())}
        f2 = Timed(c, "POST", "/v2/signup/finish", json={"attempt": o2["attempt"], "sealed": w.seal(o2["key"], os.urandom(32), o2["attempt"]),
                                                         "client": cb2}).json()
        red = f2.get("redirect")
        if f2.get("continue"):
            t = Timed(c, "POST", "/v2/signup/continue", json={"continue": f2["continue"]}); keep("/v2/signup/continue", t, temp)
            red = t.json()["redirect"]
        if red:
            code2 = parse_qs(urlsplit(red).query)["code"][0]
            k2 = w.dpop_key()
            r2 = c.post("/v2/token", json={"code": code2, "code_verifier": v2, "client": CLIENT, "redirect_uri": CALLBACK},
                        headers={"dpop": w.dpop(k2, "POST", f"{D}/v2/token")})
            if r2.status_code == 200:
                bye(r2.json()["access_token"], k2)
        c.post("/v2/signout")

    def flow_kept(c, temp):
        try:
            flow(c, temp)
        except Refused as e:
            say("a flow refused:", e)

    a0 = uptime()
    wc = run.client()
    flow_kept(wc, "warm")                             # warm-up, discarded below
    steps.clear()
    for _ in range(max(3, WARM // 3)):
        flow_kept(wc, "warm")
    wc.close()
    for _ in range(max(2, COLD // 3)):
        c = run.client()
        flow_kept(c, "cold")
        c.close()
    gone = lost(a0, uptime())
    for p, d in steps.items():
        run.record("POST", p, "none", d["warm"], d["cold"], d["st"], "each step inside a whole flow; proof of work solved before the clock; "
                   "cold here: a new client per flow, its connection opened by the untimed proof-of-work fetch",
                   gone if gone > 1 else 0.0)

    # A member of Financial Support, and his site token (DPoP), as a site holds one.
    w.CLAIM_MJS = Path(os.environ.get("JA_KIOSK", "")).resolve() / "egg" / "kiosk" / "claim.mjs"
    m = run.client()
    m.get("/join", params={"claim": J.claim(site, "financial", None)}, follow_redirects=False)
    who = w.signup(s, "latency member", m)
    r = m.post("/v2/join")
    while r.status_code == 503:
        time.sleep(1)
        r = m.post("/v2/join")
    verifier, state = w.b64u(os.urandom(32)), os.urandom(8).hex()
    cb = {"client": CLIENT, "redirect_uri": CALLBACK, "state": state, "code_challenge": w.b64u(hashlib.sha256(verifier.encode()).digest())}
    sc = run.client()
    a = sc.post("/v2/signin", json=w.work(sc, "signin") or None).json()
    f = sc.post("/v2/signin/finish", json={"attempt": a["attempt"], "handle": who["handle"], "sealed": w.seal(a["key"], who["prf"], a["attempt"]),
                                           "client": cb}).json()
    code = parse_qs(urlsplit(f["redirect"]).query)["code"][0]
    k = w.dpop_key()
    tok = httpx.post(f"{D}/v2/token", json={"code": code, "code_verifier": verifier, "client": CLIENT, "redirect_uri": CALLBACK},
                     headers={"dpop": w.dpop(k, "POST", f"{D}/v2/token")}, verify=s.verify, timeout=30).json()["access_token"]

    def dp(method: str, path: str, **kw):
        def make(c):
            h = {"authorization": f"DPoP {tok}", "dpop": w.dpop(k, method, f"{D}{path.split('?')[0]}", tok)}
            return path, {"headers": h, **kw}
        return make

    run.batch("GET", "/v2/me", "dpop", dp("GET", "/v2/me"))
    run.batch("GET", "/v2/kinds", "dpop", dp("GET", "/v2/kinds"))
    run.batch("GET", "/v2/graph", "dpop", dp("GET", "/v2/graph"))
    run.batch("GET", "/v2/draft/:kind", "dpop", dp("GET", "/v2/draft/forum"), note="kind forum")
    n = [0]

    def mint(c):
        n[0] += 1
        return dp("POST", "/v2/mint", json={"kind": "group", "draft": {"name": f"latency {n[0]}"}})(c)

    def post(c):
        n[0] += 1
        return dp("POST", "/v2/apply", json={"object": fs, "op": "forum.post", "args": {"text": f"latency {n[0]}"}})(c)
    run.batch("POST", "/v2/mint", "dpop", mint, note="a group, by the site's token")
    run.batch("POST", "/v2/apply", "dpop", post, note="forum.post in Financial Support, by the site's token")
    run.batch("GET", "/v2/events", "dpop", dp("GET", "/v2/events"), stream=True,
              note="the stream, timed to its headers; a stream's connection closes with it, so each warm call opens one too")

    # COOKIE writes, the webapp's: a third of the samples, each on fresh objects.
    bundle = httpx.get(f"{ARC}/v1/bundle", timeout=30).text

    def batch_mint(c):
        n[0] += 1
        return "/v2/batch", {"json": {"steps": [{"do": "mint", "kind": "group", "draft": {"name": f"latency batch {n[0]}"}}]}}

    def add(c):
        oid = c.post("/v2/mint", json={"kind": "group", "draft": {"name": "latency add"}}).json()["object_id"]
        return "/v2/add", {"json": {"object": oid, "bundle": bundle}}

    def address(c):
        n[0] += 1
        host = c.post("/v2/batch", json={"steps": [{"do": "mint", "kind": "host", "draft": {"name": "latency host"}}]}).json()["made"][0]
        return "/v2/site/address", {"json": {"slug": f"lat-{int(time.time())}-{n[0]}", "host": host}}
    def cw(make):
        """The member's webapp session on the batch's client, then the call."""
        def made(c):
            c.cookies.update(m.cookies)
            return make(c)
        return made
    run.batch("POST", "/v2/batch", "cookie", cw(batch_mint), max(3, WARM // 3), max(2, COLD // 3))
    run.batch("POST", "/v2/add", "cookie", cw(add), max(3, WARM // 6), max(2, COLD // 5), note="the Arc added to a fresh object (minted before the clock); the add waits on that mint's write tail (NC-136)")
    run.batch("POST", "/v2/site/address", "cookie", cw(address), max(3, WARM // 6), max(2, COLD // 5),
              note="a fresh slug on a fresh Host (minted before the clock); the claim waits on that Host's write tail (NC-136)")

    def join(c):
        c.post("/v2/signout")                         # the previous joiner's session, untimed
        c.get("/join", params={"claim": J.claim(site, "financial", None)}, follow_redirects=False)
        w.signup(s, "latency joiner", c)
        return "/v2/join", {}
    run.batch("POST", "/v2/join", "cookie", join, max(3, WARM // 6), max(2, COLD // 5),
              note="each a new account with a fresh claim, made before the clock")
    bye(tok, k)
    sc.post("/v2/signout")
    m.post("/v2/signout")

    # THE ARC's routes, at loopback only.
    arc_public = {"/v1/face/:slug": f"/v1/face/{SLUG}", "/v1/face/:slug/items": f"/v1/face/{SLUG}/items",
                  "/v1/face/:slug/escape": f"/v1/face/{SLUG}/escape", "/v1/face/:slug/door": f"/v1/face/{SLUG}/door",
                  "/v1/face/:slug/brand": f"/v1/face/{SLUG}/brand", "/v1/face/:slug/m/:slot": f"/v1/face/{SLUG}/m/mark",
                  "/v1/health": "/v1/health", "/v1/arc": "/v1/arc", "/v1/version": "/v1/version", "/v1/bundle": "/v1/bundle"}
    for p, url in arc_public.items():
        if LINK == "venue":
            run.rows[("GET", p)] = {"method": "GET", "path": p, "n": 0, "p50_ms": None, "p95_ms": None, "status": None, "auth": "none",
                                    "link": LINK, "note": "the Arc's: no venue distance to kenjin-01 measured; see door-test-loopback.json"}
        else:
            run.batch("GET", p, "none", lambda c, u=url: (u, {}), base=ARC)

    # Every route, and what this run did with it.
    for (meth, p), r in inv.items():
        if (meth, p) in run.rows or (meth == "ANY" and any(k[1] == p for k in run.rows)):
            continue
        why = "internal: the Door to the person's process, inside the routes above" if r["audience"] == "internal" else \
              "not a site's call: the Door and the Arc between themselves" if r["router"] == "gateway" and meth == "POST" else \
              "not a site's call (the Arc's own consoles and planes)" if r["router"] == "gateway" else "no recipe yet"
        run.rows[(meth, p)] = {"method": meth, "path": p, "n": 0, "p50_ms": None, "p95_ms": None, "status": None, "auth": None,
                               "link": LINK, "note": why}
    out["routes"] = sorted(run.rows.values(), key=lambda r: (r["path"], r["method"]))


def main() -> int:
    if not OUT.name:
        print(__doc__)
        return 2
    out = {"at": datetime.now(timezone.utc).isoformat(timespec="seconds"), "from": "Ralph's Mac, Seoul (the harness's own machine)"}
    if PROD:
        out.update({"door": httpx.get("https://app.wallflowers.io/v2/icd", timeout=30).headers.get("x-door-commit", "production"),
                    "where": "production (app.wallflowers.io)", "link": "this Mac's own internet"})
        run_prod(out)
    else:
        if not (w.DEPLOYED and bp.DOOR_IP and STATE.is_file()):
            print(__doc__)
            return 2
        commit = subprocess.run(["limactl", "shell", "door-test", "cut", "-c1-40", "/opt/door/COMMIT"], capture_output=True, text=True).stdout.strip()
        out.update({"door": commit, "where": "door-test", "link": LINK})
        if LINK == "venue":
            import venue_link
            link = venue_link.Link().__enter__()
            import socket
            real = socket.getaddrinfo
            host = urlsplit(w.DEPLOYED).hostname

            def via(name, port, *a, **k):
                if name == host and int(port or 443) == 443:
                    return real("127.0.0.1", link.port, *a, **k)
                return real(name, port, *a, **k)
            socket.getaddrinfo = via
            out["link_detail"] = f"venue_link's hop, {venue_link.DOOR_RTT:g} ms round trip to the Door" + (" with jitter" if venue_link.JITTER else "")
        else:
            bp.map_name()
            out["link_detail"] = "direct to door-test's address"
        s = w.Stack(host="localhost")
        s.up()
        try:
            run_door_test(s, out)
        finally:
            s.down()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(out, indent=1, ensure_ascii=False) + "\n")
    measured = [r for r in out["routes"] if r["n"]]
    say(f"written {OUT}: {len(out['routes'])} routes, {len(measured)} measured")
    return 0


if __name__ == "__main__":
    sys.exit(main())
