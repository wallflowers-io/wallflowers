"""Proving an account to this Arc.

The Arc knows an account by its Ed25519 identity public key and challenges it
on that. There is no cookie, no bearer token and no WebAuthn in this service:
the passkey is a LOCAL UNLOCK that opens the wrap on the device, and it never
reaches us. That is the whole authentication model.

WHY NO COOKIE. A cookie is a bearer credential -- whoever holds it is the
account -- so it has to be issued, scoped, expired, refreshed, and protected
from every context that might carry it somewhere it should not go. A signature
over a nonce we just issued is none of those things: it proves possession of
the key at this moment, it cannot be replayed, and it works identically from a
phone, a script and a cross-site iframe. The last of those matters more than it
looks: a SameSite cookie does not reach an embedded frame in production, which
is the failure the web client would otherwise inherit.

ONE DEFINITION, TWO LANGUAGES. The bytes signed here must match
`pacific_core::identity::auth_payload` exactly -- a signer and a verifier that
disagree by one byte fail in a way neither side can see. The Rust builds:

    pacific-auth:v1\\n<audience>\\n<identity_key>\\n<nonce>

where `identity_key` is the RENDERED key (`ed25519:` + 64 hex), not the bare
hex the URL carries. Two properties come from that payload: the audience is
inside the signed bytes, so a signature captured by one Arc cannot be replayed
at another; and the nonce is ours, single-use, so there is no replay window at
all -- where a timestamp would leave one and need a clock both sides agree on.
"""
from __future__ import annotations

import re

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

from .errors import AuthError

# `pacific_core::identity::IDENTITY_KEY_PREFIX`.
KEY_PREFIX = "ed25519:"
# The first line of the payload; also its domain separation.
DOMAIN = "pacific-auth:v1"
# How long an unspent challenge lives. Long enough for a human to approve a
# passkey prompt, short enough that a captured one is worthless by the time it
# is useful. Single use regardless -- this is the ceiling, not the mechanism.
CHALLENGE_TTL = 120

# The address grammar in every route: a 32-byte Ed25519 public key as lowercase
# hex. Anything else is malformed rather than a miss -- it could never name an
# account we hold, so saying "not found" would be a guess dressed as an answer.
_PK = re.compile(r"^[0-9a-f]{64}$")


def identity_pk(raw: str) -> str:
    """The path segment as a public key, or a refusal.

    LOWERCASE ONLY, and not normalised into it. The core renders a key as
    lowercase hex, so accepting `AB…` and folding it would give one account two
    spellings of its own address — two cache entries, two log lines, and two
    strings a reader has to know are the same account. One address, one
    spelling, and anything else is malformed.
    """
    pk = (raw or "").strip()
    if not _PK.match(pk):
        raise AuthError(
            "invalid_request",
            f"identity key must be 64 lowercase hex characters, got {raw!r}",
        )
    return pk


def payload(audience: str, identity_pk_hex: str, nonce: str) -> bytes:
    """The bytes a device signs. Mirrors `identity::auth_payload` in the core."""
    return f"{DOMAIN}\n{audience}\n{KEY_PREFIX}{identity_pk_hex}\n{nonce}".encode()


def verify(audience: str, identity_pk_hex: str, nonce: str, signature_hex: str) -> None:
    """Check the signature, or raise. Nothing is returned: the caller already
    knows which account it is asking about -- the point of this is only whether
    that account authorised the request."""
    try:
        sig = bytes.fromhex((signature_hex or "").strip())
    except ValueError:
        raise AuthError("bad_signature", "signature is not hex")
    if len(sig) != 64:
        raise AuthError("bad_signature", f"signature is {len(sig)} bytes, want 64")

    try:
        key = Ed25519PublicKey.from_public_bytes(bytes.fromhex(identity_pk_hex))
    except ValueError:
        raise AuthError("invalid_request", "identity key is not a valid Ed25519 public key")

    try:
        key.verify(sig, payload(audience, identity_pk_hex, nonce))
    except InvalidSignature:
        # Deliberately says nothing about WHICH part failed. A caller that could
        # tell "wrong key" from "wrong nonce" could enumerate accounts.
        raise AuthError("bad_signature", "signature does not prove possession of this key")
