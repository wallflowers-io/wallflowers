"""model — a GroupObject that is one of the REAL kinds, folding by the REAL rules.

The harness's Group used to be generic: a name, a roster, an epoch. Generic is
what lets a scenario say something it has not earned — "ada posts to the group"
is true of anything. Here an object is a KIND with a wire type id and an op table
read from the catalogue, so `notes.author(phone, "setProfile")` either is a real
op on a real kind or fails at the point of authoring.

WHAT IS ENFORCED, and each one is a rule the catalogue states rather than a rule
this module invents:

  * an owner-authority op authored by a non-owner is UNAUTHORIZED;
  * a commutative op MUST be any-member (`OpDecl::is_well_formed`) — checked when
    the object is constructed, which is where the Rust checks it too ("call this
    for every op at registration to fail loudly");
  * a SPECIFIED-ONLY op refuses with `not implemented` and changes nothing;
  * a Conversation(26) accepts forum.post / react / receipt / vote and NOTHING
    else — it has no ops of its own, it `$ref`s the chat band;
  * a delta for another kind, an unknown op, or a RESERVED type id is UnknownType.

THE TWO FOLDS, modelled apart because they differ in kind and not in degree:

  SEQUENCED   an owner-stamped total order on (epoch, seq), hash-chained by
              `prev`. A gap, a bad `prev` or a stale epoch is refused LOUDLY; two
              different deltas at one position is ForkDetected, a divergence halt.
              State is a function of the ORDER.
  COMMUTATIVE an OR-set, deduped per (author, DeltaId) and folded in canonical
              (gen, author, DeltaId) order, per-author last-writer-wins on a slot.
              State is a function of the SET. Arrival order cannot be observed.

HONESTY, because a richer model is exactly where a harness starts to look more
real than it is:

  * no reducer runs here. The fold applies what the ICD's `x-graph` clause
    DECLARES each op does to the read model — which field it writes, which edge
    it mints — and never invents the value mapping, which belongs to the
    projector in another repo entirely.
  * the DeltaId is a SHA-256 over canonical JSON, not over the canonical CBOR
    `Delta::id()` hashes. Ids here are stable and comparable to each other. They
    are NOT comparable to the Rust's, and nothing should pretend otherwise.
  * "not implemented" is not a `DeltaRejection` variant. In the tree an op with no
    reducer surfaces as a milestone-tagged NotImplemented at the verb boundary
    (`lib.rs`), one layer above the fold. The harness refuses it before the fold
    for the same reason: there is nothing behind it.
"""
from __future__ import annotations

import hashlib
import itertools
import json
from dataclasses import dataclass, field
from enum import Enum
from typing import Iterable, Mapping, Sequence

from .catalogue import (Catalogue, CatalogueError, Kind, Op, ReservedTypeId,
                        UnknownOp, catalogue)

#: `coordinator::GENESIS_PREV` — what the first delta on a spine chains to.
GENESIS_PREV = "00" * 32


class SpecViolation(CatalogueError):
    """An op table that breaks the one invariant `object.rs` enforces."""


# ── who ─────────────────────────────────────────────────────────────────────


@dataclass(frozen=True, order=True)
class Leaf:
    """One device of one person — one MLS leaf in the object's roster.

    The distinction this type exists to keep: MEMBERSHIP is per leaf (a device
    holds keys or it does not), AUTHORITY is per person (the owner is a person,
    and the owner's second device is still the owner). A2 in the-harness.html is
    that distinction as an assertion — two people with two devices each is two
    rows in the roster panel and four leaves in the group.
    """

    person: str
    device: str = "device"

    def __str__(self) -> str:
        return f"{self.person}/{self.device}"

    @classmethod
    def parse(cls, who: "Leaf | str") -> "Leaf":
        if isinstance(who, Leaf):
            return who
        person, _, device = str(who).partition("/")
        return cls(person=person, device=device or "device")


# ── the rejection taxonomy ──────────────────────────────────────────────────


class Rejection(Enum):
    """`DeltaRejection`, plus the one refusal that lives a layer above it.

    Values are the Rust variant names so a divergence reads as a divergence
    rather than as a translation.
    """

    UNAUTHORIZED = "Unauthorized"
    PRECONDITION_FAILED = "PreconditionFailed"
    MALFORMED_ARGS = "MalformedArgs"
    CHAIN_BROKEN = "ChainBroken"
    FORK_DETECTED = "ForkDetected"
    STALE_EPOCH = "StaleEpoch"
    UNKNOWN_TYPE = "UnknownType"
    SENDER_MISMATCH = "SenderMismatch"
    DUPLICATE = "Duplicate"
    #: NOT a DeltaRejection variant — see the module docstring. An op the
    #: catalogue marks `specified` has no reducer to reach.
    NOT_IMPLEMENTED = "NotImplemented"

    def __str__(self) -> str:
        return self.value


@dataclass(frozen=True)
class Verdict:
    """What one delivery did. Truthy iff the delta was newly accepted."""

    accepted: bool
    rejection: Rejection | None = None
    why: str = ""
    duplicate: bool = False

    def __bool__(self) -> bool:
        return self.accepted

    @property
    def refused(self) -> bool:
        return self.rejection is not None

    def line(self) -> str:
        if self.accepted:
            return "accepted"
        if self.duplicate:
            return "duplicate (idempotent no-op)"
        return f"{self.rejection}: {self.why}"


ACCEPTED = Verdict(True)
DUPLICATE = Verdict(False, duplicate=True, why="this exact delta was already folded")


def _refuse(r: Rejection, why: str) -> Verdict:
    return Verdict(False, rejection=r, why=why)


# ── one delta ───────────────────────────────────────────────────────────────


def _canon(value):
    """Canonical, hashable arg values. A dict arg would make two deltas with the
    same content hash differently depending on insertion order."""
    if isinstance(value, Mapping):
        return tuple(sorted((str(k), _canon(v)) for k, v in value.items()))
    if isinstance(value, (list, tuple)):
        return tuple(_canon(v) for v in value)
    return value


@dataclass(frozen=True)
class Delta:
    """One authored delta: the envelope, minus the author.

    The author is NOT in the id, exactly as in `coordinator::Delta` — authorship
    comes from the MLS-authenticated transport and never rides on the wire, so it
    cannot be part of what the wire hashes.
    """

    type_id: int
    op: Op
    args: tuple = ()
    epoch: int = 0
    prev: str = GENESIS_PREV
    seq: int | None = None
    gen: int | None = None
    #: The commutative LWW slot this delta writes. The ICD does not declare slots
    #: (the `x-graph` clause of a commutative op is almost always an exclusion),
    #: so the SCENARIO names it and the harness does not guess. `None` means
    #: add-only: the delta is its own slot, and nothing it can ever overwrite.
    slot: str | None = None

    @property
    def op_id(self) -> int:
        return self.op.op_id

    @property
    def name(self) -> str:
        return self.op.name

    @property
    def sequenced(self) -> bool:
        return self.op.sequenced

    def canonical_bytes(self) -> bytes:
        return json.dumps(
            {"type_id": self.type_id, "op_id": self.op_id, "op_version": 1,
             "args": [[k, v] for k, v in self.args], "epoch": self.epoch,
             "prev": self.prev, "seq": self.seq, "gen": self.gen,
             "slot": self.slot},
            sort_keys=True, separators=(",", ":"), default=str).encode()

    @property
    def id(self) -> str:
        return hashlib.sha256(self.canonical_bytes()).hexdigest()

    @property
    def short(self) -> str:
        return self.id[:8]

    @property
    def pos(self) -> tuple[int, int] | None:
        return None if self.seq is None else (self.epoch, self.seq)

    def line(self) -> str:
        where = f"e{self.epoch}·s{self.seq}" if self.sequenced else f"gen{self.gen}"
        return f"{self.name:<26} {where:<10} {self.short}"


# ── the folded state ────────────────────────────────────────────────────────


@dataclass(frozen=True)
class NodeWrite:
    """Which op last wrote one declared graph field.

    Deliberately not a VALUE. The ICD declares that `group.setProfile` writes
    `label` and `type`; how the args become those values is the projector's job,
    it lives in arc-resolver, and a harness that made one up would be asserting
    against its own invention.
    """

    field: str
    op: str
    delta: str

    def line(self) -> str:
        return f"{self.field} ← {self.op} ({self.delta})"


@dataclass(frozen=True)
class EdgeWrite:
    rel: str
    frm: str
    to: str
    action: str      # mint | invalidate | annotate
    op: str
    delta: str

    def line(self) -> str:
        return f"{self.action} {self.rel}: {self.frm} → {self.to}  ({self.op})"


@dataclass(frozen=True)
class Entry:
    """One survivor of the OR-set, after per-(author, slot) last-writer-wins."""

    author: str
    slot: str
    gen: int
    delta: str
    op: str
    args: tuple = ()

    def line(self) -> str:
        slot = self.slot if len(self.slot) <= 24 else self.slot[:8] + "…(own id)"
        return f"{self.author:<12} {self.op:<20} gen{self.gen:<4} {slot}"


@dataclass
class ObjectState:
    """The compiled projection: the spine folded first, then the OR-set."""

    node: dict[str, NodeWrite] = field(default_factory=dict)
    edges: list[EdgeWrite] = field(default_factory=list)
    spine: list[str] = field(default_factory=list)     # op names in total order
    entries: list[Entry] = field(default_factory=list)  # canonical order
    inert: list[str] = field(default_factory=list)

    def digest(self) -> str:
        """A hash of the whole projection. Two replicas agree iff this agrees."""
        blob = json.dumps(
            {"node": {k: [v.op, v.delta] for k, v in sorted(self.node.items())},
             "edges": [[e.action, e.rel, e.frm, e.to, e.delta] for e in self.edges],
             "spine": self.spine,
             "entries": [[e.author, e.slot, e.gen, e.delta] for e in self.entries]},
            sort_keys=True, separators=(",", ":")).encode()
        return hashlib.sha256(blob).hexdigest()[:16]


# ── the object ──────────────────────────────────────────────────────────────


def validate_ops(kind: Kind) -> None:
    """`OpDecl::is_well_formed` over a whole table, at registration.

    "a COMMUTATIVE op MUST be ANY_MEMBER authority — a broadcast op can't be
    owner-gated before it propagates." Today's catalogue satisfies this for all
    88 ops; the check is here so that the day one does not, an object of that
    kind cannot be constructed.
    """
    bad = [o.name for o in kind.ops.values() if not o.well_formed]
    if bad:
        raise SpecViolation(
            f"{kind.name}({kind.type_id}) declares commutative owner-only ops "
            f"{bad} — a broadcast op cannot be owner-gated before it propagates")


class GroupObject:
    """An object of one real kind: an MLS group of N leaves plus a delta log.

    Every object is a group of 1 that grew — `add_leaf` is the whole of
    "collaboration", and the epoch bump that comes with it is what makes a
    removed leaf go deaf.
    """

    def __init__(self, name: str, kind: "str | int | Kind", owner: str,
                 roster: Iterable["Leaf | str"] = (), epoch: int = 0,
                 cat: Catalogue | None = None) -> None:
        self.cat = cat or catalogue()
        self.kind = self._resolve(kind)
        validate_ops(self.kind)
        self.name = name
        self.owner = owner
        self._roster: list[Leaf] = []
        for who in roster:
            self._roster.append(Leaf.parse(who))
        self.epoch = epoch
        self.last_rekey = ""
        #: accepted sequenced deltas, in (epoch, seq) order
        self._spine: list[tuple[tuple[int, int], Delta, Leaf]] = []
        #: (epoch, seq) -> DeltaId, the fork-detection index
        self._by_pos: dict[tuple[int, int], str] = {}
        #: accepted commutative deltas, folded in canonical order at read time
        self._orset: list[tuple[Delta, Leaf]] = []
        self._seen: set[tuple[str, str]] = set()
        #: every refusal, for the render and for the case report
        self.refusals: list[tuple[Delta, Leaf, Verdict]] = []
        if owner and owner not in self.people:
            raise ValueError(
                f"{name}: owner `{owner}` holds no leaf in the roster "
                f"{[str(l) for l in self._roster]} — an owner with no device cannot "
                "sequence anything")

    def _resolve(self, kind: "str | int | Kind") -> Kind:
        if isinstance(kind, Kind):
            return kind
        if isinstance(kind, int):
            return self.cat.kind_for_type_id(kind)   # ReservedTypeId is loud
        return self.cat.kind(kind)

    # -- the roster ----------------------------------------------------------

    @property
    def leaves(self) -> tuple[Leaf, ...]:
        return tuple(self._roster)

    @property
    def people(self) -> tuple[str, ...]:
        """The roster FOLDED TO PEOPLE, first appearance order (A2)."""
        out: list[str] = []
        for leaf in self._roster:
            if leaf.person not in out:
                out.append(leaf.person)
        return tuple(out)

    @property
    def type_id(self) -> int:
        return self.kind.type_id

    def holds(self, who: "Leaf | str") -> bool:
        return Leaf.parse(who) in self._roster

    def add_leaf(self, who: "Leaf | str") -> Leaf:
        """An MLS Add: a new leaf, and a new epoch."""
        leaf = Leaf.parse(who)
        if leaf not in self._roster:
            self._roster.append(leaf)
            self.rekey(f"add {leaf}")
        return leaf

    def remove_leaf(self, who: "Leaf | str") -> Leaf:
        """An MLS Remove: the leaf goes, and the epoch moves past it."""
        leaf = Leaf.parse(who)
        if leaf in self._roster:
            self._roster.remove(leaf)
            self.rekey(f"remove {leaf}")
        return leaf

    def rekey(self, why: str = "") -> int:
        """The cheap epoch pump. `seq` resets to 0 in the new epoch, and the
        chain still links to the old head."""
        self.epoch += 1
        self.last_rekey = why
        return self.epoch

    # -- authoring -----------------------------------------------------------

    def op(self, name: str) -> Op:
        """Resolve an op name against THIS kind's table. Loud if it is not there —
        which is how a Conversation refuses `forum.setRoom` before a delta for it
        even exists."""
        return self.kind.op(name)

    def head(self) -> tuple[tuple[int, int], str] | None:
        if not self._spine:
            return None
        pos, delta, _ = self._spine[-1]
        return pos, delta.id

    def next_pos(self) -> tuple[int, str]:
        """`coordinator::next_sequenced_pos` — the (seq, prev) the next sequenced
        delta must carry."""
        head = self.head()
        if head is None:
            return 0, GENESIS_PREV
        (epoch, seq), ident = head
        return (seq + 1, ident) if self.epoch == epoch else (0, ident)

    def next_gen(self, by: "Leaf | str") -> int:
        """The next LWW generation for one AUTHOR — per author, not global."""
        who = str(Leaf.parse(by))
        mine = [d.gen or 0 for d, a in self._orset if str(a) == who]
        return max(mine, default=-1) + 1

    def author(self, by: "Leaf | str", op: str, args: Mapping | None = None,
               slot: str | None = None, at: tuple[int, int] | None = None,
               prev: str | None = None, gen: int | None = None,
               type_id: int | None = None) -> Delta:
        """Stamp a delta. Overrides exist so a case can author a FORK on purpose."""
        who = Leaf.parse(by)
        decl = self.op(op)
        packed = tuple(sorted((str(k), _canon(v)) for k, v in (args or {}).items()))
        if decl.sequenced:
            if at is not None:
                epoch, seq = at
            else:
                seq, chain = self.next_pos()
                epoch = self.epoch
                prev = chain if prev is None else prev
            return Delta(type_id=self.type_id if type_id is None else type_id,
                         op=decl, args=packed, epoch=epoch,
                         prev=GENESIS_PREV if prev is None else prev, seq=seq)
        return Delta(type_id=self.type_id if type_id is None else type_id,
                     op=decl, args=packed, epoch=self.epoch,
                     prev=GENESIS_PREV,
                     gen=self.next_gen(who) if gen is None else gen,
                     slot=slot)

    # -- delivery ------------------------------------------------------------

    def deliver(self, delta: Delta, by: "Leaf | str") -> Verdict:
        """`Coordinator::deliver`, with the catalogue's declarations as the gates.

        The ORDER of the gates is the contract: a specified-only op is refused
        before authority, and authority before the fold, so a refused delta never
        touches state on its way to being refused.
        """
        who = Leaf.parse(by)

        # 1. the delta must carry this type's id — and a reserved id is loud
        if delta.type_id != self.type_id:
            was = self.cat.reserved.get(delta.type_id)
            note = (f"type id {delta.type_id} is RESERVED ({was}) — folded away, "
                    "never reissued") if was else \
                   (f"delta carries type id {delta.type_id}, this object is "
                    f"{self.kind.name}({self.type_id})")
            return self._no(delta, who, _refuse(Rejection.UNKNOWN_TYPE, note))

        # 2. the op must be one THIS kind declares
        try:
            mine = self.kind.op_by_id(delta.op_id)
        except UnknownOp as e:
            return self._no(delta, who, _refuse(Rejection.UNKNOWN_TYPE, str(e)))
        if mine.name != delta.name:
            return self._no(delta, who, _refuse(
                Rejection.UNKNOWN_TYPE,
                f"op id {delta.op_id} is `{mine.name}` on {self.kind.name}, not "
                f"`{delta.name}`"))

        # 3. specified-only: there is no reducer behind it
        if not mine.implemented:
            return self._no(delta, who, _refuse(
                Rejection.NOT_IMPLEMENTED,
                f"`{mine.name}` is status=specified in the ICD — declared, not built"))

        # 4. authority. Membership is per LEAF; authority is per PERSON.
        if who not in self._roster:
            return self._no(delta, who, _refuse(
                Rejection.UNAUTHORIZED,
                f"{who} holds no leaf in {self.name} — membership IS access"))
        if mine.owner_only and who.person != self.owner:
            return self._no(delta, who, _refuse(
                Rejection.UNAUTHORIZED,
                f"`{mine.name}` is owner-authority and {who} is {who.person}, "
                f"not the owner ({self.owner})"))

        # 5. the fold the op DECLARES
        if mine.sequenced:
            return self._sequenced(delta, who)
        return self._commutative(delta, who)

    def _sequenced(self, delta: Delta, who: Leaf) -> Verdict:
        # "only the owner sequences the spine" — the Rust checks this on the ARM,
        # not on the op's declared authority, so a sequenced any-member op would
        # be refused here even though its declaration permits it. No such op
        # exists in today's catalogue (all 54 sequenced ops are owner-authority),
        # so the two rules coincide; the day they do not, the spine wins, and
        # modelling it on the authority alone would have hidden that.
        if who.person != self.owner:
            return self._no(delta, who, _refuse(
                Rejection.UNAUTHORIZED,
                f"only the owner sequences the spine; {who} is {who.person}, not "
                f"{self.owner}"))
        if delta.seq is None:
            return self._no(delta, who, _refuse(
                Rejection.MALFORMED_ARGS, "a sequenced delta must carry a seq"))
        pos = (delta.epoch, delta.seq)

        existing = self._by_pos.get(pos)
        if existing is not None:
            if existing == delta.id:
                return DUPLICATE
            return self._no(delta, who, _refuse(
                Rejection.FORK_DETECTED,
                f"two truths at (e{pos[0]}, s{pos[1]}): {existing[:8]} and "
                f"{delta.short} — a divergence halt"))

        head = self.head()
        if head is None:
            if delta.seq != 0:
                return self._no(delta, who, _refuse(
                    Rejection.CHAIN_BROKEN,
                    f"an empty spine anchors at seq 0; this delta opens at "
                    f"s{delta.seq}"))
        else:
            (h_epoch, h_seq), h_id = head
            if delta.epoch < h_epoch:
                return self._no(delta, who, _refuse(
                    Rejection.STALE_EPOCH,
                    f"e{delta.epoch} is behind the fence at e{h_epoch}"))
            if delta.prev != h_id:
                return self._no(delta, who, _refuse(
                    Rejection.CHAIN_BROKEN,
                    f"prev {delta.prev[:8]} does not link to the head {h_id[:8]}"))
            expected = (h_epoch, h_seq + 1) if delta.epoch == h_epoch else (delta.epoch, 0)
            if pos != expected:
                return self._no(delta, who, _refuse(
                    Rejection.CHAIN_BROKEN,
                    f"expected (e{expected[0]}, s{expected[1]}), got "
                    f"(e{pos[0]}, s{pos[1]}) — a gap breaks the chain"))

        self._by_pos[pos] = delta.id
        self._spine.append((pos, delta, who))
        return ACCEPTED

    def _commutative(self, delta: Delta, who: Leaf) -> Verdict:
        if delta.gen is None:
            return self._no(delta, who, _refuse(
                Rejection.MALFORMED_ARGS,
                "a commutative delta must carry its (gen) LWW key"))
        key = (str(who), delta.id)
        if key in self._seen:
            return DUPLICATE
        self._seen.add(key)
        self._orset.append((delta, who))
        return ACCEPTED

    def _no(self, delta: Delta, who: Leaf, verdict: Verdict) -> Verdict:
        self.refusals.append((delta, who, verdict))
        return verdict

    # -- reading -------------------------------------------------------------

    @property
    def spine(self) -> tuple[Delta, ...]:
        return tuple(d for _, d, _ in self._spine)

    @property
    def orset(self) -> tuple[Delta, ...]:
        return tuple(d for d, _ in self._orset)

    @property
    def log(self) -> tuple[Delta, ...]:
        return self.spine + self.orset

    def state(self) -> ObjectState:
        """Fold. The spine in (epoch, seq) order FIRST, then the OR-set in
        canonical (gen, author, DeltaId) order — "so state is a function of the
        delta SET, never arrival order"."""
        st = ObjectState()
        for pos, delta, _author in sorted(self._spine, key=lambda r: r[0]):
            st.spine.append(f"{delta.name}@e{pos[0]}s{pos[1]}")
            for f in delta.op.node:
                st.node[f] = NodeWrite(field=f, op=delta.name, delta=delta.short)
            for rel, frm, to, action in delta.op.edges:
                st.edges.append(EdgeWrite(rel=rel, frm=frm, to=to, action=action,
                                          op=delta.name, delta=delta.short))

        # the OR-set, per-(author, slot) last-writer-wins by (gen, DeltaId)
        canonical = sorted(self._orset,
                           key=lambda r: (r[0].gen or 0, str(r[1]), r[0].id))
        best: dict[tuple[str, str], Entry] = {}
        for delta, author in canonical:
            slot = delta.slot or delta.id      # add-only: it is its own slot
            key = (str(author), slot)
            cand = Entry(author=str(author), slot=slot, gen=delta.gen or 0,
                         delta=delta.short, op=delta.name, args=delta.args)
            held = best.get(key)
            if held is None or (cand.gen, cand.delta) > (held.gen, held.delta):
                if held is not None:
                    st.inert.append(f"{held.delta} superseded by {cand.delta}")
                best[key] = cand
            else:
                st.inert.append(f"{cand.delta} loses to {held.delta} at {key[1]}")
            for f in delta.op.node:
                st.node[f] = NodeWrite(field=f, op=delta.name, delta=delta.short)
            for rel, frm, to, action in delta.op.edges:
                st.edges.append(EdgeWrite(rel=rel, frm=frm, to=to, action=action,
                                          op=delta.name, delta=delta.short))
        st.entries = [best[k] for k in sorted(best)]
        return st

    def digest(self) -> str:
        return self.state().digest()

    # -- the render's row ----------------------------------------------------

    def ops_summary(self) -> str:
        return f"{self.kind.implemented}/{self.kind.total} implemented"

    def line(self) -> str:
        person = "person" if len(self.people) == 1 else "people"
        leaf = "leaf" if len(self.leaves) == 1 else "leaves"
        return (f"{self.name:<12} {self.kind.name}·{self.type_id:<3} "
                f"{len(self.people)} {person} · {len(self.leaves)} {leaf} · "
                f"e{self.epoch} · ops {self.ops_summary()}")


# ── replicas, and what each fold promises about order ───────────────────────


def replica(of: GroupObject, deltas: Sequence[tuple[Delta, Leaf]]) -> GroupObject:
    """A second replica of the same object, fed `deltas` in THAT arrival order.

    The roster and epoch are copied; nothing else is. This is the only honest way
    to ask the convergence question: two replicas, same set, different order.
    """
    twin = GroupObject(of.name, of.kind, of.owner, of.leaves, epoch=of.epoch,
                       cat=of.cat)
    for delta, who in deltas:
        twin.deliver(delta, who)
    return twin


def converges(of: GroupObject, deltas: Sequence[tuple[Delta, Leaf]],
              orders: Iterable[Sequence[int]] | None = None) -> tuple[bool, dict[str, list]]:
    """Feed the same delta SET in many orders; report the digests reached.

    Returns (all_agree, {digest: the first order that reached it}).
    """
    deltas = list(deltas)
    if orders is None:
        if len(deltas) > 7:
            raise ValueError(
                f"{len(deltas)} deltas is {len(deltas)}! orders — pass `orders=` with "
                "a sample rather than waiting on the factorial")
        orders = itertools.permutations(range(len(deltas)))
    reached: dict[str, list] = {}
    for order in orders:
        twin = replica(of, [deltas[i] for i in order])
        reached.setdefault(twin.digest(), list(order))
    return len(reached) == 1, reached
