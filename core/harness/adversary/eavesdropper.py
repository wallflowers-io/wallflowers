"""eavesdropper — E10, THE SHOWCASE. A relay that opens an intro blob.

Gate G0, the tripwire (execution.html): "Cádiz, the yard. szonja scans axel's code
to join STOMA. eve is subscribed to the relay. Eve opens the intro blob and reads
szonja, axel's key, and why she scanned."

The mechanism, from seal.rs + node.rs + pacific_wire:
  - To ROUTE an intro blob the relay must hold its tag. `tag_hex` is `hex::encode`,
    the RAW tag with no digest, so the tag on the wire IS the 32 secret bytes.
  - At pairing, node.rs seals the intro with conn_secret == dest_tag == that tag:
        seal::seal(&payload, &dest, &dest)        # node.rs:319 and :632
  - So the seal key = HKDF(salt=tag, ikm=b"", info="pacific/seal/v1"||tag), AAD=tag,
    is a PURE FUNCTION OF THE ROUTING ADDRESS. Anyone who can route can decrypt.

Eve does not need to be told which tag is an intro tag. She derives a key from EVERY
tag she sees and tries to open the blob under it. A blob sealed with a real MLS
conn_secret (!= its tag) fails the AEAD tag check; an intro blob pops open. The
AEAD check is itself the discriminator.

She reports what she LEARNED as a growing set, so the render can watch the social
graph assemble in her panel as people pair.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from knowledge import Knowledge
from pacific_seal import decode_intro, open_sealed
from relay_client import RelayClient, blob_unb64


@dataclass
class IntroRead:
    """The result of eve trying to open one blob keyed by its own tag."""

    opened: bool
    tag_hex: str
    fields: dict = field(default_factory=dict)
    learned: set[str] = field(default_factory=set)


# The exact IntroPayload fields eve recovers when she opens an intro blob. This is
# the set the showcase asserts against (the-harness §03, execution.html G0/G4).
INTRO_LEAK = {"scanner_pk", "scanner_name", "why", "kind", "arc", "owner"}


class Eavesdropper:
    """A hostile relay/eavesdropper subscribed to opaque tags."""

    def __init__(self, name: str = "eve"):
        self.name = name
        self.knowledge = Knowledge(label=name)
        self.intro_reads: list[IntroRead] = []

    def observe(self, tag_hex: str, blob_b64: str) -> IntroRead:
        """Try to open one (tag, blob) as an intro blob keyed by the tag itself."""
        self.knowledge.saw_tag(tag_hex)
        raw_tag = bytes.fromhex(tag_hex)
        blob = blob_unb64(blob_b64)
        try:
            # conn_secret == dest_tag == the raw routing tag.
            inner = open_sealed(blob, dest_tag=raw_tag, conn_secret=raw_tag)
        except Exception:
            # Not an intro blob (or tampered): the AEAD tag check refused. Eve
            # learns nothing but the tag/seq/size she already had.
            return IntroRead(opened=False, tag_hex=tag_hex)

        # It opened. Decode the plaintext IntroPayload and harvest the fields.
        try:
            fields = decode_intro(inner)
        except Exception:
            # Opened but not a recognisable IntroPayload — still a decryption win.
            r = IntroRead(opened=True, tag_hex=tag_hex, learned={"<sealed-plaintext>"})
            self.knowledge.opened_plaintext()
            self.intro_reads.append(r)
            return r

        learned = {k for k in INTRO_LEAK if fields.get(k) is not None}
        r = IntroRead(opened=True, tag_hex=tag_hex, fields=fields, learned=learned)
        self.intro_reads.append(r)
        # Fold the harvest into the growing knowledge set.
        self.knowledge.opened_plaintext(name=fields.get("scanner_name"))
        for k in learned:
            self.knowledge.learn(f"{k}={fields.get(k)!r}")
        if fields.get("owner"):
            self.knowledge.learn(f"edge: {fields.get('scanner_name')} -> owner:{fields['owner'][:12]}…")
        return r

    async def attempt_intro_read(self, client: RelayClient, tags: list[str]) -> IntroRead:
        """Subscribe to `tags`, drain the backlog, and open the first intro blob.

        Returns the first successful IntroRead (opened=True) or, if none opened, the
        last attempt. Real socket, real backlog replay.
        """
        await client.subscribe(tags, since=0, v=1)
        frames = await client.drain_until_eose()
        last = IntroRead(opened=False, tag_hex="")
        for f in frames:
            if f.get("t") != "msg":
                continue
            r = self.observe(f["tag"], f["blob"])
            last = r
            if r.opened:
                return r
        return last

    def report(self) -> dict:
        return {
            "adversary": self.name,
            "intro_blobs_opened": sum(1 for r in self.intro_reads),
            "knowledge": self.knowledge.as_dict(),
            "reads": [
                {"tag": r.tag_hex[:16] + "…", "opened": r.opened, "fields": r.fields}
                for r in self.intro_reads
            ],
        }
