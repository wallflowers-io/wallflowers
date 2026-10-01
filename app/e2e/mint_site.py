"""The go-live runbook's step 3, rehearsed: a Site minted through a deployed Door, and the
files that register it (K-44). A stand-in until A-10 part 2 gives the Site its founders.

    E2E_DOOR=https://door.localhost E2E_DOOR_CA=/opt/door/edge-root.crt E2E_AUTH=http://127.0.0.1:18021 \\
        .venv/bin/python app/e2e/mint_site.py <site file> <clients file>

A founder signs up through the Door (the window simulated, as layer 1 plays it) and mints
the Site. The site file holds the Site's id and the founder's handle, key and simulated
passkey output, for E8 (`E2E_SITE_FILE`): it is a credential, written 0600, for a rehearsal
host only. The clients file is the Door's stand-in registration of egregores-echoes.com for
that Site, for `DOOR_CLIENTS_FILE=<file> deploy-door.sh clients`.
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import wallflowers_path as w  # noqa: E402


def main() -> int:
    if len(sys.argv) != 3 or not w.DEPLOYED:
        print(__doc__)
        return 64
    site_file, clients_file = Path(sys.argv[1]), Path(sys.argv[2])
    s = w.Stack()
    s.up()
    founder = w.signup(s, "Egregore founder (rehearsal)")
    r = founder["client"].post("/v2/mint", json={"kind": "group", "draft": {"name": "Egregore (rehearsal)"}})
    if r.status_code != 200:
        raise SystemExit(f"/v2/mint: {r.status_code} {r.text[:200]}")
    site = r.json()["object_id"]
    site_file.write_text(json.dumps({"site": site, "handle": founder["handle"], "prf": founder["prf"].hex(), "pk": founder["pk"]}))
    os.chmod(site_file, 0o600)
    clients_file.write_text(json.dumps({w.CLIENT: {"callbacks": [w.CALLBACK], "origins": ["http://localhost:3100"], "site": site}}, indent=2) + "\n")
    print(f"Site {site}; founder {w.key(founder['pk'])[:12]}; {site_file}; {clients_file}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
