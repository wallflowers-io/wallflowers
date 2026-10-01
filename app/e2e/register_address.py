"""Register through the Door claims the draft's address (www deploy/launch ee14087's proof, WEBAPP_HUMAN, 29 Sep).

www checks the address, then hands the draft to the Door's window; the webapp's Register makes the Site and claims
the address at /v2/site/address. On a deployed Door, a new owner each time, through the window (?new&return=
/#register=<draft>), the webapp's own /v2/site/address call recorded by a fetch trap:

  A1  a fresh slug: the claim answers 2xx, and the Site opens
  A2  a slug already held (REG_TAKEN, `ja-rehearsal` on door-test): the claim is refused and the Register sheet
      stays, saying the address is taken, with a field for another (7e347f7b); the Site is made already
  A3  another address typed there: the retry claims it (2xx) and the Site opens, and no second Site is made

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  [REG_TAKEN=ja-rehearsal] .venv/bin/python app/e2e/register_address.py
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import sys
import time
import urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import journeys as J  # noqa: E402
import wallflowers_path as w  # noqa: E402

TAG = time.strftime("%m%d%H%M%S", time.gmtime())
TAKEN = os.environ.get("REG_TAKEN", "ja-rehearsal")
say = J.say
# The webapp's /v2/site/address call, as it answered.
TRAP = r"""(() => {
  const f = window.fetch; window.__addr = [];
  window.fetch = async (...a) => {
    const r = await f(...a);
    if (String(a[0]?.url ?? a[0]).endsWith('/v2/site/address')) {
      const body = await r.clone().text().catch(() => '');
      window.__addr.push({ status: r.status, body: body.slice(0, 300), sent: String(a[1]?.body || '').slice(0, 200) });
    }
    return r;
  };
})();"""


def draft(name: str, slug: str) -> str:
    d = {"kind": "Community", "name": name, "purpose": "register_address.py", "slug": slug, "pname": "Address " + TAG, "face": None}
    return base64.urlsafe_b64encode(json.dumps(d).encode()).rstrip(b"=").decode()


async def register(s: w.Stack, tag: str, name: str, slug: str, retry: str | None = None) -> dict:
    D = s.door_url
    b = await J.Browser().open(s.work, f"reg-{tag}")
    try:
        await b.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=TRAP)
        await b.p.go(D + "/signin?new&return=" + urllib.parse.quote("/#register=" + draft(name, slug), safe=""))
        words = await b.sign_up()
        await b.p.until("!document.getElementById('sheet').hidden && (document.getElementById('sheetGo') || {}).textContent === 'Register'",
                        "the Register sheet", 90)
        await b.p.click("sheetGo")
        out = {"words": words}
        if retry is None:
            await b.p.until("document.getElementById('sheet').hidden && location.hash === ''", "the Site to open", 120)
        else:
            # A refused address keeps the sheet (7e347f7b): its why, and a field for another.
            await b.p.until("window.__addr && window.__addr.length > 0", "the address claimed", 60)
            await asyncio.sleep(1.5)
            out["kept"] = await b.p.ev("({shown: !document.getElementById('sheet').hidden, why: document.getElementById('sheetWhy').textContent, "
                                      "field: !!document.getElementById('regSlug')})")
            out["site_before_retry"] = await b.p.ev(f"fetch('/v2/graph').then(r => r.json()).then(g => (g.objects || []).filter(o => o.kind === 'group' && o.name === {json.dumps(name)}).length)")
            await b.p.shot(s.work, f"A-{tag}-kept")
            await b.p.ev(f"(() => {{ const i = document.getElementById('regSlug'); i.value = {json.dumps(retry)}; i.dispatchEvent(new Event('input', {{bubbles: true}})); }})()")
            await b.p.click("sheetGo")
            await b.p.until("document.getElementById('sheet').hidden && location.hash === ''", "the Site to open after the retry", 120)
        await asyncio.sleep(2)
        out["address"] = await b.p.ev("window.__addr || []")
        g = await b.graph()
        sites = [o for o in g.get("objects", []) if o.get("kind") == "group" and o.get("name") == name]
        out["site"] = (sites[0] if sites else {}).get("id", "")[:16] or None
        out["sites_named"] = len(sites)
        feed = await b.p.ev("(document.getElementById('feed') || {}).innerText || ''")
        out["noted"] = [l for l in feed.splitlines() if slug in l or "address" in l.lower()][:3]
        await b.p.shot(s.work, f"A-{tag}")
        await b.p.ev("fetch('/v2/signout', {method: 'POST'}).then(r => r.status, () => 0)")
        return out
    finally:
        b.close()


async def run(s: w.Stack, results: list) -> None:
    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail, ensure_ascii=False)[:900]))
        say(sid, "G" if ok else "NG", json.dumps(detail, ensure_ascii=False)[:900])

    fresh = f"regaddr-{TAG}"
    a1 = await register(s, "fresh", f"Register address {TAG}", fresh)
    st1 = [x["status"] for x in a1["address"]]
    result("A1", a1["words"] == 24 and len(st1) == 1 and 200 <= st1[0] < 300 and bool(a1["site"]),
           f"a fresh slug ({fresh}): Register claims it, and the Site opens", slug=fresh, **a1)
    again = f"regretry-{TAG}"
    a2 = await register(s, "taken", f"Register taken {TAG}", TAKEN, retry=again)
    st2 = [x["status"] for x in a2["address"]]
    kept = a2.get("kept") or {}
    result("A2", a2["words"] == 24 and len(st2) >= 1 and st2[0] >= 400 and kept.get("shown") and kept.get("field")
           and "taken" in (kept.get("why") or "") and a2.get("site_before_retry") == 1,
           f"a slug already held ({TAKEN}): the claim refused, the sheet kept saying so, with a field for another; the Site made",
           slug=TAKEN, kept=kept, first_claim=a2["address"][:1], site_before_retry=a2.get("site_before_retry"))
    result("A3", len(st2) == 2 and 200 <= st2[1] < 300 and a2["sites_named"] == 1 and bool(a2["site"]),
           f"another address ({again}) typed there: claimed, the Site opens, no second Site", retry=again,
           retry_claim=a2["address"][1:], sites_named=a2["sites_named"])


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP):
        print(__doc__)
        return 2
    bp.map_name()
    s = w.Stack(host="localhost")
    results: list = []
    try:
        s.up()
        asyncio.run(asyncio.wait_for(run(s, results), 600))
    except (w.Fail, TimeoutError, OSError) as e:
        results.append(("--", "NG", f"{str(e) or type(e).__name__}"))
    finally:
        kept = s.work
        s.down()
    for sid, st, text in results:
        print(f"  {sid:<3} {st:<3}  {text}")
    print(f"screenshots: {kept}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("register address: " + ("G" if not bad else f"NG at {', '.join(bad)}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
