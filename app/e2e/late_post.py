"""Another member's post in a newcomer's open room: when it reaches his fold, his page's stream, and the room (NC-133).

A newcomer by a financial claim, the one-tap sign-up and his card, handed to EGREGORE's site, in the egg's
Financial Support room; EGREGORE's trap (29 Sep) in his page before any script records each /v2/events opened,
each `changed`, and each stream's end. Then LATE_TRIALS times, the Site's owner (layer 1) posts there, and three
clocks run from the post: the page's scoped fold (WallFlowers.current's /v2/graph), the page's next `changed`, and
the room drawing it. LATE_NEWCOMERS newcomers, each in a window of his own: which stream a room ends up on is
decided once per page (NC-133). LATE_OFFLINE=1 then, in the last newcomer's page, ends every /v2/events it holds
(the trap's AbortControllers: a connection lost, the session kept), and the owner posts again.

  L1  every post drawn within LATE_WITHIN seconds (5 by default), for every newcomer
  L2  (LATE_OFFLINE) with the page's streams ended, the next post drawn within 20 s

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  RESTATE_STATE=<the owner's file> JA_KIOSK=<egregore checkout: its egg/kiosk/claim.mjs signs the claim>
  JA_SITE=http://127.0.0.1:3471 [LATE_TRIALS=5] [LATE_OFFLINE=1]
  .venv/bin/python app/e2e/late_post.py
"""
from __future__ import annotations

import asyncio
import json
import os
import sys
import time
import urllib.parse
from pathlib import Path

os.environ.setdefault("JA_CARD", "2.2.2")
sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import ja_rehearsal as JR  # noqa: E402
import journeys as J  # noqa: E402
import wallflowers_path as w  # noqa: E402

STATE = Path(os.environ.get("RESTATE_STATE", ""))
TRIALS = int(os.environ.get("LATE_TRIALS", "5"))
NEWCOMERS = int(os.environ.get("LATE_NEWCOMERS", "1"))
WITHIN = float(os.environ.get("LATE_WITHIN", "5"))
OFFLINE = os.environ.get("LATE_OFFLINE") == "1"
TAG = time.strftime("%m%d%H%M%S", time.gmtime())
say = JR.say
# EGREGORE's trap: wraps each session the Door's element hands out, and tees each /v2/events body for its end.
TRAP = r"""(() => {
  const log = (window.__eggEvents = []);
  const wrap = (s) => s && Object.assign(s, { events: ((orig) => (cb) => orig.call(s, (v) => { log.push({ t: Date.now(), kind: 'changed', v }); cb(v); }))(s.events) });
  let wf;
  Object.defineProperty(window, 'WallFlowers', { configurable: true, get: () => wf,
    set: (x) => { wf = x && { ...x, current: (o) => x.current(o).then(wrap), finish: (o) => x.finish(o).then(wrap) }; } });
  const f = window.fetch;
  window.__eggStreams = [];
  window.fetch = async (...a) => {
    const u = String(a[0]?.url ?? a[0]);
    if (u.endsWith('/v2/events')) {   // TEST: a handle to end this stream as a lost connection would
      const ac = new AbortController(); window.__eggStreams.push(ac);
      a[1] = { ...(a[1] || {}), signal: ac.signal };
    }
    const r = await f(...a);
    if (!u.endsWith('/v2/events')) return r;
    log.push({ t: Date.now(), kind: 'open', status: r.status });
    if (!r.body) return r;
    const [mine, theirs] = r.body.tee();
    (async () => { const rd = mine.getReader(); for (;;) { const x = await rd.read(); if (x.done) { log.push({ t: Date.now(), kind: 'ended' }); return; } } })()
      .catch((e) => log.push({ t: Date.now(), kind: 'ended', error: String(e) }));
    return new Response(theirs, { status: r.status, statusText: r.statusText, headers: r.headers });
  };
})();"""
SAY = "dialog[open] section[lang] ol[class*=thread] > li p[class*=say]"


def drawn(text: str) -> str:
    return f"[...document.querySelectorAll({json.dumps(SAY)})].some(p => p.textContent.includes({json.dumps(text)}))"


def in_fold(text: str) -> str:
    return (f"WallFlowers.current({{client: 'egregore-local'}}).then(s => s.fetch('/v2/graph')).then(r => r.text())"
            f".then(t => t.includes({json.dumps(text)}), () => null)")


async def newcomer(s: w.Stack, v, site: str) -> None:
    """The newcomer, in the egg's Financial Support room: J3–J5's path, and J7's room."""
    D = s.door_url
    await v.p.cdp("Page.addScriptToEvaluateOnNewDocument", source=TRAP)
    await v.p.go(D + "/join?claim=" + urllib.parse.quote(J.claim(site, "financial", None), safe=""))
    await v.p.go(D + "/signin?new&return=" + urllib.parse.quote("/#join", safe=""))
    await v.sign_up()
    await v.p.until(JR.CARD_OPEN, "the card", 60)
    await v.p.until(f"(() => {{ const b = document.querySelector({json.dumps(JR.CARD['save'])}); return !!b && !b.disabled; }})()", "Save", 40)
    await v.p.ev(f"(() => {{ const i = document.querySelector({json.dumps(JR.CARD['name'])}); i.value = {json.dumps('Late post ' + TAG)}; "
                 "i.dispatchEvent(new Event('input', {bubbles: true})); })()")
    await v.p.ev(f"document.querySelector({json.dumps(JR.CARD['save'])}).click()")
    site_js = json.dumps(JR.SITE)
    t0 = time.monotonic()
    while time.monotonic() - t0 < 90:
        if await v.p.ev(f"location.origin === {site_js} && (({JR.LANDED}) || ({JR.WELCOME}))"):
            break
        if await v.p.ev(f"location.origin === {json.dumps(D)} && !!document.getElementById('in') && !document.getElementById('start').hidden "
                        "&& !document.getElementById('in').disabled && performance.now() > 8000"):
            await v.p.click("in")
        await asyncio.sleep(0.5)
    if await v.p.ev(f"location.origin === {site_js} && ({JR.WELCOME})"):
        await v.p.ev(f"{JR.DIALBTN}.find(x => /ENGLISH/i.test(x.textContent)).click()")
        await v.p.until(f"{JR.DIALBTN}.some(x => x.textContent === 'FORUMS')", "the dial", 30)
        await v.p.ev(f"{JR.DIALBTN}.find(x => x.textContent === 'FORUMS').click()")
    await v.p.until(f"location.origin === {site_js} && ({JR.LANDED})", "the community", 30)
    await v.p.until(f"{JR.ROWS}.length > 0", "the rooms", 45)
    await v.p.ev("(() => { const all = " + JR.ROWS + "; (all.find(x => /Financial Support/.test(x.textContent)) || all[0]).click(); })()")
    await v.p.until(f"document.querySelectorAll({json.dumps(SAY)}).length > 0", "the room's thread", 30)


async def trial(v, owner, room: str, text: str, secs: float = 30) -> dict:
    t0 = time.time()
    r = owner.post("/v2/apply", json={"object": room, "op": "forum.post", "args": {"text": text}})
    out = {"apply": r.status_code, "fold_ms": None, "changed_ms": None, "drawn_ms": None}
    last_fold = 0.0
    while time.time() - t0 < secs and out["drawn_ms"] is None:
        now = time.time()
        if out["drawn_ms"] is None and await v.p.ev(drawn(text)):
            out["drawn_ms"] = round((time.time() - t0) * 1000)
        if out["fold_ms"] is None and now - last_fold > 0.5:
            last_fold = now
            if await v.p.ev(in_fold(text)):
                out["fold_ms"] = round((time.time() - t0) * 1000)
        await asyncio.sleep(0.1)
    ev = await v.p.ev("window.__eggEvents || []")
    after = [e for e in ev if e["t"] >= t0 * 1000]
    ch = next((e for e in after if e["kind"] == "changed"), None)
    out["changed_ms"] = round(ch["t"] - t0 * 1000) if ch else None
    return out


async def run(s: w.Stack, results: list) -> None:
    st = json.loads(STATE.read_text())
    site, room = st["site"], st["rooms"]["Financial Support"]
    owner = w.signin(s, {"handle": st["owner"]["handle"], "pk": st["owner"]["pk"], "prf": bytes.fromhex(st["owner"]["prf"])})["client"]
    v = None

    def result(sid, ok, what, **detail):
        results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail)[:2400]))
        say(sid, "G" if ok else "NG", json.dumps(detail)[:2400])

    def streams(ev: list, since: float | None = None) -> dict:
        """Opened and ended; the most open at once (by the trap's order); and ends since `since` (ms), the room open."""
        n = top = 0
        for e in sorted(ev, key=lambda e: e["t"]):
            n += {"open": 1, "ended": -1}.get(e["kind"], 0)
            top = max(top, n)
        out = {"opened": sum(e["kind"] == "open" for e in ev), "ended": sum(e["kind"] == "ended" for e in ev), "max_open": top}
        if since is not None:
            out["ended_while_room_open"] = sum(e["kind"] == "ended" and e["t"] >= since for e in ev)
        return out

    try:
        pages = []
        for n in range(NEWCOMERS):
            last = n == NEWCOMERS - 1
            # Opened here, so the finally below closes it however the newcomer's path fails.
            v = await JR.window(s.work, f"late-newcomer-{n + 1}")
            try:   # bounded: run 98's sixth newcomer stood at the Door's /#join for 17 minutes
                await asyncio.wait_for(newcomer(s, v, site), 180)
            except (w.Fail, TimeoutError) as e:
                await v.p.shot(s.work, f"L-stuck-{n + 1}")
                pages.append({"newcomer": n + 1, "stuck": (str(e) or type(e).__name__)[:200], "at": await v.p.ev("location.href")})
                say(f"newcomer {n + 1} stuck", json.dumps(pages[-1]))
                v.close()
                v = None
                continue
            await asyncio.sleep(5)
            room_open = time.time() * 1000
            trials = []
            for i in range(TRIALS):
                trials.append(await trial(v, owner, room, f"late post {TAG} n{n + 1} t{i + 1}"))
                await asyncio.sleep(2)
            ev = await v.p.ev("window.__eggEvents || []")
            pages.append({"newcomer": n + 1, "streams": streams(ev, room_open), "trials": trials})
            say(f"newcomer {n + 1}", json.dumps(pages[-1]))
            await v.p.shot(s.work, f"L-room-{n + 1}")
            if last and OFFLINE:
                await v.p.ev("(window.__eggStreams || []).forEach(c => c.abort()); true")
                await asyncio.sleep(2)
                t = await trial(v, owner, room, f"late post {TAG} after the streams ended", secs=30)
                ev = await v.p.ev("window.__eggEvents || []")
                result("L2", t["drawn_ms"] is not None and t["drawn_ms"] <= 20000,
                       "with the page's streams ended (the session kept), the next post drawn within 20 s", **t, streams=streams(ev))
            v.close()
            v = None
        every = [t for p in pages for t in p.get("trials", [])]
        ok = all(t["drawn_ms"] is not None and t["drawn_ms"] <= WITHIN * 1000 for t in every)
        missed = [p["newcomer"] for p in pages if any(t["drawn_ms"] is None or t["drawn_ms"] > WITHIN * 1000 for t in p.get("trials", []))]
        stuck = [p["newcomer"] for p in pages if "stuck" in p]
        result("L1", ok, f"another member's post drawn in each newcomer's open room within {WITHIN:g} s, every time",
               drawn=f"{sum(t['drawn_ms'] is not None and t['drawn_ms'] <= WITHIN * 1000 for t in every)} of {len(every)}",
               newcomers_missing=missed, newcomers_stuck=stuck, pages=pages)
    finally:
        if v is not None:
            v.close()
        owner.post("/v2/signout")


def main() -> int:
    claim_mjs = Path(os.environ.get("JA_KIOSK", "")).resolve() / "egg" / "kiosk" / "claim.mjs"
    if not (w.DEPLOYED and bp.DOOR_IP and STATE.is_file() and claim_mjs.is_file()):
        print(__doc__)
        return 2
    w.CLAIM_MJS = claim_mjs   # as ja_rehearsal.py's main sets it: the rehearsal key's claims, signed by the kiosk's own code
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
        print(f"  {sid:<3} {st:<3}  {text}")
    print(f"screenshots: {kept}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("late post: " + ("G" if not bad else f"NG at {', '.join(bad)}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
