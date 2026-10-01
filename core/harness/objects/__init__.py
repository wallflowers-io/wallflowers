"""objects — the REAL object catalogue, and a GroupObject that is one of its kinds.

    from harness.objects import GroupObject, Leaf, Rejection, catalogue

    cat = catalogue()                       # parsed from core/, never transcribed
    notes = GroupObject("notes", "group", owner="ada",
                        roster=["ada/phone", "ada/laptop", "bo/sim"])

    notes.people            # ('ada', 'bo')      — the roster FOLDED TO PEOPLE
    notes.type_id           # 18                 — the wire id, from object.rs
    notes.kind.implemented  # 9 of 26            — what is actually built

    d = notes.author("bo/sim", "setCover", {"media": "blob:x"})
    notes.deliver(d, "bo/sim").rejection        # Rejection.UNAUTHORIZED

Three sentences of context, because they are the whole design:

  * the KINDS and their wire type ids come from `pacific-core/src/object.rs`; the
    OPS come from `coordination/delta-graph.icd.json`. Both are parsed. Nothing
    in here is a copy, which is the only way a second implementation of a
    catalogue stays in step with the first.
  * every op carries an authority, a fold and a STATUS, and the harness enforces
    all three — including the status, so a scenario reaching for one of the 43
    specified-only ops is told at the first step instead of being quietly humoured.
  * no reducer runs here. A green run proves the DESIGN is coherent. The Rust
    milestone tests remain the only proof of the implementation.
"""
from .catalogue import (BASE_OP_NAMESPACE, Catalogue, CatalogueError, ICD_PATH, Kind,
                        Op, OBJECT_RS_PATH, ReservedTypeId, UnknownKind, UnknownOp,
                        catalogue)
from .model import (GENESIS_PREV, Delta, Entry, GroupObject, Leaf, NodeWrite,
                    ObjectState, Rejection, SpecViolation, Verdict, converges,
                    replica, validate_ops)

__all__ = [
    "Catalogue", "CatalogueError", "Kind", "Op", "ReservedTypeId", "UnknownKind",
    "UnknownOp", "catalogue", "BASE_OP_NAMESPACE", "ICD_PATH", "OBJECT_RS_PATH",
    "Delta", "Entry", "GroupObject", "Leaf", "NodeWrite", "ObjectState", "Rejection",
    "SpecViolation", "Verdict", "converges", "replica", "validate_ops", "GENESIS_PREV",
]
