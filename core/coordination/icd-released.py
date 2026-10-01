#!/usr/bin/env python3
"""The released-op check (PIN-7's first half; mdr/icd-pin.md, architecture.md § Change without loss):
the ICD in the tree against the ICD a release carried. What a released build wrote must keep its
meaning, so a revision may only add:

  - every released kind stays, with its typeId; no new kind takes a released typeId;
  - every released op stays in its kind, with its op id, its authority (ego) and its fold;
    none is removed (a retired op keeps its entry); no new op takes a released op id;
  - every released arg stays, with its type and whether it is required; a new arg is optional;
  - the same for every facet's ops (op ids are global there), and a facet stays on every kind it was;
  - retired ids (`kinds.<k>.retired` or `facets.<f>.retired`, op id → former op name): no op takes
    one; a released retirement stays, with its name; a released op dropped is retired on its id.

  icd-released.py [<ref>]   the release: the newest icd/* tag, or before the first, the mint's
                            commit (fd81568, the pin d3d5ef28…); exit 1, naming each break
  icd-released.py --selftest  each rule broken in turn on a copy of the tree's ICD must be refused

A break Ralph waives is named, exactly, in delta-graph.icd.departures.json beside the ICD, with
the W- row that waives it: {"waiver": "W-NN", "departures": ["<the break as printed>", ...]}.
The row must be in status.md. A break not named stays red, and a named one that no longer
occurs is red too, so a waiver cannot outlive its break or cover another.
"""
from __future__ import annotations

import copy
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ICD = HERE / "delta-graph.icd.json"
DEPARTURES = HERE / "delta-graph.icd.departures.json"
STATUS = HERE.parent / "docs" / "launch" / "status.md"
MINT = "fd81568"   # the build Ralph's mint ran on (status.md, 28 Sep), until icd/2.0.0 is tagged


def released_ref() -> str:
    tags = subprocess.run(["git", "-C", str(HERE), "for-each-ref", "--sort=-v:refname", "--format=%(refname:short)",
                           "refs/tags/icd/"], capture_output=True, text=True).stdout.split()
    return tags[0] if tags else MINT


def released(ref: str) -> dict:
    rel = ICD.relative_to(Path(subprocess.run(["git", "-C", str(HERE), "rev-parse", "--show-toplevel"],
                                              capture_output=True, text=True, check=True).stdout.strip()))
    out = subprocess.run(["git", "-C", str(HERE), "show", f"{ref}:{rel}"], capture_output=True, text=True)
    if out.returncode:
        sys.exit(f"✗ icd: no ICD at {ref}: {out.stderr.strip()[:200]}")
    return json.loads(out.stdout)


def op_breaks(name: str, o_ops: dict, n_ops: dict, taken: dict) -> list[str]:
    """One kind's or facet's ops: `taken` maps each released op id the new ops may not reuse to its op."""
    found = []
    for op, o in n_ops.items():
        if op not in o_ops and o.get("op") in taken:
            found.append(f"{name}: {op} takes {taken[o['op']]}'s op id {o['op']}")
    for op, o in o_ops.items():
        if op not in n_ops:
            found.append(f"{name}: {op} removed")
            continue
        p = n_ops[op]
        for field in ("op", "ego", "fold"):
            if p.get(field) != o.get(field):
                found.append(f"{name}: {op}: {field} {o.get(field)!r} became {p.get(field)!r}")
        oa, na = o.get("args", {}), p.get("args", {})
        for arg, a in oa.items():
            if arg not in na:
                found.append(f"{name}: {op}: arg {arg} removed")
                continue
            b = na[arg]
            if b.get("type") != a.get("type"):
                found.append(f"{name}: {op}: arg {arg}'s type {a.get('type')!r} became {b.get('type')!r}")
            if bool(b.get("required")) != bool(a.get("required")):
                found.append(f"{name}: {op}: arg {arg} {'became required' if b.get('required') else 'is required no longer'}")
        for arg, b in na.items():
            if arg not in oa and b.get("required"):
                found.append(f"{name}: {op}: new arg {arg} is required")
    return found


def retired_breaks(name: str, old: dict, new: dict) -> list[str]:
    """No op on a retired id; a released retirement stays as it was; a released op dropped is retired."""
    found = []
    o_ret, n_ret = old.get("retired", {}), new.get("retired", {})
    for op, o in new.get("ops", {}).items():
        if str(o.get("op")) in n_ret:
            found.append(f"{name}: {op} takes retired op id {o.get('op')} ({n_ret[str(o.get('op'))]})")
    for i, was in o_ret.items():
        if i not in n_ret:
            found.append(f"{name}: retired op id {i} ({was}) removed")
        elif n_ret[i] != was:
            found.append(f"{name}: retired op id {i} ({was}) became {n_ret[i]}")
    for op, o in old.get("ops", {}).items():
        if op not in new.get("ops", {}) and n_ret.get(str(o.get("op"))) != op:
            found.append(f"{name}: {op} dropped without retiring op id {o.get('op')}")
    return found


def breaks(old: dict, new: dict) -> list[str]:
    found = []
    ok, nk = old["kinds"], new["kinds"]
    released_types = {k["typeId"]: name for name, k in ok.items()}
    for name, k in nk.items():
        if name not in ok and k.get("typeId") in released_types:
            found.append(f"kind {name} takes {released_types[k['typeId']]}'s typeId {k['typeId']}")
    for name, k in ok.items():
        if name not in nk:
            found.append(f"kind {name} removed")
            continue
        n = nk[name]
        if n.get("typeId") != k["typeId"]:
            found.append(f"kind {name}: typeId {k['typeId']} became {n.get('typeId')}")
        oo = k.get("ops", {})
        found += op_breaks(name, oo, n.get("ops", {}), {o["op"]: op for op, o in oo.items()})
        found += retired_breaks(name, k, n)
    for name, n in nk.items():
        if name not in ok:
            found += retired_breaks(name, {}, n)
    of, nf = old.get("facets", {}), new.get("facets", {})
    facet_ids = {o["op"]: op for f in of.values() for op, o in f.get("ops", {}).items()}
    for name, f in of.items():
        if name not in nf:
            found.append(f"facet {name} removed")
            continue
        n = nf[name]
        for kind in f.get("on", []):
            if kind not in n.get("on", []):
                found.append(f"facet {name}: no longer on {kind}")
        found += op_breaks(f"facet {name}", f.get("ops", {}), n.get("ops", {}), facet_ids)
        found += retired_breaks(f"facet {name}", f, n)
    for name, n in nf.items():
        if name not in of:
            found += op_breaks(f"facet {name}", {}, n.get("ops", {}), facet_ids)
            found += retired_breaks(f"facet {name}", {}, n)
    return found


def judge(found: list[str], waived: dict | None) -> tuple[list[str], list[str]]:
    """(the breaks that fail, the waived ones). A waiver's row must be in status.md; a named
    departure that is not among the breaks fails as stale."""
    if not waived:
        return found, []
    row = waived.get("waiver", "")
    listed = waived.get("departures", [])
    have = STATUS.exists() and any(l.startswith(f"| {row} |") for l in STATUS.read_text().splitlines())
    if not have:
        return found + [f"the departures' waiver {row!r} is no W- row in status.md"], []
    fail = [f for f in found if f not in listed] + [f"waived, and no longer a break: {d}" for d in listed if d not in found]
    return fail, [f for f in found if f in listed]


def selftest() -> int:
    tree = json.loads(ICD.read_text())
    kind = next(k for k, v in tree["kinds"].items() if v.get("ops"))
    ops = tree["kinds"][kind]["ops"]
    op = next(o for o, v in ops.items() if v.get("args"))
    arg = next(iter(ops[op]["args"]))
    other = next(k for k in tree["kinds"] if k != kind)
    facet = next(f for f, v in tree["facets"].items() if v.get("ops") and v.get("on"))
    fops = tree["facets"][facet]["ops"]
    fop = next(iter(fops))
    # The kind's own retirements, kept in every mutant: an ICD may already carry some (2.1.0 does).
    held = dict(tree["kinds"][kind].get("retired", {}))
    ret = lambda extra: {**held, **extra}
    unused = 1 + max([v["op"] for v in ops.values()] + [int(i) for i in held])

    def m(f):
        t = copy.deepcopy(tree)
        f(t)
        return t
    mutants = {
        "an op's id changed": m(lambda t: t["kinds"][kind]["ops"][op].__setitem__("op", 9999)),
        "an op's authority changed": m(lambda t: t["kinds"][kind]["ops"][op].__setitem__("ego", "anyone" if ops[op].get("ego") != "anyone" else "owner")),
        "an op's fold changed": m(lambda t: t["kinds"][kind]["ops"][op].__setitem__("fold", "commutative" if ops[op].get("fold") != "commutative" else "sequenced")),
        "an op removed": m(lambda t: t["kinds"][kind]["ops"].pop(op)),
        "a new op on a released id": m(lambda t: t["kinds"][kind]["ops"].__setitem__(f"{kind}.selftestOp", {"op": ops[op]["op"], "ego": "owner", "fold": "sequenced", "args": {}})),
        "an arg removed": m(lambda t: t["kinds"][kind]["ops"][op]["args"].pop(arg)),
        "an arg's type changed": m(lambda t: t["kinds"][kind]["ops"][op]["args"][arg].__setitem__("type", "integer" if ops[op]["args"][arg].get("type") != "integer" else "string")),
        "an arg's required changed": m(lambda t: t["kinds"][kind]["ops"][op]["args"][arg].__setitem__("required", not ops[op]["args"][arg].get("required"))),
        "a new required arg": m(lambda t: t["kinds"][kind]["ops"][op]["args"].__setitem__("selftestArg", {"type": "string", "required": True})),
        "a kind's typeId changed": m(lambda t: t["kinds"][kind].__setitem__("typeId", 9999)),
        "a kind removed": m(lambda t: t["kinds"].pop(other)),
        "a new kind on a released typeId": m(lambda t: t["kinds"].__setitem__("selftestKind", {"typeId": tree["kinds"][kind]["typeId"], "ops": {}})),
        "a facet op's id changed": m(lambda t: t["facets"][facet]["ops"][fop].__setitem__("op", 9999)),
        "a facet op removed": m(lambda t: t["facets"][facet]["ops"].pop(fop)),
        "a new required arg on a facet op": m(lambda t: t["facets"][facet]["ops"][fop].setdefault("args", {}).__setitem__("selftestArg", {"type": "string", "required": True})),
        "a new facet op on a released id": m(lambda t: t["facets"][facet]["ops"].__setitem__("base.selftestOp", {"op": fops[fop]["op"], "ego": "owner", "fold": "sequenced", "args": {}})),
        "a facet no longer on a kind": m(lambda t: t["facets"][facet]["on"].pop()),
        "a facet removed": m(lambda t: t["facets"].pop(facet)),
        "a new op on a retired id": m(lambda t: (t["kinds"][kind].__setitem__("retired", ret({str(unused): f"{kind}.selftestGone"})),
                                                 t["kinds"][kind]["ops"].__setitem__(f"{kind}.selftestOp", {"op": unused, "ego": "owner", "fold": "sequenced", "args": {}}))),
    }
    allowed = {
        "a new optional arg": m(lambda t: t["kinds"][kind]["ops"][op]["args"].__setitem__("selftestArg", {"type": "string"})),
        "a new op on a new id": m(lambda t: t["kinds"][kind]["ops"].__setitem__(f"{kind}.selftestOp", {"op": 1 + max(v["op"] for v in ops.values()), "ego": "owner", "fold": "sequenced", "args": {}})),
        "a retirement on an unused id": m(lambda t: t["kinds"][kind].__setitem__("retired", ret({str(unused): f"{kind}.selftestGone"}))),
        "a new facet": m(lambda t: t["facets"].__setitem__("selftestFacet", {"on": [kind], "ops": {"base.selftestOp": {"op": 1 + max(o["op"] for f in tree["facets"].values() for o in f.get("ops", {}).values()), "ego": "owner", "fold": "sequenced", "args": {}}}})),
        "a facet on one more kind": m(lambda t: t["facets"][facet]["on"].append("selftestKind")),
    }
    bad = [what for what, t in mutants.items() if not breaks(tree, t)]
    wrong = [what for what, t in allowed.items() if breaks(tree, t)]
    # Retirement against a release that carried one: it stays, with its name; a dropped op is retired.
    base = m(lambda t: t["kinds"][kind].__setitem__("retired", ret({str(unused): f"{kind}.selftestGone"})))
    retiring = {
        "a released retirement removed": (base, m(lambda t: None), f"retired op id {unused} ({kind}.selftestGone) removed"),
        "a released retirement renamed": (base, m(lambda t: t["kinds"][kind].__setitem__("retired", ret({str(unused): f"{kind}.other"}))), "became"),
        "a released op dropped, not retired": (tree, mutants["an op removed"], f"{op} dropped without retiring op id {ops[op]['op']}"),
    }
    for what, (b, t, says) in retiring.items():
        hit = any(says in f for f in breaks(b, t))
        print(f"  {'R' if hit else 'MISSED'}  {what}")
        if not hit:
            bad.append(what)
    dropped = breaks(tree, m(lambda t: (t["kinds"][kind]["ops"].pop(op), t["kinds"][kind].__setitem__("retired", ret({str(ops[op]["op"]): op})))))
    ok = dropped == [f"{kind}: {op} removed"]
    print(f"  {'G' if ok else 'WRONG'}  a released op dropped and retired: still the one break, removed")
    if not ok:
        wrong.append("a dropped op, retired")
    # The waiver: a named break passes under a real W- row; an unnamed one does not; a named
    # break that no longer occurs fails; a row not in status.md waives nothing.
    two = breaks(tree, mutants["an op's fold changed"]) + breaks(tree, mutants["a new required arg"])
    row = next((l.split("|")[1].strip() for l in STATUS.read_text().splitlines() if l.startswith("| W-")), "W-0")
    shared = [(k1, o) for k1, v1 in tree["kinds"].items() for o in v1.get("ops", {})
              for k2, v2 in tree["kinds"].items() if k2 > k1 and o in v2.get("ops", {})]
    if shared:
        k1, o = shared[0]
        k2 = next(k for k, v in tree["kinds"].items() if k > k1 and o in v.get("ops", {}))
        both = m(lambda t: (t["kinds"][k1]["ops"].pop(o), t["kinds"][k2]["ops"].pop(o)))
        found = breaks(tree, both)
        want = {f"{k}: {o} {w}" for k in (k1, k2) for w in ("removed", f"dropped without retiring op id {tree['kinds'][k]['ops'][o]['op']}")}
        print(f"  {'G' if sorted(found) == sorted(want) else 'WRONG'}  {o} under {k1} and {k2}: each break names its kind")
        if sorted(found) != sorted(want):
            wrong.append("a shared op name")
    cases = {
        "a named break, a real row: waived": judge(two[:1], {"waiver": row, "departures": two[:1]}) == ([], two[:1]),
        "an unnamed break beside a waiver: red": judge(two, {"waiver": row, "departures": two[:1]})[0] == two[1:],
        "a named break gone: red": bool(judge([], {"waiver": row, "departures": two[:1]})[0]),
        "a waiver with no row: red": bool(judge(two[:1], {"waiver": "W-99999", "departures": two[:1]})[0]),
    }
    for what, ok in cases.items():
        print(f"  {'G' if ok else 'WRONG'}  {what}")
    wrong += [w for w, ok in cases.items() if not ok]
    for what in mutants:
        print(f"  {'R' if what not in bad else 'MISSED'}  {what}")
    for what in allowed:
        print(f"  {'G' if what not in wrong else 'REFUSED'}  {what} (allowed)")
    if bad or wrong:
        print(f"✗ icd-released.py --selftest: missed {bad}, refused {wrong}", file=sys.stderr)
        return 1
    print(f"icd-released.py --selftest: {len(mutants) + len(retiring)} breaks refused, {len(allowed)} additions allowed, the waiver's {len(cases)} cases ({kind}, {op}, {arg}; facet {facet}, {fop})")
    return 0


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    ref = sys.argv[1] if len(sys.argv) > 1 else released_ref()
    found = breaks(released(ref), json.loads(ICD.read_text()))
    waivers = json.loads(DEPARTURES.read_text()) if DEPARTURES.exists() else None
    fail, waived = judge(found, waivers)
    for f in waived:
        print(f"icd: waived by {waivers['waiver']}: {f} (released at {ref})")
    if fail:
        for f in fail:
            print(f"✗ icd: {f} (released at {ref})", file=sys.stderr)
        print(f"✗ icd: the tree's ICD breaks {len(fail)} released declaration(s) at {ref}", file=sys.stderr)
        return 1
    print(f"icd: every declaration released at {ref} kept" + (f", {len(waived)} departure(s) waived" if waived else "; only additions"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
