"""The OS secure store, as WallFlowers uses it.

A SECOND IMPLEMENTATION of the two Swift vaults — ``DeviceKey.swift`` (the
at-rest master key) and ``Keychain.swift`` (System credentials) — and it says
so. It exists because a Python process on the same Mac should be able to reach
the SAME keychain items the app does, rather than keep a parallel secret of its
own.

The service and account names below are the interop contract. They mirror:

    network.pacific.atrest       / device-master-key   DeviceKey.swift:59-60
    network.pacific.credentials  / <handle id>         Keychain.swift:10

If either side renames one, they stop sharing a vault silently — which is why
they are named here in one place with the file and line that declares them,
instead of spelled inline at each call site.

**One deliberate divergence.** ``Keychain.swift``'s ``getData`` returns nil on
any OSStatus and its ``setData`` deletes before adding. This module does
neither: reads distinguish absence from failure, and writes add-then-update. On
the credentials path the lenient version is survivable (a credential can be
re-entered, and nothing is minted from its absence); this module is strict
anyway, because strictness on the read path costs nothing when you are not
about to mint.
"""
from __future__ import annotations

from . import _darwin
from .errors import NotFound, Unreadable, Unwritable

#: The at-rest master key's vault. DeviceKey.swift:59.
ATREST_SERVICE = "network.pacific.atrest"

#: The account holding the 32-byte device master key. DeviceKey.swift:60.
DEVICE_KEY_ACCOUNT = "device-master-key"

#: Any System's credential, keyed by the CredentialHandle id. Keychain.swift:10.
CREDENTIALS_SERVICE = "network.pacific.credentials"


class Keychain:
    """One service's worth of generic-password items.

    >>> vault = Keychain(CREDENTIALS_SERVICE)
    >>> vault.put("my-handle", b"a secret")
    >>> vault.get("my-handle")
    b'a secret'
    """

    def __init__(self, service: str) -> None:
        self.service = service

    def find(self, account: str) -> bytes | None:
        """The item, or ``None`` **only** when the store positively reports
        ``errSecItemNotFound``.

        Every other status raises :class:`~wallflowers.errors.Unreadable`. This
        is the fail-closed primitive; prefer it to :meth:`get` when absence is
        a normal outcome you intend to act on, and read the warning in
        :mod:`wallflowers.errors` before you act on it by writing.
        """
        status, raw = _darwin.query_bytes(self.service, account)
        if status == _darwin.errSecItemNotFound:
            return None
        if status != _darwin.errSecSuccess:
            raise Unreadable(status)
        return raw

    def get(self, account: str) -> bytes:
        """The item, raising :class:`~wallflowers.errors.NotFound` if absent."""
        raw = self.find(account)
        if raw is None:
            raise NotFound(self.service, account)
        return raw

    def put(self, account: str, data: bytes) -> None:
        """Store the item, device-only and after-first-unlock.

        Add-then-update; never delete-then-add. See
        :func:`wallflowers._darwin.add_or_update`.
        """
        status = _darwin.add_or_update(self.service, account, bytes(data))
        if status != _darwin.errSecSuccess:
            raise Unwritable(status)

    def delete(self, account: str) -> bool:
        """Remove the item. ``True`` if one was removed, ``False`` if there was
        nothing to remove. Any other status raises."""
        status = _darwin.delete(self.service, account)
        if status == _darwin.errSecSuccess:
            return True
        if status == _darwin.errSecItemNotFound:
            return False
        raise Unwritable(status)

    def __repr__(self) -> str:
        return f"Keychain({self.service!r})"


def credentials() -> Keychain:
    """The System credential vault — ``network.pacific.credentials``."""
    return Keychain(CREDENTIALS_SERVICE)
