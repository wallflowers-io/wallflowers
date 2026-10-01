"""protocol — one request, one answer, and a device behind each.

The verb protocol of docs/multi-platform-harness.html §03, spoken to real
devices. Every device is its own process — a headless Chromium per browser
device, a simulator per phone — so nothing about one device is shared with
another in-process, and concurrency is a thing the harness HAS rather than a
thing it models.

    → {"id": 41, "verb": "rpc", "args": {"m": "store.commit", "a": {…}}}
    ← {"id": 41, "ok": true, "v": {…}}
    ← {"id": 42, "ok": false, "e": "store.commit: not paired"}

The value rides under `v` rather than flattened beside `id` and `ok`. The
keyholder's own replies already have that shape, and a flattened answer would
collide the first time a result carried a field named `ok`.

ASSERTIONS DO NOT LIVE HERE. An agent reports what its device did and what it
holds. Whether that counts as convergence is the runner's call, made once and
applied to every platform alike — §01's "assertions belong to the runner, not
the agent".
"""
from __future__ import annotations

import asyncio
import itertools
import json
import os
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


class AgentError(RuntimeError):
    """A verb the device attempted and failed. Carries the answer as evidence."""

    def __init__(self, answer: "Answer") -> None:
        super().__init__(f"{answer.device} {answer.verb}: {answer.e}")
        self.answer = answer


class Unsupported(RuntimeError):
    """A verb this platform cannot perform, raised with the reason.

    A platform that cannot do a thing says so. It never approximates it: a phone
    asked for a DM transcript it has no way to write out does not answer with
    the nearest thing it can reach."""


@dataclass
class Answer:
    device: str
    verb: str
    ok: bool
    v: Any = None
    e: str | None = None
    ms: int = 0
    #: True when the platform refused the verb outright, as opposed to trying it
    #: and failing. The two are different news and are reported differently.
    unsupported: bool = False

    def unwrap(self) -> Any:
        if not self.ok:
            raise AgentError(self)
        return self.v

    def line(self, width: int = 160) -> str:
        if self.ok:
            body = json.dumps(self.v, default=str, ensure_ascii=False)
            if len(body) > width:
                body = body[:width] + "…"
            return f"{self.device:<16} {self.verb:<14} ok   {self.ms:>6}ms  {body}"
        mark = "SKIP" if self.unsupported else "FAIL"
        return f"{self.device:<16} {self.verb:<14} {mark} {self.ms:>6}ms  {self.e}"


class Agent:
    """One device. Subclasses implement verbs as `async def v_<verb>(**args)`."""

    platform = "?"
    #: Verbs from the shared vocabulary this platform cannot perform, and why.
    CANNOT: dict[str, str] = {}

    def __init__(self, name: str) -> None:
        self.name = name

    async def start(self) -> "Agent":
        return self

    async def stop(self) -> None:
        return None

    def backend(self) -> str:
        """One line for the footer: what is real on this device."""
        return self.platform

    def verbs(self) -> list[str]:
        return sorted(n[2:] for n in dir(self) if n.startswith("v_"))

    async def do(self, verb: str, **args: Any) -> Answer:
        t0 = time.monotonic()

        def answer(**kw: Any) -> Answer:
            return Answer(self.name, verb, ms=int((time.monotonic() - t0) * 1000), **kw)

        if verb in self.CANNOT:
            return answer(ok=False, unsupported=True,
                          e=f"{self.platform} cannot {verb}: {self.CANNOT[verb]}")
        fn = getattr(self, "v_" + verb, None)
        if fn is None:
            return answer(ok=False, unsupported=True,
                          e=f"{self.platform} has no verb {verb!r} "
                            f"(it has: {', '.join(self.verbs())})")
        try:
            return answer(ok=True, v=await fn(**args))
        except Unsupported as ex:
            return answer(ok=False, unsupported=True, e=str(ex))
        except Exception as ex:  # noqa: BLE001 — the answer IS the report
            return answer(ok=False, e=f"{type(ex).__name__}: {ex}")


class SidecarDied(RuntimeError):
    pass


class Sidecar:
    """A child process speaking one JSON object per line on stdin and stdout.

    The first line it writes must be `{"t": "ready", …}`. After that every line
    is an answer, matched to its request by `id`. stderr is the child's own and
    goes to a log file, which is quoted back if the child dies — a device that
    vanished mid-verb should say why, not time out.
    """

    #: A whole world is one line, and the stoma seed on its own is 31 KB.
    LINE_LIMIT = 32 * 1024 * 1024

    def __init__(self, argv: list[str], *, cwd: str | os.PathLike | None = None,
                 env: dict[str, str] | None = None, log: Path | None = None) -> None:
        self.argv = argv
        self.cwd = cwd
        self.env = {**os.environ, **(env or {})}
        self.log_path = Path(log) if log else None
        self.proc: asyncio.subprocess.Process | None = None
        self.hello: dict[str, Any] = {}
        self._ids = itertools.count(1)
        self._pending: dict[int, asyncio.Future] = {}
        self._ready: asyncio.Future | None = None
        self._reader: asyncio.Task | None = None
        self._log = None

    async def start(self, timeout: float = 60.0) -> dict[str, Any]:
        loop = asyncio.get_running_loop()
        self._ready = loop.create_future()
        if self.log_path:
            self.log_path.parent.mkdir(parents=True, exist_ok=True)
            self._log = open(self.log_path, "ab")
        self.proc = await asyncio.create_subprocess_exec(
            *self.argv, cwd=self.cwd, env=self.env, limit=self.LINE_LIMIT,
            stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
            stderr=self._log if self._log else asyncio.subprocess.DEVNULL)
        self._reader = asyncio.create_task(self._read())
        self.hello = await asyncio.wait_for(asyncio.shield(self._ready), timeout)
        return self.hello

    async def _read(self) -> None:
        assert self.proc and self.proc.stdout
        try:
            while True:
                raw = await self.proc.stdout.readline()
                if not raw:
                    break
                try:
                    msg = json.loads(raw)
                except json.JSONDecodeError:
                    continue          # a stray print is the child's bug, not a reply
                if msg.get("t") == "ready" and self._ready and not self._ready.done():
                    self._ready.set_result(msg)
                    continue
                fut = self._pending.pop(msg.get("id"), None)
                if fut and not fut.done():
                    fut.set_result(msg)
        except Exception:             # noqa: BLE001 — an unreadable stream is a dead one
            pass
        finally:
            if self.proc.returncode is None and self.proc.stdout.at_eof() is False:
                self.proc.kill()
            code = await self.proc.wait()
            err = SidecarDied(f"{self.argv[0]} exited {code}\n{self.log_tail()}")
            if self._ready and not self._ready.done():
                self._ready.set_exception(err)
            for fut in self._pending.values():
                if not fut.done():
                    fut.set_exception(err)
            self._pending.clear()

    async def call(self, verb: str, args: dict[str, Any] | None = None,
                   timeout: float = 90.0) -> dict[str, Any]:
        if not self.proc or self.proc.returncode is not None or not self.proc.stdin:
            raise SidecarDied(f"{self.argv[0]} is not running\n{self.log_tail()}")
        rid = next(self._ids)
        fut = asyncio.get_running_loop().create_future()
        self._pending[rid] = fut
        line = json.dumps({"id": rid, "verb": verb, "args": args or {}}) + "\n"
        self.proc.stdin.write(line.encode())
        await self.proc.stdin.drain()
        try:
            return await asyncio.wait_for(fut, timeout)
        finally:
            self._pending.pop(rid, None)

    async def stop(self, grace: float = 15.0) -> None:
        if self.proc and self.proc.returncode is None:
            try:
                # EOF on stdin is the child's cue to close its browser cleanly.
                self.proc.stdin.close()
            except Exception:         # noqa: BLE001
                pass
            try:
                await asyncio.wait_for(self.proc.wait(), grace)
            except asyncio.TimeoutError:
                self.proc.kill()
                await self.proc.wait()
        if self._reader:
            await asyncio.gather(self._reader, return_exceptions=True)
        if self._log:
            self._log.close()
            self._log = None

    def log_tail(self, lines: int = 30) -> str:
        if not self.log_path or not self.log_path.exists():
            return ""
        text = self.log_path.read_text(errors="replace").splitlines()
        return "\n".join(text[-lines:])
