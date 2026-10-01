"""relay_client — a minimal, REAL WebSocket client to the semaphore relay.

The relay (`arc/target/debug/semaphore`) speaks `pacific_wire::Frame` as JSON text
frames over a plain WebSocket (see core/pacific-wire/src/lib.rs and
arc/planes/relay/src/wire.rs). No hello/auth frame. Tags are hex(32 bytes), blobs
are base64 — both opaque to the relay.

Frames we use:
  PUB : {"t":"pub","tag": <hex>, "blob": <b64> [, "commit": true]}
  SUB : {"t":"sub","tags":[<hex>...], "since": <u64> [, "v": <u8>]}
  MSG : {"t":"msg","tag": <hex>, "seq": <u64>, "blob": <b64>}   relay->client
  ACK : {"t":"ack","seq": <u64> [, "ok": bool]}                 relay->client
  EOSE: {"t":"eose"}                                            relay->client

This is a real socket doing exactly what a subscriber and a publisher do; nothing
is simulated.
"""

from __future__ import annotations

import asyncio
import base64
import json
import sys
from pathlib import Path

import websockets

# harness/adversary -> harness: the one Python copy of the relay's write rule, held
# to the Rust by the vector in pacific-wire/wire-spec.v1.json. These scripts run
# standalone, so it is reached by path rather than as `harness.address`.
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import address  # noqa: E402


def blob_b64(raw: bytes) -> str:
    return base64.standard_b64encode(raw).decode("ascii")


def blob_unb64(s: str) -> bytes:
    return base64.standard_b64decode(s)


class RelayClient:
    """One live ws session to the relay. Async; caller drives pub/sub/drain."""

    def __init__(self, url: str):
        self.url = url
        self.ws = None

    async def __aenter__(self) -> "RelayClient":
        self.ws = await websockets.connect(self.url, open_timeout=10, max_size=None)
        return self

    async def __aexit__(self, *exc) -> None:
        if self.ws is not None:
            await self.ws.close()
            self.ws = None

    async def _send(self, frame: dict) -> None:
        await self.ws.send(json.dumps(frame))

    async def _recv(self) -> dict:
        return json.loads(await self.ws.recv())

    async def publish(self, addr, raw_blob: bytes, commit: bool = False) -> int:
        """PUB one blob to ``addr`` (an ``address.Address``); wait for the Ack.
        Returns the assigned seq.

        Signed by the key the address IS — the relay refuses anything else — so this
        takes the address, not a bare tag. Raises on a rejecting ack for a plain
        publish, naming the relay's reason, which mirrors transport.rs::publish.
        """
        blob = blob_b64(raw_blob)
        frame = {"t": "pub", "tag": addr.tag_hex, "blob": blob, "sig": addr.sign_pub(blob)}
        if commit:
            frame["commit"] = True
        await self._send(frame)
        while True:
            f = await self._recv()
            if f.get("t") == "ack":
                ok = f.get("ok", True)
                if not ok and (not commit or f.get("reason")):
                    raise RuntimeError(f"the relay refused a publish: {f.get('reason')}")
                return f["seq"]
            # ignore interleaved live msgs while waiting for our ack

    async def subscribe(self, tags: list[str], since: int = 0, v: int = 1) -> None:
        """SUB to `tags` from `since`. v>=1 so we may also be sent Gap frames."""
        frame = {"t": "sub", "tags": tags, "since": since}
        if v:
            frame["v"] = v
        await self._send(frame)

    async def drain_until_eose(self, timeout: float = 5.0) -> list[dict]:
        """Read frames until the single bare Eose that ends the replayed backlog.

        Returns the list of MSG frames seen (each {tag, seq, blob}); Gap/Ack frames
        are folded into the returned list too so a correlation adversary can see
        everything the relay told it.
        """
        out: list[dict] = []
        while True:
            try:
                f = await asyncio.wait_for(self._recv(), timeout=timeout)
            except asyncio.TimeoutError:
                break
            t = f.get("t")
            if t == "eose":
                break
            if t in ("msg", "gap"):
                out.append(f)
            # ack frames from our own publishes are ignored here
        return out

    async def collect_live(self, expected: int, timeout: float = 5.0) -> list[dict]:
        """Read up to `expected` live MSG frames (or until `timeout` of silence)."""
        out: list[dict] = []
        while len(out) < expected:
            try:
                f = await asyncio.wait_for(self._recv(), timeout=timeout)
            except asyncio.TimeoutError:
                break
            if f.get("t") == "msg":
                out.append(f)
        return out
