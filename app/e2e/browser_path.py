"""The WallFlowers path in a browser: layer 2 (srr/vv.md; mdr/door.md §4, §5).

    make e2e-browser              # or: .venv/bin/python app/e2e/browser_path.py

The same loopback system as layer 1 (wallflowers_path.py), the Door named `localhost`
because WebAuthn refuses an IP origin. A VISIBLE Chrome for Testing, driven over the
DevTools protocol, with a virtual passkey authenticator that has PRF: the real sign-in
window (DR-4) and the real webapp. The authenticator is still a virtual one; a person's
own device is the last check, and this is not it.

Steps fail loudly: a missing element or a refusal on the page is named, with what the
page said (#status). Screenshots go to the run's directory (kept with E2E_KEEP=1).
"""
from __future__ import annotations

import asyncio
import base64
import hashlib
import http.server
import threading
import urllib.parse
import glob
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import httpx
import websockets

sys.path.insert(0, str(Path(__file__).resolve().parent))
from wallflowers_path import DEPLOYED, ICD, Absent, Fail, Stack, key  # noqa: E402

LAYER = "layer 2: a real browser and the real window; the authenticator is virtual (CDP, with PRF)"
# B9's public site, on the other loopback origin.
SITE = "site.example"
# E2E_DOOR=<a deployed Door> with E2E_DOOR_IP=<its address> (door-test over vzNAT): the Door's
# name is mapped to that address for this process and for Chrome, and nothing else resolves in
# Chrome but localhost, so the browser reaches the Door and no other host. Its certificate is
# trusted only once the chain verifies against E2E_DOOR_CA. Headless there (Ralph, 26 Sep).
# E2E_DOOR_VIA=<host:port>: what stands in front of it for Chrome (venue_link's hop), read at
# each launch.
DOOR_IP = os.environ.get("E2E_DOOR_IP", "")


def pinned(to: str = "") -> list[str]:
    """Chrome's flags for a deployed Door at DOOR_IP: its name mapped (to `to`, a host[:port],
    when something stands in front of it), every other name unresolvable, and its served
    certificate's key trusted, the chain checked here first."""
    import socket
    import ssl
    host = urllib.parse.urlsplit(DEPLOYED).hostname
    ctx = ssl.create_default_context(cafile=os.environ["E2E_DOOR_CA"])
    with socket.create_connection((DOOR_IP, 443), timeout=10) as raw, ctx.wrap_socket(raw, server_hostname=host) as tls:
        der = tls.getpeercert(binary_form=True)
    from cryptography import x509
    from cryptography.hazmat.primitives import serialization
    spki = x509.load_der_x509_certificate(der).public_key().public_bytes(
        serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo)
    pin = base64.b64encode(hashlib.sha256(spki).digest()).decode()
    return ["--headless=new", f"--host-resolver-rules=MAP {host} {to or os.environ.get('E2E_DOOR_VIA') or DOOR_IP}, MAP * ~NOTFOUND, EXCLUDE localhost",
            f"--ignore-certificate-errors-spki-list={pin}"]


def map_name() -> None:
    """This process reaches a deployed Door's name at DOOR_IP, and every other name as before."""
    import socket
    host, real = urllib.parse.urlsplit(DEPLOYED).hostname, socket.getaddrinfo

    def mapped(name, *a, **k):
        return real(DOOR_IP if name == host else name, *a, **k)
    socket.getaddrinfo = mapped


def chrome() -> str:
    """Chrome for Testing: $CHROME, or the one Playwright keeps under the home directory."""
    if os.environ.get("CHROME"):
        return os.environ["CHROME"]
    found = sorted(glob.glob(str(Path.home() / "Library/Caches/ms-playwright/chromium-*/chrome-mac*/*.app/Contents/MacOS/*")))
    if not found:
        raise Fail("no Chrome for Testing: set CHROME, or install Playwright's chromium")
    return found[-1]


# The page's own reports a failure carries: console, uncaught exceptions, dialogs, navigations.
HEARD = {
    "Runtime.consoleAPICalled": lambda e: "console." + e.get("type", "") + ": " + " ".join(
        str(a.get("value", a.get("description", ""))) for a in e.get("args", []))[:200],
    "Runtime.exceptionThrown": lambda e: "exception: " + str((e.get("exceptionDetails") or {}).get("exception", {}).get(
        "description", (e.get("exceptionDetails") or {}).get("text", "")))[:200],
    "Page.javascriptDialogOpening": lambda e: f"dialog {e.get('type')}: {e.get('message', '')[:200]}",
    "Page.frameNavigated": lambda e: "navigated: " + (e.get("frame") or {}).get("url", "")[:200],
    "Inspector.targetCrashed": lambda e: "the renderer crashed",
    "Inspector.detached": lambda e: "DevTools detached: " + e.get("reason", ""),
    "WebAuthn.credentialAdded": lambda e: "a passkey was made",
    "WebAuthn.credentialAsserted": lambda e: "a passkey was used",
}


class Page:
    """One page target over the DevTools protocol."""

    def __init__(self, ws) -> None:
        self.ws, self.n, self.waiting = ws, 0, {}
        self.said: list[str] = []  # what the page reported, for a failure to carry
        self.reader = asyncio.create_task(self._read())

    async def _read(self) -> None:
        async for raw in self.ws:
            d = json.loads(raw)
            if "id" in d and d["id"] in self.waiting:
                self.waiting.pop(d["id"]).set_result(d)
            elif d.get("method") in HEARD:
                self.said = (self.said + [HEARD[d["method"]](d.get("params", {}))])[-8:]

    async def cdp(self, method: str, **params):
        self.n += 1
        fut = asyncio.get_running_loop().create_future()
        self.waiting[self.n] = fut
        await self.ws.send(json.dumps({"id": self.n, "method": method, "params": params}))
        d = await asyncio.wait_for(fut, 60)
        if "error" in d:
            raise Fail(f"{method}: {d['error']}")
        return d.get("result", {})

    async def ev(self, expr: str):
        r = await self.cdp("Runtime.evaluate", expression=expr, awaitPromise=True, returnByValue=True)
        if "exceptionDetails" in r:
            raise Fail(f"{expr[:60]}: {r['exceptionDetails'].get('text')}")
        return r["result"].get("value")

    async def until(self, expr: str, what: str, secs: float = 30) -> None:
        end = time.monotonic() + secs
        while time.monotonic() < end:
            try:
                if await self.ev(expr):
                    return
            except Fail:
                pass
            await asyncio.sleep(0.2)
        said = await self.ev("(document.getElementById('status')||{}).textContent||''")
        raise Fail(f"timed out waiting for {what} at {await self.ev('location.href')}" + (f"; the page said: {said}" if said else ""))

    async def go(self, url: str) -> None:
        await self.cdp("Page.navigate", url=url)
        await asyncio.sleep(0.5)
        await self.until('document.readyState === "complete"', f"{url} to load")

    async def click(self, element_id: str) -> None:
        if not await self.ev(f"!!document.getElementById({json.dumps(element_id)})"):
            raise Fail(f"no #{element_id} on {await self.ev('location.href')}")
        await self.ev(f"document.getElementById({json.dumps(element_id)}).click()")

    async def shot(self, work: Path, name: str) -> None:
        """Evidence, not an assertion: a window macOS is not painting gives no frame, and
        the run goes on without it."""
        try:
            r = await asyncio.wait_for(self.cdp("Page.captureScreenshot", format="png"), 10)
            (work / f"{name}.png").write_bytes(base64.b64decode(r["data"]))
        except (Fail, TimeoutError):
            self.said = (self.said + [f"no screenshot {name}: no frame in 10 s"])[-8:]


def elsewhere(pages: dict) -> int:
    """Another origin on this machine, for DT-28: whatever path is asked, it answers; under
    a prefix `pages` names, that page (a public site's, B9)."""
    class Landed(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            page = next((v for k, v in pages.items() if self.path.startswith(k)), "<title>elsewhere</title>landed elsewhere")
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(page.encode())

        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Landed)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv.server_address[1]


async def run(s: Stack, results: list) -> None:
    pages: dict = {}
    other = elsewhere(pages)
    profile = s.work / "chrome"
    # Port 0: Chrome picks a free port and writes it into this run's own profile, so the
    # test can only ever drive the browser it started, never one another session holds.
    # Visible, and kept painting when another window covers it: a covered window is
    # throttled, and a screenshot of it never returns.
    proc = subprocess.Popen([chrome(), *(pinned() if DEPLOYED and DOOR_IP else []),
                             "--remote-debugging-port=0", f"--user-data-dir={profile}",
                             "--no-first-run", "--no-default-browser-check", "--window-size=430,932",
                             "--disable-backgrounding-occluded-windows", "--disable-renderer-backgrounding",
                             "--disable-background-timer-throttling", "about:blank"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    s.procs.append(proc)
    for _ in range(600):
        try:
            port = int((profile / "DevToolsActivePort").read_text().split()[0])
            targets = httpx.get(f"http://127.0.0.1:{port}/json/list", timeout=1).json()
            target = next(t for t in targets if t["type"] == "page")
            break
        except (OSError, ValueError, IndexError, httpx.HTTPError, StopIteration):
            await asyncio.sleep(0.2)
    else:
        raise Fail("Chrome for Testing did not open its DevTools port")
    async with websockets.connect(target["webSocketDebuggerUrl"], max_size=None) as ws:
        p = Page(ws)
        for m in ("Page.enable", "Runtime.enable", "Inspector.enable", "Page.bringToFront"):
            await p.cdp(m)
        await p.cdp("Emulation.setFocusEmulationEnabled", enabled=True)
        await p.cdp("WebAuthn.enable", enableUI=False)
        auth = await p.cdp("WebAuthn.addVirtualAuthenticator", options={
            "protocol": "ctap2", "ctap2Version": "ctap2_1", "transport": "internal", "hasResidentKey": True,
            "hasUserVerification": True, "isUserVerified": True, "automaticPresenceSimulation": True, "hasPrf": True})
        D = s.door_url

        async def step(sid: str, what: str, body) -> bool:
            try:
                results.append((sid, "PASS", f"{what}: {await body()}"))
                return True
            except Absent as e:
                results.append((sid, "ABSENT", f"{what}. {e}"))
                return True
            except (Fail, TimeoutError) as e:
                heard = f" [the page: {' | '.join(p.said[-4:])}]" if p.said else ""
                if p.reader.done():
                    heard += " [the DevTools connection closed]"
                results.append((sid, "FAIL", f"{what}. {str(e) or type(e).__name__}{heard}"))
                await p.shot(s.work, f"{sid}-failed")
                return False

        async def b1():
            # A sign-in portal (Ralph, 28 Sep): signed out, the webapp is the window, at once,
            # and the window comes back to it.
            await p.go(D + "/")
            await p.until("location.pathname === '/signin'", "the window, signed out")
            await p.until("document.readyState === 'complete'", "the window to load")
            await p.shot(s.work, "B1-signed-out")
            back = await p.ev("new URLSearchParams(location.search).get('return')")
            if back != "/":
                raise Fail(f"signed out, the window returns to {back!r}, not /")
            return "the window, returning to /"

        async def b2():
            # SEC-6: the words are shown once, and only after the account they recover exists.
            await p.go(D + "/signin?new&return=%2F")
            # One tap: Create an account asks the passkey at once (Ralph, 29 Sep).
            await p.ev("document.getElementById('new').click(), true")
            early = await p.ev("document.querySelectorAll('#list li').length")
            if early:
                raise Fail(f"{early} recovery words shown before the account exists (SEC-6)")
            await p.until("!document.getElementById('shown').hidden", "the recovery words", 60)
            if not await p.ev("document.getElementById('words').hidden"):
                raise Fail("a step between Create an account and the passkey")
            n = await p.ev("document.querySelectorAll('#list li').length")
            exists = await p.ev("fetch('/v2/me').then(r => r.status)")
            await p.shot(s.work, "B2-words")
            if n != 24:
                raise Fail(f"{n} recovery words shown, not 24")
            if exists != 200:
                raise Fail(f"the words are shown and /v2/me answers {exists}: no account behind them")
            await p.click("done")
            await p.until("location.pathname === '/' && !location.search.includes('new')", "the return to the webapp", 60)
            return "a passkey made with PRF; the account, then its 24 words, once"

        async def b3():
            await p.until("fetch('/v2/me').then(r => r.status === 200)", "/v2/me to answer", 30)
            await p.until("!!document.querySelector('main[data-screen=\"inside\"]#app')", "the webapp's interior", 30)
            await p.shot(s.work, "B3-inside")
            me = await p.ev("fetch('/v2/me').then(r => r.json())")
            state["pk"] = me["pk"]
            if me.get("noncompliant"):
                raise Fail(f"noncompliant objects: {me['noncompliant']}")
            return f"signed in as {key(me['pk'])[:12]}, inside, nothing noncompliant"

        async def b4():
            await p.ev("fetch('/v2/signout', {method: 'POST'}).then(r => r.status)")
            # The open page meets its ended session and goes to the window (NC-91, the portal).
            await p.until("location.pathname === '/signin'", "the open page to go to the window", 30)
            code = await p.ev("fetch('/v2/me').then(r => r.status)")
            if code == 200:
                raise Fail("/v2/me still answers after sign-out")
            return f"/v2/me answers {code}; the open page went to the window"

        async def b5():
            await p.go(D + "/signin?return=%2F")
            await p.click("in")
            await p.until("location.pathname === '/' && location.search === ''", "the return after sign-in", 60)
            await p.until("fetch('/v2/me').then(r => r.status === 200)", "/v2/me after sign-in", 30)
            await p.shot(s.work, "B5-inside-again")
            me = await p.ev("fetch('/v2/me').then(r => r.json())")
            if key(me["pk"]) != key(state["pk"]):
                raise Fail("the passkey signed in as another account")
            verdict = ((me.get("resume") or {}).get("chain") or {}).get("verdict")
            if verdict != "Whole":
                raise Fail(f"the restore's chain verdict is {verdict!r}, not Whole")
            creds = await p.cdp("WebAuthn.getCredentials", authenticatorId=auth["authenticatorId"])
            rp = {c.get("rpId") for c in creds.get("credentials", [])}
            return f"the same account, a fresh session, chain Whole; passkey rp {', '.join(sorted(rp))}"

        async def b6():
            # DT-28 (SEC-41, CS-39): the window returns only to its own origin, judged by where
            # each return resolves. The other origin is 127.0.0.1's own; nothing leaves the host.
            escaped = []
            vectors = [("%09", "/\t/"), ("%0A", "/\n/"), ("%0D", "/\r/"), ("//", "//"), ("/\\", "/\\"),
                       ("/.//", "/.//"), ("/..//", "/..//"), ("/x/..//", "/x/..//"), ("/%2e//", "/%2e//")]
            for i, (label, lead) in enumerate(vectors, 1):
                raw = f"{lead}localhost:{other}/landed"
                await p.go(D + "/signin?return=" + urllib.parse.quote(raw, safe=""))
                await p.click("in")
                await p.until("location.pathname !== '/signin'", f"the return after sign-in ({label})", 60)
                await asyncio.sleep(0.5)
                # The URL navigated to, not the document's: an error page's origin is null.
                h = await p.cdp("Page.getNavigationHistory")
                u = urllib.parse.urlsplit(h["entries"][h["currentIndex"]]["url"])
                origin = f"{u.scheme}://{u.netloc}"
                if origin != D:
                    escaped.append(f"{label} to {origin}")
                    await p.shot(s.work, f"B6-escaped-{i}")
            if escaped:
                raise Fail("the window returned off its origin: " + "; ".join(escaped) + " (DT-28, SEC-41, CS-39)")
            return f"each of {', '.join(l for l, _ in vectors)} lands on the Door's own origin"

        async def b7():
            # K-45: a browser whose passkey has no PRF. Sign-up is refused, in words, and no
            # session opens. The trail names the last thing done if the page stops answering.
            trail: list[str] = []
            try:
                return await b7_steps(trail)
            except TimeoutError:
                raise Fail("the page stopped answering after: " + " > ".join(trail))

        async def b7_steps(trail: list) -> str:
            trail.append("to the Door")  # B6 leaves the tab on an error page, where nothing can fetch
            await p.go(D + "/")
            trail.append("sign out")
            await p.ev("fetch('/v2/signout', {method: 'POST'}).then(r => r.status)")
            trail.append("swap in an authenticator with no PRF")
            await p.cdp("WebAuthn.removeVirtualAuthenticator", authenticatorId=auth["authenticatorId"])
            state["authenticator"] = await p.cdp("WebAuthn.addVirtualAuthenticator", options={
                "protocol": "ctap2", "ctap2Version": "ctap2_1", "transport": "internal", "hasResidentKey": True,
                "hasUserVerification": True, "isUserVerified": True, "automaticPresenceSimulation": True,
                "hasPrf": False})
            trail.append("open /signin?new")
            await p.go(D + "/signin?new&return=%2F")
            trail.append("New, and the passkey at once")
            await p.click("new")
            trail.append("wait for a refusal or the words")
            await p.until("!document.getElementById('status').hidden || !document.getElementById('shown').hidden"
                          " || location.pathname !== '/signin'", "a refusal, the words, or a landing", 60)
            shown = await p.ev("document.querySelectorAll('#list li').length")
            await p.shot(s.work, "B7-no-prf")
            if await p.ev("location.pathname") != "/signin":
                raise Fail("a passkey with no PRF signed up and landed")
            said = await p.ev("document.getElementById('status').textContent")
            code = await p.ev("fetch('/v2/me').then(r => r.status)")
            if "PRF" not in said:
                raise Fail(f"refused, but not for PRF: {said!r}")
            if code == 200:
                raise Fail("refused on the page, but a session is open (/v2/me 200)")
            if shown:
                raise Fail(f"refused, but {shown} recovery words were shown for an account that does not exist (SEC-6)")
            return f"refused, \"{said}\"; /v2/me {code}; no words shown"

        async def b8():
            # Consent's Cancel (step 5; SEC-41): back to the REGISTERED callback with
            # error=access_denied and the state unchanged, and no code. The callback is the
            # other loopback origin, so the landing is a page, not an error. Registering
            # restarts the Door (its stand-in, DOOR_CLIENTS), so this step and B9 run last,
            # registered at once.
            client, callback = "e2e.example", f"http://localhost:{other}/signin/callback"
            await asyncio.to_thread(s.register, {
                client: {"callbacks": [callback], "origins": [], "site": ""},
                SITE: {"callbacks": [f"http://localhost:{other}/site/callback"], "origins": [f"http://localhost:{other}"], "site": ""}})
            verifier = base64.urlsafe_b64encode(os.urandom(32)).rstrip(b"=")
            challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier).digest()).rstrip(b"=").decode()
            st8 = os.urandom(8).hex()
            q = urllib.parse.urlencode({"client": client, "redirect_uri": callback, "code_challenge": challenge,
                                        "code_challenge_method": "S256", "state": st8})
            await p.go(f"{D}/signin?{q}")
            # One step (Ralph, 29 Sep): the site named on the screen the passkey is asked from,
            # asked at once; Cancel there, once the passkey's prompt is done with.
            await p.until("!document.getElementById('deny').hidden && !document.getElementById('deny').disabled", "the site's one screen")
            named = await p.ev("(document.querySelector('.site-name') || {}).textContent || ''")
            await p.shot(s.work, "B8-site")
            if client not in named:
                raise Fail(f"the window does not name the site: {named!r}")
            await p.click("deny")
            await p.until(f"location.origin !== {json.dumps(D)}", "the return to the callback", 30)
            h = await p.cdp("Page.getNavigationHistory")
            u = urllib.parse.urlsplit(h["entries"][h["currentIndex"]]["url"])
            got = urllib.parse.parse_qs(u.query)
            if f"{u.scheme}://{u.netloc}{u.path}" != callback:
                raise Fail(f"Cancel went to {u.scheme}://{u.netloc}{u.path}, not the registered callback")
            if got.get("error") != ["access_denied"] or got.get("state") != [st8] or "code" in got:
                raise Fail(f"Cancel returned {u.query!r}")
            return f"consent named {named!r}; Cancel returned access_denied and the state to the registered callback, no code"

        async def b9():
            # DR-5: a public site signs in with the Door's own script, /v2/signin.js, loaded as
            # its header says a site loads it, pinned by SRI. A new account: consent, the
            # passkey, the words, Continue (NC-53), the site's callback, the token (DPoP,
            # SEC-39); a write and its change on the stream; sign-out revokes the token.
            if DEPLOYED:  # its origin and callback are B8's registration, which a deployed Door cannot take
                s.register({})
            origin, callback = f"http://localhost:{other}", f"http://localhost:{other}/site/callback"
            js = (await asyncio.to_thread(httpx.get, f"{D}/v2/signin.js", verify=s.verify, timeout=10)).content
            sri = "sha384-" + base64.b64encode(hashlib.sha384(js).digest()).decode()
            pages["/site/"] = (f'<!doctype html><title>a public site</title>'
                               f'<script src="{D}/v2/signin.js" integrity="{sri}" crossorigin="anonymous"></script>')
            await p.cdp("WebAuthn.removeVirtualAuthenticator", authenticatorId=state["authenticator"]["authenticatorId"])
            await p.cdp("WebAuthn.addVirtualAuthenticator", options={
                "protocol": "ctap2", "ctap2Version": "ctap2_1", "transport": "internal", "hasResidentKey": True,
                "hasUserVerification": True, "isUserVerified": True, "automaticPresenceSimulation": True, "hasPrf": True})
            opts = json.dumps({"client": SITE, "callback": callback})
            await p.go(f"{origin}/site/")
            await p.until("!!window.WallFlowers", "/v2/signin.js to load under its SRI pin", 10)
            await p.ev(f"void WallFlowers.signIn({opts}), true")
            # One step: the passkey asked at once finds none (a new account), and the screen
            # stays; Create an account asks the next one at once.
            await p.until(f"location.origin === {json.dumps(D)} && !document.getElementById('new').disabled"
                          " && !!document.querySelector('.site-name')", "the site's one screen")
            await p.shot(s.work, "B9-site")
            await p.click("new")
            await p.until("!document.getElementById('shown').hidden", "the recovery words", 60)
            n = await p.ev("document.querySelectorAll('#list li').length")
            await p.shot(s.work, "B9-words")
            if n != 24:
                raise Fail(f"{n} recovery words shown, not 24")
            await p.click("done")
            await p.until(f"location.origin === {json.dumps(origin)} && location.pathname === '/site/callback'",
                          "Continue, to the site's callback", 30)
            await p.until("!!window.WallFlowers", "/v2/signin.js on the callback page", 10)
            if not await p.ev(f"WallFlowers.finish({opts}).then(x => (window.S = x, !!x))"):
                raise Fail("WallFlowers.finish gave no session")
            me = await p.ev("S.fetch('/v2/me').then(r => r.json())")
            if me.get("noncompliant"):
                raise Fail(f"noncompliant objects: {me['noncompliant']}")
            made = await p.ev("S.fetch('/v2/mint', {method: 'POST', body: JSON.stringify({kind: 'group', draft: {name: 'b9'}})})"
                              ".then(r => r.json())")
            obj = made.get("object_id")
            if not obj:
                raise Fail(f"the token's mint: {made}")
            if not await p.ev(f"S.fetch('/v2/graph').then(r => r.json()).then(g => g.objects.some(o => o.id === {json.dumps(obj)}))"):
                raise Fail("what the token minted is not in its graph")
            op = next(n for n, o in json.loads(ICD.read_text())["kinds"]["group"]["ops"].items() if o["op"] == 0)
            await p.ev("window.E = []; window.stopE = S.events(v => E.push(v)); true")
            await asyncio.sleep(1)
            wrote = await p.ev(f"S.fetch('/v2/apply', {{method: 'POST', body: JSON.stringify({{object: {json.dumps(obj)}, "
                               f"op: {json.dumps(op)}, args: {{displayName: 'b9, again', shape: 'team'}}}})}}).then(r => r.status)")
            if wrote != 200:
                raise Fail(f"the token's write, {op}: {wrote}")
            await p.until("E.length > 0", "a change on the token's stream after its write", 30)
            versions = await p.ev("E")
            if not all(str(v).isdigit() for v in versions):
                raise Fail(f"the stream's events are not versions: {versions}")
            current = f"WallFlowers.current({json.dumps({'client': SITE})})"
            if not await p.ev(f"{current}.then(x => !!x)"):
                raise Fail("WallFlowers.current does not hold the session")
            await p.ev("stopE(), S.signOut().then(() => true)")
            after = await p.ev("S.fetch('/v2/me').then(r => r.status)")
            forgot = await p.ev(f"{current}.then(x => !x)")
            if after != 401 or not forgot:
                raise Fail(f"after S.signOut(): the token's /v2/me {after}, the session forgotten: {forgot}")
            return (f"signin.js under SRI; consent, a passkey, 24 words, Continue, the callback; the token as "
                    f"{key(me['pk'])[:12]}: minted, listed, {op} written, stream {versions}; signed out, /v2/me {after}")

        state: dict = {}
        steps = [("B1", "signed out, the webapp is the window", b1),
                 ("B2", "sign-up in the window at /signin: passkey with PRF, the account, then its words once", b2),
                 ("B3", "landing: signed in, in the interior", b3),
                 ("B4", "sign-out ends the session, and the open page goes to the window", b4),
                 ("B5", "the same passkey signs in again: a fresh session restored, chain Whole", b5),
                 ("B6", "the window returns only to its own origin (DT-28)", b6),
                 ("B7", "a passkey with no PRF: sign-up refused and named, no session (K-45)", b7),
                 ("B8", "a site's one screen, cancelled: access_denied and the state at its registered callback", b8),
                 ("B9", "a public site signs in with /v2/signin.js: a new account's words, Continue, the token, sign-out", b9)]
        ok = True
        for sid, what, body in steps:
            if not ok:
                results.append((sid, "NOT REACHED", what))
                continue
            ok = await step(sid, what, body)
        p.reader.cancel()


def main() -> int:
    print(f"WallFlowers path, {LAYER}" + (f", the Door deployed at {DEPLOYED}" + (f" ({DOOR_IP})" if DOOR_IP else "") if DEPLOYED else ""))
    if DEPLOYED and DOOR_IP:
        map_name()
    s = Stack(host="localhost")
    results: list = []
    try:
        s.build()
        s.up()
        asyncio.run(run(s, results))
    except (Fail, TimeoutError, OSError) as e:
        results.append(("--", "FAIL", f"{str(e) or type(e).__name__}"))
    finally:
        kept = s.work
        s.down()
    for sid, st, text in results:
        print(f"  {sid:<4} {st:<11}  {text}")
    if os.environ.get("E2E_KEEP"):
        print(f"screenshots: {kept}")
    passed = sum(1 for _, st, _ in results if st == "PASS")
    steps = sum(1 for sid, _, _ in results if sid != "--")
    first = next((t for _, st, t in results if st not in ("PASS", "ABSENT")), None)
    print(f"e2e-browser: {passed} of {steps} steps pass" + (f"; first: {first}" if first else ""))
    return 0 if first is None and passed else 1


if __name__ == "__main__":
    sys.exit(main())
