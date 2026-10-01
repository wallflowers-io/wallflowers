"""showcase — end-to-end runners for E10, E11, E12 against the REAL relay.

Each runner returns a structured result and folds what the adversary LEARNED into a
Knowledge set, so a render can watch the sets grow. Nothing here is simulated:
E10/E11 drive the real semaphore relay over real WebSockets with real crypto; E12
drives a real boto3 client against a modelled content-addressed bucket.

E10 IS NOT A DEMONSTRATION, IT IS A TRIPWIRE, and that changes what it is allowed to
do. It is the acceptance test for The Build's D-E and A5, so its only value is that it
stops passing when those land. Two rules follow, and both were broken here before:

  * THE CIPHERTEXT COMES OUT OF THE CALL SITES THE FIX WILL CHANGE. `emitter.emit()`
    runs `Node::pair_scan_why` (node.rs:319) and `Node::group_add_member` (node.rs:632)
    and lets NODE.RS publish to the relay. Nothing here hand-rolls an IntroPayload or
    calls `seal::seal`; a blob this harness sealed itself would stay openable from its
    tag forever, whatever node.rs did, and the tripwire would never fire.

  * THERE IS NO FALLBACK. If the live blob cannot be obtained, `EmitterUnavailable`
    propagates and the caller has NO VERDICT. It must never be caught and turned into
    "still vulnerable" — a landed fix looks exactly like a failed emitter run.

The `why` and `arc` handed to the emitter are fresh per-run nonces that appear nowhere
in this tree. Recovering them out of relay ciphertext is the proof that Eve OPENED
something rather than restated something she was handed.
"""

from __future__ import annotations

import base64
import sqlite3
import uuid
from pathlib import Path

from eavesdropper import INTRO_LEAK, Eavesdropper
from emitter import emit, nonce
from ordering import OrderingObserver
from pacific_seal import open_sealed
from r2_oracle import ModelBucket, R2Operator, demonstrate_cross_epoch_dedup, r2_key
from relay_client import RelayClient, address

HERE = Path(__file__).resolve().parent
CORE_ROOT = HERE.parents[1]  # core/harness/adversary -> core (unused; kept accurate)

# The six IntroPayload fields the spec pins as what one opened intro blob costs.
GRADED_FIELDS = ("scanner_pk", "scanner_name", "why", "kind", "arc", "owner")


def read_blob_rows(store_path: str) -> list[tuple[str, int, str]]:
    """The relay's own `blob` table, read-only — the HostileRelay's whole view.

    Not a model of the relay: the file `semaphore` is writing, opened `query_only`.
    Returns [(tag_hex, seq, body_b64)] in global seq order.
    """
    conn = sqlite3.connect(f"file:{store_path}", uri=True, timeout=10.0)
    try:
        conn.execute("PRAGMA query_only = ON")
        return [(t, int(q), b) for (t, q, b) in
                conn.execute("SELECT tag, seq, body FROM blob ORDER BY seq")]
    finally:
        conn.close()


def _operator_sweep(store_path: str | None) -> dict:
    """The HostileRelay's own view: try every stored blob's OWN tag as its key.

    Eve is handed nothing here — not even which tag is an intro tag. The relay holds
    the tag because it must, and that is the whole attack. This is also the leg that
    proves the adversary DISCRIMINATES: a pairing publishes an intro blob and a pile
    of prekey blobs, and only the intro should open.
    """
    if not store_path or not Path(store_path).is_file():
        return {"available": False}
    rows = read_blob_rows(store_path)
    opened: list[str] = []
    for tag_hex, _seq, body in rows:
        try:
            raw_tag = bytes.fromhex(tag_hex)
        except ValueError:
            continue
        if len(raw_tag) != 32:
            continue
        try:
            open_sealed(base64.standard_b64decode(body), dest_tag=raw_tag, conn_secret=raw_tag)
        except Exception:  # noqa: BLE001 — the AEAD tag check IS the discriminator
            continue
        opened.append(tag_hex)
    return {
        "available": True,
        "blobs_stored": len(rows),
        "distinct_tags": len({t for t, _, _ in rows}),
        "tags_opened": sorted(set(opened)),
        "blobs_opened": len(opened),
    }


async def run_e10(relay_url: str, store_path: str | None = None) -> dict:
    """E10 / G0: the REAL pairing path publishes an intro; eve opens it off the relay.

    Raises `EmitterUnavailable` when no live blob can be produced. DO NOT CATCH THAT
    AND RETURN A RESULT — there is no verdict to report, and the false one ("still
    vulnerable") is the expensive answer to get wrong.
    """
    # Fresh per-run tokens. Nothing in this tree can produce them but the run itself,
    # so recovering them out of ciphertext is the proof the open was real.
    why = nonce("why-yard-cadiz")
    arc = f"{relay_url}/arc-{uuid.uuid4().hex[:8]}"

    # THE REAL CALL SITE. node.rs:319 seals and node.rs's own session publishes.
    truth = emit("pair", relay_url=relay_url, why=why, arc=arc)
    # The emitter reports the bundle's intro VALUE — since 18 Sep 2026 a seed, not a
    # tag. The node publishes to the ADDRESS it names (pacific_wire::address), so that
    # is where the blob is and where Eve listens: an empty listen proves nothing.
    intro_value = truth["intro_tag"]
    tag_hex = address.Address(bytes.fromhex(intro_value)).tag_hex
    expect = truth["expect"]

    # Eve: a subscriber on the tag, which the relay must hold to route at all. She
    # derives her key from the tag ON THE FRAME, never from anything the emitter said.
    eve = Eavesdropper("eve")
    async with RelayClient(relay_url) as eve_sock:
        read = await eve.attempt_intro_read(eve_sock, [tag_hex])

    learned_matches_spec = read.learned == INTRO_LEAK
    field_match = all(read.fields.get(k) == expect.get(k) for k in GRADED_FIELDS)
    # The nonce leg: `why` and `arc` were minted microseconds ago by this process and
    # travelled only as ciphertext. Finding them in the plaintext is not a coincidence.
    nonce_recovered = read.fields.get("why") == why and read.fields.get("arc") == arc
    sweep = _operator_sweep(store_path)
    # Discrimination: a pairing puts an intro AND a stack of prekey blobs on the relay.
    # Exactly the intro should open; anything else means the adversary is broken and
    # every green result here is worthless.
    sweep_ok = (not sweep["available"]) or sweep["tags_opened"] == [tag_hex]

    return {
        "case": "E10",
        "gate": "G0",
        "blob_source": truth["source"],
        "call_site": truth["call_site"],
        "via": truth["via"],
        "intro_tag": tag_hex,
        "intro_value": intro_value,
        "opened": read.opened,
        "learned": sorted(read.learned),
        "learned_matches_intro_leak": learned_matches_spec,
        "fields_match_ground_truth": field_match,
        "per_run_nonce_recovered": nonce_recovered,
        "recovered_fields": read.fields,
        "operator_sweep": sweep,
        "sweep_discriminates": sweep_ok,
        "eve_knowledge": eve.knowledge.as_dict(),
        "pass": bool(
            read.opened
            and learned_matches_spec
            and field_match
            and nonce_recovered
            and sweep_ok
        ),
    }


async def run_e11(relay_url: str) -> dict:
    """E11: two unrelated tags, one global seq, exact interleaving recovered."""
    # Two unrelated addresses. Real keys: the relay refuses a Pub its tag did not sign.
    addr_a, addr_b = address.named("e11-A"), address.named("e11-B")
    tag_a, tag_b = addr_a.tag_hex, addr_b.tag_hex
    labels = {tag_a: "conversation-A", tag_b: "conversation-B"}
    by_tag = {tag_a: addr_a, tag_b: addr_b}
    # Publish a KNOWN interleaving: A, B, B, A, A, B — writers who never coordinate.
    published = [tag_a, tag_b, tag_b, tag_a, tag_a, tag_b]
    async with RelayClient(relay_url) as writer:
        for i, tag in enumerate(published):
            await writer.publish(by_tag[tag], f"opaque-blob-{i}".encode())

    obs = OrderingObserver("eve-order")
    async with RelayClient(relay_url) as eve_sock:
        finding = await obs.observe_two_tags(eve_sock, tag_a, tag_b, labels)

    recovered_labels = [lab for lab, _ in finding.recovered_order]
    published_labels = [labels[t] for t in published]
    # The recovered global order must equal the true publish interleaving.
    interleaving_recovered = recovered_labels == published_labels
    return {
        "case": "E11",
        "published_interleaving": published_labels,
        "recovered_interleaving": recovered_labels,
        "interleaving_recovered": interleaving_recovered,
        "total_order_across_tags": finding.total_order_across_tags,
        "distinct_tags": finding.distinct_tags,
        "eve_knowledge": obs.knowledge.as_dict(),
        "pass": bool(interleaving_recovered and finding.total_order_across_tags),
    }


def run_e12() -> dict:
    """E12: content-addressed keys make the bucket a confirmation oracle."""
    bucket = ModelBucket()
    # Plant some objects (as the group's real uploads would be): sealed ciphertext
    # blobs and a couple of "known" byte-strings the operator will later probe for.
    secret_upload = b"a private sealed archive blob nobody guesses"
    known_public_file = b"the STOMA zine cover, a public PDF everyone has"
    bucket.put(secret_upload)
    bucket.put(known_public_file)

    op = R2Operator(bucket)
    results = []
    # The oracle: probe candidates the adversary can PRODUCE the bytes for.
    results.append(op.confirm("known public file (present)", known_public_file))
    results.append(op.confirm("a guessed message that is NOT stored", b"was this ever uploaded?"))
    results.append(op.confirm("the secret upload, bytes obtained elsewhere", secret_upload))
    listing = op.sweep()
    dedup = demonstrate_cross_epoch_dedup(b"identical group plaintext")
    op.close()

    # Oracle is sound: present candidates confirmed, absent ones denied.
    oracle_sound = (
        results[0].present is True
        and results[1].present is False
        and results[2].present is True
        and all(len(r.key) == 64 for r in results)
    )
    return {
        "case": "E12",
        "gate": "G5 (dedup nuance)",
        "oracle_results": [
            {"candidate": r.candidate_label, "key": r.key[:12] + "…", "present": r.present}
            for r in results
        ],
        "bucket_listing_keys": [c["Key"][:12] + "…" for c in listing],
        "cross_epoch_dedup": dedup,
        "eve_knowledge": op.knowledge.as_dict(),
        "pass": bool(
            oracle_sound
            and dedup["same_plaintext_two_epochs_give_two_keys"]
            and dedup["identical_bytes_dedup_to_one_key"]
        ),
    }
