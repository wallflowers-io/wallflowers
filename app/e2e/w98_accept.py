"""W-98's routes accepted on door-test (W-99; Software Management, 1 Oct). Each route through the Door's own routes, as a
person's webapp (a cookie) or a Site (a DPoP token) reaches them, and on every route the same five things: the op goes
through the Door's route; a second member sees it on their own device; its author on a device that has never seen it still
has it (the Persistent State test); /v2/me's noncompliant stays []; an unauthorised member is refused in core's words.

A throwaway Site per run: its owner A, members B and C by kiosk-format claims (the rehearsal key; a Site with no registered
home joins by the webapp's cookie), the Arc its admitter.

  ROOMS (BW-A, door/w98-rooms)
    RM1  B deletes B's own message (forum.retract): gone for A, and for B on a new device; its words served by no route
         (D-60); C's retract of A's message refused
    RM2  A describes the room (forum.editDescription): B sees it, A on a new device has it; C refused; over 2048 bytes
         refused, never cut; "" clears it
    RM3  A makes B an admin (base.setRole on the Site and the room): B's description counts; Remove admin (base.clearRole):
         B's writes stop counting, the room falls back to A's
  MEMBERS (EGREGORE, w98/members)
    MB1  GET /v2/members/<site>: its members, the owner first, never the Arc, no gen; the owner may define, a member not
    MB2  no Site named 400; a Site not in reach 404
    MB3  B's about (base.publishAbout): A sees it; B on a new device has it
    MB4  A's question (base.defineQuestion), B's answer, the tally, A's retireQuestion (answers kept); C may not define
  TRADE, the board (BW-E, w98/trade core; the deal waits on /v2/trade/*)
    TB1  B lists a Thing on the Site (group.publishListing): A sees it, B on a new device has it; the Thing's condition
    TB2  A takes it down (group.removeListing): gone, and a relisting of that key inert; C may not take one down
    TB3  site 1: a member's refused, the owner's stands, marked the Site's
    TB4  the Thing's photo (addPhoto, removePhoto) and reservation (setDisposition reserved, available after until)

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  JA_KIOSK=<egregore> W98_ROUTES=rooms .venv/bin/python app/e2e/w98_accept.py
  EVENTS (BW-D core, BW-B UI)
    EV1-EV6  an event made and tied to its Site in one batch; its profile, media, lineup, RSVP and refusals; its Face

  The harness is w98_harness.py; each lane is its own module (w98_rooms, w98_members, w98_trade, w98_events).

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  JA_KIOSK=<egregore> W98_ROUTES=rooms,members,trade,events .venv/bin/python app/e2e/w98_accept.py
"""
from __future__ import annotations

import importlib
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import w98_harness as H  # noqa: E402
from w98_harness import TAG, Person, Site, bp, results, say, w  # noqa: E402

ROUTES = [r for r in os.environ.get("W98_ROUTES", "rooms").split(",") if r]
LANES = {"rooms": ("w98_rooms", "rooms"), "members": ("w98_members", "members"), "trade": ("w98_trade", "trade_board"),
         "events": ("w98_events", "events")}


def run(s: w.Stack) -> None:
    A = Person(s, f"w98 A {TAG}")
    site = Site(s, A)
    # The Arc's node folds a Site minted a moment ago late: a claim before then is admitted to no room, and the join says
    # nothing is unjoined (TEST's finding, 1 Oct). A real Site is set up before its claims; here, a pause.
    time.sleep(20)
    site.B, site.C = site.member(f"w98 B {TAG}"), site.member(f"w98 C {TAG}")
    say("the Site", site.id[:12], "the room", site.room[:12], "A B C", A.pk[:8], site.B.pk[:8], site.C.pk[:8])
    try:
        for route in ROUTES:
            mod, fn = LANES[route]
            getattr(importlib.import_module(mod), fn)(site)
    finally:
        for p in (A, site.B, site.C):
            p.out()


def main() -> int:
    """door-test (E2E_DOOR set), or a loopback stack built from this checkout: before 2.3.1 is pinned, a deploy refuses
    it (PIN-8), and a batch is accepted here first."""
    if os.environ.get("JA_KIOSK"):
        w.CLAIM_MJS = Path(os.environ["JA_KIOSK"]).resolve() / "egg" / "kiosk" / "claim.mjs"
    if w.DEPLOYED:
        if not bp.DOOR_IP:
            print(__doc__)
            return 2
        bp.map_name()
    s = w.Stack(host="localhost")
    try:
        if not w.DEPLOYED:
            s.build()
        s.up()
        if not w.DEPLOYED:
            H.ARC = f"http://127.0.0.1:{s.gw}"
        say("the Door", s.door_url, "the Arc", H.ARC)
        run(s)
    except Exception as e:                            # a step that could not go on is an NG, named, with where
        import traceback
        at = [f"{f.name}:{f.lineno}" for f in traceback.extract_tb(e.__traceback__)][-3:]
        results.append(("--", "NG", f"{type(e).__name__}: {e} at {' < '.join(reversed(at))}"))
    finally:
        if not w.DEPLOYED:
            s.down()
    for sid, st, text in results:
        print(f"  {sid:<4} {st:<3}  {text}")
    bad = [sid for sid, st, _ in results if st == "NG"]
    print("w98 accept: " + ("G" if results and not bad else f"NG at {', '.join(bad) or 'nothing run'}"))
    return 0 if results and not bad else 1


if __name__ == "__main__":
    sys.exit(main())
