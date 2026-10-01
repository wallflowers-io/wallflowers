#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# arc-heartbeat.py — the Arc's liveness + capacity beat.
#
# Every ~2s it probes the local arc-gateway (/v1/health) and the GPU, then POSTs a
# heartbeat to the control plane, which shows the fleet: a fresh beat → the Arc is up; a
# missed beat → it is shown dark. Pure Python stdlib so it keeps beating even when a plane
# is degraded — that is exactly when the control plane most needs to know.
#
# Honesty rule (matching the Arc OS): it reports what it actually observes. If the gateway
# is not answering, arc_live is false — it never fabricates a healthy beat. `model_live` is
# reported as a constant FALSE: this Arc serves no model, and the control plane's ingest
# still requires the field. Missing REQUIRED config is a fail-loud startup error, not a
# silent no-op.
#
# Config comes from /etc/arc/arc.env (loaded by systemd EnvironmentFile). Required:
#   ARC_ID              this Arc's id (e.g. arc.ldn.01)
#   CONTROL_PLANE_URL   base URL of the control plane (heartbeats POST to /api/arcs/:id/heartbeat)
# Optional:
#   ARC_GATEWAY_PORT    local arc-gateway port (default 8080)
#   ARC_HEARTBEAT_TOKEN bearer token the control plane checks (recommended; read from the
#                       systemd-sealed credential when present, else this env var)
#   ARC_HEARTBEAT_INTERVAL  seconds between beats (default 2)

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request


def _read_secret(name: str, env_var: str) -> str:
    """Prefer a systemd-sealed credential ($CREDENTIALS_DIRECTORY/<name>, placed by the
    unit's LoadCredentialEncrypted=), falling back to the env var on a non-systemd host."""
    creds = os.environ.get("CREDENTIALS_DIRECTORY")
    if creds:
        try:
            with open(os.path.join(creds, name), encoding="utf-8") as f:
                return f.read().rstrip("\n")
        except FileNotFoundError:
            pass
    return os.environ.get(env_var, "")


ARC_ID = os.environ.get("ARC_ID")
CONTROL_PLANE = (os.environ.get("CONTROL_PLANE_URL") or "").rstrip("/")
GW_PORT = os.environ.get("ARC_GATEWAY_PORT", "8080")
TOKEN = _read_secret("arc_heartbeat_token", "ARC_HEARTBEAT_TOKEN")
INTERVAL = float(os.environ.get("ARC_HEARTBEAT_INTERVAL", "2"))
SRV = f"http://127.0.0.1:{GW_PORT}"

# Fail loud on missing required config — a heartbeat with no id/target is meaningless.
missing = [k for k, v in (("ARC_ID", ARC_ID), ("CONTROL_PLANE_URL", CONTROL_PLANE)) if not v]
if missing:
    sys.exit(f"arc-heartbeat: missing required config: {', '.join(missing)} (see /etc/arc/arc.env)")


def _get(url: str, timeout: float = 1.5) -> tuple[int, str]:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, ""
    except Exception:
        return 0, ""


def probe_gateway() -> dict:
    """Liveness from the Arc's own gateway (/v1/health). `model_live` is a constant false —
    this Arc runs no model plane — but the control plane's ingest still requires the field."""
    status, body = _get(f"{SRV}/v1/health")
    arc_live = status == 200
    if body:
        try:
            arc_live = json.loads(body).get("status") == "ok"
        except Exception:
            pass
    return {"arc_live": arc_live, "model_live": False}


def probe_gpu() -> dict:
    """utilization + memory from nvidia-smi; None on a CPU-only (dev) node."""
    try:
        out = subprocess.run(
            ["nvidia-smi", "--query-gpu=utilization.gpu,memory.used,memory.total",
             "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=2,
        )
        if out.returncode != 0 or not out.stdout.strip():
            return {"gpu": None}
        util, used, total = (x.strip() for x in out.stdout.strip().splitlines()[0].split(","))
        return {"gpu_util_pct": int(util), "gpu_mem_used_mb": int(used), "gpu_mem_total_mb": int(total)}
    except Exception:
        return {"gpu": None}


def beat() -> None:
    payload = {"arc_id": ARC_ID, "ts": int(time.time() * 1000), **probe_gateway(), **probe_gpu()}
    data = json.dumps(payload).encode()
    req = urllib.request.Request(
        f"{CONTROL_PLANE}/api/arcs/{ARC_ID}/heartbeat",
        data=data, method="POST",
        headers={"content-type": "application/json",
                 **({"authorization": f"Bearer {TOKEN}"} if TOKEN else {})},
    )
    try:
        with urllib.request.urlopen(req, timeout=2) as r:
            if r.status >= 300:
                print(f"arc-heartbeat: control plane returned {r.status}", file=sys.stderr)
    except Exception as e:
        # Transient control-plane unreachability must not kill the agent — but log it
        # loudly so a real outage is visible, never silently swallowed.
        print(f"arc-heartbeat: POST failed: {e}", file=sys.stderr)


if __name__ == "__main__":
    print(f"arc-heartbeat: {ARC_ID} → {CONTROL_PLANE} (gateway {SRV}, every {INTERVAL}s)", file=sys.stderr)
    while True:
        beat()
        time.sleep(INTERVAL)
