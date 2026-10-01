"""pacific_seal — the seal, re-implemented in Python, byte-identical to seal.rs.

This is the load-bearing crypto of the adversary harness. It re-implements
`core/pacific-core/src/seal.rs` so a hostile observer can OPEN a sealed blob it
scraped off the relay, given only the routing tag the blob was published under.

seal.rs, verbatim:
    key = HKDF-SHA256(salt = conn_secret, ikm = b"",
                      info = b"pacific/seal/v1" || dest_tag)   -> 32 bytes
    then XChaCha20-Poly1305, 24-byte random nonce, AAD = dest_tag,
    wire layout: nonce(24) || ciphertext(|| 16-byte poly1305 tag)

The vulnerability this harness demonstrates (E10 / gate G0): at pairing,
`node.rs:319` and `node.rs:632` call `seal::seal(&payload, &dest, &dest)` where
`dest == bundle.intro_tag`. So conn_secret == dest_tag == the tag. The seal key
is therefore a pure function of the routing address, and `pacific_wire::tag_hex`
is `hex::encode` (the RAW tag, no digest), so the relay holds the tag itself.
Anyone who can route can derive the key and decrypt.

Primitives, per the task:
  - cryptography's HKDF for the derivation
  - PyNaCl's crypto_aead_xchacha20poly1305_ietf_decrypt for the open
"""

from __future__ import annotations

import cbor2
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from nacl.bindings import (
    crypto_aead_xchacha20poly1305_ietf_decrypt,
    crypto_aead_xchacha20poly1305_ietf_encrypt,
)

SEAL_INFO_PREFIX = b"pacific/seal/v1"  # seal.rs: SEAL_INFO_PREFIX
NONCE_LEN = 24  # XChaCha20 nonce; seal.rs: NONCE_LEN


def derive_key(conn_secret: bytes, dest_tag: bytes) -> bytes:
    """seal.rs::derive_key — HKDF-SHA256(salt=conn_secret, ikm=b"", info=prefix||dest_tag).

    Both arguments are 32 bytes. Returns a 32-byte XChaCha20-Poly1305 key.
    """
    if len(conn_secret) != 32 or len(dest_tag) != 32:
        raise ValueError("conn_secret and dest_tag must be 32 bytes each")
    hkdf = HKDF(
        algorithm=hashes.SHA256(),
        length=32,
        salt=conn_secret,  # HKDF salt == conn_secret
        info=SEAL_INFO_PREFIX + dest_tag,  # info == "pacific/seal/v1" || dest_tag
    )
    return hkdf.derive(b"")  # ikm == empty, exactly Hkdf::new(salt, &[])


def open_sealed(blob: bytes, dest_tag: bytes, conn_secret: bytes) -> bytes:
    """seal.rs::open — split nonce(24) || ct, AEAD-open with AAD = dest_tag.

    Raises on a wrong key or any tamper (the AEAD tag check fails loudly).
    """
    if len(blob) < NONCE_LEN:
        raise ValueError(f"sealed blob too short: {len(blob)} bytes")
    nonce, ct = blob[:NONCE_LEN], blob[NONCE_LEN:]
    key = derive_key(conn_secret, dest_tag)
    return crypto_aead_xchacha20poly1305_ietf_decrypt(ct, dest_tag, nonce, key)


def seal(inner: bytes, dest_tag: bytes, conn_secret: bytes, nonce: bytes | None = None) -> bytes:
    """seal.rs::seal — for parity checks only (the adversary never needs to seal).

    Provide `nonce` (24 bytes) to make output deterministic; otherwise a random
    24-byte nonce is used. Output: nonce(24) || ciphertext.
    """
    import os

    if nonce is None:
        nonce = os.urandom(NONCE_LEN)
    if len(nonce) != NONCE_LEN:
        raise ValueError("nonce must be 24 bytes")
    key = derive_key(conn_secret, dest_tag)
    ct = crypto_aead_xchacha20poly1305_ietf_encrypt(inner, dest_tag, nonce, key)
    return nonce + ct


# ---------------------------------------------------------------------------
# IntroPayload — the CBOR shape sealed to a peer's intro mailbox.
# handshake.rs: IntroPayload::encode() is canonical CBOR (ciborium) of a struct
# with fields serialized as a MAP keyed by field NAME:
#   scanner_pk (32 bytes), scanner_name (str), welcome (bytes),
#   why/kind/arc (optional str, omitted when None),
#   owner (optional 32 bytes, omitted when None).
# ---------------------------------------------------------------------------

INTRO_FIELDS = ("scanner_pk", "scanner_name", "why", "kind", "arc", "owner")


def decode_intro(inner: bytes) -> dict:
    """Decode the CBOR IntroPayload plaintext into a plain dict.

    Byte fields (scanner_pk, owner, welcome) come back as `bytes`; we hex them for
    the ones the adversary reports on, and keep welcome as raw (opaque to eve).
    """
    obj = cbor2.loads(inner)
    if not isinstance(obj, dict):
        raise ValueError(f"IntroPayload is not a CBOR map: {type(obj)!r}")
    out: dict = {}
    if "scanner_pk" in obj:
        out["scanner_pk"] = bytes(obj["scanner_pk"]).hex()
    out["scanner_name"] = obj.get("scanner_name")
    out["why"] = obj.get("why")
    out["kind"] = obj.get("kind")
    out["arc"] = obj.get("arc")
    if obj.get("owner") is not None:
        out["owner"] = bytes(obj["owner"]).hex()
    else:
        out["owner"] = None
    w = obj.get("welcome")
    out["welcome_len"] = len(w) if w is not None else 0
    return out
