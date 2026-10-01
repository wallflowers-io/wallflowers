"""test_objects — pytest over the catalogue and the object model.

`python -m harness objects` runs the ten cases as a narrated report; this runs
the same rules as assertions, plus the unit-level edges a case would not narrate.
Both are worth having: the cases are the argument, these are the fence.

Nothing here needs the relay, the bucket, a device or a network. It needs
`core/` on disk, because that is where the catalogue is.
"""
from __future__ import annotations

import itertools

import pytest

from harness.objects import (BASE_OP_NAMESPACE, Catalogue, GroupObject, Kind, Leaf,
                             Op, Rejection, ReservedTypeId, SpecViolation, UnknownOp,
                             catalogue, converges, replica)
from harness.objects.cases import ALL as OBJECT_CASES


@pytest.fixture(scope="module")
def cat() -> Catalogue:
    return catalogue()


# ── the catalogue ───────────────────────────────────────────────────────────


def test_the_two_sources_parse_and_agree(cat):
    assert cat.check() == []
    assert len(cat.all_order) == 12          # all_lists_every_kind — a CLOSED taxonomy
    # NOT a magic 9. This read "== 9" until 15 Sep 2026 and went red with core green:
    # System(21) and Post(30) had crossed from declared to built. How many kinds are
    # built is a burndown and is MEANT to move, so the structural fact is asserted
    # instead — every built kind is built on a channel, and the two sets partition
    # the taxonomy with nothing in both and nothing in neither.
    assert all(k.channel for k in cat.built)
    assert len(cat.built) + len(cat.declared_only) == len(cat.all_order)
    assert not ({k.name for k in cat.built} & {k.name for k in cat.declared_only})
    total, impl, spec = cat.totals()
    assert total == impl + spec and spec > 0
    for kind in cat.built:
        assert kind.total > 0
        assert cat.kind_for_type_id(kind.type_id) is kind


def test_every_op_declares_authority_fold_and_status(cat):
    for name, op in cat.ops.items():
        assert op.authority in ("owner", "anyMember"), name
        assert op.fold in ("sequenced", "commutative"), name
        assert op.status in ("implemented", "specified"), name
        assert op.well_formed, name          # spec_invariant_commutative_is_any_member
        assert op.base == (op.op_id >= BASE_OP_NAMESPACE), name


def test_reserved_type_ids_raise(cat):
    assert sorted(cat.reserved) == [0, 16, 17, 24]
    for tid in cat.reserved:
        with pytest.raises(ReservedTypeId):
            cat.kind_for_type_id(tid)
    assert 24 not in {k.type_id for k in cat}
    assert cat.kind("place").type_id == 28   # the next FREE id, not 24


def test_conversation_carries_the_chat_band_alone(cat):
    conv, forum = cat.kind("conversation"), cat.kind("forum")
    assert sorted(conv.ops) == ["forum.post", "forum.react", "forum.receipt",
                                "forum.vote"]
    assert all(conv.ops[n] is forum.ops[n] for n in conv.ops)
    assert "forum.setRoom" in forum.ops and "forum.setRoom" not in conv.ops
    with pytest.raises(UnknownOp):
        conv.op("setRoom")


def test_declared_but_unbuilt_kinds_have_no_ops(cat):
    hollow = {k.name: k.type_id for k in cat.declared_only}
    # Field(20) and Topic(23), and only those two. This is not a count that drifts:
    # `catalogue_wire_values.rs` excludes exactly these from its sweep "because
    # neither has an `impl ObjectType`", and the ICD's `anchor` excludes them on the
    # same grounds and in the same words. System(21) was in this set until 15 Sep
    # 2026 and has since been built; a THIRD name appearing here means a kind lost
    # its lens, which should fail loudly.
    assert hollow == {"field": 20, "topic": 23}
    for name in hollow:
        assert cat.kind(name).total == 0
        with pytest.raises(UnknownOp):
            cat.kind(name).op("anything")


# ── the model ───────────────────────────────────────────────────────────────


def obj(kind="group", owner="ada", roster=("ada/phone",), name="o") -> GroupObject:
    return GroupObject(name, kind, owner=owner, roster=list(roster))


def test_the_roster_folds_to_people():
    o = obj(roster=["ada/phone", "ada/laptop", "bo/sim"])
    assert len(o.leaves) == 3
    assert o.people == ("ada", "bo")


def test_an_owner_with_no_leaf_cannot_exist():
    with pytest.raises(ValueError):
        obj(owner="cal", roster=["ada/phone"])


def test_a_reserved_id_cannot_back_an_object():
    with pytest.raises(ReservedTypeId):
        GroupObject("poll", 16, owner="ada", roster=["ada/phone"])


def test_a_commutative_owner_op_cannot_back_an_object():
    rogue = Kind(name="rogue", type_id=99, channel="rogue", ops={
        "rogue.broadcast": Op(name="rogue.broadcast", channel="rogue", verb="broadcast",
                              op_id=0, authority="owner", fold="commutative",
                              status="implemented", node=("label",))})
    with pytest.raises(SpecViolation):
        GroupObject("rogue", rogue, owner="ada", roster=["ada/phone"])


def test_owner_authority_is_per_person_not_per_leaf():
    o = obj(roster=["ada/phone", "ada/laptop", "bo/sim"])
    assert o.deliver(o.author("ada/phone", "setProfile",
                              {"displayName": "n", "shape": "team"}), "ada/phone")
    # the owner's SECOND DEVICE is still the owner
    assert o.deliver(o.author("ada/laptop", "setCover", {"media": "x"}), "ada/laptop")
    # a member who is not the owner is not
    v = o.deliver(o.author("bo/sim", "setCover", {"media": "y"}), "bo/sim")
    assert v.rejection is Rejection.UNAUTHORIZED
    # and someone holding no leaf at all is not a member
    v = o.deliver(o.author("cal/phone", "setCover", {"media": "z"}), "cal/phone")
    assert v.rejection is Rejection.UNAUTHORIZED


def test_the_spine_is_owner_only_even_where_the_declaration_is_not():
    """The one place the model and the declaration could disagree, pinned.

    `SequencedCoordinator::deliver` refuses a non-owner on the ARM ("only the
    owner sequences the spine"), not on the op's declared authority. Every
    sequenced op in today's catalogue is owner-authority, so the two rules
    coincide and nothing distinguishes them — this builds the op that would.
    """
    odd = Kind(name="odd", type_id=98, channel="odd", ops={
        "odd.announce": Op(name="odd.announce", channel="odd", verb="announce",
                           op_id=0, authority="anyMember", fold="sequenced",
                           status="implemented", node=("label",))})
    o = GroupObject("odd", odd, owner="ada", roster=["ada/phone", "bo/sim"])
    assert odd.ops["odd.announce"].well_formed          # not the spec invariant
    v = o.deliver(o.author("bo/sim", "announce", {}), "bo/sim")
    assert v.rejection is Rejection.UNAUTHORIZED
    assert "sequences the spine" in v.why
    assert o.deliver(o.author("ada/phone", "announce", {}), "ada/phone")


def test_a_specified_only_op_refuses_and_changes_nothing():
    o = obj()
    before = o.digest()
    v = o.deliver(o.author("ada/phone", "setMemberRole", {"role": "admin"}), "ada/phone")
    assert v.rejection is Rejection.NOT_IMPLEMENTED
    assert o.digest() == before and not o.spine


@pytest.mark.parametrize("name", ["base.setWalletPolicy", "base.recordDeposit",
                                  "base.attestSettlement", "base.attestBalance"])
def test_the_wallet_is_specification_only(name):
    o = obj()
    v = o.deliver(o.author("ada/phone", name, {}), "ada/phone")
    assert v.rejection is Rejection.NOT_IMPLEMENTED


def test_every_op_folds_iff_the_catalogue_says_it_is_implemented(cat):
    """The sweep: 94 channel rows, each authored by the object's owner."""
    for kind in cat.built:
        o = GroupObject(kind.name, kind, owner="ada", roster=["ada/phone"])
        for op in kind.ops.values():
            v = o.deliver(o.author("ada/phone", op.name, {}), "ada/phone")
            missing = v.rejection is Rejection.NOT_IMPLEMENTED
            assert missing == (not op.implemented), f"{kind.name}/{op.name}: {v.line()}"


def test_a_delta_for_another_kind_is_unknown_type():
    o = obj()
    v = o.deliver(o.author("ada/phone", "setProfile",
                           {"displayName": "n", "shape": "team"}, type_id=19), "ada/phone")
    assert v.rejection is Rejection.UNKNOWN_TYPE


def test_a_delta_on_a_reserved_type_id_names_what_it_used_to_be():
    o = obj()
    v = o.deliver(o.author("ada/phone", "setProfile",
                           {"displayName": "n", "shape": "team"}, type_id=17), "ada/phone")
    assert v.rejection is Rejection.UNKNOWN_TYPE
    assert "Message" in v.why and "RESERVED" in v.why


# ── the two folds ───────────────────────────────────────────────────────────


def _chat() -> tuple[GroupObject, Leaf, Leaf]:
    c = GroupObject("chat", "conversation", owner="ada",
                    roster=["ada/phone", "bo/sim"])
    return c, Leaf.parse("ada/phone"), Leaf.parse("bo/sim")


def test_the_or_set_converges_under_every_order():
    chat, ada, bo = _chat()
    deltas = [(chat.author(ada, "post", {"body": "a"}), ada),
              (chat.author(bo, "post", {"body": "b"}), bo),
              (chat.author(ada, "react", {"e": "+1"}, slot="r1", gen=1), ada),
              (chat.author(bo, "receipt", {"upto": "b"}, slot="cursor", gen=1), bo)]
    agree, reached = converges(chat, deltas)
    assert agree, reached
    assert len(list(itertools.permutations(range(4)))) == 24


def test_the_or_set_is_per_author_last_writer_wins():
    chat, ada, bo = _chat()
    first = chat.author(ada, "react", {"e": "+1"}, slot="r1", gen=1)
    second = chat.author(ada, "react", {"e": "heart"}, slot="r1", gen=2)
    theirs = chat.author(bo, "react", {"e": "eyes"}, slot="r1", gen=1)
    for d, who in [(first, ada), (second, ada), (theirs, bo)]:
        assert chat.deliver(d, who)
    entries = chat.state().entries
    assert len(entries) == 2                       # one per author, not one per slot
    mine = [e for e in entries if e.author == str(ada)]
    assert mine[0].gen == 2 and mine[0].delta == second.short


def test_a_duplicate_is_idempotent():
    chat, ada, _ = _chat()
    d = chat.author(ada, "post", {"body": "once"})
    assert chat.deliver(d, ada).accepted
    again = chat.deliver(d, ada)
    assert again.duplicate and not again.accepted
    assert len(chat.state().entries) == 1


def test_the_spine_does_not_converge_under_reordering():
    o = obj()
    ada = Leaf.parse("ada/phone")
    chain = []
    for op, args in [("setProfile", {"displayName": "n", "shape": "team"}),
                     ("setCover", {"media": "c"}),
                     ("setAffiliation", {"peer": "bb" * 32, "rel": "parent"})]:
        d = o.author(ada, op, args)
        assert o.deliver(d, ada)
        chain.append((d, ada))
    agree, reached = converges(o, chain)
    assert not agree and len(reached) > 1
    # ...and re-presenting what was refused recovers the one true spine
    twin = replica(o, list(reversed(chain)))
    assert twin.digest() != o.digest()
    for d, who in chain:
        twin.deliver(d, who)
    assert twin.digest() == o.digest()


def test_the_spine_is_loud_on_every_violation():
    o = obj()
    ada = Leaf.parse("ada/phone")
    d0 = o.author(ada, "setProfile", {"displayName": "n", "shape": "team"})
    o.deliver(d0, ada)
    d1 = o.author(ada, "setCover", {"media": "1"})
    o.deliver(d1, ada)

    assert o.deliver(d1, ada).duplicate
    assert o.deliver(o.author(ada, "setCover", {"media": "2"}, at=(0, 1), prev=d0.id),
                     ada).rejection is Rejection.FORK_DETECTED
    assert o.deliver(o.author(ada, "setCover", {"media": "3"}, at=(0, 2), prev="ff" * 32),
                     ada).rejection is Rejection.CHAIN_BROKEN
    assert o.deliver(o.author(ada, "setCover", {"media": "4"}, at=(0, 9), prev=d1.id),
                     ada).rejection is Rejection.CHAIN_BROKEN
    o.rekey("bo joins")
    nxt = o.author(ada, "setCover", {"media": "5"})
    assert (nxt.epoch, nxt.seq, nxt.prev) == (1, 0, d1.id)
    assert o.deliver(nxt, ada)
    assert o.deliver(o.author(ada, "setCover", {"media": "6"}, at=(0, 2), prev=d1.id),
                     ada).rejection is Rejection.STALE_EPOCH
    assert len(o.spine) == 3


def test_the_delta_id_excludes_the_author():
    """`Delta::id()` hashes the envelope, never the author — authorship comes from
    the MLS transport. Two members authoring the identical envelope produce the
    identical id, which is exactly why the OR-set dedups on (author, id)."""
    chat, ada, bo = _chat()
    mine = chat.author(ada, "post", {"body": "same"}, gen=0)
    theirs = chat.author(bo, "post", {"body": "same"}, gen=0)
    assert mine.id == theirs.id
    assert chat.deliver(mine, ada).accepted
    assert chat.deliver(theirs, bo).accepted          # a different author: not a dup
    assert chat.deliver(theirs, bo).duplicate
    assert len(chat.state().entries) == 2


# ── the cases themselves ────────────────────────────────────────────────────


@pytest.mark.parametrize("case", OBJECT_CASES, ids=lambda c: c.__name__)
def test_case_is_green(case):
    result = case()
    failed = [c.label for c in result.checks if not c.ok]
    assert not failed, f"{result.name}: {failed}"
    assert result.checks, f"{result.name} asserted nothing"
