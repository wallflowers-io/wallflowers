"""ICD 2.1.0's gate 3, the upgrade test (pdr/icd-2.1.0.md): what a released build wrote still reads
the same under the revision's build. door-test only, throwaways, the rehearsal claim key.

  write   at the released build: an owner Registers as the webapp does (/v2/batch: the Site, its
          Host, the edge both ways, a face, the Host's mark; the address), then the one-sign-in
          snippet's ops (the Arc added to the Site, the room and the Host, admitter on two; the
          claim key; the face without pictures; the mark; the Face hydrated; published), a room and
          posts; two visitors by claim, one with a share, and a post each. Every account then signs
          in afresh and its /v2/me and /v2/graph are kept, with its credentials, in UPGRADE_STATE
          (scratch; never committed).
  reopen  at the revision's build: every account signs in again. Each must be Whole with nothing
          noncompliant, and every object it held must still show every field its view showed,
          unchanged. A field the revision adds is allowed: 2.1.0 only adds.

  E2E_DOOR=https://door.localhost E2E_DOOR_CA=/opt/door/edge-root.crt E2E_AUTH=<DOOR_AUTH>
  E2E_ARC=http://127.0.0.1:8090 UPGRADE_STATE=<file> .venv/bin/python app/e2e/upgrade.py write|reopen
"""
import base64
import json
import os
import sys
import time
from pathlib import Path

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import wallflowers_path as w  # noqa: E402

STATE = Path(os.environ.get("UPGRADE_STATE", ""))
# UPGRADE_ICD21=1, a write at 2.1.0 or later, adds what 2.1.0 brought: the room marked with its choice
# and a second room for another (row 3), and each account's card on its self record (row 4), so the
# next revision's reopen holds them too.
ICD21 = os.environ.get("UPGRADE_ICD21") == "1"
ARC = os.environ.get("E2E_ARC", "http://127.0.0.1:8090").rstrip("/")
TAG = time.strftime("%m%d%H%M", time.gmtime())
SHARE = "upgrade2share3xyzabcd"[:20]
# A 1x1 PNG: the Host's mark, as Register and the snippet carry one.
MARK = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def ok(r: httpx.Response) -> dict:
    if r.status_code != 200:
        raise w.Fail(f"{r.request.method} {r.request.url.path}: {r.status_code} {r.text[:200]}")
    return r.json() if r.content else {}


def apply(c, obj, op, **args):
    return ok(c.post("/v2/apply", json={"object": obj, "op": op, "args": args}))


def card(c, name: str) -> None:
    """2.1.0's card (row 4): the self record's group.setProfile with a picture, as the webapp's sheet writes it."""
    if not ICD21:
        return
    g = ok(c.get("/v2/graph"))
    spine = sorted(g.get("spine") or [], key=lambda x: x.get("index", 0))
    if not spine:
        raise w.Fail("no self record in the spine")
    apply(c, spine[0]["object"], "group.setProfile", displayName=name, shape="individual",
          card=json.dumps({"photo": MARK, "photo_mime": "image/png"}))


def seen(c) -> dict:
    """What a fresh session shows: the chain, what will not fold, and every object's view."""
    me = w.me(c)
    g = ok(c.get("/v2/graph"))
    return {"chain": ((me.get("resume") or {}).get("chain") or {}).get("verdict"), "noncompliant": me.get("noncompliant"),
            "objects": {o["id"]: {"kind": o.get("kind"), "name": o.get("name"), "view": o.get("view")} for o in g.get("objects", [])}}


def write(s: w.Stack) -> None:
    people = {}
    owner = w.signup(s, f"upgrade owner {TAG}")
    c = owner["client"]
    slug = f"upgrade-{TAG}"
    face = json.dumps({"mark": "data URL, 46303 chars", "header": {"logo": "", "banner": ""}, "headline": "upgrade"})
    made = ok(c.post("/v2/batch", json={"steps": [
        {"do": "mint", "kind": "group", "draft": {"name": f"Upgrade {TAG}", "shape": "community"}},
        {"do": "mint", "kind": "host", "draft": {"name": f"Upgrade {TAG}"}},
        {"do": "apply", "object": {"$step": 0}, "op": "base.setPart", "args": {"part": {"$step": 1}, "role": "host", "at": int(time.time() * 1000)}},
        {"do": "apply", "object": {"$step": 1}, "op": "base.setParent", "args": {"parent": {"$step": 0}, "role": "host", "at": int(time.time() * 1000)}},
        {"do": "apply", "object": {"$step": 0}, "op": "group.setFace", "args": {"face": face}},
        {"do": "apply", "object": {"$step": 1}, "op": "host.setMedia", "args": {"slot": "mark", "media": MARK, "mediaMime": "image/png"}},
    ]}))
    if made.get("refused"):
        raise w.Fail(f"/v2/batch refused: {made['refused']}")
    site, host = made["made"][0], made["made"][1]
    addr = c.post("/v2/site/address", json={"slug": slug, "host": host})
    say("write: Registered", json.dumps({"site": site[:16], "host": host[:16], "address": addr.status_code}))
    room = ok(c.post("/v2/mint", json={"kind": "forum", "draft": {"name": "upgrade room"}}))["object_id"]
    at = int(time.time() * 1000)
    apply(c, site, "base.setPart", part=room, role="room", at=at, **({"choice": "skills"} if ICD21 else {}))
    apply(c, room, "base.setParent", parent=site, role="room", at=at)
    rooms = [room]
    if ICD21:
        second = ok(c.post("/v2/mint", json={"kind": "forum", "draft": {"name": "upgrade room, financial"}}))["object_id"]
        at = int(time.time() * 1000)
        apply(c, site, "base.setPart", part=second, role="room", at=at, choice="financial")
        apply(c, second, "base.setParent", parent=site, role="room", at=at)
        rooms.append(second)
    # The one-sign-in snippet's ops, at layer 1.
    arc = None
    for obj in (site, *rooms, host):
        arc = ok(c.post("/v2/add", json={"object": obj, "bundle": httpx.get(f"{ARC}/v1/bundle", timeout=30).text}))["member"]
    for obj in (site, *rooms):
        apply(c, obj, "base.setRole", member=arc, role="admitter")
    kid, key = next(iter(json.loads(w.CLAIM_KEYS.read_text()).items()))
    apply(c, site, "group.setClaimIssuer", kid=kid, key=key)
    clean = json.dumps({"mark": "", "header": {"logo": "", "banner": ""}, "headline": "upgrade"})
    apply(c, site, "group.setFace", face=clean)
    apply(c, host, "host.setMedia", slot="mark", media=MARK, mediaMime="image/png")
    payload = json.dumps({"v": 1, "profile": {"displayName": f"Upgrade {TAG}", "card": {"note": "", "urls": []}}, "face": json.loads(clean)})
    apply(c, host, "host.hydrate", key="face", payload=payload, fetchedAt=int(time.time() * 1000), rev=int(time.time() * 1000))
    apply(c, host, "base.publish", slug=slug, publisher=arc)
    for i in range(3):
        apply(c, room, "forum.post", text=f"the owner's post {i}")
    card(c, f"Upgrade owner {TAG}")
    people["owner"] = owner
    # The visitors, by claim: journey 1 and journey 2 (a share).
    for who, choice, share in (("visitor1", "skills", None), ("visitor2", "financial", SHARE)):
        v = w.webapp(s)
        spec = {"site": site, "choice": choice, "ttlSeconds": 600, "now": int(time.time() * 1000)}
        if share:
            spec["share"] = share
        claim = w.subprocess.run(["node", "-e", "import(process.argv[1]).then(m => process.stdout.write(m.issueClaim("
                                  "m.claimKey(process.argv[2]), JSON.parse(process.argv[3]))))", w.CLAIM_MJS.as_uri(), w.KIOSK_SEED,
                                  json.dumps(spec)], capture_output=True, text=True, check=True).stdout
        v.get("/join", params={"claim": claim}, follow_redirects=False)
        p = w.signup(s, f"upgrade {who} {TAG}", v)
        r, t0 = v.post("/v2/join"), time.monotonic()
        while r.status_code in (503, 429) and time.monotonic() - t0 < 60:
            time.sleep(2)
            r = v.post("/v2/join")
        ok(r)
        apply(v, room if choice == "skills" or not ICD21 else rooms[-1], "forum.post", text=f"{who}'s post")
        card(v, f"Upgrade {who} {TAG}")
        people[who] = p
        say(f"write: {who} joined by claim ({choice}{', a share' if share else ''})")
    time.sleep(5)
    kept = {}
    for who, p in people.items():
        p["client"].post("/v2/signout")
        fresh = w.signin(s, p)["client"]
        kept[who] = {"handle": p["handle"], "pk": p["pk"], "prf": p["prf"].hex(), "seen": seen(fresh)}
        fresh.post("/v2/signout")
        say(f"write: {who} kept", json.dumps({"chain": kept[who]["seen"]["chain"], "noncompliant": kept[who]["seen"]["noncompliant"],
                                             "objects": len(kept[who]["seen"]["objects"])}))
    # 0600 from the first byte: it holds each account's PRF. fchmod covers a file already there.
    fd = os.open(STATE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(json.dumps({"build": os.environ.get("UPGRADE_BUILD", ""), "at": TAG, "site": site, "host": host, "room": room, "rooms": rooms,
                            "slug": slug, "people": kept}))
    say("write: done;", len(kept), "accounts kept in", STATE)


def missing(old, new, path="") -> list[str]:
    """Where `new` does not show what `old` showed: an old key gone, or a value changed. New keys pass."""
    if isinstance(old, dict) and isinstance(new, dict):
        out = []
        for k, v in old.items():
            if k not in new:
                out.append(f"{path}.{k} gone")
            else:
                out += missing(v, new[k], f"{path}.{k}")
        return out
    return [] if old == new else [f"{path}: {json.dumps(old)[:80]} became {json.dumps(new)[:80]}"]


def reopen(s: w.Stack) -> int:
    st = json.loads(STATE.read_text())
    bad = 0
    for who, k in st["people"].items():
        c = w.signin(s, {"handle": k["handle"], "pk": k["pk"], "prf": bytes.fromhex(k["prf"])})["client"]
        now = seen(c)
        c.post("/v2/signout")
        diffs = []
        for oid, o in k["seen"]["objects"].items():
            if oid not in now["objects"]:
                diffs.append(f"{o['kind']} {oid[:16]} no longer held")
            else:
                diffs += [f"{o['kind']} {oid[:16]}{d}" for d in missing(o, now["objects"][oid])]
        good = now["chain"] == "Whole" and now["noncompliant"] == [] and not diffs
        bad += not good
        say(f"reopen: {who}", "G" if good else "R", json.dumps({"chain": now["chain"], "noncompliant": now["noncompliant"],
                                                              "objects": [len(k["seen"]["objects"]), len(now["objects"])],
                                                              "unchanged": not diffs, "diffs": diffs[:12]}))
    say("reopen:", "G" if not bad else f"R, {bad} of {len(st['people'])}")
    return 0 if not bad else 1


def main() -> int:
    phase = sys.argv[1] if len(sys.argv) > 1 else ""
    if phase not in ("write", "reopen") or not w.DEPLOYED or not STATE.name:
        print(__doc__)
        return 2
    s = w.Stack()
    s.up()
    if phase == "write":
        write(s)
        return 0
    return reopen(s)


if __name__ == "__main__":
    sys.exit(main())
