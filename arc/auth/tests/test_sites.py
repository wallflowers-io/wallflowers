"""Slug reservation.

A slug is a path on kenjin.cc, so it collides with every real page and every
proxied prefix. Checking is free and needs no account; claiming needs the key.
"""
from __future__ import annotations

from conftest import Device


def test_checking_is_free_and_says_why(client):
    assert client.get("/auth/site/check?slug=stoma").json() == {
        "slug": "stoma", "available": True, "reason": "ok"}
    assert client.get("/auth/site/check?slug=auth").json()["reason"] == "reserved"
    assert client.get("/auth/site/check?slug=a").json()["reason"] == "shape"
    assert client.get("/auth/site/check?slug=-nope").json()["reason"] == "shape"
    assert client.get("/auth/site/check?slug=").json()["reason"] == "empty"


def test_claiming_needs_the_key_and_is_idempotent(client, device):
    device.register()

    assert client.post(f"/auth/users/{device.pk}/sites",
                       json={"slug": "stoma"}).status_code == 401

    first = device.claim("stoma").json()
    assert first["claimed"] is True and first["reason"] == "claimed"
    # Claiming one you already hold changes nothing, so a retried request, a
    # double-tapped button and a resumed wizard all land in the same place.
    assert device.claim("stoma").json()["reason"] == "yours"
    assert [s["slug"] for s in device.account().json()["sites"]] == ["stoma"]


def test_someone_elses_slug_is_refused_without_saying_whose(client, device):
    device.register()
    device.claim("stoma")

    other = Device(client)
    other.register()
    r = other.claim("stoma")
    assert r.status_code == 409
    assert r.json()["error"] == "slug_taken"
    assert "identity" not in r.text and other.pk not in r.text and device.pk not in r.text


def test_check_and_claim_agree_on_what_is_taken(client, device):
    """The two halves must not diverge: a slug the check calls free and the
    claim refuses is a wizard that fails at its last step."""
    device.register()
    device.claim("stoma")
    assert client.get("/auth/site/check?slug=stoma").json() == {
        "slug": "stoma", "available": False, "reason": "taken"}


def test_a_reserved_slug_cannot_be_claimed_either(client, device):
    device.register()
    r = device.claim("signup")
    assert r.status_code == 400
    assert r.json() == {"error": "invalid_slug", "reason": "reserved"}


# ─────────────────────── holding an address during signup ────────────────────
#
# Finishing step 1 holds the address; the claim at the end consumes it; closing
# the page releases it; and it lapses on its own. The holder is the caller's IP,
# which the edge sends as CF-Connecting-IP — so two callers are two headers here.

HERE = {"CF-Connecting-IP": "203.0.113.7"}
THERE = {"CF-Connecting-IP": "198.51.100.9"}


def test_a_held_address_is_unavailable_to_anyone_else(client):
    assert client.post("/auth/site/hold", json={"slug": "cadiz"}, headers=HERE).status_code == 200
    assert client.get("/auth/site/check?slug=cadiz", headers=THERE).json() == {
        "slug": "cadiz", "available": False, "reason": "held"}
    # The one holding it still sees it as theirs to finish.
    assert client.get("/auth/site/check?slug=cadiz", headers=HERE).json()["available"] is True
    r = client.post("/auth/site/hold", json={"slug": "cadiz"}, headers=THERE)
    assert r.status_code == 409 and r.json()["reason"] == "held"


def test_the_claim_consumes_the_hold_and_nobody_else_can_claim_over_it(client, device):
    client.post("/auth/site/hold", json={"slug": "cadiz"}, headers=HERE)

    other = Device(client)
    other.register()
    r = client.post(f"/auth/users/{other.pk}/sites", json={"slug": "cadiz"},
                    headers={**other.headers(), **THERE})
    assert r.status_code == 409, "a hold by another caller blocks the claim"

    device.register()
    r = client.post(f"/auth/users/{device.pk}/sites", json={"slug": "cadiz"},
                    headers={**device.headers(), **HERE})
    assert r.status_code == 200 and r.json()["claimed"] is True
    assert client.get("/auth/site/check?slug=cadiz", headers=HERE).json()["reason"] == "taken"


def test_closing_the_page_releases_the_hold(client):
    client.post("/auth/site/hold", json={"slug": "cadiz"}, headers=HERE)
    # A beacon is text/plain; the body is still JSON.
    assert client.post("/auth/site/release", content='{"slug":"cadiz"}',
                       headers={**THERE, "Content-Type": "text/plain"}).json()["released"] is False, \
        "only the holder can release a hold"
    assert client.post("/auth/site/release", content='{"slug":"cadiz"}',
                       headers={**HERE, "Content-Type": "text/plain"}).json()["released"] is True
    assert client.get("/auth/site/check?slug=cadiz", headers=THERE).json()["available"] is True


def test_a_hold_lapses_on_its_own(client, monkeypatch):
    import time as _time
    client.post("/auth/site/hold", json={"slug": "cadiz"}, headers=HERE)
    later = _time.time() + 45 * 60 + 1
    monkeypatch.setattr(_time, "time", lambda: later)
    assert client.get("/auth/site/check?slug=cadiz", headers=THERE).json()["available"] is True
    assert client.post("/auth/site/hold", json={"slug": "cadiz"}, headers=THERE).status_code == 200


def test_one_caller_cannot_hold_the_namespace(client):
    for slug in ("alpha", "bravo", "charlie"):
        assert client.post("/auth/site/hold", json={"slug": slug}, headers=HERE).status_code == 200
    r = client.post("/auth/site/hold", json={"slug": "delta"}, headers=HERE)
    assert r.status_code == 409 and r.json()["reason"] == "too_many"
    # Renewing one already held is not a new hold.
    assert client.post("/auth/site/hold", json={"slug": "alpha"}, headers=HERE).status_code == 200


def test_a_taken_or_malformed_address_cannot_be_held(client, device):
    device.register()
    device.claim("stoma")
    assert client.post("/auth/site/hold", json={"slug": "stoma"}, headers=HERE).json()["reason"] == "taken"
    assert client.post("/auth/site/hold", json={"slug": "a b"}, headers=HERE).status_code == 400


# ───────────────────── which Host an address is served from ──────────────────
#
# The claim owns the name (ruled 22 Sep 2026): the owner binds their address to
# the Host that serves it, and the Arc serves from no other. The Arc's question
# is answered with an object id and never with a person.

HOST_A = "a1" * 32
HOST_B = "b2" * 32


def bind(device: Device, slug: str, host: str):
    return device._client.put(f"/auth/users/{device.pk}/sites/{slug}/host",
                              json={"host": host}, headers=device.headers())


def test_an_unclaimed_address_has_no_host(client):
    r = client.get("/auth/site/stoma/host")
    assert r.status_code == 404 and r.json() == {"error": "unclaimed", "slug": "stoma"}


def test_a_claimed_address_is_reserved_until_its_owner_binds_a_host(client, device):
    device.register()
    device.claim("stoma")
    assert client.get("/auth/site/stoma/host").json() == {"slug": "stoma", "host": None}

    r = bind(device, "stoma", HOST_A)
    assert r.status_code == 200 and r.json() == {"slug": "stoma", "host": HOST_A, "reason": "bound"}
    assert client.get("/auth/site/stoma/host").json() == {"slug": "stoma", "host": HOST_A}
    # Binding the same Host again is a no-op, so a retried publish lands in place.
    assert bind(device, "stoma", HOST_A).json()["reason"] == "unchanged"


def test_an_owner_may_move_their_address_to_another_of_their_hosts(client, device):
    device.register()
    device.claim("stoma")
    bind(device, "stoma", HOST_A)
    assert bind(device, "stoma", HOST_B).json()["reason"] == "bound"
    assert client.get("/auth/site/stoma/host").json()["host"] == HOST_B


def test_binding_needs_the_claimants_key(client, device):
    device.register()
    device.claim("stoma")

    # Unsigned.
    r = client.put(f"/auth/users/{device.pk}/sites/stoma/host", json={"host": HOST_A})
    assert r.status_code == 401

    # Someone else's account, signed properly, binding an address it does not hold.
    other = Device(client)
    other.register()
    r = bind(other, "stoma", HOST_B)
    assert r.status_code == 403 and r.json()["error"] == "slug_not_yours"
    assert device.pk not in r.text, "the refusal must not say whose it is"
    assert client.get("/auth/site/stoma/host").json()["host"] is None, "nothing moved"


def test_unclaimed_and_not_yours_are_one_answer(client, device):
    """The check route says whether an address is taken; this one must not add
    a second signal that tells a stranger more."""
    device.register()
    device.claim("stoma")
    other = Device(client)
    other.register()
    taken, free = bind(other, "stoma", HOST_A), bind(other, "orchard", HOST_A)
    assert (taken.status_code, taken.json()) == (free.status_code, free.json())


def test_a_host_must_be_an_object_id(client, device):
    device.register()
    device.claim("stoma")
    for bad in ["", "not-hex", "a1" * 31, "a1" * 33]:
        r = bind(device, "stoma", bad)
        assert r.status_code == 400, bad
    # Hex is hex: upper case is the same id, and it is stored the way the Arc
    # will compare it.
    assert bind(device, "stoma", "A1" * 32).json()["host"] == "a1" * 32


def test_the_host_lookup_never_names_the_owner(client, device):
    device.register()
    device.claim("stoma")
    bind(device, "stoma", HOST_A)
    assert device.pk not in client.get("/auth/site/stoma/host").text


def test_the_account_lists_the_host_each_address_is_bound_to(client, device):
    device.register()
    device.claim("stoma")
    bind(device, "stoma", HOST_A)
    [site] = device.account().json()["sites"]
    assert site["slug"] == "stoma" and site["host_id"] == HOST_A
