"""Seals: a Door node's counter for a person's sealed state (D-34 (c)).

Written and read only against a signature, like every write here. The counter
moves compare-and-set: stored from the value the Door says it holds, a lost
reply's retry is a no-op, and anything else is refused with both counters.
"""
from __future__ import annotations

from conftest import Device, a_head
from app.main import SEAL_COUNTER_MAX

NODE = "door.wallflowers.io"
DEV_NODE = "127.0.0.1:8233"


def put_seal(device: Device, held, counter, node: str = NODE, signed: dict | None = None):
    headers = dict(device.headers() if signed is None else signed)
    if held is not None:
        headers["X-Seal-Held"] = str(held)
    if counter is not None:
        headers["X-Seal-Counter"] = str(counter)
    return device._client.put(f"/auth/users/{device.pk}/seals/{node}", headers=headers)


def get_seal(device: Device, node: str = NODE, signed: dict | None = None):
    return device._client.get(f"/auth/users/{device.pk}/seals/{node}",
                              headers=device.headers() if signed is None else signed)


def test_a_seal_is_stored_from_the_counter_held_and_read_signed(device):
    device.register()
    r = put_seal(device, 0, 1)
    assert r.status_code == 200
    assert r.json() == {"ok": True, "counter": 1, "stored": True}

    got = get_seal(device)
    assert got.status_code == 200
    assert got.json()["counter"] == 1
    assert isinstance(got.json()["updated_at"], int)

    assert put_seal(device, 1, 2).json() == {"ok": True, "counter": 2, "stored": True}
    assert get_seal(device).json()["counter"] == 2


def test_a_seal_from_a_counter_that_has_moved_is_refused_and_changes_nothing(device):
    device.register()
    put_seal(device, 0, 1)
    put_seal(device, 1, 2)

    r = put_seal(device, 1, 3)
    assert r.status_code == 409
    assert r.json() == {"error": "stale_seal", "reason": "moved", "held_counter": 2, "offered_counter": 3}
    assert get_seal(device).json()["counter"] == 2

    # With none stored, the counter held is 0.
    fresh = put_seal(device, 5, 6, node=DEV_NODE)
    assert fresh.status_code == 409
    assert fresh.json()["held_counter"] == 0
    assert get_seal(device, node=DEV_NODE).status_code == 404


def test_a_retry_whose_reply_was_lost_is_a_no_op(device):
    device.register()
    put_seal(device, 0, 1)
    first = get_seal(device).json()["updated_at"]

    again = put_seal(device, 0, 1)
    assert again.status_code == 200
    assert again.json() == {"ok": True, "counter": 1, "stored": False}
    assert get_seal(device).json() == {"counter": 1, "updated_at": first}


def test_a_seal_is_written_and_read_only_with_a_signature(client, device):
    device.register()
    unsigned = client.put(f"/auth/users/{device.pk}/seals/{NODE}",
                          headers={"X-Seal-Held": "0", "X-Seal-Counter": "1"})
    assert unsigned.status_code == 401
    assert unsigned.json()["error"] == "challenge_spent"

    other = Device(client)
    other.register()
    forged = put_seal(device, 0, 1, signed=other.sign_for(device.pk))
    assert forged.status_code == 401
    assert forged.json()["error"] == "bad_signature"

    assert client.get(f"/auth/users/{device.pk}/seals/{NODE}").status_code == 401
    assert get_seal(device, signed=other.sign_for(device.pk)).status_code == 401

    none = get_seal(device)
    assert none.status_code == 404
    assert none.json() == {"error": "no_seal"}


def test_a_counter_is_capped_and_a_u64_is_a_refusal_not_a_stack_trace(device):
    device.register()
    for held, counter in ((0, SEAL_COUNTER_MAX + 1), (0, 2**64 - 1), (SEAL_COUNTER_MAX + 1, SEAL_COUNTER_MAX + 2)):
        r = put_seal(device, held, counter)
        assert r.status_code == 400, (held, counter)
        assert r.json()["error"] == "invalid_request"
    assert put_seal(device, 0, SEAL_COUNTER_MAX).json()["counter"] == SEAL_COUNTER_MAX


def test_both_counters_are_required_whole_and_the_new_one_follows(device):
    device.register()
    for held, counter in ((None, 1), (0, None), ("x", 1), (0, "4.5"), (0, ""), ("-1", 1), (0, "-2"), (0, "+3"), (3, 3), (3, 2)):
        r = put_seal(device, held, counter)
        assert r.status_code == 400, (held, counter)
        assert r.json()["error"] == "invalid_request", (held, counter)
    assert get_seal(device).status_code == 404


def test_a_node_is_a_host_or_a_host_and_port(device):
    device.register()
    for bad in ("door_wallflowers.io", "a" * 254, "door%20wallflowers.io", "door%40wallflowers.io"):
        r = put_seal(device, 0, 1, node=bad)
        assert r.status_code == 400, bad
        assert r.json()["error"] == "invalid_request", bad
    assert put_seal(device, 0, 1, node=DEV_NODE).status_code == 200
    assert put_seal(device, 0, 1, node="a" * 253).status_code == 200


def test_each_node_keeps_its_own_counter(device):
    device.register()
    put_seal(device, 0, 7)
    put_seal(device, 0, 2, node=DEV_NODE)
    assert get_seal(device).json()["counter"] == 7
    assert get_seal(device, node=DEV_NODE).json()["counter"] == 2


def test_a_seal_cannot_be_stored_for_a_key_with_no_account(device):
    r = put_seal(device, 0, 1)
    assert r.status_code == 404
    assert r.json()["error"] == "no_wrap"


def test_the_head_is_untouched_by_seals(client, device):
    device.register()
    device.put_head(41, a_head(0x11))
    before = client.get(f"/auth/users/{device.pk}/head/meta").json()

    put_seal(device, 0, 1)
    put_seal(device, 0, 1)
    put_seal(device, 5, 9)
    put_seal(device, 1, 2, node=DEV_NODE)

    assert client.get(f"/auth/users/{device.pk}/head/meta").json() == before
    head = client.get(f"/auth/users/{device.pk}/head")
    assert head.content == a_head(0x11)
    assert head.headers["x-chain-position"] == "41"
