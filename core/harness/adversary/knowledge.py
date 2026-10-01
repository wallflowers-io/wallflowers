"""knowledge — a thin, local interface the adversaries report against.

Track 2 owns the real harness state objects (RelayState, R2State) and the render.
They do not exist yet, so Track 3 codes against this small interface of its own and
keeps the coupling thin. When `harness/` state modules land, an adversary can be
handed one instead of a live socket; nothing here reaches into Track 2's files.

The one shape the render cares about: each adversary reports what it LEARNED as a
GROWING SET, so a live render can watch the set assemble. `Knowledge` is that set,
plus a couple of labelled facts (names, opened counts) for the panel.
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass
class Knowledge:
    """What one adversary has observed and derived. A growing set of facts."""

    label: str
    facts: set[str] = field(default_factory=set)
    # Structured extras the render can show as distinct rows.
    names_learned: set[str] = field(default_factory=set)
    tags_seen: set[str] = field(default_factory=set)
    plaintext_opened: int = 0
    notes: list[str] = field(default_factory=list)

    def learn(self, fact: str) -> None:
        self.facts.add(fact)

    def saw_tag(self, tag_hex: str) -> None:
        self.tags_seen.add(tag_hex)

    def opened_plaintext(self, name: str | None = None) -> None:
        self.plaintext_opened += 1
        if name:
            self.names_learned.add(name)

    def note(self, msg: str) -> None:
        self.notes.append(msg)

    def as_dict(self) -> dict:
        return {
            "label": self.label,
            "tags_seen": sorted(self.tags_seen),
            "plaintext_opened": self.plaintext_opened,
            "names_learned": sorted(self.names_learned),
            "facts": sorted(self.facts),
            "notes": list(self.notes),
        }
