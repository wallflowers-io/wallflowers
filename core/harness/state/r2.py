"""r2 — the bucket modelled as what it is: one content-addressed table plus a meter.

Mirrors `arc/planes/relay/src/budget.rs` (the meter and the list parser) and
`media.rs` (key validation and the three presigns). The relay never holds bytes —
it MINTS CAPABILITIES: `presign_put` bound to an exact length, `presign_get` TTL'd,
`presign_list` for one page. So this module is two halves that meet at a URL: the
bucket (`R2State`) and the authoriser (`R2Relay`).

The budget fails CLOSED, and that is the whole design: `Stale` means no recent
measurement — refusing rather than spending blind. A meter that cannot see is not
permission to keep spending.
"""
from __future__ import annotations

import hashlib
import time
from dataclasses import dataclass
from enum import Enum

from .. import sigv4

# ── constants, verbatim from budget.rs and media.rs ─────────────────────────

#: 9 GB against R2's 10 GB free tier; the GB of headroom absorbs measurement lag.
DEFAULT_BUDGET_BYTES = 9_000_000_000
#: How often to ask R2 how big it actually is (15 min, costed against Class A ops).
DEFAULT_POLL_SECS = 900
#: Objects per ListObjectsV2 page — 1000 is the S3 maximum.
LIST_PAGE_SIZE = 1000
#: Hard ceiling on pages walked in one measurement.
MAX_LIST_PAGES = 64
#: media.rs DEFAULT_MAX_BYTES — 25 MiB, a placeholder, not a ruled product limit.
DEFAULT_MAX_OBJECT_BYTES = 25 * 1024 * 1024
#: media.rs DEFAULT_TTL — five minutes.
DEFAULT_TTL_SECS = 300


class Verdict(str, Enum):
    """`budget.rs::Verdict`."""
    OK = "ok"
    OVER_BUDGET = "over_budget"
    #: No recent measurement — refusing rather than spending blind.
    STALE = "stale"


class MediaError(str, Enum):
    """`media.rs::MediaError`. The value IS the stable wire token."""
    UNCONFIGURED = "unconfigured"
    BAD_KEY = "bad_key"
    TOO_LARGE = "too_large"
    OVER_BUDGET = "over_budget"
    BUDGET_STALE = "budget_stale"


def now_secs() -> int:
    return int(time.time())


# ── the bucket ──────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Object:
    """One row. `key` is a 64-char sha256 hex — the object IS its content."""
    key: str
    size: int
    last_modified: float
    etag: str


@dataclass(frozen=True)
class ListPage:
    """One ListObjectsV2 page: what it accounts for, and the continuation token."""
    bytes: int
    next: str | None
    keys: tuple[str, ...] = ()


def is_valid_key(key: str) -> bool:
    """`media.rs::require_valid_key` — exactly 64 lowercase hex characters.

    The only thing standing between a presigning endpoint and an arbitrary-path
    write oracle: no traversal, no prefix collisions, no meaning in a name.
    """
    return len(key) == 64 and all(c in "0123456789abcdef" for c in key)


class R2State:
    """`object (key, size, last_modified, etag)` — content addressed.

    Content addressing dedups byte-identical objects, and that is exactly the
    tension B1 has to measure: archive blobs are sealed per epoch, so the same
    plaintext under two epoch keys is two ciphertexts and two keys. "Stored once
    for the whole group" holds WITHIN an epoch, not across them — see
    :meth:`dedup_ratio`.
    """

    def __init__(self) -> None:
        self.objects: dict[str, Object] = {}

    # -- writes (the client's side of the two-sided insert rule) -------------

    def put_bytes(self, body: bytes, now: float | None = None) -> Object:
        """Store ciphertext under its own sha256. Byte-identical bodies collapse."""
        return self.put(hashlib.sha256(body).hexdigest(), len(body), now,
                        etag=hashlib.md5(body).hexdigest())

    def put(self, key: str, size: int, now: float | None = None,
            etag: str | None = None) -> Object:
        if not is_valid_key(key):
            raise ValueError(f"not a 64-char sha256 hex key: {key!r}")
        obj = Object(key, size, time.time() if now is None else now,
                     etag or hashlib.md5(key.encode()).hexdigest())
        self.objects[key] = obj          # same key = same bytes = one row
        return obj

    def delete(self, key: str) -> bool:
        return self.objects.pop(key, None) is not None

    # -- reads ---------------------------------------------------------------

    def total_bytes(self) -> int:
        return sum(o.size for o in self.objects.values())

    def __len__(self) -> int:
        return len(self.objects)

    def list_objects_v2(self, continuation: str | None = None,
                        max_keys: int = LIST_PAGE_SIZE) -> ListPage:
        """One page, in key order — the shape `parse_list_page` consumes."""
        keys = sorted(self.objects)
        start = keys.index(continuation) if continuation in keys else 0
        page = keys[start:start + max_keys]
        rest = keys[start + max_keys:]
        return ListPage(bytes=sum(self.objects[k].size for k in page),
                        next=rest[0] if rest else None, keys=tuple(page))

    def measure(self, budget_max: int) -> tuple[int, int, bool]:
        """Walk the listing and sum the bucket — `lib.rs::measure_usage`.

        Returns (bytes, pages walked, capped). Two early exits, both rounding
        TOWARDS refusing: stop once the total passes the budget, and report the
        budget as fully consumed if the walk needs more than MAX_LIST_PAGES — a
        measurement we declined to finish is not evidence of headroom.
        """
        total, token = 0, None
        for page_no in range(1, MAX_LIST_PAGES + 1):
            page = self.list_objects_v2(token)
            total += page.bytes
            if total > budget_max:
                return (total, page_no, False)
            if page.next is None:
                return (total, page_no, False)
            token = page.next
        return (budget_max, MAX_LIST_PAGES, True)

    # -- the R2 operator's view (E12) ---------------------------------------

    def operator_view(self) -> list[tuple[str, int, float]]:
        """Keys, sizes, timestamps. Nothing else — a `list_objects_v2` sweep."""
        return sorted((o.key, o.size, o.last_modified) for o in self.objects.values())

    def confirms(self, body: bytes) -> bool:
        """E12: the bucket is a CONFIRMATION ORACLE. Anyone holding candidate bytes
        can hash them and ask whether they are stored — no key, no decryption."""
        return hashlib.sha256(body).hexdigest() in self.objects

    def dedup_ratio(self, bodies: list[bytes]) -> tuple[int, int]:
        """(distinct objects, bodies offered) if these were all stored. Sealed
        content dedups only byte-identically, so two epochs' ciphertexts of one
        plaintext count twice."""
        return (len({hashlib.sha256(b).hexdigest() for b in bodies}), len(bodies))


def parse_list_page(xml: str) -> ListPage:
    """`budget.rs::parse_list_page`. A deliberate scan, not an XML parser.

    Malformed input must be an ERROR, never a zero: a silent zero reads as "empty
    bucket", and an empty bucket means plenty of room.
    """
    if "<ListBucketResult" not in xml:
        raise ValueError("not a ListBucketResult response")
    total = 0
    for chunk in xml.split("<Size>")[1:]:
        if "</Size>" not in chunk:
            raise ValueError("unterminated <Size> element")
        raw = chunk.split("</Size>", 1)[0].strip()
        if not raw.isdigit():
            raise ValueError(f"unparseable <Size> value: {raw!r}")
        total += int(raw)
    nxt = None
    if "<NextContinuationToken>" in xml:
        rest = xml.split("<NextContinuationToken>", 1)[1]
        if "</NextContinuationToken>" in rest:
            tok = rest.split("</NextContinuationToken>", 1)[0].strip()
            nxt = tok or None
    return ListPage(bytes=total, next=nxt)


def list_page_xml(state: R2State, continuation: str | None = None,
                  bucket: str = "pacific-media") -> str:
    """Render a page as R2 really renders it, so the parser can be driven from
    the model. Shape captured from the live `pacific-media` bucket."""
    page = state.list_objects_v2(continuation)
    rows = "".join(
        f"<Contents><Key>{k}</Key><Size>{state.objects[k].size}</Size>"
        f"<LastModified>{time.strftime('%Y-%m-%dT%H:%M:%S.000Z', time.gmtime(state.objects[k].last_modified))}</LastModified>"
        f"<ETag>&quot;{state.objects[k].etag}&quot;</ETag>"
        f"<StorageClass>STANDARD</StorageClass></Contents>"
        for k in page.keys)
    token = (f"<NextContinuationToken>{page.next}</NextContinuationToken>"
             if page.next else "")
    return ('<?xml version="1.0" encoding="UTF-8"?>'
            '<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">'
            f"<Name>{bucket}</Name>{rows}"
            f"<IsTruncated>{'true' if page.next else 'false'}</IsTruncated>"
            f"<KeyCount>{len(page.keys)}</KeyCount>{token}</ListBucketResult>")


# ── the meter ───────────────────────────────────────────────────────────────


class Budget:
    """`budget.rs::Budget`. Two numbers decide every request.

    `measured` is the bucket's real size from ListObjectsV2 — NOT a lagged
    analytics surface, because a measurement RESETS `reserved`, so a stale zero
    would repeatedly wipe the in-flight accounting and re-open the gate.
    `reserved` is every byte authorised since that measurement: a presign is a
    promise the client may redeem at any moment, so counting it as already stored
    is deliberately pessimistic. The error is always towards refusing too early.
    """

    def __init__(self, max_bytes: int = DEFAULT_BUDGET_BYTES,
                 stale_after_secs: int = DEFAULT_POLL_SECS * 3) -> None:
        self.max_bytes = max_bytes
        self.stale_after = stale_after_secs
        self.measured = 0
        self.reserved = 0
        #: 0 means "never", which is stale by definition — a relay that has not
        #: yet heard from R2 does not presign.
        self.measured_at = 0

    def committed(self) -> int:
        """Last measurement plus everything authorised since. Reporting only."""
        return self.measured + self.reserved

    def record(self, bytes_: int, now_secs_: int | None = None) -> None:
        """A fresh measurement replaces `measured` and CLEARS `reserved`, so
        unredeemed presigns wash out rather than accumulating forever."""
        self.measured = bytes_
        self.reserved = 0
        self.measured_at = now_secs() if now_secs_ is None else now_secs_

    def reserve(self, length: int, now: int | None = None) -> Verdict:
        """Claim `length` bytes of headroom, or say why not."""
        now = now_secs() if now is None else now
        if self.measured_at == 0 or now - self.measured_at > self.stale_after:
            return Verdict.STALE
        if self.measured + self.reserved + length > self.max_bytes:
            return Verdict.OVER_BUDGET
        self.reserved += length
        return Verdict.OK


# ── the authoriser ──────────────────────────────────────────────────────────


class R2Relay:
    """`media.rs::Media` — the relay's media half. It holds no bytes, ever.

    The insert rule is two-sided: the relay AUTHORISES, the client writes, the
    budget gates. Headroom is claimed BEFORE signing, because a URL that exists is
    a URL that may be redeemed.
    """

    def __init__(self, host: str = "acct.r2.cloudflarestorage.com",
                 bucket: str = "pacific-media", region: str = "auto",
                 access_key_id: str = "AKIAIOSFODNN7EXAMPLE",
                 secret_access_key: str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
                 max_bytes: int = DEFAULT_MAX_OBJECT_BYTES,
                 ttl_secs: int = DEFAULT_TTL_SECS,
                 budget: Budget | None = None) -> None:
        self.host, self.bucket, self.region = host, bucket, region
        self.access_key_id, self.secret_access_key = access_key_id, secret_access_key
        self.max_bytes, self.ttl_secs = max_bytes, ttl_secs
        #: None only when explicitly uncapped (RELAY_R2_BUDGET_BYTES=0).
        self.budget = budget

    def presign_put(self, key: str, length: int, now=None,
                    now_s: int | None = None) -> str | MediaError:
        """Presign an upload of EXACTLY `length` bytes. Order matters and is tested:
        key shape, then the object cap, then the budget."""
        if not is_valid_key(key):
            return MediaError.BAD_KEY
        if length > self.max_bytes:
            return MediaError.TOO_LARGE
        if self.budget is not None:
            verdict = self.budget.reserve(length, now_s)
            if verdict is Verdict.OVER_BUDGET:
                return MediaError.OVER_BUDGET
            if verdict is Verdict.STALE:
                return MediaError.BUDGET_STALE
        return self._sign("PUT", [self.bucket, key],
                          [("content-length", str(length))], [], now)

    def presign_get(self, key: str, now=None) -> str | MediaError:
        """The relay does NOT check existence: a blind relay has no business
        tracking which keys are live, and the client finds out on fetch anyway."""
        if not is_valid_key(key):
            return MediaError.BAD_KEY
        return self._sign("GET", [self.bucket, key], [], [], now)

    def presign_list(self, continuation: str | None = None, now=None) -> str:
        query = [("list-type", "2"), ("max-keys", str(LIST_PAGE_SIZE))]
        if continuation is not None:
            query.append(("continuation-token", continuation))
        return self._sign("GET", [self.bucket], [], query, now)

    def _sign(self, method: str, segments: list[str],
              headers: list[tuple[str, str]], query: list[tuple[str, str]], now) -> str:
        from datetime import datetime, timezone
        return sigv4.presign(
            method=method, host=self.host, path_segments=segments,
            region=self.region, expires_in=self.ttl_secs,
            signed_headers=headers, query=query,
            now=now or datetime.now(timezone.utc),
            access_key_id=self.access_key_id,
            secret_access_key=self.secret_access_key)
