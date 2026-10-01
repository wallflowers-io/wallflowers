"""End to end: a seed, a wrap, an account on an Arc, and a delta between devices.

WHAT THIS IS FOR. The doors had never been walked. Every part of the chain that
begins with a real key -- the account the wrap creates, the signature the Arc
verifies, the channel derived from the seed, the sealed blob on the relay -- was
written and never run against the deployed service. This runs it, against the
real service and the real relay, with nothing stubbed but the authenticator.

RULED 13 SEP 2026, and it is why this file was rewritten. The seed is the root.
A person owns 32 bytes and derives an Ed25519 identity from them; the passkey is
a LOCAL UNLOCK that opens a wrapped copy of the seed on one device and never
reaches the Arc. So there is no WebAuthn in the service, no ceremony, no session
cookie and no credential table -- `/auth/passkey/*`, `/auth/session` and
`/auth/signout` are gone, and every test that drove them went with them. What the
Arc does is issue a nonce and check a signature over it.

THE CHAIN, in the order it is asserted:

   1  a seed, and the identity it derives    32 bytes -> Ed25519, derived here
   2  a wrap registers the account           PUT /auth/users/<pk>, signed
   3  the wrap read is NOT gated             GET /auth/users/<pk>, no signature
   4  a device opens it with its passkey     the seed comes back out
   5  what the Arc holds for the key         GET .../account -> key, Arc, relay
   6  a forged signature is refused          or 2 and 5 prove nothing
   7  a spent challenge cannot be replayed   single use, or there is a window
   8  a signature for another Arc is refused the audience is inside the bytes
   9  another ACCOUNT's real signature too   authentication is not authorization
  10  the history is gated, the wrap is not  the split the public read rests on
  11  pacific-core derives what this does    both ways: it opens what this sealed
  12  a history round trip, and its guard    a stale export loses, and says so
  13  the seed derives the same channel      on a device that never held it
  14  A seals a delta to the relay           real semaphore, real Frame vocabulary
  15  B receives and unseals it              the delta arrives on the other device
  16  a third party on the tag               is handed the same opaque bytes
  17  an address is checked, claimed, held   against a second account

HOW THIS RUN ENDS, because a suite that lies about that is worse than none:
0 every check ran and passed; 1 something was asked and the answer was wrong;
3 nothing failed but a check COULD NOT BE RUN and so was not answered. A skip is
never folded into a pass -- `cannot_run` below is why, and the summary and the
exit code both carry it.

WHAT IS SIMULATED, and it is one thing: the authenticator (see authn.py), which
the SERVICE never sees at all any more. Its PRF secret keys the wrap and nothing
else, and step 4 is what keeps that honest -- if the wrap were not really sealed
under it, the seed would come back out under any key at all.

INDEPENDENT BY CONSTRUCTION, AND THEN CHECKED. Every constant below is retyped
from `pacific-core` rather than imported, and the payload the Arc verifies is
built here by hand -- because a signer and a verifier that agree only because
they share code agree about nothing. Each constant names the Rust that defines
it, and `the_core` puts all four derivations against that Rust through the same
wasm the browser loads, in both directions. Without that step the derivations
here would agree with themselves just as happily if every constant were wrong
together, and the failure would be silent: a wrong tag raises nothing at all.

WHAT IS NOT COVERED HERE: the browser's own ceremony, and keyholder.js deriving
the channel from the seed it holds. That runs in browser.py, against the same
service, so the two can be compared byte for byte.
"""
from __future__ import annotations

import asyncio
import base64
import json
import os
import secrets
import subprocess
import sys
import time

import httpx
import websockets

# The relay's write rule, one Python copy of it, held to the Rust by the vector in
# pacific-wire/wire-spec.v1.json. smoke/ -> web/ -> app/ -> product/, then core/harness.
sys.path.insert(0, str(__import__("pathlib").Path(__file__).resolve().parents[3] / "core" / "harness"))
import address  # noqa: E402
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.hashes import SHA256
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from nacl.bindings import (crypto_aead_xchacha20poly1305_ietf_decrypt,
                           crypto_aead_xchacha20poly1305_ietf_encrypt)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from authn import Authenticator   # noqa: E402

AUTH = os.environ.get("AUTH", "http://127.0.0.1:8021")
RELAY = os.environ.get("RELAY", "ws://localhost:8787")

# Everything sealed for this Arc is bound to its HOST, exactly as the user handle
# carries it, port included (wrap.rs `aad`). In the harness the keyholder proxies
# /auth, so the host a device signs and seals for is the keyholder's, not :8021's.
HOST = AUTH.split("//", 1)[-1].rstrip("/")
# The passkey's RP ID. localhost in the harness; kenjin.cc in production, which
# is the registrable domain rather than an origin.
RP_ID = HOST.split(":")[0]

# ── the core's constants, retyped ────────────────────────────────────────────
# Drift in any of these is SILENT -- two implementations derive different bytes
# and nothing anywhere reports an error -- which is exactly why they are asserted
# from a second implementation rather than imported from the first.
SEED_SALT = b"pacific/identity/seed/v1"            # identity.rs SEED_SALT
SEED_INFO = b"pacific/identity/current/v1"         # identity.rs SEED_INFO_CURRENT
CHANNEL_DOMAIN = b"pacific/account-channel/v0"     # identity.rs ACCOUNT_CHANNEL_DOMAIN
WRAP_DOMAIN = b"pacific/wrap/v1"                   # wrap.rs WRAP_DOMAIN
HISTORY_DOMAIN = b"pacific/history/v1"             # backup.rs HISTORY_DOMAIN
AUTH_DOMAIN = "pacific-auth:v1"                    # identity.rs auth_payload
KEY_PREFIX = "ed25519:"                            # identity.rs IDENTITY_KEY_PREFIX
WRAP_MAGIC = b"PWR1"                               # wrap.rs MAGIC
HISTORY_MAGIC = b"PHS1"                            # backup.rs MAGIC

# HOW THIS FILE REPORTS, in three buckets and not two. `ok` and `bad` are the
# obvious pair; `unrun` is the one that was missing, and its absence was a bug:
# a check that could not be run used to print a line and return, leaving a green
# summary standing over something nobody had asked. Passed, failed and NOT ASKED
# are three different answers, and only the first of them is green.
ok = 0
bad: list[str] = []
unrun: list[str] = []

# What this process exits with. 1 and 3 are both non-zero on purpose: a run that
# could not put its own derivations against the core is not a run that checked
# them, whatever everything else says.
EXIT_OK = 0
EXIT_FAILED = 1
EXIT_INCOMPLETE = 3


def check(label: str, cond: bool, detail: str = "") -> bool:
    global ok
    if cond:
        ok += 1
        print(f"  \033[32m✓\033[0m {label}")
    else:
        bad.append(label)
        print(f"  \033[31m✗ {label}\033[0m{('  ' + detail) if detail else ''}")
    return cond


def bad_sig(r) -> bool:
    """401, and 401 for the right reason.

    A bare status check would pass on `challenge_spent` -- which is what a
    refusal looks like when the harness has fumbled a nonce rather than when the
    service has refused a key. The two are one HTTP code apart and worlds apart
    in what they prove, so the refusals below name the one they mean.
    """
    if r.status_code != 401:
        return False
    try:
        return r.json().get("error") == "bad_signature"
    except Exception:                                         # noqa: BLE001
        return False


def cannot_run(label: str, why: str) -> None:
    """A check this run could not ask. Loud, and never silently green.

    THE RULE: a suite that cannot run one of its own checks must not print a
    passing summary. This is kept apart from `bad` because "we asked and the
    answer was wrong" and "we could not ask" are different news and want
    different fixes -- but both leave by the same door, non-zero.
    """
    unrun.append(label)
    print(f"  \033[33m– {label}: COULD NOT BE RUN\033[0m — {why}")


def summary() -> int:
    """The one place this file decides what it says and what it returns.

    Every exit goes through here, including the early ones, so that a skip can
    never leave by a door that does not report it.
    """
    print()
    if bad:
        tail = f", {len(unrun)} could not be run" if unrun else ""
        print(f"\033[31m{len(bad)} failed\033[0m, {ok} passed{tail}:")
        for b in bad:
            print(f"  · {b}")
        for s in unrun:
            print(f"  – {s}  (not run)")
        return EXIT_FAILED
    if unrun:
        print(f"\033[33m{ok} passed, and {len(unrun)} could not be run — "
              f"THIS IS NOT A GREEN RUN\033[0m:")
        for s in unrun:
            print(f"  – {s}")
        return EXIT_INCOMPLETE
    print(f"\033[32m{ok} passed\033[0m")
    return EXIT_OK


# ── the derivations ──────────────────────────────────────────────────────────
def hkdf(salt: bytes, ikm: bytes, info: bytes, n: int = 32) -> bytes:
    return HKDF(algorithm=SHA256(), length=n, salt=salt, info=info).derive(ikm)


def split(secret: bytes) -> tuple[str, bytes]:
    """`identity::account_channel`, in Python.

    One secret in, two unrelated keys out, domain-separated by the info string
    so the mailbox address cannot be derived from the cipher key or the reverse.
    WebCrypto's HKDF is RFC 5869 and so is the core's, so this is one function
    written three times -- which is the only reason it is worth asserting.
    """
    return (hkdf(CHANNEL_DOMAIN, secret, b"tag").hex(),
            hkdf(CHANNEL_DOMAIN, secret, b"seal"))


def seal(key: bytes, obj) -> str:
    """AES-GCM with the IV prefixed, base64 -- keyholder.js's seal()."""
    iv = os.urandom(12)
    ct = AESGCM(key).encrypt(iv, json.dumps(obj, separators=(",", ":")).encode(), None)
    return base64.b64encode(iv + ct).decode()


def unseal(key: bytes, blob: str):
    raw = base64.b64decode(blob)
    return json.loads(AESGCM(key).decrypt(raw[:12], raw[12:], None))


def seal_wrap(prf: bytes, seed: bytes, host: str) -> bytes:
    """`wrap::seal_wrap`: MAGIC || nonce(24) || XChaCha20-Poly1305(seed).

    THE HOST IS IN THE AAD, which is the one thing this does that a bare AEAD
    would not: a wrap lifted onto an arc the attacker controls will not open
    there, even with the right passkey.
    """
    nonce = os.urandom(24)
    aad = WRAP_DOMAIN + b"|" + host.encode()
    key = hkdf(WRAP_DOMAIN, prf, b"kek")
    ct = crypto_aead_xchacha20poly1305_ietf_encrypt(seed, aad, nonce, key)
    return WRAP_MAGIC + nonce + ct


def open_wrap(prf: bytes, blob: bytes, host: str) -> bytes:
    """`wrap::open_wrap`. Raises if the passkey, the host or the bytes are wrong."""
    if len(blob) < 28 or blob[:4] != WRAP_MAGIC:
        raise ValueError(f"not a wrap: {len(blob)} bytes, magic {blob[:4]!r}")
    key = hkdf(WRAP_DOMAIN, prf, b"kek")
    return crypto_aead_xchacha20poly1305_ietf_decrypt(
        blob[28:], WRAP_DOMAIN + b"|" + host.encode(), blob[4:28], key)


def seal_history(seed: bytes, plain: bytes) -> bytes:
    """`backup::seal_history`. Keyed from the SEED, not the passkey -- which is
    the point of the split: twenty-four words restore a whole account, having
    never had a passkey at all."""
    nonce = os.urandom(24)
    key = hkdf(HISTORY_DOMAIN, seed, b"seal")
    ct = crypto_aead_xchacha20poly1305_ietf_encrypt(plain, HISTORY_DOMAIN, nonce, key)
    return HISTORY_MAGIC + nonce + ct


def open_history(seed: bytes, blob: bytes) -> bytes:
    key = hkdf(HISTORY_DOMAIN, seed, b"seal")
    return crypto_aead_xchacha20poly1305_ietf_decrypt(
        blob[28:], HISTORY_DOMAIN, blob[4:28], key)


class Device:
    """A device holding a seed: the identity it derives, and how it proves it.

    `identity::keys_from_seed` in Python. The private scalar is HKDF over the
    seed, so the same seed always yields the same key -- which is what makes a
    words restore land on the same account and not a stranger.
    """

    def __init__(self, seed: bytes):
        self.seed = seed
        self.key = Ed25519PrivateKey.from_private_bytes(
            hkdf(SEED_SALT, seed, SEED_INFO))
        self.pk = self.key.public_key().public_bytes_raw().hex()

    def payload(self, audience: str, nonce: str) -> bytes:
        """`identity::auth_payload`. The audience is inside the signed bytes, so
        a signature captured by one Arc cannot be replayed at another; the nonce
        is the Arc's and single use, so there is no replay window at all."""
        return f"{AUTH_DOMAIN}\n{audience}\n{KEY_PREFIX}{self.pk}\n{nonce}".encode()

    def sign(self, audience: str, nonce: str) -> str:
        return self.key.sign(self.payload(audience, nonce)).hex()

    def headers(self, c: httpx.Client, *, audience: str | None = None) -> dict:
        """A fresh challenge, signed. One per request: the Arc spends the nonce
        BEFORE it checks the signature, so a nonce is worth exactly one attempt
        whether or not that attempt succeeds."""
        ch = c.get(AUTH + "/auth/challenge").json()
        return {"X-Pacific-Challenge": ch["nonce"],
                "X-Pacific-Signature": self.sign(audience or ch["audience"], ch["nonce"])}


# ── the relay ───────────────────────────────────────────────────────────────
async def subscribe(ws, tag: str) -> None:
    await ws.send(json.dumps({"t": "sub", "tags": [tag], "since": 0, "v": 1}))
    while True:                                   # drain replay up to end-of-stored
        f = json.loads(await asyncio.wait_for(ws.recv(), timeout=5))
        if f["t"] == "eose":
            return


async def relay_leg(tag: str, key: bytes, delta: dict,
                    marker: str) -> tuple[bool, bool, str]:
    """A publishes, B receives, and a third socket on the tag learns nothing.

    WHAT THE THIRD SOCKET IS FOR, since it used to be for nothing. It decrypted
    eve's blob under a freshly random key and called the exception a pass --
    which asserts that AES-GCM rejects the wrong key, a property of the cipher
    that no run of this suite could ever fail. Replaced, not removed, because
    the socket is testing a real thing once it is pointed at the relay instead:

      · eve is subscribed to the tag with no key at all and IS SERVED, and is
        served EXACTLY the bytes the laptop was. Privacy here rests on the seal
        and on nothing the relay does, so a relay that started filtering by
        identity, re-sealing, or summarising would break this and should.
      · and none of the delta is in the clear in what it forwarded. A relay that
        ever passed a plaintext op -- unsealed by a client, or unwrapped by the
        relay to index it -- breaks this and nothing else here would notice.

    Returns (arrived, third_socket_got_the_same_bytes, leak) where `leak` is
    empty when the forwarded bytes carry no cleartext of the delta.
    """
    # The channel's first half is the relay-address SEED (18 Sep 2026): the tag on
    # the wire is its public key, and a pub must be signed by it or it is refused.
    addr = address.Address(bytes.fromhex(tag))
    async with websockets.connect(RELAY) as a, \
               websockets.connect(RELAY) as b, \
               websockets.connect(RELAY) as eve:
        await subscribe(b, addr.tag_hex)
        await subscribe(eve, addr.tag_hex)
        blob = seal(key, delta)
        await a.send(json.dumps({"t": "pub", "tag": addr.tag_hex, "blob": blob,
                                 "sig": addr.sign_pub(blob)}))

        async def first_msg(ws):
            while True:
                f = json.loads(await asyncio.wait_for(ws.recv(), timeout=5))
                if f["t"] == "msg":
                    return f

        got = await first_msg(b)
        seen = await first_msg(eve)

    arrived = unseal(key, got["blob"]) == delta
    same = seen["blob"] == got["blob"]
    raw = base64.b64decode(seen["blob"])
    plain = json.dumps(delta, separators=(",", ":")).encode()
    # The marker is a fresh 16 hex digits, so "it happened to be in there" is
    # not an explanation any run of this will ever need.
    leak = ""
    if plain in raw:
        leak = "the delta is in the relay's copy verbatim"
    elif marker.encode() in raw:
        leak = f"the conversation id {marker} is in the relay's copy in the clear"
    return arrived, same, leak


# ── the run ─────────────────────────────────────────────────────────────────
def the_door(c: httpx.Client, phone: Device, prf: bytes) -> bytes | None:
    """Registration, the public read, and every way the Arc says no.

    Four of those refusals are the whole authentication model stated as failures
    -- a forged signature, a spent nonce, a signature minted for another Arc, and
    none at all. Without them the successes above prove only that the service
    answers 200 to something.

    THE FIFTH IS THE OTHER HALF, and it was missing. Those four all ask whether
    a signature is REAL. This one is real -- eve holds her own key, signed this
    Arc's own fresh nonce, and the identical header set opens her own door two
    lines above -- and asks whether it is HERS TO USE HERE. Authentication and
    authorization are separate mechanisms in this service and only one of them
    was under test: `auth.verify` is handed the key from the ADDRESS, so a
    verifier "fixed" to take it from the header or from inside the payload would
    pass every other check in this file while letting any account read and
    overwrite any other account's wrap. Nothing else here would see it.

    Returns the wrap the Arc served back, or None if there is no account -- in
    which case nothing after this can mean anything.
    """
    print("the door")
    ch = c.get(AUTH + "/auth/challenge")
    body = ch.json() if ch.status_code == 200 else {}
    check("the Arc issues a challenge", bool(body.get("nonce")), ch.text[:200])
    check("and names the audience a device must sign for",
          body.get("audience") == HOST, f"{body.get('audience')} vs {HOST}")

    wrap = seal_wrap(prf, phone.seed, HOST)
    r = c.put(AUTH + "/auth/users/" + phone.pk, content=wrap,
              headers={**phone.headers(c), "Content-Type": "application/octet-stream"})
    made = r.json() if r.status_code == 200 else {}
    if not check("a wrap registers the account", made.get("created") is True,
                 f"HTTP {r.status_code} {r.text[:200]}"):
        return None

    # THE READ THAT CANNOT BE GATED. It is how a new device obtains the key it
    # would otherwise have to authenticate with, so it is served to anyone who
    # asks -- and asserting that here is the point, not an oversight.
    with httpx.Client(timeout=10) as stranger:
        g = stranger.get(AUTH + "/auth/users/" + phone.pk)
    check("the wrap is served to whoever asks for it", g.status_code == 200,
          f"HTTP {g.status_code}")
    check("byte for byte", g.content == wrap, f"{len(g.content)} vs {len(wrap)} bytes")

    h = c.get(AUTH + "/auth/users/" + phone.pk + "/account", headers=phone.headers(c))
    acc = h.json() if h.status_code == 200 else {}
    check("the account is served against a signature", h.status_code == 200, h.text[:200])
    check("named by the key itself", acc.get("identity_key") == KEY_PREFIX + phone.pk,
          str(acc.get("identity_key")))
    arc = acc.get("arc") or {}
    check("and it records the Arc hosting it", bool(arc.get("arc_url")),
          "no Arc recorded — is the Arc up on the address the service was given?")
    # The distinction this exists for: a device dials the RELAY, which the Arc
    # advertises on a path, not the Arc's own https address.
    check("with the relay address a device can actually dial",
          (arc.get("arc_relay_url") or "").startswith(("ws://", "wss://")), str(arc))
    check("which is not the Arc's own address",
          arc.get("arc_relay_url") != arc.get("arc_url"), str(arc))

    # ONE WRAP PER ACCOUNT: writing a second is enrolling another passkey, not
    # making a second account. The successor to the old "returning user" test.
    again = c.put(AUTH + "/auth/users/" + phone.pk, content=wrap,
                  headers={**phone.headers(c), "Content-Type": "application/octet-stream"})
    check("a second wrap is not a second account",
          again.status_code == 200 and again.json().get("created") is False,
          f"HTTP {again.status_code} {again.text[:120]}")

    # ---- and the ways it is refused -----------------------------------------
    hdr = phone.headers(c)
    forged = bytearray(bytes.fromhex(hdr["X-Pacific-Signature"]))
    forged[-1] ^= 0xFF                                        # one bit of it
    r = c.get(AUTH + "/auth/users/" + phone.pk + "/account",
              headers={**hdr, "X-Pacific-Signature": bytes(forged).hex()})
    check("a forged signature is refused", r.status_code == 401,
          f"HTTP {r.status_code} — verification is not doing anything")

    spent = phone.headers(c)
    c.get(AUTH + "/auth/users/" + phone.pk + "/account", headers=spent)
    r = c.get(AUTH + "/auth/users/" + phone.pk + "/account", headers=spent)
    check("a spent challenge cannot be used twice",
          r.status_code == 401 and r.json().get("error") == "challenge_spent",
          f"HTTP {r.status_code} {r.text[:120]}")

    r = c.get(AUTH + "/auth/users/" + phone.pk + "/account",
              headers=phone.headers(c, audience="evil.example"))
    check("a signature minted for another Arc is refused", r.status_code == 401,
          f"HTTP {r.status_code} — the audience is not inside the signed bytes")

    r = c.get(AUTH + "/auth/users/" + phone.pk + "/account")
    check("and no signature at all is refused", r.status_code == 401,
          f"HTTP {r.status_code}")

    # ---- and the one that is about authorization, not authentication --------
    # A second, ENTIRELY LEGITIMATE account. Everything eve sends below would be
    # accepted at her own address; the control immediately after registering her
    # is what makes that a statement and not an assumption.
    eve = Device(secrets.token_bytes(32))
    c.put(AUTH + "/auth/users/" + eve.pk,
          content=seal_wrap(os.urandom(32), eve.seed, HOST),
          headers={**eve.headers(c), "Content-Type": "application/octet-stream"})
    mine = c.get(AUTH + "/auth/users/" + eve.pk + "/account", headers=eve.headers(c))
    check("a second account's signature opens its own door", mine.status_code == 200,
          f"HTTP {mine.status_code} {mine.text[:160]} — without this the refusals "
          "below could be refusing a signature that was simply invalid")

    r = c.get(AUTH + "/auth/users/" + phone.pk + "/account", headers=eve.headers(c))
    check("but a valid signature from another account is refused", bad_sig(r),
          f"HTTP {r.status_code} {r.text[:120]} — any account can read any other "
          "account's")

    r = c.get(AUTH + "/auth/users/" + phone.pk + "/history", headers=eve.headers(c))
    check("it cannot reach another account's history either", bad_sig(r),
          f"HTTP {r.status_code} {r.text[:120]} — the gate on the history is not "
          "checking WHOSE")

    # THE ONE THAT COSTS. A wrap PUT for a key you do not hold is not a read of
    # someone's account, it is the end of it: the wrap that opens on their
    # passkey replaced by one that opens on yours, on an address they cannot
    # take back, with no second factor anywhere in this service to stop you.
    r = c.put(AUTH + "/auth/users/" + phone.pk,
              content=seal_wrap(os.urandom(32), eve.seed, HOST),
              headers={**eve.headers(c), "Content-Type": "application/octet-stream"})
    check("and above all it cannot overwrite another account's wrap", bad_sig(r),
          f"HTTP {r.status_code} {r.text[:120]} — an account could be taken over "
          "by anyone holding its public key")
    with httpx.Client(timeout=10) as stranger:
        still = stranger.get(AUTH + "/auth/users/" + phone.pk)
    check("the wrap the Arc serves is still the one the phone stored",
          still.status_code == 200 and still.content == wrap,
          f"HTTP {still.status_code}, {len(still.content)} bytes")

    # The devious shape of the same thing: the signed bytes name the VICTIM's
    # key, so the payload is exactly the one this route wants -- only the key
    # that signed it is wrong. This is what would get through a verifier that
    # read the identity out of the payload it was handed instead of out of the
    # address it was asked about, and it is indistinguishable from the line
    # above at the HTTP layer, which is why both are here.
    chal = c.get(AUTH + "/auth/challenge").json()
    r = c.get(AUTH + "/auth/users/" + phone.pk + "/account",
              headers={"X-Pacific-Challenge": chal["nonce"],
                       "X-Pacific-Signature": eve.key.sign(
                           phone.payload(chal["audience"], chal["nonce"])).hex()})
    check("even when the signed bytes name the account being asked about",
          bad_sig(r),
          f"HTTP {r.status_code} {r.text[:120]} — the key is being taken from the "
          "payload, not from the address")

    # THE SPLIT. The wrap is public because it carries no history; the history is
    # a separate blob under a separate key behind a signature. If this ever
    # answers 200, the public read has stopped being safe.
    r = c.get(AUTH + "/auth/users/" + phone.pk + "/history")
    check("the history is NOT served the way the wrap is", r.status_code == 401,
          f"HTTP {r.status_code} — the wrap read rests on this being gated")

    r = c.put(AUTH + "/auth/users/" + phone.pk, content=b"NOPE" + os.urandom(72),
              headers={**phone.headers(c), "Content-Type": "application/octet-stream"})
    check("something that is not a wrap is refused now, not at the restore",
          r.status_code == 400 and r.json().get("error") == "not_a_wrap",
          f"HTTP {r.status_code} {r.text[:120]}")

    return g.content


def the_core(phone: Device, prf: bytes) -> None:
    """Put every derivation in this file against the Rust that defines it.

    THIS IS THE CHECK THAT MAKES THE REST MEAN ANYTHING. Everything above is
    Python talking to a service; the three derivations below are Python talking
    to itself, and they would agree just as happily if all the constants were
    wrong together. The failure that costs is silent -- a wrong tag raises
    nothing, it puts two devices on mailboxes where neither hears the other --
    so the Python is put against `pacific-core`, through the same wasm module the
    browser loads. Both directions: what the core derives, and the core opening
    what this file sealed.

    SKIPPED, LOUDLY, if node or the staged wasm is absent -- and a skip here is
    carried all the way out to the exit code. It used to print a line and return,
    which left the run printing a green summary having never asked the one
    question this function exists for: a tree without the wasm is a tree where
    this cannot be answered, not one where the answer is yes. `cannot_run` is
    what keeps those two apart. The wasm is built by app/web/build-wasm.sh.
    """
    print("\nthe core")
    script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "core-vectors.mjs")
    wasm = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                        "docs", "core", "core_wasm_bg.wasm")
    if not os.path.exists(wasm):
        cannot_run("the core cross-check",
                   f"no {wasm} — run app/web/build-wasm.sh")
        return

    # No history pair: the core's history key and reader went with the escrow
    # (a58798c). `the_history` below still checks the AUTH SERVICE's history
    # route, which seals with this file's own `seal_history` and needs no core.
    args = ["node", script, phone.seed.hex(), prf.hex(), HOST,
            seal_wrap(prf, phone.seed, HOST).hex()]
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=120)
    except (FileNotFoundError, subprocess.TimeoutExpired) as e:
        cannot_run("the core cross-check",
                   f"{type(e).__name__} running node — {e}")
        return
    if r.returncode != 0:
        check("the core stands up under node", False, r.stderr.strip()[:300])
        return

    core = json.loads(r.stdout)
    tag, seal_key = split(phone.seed)
    check("the core derives the same identity from the seed",
          core["identity_key"] == phone.pk, f"{core['identity_key'][:16]} vs {phone.pk[:16]}")
    check("and the same account channel",
          core["account_channel"] == tag + seal_key.hex(), core["account_channel"][:32])
    check("and the same wrap key",
          core["wrap_key"] == hkdf(WRAP_DOMAIN, prf, b"kek").hex(), core["wrap_key"][:32])
    check("the core opens a wrap this file sealed",
          core["opened_wrap"] == phone.seed.hex(), core["opened_wrap"][:80])


def the_history(c: httpx.Client, phone: Device) -> None:
    print("\nthe history")
    plain = b"the archive, as it was" + os.urandom(64)
    first = seal_history(phone.seed, plain)
    at = int(time.time() * 1000)

    def put(blob: bytes, exported_at: int | None) -> httpx.Response:
        h = {**phone.headers(c), "Content-Type": "application/octet-stream"}
        if exported_at is not None:
            h["X-History-Exported-At"] = str(exported_at)
        return c.put(AUTH + "/auth/users/" + phone.pk + "/history", content=blob, headers=h)

    r = put(first, at)
    if not check("a sealed history is stored", r.status_code == 200, r.text[:200]):
        return

    g = c.get(AUTH + "/auth/users/" + phone.pk + "/history", headers=phone.headers(c))
    check("and released against a signature", g.status_code == 200, g.text[:200])
    check("byte for byte", g.content == first, f"{len(g.content)} bytes")
    # The seed alone opens it -- no passkey anywhere in this line, which is what
    # makes twenty-four words enough to restore a whole account.
    try:
        back = open_history(phone.seed, g.content)
    except Exception as e:                                    # noqa: BLE001
        back = f"{type(e).__name__}: {e}".encode()
    check("and opens under a key derived from the seed alone", back == plain,
          back[:80].hex())

    m = c.get(AUTH + "/auth/users/" + phone.pk + "/history/meta",
              headers=phone.headers(c)).json()
    check("the Arc says what it is holding",
          m.get("exists") is True and m.get("size") == len(first)
          and m.get("exported_at") == at, json.dumps(m)[:200])

    # A phone that wakes with a week-old view must not overwrite a laptop's
    # current one. Both stamps come back, so the loser can tell how far behind.
    r = put(seal_history(phone.seed, b"a week-old view"), at - 604_800_000)
    body = r.json() if r.status_code == 409 else {}
    check("an older export loses to the one already stored",
          r.status_code == 409 and body.get("error") == "stale_history",
          f"HTTP {r.status_code} {r.text[:160]}")
    check("and is told how far behind it is",
          body.get("held_exported_at") == at
          and body.get("offered_exported_at") == at - 604_800_000, json.dumps(body)[:160])

    second = seal_history(phone.seed, b"the archive, one delta later" + os.urandom(64))
    r = put(second, at + 1000)
    check("a newer one is taken", r.status_code == 200, r.text[:160])
    g = c.get(AUTH + "/auth/users/" + phone.pk + "/history?which=previous",
              headers=phone.headers(c))
    check("and the one it replaced is still there to fall back to",
          g.status_code == 200 and g.content == first, f"HTTP {g.status_code}")

    r = c.put(AUTH + "/auth/users/" + phone.pk + "/history", content=b"NOPE" + os.urandom(40),
              headers={**phone.headers(c), "Content-Type": "application/octet-stream"})
    check("something that is not a history is refused",
          r.status_code == 400 and r.json().get("error") == "not_a_history",
          f"HTTP {r.status_code} {r.text[:120]}")


def the_address(c: httpx.Client, phone: Device, slug: str) -> None:
    print("\nan address")
    free = c.get(AUTH + "/auth/site/check", params={"slug": slug}).json()
    check("checking an address needs no account",
          free.get("available") is True, json.dumps(free))
    taken = c.get(AUTH + "/auth/site/check", params={"slug": "signup"}).json()
    check("a path the site already serves is not on offer",
          taken.get("available") is False and taken.get("reason") == "reserved",
          json.dumps(taken))
    junk = c.get(AUTH + "/auth/site/check", params={"slug": "A b-"}).json()
    check("nor is one nobody could type", junk.get("reason") == "shape", json.dumps(junk))

    r = c.post(AUTH + "/auth/users/" + phone.pk + "/sites", json={"slug": slug})
    check("claiming one needs the key", r.status_code == 401, f"HTTP {r.status_code}")

    r = c.post(AUTH + "/auth/users/" + phone.pk + "/sites", json={"slug": slug},
               headers=phone.headers(c))
    body = r.json() if r.status_code == 200 else {}
    check("the account claims it", body.get("claimed") is True, r.text[:200])
    check("and holds it", slug in [s["slug"] for s in body.get("sites", [])],
          json.dumps(body)[:200])

    after = c.get(AUTH + "/auth/site/check", params={"slug": slug}).json()
    check("it stops being on offer", after.get("reason") == "taken", json.dumps(after))

    # A SECOND ACCOUNT, which is the only version of this test that means
    # anything: claiming your own slug again is idempotent and proves nothing.
    other = Device(secrets.token_bytes(32))
    c.put(AUTH + "/auth/users/" + other.pk,
          content=seal_wrap(os.urandom(32), other.seed, HOST),
          headers={**other.headers(c), "Content-Type": "application/octet-stream"})
    r = c.post(AUTH + "/auth/users/" + other.pk + "/sites", json={"slug": slug},
               headers=other.headers(c))
    check("and nobody else can take it", r.status_code == 409, f"HTTP {r.status_code}")


def main() -> int:
    print("\n\033[1mPACIFIC · end to end\033[0m")
    print(f"  auth {AUTH}    relay {RELAY}\n  host {HOST}\n")

    # The passkey. It never reaches the Arc: all it does is hold the key that
    # opens the wrap, which is the whole of what a passkey is here.
    phone = Device(secrets.token_bytes(32))
    authenticator = Authenticator()
    cred_id = authenticator.enrol(phone.pk, HOST, RP_ID)
    prf = authenticator.prf(cred_id, WRAP_DOMAIN)

    with httpx.Client(follow_redirects=False, timeout=10) as c:
        served = the_door(c, phone, prf)
        if served is None:
            print("\n\033[31mno account — nothing after the door can mean anything\033[0m")
            return summary()
        the_core(phone, prf)
        the_history(c, phone)
        the_address(c, phone, "smoke-" + secrets.token_hex(4))

        # ---- a device that has never held the seed --------------------------
        # THE WHOLE MULTI-DEVICE CLAIM, and the reason the wrap read is public:
        # the laptop has the passkey and the public key, nothing else. It pulls
        # the wrap off the Arc, opens it, and is the same account.
        print("\ntwo devices")
        try:
            recovered = open_wrap(prf, served, HOST)
            why = ""
        except Exception as e:                                # noqa: BLE001
            recovered, why = b"", f"{type(e).__name__}: {e}"
        if not check("the laptop opens the wrap it pulled off the Arc",
                     recovered == phone.seed, why):
            return summary()
        laptop = Device(recovered)
        check("and derives the key the wrap was stored under", laptop.pk == phone.pk,
              f"{laptop.pk[:16]} vs {phone.pk[:16]}")

        # A wrap is bound to the host that serves it, so one lifted onto an arc
        # the attacker controls does not open there even with the right passkey.
        try:
            open_wrap(prf, served, "evil.example")
            check("and not on an Arc that merely holds a copy", False,
                  "the wrap opened against the wrong host — the AAD is not binding")
        except Exception:                                     # noqa: BLE001
            check("and not on an Arc that merely holds a copy", True)

    tag_a, key_a = split(phone.seed)
    tag_b, key_b = split(laptop.seed)
    check("both devices derive the same channel", tag_a == tag_b and key_a == key_b,
          f"{tag_a[:16]} vs {tag_b[:16]}")
    check("and the tag is not the seal key", tag_a != key_a.hex())
    print(f"    tag {tag_a[:24]}…")

    # ---- a delta crosses ----------------------------------------------------
    print("\nthe relay")
    marker = "c-e2e-" + secrets.token_hex(8)
    delta = {"a": "phone", "seq": 1, "at": {"phone": 1},
             "op": {"t": "read", "conv": marker}}
    try:
        arrived, same, leak = asyncio.run(relay_leg(tag_a, key_a, delta, marker))
        check("a delta authored on the phone reaches the laptop", arrived)
        check("a third socket on the tag is handed the very same bytes", same,
              "the relay did not forward what was published, unchanged")
        check("and none of the delta is in the clear in them", not leak, leak)
    except Exception as e:                                   # noqa: BLE001
        check("the relay leg runs", False, f"{type(e).__name__}: {e}")

    return summary()


if __name__ == "__main__":
    raise SystemExit(main())
