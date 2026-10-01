"""The one-page join's legs that stay while its window is reworked, witnessed on door-test (EGREGORE's
door/one-page-join and site/one-page-join; SCM, 30 Sep). A kiosk-format claim for the rehearsal Site:

  L1  /join?claim= answers 303 to the Site's registered home with the claim in the fragment: no-store,
      no-referrer, no cookie set, no page
  L2  /v2/join by the Site's own session (a DPoP token for egregore-local): the claim in the body, admitted, no
      room unjoined; a claim for another Site refused, 403
  L3  the site, in a headless Chrome pinned to door-test, while the claim is in its address (BUILD, 30 Sep): no script
      but the site's own, the strip a replaceState of the same entry, nothing copying it into a request or a log (its
      going against first paint kept, not judged); the claim held in sessionStorage only; the member signed in through the Door's window (used, not judged: it is
      being rewritten) and back, the site's own POST /v2/join (DPoP), admitted to the Site and a room; the egg

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  JA_SITE=http://127.0.0.1:3471 JA_HOME=http://127.0.0.1:3471/community JA_SITE_ID=<the Site> JA_KIOSK=<egregore>
  .venv/bin/python app/e2e/one_page_join.py
"""
from __future__ import annotations

import asyncio
import json
import os
import re
import sys
import time
import urllib.parse
from pathlib import Path

import httpx
import websockets

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import journeys as J  # noqa: E402
import wallflowers_path as w  # noqa: E402

SITE = os.environ.get("JA_SITE", "http://127.0.0.1:3471").rstrip("/")
HOME = os.environ.get("JA_HOME", SITE + "/community")
SITE_ID = os.environ.get("JA_SITE_ID", "104b4824b32806cc31838d696f9de87c37a86fae52a94b83408c70a5ed982b8d")
TAG = time.strftime("%m%d%H%M", time.gmtime())
LEGS = os.environ.get("ONE_PAGE_LEGS", "L")                   # L: the lasting legs; V: the reworked window, virtual
say = J.say
# The egg's language choice, or its room list (ja_rehearsal's markers).
EGG = ("(document.querySelector('dialog[open] section[lang]')?.getAttribute('aria-label') || '').includes('/forums') || "
       "[...document.querySelectorAll('dialog[open] [role=menu] button')].some(x => /ENGLISH/i.test(x.textContent))")
# On every document: the Door's recovery words, if they ever show; the site's address at its first render.
WATCH = r"""(() => {
  const said = new Set(), tell = (m) => { if (!said.has(m)) { said.add(m); console.log(m); } };
  const words = () => {
    const l = document.querySelectorAll('#list li').length, s = document.getElementById('shown');
    if (l > 0 || (s && !s.hidden)) tell('wf-words ' + location.pathname + ' ' + l);
  };
  new MutationObserver(words).observe(document, {subtree: true, childList: true, attributes: true});
  const CLAIM = /v1\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}/, store = () => {
    const ls = Object.keys(localStorage).filter(k => CLAIM.test(localStorage.getItem(k) || '')).length;
    return 'ss=' + CLAIM.test(sessionStorage.getItem('egg.join') || '') + ' ls=' + ls + ' ck=' + CLAIM.test(document.cookie);
  };
  const HELD = () => location.hash.startsWith('#join=');
  if (HELD()) {
    const len0 = history.length;
    for (const k of ['pushState', 'replaceState']) {
      const f = history[k].bind(history);
      history[k] = (s, t, u) => {
        if (HELD()) tell('wf-hist ' + k + ' ' + (u == null ? 'same' : String(u).includes('#join=') ? 'join' : 'clean') + ' len=' + history.length);
        return f(s, t, u);
      };
    }
    const third = () => new Set([...[...document.scripts].map(s => s.src).filter(Boolean),
      ...performance.getEntriesByType('resource').filter(e => e.initiatorType === 'script').map(e => e.name)]
      .filter(u => new URL(u, location.href).origin !== location.origin)).size;
    const leaked = () => performance.getEntriesByType('resource').filter(e => /join=v1|%23join/.test(e.name)).length;
    document.addEventListener('DOMContentLoaded', () => { if (HELD()) tell('wf-dcl-held ' + location.origin + ' 3p=' + third() + ' len=' + history.length); });
    const gone = setInterval(() => {
      if (HELD()) return;
      clearInterval(gone);
      tell('wf-hash-gone ' + location.origin + ' ' + Math.round(performance.now()));
      tell('wf-strip ' + location.origin + ' 3p=' + third() + ' len0=' + len0 + ' len=' + history.length + ' leaked=' + leaked());
    }, 2);
  }
  try {
    new PerformanceObserver((l) => { for (const e of l.getEntries()) tell('wf-' + e.name + ' ' + location.origin + ' ' + Math.round(e.startTime)); })
      .observe({type: 'paint', buffered: true});
  } catch (e) { /* no paint timing */ }
  if (location.pathname.startsWith('/signin')) {
    let n = 0;
    const c = navigator.credentials, cr = c.create.bind(c), gt = c.get.bind(c);
    c.create = (o) => {
      tell('wf-cred create ' + (++n) + ' prf-eval=' + !!(o && o.publicKey && o.publicKey.extensions && o.publicKey.extensions.prf));
      return cr(o).then((r) => { const x = (r.getClientExtensionResults() || {}).prf || {};
        const g = r.response.getAuthenticatorData.bind(r.response), d = new Uint8Array(g());
        const hx = (b) => [...b].map((v) => v.toString(16).padStart(2, '0')).join('');
        if (window.__asAaguid && d.length >= 53) {
          // The witnessed branch's logic only: the page is shown this AAGUID; the Door's attestation is untouched.
          const as = window.__asAaguid.match(/../g).map((h) => parseInt(h, 16));
          r.response.getAuthenticatorData = () => { const c = new Uint8Array(g()); c.set(as, 37); return c.buffer; };
        }
        tell('wf-cred created ' + n + ' results=' + !!(x.results && x.results.first) + ' enabled=' + !!x.enabled + ' aaguid=' + hx(d.subarray(37, 53))
          + (window.__asAaguid ? ' shown=' + window.__asAaguid : '')); return r; });
    };
    c.get = (o) => {
      const m = (o && o.mediation) || 'required', k = ++n;
      tell('wf-cred get ' + k + ' ' + m);
      return gt(o).then((r) => { tell('wf-cred got ' + k + ' ' + m); return r; }, (e) => { tell('wf-cred refused ' + k + ' ' + m + ' ' + (e && e.name)); throw e; });
    };
    const f = window.fetch.bind(window);
    window.fetch = (...a) => f(...a).then((r) => {
      const u = String((a[0] && a[0].url) || a[0]), m = /\/v2\/(signup|signup\/finish|signin\/finish|signup\/continue)$/.exec(u);
      if (m) r.clone().json().then((j) => { if (j && j.pk) tell('wf-pk ' + m[1] + ' ' + String(j.pk).replace('ed25519:', '').slice(0, 16)); }, () => {});
      return r;
    });
    const checks = () => Object.keys(localStorage).filter((k) => k.startsWith('wallflowers.prf-check.')).length;
    let said = 0;
    new MutationObserver(() => { const s = document.getElementById('status'), x = s && s.textContent.trim();
      if (x) tell('wf-said ' + (++said) + ' ' + x.slice(0, 90)); }).observe(document, {subtree: true, childList: true, characterData: true});
    tell('wf-checks-at-open ' + checks());
    window.addEventListener('pagehide', () => tell('wf-checks-at-leave ' + checks()));
    document.addEventListener('DOMContentLoaded', () => {
      const $ = (id) => document.getElementById(id), shown = (e) => !!e && !e.hidden && e.offsetParent !== null;
      const wf = [...document.querySelectorAll('img, footer')].filter((e) => shown(e) && (e.matches('footer.wf') || /wallflowers\.png|wordmark/i.test(e.src || ''))).length;
      tell('wf-window ' + JSON.stringify({site: document.documentElement.hasAttribute('data-site'), face: document.documentElement.hasAttribute('data-face'),
        mark: document.querySelectorAll('img.site-mark').length, wfmark: wf, in: ($('in') || {}).textContent, new: !$('new') || $('new').hidden,
        name: shown($('name')), webauthn: /webauthn/.test(($('name') || {}).autocomplete || ''), known: localStorage.getItem('wallflowers.known') === '1'}));
    });
  }
  const poll = setInterval(() => { if (sessionStorage.getItem('egg.join')) { clearInterval(poll); tell('wf-store ' + location.origin + ' ' + store()); } }, 25);
})();"""


def site_watch() -> str:
    """On the site's documents: a CSP violation, the rosette's status line, the egg's hand-on, and any N% bar before the egg opens."""
    return r"""(() => {
  if (location.origin !== %s) return;
  const said = new Set(), tell = (m) => { if (!said.has(m)) { said.add(m); console.log(m); } };
  document.addEventListener('securitypolicyviolation', (e) => tell('wf-csp ' + location.pathname + ' ' + e.violatedDirective + ' ' + (e.blockedURI || '').slice(0, 30)));
  try { tell('wf-handon ' + location.pathname + ' ' + !!sessionStorage.getItem('egg.rosette')); } catch (e) {}
  let open = false;
  const seen = () => {
    if (open) return;
    if (document.querySelector('dialog[open]')) { open = true; return; }
    const st = [...document.querySelectorAll('[role=status]')].map((x) => x.textContent.trim()).filter(Boolean);
    if (st.length) tell('wf-status ' + location.pathname + ' ' + JSON.stringify(st.slice(0, 2)));
    const t = (document.body && document.body.innerText) || '', m = t.match(/\b\d{1,3}\s?%%/);
    if (m) tell('wf-bar ' + location.pathname + ' ' + m[0]);
    if (/WallFlowers/.test(t)) tell('wf-wfword ' + location.pathname);
  };
  new MutationObserver(seen).observe(document, {subtree: true, childList: true, characterData: true});
})();""" % json.dumps(SITE)


def CLAIMED(s: str) -> bool:
    """A claim, or the address's #join= carrying one, in `s` (encoded or not)."""
    return bool(re.search(r"v1\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}|join%3Dv1|%23join", s or ""))


class Tap:
    """A second DevTools session on the page's target: its network and its console, kept."""

    def __init__(self, b: J.Browser) -> None:
        self.b, self.events, self.logs, self.bodies, self.joins, self.claim_logs = b, [], [], {}, set(), []

    async def open(self) -> "Tap":
        port = int((self.b.prof / "DevToolsActivePort").read_text().split()[0])
        t = next(t for t in httpx.get(f"http://127.0.0.1:{port}/json/list", timeout=5).json() if t["type"] == "page")
        self.ws = await websockets.connect(t["webSocketDebuggerUrl"], max_size=None)
        self.n, self.waiting = 0, {}
        self.task = asyncio.create_task(self._read())
        await self.cdp("Network.enable")
        await self.cdp("Runtime.enable")
        return self

    async def _read(self) -> None:
        async for raw in self.ws:
            d = json.loads(raw)
            if "id" in d and d["id"] in self.waiting:
                self.waiting.pop(d["id"]).set_result(d)
            elif d.get("method") == "Runtime.consoleAPICalled":
                a = d["params"].get("args") or [{}]
                if str(a[0].get("value", "")).startswith("wf-"):
                    self.logs.append(a[0]["value"])
                elif any(CLAIMED(json.dumps(x)) for x in a):
                    self.claim_logs.append(str(a[0].get("value", a[0].get("description", "")))[:80])
            elif d.get("method", "").startswith("Network."):
                self.events.append(d)
                pr = d["params"]
                if d["method"] == "Network.requestWillBeSent" and pr["request"]["url"].endswith("/v2/join") and pr["request"]["method"] == "POST":
                    self.joins.add(pr["requestId"])
                elif d["method"] == "Network.loadingFinished" and pr["requestId"] in self.joins:
                    # At once: the site leaves the page straight after its join, and the body with it.
                    asyncio.create_task(self._body(pr["requestId"]))

    async def _body(self, rid: str) -> None:
        try:
            self.bodies[rid] = json.loads((await self.cdp("Network.getResponseBody", requestId=rid)).get("body") or "{}")
        except Exception as e:
            self.bodies[rid] = {"unread": str(e)[:80]}

    async def cdp(self, method: str, **params):
        self.n += 1
        fut = asyncio.get_running_loop().create_future()
        self.waiting[self.n] = fut
        await self.ws.send(json.dumps({"id": self.n, "method": method, "params": params}))
        return (await asyncio.wait_for(fut, 30)).get("result", {})

    def mark(self) -> int:
        return len(self.events)

    async def read(self, since: int, D: str) -> dict:
        """From event `since` on: the /join redirect, the Door documents, the site's /v2/join and its answer."""
        ev = self.events[since:]
        sent = [e["params"] for e in ev if e["method"] == "Network.requestWillBeSent"]
        hop = next((p["redirectResponse"] for p in sent if (p.get("redirectResponse") or {}).get("url", "").startswith(D + "/join?")), None)
        docs = [p["request"]["url"].split("?")[0] for p in sent if p.get("type") == "Document" and p["request"]["url"].startswith(D)
                and not p["request"]["url"].startswith(D + "/join?")]
        joins = [p for p in sent if p["request"]["url"] == D + "/v2/join" and p["request"]["method"] == "POST"]
        # Every request carrying a claim, but the scan itself (the Door's /join?claim=) and the site's join (its body).
        carriers = [f'{p["request"]["method"]} {p["request"]["url"][:60]}' for p in sent
                    if not p["request"]["url"].startswith(D + "/join?") and not (p["request"]["url"] == D + "/v2/join" and p["request"]["method"] == "POST")
                    and (CLAIMED(p["request"]["url"]) or CLAIMED(p["request"].get("postData") or "")
                         or CLAIMED(json.dumps({k: v for k, v in p["request"].get("headers", {}).items() if k.lower() != "cookie"})))]
        out = {"redirect": None, "door_documents": docs, "join": None, "carriers": carriers}
        if hop:
            loc = {k.lower(): v for k, v in hop.get("headers", {}).items()}.get("location", "")
            out["redirect"] = {"status": hop.get("status"), "to": loc.split("#")[0], "fragment": loc.split("#", 1)[1][:6] if "#" in loc else ""}
        if joins:
            p = joins[-1]
            h = {k.lower(): v for k, v in p["request"].get("headers", {}).items()}
            got = next((e["params"]["response"] for e in ev if e["method"] == "Network.responseReceived"
                        and e["params"]["requestId"] == p["requestId"]), {})
            for _ in range(20):
                if p["requestId"] in self.bodies:
                    break
                await asyncio.sleep(0.25)
            body = self.bodies.get(p["requestId"], {})
            out["join"] = {"count": len(joins), "status": got.get("status"), "dpop": h.get("authorization", "").startswith("DPoP ") and bool(h.get("dpop")),
                           "from": h.get("origin") or p.get("documentURL", "")[:40], "claim_in_body": "claim" in (p["request"].get("postData") or ""),
                           "admitted": body.get("admitted"), "rooms": len(body.get("rooms") or []), "unjoined": body.get("unjoined"),
                           "body_read": body.get("unread") or ("ok" if body else "empty")}
        return out

    def close(self) -> None:
        self.task.cancel()


async def journey(b: J.Browser, tap: Tap, D: str, choice: str, name: str | None) -> dict:
    """A claim scanned; the one Door page (a name and a passkey, or the passkey alone); back to the egg."""
    since, logs0 = tap.mark(), len(tap.logs)
    claim = J.claim(SITE_ID, choice, None)
    await b.p.cdp("Page.navigate", url=D + "/join?claim=" + urllib.parse.quote(claim, safe=""))
    await b.p.until(f"location.origin === {json.dumps(D)} && location.pathname.startsWith('/signin') && "
                    "!!document.getElementById('in') && !document.getElementById('in').disabled", "the one Door page", 60)
    page = await b.p.ev("({name: !!document.getElementById('name') && !document.getElementById('name').hidden, "
                        "href: location.pathname + location.search.slice(0, 60)})")
    await b.p.shot(b.prof.parent, f"W-{choice}-door")
    if name is not None:
        await b.p.ev("(() => { const i = document.getElementById('name'); i.value = " + json.dumps(name) + "; "
                     "i.dispatchEvent(new Event('input', {bubbles: true})); })()")
        await b.p.click("in" if await b.p.ev("document.getElementById('new').hidden") else "new")
    else:
        await b.p.click("in")
    t0 = time.monotonic()
    while time.monotonic() - t0 < 120:
        if await b.p.ev(f"location.origin === {json.dumps(SITE)} && ({EGG})"):
            break
        # Where one tap was not ready, the passkey's own step and its button (journeys' sign_up does the same).
        if await b.p.ev(f"location.origin === {json.dumps(D)} && !!document.getElementById('saved') && "
                        "!document.getElementById('words').hidden"):
            await b.p.click("saved")
        await asyncio.sleep(0.5)
    else:
        raise w.Fail(f"not in the egg after 120 s: at {await b.p.ev('location.href')}")
    await asyncio.sleep(1)
    await b.p.shot(b.prof.parent, f"W-{choice}-egg")
    got = await tap.read(since, D)
    # The member's rooms as the egg shows them, through the site's own session: the language, then FORUMS (ja_rehearsal's way).
    dial = "[...document.querySelectorAll('dialog[open] [role=menu] button')]"
    rows = "[...document.querySelectorAll('dialog[open] section[lang] [data-row], section[lang] ul li button')]"
    try:
        if await b.p.ev(f"{dial}.some(x => /ENGLISH/i.test(x.textContent))"):
            await b.p.ev(f"{dial}.find(x => /ENGLISH/i.test(x.textContent)).click()")
            await b.p.until(f"{dial}.some(x => x.textContent === 'FORUMS')", "the dial", 30)
            await b.p.ev(f"{dial}.find(x => x.textContent === 'FORUMS').click()")
        await b.p.until(f"{rows}.length > 0", "the rooms", 45)
        got["rooms_shown"] = await b.p.ev(f"{rows}.map(x => x.textContent.trim().slice(0, 40))")
    except w.Fail as e:
        got["rooms_shown"] = [f"none: {str(e)[:120]}"]
    await b.p.shot(b.prof.parent, f"W-{choice}-rooms")
    got["page"], got["logs"] = page, tap.logs[logs0:]
    got["landed"] = await b.p.ev("location.origin + location.pathname + location.hash")
    return got


OTHER_SITE = "36625ad10052faeeb0549346048f98b44d484ee3835b5e19abba4351700ea63a"   # door-test's stand-in Site of egregores-echoes.com


def site_token(s: w.Stack, who: dict) -> tuple[str, object]:
    """`who`'s token for egregore-local, as the site holds one after its sign-in (PKCE, DPoP)."""
    import hashlib
    from urllib.parse import parse_qs, urlsplit
    client, callback = "egregore-local", SITE + "/signin/callback"
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


async def run(s: w.Stack, results: list) -> None:
    D = s.door_url

    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail, ensure_ascii=False)[:1200]))
        say(sid, "G" if ok else "NG", json.dumps(detail, ensure_ascii=False)[:1200])

    # L1: the scanned claim's hop, as any client sees it.
    claim = J.claim(SITE_ID, "financial", None)
    r = httpx.get(f"{D}/join", params={"claim": claim}, follow_redirects=False, verify=s.verify, timeout=30)
    h = {k.lower(): v for k, v in r.headers.items()}
    loc = h.get("location", "")
    checks = {"303": r.status_code == 303, "to <home>#join=<the claim>": loc == f"{HOME}#join={claim}",
              "no-store": "no-store" in h.get("cache-control", ""), "no-referrer": h.get("referrer-policy") == "no-referrer",
              "no cookie set": "set-cookie" not in h, "no page": len(r.content) == 0}
    result("L1", all(checks.values()), "/join?claim= answers 303 to the Site's home, the claim in the fragment, and nothing else",
           failed=[k for k, v in checks.items() if not v], status=r.status_code, to=loc.split("#")[0], fragment=loc.split("#", 1)[1][:8] if "#" in loc else "",
           cache_control=h.get("cache-control"), referrer_policy=h.get("referrer-policy"), body_bytes=len(r.content))

    # L2: the Site's own session joins; another Site's claim is refused.
    who = w.signup(s, f"L2 {TAG}")
    tok, k = site_token(s, who)

    def join(c: str) -> httpx.Response:
        return httpx.post(f"{D}/v2/join", json={"claim": c}, verify=s.verify, timeout=60,
                          headers={"authorization": f"DPoP {tok}", "dpop": w.dpop(k, "POST", f"{D}/v2/join", tok)})
    mine = join(J.claim(SITE_ID, "financial", None))
    body = mine.json() if mine.headers.get("content-type", "").startswith("application/json") else {}
    other = join(J.claim(OTHER_SITE, "financial", None))
    httpx.post(f"{D}/v2/signout", verify=s.verify, timeout=30,
               headers={"authorization": f"DPoP {tok}", "dpop": w.dpop(k, "POST", f"{D}/v2/signout", tok)})
    who["client"].post("/v2/signout")
    checks = {"admitted": mine.status_code == 200 and body.get("admitted") is True, "a room": len(body.get("rooms") or []) >= 1,
              "none unjoined": body.get("unjoined") == [], "the Site's": body.get("site") == SITE_ID,
              "another Site's claim 403": other.status_code == 403 and "another Site" in other.text}
    result("L2", all(checks.values()), "/v2/join by the Site's own DPoP session: admitted; a claim for another Site refused",
           failed=[k for k, v in checks.items() if not v], join=mine.status_code, rooms=len(body.get("rooms") or []),
           unjoined=body.get("unjoined"), other=[other.status_code, other.text[:120]])

    # L3: the site's half, in the browser: the Door's pin, and the site on the loopback address (ja_rehearsal's rule).
    base = bp.pinned
    bp.pinned = lambda to="": [x.replace("EXCLUDE localhost", "EXCLUDE localhost, EXCLUDE 127.0.0.1") if x.startswith("--host-resolver-rules")
                               else x for x in base(to)]
    try:
        b = await J.Browser().open(s.work, "one-page")
    finally:
        bp.pinned = base
    tap = None
    try:
        await b.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=WATCH)
        await b.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=site_watch())
        tap = await Tap(b).open()
        g = await journey(b, tap, D, "financial", f"Witness {TAG}")
        j = g["join"] or {}
        held = [l for l in g["logs"] if l.startswith(("wf-dcl-held " + SITE, "wf-strip " + SITE, "wf-hist "))]
        strip = next((dict(kv.split("=") for kv in l.split()[2:]) for l in g["logs"] if l.startswith("wf-strip " + SITE)), {})
        dcl = next((dict(kv.split("=") for kv in l.split()[2:]) for l in g["logs"] if l.startswith("wf-dcl-held " + SITE)), {})
        hist = [l for l in g["logs"] if l.startswith("wf-hist ")]
        num = lambda pre: next((int(l.rsplit(" ", 1)[1]) for l in g["logs"] if l.startswith(pre + " " + SITE)), None)
        timing = {"hash_gone_ms": num("wf-hash-gone"), "first_paint_ms": num("wf-first-paint"), "first_contentful_paint_ms": num("wf-first-contentful-paint")}
        stores = [l for l in g["logs"] if l.startswith("wf-store " + SITE)]
        checks = {
            "303 to the home": (g["redirect"] or {}).get("status") == 303 and (g["redirect"] or {}).get("to") == HOME,
            # BUILD's L3 (30 Sep): (a) while the claim is in the address no script but the site's own; (b) the strip a
            # replaceState of the same entry; (c) nothing copies it into a request or a log first.
            "(a) no third-party script while the claim is in the address": bool(strip) and strip.get("3p") == "0" and dcl.get("3p", "0") == "0",
            "(b) the strip a replaceState of the same entry": bool(strip) and strip.get("len") == strip.get("len0")
            and all(h.split()[1] == "replaceState" for h in hist) and any(h.split()[2] == "clean" for h in hist),
            "(c) no request or log carries the claim": bool(strip) and strip.get("leaked") == "0" and not g["carriers"] and not tap.claim_logs,
            "the claim in sessionStorage only": bool(stores) and stores[0].endswith("ss=true ls=0 ck=false"),
            "the site's own POST /v2/join, DPoP, the claim in the body": j.get("count") == 1 and j.get("dpop") and j.get("claim_in_body")
            and j.get("from", "").startswith(SITE),
            "admitted (200), and the egg shows the claim's room": j.get("status") == 200
            and any("financial" in r.lower() for r in g.get("rooms_shown") or []),
            "in the egg": g["landed"].startswith(SITE + "/"),
            # The rosette (egregore 5a9630d; SCM, 30 Sep): the join from the callback, then the egg, which takes the hand-on.
            "the join from /signin/callback": j.get("from", "").startswith(SITE + "/signin/callback"),
            "no CSP violation on the site": not [l for l in g["logs"] if l.startswith("wf-csp")],
            "the rosette's status on /community and the callback": any(l.startswith("wf-status /community ") for l in g["logs"])
            and any(l.startswith("wf-status /signin/callback ") for l in g["logs"]),
            "no WallFlowers words on the site's way in": not [l for l in g["logs"] if l.startswith("wf-wfword")],
            "the egg takes the rosette: no bar": "wf-handon / true" in g["logs"] and not [l for l in g["logs"] if l.startswith("wf-bar /")],
        }
        window = {"door_documents": g["door_documents"], "name_asked": g["page"]["name"],
                  "words_seen": [l for l in g["logs"] if l.startswith("wf-words")]}
        result("L3", all(checks.values()), "the site: the claim off the address and in this tab only; its own join; the Site, a room, the egg",
               failed=[k for k, v in checks.items() if not v], held=held,
               site_lines=[l for l in g["logs"] if l.startswith(("wf-csp", "wf-status", "wf-handon", "wf-bar", "wf-wfword"))], carriers=g["carriers"], claim_logs=tap.claim_logs[:3],
               timing=timing, store=stores[:1], join=j, landed=g["landed"],
               rooms_shown=g.get("rooms_shown"),
               window_used_not_judged=window)
    finally:
        try:
            await b.p.cdp("Page.navigate", url=D + "/v2/icd")
            await asyncio.sleep(1)
            await b.p.ev("fetch('/v2/signout', {method: 'POST'}).then(r => r.status, () => 0)")
        except Exception:
            pass
        if tap is not None:
            tap.close()
        b.close()
        say("the Chrome closed")


async def arrive(b: J.Browser, D: str, url: str, name: str | None, secs: float = 120) -> str:
    """To `url`, then wherever the window asks a name: `name` and SIGN IN, or stop there if None. 'egg' or 'named'."""
    await b.p.cdp("Page.navigate", url=url)
    t0 = time.monotonic()
    while time.monotonic() - t0 < secs:
        if await b.p.ev(f"location.origin === {json.dumps(SITE)} && ({EGG})"):
            return "egg"
        asks = await b.p.ev(f"location.origin === {json.dumps(D)} && !!document.getElementById('name') && !document.getElementById('name').hidden "
                            "&& !document.getElementById('in').disabled")
        if asks:
            if name is None:
                return "named"
            await b.p.shot(b.prof.parent, "V-door")
            await b.p.ev("(() => { const i = document.getElementById('name'); i.value = " + json.dumps(name) + "; "
                         "i.dispatchEvent(new Event('input', {bubbles: true})); })()")
            await b.p.click("in" if await b.p.ev("document.getElementById('new').hidden") else "new")
            name = None
            await asyncio.sleep(1)
        await asyncio.sleep(0.3)
    raise w.Fail(f"neither the egg nor a name asked in {secs:g} s: at {await b.p.ev('location.href')}")


async def pinned_browser(s: w.Stack, tag: str) -> J.Browser:
    base = bp.pinned
    bp.pinned = lambda to="": [x.replace("EXCLUDE localhost", "EXCLUDE localhost, EXCLUDE 127.0.0.1") if x.startswith("--host-resolver-rules")
                               else x for x in base(to)]
    try:
        b = await J.Browser().open(s.work, tag)
    finally:
        bp.pinned = base
    await b.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=WATCH)
    await b.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=site_watch())
    return b


async def window_run(s: w.Stack, results: list) -> None:
    """The reworked window, in Chrome's virtual PRF authenticator (virtual: not any real platform's PRF)."""
    D = s.door_url

    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail, ensure_ascii=False)[:1400]))
        say(sid, "G" if ok else "NG", json.dumps(detail, ensure_ascii=False)[:1400])

    def lines(logs, pre):
        return [l for l in logs if l.startswith(pre)]

    opened = []
    try:
        b = await pinned_browser(s, "window")
        opened.append(b)
        tap = await Tap(b).open()
        # V1: a device new here, a new member, by a scanned claim.
        since, l0 = tap.mark(), len(tap.logs)
        how = await arrive(b, D, D + "/join?claim=" + urllib.parse.quote(J.claim(SITE_ID, "financial", None), safe=""), f"Witness {TAG}")
        logs = tap.logs[l0:]
        g = await tap.read(since, D)
        win = next((json.loads(l.split(" ", 1)[1]) for l in lines(logs, "wf-window")), {})
        cred, pk1 = lines(logs, "wf-cred"), [l.split()[2] for l in lines(logs, "wf-pk")]
        made = next((l for l in cred if l.startswith("wf-cred created")), "")
        gets_after = [l for l in cred if l.startswith("wf-cred get") and "conditional" not in l]
        checks = {
            "in the egg": how == "egg", "one Door page": len(g["door_documents"]) == 1,
            "the Site's Face": win.get("site") is True and win.get("face") is True and win.get("mark", 0) > 0,
            "no WallFlowers mark": win.get("wfmark") == 0, "one SIGN IN": win.get("in") == "SIGN IN" and win.get("new") is True,
            "the name asked, passkeys offered in it": win.get("name") is True and win.get("webauthn") is True,
            "one create, with prf.eval": sum(1 for l in cred if l.startswith("wf-cred create ")) == 1 and "prf-eval=true" in " ".join(cred),
            "no recovery words": not lines(logs, "wf-words"), "an account": bool(pk1),
            "(ii) an unlisted AAGUID: one get() after create, no check kept": [l.split()[1] for l in cred[cred.index(made):] if l.startswith("wf-cred get")
            and "conditional" not in l].count("get") == 1 if made in cred else False,
        }
        branch = "PRF at create" if "results=true" in made else "enabled only, then one get()" if "enabled=true" in made else "no PRF"
        result("V1", all(checks.values()), "a device new here: the Site's window, a name and SIGN IN, one create, the egg (virtual)",
               failed=[k for k, v in checks.items() if not v], window=win, cred=cred, create_branch_virtual=branch, pk=pk1[:2])
        # V2: signed out on the Site (its storage cleared, the Door's known flag kept); SIGN IN on the same device.
        await b.p.cdp("Storage.clearDataForOrigin", origin=SITE, storageTypes="all")
        since, l0 = tap.mark(), len(tap.logs)
        how = await arrive(b, D, SITE + "/community", None)
        logs = tap.logs[l0:]
        g = await tap.read(since, D)
        cred, pk2 = lines(logs, "wf-cred"), [l.split()[2] for l in lines(logs, "wf-pk")]
        signups = [e for e in tap.events[since:] if e["method"] == "Network.requestWillBeSent" and e["params"]["request"]["url"] == D + "/v2/signup"]
        win = next((json.loads(l.split(" ", 1)[1]) for l in lines(logs, "wf-window")), {})
        checks = {"in the egg, no name asked": how == "egg", "known here": win.get("known") is True,
                  "the passkey asked at once, once, no create": [l.split()[0:2] for l in cred if not l.endswith("conditional")][:1] == [["wf-cred", "get"]]
                  and not any(l.startswith("wf-cred create ") for l in cred),
                  "no new account": not signups, "the same account": bool(pk1) and bool(pk2) and set(pk2) == {pk1[-1]}}
        result("V2", all(checks.values()), "signed out on the Site, SIGN IN on the same device: the passkey at once, the same account (virtual)",
               failed=[k for k, v in checks.items() if not v], cred=cred, pk=pk2[:2], door_documents=g["door_documents"])
    finally:
        for b in opened:
            b.close()
    # V3: a device known here whose passkey is not there (a dismissal's NotAllowedError): the name and SIGN IN.
    b = await pinned_browser(s, "known-empty")
    try:
        await b.p.go(D + "/v2/icd")
        await b.p.ev("localStorage.setItem('wallflowers.known', '1')")
        tap = await Tap(b).open()
        l0 = len(tap.logs)
        how = await arrive(b, D, SITE + "/community", None, 60)
        logs = tap.logs[l0:]
        cred = lines(logs, "wf-cred")
        checks = {"the passkey asked at once": any(l.startswith("wf-cred get") and not l.endswith("conditional") for l in cred),
                  "refused, then the name and SIGN IN": how == "named" and any("refused" in l and "NotAllowedError" in l for l in cred)}
        await b.p.shot(s.work, "V3-named")
        result("V3", all(checks.values()), "a device known here, its passkey refused: the name and SIGN IN, nothing made (virtual)",
               failed=[k for k, v in checks.items() if not v], cred=cred)
    finally:
        b.close()
    await witnessed_legs(s, result, lines)
    say("every Chrome closed")


GPM = "ea9b8d664d011d213ce4b6b48cb575d4"   # signin.js's WITNESSED: Google Password Manager, in desktop Chrome


async def witnessed_legs(s: w.Stack, result, lines) -> None:
    """V4-V6: the witnessed branch's logic. The page is shown GPM's AAGUID (the attestation the Door sees is untouched):
    create()'s PRF used and its check kept; the first get() after sign-out matches and drops it; a wrong check refused by name."""
    D = s.door_url
    b = await pinned_browser(s, "witnessed")
    try:
        await b.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=f"window.__asAaguid = {json.dumps(GPM)};")
        tap = await Tap(b).open()
        kept = "(async () => { const r = await fetch('/v2/icd'); return Object.keys(localStorage).filter(k => k.startsWith('wallflowers.prf-check.')); })()"
        # V4
        l0 = len(tap.logs)
        how = await arrive(b, D, D + "/join?claim=" + urllib.parse.quote(J.claim(SITE_ID, "financial", None), safe=""), f"Witnessed {TAG}")
        logs = tap.logs[l0:]
        cred, pk1 = lines(logs, "wf-cred"), [l.split()[2] for l in lines(logs, "wf-pk")]
        made = next((l for l in cred if l.startswith("wf-cred created")), "")
        after = [l for l in cred[cred.index(made) + 1:] if l.startswith("wf-cred get") and "conditional" not in l] if made else ["?"]
        await b.p.go(D + "/v2/icd")
        keys = await b.p.ev(kept)
        result("V4", how == "egg" and f"shown={GPM}" in made and "results=true" in made and not after and len(keys) == 1 and bool(pk1),
               "(ii) a witnessed provider (the page shown GPM's AAGUID): create()'s PRF used, no get(), its check kept (virtual)",
               created=made, gets_after_create=after, checks_kept=len(keys), pk=pk1[:1])
        # V5
        await b.p.cdp("Storage.clearDataForOrigin", origin=SITE, storageTypes="all")
        l0 = len(tap.logs)
        how = await arrive(b, D, SITE + "/community", None)
        logs = tap.logs[l0:]
        cred, pk2 = lines(logs, "wf-cred"), [l.split()[2] for l in lines(logs, "wf-pk")]
        await b.p.go(D + "/v2/icd")
        left = await b.p.ev(kept)
        result("V5", how == "egg" and [l.split()[1] for l in cred if "conditional" not in l][:1] == ["get"] and set(pk2) == set(pk1[-1:]) and not left,
               "(ii) signed out on the Site, SIGN IN: the check compared and dropped, the same account (virtual)",
               cred=cred, pk=pk2[:1], checks_left=len(left))
        # V6: a check that does not match, kept under this passkey's id: refused by name, nothing opened.
        key = keys[0] if keys else "wallflowers.prf-check.?"
        no_offer = (await b.p.cdp("Page.addScriptToEvaluateOnNewDocument", source="if (window.PublicKeyCredential) "
                    "PublicKeyCredential.isConditionalMediationAvailable = () => Promise.resolve(false);")).get("identifier")
        await b.p.ev(f"localStorage.setItem({json.dumps(key)}, '00000000000000000000000000000000')")
        await b.p.cdp("Storage.clearDataForOrigin", origin=SITE, storageTypes="all")
        l0, since = len(tap.logs), tap.mark()
        await b.p.cdp("Page.navigate", url=SITE + "/community")
        said = ""
        t0 = time.monotonic()
        while time.monotonic() - t0 < 60:
            said = await b.p.ev(f"location.origin === {json.dumps(D)} ? ((document.getElementById('status') || {{}}).textContent || '') : ''")
            if "no longer gives the key" in said or await b.p.ev(f"location.origin === {json.dumps(SITE)} && ({EGG})"):
                break
            await asyncio.sleep(0.3)
        await b.p.shot(s.work, "V6-refused")
        sent = [e["params"]["request"]["url"] for e in tap.events[since:] if e["method"] == "Network.requestWillBeSent"
                and e["params"]["request"]["url"].startswith(D + "/v2/signin/finish")]
        on = await b.p.ev("location.origin")
        shown = [l for l in tap.logs[l0:] if l.startswith("wf-said")]
        result("V6", any("no longer gives the key it was made with" in l for l in shown) and not sent and on == D,
               "(ii) a check that does not match: refused by name before anything is sent; nothing opened (virtual)",
               said=[l[:110] for l in shown][:4], signin_finish_sent=len(sent), at=on, cred=lines(tap.logs[l0:], "wf-cred")[:6])
        # V6b: the same wrong check, the offer left on. Chrome's virtual authenticator completes an offer unasked, so the
        # window retries in a loop: with the check before the attempt (prf-check-first), no /v2/signin is opened, no 429.
        if no_offer:
            await b.p.cdp("Page.removeScriptToEvaluateOnNewDocument", identifier=no_offer)
        await b.p.cdp("Storage.clearDataForOrigin", origin=SITE, storageTypes="all")
        l0, since = len(tap.logs), tap.mark()
        await b.p.cdp("Page.navigate", url=SITE + "/community")
        await asyncio.sleep(12)
        ev_ = tap.events[since:]
        opened = [e for e in ev_ if e["method"] == "Network.requestWillBeSent" and e["params"]["request"]["url"] == D + "/v2/signin"
                  and e["params"]["request"]["method"] == "POST"]
        too_many = [e for e in ev_ if e["method"] == "Network.responseReceived" and e["params"]["response"]["status"] == 429]
        gets = [l for l in lines(tap.logs[l0:], "wf-cred get")]
        shown = [l for l in tap.logs[l0:] if l.startswith("wf-said")]
        result("V6b", any("no longer gives the key it was made with" in l for l in shown) and len(gets) >= 2 and not opened and not too_many,
               "(ii) the check before the attempt: retried with the offer on, no /v2/signin opened, no 429 (virtual)",
               retries=len(gets), signin_opened=len(opened), answered_429=len(too_many), said=[l[:90] for l in shown][:2])
        tap.close()
    finally:
        b.close()


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP):
        print(__doc__)
        return 2
    w.CLAIM_MJS = Path(os.environ.get("JA_KIOSK", "")).resolve() / "egg" / "kiosk" / "claim.mjs"
    bp.map_name()
    s = w.Stack(host="localhost")
    results: list = []
    try:
        s.up()
        asyncio.run(asyncio.wait_for((window_run if LEGS == "V" else run)(s, results), 600))
    except Exception as e:                            # a step that could not go on is an NG, named
        results.append(("--", "NG", f"{type(e).__name__}: {e}"))
    finally:
        kept = s.work
        s.down()
    for sid, st, text in results:
        print(f"  {sid:<3} {st:<3}  {text}")
    print(f"screenshots: {kept}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("one-page join: " + ("G" if not bad and len(results) == (7 if LEGS == "V" else 3) else f"NG at {', '.join(bad) or 'a step not reached'}"))
    return 0 if not bad and len(results) == (7 if LEGS == "V" else 3) else 1


if __name__ == "__main__":
    sys.exit(main())
