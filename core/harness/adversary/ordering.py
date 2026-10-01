"""ordering — E11, global seq is a correlation channel.

the-harness §02: "seq is GLOBALLY monotonic, not per-tag — one meta['last_seq'],
and evict sweeps WHERE seq <= cutoff across every tag at once. So anyone who sees
seqs can totally order writes across the whole system. An observer on two tags
learns their exact interleaving, which is a correlation channel for deciding two
tags belong to one conversation."

Confirmed against the real relay: `Hub::next_seq` (arc/planes/relay/src/lib.rs) is
`self.seq = now_micros().max(self.seq + 1)` — one counter shared by every tag, and
store.rs stores `meta['last_seq']` as a single global high-water. So the `seq` on a
`Msg` frame for tag A and the `seq` on a `Msg` frame for tag B are drawn from ONE
sequence, and sorting all observed (tag, seq) by seq reconstructs the exact global
order in which the writes were accepted — across unrelated tags.

This adversary subscribes to two unrelated tags, records every (tag, seq) it is
delivered, and recovers the interleaving. It reports, as its growing knowledge set,
the ordered cross-tag transcript it reconstructed.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from knowledge import Knowledge
from relay_client import RelayClient


@dataclass
class OrderingFinding:
    recovered_order: list[tuple[str, int]] = field(default_factory=list)  # (tag_label, seq)
    total_order_across_tags: bool = False
    distinct_tags: int = 0


class OrderingObserver:
    """Subscribes to two unrelated tags and recovers their interleaving."""

    def __init__(self, name: str = "eve-order"):
        self.name = name
        self.knowledge = Knowledge(label=name)

    async def observe_two_tags(
        self,
        client: RelayClient,
        tag_a: str,
        tag_b: str,
        labels: dict[str, str] | None = None,
    ) -> OrderingFinding:
        """Subscribe to (tag_a, tag_b), drain, and reconstruct the global order.

        `labels` maps a hex tag to a human label for the report (else a short hex).
        """
        labels = labels or {}

        def lab(tag: str) -> str:
            return labels.get(tag, tag[:8] + "…")

        await client.subscribe([tag_a, tag_b], since=0, v=1)
        frames = await client.drain_until_eose()

        events: list[tuple[int, str]] = []  # (seq, tag)
        for f in frames:
            if f.get("t") != "msg":
                continue
            tag, seq = f["tag"], f["seq"]
            self.knowledge.saw_tag(tag)
            events.append((seq, tag))

        # The relay handed them back per-tag in the replay; the CHANNEL is that a
        # single sort by the global seq re-interleaves them. This is the leak.
        events.sort(key=lambda e: e[0])
        recovered = [(lab(tag), seq) for seq, tag in events]

        # A total order exists across tags iff every seq is distinct (they are: one
        # microsecond counter, forced strictly increasing by max(.., seq+1)).
        seqs = [s for s, _ in events]
        total = len(set(seqs)) == len(seqs) and seqs == sorted(seqs)
        distinct_tags = len({t for _, t in events})

        self.knowledge.note(
            f"reconstructed a single global order over {len(events)} writes "
            f"across {distinct_tags} unrelated tags"
        )
        for label, seq in recovered:
            self.knowledge.learn(f"seq {seq} -> {label}")
        if total and distinct_tags >= 2:
            self.knowledge.learn("CORRELATION: two tags share one global sequence -> same system, orderable")

        return OrderingFinding(
            recovered_order=recovered,
            total_order_across_tags=total and distinct_tags >= 2,
            distinct_tags=distinct_tags,
        )

    def report(self) -> dict:
        return {"adversary": self.name, "knowledge": self.knowledge.as_dict()}
