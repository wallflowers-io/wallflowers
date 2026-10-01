"""cases — the object catalogue, asserted.

Ten cases. Each one takes a rule the catalogue STATES and makes it fail when it
is broken, which is the only way a model earns its keep. They are named `O1…O10`
and every title carries the name of the RUST TEST it mirrors, so that a
divergence between the two implementations reads as a divergence rather than as
a translation — the same convention `F13/F14/F15` follow for the relay.

Where a case has no Rust counterpart the title says so.

WHAT A GREEN RUN HERE PROVES: that the rules the ICD and `object.rs` declare are
coherent, and that a scenario written against them cannot quietly do something
the catalogue forbids. It does NOT prove the reducers implement them. No reducer
runs in this file.
"""
from __future__ import annotations

import itertools
from typing import Callable

from ..cases.framework import CaseResult, Run
from ..render import ObjectRow
from .catalogue import (BASE_OP_NAMESPACE, Catalogue, Kind, Op, ReservedTypeId,
                        UnknownOp, catalogue)
from .model import (Delta, GroupObject, Leaf, Rejection, SpecViolation, converges,
                    replica)

BACKEND = "model"


def _rows(cat: Catalogue) -> list[ObjectRow]:
    """Every declared kind, whether or not it is built — the panel's whole point
    is that `field·20  0 ops` is visible next to `post·30  5/5`."""
    return [ObjectRow(name=k.name, kind=k.name, type_id=k.type_id, people=0,
                      leaves=0, epoch=0, implemented=k.implemented, ops=k.total)
            for k in cat]


def _expose(r: Run, cat: Catalogue, *objects: GroupObject, note: str = "") -> None:
    rows = [ObjectRow.of(o, note=note) for o in objects] or _rows(cat)
    r.expose(objects=rows, catalogue=cat.line())


# ── O1 ──────────────────────────────────────────────────────────────────────


def o1_the_catalogue_is_read_not_transcribed(on_step: Callable | None = None,
                                             backend: str = BACKEND) -> CaseResult:
    """icd_matches_every_op_table — both sources parse, and they agree.

    The Python half of `icd.rs`. The Rust anchors the ICD to the op tables it
    compiles; this anchors the harness to the same two files, so a scenario can
    never be written against an op that is not in the tree.
    """
    r = Run("O1", "icd_matches_every_op_table — the catalogue is read, not transcribed",
            backend, on_step)

    r.step("parse core/coordination/delta-graph.icd.json and pacific-core/src/object.rs")
    cat = Catalogue.load()
    _expose(r, cat)
    icd, rs = cat.sources["icd"], cat.sources["object.rs"]
    r.check("the ICD parsed", bool(cat.ops), f"{icd.path} sha256:{icd.digest}")
    r.check("object.rs parsed — the wire type ids come from the Rust, not from here",
            bool(cat.kinds), f"{rs.path} sha256:{rs.digest}")

    r.step("the inventory")
    total, impl, spec = cat.totals()
    rows = sum(len(k.ops) for k in cat.built)
    r.check("every channel message resolves to a definition",
            rows >= total and total > 0,
            f"{rows} channel rows over {total} definitions — the difference is the "
            "ops carried by two channels (the chat band, base.setLocation)")
    r.check("and every one carries authority, fold and status",
            impl + spec == total and spec > 0,
            f"{total} ops · {impl} implemented / {spec} specified — status comes from "
            "x-graph.status, not from this file")
    # THE OLD CHECK HERE ASSERTED "nine channels and twelve kinds", and said of it:
    # "pinned in the RUST, so it cannot fail on its own — if it goes red, cargo test
    # went red first". BOTH HALVES WERE WRONG, and the comment was the worse half:
    # it told a reader this line could not fail alone, so when it did fail alone the
    # reader would look anywhere but here.
    #
    # Measured 15 Sep 2026 — core was 806/806 green and this was red. Post(30) landed
    # its channel, the ICD went from 9 channels to 10 and from 88 messages to 90, the
    # Rust pins moved with it, and this transcribed pair did not. A number written
    # down in two places is not pinned; it is duplicated, and the copy rots.
    #
    # So the channel COUNT is now reported and not asserted, on the file's own stated
    # rule: a harness must not be the thing that breaks when the catalogue legitimately
    # grows. What IS asserted is the pair of facts that genuinely do not move — every
    # built kind has a channel to be built ON, and the taxonomy is twelve kinds, which
    # is closed in object.rs and where 0/16/17/24 are RESERVED forever.
    r.check("every built kind is built on a channel, and the taxonomy is twelve kinds",
            all(k.channel for k in cat.built) and len(cat.all_order) == 12,
            f"{len(cat.built)} channels ({', '.join(k.name for k in cat.built)}) "
            f"of {len(cat.all_order)} kinds")

    r.step("every rule the two sources must satisfy, checked in both directions")
    bad = cat.check()
    r.check("no violations", not bad, "; ".join(bad) if bad else
            "authority/fold/status present and in range · every $ref resolves · "
            "channel key == message name · x-object.typeId == object.rs · base ops "
            "in the 0xF0000000 band · every op declares a graph effect or an exclusion")

    r.step("authority × fold, the correlation and the invariant")
    tally: dict[tuple[str, str], int] = {}
    for op in cat.ops.values():
        tally[(op.authority, op.fold)] = tally.get((op.authority, op.fold), 0) + 1
    owner_comm = tally.get(("owner", "commutative"), 0)
    r.check("no op is commutative AND owner-gated",
            owner_comm == 0,
            " · ".join(f"{a}+{f}: {n}" for (a, f), n in sorted(tally.items())))
    r.check("the base op-group is namespaced away from every type's own ops",
            all(o.op_id >= BASE_OP_NAMESPACE for o in cat.ops.values() if o.base)
            and all(o.op_id < BASE_OP_NAMESPACE for o in cat.ops.values() if not o.base),
            f"{sum(1 for o in cat.ops.values() if o.base)} base ops at "
            f"{BASE_OP_NAMESPACE:#x}+")
    r.note("owner⇔sequenced is exactly correlated in today's catalogue "
           f"({tally.get(('owner','sequenced'),0)} owner+sequenced, "
           f"{tally.get(('anyMember','commutative'),0)} anyMember+commutative), but only "
           "the commutative⇒anyMember half is an invariant. The other half is held by "
           "discipline, and nothing fails if it slips.")
    return r.done()


# ── O2 ──────────────────────────────────────────────────────────────────────


def o2_reserved_type_ids_fail_loud(on_step: Callable | None = None,
                                   backend: str = BACKEND) -> CaseResult:
    """type_id_roundtrip_and_names — and the RESERVED band, which is the half the
    round-trip test cannot state.

    "type_ids are WIRE values on an append-only log: they are assigned once and
    never reused." 0/16/17/24 were Role, Poll, Message and Question; they were
    folded into Forum and the base op-group, and an old delta carrying one must
    fail loud as UnknownType rather than fold into whatever now holds the id.
    """
    r = Run("O2", "type_id_roundtrip_and_names — reserved ids fail loud", backend, on_step)
    cat = catalogue()
    _expose(r, cat)

    r.step("every declared kind round-trips through its wire type id")
    ok, detail = True, []
    for k in cat:
        try:
            ok = ok and cat.kind_for_type_id(k.type_id).name == k.name
        except Exception as e:      # noqa: BLE001 — the failure IS the evidence
            ok = False
            detail.append(f"{k.name}: {e}")
        detail.append(f"{k.name}={k.type_id}")
    r.check("12 kinds, 12 ids, no collisions", ok and len(cat.all_order) == 12,
            " ".join(detail))

    r.step("the reserved band")
    r.check("0/16/17/24 are reserved and named", sorted(cat.reserved) == [0, 16, 17, 24],
            " ".join(f"{i}={n}" for i, n in sorted(cat.reserved.items())))
    refused = []
    for tid in sorted(cat.reserved):
        try:
            cat.kind_for_type_id(tid)
            refused.append(f"{tid}: SILENTLY ACCEPTED")
        except ReservedTypeId as e:
            refused.append(f"{tid}: {str(e).split(' — ')[0]}")
    r.check("a lookup on a reserved id raises rather than returning None",
            all("RESERVED" in x for x in refused), " · ".join(refused))
    r.check("24 is skipped deliberately — Place took 28, the next FREE id",
            cat.kind("place").type_id == 28 and 24 not in
            {k.type_id for k in cat}, "28 is next after Thing(27), not 24")

    r.step("an old delta carrying a reserved id arrives at a real Group")
    notes = GroupObject("notes", "group", owner="ada", roster=["ada/phone"])
    stale = notes.author("ada/phone", "setProfile", {"displayName": "notes",
                                                     "shape": "team"}, type_id=16)
    v = notes.deliver(stale, "ada/phone")
    r.check("refused as UnknownType", v.rejection is Rejection.UNKNOWN_TYPE, v.why)
    r.check("and the refusal NAMES what the id used to be", "Poll" in v.why, v.why)
    r.check("nothing was folded", len(notes.spine) == 0, f"spine={len(notes.spine)}")

    r.step("and an object cannot be minted on a reserved id at all")
    try:
        GroupObject("poll", 16, owner="ada", roster=["ada/phone"])
        minted = True
    except ReservedTypeId:
        minted = False
    r.check("GroupObject(kind=16) raises", not minted,
            "the harness models the failure rather than inventing a Poll kind")
    return r.done()


# ── O3 ──────────────────────────────────────────────────────────────────────


def o3_the_spec_invariant(on_step: Callable | None = None,
                          backend: str = BACKEND) -> CaseResult:
    """spec_invariant_commutative_is_any_member — checked at registration."""
    r = Run("O3", "spec_invariant_commutative_is_any_member — checked at registration",
            backend, on_step)
    cat = catalogue()
    _expose(r, cat)

    r.step("every op in the catalogue, against OpDecl::is_well_formed")
    bad = [o.name for o in cat.ops.values() if not o.well_formed]
    r.check("no commutative op is owner-gated", not bad,
            f"{sum(1 for o in cat.ops.values() if o.commutative)} commutative ops, all "
            "anyMember — a broadcast op cannot be owner-gated before it propagates")

    r.step("a table that breaks it cannot back an object")
    rogue = Kind(name="rogue", type_id=99, channel="rogue", ops={
        "rogue.broadcast": Op(name="rogue.broadcast", channel="rogue", verb="broadcast",
                              op_id=0, authority="owner", fold="commutative",
                              status="implemented", node=("label",))})
    try:
        GroupObject("rogue", rogue, owner="ada", roster=["ada/phone"])
        constructed, why = True, ""
    except SpecViolation as e:
        constructed, why = False, str(e)
    r.check("GroupObject refuses to exist over it", not constructed, why)
    r.note("this is the ONE invariant object.rs enforces (`is_well_formed`). The "
           "harness enforces it in the same place — at registration, not at delivery — "
           "because an op that reaches delivery has already been authored.")
    return r.done()


# ── O4 ──────────────────────────────────────────────────────────────────────


def o4_owner_authority_is_per_person(on_step: Callable | None = None,
                                     backend: str = BACKEND) -> CaseResult:
    """non_owner_cannot_sequence + non_member_is_rejected_loudly.

    And the distinction those two Rust tests do not have to make, because a Rust
    MemberId is one key: MEMBERSHIP is per LEAF, AUTHORITY is per PERSON. Ada's
    laptop is not a second member, it is the same owner holding a second leaf —
    which is A2 in the-harness.html, stated as a rejection rather than a count.
    """
    r = Run("O4", "non_owner_cannot_sequence — authority is per person, membership per leaf",
            backend, on_step)
    cat = catalogue()

    r.step("ada (two devices) and bo (one) share a Group; ada owns it")
    notes = GroupObject("notes", "group", owner="ada",
                        roster=["ada/phone", "ada/laptop", "bo/sim"])
    _expose(r, cat, notes)
    r.check("three leaves, two people — the roster folds", 
            (len(notes.leaves), len(notes.people)) == (3, 2),
            f"leaves={[str(l) for l in notes.leaves]} people={list(notes.people)}")
    r.check("group carries wire type id 18", notes.type_id == 18)

    r.step("ada's phone sets the profile — owner authority, sequenced spine")
    d = notes.author("ada/phone", "setProfile", {"displayName": "notes", "shape": "team"})
    r.check("accepted", bool(notes.deliver(d, "ada/phone")), d.line())

    r.step("ada's LAPTOP sets the cover — the same owner, a different leaf")
    d = notes.author("ada/laptop", "setCover", {"media": "blob:cover"})
    v = notes.deliver(d, "ada/laptop")
    r.check("accepted — the owner's second device is not a second author", bool(v),
            "authority resolves to the PERSON; the leaf only decides membership")

    r.step("bo tries the same op")
    before = notes.digest()
    d = notes.author("bo/sim", "setCover", {"media": "blob:bo"})
    v = notes.deliver(d, "bo/sim")
    r.check("refused as Unauthorized", v.rejection is Rejection.UNAUTHORIZED, v.why)
    r.check("and the spine is untouched", notes.digest() == before,
            f"digest {before} unchanged")

    r.step("cal, who holds no leaf at all, tries it")
    d = notes.author("cal/phone", "setCover", {"media": "blob:cal"})
    v = notes.deliver(d, "cal/phone")
    r.check("refused as Unauthorized — membership IS access",
            v.rejection is Rejection.UNAUTHORIZED, v.why)

    r.step("what bo CAN author on a Group today")
    usable = [o for o in cat.kind("group").ops.values()
              if o.implemented and o.authority == "anyMember"]
    # THIS CHECK READ "nothing" UNTIL 16 SEP 2026, and it was wrong the whole time —
    # not because the model changed but because the catalogue did. The base RATIFY ops
    # (propose/vote: anyMember, commutative, folded by the Coordinator on EVERY object)
    # were on the wire and in no ICD channel, so the one thing a non-owner member of a
    # Group has always been able to do was invisible here. The claim the case is making
    # is about the GROUP's own vocabulary, and that half is unchanged.
    own = sorted(o.name for o in usable if o.channel != "ratify")
    base = sorted(o.name for o in usable if o.channel == "ratify")
    r.check("nothing of the group's OWN: every implemented group.* op is owner-only, "
            "and a member's only implemented verb here is a ratify ballot",
            not own and base == ["ratify.propose", "ratify.vote"],
            f"group is {cat.kind('group').implemented}/{cat.kind('group').total} "
            f"implemented · own anyMember: {' '.join(own) or 'none'} · "
            f"base: {' '.join(base)}")
    r.note("a non-owner member of a Group can author nothing in the group.* vocabulary "
           "that is built today — every anyMember op ON THE KIND (memberJoined/"
           "memberLeft, the wallet attestations, the whole notebook) is "
           "status=specified. What they CAN do is open a ratification and vote in one: "
           "the base RATIFY ops are implemented, any-member, and on every object. Worth "
           "knowing before a scenario is written where a member acts.")
    return r.done()


# ── O5 ──────────────────────────────────────────────────────────────────────


def o5_specified_only_refuses(on_step: Callable | None = None,
                              backend: str = BACKEND) -> CaseResult:
    """No Rust counterpart — there cannot be one.

    An op with no reducer is refused in the tree one layer above the fold, as a
    milestone-tagged NotImplemented at the verb boundary (`lib.rs`). The harness
    refuses it at the same point in the sequence: after the op is recognised,
    before authority is even consulted, and with nothing written.

    43 of 88 ops are in this state. A scenario built on one cannot run, and the
    single most useful thing this model does is say so at the first step rather
    than the last.
    """
    r = Run("O5", "a specified-only op refuses with `not implemented` (no Rust twin)",
            backend, on_step)
    cat = catalogue()
    notes = GroupObject("notes", "group", owner="ada", roster=["ada/phone"])
    _expose(r, cat, notes)

    r.step("the OWNER authors group.setMemberRole — declared, never built")
    decl = cat.op("group", "setMemberRole")
    r.check("the ICD says status=specified", decl.status == "specified",
            f"opId {decl.op_id} · {decl.authority} · {decl.fold}")
    before = notes.digest()
    d = notes.author("ada/phone", "setMemberRole", {"member": "bo", "role": "admin"})
    v = notes.deliver(d, "ada/phone")
    r.check("refused as NotImplemented — not as Unauthorized",
            v.rejection is Rejection.NOT_IMPLEMENTED, v.why)
    r.check("the gate order is the contract: recognised, then refused, then nothing",
            notes.digest() == before and not notes.spine,
            "a refused delta never touches state on its way to being refused")

    r.step("the wallet — all four base ops, looked up by name so a rename is loud")
    verdicts = {}
    for name in ["base.setWalletPolicy", "base.recordDeposit",
                 "base.attestSettlement", "base.attestBalance"]:
        cat.op("group", name)          # raises UnknownOp if the ICD renamed it
        v = notes.deliver(notes.author("ada/phone", name, {}), "ada/phone")
        verdicts[name] = v
    r.check("all four refuse with `not implemented`",
            all(v.rejection is Rejection.NOT_IMPLEMENTED for v in verdicts.values()),
            "the whole treasury is a specification; no scenario may depend on it")
    r.check("and the object is still empty", notes.digest() == before,
            f"{len(notes.refusals)} refusals recorded, 0 deltas folded")

    r.step("every op in the catalogue, authored by the owner of an object of its kind")
    ran, refused, wrong = [], [], []
    for kind in cat.built:
        obj = GroupObject(kind.name, kind, owner="ada", roster=["ada/phone"])
        for op in kind.ops.values():
            v = obj.deliver(obj.author("ada/phone", op.name, {}), "ada/phone")
            missing = v.rejection is Rejection.NOT_IMPLEMENTED
            (refused if missing else ran).append(f"{kind.name}:{op.verb}")
            if missing != (not op.implemented):
                wrong.append(f"{op.name} on {kind.name}: {v.line()}")
    r.check("the implemented ones fold and the specified ones refuse — no exceptions",
            not wrong, "; ".join(wrong) if wrong else
            f"{len(ran)} folded, {len(refused)} refused with `not implemented` "
            f"across {len(cat.built)} kinds")
    worst = sorted(cat.built, key=lambda k: (k.implemented / k.total if k.total else 0))
    total, _impl, spec = cat.totals()
    r.check(f"{spec} of {total} definitions cannot run", len(refused) > 0,
            " · ".join(f"{k.name} {k.implemented}/{k.total}" for k in worst))
    return r.done()


# ── O6 ──────────────────────────────────────────────────────────────────────


def o6_conversation_borrows_the_chat_band(on_step: Callable | None = None,
                                          backend: str = BACKEND) -> CaseResult:
    """chat_ops_are_spliced_verbatim — "Conversation carries the chat band ALONE".

    Conversation has no ops of its own. It `$ref`s forum.post/react/receipt/vote
    — ONE definition carried by two channels — and gets its own kind so the
    prekey pool stays on Contact. The room vocabulary is exactly the difference.

    The base RATIFY three ride every channel (16 Sep 2026) and are not part of that
    claim: they belong to no kind, being recognised by `Coordinator::deliver` ahead of
    every op table. They are checked here as themselves rather than filtered away —
    a conversation really does accept a ratification.
    """
    r = Run("O6", "chat_ops_are_spliced_verbatim — conversation has no ops of its own",
            backend, on_step)
    cat = catalogue()
    chat = GroupObject("chat", "conversation", owner="ada",
                       roster=["ada/phone", "bo/sim"])
    forum = GroupObject("forum", "forum", owner="ada", roster=["ada/phone", "bo/sim"])
    _expose(r, cat, chat, forum)

    r.step("what the kind declares")
    names = sorted(chat.kind.ops)
    own = [n for n in names if not n.startswith("ratify.")]
    base = [n for n in names if n.startswith("ratify.")]
    r.check("four ops, and every one of them is a forum op",
            own == ["forum.post", "forum.react", "forum.receipt", "forum.vote"],
            " ".join(own))
    r.check("plus the base RATIFY three, which every channel carries",
            base == ["ratify.close", "ratify.propose", "ratify.vote"],
            " ".join(base) + " — governance is orthogonal to the kind")
    r.check("they are the SAME objects the forum channel carries — one definition",
            all(chat.kind.ops[n] is forum.kind.ops[n] for n in names),
            "identity, not equality: the ICD $refs components.messages once")
    r.check("all four are implemented, anyMember and commutative",
            all(chat.kind.ops[n].implemented
                and chat.kind.ops[n].authority == "anyMember"
                and chat.kind.ops[n].commutative for n in own),
            f"conversation is {chat.kind.implemented}/{chat.kind.total} implemented")
    r.check("and it is its own kind on the wire — 26, not 19",
            (chat.type_id, forum.type_id) == (26, 19),
            "its own kind so the prekey pool stays on Contact(25)")

    r.step("bo, who is NOT the owner, posts")
    d = chat.author("bo/sim", "post", {"body": "on my way"})
    r.check("accepted — anyMember", bool(chat.deliver(d, "bo/sim")), d.line())

    r.step("the room vocabulary, asked for on a conversation")
    # Counted over the kind's OWN rows: both channels carry the same base ratify three,
    # so counting those would move both sides of a difference that is about rooms.
    forum_own = [n for n in forum.kind.ops if not n.startswith("ratify.")]
    r.check("forum has the two room ops; conversation does not",
            len(forum_own) == 6 and "forum.setRoom" in forum.kind.ops
            and "forum.setRoom" not in chat.kind.ops,
            "no room vocabulary on a thread")
    try:
        chat.author("ada/phone", "setRoom", {"room": "f0" * 32})
        authored, why = True, ""
    except UnknownOp as e:
        authored, why = False, str(e)
    r.check("authoring one fails at the point of authoring", not authored, why)

    r.step("a peer hand-builds the delta anyway, stamped with the right type id")
    smuggled = Delta(type_id=26, op=forum.kind.op("setRoom"), epoch=0, seq=0,
                     args=(("room", "f0" * 32),))
    v = chat.deliver(smuggled, "ada/phone")
    r.check("refused as UnknownType at delivery", v.rejection is Rejection.UNKNOWN_TYPE,
            v.why)

    r.step("and an op from another kind entirely")
    try:
        chat.author("ada/phone", "prekeySupply", {})
        authored = True
    except UnknownOp:
        authored = False
    r.check("contact.prekeySupply is not on a Conversation", not authored,
            "prekeys live on Contact(25); that is why Conversation is a separate kind")
    return r.done()


# ── O7 ──────────────────────────────────────────────────────────────────────


def o7_or_set_fold_is_order_independent(on_step: Callable | None = None,
                                        backend: str = BACKEND) -> CaseResult:
    """or_set_fold_is_order_independent — every permutation, not two.

    "folded in canonical (gen, author, id) order so state is a function of the
    delta SET, never arrival order." The Rust test drives two orders; the model
    can afford all of them, so it drives all of them.
    """
    r = Run("O7", "or_set_fold_is_order_independent — commutative deltas converge",
            backend, on_step)
    cat = catalogue()
    chat = GroupObject("chat", "conversation", owner="ada",
                       roster=["ada/phone", "bo/sim"])
    _expose(r, cat, chat)

    r.step("ada and bo each post, then each react to the other's post")
    ada, bo = Leaf.parse("ada/phone"), Leaf.parse("bo/sim")
    d1 = chat.author(ada, "post", {"body": "from ada"})
    d2 = chat.author(bo, "post", {"body": "from bo"})
    d3 = chat.author(ada, "react", {"emoji": "+1"}, slot=f"react:{d2.short}", gen=1)
    d4 = chat.author(bo, "react", {"emoji": "eyes"}, slot=f"react:{d1.short}", gen=1)
    deltas = [(d1, ada), (d2, bo), (d3, ada), (d4, bo)]
    for d, who in deltas:
        chat.deliver(d, who)
    r.check("all four accepted", len(chat.orset) == 4,
            " | ".join(d.line() for d, _ in deltas))
    r.check("the OR-set folds to four entries", len(chat.state().entries) == 4)

    r.step("the same SET, delivered in all 24 orders")
    agree, reached = converges(chat, deltas)
    r.check("every permutation reaches ONE state", agree,
            f"{len(list(itertools.permutations(range(4))))} orders → "
            f"{len(reached)} distinct digest(s): {list(reached)}")
    r.check("and it is the state the in-order replica reached",
            chat.digest() in reached, chat.digest())

    r.step("the same delta, delivered twice")
    v = chat.deliver(d1, ada)
    r.check("an idempotent no-op, not an error and not a second entry",
            v.duplicate and not v.accepted and len(chat.state().entries) == 4, v.line())

    r.step("ada reacts again on the same slot — per-author last-writer-wins")
    d5 = chat.author(ada, "react", {"emoji": "heart"}, slot=f"react:{d2.short}", gen=2)
    chat.deliver(d5, ada)
    st = chat.state()
    r.check("still four entries: gen 2 supersedes gen 1 for that author+slot",
            len(st.entries) == 4, "; ".join(e.line() for e in st.entries))
    r.check("bo's reaction on his own slot is untouched",
            any(e.author == str(bo) and e.slot == f"react:{d1.short}" for e in st.entries),
            "LWW is per AUTHOR — one member's write never silences another's")
    agree, reached = converges(chat, deltas + [(d5, ada)])
    r.check("and the five-delta set still converges from any order", agree,
            f"120 orders → {len(reached)} digest(s)")
    r.note("the slot is supplied by the SCENARIO, not derived: the ICD's x-graph "
           "clause for a commutative op is almost always an exclusion "
           "(`reactions carry no extractable fact`), so the catalogue does not say "
           "what a react overwrites and the harness will not guess.")
    return r.done()


# ── O8 ──────────────────────────────────────────────────────────────────────


def o8_the_spine_does_not_converge(on_step: Callable | None = None,
                                   backend: str = BACKEND) -> CaseResult:
    """The other half of O7, and the reason the two folds are modelled apart.

    A commutative delta carries no order and needs none. A sequenced delta
    carries its order — (epoch, seq) plus a `prev` hash — and a replica that
    ignores it does not arrive at the same state. That is not a defect to be
    smoothed over: it is what makes `configure`, membership and the roles
    vocabulary safe to own.
    """
    r = Run("O8", "the sequenced spine is a function of the ORDER, not of the set",
            backend, on_step)
    cat = catalogue()
    notes = GroupObject("notes", "group", owner="ada", roster=["ada/phone"])
    _expose(r, cat, notes)
    ada = Leaf.parse("ada/phone")

    r.step("ada authors three owner-sequenced deltas in order")
    chain = []
    for op, args in [("setProfile", {"displayName": "notes", "shape": "team"}),
                     ("setCover", {"media": "blob:cover"}),
                     ("setAffiliation", {"peer": "bb" * 32, "rel": "parent"})]:
        d = notes.author(ada, op, args)
        assert notes.deliver(d, ada), op
        chain.append((d, ada))
    in_order = notes.digest()
    r.check("the spine holds three, hash-chained",
            len(notes.spine) == 3 and notes.spine[1].prev == notes.spine[0].id,
            " → ".join(f"{d.short}@s{d.seq}" for d, _ in chain))

    r.step("a second replica is handed the same SET, in reverse")
    backwards = replica(notes, list(reversed(chain)))
    r.check("it does NOT reach the same state", backwards.digest() != in_order,
            f"in order {in_order} · reversed {backwards.digest()}")
    r.check("because the chain refused what arrived out of turn",
            [v.rejection for _, _, v in backwards.refusals] ==
            [Rejection.CHAIN_BROKEN, Rejection.CHAIN_BROKEN],
            "; ".join(v.why for _, _, v in backwards.refusals))
    r.check("only the genesis delta survived", len(backwards.spine) == 1,
            f"spine={len(backwards.spine)} of 3")

    r.step("every permutation of the same three")
    agree, reached = converges(notes, chain)
    r.check("the sequenced fold reaches several different states", not agree,
            f"6 orders → {len(reached)} distinct digests")
    r.check("and only ONE of them is the full spine",
            sum(1 for digest in reached if digest == in_order) == 1,
            "the total order is carried in the delta; ignoring it is detected, "
            "not tolerated")

    r.step("the refused deltas are re-presented once their predecessor has landed")
    for d, who in chain:
        backwards.deliver(d, who)
    r.check("and the replica converges", backwards.digest() == in_order,
            "a replica must BUFFER and re-present; what it may not do is reorder")
    r.step("the same experiment, run on a commutative set of the same size")
    chat = GroupObject("chat", "conversation", owner="ada",
                       roster=["ada/phone", "bo/sim"])
    bo = Leaf.parse("bo/sim")
    comm = [(chat.author(ada, "post", {"body": "one"}), ada),
            (chat.author(bo, "post", {"body": "two"}), bo),
            (chat.author(ada, "receipt", {"upto": "two"}, slot="cursor", gen=1), ada)]
    agree2, reached2 = converges(chat, comm)
    r.check("three commutative deltas, six orders, ONE state", agree2,
            f"{len(reached2)} digest: {list(reached2)}")
    r.check("three sequenced deltas, six orders, several",
            len(reached) > 1,
            "the difference is not a bug in either fold — it is what `sequenced` and "
            "`commutative` MEAN, and the catalogue says which each op is")
    return r.done()


# ── O9 ──────────────────────────────────────────────────────────────────────


def o9_the_spine_is_loud(on_step: Callable | None = None,
                         backend: str = BACKEND) -> CaseResult:
    """chain_broken_on_bad_prev · gap_in_seq_breaks_chain ·
    fork_detected_at_same_position · duplicate_is_idempotent ·
    epoch_advance_resets_seq · stale_epoch_rejected_after_fence.

    Six Rust tests in one case, because they are six arms of one gate and the
    ORDER of the arms is itself the contract: fork before chain, chain before
    position.
    """
    r = Run("O9", "chain_broken · fork_detected · stale_epoch — nothing is swallowed",
            backend, on_step)
    cat = catalogue()
    notes = GroupObject("notes", "group", owner="ada", roster=["ada/phone"])
    _expose(r, cat, notes)
    ada = Leaf.parse("ada/phone")

    r.step("genesis, then one more")
    d0 = notes.author(ada, "setProfile", {"displayName": "notes", "shape": "team"})
    notes.deliver(d0, ada)
    d1 = notes.author(ada, "setCover", {"media": "blob:1"})
    notes.deliver(d1, ada)
    r.check("two on the spine", len(notes.spine) == 2)

    r.step("the same delta again")
    v = notes.deliver(d1, ada)
    r.check("duplicate_is_idempotent — a no-op, not a fork",
            v.duplicate and len(notes.spine) == 2, v.line())

    r.step("a DIFFERENT delta at the same position")
    fork = notes.author(ada, "setCover", {"media": "blob:other"}, at=(0, 1),
                        prev=d0.id)
    v = notes.deliver(fork, ada)
    r.check("fork_detected_at_same_position — a divergence halt",
            v.rejection is Rejection.FORK_DETECTED, v.why)

    r.step("a delta whose prev points nowhere")
    bad = notes.author(ada, "setCover", {"media": "blob:2"}, at=(0, 2), prev="ff" * 32)
    v = notes.deliver(bad, ada)
    r.check("chain_broken_on_bad_prev", v.rejection is Rejection.CHAIN_BROKEN, v.why)

    r.step("a delta that skips a seq")
    gap = notes.author(ada, "setCover", {"media": "blob:3"}, at=(0, 7), prev=d1.id)
    v = notes.deliver(gap, ada)
    r.check("gap_in_seq_breaks_chain", v.rejection is Rejection.CHAIN_BROKEN, v.why)

    r.step("the group rekeys — an MLS epoch bump")
    notes.rekey("bo joins")
    nxt = notes.author(ada, "setCover", {"media": "blob:e1"})
    r.check("epoch_advance_resets_seq — (e1, s0), still chained to the old head",
            (nxt.epoch, nxt.seq, nxt.prev) == (1, 0, d1.id),
            f"e{nxt.epoch}·s{nxt.seq} prev={nxt.prev[:8]}")
    r.check("accepted", bool(notes.deliver(nxt, ada)))

    r.step("a delta authored before the fence arrives late")
    late = notes.author(ada, "setCover", {"media": "blob:late"}, at=(0, 2), prev=d1.id)
    v = notes.deliver(late, ada)
    r.check("stale_epoch_rejected_after_fence", v.rejection is Rejection.STALE_EPOCH,
            v.why)
    r.check("every refusal is typed, and none of them touched the spine",
            len(notes.refusals) == 4 and len(notes.spine) == 3,
            f"{len(notes.refusals)} refusals: "
            + " · ".join(sorted({str(v.rejection) for _, _, v in notes.refusals}))
            + f" · spine still {len(notes.spine)} (the duplicate is not a refusal)")
    return r.done()


# ── O10 ─────────────────────────────────────────────────────────────────────


def o10_declared_but_not_built(on_step: Callable | None = None,
                               backend: str = BACKEND) -> CaseResult:
    """every_built_kind_has_a_channel — and the three kinds that do not have one.

    Field(20), System(21) and Topic(23) are in `ObjectKind::ALL`, have wire type
    ids, and round-trip through `from_type_id`. They have no ICD channel and no
    op table, which is a different thing from a kind whose ops are specified: an
    op on a Project can be named and refused, and an op on a Field cannot be
    named at all.
    """
    r = Run("O10", "every_built_kind_has_a_channel — declared is not built",
            backend, on_step)
    cat = catalogue()

    r.step("the twelve kinds, and which of them have an op table")
    built = {k.name for k in cat.built}
    hollow = {k.name for k in cat.declared_only}
    # COUNTED, NOT TRANSCRIBED. This pair used to read "nine built, three declared
    # only" against a hardcoded (9, 3). It went red on 15 Sep 2026 with core green,
    # because System(21) and Post(30) had both crossed from declared to built and
    # nobody came back here. The split is a thing that MOVES — that is the whole
    # point of the project — so the number is reported and the INVARIANT is asserted:
    # the two sets partition the taxonomy exactly, with nothing in both and nothing
    # in neither.
    r.check("built and declared-only partition the twelve kinds, with no overlap",
            not (built & hollow) and len(built) + len(hollow) == len(cat.all_order) == 12,
            f"{len(built)} built: {' '.join(sorted(built))} · "
            f"{len(hollow)} declared only: {' '.join(sorted(hollow))}")
    # THIS one is genuinely pinned, and pinned in the Rust with a stated reason:
    # `catalogue_wire_values.rs` excludes exactly Field(20) and Topic(23) from its
    # sweep "because neither has an `impl ObjectType`", and the ICD's `anchor`
    # excludes them on the same grounds and in the same words. A third kind arriving
    # here means a kind lost its lens, which is a finding and should be loud.
    r.check("and the ones with no lens are Field(20) and Topic(23) — the two the Rust excludes",
            {k.type_id for k in cat.declared_only} == {20, 23},
            " ".join(f"{k.name}={k.type_id}" for k in cat.declared_only))

    r.step("an object of one of them")
    atlas = GroupObject("atlas", "field", owner="ada", roster=["ada/phone"])
    objects = [GroupObject("notes", "group", owner="ada", roster=["ada/phone"]),
               GroupObject("build", "project", owner="ada", roster=["ada/phone"]),
               GroupObject("chat", "conversation", owner="ada",
                           roster=["ada/phone", "bo/sim"]),
               atlas]
    _expose(r, cat, *objects)
    r.check("it mints — the kind is real", atlas.type_id == 20)
    try:
        atlas.author("ada/phone", "setKeyIndex", {})
        named, why = True, ""
    except UnknownOp as e:
        named, why = False, str(e)
    r.check("but no op on it can even be NAMED", not named, why)
    r.check("0 ops, and the panel says so", atlas.kind.total == 0, atlas.line())

    r.step("the built kinds, and how much of each is real")
    project = cat.kind("project")
    spec = sorted(o.verb for o in project.ops.values() if not o.implemented)
    r.check(f"project is {project.implemented}/{project.total} implemented",
            project.implemented < project.total,
            "specified: " + " ".join(spec))
    whole = [k.name for k in cat.built if k.implemented == k.total]
    # Named a fixed three until 15 Sep 2026, when System(21) finished and made it
    # four. The CLAIM this case exists to make is not which kinds are whole — that
    # is a burndown and it is supposed to move — it is that the catalogue is still
    # PARTIAL: some kinds fold everything they declare and some do not, so no
    # scenario may assume an op exists merely because the ICD names it.
    r.check("the catalogue is still partial — some kinds are whole, some are not",
            0 < len(whole) < len(cat.built),
            f"whole: {' '.join(sorted(whole))} · "
            + " · ".join(f"{k.name} {k.implemented}/{k.total}" for k in cat.built))
    r.note("RATIFY (propose/vote/close, 0xF000-0xF002) is accepted by the Coordinator "
           "on EVERY object and was in NO op table and NO ICD channel — three ops on "
           "the wire the catalogue did not document. DOCUMENTED 16 Sep 2026: all three "
           "are now messages on every channel, pinned by icd.rs against "
           "coordinator::RATIFY_OPS rather than against an op table, since they are in "
           "none and must stay in none — the Coordinator folds them so that no kind's "
           "reducer learns about voting. They are still in no op table, which is why "
           "they are absent from core-wasm's ops_catalogue and cannot be authored from "
           "a browser.")
    return r.done()


ALL = [o1_the_catalogue_is_read_not_transcribed,
       o2_reserved_type_ids_fail_loud,
       o3_the_spec_invariant,
       o4_owner_authority_is_per_person,
       o5_specified_only_refuses,
       o6_conversation_borrows_the_chat_band,
       o7_or_set_fold_is_order_independent,
       o8_the_spine_does_not_converge,
       o9_the_spine_is_loud,
       o10_declared_but_not_built]


def run_all(on_step: Callable | None = None) -> list[CaseResult]:
    return [case(on_step=on_step) for case in ALL]
