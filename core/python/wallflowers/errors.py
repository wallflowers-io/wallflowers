"""The error taxonomy, mirroring ``DeviceKeyError`` in DeviceKey.swift.

The distinction this module exists to preserve: **"there is no key" and "the
store did not answer" are opposite situations.** Conflating them was a
data-loss bug on iOS (see the header of
``app/ios/app/ui/App/System/DeviceKey.swift``) — the old ``load()`` returned
nil on ANY OSStatus, so a transient read failure minted a brand-new master
key and overwrote the only one that could open the existing sealed store.

Every error here carries the raw OSStatus where it has one, because the number
is the evidence and swallowing it is how the original bug hid.
"""
from __future__ import annotations


class KeychainError(Exception):
    """Base for every failure in this package."""


class NotFound(KeychainError):
    """The store positively reported no such item (``errSecItemNotFound``).

    The ONLY condition under which minting a new key is safe.
    """

    def __init__(self, service: str, account: str) -> None:
        super().__init__(f"no item {service}/{account}")
        self.service = service
        self.account = account


class Unreadable(KeychainError):
    """The store failed to answer a read. NOT the same as "no key stored".

    A key we cannot read is not a key that is absent. Callers must not treat
    this as grounds to mint, replace or discard anything.
    """

    def __init__(self, status: int) -> None:
        super().__init__(
            f"the secure store did not answer (OSStatus {status}); the data is "
            "intact and sealed — unlock the device and retry"
        )
        self.status = status


class Unwritable(KeychainError):
    """The store refused to persist an item."""

    def __init__(self, status: int) -> None:
        super().__init__(f"could not store the item (OSStatus {status})")
        self.status = status


class Malformed(KeychainError):
    """A stored item exists but is not the expected size.

    Reported, never silently replaced: overwriting it would discard the only
    key that could open the existing sealed store.
    """

    def __init__(self, got: int, want: int) -> None:
        super().__init__(f"the stored device key is {got} bytes, not {want}")
        self.got = got
        self.want = want


class RandomFailed(KeychainError):
    """The OS CSPRNG refused. No key is minted from a weaker source."""


class Unsupported(KeychainError):
    """This platform has no secure store this package can reach.

    Raised instead of falling back to a file. Fail loud, never a plaintext
    fallback — a secret written in the clear because the keychain was missing
    is the failure it looks like it is avoiding.
    """
