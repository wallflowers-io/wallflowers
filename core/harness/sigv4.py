"""sigv4 — a Python port of `arc/planes/relay/src/sigv4.rs`, for second-opinion use.

Hand-rolled SigV4 is where a second implementation is worth most: the failure mode
is an opaque 403 from R2 at runtime, and nothing before then. The Rust pins itself
to botocore with two known-answer vectors; this reproduces both, so the presigner
now has three implementations agreeing rather than two.

Pure: no I/O, no clock, no randomness — `now` is injected.
"""
from __future__ import annotations

import hashlib
import hmac as _hmac
from datetime import datetime, timezone
from urllib.parse import quote

#: RFC 3986 unreserved set. Python's `quote` never escapes A-Za-z0-9 and `_.-~`,
#: which is exactly the Rust's UNRESERVED, and it emits uppercase hex like AWS.
def _enc(s: str) -> str:
    return quote(s, safe="")


def _enc_path(s: str) -> str:
    return quote(s, safe="/")


def _hmac_sha256(key: bytes, msg: bytes) -> bytes:
    return _hmac.new(key, msg, hashlib.sha256).digest()


def presign(*, method: str, host: str, path_segments: list[str], region: str,
            expires_in: int, signed_headers: list[tuple[str, str]],
            query: list[tuple[str, str]], now: datetime,
            access_key_id: str, secret_access_key: str) -> str:
    """Build a presigned `https://` URL — `sigv4.rs::presign`, line for line."""
    now = now.astimezone(timezone.utc)
    amz_date = now.strftime("%Y%m%dT%H%M%SZ")
    datestamp = now.strftime("%Y%m%d")
    scope = f"{datestamp}/{region}/s3/aws4_request"

    canonical_uri = "/" + "/".join(_enc(s) for s in path_segments)

    headers = [("host", host)] + [(k.lower(), v) for k, v in signed_headers]
    headers.sort(key=lambda kv: kv[0])
    signed_header_list = ";".join(k for k, _ in headers)
    canonical_headers = "".join(f"{k}:{v.strip()}\n" for k, v in headers)

    params = [
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256"),
        ("X-Amz-Credential", f"{access_key_id}/{scope}"),
        ("X-Amz-Date", amz_date),
        ("X-Amz-Expires", str(expires_in)),
        ("X-Amz-SignedHeaders", signed_header_list),
    ] + list(query)
    params.sort(key=lambda kv: kv[0])
    canonical_query = "&".join(f"{_enc(k)}={_enc(v)}" for k, v in params)

    # Presigned URLs carry no body at signing time: the payload hash is the
    # literal UNSIGNED-PAYLOAD sentinel, not a digest of nothing. (Generic SigV4
    # query signing hashes the empty body and produces a DIFFERENT signature.)
    canonical_request = "\n".join([
        method, canonical_uri, canonical_query, canonical_headers,
        signed_header_list, "UNSIGNED-PAYLOAD",
    ])
    string_to_sign = "\n".join([
        "AWS4-HMAC-SHA256", amz_date, scope,
        hashlib.sha256(canonical_request.encode()).hexdigest(),
    ])

    k_date = _hmac_sha256(f"AWS4{secret_access_key}".encode(), datestamp.encode())
    k_region = _hmac_sha256(k_date, region.encode())
    k_service = _hmac_sha256(k_region, b"s3")
    k_signing = _hmac_sha256(k_service, b"aws4_request")
    signature = _hmac_sha256(k_signing, string_to_sign.encode()).hex()

    return (f"https://{host}{_enc_path(canonical_uri)}"
            f"?{canonical_query}&X-Amz-Signature={signature}")
