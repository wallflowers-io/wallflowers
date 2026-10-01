"""The browser leg: keyholder.js opening the same account, in a real browser.

e2e.py proves the auth service and the relay against an independent
implementation. It cannot prove what actually ships. This can:

   1  mint a seed and a passkey here, and register the account for real
   2  hand the SAME credential to the browser  (what iCloud Keychain does)
   3  the browser runs keyholder.js's account.signin, unmodified: it fetches the
      wrap off the Arc, opens it with the passkey, and has the seed
   4  it derives its channel from that seed, dials the relay and commits a delta
   5  this process, subscribed to the tag IT derived, reads that delta

THE BROWSER IS NEVER TOLD THE SEED. That is the change worth stating. It used to
be handed a passkey and asked to derive the channel from the passkey's PRF; the
channel comes from the SEED now (`identity::account_channel`), because whether a
platform carries a credential's PRF to a person's next device is up to the
platform, and where it does not two devices derived different channels and never
met. So the browser has to go and get the seed the way a real second device does
-- GET /auth/users/<pk>, then open the wrap under the passkey it holds -- and
arrival on the tag is the proof that it did.

THE ASSERTION IS THE ARRIVAL. There is no result channel from the page and none
is needed. If the browser derived a different tag, its delta lands on a different
mailbox and nothing is received -- silence is the failure. Arrival proves, in one
observation, that both sides agree on the wrap envelope and its AAD, the PRF salt,
the seed-to-channel derivation, the tag encoding, the AES-GCM framing, the JSON,
and the relay's frame vocabulary. Any one of those drifting apart is two devices
that never see each other, which is the failure this exists to catch.

WHAT IS SIMULATED: the authenticator, in _authn-shim.js, and nothing else. A
headless WKWebView cannot reach the virtual authenticators Chrome and Safari
expose to WebDriver, so the shim is that. It implements
`navigator.credentials.get` only -- which is exactly right now, because the
browser's job here is to SIGN IN to an account this process already made.
"""
from __future__ import annotations

import asyncio
import json
import os
import secrets
import subprocess
import sys

import httpx
import websockets

# smoke/ -> web/ -> app/ -> product/, then core/harness: the one Python copy of the
# relay's address rule, held to the Rust by the vector in the wire spec.
sys.path.insert(0, str(__import__("pathlib").Path(__file__).resolve().parents[3] / "core" / "harness"))
import address  # noqa: E402
from cryptography.hazmat.primitives import serialization

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

# The browser reaches /auth through the KEYHOLDER'S OWN ORIGIN, which proxies it
# (keyholder/serve.py). That origin is what `location.host` gives keyholder.js,
# and the host is inside the wrap's AAD and inside the bytes a device signs -- so
# this process has to seal and sign for the same one or nothing opens. Set before
# e2e is imported, because that is where it is read.
os.environ.setdefault("AUTH", "http://localhost:8103")

from authn import Authenticator, b64u                                   # noqa: E402
from e2e import (AUTH, HOST, RP_ID, WRAP_DOMAIN, Device, bad, check,    # noqa: E402
                 seal_wrap, split, unseal)

HERE = os.path.dirname(os.path.abspath(__file__))
WEB = os.path.dirname(HERE)
CRED_FILE = os.path.join(WEB, "keyholder", "_smoke-cred.json")

RELAY = os.environ.get("RELAY", "ws://localhost:8787")
PAGE = os.environ.get("PAGE", "http://localhost:8100/_smoke.html")
# The same page on disk, read to check it is not stale before waiting 45s on it.
# Overridable alongside PAGE so a different driver page can be pointed at both.
PAGE_FILE = os.environ.get("PAGE_FILE", os.path.join(WEB, "docs", "_smoke.html"))
SHOT = os.environ.get("SHOT", "")


def hand_the_passkey_to_the_browser(auth: Authenticator, cred_id: bytes) -> None:
    """Everything the shim needs to be the same authenticator this one is.

    The private key travels because a synced passkey IS the same private key in
    two places -- that is what synchronisation means, and modelling it any other
    way would test something Pacific does not rely on. The SEED does not travel:
    the browser has to earn that from the Arc, which is the point of the leg.

    It is written under the keyholder's own directory, with a leading underscore,
    and removed after the run: a credential file left behind on a served origin
    is a credential file somebody's next browser session can fetch.
    """
    cred = auth.creds[cred_id]
    with open(CRED_FILE, "w") as fh:
        json.dump({
            "_smoke": "A TEST CREDENTIAL for _authn-shim.js. Deleted when the run "
                      "ends. Nothing but the smoke frame reads it, and index.html "
                      "must never load the shim that does.",
            "cred_id": b64u(cred_id),
            "rp_id": cred.rp_id,
            "user_handle": b64u(cred.user_handle),
            "cred_random": b64u(auth._cred_random(cred_id)),
            "pkcs8": b64u(cred.key.private_bytes(
                encoding=serialization.Encoding.DER,
                format=serialization.PrivateFormat.PKCS8,
                encryption_algorithm=serialization.NoEncryption())),
        }, fh, indent=2)


def the_page_signs_in() -> bool:
    """Does the driver page still open an account before asking for a channel?

    THE PAGE IS NOT THIS TRACK'S FILE, and it is the half of this leg that has
    to change with the model: `account.link` now requires a seed already on the
    device, so a page that calls it first gets "no account on this device" and
    this process waits 45 seconds for a delta nobody was ever going to publish.
    Checked here so the failure names its cause instead of looking like a wrong
    tag -- which is the one failure mode this whole leg exists to make visible.
    """
    try:
        with open(PAGE_FILE) as fh:
            src = fh.read()
    except OSError:
        return True                     # not ours to judge; let the run say so
    return "account.signin" in src or "account.restore" in src


async def watch(tag: str, key: bytes, page: str) -> dict | None:
    """Subscribe first, then open the page, then wait for what it publishes."""
    async with websockets.connect(RELAY) as ws:
        await ws.send(json.dumps({"t": "sub", "tags": [tag], "since": 0, "v": 1}))
        while True:                                   # drain the backlog
            f = json.loads(await asyncio.wait_for(ws.recv(), timeout=10))
            if f["t"] == "eose":
                break

        proc = None
        if SHOT:
            # The sixth argument is JS for the shot tool to evaluate, and the
            # only reason it is passed is that doing so buys the page several
            # more seconds before the snapshot -- the chain is three HTTP round
            # trips, a websocket and a commit, and a browser killed halfway
            # through publishes nothing and looks exactly like a wrong tag.
            proc = subprocess.Popen([SHOT, page, os.path.join(HERE, "browser.png"),
                                     "1000", "620", "1"],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        else:
            print("  (set SHOT=/path/to/shot to drive the page automatically;")
            print(f"   open {page} in a browser now)")

        try:
            while True:
                f = json.loads(await asyncio.wait_for(ws.recv(), timeout=45))
                if f["t"] != "msg":
                    continue
                try:
                    return unseal(key, f["blob"])
                except Exception:                     # noqa: BLE001 — not ours
                    continue
        except asyncio.TimeoutError:
            return None
        finally:
            if proc:
                proc.wait(timeout=30)


def main() -> int:
    print("\n\033[1mPACIFIC · browser leg\033[0m")
    print(f"  auth {AUTH}    relay {RELAY}\n  page {PAGE}\n")

    if not the_page_signs_in():
        print("\033[31m  the browser leg cannot run against this page.\033[0m\n")
        print(f"  {PAGE_FILE} still calls account.link first, which is the passkey")
        print("  ceremony that was retired: keyholder.js derives the channel from the")
        print("  SEED now, and account.link needs one already on the device. The page")
        print("  has to sign in first —")
        print("      k.call('account.signin')   (the shim answers the ceremony)")
        print("      k.call('account.link')     (the channel, from the seed it opened)")
        print("      k.call('sync.start', { relay: RELAY })")
        print("      k.call('store.commit', { op: … })")
        print("\n  That file is not in this track. Nothing was faked to get past it.")
        return 2

    # The account, made here: a seed, a passkey that wraps it, and the wrap
    # stored on the Arc. This is the whole of what the browser will find.
    phone = Device(secrets.token_bytes(32))
    authenticator = Authenticator()
    cred_id = authenticator.enrol(phone.pk, HOST, RP_ID)
    prf = authenticator.prf(cred_id, WRAP_DOMAIN)

    with httpx.Client(follow_redirects=False, timeout=10) as c:
        r = c.put(AUTH + "/auth/users/" + phone.pk,
                  content=seal_wrap(prf, phone.seed, HOST),
                  headers={**phone.headers(c),
                           "Content-Type": "application/octet-stream"})
        if not check("an account is registered for the browser to open",
                     r.status_code == 200, f"HTTP {r.status_code} {r.text[:200]}"):
            return 1

    # The tag this side expects. The browser is never told it, and is never told
    # the seed it comes from either.
    seed_half, key = split(phone.seed)
    # The channel's first half is the relay-address SEED (18 Sep 2026); the browser
    # publishes to its public key, and so that is what this side listens on.
    tag = address.Address(bytes.fromhex(seed_half)).tag_hex
    print(f"    account {phone.pk[:16]}…")
    print(f"    tag     {tag}   (derived here, never sent)\n")

    hand_the_passkey_to_the_browser(authenticator, cred_id)
    try:
        got = asyncio.run(watch(tag, key, PAGE))
    finally:
        if os.path.exists(CRED_FILE):
            os.remove(CRED_FILE)

    check("keyholder.js opened the wrap and derived the same channel",
          got is not None,
          "nothing arrived on this tag — the browser derived a different one, "
          "or never got as far as publishing (see smoke/browser.png)")
    if got is not None:
        check("and the delta it sealed is readable here",
              got.get("op", {}).get("conv") == "c-smoke-browser", json.dumps(got)[:200])
        check("carrying the browser's own device as its author",
              bool(got.get("a")) and got.get("seq") is not None, json.dumps(got)[:200])
        print(f"\n    {json.dumps(got)}")

    print()
    if bad:
        print(f"\033[31m{len(bad)} failed\033[0m")
        for b in bad:
            print(f"  · {b}")
        return 1
    print("\033[32mall passed\033[0m")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
