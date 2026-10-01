"""framework — a case is a narrated sequence of steps with assertions attached.

Cases carry the SAME names as the Rust milestone cases (F13, F14, F15 …) so that
drift between the two implementations is visible rather than comfortable. Each run
declares its backend, and the render puts that on screen: a model run proves the
design, not the code.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Callable


@dataclass
class Check:
    label: str
    ok: bool
    detail: str = ""

    def line(self, width: int = 150) -> str:
        mark = "PASS" if self.ok else "FAIL"
        # A passing check's detail is context; a failing one's is evidence, so only
        # the passing side is truncated.
        detail = self.detail if not self.ok else (
            self.detail if len(self.detail) <= width else self.detail[:width] + "…")
        return f"  [{mark}] {self.label}" + (f"   ({detail})" if detail else "")


@dataclass
class CaseResult:
    name: str
    title: str
    backend: str
    checks: list[Check] = field(default_factory=list)
    steps: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return all(c.ok for c in self.checks)

    def report(self) -> str:
        head = f"{self.name}  {self.title}   [{self.backend}]"
        body = "\n".join(c.line() for c in self.checks)
        notes = "".join(f"\n  note: {n}" for n in self.notes)
        return f"{head}\n{body}{notes}"


class Run:
    """One case in flight. `step` narrates, `check` asserts, both feed the render."""

    def __init__(self, name: str, title: str, backend: str = "model",
                 on_step: Callable[["Run", str], None] | None = None) -> None:
        self.result = CaseResult(name=name, title=title, backend=backend)
        self._on_step = on_step
        self.last_step = ""
        #: Live objects the case wants on screen — `hub`, `bucket`, `budget`,
        #: `adversary`, `people`, `groups`. The render binds these onto its Scene,
        #: so a panel shows the case's OWN state rather than a decorative copy.
        self.state: dict[str, object] = {}

    def step(self, text: str) -> None:
        self.last_step = text
        self.result.steps.append(text)
        if self._on_step:
            self._on_step(self, text)

    def check(self, label: str, ok: bool, detail: str = "") -> bool:
        self.result.checks.append(Check(label, bool(ok), detail))
        if self._on_step:
            self._on_step(self, self.last_step)
        return bool(ok)

    def expose(self, **objects) -> None:
        """Hand the render the objects this case is actually mutating."""
        self.state.update(objects)
        if self._on_step:
            self._on_step(self, self.last_step)

    def note(self, text: str) -> None:
        self.result.notes.append(text)

    def done(self) -> CaseResult:
        return self.result


def report(results: list[CaseResult]) -> tuple[str, bool]:
    """Render a list of case results; returns (text, all_green)."""
    lines, green = [], True
    for r in results:
        lines.append(r.report())
        green = green and r.ok
    passed = sum(len([c for c in r.checks if c.ok]) for r in results)
    total = sum(len(r.checks) for r in results)
    lines.append(f"\n{passed}/{total} checks green across {len(results)} cases")
    return ("\n\n".join(lines), green)
