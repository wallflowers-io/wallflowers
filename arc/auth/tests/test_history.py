"""The history: the half of an account that is not public.

The wrap is served to anyone; this is not, and the difference is the whole
reason the service no longer needs a second factor. These tests pin that
difference, and the two-slot discipline that makes a bad upload survivable.
"""
from __future__ import annotations

from conftest import OCTET, Device, a_history, a_wrap


def test_history_round_trips_only_for_the_account_that_stored_it(client, device):
    device.register()
    body = a_history(128)
    assert device.put_history(body).json()["size"] == len(body)

    got = device.get_history()
    assert got.status_code == 200
    assert got.content == body
    assert got.headers["cache-control"] == "no-store"

    # UNLIKE THE WRAP: no signature, nothing. This read is not the bootstrap.
    assert client.get(f"/auth/users/{device.pk}/history").status_code == 401

    # And another account's signature does not open it either.
    other = Device(client)
    other.register()
    assert client.get(f"/auth/users/{device.pk}/history",
                      headers=other.sign_for(device.pk)).status_code == 401


def test_a_second_upload_demotes_the_first_rather_than_replacing_it(client, device):
    device.register()
    device.put_history(a_history(16))
    device.put_history(a_history(32))

    assert device.get_history("current").content == a_history(32)
    assert device.get_history("previous").content == a_history(16)

    meta = client.get(f"/auth/users/{device.pk}/history/meta",
                      headers=device.headers()).json()
    assert meta["exists"] is True
    assert meta["size"] == len(a_history(32))
    assert meta["previous"]["size"] == len(a_history(16))


def test_the_exported_at_header_survives_the_round_trip(client, device):
    device.register()
    device.put_history(a_history(), **{"X-History-Exported-At": "1757500000000"})
    assert device.get_history().headers["x-history-exported-at"] == "1757500000000"


def test_a_stale_export_cannot_overwrite_a_newer_one(client, device):
    """An account has devices and devices go offline. A phone that wakes with a
    week-old view and exports must not overwrite the laptop's current history —
    two slots survive one such write and not two in a row, so the guard is at
    the door rather than in the slots."""
    device.register()
    device.put_history(a_history(32), **{"X-History-Exported-At": "2000"})

    r = device.put_history(a_history(16), **{"X-History-Exported-At": "1000"})
    assert r.status_code == 409
    assert r.json()["error"] == "stale_history"
    assert r.json()["held_exported_at"] == 2000
    assert r.json()["offered_exported_at"] == 1000

    # And the newer one is untouched — refusing must not have cost the account
    # the history it already had.
    meta = client.get(f"/auth/users/{device.pk}/history/meta",
                      headers=device.headers()).json()
    assert meta["size"] == len(a_history(32))
    assert meta["exported_at"] == 2000

    # The same stamp again is NOT stale: a device re-exporting at the same
    # instant, or retrying a request whose reply was lost, is not going
    # backwards and must not be refused.
    assert device.put_history(a_history(8),
                              **{"X-History-Exported-At": "2000"}).status_code == 200
    # Nor is an export carrying no stamp at all, which is how a client that
    # predates the header behaves.
    assert device.put_history(a_history(8)).status_code == 200


def test_a_wrap_in_the_history_slot_is_refused(client, device):
    """Both artefacts are opaque ciphertext, so the magic is the only thing
    that can tell a client it has sent the wrong file — and the only moment it
    can be told is now, rather than at the restore that needed it."""
    device.register()
    r = device.put_history(a_wrap())
    assert r.status_code == 400
    assert r.json()["error"] == "not_a_history"


def test_a_history_cannot_be_stored_for_a_key_with_no_account(client, device):
    r = device.put_history()
    assert r.status_code == 404
    assert r.json()["error"] == "no_wrap"


def test_a_history_larger_than_the_cap_is_refused_before_it_is_read(client, device):
    device.register()
    cap = client.app.state.cfg.history_max_bytes
    r = client.put(f"/auth/users/{device.pk}/history",
                   content=b"PHS1" + bytes(cap),
                   headers={**device.headers(), **OCTET})
    assert r.status_code == 413
    assert r.json()["error"] == "too_large"


def test_an_account_with_no_history_says_so(client, device):
    device.register()
    r = device.get_history()
    assert r.status_code == 404
    assert r.json()["error"] == "no_history"
