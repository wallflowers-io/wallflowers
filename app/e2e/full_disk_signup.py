"""NC-132: on a full disk, a sign-up is refused before any account exists (BW-A's fix, fdee52ca).

door-test only, on Software Configuration Management's word, and last in a window. One file in the VM's /var/tmp,
fallocate'd to everything the disk has left for non-root writers (the Door, PostgreSQL); one sign-up at layer 1;
the file deleted at once, whatever happened. Then the units are read.

  F1  /v2/signup/finish answers 5xx, and the auth service stores no wrap for it
  F2  the file gone, the disk back; door, door-edge, arc-node, arc-relay and postgresql active; a sign-up then
      answers 200 (the Door recovered)

  E2E_DOOR=https://door.localhost E2E_DOOR_IP=192.168.64.2 E2E_DOOR_CA=<edge root> E2E_AUTH=http://127.0.0.1:18021
  .venv/bin/python app/e2e/full_disk_signup.py
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import browser_path as bp  # noqa: E402
import wallflowers_path as w  # noqa: E402

FILL = "/var/tmp/nc132-fill"
UNITS = ("door", "door-edge", "arc-node", "arc-relay", "postgresql@16-main")


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def vm(cmd: str) -> str:
    return subprocess.run(["limactl", "shell", "door-test", "sudo", "sh", "-c", cmd], capture_output=True, text=True, timeout=60).stdout.strip()


def df() -> str:
    return vm("df -h / | tail -1")


def attempt(s: w.Stack) -> tuple[int, str]:
    """A sign-up at layer 1, as E2's: /v2/signup, then /v2/signup/finish with a sealed PRF; its status and body."""
    c = w.webapp(s)
    o = c.post("/v2/signup", json={"name": "full disk", **w.work(c, "signup")})
    if o.status_code != 200:
        return o.status_code, "/v2/signup: " + o.text[:200]
    o = o.json()
    r = c.post("/v2/signup/finish", json={"attempt": o["attempt"], "sealed": w.seal(o["key"], os.urandom(32), o["attempt"])})
    return r.status_code, r.text[:200]


def main() -> int:
    if not (w.DEPLOYED and bp.DOOR_IP):
        print(__doc__)
        return 2
    bp.map_name()
    s = w.Stack(host="localhost")
    s.up()
    results = []
    before = df()
    say("disk before:", before)
    t0 = datetime.now(timezone.utc)
    try:
        vm(f"fallocate -l $(df --output=avail -B1 / | tail -1) {FILL}")
        full = df()
        say("filled:", full)
        status, body = attempt(s)
    finally:
        vm(f"rm -f {FILL}")
        say("file removed:", vm(f"ls {FILL} 2>&1 | tail -1"))
    since = t0.strftime("%Y-%m-%d %H:%M:%S UTC")
    wraps = vm(f"journalctl --since '{since}' --no-pager -o cat | grep -c 'wrap stored'")
    results.append(("F1", 500 <= status < 600 and wraps == "0", "on a full disk, sign-up refused with a 5xx before any account exists",
                    {"disk_full": full, "signup_finish": status, "body": body, "wraps_stored": wraps}))
    time.sleep(5)
    units = {u: vm(f"systemctl is-active {u}") for u in UNITS}
    after = df()
    status2, body2 = attempt(s)
    results.append(("F2", all(v == "active" for v in units.values()) and status2 == 200 and FILL not in vm(f"ls {FILL} 2>/dev/null"),
                    "the file gone: the units active, and a sign-up answers 200", {"disk_after": after, "units": units, "signup_finish": status2}))
    s.down()
    for sid, ok, what, detail in results:
        print(f"  {sid:<3} {'G' if ok else 'NG':<3}  {what}: {json.dumps(detail)}")
    bad = [sid for sid, ok, _, _ in results if not ok]
    print("full disk sign-up: " + ("G" if not bad else f"NG at {', '.join(bad)}") + f"; disk before {before!r}")
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
