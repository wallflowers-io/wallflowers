"""training_wheels P0 on a deployed Door (mdr/training-wheels.md § Acceptance), door-test only:
  TWG-1  a layer-1 run's requests to the recorded routes, counted by route and status, against
         the `action` rows written meanwhile (NOT backfilled): one row each, and the Delta and
         object ids its 2xx answers named, each in a row's outcome.
  TWG-2  PostgreSQL stopped mid-run with writes continuing, then started: no write refused, and
         once the spool drains, one row per write and every Delta written in `delta`.
  TWG-8  every credential those runs used (cookies, the words, codes, verifiers, DPoP proofs,
         access tokens, the claim, the sealed PRF) searched in action::text, delta::text and the
         spool. Nothing of them is printed: a match names its class only.
  restore  the backup (Ralph, 28 Sep: "to rebuild the database if the MLS system goes down"): the
         shadow database dumped, restored into a fresh cluster (TWG_RESTORE_PORT), and every table
         read back row for row the same; the /v2/join rows since TWG_JOIN_SINCE the same, and
         each args object in TWG_JOIN_EXPECT (a JSON list) among them; TWG-8's search over every restored table. The cluster and the dump removed.
         TWG_RESTORE_MUTATE=1 loses one restored /v2/join row: R.
In the VM, beside wallflowers_path.py's own settings (E2E_DOOR, E2E_DOOR_CA, E2E_AUTH, E2E_ARC,
E2E_AUTH_STOP/START, E2E_SITE_FILE unset):
  .venv/bin/python app/e2e/twg_check.py [twg1] [twg2] [twg8] [restore]"""
import json, os, subprocess, sys, time
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import parse_qs, urlsplit
import httpx
sys.path.insert(0, str(Path(__file__).resolve().parent))
import wallflowers_path as w

RECORDED = {"/v2/mint", "/v2/apply", "/v2/add", "/v2/site/address", "/v2/join", "/v2/signup/finish",
            "/v2/signup/continue", "/v2/signin/finish", "/v2/signout", "/v2/token"}
ACCOUNT = {"/v2/signup", "/v2/signup/finish", "/v2/signup/continue", "/v2/signin", "/v2/signin/finish", "/v2/token", "/join", "/v2/join"}
PSQL = ["sudo", "-u", "door", "psql", "host=/var/run/postgresql dbname=training_wheels user=door", "-Atqc"]
SPOOL = os.environ.get("TWG_SPOOL", "/var/lib/door/tw-spool")
PG_UNIT = os.environ.get("TWG_PG_UNIT", "postgresql@16-main")
SOURCE_PORT = os.environ.get("TWG_SOURCE_PORT", "5432")
RESTORE_PORT = os.environ.get("TWG_RESTORE_PORT", "5499")
JOIN_SINCE = os.environ.get("TWG_JOIN_SINCE", "")
JOIN_EXPECT = json.loads(os.environ.get("TWG_JOIN_EXPECT", "[]"))

SEEN: list = []                      # (route, method, status, delta, object)
SECRETS: dict = {}                   # value -> class; never printed
PUBLIC: set = set()                  # ids and keys a row may rightly hold


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def secret(v, cls):
    if isinstance(v, str) and len(v) >= 16 and v not in PUBLIC:
        SECRETS[v] = cls


def walk(o, cls):
    if isinstance(o, dict):
        for k, v in o.items():
            walk(v, f"{cls}.{k}")
    elif isinstance(o, list):
        if o and all(isinstance(x, str) and " " not in x for x in o) and len(o) >= 12:
            secret(" ".join(o), f"{cls} (the words)")
        for x in o:
            walk(x, cls)
    else:
        secret(o, cls)


def hook(r: httpx.Response):
    q = r.request
    path = urlsplit(str(q.url)).path
    for h in ("cookie", "dpop", "authorization"):
        if h in q.headers:
            for part in q.headers[h].split(";"):
                secret(part.split("=", 1)[-1].strip(), f"request {h}")
    for c in r.headers.get_list("set-cookie"):
        secret(c.split(";", 1)[0].split("=", 1)[-1], "set-cookie")
    if path == "/join":
        for v in parse_qs(urlsplit(str(q.url)).query).get("claim", []):
            secret(v, "the claim")
    body = None
    if path in ACCOUNT or path in RECORDED:
        try:
            body = json.loads(q.content or b"null")
        except (ValueError, UnicodeDecodeError, httpx.RequestNotRead):
            body = None
        if path in ACCOUNT and isinstance(body, dict):
            # A person's name and handle are theirs to show, not credentials: the name is set on
            # their profile by a Delta, and rightly there (TWG-8 names the claim, the wrap,
            # assertions, proofs, codes and verifiers).
            walk({k: v for k, v in body.items() if k not in ("name", "handle")}, f"{path} request")
    delta = obj = None
    if path in RECORDED or path in ACCOUNT:
        try:
            r.read()
            a = r.json()
        except Exception:
            a = None
        if isinstance(a, dict):
            delta, obj = a.get("delta"), a.get("object_id") or a.get("object")
            # Ids, not credentials: a token's `scope` is its Site's id (main.rs, /v2/token), and a sign-in's
            # `resume.spine` the self record's id (resumption.rs); rows rightly hold both (run 87).
            for k, v in (*((k, a.get(k)) for k in ("pk", "handle", "member", "object_id", "delta", "site", "rooms", "scope")),
                         ("resume.spine", (a.get("resume") or {}).get("spine") if isinstance(a.get("resume"), dict) else None)):
                for x in (v if isinstance(v, list) else [v]):
                    if isinstance(x, str):
                        PUBLIC.add(x)
                        SECRETS.pop(x, None)
            if path in ACCOUNT:
                walk({k: v for k, v in a.items() if k not in ("pk", "handle", "member", "site", "rooms")}, f"{path} answer")
    if path in RECORDED:
        SEEN.append((path, q.method, r.status_code, delta, obj))


def install():
    for cls in (httpx.Client,):
        init = cls.__init__
        def patched(self, *a, _init=init, **k):
            hooks = dict(k.pop("event_hooks", None) or {})
            hooks["response"] = list(hooks.get("response", [])) + [hook]
            _init(self, *a, event_hooks=hooks, **k)
        cls.__init__ = patched


def sql(q):
    r = subprocess.run(PSQL + [q], capture_output=True, text=True)
    if r.returncode:
        raise w.Fail(f"psql: {r.stderr.strip()[:300]}")
    return [l.split("|") for l in r.stdout.strip().splitlines() if l]


def now():
    return datetime.now(timezone.utc).isoformat()


def rows_between(t0, t1):
    return sql(f"select route, outcome->>'status', coalesce(outcome->>'delta',''), coalesce(outcome->>'object','') "
               f"from action where not backfilled and at >= '{t0}' and at <= '{t1}'")


def compare(label, t0, t1, seen):
    got = rows_between(t0, t1)
    want: dict = {}
    for (path, _m, st, _d, _o) in seen:
        want[(path, str(st))] = want.get((path, str(st)), 0) + 1
    have: dict = {}
    for (route, st, _d, _o) in got:
        have[(route, st)] = have.get((route, st), 0) + 1
    keys = sorted(set(want) | set(have))
    diff = {f"{k[0]} {k[1]}": [want.get(k, 0), have.get(k, 0)] for k in keys if want.get(k, 0) != have.get(k, 0)}
    ids_row = {d for (_r, _s, d, _o) in got if d} | {o for (_r, _s, _d, o) in got if o}
    ids_seen = {x for (_p, _m, st, d, o) in seen if 200 <= st < 300 for x in (d, o) if x}
    missing = sorted(ids_seen - ids_row)
    ok = not diff and not missing and len(got) == len(seen)
    say(f"{label}: {'G' if ok else 'R'}", json.dumps({
        "requests to recorded routes": len(seen), "action rows": len(got),
        "by route, requests=rows": {f"{k[0]} {k[1]}": want.get(k, 0) for k in keys if want.get(k, 0) == have.get(k, 0)},
        "unequal (requests, rows)": diff, "answered ids not in a row": len(missing)}))
    return ok


def twg1():
    # Layer 1's steps that restart nothing and wait on no idle timer (E10-E12 are TWG-2's kind).
    # Not E8: it reaches the Door from node (site_api.mjs), which this count cannot see; E15
    # makes the token exchange from here.
    os.environ["E2E_ONLY"] = "E0,E0b,E0c,E0d,E1,E2,E3,E4,E5,E6,E6b,E7,E7b,E9,E13,E14,E15"
    os.environ.setdefault("E2E_WORDS_SECS", "5")
    w.ONLY[:] = os.environ["E2E_ONLY"].split(",")
    w.WORDS = int(os.environ["E2E_WORDS_SECS"])
    t0 = now()
    SEEN.clear()
    code = w.main()
    # No layer-1 step claims an address: one call, whatever it answers (a refusal has a row too).
    s = w.Stack()
    s.up()
    o = w.signup(s, "twg1-address")
    site = o["client"].post("/v2/mint", json={"kind": "group", "draft": {"name": "twg1 address"}}).json().get("object_id")
    r = o["client"].post("/v2/site/address", json={"slug": f"twg1-{int(time.time())}", "host": site})
    say(f"TWG-1: /v2/site/address answered {r.status_code} {r.text[:120]}")
    o["client"].post("/v2/signout")
    time.sleep(3)
    t1 = now()
    say(f"TWG-1: layer 1 exit {code}")
    return compare("TWG-1", t0, t1, list(SEEN))


def twg2():
    s = w.Stack()
    s.up()
    owner = w.signup(s, "twg2-owner")
    c = owner["client"]
    room = c.post("/v2/mint", json={"kind": "forum", "draft": {"name": "twg2 room"}}).json()["object_id"]
    t0 = now()
    SEEN.clear()
    start, stopped, started, n = time.monotonic(), None, None, 0
    while time.monotonic() - start < 100:
        el = time.monotonic() - start
        if stopped is None and el >= 20:
            subprocess.run(["sudo", "systemctl", "stop", PG_UNIT], check=True)
            stopped = now()
            say(f"TWG-2: {PG_UNIT} stopped")
        if started is None and el >= 60:
            subprocess.run(["sudo", "systemctl", "start", PG_UNIT], check=True)
            started = now()
            say(f"TWG-2: {PG_UNIT} started")
        c.post("/v2/apply", json={"object": room, "op": "forum.post", "args": {"text": f"twg2 {n}"}})
        n += 1
        if n % 20 == 0:                         # the account routes too, while it is down
            v = w.signup(s, f"twg2-member-{n}")
            v["client"].post("/v2/signout")
            v2 = w.signin(s, v)
            v2["client"].post("/v2/signout")
        time.sleep(0.5)
    c.post("/v2/signout")
    for _ in range(60):                          # the drain
        left = subprocess.run(f"sudo sh -c 'cat {SPOOL}/* 2>/dev/null | wc -c'", shell=True, capture_output=True, text=True).stdout.strip()
        if left in ("0", ""):
            break
        time.sleep(2)
    time.sleep(3)
    t1 = now()
    refused = [(p, st) for (p, _m, st, _d, _o) in SEEN if not 200 <= st < 300]
    ok = compare("TWG-2 actions", t0, t1, list(SEEN)) and not refused
    posts = {d for (p, _m, st, d, _o) in SEEN if p == "/v2/apply" and 200 <= st < 300 and d}
    tapped = {r[0] for r in sql(f"select distinct delta_id from delta where object = '{room}'")}
    missing = posts - tapped
    ok = ok and not missing
    say(f"TWG-2: {'G' if ok else 'R'}", json.dumps({"writes": len(SEEN), "refused": refused[:5], "posts": len(posts),
        "posts tapped in delta": len(posts & tapped), "posts not tapped": len(missing), "spool bytes after drain": left,
        "database stopped": stopped, "started": started}))
    return ok


def twg8():
    dump = subprocess.run(PSQL[:-1] + ["-Atqc", "select action::text from action union all select delta::text from delta"],
                          capture_output=True, text=True).stdout
    spool = subprocess.run(f"sudo sh -c 'cat {SPOOL}/* 2>/dev/null'", shell=True, capture_output=True).stdout.decode("utf-8", "replace")
    hits = {}
    for v, cls in SECRETS.items():
        for where, text in (("action/delta", dump), ("spool", spool)):
            if v in text:
                hits[f"{cls} in {where}"] = hits.get(f"{cls} in {where}", 0) + 1
    ok = not hits
    say(f"TWG-8: {'G' if ok else 'R'}", json.dumps({"credentials searched": len(SECRETS),
        "classes": sorted({c.split(' (')[0].split('.')[0] for c in SECRETS.values()}), "rows searched (chars)": len(dump),
        "spool searched (chars)": len(spool), "found": hits}))
    return ok


def sh(cmd, check=True):
    r = subprocess.run(cmd, shell=isinstance(cmd, str), capture_output=True, text=True)
    if check and r.returncode:
        raise w.Fail(f"{str(cmd)[:80]}: {r.stderr.strip()[:300]}")
    return r.stdout


def pq(port, s) -> str:
    """One query as postgres on a cluster's port, times in UTC, so two clusters print a row alike."""
    return sh(["sudo", "-u", "postgres", "env", "PGTZ=UTC", "psql", "-p", port, "-d", "training_wheels", "-Atqc", s])


def snapshot(port) -> dict:
    """Every table's row count and the md5 of its rows' text in order; the /v2/join rows since JOIN_SINCE."""
    q = lambda s: [l.split("|") for l in pq(port, s).splitlines() if l]
    tables = sorted(r[0] for r in q("select schemaname||'.'||tablename from pg_tables "
                                     "where schemaname not in ('pg_catalog','information_schema')"))
    out = {"tables": {}}
    for tab in tables:
        n, h = q(f"select count(*), md5(coalesce(string_agg(t, E'\\n' order by t), '')) from (select x::text t from {tab} x) s")[0]
        out["tables"][tab] = [int(n), h]
    out["joins"] = [tuple(r) for r in q("select at, args::text, outcome->>'status' from action where route = '/v2/join' "
                                        f"and at >= '{JOIN_SINCE or '-infinity'}' order by at, args::text")]
    return out


def restore():
    dump, port = "/tmp/twg-restore.dump", RESTORE_PORT
    used = lambda: int(sh("df --output=used -B1M /var/lib/postgresql | tail -1").strip())
    used0 = used()
    before = snapshot(SOURCE_PORT)
    sh(f"sudo -u postgres sh -c 'umask 077; pg_dump -p {SOURCE_PORT} -Fc -f {dump} training_wheels'")
    after = snapshot(SOURCE_PORT)
    try:
        # Socket only, as the main cluster is (conf.d/socket.conf): pg_createcluster drops an empty -o value.
        sh(f"sudo pg_createcluster 16 twrestore --port {port}")
        sh("echo \"listen_addresses = ''\" | sudo tee /etc/postgresql/16/twrestore/conf.d/socket.conf >/dev/null")
        sh("sudo pg_ctlcluster 16 twrestore start")
        tcp = sh(f"ss -ltnH | awk '$4 ~ /:{port}$/'").strip()
        sh(f"sudo -u postgres createdb -p {port} training_wheels")
        sh(f"sudo -u postgres pg_restore -p {port} -d training_wheels --no-owner --no-acl {dump}")
        if os.environ.get("TWG_RESTORE_MUTATE"):   # the check shown red: the newest /v2/join row lost in the restore
            pq(port, "delete from action where id = (select id from action where route = '/v2/join' order by at desc limit 1)")
        restored = snapshot(port)
        text = "".join(pq(port, f"select x::text from {tab} x") for tab in restored["tables"])
    finally:
        sh(f"sudo pg_dropcluster 16 twrestore --stop", check=False)
        sh(f"sudo rm -f {dump}", check=False)
    gone = not sh(f"pg_lsclusters -h | awk '$2 == \"twrestore\"'") and not os.path.exists(dump)
    grew = used() - used0
    hits = {}
    for v, cls in SECRETS.items():
        if v in text:
            hits[cls] = hits.get(cls, 0) + 1
    # The dump is one snapshot; the Door's ticks may write beside it. The restore must equal a state the
    # source held, read just before the dump or just after.
    match = "before" if restored == before else "after" if restored == after else None
    near = after if match == "after" else before
    unequal = {t: [near["tables"].get(t), restored["tables"].get(t)]
               for t in sorted(set(near["tables"]) | set(restored["tables"])) if near["tables"].get(t) != restored["tables"].get(t)}
    joins = restored["joins"]
    held = [json.loads(j[1]) if j[1] else None for j in joins]
    absent = [a for a in JOIN_EXPECT if a not in held]
    ok = bool(match) and (not JOIN_SINCE or joins) and not absent \
        and not hits and gone and not tcp and grew < 16
    say(f"restore: {'G' if ok else 'R'}", json.dumps({
        "source written while dumping": before != after, "restored equals the source": match, "tables": {t: v[0] for t, v in restored["tables"].items()},
        "unequal (source, restored)": unequal, "/v2/join rows since": [JOIN_SINCE or "the start", len(joins)],
        "with c": sum('"c"' in j[1] for j in joins), "with a": sum('"a"' in j[1] for j in joins),
        "join rows equal": joins == near["joins"], "expected rows held": [len(JOIN_EXPECT) - len(absent), len(JOIN_EXPECT)],
        "credentials searched": len(SECRETS), "restored chars searched": len(text), "found": hits, "cluster and dump removed": gone,
        "TCP listener on the restore port": bool(tcp), "volume MB used, before and after": [used0, used0 + grew]}))
    return ok


if __name__ == "__main__":
    want = sys.argv[1:] or ["twg1", "twg2", "twg8"]
    install()
    results = {t: {"twg1": twg1, "twg2": twg2, "twg8": twg8, "restore": restore}[t]() for t in want}
    say("done:", json.dumps({k: "G" if v else "R" for k, v in results.items()}))
    sys.exit(0 if all(results.values()) else 1)
