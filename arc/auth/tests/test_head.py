"""The head: where the account's archive chain ends, and who is allowed to see it.

Two things are being pinned here and they pull in opposite directions.

READING IS FREE. Like the wrap, and for a different reason: the wrap is ungated
because that read is how a new device gets the key it would authenticate with,
and the head is ungated because an anchor nobody can watch is an anchor an arc
can move. A monitor, a mirroring arc, or the person's own second device sees the
ciphertext revert without ever being able to read it.

WRITING IS ORDERED. Every write is signed, and a signature is not enough: the
declared chain position must move forward. This is the guard `PUT /history` has
in advisory form — there it compares the exporter's own clock, which is a claim;
here it compares a COUNT, where position n names exactly one tail forever. That
is the difference between a hint this service respects and an ordering it can
enforce.

The last test in this file is G6 whole: axel has lost his phone in Ferizaj, he
has his 24 words and a borrowed laptop, and the bytes he pulls down are real
ones sealed by `pacific-core/src/head.rs`.
"""
from __future__ import annotations

import pytest
from conftest import OCTET, Device, a_head, a_wrap
from app.main import CHAIN_POSITION_MAX, HEAD_MAX_BYTES


def test_a_head_is_stored_signed_and_read_by_anyone(client, device):
    device.register()
    body = a_head(0x11)
    r = device.put_head(41, body)
    assert r.status_code == 200
    assert r.json()["position"] == 41
    assert r.json()["stored"] is True

    # THE BORROWED LAPTOP. No challenge, no signature, no passkey — only the
    # public key, which is all someone restoring from words has at this moment.
    got = client.get(f"/auth/users/{device.pk}/head")
    assert got.status_code == 200
    assert got.content == body
    assert got.headers["x-chain-position"] == "41"
    assert got.headers["cache-control"] == "no-store"
    assert got.headers["x-head-updated-at"].isdigit()


def test_a_head_cannot_go_backwards(client, device):
    """The whole reason the position is there. Two hashes cannot be compared for
    recency; a count can, so a session that has been asleep is refused instead of
    quietly clobbering a newer anchor."""
    device.register()
    device.put_head(41, a_head(0x11))

    r = device.put_head(17, a_head(0x22))
    assert r.status_code == 409
    assert r.json()["error"] == "stale_head"
    assert r.json()["reason"] == "behind"
    assert r.json()["held_position"] == 41
    assert r.json()["offered_position"] == 17

    # Refusing must not have cost the account the anchor it already had.
    still = client.get(f"/auth/users/{device.pk}/head")
    assert still.content == a_head(0x11)
    assert still.headers["x-chain-position"] == "41"

    # And forward still works, so the refusal is an ordering and not a lock.
    assert device.put_head(42, a_head(0x33)).status_code == 200
    assert client.get(f"/auth/users/{device.pk}/head").headers["x-chain-position"] == "42"


def test_one_position_with_two_heads_is_a_fork_and_an_identical_retry_is_not(client, device):
    """Position n names exactly one tail. Two different heads at the same
    position means two writers appended against one predecessor — refused rather
    than picked between, because whoever reads next could not tell which was
    real. A byte-identical re-write is the other thing entirely: a retry whose
    reply was lost, and punishing it for arriving twice would be wrong."""
    device.register()
    device.put_head(41, a_head(0x11))

    r = device.put_head(41, a_head(0x22))
    assert r.status_code == 409
    assert r.json()["reason"] == "fork"
    assert r.json()["held_position"] == 41

    again = device.put_head(41, a_head(0x11))
    assert again.status_code == 200
    assert again.json()["stored"] is False, "a retry is a no-op, not a second write"
    assert client.get(f"/auth/users/{device.pk}/head").content == a_head(0x11)


def test_a_retry_does_not_move_the_timestamp_a_witness_watches(client, device):
    """`updated_at` names the moment the anchor MOVED. If a retry refreshed it, a
    witness would see motion where the chain did not grow — and the value of
    watching would be exactly the value of watching a clock."""
    device.register()
    device.put_head(41, a_head(0x11))
    first = client.get(f"/auth/users/{device.pk}/head/meta").json()["updated_at"]
    device.put_head(41, a_head(0x11))
    assert client.get(f"/auth/users/{device.pk}/head/meta").json()["updated_at"] == first


def test_writing_a_head_needs_a_signature_even_though_reading_it_does_not(client, device):
    device.register()
    unsigned = client.put(f"/auth/users/{device.pk}/head", content=a_head(),
                          headers={**OCTET, "X-Chain-Position": "1"})
    assert unsigned.status_code == 401

    other = Device(client)
    other.register()
    forged = client.put(f"/auth/users/{device.pk}/head", content=a_head(),
                        headers={**other.sign_for(device.pk), **OCTET,
                                 "X-Chain-Position": "1"})
    assert forged.status_code == 401
    assert client.get(f"/auth/users/{device.pk}/head").status_code == 404


def test_a_head_must_declare_a_position_and_it_must_be_a_whole_number(client, device):
    """Mandatory, unlike the history's optional export stamp: nothing has ever
    written a head, so there is no older client to lock out — and a head with no
    position is a head with no compare-and-set, which is the entire mechanism."""
    device.register()
    missing = client.put(f"/auth/users/{device.pk}/head", content=a_head(),
                         headers={**device.headers(), **OCTET})
    assert missing.status_code == 400
    assert missing.json()["error"] == "invalid_request"

    for bad in ("tomorrow", "4.5", "-1", ""):
        r = device.put_head(1, a_head(), **{"X-Chain-Position": bad})
        assert r.status_code == 400, bad
        assert r.json()["error"] == "invalid_request", bad

    # Zero is legal and is how an account anchors an empty chain.
    assert device.put_head(0, a_head()).status_code == 200


def test_a_wrap_in_the_head_slot_is_refused(client, device):
    """Three opaque artefacts now share one account record, so the magic is the
    only thing that can tell a client it sent the wrong file — and now is the
    only moment it can be told."""
    device.register()
    r = device.put_head(1, a_wrap())
    assert r.status_code == 400
    assert r.json()["error"] == "not_a_head"

    short = device.put_head(1, b"PHD1short")
    assert short.status_code == 400
    assert short.json()["error"] == "not_a_head"


def test_a_head_cannot_be_stored_for_a_key_with_no_account(client, device):
    r = device.put_head(1)
    assert r.status_code == 404
    assert r.json()["error"] == "no_wrap"


def test_a_head_larger_than_the_cap_is_refused_before_it_is_read(client, device):
    device.register()
    r = client.put(f"/auth/users/{device.pk}/head",
                   content=b"PHD1" + bytes(HEAD_MAX_BYTES),
                   headers={**device.headers(), **OCTET, "X-Chain-Position": "1"})
    assert r.status_code == 413
    assert r.json()["error"] == "too_large"


def test_an_account_with_no_head_says_so_rather_than_guessing(client, device):
    device.register()
    r = client.get(f"/auth/users/{device.pk}/head")
    assert r.status_code == 404
    assert r.json()["error"] == "no_head"

    meta = client.get(f"/auth/users/{device.pk}/head/meta").json()
    assert meta["exists"] is False
    assert meta["position"] is None
    assert meta["digest"] is None


def test_the_account_record_reports_the_head_beside_the_wrap(client, device):
    device.register()
    assert device.account().json()["head"] is None
    device.put_head(41, a_head(0x11))

    acct = device.account().json()
    assert acct["head"]["position"] == 41
    assert acct["head"]["size"] == len(a_head(0x11))
    assert acct["wrap"] is not None, "the head does not replace the wrap, it sits beside it"


def test_a_witness_watches_the_digest_without_ever_reading_the_head(client, device):
    """The transparency-log shape, for free. A witness holds only the public key.
    It cannot open the blob, and it does not need to: the digest names these
    exact bytes, so "those bytes are back" is a statement it can make from
    yesterday's observation alone."""
    device.register()
    device.put_head(41, a_head(0x11))
    at41 = client.get(f"/auth/users/{device.pk}/head/meta").json()
    assert at41["digest"].startswith("sha256:")
    assert at41["position"] == 41

    device.put_head(42, a_head(0x33))
    at42 = client.get(f"/auth/users/{device.pk}/head/meta").json()
    assert at42["digest"] != at41["digest"], "the anchor moved and a witness can see it"
    assert at42["position"] == 42

    # A rollback ATTEMPT is refused, and the witness sees no motion at all — the
    # refusal is silent to it, which is correct: nothing happened.
    device.put_head(41, a_head(0x11))
    assert client.get(f"/auth/users/{device.pk}/head/meta").json() == at42


def test_a_position_no_chain_could_reach_is_refused_rather_than_anchored(client, device):
    """THE ONE-WRITE BRICK. The position only ever goes up, so a single write at
    an absurd number is not an odd row in a table — it is the end of this
    account's anchor. There is no DELETE route, no reset, and re-registering the
    wrap does not clear it: every honest write afterwards is `behind`, forever.

    The number here is the accident that actually happens — a client that sends
    a CLOCK where a count belongs. It is refused at the door, and the anchor the
    account already had is still there afterwards, which is the whole point of
    refusing early rather than storing and regretting.
    """
    device.register()
    device.put_head(41, a_head(0x11))

    nanoseconds_since_1970 = 1_789_000_000_000_000_000
    r = device.put_head(nanoseconds_since_1970)
    assert r.status_code == 400
    assert r.json()["error"] == "invalid_request"

    held = client.get(f"/auth/users/{device.pk}/head")
    assert held.headers["x-chain-position"] == "41", "the refusal cost the account nothing"
    assert held.content == a_head(0x11)
    assert device.put_head(42, a_head(0x22)).status_code == 200, "and the chain still moves"

    # A bound, not an off-by-one: the ceiling itself is a legal position, and the
    # first number above it is not.
    assert device.put_head(CHAIN_POSITION_MAX, a_head(0x33)).status_code == 200
    over = device.put_head(CHAIN_POSITION_MAX + 1, a_head(0x44))
    assert over.status_code == 400
    assert over.json()["error"] == "invalid_request"


def test_a_u64_position_is_a_refusal_and_never_a_stack_trace(client, device):
    """2**63 to 2**64-1 is a LEGAL value for the u64 the head itself carries, and
    it is exactly the range this service's own column cannot hold: SQLite's
    INTEGER is signed 64-bit, so those positions raise OverflowError on the way
    in. Unhandled, that is a 500 — and the module's docstring says failure is a
    JSON error code, never a stack trace, because the person cannot act on our
    exception and can act on being told what did not work.

    A client is told the same thing for 2**63 as for a trillion and one, which is
    correct: both are numbers no chain arrives at, and neither of them is the
    caller's database problem to understand.
    """
    device.register()
    for u64 in (2**63, 2**63 + 1, 2**64 - 1):
        r = device.put_head(u64, a_head(0x11))
        assert r.status_code == 400, u64
        assert r.json()["error"] == "invalid_request", u64

    # Nothing was written on the way through, so the account is where it started.
    assert client.get(f"/auth/users/{device.pk}/head").status_code == 404
    assert device.put_head(1, a_head(0x11)).status_code == 200


def test_the_service_says_what_position_it_will_accept(client):
    """A client that is about to declare something near the ceiling should be
    able to learn where it is, rather than discovering it by being refused."""
    health = client.get("/auth/health").json()
    assert health["chain_position_max"] == CHAIN_POSITION_MAX


# ─────────────────────────── G6, end to end ───────────────────────────────
#
# Everything below runs against a head sealed by the REAL implementation
# (`pacific-core/src/head.rs`), frozen as hex. The same constant is opened from
# the Rust side by `head::tests::the_frozen_fixture_still_opens`, so if either
# implementation drifts — the domain string, the magic, the nonce length, the
# AEAD's associated data, a CBOR field name — one of the two goes red. A signer
# and a verifier that disagree by one byte fail in a way neither side can see,
# and this is the pair of tests that makes them see it.

# Seed `a5` × 32, through the storage root; head v2, arc "arc.kenjin.cc",
# position 41, tail `7b` × 32, chain_gen 3.
RUST_SEALED_HEAD = bytes.fromhex(
    "50484431ea9cf745c6a793ef06107373e7cb64518274defaa191fe02b41fffe7"
    "f5ad780f25740dea8a2828ed624fdd5f6a49363971ebeaba1830ff3a8218f284"
    "b27c588c3387ecd02caee4fd636c8ac17876f7fd645e178f6d2a6361d2ca2a7e"
    "ffdd4d45d1dc67e9ebe5d7a3b8c019c1b57504996d48baaf069a0eca467e6a"
)
AXELS_SEED = bytes([0xA5]) * 32          # what his 24 words come out as
STORAGE_ROOT_LABEL = b"pacific/storage/root/v1"   # `locator::STORAGE_ROOT_LABEL`
HEAD_DOMAIN = b"pacific/storage/head/seal/v1"     # `head::HEAD_DOMAIN`

try:                                                          # pragma: no cover
    import cbor2
    from cryptography.hazmat.primitives import hashes
    from cryptography.hazmat.primitives.kdf.hkdf import HKDF
    from nacl.bindings import crypto_aead_xchacha20poly1305_ietf_decrypt as _xopen
    _CAN_OPEN = True
except ImportError:                                           # pragma: no cover
    _CAN_OPEN = False


def _kdf(label: bytes, ikm: bytes) -> bytes:
    """`locator.rs`'s one shape: HKDF-SHA256(salt = label, ikm, info = "v1")."""
    return HKDF(algorithm=hashes.SHA256(), length=32, salt=label, info=b"v1").derive(ikm)


def _open_head(blob: bytes, seed: bytes) -> dict:
    """What a borrowed laptop does with the bytes: derive the key from the seed
    and nothing else, then open. Mirrors `head::open` (v2) — the storage root off
    the seed, the head key off the root, XChaCha20-Poly1305 with the domain as
    AAD, CBOR."""
    assert blob[:4] == b"PHD1", "not a sealed head"
    key = _kdf(HEAD_DOMAIN, _kdf(STORAGE_ROOT_LABEL, seed))
    return cbor2.loads(_xopen(blob[28:], HEAD_DOMAIN, blob[4:28], key))


@pytest.mark.skipif(not _CAN_OPEN, reason="needs cbor2 and PyNaCl (the dev extra)")
def test_g6_axel_loses_his_phone_in_ferizaj(client, device):
    """He has the 24 words and a borrowed laptop. He reaches the head, and he can
    tell whether the arc is showing him the whole chain.

    The laptop signs nothing and holds no passkey. Everything it needs is the
    public key — which the words give it — and two numbers that come back from
    this service: a position it can compare, and a tail it can compare against.
    """
    device.register()                                   # the phone, while it existed
    assert device.put_head(41, RUST_SEALED_HEAD).status_code == 200

    # FERIZAJ. A browser with no local state, holding only the identity key.
    got = client.get(f"/auth/users/{device.pk}/head")
    assert got.status_code == 200, "the words door does not pass through a signature"

    head = _open_head(got.content, AXELS_SEED)
    assert head["v"] == 2
    assert head["position"] == 41
    assert head["tail"] == bytes([0x7B]) * 32
    assert head["chain_gen"] == 3, "which chain the anchor is for — v2"
    assert "updated_at" not in head, "v2 carries no timestamp; the arc's column is the witness"

    # THE ARC IS HELD TO THE NUMBER IT DECLARED. It orders writes on a position it
    # cannot decrypt, so the header and the ciphertext are checked against each
    # other — `head::open_declared` is this comparison in the core.
    assert int(got.headers["x-chain-position"]) == head["position"]

    # AND NOW THE QUESTION G6 ACTUALLY ASKS. Suppose the arc serves him a chain
    # that ends at 17. A chain cannot say this about itself: a prefix of a valid
    # chain is a valid chain. The anchor can, and this is the whole comparison.
    walked_position, walked_tail = 17, bytes([0x99]) * 32
    assert walked_position < head["position"], "truncated, and by 24 entries"
    assert head["position"] - walked_position == 24

    # A complete walk is the same comparison with the other answer.
    assert (41, bytes([0x7B]) * 32) == (head["position"], head["tail"])

    # Same position, different tail, is neither truncation nor completeness: it
    # is a fork, and it is a different thing to tell him.
    assert walked_tail != head["tail"]


@pytest.mark.skipif(not _CAN_OPEN, reason="needs cbor2 and PyNaCl (the dev extra)")
def test_the_head_is_not_readable_with_the_wrong_seed(client, device):
    """The other half of the same guarantee: reading the blob is free, and it is
    worth nothing without the words."""
    device.register()
    device.put_head(41, RUST_SEALED_HEAD)
    blob = client.get(f"/auth/users/{device.pk}/head").content
    with pytest.raises(Exception):
        _open_head(blob, bytes([0xA6]) * 32)
