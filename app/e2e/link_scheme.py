"""A member's post link is a link only as a web address (webapp/link-scheme 53209828; SECURITY, 29 Sep).

A throwaway account on the deployed Door posts one link in each form the webapp draws (link, image, pdf),
hostile and not: `javascript:` as written, in capitals, after a space, and with a tab or newline inside the
word (the URL parser drops those, so a check on the string alone passes them), `data:text/html` and a
`data:` pdf; https for the control. Each payload, if it ever runs, fetches a beacon on loopback. In the
account's window, each post's page from the Resources tab, and for a pdf the viewer too:

  hostile  no a, area, iframe, embed, object, form or formaction in the document resolves to `javascript:` or
           `data:` (the app's own `data:image` pictures aside); no "Open at"; the viewer's Open hidden, with
           no href; the title drawn; every link and button in the post's page and the viewer tapped (CDP
           mouse), and the beacon not fetched, no dialog, no new tab
  control  "Open at example.org" (a pdf's "Open the PDF at"), its href the link; the viewer's Open shown

Where a src names the link (img, video: inert in Chrome), it is reported; with LINK_MEDIA=1 (webapp/media-src:
pictureOf() and the film through webUrl()) an img or video src naming it is graded, and a broken picture (an img drawn
with no width) with it; a pdf's canvas (data-pdf, read by pdf.js, which runs no script) is reported still.
LINK_RED=<rev> serves this Chrome that rev's webapp.js in place of the Door's (Fetch interception, this
window only): the check's red control.

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  [LINK_RED=64d16114] .venv/bin/python app/e2e/link_scheme.py
"""
from __future__ import annotations

import asyncio
import base64
import hashlib
import http.server
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import ja_rehearsal as JR  # noqa: E402
import pdf_viewer as PV  # noqa: E402
import wallflowers_path as w  # noqa: E402

RED = os.environ.get("LINK_RED", "")
MEDIA = os.environ.get("LINK_MEDIA") == "1"
TAG = time.strftime("%m%d%H%M", time.gmtime())
say = JR.say
HITS: list = []


def beacon() -> http.server.ThreadingHTTPServer:
    """Loopback only; answers CORS and Private Network Access, so a payload that runs is heard."""
    class H(http.server.BaseHTTPRequestHandler):
        def cors(self):
            self.send_header("Access-Control-Allow-Origin", "*")
            if self.headers.get("Access-Control-Request-Private-Network"):
                self.send_header("Access-Control-Allow-Private-Network", "true")

        def do_OPTIONS(self):
            self.send_response(204)
            self.cors()
            self.end_headers()

        def do_GET(self):
            HITS.append(self.path)
            self.send_response(204)
            self.cors()
            self.end_headers()

        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def posts(b: str) -> list[dict]:
    run = lambda tag: f"fetch('{b}/ran/{tag}')"
    pdf = base64.b64encode(PV.one_page_pdf("data: pdf")).decode()
    return [
        {"tag": "https", "form": "link", "link": "https://example.org/link-scheme", "good": True},
        {"tag": "https-pdf", "form": "pdf", "link": "https://example.org/link-scheme.pdf", "good": True},
        {"tag": "js", "form": "link", "link": "javascript:" + run("js")},
        {"tag": "js-caps", "form": "link", "link": "JAVASCRIPT:" + run("js-caps")},
        {"tag": "js-space", "form": "link", "link": " javascript:" + run("js-space")},
        {"tag": "js-tab", "form": "link", "link": "java\tscript:" + run("js-tab")},
        {"tag": "js-newline", "form": "link", "link": "java\nscript:" + run("js-newline")},
        {"tag": "js-image", "form": "image", "link": "javascript:" + run("js-image") + "//x.png"},
        {"tag": "js-film", "form": "link", "link": "javascript:" + run("js-film") + "//x.mp4"},
        {"tag": "js-pdf", "form": "pdf", "link": "javascript:" + run("js-pdf")},
        {"tag": "data-html", "form": "link", "link": f"data:text/html,<script>{run('data-html')}</script>"},
        {"tag": "data-pdf", "form": "pdf", "link": "data:application/pdf;base64," + pdf},
    ]


# Every attribute that navigates or runs, resolved as the browser will, and every src naming the link.
SINKS = """((link) => {
  const bad = [], srcs = [];
  const scheme = v => { try { return new URL(v, location.href).protocol; } catch (e) { return ''; } };
  const nav = [['a', 'href'], ['area', 'href'], ['iframe', 'src'], ['frame', 'src'], ['embed', 'src'], ['object', 'data'],
               ['form', 'action'], ['[formaction]', 'formaction'], ['a', 'xlink:href']];
  for (const [sel, at] of nav) for (const e of document.querySelectorAll(sel)) {
    const v = e.getAttribute(at); if (v == null) continue;
    const p = scheme(v);
    if (p === 'javascript:' || p === 'vbscript:' || (p === 'data:' && !/^\\s*data:image\\/(png|jpeg|gif|webp);base64,/i.test(v)))
      bad.push({el: e.tagName.toLowerCase() + (e.id ? '#' + e.id : '') + (e.className ? '.' + String(e.className).split(' ')[0] : ''), at, v: v.slice(0, 80)});
  }
  for (const e of document.querySelectorAll('[src], [data-pdf]')) {
    const v = e.getAttribute('src') || e.getAttribute('data-pdf');
    if (v === link || v === link + '#t=0.5') srcs.push(e.tagName.toLowerCase() + (e.className ? '.' + String(e.className).split(' ')[0] : ''));
  }
  const obj = document.querySelector('#feed .obj');
  const go = obj ? [...obj.querySelectorAll('a.golink')].map(a => ({text: a.textContent, href: a.getAttribute('href')})) : [];
  const broken = obj ? [...obj.querySelectorAll('img')].filter(i => i.complete && i.naturalWidth === 0).length : 0;
  const po = document.getElementById('pdfOpen');
  return {bad, srcs, broken, title: obj ? (obj.querySelector('h1') || {}).textContent || '' : null, go,
          pdfOpen: po ? {shown: !po.hidden && po.checkVisibility(), href: po.getAttribute('href')} : null};
})"""

# The centres of what a person can tap in the post's page, or in the viewer.
TAPS = """((where) => [...document.querySelectorAll(where)].filter(e => e.checkVisibility())
  .map(e => { const r = e.getBoundingClientRect(); return {x: r.left + r.width / 2, y: r.top + r.height / 2,
    what: e.tagName.toLowerCase() + (e.id ? '#' + e.id : '') + (e.className ? '.' + String(e.className).split(' ')[0] : '')}; })
  .filter(t => t.x > 0 && t.y > 0 && t.x < innerWidth && t.y < innerHeight))"""


async def tap(p, x: float, y: float) -> None:
    for t in ("mousePressed", "mouseReleased"):
        await p.cdp("Input.dispatchMouseEvent", type=t, x=x, y=y, button="left", clickCount=1)


def pages(port: int) -> list[str]:
    return [t["url"] for t in httpx.get(f"http://127.0.0.1:{port}/json/list", timeout=5).json() if t["type"] == "page"]


async def red_webapp(o, rev: str) -> None:
    """This window only: webapp.js as it was at `rev`, for the Door's webapp.js or webapp.<hash>.js."""
    old = subprocess.run(["git", "show", f"{rev}:app/web/webapp/webapp.js"], capture_output=True, check=True,
                         cwd=Path(__file__).resolve().parents[2]).stdout
    body = base64.b64encode(old).decode()
    # The page holds its script to a sha384 (SRI): the document's integrity is made the old script's own, so the
    # control runs what 64d16114 would, and nothing else changes.
    sri = "sha384-" + base64.b64encode(hashlib.sha384(old).digest()).decode()

    async def serve():
        # Page's reader drops events it does not know; a second socket on the same target takes them.
        import websockets
        port = int((o.prof / "DevToolsActivePort").read_text().split()[0])
        t = next(t for t in httpx.get(f"http://127.0.0.1:{port}/json/list", timeout=5).json() if t["type"] == "page")
        async with websockets.connect(t["webSocketDebuggerUrl"], max_size=None) as ws:
            await ws.send(json.dumps({"id": 1, "method": "Fetch.enable", "params": {"patterns": [
                *({"urlPattern": u, "requestStage": "Request"} for u in ("*/webapp.js*", "*/webapp.*.js*")),
                {"urlPattern": "*", "resourceType": "Document", "requestStage": "Response"}]}}))
            n, asked = 1, {}
            async for raw in ws:
                d = json.loads(raw)
                if d.get("method") == "Fetch.requestPaused":
                    q, n = d["params"], n + 1
                    if q.get("resourceType") != "Document":
                        await ws.send(json.dumps({"id": n, "method": "Fetch.fulfillRequest", "params": {
                            "requestId": q["requestId"], "responseCode": 200, "body": body,
                            "responseHeaders": [{"name": "Content-Type", "value": "text/javascript"}]}}))
                    elif q.get("responseStatusCode") == 200:
                        asked[n] = q
                        await ws.send(json.dumps({"id": n, "method": "Fetch.getResponseBody", "params": {"requestId": q["requestId"]}}))
                    else:
                        await ws.send(json.dumps({"id": n, "method": "Fetch.continueRequest", "params": {"requestId": q["requestId"]}}))
                elif d.get("id") in asked:
                    q, r = asked.pop(d["id"]), d.get("result") or {}
                    html = base64.b64decode(r["body"]).decode() if r.get("base64Encoded") else r.get("body", "")
                    html = re.sub(r'(<script src="webapp\.[^"]*" integrity=")[^"]+"', lambda m: m.group(1) + sri + '"', html)
                    n += 1
                    await ws.send(json.dumps({"id": n, "method": "Fetch.fulfillRequest", "params": {
                        "requestId": q["requestId"], "responseCode": 200, "body": base64.b64encode(html.encode()).decode(),
                        "responseHeaders": q.get("responseHeaders") or []}}))
    o.red = asyncio.create_task(serve())
    await asyncio.sleep(1)


async def run(s: w.Stack, results: list) -> None:
    D = s.door_url

    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail, ensure_ascii=False)[:1400]))
        say(sid, "G" if ok else "NG", json.dumps(detail, ensure_ascii=False)[:1400])

    srv = beacon()
    b = f"http://127.0.0.1:{srv.server_address[1]}"
    who = w.signup(s, f"link scheme {TAG}")
    home = who["client"]
    made = []
    o = None
    try:
        for x in posts(b):
            x["title"] = f"link {x['tag']} {TAG}"
            r = home.post("/v2/mint", json={"kind": "post", "draft": {"name": x["title"], "form": x["form"], "link": x["link"]}})
            x["mint"] = r.status_code
            if r.status_code == 200:
                x["id"] = r.json()["object_id"]
                made.append(x)
            else:
                # Core refusing such a link at write is the second half (BUILD's): a refusal is reported, and there is
                # then nothing for the webapp to draw.
                result(x["tag"], not x.get("good"), f"/v2/mint of a {x['form']} post linking {x['link'][:40]!r}",
                       status=r.status_code, why=r.text[:200])
        say("made", len(made), "posts;", "the red control: webapp.js at " + RED if RED else "the Door's webapp.js")
        o = await JR.window(s.work, "link")
        port = int((o.prof / "DevToolsActivePort").read_text().split()[0])
        if RED:
            await red_webapp(o, RED)
        await JR.carry_session(o, D, home)
        await o.p.go(D + "/")
        await o.p.until("!!document.getElementById('tabs') && [...document.querySelectorAll('#tabs button')].some(e => e.textContent.trim() === 'Resources')",
                        "the tabs", 60)
        # The script this page ran (a deployed Door serves it fingerprinted, webapp.<hash>.js), read again through the same path.
        fixed = await o.p.ev("(() => { const s = [...document.scripts].find(s => /\\/webapp\\.([0-9a-f]+\\.)?js/.test(s.src));"
                             " return s ? fetch(s.src).then(r => r.text()).then(t => [s.src, t.includes('function webUrl(')]) : null; })()")
        say("this page's webapp script, and whether it has webUrl():", fixed)
        for x in made:
            await o.p.ev("document.getElementById('pdfX') && !document.getElementById('pdfv').hidden && document.getElementById('pdfX').click()")
            await o.p.until("(() => { const b = [...document.querySelectorAll('#tabs button')].find(e => e.textContent.trim() === 'Resources');"
                            " if (!b) return false; b.click(); return true; })()", "the Resources tab", 20)
            await o.p.until(f"[...document.querySelectorAll('#feed .card')].some(c => c.textContent.includes({json.dumps(x['title'])}))",
                            f"{x['tag']}'s card", 30)
            before, hits0, dialogs0 = pages(port), len(HITS), len([m for m in o.p.said if "dialog" in m])
            await o.p.ev(f"[...document.querySelectorAll('#feed .card')].find(c => c.textContent.includes({json.dumps(x['title'])})).click()")
            await o.p.until(f"!!document.querySelector('#feed .obj h1') && document.querySelector('#feed .obj h1').textContent.includes({json.dumps(x['title'])})",
                            f"{x['tag']}'s page", 20)
            await asyncio.sleep(1.2)
            # A pdf's card opens the viewer over the page: read both, then close it for the page's own taps.
            viewer = None
            if x["form"] == "pdf":
                await o.p.until("!document.getElementById('pdfv').hidden", f"{x['tag']}'s viewer", 20)
                await asyncio.sleep(1.5)
                viewer = await o.p.ev(SINKS + f"({json.dumps(x['link'])})")
                await o.p.shot(s.work, f"V-{x['tag']}")
                if not x.get("good"):
                    for t in await o.p.ev(TAPS + "('#pdfv a, #pdfv button:not(#pdfX)')"):
                        await tap(o.p, t["x"], t["y"])
                        await asyncio.sleep(0.6)
                await o.p.ev("document.getElementById('pdfX').click()")
                await asyncio.sleep(0.5)
            page = await o.p.ev(SINKS + f"({json.dumps(x['link'])})")
            await o.p.shot(s.work, f"P-{x['tag']}")
            tapped = []
            if not x.get("good"):
                for t in await o.p.ev(TAPS + "('#feed .obj a, #feed .obj button:not(.pdfcover), #feed .obj [onclick]')"):
                    tapped.append(t["what"])
                    await tap(o.p, t["x"], t["y"])
                    await asyncio.sleep(0.6)
                await asyncio.sleep(1.5)
            after, hits = pages(port), HITS[hits0:]
            new = [u for u in after if u not in before]
            for u in new:   # a tab a tap opened: closed, so the next post starts from one
                t = next((t for t in httpx.get(f"http://127.0.0.1:{port}/json/list", timeout=5).json() if t["url"] == u), None)
                if t:
                    httpx.get(f"http://127.0.0.1:{port}/json/close/{t['id']}", timeout=5)
            dialogs = len([m for m in o.p.said if "dialog" in m]) - dialogs0
            seen = {"title": page["title"], "go": page["go"], "bad": page["bad"] + ((viewer or {}).get("bad") or []),
                    "pdfOpen": (viewer or {}).get("pdfOpen"), "inert srcs": sorted(set(page["srcs"] + ((viewer or {}).get("srcs") or []))),
                    "broken pictures": page["broken"]}
            if x.get("good"):
                want = "Open the PDF at example.org" if x["form"] == "pdf" else "Open at example.org"
                ok = (page["title"] == x["title"] and [g["href"] for g in page["go"]] == [x["link"]] and page["go"][0]["text"] == want
                      and (x["form"] != "pdf" or (viewer["pdfOpen"]["shown"] and viewer["pdfOpen"]["href"] == x["link"])) and not seen["bad"])
                result(x["tag"], ok, f"a {x['form']} post linking https: drawn as a link, as before", **seen)
            else:
                ok = (page["title"] == x["title"] and not page["go"] and not seen["bad"] and not hits and not new and not dialogs
                      and (not MEDIA or (not [e for e in seen["inert srcs"] if e.split(".")[0] in ("img", "video")] and not seen["broken pictures"]))
                      and (x["form"] != "pdf" or (not viewer["pdfOpen"]["shown"] and viewer["pdfOpen"]["href"] is None)))
                result(x["tag"], ok, f"a {x['form']} post linking {x['link'][:24]!r}…: no link, nothing runs when tapped",
                       **seen, tapped=tapped, beacon=hits, new_tabs=new, dialogs=dialogs)
    finally:
        if o is not None:
            if getattr(o, "red", None):
                o.red.cancel()
            o.close()
        srv.shutdown()
        home.post("/v2/signout")


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP):
        print(__doc__)
        return 2
    bp.map_name()
    s = w.Stack(host="localhost")
    results: list = []
    try:
        s.up()
        asyncio.run(run(s, results))
    except (w.Fail, TimeoutError, OSError) as e:
        results.append(("--", "NG", f"{str(e) or type(e).__name__}"))
    finally:
        kept = s.work
        s.down()
    for sid, st, text in results:
        print(f"  {sid:<10} {st:<3}  {text}")
    print(f"screenshots: {kept}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("link scheme: " + ("G" if not bad else f"NG at {', '.join(bad)}") + (f" (red control, webapp.js at {RED})" if RED else "") + (" (media graded)" if MEDIA else ""))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
