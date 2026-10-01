"""W-90's `make site-setup`, rehearsed on door-test: this is SITE_SETUP_OPEN's stand-in for the window Ralph's
Touch ID answers. It reads the Door's window for the setup client (headless, pinned: one step, the client's
name heading it), then signs the Site's layer-1 owner (RESTATE_STATE, restate_timing.py's) in with the
window's client, PKCE and state, and follows the redirect to site-setup's one loopback callback. The
passkey itself is stood in: a layer-1 account's sealed PRF, not a platform authenticator.

A run (run 82): a scratch copy of app/door/hosting/one-signin/ whose one-signin-2.1.0.js names door-test's
SITE, SLUG and the rehearsal kid and key (site-setup.mjs checks them against the client), the client
wallflowers-setup-<SLUG> in door-test.clients.json, then
  DOOR=https://door.localhost NODE_EXTRA_CA_CERTS=<edge root> ARC_BUNDLE_URL=http://127.0.0.1:8090/v1/bundle
  SITE_SETUP_CLIENTS=app/e2e/door-test.clients.json SITE_SETUP_OPEN=<a script exec'ing this with E2E_DOOR,
  E2E_DOOR_IP, E2E_DOOR_CA, E2E_AUTH, RESTATE_STATE, STANDIN_LOG, STANDIN_WORK> node site-setup.mjs <SLUG> <mark>"""

import asyncio, json, os, sys, time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit
import httpx
PRODUCT = Path(__file__).resolve().parents[2]; STATE = Path(os.environ["RESTATE_STATE"]); LOG = open(os.environ["STANDIN_LOG"], "a")
sys.path.insert(0, str(PRODUCT / "app" / "e2e"))
import browser_path as bp, journeys as J, wallflowers_path as w
def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, file=LOG, flush=True)
url = sys.argv[1]; q = {k: v[0] for k, v in parse_qs(urlsplit(url).query).items()}
say("window:", url.split("&code_challenge")[0])
bp.map_name(); s = w.Stack(host="localhost"); s.up()
async def consent():
    b = await J.Browser().open(Path(os.environ["STANDIN_WORK"]), "setup-window")
    try:
        await b.p.go(url)
        # One step (2f3285ae): no consent; the client's name heads the window the passkey is asked from.
        await b.p.until("!!document.querySelector('h1.site-name')", "the window's name", 30)
        await asyncio.sleep(1)
        await b.p.shot(Path(os.environ["STANDIN_WORK"]), "setup-window")
        return await b.p.ev("""({name: document.querySelector('h1.site-name').textContent, site: document.documentElement.hasAttribute('data-site'),
          face: document.documentElement.hasAttribute('data-face'), foot: !!document.querySelector('footer.wf img[src="/door/wallflowers.png"]'),
          wordmark: !!document.querySelector('img.mark'), consent: !!document.getElementById('consent')})""")
    finally:
        b.close()
try:
    text = asyncio.run(consent())
    say("window:", json.dumps(text)[:300])
except Exception as e:
    say("consent: not read:", str(e)[:200])
st = json.loads(STATE.read_text())
who = {"handle": st["owner"]["handle"], "pk": st["owner"]["pk"], "prf": bytes.fromhex(st["owner"]["prf"])}
c = w.webapp(s)
o = c.post("/v2/signin", json=w.work(c, "signin") or None).json()
r = c.post("/v2/signin/finish", json={"attempt": o["attempt"], "handle": who["handle"], "sealed": w.seal(o["key"], who["prf"], o["attempt"]),
                                       "client": {"client": q["client"], "redirect_uri": q["redirect_uri"], "state": q["state"],
                                                  "code_challenge": q["code_challenge"]}})
say("signin/finish", r.status_code, "redirect to", (r.json().get("redirect") or "")[:40] if r.status_code == 200 else r.text[:200])
if r.status_code == 200:
    cb = httpx.get(r.json()["redirect"], timeout=30)
    say("callback", cb.status_code, cb.text[:40])
