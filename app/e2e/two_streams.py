"""Two change streams on one site token: each must hear its scope, and neither may end (UX's timings, 29 Sep).

A site's page may open /v2/events more than once on one token (egregore's rail and its rooms each do). The Door
keeps one watcher per session ref, so a second stream replaces the first, whose response then ends; the
webapp's element never reconnects, and a room on the first stream hears no one else's post again.

On a deployed Door, with a client it registers (E2E_CLIENT, E2E_CALLBACK): a site's sign-up and its DPoP token
(as E15), then
  S1  one stream, then a write in the token's scope (a mint): the stream says `changed` (the control)
  S2  a second stream on the same token, then another write: both streams say `changed`, and the first has
      not ended
  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  E2E_CLIENT=egregore-local E2E_CALLBACK=http://127.0.0.1:3471/signin/callback .venv/bin/python app/e2e/two_streams.py
"""
from __future__ import annotations

import hashlib
import json
import os
import sys
import threading
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import wallflowers_path as w  # noqa: E402


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def token_of(s: w.Stack) -> tuple:
    """A site's sign-up for CLIENT, its code, and a DPoP token (E15's path, no wait on the words)."""
    c = w.webapp(s)
    o = c.post("/v2/signup", json={"name": "two streams", **w.work(c, "signup")}).json()
    verifier, state = w.b64u(os.urandom(32)), os.urandom(8).hex()
    cb = {"client": w.CLIENT, "redirect_uri": w.CALLBACK, "state": state,
          "code_challenge": w.b64u(hashlib.sha256(verifier.encode()).digest())}
    r = c.post("/v2/signup/finish", json={"attempt": o["attempt"], "sealed": w.seal(o["key"], os.urandom(32), o["attempt"]), "client": cb})
    body = r.json()
    r = c.post("/v2/signup/continue", json={"continue": body["continue"]})
    q = parse_qs(urlsplit(r.json()["redirect"]).query)
    k, url = w.dpop_key(), f"{s.door_url}/v2/token"
    r = httpx.post(url, json={"code": q["code"][0], "code_verifier": verifier, "client": w.CLIENT, "redirect_uri": w.CALLBACK},
                   headers={"dpop": w.dpop(k, "POST", url)}, verify=s.verify, timeout=30)
    if r.status_code != 200:
        raise w.Fail(f"/v2/token: {r.status_code} {r.text[:200]}")
    return c, k, r.json()["access_token"]


class Stream:
    """One /v2/events response, read on its own thread: every event, and when (if) it ended."""

    def __init__(self, s: w.Stack, k, token: str, name: str) -> None:
        self.name, self.events, self.ended, self.status = name, [], None, None
        url = f"{s.door_url}/v2/events"
        self.headers = {"authorization": f"DPoP {token}", "dpop": w.dpop(k, "GET", url, token), "accept": "text/event-stream"}
        self.url, self.verify = url, s.verify
        self.t = threading.Thread(target=self._read, daemon=True)
        self.t.start()

    def _read(self) -> None:
        try:
            with httpx.stream("GET", self.url, headers=self.headers, verify=self.verify, timeout=httpx.Timeout(10, read=None)) as r:
                self.status = r.status_code
                ev = None
                for line in r.iter_lines():
                    if line.startswith("event:"):
                        ev = line[6:].strip()
                    elif line.startswith("data:") and ev:
                        self.events.append((round(time.monotonic(), 2), ev, line[5:].strip()))
                        ev = None
            self.ended = round(time.monotonic(), 2)
        except Exception as e:  # noqa: BLE001
            self.ended = round(time.monotonic(), 2)
            self.events.append((self.ended, "error", str(e)[:120]))

    def changed_since(self, t: float) -> list:
        return [e for e in self.events if e[1] == "changed" and e[0] >= t]


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP):
        print(__doc__)
        return 2
    bp.map_name()
    s = w.Stack(host="localhost")
    s.up()
    results = []
    c = None
    try:
        c, k, token = token_of(s)
        mint_url = f"{s.door_url}/v2/mint"

        def write(label: str) -> float:
            t = time.monotonic()
            r = httpx.post(mint_url, json={"kind": "group", "draft": {"name": label}},
                           headers={"authorization": f"DPoP {token}", "dpop": w.dpop(k, "POST", mint_url, token)}, verify=s.verify, timeout=30)
            if r.status_code != 200:
                raise w.Fail(f"/v2/mint by the token: {r.status_code} {r.text[:200]}")
            return t

        a = Stream(s, k, token, "first")
        time.sleep(2)
        t1 = write("two streams, one")
        time.sleep(4)
        s1 = {"status": a.status, "changed": len(a.changed_since(t1)), "ended": a.ended is not None}
        ok1 = a.status == 200 and s1["changed"] >= 1 and not s1["ended"]
        results.append(("S1", ok1, "one stream hears a write in its scope (the control)", s1))
        b = Stream(s, k, token, "second")
        time.sleep(2)
        t2 = write("two streams, two")
        time.sleep(4)
        s2 = {"second": {"status": b.status, "changed": len(b.changed_since(t2)), "ended": b.ended is not None},
              "first": {"changed": len(a.changed_since(t2)), "ended_s_after_the_second_opened": None if a.ended is None else round(a.ended - (t2 - 2), 1)}}
        ok2 = s2["second"]["changed"] >= 1 and s2["first"]["changed"] >= 1 and a.ended is None and b.ended is None
        results.append(("S2", ok2, "a second stream on the same token: both hear the next write, and neither ends", s2))
        # Signed out, not left to idle: door-test's timing runs gate on its session count.
        so = "/v2/signout"
        httpx.post(f"{s.door_url}{so}", headers={"authorization": f"DPoP {token}", "dpop": w.dpop(k, "POST", f"{s.door_url}{so}", token)},
                   verify=s.verify, timeout=10)
    except (w.Fail, httpx.HTTPError, KeyError) as e:
        results.append(("--", False, "the run", {"why": str(e)[:300]}))
    finally:
        if c is not None:
            c.post("/v2/signout")
        s.down()
    for sid, ok, what, detail in results:
        print(f"  {sid:<3} {'G' if ok else 'R':<2} {what}: {json.dumps(detail)}")
    bad = [sid for sid, ok, _, _ in results if not ok]
    print("two streams: " + ("G" if not bad else f"R at {', '.join(bad)}"))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
