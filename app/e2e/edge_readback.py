"""DT-29: the edge rewrites nothing the Door's security rests on (SEC-7, SEC-8, SEC-27; PC-1).

    .venv/bin/python app/e2e/edge_readback.py                    # a loopback stack behind Caddy
    .venv/bin/python app/e2e/edge_readback.py --door URL --edge URL [--ca FILE] [--origin URL]

The same requests go to the Door itself and through the edge, with the same Host, and
Set-Cookie, Access-Control-*, Content-Security-Policy, X-Frame-Options, Cache-Control,
Referrer-Policy and Strict-Transport-Security must read back as the Door sent them, and at
an https edge Strict-Transport-Security must be there at all (a first visit downgraded). A cookie's value differs by nature
between two sign-ups; its attributes may not. With --door and --edge it runs against a
deployed host from that host (the Door's own port is reachable only there); a sign-up
through each makes a real account, so a deployed run needs Ralph's word first.
"""
from __future__ import annotations

import argparse
import os
import ssl
import sys
from pathlib import Path
from urllib.parse import urlsplit

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent))
import wallflowers_path as w  # noqa: E402

WATCHED = ("set-cookie", "content-security-policy", "x-frame-options", "cache-control", "referrer-policy",
           "strict-transport-security")


def watched(r: httpx.Response) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for k, v in r.headers.multi_items():
        k = k.lower()
        if k in WATCHED or k.startswith("access-control-"):
            if k == "set-cookie":
                name, _, rest = v.partition("=")
                v = name + "=<value>;" + rest.partition(";")[2]
            out.setdefault(k, []).append(v)
    return {k: sorted(v) for k, v in out.items()}


def sign_up(base: str, host: str, verify, headers: dict) -> httpx.Response:
    """A sign-up to its finish, the window simulated as layer 1 does: the finish sets the cookie."""
    c = httpx.Client(base_url=base, timeout=60, verify=verify, headers=headers)
    o = c.post("/v2/signup", json={"name": "readback", **w.work(c, "signup")}).json()
    return c.post("/v2/signup/finish", json={"attempt": o["attempt"], "sealed": w.seal(o["key"], os.urandom(32), o["attempt"])})


def compare(door: str, edge: str, verify, origin: str) -> list[str]:
    host = urlsplit(edge).netloc
    same = {"host": host}
    asks = [
        ("GET /signin", "GET", "/signin", {}),
        ("GET /v2/icd", "GET", "/v2/icd", {}),
        ("GET /v2/me, signed out", "GET", "/v2/me", {}),
        ("preflight from the listed origin", "OPTIONS", "/v2/me",
         {"origin": origin, "access-control-request-method": "POST", "access-control-request-headers": "content-type"}),
        ("preflight from an unlisted origin", "OPTIONS", "/v2/me",
         {"origin": "https://unlisted.example", "access-control-request-method": "POST"}),
    ]
    diffs, seen = [], 0
    for label, method, path, extra in asks:
        a = httpx.request(method, door + path, headers={**same, **extra}, timeout=30)
        b = httpx.request(method, edge + path, headers=extra, timeout=30, verify=verify)
        ha, hb = watched(a), watched(b)
        seen += len(ha)
        if (a.status_code, ha) != (b.status_code, hb):
            diffs.append(f"{label}: the Door {a.status_code} {ha}; the edge {b.status_code} {hb}")
        if edge.startswith("https:") and "strict-transport-security" not in hb:
            diffs.append(f"{label}: no Strict-Transport-Security at the https edge")
    a, b = sign_up(door, host, True, same), sign_up(edge, host, verify, {})
    ha, hb = watched(a), watched(b)
    if "set-cookie" not in ha:
        diffs.append(f"sign-up's finish: the Door set no cookie ({a.status_code})")
    elif (a.status_code, ha) != (b.status_code, hb):
        diffs.append(f"sign-up's finish: the Door {a.status_code} {ha}; the edge {b.status_code} {hb}")
    return diffs if diffs or seen else ["no watched header came back from the Door: nothing was compared"]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--door"), ap.add_argument("--edge"), ap.add_argument("--ca"), ap.add_argument("--origin")
    a = ap.parse_args()
    if a.door and a.edge:
        verify = ssl.create_default_context(cafile=a.ca) if a.ca else True
        diffs = compare(a.door.rstrip("/"), a.edge.rstrip("/"), verify, a.origin or a.edge.rstrip("/"))
        where = f"{a.door} and {a.edge}"
    else:
        os.environ["E2E_EDGE"] = "caddy"
        w.EDGE = "caddy"
        s = w.Stack()
        try:
            s.build()
            s.up()
            diffs = compare(f"http://127.0.0.1:{s.door}", s.door_url, s.verify, s.door_url)
        except w.Fail as e:
            print(f"edge read-back: the stack did not come up: {e}")
            return 2
        finally:
            s.down()
        where = "a loopback stack behind Caddy (tls internal)"
    print(f"DT-29 at {where}:")
    for d in diffs:
        print(f"  DIFFERS  {d}")
    print("edge read-back: " + ("the edge rewrote nothing watched" if not diffs else f"{len(diffs)} differ"))
    return 1 if diffs else 0


if __name__ == "__main__":
    sys.exit(main())
