"""E10 / gate G0 — THE TRIPWIRE. READ THIS BEFORE YOU "FIX" A FAILURE HERE.

  Cádiz, the yard. szonja scans axel's code to join STOMA. eve is subscribed to the
  relay. Eve opens the intro blob and reads szonja, axel's key, and why she scanned.
                                                    — execution.html §02, gate G0

╔══════════════════════════════════════════════════════════════════════════════════╗
║  G0 CLEARED 18 SEP 2026 — THIS FILE IS NOW A GATE, WITH ONE TRIPWIRE LEFT.        ║
║                                                                                  ║
║  What cleared it: self-certifying relay addresses. An intro is published to the   ║
║  ADDRESS its value names (an Ed25519 public key derived from it) and sealed under ║
║  the value, which the relay never sees. The tag on the frame no longer opens the  ║
║  blob it carries, so neither the relay nor anyone watching it can read an intro.  ║
║                                                                                  ║
║  GATES — must stay green:                                                        ║
║    test_g0_the_relay_cannot_open_the_pairing_intro                               ║
║    test_g0_the_group_join_intro_is_shut_too                                      ║
║    test_opening_an_intro_does_not_confer_group_access  — the narrowing            ║
║    test_the_tripwire_itself_can_fail                   — the negative control     ║
║    test_the_tripwire_refuses_to_grade_without_a_live_blob — no fallbacks          ║
║                                                                                  ║
║  STILL A TRIPWIRE — INVERTED, green because a narrower hole is open:             ║
║    test_g0b_a_bundle_holder_can_still_open_an_intro                              ║
║  The intro is sealed under the bundle's intro value, so whoever holds the bundle ║
║  can read intros to its owner. Sealing to the KeyPackage init key (HPKE) closes  ║
║  it; the red that day is the deliverable.                                        ║
╚══════════════════════════════════════════════════════════════════════════════════╝

WHAT MAKES THE VERDICT HONEST (both of these were broken here once):

 1. THE CIPHERTEXT COMES FROM THE CALL SITES THE FIX WILL CHANGE. `intro_emit` runs
    `Node::pair_scan_why` and `Node::group_add_member` against a real relay and lets
    node.rs publish. Nothing in this harness hand-rolls an IntroPayload or calls
    `seal::seal` on the intro path — a blob the harness sealed itself would stay
    openable from its tag forever, so a tripwire aimed at one could never fire.

 2. THERE IS NO RECORDED-BLOB FALLBACK. A landed fix and a failed emitter run look
    identical from here, so a fallback would report "still vulnerable" forever. If a
    live blob cannot be obtained, this file produces NO VERDICT and fails saying so.

Run standalone:  .venv/bin/python test_e10_intro_tripwire.py
Or under pytest: .venv/bin/python -m pytest test_e10_intro_tripwire.py -v
"""

from __future__ import annotations

import asyncio
import base64
import json
import os
import uuid
from dataclasses import dataclass
from pathlib import Path

import pytest

import emitter
from eavesdropper import INTRO_LEAK
from emitter import EmitterUnavailable, nonce
from pacific_seal import decode_intro, open_sealed, seal
from relay_client import RelayClient, address
from relay_harness import relay
from showcase import GRADED_FIELDS, read_blob_rows, run_e10

HERE = Path(__file__).resolve().parent

CLEARED = """
================================================================================
  G0 TRIPWIRE CLEARED — THE INTRO FIX HAS LANDED. UPDATE THIS TEST.

  THIS IS GOOD NEWS AND NOT A REGRESSION. An intro blob on the relay no longer
  opens under a key derived from its own routing tag. That is D-E (sealing the
  intro to a real recipient key) and/or A5 (seal() refusing tag == secret)
  working as designed.

  DO NOT revert the protocol change. DO NOT "repair" the adversary. Do this:

    1. delete or xfail the INVERTED tests in test_e10_intro_tripwire.py,
    2. keep the three that are NOT inverted — the narrowing, the negative
       control, and the no-fallback test — which must stay green either way,
    3. add the positive gate in their place: "the relay CANNOT open an intro
       blob", and flip G0 from tripwire to gate in execution.html §02.

  What the run actually saw:
================================================================================
"""

NO_VERDICT = """
================================================================================
  NO VERDICT — this tripwire could not obtain a LIVE intro blob, so it has no
  answer to give. It is not reporting "still vulnerable" and it is not reporting
  "fixed": it refuses to guess, because the two look identical from here and a
  recorded blob would say "still vulnerable" forever.

  This is a HARNESS failure, not a protocol result. Fix the cause below, then
  re-run. Do not add a fixture fallback; that is the defect this shape exists to
  prevent.

  The cause:
================================================================================
"""


def tripwire_cleared(ok: bool, detail: str) -> None:
    """Fail with a message that cannot be mistaken for a regression."""
    if not ok:
        pytest.fail(CLEARED + "  " + detail + "\n", pytrace=False)


def no_verdict(exc: BaseException) -> None:
    """No live blob. Fail loudly — and say which of the two failures this is.

    The hint is only read off a RUN failure. A cargo error can mention seal.rs for
    reasons that have nothing to do with A5, and misfiling a broken build as "the fix
    landed" is the same laundering in a different direction.
    """
    text = str(exc)
    if getattr(exc, "phase", "run") == "run" and emitter.looks_like_the_fix(text):
        pytest.fail(
            CLEARED
            + "  The emitter itself died inside the seal, which is what A5 does to\n"
            "  node.rs:319/:632 once it refuses tag == secret:\n\n" + text + "\n",
            pytrace=False,
        )
    pytest.fail(NO_VERDICT + text + "\n", pytrace=False)


# --------------------------------------------------------------------------
# the live runs — one real relay + one real pairing each, shared by the tests
# --------------------------------------------------------------------------
def _run_pair() -> dict:
    async def _go():
        async with relay(store=True) as r:
            result = await run_e10(r.url, store_path=str(r.store) if r.store else None)
            result["_rows"] = read_blob_rows(str(r.store)) if r.store else []
            return result

    return asyncio.run(_go())


def _run_groupjoin(marker: str, arc_suffix: str) -> dict:
    async def _go():
        async with relay(store=True) as r:
            arc = f"{r.url}/arc-{arc_suffix}"
            truth = emitter.emit("groupjoin", relay_url=r.url, arc=arc, group_name=marker)
            truth["_rows"] = read_blob_rows(str(r.store)) if r.store else []
            truth["_arc"] = arc
            return truth

    return asyncio.run(_go())


@pytest.fixture(scope="module")
def pair_run():
    """One real pairing over one real relay, shared by the tests that grade it."""
    try:
        return _run_pair()
    except (EmitterUnavailable, FileNotFoundError) as e:
        no_verdict(e)


@pytest.fixture(scope="module")
def groupjoin_run():
    """One real group admission over one real relay, with a nonce group name."""
    marker = nonce("STOMA-zine")
    try:
        run = _run_groupjoin(marker, uuid.uuid4().hex[:8])
    except (EmitterUnavailable, FileNotFoundError) as e:
        no_verdict(e)
    run["_marker"] = marker
    return run


@dataclass(frozen=True)
class Opened:
    """One blob the sweep got into: its address, its ciphertext, its plaintext."""

    tag: str
    blob: bytes
    plaintext: bytes


def _sweep(rows: list[tuple[str, int, str]]) -> list[Opened]:
    """Try EVERY stored blob under the key derived from ITS OWN routing tag.

    Eve is handed nothing — not even which tag is an intro tag. The relay holds the
    tag because it must, and that is the entire attack. The AEAD tag check is the
    discriminator: a blob sealed under a real MLS secret simply will not open.
    """
    out: list[Opened] = []
    for tag_hex, _seq, body in rows:
        try:
            raw = bytes.fromhex(tag_hex)
        except ValueError:
            continue
        if len(raw) != 32:
            continue
        blob = base64.b64decode(body)
        try:
            out.append(Opened(tag_hex, blob, open_sealed(blob, raw, raw)))
        except Exception:  # noqa: BLE001 — a refusal is a result, not an error
            continue
    return out


# --------------------------------------------------------------------------
# G0 — NOW A GATE. Cleared 18 Sep 2026; these must stay green.
# --------------------------------------------------------------------------
def _intro_address(intro_value: str) -> str:
    """The relay address an intro value names. The node publishes there, and the
    relay sees only this public key — never the value the blob is sealed under."""
    return address.Address(bytes.fromhex(intro_value)).tag_hex


def test_g0_the_relay_cannot_open_the_pairing_intro(pair_run):
    """GATE. node.rs's own pairing path, against a real relay.

    Cleared on 18 Sep 2026 by self-certifying addresses: an intro is published to the
    ADDRESS its value names — an Ed25519 public key derived from it — and sealed under
    the value itself, which the relay never sees. So the tag on the frame no longer
    opens the blob it carries. This is THE CHECK that stays: it is the old tripwire
    with its assertion turned the right way up.
    """
    r = pair_run
    assert r["via"] == "Node::pair_scan_why", r["via"]
    assert r["blob_source"].startswith("LIVE:"), r["blob_source"]
    assert r["intro_tag"] == _intro_address(r["intro_value"]), (
        "the node must publish an intro to the address its value names"
    )
    # The absence of an open must not be the absence of a blob.
    stored_tags = {t for t, _, _ in r["_rows"]} if r.get("_rows") else set()
    if stored_tags:
        assert r["intro_tag"] in stored_tags, "the intro never reached the relay's store"
    assert not r["opened"], (
        "G0 IS OPEN AGAIN: Eve opened the intro from the tag on the frame. Something is "
        "sealing an intro under the relay-visible address — check node.rs's pairing seal"
    )
    assert not _sweep(r["_rows"]) if r.get("_rows") else True, (
        "a stored blob opened under a key derived from its own tag"
    )


def test_g0_the_group_join_intro_is_shut_too(groupjoin_run):
    """GATE. The group-admission call site — the SAME IntroPayload, a second publish.

    A fix applied to one call site only would leave this one open; the gate covers both.
    """
    t = groupjoin_run
    assert t["via"] == "Node::group_add_member", t["via"]
    rows = t["_rows"]
    assert len(rows) >= 2, (
        "a group add publishes a sealed COMMIT and a sealed WELCOME; "
        f"only {len(rows)} blob(s) reached the relay"
    )
    assert _intro_address(t["intro_tag"]) in {r[0] for r in rows}, (
        "the Welcome never reached the address its intro value names"
    )
    opened = _sweep(rows)
    assert not opened, (
        f"G0 IS OPEN AGAIN: {sorted(o.tag[:16] for o in opened)} opened under a key "
        "derived from the tag the relay holds"
    )


# --------------------------------------------------------------------------
# G0b — INVERTED. The exposure G0's clearing did NOT close. Green = still open.
# --------------------------------------------------------------------------
def test_g0b_a_bundle_holder_can_still_open_an_intro(groupjoin_run):
    """INVERTED — green because a narrower hole is still open, and it says so.

    The relay is out; the BUNDLE HOLDER is not. The intro is still sealed under the
    bundle's intro value — node.rs `seal(&payload, intro, intro)` — and that value is
    in the contact bundle, so anyone the person has ever given their bundle to can
    open every intro addressed to them: who is joining them, and to what.

    The fix is the one this file always named: seal the intro to a key only the
    recipient holds (the KeyPackage's init key — HPKE), not to a value that travels
    in the bundle. When that lands this test goes red, and the red is the deliverable:
    flip it into a gate beside the two above.
    """
    t = groupjoin_run
    value = bytes.fromhex(t["intro_tag"])
    at = _intro_address(t["intro_tag"])
    blobs = [base64.b64decode(b) for tag, _s, b in t["_rows"] if tag == at]
    assert blobs, "no intro blob at the intro's address — no verdict"
    opened = []
    for blob in blobs:
        try:
            opened.append(open_sealed(blob, value, value))
        except Exception:  # noqa: BLE001 — a refusal is a result
            continue
    if not opened:
        pytest.fail(
            "\n  G0b CLEARED — a bundle holder can no longer open an intro. THE HPKE FIX\n"
            "  HAS LANDED: turn this test into a gate beside the G0 gates above.\n",
            pytrace=False,
        )
    fields = decode_intro(opened[0])
    assert fields.get("kind") == "forum", "what a bundle holder learns: the object kind"


# --------------------------------------------------------------------------
# THE NARROWING — not inverted. Green today, green after the fix, green forever.
# --------------------------------------------------------------------------
def test_opening_an_intro_does_not_confer_group_access(groupjoin_run):
    """NOT INVERTED — must pass today AND after the fix.

    The Welcome inside the intro is separately HPKE-sealed to the joiner's KeyPackage
    init_key, so opening an intro blob buys the social graph and the routing metadata
    and nothing inside the group. The group's display name rides the GroupContext
    extension — i.e. inside the Welcome — so it is the marker: a per-run token that
    must appear nowhere Eve can reach.

    If this ever fails, E10 is not a metadata leak but a content compromise, and the
    severity of the finding changes.

    This test does NOT call `tripwire_cleared`. Its first claim — the name is in clear
    nowhere on the relay — holds whether or not the intro opens, and when the fix lands
    and nothing opens at all, the narrowing has simply become stronger. A test that
    went red on the good news would be one more thing somebody had to reason about at
    exactly the wrong moment.
    """
    t = groupjoin_run
    marker = t["_marker"]
    assert t["absent_marker"] == marker, "the emitter did not use our per-run group name"
    m = marker.encode()

    # (1) Unconditional: the group NAME is in clear on no blob the relay stores.
    for tag_hex, _seq, body in t["_rows"]:
        assert m not in base64.b64decode(body), f"the group NAME is in clear on {tag_hex[:16]}…"

    opened = _sweep(t["_rows"])
    if not opened:
        # The intro no longer opens from its own tag, so Eve cannot reach the Welcome
        # at all. The narrowing holds a fortiori; the INVERTED tests above are the ones
        # that report the change.
        return
    plaintext = opened[0].plaintext

    # The control: this is only meaningful if the search CAN find a string that really
    # is present. The scanner's name is in clear; the group's name is not. (After the
    # fix this branch is not reached, and claim (1) stands on its own.)
    assert t["expect"]["scanner_name"].encode() in plaintext, (
        "the marker-absence check is worthless unless this search can find a string "
        "that IS present"
    )

    # (2) And the Welcome itself: sealed to a KeyPackage, opaque to every address Eve
    #     holds. The tag set is her whole key space.
    payload_welcome = _welcome_bytes(plaintext)
    assert len(payload_welcome) > 200, f"a real MLS Welcome is not {len(payload_welcome)} bytes"
    assert m not in payload_welcome, "the group NAME leaked out of the HPKE-sealed Welcome"
    assert m not in plaintext, "the group NAME leaked into the intro payload"
    for tag_hex in {r[0] for r in t["_rows"]}:
        raw = bytes.fromhex(tag_hex)
        try:
            open_sealed(payload_welcome, raw, raw)
        except Exception:  # noqa: BLE001 — expected
            continue
        pytest.fail(
            f"the Welcome opened under routing tag {tag_hex[:16]}… — that is not a "
            "metadata leak, it is a content compromise"
        )


def _welcome_bytes(plaintext: bytes) -> bytes:
    """The raw `welcome` field out of the CBOR IntroPayload."""
    import cbor2

    return bytes(cbor2.loads(plaintext)["welcome"])


# --------------------------------------------------------------------------
# THE CONTROLS — not inverted. These are what make the green above mean anything.
# --------------------------------------------------------------------------
def test_the_tripwire_itself_can_fail():
    """A tripwire that cannot go red is decoration.

    Publish a blob sealed to a tag under a DIFFERENT secret — which is what the intro
    hop will look like after D-E — and confirm the sweep leaves it shut. Without this,
    a broken sweep that opened everything would look green above, and the day the fix
    landed nobody would be told.
    """

    async def _go():
        async with relay(store=True) as r:
            addr = address.Address(os.urandom(32))
            tag = addr.tag
            blob = seal(b"what the fix will look like", tag, os.urandom(32))  # secret != tag
            async with RelayClient(r.url) as pub:
                await pub.publish(addr, blob)
            await asyncio.sleep(0.2)
            return tag.hex(), read_blob_rows(str(r.store))

    tag_hex, rows = asyncio.run(_go())
    assert any(t == tag_hex for t, _, _ in rows), "the probe never reached the relay's store"
    assert not _sweep(rows), (
        "the sweep opened a blob whose key is NOT its address — the adversary is "
        "broken and every green result in this file is worthless"
    )


def test_the_tripwire_refuses_to_grade_without_a_live_blob(monkeypatch, tmp_path):
    """NOT INVERTED. The no-fallback rule, as a test.

    This file used to fall back to a recorded blob when the emitter failed, and report
    the source as though it were live. A landed fix looks EXACTLY like a failed
    emitter run, so that fallback would have reported "still vulnerable" forever —
    the one answer that must never be wrong. Every one of these must raise.
    """
    missing = tmp_path / "not-a-binary"
    monkeypatch.setenv("INTRO_EMIT_BIN", str(missing))
    with pytest.raises(EmitterUnavailable, match="is not a file"):
        emitter.emit("pair", relay_url="ws://127.0.0.1:1")

    # Exits 0 and prints nothing: no JSON, therefore no verdict.
    monkeypatch.setenv("INTRO_EMIT_BIN", "/usr/bin/true")
    with pytest.raises(EmitterUnavailable, match="printed no JSON"):
        emitter.emit("pair", relay_url="ws://127.0.0.1:1")

    # Exits non-zero: no verdict.
    monkeypatch.setenv("INTRO_EMIT_BIN", "/usr/bin/false")
    with pytest.raises(EmitterUnavailable, match="failed"):
        emitter.emit("pair", relay_url="ws://127.0.0.1:1")

    # And there is no recorded blob anywhere for anything to fall back TO.
    for stale in (HERE / "fixtures", HERE / "fixtures" / "rust_intro_vector.json"):
        assert not stale.exists(), (
            f"{stale} is back. A pre-recorded intro blob within reach of this harness is "
            "how the signal gets laundered — see the module header."
        )


def test_an_emitter_death_inside_the_seal_reads_as_the_fix_not_as_breakage():
    """NOT INVERTED. A5 kills the emitter; that must not be filed as 'harness broken'.

    When `seal()` starts refusing `tag == secret`, `pair_scan_why` returns Err and the
    emitter exits non-zero. The verdict path has to recognise that as the fix landing.
    """
    assert emitter.looks_like_the_fix(
        "thread 'main' panicked: pair_scan_why: Seal(\"conn_secret must not equal dest_tag\")"
    )
    assert not emitter.looks_like_the_fix(
        "error: linking with `cc` failed: exit status 1"
    )


if __name__ == "__main__":
    print("== E10 tripwire: the REAL pairing path, end to end over a real relay ==")
    try:
        res = _run_pair()
    except EmitterUnavailable as e:
        print(NO_VERDICT + str(e))
        raise SystemExit(2) from None
    print(json.dumps({k: v for k, v in res.items() if not k.startswith("_")}, indent=2))
    print()
    print(f"   blob source : {res['blob_source']}")
    print(f"   call site   : {res['call_site']}")
    print(f"   verdict     : pass={res['pass']}")
    print("   A PASS TODAY MEANS THE VULNERABILITY IS STILL OPEN. When this file goes")
    print("   red, read the banner: the fix has landed and the test wants updating.")
    raise SystemExit(0 if res["pass"] else 1)
