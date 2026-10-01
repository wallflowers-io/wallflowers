"""r2_oracle — E12, R2 is a confirmation oracle.

the-harness §02 (R2State): "object (key, size, last_modified, etag) — key = sha256
hex, 64 chars." §05 E12: "keys are sha256 content hashes, so whoever holds the
bucket can test whether specific known bytes are present by hashing them."

The leak: content-addressing names each object by sha256(bytes). So the question
"is this exact object in the bucket?" is answerable by anyone who can PRODUCE the
bytes — hash them, then ask the store whether that key exists (HEAD, or a LIST
sweep). No read of the object is required; presence of the key IS the answer. That
turns the store into a confirmation oracle for any candidate whose bytes you can
guess or obtain elsewhere.

We demonstrate with REAL boto3: a boto3 S3 client driven against a MODELLED
content-addressed bucket via botocore's Stubber (the client code path — SigV4
request building, HeadObject/ListObjectsV2 shapes — is the genuine article; the
responses come from our model of R2). This is "be R2" and "be the R2 adversary"
from the-harness §02, on the real client.

It also checks the tension G5 cares about: sealed content dedups ONLY byte-
identically, so identical plaintext under two epoch keys is two objects, two keys —
"stored once for the whole group" holds within an epoch, not across them.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from datetime import datetime, timezone

import boto3
from botocore.config import Config
from botocore.stub import Stubber

from knowledge import Knowledge
from pacific_seal import seal as pacific_seal


def r2_key(content: bytes) -> str:
    """R2's content address: sha256 hex, 64 chars (matches budget.rs's real keys)."""
    return hashlib.sha256(content).hexdigest()


@dataclass
class ModelBucket:
    """A modelled R2 bucket: content-addressed store, key = sha256(bytes)."""

    name: str = "pacific-media"
    objects: dict[str, bytes] = field(default_factory=dict)

    def put(self, content: bytes) -> str:
        key = r2_key(content)
        self.objects.setdefault(key, content)  # dedup is byte-identical only
        return key

    def has_key(self, key: str) -> bool:
        return key in self.objects

    def listing(self) -> list[dict]:
        now = datetime.now(timezone.utc)
        return [
            {"Key": k, "Size": len(v), "LastModified": now, "ETag": f'"{hashlib.md5(v).hexdigest()}"'}
            for k, v in self.objects.items()
        ]


@dataclass
class OracleResult:
    candidate_label: str
    key: str
    present: bool


class R2Operator:
    """Whoever holds the bucket. Tests membership of known bytes by hashing them."""

    def __init__(self, bucket: ModelBucket, name: str = "r2-operator"):
        self.bucket = bucket
        self.name = name
        self.knowledge = Knowledge(label=name)
        # A REAL boto3 S3 client (region/endpoint shaped like an R2 client), stubbed.
        self.s3 = boto3.client(
            "s3",
            region_name="auto",
            endpoint_url="https://pacific.r2.cloudflarestorage.com",
            aws_access_key_id="model",
            aws_secret_access_key="model",
            config=Config(signature_version="s3v4"),
        )
        self.stubber = Stubber(self.s3)
        self.stubber.activate()

    def confirm(self, candidate_label: str, content: bytes) -> OracleResult:
        """The oracle: hash the candidate bytes, HEAD the key, read the verdict.

        Uses the real boto3 `head_object` call; the response is what the modelled
        bucket would return (200 with size/etag if present, 404 if absent).
        """
        key = r2_key(content)
        present = self.bucket.has_key(key)
        expected = {"Bucket": self.bucket.name, "Key": key}
        if present:
            self.stubber.add_response(
                "head_object",
                {
                    "ContentLength": len(self.bucket.objects[key]),
                    "ETag": f'"{hashlib.md5(self.bucket.objects[key]).hexdigest()}"',
                    "LastModified": datetime.now(timezone.utc),
                },
                expected,
            )
            self.s3.head_object(**expected)  # real client call, returns 200
            self.knowledge.learn(f"PRESENT: {candidate_label} (key {key[:12]}…)")
        else:
            self.stubber.add_client_error(
                "head_object",
                service_error_code="404",
                service_message="Not Found",
                http_status_code=404,
                expected_params=expected,
            )
            try:
                self.s3.head_object(**expected)
            except self.s3.exceptions.ClientError:
                pass
            self.knowledge.learn(f"ABSENT: {candidate_label} (key {key[:12]}…)")
        self.knowledge.saw_tag(key)
        return OracleResult(candidate_label=candidate_label, key=key, present=present)

    def sweep(self) -> list[dict]:
        """'Be the R2 adversary': a ListObjectsV2 sweep — keys, sizes, timestamps."""
        listing = self.bucket.listing()
        self.stubber.add_response(
            "list_objects_v2",
            {
                "Name": self.bucket.name,
                "KeyCount": len(listing),
                "IsTruncated": False,
                "Contents": listing,
            },
            {"Bucket": self.bucket.name},
        )
        resp = self.s3.list_objects_v2(Bucket=self.bucket.name)
        keys = [c["Key"] for c in resp.get("Contents", [])]
        self.knowledge.note(f"listed {len(keys)} objects (keys+sizes+timestamps only)")
        return resp.get("Contents", [])

    def close(self) -> None:
        try:
            self.stubber.deactivate()
        except Exception:
            pass

    def report(self) -> dict:
        return {"adversary": self.name, "knowledge": self.knowledge.as_dict()}


def demonstrate_cross_epoch_dedup(plaintext: bytes) -> dict:
    """G5's nuance: identical plaintext under two epoch keys => two keys, no dedup.

    Seals the SAME plaintext with two different epoch (conn_secret) keys using the
    real pacific seal, content-addresses each ciphertext, and shows the keys differ
    — while storing identical BYTES twice yields one key.
    """
    tag = bytes([0x11]) * 32  # a fixed dest tag; irrelevant to the point
    epoch_a = bytes([0xA1]) * 32
    epoch_b = bytes([0xB2]) * 32
    # Deterministic nonce so the demonstration is reproducible; the point is the KEY.
    ct_a = pacific_seal(plaintext, dest_tag=tag, conn_secret=epoch_a, nonce=bytes(24))
    ct_b = pacific_seal(plaintext, dest_tag=tag, conn_secret=epoch_b, nonce=bytes(24))
    key_a, key_b = r2_key(ct_a), r2_key(ct_b)
    # Byte-identical store: same bytes => same key => dedup.
    key_a_again = r2_key(ct_a)
    return {
        "same_plaintext_two_epochs_give_two_keys": key_a != key_b,
        "key_epoch_a": key_a,
        "key_epoch_b": key_b,
        "identical_bytes_dedup_to_one_key": key_a == key_a_again,
    }
