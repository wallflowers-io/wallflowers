"""The device master key — the one secret every other at-rest secret hangs off.

Mirrors ``pacific_core::atrest::device_key`` and ``DeviceKey.provision`` in
Swift. What the key does, in core's words (``core/docs/keys.md`` §5): it seals
the identity key file, the MLS leaf secret, and every group's ratchet and epoch
state. Losing it means the sealed store cannot be opened, and that is the
design — **no key, no plaintext.**

Two sources, in the order ``atrest.rs`` resolves them:

1. the OS secure store — what the iOS host injects at startup
2. ``$PACIFIC_ATREST_KEY``, 64 hex chars — "for the CLI and tests", which is
   what a Python process is

``resolve()`` returns ``None`` when neither is configured, and raises when one
is present but malformed. A bad key is never silently ignored, and a key that
is merely unavailable is never mistaken for one that is absent.

**On minting.** :func:`provision` will create a key when the store positively
reports there is none. That is safe on a machine that has no sealed store yet
and catastrophic on one that does, so it is a separate function from
:func:`load` and it is not what :func:`resolve` calls. If you are unsure which
you want, you want :func:`load`.
"""
from __future__ import annotations

import binascii
import os
import secrets

from .errors import Malformed, RandomFailed
from .keychain import ATREST_SERVICE, DEVICE_KEY_ACCOUNT, Keychain

#: The key is 32 bytes. atrest.rs: `[u8; 32]`; DeviceKey.swift: `Data(count: 32)`.
KEY_BYTES = 32

#: The CLI/test injection path. atrest.rs:6-8, "64 hex chars".
ENV_VAR = "PACIFIC_ATREST_KEY"


def vault() -> Keychain:
    """The at-rest vault — ``network.pacific.atrest``."""
    return Keychain(ATREST_SERVICE)


def from_env() -> bytes | None:
    """The key from ``$PACIFIC_ATREST_KEY``, or ``None`` if unset.

    Raises :class:`~wallflowers.errors.Malformed` if the variable is set to
    anything that is not 64 hex characters. An operator who exported a truncated
    key wants to hear about it now, not to have it ignored and the store opened
    unsealed.
    """
    raw = os.environ.get(ENV_VAR)
    if raw is None:
        return None
    text = raw.strip()
    if len(text) != KEY_BYTES * 2:
        raise Malformed(len(text) // 2, KEY_BYTES)
    try:
        return binascii.unhexlify(text)
    except (binascii.Error, ValueError) as exc:
        raise Malformed(-1, KEY_BYTES) from exc


def load() -> bytes | None:
    """The stored key, or ``None`` **only** when the store reports no such item.

    Never mints. Raises :class:`~wallflowers.errors.Unreadable` if the keychain
    did not answer, and :class:`~wallflowers.errors.Malformed` if a stored item
    is the wrong size — reported, never silently replaced, because overwriting
    it would discard the only key that could open the existing sealed store.
    """
    raw = vault().find(DEVICE_KEY_ACCOUNT)
    if raw is None:
        return None
    if len(raw) != KEY_BYTES:
        raise Malformed(len(raw), KEY_BYTES)
    return raw


def resolve() -> bytes | None:
    """The key from the secure store, else the environment, else ``None``.

    The order matches ``atrest.rs``: the store the host injects from wins, and
    the env var is the fallback for a machine that has no keychain item — or no
    keychain at all, which is why this function works on Linux.
    """
    try:
        stored = load()
    except Exception:
        # A keychain that will not answer must not silently demote to the env
        # var: the two may differ, and opening a store with the wrong key is a
        # tag failure at best. Let the caller see it.
        raise
    if stored is not None:
        return stored
    return from_env()


def provision() -> bytes:
    """The existing key, or — ONLY when the store positively reports none — a
    freshly minted and persisted one.

    Read the warning in this module's docstring first. On a machine that already
    holds a sealed store, minting is how you lose it.
    """
    existing = load()
    if existing is not None:
        return existing
    try:
        fresh = secrets.token_bytes(KEY_BYTES)
    except NotImplementedError as exc:  # pragma: no cover - no OS CSPRNG
        raise RandomFailed("the OS CSPRNG refused") from exc
    vault().put(DEVICE_KEY_ACCOUNT, fresh)
    return fresh


def is_sealed(blob: bytes) -> bool:
    """Whether a blob carries the at-rest seal magic, ``PxS1``.

    atrest.rs: ``MAGIC(4) || nonce(24) || ciphertext``. Useful for telling a
    sealed secret from a legacy plaintext one without holding the key. This
    package does not open seals — it manages the key that does.
    """
    return blob[:4] == b"PxS1"
