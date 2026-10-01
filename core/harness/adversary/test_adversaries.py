"""test_adversaries — E11 and E12 as tests (E10 lives in test_e10_intro_tripwire).

These are REAL: E11 drives the live semaphore relay over WebSockets; E12 drives a
real boto3 client against a modelled content-addressed bucket via botocore's
Stubber. Like E10, they demonstrate capabilities that are TRUE TODAY because the
legs they exercise (the relay's global seq, R2's content addressing) are real.
"""

from __future__ import annotations

import asyncio

from relay_harness import relay
from showcase import run_e11, run_e12


def test_e11_global_seq_recovers_interleaving():
    """A single global seq lets an observer on two unrelated tags recover order."""

    async def _run():
        async with relay() as r:
            return await run_e11(r.url)

    result = asyncio.run(_run())
    assert result["interleaving_recovered"], (
        f"recovered {result['recovered_interleaving']} != "
        f"published {result['published_interleaving']}"
    )
    assert result["total_order_across_tags"], "seqs did not totally order across tags"
    assert result["distinct_tags"] == 2
    assert result["pass"]


def test_e12_r2_confirmation_oracle():
    """Content-addressed keys make the bucket a membership oracle for known bytes."""
    result = run_e12()
    probes = {r["candidate"]: r["present"] for r in result["oracle_results"]}
    # Present candidates confirmed, absent ones denied — the oracle is sound.
    assert probes["known public file (present)"] is True
    assert probes["a guessed message that is NOT stored"] is False
    assert probes["the secret upload, bytes obtained elsewhere"] is True
    # Cross-epoch dedup nuance (G5): same plaintext, two epochs, two keys.
    assert result["cross_epoch_dedup"]["same_plaintext_two_epochs_give_two_keys"] is True
    assert result["cross_epoch_dedup"]["identical_bytes_dedup_to_one_key"] is True
    assert result["pass"]


if __name__ == "__main__":
    test_e11_global_seq_recovers_interleaving()
    print("E11 PASS: global seq recovered the exact interleaving across two tags.")
    test_e12_r2_confirmation_oracle()
    print("E12 PASS: R2 content-addressing confirmed membership of known bytes.")
