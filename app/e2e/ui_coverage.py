"""Which ICD ops and Door routes the WebApp reaches, measured from its UI.

The loopback system (the Door built from this checkout), headless Chromes each with its own
profile and a virtual PRF passkey. Every step is a click or a keystroke in the page; nothing here
calls the Door. Each page's requests to the Door are recorded off the DevTools protocol, with each
op a /v2/apply or /v2/batch carries and each kind a mint names, and the status the Door answered.

  make ui-coverage            or: .venv/bin/python app/e2e/ui_coverage.py
  E2E_KEEP=1 keeps the run's directory: screenshots, logs, and ui-coverage.json

Prints every ICD op and every route in app/door/routes.json, each with what the UI sent and what
the Door said; a refusal is a finding, printed as one. A lane that could not finish is NG, named.
"""
from __future__ import annotations

import asyncio
import base64
import importlib.util
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import time
import urllib.parse
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
os.environ.setdefault("E2E_IDLE_SECS", "900")         # lanes wait minutes between steps
import contact_code as cc  # noqa: E402
import journeys as J  # noqa: E402
import wallflowers_path as w  # noqa: E402

ROUTES = w.PRODUCT / "app" / "door" / "routes.json"
_spec = importlib.util.spec_from_file_location("icd_coverage", w.PRODUCT / "core" / "coordination" / "icd-coverage.py")
cov = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cov)
say = J.say


class Recorder:
    """A page's requests to the Door, off the DevTools protocol: route, status, and the ops in a write."""

    def __init__(self, who: str, b: J.Browser, origin: str) -> None:
        self.who, self.p, self.origin = who, b.p, origin
        self.open: dict = {}
        self.hits: list[dict] = []
        self.p.tap = self.tap

    async def start(self) -> None:
        await self.p.cdp("Network.enable", maxPostDataSize=65536)

    def tap(self, d: dict) -> None:
        m, e = d["method"], d.get("params", {})
        if m == "Network.requestWillBeSent":
            u = urllib.parse.urlsplit(e["request"]["url"])
            if f"{u.scheme}://{u.netloc}" == self.origin:
                self.open[e["requestId"]] = {"who": self.who, "method": e["request"]["method"], "path": u.path,
                                             "post": e["request"].get("postData"), "has_post": e["request"].get("hasPostData")}
        elif m == "Network.responseReceived" and e["requestId"] in self.open:
            self.open[e["requestId"]]["status"] = e["response"]["status"]
        elif m in ("Network.loadingFinished", "Network.loadingFailed") and e["requestId"] in self.open:
            asyncio.get_running_loop().create_task(self.finish(e["requestId"], m == "Network.loadingFailed"))

    async def finish(self, rid: str, failed: bool) -> None:
        r = self.open.pop(rid)
        r.setdefault("status", 0 if failed else None)
        if r["has_post"] and not r["post"]:
            try:
                r["post"] = (await self.p.cdp("Network.getRequestPostData", requestId=rid)).get("postData")
            except w.Fail:
                r["post"] = None
        reply = None
        if r["path"] == "/v2/batch" and not failed:
            try:
                reply = json.loads((await self.p.cdp("Network.getResponseBody", requestId=rid)).get("body") or "null")
            except (w.Fail, ValueError):
                reply = None
        r["ops"], r["kinds"] = writes(r, reply)
        r.pop("post", None)
        self.hits.append(r)


def writes(r: dict, reply) -> tuple[list, list]:
    """(op, status) and (kind, status) for each write a request carries. In a batch, the steps before
    the refused one stood, the refused one carries the batch's status, and the rest never ran."""
    try:
        body = json.loads(r["post"] or "null")
    except ValueError:
        return [], []
    if not isinstance(body, dict):
        return [], []
    if r["path"] == "/v2/apply":
        return [(body.get("op"), r["status"])], []
    if r["path"] == "/v2/mint":
        return [], [(body.get("kind"), r["status"])]
    if r["path"] != "/v2/batch":
        return [], []
    refused = (reply or {}).get("refused") or {}
    at = refused.get("step") if isinstance(refused, dict) else None
    ops, kinds = [], []
    for i, st in enumerate(body.get("steps") or []):
        status = 200 if at is None or i < at else r["status"] if i == at else None
        if st.get("do") == "apply":
            ops.append((st.get("op"), status))
        elif st.get("do") == "mint":
            kinds.append((st.get("kind"), status))
    return ops, kinds


def route_of(method: str, path: str, routes: list) -> str | None:
    for x in routes:
        if x["method"] == method and re.fullmatch(re.sub(r":[^/]+", "[^/]+", x["path"]), path):
            return f"{x['method']} {x['path']}"
    return None


def report(hits: list) -> tuple[str, list]:
    rows, _ = cov.rows()
    routes = json.loads(ROUTES.read_text())
    routes = routes if isinstance(routes, list) else routes["routes"]
    seen: dict = {}
    for h in hits:
        for op, st in h["ops"]:
            seen.setdefault(op, []).append(st)
    L, findings = [], []
    sent = [r for r in rows if r["op"] in seen]
    L.append(f"ops: {len(sent)} of {len(rows)} sent from the UI")
    width = max(len(r["op"]) for r in rows)
    for r in rows:
        st = seen.get(r["op"])
        L.append(f"  {r['op']:<{width}}  " + (" ".join(str(s) for s in sorted(set(st), key=str)) + f"  ×{len(st)}" if st else "—"))
        if st and any(s is None or not 200 <= s < 300 for s in st):
            findings.append(f"{r['op']}: the Door answered {sorted(set(map(str, st)))}")
    stray = sorted(set(seen) - {r["op"] for r in rows})
    for op in stray:
        findings.append(f"{op}: sent, and not an ICD op")
    made: dict = {}
    for h in hits:
        for kind, st in h["kinds"]:
            made.setdefault(kind, []).append(st)
    L.append("")
    L.append("kinds minted: " + (", ".join(f"{k} {' '.join(map(str, sorted(set(v), key=str)))} ×{len(v)}" for k, v in sorted(made.items())) or "none"))
    # What is not a route is a file the Door serves from the webapp's directory (its fallback).
    by: dict = {}
    files = 0
    for h in hits:
        k = route_of(h["method"], h["path"], routes)
        if k is None and not re.match(r"/(v1|v2|i)/", h["path"]):
            files += 1
            continue
        k = k or f"{h['method']} {h['path']} (no route)"
        by.setdefault(k, []).append(h["status"])
    hit = [x for x in routes if f"{x['method']} {x['path']}" in by]
    L.append("")
    L.append(f"routes: {len(hit)} of {len(routes)} reached from the UI, and {files} webapp files")
    for x in routes:
        k = f"{x['method']} {x['path']}"
        st = by.get(k)
        L.append(f"  {k:<44} {x['router']:<8} " + (" ".join(str(s) for s in sorted(set(st), key=str)) + f"  ×{len(st)}" if st else "—"))
    for k in sorted(k for k in by if k.endswith("(no route)")):
        findings.append(f"{k}: asked by the page, not in routes.json")
    for h in hits:
        if h["method"] == "POST" and not h["ops"] and h["status"] is not None and not 200 <= h["status"] < 300:
            findings.append(f"{h['who']} POST {h['path']}: the Door answered {h['status']}")
    return "\n".join(L), findings


OPT = cc.OPT


def js(v) -> str:
    return json.dumps(v)


async def fill(p, sel: str, value: str) -> None:
    """A field set as typing sets it: the value, then its input event."""
    await p.ev(f"(() => {{ const i = document.querySelector({js(sel)}); i.value = {js(value)}; "
               "i.dispatchEvent(new Event('input', {bubbles: true})); return true; })()")


async def press(p, sel: str) -> None:
    if not await p.ev(f"!!document.querySelector({js(sel)})"):
        raise w.Fail(f"no {sel} on the page")
    await p.ev(f"document.querySelector({js(sel)}).click(), true")


async def setting(p, name: str) -> None:
    """Site settings, then the option named."""
    await p.click("siteRole")
    await p.until(f"{OPT}.includes({js(name)})", f"Site settings' {name}", 15)
    await p.ev(f"[...document.querySelectorAll('#manageBody .opt')].find(b => b.querySelector('b').textContent === {js(name)}).click(), true")


async def open_site(p, name: str) -> None:
    if await p.ev("!document.getElementById('siteRole').hidden"):
        return
    await p.click("siteBtn")
    await p.until(f"[...document.querySelectorAll('#siteMenu .mi')].some(e => e.textContent.includes({js(name)}))", "the Sites menu", 15)
    await p.ev(f"[...document.querySelectorAll('#siteMenu .mi')].find(e => e.textContent.includes({js(name)})).click(), true")
    await p.until("!document.getElementById('siteRole').hidden", "the Site open", 30)


ROW = "[...document.querySelectorAll('#feed .msg')].find(r => (r.querySelector('.mb > p')||{}).textContent === %s)"


async def lane_rooms(A: J.Browser) -> str:
    p, room, text, desc = A.p, f"Room {cc.TAG}", f"Hello {cc.TAG}", f"About {cc.TAG}"
    await setting(p, "New room")
    await p.until("!document.getElementById('sheet').hidden && !!document.getElementById('f_name')", "the room sheet", 30)
    for f in ("#f_name", "#f_0"):
        if await p.ev(f"!!document.querySelector({js(f)})"):
            await fill(p, f, room)
    await p.click("sheetGo")
    await p.until(f"document.getElementById('sheet').hidden && !document.getElementById('compose').hidden"
                  f" || !!document.getElementById('sheetWhy').textContent", "the room open", 60)
    if await p.ev("!!document.getElementById('sheetWhy').textContent"):
        raise w.Fail("the room sheet said: " + await p.ev("document.getElementById('sheetWhy').textContent"))
    await fill(p, "#say", text)
    await p.click("send")
    row = ROW % js(text)
    await p.until(f"!!{row}", "the message posted", 60)
    await p.ev(f"{row}.dispatchEvent(new MouseEvent('contextmenu', {{bubbles: true, cancelable: true}})), true")
    await p.until("!document.getElementById('mmenu').hidden", "the message menu", 10)
    await press(p, '#mmenu .mm-emo button[aria-label="React ❤️"]')
    await p.until(f"!!{row} && [...{row}.querySelectorAll('.rx button')].some(b => b.textContent.startsWith('❤️'))", "the reaction", 60)
    await p.ev("""(() => { document.querySelector('.room-about button[aria-label="Edit description"]').click();
      const a = document.querySelector('.room-edit textarea'); a.value = %s; a.dispatchEvent(new Event('input', {bubbles: true}));
      document.querySelector('.room-edit .save').click(); return true; })()""" % js(desc))
    await p.until(f"(document.querySelector('.room-desc')||{{}}).textContent === {js(desc)}", "the description", 60)
    await p.ev(f"{row}.dispatchEvent(new MouseEvent('contextmenu', {{bubbles: true, cancelable: true}})), true")
    await p.until("!document.getElementById('mmenu').hidden", "the message menu", 10)
    await press(p, "#mmenu .mm-acts button.del")
    await p.until("[...document.querySelectorAll('#mmenu .mm-acts button.del')].some(b => b.textContent === 'Delete for everyone')", "Delete for everyone", 10)
    await p.ev("[...document.querySelectorAll('#mmenu .mm-acts button.del')].find(b => b.textContent === 'Delete for everyone').click(), true")
    await p.until(f"!{row}", "the message gone", 60)
    return "a room made, a message posted, reacted to and deleted, the description edited"


async def person_card(p, name: str) -> None:
    await p.until(f"[...document.querySelectorAll('#rbody .it')].some(b => (b.querySelector('.t')||{{}}).textContent === {js(name)})", f"{name} listed", 60)
    await p.ev(f"[...document.querySelectorAll('#rbody .it')].find(b => (b.querySelector('.t')||{{}}).textContent === {js(name)}).click(), true")
    await p.until("!document.getElementById('person').hidden", f"{name}'s card", 15)


CARD_BTN = "[...document.querySelectorAll('#personBody .pc-alt')].find(b => b.textContent === %s)"


async def lane_admin(A: J.Browser, b_name: str) -> str:
    p = A.p
    await person_card(p, b_name)
    await p.until(f"!!{CARD_BTN % js('Make admin')}", "Make admin", 15)
    await p.ev(f"{CARD_BTN % js('Make admin')}.click(), true")
    await p.until(f"!!{CARD_BTN % js('Remove admin')} && !{CARD_BTN % js('Remove admin')}.disabled", "Remove admin", 90)
    await p.ev(f"{CARD_BTN % js('Remove admin')}.click(), true")
    await p.until(f"!!{CARD_BTN % js('Make admin')} && !{CARD_BTN % js('Make admin')}.disabled", "Make admin again", 90)
    why = await p.ev("(document.getElementById('pcWhy')||{}).textContent || ''")
    await p.click("personX")
    return "B made admin and the role removed" + (f"; the card said: {why}" if why else "")


async def members_page(p) -> None:
    await p.until("!!document.querySelector('#rbody button.sec.go')", "the Members heading", 30)
    await press(p, "#rbody button.sec.go")
    await p.until("!!document.querySelector('#feed .members [data-sec=roster]')", "the Members page", 30)


QROW = "[...document.querySelectorAll('[data-sec=%s] .mem-q')].find(r => (r.querySelector('.qt')||{}).textContent === %s)"


async def lane_members(A: J.Browser, B: J.Browser, site_name: str) -> str:
    bio, q, ans = f"Bio {cc.TAG}", f"Question {cc.TAG}?", f"Answer {cc.TAG}"
    p = A.p
    await members_page(p)
    await press(p, "#feed [data-sec=about] button[data-go=edit]")
    await fill(p, "[data-sec=about] textarea[data-f=bio]", bio)
    await press(p, "[data-sec=about] button[data-go=save]")
    await p.until(f"!document.querySelector('[data-sec=about] textarea') && (document.querySelector('[data-sec=about] .mem-text')||{{}}).textContent === {js(bio)}", "the bio saved", 60)
    await press(p, "[data-sec=questions] button[data-go=new]")
    await p.until("!!document.querySelector('.mem-new[data-edit=define]')", "the question form", 15)
    await fill(p, ".mem-new input[data-f=qtext]", q)
    await press(p, ".mem-new button[data-go=define]")
    await p.until(f"!!{QROW % ('questions', js(q))} && !document.querySelector('.mem-new[data-edit=define]')", "the question defined", 60)
    b = B.p
    await open_site(b, site_name)
    end = time.monotonic() + 90
    while True:
        await members_page(b)
        if await b.ev(f"!!{QROW % ('answers', js(q))}"):
            break
        if time.monotonic() > end:
            raise w.Fail("the question never reached B's Members page")
        await b.ev("[...document.querySelectorAll('#tabs .tab')][0].click(), true")
        await asyncio.sleep(3)
    r = QROW % ("answers", js(q))
    await b.ev(f"{r}.querySelector('button[data-go=edit]').click(), true")
    await b.until(f"!!{r}.querySelector('textarea[data-f=text]')", "the answer field", 15)
    await b.ev(f"(() => {{ const t = {r}.querySelector('textarea[data-f=text]'); t.value = {js(ans)}; t.dispatchEvent(new Event('input', {{bubbles: true}})); "
               f"{r}.querySelector('button[data-go=save]').click(); return true; }})()")
    await b.until(f"(({r}||{{}}).querySelector && ({r}.querySelector('.mem-a .mem-text')||{{}}).textContent) === {js(ans)}", "B's answer saved", 60)
    r = QROW % ("questions", js(q))
    await p.ev(f"{r}.querySelector('button[data-go=retire]').click(), true")
    await p.until("!!document.querySelector('[data-sec=questions] button[data-go=retire-yes]')", "retire, confirmed", 15)
    await press(p, "[data-sec=questions] button[data-go=retire-yes]")
    await p.until(f"[...document.querySelectorAll('[data-sec=questions] .mem-q.retired .qt')].some(e => e.textContent === {js(q)})", "the question retired", 60)
    return "A's bio; A's question, B's answer, A's retirement"


async def lane_face(A: J.Browser) -> str:
    p = A.p
    await setting(p, "Edit the face")
    await p.until("!document.getElementById('faceEd').hidden && !document.getElementById('feSave').disabled && !!document.querySelector('#feMount .fnav-item')"
                  " || !!document.getElementById('feWhy').textContent", "the face editor", 60)
    if await p.ev("!!document.getElementById('feWhy').textContent"):
        raise w.Fail("the face editor said: " + await p.ev("document.getElementById('feWhy').textContent"))
    await p.ev("[...document.querySelectorAll('#feMount .fnav-item')].find(e => (e.querySelector('.fnav-name')||{}).textContent === 'Header').click(), true")
    picker = '#feMount input[type=file][aria-label="Profile image"]'
    await p.until(f"!!document.querySelector({js(picker)})", "the profile image picker", 15)
    await p.ev("""(async () => { const c = document.createElement('canvas'); c.width = c.height = 64;
      const g = c.getContext('2d'); g.fillStyle = '#c33'; g.fillRect(0, 0, 64, 64);
      const b = await new Promise(r => c.toBlob(r, 'image/png')); const dt = new DataTransfer();
      dt.items.add(new File([b], 'mark.png', {type: 'image/png'}));
      const i = document.querySelector(%s); i.files = dt.files; i.dispatchEvent(new Event('change', {bubbles: true})); return true; })()""" % js(picker))
    await p.until("!!document.querySelector('#feMount .fmark-round.has img')", "the picture taken", 30)
    await p.click("feSave")
    await p.until("document.getElementById('faceEd').hidden || !!document.getElementById('feWhy').textContent", "the face saved", 90)
    if await p.ev("!!document.getElementById('feWhy').textContent"):
        raise w.Fail("the face editor said: " + await p.ev("document.getElementById('feWhy').textContent"))
    return "the face and its mark saved"


def png(rgb=(200, 60, 60), size: int = 64) -> bytes:
    """A solid square, as a picker would hand over a photo."""
    raw = b"".join(b"\0" + bytes(rgb) * size for _ in range(size))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)) + \
        chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def pdf() -> bytes:
    """One page, with its words, and the cross-reference a reader checks."""
    text = b"BT /F1 24 Tf 72 720 Td (ui_coverage) Tj ET"
    objs = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
            b"<< /Length %d >>\nstream\n" % len(text) + text + b"\nendstream",
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    out, at = b"%PDF-1.4\n", []
    for i, o in enumerate(objs, 1):
        at.append(len(out))
        out += b"%d 0 obj\n" % i + o + b"\nendobj\n"
    x = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1) + b"".join(b"%010d 00000 n \n" % a for a in at)
    return out + b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, x)


def mp4(work: Path) -> bytes | None:
    """A second of colour, H.264 in MP4 (a clip is video/mp4 only): made here, or None with no ffmpeg."""
    ff = shutil.which("ffmpeg")
    if not ff:
        return None
    out = work / "clip.mp4"
    r = subprocess.run([ff, "-loglevel", "error", "-y", "-f", "lavfi", "-i", "color=c=red:s=64x64:d=1", "-c:v", "libx264",
                        "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(out)], capture_output=True)
    return out.read_bytes() if r.returncode == 0 and out.exists() else None


async def upload(p, sel: str, data: bytes, name: str, mime: str) -> None:
    """A file chosen in a picker: the input's files set, then its change event."""
    if not await p.ev(f"!!document.querySelector({js(sel)})"):
        raise w.Fail(f"no {sel} on the page")
    await p.ev(f"""(() => {{ const i = document.querySelector({js(sel)}); const u = Uint8Array.from(atob({js(base64.b64encode(data).decode())}), c => c.charCodeAt(0));
      const dt = new DataTransfer(); dt.items.add(new File([u], {js(name)}, {{type: {js(mime)}}})); i.files = dt.files;
      i.dispatchEvent(new Event('change', {{bubbles: true}})); return true; }})()""")


async def choose(p, sel: str, text: str) -> None:
    """A select set to the option whose words include `text`, as a person picks it."""
    ok = await p.ev(f"""(() => {{ const s = document.querySelector({js(sel)}); if (!s) return false;
      const o = [...s.options].find(o => o.textContent.includes({js(text)})); if (!o) return false;
      s.value = o.value; s.dispatchEvent(new Event('change', {{bubbles: true}})); return true; }})()""")
    if not ok:
        raise w.Fail(f"no option {text!r} in {sel}")


BTN = "[...document.querySelectorAll(%s)].find(b => b.textContent.trim() === %s)"


async def tap(p, scope: str, label: str) -> None:
    sel = BTN % (js(scope + " button"), js(label))
    if not await p.ev(f"!!{sel}"):
        raise w.Fail(f"no {label!r} button in {scope}")
    await p.ev(f"{sel}.click(), true")


async def home(p, D: str, site: str) -> None:
    await p.go(D + "/#site=" + site)
    await p.until("!document.getElementById('siteRole').hidden", "the Site open", 60)


async def lane_events(A: J.Browser, B: J.Browser, D: str, site: str, a_name: str, clip: bytes | None) -> str:
    p, title = A.p, f"Event {cc.TAG}"
    ee = "#feed .ee"
    await home(p, D, site)
    await setting(p, "New event")
    await p.until(f"!!document.querySelector('{ee} [data-act=publish]')", "the event editor", 30)
    await fill(p, f"{ee} [name=title]", title)
    await press(p, f"{ee} [data-step=when]")
    await fill(p, f"{ee} [name=startMs]", "2026-11-20T19:00")
    await press(p, f"{ee} [data-step=registration]")
    await fill(p, f"{ee} [name=capacity]", "10")
    await press(p, f"{ee} [name=approval]")
    await press(p, f"{ee} [data-step=lineup]")
    await choose(p, f"{ee} [name=act-pick]", a_name)
    await press(p, f"{ee} [data-act=act-add]")
    await p.until(f"!!document.querySelector('{ee} .ee-act')", "the act added", 15)
    await upload(p, f"{ee} input[type=file][name=banner]", png((40, 90, 160)), "banner.png", "image/png")
    await p.until(f"!!document.querySelector('{ee} .ee-cover img.ee-still')", "the banner", 30)
    await press(p, f"{ee} [data-step=media]")
    await upload(p, f"{ee} input[name=photo]", png((60, 160, 90)), "photo.png", "image/png")
    await p.until(f"document.querySelectorAll('{ee} .ee-photos figure').length >= 1", "the photo", 30)
    if clip:
        await upload(p, f"{ee} input[name=clip]", clip, "clip.mp4", "video/mp4")
        await p.until(f"!!document.querySelector('{ee} video.ee-still')", "the clip", 30)
    await press(p, f"{ee} [data-act=publish]")
    await p.until(f"!!document.querySelector('#feed article.ev h1.ev-title') && !document.querySelector('{ee}')"
                  f" || !!document.querySelector('{ee} .ee-why[data-why]')", "the event published", 120)
    if await p.ev(f"!!document.querySelector('{ee} .ee-why[data-why]')"):
        raise w.Fail("publish: " + await p.ev(f"document.querySelector('{ee} .ee-why').textContent"))
    did = ["published" + ("" if clip else " (no clip: no ffmpeg)")]
    # The performer's own confirm: A is the act.
    if await p.ev("!!document.querySelector('#feed .ev-lineup .ev-confirm:not([disabled])')"):
        await press(p, "#feed .ev-lineup .ev-confirm:not([disabled])")
        await p.until("!document.querySelector('#feed .ev-confirm')", "the act confirmed", 60)
        did.append("A confirmed as its act")
    else:
        did.append("no Confirm drawn for A's own act")
    await press(p, "#feed .ev-manage")
    await p.until(f"!!document.querySelector('{ee} [data-act=save]')", "Manage", 30)
    await fill(p, f"{ee} [name=title]", title + " (edited)")
    await press(p, f"{ee} [data-step=media]")
    if await p.ev(f"!!document.querySelector('{ee} [data-act=photo-remove-0]')"):
        await press(p, f"{ee} [data-act=photo-remove-0]")
    await press(p, f"{ee} [data-act=save]")
    await p.until(f"!!document.querySelector('#feed .ev-manage') && !document.querySelector('{ee}')"
                  f" || !!document.querySelector('{ee} .ee-why[data-why]')", "the event saved", 120)
    if await p.ev(f"!!document.querySelector('{ee} .ee-why[data-why]')"):
        raise w.Fail("save: " + await p.ev(f"document.querySelector('{ee} .ee-why').textContent"))
    did.append("edited and saved")
    # B answers from the Site's Events.
    b = B.p
    await home(b, D, site)
    await b.ev("[...document.querySelectorAll('#tabs .tab')].find(t => t.textContent.trim() === 'Events').click(), true")
    try:
        await b.until(f"[...document.querySelectorAll('#feed .card')].some(c => c.textContent.includes({js(title)}))", "the event on B's Events", 60)
    except w.Fail:
        return "; ".join(did + ["B never saw the event: its card is drawn from the Site's public page, which did not go live here"])
    await b.ev(f"[...document.querySelectorAll('#feed .card')].find(c => c.textContent.includes({js(title)})).click(), true")
    await b.until("!!document.querySelector('#feed .ev-rsvp .ev-choice[data-status=going]')", "B's RSVP", 30)
    await press(b, "#feed .ev-rsvp .ev-choice[data-status=going]")
    await b.until("!!document.querySelector('#feed .ev-choice[data-status=going][aria-pressed=true]')", "B going", 60)
    did.append("B going")
    await press(p, "#feed .ev-manage")
    await press(p, f"{ee} [data-step=guests]")
    await p.until(f"!!document.querySelector('{ee} [data-act^=\"decide-approved-\"]')", "B's answer to decide", 90)
    await press(p, f"{ee} [data-act^=\"decide-approved-\"]")
    await p.until(f"!document.querySelector('{ee} [data-act^=\"decide-\"]')", "B approved", 60)
    return "; ".join(did + ["A approved B"])


def field(label: str) -> str:
    return f"[...document.querySelectorAll('#sheetFields label.fld')].find(l => l.querySelector('span').textContent.startsWith({js(label)})).querySelector('input,select')"


async def lane_trade(A: J.Browser, D: str, site: str) -> str:
    p, name = A.p, f"Thing {cc.TAG}"
    await home(p, D, site)
    await setting(p, "New listing")
    await p.until("!document.getElementById('sheet').hidden && document.getElementById('sheetTitle').textContent.includes('New listing')", "the listing sheet", 30)
    await p.until(f"!!{field('name')}", "the listing's fields", 15)
    await p.ev(f"(() => {{ const i = {field('name')}; i.value = {js(name)}; i.dispatchEvent(new Event('input', {{bubbles: true}})); return true; }})()")
    await p.click("sheetGo")
    await p.until("document.getElementById('sheet').hidden && !!document.querySelector('#feed .tedit') || !!document.getElementById('sheetWhy').textContent", "the Thing open", 90)
    if await p.ev("!!document.getElementById('sheetWhy').textContent"):
        raise w.Fail("the sheet said: " + await p.ev("document.getElementById('sheetWhy').textContent"))
    cond = await p.ev("[...document.querySelector('#feed .tedit select[aria-label=Condition]').options].map(o => o.value).filter(Boolean).pop()")
    await p.ev(f"(() => {{ const s = document.querySelector('#feed .tedit select[aria-label=Condition]'); s.value = {js(cond)}; s.dispatchEvent(new Event('change', {{bubbles: true}})); return true; }})()")
    await fill(p, "#feed .tedit textarea[aria-label=Description]", "Made by ui_coverage")
    await tap(p, "#feed .tedit .tact", "Save")
    await p.until(f"(document.querySelector('#feed .tedit select[aria-label=Condition]')||{{}}).value === {js(cond)}", "the profile saved", 60)
    await upload(p, "#feed .gallery input.addpic", png((90, 60, 160)), "thing.png", "image/png")
    await p.until("document.querySelectorAll('#feed .gallery figure').length === 1", "the photo added", 60)
    await press(p, "#feed .gallery figure .tact button.x")
    await p.until("document.querySelectorAll('#feed .gallery figure').length === 0", "the photo removed", 60)
    await p.ev("(() => { const s = document.querySelector('#feed select[aria-label=\"For sale\"]'); s.value = 'sold'; s.dispatchEvent(new Event('change', {bubbles: true})); return true; })()")
    await tap(p, "#feed .trow .tact", "Set")
    await p.until("!!document.querySelector('#feed .pill.disp.sold')", "sold", 60)
    listed = await p.ev("!!document.querySelector('#feed .tlist button')")
    return "a Thing made, its profile saved, a photo added and removed, marked sold" + ("" if listed else "; no List on the Site drawn")


async def lane_resources(A: J.Browser, D: str, site: str) -> str:
    p, title = A.p, f"Resource {cc.TAG}"
    bar = "#resEd .bar"
    picker = "#resEd .wf-res > input[type=file][accept=\"image/*\"]"
    await home(p, D, site)
    await setting(p, "New resource")
    await p.until("!document.getElementById('resEd').hidden && !!document.querySelector('#resEd .wf-res .bar')", "the resource editor", 30)
    await fill(p, "#resEd textarea.title", title)
    await p.until(f"!!{BTN % (js(bar + ' button'), js('Save draft'))} && !{BTN % (js(bar + ' button'), js('Save draft'))}.disabled", "Save draft ready", 30)
    await p.ev("(() => { const b = document.querySelector('#resEd .body[contenteditable]'); b.focus(); document.execCommand('insertText', false, 'Hello from ui_coverage'); return true; })()")
    await tap(p, "#resEd .cover", "+ Cover")
    await upload(p, picker, png((160, 120, 40)), "cover.png", "image/png")
    await p.until("!!document.querySelector('#resEd .cover img')", "the cover", 30)
    await press(p, "#resEd .tools button[aria-label=Picture]")
    await upload(p, picker, png((40, 160, 160)), "inline.png", "image/png")
    await p.until("!!document.querySelector('#resEd .body figure img[data-asset]')", "the inline picture", 30)
    await upload(p, "#resEd .doc input[type=file]", pdf(), "doc.pdf", "application/pdf")
    await p.until("document.querySelector('#resEd .doc').classList.contains('has')", "the PDF", 60)
    state = "(document.querySelector('#resEd .state')||{}).textContent"
    why = "(document.querySelector('#resEd .why')||{}).textContent"
    await tap(p, bar, "Save draft")
    await p.until(f"{state}.startsWith('Draft') && !{state}.includes('edited') || !!{why}", "the draft saved", 120)
    if await p.ev(f"!!{why}"):
        raise w.Fail("save draft: " + await p.ev(why))
    await p.ev("(() => { const b = document.querySelector('#resEd .body'); b.querySelector('figure').remove(); b.dispatchEvent(new Event('input', {bubbles: true})); return true; })()")
    await tap(p, bar, "Save draft")
    await p.until(f"{state}.startsWith('Draft') && !{state}.includes('edited') || !!{why}", "the picture's removal saved", 120)
    await fill(p, "#resEd textarea.title", title + " (published)")
    await tap(p, bar, "Publish")
    await p.until(f"{state}.startsWith('Published') || !!{why}", "published", 120)
    if await p.ev(f"!!{why}"):
        raise w.Fail("publish: " + await p.ev(why))
    await tap(p, bar, "Unpublish")
    await p.until(f"{state}.startsWith('Draft') || !!{why}", "unpublished", 120)
    if await p.ev(f"!!{why}"):
        raise w.Fail("unpublish: " + await p.ev(why))
    return "a resource drafted with a cover, a picture and a PDF; the picture removed; published; unpublished"


async def run(s: w.Stack, results: list, hits: list) -> None:
    result = cc.resulter(results)
    opened: list[J.Browser] = []
    rec: list[Recorder] = []

    async def browser(tag: str) -> J.Browser:
        # Nothing resolves but localhost: the page names production hosts (its Arc, www), and a
        # probe on loopback must never reach them.
        pinned, J.bp.pinned = J.bp.pinned, lambda to="": ["--headless=new", "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost"]
        try:
            b = await J.Browser().open(s.work, tag)
        finally:
            J.bp.pinned = pinned
        opened.append(b)
        r = Recorder(tag, b, s.door_url)
        await r.start()
        rec.append(r)
        return b

    try:
        B = await browser("B")
        code, _ = await cc.copy_code(B, s, "Bo " + cc.TAG)
        A = await browser("A")
        # A face with a banner, as www's form hands one over: the face to the Site, the banner to the Host.
        face = json.dumps({"header": {"banner": "data:image/png;base64," + base64.b64encode(png((30, 30, 120))).decode()}})
        site, _ = await cc.register(A, s, face)
        result("U1", bool(site), "A registers a Site in the window; B makes a contact code", site=site)
        await cc.add_by_code(A, code)
        await A.p.until("document.getElementById('sheet').hidden || !!document.getElementById('sheetWhy').textContent", "the add", 120)
        await cc.held(B, [site])
        result("U2", site in {o.get("id") for o in (await B.graph()).get("objects", [])}, "A adds B by the code")

        async def lane(sid: str, what: str, body) -> None:
            try:
                result(sid, True, what, did=await body())
            except (w.Fail, TimeoutError) as e:
                for who, b in (("A", A), ("B", B)):
                    await b.p.shot(s.work, f"{sid}-{who}-failed")
                result(sid, False, what, failed=str(e) or type(e).__name__, page=A.p.said[-3:])

        await lane("U3", "rooms", lambda: lane_rooms(A))
        await lane("U4", "admin", lambda: lane_admin(A, "Bo " + cc.TAG))
        await lane("U5", "members", lambda: lane_members(A, B, cc.NAME))
        await lane("U6", "the face", lambda: lane_face(A))
        await lane("U7", "events", lambda: lane_events(A, B, s.door_url, site, "Ada " + cc.TAG, mp4(s.work)))
        await lane("U8", "trade", lambda: lane_trade(A, s.door_url, site))
        await lane("U9", "resources", lambda: lane_resources(A, s.door_url, site))
    finally:
        await asyncio.sleep(2)                        # the last responses' bodies
        for r in rec:
            hits.extend(r.hits)
        for b in opened:
            b.close()
        say("every Chrome closed")


def main() -> int:
    if w.DEPLOYED:
        print("ui_coverage runs on the loopback system only")
        return 2
    s = w.Stack(host="localhost")
    results: list = []
    hits: list = []
    try:
        s.build()
        s.up()
        asyncio.run(asyncio.wait_for(run(s, results, hits), 1800))
    except Exception as e:                            # any step that could not go on is an NG, named
        results.append(("--", "NG", f"{type(e).__name__}: {e}"))
    finally:
        kept = s.work
        s.down()
    table, findings = report(hits)
    print(table)
    print("")
    for sid, st, text in results:
        print(f"  {sid:<3} {st:<3}  {text}")
    for f in findings:
        print(f"  finding: {f}")
    if os.environ.get("E2E_KEEP"):
        (kept / "ui-coverage.json").write_text(json.dumps({"hits": hits, "results": results, "findings": findings}, indent=1))
        print(f"kept: {kept}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("ui coverage: " + ("G" if not bad else f"NG at {', '.join(bad)}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
