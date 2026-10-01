"""The webapp and the sign-in window, timed (Ralph, 28 Sep: "the multisecond latency is
unacceptable, and renders the service unusable").

    E2E_PROFILE=release .venv/bin/python app/e2e/perf_audit.py OUT.json          # loopback
    .venv/bin/python app/e2e/perf_audit.py --live OUT.json                       # production

LOOPBACK: layer 1's system (wallflowers_path.Stack), release binaries with
E2E_PROFILE=release, and one Chrome for Testing per person, headless, each its own
profile, stopped with the Stack. A founder signs up, registers a Site, makes a room and
posts; a kiosk's visitor joins by the claim (A-3) in a second browser and watches the
room; the founder signs in again and again. What is read is the page's own User Timing
(app/web/door/perf.js), echoed to the console by this harness as each measure is made,
so a page that navigates away keeps its numbers, and the browser's Resource Timing. The
passkey is a virtual one: a person's own time at a passkey prompt is not in these numbers.
Nothing leaves the host.

TWO PASSES on one Stack. NEAR: loopback as it is, no distance anywhere. FAR: production's
distances put back (Software Engineering's measurements from door-01, 28 Sep): the Door is
in Singapore and the relay, the auth service and the Arc are in London behind Cloudflare,
so the Door reaches all three through a hop that adds a 200 ms round trip and two more to
each new connection (0.35-0.76 s a fresh request from door-01, as measured there); the browser reaches the Door over an 80 ms round trip (a visitor in
Seoul). FAR is a model, not production: its inputs are named and can be moved.

LIVE: production signed out and read-only: public GETs timed from here, and one cold load
of the webapp's face and of the window in the browser. No account, no write.
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import platform
import random
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

import httpx
import websockets

sys.path.insert(0, str(Path(__file__).resolve().parent))
from browser_path import Page, chrome  # noqa: E402
from wallflowers_path import PRODUCT, PROFILE, Fail, Stack, claim  # noqa: E402
import wallflowers_path  # noqa: E402

# PERF_SANDBOX=<seatbelt profile>: every process the Stack starts runs under it (no-egress:
# loopback only). The harness and its browsers do not: Chrome's GPU process will not run
# under a second seatbelt, and they load nothing but loopback pages.
SANDBOX = os.environ.get("PERF_SANDBOX", "")
if SANDBOX:
    _spawn = Stack.spawn
    Stack.spawn = lambda self, name, cmd, env, cwd=None: _spawn(self, name, ["sandbox-exec", "-f", SANDBOX, *cmd], env, cwd)
# Where `ps` is refused (a sandboxed harness), a process that answers has started.
try:
    subprocess.run(["ps", "-o", "time=", "-p", str(os.getpid())], capture_output=True)
except PermissionError:
    wallflowers_path.cpu_secs = lambda pid: 1.0

WARM = 5
QUICK = bool(os.environ.get("PERF_QUICK"))          # a smoke run: a few of each
PASSES = [p for p in os.environ.get("PERF_PASSES", "near,far").split(",") if p in ("near", "far")]
# FAR's inputs (ms): the Door to London, a new connection's setup, the browser to the Door.
FAR_RTT = int(os.environ.get("PERF_FAR_RTT", "200"))
FAR_CONNECT = int(os.environ.get("PERF_FAR_CONNECT", str(2 * FAR_RTT)))
FAR_BROWSER_RTT = int(os.environ.get("PERF_FAR_BROWSER_RTT", "80"))


def drawn(mean: float, mdev: float, floor: float, share: float = 1.0) -> float:
    """`share` of a round trip whose mean and deviation are `mean` and `mdev` over its minimum
    `floor`: the floor, and a gamma for what queues above it, a ping's right tail. Two draws of
    share 1/2 make one round trip, its mean, deviation and floor kept."""
    excess = mean - floor
    return share * floor + random.gammavariate(share * (excess / mdev) ** 2, mdev ** 2 / excess)


class Far:
    """A TCP hop with distance to `host`:`upstream`: each chunk delivered one-way-delay after it
    arrived, in order, bandwidth untouched; a new connection held for the setup a far TLS edge
    costs. Either end closing closes the other as soon as what it sent is delivered: an edge
    whose origin went away does not leave its client talking to nothing. It listens on
    127.0.0.1, on a port the system picks, until `close` or the process ends.

    With `jitter_ms` (a round trip's measured mdev) and `floor_ms` (its minimum), each chunk's
    one-way delay is drawn (`drawn`), so the round trip keeps its mean, deviation and floor, and
    delivered in order, as TCP delivers: a chunk drawn early waits for the one before it."""

    def __init__(self, upstream: int, rtt_ms: int, connect_ms: int, host: str = "127.0.0.1",
                 jitter_ms: float = 0, floor_ms: float = 0) -> None:
        self.host, self.up, self.one_way, self.connect = host, upstream, rtt_ms / 2000, connect_ms / 1000
        self.jitter, self.floor = jitter_ms / 1000, floor_ms / 1000
        if self.jitter and self.floor >= 2 * self.one_way:
            raise ValueError(f"a jittered round trip's floor ({floor_ms} ms) under its mean ({rtt_ms} ms)")
        self.loop = asyncio.new_event_loop()
        ready = threading.Event()
        self.thread = threading.Thread(target=self._serve, args=(ready,), daemon=True)
        self.thread.start()
        ready.wait(10)

    def _serve(self, ready: threading.Event) -> None:
        asyncio.set_event_loop(self.loop)
        self.srv = self.loop.run_until_complete(asyncio.start_server(self._conn, "127.0.0.1", 0))
        self.port = self.srv.sockets[0].getsockname()[1]
        ready.set()
        self.loop.run_forever()

    def close(self) -> None:
        """No new connection, and each it carries dropped; bounded, whatever a connection is
        doing, so a harness's teardown never waits on the hop."""
        async def shut() -> None:
            self.srv.close()
            # Until none is left: a connection's pipes start after it, and are cancelled after it.
            for _ in range(10):
                conns = [t for t in asyncio.all_tasks() if t is not asyncio.current_task()]
                if not conns:
                    break
                for t in conns:
                    t.cancel()
                await asyncio.wait(conns, timeout=1)
            self.loop.stop()
        if not self.thread.is_alive():
            return
        asyncio.run_coroutine_threadsafe(shut(), self.loop)
        self.thread.join(15)

    def _delay(self) -> float:
        """One way (s)."""
        return drawn(2 * self.one_way, self.jitter, self.floor, 0.5) if self.jitter else self.one_way

    async def _conn(self, r: asyncio.StreamReader, w: asyncio.StreamWriter) -> None:
        await asyncio.sleep(self.connect - 2 * self.one_way + self._delay() + self._delay())
        try:
            ur, uw = await asyncio.open_connection(self.host, self.up)
        except OSError:
            w.close()
            return
        ends = [asyncio.ensure_future(self._pipe(r, uw)), asyncio.ensure_future(self._pipe(ur, w))]
        try:
            await asyncio.wait(ends, return_when=asyncio.FIRST_COMPLETED)
        finally:
            for t in ends:
                t.cancel()
            for x in (w, uw):
                x.close()

    async def _pipe(self, src: asyncio.StreamReader, dst: asyncio.StreamWriter) -> None:
        q: asyncio.Queue = asyncio.Queue()

        async def deliver() -> None:
            while True:
                due, data = await q.get()
                if data is None:
                    return
                wait = due - time.monotonic()
                if wait > 0:
                    await asyncio.sleep(wait)
                dst.write(data)
                await dst.drain()

        out = asyncio.ensure_future(deliver())
        last = 0.0
        try:
            while data := await src.read(65536):
                last = max(last, time.monotonic() + self._delay())
                q.put_nowait((last, data))
            q.put_nowait((0, None))
            await out
        except OSError:
            pass
        finally:
            out.cancel()

# Each `wf:` measure, echoed to the console the moment it is made: a page that navigates
# right after (the window, on landing) keeps its numbers. The harness's, not the page's.
ECHO = """(() => {
  const P = window.performance, m = P && P.measure && P.measure.bind(P);
  if (!m) return;
  P.measure = function (name, o) {
    const e = m(name, o);
    if (e && String(name).indexOf('wf:') === 0) console.debug('wfperf ' + JSON.stringify(
      { name: name.slice(3), d: e.duration, s: e.startTime, o: P.timeOrigin, u: location.pathname, detail: e.detail || null }));
    return e;
  };
})();"""

RESOURCES = """performance.getEntriesByType('navigation').concat(performance.getEntriesByType('resource')).map(r => ({
  name: r.name.replace(location.origin, ''), type: r.initiatorType || r.entryType, start: r.startTime,
  ttfb: r.responseStart - r.startTime, end: r.responseEnd, transfer: r.transferSize, encoded: r.encodedBodySize,
  decoded: r.decodedBodySize, protocol: r.nextHopProtocol }))"""


class Tab(Page):
    """A page target whose `wf:` measures are kept as they are made."""

    def __init__(self, ws, who: str) -> None:
        self.who, self.log, self.net = who, [], {}
        super().__init__(ws)

    async def _read(self) -> None:
        async for raw in self.ws:
            d = json.loads(raw)
            if "id" in d and d["id"] in self.waiting:
                self.waiting.pop(d["id"]).set_result(d)
            elif d.get("method", "").startswith("Network."):
                p, m = d["params"], d["method"]
                if m == "Network.requestWillBeSent":
                    self.net[p["requestId"]] = {"url": p["request"]["url"], "method": p["request"]["method"], "sent": time.time()}
                elif p.get("requestId") in self.net:
                    r = self.net[p["requestId"]]
                    if m == "Network.responseReceived":
                        r["status"] = p["response"]["status"]
                    elif m in ("Network.loadingFinished", "Network.loadingFailed"):
                        r["done"] = time.time()
                        r["error"] = p.get("errorText")
            elif d.get("method") == "Runtime.consoleAPICalled":
                a = (d["params"].get("args") or [{}])[0].get("value", "")
                if isinstance(a, str) and a.startswith("wfperf "):
                    e = json.loads(a[7:])
                    e["at"] = e["o"] + e["s"] + e["d"]   # epoch ms at the measure's end
                    self.log.append(e)

    def of(self, name: str) -> list[dict]:
        return [e for e in self.log if e["name"] == name]

    async def measured(self, name: str, n: int, what: str, secs: float = 60) -> dict:
        """The n-th `name` measure, waited for. A refusal the page shows ends the wait."""
        end = time.monotonic() + secs
        while len(self.of(name)) < n:
            said = await self.ev("(() => { const s = document.getElementById('status'), r = document.getElementById('refused');"
                                 " return (s && !s.hidden && s.textContent) || (r && !r.hidden && r.textContent) || ''; })()")
            if said and not said.startswith("Your account is made"):
                raise Fail(f"{self.who}: {what}: the page said {said!r}")
            if time.monotonic() > end:
                now = time.time()
                waiting = [f"{r['method']} {r['url'].split('//', 1)[-1].split('/', 1)[-1][:60]} {now - r['sent']:.0f} s"
                           for r in self.net.values() if "done" not in r and "/v2/events" not in r["url"]]
                raise Fail(f"{self.who}: no {name} #{n} in {secs:.0f} s ({what}); last: {[e['name'] for e in self.log[-6:]]}; "
                           f"waiting on: {waiting}")
            await asyncio.sleep(0.05)
        return self.of(name)[n - 1]

    async def tap(self, element_id: str) -> float:
        """Click it; the page's own clock (epoch ms) at the click."""
        return await self.ev(f"(() => {{ const t = Date.now(); document.getElementById({json.dumps(element_id)}).click(); return t; }})()")


async def browser(work: Path, who: str, rtt: int = 0, loopback: bool = True) -> tuple[subprocess.Popen, Tab]:
    """A headless Chrome for Testing of its own, its port chosen by Chrome (port 0). Loopback
    only unless `loopback` is False (--live, production signed out)."""
    profile = work / f"chrome-{who}"
    proc = subprocess.Popen([chrome(), "--headless=new", "--remote-debugging-port=0", f"--user-data-dir={profile}",
                             "--no-first-run", "--no-default-browser-check", "--window-size=1280,900",
                             "--disable-background-networking", "--disable-component-update", "--disable-sync",
                             "--disable-default-apps", "--metrics-recording-only",
                             # Loopback and nothing else resolves: a page that names any other host reaches no one.
                             *(["--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1, EXCLUDE localhost"] if loopback else []),
                             # A headless page is never occluded, and its frames and timers run at speed.
                             "--disable-backgrounding-occluded-windows", "--disable-renderer-backgrounding",
                             "--disable-background-timer-throttling", "about:blank"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(300):
        try:
            port = int((profile / "DevToolsActivePort").read_text().split()[0])
            target = next(t for t in httpx.get(f"http://127.0.0.1:{port}/json/list", timeout=1).json() if t["type"] == "page")
            break
        except (OSError, ValueError, IndexError, httpx.HTTPError, StopIteration):
            await asyncio.sleep(0.2)
    else:
        proc.kill()
        raise Fail(f"Chrome for Testing ({who}) did not open its DevTools port")
    ws = await websockets.connect(target["webSocketDebuggerUrl"], max_size=None)
    t = Tab(ws, who)
    for m in ("Page.enable", "Runtime.enable", "Network.enable"):
        await t.cdp(m)
    await t.cdp("Page.addScriptToEvaluateOnNewDocument", source=ECHO)
    await t.cdp("WebAuthn.enable", enableUI=False)
    await t.cdp("WebAuthn.addVirtualAuthenticator", options={
        "protocol": "ctap2", "ctap2Version": "ctap2_1", "transport": "internal", "hasResidentKey": True,
        "hasUserVerification": True, "isUserVerified": True, "automaticPresenceSimulation": True, "hasPrf": True})
    t.rtt = rtt
    if rtt:
        await t.cdp("Network.emulateNetworkConditions", offline=False, latency=rtt, downloadThroughput=-1, uploadThroughput=-1)
    return proc, t


def stats(xs: list[float]) -> dict:
    if not xs:
        return {"n": 0}
    s = sorted(xs)
    rank = lambda q: s[min(len(s) - 1, max(0, -(-int(q * len(s) * 1000) // 1000) - 1))]
    return {"n": len(s), "p50": round(rank(0.5)), "p95": round(rank(0.95)), "max": round(s[-1]), "min": round(s[0]),
            "mean": round(statistics.fmean(s))}


def durations(t: Tab, name: str, since: int = 0) -> list[float]:
    return [e["d"] for e in t.log[since:] if e["name"] == name]


async def signup(t: Tab, D: str, ret: str = "/") -> dict:
    """The window's sign-up, as a person does it: New, Create the passkey, the words, Continue.
    Returns the click-to-interior times across the navigation."""
    n_land = len(t.of("landing"))
    if await t.ev("location.pathname") != "/signin":
        await t.go(f"{D}/signin?new&return=" + ret.replace("/", "%2F").replace("#", "%23"))
    # A window reached by a click has its address before its script: a tap then is lost.
    await t.until("typeof (document.getElementById('new') || {}).onclick === 'function'", "the window's script")
    await t.tap("new")
    await t.measured("signup:open", len(t.of("signup:open")) + 1, "the passkey step")
    clicked = await t.tap("saved")
    await t.measured("signup:words", len(t.of("signup:words")) + 1, "the words")
    words = await t.ev("document.querySelectorAll('#list li').length")
    unsaved = await t.ev("document.getElementById('status').hidden ? '' : document.getElementById('status').textContent")
    if unsaved:
        raise Fail(f"{t.who}: the account was made and not kept (NC-89): {unsaved!r}")
    if words != 24:
        raise Fail(f"{t.who}: {words} recovery words, not 24; the page: {await t.ev('document.getElementById(\"status\").textContent')}")
    done = await t.tap("done")
    land = await t.measured("landing", n_land + 1, "the interior after Continue")
    return {"words_after_click": t.of("signup:words")[-1]["at"] - clicked, "continue_to_interior": land["at"] - done}


def farther(s: Stack) -> dict[str, Far]:
    """The Door again, now in Singapore: relay, auth and the Arc each through a far hop."""
    hops = {k: Far(port, FAR_RTT, FAR_CONNECT) for k, port in (("relay", s.relay), ("auth", s.auth), ("arc", s.gw))}
    # The auth service names its public origin, and a Door signs for no other: from
    # Singapore that is the far hop's. Restarted with it, its store kept.
    s.stop_auth()
    w = wallflowers_path
    s.spawn("auth-far", [str(w.UVICORN), "app.main:app", "--host", "127.0.0.1", "--port", str(s.auth)],
            {"DB_PATH": str(s.work / "auth.db"), "PUBLIC_ORIGIN": f"http://127.0.0.1:{hops['auth'].port}"}, cwd=s.auth_dir)
    s.auth_proc = s.procs[-1]
    w.wait_for(f"{s.auth_url}/auth/health", "the auth service", proc=s.auth_proc, log=s.work / "auth-far.log")
    s.door_proc.terminate()
    s.door_proc.wait(10)
    s.start_door("door-far", {"DOOR_RELAY": f"ws://127.0.0.1:{hops['relay'].port}/v1/relay",
                              "DOOR_AUTH": f"http://127.0.0.1:{hops['auth'].port}",
                              "DOOR_ARC": f"http://127.0.0.1:{hops['arc'].port}"})
    return hops


async def run(s: Stack, out: dict, far: bool) -> None:
    D = s.door_url
    procs = []
    POSTS, CROSS, SIGNINS = (3, 2, 3) if QUICK else (10, 10, 10) if far else (20, 10, 20)
    rtt = FAR_BROWSER_RTT if far else 0
    out["inputs"] = {"door_to_london_rtt": FAR_RTT, "new_connection": FAR_CONNECT, "browser_to_door_rtt": FAR_BROWSER_RTT} if far else {}
    if far:
        farther(s)
    who = "far-" if far else ""
    tabs: list[Tab] = []
    try:
        pa, A = await browser(s.work, who + "founder", rtt)
        procs.append(pa)
        tabs.append(A)

        # ── signed out: "/" to the first paint of what it lands on, cold then warm, with
        # its waterfall. The face until a8276ff; since, the window it goes straight to.
        await A.cdp("Network.setCacheDisabled", cacheDisabled=True)
        asked = await A.ev("Date.now()")
        await A.go(D + "/")
        e = await A.measured("first-render", 1, "signed out, /")
        await asyncio.sleep(0.5)
        out["face_cold"] = {"summary": await A.ev("WallFlowersPerf.summary(true)"), "resources": await A.ev(RESOURCES),
                            "slash_to_first_render": e["at"] - asked}
        await A.cdp("Network.setCacheDisabled", cacheDisabled=False)
        warm = []
        for i in range(WARM):
            asked = await A.ev("Date.now()")
            await A.go(D + "/")
            warm.append((await A.measured("first-render", 2 + i, "signed out, /, warm"))["at"] - asked)
        out["face_warm_first_render"] = stats(warm)

        # ── the window, cold ──────────────────────────────────────────────────────
        await A.cdp("Network.setCacheDisabled", cacheDisabled=True)
        await A.go(D + "/signin?new&return=%2F")
        await A.measured("first-render", 2 + WARM, "the window")
        await asyncio.sleep(0.5)
        out["window_cold"] = {"summary": await A.ev("WallFlowersPerf.summary(true)"), "resources": await A.ev(RESOURCES)}

        # ── the founder signs up and lands; the interior's waterfall ─────────────
        out["founder_signup"] = await signup(A, D)
        await asyncio.sleep(1)
        out["interior_cold"] = {"summary": await A.ev("WallFlowersPerf.summary(true)"), "resources": await A.ev(RESOURCES)}
        await A.cdp("Network.setCacheDisabled", cacheDisabled=False)

        # ── Register a Site (O-58's draft), then a room ───────────────────────────
        draft = base64.urlsafe_b64encode(json.dumps({"kind": "Community", "name": "Perf", "purpose": "", "slug": "",
                                                     "pname": "", "face": None}).encode()).rstrip(b"=").decode()
        await A.go(f"{D}/?audit#register={draft}")
        await A.until("!document.getElementById('sheet').hidden", "the Register sheet")
        await A.tap("sheetGo")
        await A.measured("register", 1, "Register")
        g = await A.ev("fetch('/v2/graph').then(r => r.json())")
        # Named as the webapp names it (titleOf): the view's title, its display name, the name.
        named = lambda o: (o.get("view") or {}).get("title") or (o.get("view") or {}).get("display_name") or o.get("name")
        site = next((o["id"] for o in g["objects"] if o.get("kind") == "group" and named(o) == "Perf"), None)
        if not site:
            raise Fail(f"Register made no Site named Perf: {[(o.get('kind'), named(o)) for o in g['objects']]}")
        await A.ev("[...document.querySelectorAll('#side button.add')][0].click(), true")
        await A.until("!document.getElementById('sheet').hidden && !!document.querySelector('#sheetFields input')", "the room sheet")
        await A.ev("[...document.querySelectorAll('#sheetFields input')].forEach(i => i.value = i.type === 'number' ? '1' : 'general'), true")
        await A.tap("sheetGo")
        await A.measured("create", 1, "the room")
        await A.until("!document.getElementById('compose').hidden", "the room's composer")

        # ── the founder posts, alone: each post to the frame that shows it ────────
        mark = len(A.log)
        for i in range(POSTS):
            await A.ev(f"document.getElementById('say').value = 'solo {i}', true")
            await A.tap("send")
            await A.measured("post", i + 1, f"post {i}")
        out["post_solo"] = stats(durations(A, "post", mark))
        out["post_solo_apply"] = stats(durations(A, "POST /v2/apply", mark))
        out["post_solo_graph_fetch"] = stats(durations(A, "GET /v2/graph", mark))
        out["graph_bytes_by_post"] = [e["detail"]["bytes"] for e in A.log[mark:] if e["name"] == "GET /v2/graph" and e.get("detail")]

        # ── a kiosk's visitor: the Arc's node admitter on the Site and its room (runbook § 3,
        # D-58 (a): a fresh bundle each, since a key package is used once), then the claim ──
        g = await A.ev("fetch('/v2/graph').then(r => r.json())")
        room = next((o["id"] for o in g["objects"] if o.get("kind") == "forum" and named(o) == "general"), None)
        if not room:
            raise Fail("the room is not in the founder's graph")
        for obj in (site, room):
            b = httpx.get(f"http://127.0.0.1:{s.gw}/v1/bundle", timeout=30)
            if b.status_code != 200:
                raise Fail(f"the Arc's /v1/bundle: {b.status_code} {b.text[:200]}")
            added = await A.ev(f"fetch('/v2/add', {{method: 'POST', headers: {{'content-type': 'application/json'}}, "
                               f"body: JSON.stringify({{object: {json.dumps(obj)}, bundle: {json.dumps(b.text)}}})}}).then(r => r.json())")
            role = await A.ev(f"fetch('/v2/apply', {{method: 'POST', headers: {{'content-type': 'application/json'}}, "
                              f"body: JSON.stringify({{object: {json.dumps(obj)}, op: 'base.setRole', args: {{member: {json.dumps(added['member'])}, role: 'admitter'}}}})}}).then(r => r.status)")
            if role != 200:
                raise Fail(f"admitter on {'the Site' if obj == site else 'the room'}: {role}")
        await asyncio.sleep(5)   # a visitor arrives a while after the founder set up
        pb, B = await browser(s.work, who + "visitor", rtt)
        procs.append(pb)
        tabs.append(B)
        token = claim(site)
        t_join = await B.ev("Date.now()")
        await B.go(f"{D}/join?claim={token}")
        # Signed out, /#join is the window at once, on a new account (a8276ff).
        await B.until("location.pathname === '/signin' && new URLSearchParams(location.search).has('new')", "the window, from /join")
        out["visitor_signup"] = await signup(B, D)
        try:
            j = await B.measured("join", 1, "the Site, after /v2/join", 90)
            out["visitor_join"] = {"join_step": j["d"], "claim_to_site": j["at"] - t_join}
        except Fail as e:
            out["visitor_join"] = {"failed": str(e), "problems": await B.ev("[...document.querySelectorAll('#feed .miss')].map(x => x.textContent)")}
            raise
        # The visitor opens the room and watches it.
        try:
            await B.until("[...document.querySelectorAll('#side button.ch')].some(b => b.textContent.includes('general'))", "the room in the visitor's Site", 60)
        except Fail:
            out["visitor_room_missing"] = await B.ev("""fetch('/v2/graph').then(r => r.json()).then(g => ({
              side: [...document.querySelectorAll('#side button')].map(b => b.textContent),
              problems: [...document.querySelectorAll('#feed .miss')].map(x => x.textContent),
              objects: g.objects.map(o => [o.kind, o.name || (o.view || {}).title || '', o.folds, (o.view && o.view.parts || []).map(p => p.role)])}))""")
            raise
        await B.ev("[...document.querySelectorAll('#side button.ch')].find(b => b.textContent.includes('general')).click(), true")
        await B.ev("""window.__seen = {}; new MutationObserver(() => {
            for (const p of document.querySelectorAll('#feed .msg p')) { const t = p.textContent; if (!__seen[t]) __seen[t] = Date.now(); }
          }).observe(document.getElementById('feed'), {childList: true, subtree: true}); true""")

        # ── the founder posts; the visitor sees it ───────────────────────────────
        cross, markA, markB = [], len(A.log), len(B.log)
        for i in range(CROSS):
            text = f"cross {i}"
            await A.ev(f"document.getElementById('say').value = {json.dumps(text)}, true")
            sent = await A.tap("send")
            end = time.monotonic() + 60
            seen = None
            while seen is None and time.monotonic() < end:
                seen = await B.ev(f"window.__seen[{json.dumps(text)}] || null")
                await asyncio.sleep(0.05)
            cross.append(None if seen is None else seen - sent)
            await A.measured("post", POSTS + i + 1, f"cross post {i}")
            await asyncio.sleep(0.5)
        out["post_to_other_member"] = stats([c for c in cross if c is not None])
        out["post_to_other_member_raw"] = cross
        out["post_to_other_member_missed"] = sum(1 for c in cross if c is None)
        out["other_member_changed_to_drawn"] = stats(durations(B, "changed", markB))
        out["post_author_with_member"] = stats(durations(A, "post", markA))

        # ── sign-in, again and again ─────────────────────────────────────────────
        rows = []
        for i in range(SIGNINS):
            await A.ev("fetch('/v2/signout', {method: 'POST'}).then(r => r.status)")
            await A.go(f"{D}/signin?return=%2F")
            n_land = len(A.of("landing"))
            clicked = await A.tap("in")
            land = await A.measured("landing", n_land + 1, f"sign-in {i}")
            last = lambda n: A.of(n)[-1]["d"] if A.of(n) else None
            rows.append({"click_to_interior": land["at"] - clicked, "signin": last("signin"), "start": last("signin:start"),
                         "work": last("signin:work"), "passkey": last("signin:passkey"), "seal": last("signin:seal"),
                         "finish": last("signin:finish"), "landing": land["d"]})
        out["signin"] = {k: stats([r[k] for r in rows if r[k] is not None]) for k in rows[0]}
        out["signin_raw"] = rows

        # ── the same on a phone: 4x slower CPU, 100 ms round trips, 10/5 Mbit/s ──
        await A.cdp("Emulation.setDeviceMetricsOverride", width=402, height=874, deviceScaleFactor=3, mobile=True)
        await A.cdp("Emulation.setCPUThrottlingRate", rate=4)
        await A.cdp("Network.emulateNetworkConditions", offline=False, latency=max(100, rtt),
                    downloadThroughput=10e6 / 8, uploadThroughput=5e6 / 8)
        await A.cdp("Network.setCacheDisabled", cacheDisabled=True)
        n_land = len(A.of("landing"))
        await A.go(D + "/")
        await A.measured("landing", n_land + 1, "the interior on a phone, cold", 120)
        await asyncio.sleep(0.5)
        out["phone_interior_cold"] = {"summary": await A.ev("WallFlowersPerf.summary(true)"), "resources": await A.ev(RESOURCES)}
        await A.cdp("Network.setCacheDisabled", cacheDisabled=False)
        n_land = len(A.of("landing"))
        await A.go(D + "/")
        await A.measured("landing", n_land + 1, "the interior on a phone, warm", 120)
        # Into the room, and post.
        await A.ev("[...document.querySelectorAll('#srail button.ico')][0].click(), true")
        await A.until("[...document.querySelectorAll('#side button.ch')].some(b => b.textContent.includes('general'))", "the room, on a phone")
        await A.ev("[...document.querySelectorAll('#side button.ch')].find(b => b.textContent.includes('general')).click(), true")
        await A.until("!document.getElementById('compose').hidden", "the room's composer, on a phone", 60)
        mark, n_post = len(A.log), len(A.of("post"))
        for i in range(5):
            await A.ev(f"document.getElementById('say').value = 'phone {i}', true")
            await A.tap("send")
            await A.measured("post", n_post + i + 1, f"phone post {i}", 240 if far else 120)
        out["phone_post"] = stats(durations(A, "post", mark))
        out["phone_render"] = stats(durations(A, "render", mark))

        out["founder_summary"] = await A.ev("WallFlowersPerf.summary(true)")
        out["visitor_summary"] = await B.ev("WallFlowersPerf.summary(true)")
    finally:
        out["logs"] = {t.who: t.log for t in tabs}
        # Every request the browsers made, and how many went anywhere but this host (none may).
        out["requests"] = {t.who: len(t.net) for t in tabs}
        out["off_host"] = sorted({r["url"] for t in tabs for r in t.net.values()
                                  if not r["url"].split("//", 1)[-1].startswith(("localhost", "127.0.0.1", "[::1]"))
                                  and not r["url"].startswith(("data:", "blob:", "about:"))})
        for p in procs:
            p.kill()


async def live(out: dict) -> None:
    """Production, signed out and read-only."""
    urls = {"door_face": "https://app.wallflowers.io/", "door_window": "https://app.wallflowers.io/signin",
            "door_icd": "https://app.wallflowers.io/v2/icd", "door_script": "https://app.wallflowers.io/webapp.js",
            "www": "https://wallflowers.io/"}
    out["live_http"] = {}
    for k, u in urls.items():
        cold, warm, meta = [], [], {}
        for _ in range(8):
            with httpx.Client(http2=False, timeout=30) as c:     # a new connection: DNS, TCP, TLS each time
                t = time.perf_counter()
                with c.stream("GET", u) as r:
                    ttfb = time.perf_counter() - t
                    body = r.read()
                cold.append({"ttfb": ttfb * 1000, "total": (time.perf_counter() - t) * 1000})
                meta = {"status": r.status_code, "bytes": len(body), "encoding": r.headers.get("content-encoding"),
                        "cache": r.headers.get("cache-control"), "server": r.headers.get("server"), "cf": r.headers.get("cf-ray")}
        with httpx.Client(timeout=30) as c:
            c.get(u)
            for _ in range(8):
                t = time.perf_counter()
                c.get(u)
                warm.append((time.perf_counter() - t) * 1000)
        out["live_http"][k] = {"url": u, **meta, "cold_ttfb": stats([x["ttfb"] for x in cold]),
                               "cold_total": stats([x["total"] for x in cold]), "warm": stats(warm)}
    work = Path(os.environ.get("TMPDIR", "/tmp")) / f"wf-perf-live.{os.getpid()}"
    work.mkdir(parents=True, exist_ok=True)
    proc, t = await browser(work, "live", loopback=False)
    try:
        await t.cdp("Network.setCacheDisabled", cacheDisabled=True)
        for name, u in (("live_face_cold", "https://app.wallflowers.io/"), ("live_window_cold", "https://app.wallflowers.io/signin")):
            await t.go(u)
            await asyncio.sleep(3)
            out[name] = {"resources": await t.ev(RESOURCES),
                         "paint": await t.ev("Object.fromEntries(performance.getEntriesByType('paint').map(e => [e.name, e.startTime]))")}
    finally:
        proc.kill()


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    dest = Path(args[0] if args else "perf-audit.json")
    head = subprocess.run(["git", "-C", str(PRODUCT), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    out: dict = {"at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "commit": head, "profile": PROFILE,
                 "machine": f"{platform.system()} {platform.machine()}, {os.cpu_count()} cores",
                 "load": os.getloadavg(),
                 "chrome": Path(chrome()).parts[-5] if len(Path(chrome()).parts) > 5 else chrome()}
    if "--live" in sys.argv:
        asyncio.run(live(out))
    else:
        s = Stack(host="localhost")
        try:
            s.build()
            s.up()
            for name in PASSES:
                far = name == "far"
                out[name] = {}
                try:
                    asyncio.run(run(s, out[name], far))
                except (Fail, TimeoutError, OSError) as e:
                    import traceback
                    out[name]["failed"] = str(e) or type(e).__name__
                    out[name]["where"] = traceback.format_exc()[-1500:]
                    out["failed"] = f"{name}: {out[name]['failed']}"
        except (Fail, TimeoutError, OSError) as e:
            out["failed"] = str(e) or type(e).__name__
        finally:
            s.down()
    leaked = [u for k in ("near", "far") for u in (out.get(k) or {}).get("off_host", [])]
    if leaked:
        out["failed"] = f"requests left the host: {leaked[:5]}"
    dest.write_text(json.dumps(out, indent=1))
    brief = lambda d: {k: v for k, v in d.items() if k not in ("logs", "signin_raw") and not (isinstance(v, dict) and "resources" in v)}
    print(json.dumps({k: brief(v) if isinstance(v, dict) else v for k, v in out.items()}, indent=1)[:12000])
    return 1 if out.get("failed") else 0


if __name__ == "__main__":
    sys.exit(main())
