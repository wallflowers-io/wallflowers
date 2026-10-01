"""The rules, against the real Security.framework.

Every test that writes uses SCRATCH_SERVICE. Nothing here touches
``network.pacific.atrest`` with anything but a read — a test that minted into
the real vault could overwrite the device key of the machine running it, which
is the exact failure the package is built to prevent.
"""
from __future__ import annotations

import os
import sys

import pytest

import wallflowers
from wallflowers import _darwin, atrest, errors
from wallflowers.keychain import ATREST_SERVICE, DEVICE_KEY_ACCOUNT, Keychain

SCRATCH_SERVICE = "io.wallflowers.test.scratch"

darwin_only = pytest.mark.skipif(sys.platform != "darwin", reason="needs a keychain")


@pytest.fixture
def scratch():
    kc = Keychain(SCRATCH_SERVICE)
    yield kc
    for account in ("probe", "cred", "mint"):
        try:
            kc.delete(account)
        except errors.KeychainError:
            pass


def test_version_is_the_claimed_one():
    assert wallflowers.__version__ == "0.0.1"


def test_service_names_are_the_interop_contract():
    # If these change, the app and this package stop sharing a vault.
    assert ATREST_SERVICE == "network.pacific.atrest"
    assert DEVICE_KEY_ACCOUNT == "device-master-key"
    assert wallflowers.CREDENTIALS_SERVICE == "network.pacific.credentials"


@darwin_only
def test_absent_item_is_none_not_an_error(scratch):
    assert scratch.find("probe") is None


@darwin_only
def test_absent_item_raises_on_get(scratch):
    with pytest.raises(errors.NotFound):
        scratch.get("probe")


@darwin_only
def test_round_trip(scratch):
    scratch.put("probe", b"\x01" * 32)
    assert scratch.get("probe") == b"\x01" * 32


@darwin_only
def test_second_put_updates_rather_than_duplicating(scratch):
    scratch.put("probe", b"\x01" * 32)
    scratch.put("probe", b"\x02" * 32)
    assert scratch.get("probe") == b"\x02" * 32


@darwin_only
def test_delete_reports_whether_there_was_anything(scratch):
    scratch.put("probe", b"x")
    assert scratch.delete("probe") is True
    assert scratch.delete("probe") is False


@darwin_only
def test_arbitrary_length_credentials_round_trip(scratch):
    # The credential vault is not 32-byte-shaped; only the device key is.
    scratch.put("cred", b"a longer secret, not a key")
    assert scratch.get("cred") == b"a longer secret, not a key"


@darwin_only
def test_reading_the_real_vault_never_raises_not_found(monkeypatch):
    # A read of the real device key is safe and must not mint. On a Mac with no
    # app-provisioned key this is None; on one with a key it is 32 bytes.
    monkeypatch.delenv(atrest.ENV_VAR, raising=False)
    got = atrest.load()
    assert got is None or len(got) == atrest.KEY_BYTES


def test_env_key_unset_is_none(monkeypatch):
    monkeypatch.delenv(atrest.ENV_VAR, raising=False)
    assert atrest.from_env() is None


def test_env_key_is_64_hex(monkeypatch):
    monkeypatch.setenv(atrest.ENV_VAR, "ab" * 32)
    assert atrest.from_env() == b"\xab" * 32


def test_env_key_tolerates_surrounding_whitespace(monkeypatch):
    monkeypatch.setenv(atrest.ENV_VAR, "  " + "cd" * 32 + "\n")
    assert atrest.from_env() == b"\xcd" * 32


def test_short_env_key_is_reported_not_ignored(monkeypatch):
    monkeypatch.setenv(atrest.ENV_VAR, "ab" * 16)
    with pytest.raises(errors.Malformed):
        atrest.from_env()


def test_non_hex_env_key_is_reported_not_ignored(monkeypatch):
    monkeypatch.setenv(atrest.ENV_VAR, "z" * 64)
    with pytest.raises(errors.Malformed):
        atrest.from_env()


@darwin_only
def test_resolve_falls_back_to_env_when_the_store_has_none(monkeypatch):
    monkeypatch.setattr(atrest, "vault", lambda: Keychain(SCRATCH_SERVICE))
    monkeypatch.setenv(atrest.ENV_VAR, "ef" * 32)
    assert atrest.resolve() == b"\xef" * 32


@darwin_only
def test_resolve_prefers_the_store_over_the_env(monkeypatch):
    kc = Keychain(SCRATCH_SERVICE)
    monkeypatch.setattr(atrest, "vault", lambda: kc)
    kc.put(DEVICE_KEY_ACCOUNT, b"\x11" * 32)
    monkeypatch.setenv(atrest.ENV_VAR, "ef" * 32)
    try:
        assert atrest.resolve() == b"\x11" * 32
    finally:
        kc.delete(DEVICE_KEY_ACCOUNT)


@darwin_only
def test_a_wrong_sized_stored_key_is_reported_never_replaced(monkeypatch):
    kc = Keychain(SCRATCH_SERVICE)
    monkeypatch.setattr(atrest, "vault", lambda: kc)
    kc.put(DEVICE_KEY_ACCOUNT, b"\x09" * 16)  # short, corrupt
    try:
        with pytest.raises(errors.Malformed) as caught:
            atrest.load()
        assert caught.value.got == 16
        # and it is STILL THERE — the whole point
        assert kc.get(DEVICE_KEY_ACCOUNT) == b"\x09" * 16
    finally:
        kc.delete(DEVICE_KEY_ACCOUNT)


@darwin_only
def test_provision_mints_once_then_is_stable(monkeypatch):
    kc = Keychain(SCRATCH_SERVICE)
    monkeypatch.setattr(atrest, "vault", lambda: kc)
    kc.delete(DEVICE_KEY_ACCOUNT)
    try:
        first = atrest.provision()
        assert len(first) == atrest.KEY_BYTES
        assert atrest.provision() == first  # idempotent, does NOT re-mint
    finally:
        kc.delete(DEVICE_KEY_ACCOUNT)


@darwin_only
def test_provision_does_not_mint_when_the_store_will_not_answer(monkeypatch):
    # The data-loss bug, as a test: an unreadable store must raise, not mint.
    monkeypatch.setattr(
        _darwin, "query_bytes",
        lambda s, a: (_darwin.errSecInteractionNotAllowed, None),
    )
    minted = []
    monkeypatch.setattr(
        _darwin, "add_or_update",
        lambda s, a, d: minted.append(d) or _darwin.errSecSuccess,
    )
    with pytest.raises(errors.Unreadable):
        atrest.provision()
    assert minted == [], "minted a key over a store that merely did not answer"


def test_seal_magic():
    assert atrest.is_sealed(b"PxS1" + b"\x00" * 24)
    assert not atrest.is_sealed(b"not a seal")
    assert not atrest.is_sealed(b"")


def test_unsupported_platform_is_explicit(monkeypatch):
    monkeypatch.setattr(_darwin, "_fw", None)
    monkeypatch.setattr(sys, "platform", "linux")
    with pytest.raises(errors.Unsupported):
        _darwin.framework()
    monkeypatch.setattr(_darwin, "_fw", None)
