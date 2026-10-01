"""W-98 ROOMS (BW-A, door/w98-rooms): RM1-RM3, as w98_accept.py describes them."""
from __future__ import annotations

import json
import os
import time

import httpx

import w98_harness as H
from w98_harness import PNG, TAG, Person, Site, js, result, said, say  # noqa: F401


def rooms(site: Site) -> None:
    A, B, C = site.A, site.B, site.C
    room = site.room
    msgs = lambda o: (o.get("view") or {}).get("messages") or []

    # RM1: B deletes B's own message; C may not delete A's.
    words = f"w98 retract me {TAG}"
    r = B.apply(room, "forum.post", {"text": words})
    mine = B.until(room, lambda o: any(m.get("text") == words for m in msgs(o)))
    m = next((m for m in msgs(mine) if m.get("text") == words), {})
    seen_by_a = bool(A.until(room, lambda o: any(x.get("text") == words for x in msgs(o))))
    gone = B.apply(room, "forum.retract", {"target_author": B.pk, "target_gen": m.get("gen")})
    a_view = A.until(room, lambda o: not any(x.get("text") == words for x in msgs(o)))
    fb = B.fresh()
    b_new = B.until(room, lambda o: True, c=fb)
    a_text = A.c.get("/v2/graph").text
    A.apply(room, "forum.post", {"text": f"w98 A's {TAG}"})
    am = A.until(room, lambda o: any(x.get("text") == f"w98 A's {TAG}" for x in msgs(o)))
    a_msg = next((x for x in msgs(am) if x.get("text") == f"w98 A's {TAG}"), {})
    c_try = C.apply(room, "forum.retract", {"target_author": A.pk, "target_gen": a_msg.get("gen")})
    result("RM1", {"B posts": r.status_code == 200 and bool(m) and seen_by_a, "B's retract answers": gone.status_code == 200,
                   "gone for A": not any(x.get("text") == words for x in msgs(a_view)),
                   "gone for B on a new device": bool(b_new) and not any(x.get("text") == words for x in msgs(b_new)),
                   "its words served by no route (D-60)": words not in a_text,
                   "C's retract of A's refused, in core's words": c_try.status_code == 400 and "Unauthorized" in c_try.text,
                   "noncompliant []": A.noncompliant() == [] and B.noncompliant(fb) == []},
           "delete your own message: gone everywhere; another's refused", retract=said(gone), c_refused=said(c_try), gen=m.get("gen"))
    B.drop(fb)

    # RM2: A describes the room.
    text = f"The w98 room, {TAG}: bring what you can."
    desc = lambda o: (o.get("view") or {}).get("description")
    r = A.apply(room, "forum.editDescription", {"description": text})
    b_sees = B.until(room, lambda o: desc(o) == text)
    fa = A.fresh()
    a_new = A.until(room, lambda o: desc(o) == text, c=fa)
    c_try = C.apply(room, "forum.editDescription", {"description": "C's"})
    big = A.apply(room, "forum.editDescription", {"description": "x" * 2049})
    after = A.obj(room)
    result("RM2", {"A's description answers": r.status_code == 200, "B sees it": desc(b_sees) == text,
                   "A on a new device has it": desc(a_new) == text,
                   "C refused, in core's words": c_try.status_code == 400 and "is the owner's or an admin's" in c_try.text,
                   "over 2048 bytes refused": big.status_code == 400 and "MalformedArgs" in big.text,
                   "never cut: the description unchanged": desc(after) == text,
                   "noncompliant []": A.noncompliant(fa) == [] and B.noncompliant() == []},
           "a room's description: its owner's, seen by a member and on a new device; others refused",
           c_refused=said(c_try), over=said(big), roles=(after.get("view") or {}).get("roles"))
    A.drop(fa)

    # RM3: B an admin (Make admin), then not (Remove admin).
    grants = [A.apply(o, "base.setRole", {"member": B.pk, "role": "admin"}) for o in (site.id, room)]
    b_admin = f"B's description as admin, {TAG}"
    B.until(room, lambda o: [B.pk, "admin"] in ((o.get("view") or {}).get("roles") or []))
    rb = B.apply(room, "forum.editDescription", {"description": b_admin})
    counts = A.until(room, lambda o: desc(o) == b_admin)
    clears = [A.apply(o, "base.clearRole", {"member": B.pk}) for o in (site.id, room)]
    back = A.until(room, lambda o: desc(o) == text)
    rb2 = B.apply(room, "forum.editDescription", {"description": "B after removal"})
    later = A.obj(room)
    result("RM3", {"Make admin answers": all(g.status_code == 200 for g in grants), "B's write as admin answers": rb.status_code == 200,
                   "B's description counts": desc(counts) == b_admin, "Remove admin answers": all(c.status_code == 200 for c in clears),
                   "B's writes stop counting: back to A's": desc(back) == text,
                   "B after removal refused or inert": rb2.status_code == 400 or desc(later) == text},
           "an admin of the room describes it; removed, their writes stop counting",
           grants=[g.status_code for g in grants], clears=[c.status_code for c in clears], after_removal=said(rb2),
           roles=(later.get("view") or {}).get("roles"))
