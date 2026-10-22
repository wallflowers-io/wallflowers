"""The account model: a key, a challenge, a wrap.

Weighted towards what would be silent if it broke — a challenge that could be
replayed, a signature that verified for the wrong account, a wrap served to the
wrong key. The happy path is one test because it is cheap to notice when it
breaks.
"""
from __future__ import annotations

from conftest import AUDIENCE, OCTET, Device, a_wrap


def test_register_then_fetch_the_wrap_with_nothing_at_all(client, device):
    assert device.register().json() == {"ok": True, "created": True}

    # THE PROPERTY THE WHOLE DESIGN RESTS ON: this read carries no credential,
    # because it is how a new device gets the key it would authenticate with.
    r = client.get(f"/auth/users/{device.pk}")
    assert r.status_code == 200
    assert r.content == a_wrap()
    assert r.headers["cache-control"] == "no-store"


def test_a_challenge_is_single_use(client, device):
    headers = device.headers()
    assert client.put(f"/auth/users/{device.pk}", content=a_wrap(),
                      headers={**headers, **OCTET}).status_code == 200
    # The same nonce and the same signature, replayed verbatim.
    again = client.put(f"/auth/users/{device.pk}", content=a_wrap(),
                       headers={**headers, **OCTET})
    assert again.status_code == 401
    assert again.json()["error"] == "challenge_spent"


def test_a_signature_cannot_be_lifted_onto_another_account(client, device):
    """The forgery this exists to stop: signing a challenge that names someone
    else's key. The signature is valid — over the wrong payload."""
    victim = Device(client)
    victim.register()

    r = client.put(f"/auth/users/{victim.pk}", content=a_wrap(9),
                   headers={**device.sign_for(victim.pk), **OCTET})
    assert r.status_code == 401
    assert r.json()["error"] == "bad_signature"
    # And the victim's wrap is untouched.
    assert client.get(f"/auth/users/{victim.pk}").content == a_wrap()


def test_a_signature_for_another_arc_is_refused(client, device):
    """The audience is inside the signed bytes, so a signature captured by one
    Arc is worth nothing at another."""
    r = client.put(f"/auth/users/{device.pk}", content=a_wrap(),
                   headers={**device.headers(audience="arc.attacker.example"), **OCTET})
    assert r.status_code == 401
    assert r.json()["error"] == "bad_signature"


def test_an_unknown_nonce_is_refused_before_the_signature_is_looked_at(client, device):
    r = client.put(f"/auth/users/{device.pk}", content=a_wrap(),
                   headers={"X-Pacific-Challenge": "never-issued",
                            "X-Pacific-Signature": "00" * 64, **OCTET})
    assert r.status_code == 401
    assert r.json()["error"] == "challenge_spent"


def test_the_address_grammar_is_refused_rather_than_missed(client):
    """A malformed key could never name an account we hold, so answering 'not
    found' would be a guess dressed as an answer."""
    for bad in ("nothex", "ab", "AB" * 32, "0" * 63):
        r = client.get(f"/auth/users/{bad}")
        assert r.status_code == 400, bad
        assert r.json()["error"] == "invalid_request"


def test_an_unregistered_key_has_no_wrap(client, device):
    r = client.get(f"/auth/users/{device.pk}")
    assert r.status_code == 404
    assert r.json()["error"] == "no_wrap"


def test_bytes_that_are_not_a_wrap_are_refused_now_not_at_the_restore(client, device):
    for body in (b"", b"PHS1" + bytes(72), b"PWR1", b"nope" + bytes(72)):
        r = client.put(f"/auth/users/{device.pk}", content=body,
                       headers={**device.headers(), **OCTET})
        assert r.status_code == 400, body[:4]
        assert r.json()["error"] in ("not_a_wrap", "invalid_request")


def test_re_wrapping_replaces_and_that_is_the_revocation(client, device):
    """ONE WRAP PER ACCOUNT. Enrolling a new passkey overwrites the old wrap,
    so the previous credential stops opening the account — without this service
    being trusted to honour anything."""
    device.register()
    assert device.register(a_wrap(7)).json() == {"ok": True, "created": False}
    assert client.get(f"/auth/users/{device.pk}").content == a_wrap(7)


def test_the_account_view_needs_a_signature_and_reports_what_is_held(client, device):
    device.register()
    assert client.get(f"/auth/users/{device.pk}/account").status_code == 401

    body = device.account().json()
    assert body["identity_key"] == f"ed25519:{device.pk}"
    assert body["wrap"]["size"] == len(a_wrap())
    assert body["history"] is None
    assert body["sites"] == []


def test_health_says_which_audience_devices_must_sign_for(client):
    body = client.get("/auth/health").json()
    assert body["ok"] is True
    assert body["audience"] == AUDIENCE
