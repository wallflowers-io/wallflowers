"""W-98's acceptance harness (w98_accept.py runs the lanes): a throwaway Site, its owner A and members B and C,
and the record each lane's checks land in."""
from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
os.environ.setdefault("E2E_IDLE_SECS", "900")         # a loopback Door idles as door-test's does, not in 60 s
import browser_path as bp  # noqa: E402
import wallflowers_path as w  # noqa: E402

HERE = Path(__file__).resolve().parent
ARC = os.environ.get("E2E_ARC", "http://127.0.0.1:8090").rstrip("/")
TAG = time.strftime("%m%d%H%M", time.gmtime())
results: list = []
PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="   # 1x1


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def result(sid: str, checks: dict, what: str, **detail) -> None:
    ok = all(checks.values())
    detail = {"failed": [k for k, v in checks.items() if not v], **detail}
    results.append((sid, "G" if ok else "NG", what + ": " + json.dumps(detail, ensure_ascii=False)[:1500]))
    say(sid, "G" if ok else "NG", json.dumps(detail, ensure_ascii=False)[:1500])


class Person:
    """One account: its webapp session (a cookie), and a new device on demand."""

    def __init__(self, s: w.Stack, name: str, c: httpx.Client | None = None) -> None:
        self.s, self.name = s, name
        self.who = w.signup(s, name, c)
        self.c = self.who["client"]
        self.pk = w.key(self.who["pk"])

    def apply(self, obj: str, op: str, args: dict, c: httpx.Client | None = None) -> httpx.Response:
        return (c or self.c).post("/v2/apply", json={"object": obj, "op": op, "args": args})

    def mint(self, kind: str, draft: dict) -> str:
        r = self.c.post("/v2/mint", json={"kind": kind, "draft": draft})
        if r.status_code != 200:
            raise w.Fail(f"{self.name} /v2/mint {kind}: {r.status_code} {r.text[:200]}")
        return r.json()["object_id"]

    def obj(self, oid: str, c: httpx.Client | None = None) -> dict:
        g = (c or self.c).get("/v2/graph").json()
        return next((o for o in g.get("objects", []) if o.get("id") == oid), {})

    def until(self, oid: str, test, secs: float = 60, c: httpx.Client | None = None) -> dict:
        """The object as this person holds it, once `test(obj)` holds or `secs` pass."""
        end, o = time.monotonic() + secs, {}
        while time.monotonic() < end:
            o = self.obj(oid, c)
            try:
                if o and test(o):
                    return o
            except (KeyError, TypeError):
                pass
            time.sleep(1.5)
        return o

    def fresh(self) -> httpx.Client:
        """The same passkey on a device that has never seen the account."""
        return w.signin(self.s, self.who)["client"]

    def noncompliant(self, c: httpx.Client | None = None) -> list | None:
        return (c or self.c).get("/v2/me").json().get("noncompliant")

    def drop(self, *cs: httpx.Client) -> None:
        """Those sessions signed out; this person's webapp session goes on."""
        for c in cs:
            try:
                c.post("/v2/signout")
            except httpx.HTTPError:
                pass

    def out(self) -> None:
        self.drop(self.c)


def js(r: httpx.Response) -> dict:
    """A JSON answer, or {} for any other (a refusal's text)."""
    try:
        v = r.json()
        return v if isinstance(v, dict) else {}
    except ValueError:
        return {}


def said(r: httpx.Response) -> str:
    return f"{r.status_code} {r.text[:160]}"


class Site:
    """A throwaway Site: A its owner, a room, the Arc its admitter by the rehearsal kiosk key; members join by claim."""

    def __init__(self, s: w.Stack, owner: Person) -> None:
        self.s, self.A = s, owner
        self.id = owner.mint("group", {"name": f"w98 {TAG}"})
        self.room = owner.mint("forum", {"name": f"w98 room {TAG}"})
        at = int(time.time() * 1000)
        for obj, op, args in ((self.id, "base.setPart", {"part": self.room, "role": "room", "at": at}),
                              (self.room, "base.setParent", {"parent": self.id, "role": "room", "at": at})):
            r = owner.apply(obj, op, args)
            if r.status_code != 200:
                raise w.Fail(f"{op}: {said(r)}")
        for obj in (self.id, self.room):
            m = owner.c.post("/v2/add", json={"object": obj, "bundle": httpx.get(f"{ARC}/v1/bundle", timeout=30).text})
            if m.status_code != 200:
                raise w.Fail(f"the Arc added: {said(m)}")
            r = owner.apply(obj, "base.setRole", {"member": m.json()["member"], "role": "admitter"})
            if r.status_code != 200:
                raise w.Fail(f"the Arc its admitter: {said(r)}")
        # Its Host and address, as the webapp's Register makes them: the Face reads the Site at /v1/face/<slug>.
        self.slug, at = f"w98-{TAG}-{os.urandom(2).hex()}", int(time.time() * 1000)
        b = owner.c.post("/v2/batch", json={"steps": [{"do": "mint", "kind": "host", "draft": {"name": f"w98 {TAG}"}},
                                                      {"do": "apply", "object": self.id, "op": "base.setPart", "args": {"part": {"$step": 0}, "role": "host", "at": at}},
                                                      {"do": "apply", "object": {"$step": 0}, "op": "base.setParent", "args": {"parent": self.id, "role": "host", "at": at}}]})
        self.host = (js(b).get("made") or [None])[0]
        addr = owner.c.post("/v2/site/address", json={"slug": self.slug, "host": self.host}) if self.host else None
        if self.host:
            # The Arc on the Host, as the founders' one-sign-in snippet adds it: the Face reads the Host through it.
            h = owner.c.post("/v2/add", json={"object": self.host, "bundle": httpx.get(f"{ARC}/v1/bundle", timeout=30).text})
            if h.status_code != 200:
                say("the Arc not added to the Host:", said(h))
            elif addr is not None and addr.status_code < 300:
                # Its Face live, as hack-seoul.js (d0efef5d) puts one live after a Register: the card onto the Host, then
                # the Host published at its slug, the Arc its publisher. Without these the slug answers "no such page".
                now = int(time.time() * 1000)
                card = json.dumps({"v": 1, "profile": {"displayName": f"w98 {TAG}", "card": {"note": "", "urls": []}}, "face": None})
                for op, args in (("host.hydrate", {"key": "face", "payload": card, "fetchedAt": now, "rev": now}),
                                 ("base.publish", {"slug": self.slug, "publisher": h.json()["member"]})):
                    r = owner.apply(self.host, op, args)
                    if r.status_code != 200:
                        say(f"the Face: {op}", said(r))
        if addr is None or addr.status_code >= 300:
            say("no address for the Site:", said(b) if addr is None else said(addr))
            self.slug = None
        kid, key = next(iter(json.loads((HERE.parents[1] / "arc/hosting/claim-keys.rehearsal.json").read_text()).items()))
        r = owner.apply(self.id, "group.setClaimIssuer", {"kid": kid, "key": key})
        if r.status_code != 200:
            raise w.Fail(f"group.setClaimIssuer: {said(r)}")

    def member(self, name: str) -> Person:
        c = w.webapp(self.s)
        c.get("/join", params={"claim": w.claim(self.id)}, follow_redirects=False)
        p = Person(self.s, name, c)
        r = p.c.post("/v2/join")
        tries = 0
        while r.status_code in (503, 429) and tries < 20:
            time.sleep(3)
            r, tries = p.c.post("/v2/join"), tries + 1
        if r.status_code != 200:
            raise w.Fail(f"{name} /v2/join: {said(r)}")
        if not p.until(self.room, lambda o: True):
            raise w.Fail(f"{name} holds no room; the join answered {r.text[:300]}")
        return p
