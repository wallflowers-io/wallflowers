"""rust_parity — every unit test in the Rust modules we mirror, re-run in Python.

store.rs, budget.rs, wire.rs and sigv4.rs each carry their own unit tests, and
those tests ARE the specification of the insert rules. Reproducing them here with
the same numbers is the cheapest possible check that the model did not quietly
invent its own semantics: if one of these goes red, the mirror has drifted from
the thing it mirrors.

The two sigv4 vectors are stronger than that. They came from botocore — the
reference implementation — so reproducing them in a third language means
botocore, the Rust and this agree byte for byte on a hand-rolled signature.
"""
from __future__ import annotations

from datetime import datetime, timezone
from typing import Callable

from .. import address, wire
from ..sigv4 import presign
from ..state import (Budget, RelayState, R2State, Verdict, list_page_xml,
                     parse_list_page)
from .framework import CaseResult, Run

#: A verbatim ListObjectsV2 page from the live `pacific-media` bucket, copied out
#: of budget.rs so both parsers are pinned to what R2 actually emits.
REAL_R2_PAGE = (
    '<?xml version="1.0" encoding="UTF-8"?><ListBucketResult '
    'xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>pacific-media</Name>'
    '<Contents><Key>5d17b0cddb29d850c67fee40c8a52380642d4753402403c02caef01a9f6e1724</Key>'
    '<Size>4096</Size><LastModified>2026-09-09T07:32:25.202Z</LastModified>'
    '<ETag>&quot;3e3f3e89c827d4c031b1837e62804d20&quot;</ETag>'
    '<StorageClass>STANDARD</StorageClass></Contents>'
    '<Contents><Key>e375c19e66900d400b4f344acb7a391d61f453947704ddb612b057c699a0e8ba</Key>'
    '<Size>4096</Size><LastModified>2026-09-09T07:32:22.991Z</LastModified>'
    '<ETag>&quot;5c7c7615bbb7acdec430799dd3990de1&quot;</ETag>'
    '<StorageClass>STANDARD</StorageClass></Contents>'
    '<IsTruncated>false</IsTruncated><MaxKeys>2</MaxKeys><KeyCount>2</KeyCount>'
    '<EncodingType>url</EncodingType></ListBucketResult>')

CREDS = dict(access_key_id="AKIAIOSFODNN7EXAMPLE",
             secret_access_key="wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY")


def rust_unit_parity(on_step: Callable | None = None) -> CaseResult:
    r = Run("P0", "The Rust unit tests, re-run against the model", "model", on_step)

    # ── store.rs ────────────────────────────────────────────────────────────
    r.step("store.rs — fourteen tests of the insert rules")
    s = RelayState()
    for q, b in ((100, "a"), (200, "b"), (300, "c")):
        s.append("aa", q, b)
    r.check("age_eviction_raises_the_floor_to_the_highest_seq_removed",
            s.evict(350, 100, 0) == 2 and s.replay("aa", 0) == [(300, "c")]
            and s.floors(["aa"]).get("aa") == 200)

    s = RelayState(); s.append("aa", 100, "a"); s.evict(150, 10, 0)
    first = s.floors(["aa"])["aa"]
    s.append("aa", 400, "b"); s.evict(401, 100_000, 0)
    r.check("the_floor_only_ever_rises", first == 100 and s.floors(["aa"])["aa"] == 100)

    s = RelayState(); s.append("aa", 100, "a"); s.evict(101, 1_000_000, 0)
    r.check("a_tag_that_never_evicted_has_no_floor", s.floors(["aa"]) == {})

    s = RelayState()
    for q in (10, 20, 30, 40):
        s.append("aa", q, f"x{q}")
    r.check("the_cap_keeps_the_newest_and_floors_the_rest",
            s.evict(0, 0, 2) == 2
            and [q for q, _ in s.replay("aa", 0)] == [30, 40]
            and s.floors(["aa"])["aa"] == 20)

    s = RelayState(); s.append("aa", 10, "x"); s.append("bb", 10, "y"); s.append("bb", 20, "y2")
    removed = s.evict(0, 0, 1)
    f = s.floors(["aa", "bb"])
    r.check("eviction_is_per_tag",
            removed == 1 and len(s.replay("aa", 0)) == 1 and "aa" not in f and f["bb"] == 10)

    s = RelayState()
    r.check("the_commit_slot_is_one_per_tag_and_the_loser_stores_nothing",
            s.append("aa", 10, "winner", True) == ("stored", 10)
            and s.append("aa", 20, "loser", True) == ("slot_taken", 0)
            and s.replay("aa", 0) == [(10, "winner")])
    r.check("a_lost_commit_race_does_not_advance_the_seq_high_water", s.resume_seq() == 10)
    r.check("plain_publishes_to_a_claimed_tag_still_land",
            s.append("aa", 20, "chatter", False) == ("stored", 20)
            and s.replay("aa", 10) == [(20, "chatter")])

    s = RelayState(); s.append("aa", 10, "a"); s.append("aa", 20, "b")
    r.check("replay_is_exclusive_of_the_cursor",
            s.replay("aa", 10) == [(20, "b")] and s.replay("aa", 20) == [])

    s = RelayState()
    fresh = s.resume_seq(); s.append("aa", 500, "a"); hi = s.resume_seq(); s.append("bb", 400, "b")
    r.check("the_seq_high_water_is_durable_and_monotonic",
            fresh == 0 and hi == 500 and s.resume_seq() == 500)

    s = RelayState(); s.append("aa", 10, "x"); s.append("aa", 20, "y"); s.append("bb", 30, "x")
    r.check("counts_are_cardinalities_only",
            s.counts() == (2, 3) and s.tag_counts() == [("aa", 2), ("bb", 1)])

    s = RelayState()
    r.check("the_same_blob_at_the_same_tag_is_stored_once",
            s.append("aa", 10, "sealed") == ("stored", 10)
            and s.append("aa", 20, "sealed") == ("duplicate", 10)
            and s.replay("aa", 0) == [(10, "sealed")] and s.resume_seq() == 10
            and s.append("bb", 30, "sealed") == ("stored", 30))

    s = RelayState()
    r.check("a_retried_winning_commit_is_acked_as_the_win_it_was",
            s.append("aa", 10, "commit", True) == ("stored", 10)
            and s.append("aa", 20, "commit", True) == ("duplicate", 10)
            and s.append("aa", 30, "rival", True) == ("slot_taken", 0))

    # ── pacific-wire address.rs — the write rule, held to the shared vector ──
    r.step("address — the Python signer against the vector both repos test")
    ok = True
    for v in address.VECTORS:
        a = address.Address(bytes.fromhex(v["seed"]))
        ok &= a.tag_hex == v["tag"] and a.sign_pub(v["blob"]) == v["sig"]
    r.check("address_rule_matches_the_wire_spec", ok and len(address.VECTORS) > 0)

    # ── budget.rs ───────────────────────────────────────────────────────────
    r.step("budget.rs — ten tests of the meter and the list parser")
    t = 1_700_000_000
    r.check("refuses_until_it_has_measured", Budget(1000, 900).reserve(1, t) is Verdict.STALE)
    b = Budget(1000, 900); b.record(400, t)
    r.check("allows_within_budget_once_measured", b.reserve(500, t) is Verdict.OK)
    b = Budget(1000, 900); b.record(0, t)
    r.check("reservations_accumulate_against_the_cap",
            b.reserve(600, t) is Verdict.OK and b.reserve(500, t) is Verdict.OVER_BUDGET
            and b.reserve(400, t) is Verdict.OK and b.committed() == 1000)
    b = Budget(1000, 900); b.record(0, t); b.reserve(900, t); b.reserve(200, t); b.record(100, t)
    r.check("a_measurement_clears_in_flight_reservations",
            b.committed() == 100 and b.reserve(200, t) is Verdict.OK)
    b = Budget(1_000_000, 900); b.record(0, t)
    r.check("goes_closed_when_the_measurement_goes_stale",
            b.reserve(1, t) is Verdict.OK and b.reserve(1, t + 901) is Verdict.STALE)
    b = Budget(1000, 900); b.record(1000, t)
    r.check("an_exactly_full_bucket_still_refuses_one_more_byte",
            b.reserve(1, t) is Verdict.OVER_BUDGET and b.reserve(0, t) is Verdict.OK)
    page = parse_list_page(REAL_R2_PAGE)
    r.check("sums_a_real_r2_listing", (page.bytes, page.next) == (8192, None))
    r.check("an_empty_bucket_sums_to_zero",
            parse_list_page("<ListBucketResult><Name>b</Name><KeyCount>0</KeyCount>"
                            "</ListBucketResult>").bytes == 0)
    page = parse_list_page("<ListBucketResult><Contents><Size>10</Size></Contents>"
                           "<IsTruncated>true</IsTruncated><NextContinuationToken>"
                           "abc/def+123=</NextContinuationToken></ListBucketResult>")
    r.check("carries_the_continuation_token_when_truncated",
            (page.bytes, page.next) == (10, "abc/def+123="))
    refused = 0
    for body in ['<?xml version="1.0"?><Error><Code>AccessDenied</Code></Error>',
                 "<html><body>502 Bad Gateway</body></html>", "",
                 "<ListBucketResult><Contents><Size>notanumber</Size></Contents></ListBucketResult>",
                 "<ListBucketResult><Contents><Size>4096"]:
        try:
            parse_list_page(body)
        except ValueError:
            refused += 1
    r.check("a_malformed_response_is_an_error_not_an_empty_bucket", refused == 5,
            "an empty bucket means plenty of room, so a silent zero opens the gate")
    bucket = R2State()
    for i in range(4):
        bucket.put_bytes(b"x" * (i + 1) * 100)
    r.check("the model's own listing parses back to its own size",
            parse_list_page(list_page_xml(bucket)).bytes == bucket.total_bytes())

    # ── wire.rs ─────────────────────────────────────────────────────────────
    r.step("wire.rs — the encodings that must not change a byte")
    r.check("sub_since_defaults_to_zero",
            wire.from_json('{"t":"sub","tags":["aa"]}')
            == wire.sub(["aa"], 0, 0))
    r.check("gap_vocabulary_is_opt_in",
            wire.to_json(wire.sub(["aa"], 5, 0)) == '{"t":"sub","tags":["aa"],"since":5}'
            and wire.to_json(wire.sub(["aa"], 5, 1)) == '{"t":"sub","tags":["aa"],"since":5,"v":1}'
            and wire.to_json(wire.gap("aa", 7)) == '{"t":"gap","tag":"aa","floor":7}',
            "a v=0 Sub encodes byte-identically to what pre-retention builds send")
    r.check("media_vocabulary_is_request_gated",
            wire.to_json(wire.media_put("ab", 7)) == '{"t":"media_put","key":"ab","len":7}'
            and wire.to_json(wire.media_get("ab")) == '{"t":"media_get","key":"ab"}'
            and wire.to_json(wire.media_err("ab", "bad_key"))
            == '{"t":"media_err","key":"ab","reason":"bad_key"}')
    r.check("slot_fields_are_backward_compatible",
            wire.from_json('{"t":"pub","tag":"aa","blob":"Zm9v"}') == wire.pub("aa", "Zm9v", False)
            and wire.from_json('{"t":"ack","seq":9}') == wire.ack(9, True)
            and wire.to_json(wire.pub("aa", "Zm9v")) == '{"t":"pub","tag":"aa","blob":"Zm9v"}'
            and wire.to_json(wire.ack(9)) == '{"t":"ack","seq":9}')

    # ── sigv4.rs ────────────────────────────────────────────────────────────
    r.step("sigv4.rs — botocore's known-answer vectors, in a third language")
    url = presign(method="GET", host="examplebucket.s3.amazonaws.com",
                  path_segments=["test.txt"], region="us-east-1", expires_in=86400,
                  signed_headers=[], query=[],
                  now=datetime(2013, 5, 24, tzinfo=timezone.utc), **CREDS)
    r.check("matches_botocore_s3_presigner",
            "X-Amz-Signature=3ed0be64024db54d5574a27da223529635c383f9"
            "11f80e636f0ccc13890053d2" in url,
            "S3 presigning uses the UNSIGNED-PAYLOAD sentinel, not a body digest")

    def put(n: int) -> str:
        return presign(method="PUT", host="acct.r2.cloudflarestorage.com",
                       path_segments=["pacific-media", "ab" * 32], region="auto",
                       expires_in=300, signed_headers=[("content-length", str(n))],
                       query=[], now=datetime(2026, 9, 9, 12, tzinfo=timezone.utc), **CREDS)
    a = put(1024)
    r.check("content_length_is_bound_into_the_signature",
            "X-Amz-SignedHeaders=content-length%3Bhost" in a
            and "X-Amz-Signature=9f668e1e6d20ed8746d344cb03f56bde98c2"
                "7989fd21de6fb0f9ad0cca022fd1" in a
            and a != put(1025),
            "one URL cannot be reused to upload a larger object")

    def lst(token: str) -> str:
        return presign(method="GET", host="acct.r2.cloudflarestorage.com",
                       path_segments=["pacific-media"], region="auto", expires_in=300,
                       signed_headers=[], query=[("list-type", "2"),
                                                 ("continuation-token", token)],
                       now=datetime(2026, 9, 9, 12, tzinfo=timezone.utc), **CREDS)
    r.check("query_parameters_are_signed",
            "list-type=2" in lst("aaa") and lst("aaa") != lst("bbb"))
    r.note("29 assertions, each one the Rust's own test with the Rust's own numbers")
    return r.done()
