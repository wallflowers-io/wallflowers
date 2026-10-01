"""FC-6 (mdr/icd-pin.md, d5dfcc1+): 64 sessions on door-test holding the Site's room at 1,000
Deltas. (a) show load: 20 posts a minute, every session reading on each change as the webapp
does (GET /v2/graph, debounced 150 ms); sign-in within 3 s for 95 % (RX.6). (b) saturation:
1, 2 and 5 posts a second, offered by one serial poster. CPU from each unit's cgroup (cpu.stat
usage_usec); host busy includes the driver. Throwaways only; door-test only, in the VM:
E2E_AUTH=<door.env's DOOR_AUTH> .venv/bin/python app/e2e/fc6_load.py"""
import asyncio, json, os, subprocess, sys, time
from pathlib import Path
import httpx
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import wallflowers_path as w

DOOR, CA, ARC = "https://door.localhost", "/opt/door/edge-root.crt", "http://127.0.0.1:8090"
MEMBERS = int(os.environ.get("FC6_MEMBERS", "63"))
DELTAS = int(os.environ.get("FC6_DELTAS", "1000"))
PARTS = os.environ.get("FC6_PARTS", "ab")
UNITS = ["door.service", "arc-relay.service", "arc-node.service", "arc-gateway.service", "postgresql@16-main.service"]
CGROUP: dict = {}


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def cgroup(u):
    """A unit's own cgroup, as systemd names it: PostgreSQL's cluster sits under its own slice."""
    if u not in CGROUP:
        r = subprocess.run(["systemctl", "show", "-p", "ControlGroup", "--value", u], capture_output=True, text=True)
        CGROUP[u] = r.stdout.strip()
    return CGROUP[u]


def cpu():
    out = {}
    for u in UNITS:
        if not cgroup(u):
            continue
        try:
            for line in open(f"/sys/fs/cgroup{cgroup(u)}/cpu.stat"):
                if line.startswith("usage_usec"):
                    out[u] = int(line.split()[1])
        except OSError:
            pass
    st = open("/proc/stat").readline().split()[1:]
    vals = list(map(int, st))
    out["host_busy"], out["host_total"] = sum(vals) - vals[3] - vals[4], sum(vals)
    out["t"] = time.monotonic()
    return out


def cores(a, b):
    dt = b["t"] - a["t"]
    r = {u.replace(".service", ""): round((b[u] - a[u]) / 1e6 / dt, 2) for u in UNITS if u in a and u in b}
    r["host busy"] = f"{100 * (b['host_busy'] - a['host_busy']) / max(1, b['host_total'] - a['host_total']):.0f} % of {os.cpu_count()} vCPU"
    return r


def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(p * len(xs)))], 1) if xs else None


class Reader:
    """One member's session as the webapp holds it: the change stream, and a /v2/graph on each change."""
    def __init__(self, cookies):
        self.c = httpx.AsyncClient(base_url=DOOR, verify=w.ssl.create_default_context(cafile=CA), timeout=60,
                                   cookies=cookies, headers={"origin": DOOR})
        self.lat, self.errors, self.task, self.pending = [], 0, None, None

    async def read(self):
        t = time.monotonic()
        try:
            r = await self.c.get("/v2/graph")
            if r.status_code != 200:
                self.errors += 1
                return
            self.lat.append((time.monotonic() - t) * 1000)
        except Exception:
            self.errors += 1

    async def run(self):
        while True:
            try:
                async with self.c.stream("GET", "/v2/events", timeout=None) as s:
                    async for line in s.aiter_lines():
                        if line.startswith("event: changed") and (self.pending is None or self.pending.done()):
                            self.pending = asyncio.create_task(self.debounced())
            except asyncio.CancelledError:
                return
            except Exception:
                self.errors += 1
                await asyncio.sleep(1)

    async def debounced(self):
        await asyncio.sleep(0.15)
        await self.read()


async def posting(owner, room, rate, secs, lat):
    """Offers `rate` posts a second for `secs`, one at a time; each landed post's latency kept."""
    n, t0 = 0, time.monotonic()
    while time.monotonic() - t0 < secs:
        t = time.monotonic()
        r = await asyncio.to_thread(owner.post, "/v2/apply", json={"object": room, "op": "forum.post", "args": {"text": f"load {n}"}})
        if r.status_code == 200:
            lat.append((time.monotonic() - t) * 1000)
        n += 1
        await asyncio.sleep(max(0, t0 + n / rate - time.monotonic()))
    return n


async def main():
    os.environ.update({"E2E_DOOR": DOOR, "E2E_DOOR_CA": CA})
    w.DEPLOYED = DOOR
    s = w.Stack()
    s.up()
    say("setup: the owner")
    owner = w.signup(s, "fc6-owner")
    oc = owner["client"]
    site = oc.post("/v2/mint", json={"kind": "group", "draft": {"name": "fc6 Site"}}).json()["object_id"]
    room = oc.post("/v2/mint", json={"kind": "forum", "draft": {"name": "fc6 room"}}).json()["object_id"]
    at = int(time.time() * 1000)
    assert oc.post("/v2/apply", json={"object": site, "op": "base.setPart", "args": {"part": room, "role": "room", "at": at}}).status_code == 200
    assert oc.post("/v2/apply", json={"object": room, "op": "base.setParent", "args": {"parent": site, "role": "room", "at": at}}).status_code == 200
    for obj in (site, room):
        b = httpx.get(f"{ARC}/v1/bundle", timeout=30).text
        m = oc.post("/v2/add", json={"object": obj, "bundle": b})
        assert m.status_code == 200, m.text
        r = oc.post("/v2/apply", json={"object": obj, "op": "base.setRole", "args": {"member": m.json()["member"], "role": "admitter"}})
        assert r.status_code == 200, r.text
    # The Site registers the kiosk key it admits by (group.setClaimIssuer), as rehearsal leg 7
    # did on production: the rehearsal key, door-test's (arc/hosting/claim-keys.rehearsal.json).
    keys = json.loads((HERE.parents[1] / "arc/hosting/claim-keys.rehearsal.json").read_text())
    kid, key = next(iter(keys.items()))
    r = oc.post("/v2/apply", json={"object": site, "op": "group.setClaimIssuer", "args": {"kid": kid, "key": key}})
    assert r.status_code == 200, r.text
    say(f"setup: {DELTAS} posts into the room")
    t0 = time.monotonic()
    for i in range(DELTAS):
        r = oc.post("/v2/apply", json={"object": room, "op": "forum.post", "args": {"text": f"history {i}"}})
        assert r.status_code == 200, r.text
    say(f"setup: {DELTAS} posts in {time.monotonic() - t0:.0f} s")
    say(f"setup: {MEMBERS} members admitted by claim")
    members, t0 = [], time.monotonic()
    for i in range(MEMBERS):
        c = w.webapp(s)
        c.get("/join", params={"claim": w.claim(site)})
        who = w.signup(s, f"fc6-member-{i}", c)
        r = c.post("/v2/join")
        tries = 1
        while r.status_code in (503, 429) and tries < 20:
            await asyncio.sleep(3)
            r, tries = c.post("/v2/join"), tries + 1
        if r.status_code != 200:
            say(f"member {i}: /v2/join {r.status_code} {r.text[:120]}; stopping")
            return
        members.append(who)
    say(f"setup: {len(members)} of {MEMBERS} admitted in {time.monotonic() - t0:.0f} s")
    t0 = time.monotonic()
    while time.monotonic() - t0 < 120:
        held = sum(1 for m in members if room in {o.get("id") for o in m["client"].get("/v2/graph").json().get("objects", [])})
        if held == len(members):
            break
        await asyncio.sleep(5)
    say(f"setup: {held} of {len(members)} sessions hold the room")
    readers = [Reader(dict(m["client"].cookies)) for m in members]
    for rd in readers:
        rd.task = asyncio.create_task(rd.run())
    await asyncio.sleep(10)

    # (a) show load: 20 posts a minute for 5 min, sign-ins timed under it
    say("(a) steady state from now, 300 s")
    for rd in readers:
        rd.lat.clear()
    post_lat, signins, lost_errors = [], [], 0
    a0 = cpu()
    poster = asyncio.create_task(posting(oc, room, 20 / 60, 300, post_lat))
    for i, m in enumerate(members[:20]):
        await asyncio.sleep(12)
        readers[i].task.cancel()
        lost_errors += readers[i].errors
        await asyncio.to_thread(m["client"].post, "/v2/signout")
        t = time.monotonic()
        try:
            again = await asyncio.to_thread(w.signin, s, m)
            signins.append(time.monotonic() - t)
            m["client"] = again["client"]
            readers[i] = Reader(dict(again["client"].cookies))
            readers[i].task = asyncio.create_task(readers[i].run())
        except Exception as e:
            say(f"sign-in {i}: {type(e).__name__} {str(e)[:120]}")
            signins.append(float("inf"))
    posts = await poster
    a1 = cpu()
    reads = [x for rd in readers for x in rd.lat]
    ok = sum(1 for x in signins if x <= 3.0)
    say("(a) " + json.dumps({"posts": posts, "landed": len(post_lat), "post p50/p95 ms": [pct(post_lat, .5), pct(post_lat, .95)],
                             "reads": len(reads), "read p50/p95 ms": [pct(reads, .5), pct(reads, .95)],
                             "read errors": lost_errors + sum(rd.errors for rd in readers),
                             "sign-ins": len(signins), "within 3 s": ok, "sign-in p50/p95 s": [pct(signins, .5), pct(signins, .95)],
                             "pass (95 % within 3 s)": ok >= 0.95 * len(signins), "cores": cores(a0, a1)}))

    # (b) saturation: 1, 2, 5 posts a second, 60 s each
    for rate in ((1, 2, 5) if "b" in PARTS else ()):
        for rd in readers:
            rd.lat.clear()
        post_lat = []
        b0 = cpu()
        posts = await posting(oc, room, rate, 60, post_lat)
        await asyncio.sleep(5)
        b1 = cpu()
        reads = [x for rd in readers for x in rd.lat]
        say(f"(b) {rate}/s " + json.dumps({"posts": posts, "landed": len(post_lat), "post p50/p95 ms": [pct(post_lat, .5), pct(post_lat, .95)],
                                          "reads": len(reads), "read p50/p95 ms": [pct(reads, .5), pct(reads, .95)],
                                          "read errors": sum(rd.errors for rd in readers), "cores": cores(b0, b1)}))
    for rd in readers:
        rd.task.cancel()
    for m in members + [owner]:
        try:
            m["client"].post("/v2/signout")
        except Exception:
            pass
    say("done: every session signed out")

asyncio.run(main())
