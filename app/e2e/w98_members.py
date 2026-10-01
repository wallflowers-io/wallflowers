"""W-98 MEMBERS (EGREGORE, w98/members): MB1-MB4, as w98_accept.py describes them."""
from __future__ import annotations

import json
import os
import time

import httpx

import w98_harness as H
from w98_harness import PNG, TAG, Person, Site, js, result, said, say  # noqa: F401


def members(site: Site) -> None:
    """MEMBERS (EGREGORE, w98/members): the common route GET /v2/members, publishAbout, the questions."""
    A, B, C = site.A, site.B, site.C
    get = lambda p, path: p.c.get(path)
    listed = lambda r: [m.get("key") for m in (js(r).get("members") or [])]

    # MB1: the route, as the owner and as a member.
    ra, rb = get(A, f"/v2/members/{site.id}"), get(B, f"/v2/members/{site.id}")
    ja = js(ra)
    keys = listed(ra)
    arcs = [m for m in (A.obj(site.id).get("view") or {}).get("roles") or [] if isinstance(m, list) and m[1:] == ["admitter"]]
    result("MB1", {"200 for the owner and a member": ra.status_code == 200 and rb.status_code == 200,
                   "its shape": all(k in ja for k in ("site", "me", "can_define", "members", "questions")),
                   "the owner first": keys[:1] == [A.pk], "B and C listed": {B.pk, C.pk} <= set(keys),
                   "the Arc absent": not any(a[0] in keys for a in arcs), "no gen": '"gen"' not in ra.text,
                   "the owner may define, a member not": ja.get("can_define") is True and js(rb).get("can_define") is False},
           "GET /v2/members/<site>: its members, the owner first, never the Arc", members=len(keys), arcs=len(arcs),
           me=ja.get("me"), keys_of_a_member=sorted((ja.get("members") or [{}])[0].keys()))

    # MB2: the route's refusals.
    none = get(A, "/v2/members")
    other = get(A, "/v2/members/" + "0" * 64)
    result("MB2", {"no Site named: 400": none.status_code == 400 and "name the Site" in none.text,
                   "a Site not in reach: 404": other.status_code == 404 and "no such Site in reach" in other.text},
           "GET /v2/members refuses a missing or unreachable Site", none=said(none), other=said(other))

    # MB3: B's about.
    about = {"bio": f"B, w98 {TAG}", "links": json.dumps(["https://example.org/b"])}
    r = B.apply(site.id, "base.publishAbout", about)
    of = lambda resp, key: next((m for m in (js(resp).get("members") or []) if m.get("key") == key), {})
    end, seen = time.monotonic() + 60, {}
    while time.monotonic() < end:
        seen = of(get(A, f"/v2/members/{site.id}"), B.pk).get("about") or {}
        if seen.get("bio") == about["bio"]:
            break
        time.sleep(2)
    fb = B.fresh()
    mine = of(fb.get(f"/v2/members/{site.id}"), B.pk).get("about") or {}
    result("MB3", {"publishAbout answers": r.status_code == 200, "A sees B's about": seen.get("bio") == about["bio"],
                   "B on a new device has it": mine.get("bio") == about["bio"], "noncompliant []": B.noncompliant(fb) == []},
           "a member's site-scoped about", publish=said(r), seen=seen, mine=mine)
    B.drop(fb)

    # MB4: the questions.
    q = {"text": f"w98 {TAG}: which?", "options": json.dumps(["A", "B"]), "multi": 1, "max": 2, "hint": "pick", "free": 0}
    r = A.apply(site.id, "base.defineQuestion", q)
    qs = lambda p: (js(get(p, f"/v2/members/{site.id}")).get("questions") or [])
    end, mine_q = time.monotonic() + 60, {}
    while time.monotonic() < end:
        mine_q = next((x for x in qs(A) if x.get("text") == q["text"]), {})
        if mine_q:
            break
        time.sleep(2)
    qid = mine_q.get("id", ":")
    author, gen = qid.split(":")[0], int(qid.split(":")[1] or 0) if qid.split(":")[1].isdigit() else None
    ans = B.apply(site.id, "base.answerQuestion", {"target_author": author, "target_gen": gen, "choices": "[0,1]"})
    c_def = C.apply(site.id, "base.defineQuestion", {**q, "text": "C's"})
    end, tally, b_ans = time.monotonic() + 60, None, {}
    while time.monotonic() < end:
        tally = next((x.get("tally") for x in qs(A) if x.get("id") == qid), None)
        b_ans = (of(get(A, f"/v2/members/{site.id}"), B.pk).get("answers") or {}).get(qid) or {}
        if b_ans:
            break
        time.sleep(2)
    ret = A.apply(site.id, "base.retireQuestion", {"target_author": author, "target_gen": gen})
    end, after = time.monotonic() + 60, {}
    while time.monotonic() < end:
        after = next((x for x in qs(B) if x.get("id") == qid), {})
        if after.get("retired") is True:
            break
        time.sleep(2)
    result("MB4", {"defineQuestion answers": r.status_code == 200 and bool(mine_q), "B's answer answers": ans.status_code == 200,
                   "B's answer shows": b_ans.get("choices") in ([0, 1], "[0,1]"), "the tally moves": bool(tally),
                   "C (a member) refused, in core's words": c_def.status_code == 400 and "'base.defineQuestion' is the owner's or an admin's, and the author is neither" in c_def.text,
                   "retired, answers kept": after.get("retired") is True},
           "the owner's question, a member's answer, its retirement; a member may not define",
           question=mine_q, answer=said(ans), c_refused=said(c_def), retire=said(ret), tally=tally)
