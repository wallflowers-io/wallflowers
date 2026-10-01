"""relay — the relay modelled as what it is: a database with insert rules.

A faithful mirror of `arc/planes/relay/src/store.rs` (the four tables) and of the
`Hub` in `arc/planes/relay/src/lib.rs` (seq minting, fanout, the `Gap` rule). The
point of writing the tables down is that it makes the adversary answerable: what
the relay knows is exactly what is in here, and no more.

Three properties are load-bearing and each is a comment in the Rust:

  * `append` IS ONE TRANSACTION. A commit-flagged publish to a claimed tag returns
    False having changed NOTHING — not the blob table, not the seq high-water. The
    PRIMARY KEY is the arbitration: the second insert failing IS the rejection.
  * `evict` WRITES THE FLOOR BEFORE DELETING. Once the rows are gone you cannot
    know what the highest removed seq was.
  * `floors` OMITS TAGS THAT NEVER EVICTED. "Nothing was ever dropped here" is a
    different claim from "the floor is 0", and only the first one suppresses a Gap.

And one the table reveals that the prose did not: `seq` is GLOBALLY monotonic, not
per-tag. One `meta['last_seq']`, and the age sweep cuts `WHERE seq <= cutoff`
across every tag at once. Anyone who sees seqs can totally order writes across the
whole system — see :meth:`RelayState.interleaving`, which is E11's whole argument.
"""
from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Callable, Iterable

from .. import wire

#: `lib.rs` BOOT_SKEW_US — a second of slack per boot, so a burst that pushed seq
#: past the clock cannot be reissued by a restart inside that window.
BOOT_SKEW_US = 1_000_000


def now_micros() -> int:
    """Wall clock in microseconds — `lib.rs::now_micros`. seq IS the timestamp."""
    return int(time.time() * 1_000_000)


# ── the store ───────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Tables:
    """A frozen copy of the four tables, for diffing and for the hostile relay.

    This is the WHOLE of what the relay durably knows. If a finding cannot be
    derived from these four dicts, the relay did not learn it.
    """
    blob: dict[tuple[str, int], str]
    commit_slot: dict[str, int]
    retention_floor: dict[str, int]
    meta: dict[str, int]

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, Tables):
            return NotImplemented
        return (self.blob == other.blob
                and self.commit_slot == other.commit_slot
                and self.retention_floor == other.retention_floor
                and self.meta == other.meta)

    def diff(self, other: "Tables") -> list[str]:
        """Human-readable differences, table by table. Empty list means identical."""
        out: list[str] = []
        for name in ("blob", "commit_slot", "retention_floor", "meta"):
            a, b = getattr(self, name), getattr(other, name)
            if a == b:
                continue
            for k in sorted(set(a) | set(b), key=repr):
                if a.get(k) != b.get(k):
                    out.append(f"{name}[{k!r}]: model={a.get(k)!r} real={b.get(k)!r}")
        return out


class RelayState:
    """The durable mailbox. Mirrors `store.rs` table for table, rule for rule."""

    def __init__(self) -> None:
        self.blob: dict[tuple[str, int], str] = {}       # PK (tag, seq)
        self.commit_slot: dict[str, int] = {}            # PK tag
        self.retention_floor: dict[str, int] = {}        # PK tag, monotonic
        self.meta: dict[str, int] = {}                   # meta['last_seq']

    # -- writes --------------------------------------------------------------

    def append(self, tag: str, seq: int, body: str, commit: bool = False) -> tuple[str, int]:
        """Store one blob, optionally claiming the tag's commit slot. ONE transaction.

        Returns ``("stored", seq)``, ``("duplicate", prior_seq)`` or
        ``("slot_taken", 0)`` — `store::Store::append`'s `Appended`, in that order of
        checks. ONE COPY PER (tag, blob): the same blob at the same tag is answered
        with the seq it already has and changes nothing, which is what makes a
        replayed signed publish harmless. The duplicate check comes BEFORE the slot,
        so a retried winning commit is the win it was, not a lost race. A rejected
        commit leaves no trace: not the blob, not the high-water.
        """
        for (t, s), b in self.blob.items():  # SELECT seq WHERE tag = ? AND digest = ?
            if t == tag and b == body:
                return ("duplicate", s)
        if commit:
            if tag in self.commit_slot:      # ON CONFLICT(tag) DO NOTHING → claimed == 0
                return ("slot_taken", 0)     # tx drops = rollback.
            self.commit_slot[tag] = seq
        self.blob.setdefault((tag, seq), body)           # ON CONFLICT(tag,seq) DO NOTHING
        self.meta["last_seq"] = max(self.meta.get("last_seq", 0), seq)
        return ("stored", seq)

    def evict(self, now_us: int, window_us: int, max_per_tag: int) -> int:
        """Two independent policies, floors written BEFORE the deletes. Returns removed."""
        removed = 0

        if window_us > 0:
            cutoff = max(0, now_us - window_us)          # saturating_sub
            doomed = [(t, s) for (t, s) in self.blob if s <= cutoff]
            self._raise_floors(doomed)                   # ← before the delete
            for k in doomed:
                del self.blob[k]
            removed += len(doomed)

        if max_per_tag > 0:
            doomed = []
            for tag, seqs in self._by_tag().items():
                # ROW_NUMBER() OVER (PARTITION BY tag ORDER BY seq DESC), rn > cap
                for s in sorted(seqs, reverse=True)[max_per_tag:]:
                    doomed.append((tag, s))
            self._raise_floors(doomed)                   # ← before the delete
            for k in doomed:
                del self.blob[k]
            removed += len(doomed)

        return removed

    def _raise_floors(self, doomed: Iterable[tuple[str, int]]) -> None:
        """floor = MAX(seq) REMOVED, never a surviving one; only ever raised."""
        highest: dict[str, int] = {}
        for tag, seq in doomed:
            highest[tag] = max(highest.get(tag, 0), seq)
        for tag, seq in highest.items():
            self.retention_floor[tag] = max(self.retention_floor.get(tag, 0), seq)

    # -- reads ---------------------------------------------------------------

    def resume_seq(self) -> int:
        """The durable seq high-water, or 0 for a fresh store."""
        return self.meta.get("last_seq", 0)

    def replay(self, tag: str, since: int) -> list[tuple[int, str]]:
        """Backlog for one tag: seq > since, in seq order. EXCLUSIVE of the cursor."""
        return sorted(((s, b) for (t, s), b in self.blob.items() if t == tag and s > since))

    def floors(self, tags: Iterable[str]) -> dict[str, int]:
        """Retention floors for `tags`, OMITTING tags that have never evicted."""
        return {t: self.retention_floor[t] for t in tags if t in self.retention_floor}

    def counts(self) -> tuple[int, int]:
        """(distinct tags, total blobs) — two pure cardinalities, no names."""
        return (len(self._by_tag()), len(self.blob))

    def tag_counts(self) -> list[tuple[str, int]]:
        return sorted((t, len(s)) for t, s in self._by_tag().items())

    def _by_tag(self) -> dict[str, list[int]]:
        out: dict[str, list[int]] = {}
        for t, s in self.blob:
            out.setdefault(t, []).append(s)
        return out

    # -- the adversary's view ------------------------------------------------

    def tables(self) -> Tables:
        """A frozen copy of all four tables — the hostile relay sees exactly this."""
        return Tables(dict(self.blob), dict(self.commit_slot),
                      dict(self.retention_floor), dict(self.meta))

    def sizes(self) -> list[tuple[str, int, int]]:
        """(tag, seq, len(body)) for every stored blob — the eavesdropper's metadata."""
        return sorted((t, s, len(b)) for (t, s), b in self.blob.items())

    def interleaving(self, tags: Iterable[str] | None = None) -> list[tuple[str, int]]:
        """(tag, seq) across tags in seq order — E11: seq is GLOBAL, so this is a
        total order over writes to the whole system, and two tags' exact
        interleaving falls out of it. Per-tag sequences would leak strictly less."""
        want = set(tags) if tags is not None else None
        rows = [(s, t) for (t, s) in self.blob if want is None or t in want]
        return [(t, s) for s, t in sorted(rows)]


# ── the hub ─────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Retention:
    """`lib.rs::Retention`. Both limits default to 0 — keep everything."""
    window_us: int = 0
    max_per_tag: int = 0
    sweep_secs: int = 300


@dataclass
class Metrics:
    total_publishes: int = 0
    total_deliveries: int = 0
    rejected_commits: int = 0
    gaps_announced: int = 0
    store_errors: int = 0
    bad_frames: int = 0
    media_presigned: int = 0
    media_refused: int = 0


class SeqSource:
    """`Hub::next_seq` — the wall clock in microseconds, or one past the last
    issued number when the clock has not advanced. Strictly increasing within a
    process AND across restarts, which is what the never-rewinding client cursor
    depends on. NOT a counter from 1, and NOT per-tag."""

    def __init__(self, resumed: int = 0, clock: Callable[[], int] = now_micros) -> None:
        self._clock = clock
        self.seq = max(clock() + BOOT_SKEW_US, resumed + 1)

    def next(self) -> int:
        self.seq = max(self._clock(), self.seq + 1)
        return self.seq


class Conn:
    """One connection: its tag set and its outbound frames.

    `out` is the socket. Track 3 reads it exactly as an eavesdropper would read a
    real one — the frames are the same dicts that come off the wire.
    """

    def __init__(self, cid: int, label: str = "") -> None:
        self.id = cid
        self.label = label
        self.tags: set[str] = set()
        self.v: int = 0
        self.out: list[wire.Frame] = []

    def drain(self) -> list[wire.Frame]:
        """Take everything delivered since the last drain."""
        got, self.out = self.out, []
        return got

    def __repr__(self) -> str:
        return f"Conn({self.id}{'/' + self.label if self.label else ''}, tags={len(self.tags)})"


class RelayHub:
    """The relay as a process: a store, a seq source, and connections.

    Wire-level, so a scenario drives this the way it drives the real socket, and
    the frames compare directly against the real relay's. Every emitted frame also
    goes to `taps` — that is Track 3's observation hook.
    """

    def __init__(self, state: RelayState | None = None,
                 retention: Retention = Retention(),
                 seq: SeqSource | None = None) -> None:
        self.store = state or RelayState()
        self.retention = retention
        self.seq = seq or SeqSource(self.store.resume_seq())
        self.conns: dict[int, Conn] = {}
        self.metrics = Metrics()
        self.taps: list[Callable[[int, wire.Frame], None]] = []
        self._next_id = 0

    # -- connections ---------------------------------------------------------

    def connect(self, label: str = "") -> Conn:
        c = Conn(self._next_id, label)
        self._next_id += 1
        self.conns[c.id] = c
        return c

    def disconnect(self, conn: Conn) -> None:
        self.conns.pop(conn.id, None)

    def _send(self, conn: Conn, frame: wire.Frame) -> None:
        conn.out.append(frame)
        for tap in self.taps:
            tap(conn.id, frame)

    # -- the two verbs -------------------------------------------------------

    def publish(self, conn: Conn, tag: str, blob: str, commit: bool = False,
                seq: int | None = None) -> tuple[int, int, bool]:
        """Store and fan out. Returns (seq, fanout, accepted), as `Hub::publish`.

        A commit-flagged publish to a claimed tag is REJECTED: not stored, not
        fanned out, (0, 0, False) — and the loser is Acked `ok:false`.

        `seq` overrides the minted number. That is for parity runs only, where the
        model follows the seqs the real relay issued; leave it None otherwise.
        """
        minted = self.seq.next() if seq is None else seq
        verdict, at = self.store.append(tag, minted, blob, commit)
        if verdict == "slot_taken":
            self.metrics.rejected_commits += 1
            self._send(conn, wire.ack(0, ok=False))
            return (0, 0, False)
        if verdict == "duplicate":
            # Already here: acked as the publish it was, fanned out to nobody.
            self._send(conn, wire.ack(at, ok=True))
            return (at, 0, True)
        fanout = 0
        # Fanout happens BEFORE the Ack — a publisher subscribed to its own tag
        # sees its Msg first. That ordering is observable, so the model keeps it.
        for c in self.conns.values():
            if tag in c.tags:
                self._send(c, wire.msg(tag, minted, blob))
                fanout += 1
        self.metrics.total_publishes += 1
        self.metrics.total_deliveries += fanout
        self._send(conn, wire.ack(minted, ok=True))
        return (minted, fanout, True)

    def subscribe(self, conn: Conn, tags: list[str], since: int = 0,
                  v: int = 0) -> tuple[int, int]:
        """Replay seq > since, then Eose. Returns (replayed, gaps), as `Hub::subscribe`.

        Any tag whose floor sits above the caller's cursor gets a `Gap` FIRST, and
        only when `v >= 1`: an unknown frame is fatal on older builds, so silence
        is the only safe thing to send a v=0 client.
        """
        floors = self.store.floors(tags) if v >= 1 else {}
        backlog: list[tuple[int, str, str]] = []
        for t in tags:
            backlog.extend((s, t, b) for s, b in self.store.replay(t, since))
        backlog.sort(key=lambda r: r[0])
        gaps = 0
        for t in tags:
            floor = floors.get(t)
            if floor is not None and since < floor:
                self._send(conn, wire.gap(t, floor))
                gaps += 1
        conn.tags = set(tags)
        conn.v = v
        for seq, tag, blob in backlog:
            self._send(conn, wire.msg(tag, seq, blob))
        self._send(conn, wire.eose())
        self.metrics.total_deliveries += len(backlog)
        self.metrics.gaps_announced += gaps
        return (len(backlog), gaps)

    # -- the sweep -----------------------------------------------------------

    def evict(self, now_us: int | None = None) -> int:
        r = self.retention
        return self.store.evict(now_micros() if now_us is None else now_us,
                                r.window_us, r.max_per_tag)

    # -- reporting -----------------------------------------------------------

    def tables(self) -> Tables:
        return self.store.tables()
