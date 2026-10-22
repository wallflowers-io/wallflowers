"""The pieces, without a server.

The first test here is the most important one in the suite: the payload this
service verifies is built in Rust by the device that signs it, and a signer and
a verifier one byte apart fail in a way neither side can see.
"""
from __future__ import annotations

import os

import pytest

from app import config
from app.auth import identity_pk, payload
from app.errors import AuthError
from app.main import _slug_verdict


def test_the_payload_matches_the_rust_byte_for_byte():
    """`pacific_core::identity::auth_payload` builds

        pacific-auth:v1\\n<audience>\\n<identity_key>\\n<nonce>

    with `identity_key` RENDERED — `ed25519:` plus 64 hex — and not the bare hex
    the URL path carries. Pinned as a literal rather than rebuilt from the same
    constants this module uses, because a test that derives the expected value
    the same way the code does cannot catch the code being wrong.
    """
    assert payload("kenjin.cc", "ab" * 32, "n0nce") == (
        b"pacific-auth:v1\nkenjin.cc\ned25519:" + b"ab" * 32 + b"\nn0nce"
    )


def test_each_field_of_the_payload_actually_moves_it():
    base = payload("kenjin.cc", "ab" * 32, "n1")
    assert payload("arc.example", "ab" * 32, "n1") != base   # audience binds
    assert payload("kenjin.cc", "cd" * 32, "n1") != base     # account binds
    assert payload("kenjin.cc", "ab" * 32, "n2") != base     # challenge binds


def test_an_address_is_64_lowercase_hex_or_a_refusal():
    assert identity_pk(" " + "ab" * 32 + " ") == "ab" * 32   # surrounding space only
    # Uppercase is REFUSED, not folded: one account must not have two spellings
    # of its own address.
    for bad in ("", "AB" * 32, "zz" * 32, "ab" * 31, "ab" * 33, "not hex at all"):
        with pytest.raises(AuthError) as e:
            identity_pk(bad)
        assert e.value.code == "invalid_request"


def test_slug_verdicts():
    assert _slug_verdict("stoma") == (True, "ok")
    assert _slug_verdict("STOMA") == (True, "ok")            # lowercased
    assert _slug_verdict("a-b") == (True, "ok")
    assert _slug_verdict("") == (False, "empty")
    assert _slug_verdict("a") == (False, "shape")            # one char is not shareable aloud
    assert _slug_verdict("-lead") == (False, "shape")
    assert _slug_verdict("trail-") == (False, "shape")
    assert _slug_verdict("has space") == (False, "shape")
    assert _slug_verdict("x" * 33) == (False, "shape")
    assert _slug_verdict("auth") == (False, "reserved")


def _with_env(**kv):
    """Set env for one load(), restoring exactly what was there."""
    was = {k: os.environ.get(k) for k in kv}

    class _Ctx:
        def __enter__(self):
            for k, v in kv.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v
            return self

        def __exit__(self, *_):
            for k, v in was.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v
            return False

    return _Ctx()


def test_the_audience_comes_from_the_public_origin():
    with _with_env(PUBLIC_ORIGIN="https://dev.kenjin.cc", AUTH_AUDIENCE=None):
        assert config.load().audience == "dev.kenjin.cc"


def test_the_audience_can_be_set_outright():
    with _with_env(PUBLIC_ORIGIN="https://kenjin.cc", AUTH_AUDIENCE="arc.example.org"):
        assert config.load().audience == "arc.example.org"


def test_an_origin_with_no_host_refuses_to_boot():
    """Booting with an audience nobody can derive would mean every signature
    failing verification with nothing to point at."""
    with _with_env(PUBLIC_ORIGIN="not-a-url", AUTH_AUDIENCE=None):
        with pytest.raises(RuntimeError, match="audience"):
            config.load()


def test_a_nonsense_cap_refuses_to_boot():
    with _with_env(HISTORY_MAX_BYTES="0"):
        with pytest.raises(RuntimeError, match="must be positive"):
            config.load()
