"""A software authenticator: a real P-256 key, and a real hmac-secret behind it.

WHAT A PASSKEY IS HERE, and it is the reason this file is a third of what it was.
RULED 13 Sep 2026: the seed is the root, and the passkey is a LOCAL UNLOCK that
opens a wrapped copy of it on one device. It never reaches the Arc. The service
has no WebAuthn endpoints, verifies no assertions and stores no credentials, so
everything that existed here to be checked by py_webauthn -- the COSE encoding,
the attestation object, the rpIdHash and flags, the signature over
authData||SHA256(clientDataJSON) -- was checked by nobody and is gone.

WHAT IS LEFT is the part the system still depends on:

  · The credential's hmac-secret (`CredRandom` in CTAP2), from which the WebAuthn
    `prf` extension is computed. It never leaves a real authenticator and no
    server ever sees it -- and it is the key that opens the wrap. Here it is
    HMAC(seed, credential_id): stable, and shared by construction between two
    authenticators built from one seed, which is what a credential synced through
    iCloud Keychain is.

  · The key material itself, because the browser leg hands this exact credential
    to `_authn-shim.js` and the browser then does the ceremony for real. The shim
    implements `navigator.credentials.get` in the page -- a headless WKWebView
    cannot reach the virtual authenticators Chrome and Safari expose to WebDriver
    -- so the ceremony is genuine and only the hardware is not.

WHY THE PRF IS STILL WORTH MODELLING EXACTLY. `prf` is defined in terms of a
fixed prefix hashed with the caller's salt before it reaches CTAP2's hmac-secret
(WebAuthn L3 §10.1.4). Getting that wrong gives a secret that is self-consistent
and wrong, so the wrap this process seals would not open in the browser -- and a
wrap that does not open is indistinguishable from a wrap that was never stored.

WHAT IS NOT MODELLED, deliberately: user presence and verification. That is the
part a person does, and the part a virtual authenticator cannot have either.
"""
from __future__ import annotations

import hashlib
import hmac
import secrets
from base64 import urlsafe_b64decode, urlsafe_b64encode
from dataclasses import dataclass, field

from cryptography.hazmat.primitives.asymmetric import ec

# The salt-to-HMAC mapping the `prf` extension is defined in terms of
# (WebAuthn L3 §10.1.4): the browser hashes a fixed prefix with the caller's
# salt before handing it to CTAP2's hmac-secret, so a site cannot choose the
# raw HMAC input. Getting this exactly right is what makes a value derived here
# equal to one a real authenticator would produce for the same salt.
PRF_PREFIX = b"WebAuthn PRF\x00"


def b64u(raw: bytes) -> str:
    return urlsafe_b64encode(raw).rstrip(b"=").decode()


def unb64u(s: str) -> bytes:
    s = s.replace("-", "+").replace("_", "/")
    return urlsafe_b64decode(s.replace("+", "-").replace("/", "_") + "=" * (-len(s) % 4))


@dataclass
class Credential:
    cred_id: bytes
    key: ec.EllipticCurvePrivateKey
    rp_id: str
    # THE HANDLE CARRIES THE ACCOUNT: `public key ‖ arc host`. Those are exactly
    # the two things a fresh device cannot derive for itself -- the address IS
    # the public key, and the public key is downstream of the seed it is
    # fetching. `pacific-account.js` createPasskey builds the same 32+n bytes.
    user_handle: bytes


@dataclass
class Authenticator:
    """One authenticator. Two built from the same seed hold the same secrets --
    which is what "the same passkey, synced to another device" means."""

    seed: bytes = field(default_factory=lambda: secrets.token_bytes(32))
    creds: dict[bytes, Credential] = field(default_factory=dict)

    # ---- the per-credential secret behind the prf extension -----------------
    def _cred_random(self, cred_id: bytes) -> bytes:
        return hmac.new(self.seed, b"cred-random|" + cred_id, hashlib.sha256).digest()

    def prf(self, cred_id: bytes, salt: bytes) -> bytes:
        """What `getClientExtensionResults().prf.results.first` would hold."""
        h = hashlib.sha256(PRF_PREFIX + salt).digest()
        return hmac.new(self._cred_random(cred_id), h, hashlib.sha256).digest()

    # ---- the ceremony that is left ------------------------------------------
    def enrol(self, identity_pk: str, host: str, rp_id: str) -> bytes:
        """Mint a credential for an account, and return its id.

        THE OPTIONS ARE THE CLIENT'S NOW. There is no
        `/auth/passkey/register/options` to fetch them from: the arc is not part
        of a passkey ceremony at all, so the RP ID, the handle and the challenge
        are built by the device -- which is what `pacific-account.js`
        createPasskey does, and this is that call's other half.

        `rp_id` is the REGISTRABLE DOMAIN and not an origin: a credential is
        bound to it for life, so `kenjin.cc` covers every subdomain and one made
        under `localhost` is worthless anywhere else -- which is right for a
        throwaway smoke database and would be a serious mistake near a real one.
        """
        cred_id = secrets.token_bytes(32)
        self.creds[cred_id] = Credential(
            cred_id=cred_id,
            key=ec.generate_private_key(ec.SECP256R1()),
            rp_id=rp_id,
            user_handle=bytes.fromhex(identity_pk) + host.encode(),
        )
        return cred_id

    def clone(self) -> "Authenticator":
        """The same passkey on another device.

        A synced credential is the same credential id, the same private key and
        the same CredRandom in a second authenticator -- not a copy of a device.
        This is the only thing in the harness that models iCloud Keychain.

        IT IS NO LONGER WHAT MAKES TWO DEVICES MEET. The account channel used to
        be derived from this PRF, on the reasoning that a synced passkey carries
        it -- and whether a platform does that is up to the platform. Where it
        does not, two devices derived different channels and never met, silently.
        The channel comes from the seed now (`identity::account_channel`), and
        what a synced passkey buys is only that the wrap opens on both.
        """
        other = Authenticator(seed=self.seed)
        other.creds = {cid: Credential(c.cred_id, c.key, c.rp_id, c.user_handle)
                       for cid, c in self.creds.items()}
        return other
