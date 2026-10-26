"""Environment in, frozen settings out.

There is nothing optional left to express. The service authenticates a
signature over a nonce it issued, stores two opaque blobs and reserves slugs --
so there is no provider to configure, no mailer to wire, and no secret to sign
cookies with, because there are no cookies. What remains is where the database
lives, which Arc new accounts are told about, and how big a blob may be.

THE AUDIENCE IS LOAD-BEARING. It goes inside the bytes a device signs, so a
signature captured by this Arc cannot be replayed at another. It must be the
same string the device puts in its payload -- the arc host, exactly as the
user handle carries it -- or every request fails verification with nothing to
point at. Derived from PUBLIC_ORIGIN so there is one place it comes from.
"""
from __future__ import annotations

import os
from dataclasses import dataclass
from urllib.parse import urlparse


def _env(name: str, default: str = "") -> str:
    return (os.environ.get(name) or default).strip()


def _int(name: str, default: int) -> int:
    raw = _env(name)
    if not raw:
        return default
    try:
        return int(raw)
    except ValueError:
        raise RuntimeError(f"{name}={raw!r} is not an integer")


@dataclass(frozen=True)
class Settings:
    public_origin: str
    # The host a device signs for. Not the whole URL: the user handle carries a
    # host, and that is what the device has to work from.
    audience: str
    db_path: str
    arc_url: str
    # A wrap is a 32-byte seed in a 76-byte envelope. The cap is generous and
    # still small enough that nobody can store a file in it.
    wrap_max_bytes: int = 4096
    # A history carries an archive and an MLS snapshot, so it is the one that
    # can actually grow. This is the only control against a client filling the
    # disk with them.
    history_max_bytes: int = 64 * 1024 * 1024


def load() -> Settings:
    public_origin = _env("PUBLIC_ORIGIN", "https://kenjin.cc").rstrip("/")
    audience = _env("AUTH_AUDIENCE") or urlparse(public_origin).netloc
    if not audience:
        raise RuntimeError(
            f"cannot derive an audience from PUBLIC_ORIGIN={public_origin!r}; "
            "set AUTH_AUDIENCE to the host devices sign for"
        )

    history_max = _int("HISTORY_MAX_BYTES", Settings.history_max_bytes)
    if history_max <= 0:
        raise RuntimeError(f"HISTORY_MAX_BYTES={history_max} must be positive")

    return Settings(
        public_origin=public_origin,
        audience=audience,
        db_path=_env("DB_PATH", "/data/auth.db"),
        # The Arc this deployment hosts accounts on. Anyone can run one
        # (arc-node is a binary and the protocol is open), so this is a default
        # for new accounts, not a global truth -- each account records the Arc
        # it was actually hosted on.
        arc_url=_env("ARC_URL").rstrip("/"),
        wrap_max_bytes=_int("WRAP_MAX_BYTES", Settings.wrap_max_bytes),
        history_max_bytes=history_max,
    )
