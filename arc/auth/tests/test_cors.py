"""The Arc has to be reachable from a page on another origin.

WHY THIS IS A TEST AND NOT A CONFIG DETAIL. While the pages and this service
shared an origin, none of it applied and none of it was exercised. The moment the
Arc gets its own hostname -- `arc.wallflowers.io` while the pages are at
`wallflowers.io` -- every call becomes cross-origin, and two things fail in ways
that do not look like themselves:

  * The writes carry `X-Pacific-Challenge`, `X-Pacific-Signature` and
    `X-Chain-Position`, none of them CORS-simple, so the browser sends an OPTIONS
    preflight FIRST. An unrouted preflight surfaces as a network error with no
    status code, which reads as "the Arc is down" rather than "the Arc did not
    say I was allowed to ask".

  * Cross-origin, JavaScript can read only the CORS-safelisted response headers.
    `X-Chain-Position` is how `openHead` holds the Arc to the position it served
    -- the check that catches an Arc serving the header of one head and the
    ciphertext of another. Unexposed, `headers.get` returns null and the check
    stops happening SILENTLY, which is worse than it failing.

The second is the one worth a test of its own: nothing else in the system would
go red.
"""
from __future__ import annotations

from conftest import OCTET, a_head, a_wrap


ELSEWHERE = "https://wallflowers.io"


def test_a_preflight_for_a_signed_write_is_answered(client):
    """The exact preflight a browser sends before `PUT …/head`."""
    r = client.options(
        "/auth/users/" + "ab" * 32 + "/head",
        headers={
            "Origin": ELSEWHERE,
            "Access-Control-Request-Method": "PUT",
            "Access-Control-Request-Headers":
                "x-pacific-challenge,x-pacific-signature,x-chain-position,content-type",
        },
    )
    assert r.status_code == 200, (
        f"the preflight was refused ({r.status_code}) — a browser will never send "
        f"the write, and it will look like the Arc is unreachable"
    )
    allowed = r.headers.get("access-control-allow-headers", "").lower()
    for h in ("x-pacific-challenge", "x-pacific-signature", "x-chain-position"):
        assert h in allowed, f"{h} is not allowed through the preflight"
    assert "PUT" in r.headers.get("access-control-allow-methods", "")


def test_the_position_header_is_readable_cross_origin(client, device):
    """`X-Chain-Position` is not CORS-safelisted, so without `expose_headers` the
    client cannot read the number it is supposed to hold the Arc to."""
    client.put(f"/auth/users/{device.pk}", content=a_wrap(), headers={**OCTET, **device.headers()})
    client.put(
        f"/auth/users/{device.pk}/head",
        content=a_head(),
        headers={**OCTET, **device.headers(), "X-Chain-Position": "0"},
    )

    r = client.get(f"/auth/users/{device.pk}/head", headers={"Origin": ELSEWHERE})
    assert r.status_code == 200
    exposed = r.headers.get("access-control-expose-headers", "").lower()
    assert "x-chain-position" in exposed, (
        "X-Chain-Position is not exposed, so a cross-origin client reads null and "
        "stops checking the declared position against the sealed one — silently"
    )
    assert "x-head-updated-at" in exposed


def test_the_ungated_reads_answer_any_origin(client, device):
    """A wrap is `serve to whoever asks` and an anchor nobody can watch is an
    anchor an arc can move. Both stay true across an origin boundary."""
    client.put(f"/auth/users/{device.pk}", content=a_wrap(), headers={**OCTET, **device.headers()})
    r = client.get(f"/auth/users/{device.pk}", headers={"Origin": "https://someone.example"})
    assert r.status_code == 200
    assert r.headers.get("access-control-allow-origin") == "*"


def test_cors_grants_no_authority(client):
    """The point stated as an assertion: allowing an origin is not allowing a
    write. There are no cookies here, so a page that reaches these routes can do
    exactly what curl can — which, unsigned, is nothing."""
    r = client.put(
        "/auth/users/" + "cd" * 32 + "/head",
        content=a_head(),
        headers={**OCTET, "Origin": ELSEWHERE, "X-Chain-Position": "0"},
    )
    assert r.status_code in (401, 403), f"an unsigned cross-origin write got {r.status_code}"
