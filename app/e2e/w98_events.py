"""W-98 EVENTS (BW-D core, BW-B UI): EV1-EV6, as w98_accept.py describes them."""
from __future__ import annotations

import json
import os
import time

import httpx

import w98_harness as H
from w98_harness import PNG, TAG, Person, Site, js, result, said, say  # noqa: F401


def face_items(slug: str) -> dict:
    try:
        return httpx.get(f"{H.ARC}/v1/face/{slug}/items", timeout=30).json().get("items") or {}
    except (httpx.HTTPError, ValueError):
        return {}


def events(site: Site) -> None:
    """EVENTS (BW-D core, BW-B UI): an event made and tied to its Site (NC-81, both ends), its W-98 profile, media, lineup,
    RSVP, refusals; its Face. No Door route puts a second person on an event's roster (BW-D): "a second member sees it"
    and "a co-host's writes count" are recorded as no route yet, and the co-host rule stands on core's tests."""
    A, B = site.A, site.B
    start = int(time.time() * 1000) + 7 * 86400_000
    r = A.c.post("/v2/batch", json={"steps": [
        {"do": "mint", "kind": "event", "draft": {"name": f"w98 night {TAG}", "start_ms": start}},
        {"do": "apply", "object": site.id, "op": "group.setAffiliation", "args": {"peer": {"$step": 0}, "rel": "created", "name": f"w98 night {TAG}", "at": start - 1}},
        {"do": "apply", "object": {"$step": 0}, "op": "base.setBacklink", "args": {"object": site.id, "rel": "created", "at": start - 1}}]})
    ev = (js(r).get("made") or [None])[0]
    if not ev:
        result("EV1", {"the event made and tied": False}, "an event, both ends", batch=said(r))
        return
    view = lambda o: o.get("view") or {}
    prof = {"title": f"w98 night {TAG}", "startMs": start, "tz": "Asia/Seoul", "online": "https://example.org/live", "status": "postponed",
            "videoUrl": "https://example.org/v", "descriptorFormat": "markdown", "allDay": 1, "recurrence": "FREQ=WEEKLY;BYDAY=SA"}
    sp = A.apply(ev, "event.setProfile", prof)
    got = A.until(ev, lambda o: view(o).get("tz") == "Asia/Seoul")
    fa = A.fresh()
    new = A.until(ev, lambda o: view(o).get("tz") == "Asia/Seoul", c=fa)
    v = view(got)
    result("EV1", {"made and tied (one batch)": r.status_code == 200 and bool(ev), "setProfile answers": sp.status_code == 200,
                   "tz, status, online, video, format, all day, recurrence in the view": v.get("tz") == "Asia/Seoul" and v.get("status") == "postponed"
                   and v.get("online") == prof["online"] and v.get("video_url") == prof["videoUrl"] and v.get("descriptor_format") == "markdown"
                   and v.get("all_day") in (1, True) and v.get("recurrence") == prof["recurrence"],
                   "A on a new device has it": view(new).get("tz") == "Asia/Seoul", "noncompliant []": A.noncompliant(fa) == []},
           "an event of the Site, its W-98 profile (a second member: no route yet)", set_profile=said(sp), view_keys=sorted(v.keys()))
    A.drop(fa)

    # EV2: media.
    bn = A.apply(ev, "event.setBanner", {"banner": PNG, "bannerMime": "image/png"})
    ids = [os.urandom(8).hex() for _ in range(7)]
    adds = [A.apply(ev, "event.addPhoto", {"id": i, "photo": PNG, "photoMime": "image/png", "at": int(time.time() * 1000) + n}) for n, i in enumerate(ids)]
    rm = A.apply(ev, "event.removePhoto", {"id": ids[0]})
    again = A.apply(ev, "event.addPhoto", {"id": ids[0], "photo": PNG, "photoMime": "image/png", "at": int(time.time() * 1000)})
    ph = A.until(ev, lambda o: len(view(o).get("photos") or []) == 5)
    result("EV2", {"banner": bn.status_code == 200, "six photos": all(a.status_code == 200 for a in adds[:6]),
                   "a seventh refused": adds[6].status_code == 400 and "PreconditionFailed" in adds[6].text,
                   "removed, and not re-added": rm.status_code == 200 and again.status_code == 400 and "PreconditionFailed" in again.text,
                   "five in the view": len(view(ph).get("photos") or []) == 5},
           "an event's banner and photos, at most six live", seventh=said(adds[6]), readd=said(again), banner=bool(view(ph).get("banner")))

    # EV3: the lineup, B's act unconfirmed until B's own half.
    ln = A.apply(ev, "event.setLineup", {"acts": json.dumps([{"member": B.pk, "role": "dj"}])})
    acts = lambda o: view(o).get("acts") or []
    lv = A.until(ev, lambda o: bool(acts(o)))
    spine = sorted(js(B.c.get("/v2/graph")).get("spine") or [], key=lambda x: x.get("index", 0))
    selfid = spine[0].get("object") if spine else None     # the person's own record, as the webapp's model() finds it
    cf = B.apply(selfid, "group.setAffiliation", {"peer": ev, "rel": "performs_at", "name": f"w98 night {TAG}", "at": int(time.time() * 1000)}) if selfid else None
    later = A.until(ev, lambda o: any(a.get("confirmed") for a in acts(o)), secs=30)
    result("EV3", {"setLineup answers": ln.status_code == 200, "the act, unconfirmed": [a.get("member") for a in acts(lv)] == [B.pk]
                   and acts(lv)[0].get("confirmed") is False, "B's performs_at answers": cf is not None and cf.status_code == 200},
           "a lineup of real profiles; the act's own half", lineup=said(ln), acts=acts(lv), confirm=None if cf is None else said(cf),
           confirmed_after=[a.get("confirmed") for a in acts(later)], note="the event's own view of the performer's half is BUILD's integration item 1")

    # EV4: RSVP with approval.
    rg = A.apply(site.id, "group.setRegistration", {"event": ev, "approval": 1, "capacity": 10})
    rs = B.apply(site.id, "group.rsvp", {"event": ev, "status": "going", "guests": 1, "guestNames": json.dumps(["Guest of B"]), "at": int(time.time() * 1000)})
    pend = A.until(site.id, lambda o: B.pk in json.dumps(view(o).get("rsvps") or view(o).get("registrations") or view(o)))
    dc = A.apply(site.id, "group.rsvpDecide", {"event": ev, "member": B.pk, "decision": "approved"})
    c_dc = site.C.apply(site.id, "group.rsvpDecide", {"event": ev, "member": B.pk, "decision": "declined"})
    result("EV4", {"registration": rg.status_code == 200, "B's RSVP answers": rs.status_code == 200, "the owner approves": dc.status_code == 200,
                   "C may not decide": c_dc.status_code == 400},
           "RSVP with approval, in the Site's log", registration=said(rg), rsvp=said(rs), decide=said(dc), c_decide=said(c_dc))

    # EV5: malformed, refused by core.
    bad = {"a status outside the vocabulary": {"status": "maybe"}, "a javascript: url": {"videoUrl": "javascript:alert(1)"},
           "allDay not 0/1": {"allDay": 2}, "a long tz": {"tz": "x" * 65}}
    refused = {k: A.apply(ev, "event.setProfile", {**prof, **v}) for k, v in bad.items()}
    lineup_name = A.apply(ev, "event.setLineup", {"acts": json.dumps([{"member": B.pk, "role": "dj", "name": "free text"}])})
    result("EV5", {**{k: r_.status_code == 400 and "MalformedArgs" in r_.text for k, r_ in refused.items()},
                   "an act with a free-text name": lineup_name.status_code == 400 and "MalformedArgs" in lineup_name.text},
           "malformed event args refused, in core's words", **{k: said(r_) for k, r_ in refused.items()})

    # EV6: the Face at the Site's address.
    if not site.slug:
        result("EV6", {"the Site has an address": False}, "the event on its Site's Face")
    else:
        key = f"event:{ev}"
        end, item = time.monotonic() + 60, {}
        while time.monotonic() < end and not item:
            item = face_items(site.slug).get(key) or {}
            time.sleep(2)
        vis = A.apply(ev, "base.setVisibility", {"visibility": "private"})
        end, gone = time.monotonic() + 60, False
        while time.monotonic() < end and not gone:
            gone = key not in face_items(site.slug)
            time.sleep(2)
        result("EV6", {"on the Face": bool(item), "tz and status there": item.get("tz") == "Asia/Seoul" and item.get("status") == "postponed",
                       "never online": "online" not in item, "private withdraws it": vis.status_code == 200 and gone},
               "the event on its Site's Face; private takes it off", slug=site.slug, item_keys=sorted(item.keys()), private=said(vis))
