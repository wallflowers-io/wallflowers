"""wallflowers.io's sign-up, screen by screen, from the first load to the webapp hydrated (Ralph,
29 Sep, via UX). The egg's twin is egregore's egg-journeys.html. The page: signup_funnel_page.py.

    .venv/bin/python app/e2e/signup_funnel.py OUT.json         # FUNNEL_LEGS=live,stack (both)
    .venv/bin/python app/e2e/signup_funnel.py --html OUT.json PAGE.html [EVIDENCE.json]

LIVE: production signed out and read-only, FUNNEL_REPEAT cold runs a viewport: the entrance,
/signup's three steps, the Door's window; then /site, the mark, Skip to account and Sign in. It
stops at the window: no account, no passkey. STACK: the loopback Stack at this checkout with the
draft LIVE's form made, near and at production's distances (perf_audit.farther): the window, the
passkey sheets (a virtual PRF authenticator), the words, Register, the first card, the feed; then
a sheet dismissed and Skip to account's account with no Site.

390x844 (mobile, touch) and 1440x900, a headless Chrome for Testing each, killed at the end. Per
screen: a screenshot as its way on goes live; held (entering to the way on live); fastest (to
leaving, the way on tapped the moment it is live, timed in the page); taps, fields, typed, scroll.
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import re
import socket
import statistics
import subprocess
import sys
import time
import urllib.parse
from pathlib import Path

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
from browser_path import chrome  # noqa: E402
from perf_audit import FAR_BROWSER_RTT, Tab, browser, farther  # noqa: E402
from wallflowers_path import PRODUCT, Fail, Stack  # noqa: E402

APEX, WWW = "https://wallflowers.io", "https://www.wallflowers.io"
VIEWPORTS = {"phone": (390, 844, True), "desk": (1440, 900, False)}
LEGS = os.environ.get("FUNNEL_LEGS", "live,stack").split(",")
REPEAT = int(os.environ.get("FUNNEL_REPEAT", "3"))
SITE = {"kind": "Community", "name": "Cambridge Digital Democracy", "pname": "Ada Lovelace"}
GONE = "document.getElementById('you').hidden && document.documentElement.getAttribute('data-state') === 'inside'"

# The way on is live: there, enabled, drawn, and the top thing at its centre (or a scroll away).
# A new document's is found by its address; its first frame live is kept as window.__live.
RULES = [
    (r"^https?://[^/]*wallflowers\.io/(site)?([?#].*)?$", "!document.documentElement.classList.contains('entering') && R('#signup')"),
    (r"^https?://www\.wallflowers\.io/signup", "/^(#step-1)?$/.test(location.hash) && ($('count') || {}).textContent === '1 of 3' && R('#next')"),
    (r"/signin\?(.*&)?new(&|=|$)", "typeof ($('new') || {}).onclick === 'function' && R('#new')"),
    (r"/signin", "typeof ($('in') || {}).onclick === 'function' && R('#in')"),
    (r"#register=", "!$('sheet').hidden && R('#sheetGo')"),
    (r"^https?://(app\.wallflowers\.io|localhost:\d+)/([?#].*)?$", "!$('you').hidden && R('#youGo')"),
]
WATCH = """(function () {
  var $ = function (id) { return document.getElementById(id); };
  var R = window.__ready = function (sel) {
    var e = document.querySelector(sel);
    if (!e || e.disabled || e.closest('[hidden]')) return false;
    var r = e.getBoundingClientRect(), cs = getComputedStyle(e), x = r.left + r.width / 2, y = r.top + r.height / 2;
    if (!r.width || !r.height || cs.visibility === 'hidden' || Number(cs.opacity) === 0) return false;
    if (y < 0 || y > innerHeight || x < 0 || x > innerWidth) return true;
    var h = document.elementFromPoint(x, y);
    return !!h && (h === e || e.contains(h));
  };
  var C = navigator.credentials;   // each passkey sheet the page asks for, counted
  if (C) ['create', 'get'].forEach(function (k) {
    var f = C[k].bind(C);
    C[k] = function (o) { (window.__sheets = window.__sheets || []).push(k); return f(o); };
  });
  var rules = [""" + ",".join(f"[{json.dumps(rx)}, function () {{ return {js}; }}]" for rx, js in RULES) + """], rule = null;
  for (var i = 0; i < rules.length && !rule; i++) if (new RegExp(rules[i][0]).test(location.href)) rule = rules[i][1];
  window.__live = null;
  if (rule) (function f() {
    var v = false;
    try { v = rule(); } catch (e) {}
    if (v) window.__live = Date.now(); else requestAnimationFrame(f);
  })();
})();"""
# What a screen offers: fields (a radio group is one), the other controls, how far it scrolls.
LOOK = """((way) => {
  const vis = e => { const t = /radio|file/.test(e.type) ? (e.closest('label') || e) : e, r = t.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && getComputedStyle(t).visibility !== 'hidden' && !t.closest('[hidden]'); };
  const fields = [], groups = new Set(), w = way && document.querySelector(way);
  for (const e of document.querySelectorAll('input, textarea, select')) {
    if (e.type === 'hidden' || !vis(e) || (e.type === 'radio' && groups.has(e.name))) continue;
    groups.add(e.name);
    fields.push(e.getAttribute('aria-label') || (e.labels && e.labels[0] && e.labels[0].textContent.trim()) || e.placeholder || e.name || e.type);
  }
  const label = e => (e.getAttribute('aria-label') || e.textContent || '').trim().replace(/\\s+/g, ' ').slice(0, 40);
  const controls = [...document.querySelectorAll('button, a[href], [role=button]')].filter(e => e !== w && vis(e) && !e.disabled).map(label).filter(Boolean);
  const se = document.scrollingElement || document.documentElement, r = w && w.getBoundingClientRect();
  return { fields, controls: controls.slice(0, 40), n_controls: controls.length, vh: innerHeight, url: location.href.slice(0, 160),
           scroll: Math.round(100 * Math.max(se.scrollHeight, document.body.scrollHeight) / innerHeight) / 100,
           way_bottom: r ? Math.round(r.bottom + se.scrollTop) : null, way_label: w ? label(w) : null };
})"""
NAV = """(() => { const n = performance.getEntriesByType('navigation')[0] || {}, rs = performance.getEntriesByType('resource');
  return {ttfb: Math.round(n.responseStart || 0), dcl: Math.round(n.domContentLoadedEventEnd || 0), bytes: n.transferSize || 0,
          resources: rs.length, res_bytes: rs.reduce((a, r) => a + (r.transferSize || 0), 0)}; })()"""


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


class Walk:
    """One browser at one viewport, and the screens it passes, in order."""

    def __init__(self, tab: Tab, view: str, leg: str, run: int) -> None:
        self.t, self.view, self.leg, self.run, self.visits = tab, view, leg, run, []
        self.pending = None   # the visit whose way on was tapped: left when the next is entered

    async def ev(self, expr: str):
        r = await self.t.cdp("Runtime.evaluate", expression=expr, awaitPromise=True, returnByValue=True, userGesture=True)
        if "exceptionDetails" in r:
            raise Fail(f"{self.view}: {expr[:70]}: {r['exceptionDetails'].get('exception', {}).get('description') or r['exceptionDetails'].get('text')}")
        return r["result"].get("value")

    async def arrive(self, rx: str, secs: float = 60) -> tuple[float, float]:
        """A new document at an address matching `rx`, its way on live: (navigation start, live)."""
        end, v = time.monotonic() + secs, None
        while time.monotonic() < end:
            try:
                v = await self.ev("({href: location.href, origin: performance.timeOrigin, live: window.__live})")
                if v and re.search(rx, v["href"]) and v["live"]:
                    return v["origin"], v["live"]
            except Fail:
                pass
            await asyncio.sleep(0.05)
        raise Fail(f"{self.view}: no live {rx} in {secs:.0f} s; last {v}; the page said {self.t.said[-3:]}")

    async def within(self, cond: str, secs: float = 60) -> float:
        """The first frame from now at which `cond` holds (epoch ms)."""
        at = await self.ev(f"""new Promise(ok => {{ const $ = id => document.getElementById(id), end = Date.now() + {int(secs * 1000)};
          (function f() {{ let v = false; try {{ v = !!({cond}); }} catch (e) {{}}
            if (v) return ok(Date.now()); if (Date.now() > end) return ok(null); requestAnimationFrame(f); }})(); }})""")
        if at is None:
            raise Fail(f"{self.view}: not within {secs:.0f} s: {cond[:90]}; at {await self.ev('location.href')}")
        return at

    async def screen(self, node: str, enter: float, live: float, way: str | None, taps=1, typed=0, required=0, **known) -> dict:
        """The screen as its way on goes live: its picture and what it offers."""
        if self.pending:
            self.pending["leave"], self.pending = enter, None
        shot = await self.t.cdp("Page.captureScreenshot", format="jpeg", quality=62)
        v = {"node": node, "view": self.view, "leg": self.leg, "run": self.run, "enter": enter, "live": live,
             "held": round(live - enter), "shot": shot["data"], "taps": taps, "typed": typed, "required": required,
             **await self.ev(f"({LOOK})({json.dumps(way)})"), "nav": await self.ev(NAV), **known}
        self.visits.append(v)
        say(f"{self.leg}/{self.view}: {node} held {v['held']} ms, scroll {v['scroll']}")
        return v

    async def tap(self, v: dict | None, sel: str, choice: str) -> float:
        """Tap `sel` now; its time on the page's clock, kept on the visit it leaves."""
        at = await self.ev(f"(() => {{ const e = document.querySelector({json.dumps(sel)}); if (!e) return null; const t = Date.now(); e.click(); return t; }})()")
        if at is None:
            raise Fail(f"{self.view}: no {sel} to tap at {await self.ev('location.href')}")
        if v is not None:
            v["tap"], v["choice"], self.pending = at, choice, v
        return at

    async def fill(self, sel: str, value: str) -> None:
        await self.ev(f"(() => {{ const e = document.querySelector({json.dumps(sel)}); e.focus(); e.value = {json.dumps(value)};"
                      " ['input', 'change'].forEach(k => e.dispatchEvent(new Event(k, {bubbles: true}))); return true; })()")

    async def step(self, node: str, sel: str, choice: str, cond: str, way: str, **kw) -> dict:
        """Tap `sel` on this document, and the next screen of it once `cond` and its way on hold."""
        t = await self.tap(self.visits[-1], sel, choice)
        return await self.screen(node, t, await self.within(f"({cond}) && __ready({json.dumps(way)})", kw.pop("secs", 60)), way, **kw)


def rtt(host: str, n: int = 5) -> dict:
    """TCP connect to :443 (ms): this host's distance, not a visitor's."""
    xs = []
    for _ in range(n):
        t = time.perf_counter()
        socket.create_connection((host, 443), timeout=10).close()
        xs.append((time.perf_counter() - t) * 1000)
    return {"min": round(min(xs)), "median": round(statistics.median(xs)), "max": round(max(xs))}


async def walk(work: Path, view: str, leg: str, run: int, out: dict, body, loopback: bool, rtt_ms: int = 0) -> None:
    """One browser through `body`; a stall is kept with the screen it followed, as a visitor meets it."""
    proc, tab = await browser(work, f"{leg}{run}-{view}", rtt=rtt_ms, loopback=loopback)
    w, (width, height, mobile) = Walk(tab, view, leg, run), VIEWPORTS[view]
    try:
        await tab.cdp("Emulation.setDeviceMetricsOverride", width=width, height=height, deviceScaleFactor=1, mobile=mobile)
        if mobile:
            await tab.cdp("Emulation.setTouchEmulationEnabled", enabled=True, maxTouchPoints=5)
        await tab.cdp("Page.addScriptToEvaluateOnNewDocument", source=WATCH)
        await body(w)
    except (Fail, TimeoutError) as e:
        after = w.visits[-1]["node"] if w.visits else None
        out.setdefault("stalls", []).append({"leg": leg, "view": view, "run": run, "after": after, "error": str(e) or type(e).__name__})
        say(f"{leg}/{view} run {run}: stalled after {after}: {str(e)[:120] or type(e).__name__}")
    finally:
        out.setdefault("visits", []).extend(w.visits)
        await tab.ws.close()
        proc.kill()
        proc.wait(10)


async def signup(w: Walk, out: dict, branches: bool) -> None:
    """Production, cold: the entrance to the Door's window; with `branches`, the rest warm."""
    await w.t.cdp("Network.setCacheDisabled", cacheDisabled=True)
    await w.t.cdp("Page.navigate", url=APEX + "/")
    home = await w.screen("home", *await w.arrive(r"^https://wallflowers\.io/$", 30), "#signup")
    await w.tap(home, "#signup", "Sign up")
    await w.screen("signup·site", *await w.arrive(r"/signup", 90), "#next", taps=3, typed=len(SITE["name"]), required=2)
    await w.ev(f"document.querySelector('input[name=kind][value={SITE['kind']}]').click(), true")
    await w.fill("#name", SITE["name"])
    await w.step("signup·face", "#next", "Continue", "location.hash === '#step-2' && $('site-face').children.length && document.fonts.status === 'loaded'", "#next")
    await w.step("signup·account", "#next", "Continue", "location.hash === '#step-3'", "#next", taps=2, typed=len(SITE["pname"]), required=1)
    await w.fill("#pname", SITE["pname"])
    await w.tap(w.visits[-1], "#next", "Create my account")
    win = await w.screen("window·new", *await w.arrive(r"^https://app\.wallflowers\.io/signin\?new", 30), "#new")
    out["draft"] = out.get("draft") or urllib.parse.parse_qs(urllib.parse.urlsplit(await w.ev("location.href")).query)["return"][0]
    win["perf"] = await w.ev("window.WallFlowersPerf ? WallFlowersPerf.summary(true) : null")
    w.pending = None
    if not branches:
        return
    await w.t.cdp("Network.setCacheDisabled", cacheDisabled=False)
    # /site, where every back-link lands: no entrance. The mark: its turn, then /signup.
    await w.t.cdp("Page.navigate", url=WWW + "/site")
    enter, at = await w.arrive(r"/site$", 30)
    site = await w.screen("site", enter, at, "#signup", warm=True)
    site["mark_live"] = round(await w.within("__ready('#mark-key')", 15) - enter)
    await w.tap(site, "#mark-key", "the mark")
    enter, at = await w.arrive(r"/signup", 90)
    site["mark_turn"] = round(enter - site["tap"])
    await w.screen("signup·site", enter, at, "#next", taps=3, required=2, warm=True, via="mark")
    # Skip to account: past Site and Face, to an account with no site.
    await w.step("signup·account", "#skip", "Skip to account", "location.hash === '#step-3'", "#next",
                 taps=2, typed=len(SITE["pname"]), required=1, warm=True, via="skip")
    await w.fill("#pname", SITE["pname"])
    await w.tap(w.visits[-1], "#next", "Create my account")
    await w.screen("window·new", *await w.arrive(r"signin\?new&return=(%2F|/)$", 30), "#new", warm=True, via="skip")
    # Sign in: www's /signin hands straight on to the Door.
    w.pending = None
    await w.t.cdp("Page.navigate", url=WWW + "/site")
    await w.tap(await w.screen("site", *await w.arrive(r"/site$", 30), "#signin", warm=True, via="again"), "#signin", "Sign in")
    await w.screen("window·signin", *await w.arrive(r"^https://app\.wallflowers\.io/signin\?return=", 30), "#in", warm=True)
    w.pending = None


def own(draft: str, tag: str) -> str:
    """The draft with an address of its own: every pass registers on one Stack, and an address
    already held is refused at Register (409)."""
    head, b = draft.split("#register=", 1)
    d = json.loads(base64.urlsafe_b64decode(b + "=" * (-len(b) % 4)))
    d["slug"] = f"{d['slug'][:20]}-{tag}"[:32].strip("-")
    return head + "#register=" + base64.urlsafe_b64encode(json.dumps(d, separators=(",", ":")).encode()).rstrip(b"=").decode()


async def account(w: Walk, D: str, draft: str) -> None:
    """The window with the draft, to the Site's feed."""
    await w.t.cdp("Page.navigate", url=f"{D}/signin?new&return=" + urllib.parse.quote(own(draft, f"{w.leg}-{w.view}"), safe=""))
    await w.screen("window·new", *await w.arrive(r"/signin\?new", 30), "#new")
    # One tap: the account's start, then the passkey sheets at once (create, then get for PRF).
    words = await w.step("window·words", "#new", "Create an account", "!$('shown').hidden", "#done", secs=120)
    words.update(words=await w.ev("document.querySelectorAll('#list li').length"), sheets=await w.ev("(window.__sheets || []).join(' ')"),
                 perf=await w.ev("WallFlowersPerf.summary(true)"),   # and the start, still offered beside the words:
                 start_live=await w.ev("!document.getElementById('start').hidden && !document.getElementById('new').disabled"))
    await w.tap(words, "#done", "Continue")
    reg = await w.screen("webapp·register", *await w.arrive(r"#register=", 60), "#sheetGo")
    reg["perf"] = await w.ev("WallFlowersPerf.summary(true)")
    card = await w.step("webapp·card", "#sheetGo", "Register", "!$('you').hidden", "#youGo", secs=120, required=1)
    card.update(prefilled=await w.ev("(document.getElementById('youName') || {}).value || ''"), perf=await w.ev("WallFlowersPerf.summary(true)"))
    t = await w.tap(card, "#youGo", "Save")
    feed = await w.screen("webapp·feed", t, await w.within(GONE), None, taps=0)
    await asyncio.sleep(1.5)
    feed.update(perf=await w.ev("WallFlowersPerf.summary(true)"),
                sites=await w.ev("fetch('/v2/graph').then(r => r.json()).then(g => g.objects.filter(o => o.kind === 'group' && o.view"
                                 " && o.view.shape !== 'individual').map(o => o.view.title || o.view.display_name || o.name))"))
    w.pending = None


async def no_site(w: Walk, D: str) -> None:
    """Skip to account's window: the first sheet dismissed, Create the passkey, the words, the card."""
    await w.t.cdp("Page.navigate", url=f"{D}/signin?new&return=%2F")
    await w.screen("window·new", *await w.arrive(r"/signin\?new", 30), "#new", via="skip")
    await w.ev("(() => { const c = navigator.credentials.create.bind(navigator.credentials); let once = true;"
               " navigator.credentials.create = o => once ? (once = false, Promise.reject(new DOMException('dismissed', 'NotAllowedError'))) : c(o);"
               " return true; })()")
    await w.step("window·passkey", "#new", "Create an account, sheet dismissed", "!$('words').hidden", "#saved", via="dismissed")
    words = await w.step("window·words", "#saved", "Create the passkey", "!$('shown').hidden", "#done", secs=120, via="dismissed")
    await w.tap(words, "#done", "Continue")
    card = await w.screen("webapp·card", *await w.arrive(r"^http://localhost:\d+/$", 60), "#youGo", taps=2,
                          typed=len(SITE["pname"]), required=1, via="skip")
    await w.fill("#youName", SITE["pname"])
    t = await w.tap(card, "#youGo", "Save")
    home = await w.screen("webapp·home", t, await w.within(GONE), None, taps=0, via="skip")
    home["perf"] = await w.ev("WallFlowersPerf.summary(true)")
    w.pending = None


def main() -> int:
    if sys.argv[1:2] == ["--html"]:
        from signup_funnel_page import page
        ev = json.loads(Path(sys.argv[4]).read_text()) if len(sys.argv) > 4 else None
        Path(sys.argv[3]).write_text(page(json.loads(Path(sys.argv[2]).read_text()), ev))
        return 0
    dest = Path(sys.argv[1] if len(sys.argv) > 1 else "signup-funnel.json")
    out: dict = {"at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "legs": LEGS,
                 "commit": subprocess.run(["git", "-C", str(PRODUCT), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip(),
                 "chrome": Path(chrome()).parts[-5] if len(Path(chrome()).parts) > 5 else chrome(), "draft": os.environ.get("FUNNEL_DRAFT", "")}
    work = Path(os.environ.get("TMPDIR", "/tmp")) / f"wf-funnel.{os.getpid()}"
    work.mkdir(parents=True, exist_ok=True)

    async def live() -> None:
        for view in VIEWPORTS:
            for run in range(REPEAT):
                await walk(work, view, "live", run, out, lambda w: signup(w, out, run == 0), loopback=False)

    async def stack(s: Stack, leg: str, rtt_ms: int) -> None:
        for view in VIEWPORTS:
            await walk(work, view, leg, 0, out, lambda w: account(w, s.door_url, out["draft"]), True, rtt_ms)
            await walk(work, view, leg, 1, out, lambda w: no_site(w, s.door_url), True, rtt_ms)
    try:
        if "live" in LEGS:
            out["rtt"] = {h: rtt(h) for h in ("www.wallflowers.io", "app.wallflowers.io")}
            out["www_version"] = httpx.get(WWW + "/version.json", timeout=20).json()
            asyncio.run(live())
        if "stack" in LEGS:
            if not out.get("draft"):
                raise Fail("no draft: run the live leg first, or set FUNNEL_DRAFT to a return= from the window")
            s = Stack(host="localhost")
            try:
                s.build()
                s.up()
                asyncio.run(stack(s, "stack", 0))
                hops = farther(s)
                try:
                    asyncio.run(stack(s, "far", FAR_BROWSER_RTT))
                finally:
                    for h in hops.values():
                        h.close()
            finally:
                s.down()
    except (Fail, TimeoutError, OSError) as e:
        out["failed"] = str(e) or type(e).__name__
        say("FAILED:", out["failed"])
    out["draft_chars"] = len(out.get("draft", ""))
    dest.write_text(json.dumps(out, indent=1))
    say(f"{len(out.get('visits', []))} screens to {dest}")
    return 1 if out.get("failed") else 0


if __name__ == "__main__":
    sys.exit(main())
