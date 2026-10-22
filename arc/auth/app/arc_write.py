"""Authorising a write to an opaque locator.

The Arc holds no seed, so it cannot verify that a locator belongs to anybody.
It cannot even tell that two locators belong to the same person -- that is the
property the whole scheme exists for. So write authority is FIRST-WRITE-CLAIMS:
the first PUT to a locator records the `acct` key it was signed by, and every
later write to that locator must carry a signature under the same key.

`acct` is `Ed25519(HKDF(seed, "arc-write/v1"))`. It appears nowhere else in the
system and has no computable relation to the identity key, so this service can
hold both halves of a person and be unable to join them.

ONE DEFINITION, TWO LANGUAGES, exactly as `auth.py` says of the challenge
payload. The bytes verified here must match `pacific_core::locator::write_payload`
byte for byte; a signer and a verifier that disagree by one byte fail in a way
neither side can see, and here the failure is worse than a refused sign-in --
under first-write-claims a wrong record is PERMANENT, because the Arc cannot
arbitrate between a claimant and an impostor. `tests/test_arc_write.py` pins the
two implementations against each other and will go red if either moves.

WHY THE PAYLOAD CARRIES AN AUDIENCE AND A NONCE. The first version of this bound
only the locator and the body, which looks sufficient because the body is what
gets stored. Two things were missing and both cost something real:

  * No audience -- a signature captured at one Arc verifies at another. An
    account is meant to be re-homeable, so the same locator legitimately exists
    at more than one Arc, and a write authorised for one was authorised for all.

  * No nonce -- the signature verifies forever. Because the body is bound the
    replay is idempotent, so this is not forgery; it is ROLLBACK. Replay a
    captured history write and the stored history moves backwards, and
    `Store.put_history`'s guard cannot stop it: that guard compares an
    `exported_at` HEADER, which is inside no signature, and it accepts a write
    carrying no stamp at all. The head survives only because `put_head`
    compare-and-sets on a position it enforces itself.

So a write is a challenge-response like every other authenticated route here.
"""
from __future__ import annotations

import re

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

from .errors import AuthError

# `pacific_core::locator::ACCT_SIG_DOMAIN`. The first line of the payload, and
# its domain separation: an arc-write signature can never be replayed as any
# other signature this system produces, and vice versa.
DOMAIN = "pacific-arc-write:v1"

# A locator and an acct key are both 32 bytes, rendered the way the core renders
# them. Lowercase only, and not normalised into it -- for the reason
# `auth.identity_pk` gives: one address, one spelling, and anything else is
# malformed rather than helpfully corrected.
_HEX32 = re.compile(r"^[0-9a-f]{64}$")


def hex32(raw: str, what: str) -> str:
    v = (raw or "").strip()
    if not _HEX32.match(v):
        raise AuthError("invalid_request", f"{what} must be 64 lowercase hex characters, got {raw!r}")
    return v


def payload(audience: str, locator_hex: str, nonce: str, body: bytes) -> bytes:
    """The bytes a device signs. Mirrors `locator::write_payload` in the core.

    `domain \\n audience \\n <locator hex> \\n nonce \\n <body>`, body last and raw.

    A NEWLINE IN THE AUDIENCE OR THE NONCE IS REFUSED, and that is not tidiness.
    Without it the encoding is not injective: `nonce="n\\n", body="\\n"` and
    `nonce="n", body="\\n\\n"` build identical bytes, so one signature authorises
    BOTH writes. The nonce is issued by the Arc, which is exactly the party this
    scheme defends against -- an Arc that issued a nonce containing a newline
    could re-present a signature the device made for one body as authorising a
    different one. The locator is hex and the domain is ours, so these two fields
    are the whole of the exposure.
    """
    for what, field in (("audience", audience), ("nonce", nonce)):
        if "\n" in field:
            raise AuthError(
                "write_refused",
                f"an arc-write {what} may not contain a newline: it would make two "
                f"different writes sign to the same bytes ({field!r})",
            )
    return f"{DOMAIN}\n{audience}\n{locator_hex}\n{nonce}\n".encode() + body


def verify(
    *,
    recorded_hex: str,
    offered_hex: str,
    audience: str,
    locator_hex: str,
    nonce: str,
    body: bytes,
    signature_hex: str,
) -> None:
    """Check a write, or raise. Nothing is returned.

    `offered_hex` is the acct key the writer presents on EVERY write, not only
    the first. It is compared to the recorded key before the signature, and the
    signature is then checked against the RECORDED key and never against the
    offered one -- verifying against the key the writer supplied would authorise
    everybody.

    ONE CODE ON THE WIRE, BOTH CASES IN THE LOG. The refusal the client sees does
    not say which of the two it was, for the reason `auth.verify` gives: a caller
    that could tell "wrong key" from "wrong nonce" could enumerate. The `detail`
    does say, and `main._refuse` logs it -- because first-write-claims fails
    SILENTLY by design, and without a line naming both keys there is no way to
    tell a squatted address from a client bug. That distinction is the whole
    difference between "someone is attacking this" and "we shipped a regression",
    and it is not recoverable after the fact.
    """
    recorded = hex32(recorded_hex, "the recorded arc-write key")
    offered = hex32(offered_hex, "the offered arc-write key")
    locator = hex32(locator_hex, "locator")

    if recorded != offered:
        raise AuthError(
            "write_refused",
            f"locator {locator[:16]}… is claimed by {recorded[:16]}… and this "
            f"write offered {offered[:16]}… — a different key, so either the "
            f"address was claimed by someone else or this client derived acct wrong",
        )

    try:
        sig = bytes.fromhex((signature_hex or "").strip())
    except ValueError:
        raise AuthError("write_refused", f"locator {locator[:16]}…: signature is not hex")
    if len(sig) != 64:
        raise AuthError(
            "write_refused", f"locator {locator[:16]}…: signature is {len(sig)} bytes, want 64"
        )

    try:
        key = Ed25519PublicKey.from_public_bytes(bytes.fromhex(recorded))
    except ValueError:
        # Only reachable if the stored row is corrupt, and worth its own line:
        # the account cannot write to its own address until somebody looks.
        raise AuthError(
            "write_refused",
            f"locator {locator[:16]}…: the RECORDED key {recorded[:16]}… is not a "
            f"valid Ed25519 point — the stored claim is corrupt",
        )

    try:
        key.verify(sig, payload(audience, locator, nonce, body))
    except InvalidSignature:
        raise AuthError(
            "write_refused",
            f"locator {locator[:16]}…: the right key {recorded[:16]}… but the bytes "
            f"do not verify — a spent or wrong nonce, the wrong audience "
            f"({audience!r}), a mangled body, or a client signing an older payload",
        )
