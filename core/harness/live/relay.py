"""relay — the real semaphore process, plus a read-only window onto its store.

The parity check is only worth running if it can see the real tables. The relay
writes SQLite when `RELAY_STORE` is set, so the harness points it at a scratch
file and opens that file read-only while the relay is running: WAL lets a reader
in without blocking the writer. That turns "the model agrees with the relay" from
a claim about frames into a claim about ROWS.
"""
from __future__ import annotations

import os
import socket
import sqlite3
import subprocess
import time
from pathlib import Path

from ..state.relay import Tables

#: Built by `cargo build -p relay`; the task's tree already carries it.
SEMAPHORE = Path(os.environ.get(
    "SEMAPHORE_BIN",
    # core/harness/live -> the workspace; the relay binary is in the SIBLING arc repo.
    str(Path(__file__).resolve().parents[3] / "arc/target/debug/semaphore")))


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Relay:
    """One semaphore process on a scratch store. Context-managed; always reaped."""

    def __init__(self, workdir: Path, *, retention_secs: int = 0,
                 max_per_tag: int = 0, sweep_secs: int = 1,
                 env: dict[str, str] | None = None, label: str = "relay") -> None:
        self.workdir = Path(workdir)
        self.workdir.mkdir(parents=True, exist_ok=True)
        self.store_path = self.workdir / f"{label}.db"
        self.log_path = self.workdir / f"{label}.log"
        self.port = free_port()
        self.url = f"ws://127.0.0.1:{self.port}"
        self.env = {
            "RELAY_BIND": f"127.0.0.1:{self.port}",
            "RELAY_TUNNEL_BIND": f"127.0.0.1:{free_port()}",
            "RELAY_STORE": str(self.store_path),
            "RELAY_RETENTION_SECS": str(retention_secs),
            "RELAY_MAX_BLOBS_PER_TAG": str(max_per_tag),
            "RELAY_SWEEP_SECS": str(sweep_secs),
            "RUST_LOG": "info",
            **(env or {}),
        }
        self.proc: subprocess.Popen | None = None

    # -- lifecycle -----------------------------------------------------------

    def start(self, timeout: float = 20.0) -> "Relay":
        if not SEMAPHORE.exists():
            raise FileNotFoundError(f"no relay binary at {SEMAPHORE}")
        self._log = open(self.log_path, "wb")
        self.proc = subprocess.Popen(
            [str(SEMAPHORE)], stdout=self._log, stderr=subprocess.STDOUT,
            env={**os.environ, **self.env})
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(f"relay exited {self.proc.returncode}\n{self.log()}")
            try:
                with socket.create_connection(("127.0.0.1", self.port), 0.2):
                    return self
            except OSError:
                time.sleep(0.05)
        raise TimeoutError(f"relay did not bind {self.port}\n{self.log()}")

    def stop(self) -> None:
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        if getattr(self, "_log", None):
            self._log.close()

    def __enter__(self) -> "Relay":
        return self.start()

    def __exit__(self, *exc) -> None:
        self.stop()

    def log(self) -> str:
        try:
            return self.log_path.read_text(errors="replace")
        except OSError:
            return ""

    # -- the tables, for real ------------------------------------------------

    def tables(self) -> Tables:
        """Read the four tables out of the live SQLite file, read-only."""
        conn = sqlite3.connect(f"file:{self.store_path}?mode=ro", uri=True, timeout=5)
        try:
            blob = {(t, s): b for t, s, b in
                    conn.execute("SELECT tag, seq, body FROM blob")}
            slot = dict(conn.execute("SELECT tag, seq FROM commit_slot"))
            floor = dict(conn.execute("SELECT tag, floor FROM retention_floor"))
            meta = dict(conn.execute("SELECT k, v FROM meta"))
        finally:
            conn.close()
        return Tables(blob, slot, floor, meta)

    def wait_for_floor(self, tag: str, timeout: float = 10.0) -> int | None:
        """Block until the sweep records a floor for `tag` (or give up)."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            floor = self.tables().retention_floor.get(tag)
            if floor is not None:
                return floor
            time.sleep(0.1)
        return None
