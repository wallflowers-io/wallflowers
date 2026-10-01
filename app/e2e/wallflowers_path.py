"""The WallFlowers path, end to end, on loopback: layer 1 (srr/vv.md; mdr/door.md).

    make e2e                      # or: .venv/bin/python app/e2e/wallflowers_path.py

The real relay, arc-node, gateway, auth service and Door, built from this tree and
started on free ports of 127.0.0.1, with their state in a temp directory. The test
plays the sign-in window (DR-4): a software authenticator and a PRF value, sealed as
DR-4 seals them. That window is a SIMULATION (SWE-070): a pass here does not qualify
a real authenticator; layer 2, in a browser, does that.

Each step checks one piece of the path. A piece that is not built fails as
    ABSENT: <piece> (<requirement>, <owner>)
never as a skip, and never stood in for by a fixture. A step whose prerequisites did
not pass is NOT REACHED, naming them; the others still run. The exit status is 0
only when every step passes.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import httpx

PRODUCT = Path(__file__).resolve().parents[2]
WS = PRODUCT.parent
ARC = PRODUCT / "arc"
DOOR = PRODUCT / "app" / "door"
# THE AUTH SERVICE AT A COMMIT, NEVER THE WORKING TREE. website's WIP tree lacked the seal counter
# (ed8bdc5) the Door has called since 086f75a, so every loopback sign-up failed there (NC-89).
# By default the commit that deploys, as security.rs's AUTH_REF (NC-73); E2E_AUTH_REF names another.
WEBSITE = WS / "business" / "website"
AUTH_REF = os.environ.get("E2E_AUTH_REF", "be38a21")
EGREGORE = WS / "business" / "collaborations" / "egregore"
DOOR_TS = EGREGORE / "site" / "lib" / "social" / "door.ts"
# The kiosk's own claim issuer (A-3), and the rehearsal's key: core's test vector, seed [9; 32].
CLAIM_MJS = EGREGORE / "egg" / "kiosk" / "claim.mjs"
CLAIM_KEYS = PRODUCT / "arc" / "hosting" / "claim-keys.rehearsal.json"
KIOSK_SEED = base64.urlsafe_b64encode(bytes([9] * 32)).rstrip(b"=").decode()
# The proof of work a start pays (D-55), solved by the window's own worker under node.
POW_JS = PRODUCT / "app" / "web" / "door" / "pow-worker.js"
SITE_API = Path(__file__).resolve().parent / "site_api.mjs"
ICD = PRODUCT / "core" / "coordination" / "delta-graph.icd.json"
UVICORN = WS / ".venv" / "bin" / "uvicorn"
CARGO = Path.home() / ".cargo" / "bin" / "cargo"

LAYER = "layer 1: the sign-in window is simulated (SWE-070)"
# macOS checks a binary before its first instruction runs (syspolicyd). On this Mac, with
# every session building, that has held a binary for minutes: 26 s measured (run 13), 18
# minutes seen by Software Engineering, and runs 15's pieces past 120 s with nothing
# printed. A native piece prints as it starts, so its UP budget runs from its first line;
# the wait before that is the OS's, bounded by LAUNCH and reported as the OS's.
LAUNCH = 1800
UP = 30
# E2E_EDGE=caddy: the Door behind a TLS edge that proxies and rewrites nothing, its CA
# Caddy's own (`tls internal`), trusted by the test's clients and no one else (K-43).
EDGE = os.environ.get("E2E_EDGE", "")
# E2E_DOOR=<url>: a Door already deployed, behind its own edge (K-43). Nothing is built or
# started here; E2E_DOOR_CA is the only CA its TLS is checked against, E2E_AUTH the auth
# service it stores to, whose wraps E3 reads.
DEPLOYED = os.environ.get("E2E_DOOR", "").rstrip("/")
# How long a session may sit idle: given to a Door this test starts, and for a deployed
# one, the DOOR_IDLE_SECS it was deployed with. The Door sweeps every 10 s.
IDLE = int(os.environ.get("E2E_IDLE_SECS", "60"))
SWEEP = 10
# How long E15's visitor spends on the recovery words before Continue (NC-53).
WORDS = int(os.environ.get("E2E_WORDS_SECS", "300"))
# E2E_ONLY=E2,E3,...: run these steps and no others (a deployed Door's check runs no step
# that restarts it or stops its auth service). A step needing one not run is NOT REACHED.
ONLY = [s.strip() for s in os.environ.get("E2E_ONLY", "").split(",") if s.strip()]
# E2E_SITE_FILE: on a deployed Door, a Site it already registers and that Site's founder
# (app/e2e/mint_site.py writes it), for E8 in place of a registration of its own.
SITE_FILE = os.environ.get("E2E_SITE_FILE", "")
# E2E_PROFILE=release: build and run the release binaries, as deployed, where time is
# measured (perf_audit.py). Debug otherwise.
PROFILE = "release" if os.environ.get("E2E_PROFILE") == "release" else "debug"


class Absent(Exception):
    def __init__(self, piece: str, reqs: str, owner: str):
        super().__init__(f"ABSENT: {piece} ({reqs}, {owner})")


class Fail(Exception):
    pass


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def cpu_secs(pid: int) -> float:
    """The CPU time a process has used: none while the OS holds it before its first instruction.
    Unknown, 0, where `ps` may not run (under `no-egress.sb`): a piece's first line then starts it."""
    try:
        out = subprocess.run(["ps", "-o", "time=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    except OSError:
        return 0.0
    try:
        m, s = out.rsplit(":", 1)
        return int(m.split("-")[-1].split(":")[-1]) * 60 + float(s)
    except ValueError:
        return 0.0


def wait_for(url: str, what: str, secs: float = 30, proc: subprocess.Popen | None = None,
             log: Path | None = None, prints: bool = False, verify: bool | ssl.SSLContext = True) -> None:
    """Until `url` answers. With `prints`, the piece prints as it starts: `secs` runs from
    its first line or its first CPU time, and until then it waits on the OS, up to LAUNCH."""
    t0 = last = time.monotonic()
    start = None if prints else t0
    while True:
        now = time.monotonic()
        if proc is not None and proc.poll() is not None:
            break
        if start is None and ((log is not None and log.exists() and log.stat().st_size > 0)
                              or (proc is not None and cpu_secs(proc.pid) > 0.05)):
            start = now
        if start is None:
            if now - t0 > LAUNCH:
                raise Fail(f"{what} was never launched: the OS held it {LAUNCH} s before its first "
                           f"instruction (macOS's launch check); not a result")
            if now - last >= 60:
                print(f"  waiting on the OS to launch {what}: {now - t0:.0f} s", file=sys.stderr, flush=True)
                last = now
        elif now - start > secs:
            break
        try:
            if httpx.get(url, timeout=1, verify=verify).status_code < 500:
                return
        except httpx.HTTPError:
            pass
        time.sleep(0.2)
    said = log.read_text(errors="replace").strip()[-600:] if log and log.exists() else ""
    how = "" if proc is None else (f"; it exited {proc.returncode}" if proc.poll() is not None
                                   else f"; still running {secs:g} s after it started")
    raise Fail(f"{what} did not come up at {url}{how}" + (f"; its log: {said}" if said else "; its log is empty" if log else ""))


class Stack:
    """The loopback system. Everything it starts, it stops."""

    def __init__(self, host: str = "127.0.0.1") -> None:
        # The Door's name. A browser's WebAuthn refuses an IP origin, so layer 2 uses
        # `localhost`; the processes still bind 127.0.0.1.
        self.host = host
        self.work = Path(tempfile.mkdtemp(prefix="wf-e2e."))
        self.procs: list[subprocess.Popen] = []
        self.relay = free_port()
        self.auth = free_port()
        self.node = free_port()
        self.gw = free_port()
        self.door = free_port()
        if EDGE not in ("", "caddy"):
            raise Fail(f"E2E_EDGE={EDGE}: the edge this test knows is caddy")
        self.edge = free_port() if EDGE and not DEPLOYED else None
        self.verify: bool | ssl.SSLContext = True
        self.ca: Path | None = None
        if DEPLOYED:
            if os.environ.get("E2E_DOOR_CA"):
                self.ca = Path(os.environ["E2E_DOOR_CA"])
                self.verify = ssl.create_default_context(cafile=str(self.ca))
            if not os.environ.get("E2E_AUTH"):
                raise Fail("E2E_DOOR names a deployed Door: E2E_AUTH must name the auth service it stores to")

    def build(self) -> None:
        if DEPLOYED:
            return
        for cwd, args in ((ARC, ["build", "-p", "relay", "-p", "node", "-p", "arc-gateway"]), (DOOR, ["build"])):
            args += ["--release"] if PROFILE == "release" else []
            r = subprocess.run([str(CARGO), *args], cwd=cwd, capture_output=True, text=True)
            if r.returncode:
                raise Fail(f"cargo {' '.join(args)} in {cwd.relative_to(PRODUCT)} failed:\n{r.stderr[-2000:]}")

    def spawn(self, name: str, cmd: list[str], env: dict, cwd: Path | None = None) -> None:
        log = open(self.work / f"{name}.log", "w")
        self.procs.append(subprocess.Popen(cmd, env={**os.environ, **env}, cwd=cwd, stdout=log, stderr=subprocess.STDOUT))

    def up(self) -> None:
        if DEPLOYED:
            wait_for(f"{DEPLOYED}/v2/icd", "the deployed Door", UP, verify=self.verify)
            return
        self.start_relay("relay")
        self.auth_dir = self.export_auth()
        if not UVICORN.exists():
            raise Fail(f"no auth service: expected {UVICORN}")
        self.start_auth()
        self.spawn("node", [str(ARC / "target" / PROFILE / "node")],
                   {"PACIFIC_STATE_DIR": str(self.work / "arc"), "ARC_STATE_DIR": str(self.work / "arc-state"),
                    "ARC_NAME": "e2e Arc", "ARC_RELAY_URL": f"ws://127.0.0.1:{self.relay}",
                    "ARC_SIGNUP_BIND": f"127.0.0.1:{self.node}", "ARC_SLUG_AUTHORITY": self.auth_url, "ARC_SYNC_MS": "1000",
                    "ARC_CLAIM_KEYS": str(CLAIM_KEYS)})
        wait_for(f"http://127.0.0.1:{self.node}/health", "arc-node", UP, proc=self.procs[-1], log=self.work / "node.log", prints=True)
        self.spawn("gateway", [str(ARC / "target" / PROFILE / "arc-gateway")],
                   {"PORT": str(self.gw), "ARC_MEMBERSHIP_UPSTREAM": f"http://127.0.0.1:{self.node}",
                    "ARC_RELAY_UPSTREAM": f"ws://127.0.0.1:{self.relay}"})
        wait_for(f"http://127.0.0.1:{self.gw}/v1/health", "the gateway", UP, proc=self.procs[-1], log=self.work / "gateway.log", prints=True)
        (self.work / "door").mkdir()
        self.start_door("door", {})

    def start_door(self, log: str, env: dict) -> None:
        self.door_env = env
        self.spawn(log, [str(DOOR / "target" / PROFILE / "door")],
                   {"DOOR_PORT": str(self.door), "DOOR_ROOT": str(self.work / "door"),
                    "DOOR_RELAY": f"ws://127.0.0.1:{self.relay}/v1/relay", "DOOR_ORIGIN": self.door_url,
                    "DOOR_AUTH": self.auth_url, "DOOR_PUBLIC": self.door_url, "DOOR_IDLE_SECS": str(IDLE),
                    "DOOR_CLAIM_KEYS": str(CLAIM_KEYS), "DOOR_ARC": f"http://127.0.0.1:{self.gw}", **env})
        self.door_proc = self.procs[-1]
        wait_for(f"http://127.0.0.1:{self.door}", "the Door", UP, proc=self.door_proc, log=self.work / f"{log}.log", prints=True)
        if self.edge and self.ca is None:
            self.start_edge()

    def start_edge(self) -> None:
        """Caddy in front of the Door: TLS from its own CA, a reverse proxy, nothing rewritten."""
        caddy = shutil.which("caddy")
        if not caddy:
            raise Fail("E2E_EDGE=caddy, and there is no caddy on PATH")
        conf, data = self.work / "Caddyfile", self.work / "caddy"
        conf.write_text("{\n\tadmin off\n\tauto_https disable_redirects\n\tskip_install_trust\n\tstorage file_system " + str(data) + "\n}\n"
                        f"https://{self.host}:{self.edge} {{\n\ttls internal\n\treverse_proxy 127.0.0.1:{self.door}\n}}\n")
        self.spawn("edge", [caddy, "run", "--config", str(conf), "--adapter", "caddyfile"], {})
        root = data / "pki" / "authorities" / "local" / "root.crt"
        end = time.monotonic() + UP
        while not root.exists():
            if time.monotonic() > end or self.procs[-1].poll() is not None:
                raise Fail(f"the edge made no CA in {UP} s: {(self.work / 'edge.log').read_text()[-600:]}")
            time.sleep(0.2)
        self.ca, self.verify = root, ssl.create_default_context(cafile=str(root))
        wait_for(self.door_url, "the edge", UP, proc=self.procs[-1], log=self.work / "edge.log", verify=self.verify)

    def start_relay(self, log: str) -> None:
        # Durable, as a deployed relay is: a restart keeps what it holds (E12).
        self.spawn(log, [str(ARC / "target" / PROFILE / "semaphore")],
                   {"RELAY_BIND": f"127.0.0.1:{self.relay}", "RELAY_TUNNEL_BIND": f"127.0.0.1:{free_port()}",
                    "RELAY_STORE": str(self.work / "relay.db")})
        self.relay_proc = self.procs[-1]
        end = time.monotonic() + UP
        while True:
            try:
                socket.create_connection(("127.0.0.1", self.relay), timeout=0.5).close()
                return
            except OSError:
                if time.monotonic() > end or self.relay_proc.poll() is not None:
                    raise Fail(f"the relay did not come up on {self.relay}: {(self.work / f'{log}.log').read_text()[-600:]}")
                time.sleep(0.2)

    def restart_relay(self) -> None:
        """The relay stopped and started again under every open session (E2E_RELAY_RESTART for a
        deployed one, e.g. `sudo systemctl restart arc-relay`)."""
        if DEPLOYED:
            cmd = os.environ.get("E2E_RELAY_RESTART")
            if not cmd:
                raise Fail("a deployed Door: set E2E_RELAY_RESTART to the command that restarts its relay")
            subprocess.run(cmd, shell=True, check=True)
            return
        self.relay_proc.terminate()
        self.relay_proc.wait(timeout=30)
        self.start_relay("relay-restarted")

    def export_auth(self) -> Path:
        """website's auth/ at AUTH_REF, exported into this stack's own directory."""
        rev = subprocess.run(["git", "-C", str(WEBSITE), "rev-parse", "--short", f"{AUTH_REF}^{{commit}}"],
                             capture_output=True, text=True)
        if rev.returncode:
            raise Fail(f"no auth service: {WEBSITE} has no commit {AUTH_REF!r} (E2E_AUTH_REF): {rev.stderr.strip()[:200]}")
        out = self.work / "website"
        out.mkdir()
        tar = subprocess.run(f"git -C '{WEBSITE}' archive '{AUTH_REF}' auth | tar -x -C '{out}'", shell=True,
                             capture_output=True, text=True)
        if tar.returncode or not (out / "auth" / "app" / "main.py").exists():
            raise Fail(f"no auth service at {AUTH_REF}: {tar.stderr.strip()[:200]}")
        print(f"the auth service: website {rev.stdout.strip()} ({AUTH_REF})")
        return out / "auth"

    def start_auth(self, log: str = "auth") -> None:
        if DEPLOYED:
            cmd = os.environ.get("E2E_AUTH_START")
            if not cmd:
                raise Fail("a deployed Door: set E2E_AUTH_START to the command that starts its auth service")
            subprocess.run(cmd, shell=True, check=True)
            wait_for(f"{self.auth_url}/auth/health", "the auth service", UP)
            return
        self.spawn(log, [str(UVICORN), "app.main:app", "--host", "127.0.0.1", "--port", str(self.auth)],
                   {"DB_PATH": str(self.work / "auth.db"), "PUBLIC_ORIGIN": self.auth_url}, cwd=self.auth_dir)
        self.auth_proc = self.procs[-1]
        wait_for(f"{self.auth_url}/auth/health", "the auth service", proc=self.auth_proc, log=self.work / f"{log}.log")

    def stop_auth(self) -> None:
        if DEPLOYED:
            cmd = os.environ.get("E2E_AUTH_STOP")
            if not cmd:
                raise Fail("a deployed Door: set E2E_AUTH_STOP to the command that stops its auth service")
            subprocess.run(cmd, shell=True, check=True)
            return
        self.auth_proc.terminate()
        self.auth_proc.wait(timeout=30)

    def restart_door(self) -> None:
        """Stop the Door as its host would (SIGTERM, or E2E_DOOR_RESTART for a deployed one)
        and start it again. Every session it held ends."""
        if DEPLOYED:
            cmd = os.environ.get("E2E_DOOR_RESTART")
            if not cmd:
                raise Fail("a deployed Door: set E2E_DOOR_RESTART to the command that restarts it")
            subprocess.run(cmd, shell=True, check=True)
            wait_for(f"{DEPLOYED}/v2/icd", "the deployed Door, restarted", UP, verify=self.verify)
            return
        self.stop_door()
        self.start_door("door-restarted", self.door_env)

    def stop_door(self) -> None:
        """As systemd stops a unit: SIGTERM to the Door and to each session process it
        started, then wait for every one of them to exit (door.service: TimeoutStopSec=45)."""
        kids = subprocess.run(["pgrep", "-P", str(self.door_proc.pid)], capture_output=True, text=True).stdout.split()
        self.door_proc.terminate()
        for k in kids:
            try:
                os.kill(int(k), signal.SIGTERM)
            except ProcessLookupError:
                pass
        self.door_proc.wait(timeout=45)
        end = time.monotonic() + 45
        for k in kids:
            while time.monotonic() < end:
                try:
                    os.kill(int(k), 0)
                except ProcessLookupError:
                    break
                time.sleep(0.2)
            else:
                raise Fail(f"a session process ({k}) was still running 45 s after the Door was stopped")

    def register(self, clients: dict) -> None:
        """The Door's stand-in client registration, DOOR_CLIENTS, until D-32 makes it a Site
        op. The Door reads it at start, so it restarts, and every session it held ends."""
        if DEPLOYED:
            raise Absent("a Site's registration of its sign-in client, read by a Door already running: the deployed "
                         "stand-in (DOOR_CLIENTS) is fixed at deploy and cannot name a Site minted after it", "D-32, SEC-35",
                         "Software Engineering")
        path = self.work / "clients.json"
        path.write_text(json.dumps(clients))
        self.stop_door()
        self.start_door("door-registered", {"DOOR_CLIENTS": str(path)})

    @property
    def auth_url(self) -> str:
        return os.environ["E2E_AUTH"].rstrip("/") if DEPLOYED else f"http://127.0.0.1:{self.auth}"

    @property
    def door_url(self) -> str:
        """Where a client reaches the Door: the edge when there is one."""
        if DEPLOYED:
            return DEPLOYED
        return f"https://{self.host}:{self.edge}" if self.edge else f"http://{self.host}:{self.door}"

    def down(self) -> None:
        for p in self.procs:
            p.terminate()
        for p in self.procs:
            try:
                p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                p.kill()
        if not os.environ.get("E2E_KEEP"):
            shutil.rmtree(self.work, ignore_errors=True)


def route(stack: Stack, method: str, path: str, piece: str, reqs: str, owner: str, **kw) -> httpx.Response:
    """Call a route the path needs; a route that is not there is an absent piece."""
    r = httpx.request(method, stack.door_url + path, timeout=30, verify=stack.verify, **kw)
    if r.status_code in (404, 405):
        raise Absent(piece, reqs, owner)
    return r


# The window, played. DR-4's sealer is app/web/door/seal.js, called under node rather than
# written again here: the PRF output goes to the session's key exactly as the window sends it.
SEAL = PRODUCT / "app" / "web" / "door" / "seal.js"


def seal(key: str, prf: bytes, attempt: str) -> dict:
    if not SEAL.exists():
        raise Absent("the window's sealer, app/web/door/seal.js", "SEC-37, ICD-4", "Software Engineering")
    js = ("const {seal}=require(process.argv[1]);"
          "Promise.resolve(seal(process.argv[2],Buffer.from(process.argv[3],'hex'),process.argv[4]))"
          ".then(x=>process.stdout.write(JSON.stringify(x)))")
    r = subprocess.run(["node", "-e", js, str(SEAL), key, prf.hex(), attempt], capture_output=True, text=True)
    if r.returncode:
        raise Fail(f"seal.js failed: {r.stderr[-500:]}")
    return __import__("json").loads(r.stdout)


def work(c: httpx.Client, start: str) -> dict:
    """D-55: the proof of work `start` (signup or signin) owes, solved as the window solves it, as
    `{"work": …}` for its body; `{}` when none is owed, or from a Door before D-55."""
    r = c.get("/v2/work", params={"for": start})
    if r.status_code == 404:
        return {}
    if r.status_code != 200:
        raise Fail(f"/v2/work?for={start}: {r.status_code} {r.text[:200]}")
    challenge = r.json().get("challenge")
    if not challenge:
        return {}
    js = "process.stdout.write(require(process.argv[1]).solve(process.argv[2]))"
    s = subprocess.run(["node", "-e", js, str(POW_JS), challenge], capture_output=True, text=True)
    if s.returncode:
        raise Fail(f"pow-worker.js: {s.stderr[-300:]}")
    # A second reading of the contract: SHA-256(seed ‖ nonce digits) has `bits` leading zero bits.
    _, seed, _, bits, _ = challenge.split(".")
    digest = hashlib.sha256(base64.urlsafe_b64decode(seed + "=" * (-len(seed) % 4)) + s.stdout.encode()).digest()
    if int.from_bytes(digest, "big") >> (256 - int(bits)):
        raise Fail(f"pow-worker.js's nonce {s.stdout} does not do {challenge}'s work")
    return {"work": {"challenge": challenge, "nonce": s.stdout}}


def webapp(s: Stack, timeout: int = 60) -> httpx.Client:
    """A client as the webapp is one: its fetch sends the Door's own Origin (DV-9)."""
    return httpx.Client(base_url=s.door_url, timeout=timeout, verify=s.verify, headers={"origin": s.door_url})


def signup(s: Stack, name: str, c: httpx.Client | None = None) -> dict:
    """One person through sign-up: a new passkey's PRF output, sealed to the session."""
    c = c or webapp(s, 30)
    r = c.post("/v2/signup", json={"name": name, **work(c, "signup")})
    if r.status_code in (404, 405):
        raise Absent("the Door's sign-up, /v2/signup", "R1.1, RD.4", "Software Engineering")
    if r.status_code != 200:
        raise Fail(f"/v2/signup: {r.status_code} {r.text[:200]}")
    o = r.json()
    prf = os.urandom(32)
    r = c.post("/v2/signup/finish", json={"attempt": o["attempt"], "sealed": seal(o["key"], prf, o["attempt"])})
    if r.status_code in (404, 405):
        raise Absent("/v2/signup/finish", "R1.1, SEC-37", "Software Engineering")
    if r.status_code != 200 or key(r.json().get("pk", "")) != key(o["pk"]):
        raise Fail(f"/v2/signup/finish: {r.status_code} {r.text[:200]}")
    return {"client": c, "pk": o["pk"], "handle": o["handle"], "prf": prf}


def signin(s: Stack, who: dict) -> dict:
    """The same passkey on a device that has never seen the account: a new client, no cookie."""
    c = webapp(s)
    r = c.post("/v2/signin", json=work(c, "signin") or None)
    if r.status_code in (404, 405):
        raise Absent("the Door's sign-in, /v2/signin", "R10.1, RD.4", "Software Engineering")
    o = r.json()
    r = c.post("/v2/signin/finish", json={"attempt": o["attempt"], "handle": who["handle"],
                                           "sealed": seal(o["key"], who["prf"], o["attempt"])})
    if r.status_code in (404, 405):
        raise Absent("/v2/signin/finish", "R10.1, SEC-4", "Software Engineering")
    if r.status_code != 200 or key(r.json().get("pk", "")) != key(who["pk"]):
        raise Fail(f"/v2/signin/finish: {r.status_code} {r.text[:200]}")
    return {"client": c, "resume": r.json().get("resume")}


def key(pk: str) -> str:
    """An identity key in one form: /v2/me says `ed25519:<hex>`, sign-up says `<hex>`."""
    return pk.removeprefix("ed25519:").lower()


def me(c: httpx.Client) -> dict:
    r = c.get("/v2/me")
    if r.status_code in (404, 405):
        raise Absent("the Door's /v2/me", "R9.1, ICD-4", "Software Engineering")
    if r.status_code != 200:
        raise Fail(f"/v2/me: {r.status_code} {r.text[:200]}")
    return r.json()


def e0_site(s: Stack, st: dict) -> str:
    st["founder"] = signup(s, "founder")
    r = st["founder"]["client"].post("/v2/mint", json={"kind": "group", "draft": {"name": "Egregore"}})
    if r.status_code in (404, 405):
        raise Absent("/v2/mint", "R5.1, R8.4", "Software Engineering")
    if r.status_code != 200 or "object_id" not in r.json():
        raise Fail(f"/v2/mint: {r.status_code} {r.text[:200]}")
    st["site"] = r.json()["object_id"]
    return f"Site {st['site'][:12]} minted"


def e0b_founders(s: Stack, st: dict) -> str:
    # A-10 is ruled (a): every Delta signed and verified; a role-gated op refused at write and
    # at fold unless its author holds the role on the chain of succession from the three
    # founding roles. The creation by three is not built yet.
    raise Absent("the Site's creation signed by three founders, with Chairperson, Treasurer and Secretary",
                 "R5.4, R5.5, RX.7, A-10", "Software Engineering")


def e0c_role_gate(s: Stack, st: dict) -> str:
    raise Absent("a role-gated op by a member without the role, refused at /v2/apply and named",
                 "RX.7, RX.8, A-10", "Software Engineering")


def e0d_grant_gate(s: Stack, st: dict) -> str:
    raise Absent("a role granted by someone who does not hold the granting role, refused and named",
                 "RX.7, RX.8, A-10", "Software Engineering")


def claim(site: str, ttl: int = 600, ago: int = 0, seed: str = KIOSK_SEED, choice: str = "skills") -> str:
    """A claim as the kiosk issues one: its own issuer, egregore egg/kiosk/claim.mjs, under node."""
    if not CLAIM_MJS.exists():
        raise Absent("the kiosk's claim issuer, egg/kiosk/claim.mjs", "R0.2, SEC-A1, A-3", "WALLFLOWERS")
    js = ("import(process.argv[1]).then(m => process.stdout.write(m.issueClaim(m.claimKey(process.argv[2]), "
          "JSON.parse(process.argv[3]))))")
    spec = {"site": site, "choice": choice, "ttlSeconds": ttl, "now": int((time.time() - ago) * 1000)}
    r = subprocess.run(["node", "-e", js, CLAIM_MJS.as_uri(), seed, json.dumps(spec)], capture_output=True, text=True)
    if r.returncode:
        raise Fail(f"claim.mjs: {r.stderr[-300:]}")
    return r.stdout


def e1_claim(s: Stack, st: dict) -> str:
    # R0.1, ICD-1, SEC-A1 (A-3): the kiosk's QR opens the Door's /join. The claim is checked
    # before any account exists, kept behind a Strict cookie, and answered 303 to /#join, so
    # it leaves the address bar; it reaches no record.
    site = st.get("site") or os.urandom(32).hex()
    c = httpx.Client(base_url=s.door_url, timeout=30, verify=s.verify)
    token = claim(site)
    r = c.get("/join", params={"claim": token})
    if r.status_code in (404, 405):
        raise Absent("the Door's /join", "R0.1, ICD-1, SEC-A1", "Software Engineering")
    set_cookie = r.headers.get("set-cookie", "")
    name = "__Host-door-join" if s.door_url.startswith("https:") else "door-join"
    attrs = [a.strip().lower() for a in set_cookie.split(";")]
    if r.status_code != 303 or r.headers.get("location") != "/#join":
        raise Fail(f"/join with a valid claim: {r.status_code} to {r.headers.get('location')!r}: {r.text[:200]}")
    if not set_cookie.startswith(name + "=") or not {"httponly", "samesite=strict"} <= set(attrs) or \
            (s.door_url.startswith("https:") and "secure" not in attrs):
        raise Fail(f"/join's cookie: {set_cookie!r}")
    if "no-store" not in r.headers.get("cache-control", "") or r.headers.get("referrer-policy") != "no-referrer":
        raise Fail(f"/join: Cache-Control {r.headers.get('cache-control')!r}, Referrer-Policy {r.headers.get('referrer-policy')!r} (DT-19)")
    age = next((a.split("=", 1)[1] for a in attrs if a.startswith("max-age=")), None)
    # Refused before any account, each for its own reason where the reason is the check.
    payload = claim(site).split(".")[1]
    head, _, sig = token.split(".")
    other = base64.urlsafe_b64encode(bytes([7] * 32)).rstrip(b"=").decode()
    bad = [("not a claim", "e2e", None), ("a forged signature", f"{head}.{payload}.{sig}", None),
           ("another kiosk's key", claim(site, seed=other), None), ("a life over the maximum", claim(site, ttl=3600), "at most"),
           ("dated in the future", claim(site, ago=-3600), "future"), ("expired", claim(site, ago=7200), "expired")]
    for label, t, words in bad:
        r = c.get("/join", params={"claim": t})
        if r.status_code != 400 or r.headers.get("set-cookie") or (words and words not in r.text):
            raise Fail(f"/join with {label}: {r.status_code} {r.text[:120]!r}, cookie {r.headers.get('set-cookie')!r}")
    if not DEPLOYED:
        n = token.split(".")[1]
        record = s.work / "door" / "refusals.jsonl"
        held = sum('"route":"/join"' in line for line in (record.read_text().splitlines() if record.exists() else []))
        if held < len(bad):
            raise Fail(f"{len(bad)} refusals at /join, {held} on the record (SEC-A4)")
        for f in (record, s.work / "door.log"):
            if f.exists() and (n in f.read_text() or sig in f.read_text()):
                raise Fail(f"a claim is in {f.name}")
    return (f"303 to /#join, {name} HttpOnly Strict (Max-Age {age}), no-store, no-referrer; refused 400, no cookie: "
            f"{', '.join(l for l, _, _ in bad)}" + ("" if DEPLOYED else "; no claim in the refusal record or the Door's log"))


def e2_signup(s: Stack, st: dict) -> str:
    st["visitor"] = signup(s, "visitor")
    return f"account {key(st['visitor']['pk'])[:12]}"


def e3_wrap(s: Stack, st: dict) -> str:
    r = httpx.get(f"{s.auth_url}/auth/users/{key(st['visitor']['pk'])}", timeout=10)
    if r.status_code != 200:
        raise Fail(f"the auth service holds no wrap for the account: {r.status_code}")
    return "wrap stored"


def e4_join(s: Stack, st: dict) -> str:
    # R4.1, R4.3, SEC-A1 (A-3): a kiosk's visitor becomes a member of the Site with no owner
    # acting. The founder, once: the Arc's node added to the Site (its bundle from the Arc's
    # /v1/bundle) and granted admitter; then signed out. The visitor: the kiosk's claim, /join,
    # a sign-up (no proof of work owed with a live join), /v2/join; the Arc's node admits them,
    # and the Site reaches their session. The claim once: a second account with it, 409.
    # E0's Site, fresh each run: the Arc's node is added once to a Site, never again.
    site, founder = st["site"], st["founder"]
    arc = os.environ.get("E2E_ARC", "") if DEPLOYED else f"http://127.0.0.1:{s.gw}"
    if not arc:
        raise Fail("a deployed Door: set E2E_ARC to the Arc gateway it admits through (DOOR_ARC)")
    b = httpx.get(f"{arc}/v1/bundle", timeout=30)
    if b.status_code in (404, 405):
        raise Absent("the Arc node's contact bundle, /v1/bundle", "R4.1, A-3", "Software Engineering")
    if b.status_code != 200:
        raise Fail(f"the Arc's /v1/bundle: {b.status_code} {b.text[:200]}")
    home = signin(s, founder)["client"]
    r = home.post("/v2/add", json={"object": site, "bundle": b.text})
    if r.status_code in (404, 405):
        raise Absent("/v2/add, an owner adding a member by contact bundle", "R4.1, A-3", "Software Engineering")
    if r.status_code != 200:
        raise Fail(f"the founder adds the Arc's node: /v2/add {r.status_code} {r.text[:200]}")
    node = r.json()["member"]
    r = home.post("/v2/apply", json={"object": site, "op": "base.setRole", "args": {"member": node, "role": "admitter"}})
    if r.status_code != 200:
        raise Fail(f"the founder grants admitter: {r.status_code} {r.text[:200]}")
    home.post("/v2/signout")
    token = claim(site)
    v = webapp(s)
    if v.get("/join", params={"claim": token}).status_code != 303:
        raise Fail("/join refused the kiosk's claim")
    visitor = signup(s, "a kiosk's visitor", v)
    # 503 keeps the claim for another try (9c8ac26): the node may not yet have synced the
    # founder's grant. Tried again for up to 30 s, and the tries reported.
    tries, t0 = 1, time.monotonic()
    r = v.post("/v2/join")
    while r.status_code == 503 and time.monotonic() - t0 < 30:
        time.sleep(2)
        tries += 1
        r = v.post("/v2/join")
    if r.status_code != 200 or r.json().get("site") != site or r.json().get("c") != "skills":
        raise Fail(f"/v2/join: {r.status_code} {r.text[:200]}")
    if "max-age=0" not in r.headers.get("set-cookie", "").lower():
        raise Fail(f"/v2/join did not clear the join cookie: {r.headers.get('set-cookie')!r}")
    t0 = time.monotonic()
    while site not in {o.get("id") for o in v.get("/v2/graph").json().get("objects", [])}:
        if time.monotonic() - t0 > 60:
            raise Fail("admitted, and the Site is not in the visitor's session after 60 s")
        time.sleep(1)
    took = time.monotonic() - t0
    if me(v).get("noncompliant"):
        raise Fail(f"the visitor's session holds objects that do not fold: {me(v)['noncompliant']}")
    st["member"] = visitor
    second = webapp(s)
    second.get("/join", params={"claim": token})
    signup(s, "a second account, the same claim", second)
    r = second.post("/v2/join")
    if r.status_code != 409:
        raise Fail(f"the same claim, a second account: /v2/join {r.status_code} {r.text[:200]}")
    time.sleep(5)
    if site in {o.get("id") for o in second.get("/v2/graph").json().get("objects", [])}:
        raise Fail("the second account holds the Site")
    again = second.post("/v2/join").status_code
    return (f"the Arc's node added and made admitter, the founder signed out; the kiosk's claim, /join, a sign-up, "
            f"/v2/join ({tries} {'try' if tries == 1 else 'tries'}): admitted ({r.status_code} for the same claim's second account, not a member), the Site in the "
            f"visitor's session in {took:.0f} s, nothing noncompliant; /v2/join with no claim waiting {again}")


def e5_artefact(s: Stack, st: dict) -> str:
    raise Absent("the Transaction's ops and the artefact op", "R2.1, R3.2, R3.4, K-1", "Software Engineering")


def e6_landing(s: Stack, st: dict) -> str:
    m = me(st["visitor"]["client"])
    if key(m.get("pk", "")) != key(st["visitor"]["pk"]):
        raise Fail(f"/v2/me names another account: {m}")
    return "signed in, no further prompt"


def e6b_holdings(s: Stack, st: dict) -> str:
    raise Absent("landing with the Site and the Thing in the graph", "R9.2, R9.3", "Software Engineering")


def e7_home(s: Stack, st: dict) -> str:
    st["home"] = signin(s, st["visitor"])
    if key(me(st["home"]["client"]).get("pk", "")) != key(st["visitor"]["pk"]):
        raise Fail("the fresh device signed in as someone else")
    return "a fresh device, restored from the wrap, signed in"


def e7b_order(s: Stack, st: dict) -> str:
    raise Absent("sites in join order, Egregore first", "R10.2, A-9", "Software Engineering")


# The egregore site's client, as WALLFLOWERS registers it for development (egregore 8971f54).
# E2E_CLIENT, E2E_CALLBACK: on a deployed Door, the client its registration names for E2E_SITE_FILE's Site
# (door-test's egregore-local, whose Site's owner the rehearsal holds).
CLIENT = os.environ.get("E2E_CLIENT", "egregores-echoes.com")
CALLBACK = os.environ.get("E2E_CALLBACK", "http://localhost:3100/signin/callback")


def b64u(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def dpop_key():
    from cryptography.hazmat.primitives.asymmetric import ec
    return ec.generate_private_key(ec.SECP256R1())


def dpop(k, htm: str, htu: str, token: str | None = None) -> str:
    """A DPoP proof (RFC 9449): ES256 over {jti, htm, htu, iat, ath?}, the jwk in the header."""
    from cryptography.hazmat.primitives import hashes
    from cryptography.hazmat.primitives.asymmetric import ec
    from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature
    n = k.public_key().public_numbers()
    jwk = {"kty": "EC", "crv": "P-256", "x": b64u(n.x.to_bytes(32, "big")), "y": b64u(n.y.to_bytes(32, "big"))}
    claims = {"jti": os.urandom(16).hex(), "htm": htm, "htu": htu, "iat": int(time.time())}
    if token:
        claims["ath"] = b64u(hashlib.sha256(token.encode()).digest())
    signed = b64u(json.dumps({"typ": "dpop+jwt", "alg": "ES256", "jwk": jwk}).encode()) + "." + b64u(json.dumps(claims).encode())
    r, s_ = decode_dss_signature(k.sign(signed.encode(), ec.ECDSA(hashes.SHA256())))
    return signed + "." + b64u(r.to_bytes(32, "big") + s_.to_bytes(32, "big"))


def registered(s: Stack, st: dict) -> None:
    """The site's client, registered for the Site: the Door's stand-in (DOOR_CLIENTS), which
    restarts the Door, once. A deployed Door's is fixed at deploy (E2E_SITE_FILE)."""
    if not (DEPLOYED and SITE_FILE) and not st.get("registered"):
        s.register({CLIENT: {"callbacks": [CALLBACK], "origins": ["http://localhost:3100"], "site": st["site"]}})
        st["registered"] = True


def e8_site_api(s: Stack, st: dict) -> str:
    # The Site's own member signs in for the site: the founder, until E4 admits a visitor.
    # The registration is the Door's stand-in (Stack.register), which restarts the Door;
    # every earlier session ends, so this step runs last.
    if not DOOR_TS.exists():
        raise Absent("the egregore site's /v2 client, site/lib/social/door.ts", "R8.1, R8.2, R8.3, R8.4", "WALLFLOWERS")
    kind = json.loads(ICD.read_text())["kinds"]["group"]
    op = next(name for name, o in kind["ops"].items() if o["op"] == 0)
    founder, site = st.get("founder"), st.get("site")
    if DEPLOYED and SITE_FILE:
        # A Site already registered on the deployed Door (the runbook's step 3), and its
        # founder: written by app/e2e/mint_site.py when the Site was minted.
        spec = json.loads(Path(SITE_FILE).read_text())
        founder = {"handle": spec["handle"], "prf": bytes.fromhex(spec["prf"]), "pk": spec["pk"]}
        site = spec["site"]
    registered(s, st)
    # The same person's object outside the Site, minted by a webapp session before the
    # site's token exists: the token's session holds it and must not show or touch it.
    home = signin(s, founder)
    r = home["client"].post("/v2/mint", json={"kind": "group", "draft": {"name": "outside the Site"}})
    if r.status_code != 200:
        raise Fail(f"/v2/mint outside the Site: {r.status_code} {r.text[:200]}")
    outside = r.json()["object_id"]
    verifier = b64u(os.urandom(32))
    state = os.urandom(8).hex()
    cb = {"client": CLIENT, "redirect_uri": CALLBACK, "state": state,
          "code_challenge": b64u(hashlib.sha256(verifier.encode()).digest())}
    c = webapp(s)
    o = c.post("/v2/signin", json=work(c, "signin") or None).json()
    r = c.post("/v2/signin/finish", json={"attempt": o["attempt"], "handle": founder["handle"],
                                           "sealed": seal(o["key"], founder["prf"], o["attempt"]), "client": cb})
    if r.status_code != 200 or "redirect" not in r.json():
        raise Fail(f"/v2/signin/finish for {CLIENT}: {r.status_code} {r.text[:200]}")
    u = urlsplit(r.json()["redirect"])
    q = parse_qs(u.query)
    if f"{u.scheme}://{u.netloc}{u.path}" != CALLBACK or q.get("state") != [state] or len(q.get("code", [])) != 1:
        raise Fail(f"the sign-in did not return to the registered callback with the state: {u.scheme}://{u.netloc}{u.path}")
    args = {"door": s.door_url, "public": s.door_url, "code": q["code"][0], "verifier": verifier, "client": CLIENT,
            "redirect_uri": CALLBACK, "site": site, "op": op, "args": {"displayName": "Egregore", "shape": "community"},
            "kind": "group", "draft": {"name": "e2e part"}, "outside": outside,
            "cookie": "; ".join(f"{k}={v}" for k, v in home["client"].cookies.items()),
            "args2": {"displayName": "Egregore, again", "shape": "community"}, "settle_ms": 10_000}
    r = subprocess.run(["node", str(SITE_API), str(DOOR_TS), json.dumps(args)], capture_output=True, text=True,
                       env={**os.environ, **({"NODE_EXTRA_CA_CERTS": str(s.ca)} if s.ca else {})})
    if r.returncode:
        raise Fail(f"site_api.mjs: {r.stderr[-500:]}")
    got = json.loads(r.stdout)
    if got["token"].get("type") != "DPoP":
        raise Fail(f"/v2/token: {got['token']}")
    for step in ("before", "minted", "wrote", "after"):
        if got[step].get("ok") is False:
            raise Fail(f"door.ts {step}: refused by {got[step]['by']}: {got[step]['why']}")
    spaces = lambda h: {sp["id"] for sp in h["spaces"]}
    if site not in spaces(got["before"]):
        raise Fail(f"the token's fold does not hold the Site: {sorted(spaces(got['before']))}")
    if got["minted"]["object"] not in spaces(got["after"]):
        raise Fail("what the token minted is not in its fold")
    if got["before"]["problems"] or got["after"]["problems"]:
        raise Fail(f"objects that do not fold: {got['before']['problems'] + got['after']['problems']}")
    # Scope (RA-10; NC-41, DQ-11).
    if outside in spaces(got["before"]) | spaces(got["after"]):
        raise Fail("the token's fold holds an object outside the Site (RA-10)")
    if got["outside_by_token"].get("ok") is not False:
        raise Fail("a write outside the Site through the site's token was taken (RA-10)")
    if got["outside_by_cookie"] != 200 or got["inside_again"].get("ok") is False:
        raise Fail(f"the scope control did not run: webapp write {got['outside_by_cookie']}, token write {got['inside_again']}")
    # The stream by its content: the Door answers 200 whatever the session said (fwd_stream).
    ev = got["events"]
    if ev["foreign"]:
        raise Fail(f"the token's stream carried what is not SSE ({got['stream']}): {ev['foreign'][:3]}")
    seen = ev["before"] + ev["outside"] + ev["inside"]
    bad = [e for e in seen if e["event"] != "changed" or not e["data"].isdigit()]
    if bad:
        raise Fail(f"the token's stream carried events that are not a version: {bad[:3]}")
    versions = [int(e["data"]) for e in seen]
    if versions != sorted(set(versions)):
        raise Fail(f"the token's stream's versions do not rise: {versions}")
    if not ev["inside"]:
        raise Fail(f"the token's stream ({got['stream']}) did not move for a write inside the Site ({versions}): "
                   f"the stream check proves nothing")
    if ev["outside"]:
        raise Fail(f"the token's stream moved for a write outside the Site (NC-41): {[e['data'] for e in ev['outside']]}")
    rev = subprocess.run(["git", "-C", str(EGREGORE), "log", "-1", "--format=%h", "--", str(DOOR_TS)],
                         capture_output=True, text=True).stdout.strip()
    return (f"door.ts (egregore {rev}) on a DPoP token: the Site folds, a part minted, {op} written, nothing "
            f"noncompliant; outside the Site nothing shown, the write refused ({got['outside_by_token']['by']}), "
            f"the stream still (versions {[e['data'] for e in ev['before']]}, none for the write outside, "
            f"{[e['data'] for e in ev['inside']]} for a write inside); the registration is the "
            f"Door's stand-in (DOOR_CLIENTS, D-32)")


def e10_idle(s: Stack, st: dict) -> str:
    # mdr/door.md §4: an idle session ends, its head stored first. The deploy rehearsal
    # found the sweeper dying at the first idle session it ended (c851025), after which
    # no session ever ended: so two sessions go idle, one after the other.
    if DEPLOYED and "E2E_IDLE_SECS" not in os.environ:
        raise Fail("a deployed Door: set E2E_IDLE_SECS to the DOOR_IDLE_SECS it was deployed with")
    pk = key(st["visitor"]["pk"])
    head = lambda: httpx.get(f"{s.auth_url}/auth/users/{pk}/head/meta", timeout=10).json().get("position")
    ended = []
    for n in (1, 2):
        c = signin(s, st["visitor"])["client"]
        before = head()
        r = c.post("/v2/mint", json={"kind": "group", "draft": {"name": f"before idle {n}"}})
        if r.status_code != 200:
            raise Fail(f"/v2/mint before idle {n}: {r.status_code} {r.text[:200]}")
        time.sleep(IDLE + 2 * SWEEP + 5)  # untouched: any request would count as activity
        code = c.get("/v2/me").status_code
        if code == 200:
            raise Fail(f"session {n} still answers after {IDLE + 2 * SWEEP + 5} s idle (idle {IDLE} s, sweep {SWEEP} s)")
        # Stored, not merely kept: the head moved past the write, and a device that signs in
        # next finds the chain whole (RD.5), not a head behind its spine (DQ-16).
        now = head()
        if now is None or (before is not None and now <= before):
            raise Fail(f"session {n} ended and the head did not move past its write: {before} before, {now} after")
        verdict = ((me(signin(s, st["visitor"])["client"]).get("resume") or {}).get("chain") or {}).get("verdict")
        if verdict != "Whole":
            raise Fail(f"after session {n} ended, the next sign-in finds the chain {verdict!r}, not Whole")
        ended.append(f"{code}, head {before} to {now}, next sign-in Whole")
    if httpx.get(f"{s.door_url}/v2/icd", timeout=10, verify=s.verify).status_code != 200:
        raise Fail("the Door stopped answering after ending idle sessions")
    return f"two sessions left idle each ended ({'; '.join(ended)}), the second after the first; the Door answers"


def e11_restart(s: Stack, st: dict) -> str:
    # NC-47: a restart ends every session, and a head update not yet stored must be stored
    # as it ends (2b398f0: each session ends by syncing, then storing its head). A mint
    # stores its head before it answers, so the store is made to fail: the auth service is
    # away while the session writes, and back just before the Door is restarted, before the
    # next tick (every 3 s) retries it.
    pk = key(st["visitor"]["pk"])
    head = lambda: httpx.get(f"{s.auth_url}/auth/users/{pk}/head/meta", timeout=10).json().get("position")
    c = signin(s, st["visitor"])["client"]
    before = head()
    s.stop_auth()
    try:
        r = c.post("/v2/mint", json={"kind": "group", "draft": {"name": "while the head cannot be stored"}})
    finally:
        s.start_auth("auth-back")
    if r.status_code != 200:
        raise Fail(f"/v2/mint with the auth service away: {r.status_code} {r.text[:200]}")
    s.restart_door()
    now = head()
    if now is None or (before is not None and now <= before):
        raise Fail(f"the Door restarted and the stored head does not hold the write it could not store: {before} before, {now} after (NC-47)")
    verdict = ((me(signin(s, st["visitor"])["client"]).get("resume") or {}).get("chain") or {}).get("verdict")
    if verdict != "Whole":
        raise Fail(f"after the restart, the next sign-in finds the chain {verdict!r}, not Whole (NC-47)")
    return f"a write whose head could not be stored is in the stored head after the restart ({before} to {now}); the next sign-in finds it Whole"


def e14_signout_stores(s: Stack, st: dict) -> str:
    # D-34 (c): one process per person. A sign-out that ends the person's LAST session seals,
    # stores its head, and then answers; one of several answers at once, and the process's
    # tick (every 3 s) stores the head (NC-92). Both are held here, on a person of E14's own,
    # so which session is the last is known. As E11, each store is made to fail first: the
    # auth service away while the session writes, and back before the sign-out.
    who = signup(s, "E14's person")
    pk = key(who["pk"])
    head = lambda: httpx.get(f"{s.auth_url}/auth/users/{pk}/head/meta", timeout=10).json().get("position")

    def write_while_away(c: httpx.Client, what: str):
        before = head()
        s.stop_auth()
        try:
            r = c.post("/v2/mint", json={"kind": "group", "draft": {"name": what}})
        finally:
            s.start_auth("auth-back-e14")
        if r.status_code != 200:
            raise Fail(f"/v2/mint with the auth service away: {r.status_code} {r.text[:200]}")
        return before

    a, b = who["client"], signin(s, who)["client"]
    # One of two: answered at once, and stored by the tick.
    before = write_while_away(a, "before one of two sign-outs")
    r = a.post("/v2/signout")
    if r.status_code != 200:
        raise Fail(f"/v2/signout of one of two sessions: {r.status_code} {r.text[:200]}")
    t0, now = time.monotonic(), head()
    while (now is None or (before is not None and now <= before)) and time.monotonic() - t0 < 7:
        time.sleep(0.5)
        now = head()
    if now is None or (before is not None and now <= before):
        raise Fail(f"one of two sessions signed out, and in two ticks the stored head does not hold the write it could not store: {before} before, {now} after (NC-92)")
    tick = round(time.monotonic() - t0, 1)
    # The last: its sign-out stores the head before it answers.
    before2 = write_while_away(b, "before the last sign-out")
    r = b.post("/v2/signout")
    if r.status_code != 200:
        raise Fail(f"/v2/signout of the last session: {r.status_code} {r.text[:200]}")
    now2 = head()
    if now2 is None or (before2 is not None and now2 <= before2):
        raise Fail(f"the last session signed out, and the stored head does not hold the write it could not store: {before2} before, {now2} after (NC-92)")
    verdict = ((me(signin(s, who)["client"]).get("resume") or {}).get("chain") or {}).get("verdict")
    if verdict != "Whole":
        raise Fail(f"after the sign-outs, the next sign-in finds the chain {verdict!r}, not Whole")
    return (f"one of two sessions signed out: its write in the stored head {tick} s later ({before} to {now}); the last: "
            f"in it before the answer ({before2} to {now2}); the next sign-in finds it Whole")


def e12_relay_restart(s: Stack, st: dict) -> str:
    # The go-live runbook's step 6: an Arc restarted or rolled back while the show is open
    # assumes that a signed-in visitor rides through. One person, two sessions, one object
    # both hold: the first mints it, the second restores it at sign-in. The control first:
    # a write by the first reaches the second with nothing restarted. Then the relay
    # restarts under both, and the first, not signed in again, writes again.
    op = next(n for n, o in json.loads(ICD.read_text())["kinds"]["group"]["ops"].items() if o["op"] == 0)
    a = signin(s, st["visitor"])["client"]
    r = a.post("/v2/mint", json={"kind": "group", "draft": {"name": "held by two sessions"}})
    if r.status_code != 200:
        raise Fail(f"/v2/mint: {r.status_code} {r.text[:200]}")
    x = r.json()["object_id"]
    b = signin(s, st["visitor"])["client"]

    def seen(label: str | None) -> bool:
        for o in b.get("/v2/graph").json().get("objects", []):
            if o.get("id") == x:
                return label is None or label in json.dumps(o)
        return False

    t0 = time.monotonic()
    while not seen(None):
        if time.monotonic() - t0 > 60:
            raise Fail("the fixture: the second session does not hold the object the first minted before it signed in")
        time.sleep(3)

    def reaches(label: str, secs: float = 90) -> float | None:
        r = a.post("/v2/apply", json={"object": x, "op": op, "args": {"displayName": label, "shape": "team"}})
        if r.status_code != 200:
            raise Fail(f"the open session's write ({label}): {r.status_code} {r.text[:200]}")
        t1 = time.monotonic()
        while time.monotonic() - t1 < secs:
            if seen(label):
                return time.monotonic() - t1
            time.sleep(3)
        return None

    before = reaches("named before the relay restarted")
    if before is None:
        raise Fail("the control: with nothing restarted, one session's write to a shared object never reached the other session in 90 s")
    s.restart_relay()
    after = reaches("named after the relay restarted")
    if after is None:
        raise Fail(f"an open session's write after the relay restarted never reached the person's other session in 90 s "
                   f"(the same write with nothing restarted: {before:.0f} s)")
    return f"after the relay restarted, an open session's write reached the person's other session in {after:.0f} s (before it: {before:.0f} s)"


def e13_live_join(s: Stack, st: dict) -> str:
    # Live sync between sessions (Ralph, 27 Sep): an object one session mints appears in the
    # person's other open session, which is not signed in again (sync_once, step 7).
    a = signin(s, st["visitor"])["client"]
    b = signin(s, st["visitor"])["client"]
    ids = lambda c: {o.get("id") for o in c.get("/v2/graph").json().get("objects", [])}
    r = a.post("/v2/mint", json={"kind": "group", "draft": {"name": "minted while the other session is open"}})
    if r.status_code != 200:
        raise Fail(f"/v2/mint: {r.status_code} {r.text[:200]}")
    y = r.json()["object_id"]
    t0 = time.monotonic()
    while y not in ids(b):
        if time.monotonic() - t0 > 60:
            raise Fail("an object one session minted is not in the person's other open session after 60 s")
        time.sleep(1)
    took = time.monotonic() - t0
    bad = me(b).get("noncompliant")
    if bad:
        raise Fail(f"the other session holds it, and objects that do not fold: {bad}")
    return f"the other open session lists it in {took:.0f} s, not signed in again; nothing noncompliant"


def e15_site_signup_words(s: Stack, st: dict) -> str:
    # NC-53: a site's sign-up gives the words before any code, and holds its session while
    # they are written down (WORDS_LIFE); Continue mints the code then. WORDS s on the words,
    # nothing sent meanwhile, then Continue, /v2/token and /v2/me by the token: the session
    # lived throughout, whatever the idle sweep (IDLE s).
    registered(s, st)
    c = webapp(s)
    o = c.post("/v2/signup", json={"name": "a site's sign-up", **work(c, "signup")}).json()
    verifier, state = b64u(os.urandom(32)), os.urandom(8).hex()
    cb = {"client": CLIENT, "redirect_uri": CALLBACK, "state": state,
          "code_challenge": b64u(hashlib.sha256(verifier.encode()).digest())}
    r = c.post("/v2/signup/finish", json={"attempt": o["attempt"], "sealed": seal(o["key"], os.urandom(32), o["attempt"]),
                                          "client": cb})
    if r.status_code != 200:
        raise Fail(f"/v2/signup/finish for {CLIENT}: {r.status_code} {r.text[:200]}")
    body = r.json()
    if len((body.get("words") or "").split()) != 24:
        raise Fail("a site's sign-up gave no 24 words")
    if "redirect" in body:
        raise Fail("a site's sign-up gave its code before the words were kept (NC-53)")
    if not body.get("continue"):
        raise Absent("/v2/signup/continue: a site's sign-up held until its words are kept", "NC-53", "Software Engineering")
    time.sleep(WORDS)
    r = c.post("/v2/signup/continue", json={"continue": body["continue"]})
    if r.status_code != 200 or "redirect" not in r.json():
        raise Fail(f"Continue after {WORDS} s on the words: {r.status_code} {r.text[:200]}")
    u = urlsplit(r.json()["redirect"])
    q = parse_qs(u.query)
    if f"{u.scheme}://{u.netloc}{u.path}" != CALLBACK or q.get("state") != [state] or len(q.get("code", [])) != 1:
        raise Fail(f"Continue did not return to the registered callback with the state: {u.scheme}://{u.netloc}{u.path}")
    k, token_url, me_url = dpop_key(), f"{s.door_url}/v2/token", f"{s.door_url}/v2/me"
    r = httpx.post(token_url, json={"code": q["code"][0], "code_verifier": verifier, "client": CLIENT, "redirect_uri": CALLBACK},
                   headers={"dpop": dpop(k, "POST", token_url)}, verify=s.verify, timeout=30)
    if r.status_code != 200:
        raise Fail(f"/v2/token after {WORDS} s on the words: {r.status_code} {r.text[:200]}")
    token = r.json()["access_token"]
    r = httpx.get(me_url, headers={"authorization": f"DPoP {token}", "dpop": dpop(k, "GET", me_url, token)}, verify=s.verify, timeout=30)
    if r.status_code != 200 or key(r.json().get("pk", "")) != key(o["pk"]):
        raise Fail(f"the token, after {WORDS} s on the words: /v2/me {r.status_code} {r.text[:200]} (the session did not live "
                   f"throughout; idle {IDLE} s)")
    again = c.post("/v2/signup/continue", json={"continue": body["continue"]}).status_code
    if again == 200:
        raise Fail("Continue answered twice for one sign-up")
    return (f"the words first, no code; {WORDS} s on them, then Continue: the code at the callback, the token, /v2/me "
            f"as the new account (idle {IDLE} s); Continue again {again}")


def e9_compliance(s: Stack, st: dict) -> str:
    bad = {who: me(st[who]["client"]).get("noncompliant") for who in ("visitor", "home") if who in st}
    if any(v is None for v in bad.values()):
        raise Absent("noncompliant_objects() in /v2/me", "RX.3, O-16", "Software Engineering")
    if any(bad.values()):
        raise Fail(f"noncompliant objects: {bad}")
    return "none, on every session"


# (id, what it shows, the steps it needs, the check)
STEPS = [
    ("E0", "the Egregore Site is minted through the Door", [], e0_site),
    ("E0b", "three founders sign its creation, with the default roles", ["E0"], e0b_founders),
    ("E0c", "a role-gated op by a member without the role is refused", ["E0b"], e0c_role_gate),
    ("E0d", "a role granted without the granting role is refused", ["E0b"], e0d_grant_gate),
    ("E1", "the claim link after Touch opens the Door's /join", [], e1_claim),
    ("E2", "sign-up at the Door: a passkey's PRF, sealed; the identity made natively", [], e2_signup),
    ("E3", "the wrap is stored at the auth service", ["E2"], e3_wrap),
    ("E4", "the visitor is a member of the Site, with no owner acting", ["E0"], e4_join),
    ("E5", "the artefact Thing, on the payment route", ["E2"], e5_artefact),
    ("E6", "landing: signed in, with no further prompt", ["E2"], e6_landing),
    ("E6b", "landing with the Site and the Thing", ["E4", "E5"], e6b_holdings),
    ("E7", "at home: a fresh device signs in with the same passkey", ["E2", "E3"], e7_home),
    ("E7b", "at home: Egregore first", ["E7", "E4"], e7b_order),
    ("E9", "every session reports no noncompliant object", ["E6"], e9_compliance),
    ("E10", "a session left idle ends, its head stored, and the next one ends too", ["E2"], e10_idle),
    ("E11", "a Door restart stores a head its session could not store", ["E2"], e11_restart),
    ("E12", "a session open across a relay restart: its next write reaches the person's other session", ["E2"], e12_relay_restart),
    ("E13", "an object one session mints appears in the person's other open session", ["E2"], e13_live_join),
    ("E14", "a sign-out stores a head its session could not store: the last at once, one of several by the tick", [], e14_signout_stores),
    ("E8", "the egregore site reads and writes through /v2 with its token", [] if DEPLOYED and SITE_FILE else ["E0"], e8_site_api),
    ("E15", "a site's sign-up: minutes on the words, then Continue, and the token", [] if DEPLOYED and SITE_FILE else ["E0"],
     e15_site_signup_words),
]


def main() -> int:
    print(f"WallFlowers path, {LAYER}" + (f", the Door deployed at {DEPLOYED}" if DEPLOYED
          else ", the Door behind a TLS edge (Caddy, tls internal)" if EDGE else ""))
    stack = Stack()
    results: list[tuple[str, str, str]] = []
    try:
        stack.build()
        stack.up()
        state: dict = {}
        passed: set[str] = set()
        for sid, what, needs, check in STEPS:
            if ONLY and sid not in ONLY:
                results.append((sid, "NOT RUN", f"{what} (E2E_ONLY)"))
                continue
            unmet = [n for n in needs if n not in passed]
            if unmet:
                results.append((sid, "NOT REACHED", f"{what} (needs {', '.join(unmet)})"))
                continue
            try:
                detail = check(stack, state)
                results.append((sid, "PASS", f"{what}: {detail}"))
                passed.add(sid)
            except Absent as e:
                results.append((sid, "ABSENT", f"{what}. {e}"))
            except Fail as e:
                results.append((sid, "FAIL", f"{what}. {e}"))
            except httpx.HTTPError as e:
                results.append((sid, "FAIL", f"{what}. {type(e).__name__}: {e}"))
    except Fail as e:
        print(f"the stack did not come up: {e}")
        return 2
    finally:
        stack.down()
    for sid, state_, text in results:
        print(f"  {sid:<4} {state_:<11}  {text}")
    passed = sum(1 for _, s, _ in results if s == "PASS")
    first = next((t for _, s, t in results if s not in ("PASS", "NOT RUN")), None)
    ran = len(STEPS) - sum(1 for _, s, _ in results if s == "NOT RUN")
    print(f"e2e: {passed} of {ran} steps pass" + (f" (E2E_ONLY: {', '.join(ONLY)})" if ONLY else "")
          + (f"; first: {first}" if first else ""))
    return 0 if passed == ran else 1


if __name__ == "__main__":
    sys.exit(main())
