"""address — the relay's write rule, in Python, for the legs that drive the REAL relay.

A SECOND IMPLEMENTATION of ``pacific_wire::address``, and it says so. It exists
because ``live/`` and ``adversary/`` talk to the real relay binary over a socket,
and since 18 Sep 2026 that relay refuses any ``Pub`` not signed by the key its tag
IS. It is held to the Rust by the vector in ``pacific-wire/wire-spec.v1.json``
(``address``), which both sides test against — ``the_rule_matches_the_vector_in_the_
wire_spec`` in Rust and ``address_rule_matches_the_wire_spec`` in cases/rust_parity.py
— so if either drifts, one of them goes red.

The constants are READ from that fixture, not written here, so the only thing this
file states on its own is the arithmetic.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from nacl.signing import SigningKey

# harness/ -> core; the fixture travels inside pacific-wire.
_SPEC = json.loads((Path(__file__).resolve().parents[1]
                    / "pacific-wire" / "wire-spec.v1.json").read_text())["address"]
SIG_DOMAIN: bytes = _SPEC["sig_domain"].encode()
KEY_LABEL: bytes = _SPEC["key_label"].encode()
VECTORS: list[dict] = _SPEC["vectors"]

#: Every address made here, by its tag — so a caller holding only a tag (which is
#: all a frame carries) can find the key that signs for it.
_BY_TAG: dict[str, "Address"] = {}


class Address:
    """An address and the key that writes it. The tag is the verifying key."""

    def __init__(self, seed: bytes) -> None:
        sk = HKDF(algorithm=hashes.SHA256(), length=32, salt=KEY_LABEL,
                  info=b"v1").derive(seed)
        self._key = SigningKey(sk)
        self.tag: bytes = bytes(self._key.verify_key)
        self.tag_hex: str = self.tag.hex()
        _BY_TAG[self.tag_hex] = self

    def sign_pub(self, blob_b64: str) -> str:
        """The signature a ``Pub`` of ``blob_b64`` to this address carries, hex."""
        return self._key.sign(SIG_DOMAIN + self.tag + blob_b64.encode()).signature.hex()


def named(label: str) -> Address:
    """A stable address for a harness name, e.g. the tag the parity run calls A."""
    return Address(hashlib.sha256(b"harness:" + label.encode()).digest())


def for_tag(tag_hex: str) -> Address:
    """The address a tag belongs to — only for tags made by this module."""
    try:
        return _BY_TAG[tag_hex]
    except KeyError:
        raise KeyError(f"no key for tag {tag_hex[:8]}… — the relay only accepts a Pub "
                       "signed by the key its tag is; make the tag with address.named()")
