"""Fixtures.

There is no session to forge and no sender to capture any more: the service
authenticates a signature over a nonce it issued, so a test signs exactly as a
device does — same payload, same single-use challenge. `Device` below is that
device, and it is the only fixture with any behaviour in it.

The store is a real one on :memory:, and the app is the real app through its
real lifespan. Nothing here stubs the thing under test.
"""
from __future__ import annotations

import os

os.environ.setdefault("DB_PATH", ":memory:")
os.environ.setdefault("PUBLIC_ORIGIN", "https://kenjin.cc")
os.environ.setdefault("ARC_URL", "")          # no discovery during tests

import pytest                                                 # noqa: E402
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402
from cryptography.hazmat.primitives.serialization import (     # noqa: E402
    Encoding, PublicFormat,
)
from fastapi.testclient import TestClient                      # noqa: E402

from app.auth import payload                                   # noqa: E402
from app.main import app                                       # noqa: E402

AUDIENCE = "kenjin.cc"
OCTET = {"content-type": "application/octet-stream"}


def a_wrap(fill: int = 0) -> bytes:
    """A wrap-shaped body: MAGIC || nonce(24) || ct(32) || tag(16). The service
    judges shape and size and never opens it, so the bytes inside are free."""
    return b"PWR1" + bytes([fill]) * (24 + 32 + 16)


def a_history(extra: int = 64) -> bytes:
    """A history-shaped body. Variable length, unlike a wrap."""
    return b"PHS1" + bytes(24 + 16 + extra)


def a_head(fill: int = 0) -> bytes:
    """A head-shaped body: MAGIC || nonce(24) || ciphertext. About the size a
    real one is (`pacific_core::head` seals ~150 bytes), and opaque to the
    service in exactly the same way -- it judges magic, length and the DECLARED
    position, and never the contents. `fill` is how a test makes two heads that
    differ, which is what a fork looks like from here."""
    return b"PHD1" + bytes([fill]) * (24 + 16 + 96)


class Device:
    """One device: a keypair, and the ability to sign a challenge."""

    def __init__(self, client: TestClient):
        self._client = client
        self._sk = Ed25519PrivateKey.generate()
        self.pk = self._sk.public_key().public_bytes(
            Encoding.Raw, PublicFormat.Raw).hex()

    # A fresh challenge per call, because they are single use — which is also
    # why every helper here takes one rather than holding one.
    def headers(self, audience: str = AUDIENCE) -> dict[str, str]:
        nonce = self._client.get("/auth/challenge").json()["nonce"]
        sig = self._sk.sign(payload(audience, self.pk, nonce)).hex()
        return {"X-Pacific-Challenge": nonce, "X-Pacific-Signature": sig}

    def sign_for(self, pk: str) -> dict[str, str]:
        """Sign a challenge naming SOMEONE ELSE'S key — the forgery attempt."""
        nonce = self._client.get("/auth/challenge").json()["nonce"]
        sig = self._sk.sign(payload(AUDIENCE, pk, nonce)).hex()
        return {"X-Pacific-Challenge": nonce, "X-Pacific-Signature": sig}

    def register(self, body: bytes | None = None):
        return self._client.put(f"/auth/users/{self.pk}",
                                content=body if body is not None else a_wrap(),
                                headers={**self.headers(), **OCTET})

    def put_history(self, body: bytes | None = None, **headers: str):
        return self._client.put(f"/auth/users/{self.pk}/history",
                                content=body if body is not None else a_history(),
                                headers={**self.headers(), **OCTET, **headers})

    def put_head(self, position: int, body: bytes | None = None, **headers: str):
        """Move the chain anchor. Signed, like every other write."""
        return self._client.put(
            f"/auth/users/{self.pk}/head",
            content=body if body is not None else a_head(),
            headers={**self.headers(), **OCTET,
                     "X-Chain-Position": str(position), **headers})

    def get_history(self, which: str = "current"):
        return self._client.get(f"/auth/users/{self.pk}/history?which={which}",
                                headers=self.headers())

    def account(self):
        return self._client.get(f"/auth/users/{self.pk}/account", headers=self.headers())

    def claim(self, slug: str):
        return self._client.post(f"/auth/users/{self.pk}/sites", json={"slug": slug},
                                 headers=self.headers())


@pytest.fixture()
def client():
    with TestClient(app) as c:
        yield c


@pytest.fixture()
def device(client):
    return Device(client)
