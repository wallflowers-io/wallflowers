"""runner — drives devices, and waits on what they hold rather than on the clock.

The runner is the only place a device's answers are judged. It owns the
transcript of every request and answer, and it owns the two kinds of waiting a
scenario needs — neither of which is a duration:

    until(pred, read, who)   poll `read` on each device until `pred` holds over
                             the answers — "B has A's message". Arrival.
    settle(read, who)        poll until every device's answer has stopped
                             changing for `quiet_rounds` rounds — quiescence.

`settle` is the Rust harness's corrected rule, lifted off the relay and onto the
answers: a round in which nothing a device HOLDS changed, twice running. Not
"nothing arrived", which is the rule that deadlocked there — `sync_once` returns
the whole transcript, so it is never empty. A poll interval exists so the loop
does not spin; the condition is always state, and both waits give up at a
timeout that fails loudly with the last answers attached rather than passing
quietly.

Quiescence alone can be satisfied too early, by two rounds in which nothing has
started arriving yet. So a scenario waits `until` the thing it is about has
landed, and THEN `settle`s, which is the difference between "it arrived" and
"and nothing else was still on its way".
"""
from __future__ import annotations

import asyncio
import json
import time
from dataclasses import dataclass, field
from typing import Any, Awaitable, Callable

from .protocol import Agent, Answer

Read = Callable[[Agent], Awaitable[Any]]


@dataclass
class Waited:
    ok: bool
    rounds: int
    ms: int
    answers: dict[str, Any] = field(default_factory=dict)
    why: str = ""

    def detail(self, width: int = 400) -> str:
        body = json.dumps(self.answers, default=str, ensure_ascii=False)
        if len(body) > width:
            body = body[:width] + "…"
        head = f"{self.rounds} rounds, {self.ms}ms"
        return f"{head}; {self.why}; last answers {body}" if not self.ok else head


class Runner:
    def __init__(self) -> None:
        self.devices: dict[str, Agent] = {}
        self.transcript: list[Answer] = []
        #: Called with every answer as it lands — the render hangs off this.
        self.on_answer: Callable[[Answer], None] | None = None

    def add(self, *agents: Agent) -> None:
        for a in agents:
            if a.name in self.devices:
                raise ValueError(f"two devices named {a.name}")
            self.devices[a.name] = a

    async def start(self, *names: str) -> None:
        """Start devices concurrently. They are separate processes; there is no
        reason for a phone's boot to wait on a browser's."""
        todo = [self.devices[n] for n in (names or self.devices)]
        await asyncio.gather(*(a.start() for a in todo))

    async def do(self, device: str, verb: str, **args: Any) -> Answer:
        answer = await self.devices[device].do(verb, **args)
        self.transcript.append(answer)
        if self.on_answer:
            self.on_answer(answer)
        return answer

    async def must(self, device: str, verb: str, **args: Any) -> Any:
        return (await self.do(device, verb, **args)).unwrap()

    async def poll(self, read: Read, who: list[str]) -> dict[str, Any]:
        """One round: `read` on every named device at once."""
        agents = [self.devices[n] for n in who]
        got = await asyncio.gather(*(read(a) for a in agents), return_exceptions=True)
        return {n: ({"error": f"{type(v).__name__}: {v}"} if isinstance(v, BaseException) else v)
                for n, v in zip(who, got)}

    async def until(self, pred: Callable[[dict[str, Any]], bool], read: Read,
                    who: list[str], *, what: str, interval: float = 0.4,
                    timeout: float = 60.0) -> Waited:
        t0 = time.monotonic()
        rounds, last = 0, {}
        while True:
            rounds += 1
            last = await self.poll(read, who)
            ms = int((time.monotonic() - t0) * 1000)
            try:
                if pred(last):
                    return Waited(True, rounds, ms, last)
            except Exception:         # noqa: BLE001 — a predicate over a half-arrived answer
                pass
            if time.monotonic() - t0 > timeout:
                return Waited(False, rounds, ms, last,
                              why=f"timed out after {timeout:.0f}s waiting for {what}")
            await asyncio.sleep(interval)

    async def settle(self, read: Read, who: list[str], *, quiet_rounds: int = 2,
                     interval: float = 0.5, timeout: float = 60.0) -> Waited:
        t0 = time.monotonic()
        rounds, keys, last = 0, [], {}
        while True:
            rounds += 1
            last = await self.poll(read, who)
            keys.append(json.dumps(last, sort_keys=True, default=str))
            ms = int((time.monotonic() - t0) * 1000)
            if len(keys) >= quiet_rounds and len(set(keys[-quiet_rounds:])) == 1:
                return Waited(True, rounds, ms, last)
            if time.monotonic() - t0 > timeout:
                return Waited(False, rounds, ms, last,
                              why=f"still changing after {timeout:.0f}s")
            await asyncio.sleep(interval)

    def footer(self) -> list[str]:
        return [f"BACKEND: {name} — {a.backend()}" for name, a in self.devices.items()]

    async def close(self) -> None:
        await asyncio.gather(*(a.stop() for a in self.devices.values()),
                             return_exceptions=True)
