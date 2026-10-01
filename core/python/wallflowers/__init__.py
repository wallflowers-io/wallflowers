"""WallFlowers — key management for a WallFlowers Node.

The first published piece of the Python surface: the secrets, and where they
live. Before a Python process can speak for a Node it has to hold that Node's
keys, and on a Mac those keys are already in the OS secure store, put there by
the app. This package reaches the same items rather than keeping a second copy.

    >>> import wallflowers
    >>> wallflowers.device_key()            # None if this machine has none
    >>> vault = wallflowers.credentials()
    >>> vault.put("some-handle", b"secret")
    >>> vault.get("some-handle")
    b'secret'

The contract it keeps, inherited from the Swift and Rust implementations it
mirrors: **fail loud, never a plaintext fallback**, and never confuse "there is
no key" with "the store did not answer". The second one is not fussiness. On
iOS, conflating them minted a fresh master key over the only one that could
open the existing sealed store, and took the identity, the MLS state and the
message history with it.

No third-party dependencies, by design: ``ctypes`` against Security.framework.
"""
from __future__ import annotations

from .atrest import (
    ENV_VAR,
    KEY_BYTES,
    from_env,
    is_sealed,
    load,
    provision,
    resolve,
)
from .errors import (
    KeychainError,
    Malformed,
    NotFound,
    RandomFailed,
    Unreadable,
    Unsupported,
    Unwritable,
)
from .keychain import (
    ATREST_SERVICE,
    CREDENTIALS_SERVICE,
    DEVICE_KEY_ACCOUNT,
    Keychain,
    credentials,
)

__version__ = "0.0.1"

#: The device master key, from the secure store or ``$PACIFIC_ATREST_KEY``.
#: An alias for :func:`wallflowers.atrest.resolve`, which is the function you
#: almost always want.
device_key = resolve

__all__ = [
    "ATREST_SERVICE",
    "CREDENTIALS_SERVICE",
    "DEVICE_KEY_ACCOUNT",
    "ENV_VAR",
    "KEY_BYTES",
    "Keychain",
    "KeychainError",
    "Malformed",
    "NotFound",
    "RandomFailed",
    "Unreadable",
    "Unsupported",
    "Unwritable",
    "__version__",
    "credentials",
    "device_key",
    "from_env",
    "is_sealed",
    "load",
    "provision",
    "resolve",
]
