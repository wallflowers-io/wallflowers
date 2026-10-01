"""The redesign's PDF viewer (2.2.2, WEBAPP_HUMAN) on door-test, against a PDF whose server sends CORS.

The site's PDF host is stood in on loopback: a one-page PDF written here, served with
Access-Control-Allow-Origin for the Door's origin, ranges, and an answer to Chrome's Private Network
Access preflight (the page's origin maps to door-test's private address), every request logged. The
Site's owner (restate_timing.py's layer-1 account) makes a pdf post linking to it, declared at both
ends as the webapp's create() writes one; then, in the owner's window (its session's cookie carried
in), the Site's Resources tab and that post's card: the viewer (#pdfv) must say "N pages · <host>"
and draw its canvases, and the stand-in must have been fetched from the Door's origin. The negative:
the Site's pdf post on example.org (no CORS, and nothing resolves there in this Chrome) must show the
viewer's failure line, its "open where it lives" link still the post's.

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  RESTATE_STATE=<the owner's file> [PDF_NEGATIVE=https://example.org/egregore-resources.pdf]
  .venv/bin/python app/e2e/pdf_viewer.py
"""
from __future__ import annotations

import asyncio
import http.server
import json
import os
import re
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import entry_qr as E  # noqa: E402
import ja_rehearsal as JR  # noqa: E402
import wallflowers_path as w  # noqa: E402

STATE = Path(os.environ.get("RESTATE_STATE", ""))
NEGATIVE = os.environ.get("PDF_NEGATIVE", "https://example.org/egregore-resources.pdf")
# One port for the stand-in across runs: the post's link stays good, and EGREGORE's site reads the same post
# from the Face's items (ja_rehearsal.py's JA_UI round serves it again on this port).
PORT = int(os.environ.get("PDF_PORT", "8931"))
TITLE = "Stand-in PDF, CORS"
TAG = time.strftime("%m%d%H%M", time.gmtime())
say = JR.say
HITS: list = []


def one_page_pdf(text: str) -> bytes:
    stream = f"BT /F1 18 Tf 50 750 Td ({text}) Tj ET".encode()
    objs = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
            b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"\nendstream",
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    out, offs = bytearray(b"%PDF-1.4\n"), []
    for i, o in enumerate(objs, 1):
        offs.append(len(out))
        out += b"%d 0 obj\n" % i + o + b"\nendobj\n"
    x = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1) + b"".join(b"%010d 00000 n \n" % o for o in offs)
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, x)
    return bytes(out)


def stand_in(origin: str = "*", port: int = PORT) -> tuple[http.server.ThreadingHTTPServer, str]:
    """ACAO * by default: neither viewer sends credentials, and the site's origin is not the Door's."""
    body = one_page_pdf("WallFlowers rehearsal PDF (stand-in)")

    class H(http.server.BaseHTTPRequestHandler):
        def cors(self):
            self.send_header("Access-Control-Allow-Origin", origin)
            self.send_header("Access-Control-Expose-Headers", "Content-Length, Content-Range, Accept-Ranges")
            self.send_header("Accept-Ranges", "bytes")

        def do_OPTIONS(self):
            HITS.append(("OPTIONS", self.headers.get("Origin"), self.headers.get("Access-Control-Request-Private-Network")))
            self.send_response(204)
            self.cors()
            self.send_header("Access-Control-Allow-Headers", "Range")
            self.send_header("Access-Control-Allow-Methods", "GET, HEAD")
            if self.headers.get("Access-Control-Request-Private-Network"):
                self.send_header("Access-Control-Allow-Private-Network", "true")
            self.end_headers()

        def do_GET(self):
            HITS.append(("GET", self.headers.get("Origin"), self.headers.get("Range")))
            rng = self.headers.get("Range")
            if rng and rng.startswith("bytes="):
                a, _, b = rng[6:].partition("-")
                a, b = int(a or 0), (int(b) if b else len(body) - 1)
                part = body[a:b + 1]
                self.send_response(206)
                self.cors()
                self.send_header("Content-Range", f"bytes {a}-{a + len(part) - 1}/{len(body)}")
            else:
                part = body
                self.send_response(200)
                self.cors()
            self.send_header("Content-Type", "application/pdf")
            self.send_header("Content-Length", str(len(part)))
            self.end_headers()
            self.wfile.write(part)

        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", port), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, f"http://127.0.0.1:{srv.server_address[1]}/resources.pdf"


async def open_pdf(o, work: Path, D: str, site: str, title: str, tag: str) -> dict:
    await o.p.go(D + "/#site=" + site)
    await o.p.until("!!document.getElementById('tabs') && [...document.querySelectorAll('#tabs *')].some(e => e.textContent.trim() === 'Resources')",
                    "the Site's tabs", 60)
    # The tab, in #tabs: the Site also has a room named Resources, which a plain text match would open instead.
    await o.p.until("(() => { const b = [...document.querySelectorAll('#tabs button, #tabs [role=tab], #tabs a')]"
                    ".find(e => e.textContent.trim() === 'Resources'); if (!b) return false; b.click(); return true; })()",
                    "the Resources tab", 20)
    await o.p.until(f"[...document.querySelectorAll('#feed .card')].some(c => c.textContent.includes({json.dumps(title)}))", "the pdf post's card", 30)
    await o.p.shot(work, f"R-{tag}")
    await o.p.ev(f"[...document.querySelectorAll('#feed .card')].find(c => c.textContent.includes({json.dumps(title)})).click()")
    await o.p.until("!!document.getElementById('pdfv') && !document.getElementById('pdfv').hidden", "the viewer", 20)
    t0 = time.monotonic()
    while time.monotonic() - t0 < 20:
        sub = await o.p.ev("(document.getElementById('pdfSub') || {}).textContent || ''")
        if sub and not sub.startswith("Loading"):
            break
        await asyncio.sleep(0.5)
    await asyncio.sleep(2)
    await o.p.shot(work, f"V-{tag}")
    got = await o.p.ev("""(() => ({title: (document.getElementById('pdfTitle') || {}).textContent || '',
      sub: (document.getElementById('pdfSub') || {}).textContent || '',
      drawn: [...document.querySelectorAll('#pdfPages canvas')].filter(c => c.width > 0).length,
      open: (document.getElementById('pdfOpen') || {}).href || ''}))()""")
    await o.p.ev("(document.getElementById('pdfX') || {click() {}}).click()")
    return got


async def run(s: w.Stack, results: list) -> None:
    D = s.door_url
    st = json.loads(STATE.read_text())
    site = st["site"]
    owner = {"handle": st["owner"]["handle"], "pk": st["owner"]["pk"], "prf": bytes.fromhex(st["owner"]["prf"])}

    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail, ensure_ascii=False)[:1200]))
        say(sid, "G" if ok else "NG", json.dumps(detail, ensure_ascii=False)[:1200])

    srv, link = stand_in()
    home = w.signin(s, owner)["client"]
    o = None
    try:
        # The post, once: made where the owner holds none linking here, kept for later runs and the site.
        title = TITLE
        held = [x for x in home.get("/v2/graph").json().get("objects", []) if x.get("kind") == "post"
                and ((x.get("view") or {}).get("link") == link)]
        if not held:
            at = int(time.time() * 1000)
            r = home.post("/v2/mint", json={"kind": "post", "draft": {"name": title, "form": "pdf", "link": link}})
            post = r.json()["object_id"]
            for obj, op, args in ((site, "group.setAffiliation", {"peer": post, "rel": "created", "name": title, "at": at}),
                                  (post, "base.setBacklink", {"object": site, "rel": "created", "at": at})):
                if home.post("/v2/apply", json={"object": obj, "op": op, "args": args}).status_code != 200:
                    raise w.Fail(f"{op} refused")
        else:
            title = held[0].get("name") or title
        o = await JR.window(s.work, "owner")
        await JR.carry_session(o, D, home)
        good = await open_pdf(o, s.work, D, site, title, "cors")
        fetched = [h for h in HITS if h[0] == "GET" and h[1] == D]   # the fetch came from the Door's page
        result("P1", good["title"] == title and bool(re.match(r"1 pages? ·", good["sub"])) and good["drawn"] >= 1 and bool(fetched),
               "the viewer draws a pdf served with CORS: its pages, and the fetch from the Door's origin",
               **good, requests=[list(h) for h in HITS][:6], link=link)
        bad = await open_pdf(o, s.work, D, site, "Community resources (PDF)", "no-cors")
        result("P2", "could not be shown" in bad["sub"] and bad["drawn"] == 0 and bad["open"].startswith(NEGATIVE),
               "a pdf whose host sends no CORS (or cannot be reached): the failure line, the link to where it lives", **bad)
        # P3 (WEBAPP_HUMAN, BUILD): the Site's settings, the face editor, its colour picker; cancelled, nothing saved.
        try:
            await o.p.go(D + "/#site=" + site)
            await o.p.until("!!document.getElementById('siteRole')", "the Site's role badge", 30)
            await o.p.click("siteRole")
            await o.p.until("!!document.getElementById('manage') && document.getElementById('manage').checkVisibility()", "Site settings", 15)
            await o.p.ev("([...document.querySelectorAll('#manage .opt')].find(e => e.textContent.includes('Edit the face')) || {click() {}}).click()")
            await o.p.until("!!document.querySelector('#faceEd .face-editor .face-phone') || !!(document.getElementById('feWhy') || {}).textContent",
                            "the face editor", 30)
            why = await o.p.ev("(document.getElementById('feWhy') || {}).textContent || ''")
            await o.p.ev(f"{E.CLICK}('Colours')")
            await asyncio.sleep(0.8)
            await o.p.ev("(document.querySelector('.fcolour-input') || {click() {}}).click()")
            await asyncio.sleep(1)
            pick = await o.p.ev("""(() => { const c = document.querySelector('.clr-picker.clr-open'); if (!c || !c.checkVisibility()) return null;
              const s = [...c.querySelectorAll('.clr-swatches button')][0], cs = s && getComputedStyle(s);
              return {open: true, swatch: cs ? [s.getBoundingClientRect().width, cs.borderRadius] : null}; })()""")
            await o.p.shot(s.work, "F-picker")
            await o.p.ev("(document.getElementById('feCancel') || {click() {}}).click()")
            result("P3", bool(pick) and not why, "the face editor opens from Site settings, and a colour field opens its picker (cancelled)",
                   picker=pick, fe_why=why or None)
        except w.Fail as e:
            await o.p.shot(s.work, "F-stuck")
            result("P3", False, "the face editor's colour picker", why=str(e)[:300])
    finally:
        if o is not None:
            o.close()
        srv.shutdown()
        home.post("/v2/signout")


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP and STATE.is_file()):
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
        print(f"  {sid:<4} {st:<3}  {text}")
    print(f"screenshots: {kept}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("pdf viewer: " + ("G" if not bad else f"NG at {', '.join(bad)}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
