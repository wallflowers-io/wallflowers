"""wire — the relay's frame vocabulary, mirrored from arc/planes/relay/src/wire.rs.

Frames are plain dicts with a ``t`` discriminator, because that is what comes off
the socket and dicts diff cleanly against it. :func:`to_json` reproduces serde's
encoding exactly — including the three ``skip_serializing_if`` elisions, which are
load-bearing: a ``v = 0`` Sub must encode byte-identically to what pre-retention
builds send, or the claim that adding ``Gap`` changed nothing on the wire is false.

:func:`from_json` applies the serde defaults in the other direction (``commit``
false, ``since`` 0, ``v`` 0, ``ok`` TRUE when absent) so a frame parsed off the
real relay and a frame built by the model compare as equal dicts.
"""
from __future__ import annotations

import json
from typing import Any

Frame = dict[str, Any]

# ── client → relay ──────────────────────────────────────────────────────────


def pub(tag: str, blob: str, commit: bool = False, sig: str = "") -> Frame:
    """A ``Pub``. ``sig`` is the signature by the key ``tag`` IS (see address.py);
    the real relay refuses a Pub without one. Empty is elided, as serde does."""
    return {"t": "pub", "tag": tag, "blob": blob, "commit": bool(commit), "sig": sig}


def sub(tags: list[str], since: int = 0, v: int = 0) -> Frame:
    return {"t": "sub", "tags": list(tags), "since": int(since), "v": int(v)}


def media_put(key: str, length: int) -> Frame:
    return {"t": "media_put", "key": key, "len": int(length)}


def media_get(key: str) -> Frame:
    return {"t": "media_get", "key": key}


# ── relay → client ──────────────────────────────────────────────────────────


def msg(tag: str, seq: int, blob: str) -> Frame:
    return {"t": "msg", "tag": tag, "seq": int(seq), "blob": blob}


def ack(seq: int, ok: bool = True, reason: str | None = None) -> Frame:
    return {"t": "ack", "seq": int(seq), "ok": bool(ok), "reason": reason}


def gap(tag: str, floor: int) -> Frame:
    return {"t": "gap", "tag": tag, "floor": int(floor)}


def eose() -> Frame:
    return {"t": "eose"}


def media_url(key: str, method: str, url: str, expires_in: int) -> Frame:
    return {"t": "media_url", "key": key, "method": method, "url": url,
            "expires_in": int(expires_in)}


def media_err(key: str, reason: str) -> Frame:
    return {"t": "media_err", "key": key, "reason": reason}


# ── codec ───────────────────────────────────────────────────────────────────

#: serde's `skip_serializing_if` set, per frame tag: (field, value-that-is-omitted).
_ELIDED = {
    "pub": [("commit", False), ("sig", "")],
    "sub": [("v", 0)],
    "ack": [("ok", True), ("reason", None)],
}

#: serde's `#[serde(default)]` set, applied on parse.
_DEFAULTS = {
    "pub": {"commit": False, "sig": ""},
    "sub": {"since": 0, "v": 0},
    "ack": {"ok": True, "reason": None},
}

#: field order as the Rust enum declares it — serde emits struct fields in
#: declaration order, and a byte-identical encoding has to match that.
_ORDER = {
    "pub": ["tag", "blob", "commit", "sig"],
    "sub": ["tags", "since", "v"],
    "msg": ["tag", "seq", "blob"],
    "ack": ["seq", "ok", "reason"],
    "gap": ["tag", "floor"],
    "media_put": ["key", "len"],
    "media_get": ["key"],
    "media_url": ["key", "method", "url", "expires_in"],
    "media_err": ["key", "reason"],
    "eose": [],
}


def to_json(frame: Frame) -> str:
    """Encode exactly as `Frame::to_json` does, elisions and field order included."""
    t = frame["t"]
    out: dict[str, Any] = {"t": t}
    for field in _ORDER.get(t, [k for k in frame if k != "t"]):
        if field not in frame:
            continue
        if any(field == f and frame[field] == v for f, v in _ELIDED.get(t, [])):
            continue
        out[field] = frame[field]
    return json.dumps(out, separators=(",", ":"))


def from_json(text: str) -> Frame:
    """Parse a wire frame and fill in serde's defaults, so dicts compare equal."""
    f = json.loads(text)
    for field, default in _DEFAULTS.get(f.get("t", ""), {}).items():
        f.setdefault(field, default)
    return f


def normalise(frame: Frame) -> Frame:
    """Round-trip through the codec: defaults applied, elided fields restored."""
    return from_json(to_json(frame))
