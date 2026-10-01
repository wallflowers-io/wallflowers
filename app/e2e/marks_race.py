"""NC-123's regression (run 77; Software Engineering, 29 Sep): a claim spent in the seconds after the
owner marks the Site's rooms with their choice (2.1.0 row 3) must land in the chosen room only. Red
while the Arc admits on the state it last folded: before it holds the marks, every admitter room.

A fresh Site (restate_timing.py's setup: Healing Resistance and the three rooms, unmarked, the Arc
admitter on the Site and the three, the rehearsal key). Each trial: a visitor readied first (a resources
claim at /join, a fresh account), then the owner marks the rooms as one-signin-2.1.0.js does
(base.setPart with `choice`), then at once the visitor's /v2/join; it must hold the room marked
`resources` now, and no other. MARKS_TRIALS trials, the first on unmarked rooms, each after it swapping
which room carries `resources` and which `skills`, so a stale Arc shows as the old room. Then,
MARKS_SETTLE seconds after the last marks, the same without the race: the control.

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  RESTATE_STATE=<a new file> RESTATE_BUNDLES=<4 Arc bundles> [MARKS_SETTLE=60] .venv/bin/python app/e2e/marks_race.py
"""
from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import restate_timing as R  # noqa: E402
import wallflowers_path as w  # noqa: E402

SETTLE = int(os.environ.get("MARKS_SETTLE", "60"))
TRIALS = int(os.environ.get("MARKS_TRIALS", "4"))
CHOICES = {"resources": "Resources", "skills": "Skills, Time & Services", "financial": "Financial Support"}
say = R.say


def ready(s: w.Stack, site: str, choice: str) -> httpx.Client:
    """A visitor with the claim held at /join and an account, not yet joined."""
    v = w.webapp(s)
    v.get("/join", params={"claim": R.claim(site, choice, None)}, follow_redirects=False)
    w.signup(s, f"marks race {choice} {R.TAG}", v)
    return v


def admitted(v: httpx.Client, rooms: dict) -> dict:
    r, t0 = v.post("/v2/join"), time.monotonic()
    while r.status_code == 503 and time.monotonic() - t0 < 30:
        time.sleep(1)
        r = v.post("/v2/join")
    at = time.monotonic()
    time.sleep(3)
    ids = {o.get("id") for o in R.ok(v.get("/v2/graph")).get("objects", [])}
    v.post("/v2/signout")
    return {"at": at, "join": r.status_code, "fallback": (r.json() if r.status_code == 200 else {}).get("fallback"),
            "held": {n: rid in ids for n, rid in rooms.items()}}


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP and R.STATE.name) or len(R.BUNDLES) < 4:
        print(__doc__)
        return 2
    bp.map_name()
    s = w.Stack(host="localhost")
    s.up()
    R.setup(s)
    st = json.loads(R.STATE.read_text())
    site, rooms = st["site"], st["rooms"]
    owner = w.signin(s, {"handle": st["owner"]["handle"], "pk": st["owner"]["pk"], "prf": bytes.fromhex(st["owner"]["prf"])})["client"]
    def mark(res: str, ski: str) -> None:
        for choice, name in (("resources", res), ("skills", ski), ("financial", "Financial Support")):
            R.apply(owner, site, "base.setPart", part=rooms[name], role="room", choice=choice, at=int(time.time() * 1000))
    maps = [("Resources", "Skills, Time & Services"), ("Skills, Time & Services", "Resources")]
    raced = []
    for i in range(TRIALS):
        res, ski = maps[i % 2]
        v = ready(s, site, "resources")
        mark(res, ski)
        marked = time.monotonic()
        got = admitted(v, rooms)
        ok = got["join"] == 200 and got["held"] == {n: n == res for n in rooms}
        raced.append(ok)
        say(f"marks race: trial {i + 1}, resources marked on {res}", "G" if ok else "R",
            json.dumps({"seconds after the marks": round(got["at"] - marked, 1), **{k: got[k] for k in ("join", "fallback", "held")}}))
    time.sleep(max(0, SETTLE - (time.monotonic() - marked)))
    later = admitted(ready(s, site, "resources"), rooms)
    ok_later = later["join"] == 200 and later["held"] == {n: n == res for n in rooms}
    say("marks race: settled", "G" if ok_later else "R",
        json.dumps({"seconds after the marks": round(later["at"] - marked, 1), **{k: later[k] for k in ("join", "fallback", "held")}}))
    owner.post("/v2/signout")
    ok_first = all(raced)
    say("marks race:", "G" if ok_first and ok_later else
        "R (NC-123: a claim spent before the Arc holds the marks lands in every admitter room)" if ok_later else "R, the control too")
    return 0 if ok_first and ok_later else 1


if __name__ == "__main__":
    sys.exit(main())
