"""The venue's distances, in front of door-test (UX, 29 Sep, for Ralph's "a full performance
analysis, for the time it takes for a new member to get into the live forums (with history)
and sending/receiving messages").

    PERF_LINK=venue: new_member_perf.py calls venue_link.start() before its first window, and
    `await venue_link.attach(b)` on each (a journeys.Browser); venue_link.stop() at the end.

    E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<door-test's edge root> \\
      .venv/bin/python app/e2e/venue_link.py       # the check: curl and a browser through it

THE DOOR LEG: perf_audit's Far, to door-test's Door (E2E_DOOR_IP, 443). Each chunk is
delivered one way later, so TLS, the event stream and what the relay pushes through the Door
all cross the round trip; a new connection is held one round trip for its TCP handshake, since
TLS then crosses the hop itself. Chrome reaches it by the resolver rule, MAP door.localhost
127.0.0.1:<hop> (browser_path.pinned() reads E2E_DOOR_VIA, which start() sets); the name, its
SNI and the edge's pin are unchanged.

THE SITE LEG: each request to the site's origin (ja_rehearsal.SITE) held one round trip before
it goes, from a second DevTools session on the same page (Fetch), which leaves the harness's
own session as it is.

THE INPUTS (ms), each overridable. The venue's Pi, measured there (MANAGE, ssh egg, 20 pings
each, 29 Sep): door-01 (app.wallflowers.io) min 103.7, avg 132.5, max 258.8, mdev 38.3;
egregores-echoes.com min 130.5, avg 153.3, max 239.7, mdev 29.8. So VENUE_DOOR_RTT 130 and
VENUE_SITE_RTT 150. With PERF_JITTER=1 each leg's round trips keep those means, the mdev
(VENUE_DOOR_MDEV, VENUE_SITE_MDEV) and the minimum (VENUE_DOOR_MIN, VENUE_SITE_MIN): the minimum
and a gamma above it (perf_audit.drawn).
For contrast, this Mac's own link, 29 Sep about 12:00Z: door-01 513-548 by ping, 516-552 by TCP
connect; Cloudflare 278-417 by ping. That is this Mac's, not the venue's.

NOT MODELLED: door-test's Door reaches its Arc and relay on its own host. door-01 reaches
kenjin-01 in London (perf_audit's FAR has it at 200 ms): the admission and the history's
fetch pay that in production and not here.

WHAT A USERSPACE HOP CANNOT DO: hold the TCP handshake itself, which the kernel completes
before accept. Through it curl's time_connect is ~0, and the round trip it stands for is in
time_appconnect instead: 2 RTT, the hold and TLS 1.3's. A new connection's first byte is 3 RTT
either way, and the check holds the hop to that.
"""
from __future__ import annotations

import asyncio
import atexit
import json
import os
import statistics
import subprocess
import sys
import tempfile
import urllib.parse
from pathlib import Path

import httpx
import websockets

sys.path.insert(0, str(Path(__file__).resolve().parent))
from browser_path import DOOR_IP, Page, chrome, elsewhere, pinned  # noqa: E402
from perf_audit import Far, drawn  # noqa: E402
from wallflowers_path import DEPLOYED, Fail  # noqa: E402

DOOR_RTT = float(os.environ.get("VENUE_DOOR_RTT", "130"))
SITE_RTT = float(os.environ.get("VENUE_SITE_RTT", "150"))
JITTER = os.environ.get("PERF_JITTER") == "1"
DOOR_MDEV, DOOR_MIN = float(os.environ.get("VENUE_DOOR_MDEV", "38.3")), float(os.environ.get("VENUE_DOOR_MIN", "103.7"))
SITE_MDEV, SITE_MIN = float(os.environ.get("VENUE_SITE_MDEV", "29.8")), float(os.environ.get("VENUE_SITE_MIN", "130.5"))
DOOR_PORT = 443


class Link:
    """The Door leg, for as long as the `with` lasts: Chrome launched in it (browser_path.pinned())
    reaches the Door through the hop."""

    def __init__(self, rtt_ms: float = DOOR_RTT, jitter: bool = JITTER) -> None:
        if not (DEPLOYED and DOOR_IP and os.environ.get("E2E_DOOR_CA")):
            raise Fail("the venue link is door-test's: set E2E_DOOR, E2E_DOOR_IP and E2E_DOOR_CA")
        self.rtt, self.jitter = rtt_ms, jitter

    def __enter__(self) -> "Link":
        self.hop = Far(DOOR_PORT, self.rtt, self.rtt, host=DOOR_IP,
                       jitter_ms=DOOR_MDEV if self.jitter else 0, floor_ms=DOOR_MIN if self.jitter else 0)
        self.port = self.hop.port
        self.via = f"127.0.0.1:{self.port}"
        os.environ["E2E_DOOR_VIA"] = self.via
        return self

    def __exit__(self, *exc) -> None:
        if os.environ.get("E2E_DOOR_VIA") == self.via:
            del os.environ["E2E_DOOR_VIA"]
        self.hop.close()

    def flags(self) -> list[str]:
        """Chrome's: the Door's name at the hop, its edge's key trusted."""
        return pinned(to=f"127.0.0.1:{self.port}")


def _page_ws(page) -> str:
    """The DevTools address of a page: a browser_path.Page's own, the first page of a DevTools
    port's, or a page's webSocketDebuggerUrl as given."""
    if isinstance(page, Page):
        return f"ws://127.0.0.1:{page.ws.remote_address[1]}{page.ws.request.path}"
    if isinstance(page, int):
        return next(t for t in httpx.get(f"http://127.0.0.1:{page}/json/list", timeout=5).json()
                    if t["type"] == "page")["webSocketDebuggerUrl"]
    return page


async def site_delay(page, site: str, rtt_ms: float = SITE_RTT, jitter: bool = JITTER):
    """The site leg: every request to `site`'s origin held `rtt_ms` before it goes (with
    `jitter`, drawn with SITE_MDEV over SITE_MIN), until the function returned is awaited. `page`: a browser_path.Page, a DevTools port, or a page's webSocketDebuggerUrl."""
    ws = await websockets.connect(_page_ws(page), max_size=None)
    waiting: dict[int, asyncio.Future] = {}
    held: set[asyncio.Task] = set()
    n = 0

    async def cdp(method: str, **params) -> dict:
        nonlocal n
        n += 1
        waiting[n] = asyncio.get_running_loop().create_future()
        await ws.send(json.dumps({"id": n, "method": method, "params": params}))
        d = await asyncio.wait_for(waiting[n], 30)
        if "error" in d:
            raise Fail(f"{method}: {d['error']}")
        return d.get("result", {})

    async def hold(rid: str) -> None:
        await asyncio.sleep((drawn(rtt_ms, SITE_MDEV, SITE_MIN) if jitter else rtt_ms) / 1000)
        try:
            await cdp("Fetch.continueRequest", requestId=rid)
        except (Fail, websockets.ConnectionClosed, TimeoutError):
            pass   # the page went, or the leg was stopped

    async def read() -> None:
        try:
            async for raw in ws:
                d = json.loads(raw)
                if d.get("method") == "Fetch.requestPaused":
                    t = asyncio.create_task(hold(d["params"]["requestId"]))
                    held.add(t)
                    t.add_done_callback(held.discard)
                elif d.get("id") in waiting:
                    waiting.pop(d["id"]).set_result(d)
        except websockets.ConnectionClosed:
            pass
        finally:
            # the page gone (its browser closed) or the leg stopped: nothing is left waiting
            for t in list(held):
                t.cancel()
            for f in waiting.values():
                f.cancel()

    reader = asyncio.create_task(read())
    await cdp("Fetch.enable", patterns=[{"urlPattern": site.rstrip("/") + "/*", "requestStage": "Request"}])

    async def stop() -> None:
        for t in list(held):
            t.cancel()
        if not reader.done():
            try:
                await asyncio.wait_for(cdp("Fetch.disable"), 5)
            except (Fail, websockets.ConnectionClosed, TimeoutError, asyncio.CancelledError):
                pass
        reader.cancel()
        try:
            await asyncio.wait_for(ws.close(), 5)
        except (TimeoutError, websockets.ConnectionClosed):
            pass
    return stop


_link: Link | None = None


def start() -> Link:
    """The Door leg for this process, before its first window."""
    global _link
    if _link is None:
        _link = Link().__enter__()
        atexit.register(stop)
    return _link


def stop() -> None:
    global _link
    if _link is not None:
        _link.__exit__(None, None, None)
        _link = None


async def attach(b) -> None:
    """A window (journeys.Browser) at the venue: refused unless it was launched through the
    Door leg; then the site leg on its page, until b.close()."""
    if _link is None:
        raise Fail("PERF_LINK=venue: venue_link.start() before the first window")
    if not any(_link.via in a for a in b.proc.args):
        raise Fail(f"a window launched before the venue link, or around it: its Door is not {_link.via}")
    from ja_rehearsal import SITE
    b.venue_site = await site_delay(b.p, SITE)


# ── the check ─────────────────────────────────────────────────────────────────

def door_sessions() -> int:
    """The Door's session processes on door-test: the gate before anything loads it."""
    r = subprocess.run(["limactl", "shell", "door-test", "--", "sudo", "sh", "-c",
                        "pgrep -P $(systemctl show -p MainPID --value door.service) | wc -l"],
                       capture_output=True, text=True, timeout=60)
    return int(r.stdout.strip() or "-1")


def curl(url: str, to: str = "", n: int = 2) -> list[dict]:
    """`n` transfers of `url` on one connection, each timed by curl (s)."""
    host = urllib.parse.urlsplit(url).hostname
    route = ["--connect-to", f"{host}:{DOOR_PORT}:{to}"] if to else ["--resolve", f"{host}:{DOOR_PORT}:{DOOR_IP}"]
    w = '{"connect":%{time_connect},"tls":%{time_appconnect},"first_byte":%{time_starttransfer},"http":"%{http_version}","code":%{http_code}}\\n'
    r = subprocess.run(["curl", "-s", "--max-time", "30", "--cacert", os.environ["E2E_DOOR_CA"], *route, "-w", w,
                        *sum([[url, "-o", "/dev/null"] for _ in range(n)], [])],
                       capture_output=True, text=True, timeout=60)
    return [json.loads(line) for line in r.stdout.splitlines() if line.startswith("{")]


def within(got: float, model: float, tol: float = 0.10) -> bool:
    return abs(got - model) <= tol * model


async def in_chrome(link: Link, work: Path, n: int = 5, jitter: bool = False) -> dict:
    """One headless Chrome through the link: the Door's navigation timing, and `n` fetches to a
    site origin under site_delay and to another that is not."""
    site_port, other_port = elsewhere({}), elsewhere({})
    site, other = f"http://localhost:{site_port}", f"http://localhost:{other_port}"
    profile = work / "chrome-venue"
    proc = subprocess.Popen([chrome(), *link.flags(), "--remote-debugging-port=0", f"--user-data-dir={profile}",
                             "--no-first-run", "--no-default-browser-check", "--disable-background-networking",
                             "--disable-component-update", "--disable-sync", "about:blank"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(300):
            try:
                port = int((profile / "DevToolsActivePort").read_text().split()[0])
                target = next(t for t in httpx.get(f"http://127.0.0.1:{port}/json/list", timeout=1).json() if t["type"] == "page")
                break
            except (OSError, ValueError, IndexError, httpx.HTTPError, StopIteration):
                await asyncio.sleep(0.2)
        else:
            raise Fail("Chrome for Testing did not open its DevTools port")
        async with websockets.connect(target["webSocketDebuggerUrl"], max_size=None) as ws:
            p = Page(ws)
            await p.cdp("Page.enable")
            await p.cdp("Runtime.enable")
            await p.go(f"{DEPLOYED}/v2/icd")
            nav = await p.ev("(() => { const n = performance.getEntriesByType('navigation')[0];"
                             " return {connect: n.connectEnd - n.connectStart, first_byte: n.responseStart - n.requestStart,"
                             " protocol: n.nextHopProtocol}; })()")
            stop = await site_delay(p, site, jitter=jitter)
            await p.go(f"{site}/")
            timed = ("(async u => { const t = performance.now(); await fetch(u, {cache: 'no-store', mode: 'no-cors'});"
                     " return performance.now() - t; })")
            to_site = [await p.ev(f"{timed}({json.dumps(site + '/x' + str(i))})") for i in range(n)]
            to_other = [await p.ev(f"{timed}({json.dumps(other + '/x' + str(i))})") for i in range(5)]
            await stop()
            after = [await p.ev(f"{timed}({json.dumps(site + '/y' + str(i))})") for i in range(3)]
            p.reader.cancel()
        return {"door_navigation": nav, "site_fetch_ms": to_site, "other_fetch_ms": to_other, "site_after_stop_ms": after}
    finally:
        proc.kill()
        proc.wait(10)


def simulated(rtt: float, mdev: float, floor: float, legs: int, n: int = 100_000) -> tuple[float, float]:
    """The jittered round trip's mean and deviation, drawn as the link draws it: the site's in
    one draw, the Door's as two one-way draws (Far._delay)."""
    draw = (lambda: drawn(rtt, mdev, floor)) if legs == 1 else (lambda: drawn(rtt, mdev, floor, 0.5) + drawn(rtt, mdev, floor, 0.5))
    xs = [draw() for _ in range(n)]
    return statistics.fmean(xs), statistics.pstdev(xs)


def main() -> int:
    busy = door_sessions()
    if busy != 0:
        print(f"venue_link.py: the Door on door-test has {busy} session process(es); the check waits for a quiet Door")
        return 3
    url = f"{DEPLOYED}/v2/icd"
    direct = [curl(url) for _ in range(5)]
    base_new = statistics.median(t[0]["first_byte"] - t[0]["tls"] for t in direct) * 1000
    base_again = statistics.median(t[1]["first_byte"] for t in direct) * 1000
    rtt = DOOR_RTT
    out: dict = {"inputs": {"door_rtt": DOOR_RTT, "site_rtt": SITE_RTT}, "direct": direct}
    with Link(jitter=False) as link, tempfile.TemporaryDirectory(prefix="venue-") as work:
        hopped = [curl(url, f"127.0.0.1:{link.port}") for _ in range(5)]
        out["hop"] = hopped
        out["chrome"] = asyncio.run(in_chrome(link, Path(work)))
    if JITTER:
        with Link(jitter=True) as link, tempfile.TemporaryDirectory(prefix="venue-") as work:
            again_j = [t["first_byte"] * 1000 - base_again for t in curl(url, f"127.0.0.1:{link.port}", 41)[1:]]
            site_j = asyncio.run(in_chrome(link, Path(work), 40, jitter=True))["site_fetch_ms"]
    ms = lambda xs: [round(x * 1000) for x in xs]
    tls, first, again = ms([t[0]["tls"] for t in hopped]), ms([t[0]["first_byte"] for t in hopped]), ms([t[1]["first_byte"] for t in hopped])
    nav, ch = out["chrome"]["door_navigation"], out["chrome"]
    checks = [
        ("a new connection's TLS done: the hold and TLS 1.3's round trip, 2 RTT", tls, 2 * rtt),
        ("its first byte: 3 RTT and the Door's own time", first, 3 * rtt + base_new),
        ("a request again on that connection: 1 RTT and the Door's own time", again, rtt + base_again),
        ("Chrome's connection to the Door, through the resolver rule: 2 RTT", [round(nav["connect"])], 2 * rtt),
        ("Chrome's request to the Door: 1 RTT", [round(nav["first_byte"])], rtt),
        ("a fetch to the site's origin: 1 site RTT", [round(x) for x in ch["site_fetch_ms"]], SITE_RTT),
    ]
    ok = True
    out["checks"] = []
    for what, got, model in checks:
        good = all(within(g, model) for g in got)
        ok &= good
        out["checks"].append({"what": what, "model_ms": round(model), "got_ms": got, "within_10pc": good})
    others = [round(x) for x in ch["other_fetch_ms"] + ch["site_after_stop_ms"]]
    untouched = all(x < 0.1 * SITE_RTT for x in others)
    ok &= untouched
    out["checks"].append({"what": "another origin, and the site once the leg is stopped: not held",
                          "limit_ms": round(0.1 * SITE_RTT), "got_ms": others, "within": untouched})
    if JITTER:
        for what, got, (mean, sd) in (("the Door, jittered: a request on a held connection", again_j, simulated(DOOR_RTT, DOOR_MDEV, DOOR_MIN, 2)),
                                      ("the site, jittered: a fetch", site_j, simulated(SITE_RTT, SITE_MDEV, SITE_MIN, 1))):
            m, d = statistics.fmean(got), statistics.pstdev(got)
            good = within(m, mean) and within(d, sd, 0.5)
            ok &= good
            out["checks"].append({"what": what, "model_mean_ms": round(mean), "model_sd_ms": round(sd), "n": len(got),
                                  "mean_ms": round(m), "sd_ms": round(d), "min_ms": round(min(got)), "max_ms": round(max(got)),
                                  "mean_within_10pc_sd_within_50pc": good})
    out["curl_time_connect_ms"] = ms([t[0]["connect"] for t in hopped])
    print(json.dumps({k: v for k, v in out.items() if k not in ("direct", "hop")}, indent=1))
    print(f"venue_link: {'the model holds' if ok else 'OFF THE MODEL'} (Door {DOOR_RTT} ms, site {SITE_RTT} ms, "
          f"{nav['protocol']}; curl's time_connect through the hop {out['curl_time_connect_ms']} ms, as a userspace hop has it)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
