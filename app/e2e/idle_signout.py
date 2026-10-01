"""E14's idle case (run 84; Software Security, 29 Sep): a sign-out after an idle pause stores its state.

The Door's clients pool their connections to the auth service longer than the auth keeps them open, so
the first call after a pause can go down a dead socket. A seal is retried each tick; a sign-out's store is
not ("the state was not kept at its end"). On a deployed Door, in its VM (the journals are read):

  a person signs up and mints (IDLE_MINTS, 11 by default); the heads are stored; IDLE_SECS idle; the sign-out, 200. G only if
  the person's process logs no "not kept at its end", the auth logs a store for the person after the
  sign-out, and the next sign-in is Whole and holds what was minted.

  E2E_DOOR=https://door.localhost E2E_DOOR_CA=/opt/door/edge-root.crt E2E_AUTH=http://127.0.0.1:18021
  [IDLE_SECS=40] [AUTH_UNIT=door-rehearsal-auth.service] ~/wf/.venv/bin/python app/e2e/idle_signout.py
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
import wallflowers_path as w  # noqa: E402

IDLE = int(os.environ.get("IDLE_SECS", "40"))
# 11 in a row, as run 84 made them: the late-joiner restate follows, its seal's tick 30 s later finds the
# auth unreached and is stored on the retry, and the sign-out 6 s after that is not kept (run 89, 2 of 2).
# One mint, 40 s idle: the sign-out's store lands (run 89).
MINTS = int(os.environ.get("IDLE_MINTS", "11"))
AUTH_UNIT = os.environ.get("AUTH_UNIT", "door-rehearsal-auth.service")


def say(*a):
    print(time.strftime("%H:%M:%SZ", time.gmtime()), *a, flush=True)


def sh(cmd: str) -> str:
    return subprocess.run(cmd, shell=True, capture_output=True, text=True).stdout


def kids() -> set[str]:
    return set(sh("sudo sh -c 'pgrep -P $(systemctl show -p MainPID --value door.service)'").split())


def since(t: datetime) -> str:
    return t.astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")


def main() -> int:
    if not w.DEPLOYED:
        print(__doc__)
        return 2
    s = w.Stack()
    s.up()
    before = kids()
    t_up = datetime.now(timezone.utc)
    who = w.signup(s, f"idle sign-out {time.strftime('%H%M%S', time.gmtime())}")
    c = who["client"]
    made = []
    for i in range(MINTS):
        r = c.post("/v2/mint", json={"kind": "group", "draft": {"name": f"before the pause {i}"}})
        if r.status_code != 200:
            raise w.Fail(f"/v2/mint: {r.status_code} {r.text[:200]}")
        made.append(r.json()["object_id"])
    new = kids() - before
    if len(new) != 1:
        raise w.Fail(f"cannot tell this person's process: {sorted(new)}")
    pid = new.pop()
    key = w.key(who["pk"])[:16]
    say(f"signed up, minted {MINTS}, process {pid}, auth key {key}…; idle {IDLE} s")
    time.sleep(IDLE)
    t0 = datetime.now(timezone.utc)
    r = c.post("/v2/signout")
    time.sleep(4)
    door = sh(f"sudo journalctl -u door.service _PID={pid} --since '{since(t0)}' --no-pager -o cat")
    auth = sh(f"sudo journalctl --since '{since(t0)}' --no-pager -o cat | grep 'kenjin.auth' | grep '{key}'")
    not_kept = [l for l in door.splitlines() if "not kept at its end" in l]
    unreached = [l for l in door.splitlines() if "could not be reached" in l]
    stored = [l for l in auth.splitlines() if "stored=True" in l]
    f = w.signin(s, who)["client"]
    me = w.me(f)
    ids = {o.get("id") for o in f.get("/v2/graph").json().get("objects", [])}
    held = all(m in ids for m in made)
    verdict = ((me.get("resume") or {}).get("chain") or {}).get("verdict")
    f.post("/v2/signout")
    s.down()
    ok = r.status_code == 200 and not not_kept and bool(stored) and verdict == "Whole" and held
    say("idle sign-out:", "G" if ok else "R", json.dumps({
        "mints": MINTS, "signout": r.status_code, "idle_s": IDLE, "door: not kept at its end": len(not_kept),
        "door: auth could not be reached": len(unreached), "auth: stores for the person after the sign-out": len(stored),
        "next sign-in": verdict, "every mint held": held,
        "auth refusals for the person": len([l for l in sh(f"sudo journalctl --since '{since(t_up)}' --no-pager -o cat | grep 'kenjin.auth' | grep '{key}' | grep refused").splitlines()])}))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
