"""Which Arc an account is hosted on.

THE DIVISION OF LABOUR, because getting this wrong is the whole risk.

An Arc's `POST /signup` takes the user's CONTACT BUNDLE and calls pair_scan to
form a 2-member MLS group {user, Arc}. Only a device holding the Ed25519
identity can mint that bundle. A browser holding a passkey cannot, and must not
pretend to: `SOVEREIGN-GRAPH.txt` states the edge plainly -- "a person IS their
Ed25519 public key ... Bind, never source." The moment a web sign-in can create
a Pacific identity, Pacific has accounts and the restore design loses the
property it is built on.

So this service NEVER establishes a tether. It records WHICH ARC an account is
hosted on, verifies that Arc is real, and hands the address to the device that
can act on it. Kenjin runs an Arc; it does not run Pacific.

WHY THE ARC IS RECORDED PER ACCOUNT AND NOT AS ONE GLOBAL SETTING: anyone can
run an Arc -- the protocol is open and arc-node is a binary. The same sign-in
flow is meant to register accounts against whichever Arc a person chose, so the
address belongs to the account, not to the deployment.
"""
from __future__ import annotations

import logging
import time
from dataclasses import dataclass

import httpx

log = logging.getLogger("kenjin.auth.arc")

_DISCOVERY_TTL = 300.0
_cache: dict[str, tuple[float, "ArcIdentity"]] = {}


@dataclass(frozen=True)
class ArcIdentity:
    url: str
    name: str
    identity_key: str
    fingerprint: str
    relay_url: str


async def discover(http: httpx.AsyncClient, arc_url: str) -> ArcIdentity | None:
    """Ask an Arc who it is. Never raises: an Arc that is down must not stop
    somebody signing in, it only means we cannot verify it right now."""
    arc_url = arc_url.rstrip("/")
    if not arc_url:
        return None

    hit = _cache.get(arc_url)
    if hit and time.monotonic() - hit[0] < _DISCOVERY_TTL:
        return hit[1]

    try:
        # /v1/arc, not /arc: the gateway is the only public port, and it is what
        # rewrites relay_url to the reachable address. The node behind it
        # reports its own loopback, which is useless to anyone else.
        r = await http.get(f"{arc_url}/v1/arc", timeout=10.0)
        r.raise_for_status()
        d = r.json()
        got = ArcIdentity(
            url=arc_url,
            name=d.get("name", ""),
            identity_key=d.get("identity_key", ""),
            fingerprint=d.get("fingerprint", ""),
            relay_url=(d.get("relay_url")
                       or (d.get("planes", {}).get("relay", {}) or {}).get("url", "")),
        )
    except Exception as e:                                  # noqa: BLE001
        log.warning("arc discovery failed for %s: %s", arc_url, e)
        return hit[1] if hit else None                      # stale beats nothing

    if not got.identity_key:
        log.warning("arc at %s advertises no identity_key", arc_url)
        return None

    _cache[arc_url] = (time.monotonic(), got)
    return got
