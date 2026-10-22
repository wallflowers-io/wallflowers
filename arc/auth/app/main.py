"""The Arc's account service: a key, a wrap, a history, and where the chain ends.

RULED 13 Sep 2026. The seed is the root. A user owns 32 bytes, writes them down
as 24 words, and derives an Ed25519 identity from them. The passkey is a LOCAL
UNLOCK that opens a wrapped copy of the seed on a device -- it never reaches
this service, which is why there is no WebAuthn here, no ceremony, no session
cookie and no credential table. What this service does is:

  · GET  /auth/challenge              a nonce, single use, short-lived.
  · PUT  /auth/users/<pk>             store the wrap. Signed.
  · GET  /auth/users/<pk>             serve the wrap. UNAUTHENTICATED, by design.
  · PUT  /auth/users/<pk>/history     store the sealed history. Signed.
  · GET  /auth/users/<pk>/history     serve it. Signed.
  · PUT  /auth/users/<pk>/head        move the chain anchor. Signed, and
                                      compare-and-set on the chain position.
  · GET  /auth/users/<pk>/head        serve it. UNAUTHENTICATED, by design.
  · GET  /auth/users/<pk>/head/meta   position, size, and a digest to watch.
  · GET  /auth/users/<pk>/account     what we hold for this key. Signed.
  · PUT  /auth/users/<pk>/seals/<node>
                                      move a Door node's seal counter (D-34 (c)).
                                      Signed, and compare-and-set on the counter.
  · GET  /auth/users/<pk>/seals/<node>
                                      the counter held. Signed.
  · POST /auth/users/<pk>/sites       reserve a slug. Signed.
  · PUT  /auth/users/<pk>/sites/<slug>/host
                                      bind a claimed slug to the Host that
                                      serves it. Signed, by the claimant.
  · GET  /auth/site/check             is a slug free? Free, and needs no account.
  · GET  /auth/site/<slug>/host       which Host a slug is served from. Free:
                                      the Arc asks it. Names an object, never
                                      a person.
  · GET  /.well-known/passkey-endpoints
                                      where a credential manager sends someone
                                      to make or manage a passkey, and what we
                                      do with the PRF. Public, and the last of
                                      those three is the reason it is published.

WHY THE WRAP READ CANNOT BE GATED. It is how a NEW DEVICE obtains the key it
would authenticate with: the address is the public key and the public key is
downstream of the seed the device is fetching, so demanding a signature first
would demand the very thing being fetched. What it gives up is that holding
someone's public key is enough to download their encrypted seed -- ciphertext
under a key only their authenticator can produce. What it does NOT give up is
their history, which is a separate blob under a separate key behind a
signature. That split is the whole reason this service no longer needs a
second factor, and no longer holds anybody's email address or phone number.

WHY THE HEAD READ IS UNGATED TOO, and it is a different reason. The head is the
anchor an archive chain is checked against -- the one thing a chain cannot prove
about itself, because a prefix of a valid chain is itself a valid chain. Serving
it to anyone makes it WITNESSABLE: another arc mirroring the account, a monitor,
or simply the person's own second device sees the ciphertext revert without ever
being able to read it. An anchor nobody can watch is an anchor an arc can move.
The cost is that the chain position is then public per key -- an activity
counter -- and that is the price of the two properties that make the head work
at all: a witness has to see it move, and this service has to refuse a write
that moves it backwards. Both need a number neither of them can decrypt.

FAILURE IS A JSON ERROR CODE, never a stack trace and never a 500 page: the
person cannot act on our exception, only on being told plainly what did not
work and whether it was theirs to fix.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import logging
import os
import re
import secrets
from contextlib import asynccontextmanager
from typing import Any

import httpx
from fastapi import FastAPI, Request, Response
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse

from . import arc as arcmod
from . import auth
from .config import Settings, load
from .db import SLOT_CURRENT, SLOT_PREVIOUS, StaleHead, StaleHistory, StaleSeal, Store
from .errors import AuthError

logging.basicConfig(level=os.environ.get("LOG_LEVEL", "INFO"))
log = logging.getLogger("kenjin.auth")

# The first four bytes of each sealed artefact (pacific-core: `wrap.rs` MAGIC
# and `backup.rs` MAGIC). Checked on upload so a client that PUTs the wrong
# file -- a history where a wrap goes, a plain archive, an empty body -- is
# told now rather than at the restore that needed it.
WRAP_MAGIC = b"PWR1"
HISTORY_MAGIC = b"PHS1"
HEAD_MAGIC = b"PHD1"
# MAGIC(4) || nonce(24) || ciphertext(32) || tag(16) for a wrap; a history's
# ciphertext is variable, so only its floor is fixed. Anything shorter is not
# an envelope, whatever its first four bytes say.
WRAP_MIN_BYTES = 4 + 24 + 32 + 16
HISTORY_MIN_BYTES = 4 + 24 + 16
HEAD_MIN_BYTES = 4 + 24 + 16
# A head is a tail hash, a position, a host and a timestamp -- about 150 bytes
# sealed. NOT a setting, unlike the wrap and history caps: there is nothing about
# an anchor an operator could reasonably want bigger, and a per-deployment
# ceiling on it would only be a way for one arc to refuse what another accepts.
HEAD_MAX_BYTES = 4096
# THE CEILING ON A DECLARED CHAIN POSITION, and it guards a worse failure than a
# size cap does. A position only ever goes UP -- that is the whole compare-and-set
# -- so one write at an absurd number is not an odd row, it is the end of the
# account's anchor: there is no DELETE route, no reset, and re-registering the
# wrap does not clear it. Every later write is `behind`, forever.
#
# WHERE THE LINE IS, and it is drawn to MATCH UNIX. The accident that actually
# happens is a client sending a CLOCK where a count belongs, so the cap sits
# exactly where unix crossed it: unix SECONDS passed 1,000,000,000 on 9 September
# 2001, and every coarser unit passed it long before. So every unix reading, in
# every unit, in any year this service will run, is above the cap and refused.
#
# A PREVIOUS VERSION OF THIS COMMENT WAS WRONG AND THE CAP WITH IT. It read "a
# trillion... every unix reading one could reach for is above it" and named
# milliseconds, microseconds and nanoseconds -- omitting SECONDS, which is
# ~1.79e9 and therefore 560x BELOW a trillion. It is also the unit this very
# service keeps time in (`int(time.time())` in db.py). A unix-seconds clock sent
# here sailed past the old cap and bricked the anchor exactly as an unbounded
# value would; that was reproduced against the tree before this change.
#
# No chain arrives at a billion either: one entry per epoch transition, a billion
# of them is thirty entries a second for a year. The cap is where the number
# stops being a COUNT, not where it stops being plausible work.
#
# It is a bound and not a cure: a device that declares 999,999,999,999 has still
# stranded its own anchor, and nothing here can undo that (see README, "Not
# built"). What it does remove is the unbounded version of that accident, and one
# 500 -- the head carries its position as a u64 (`pacific_core::head`) while
# SQLite's INTEGER is SIGNED 64-bit, so anything at or above 2**63 raises
# OverflowError on the way into the column.
CHAIN_POSITION_MAX = 1_000_000_000

CHALLENGE_HEADER = "X-Pacific-Challenge"
SIGNATURE_HEADER = "X-Pacific-Signature"
EXPORTED_AT_HEADER = "X-History-Exported-At"
UPLOADED_AT_HEADER = "X-History-Uploaded-At"
# The chain position, declared in the clear because the blob is opaque to us and
# the compare-and-set has to run on something we can read. The same number is
# sealed inside, so a client can catch us reporting a different one.
CHAIN_POSITION_HEADER = "X-Chain-Position"
HEAD_UPDATED_AT_HEADER = "X-Head-Updated-At"
# A Door node's seal counter (D-34 (c)): the one the Door holds, and the next.
SEAL_HELD_HEADER = "X-Seal-Held"
SEAL_COUNTER_HEADER = "X-Seal-Counter"
# Bounded as the chain position is, for the same column.
SEAL_COUNTER_MAX = 1_000_000_000
# door.wallflowers.io, or 127.0.0.1:8233 in development.
_SEAL_NODE = re.compile(r"[A-Za-z0-9.:-]{1,253}")
_WHOLE = re.compile(r"[0-9]+")


@asynccontextmanager
async def lifespan(app: FastAPI):
    cfg = load()
    app.state.cfg = cfg
    app.state.db = Store(cfg.db_path)
    app.state.http = httpx.AsyncClient(follow_redirects=False)

    # Say out loud what is not wired. A door that silently does nothing is the
    # hardest kind of outage to find.
    if not cfg.arc_url:
        log.warning("ARC_URL unset — accounts will not record which Arc hosts them")
    log.info("audience=%s — devices must sign for exactly this host", cfg.audience)

    try:
        yield
    finally:
        await app.state.http.aclose()
        app.state.db.close()


app = FastAPI(title="kenjin-auth", lifespan=lifespan, docs_url=None, redoc_url=None)

# ─────────────────────────────────── CORS ──────────────────────────────────
#
# WITHOUT THIS THE ARC CANNOT MOVE TO ITS OWN HOSTNAME. While the pages and the
# service shared an origin, every call here was same-origin and no browser asked
# a question. Put the Arc on `arc.wallflowers.io` and the pages on
# `wallflowers.io` and they are different origins: the writes carry
# `X-Pacific-Challenge`, `X-Pacific-Signature` and `X-Chain-Position`, which are
# not simple headers, so the browser sends an OPTIONS PREFLIGHT first -- and an
# unrouted preflight fails as a network error with no status to point at, which
# is the worst way to start debugging an auth problem.
#
# `*`, AND THAT IS ARCHITECTURAL RATHER THAN LAZY. Two reasons, and the second
# is the one that settles it:
#
#   * CORS IS NOT AN AUTHORISATION MECHANISM HERE, and cannot be. This service
#     has no cookies and no bearer tokens -- it verifies a signature over a nonce
#     it issued -- so there is no ambient authority for a hostile page to borrow.
#     An attacker's page that reaches these routes can do exactly what `curl`
#     can, which for the gated routes is nothing. Narrowing the origin would look
#     like a boundary and enforce nothing; the signature is the boundary.
#
#   * THE SET OF LEGITIMATE ORIGINS IS UNBOUNDED BY DESIGN. CLAUDE.md: the web
#     app can be embedded in a member's own site, and "embedding changes nothing"
#     about how it authenticates. An allow-list cannot enumerate member domains,
#     so it would break that deployment silently the first time someone used it.
#
# The ungated reads are `*` for the reason they are ungated at all: a wrap is
# "serve to whoever asks", and an anchor nobody can watch is an anchor an arc can
# move.
#
# EXPOSE_HEADERS IS THE ONE THAT BITES. Cross-origin, JavaScript can read only
# CORS-safelisted response headers unless they are named here -- so without this
# line `fetchHead`'s `r.headers.get("X-Chain-Position")` returns null, the client
# cannot check the position the arc served against the one sealed in the blob,
# and the check silently stops happening rather than failing.
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    # No cookies anywhere in this service, and `*` with credentials is forbidden
    # by the spec regardless. Stated rather than defaulted, because the two facts
    # are connected and a future reader should see that they are.
    allow_credentials=False,
    allow_methods=["GET", "PUT", "POST", "OPTIONS"],
    allow_headers=[CHALLENGE_HEADER, SIGNATURE_HEADER, CHAIN_POSITION_HEADER,
                   EXPORTED_AT_HEADER, "Content-Type"],
    expose_headers=[CHAIN_POSITION_HEADER, HEAD_UPDATED_AT_HEADER,
                    EXPORTED_AT_HEADER, UPLOADED_AT_HEADER],
    max_age=600,
)


# ────────────────────────────── plumbing ──────────────────────────────────

_MESSAGES = {
    "bad_signature": "That signature does not prove possession of this key.",
    "challenge_spent": "That challenge is unknown, expired or already used. Ask for another.",
    "no_wrap": "There is no account stored for this key.",
    "no_history": "There is no history stored for this account.",
    "no_head": "This account has never recorded where its chain ends.",
    "too_large": "That is larger than this Arc accepts.",
    "not_a_wrap": "That is not a sealed wrap.",
    "not_a_history": "That is not a sealed history.",
    "not_a_head": "That is not a sealed head.",
    "slug_taken": "That address is already held.",
    "slug_not_yours": "That address is not claimed by this account.",
    # ONE MESSAGE FOR EVERY WRITE REFUSAL. A caller that could tell "wrong key"
    # from "wrong nonce" could enumerate; the log line (arc_write.verify's
    # detail, printed by _refuse) says which it was, and only the operator sees
    # it. See app/arc_write.py.
    "write_refused": "That write is not authorised for this address.",
}

_STATUS = {
    "bad_signature": 401, "challenge_spent": 401, "write_refused": 403,
    "no_wrap": 404, "no_history": 404, "no_head": 404, "no_seal": 404,
    "too_large": 413, "slug_taken": 409, "slug_not_yours": 403, "server_error": 500,
}


def _refuse(e: AuthError) -> JSONResponse:
    if e.detail:
        log.warning("[%s]: %s", e.code, e.detail)
    body: dict[str, Any] = {"error": e.code}
    if e.code in _MESSAGES:
        body["message"] = _MESSAGES[e.code]
    return JSONResponse(body, status_code=_STATUS.get(e.code, 400))


def _authenticated(request: Request, pk: str) -> str:
    """Prove the caller holds the private key for `pk`, or raise.

    Every route that touches an account's own data goes through here. Two reads
    do not, for two different reasons: the WRAP, because that read is how a new
    device obtains the key it would authenticate with, and the HEAD, because an
    anchor nobody can watch is an anchor an arc can move. Both are ciphertext.
    Every WRITE, the head's included, is signed.

    The challenge is spent BEFORE the signature is checked. A nonce burned by a
    failed attempt costs the caller one round trip and costs an attacker the
    ability to grind signatures against a live challenge.
    """
    cfg: Settings = request.app.state.cfg
    db: Store = request.app.state.db

    nonce = request.headers.get(CHALLENGE_HEADER, "")
    if not db.spend(nonce, auth.CHALLENGE_TTL):
        raise AuthError("challenge_spent", "challenge unknown, expired or already used")
    auth.verify(cfg.audience, pk, nonce, request.headers.get(SIGNATURE_HEADER, ""))
    return pk


async def _read_capped(request: Request, cap: int) -> bytes:
    """The body, or `too_large`. The declared length is checked first so an
    honest oversized client is refused before it sends anything; the stream is
    counted anyway, because a Content-Length is a claim."""
    declared = request.headers.get("content-length")
    if declared is not None and declared.isdigit() and int(declared) > cap:
        raise AuthError("too_large", f"declared {declared} bytes, cap {cap}")
    chunks: list[bytes] = []
    total = 0
    async for chunk in request.stream():
        total += len(chunk)
        if total > cap:
            raise AuthError("too_large", f"body exceeded {cap} bytes")
        chunks.append(chunk)
    return b"".join(chunks)


def _octet_stream(request: Request) -> None:
    ctype = request.headers.get("content-type", "").split(";")[0].strip().lower()
    if ctype != "application/octet-stream":
        raise AuthError("invalid_request", f"content-type {ctype!r}; send application/octet-stream")


def _exported_at(request: Request) -> int | None:
    raw = request.headers.get(EXPORTED_AT_HEADER)
    if raw is None or raw == "":
        return None
    try:
        return int(raw)
    except ValueError:
        raise AuthError("invalid_request", f"{EXPORTED_AT_HEADER} must be an integer (unix ms)")


def _chain_position(request: Request) -> int:
    """The declared chain position, or a refusal.

    MANDATORY, unlike `X-History-Exported-At`. That header is optional because
    refusing a request without it would lock out a client written before it
    existed; nothing has ever written a head, so there is no such client, and a
    head without a position is a head with no compare-and-set -- which is the
    entire mechanism. Accepting one "for now" would ship the stale-clobber shape
    this route exists to remove.

    BOUNDED AT BOTH ENDS. Below zero there is no chain, and above
    `CHAIN_POSITION_MAX` there is no chain either -- only a clock, or a u64 whose
    top half this service's own column cannot hold. Both are refused HERE, which
    is the only place the number is still a request rather than an anchor.
    """
    raw = request.headers.get(CHAIN_POSITION_HEADER)
    if raw is None or raw == "":
        raise AuthError("invalid_request",
                        f"{CHAIN_POSITION_HEADER} is required: a head without a "
                        "position cannot be ordered against the one already held")
    try:
        position = int(raw)
    except ValueError:
        raise AuthError("invalid_request",
                        f"{CHAIN_POSITION_HEADER} must be an integer, got {raw!r}")
    if position < 0:
        raise AuthError("invalid_request",
                        f"{CHAIN_POSITION_HEADER} must not be negative, got {position}")
    if position > CHAIN_POSITION_MAX:
        raise AuthError("invalid_request",
                        f"{CHAIN_POSITION_HEADER}={position} is above "
                        f"{CHAIN_POSITION_MAX}: no chain counts that far, and an "
                        "anchor set there could never be moved forward again")
    return position


# ─────────────────────────────── challenge ────────────────────────────────

@app.get("/auth/challenge")
async def challenge(request: Request):
    """A nonce to sign. Free and unauthenticated: it proves nothing by itself,
    and refusing to issue one to a stranger would only mean a device had to be
    known before it could show that it is."""
    db: Store = request.app.state.db
    return {"nonce": db.issue(secrets.token_hex(16), auth.CHALLENGE_TTL),
            "audience": request.app.state.cfg.audience}


# ──────────────────────────────── the wrap ────────────────────────────────

@app.put("/auth/users/{pk}")
async def wrap_put(pk: str, request: Request):
    """Store the wrap — and, on the first one, create the account.

    Registration and re-wrapping are the same call, which is the point: an
    account IS its key, so writing a wrap for a key that has none is
    registration and writing one for a key that has is enrolling a new passkey.
    ONE WRAP PER ACCOUNT, so the second silently revokes the first — which is
    exactly what someone restoring from their words on a new device wants, and
    it needs this service to be trusted with nothing.
    """
    cfg: Settings = request.app.state.cfg
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        _octet_stream(request)
        _authenticated(request, key)
        blob = await _read_capped(request, cfg.wrap_max_bytes)
        if len(blob) < WRAP_MIN_BYTES or blob[:4] != WRAP_MAGIC:
            raise AuthError("not_a_wrap", f"{len(blob)} bytes, magic {blob[:4]!r}")
        created = db.upsert(key)
        db.put_wrap(key, blob)
    except AuthError as e:
        return _refuse(e)

    # WHICH ARC HOSTS THIS ACCOUNT. Recorded, never acted on: establishing the
    # tether needs a contact bundle only a device holding the Ed25519 key can
    # mint, so this service records the address and the device does the signup.
    # Bind, never source.
    if created and cfg.arc_url:
        seen = await arcmod.discover(request.app.state.http, cfg.arc_url)
        if seen:
            was = db.host_on_arc(key, seen.url, seen.identity_key, seen.relay_url)
            if was:
                log.warning(
                    "arc identity CHANGED for %s at %s: pinned %s, now %s — "
                    "the pin is kept and this account is NOT re-pointed",
                    key[:16], seen.url, was, seen.identity_key,
                )

    log.info("wrap stored for %s… new=%s", key[:16], created)
    return {"ok": True, "created": created}


@app.get("/auth/users/{pk}")
async def wrap_get(pk: str, request: Request):
    """Serve the wrap to whoever asks. See the module note: this read is how a
    new device gets the key it would authenticate with, so it cannot be gated
    on holding that key. It is ciphertext under a key only an authenticator can
    produce, and it carries no history."""
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
    except AuthError as e:
        return _refuse(e)
    row = db.wrap(key)
    if not row:
        return _refuse(AuthError("no_wrap", f"{key[:16]}… has no wrap"))
    return Response(content=row["blob"], media_type="application/octet-stream",
                    headers={"Cache-Control": "no-store"})


# ────────────────────────────── the history ───────────────────────────────

@app.put("/auth/users/{pk}/history")
async def history_put(pk: str, request: Request):
    """Store a sealed history, demoting the previous one. Signed, like every
    other route that touches an account's own data — and unlike the wrap, this
    one has no reason not to be."""
    cfg: Settings = request.app.state.cfg
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        _octet_stream(request)
        _authenticated(request, key)
        exported_at = _exported_at(request)
        blob = await _read_capped(request, cfg.history_max_bytes)
        if len(blob) < HISTORY_MIN_BYTES or blob[:4] != HISTORY_MAGIC:
            raise AuthError("not_a_history", f"{len(blob)} bytes, magic {blob[:4]!r}")
        if not db.account(key):
            raise AuthError("no_wrap", "register a wrap for this key first")
        try:
            meta = db.put_history(key, blob, exported_at=exported_at)
        except StaleHistory as e:
            # 409, not 400: nothing is wrong with the request, it simply lost a
            # race with a device that had a newer view. Both stamps go back so
            # the loser can tell how far behind it is and decide whether to
            # re-export or to pull first.
            log.info("stale history refused for %s… held=%d offered=%d",
                     key[:16], e.held, e.offered)
            return JSONResponse(
                {"error": "stale_history",
                 "message": "a newer history is already stored for this account",
                 "held_exported_at": e.held, "offered_exported_at": e.offered},
                status_code=409)
    except AuthError as e:
        return _refuse(e)
    log.info("history stored for %s… size=%d", key[:16], meta["size"])
    return {"ok": True, **meta}


@app.get("/auth/users/{pk}/history")
async def history_get(pk: str, request: Request):
    """Hand over the ciphertext, against a signature.

    `?which=previous` fetches the copy before the current one, for the case the
    current upload turns out to be the bad one.
    """
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        _authenticated(request, key)
        which = request.query_params.get("which", "current")
        if which not in ("current", "previous"):
            raise AuthError("invalid_request", "which must be current or previous")
        row = db.history(key, SLOT_PREVIOUS if which == "previous" else SLOT_CURRENT)
        if not row:
            raise AuthError("no_history", f"{key[:16]}… has no {which} history")
    except AuthError as e:
        return _refuse(e)

    log.info("history released to %s… which=%s size=%d", key[:16], which, row["size"])
    headers = {UPLOADED_AT_HEADER: str(row["uploaded_at"]), "Cache-Control": "no-store"}
    if row["exported_at"] is not None:
        headers[EXPORTED_AT_HEADER] = str(row["exported_at"])
    return Response(content=row["blob"], media_type="application/octet-stream",
                    headers=headers)


@app.get("/auth/users/{pk}/history/meta")
async def history_meta(pk: str, request: Request):
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        _authenticated(request, key)
    except AuthError as e:
        return _refuse(e)
    cur = db.history_meta(key, SLOT_CURRENT)
    return {
        "exists": cur is not None,
        "size": cur["size"] if cur else 0,
        "uploaded_at": cur["uploaded_at"] if cur else None,
        "exported_at": cur["exported_at"] if cur else None,
        "previous": db.history_meta(key, SLOT_PREVIOUS),
    }


# ───────────────────────────────── the head ───────────────────────────────
#
# WHERE THE CHAIN ENDS. A chain of sealed entries catches modification,
# reordering, entries dropped from the middle, and forks. It does NOT catch
# TRUNCATION OF THE TAIL, because a prefix of a valid chain is itself a valid
# chain -- so detecting that needs an anchor the reader already holds. A device
# that persists state between runs has one; a borrowed laptop has none, and a
# borrowed laptop is the case that matters, because it is what someone reaches
# for after losing the phone.
#
# SEALED UNDER THE SEED, NOT THE PASSKEY PRF, and this service never sees either.
# It matters here only as the reason the head is its OWN blob rather than a field
# inside the wrap: the wrap is PRF-sealed, and a head in there could only be
# reached by someone holding the passkey. A second blob under a seed-derived key
# means both doors arrive -- passkey→PRF→wrap→seed→head, and words→seed→head.

@app.put("/auth/users/{pk}/head")
async def head_put(pk: str, request: Request):
    """Move the anchor forward, or refuse to move it at all.

    THE COMPARE-AND-SET IS THE POINT, and it is what makes this route different
    from `PUT /history`. That one guards on the exporter's own CLOCK and is
    advisory -- a clock is a claim, and the caller already proved it owns the
    account, so an attacker could simply send a newer stamp. A chain position is
    a COUNT: position n names exactly one tail, forever. So this is an ordering
    this service can enforce rather than a hint it can respect, and it is the
    guard that lets `PUT /history`'s eventually go away.

    Refusing costs the account nothing -- the stored head is left exactly as it
    was, and both positions come back so the loser can tell how far behind it is.
    """
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        _octet_stream(request)
        _authenticated(request, key)
        position = _chain_position(request)
        blob = await _read_capped(request, HEAD_MAX_BYTES)
        if len(blob) < HEAD_MIN_BYTES or blob[:4] != HEAD_MAGIC:
            raise AuthError("not_a_head", f"{len(blob)} bytes, magic {blob[:4]!r}")
        if not db.account(key):
            raise AuthError("no_wrap", "register a wrap for this key first")
        try:
            meta = db.put_head(key, blob, position)
        except StaleHead as e:
            # 409, like the history's: nothing is wrong with the request, it
            # simply does not move the chain forward. `reason` separates the two
            # events a client must respond to differently -- `behind` means read
            # the head and catch up, `fork` means two writers appended against
            # one predecessor and neither of them can be believed.
            log.info("head refused for %s… held=%d offered=%d reason=%s",
                     key[:16], e.held, e.offered, e.reason)
            return JSONResponse(
                {"error": "stale_head",
                 "message": ("that head does not move the chain forward"
                             if e.reason == "behind" else
                             "a different head is already stored at that position"),
                 "reason": e.reason,
                 "held_position": e.held, "offered_position": e.offered},
                status_code=409)
    except AuthError as e:
        return _refuse(e)
    log.info("head stored for %s… position=%d size=%d stored=%s",
             key[:16], meta["position"], meta["size"], meta["stored"])
    return {"ok": True, **meta}


@app.get("/auth/users/{pk}/head")
async def head_get(pk: str, request: Request):
    """Serve the head to whoever asks. See the module note: an anchor nobody can
    watch is an anchor an arc can move, and this is ciphertext under a key
    derived from a seed this service has never held.

    The position goes back in a header because the caller needs it before it can
    decide anything -- and the same number is sealed inside the blob, so a client
    that opens it catches us if the two disagree."""
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
    except AuthError as e:
        return _refuse(e)
    row = db.head(key)
    if not row:
        return _refuse(AuthError("no_head", f"{key[:16]}… has no head"))
    return Response(content=row["blob"], media_type="application/octet-stream",
                    headers={CHAIN_POSITION_HEADER: str(row["position"]),
                             HEAD_UPDATED_AT_HEADER: str(row["updated_at"]),
                             "Cache-Control": "no-store"})


@app.get("/auth/users/{pk}/head/meta")
async def head_meta(pk: str, request: Request):
    """What a WITNESS needs, without the blob.

    Unauthenticated for the same reason the blob is, and cheap enough to poll: a
    monitor, a mirroring arc or the person's own second device can watch the
    position and the digest and see the anchor revert without ever being able to
    read it. The digest is over the CIPHERTEXT -- it is not a promise about the
    contents, it is a name for "these exact bytes", which is all a witness that
    kept yesterday's observation needs in order to say "those bytes are back"."""
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
    except AuthError as e:
        return _refuse(e)
    row = db.head(key)
    if not row:
        return {"exists": False, "position": None, "size": 0,
                "updated_at": None, "digest": None}
    return {
        "exists": True,
        "position": row["position"],
        "size": row["size"],
        "updated_at": row["updated_at"],
        "digest": "sha256:" + hashlib.sha256(row["blob"]).hexdigest(),
    }


# ──────────────────────────────── seals ───────────────────────────────────
#
# D-34 (c): a person's device state on a Door node is sealed at rest between
# sign-ins. The Door records each seal's counter here and moves it compare-and-set,
# signed as the head is. The state itself never comes here.

def _seal_node(raw: str) -> str:
    if not _SEAL_NODE.fullmatch(raw or ""):
        raise AuthError("invalid_request", f"node {raw[:64]!r} is not a host or host:port")
    return raw


def _seal_counter(request: Request, header: str) -> int:
    raw = request.headers.get(header)
    if raw is None or not _WHOLE.fullmatch(raw):
        raise AuthError("invalid_request", f"{header} must be a whole number, got {raw!r}")
    n = int(raw)
    if n > SEAL_COUNTER_MAX:
        raise AuthError("invalid_request", f"{header}={n} is above {SEAL_COUNTER_MAX}")
    return n


@app.put("/auth/users/{pk}/seals/{node}")
async def seal_put(pk: str, node: str, request: Request):
    """Move the node's seal counter from `X-Seal-Held` to `X-Seal-Counter`.

    200 stored when the counter held is `X-Seal-Held` (0: none yet); 200 with
    `stored: false` when it already is `X-Seal-Counter`, a retry; otherwise 409
    `stale_seal`, the row untouched."""
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        where = _seal_node(node)
        _authenticated(request, key)
        held = _seal_counter(request, SEAL_HELD_HEADER)
        counter = _seal_counter(request, SEAL_COUNTER_HEADER)
        if counter <= held:
            raise AuthError("invalid_request",
                            f"{SEAL_COUNTER_HEADER}={counter} does not follow {SEAL_HELD_HEADER}={held}")
        if not db.account(key):
            raise AuthError("no_wrap", "register a wrap for this key first")
        try:
            meta = db.put_seal(key, where, held, counter)
        except StaleSeal as e:
            log.info("seal refused for %s… on %s held=%d offered=%d", key[:16], where, e.held, e.offered)
            return JSONResponse({"error": "stale_seal", "reason": "moved",
                                 "held_counter": e.held, "offered_counter": e.offered},
                                status_code=409)
    except AuthError as e:
        return _refuse(e)
    log.info("seal for %s… on %s counter=%d stored=%s", key[:16], where, meta["counter"], meta["stored"])
    return {"ok": True, **meta}


@app.get("/auth/users/{pk}/seals/{node}")
async def seal_get(pk: str, node: str, request: Request):
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        where = _seal_node(node)
        _authenticated(request, key)
    except AuthError as e:
        return _refuse(e)
    row = db.seal(key, where)
    if not row:
        return _refuse(AuthError("no_seal", f"{key[:16]}… has no seal on {where}"))
    return {"counter": row["counter"], "updated_at": row["updated_at"]}


# ──────────────────────────────── account ─────────────────────────────────

@app.get("/auth/users/{pk}/account")
async def account(pk: str, request: Request):
    """Everything this service holds for a key. Replaces `/auth/session`: there
    is no session to describe, so the question is not "who am I" — the caller
    proved that by signing — but "what do you have of mine"."""
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        _authenticated(request, key)
        row = db.account(key)
        if not row:
            raise AuthError("no_wrap", f"{key[:16]}… is not registered here")
    except AuthError as e:
        return _refuse(e)

    wrap = db.wrap(key)
    return {
        "identity_key": f"{auth.KEY_PREFIX}{key}",
        "created_at": row["created_at"],
        "arc": db.arc_of(key),
        "sites": db.sites_of(key),
        "wrap": {"size": wrap["size"], "updated_at": wrap["updated_at"]} if wrap else None,
        "history": db.history_meta(key, SLOT_CURRENT),
        # Reported so a surface can say what is actually anchored rather than
        # what could be -- the same discipline `backup.rs` asks for about the
        # history: say what the Arc holds, never what it could hold.
        "head": db.head_meta(key),
    }


# ──────────────────────────────── sites ───────────────────────────────────
#
# A site's slug is a PATH ON kenjin.cc, so it shares a namespace with every real
# page and every proxied prefix. A slug that collides is a site nobody can
# reach, so the collision is refused here rather than discovered later.
#
# CHECKING IS FREE AND NEEDS NO ACCOUNT: you should learn an address is taken
# before being asked to make a key for it. CLAIMING needs a signature, and is
# idempotent — claiming a slug you already hold changes nothing.
#
# A RESERVATION IS NOT A SITE. The site is a GroupObject with Authority = Owner,
# and a non-member cannot even read it, so only the owner's device can author
# it. This service holds the address until that happens.

# Everything the site image serves or nginx proxies. Kept explicit: deriving it
# at runtime would go stale the moment a page is added by hand.
#
# THIS IS THE ONLY LIST. signup.html used to keep a second one for instant
# feedback, and the two had drifted in BOTH directions — the page reserved nine
# names the service would have handed out (investors, profile, unlock, soon,
# pacific, app, login, status, kenjin — every one of them a real path on
# kenjin.cc, so a claim would have been shadowed by the page that already
# answers there), and the service reserved seven the page would have accepted.
# A client-side copy of a server-side rule is a copy that goes stale silently,
# so the page now validates SHAPE locally (instant, no round trip, no list) and
# asks /auth/site/check for everything else. Names merged in from that list are
# marked below.
_RESERVED = {
    "about", "arc", "assets", "auth", "brand", "collaborate", "index", "lock",
    "market", "overview", "signin", "signup", "site", "sites", "well-known",
    "admin", "api", "static", "v1", "www", "help", "support",
    # from the page's list: real paths this service did not know about
    "app", "investors", "kenjin", "login", "pacific", "profile", "soon",
    "status", "unlock",
    # the page `prfUsageDetails` points at, below. A slug shadowing it would
    # make the URL in the passkey-endpoints document lead to somebody's site.
    "passkey",
    # real pages and a real directory this list did not know about, found by
    # tests/test_reserved_names.py when a slug became a face at /<slug> (22 Sep).
    "demo", "demo-media", "escape",
}
# 2-32 chars, a-z 0-9 and hyphens, never a hyphen at either end. Two is the
# floor because a one-character address is not one anybody can share aloud.
_SLUG_OK = re.compile(r"^[a-z0-9][a-z0-9-]{0,30}[a-z0-9]$")


def _slug_verdict(raw: str) -> tuple[bool, str]:
    slug = (raw or "").strip().lower()
    if not slug:
        return False, "empty"
    if not _SLUG_OK.match(slug):
        return False, "shape"          # a-z 0-9 and hyphens, 2-32, not at the ends
    if slug in _RESERVED:
        return False, "reserved"
    return True, "ok"


# ───────────────────── holding an address during signup ──────────────────────
#
# Step 1 of signup picks an address, and the claim that binds it to an account
# comes at the END — after the face, the passkey and the words. Between the two,
# nothing stopped someone else taking the address the person had just chosen. So
# finishing step 1 HOLDS it: for HOLD_TTL, bound to the requester's IP (there is no
# account yet to bind it to), released when the page closes, consumed by the claim.
#
# THE IP IS NEVER STORED. The holder is an HMAC of it under a key the service mints
# once (`Store.hold_key`), so the table can say "the same caller" and nothing more.
# Behind Cloudflare the caller's address arrives as CF-Connecting-IP, which the edge
# sets and a client cannot; the service itself listens on loopback only.
HOLD_TTL = 45 * 60
HOLD_MAX_PER_CALLER = 3


def _holder(request: Request) -> str:
    ip = (request.headers.get("cf-connecting-ip")
          or (request.headers.get("x-forwarded-for") or "").split(",")[0].strip()
          or (request.client.host if request.client else ""))
    db: Store = request.app.state.db
    return hmac.new(db.hold_key(), ip.encode(), hashlib.sha256).hexdigest()[:32]


async def _slug_from_body(request: Request) -> str:
    # A beacon sent on page close is text/plain (the one body a page may send
    # cross-origin without a preflight), so the JSON is read from the raw body.
    try:
        return str(json.loads((await request.body()) or b"{}").get("slug", ""))
    except Exception:                                       # noqa: BLE001
        return ""


@app.get("/auth/site/check")
async def site_check(request: Request, slug: str = ""):
    """Is this address usable? Idempotent, unauthenticated, side-effect free."""
    db: Store = request.app.state.db
    ok, why = _slug_verdict(slug)
    if not ok:
        return {"slug": slug.lower(), "available": False, "reason": why}
    taken = db.site(slug) is not None
    if taken:
        # Deliberately does not say WHO holds it.
        return {"slug": slug.lower(), "available": False, "reason": "taken"}
    hold = db.site_hold(slug)
    if hold and hold["holder"] != _holder(request):
        return {"slug": slug.lower(), "available": False, "reason": "held"}
    return {"slug": slug.lower(), "available": True, "reason": "ok"}


@app.post("/auth/site/hold")
async def site_hold(request: Request):
    """Hold an address while its signup finishes. See the note above."""
    db: Store = request.app.state.db
    slug = await _slug_from_body(request)
    ok, why = _slug_verdict(slug)
    if not ok:
        return JSONResponse({"error": "invalid_slug", "reason": why}, status_code=400)
    held, reason, until = db.hold_site(slug, _holder(request), HOLD_TTL, HOLD_MAX_PER_CALLER)
    if not held:
        return JSONResponse({"error": reason, "reason": reason, "slug": slug.lower()},
                            status_code=409)
    return {"slug": slug.lower(), "held": True, "held_until": until, "ttl": HOLD_TTL}


@app.post("/auth/site/release")
async def site_release(request: Request):
    """Drop the caller's own hold. What the page sends as a beacon when it closes."""
    db: Store = request.app.state.db
    slug = await _slug_from_body(request)
    return {"slug": slug.lower(), "released": db.release_site(slug, _holder(request))}


@app.post("/auth/users/{pk}/sites")
async def site_claim(pk: str, request: Request):
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        _authenticated(request, key)
        if not db.account(key):
            raise AuthError("no_wrap", "register a wrap for this key first")
        try:
            body = await request.json()
        except Exception:                                   # noqa: BLE001
            raise AuthError("invalid_request", "body was not JSON")
        ok, why = _slug_verdict(str(body.get("slug", "")))
        if not ok:
            return JSONResponse({"error": "invalid_slug", "reason": why}, status_code=400)
        claimed, reason = db.claim_site(str(body["slug"]), key, _holder(request))
        if not claimed:
            raise AuthError("slug_taken", f"{body['slug']} is held by another account")
    except AuthError as e:
        return _refuse(e)
    return {"slug": str(body["slug"]).lower(), "claimed": True, "reason": reason,
            "sites": db.sites_of(key)}


# ───────────────────── which Host an address is served from ──────────────────
#
# THE CLAIM OWNS THE NAME (ruled 22 Sep 2026). An address is claimed here at
# signup; the Arc serves it from a Host, a GroupObject whose roster is the owner
# and the Arc. Until this route existed the two never met: the Arc gave a name to
# the first Host it folded that asked for it, so an address reserved at signup
# could be served for somebody else's Host.
#
# So the owner BINDS the address to their Host when they publish, signed like
# every other write here, and the Arc asks which Host an address is bound to and
# serves from no other. The answer names an object, never a person: the check
# route above deliberately does not say who holds a slug, and neither does this.
_HOST_ID = re.compile(r"^[0-9a-f]{64}$")   # a 32-byte MLS group id, CURVE25519_AES128


@app.put("/auth/users/{pk}/sites/{slug}/host")
async def site_bind_host(pk: str, slug: str, request: Request):
    db: Store = request.app.state.db
    try:
        key = auth.identity_pk(pk)
        _authenticated(request, key)
        try:
            host = str((await request.json()).get("host", "")).lower()
        except Exception:                                   # noqa: BLE001
            raise AuthError("invalid_request", "body was not JSON")
        if not _HOST_ID.match(host):
            raise AuthError("invalid_request", "host must be a Host's object id, 64 hex")
        ok, reason = db.bind_host(slug, key, host)
        if not ok:
            # Unclaimed and not-yours are one answer on purpose: the check route
            # already says whether an address is taken, and this must not add who.
            raise AuthError("slug_not_yours", f"{slug.lower()} is not claimed by this account")
    except AuthError as e:
        return _refuse(e)
    return {"slug": slug.lower(), "host": host, "reason": reason}


@app.get("/auth/site/{slug}/host")
async def site_host(slug: str, request: Request):
    """Which Host an address is served from. Unauthenticated: the Arc asks it."""
    db: Store = request.app.state.db
    bound = db.site_host(slug)
    if bound is None:
        return JSONResponse({"error": "unclaimed", "slug": slug.lower()}, status_code=404)
    return bound


# ──────────────────── the passkey endpoints document ──────────────────────
#
# W3C, "A Well-Known URL for Relying Party Passkey Endpoints": an RP publishes a
# JSON document at https://<rp id>/.well-known/passkey-endpoints so a credential
# manager showing a list of saved passkeys can send someone to the page that
# makes them (`enroll`) or manages them (`manage`). Our RP ID is kenjin.cc for
# every account on every Arc (ruling D2, 13 Sep), so there is exactly one of
# these documents and this is it. An EMPTY object is legal and says only "there
# are passkeys here"; ours is not empty, and the optional third member is why.
#
# `prfUsageDetails` IS THE REASON THIS ROUTE EXISTS. WebAuthn Level 3's own
# co-editor has warned publicly that deriving an encryption key from the PRF
# extension is dangerous, because it turns a REPLACEABLE authentication
# credential into a NON-REPLACEABLE encryption root -- delete the passkey, lose
# the data -- and the credential manager offering the delete button cannot warn
# anybody, because it has no idea the site uses PRF at all. `prfUsageDetails` is
# the field that closes that gap, and it is the field most RPs will not fill in.
#
# WE ARE THE CASE THE WARNING IS NOT ABOUT, and the page says so in the plainest
# words we could find. The SEED is the root. The PRF only WRAPS it (`wrap.rs`),
# and the 24 words ARE the seed -- so deleting the passkey closes one door onto
# 32 bytes that two doors reach. It costs a local unlock, never an account.
#
# BUILT FROM PUBLIC_ORIGIN rather than frozen into the site image. The three
# URLs are pages of THIS deployment, and PUBLIC_ORIGIN is already the one place
# a deployment's origin comes from -- the audience devices sign for derives from
# it too. A staging Arc with kenjin.cc hardcoded here would walk its tester out
# of the copy under test and into the live site.
#
# WHAT IS NOT DONE HERE, and cannot be from inside this service: at the edge,
# /.well-known/ is served out of the SITE image as static files
# (`site/default.conf.template`, `location ^~ /.well-known/ { try_files $uri
# =404; }`), so this route answers on the service and 404s on kenjin.cc until
# that location forwards this one path to the /auth/ upstream. That file belongs
# to the site, not to this track.

# The pages, as paths. `/signup` and `/profile` are served from site/soon/
# behind the launch gate; `/passkey` is the public explainer and is not gated.
PASSKEY_ENROLL_PATH = "/signup"
PASSKEY_MANAGE_PATH = "/profile"
PRF_USAGE_DETAILS_PATH = "/passkey"


@app.get("/.well-known/passkey-endpoints")
async def passkey_endpoints(request: Request):
    """Where passkeys are made and managed here, and what we do with the PRF.

    Public, cheap, and the same for everyone: it describes the relying party,
    not an account, so there is nothing in it to authenticate and nothing in it
    to leak. CORS is open for the same reason -- a reader is as likely to be an
    extension or a page as a server, and this document has no secret in it.
    """
    cfg: Settings = request.app.state.cfg
    return JSONResponse(
        {
            "enroll": cfg.public_origin + PASSKEY_ENROLL_PATH,
            "manage": cfg.public_origin + PASSKEY_MANAGE_PATH,
            # OPTIONAL in the spec, and the one member we are here for.
            "prfUsageDetails": cfg.public_origin + PRF_USAGE_DETAILS_PATH,
        },
        # Revalidated, not cached: the same discipline as the site's other
        # well-known documents, so a corrected page reaches a credential manager
        # without waiting out an edge TTL.
        headers={"Cache-Control": "no-cache, must-revalidate",
                 "Access-Control-Allow-Origin": "*"},
    )


# ──────────────────────────────── health ──────────────────────────────────

@app.get("/auth/health")
async def health(request: Request):
    cfg: Settings = request.app.state.cfg
    return {
        "ok": True,
        "audience": cfg.audience,
        "wrap_max_bytes": cfg.wrap_max_bytes,
        "history_max_bytes": cfg.history_max_bytes,
        "head_max_bytes": HEAD_MAX_BYTES,
        # So a client can learn the ceiling rather than discover it by being
        # refused -- and, if it is about to declare something near it, know that
        # the number it is holding is a clock and not a count.
        "chain_position_max": CHAIN_POSITION_MAX,
        "arc": cfg.arc_url or None,
    }
