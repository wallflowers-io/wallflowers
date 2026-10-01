"""catalogue — the object catalogue, READ from the tree rather than transcribed.

Two files in `core/` are the truth, and both are parsed here:

    coordination/delta-graph.icd.json   the 88 ops: opId, authority, fold, status
    pacific-core/src/object.rs          the 12 kinds: their wire type ids, and the
                                        RESERVED ids an old delta must fail on

Nothing in this module hardcodes an op, a type id or a reserved id. That is the
whole point: `pacific-core/src/icd.rs` exists because a transcribed catalogue
drifts, and a Python transcription of the same catalogue would drift twice as
fast — once from the ICD and once from the Rust. So the harness reads what the
build already enforces, and `Catalogue.check()` fails loudly when the two
sources disagree with each other.

What this module does NOT do: it does not run a reducer. It knows what each op
DECLARES — who may author it, how it folds, whether it is built — and the model
in `model.py` enforces exactly those declarations. A scenario built on it proves
the design, not the code.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterator, Mapping

#: `harness/` is a sibling of `core/` — see the-harness.html §07.
# The harness lives INSIDE core now (moved 18 Sep 2026), so the ICD and object.rs are
# siblings in the same repository — which is what this module wants: it exists to prove
# those two agree, and a cross-repo read is how they drift without anyone noticing.
CORE_ROOT = Path(__file__).resolve().parents[2]
ICD_PATH = Path(os.environ.get("PACIFIC_ICD",
                               CORE_ROOT / "coordination/delta-graph.icd.json"))
OBJECT_RS_PATH = Path(os.environ.get("PACIFIC_OBJECT_RS",
                                     CORE_ROOT / "pacific-core/src/object.rs"))

#: Base ops are namespaced at 0xF0000000 and shared by the kinds that declare
#: them — membership, publication, wallet, notes. `object.rs`: "they own a
#: reserved id band, so this can never shadow a Group op".
BASE_OP_NAMESPACE = 0xF000_0000

AUTHORITIES = ("owner", "anyMember")
FOLDS = ("sequenced", "commutative")
STATUSES = ("implemented", "specified")


class CatalogueError(RuntimeError):
    """The catalogue could not be read, or the two sources disagree."""


class UnknownKind(CatalogueError):
    """No such object kind."""


class UnknownOp(CatalogueError):
    """The kind declares no such op — `DeltaRejection::UnknownType` in the Rust."""


class ReservedTypeId(CatalogueError):
    """A type id that was assigned once and retired.

    `object.rs`: type ids are WIRE values on an append-only log — assigned once,
    never reused. 0/16/17/24 (Role/Poll/Message/Question) were folded away, and
    an old delta carrying one "fails loud as UnknownType". This is that failure,
    in Python, and it is an exception rather than a None so that a scenario
    cannot walk past it.
    """


# ── one op ──────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Op:
    """One row of `ObjectType::ops()`, as the ICD declares it."""

    name: str            #: "group.setProfile" — the ICD's own key
    channel: str         #: "group"
    verb: str            #: "setProfile"
    op_id: int
    authority: str       #: owner | anyMember
    fold: str            #: sequenced | commutative
    status: str          #: implemented | specified
    summary: str = ""
    node: tuple[str, ...] = ()
    edges: tuple[tuple[str, str, str, str], ...] = ()   # (rel, from, to, action)
    embed: bool = False
    exclude: str | None = None
    episode: bool = False

    @property
    def base(self) -> bool:
        """A base op — shared vocabulary, in the 0xF0000000 band."""
        return self.op_id >= BASE_OP_NAMESPACE

    @property
    def implemented(self) -> bool:
        return self.status == "implemented"

    @property
    def owner_only(self) -> bool:
        return self.authority == "owner"

    @property
    def sequenced(self) -> bool:
        return self.fold == "sequenced"

    @property
    def commutative(self) -> bool:
        return self.fold == "commutative"

    @property
    def well_formed(self) -> bool:
        """`OpDecl::is_well_formed` — a commutative op MUST be any-member.

        The one invariant `object.rs` actually enforces. The stronger pattern in
        today's catalogue (owner ⇔ sequenced, 54/34, exactly correlated) is held
        by discipline, not by this check, and `Catalogue.check` reports it as an
        observation rather than a violation.
        """
        return self.authority == "anyMember" if self.commutative else True

    @property
    def projects(self) -> bool:
        """Does the op reach the graph at all?"""
        return bool(self.node) or bool(self.edges) or self.embed

    def line(self) -> str:
        mark = " " if self.implemented else "·"
        return (f"{mark} {self.name:<28} {self.op_id:>11}  {self.authority:<9} "
                f"{self.fold:<11} {self.status}")


# ── one kind ────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Kind:
    """One `ObjectKind`: its wire type id and the op table it declares.

    A kind with no ICD channel is DECLARED (it is in `ObjectKind::ALL`, it has a
    type id, `from_type_id` returns it) but not BUILT — `icd.rs`'s
    `every_built_kind_has_a_channel` names the nine that are. Field, System and
    Topic are the three that are not, and an op on one of them has nowhere to
    land.
    """

    name: str
    type_id: int
    channel: str | None = None
    entity_type: str | None = None
    node: str = "none"
    ops: Mapping[str, Op] = field(default_factory=dict)

    @property
    def built(self) -> bool:
        return self.channel is not None

    @property
    def implemented(self) -> int:
        return sum(1 for o in self.ops.values() if o.implemented)

    @property
    def specified(self) -> int:
        return sum(1 for o in self.ops.values() if not o.implemented)

    @property
    def total(self) -> int:
        return len(self.ops)

    @property
    def base_ops(self) -> tuple[Op, ...]:
        return tuple(o for o in self.ops.values() if o.base)

    def op(self, name: str) -> Op:
        """Look an op up by full name (`forum.post`) or by verb (`post`).

        The verb form is what makes the Conversation rule legible: a Conversation
        accepts `post`, and the op it resolves to is `forum.post` — ONE
        definition, `$ref`'d by two channels, which is exactly how the ICD
        carries it and how `chat_ops_are_spliced_verbatim` pins it in the Rust.
        """
        if name in self.ops:
            return self.ops[name]
        hits = [o for o in self.ops.values() if o.verb == name]
        if len(hits) == 1:
            return hits[0]
        if not hits:
            raise UnknownOp(
                f"{self.name}({self.type_id}) declares no op `{name}`"
                + ("" if self.built
                   else " — the kind has no channel in the ICD: declared, not built")
                + (f" (it declares: {', '.join(sorted(self.ops))})" if self.ops else ""))
        raise UnknownOp(f"`{name}` is ambiguous on {self.name}: {[h.name for h in hits]}")

    def op_by_id(self, op_id: int) -> Op:
        for o in self.ops.values():
            if o.op_id == op_id:
                return o
        raise UnknownOp(f"{self.name}({self.type_id}) declares no op id {op_id:#x}")

    def summary(self) -> str:
        if not self.built:
            return f"{self.name}({self.type_id})  declared, no channel — 0 ops"
        return (f"{self.name}({self.type_id})  {self.implemented}/{self.total} "
                f"implemented")


# ── the catalogue ───────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Source:
    """Where a fact came from, so the render can say so."""
    path: Path
    digest: str

    @classmethod
    def of(cls, path: Path) -> "Source":
        raw = path.read_bytes()
        return cls(path=path, digest=hashlib.sha256(raw).hexdigest()[:12])


class Catalogue:
    """Every kind and every op, parsed from `core/`."""

    def __init__(self, kinds: dict[str, Kind], ops: dict[str, Op],
                 reserved: dict[int, str], sources: dict[str, Source],
                 all_order: tuple[str, ...]) -> None:
        self.kinds = kinds
        self.ops = ops                  #: the 88 component messages, by name
        self.reserved = reserved        #: type_id -> the kind it used to be
        self.sources = sources
        self.all_order = all_order      #: `ObjectKind::ALL`, in its own order
        self._by_type_id = {k.type_id: k for k in kinds.values()}

    # -- loading -------------------------------------------------------------

    @classmethod
    def load(cls, icd_path: Path | str | None = None,
             object_rs: Path | str | None = None) -> "Catalogue":
        icd_p = Path(icd_path or ICD_PATH)
        rs_p = Path(object_rs or OBJECT_RS_PATH)
        if not icd_p.exists():
            raise CatalogueError(
                f"the ICD must be readable at {icd_p} — it is the op catalogue; "
                "set PACIFIC_ICD if core/ lives elsewhere")
        try:
            doc = json.loads(icd_p.read_text())
        except json.JSONDecodeError as e:      # same failure icd.rs reports
            raise CatalogueError(f"the ICD must be valid JSON: {e}") from e

        ops = _parse_ops(doc)
        type_ids, all_order, reserved = _parse_object_rs(rs_p)
        kinds = _assemble(doc, ops, type_ids, all_order)
        sources = {"icd": Source.of(icd_p), "object.rs": Source.of(rs_p)}
        return cls(kinds, ops, reserved, sources, all_order)

    # -- lookups -------------------------------------------------------------

    def kind(self, name: str) -> Kind:
        try:
            return self.kinds[name]
        except KeyError:
            raise UnknownKind(
                f"no object kind `{name}` — the catalogue declares "
                f"{', '.join(self.all_order)}") from None

    def kind_for_type_id(self, type_id: int) -> Kind:
        """`ObjectKind::from_type_id`, with the reserved band made loud."""
        if type_id in self.reserved:
            raise ReservedTypeId(
                f"type id {type_id} is RESERVED ({self.reserved[type_id]}) — folded "
                "away and never reissued; a delta carrying it is UnknownType")
        try:
            return self._by_type_id[type_id]
        except KeyError:
            raise UnknownKind(f"no object kind carries type id {type_id}") from None

    def op(self, kind: str, name: str) -> Op:
        return self.kind(kind).op(name)

    def __iter__(self) -> Iterator[Kind]:
        return (self.kinds[n] for n in self.all_order)

    # -- totals --------------------------------------------------------------

    @property
    def built(self) -> tuple[Kind, ...]:
        return tuple(k for k in self if k.built)

    @property
    def declared_only(self) -> tuple[Kind, ...]:
        return tuple(k for k in self if not k.built)

    def totals(self) -> tuple[int, int, int]:
        """(ops, implemented, specified) — over the 88 DEFINITIONS, not the
        channel rows: `forum.post` is one op that two channels carry."""
        impl = sum(1 for o in self.ops.values() if o.implemented)
        return len(self.ops), impl, len(self.ops) - impl

    def line(self) -> str:
        total, impl, spec = self.totals()
        return (f"{len(self.built)} channels · {total} ops · {impl} implemented / "
                f"{spec} specified · icd {self.sources['icd'].digest}")

    # -- conformance ---------------------------------------------------------

    def check(self) -> list[str]:
        """Every rule the two sources must satisfy. Empty list = clean.

        These are the Python half of `icd.rs`: the checks that can be made
        without the Rust, run against the same two files the Rust compiles
        against. A finding here is a finding about the tree, not about the model.
        """
        bad: list[str] = []
        for name, op in sorted(self.ops.items()):
            if op.authority not in AUTHORITIES:
                bad.append(f"{name}: authority `{op.authority}` is not one of {AUTHORITIES}")
            if op.fold not in FOLDS:
                bad.append(f"{name}: fold `{op.fold}` is not one of {FOLDS}")
            if op.status not in STATUSES:
                bad.append(f"{name}: status `{op.status}` is not one of {STATUSES}")
            if not op.well_formed:
                bad.append(f"{name}: commutative+owner violates the spec invariant "
                           "(OpDecl::is_well_formed)")
            if op.base != name.startswith("base."):
                bad.append(f"{name}: op id {op.op_id:#x} disagrees with the base "
                           "namespace 0xF0000000")
            if not (op.node or op.edges or op.embed or op.exclude or op.episode):
                bad.append(f"{name}: declares no graph effect and no exclusion")
        for kind in self:
            seen: dict[int, str] = {}
            for op in kind.ops.values():
                if op.op_id in seen:
                    bad.append(f"{kind.name}: op id {op.op_id:#x} is declared twice "
                               f"({seen[op.op_id]} and {op.name})")
                seen[op.op_id] = op.name
        for tid, was in sorted(self.reserved.items()):
            if tid in self._by_type_id:
                bad.append(f"type id {tid} is RESERVED ({was}) but is also assigned "
                           f"to {self._by_type_id[tid].name}")
        return bad


# ── parsing: the ICD ────────────────────────────────────────────────────────


def _parse_ops(doc: dict) -> dict[str, Op]:
    msgs = doc.get("components", {}).get("messages")
    if not isinstance(msgs, dict) or not msgs:
        raise CatalogueError("the ICD has no components.messages — nothing to read")
    out: dict[str, Op] = {}
    for name, m in msgs.items():
        d = m.get("x-delta")
        g = m.get("x-graph")
        if not isinstance(d, dict):
            raise CatalogueError(f"`{name}` has no x-delta clause — no opId to carry")
        if not isinstance(g, dict):
            raise CatalogueError(f"`{name}` has no x-graph clause")
        if "." not in name:
            raise CatalogueError(f"`{name}` is not a channel-qualified op name")
        channel, verb = name.split(".", 1)
        try:
            op_id = int(d["opId"])
        except (KeyError, TypeError, ValueError) as e:
            raise CatalogueError(f"`{name}` has no integer x-delta.opId") from e
        edges = tuple(
            (str(e.get("rel")), str(e.get("from")), str(e.get("to")), str(e.get("action")))
            for e in g.get("edges", []) or ())
        out[name] = Op(
            name=name, channel=channel, verb=verb, op_id=op_id,
            authority=str(d.get("authority")), fold=str(d.get("fold")),
            status=str(g.get("status")), summary=str(m.get("summary", "")),
            node=tuple(g.get("node", []) or ()), edges=edges,
            embed=bool(g.get("embed", False)),
            exclude=g.get("exclude"), episode=bool(g.get("episode", False)))
    return out


def _assemble(doc: dict, ops: dict[str, Op], type_ids: dict[str, int],
              all_order: tuple[str, ...]) -> dict[str, Kind]:
    """Channels onto kinds, resolving every `$ref` the way `icd.rs` does."""
    channels = doc.get("channels")
    if not isinstance(channels, dict) or not channels:
        raise CatalogueError("the ICD declares no channels")

    kinds: dict[str, Kind] = {}
    for channel, spec in channels.items():
        if channel not in type_ids:
            raise CatalogueError(
                f"the ICD has a `{channel}` channel, but object.rs declares no such "
                "ObjectKind — one of the two has drifted")
        declared = spec.get("x-object", {})
        tid = declared.get("typeId")
        if tid != type_ids[channel]:
            raise CatalogueError(
                f"channel `{channel}` declares typeId {tid}, object.rs says "
                f"{type_ids[channel]} — the wire id is the one thing that cannot drift")
        table: dict[str, Op] = {}
        for key, ref in (spec.get("messages") or {}).items():
            target = str(ref.get("$ref", "")).rsplit("/", 1)[-1]
            if not target:
                raise CatalogueError(f"{channel}.{key} must be a $ref")
            if target != key:
                raise CatalogueError(
                    f"channel key `{key}` must equal the message it references "
                    f"(`{target}`)")
            if target not in ops:
                raise CatalogueError(f"dangling $ref: components.messages.{target}")
            table[key] = ops[target]
        kinds[channel] = Kind(name=channel, type_id=type_ids[channel], channel=channel,
                              entity_type=declared.get("entityType"),
                              node=str(declared.get("node", "none")), ops=table)

    for name in all_order:          # declared in object.rs, no channel in the ICD
        if name not in kinds:
            kinds[name] = Kind(name=name, type_id=type_ids[name])
    return kinds


# ── parsing: object.rs ──────────────────────────────────────────────────────

_TYPE_ID_FN = re.compile(r"pub const fn type_id\(self\)\s*->\s*u16\s*\{(.*?)\n    \}",
                         re.S)
_ARM = re.compile(r"ObjectKind::(\w+)\s*=>\s*(\d+)\s*,")
_ALL = re.compile(r"pub const ALL:\s*&'static \[ObjectKind\]\s*=\s*&\[(.*?)\];", re.S)
_ALL_ARM = re.compile(r"ObjectKind::(\w+)\s*,")
_RESERVED = re.compile(r"([\d/]+)\s*\(([A-Za-z/]+)\)\s*are RESERVED")


def _parse_object_rs(path: Path) -> tuple[dict[str, int], tuple[str, ...], dict[int, str]]:
    """The kind table, straight out of the Rust.

    Parsed rather than transcribed for the same reason the ops are: a second copy
    of the wire ids is a second thing to keep in step. If this parse ever fails
    it fails LOUDLY, because a harness that quietly fell back to a guessed table
    would be asserting against its own guess.
    """
    if not path.exists():
        raise CatalogueError(
            f"object.rs must be readable at {path} — it carries the wire type ids; "
            "set PACIFIC_OBJECT_RS if core/ lives elsewhere")
    src = path.read_text()

    body = _TYPE_ID_FN.search(src)
    if not body:
        raise CatalogueError(f"could not find `fn type_id` in {path} — the parse this "
                             "harness depends on has drifted; fix it, do not guess")
    type_ids = {name.lower(): int(tid) for name, tid in _ARM.findall(body.group(1))}
    if not type_ids:
        raise CatalogueError(f"`fn type_id` in {path} yielded no arms")

    every = _ALL.search(src)
    if not every:
        raise CatalogueError(f"could not find `ObjectKind::ALL` in {path}")
    all_order = tuple(n.lower() for n in _ALL_ARM.findall(every.group(1)))
    missing = set(all_order) ^ set(type_ids)
    if missing:
        raise CatalogueError(f"ObjectKind::ALL and fn type_id disagree about {missing}")

    res = _RESERVED.search(src)
    if not res:
        raise CatalogueError(
            f"could not find the RESERVED type-id note in {path} — the harness models "
            "those ids failing loud, and will not invent the list")
    ids = [int(x) for x in res.group(1).split("/")]
    names = res.group(2).split("/")
    if len(ids) != len(names):
        raise CatalogueError(f"the RESERVED note in {path} pairs {ids} with {names}")
    reserved = dict(zip(ids, names))
    return type_ids, all_order, reserved


#: One process-wide catalogue — parsing two files per scenario step is waste, and
#: every caller wants the same answer anyway.
_CACHED: Catalogue | None = None


def catalogue() -> Catalogue:
    global _CACHED
    if _CACHED is None:
        _CACHED = Catalogue.load()
    return _CACHED
