"""driver — a websocket client that speaks the relay's frame vocabulary.

Deliberately thin. The frames are `harness.wire` dicts in both directions, so what
comes back off a real socket compares directly with what the model put in a
`Conn.out` — which is the whole trick that makes the parity diff possible.
"""
from __future__ import annotations

import asyncio

import websockets

from .. import address, wire

#: How long to wait for a socket to prove it has nothing more to say. Local
#: loopback against a process on the same machine; anything slower than this is a
#: hang, not latency.
QUIET = 0.35


class Client:
    """One connection to the real relay."""

    def __init__(self, ws, label: str = "") -> None:
        self.ws = ws
        self.label = label

    @classmethod
    async def connect(cls, url: str, label: str = "") -> "Client":
        return cls(await websockets.connect(url), label)

    async def close(self) -> None:
        await self.ws.close()

    async def send(self, frame: wire.Frame) -> None:
        await self.ws.send(wire.to_json(frame))

    async def recv(self, timeout: float = 5.0) -> wire.Frame:
        return wire.from_json(await asyncio.wait_for(self.ws.recv(), timeout))

    async def until(self, kind: str, timeout: float = 5.0) -> list[wire.Frame]:
        """Collect frames up to and including the first of `kind`."""
        got: list[wire.Frame] = []
        while True:
            f = await self.recv(timeout)
            got.append(f)
            if f["t"] == kind:
                return got

    async def quiet(self, seconds: float = QUIET) -> list[wire.Frame]:
        """Everything that arrives in the next `seconds` — usually nothing, and
        'nothing' is the assertion (a rejected commit must not fan out)."""
        got: list[wire.Frame] = []
        loop = asyncio.get_running_loop()
        deadline = loop.time() + seconds
        while True:
            left = deadline - loop.time()
            if left <= 0:
                return got
            try:
                got.append(await self.recv(left))
            except (asyncio.TimeoutError, TimeoutError):
                return got

    # -- the verbs -----------------------------------------------------------

    async def pub(self, tag: str, blob: str, commit: bool = False) -> list[wire.Frame]:
        # Signed by the key the tag IS — the real relay refuses anything else.
        await self.send(wire.pub(tag, blob, commit, address.for_tag(tag).sign_pub(blob)))
        return await self.until("ack")

    async def sub(self, tags: list[str], since: int = 0, v: int = 0) -> list[wire.Frame]:
        await self.send(wire.sub(tags, since, v))
        return await self.until("eose")

    async def declare(self, v: int) -> list[wire.Frame]:
        """Raise this connection's vocabulary to `v` without watching anything.

        THE WIRE HAS NO OTHER DOOR. `declared_v` starts at V_BASE and is only ever
        raised by a Sub (relay lib.rs: `declared_v = declared_v.max(v)`), so a client
        that only ever UPLOADS — which is what every media case here is — has to Sub
        to the empty set to say what it can read. That is not a workaround: the Sub
        frame IS the vocabulary declaration, and `subscribe` iterates the tag list,
        so an empty one replays nothing and answers Eose immediately.
        """
        return await self.sub([], since=0, v=v)

    async def media_put(self, key: str, length: int) -> wire.Frame:
        await self.send(wire.media_put(key, length))
        return await self.recv()

    async def media_get(self, key: str) -> wire.Frame:
        await self.send(wire.media_get(key))
        return await self.recv()
