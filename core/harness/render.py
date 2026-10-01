"""render — one frame per step, `rich.live.Live`, and the honesty rule on screen.

The panel layout is the sketch in the-harness.html §04, with one panel added for
the bucket, because Track 2 has a real R2State to show:

    ┌─ PEOPLE ────────┐┌─ OBJECTS ─────────┐
    ┌─ RELAY ─────────┐┌─ EVE ─────────────┐
    ┌─ BUCKET ────────────────────────────┐
    BACKEND: model — proves the design, not the code

Eve's panel is the one you actually watch. The footer is not decoration: it is
the honesty rule, on screen, every run. A run against modelled devices says so.

PEOPLE is fed by devices that do not exist yet (step 04, ModelDevice) and says so.
OBJECTS is fed by `harness.objects`, whose kinds and op tables are read from
`core/coordination/delta-graph.icd.json` and `pacific-core/src/object.rs` — so the
panel can say `project·22  7/20 implemented` and mean it. A harness that showed a
plausible-looking roster it had not computed would be the exact failure the
backend footer exists to prevent, which is why the footer now carries a second
line naming the catalogue and stating that no reducer runs here.
"""
from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Iterable, Protocol, runtime_checkable

from rich.align import Align
from rich.console import Group
from rich.live import Live
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

from .cases.framework import Run
from .state import DEFAULT_BUDGET_BYTES, Budget, R2State, RelayHub, RelayState

#: The honesty rule from the-harness.html §00, verbatim per backend.
BACKEND_LINE = {
    "model": "BACKEND: model — proves the design, not the code",
    "agent": "BACKEND: agent — drives the real core over stdin/stdout JSON",
}


def backend_line(backend: str) -> str:
    if backend in BACKEND_LINE:
        return BACKEND_LINE[backend]
    if "REAL" in backend:
        return (f"BACKEND: {backend} — the relay, its store and the wire are real; "
                "devices are not modelled here at all")
    return f"BACKEND: {backend}"


# ── what the panels are fed ─────────────────────────────────────────────────


@dataclass
class PersonRow:
    """One device of one person. Filled by step 04; empty until then."""
    person: str
    device: str
    online: bool = True


@dataclass
class GroupRow:
    """A generic group — kept for callers that have no catalogue to hand.

    Superseded by `ObjectRow`, which carries the KIND. A row that cannot say
    which of the twelve kinds it is cannot say what its ops are, which is the
    whole difference the object catalogue makes.
    """
    name: str
    people: int
    leaves: int
    epoch: int


@dataclass
class ObjectRow:
    """One object of one REAL kind, as the OBJECTS panel shows it.

    kind + wire type id, the roster FOLDED TO PEOPLE (A2: two people with two
    devices each is two, not four), the epoch, and the implemented/specified
    split of that kind's op table. "project 7/20 implemented" on screen is worth
    more than a paragraph saying most of the wallet is a spec.
    """
    name: str
    kind: str = "group"
    type_id: int = 18
    people: int = 0
    leaves: int = 0
    epoch: int = 0
    implemented: int = 0
    ops: int = 0
    note: str = ""

    @classmethod
    def of(cls, obj, note: str = "") -> "ObjectRow":
        """Build from a `harness.objects.GroupObject` — duck-typed on purpose, so
        the render keeps knowing nothing about the object model's internals."""
        return cls(name=obj.name, kind=obj.kind.name, type_id=obj.type_id,
                   people=len(obj.people), leaves=len(obj.leaves), epoch=obj.epoch,
                   implemented=obj.kind.implemented, ops=obj.kind.total, note=note)

    def ops_cell(self) -> str:
        if not self.ops:
            return "[red]0 ops — declared, not built[/]"
        spec = self.ops - self.implemented
        colour = "green" if spec == 0 else ("yellow" if self.implemented else "red")
        return f"[{colour}]{self.implemented}/{self.ops} implemented[/]"


@runtime_checkable
class AdversaryView(Protocol):
    """What the EVE panel needs. Track 3's adversaries implement this and are
    rendered without this module knowing anything else about them."""
    name: str

    def rows(self) -> Iterable[tuple[str, str]]:
        """(label, value) lines, in the order they should be shown."""


class MetadataView:
    """The eavesdropper's view, derived from RelayState alone — a placeholder for
    Track 3's real `Eavesdropper`, and an honest one: every line here is something
    the relay's own four tables hand over, with no crypto and no guessing."""

    name = "EVE"

    def __init__(self, state: RelayState, opened: int = 0,
                 names: Iterable[str] = ()) -> None:
        self.state, self.opened, self.names = state, opened, list(names)

    def rows(self) -> list[tuple[str, str]]:
        tags, blobs = self.state.counts()
        order = self.state.interleaving()
        return [
            ("tags seen", str(tags)),
            ("blobs seen", str(blobs)),
            ("bytes seen", str(sum(n for _, _, n in self.state.sizes()))),
            ("plaintext opened", str(self.opened) + ("  ← intro" if self.opened else "")),
            ("names learned", ", ".join(self.names) or "—"),
            ("ordering across tags", "TOTAL" if len(order) > 1 else "—"),
        ]


@dataclass
class Scene:
    """Everything on screen. Track 3 and Track 4 fill their own fields."""
    backend: str = "model"
    title: str = ""
    step: str = ""
    people: list[PersonRow] = field(default_factory=list)
    groups: list[GroupRow] = field(default_factory=list)
    #: Objects of REAL kinds. Takes precedence over `groups` when present.
    objects: list[ObjectRow] = field(default_factory=list)
    #: One line naming where the op catalogue was read from, and the honest
    #: implemented/specified split across it. Empty when no catalogue is loaded.
    catalogue: str = ""
    relay: RelayState | None = None
    hub: RelayHub | None = None
    bucket: R2State | None = None
    budget: Budget | None = None
    adversary: AdversaryView | None = None
    checks: list[tuple[bool, str]] = field(default_factory=list)

    def from_run(self, run: Run) -> "Scene":
        self.title = f"{run.result.name}  {run.result.title}"
        self.backend = run.result.backend
        self.step = run.last_step
        self.checks = [(c.ok, c.label) for c in run.result.checks]
        for key, value in run.state.items():   # whatever the case exposed
            setattr(self, key, value)
        return self


# ── panels ──────────────────────────────────────────────────────────────────


def _kv(rows: Iterable[tuple[str, str]], key_style: str = "") -> Table:
    """Key left, value left just past it — the shape the §04 sketch draws."""
    rows = list(rows)
    width = max((len(k) for k, _ in rows), default=0) + 1
    t = Table.grid(padding=(0, 1))
    t.add_column(style=key_style, no_wrap=True, min_width=width)
    t.add_column(overflow="ellipsis")
    for k, v in rows:
        t.add_row(k, v)
    return t


def _missing(what: str) -> Align:
    return Align.left(Text(what, style="dim italic"))


def people_panel(scene: Scene) -> Panel:
    if not scene.people:
        body = _missing("devices arrive with step 04 — ModelDevice")
    else:
        t = Table.grid(padding=(0, 2))
        for c in range(3):
            t.add_column(no_wrap=True)
        last = None
        for row in scene.people:
            t.add_row("" if row.person == last else row.person, row.device,
                      "[green]● online[/]" if row.online else "[dim]○ offline[/]")
            last = row.person
        body = t
    return Panel(body, title="PEOPLE", title_align="left", border_style="cyan")


def objects_panel(scene: Scene) -> Panel:
    """OBJECTS — kind, wire type id, roster folded to people, epoch, op split."""
    if scene.objects:
        t = Table.grid(padding=(0, 1))
        t.add_column(no_wrap=True)                 # name
        t.add_column(no_wrap=True)                 # kind · wire type id
        t.add_column(no_wrap=True)                 # roster, FOLDED TO PEOPLE
        t.add_column(overflow="fold")              # implemented / specified
        for o in scene.objects:
            person = "person" if o.people == 1 else "people"
            leaf = "leaf" if o.leaves == 1 else "leaves"
            t.add_row(o.name,
                      f"[bold]{o.kind}[/]·{o.type_id}",
                      f"{o.people} {person} · {o.leaves} {leaf} · e{o.epoch}",
                      o.ops_cell() + (f" [dim]{o.note}[/]" if o.note else ""))
        return Panel(t, title="OBJECTS", title_align="left", border_style="cyan")
    if scene.groups:
        t = Table.grid(padding=(0, 2))
        t.add_column(no_wrap=True)
        t.add_column(no_wrap=True)
        for g in scene.groups:
            person = "person" if g.people == 1 else "people"
            t.add_row(g.name, f"{g.people} {person} · {g.leaves} leaves · e{g.epoch}")
        return Panel(t, title="GROUPS", title_align="left", border_style="cyan")
    return Panel(_missing("objects arrive with a catalogue — kind, type id, op split"),
                 title="OBJECTS", title_align="left", border_style="cyan")


#: The old name, for callers written before the catalogue landed.
groups_panel = objects_panel


def relay_panel(scene: Scene) -> Panel:
    state = scene.relay or (scene.hub.store if scene.hub else None)
    if state is None:
        return Panel(_missing("no relay in this scene"), title="RELAY",
                     title_align="left", border_style="magenta")
    tags, blobs = state.counts()
    rows = [("blob", f"{blobs} rows"),
            ("commit_slot", f"{len(state.commit_slot)} tags"),
            ("floors", str(len(state.retention_floor))),
            ("last_seq", f"{state.resume_seq()}  (global)")]
    if scene.hub is not None:
        m = scene.hub.metrics
        rows += [("commits refused", str(m.rejected_commits)),
                 ("gaps announced", str(m.gaps_announced))]
    return Panel(_kv(rows), title="RELAY", title_align="left", border_style="magenta")


def adversary_panel(scene: Scene) -> Panel:
    view = scene.adversary
    if view is None and (scene.relay or scene.hub):
        view = MetadataView(scene.relay or scene.hub.store)
    if view is None:
        return Panel(_missing("no observer in this scene"), title="EVE",
                     title_align="left", border_style="red")
    return Panel(_kv(view.rows()), title=view.name, title_align="left",
                 border_style="red")


def bucket_panel(scene: Scene) -> Panel:
    if scene.bucket is None and scene.budget is None:
        return Panel(_missing("no bucket in this scene"), title="BUCKET",
                     title_align="left", border_style="yellow")
    rows: list[tuple[str, str]] = []
    if scene.bucket is not None:
        rows += [("objects", f"{len(scene.bucket)} keys (sha256)"),
                 ("bytes", f"{scene.bucket.total_bytes():,}")]
    if scene.budget is not None:
        b = scene.budget
        pct = (b.committed() * 100 // b.max_bytes) if b.max_bytes else 0
        rows += [("measured + in flight", f"{b.committed():,} / {b.max_bytes:,}  ({pct}%)"),
                 ("meter", "stale — refusing" if b.measured_at == 0 else "measured")]
    else:
        rows += [("budget", f"{DEFAULT_BUDGET_BYTES:,} default")]
    return Panel(_kv(rows), title="BUCKET", title_align="left", border_style="yellow")


def checks_panel(scene: Scene) -> Panel:
    if not scene.checks:
        body = _missing("no assertions yet")
    else:
        t = Table.grid(padding=(0, 1))
        t.add_column(no_wrap=True)
        t.add_column(overflow="ellipsis")
        for ok, label in scene.checks[-6:]:
            t.add_row("[green]PASS[/]" if ok else "[bold red]FAIL[/]", label)
        body = t
    return Panel(body, title="ASSERTIONS", title_align="left", border_style="green")


def frame(scene: Scene) -> Group:
    """One frame: header, five panels, the assertions tail, and the footer."""
    top = Table.grid(expand=True)
    # OBJECTS carries four columns to PEOPLE's three and is the panel with
    # something to say before step 04 lands, so it gets the wider half.
    top.add_column(ratio=2)
    top.add_column(ratio=3)
    top.add_row(people_panel(scene), objects_panel(scene))
    top.add_row(relay_panel(scene), adversary_panel(scene))

    header = Text.assemble((scene.title or "PACIFIC · the harness", "bold"),
                           ("   " + scene.step if scene.step else "", "dim"))
    failed = any(not ok for ok, _ in scene.checks)
    footer = Text(backend_line(scene.backend),
                  style="bold yellow" if scene.backend == "model" else "bold green")
    tally = Text.assemble(
        (f"{sum(1 for ok, _ in scene.checks if ok)}/{len(scene.checks)} checks",
         "bold red" if failed else "bold green"))
    lines = [Text.assemble(footer, "        ", tally)]
    if scene.catalogue:
        # The second half of the honesty rule, and the one the object model
        # makes necessary: the harness enforces what the catalogue DECLARES.
        # It does not run a reducer, and a richer object model must not be
        # allowed to read as a realer one.
        lines.append(Text(f"CATALOGUE: {scene.catalogue}", style="dim"))
        lines.append(Text("           declarations enforced; no reducer runs here",
                          style="dim italic"))
    return Group(header, top, bucket_panel(scene), checks_panel(scene), *lines,
                 Text(""))


# ── the live loop ───────────────────────────────────────────────────────────


class Dashboard:
    """`rich.live.Live`, one frame per step. Pass `on_step` into a case."""

    def __init__(self, scene: Scene | None = None, pace: float = 0.0,
                 refresh_per_second: int = 12) -> None:
        self.scene = scene or Scene()
        self.pace = pace
        self.live = Live(frame(self.scene), refresh_per_second=refresh_per_second,
                         transient=False)

    def __enter__(self) -> "Dashboard":
        self.live.__enter__()
        return self

    def __exit__(self, *exc) -> None:
        self.live.__exit__(*exc)

    def update(self, scene: Scene | None = None) -> None:
        if scene is not None:
            self.scene = scene
        self.live.update(frame(self.scene))

    def on_step(self, run: Run, _text: str) -> None:
        """The callback a case calls on every step and every check."""
        self.update(self.scene.from_run(run))
        if self.pace:
            time.sleep(self.pace)

    def bind(self, **fields) -> "Dashboard":
        for k, v in fields.items():
            setattr(self.scene, k, v)
        return self
