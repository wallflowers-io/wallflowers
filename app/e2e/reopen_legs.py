"""The reopen legs, 4 to 7 (runbook.md Pre-flight 2a), as rehearsal (b) ran them (str.md run 48),
at layer 1 against a deployed Door: the webapp's calls, a virtual PRF passkey. door-test only
(Software Management, 28 Sep: never production with a throwaway). In the VM:
  E2E_DOOR=https://door.localhost E2E_DOOR_CA=/opt/door/edge-root.crt E2E_AUTH=<door.env's DOOR_AUTH>
  E2E_ARC=http://127.0.0.1:8090 E2E_IDLE_SECS=900 E2E_DOOR_RESTART="sudo systemctl restart door.service"
  .venv/bin/python app/e2e/reopen_legs.py"""
import json, os, sys, time
from pathlib import Path
import httpx
sys.path.insert(0, str(Path(__file__).resolve().parent))
import wallflowers_path as w

NAME = os.environ.get("LEGS_NAME", f"reopen-{time.strftime('%m%d-%H%M', time.gmtime())}")
ARC = os.environ["E2E_ARC"].rstrip("/")
ST: dict = {}


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def leg(n, ok, **what):
    say(f"leg {n}: {'G' if ok else 'R'}", json.dumps(what))
    return ok


def post(c, path, body):
    r = c.post(path, json=body)
    return r.status_code, r.text[:160]


def head():
    return httpx.get(f"{ST['s'].auth_url}/auth/users/{w.key(ST['owner']['pk'])}/head/meta", timeout=10).json().get("position")


def reopen(n, why, holds=()):
    """The owner on a device that has never seen the account: chain Whole, nothing noncompliant, a write;
    `holds`, what must have survived, found in its graph."""
    t = time.monotonic()
    c = w.signin(ST["s"], ST["owner"])["client"]
    secs = round(time.monotonic() - t, 1)
    m = w.me(c)
    wr = post(c, "/v2/apply", {"object": ST["site"], "op": "base.setPart",
                               "args": {"part": ST["room"], "role": "room", "at": int(time.time() * 1000)}})
    chain = ((m.get("resume") or {}).get("chain") or {}).get("verdict")
    g = c.get("/v2/graph").text
    held = {h[:24]: h in g for h in holds}
    ok = chain == "Whole" and m.get("noncompliant") == [] and wr[0] == 200 and all(held.values())
    leg(n, ok, after=why, sign_in_s=secs, chain=chain, noncompliant=m.get("noncompliant"), owner_write=wr, holds=held)
    ST["oc"] = c
    return ok


def signout(c):
    r = c.post("/v2/signout")
    return r.status_code, c.get("/v2/me").status_code


def main():
    s = ST["s"] = w.Stack()
    s.up()
    # Legs 1 and 2: the owner, a Site and its room.
    o = ST["owner"] = w.signup(s, NAME)
    c = o["client"]
    at = int(time.time() * 1000)
    ST["site"] = c.post("/v2/mint", json={"kind": "group", "draft": {"name": NAME}}).json()["object_id"]
    ST["room"] = c.post("/v2/mint", json={"kind": "forum", "draft": {"name": "reopen room"}}).json()["object_id"]
    a = post(c, "/v2/apply", {"object": ST["site"], "op": "base.setPart", "args": {"part": ST["room"], "role": "room", "at": at}})
    b = post(c, "/v2/apply", {"object": ST["room"], "op": "base.setParent", "args": {"parent": ST["site"], "role": "room", "at": at}})
    if not leg("1-2", a[0] == 200 and b[0] == 200, owner=NAME, pk=o["pk"][:16], site=ST["site"][:16], room=ST["room"][:16], setPart=a, setParent=b):
        return 1

    # Leg 4: idle out. Writes, then nothing: the session ends, its head stored first. The head is
    # where the person's archive chain ends; a mint moves it, a room's post does not (E10 mints).
    before = head()
    mint = c.post("/v2/mint", json={"kind": "group", "draft": {"name": "minted before the idle"}})
    minted = mint.json().get("object_id", "") if mint.status_code == 200 else ""
    wr = post(c, "/v2/apply", {"object": ST["room"], "op": "forum.post", "args": {"text": "written before the idle"}})
    say(f"leg 4: the last request; idle {w.IDLE} s, sweep {w.SWEEP} s")
    time.sleep(w.IDLE + 2 * w.SWEEP + 5)
    code, now = c.get("/v2/me").status_code, head()
    ok4 = leg(4, bool(minted) and wr[0] == 200 and code == 401 and now is not None and (before is None or now > before),
              mint=[mint.status_code, minted[:16]], write=wr, me_after=code, head=[before, now])

    # Leg 5: reopen after the idle end; then after an explicit sign-out.
    ok5a = reopen("5a", "the idle end", holds=(minted, "written before the idle"))
    so = signout(ST["oc"])
    ok5b = so == (200, 401) and reopen("5b", f"a sign-out, {so}")

    # Leg 6: signed out, the Door restarted, reopen.
    so = signout(ST["oc"])
    t = time.monotonic()
    s.restart_door()
    say(f"leg 6: the Door restarted and answering in {time.monotonic() - t:.1f} s")
    ok6 = so == (200, 401) and reopen(6, "a Door restart while signed out")

    # Leg 7: an Arc admission by claim while the owner is sealed.
    c = ST["oc"]
    grants = []
    for obj in (ST["site"], ST["room"]):
        m = c.post("/v2/add", json={"object": obj, "bundle": httpx.get(f"{ARC}/v1/bundle", timeout=30).text})
        member = m.json().get("member") if m.status_code == 200 else None
        grants.append([m.status_code, post(c, "/v2/apply", {"object": obj, "op": "base.setRole", "args": {"member": member, "role": "admitter"}})[0]])
    kid, key = next(iter(json.loads(w.CLAIM_KEYS.read_text()).items()))
    issuer = post(c, "/v2/apply", {"object": ST["site"], "op": "group.setClaimIssuer", "args": {"kid": kid, "key": key}})
    before_post = post(c, "/v2/apply", {"object": ST["room"], "op": "forum.post", "args": {"text": "written before the visitor"}})
    so = signout(c)
    vc = w.webapp(s)
    j = vc.get("/join", params={"claim": w.claim(ST["site"])}, follow_redirects=False)
    v = w.signup(s, NAME + "-v", vc)
    t, r = time.monotonic(), vc.post("/v2/join")
    while r.status_code in (503, 429) and time.monotonic() - t < 60:
        time.sleep(3)
        r = vc.post("/v2/join")
    g = vc.get("/v2/graph").json()
    ids = {x.get("id") for x in g.get("objects", [])}
    vm = w.me(vc)
    sees_before = "written before the visitor" in json.dumps(g)
    admitted = r.status_code == 200 and ST["site"] in ids and ST["room"] in ids and vm.get("noncompliant") == []
    leg("7a", all(x == [200, 200] for x in grants) and issuer[0] == 200 and so == (200, 401) and j.status_code == 303 and admitted,
        add_and_admitter=grants, setClaimIssuer=issuer[0], owner_signout=so, join=[j.status_code, j.headers.get("location")],
        v2_join=[r.status_code, r.text[:120]], join_s=round(time.monotonic() - t, 1), holds_site=ST["site"] in ids,
        holds_room=ST["room"] in ids, noncompliant=vm.get("noncompliant"), sees_the_post_before_joining=sees_before)
    ok7 = admitted and reopen("7b", "the visitor's admission")
    after = post(ST["oc"], "/v2/apply", {"object": ST["room"], "op": "forum.post", "args": {"text": "written after the visitor joined"}})
    t, seen = time.monotonic(), None
    while time.monotonic() - t < 60:
        if "written after the visitor joined" in vc.get("/v2/graph").text:
            seen = round(time.monotonic() - t, 1)
            break
        time.sleep(3)
    ok7 = leg("7c", ok7 and after[0] == 200 and seen is not None, owner_post=after, visitor_sees_it_s=seen) and ok7
    signout(ST["oc"]), signout(vc)
    ok = ok4 and ok5a and ok5b and ok6 and ok7
    say("done:", "G" if ok else "R", "; both signed out")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
