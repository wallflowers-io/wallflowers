"""relay_harness — bring up a REAL semaphore relay to attack.

The relay binary is `arc/target/debug/semaphore` (per the task and the-harness §07).
Bind address: RELAY_BIND, else $PORT, else 8787. Store: RELAY_STORE, else in-memory.

Two modes:
  - If RELAY_URL is set in the environment, we attack THAT relay (attach mode) and
    spawn nothing. Use this to point the harness at an already-running relay.
  - Otherwise we spawn the local `semaphore` binary on a free localhost port with an
    in-memory store (RELAY_STORE unset), wait for it to accept a WebSocket, yield the
    ws:// url, and tear it down on exit.

Nothing here is a mock: it is the production relay process, blind, ordering opaque
blobs on real sockets.
"""

from __future__ import annotations

import asyncio
import contextlib
import os
import socket
import subprocess
import tempfile
import time
from pathlib import Path

import websockets

# core/harness/adversary/ -> core -> product; the relay binary is in the SIBLING arc.
WORKSPACE = Path(__file__).resolve().parents[3]
SEMAPHORE_BIN = WORKSPACE / "arc" / "target" / "debug" / "semaphore"


def _free_port() -> int:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


async def _wait_ws(url: str, timeout: float = 15.0) -> None:
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            ws = await websockets.connect(url, open_timeout=2)
            await ws.close()
            return
        except Exception as e:  # noqa: BLE001 — waiting for the port to come up
            last = e
            await asyncio.sleep(0.15)
    raise RuntimeError(f"relay at {url} never accepted a ws connection: {last}")


class Relay:
    """A running relay to attack. Async context manager yielding `self`.

    `store` is the relay's own SQLite file when one was asked for, else None. It is
    the HostileRelay's view — "the four tables entire" — and E10 needs it to sweep
    every stored blob without being handed a tag. An attached relay (RELAY_URL) is
    somebody else's process, so its store is not ours to read: `store` is None and
    the sweep leg reports itself unavailable rather than pretending.
    """

    def __init__(self, url: str, proc: subprocess.Popen | None, store: Path | None = None):
        self.url = url
        self.proc = proc
        self.store = store

    @property
    def spawned(self) -> bool:
        return self.proc is not None


@contextlib.asynccontextmanager
async def relay(store: bool = False):
    """Yield a live `Relay`. Attaches to RELAY_URL, else spawns semaphore.

    Pass `store=True` for a durable SQLite store in a temp dir (torn down with the
    relay), which is what an operator-side sweep reads.
    """
    attach = os.environ.get("RELAY_URL")
    if attach:
        await _wait_ws(attach)
        yield Relay(attach, None)
        return

    if not SEMAPHORE_BIN.exists():
        raise FileNotFoundError(
            f"relay binary not found at {SEMAPHORE_BIN}. Build it "
            f"(cargo build -p relay --bin semaphore in arc/) or set RELAY_URL."
        )
    port = _free_port()
    bind = f"127.0.0.1:{port}"
    url = f"ws://{bind}"
    env = dict(os.environ)
    env["RELAY_BIND"] = bind
    env.pop("PORT", None)
    # Give the tunnel listener its own free port so two harness runs never collide.
    env["RELAY_TUNNEL_BIND"] = f"127.0.0.1:{_free_port()}"
    tmp: tempfile.TemporaryDirectory | None = None
    store_path: Path | None = None
    if store:
        tmp = tempfile.TemporaryDirectory(prefix="adversary-relay-")
        store_path = Path(tmp.name) / "relay.sqlite"
        env["RELAY_STORE"] = str(store_path)
    else:
        env.pop("RELAY_STORE", None)  # in-memory, nothing to clean up
    proc = subprocess.Popen(
        [str(SEMAPHORE_BIN)],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        await _wait_ws(url)
        yield Relay(url, proc, store_path)
    finally:
        with contextlib.suppress(Exception):
            proc.terminate()
            proc.wait(timeout=5)
        if tmp is not None:
            with contextlib.suppress(Exception):
                tmp.cleanup()
