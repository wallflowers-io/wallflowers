"""W-98 TRADE, the board (BW-E, w98/trade core): TB1-TB4, as w98_accept.py describes them."""
from __future__ import annotations

import json
import os
import time

import httpx

import w98_harness as H
from w98_harness import PNG, TAG, Person, Site, js, result, said, say  # noqa: F401


def trade_board(site: Site) -> None:
    """TRADE's board (BW-E, w98/trade core): a Thing's listing on its Site, taken down by the owner; the Thing's own ops."""
    A, B, C = site.A, site.B, site.C
    listings = lambda o: (o.get("view") or {}).get("listings") or []
    has = lambda o, thing: any(thing in json.dumps(x) for x in listings(o))

    # TB1: B lists a Thing on the Site.
    thing = B.mint("thing", {"name": f"w98 drill {TAG}"})
    prof = B.apply(thing, "thing.setProfile", {"name": f"w98 drill {TAG}", "condition": "like_new", "description": "A drill, barely used."})
    lst = {"thingId": thing, "posture": "selling", "title": f"Drill {TAG}", "reach": "private", "rev": 0, "price": "£15"}
    r = B.apply(site.id, "group.publishListing", lst)
    a_sees = A.until(site.id, lambda o: has(o, thing))
    fb = B.fresh()
    b_new = B.until(site.id, lambda o: has(o, thing), c=fb)
    t_view = B.obj(thing).get("view") or {}
    result("TB1", {"setProfile answers": prof.status_code == 200, "publishListing answers": r.status_code == 200,
                   "A sees the listing": has(a_sees, thing), "B on a new device has it": has(b_new, thing),
                   "the Thing's condition": t_view.get("condition") == "like_new", "noncompliant []": B.noncompliant(fb) == [] and A.noncompliant() == []},
           "a member's listing on the Site", publish=said(r), listing=[x for x in listings(a_sees) if thing in json.dumps(x)][:1],
           thing_view_keys=sorted(t_view.keys()))
    B.drop(fb)

    # TB2: the owner takes it down; a relisting of that key is inert; a member may not take one down.
    A.apply(site.id, "group.publishListing", {**lst, "thingId": "a" * 64, "title": "A's own"})
    c_rm = C.apply(site.id, "group.removeListing", {"author": A.pk, "thingId": "a" * 64})
    rm = A.apply(site.id, "group.removeListing", {"author": B.pk, "thingId": thing})
    gone = B.until(site.id, lambda o: not has(o, thing))
    again = B.apply(site.id, "group.publishListing", {**lst, "rev": 1})
    time.sleep(4)
    still = A.obj(site.id)
    result("TB2", {"removeListing answers": rm.status_code == 200, "gone for B": not has(gone, thing),
                   "a relisting of that key inert": not has(still, thing),
                   "C (a member) refused, in core's words": c_rm.status_code == 400 and "'group.removeListing' is the owner's or an admin's, and the author is neither" in c_rm.text},
           "the owner takes a listing down; it stays down; a member may not", remove=said(rm), relist=said(again), c_refused=said(c_rm))

    # TB3: the Site's own listing (site 1): a member's refused, the owner's stands.
    other = B.mint("thing", {"name": f"w98 lamp {TAG}"})
    b_site = B.apply(site.id, "group.publishListing", {**lst, "thingId": other, "title": "B as the Site", "site": 1})
    own = A.mint("thing", {"name": f"w98 chair {TAG}"})
    a_site = A.apply(site.id, "group.publishListing", {**lst, "thingId": own, "title": "The Site's chair", "site": 1})
    seen = B.until(site.id, lambda o: has(o, own))
    mine = next((x for x in listings(seen) if own in json.dumps(x)), {})
    result("TB3", {"a member's site listing refused": b_site.status_code == 400 or not has(A.obj(site.id), other),
                   "the owner's stands": a_site.status_code == 200 and bool(mine), "marked the Site's": mine.get("site") in (True, 1)},
           "a listing as the Site: the owner's or an admin's only", member=said(b_site), owner=said(a_site), listing=mine)

    # TB4: the Thing's photos and disposition.
    pid, at = os.urandom(8).hex(), int(time.time() * 1000)
    add = B.apply(thing, "thing.addPhoto", {"id": pid, "photo": PNG, "photoMime": "image/png", "at": at})
    has_photo = B.until(thing, lambda o: pid in json.dumps(o.get("view") or {}))
    rmp = B.apply(thing, "thing.removePhoto", {"id": pid})
    no_photo = B.until(thing, lambda o: pid not in json.dumps(o.get("view") or {}))
    until = int(time.time() * 1000) + 5000
    res = B.apply(thing, "thing.setDisposition", {"state": "reserved", "for": A.pk, "until": until, "at": int(time.time() * 1000)})
    reserved = B.until(thing, lambda o: "reserved" in json.dumps((o.get("view") or {})))
    time.sleep(7)
    later = B.obj(thing).get("view") or {}
    result("TB4", {"addPhoto answers and shows": add.status_code == 200 and pid in json.dumps(has_photo.get("view") or {}),
                   "removePhoto answers and it goes": rmp.status_code == 200 and pid not in json.dumps(no_photo.get("view") or {}),
                   "reserved": res.status_code == 200 and "reserved" in json.dumps(reserved.get("view") or {}),
                   "available after until": "available" in json.dumps(later)},
           "a Thing's photo and its reservation", add=said(add), reserve=said(res), later={k: later.get(k) for k in ("disposition", "state", "reserved") if k in later})
