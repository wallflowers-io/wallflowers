# wallflowers

Key management for a WallFlowers Node.

Before a Python process can speak for a Node it has to hold that Node's keys.
On a Mac those keys are already in the OS secure store, put there by the
WallFlowers app — so this package reaches the same items rather than keeping a
second copy of the secret.

No third-party dependencies: `ctypes` against Security.framework.

```python
import wallflowers

# the device master key, from the secure store or $PACIFIC_ATREST_KEY
key = wallflowers.device_key()          # None if this machine has none

# any System's credential, keyed by its handle
vault = wallflowers.credentials()
vault.put("some-handle", b"a secret")
vault.get("some-handle")                # b'a secret'
```

## The one rule worth reading

**"There is no key" and "the store did not answer" are opposite situations.**

`find()` returns `None` only when the keychain positively reports no such item.
Every other status raises `Unreadable`. `get()` raises `NotFound` instead of
returning a falsy value.

This is not fussiness. Every convenience wrapper around the macOS keychain —
including `security(1)` — collapses both cases into one falsy return, and code
built on that shape mints a replacement key when a read merely failed. Doing
that on a machine with an existing sealed store destroys it: the new key cannot
open what the old one sealed, and there is no escrow. WallFlowers shipped that
bug on iOS once. This package is written so you cannot reproduce it by accident,
which is why `provision()` — the only function that mints — is separate from
`load()`, and why a stored key of the wrong size is reported rather than
replaced.

The same rule, in the words of the Rust it mirrors: *a key we cannot read is not
a key that is wrong.* Fail loud, never a plaintext fallback.

## Where the secrets live

| Item | Service | Account |
|---|---|---|
| device master key, 32 bytes | `network.pacific.atrest` | `device-master-key` |
| a System's credential | `network.pacific.credentials` | the handle id |

Both are stored device-only, available after first unlock — so they are excluded
from backup and device transfer by construction.

Off macOS, set `PACIFIC_ATREST_KEY` to 64 hex characters; `device_key()` reads
it anywhere. A malformed value raises rather than being ignored.

## Status

Early. This is the key-management layer only — it manages the key that opens
sealed state, and does not itself open, sync or speak the protocol. The wire
client follows.

AGPL-3.0-or-later.
