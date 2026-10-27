#!/usr/bin/env python3
"""Which ICD ops the WebApp sends. Read from the ICD, never by hand.

    icd-coverage.py           a table, by kind
    icd-coverage.py --json    the same rows, for tooling

THE LEVELS.
    mls      recorded by the MLS doors, never authored as a delta
    none     no WebApp source names it
    ui       a WebApp source names it
    ui+e2e   and an e2e journey names it too

WHY A NAME IS NEARLY A SENDER.  The WebApp authors through the Door's generic
routes (/v2/apply, /v2/batch, /v2/mint), each taking the op by its ICD name.  A
quoted name in its source is how it sends one, and also how it reads one: the
face editor's FEEDS name the ops a tile is drawn from.  `ui` is a lead, not proof.

UNDECLARED.  A quoted name with an op's prefix that the ICD does not declare is
listed after the table: a retired op, or one that was never added.

WHAT IT CANNOT SEE.  An op the core writes inside a larger action, by its OP_*
constant (`pair_accept` writes `contact.setLink`), reads `none` here though a
control may reach it.  Telling those from the truly unreachable takes a trace.

Core folding every op is not a column: `icd.rs` holds that, under check-icd.
"""
import importlib.util
import json
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
WEB = ROOT / "app" / "web"
E2E = ROOT / "app" / "e2e"

# The op list and the MLS doors' ops are gen-web-ops.py's; read there, not copied.
_spec = importlib.util.spec_from_file_location("gen_web_ops", HERE / "gen-web-ops.py")
gen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen)

QUOTED = re.compile(r"""(['"])([a-z]+\.[A-Za-z]+)\1""")


def homes(icd: dict) -> dict:
    """op -> (where it is declared, the kinds that carry it)."""
    out = {}
    for name, kind in icd["kinds"].items():
        for op in kind["ops"]:
            home, kinds = out.get(op, (name, []))
            out[op] = (home, kinds + [name])
    for name, facet in icd["facets"].items():
        for op in facet["ops"]:
            out[op] = ("facet " + name, list(facet["on"]))
    return out


def webapp_sources():
    for p in sorted([*(WEB / "webapp").rglob("*.js"), *(WEB / "shared").glob("*.js")]):
        if "vendor" in p.parts or p.name.endswith(".test.mjs") or p == gen.OUT:
            continue
        yield p


def named(paths, prefixes) -> dict:
    """name -> every file:line that quotes it, for names with an op's prefix."""
    out = {}
    for p in paths:
        for i, line in enumerate(p.read_text(errors="replace").splitlines(), 1):
            for m in QUOTED.finditer(line):
                if m.group(2).split(".")[0] in prefixes:
                    out.setdefault(m.group(2), []).append(f"{p.relative_to(ROOT)}:{i}")
    return out


def retired(icd: dict) -> set:
    return {n for k in icd["kinds"].values() for n in (k.get("retired") or {}).values()} | \
           {n for f in icd["facets"].values() for n in (f.get("retired") or {}).values()}


def rows() -> tuple:
    icd = json.loads(gen.ICD.read_text())
    ops = gen.every_op(icd)
    where = homes(icd)
    prefixes = {op.split(".")[0] for op in ops}
    ui = named(webapp_sources(), prefixes)
    e2e = named(sorted(E2E.glob("*.py")), prefixes)
    out = []
    for op in ops:
        if op in gen.MEMBERSHIP_REFUSED:
            level = "mls"
        elif op not in ui:
            level = "none"
        else:
            level = "ui+e2e" if op in e2e else "ui"
        home, kinds = where[op]
        out.append({"op": op, "home": home, "kinds": kinds, "level": level,
                    "ui": ui.get(op, []), "e2e": e2e.get(op, [])})
    gone = retired(icd)
    stray = [{"name": n, "retired": n in gone, "ui": at} for n, at in sorted(ui.items()) if n not in ops]
    return out, stray


def table(rs: list, stray: list) -> str:
    levels = {}
    for r in rs:
        levels[r["level"]] = levels.get(r["level"], 0) + 1
    L = [f"{len(rs)} ops · " + " · ".join(f"{k} {v}" for k, v in sorted(levels.items()))]
    width = max(len(r["op"]) for r in rs)
    by = {}
    for r in rs:
        by.setdefault(r["home"], []).append(r)
    for home, group in by.items():
        L.append("")
        L.append(f"{home} ({len(group)})")
        for r in sorted(group, key=lambda r: r["op"]):
            ev = " · ".join(x[0] + (f" (+{len(x) - 1})" if len(x) > 1 else "") for x in (r["ui"], r["e2e"]) if x)
            L.append(f"  {r['op']:<{width}}  {r['level']:<6}  {ev}".rstrip())
    if stray:
        L.append("")
        L.append(f"undeclared: named in the WebApp, not an ICD op ({len(stray)})")
        for x in stray:
            L.append(f"  {x['name']:<{width}}  {'retired' if x['retired'] else '':<7} {x['ui'][0]}".rstrip()
                     + (f" (+{len(x['ui']) - 1})" if len(x["ui"]) > 1 else ""))
    return "\n".join(L)


def main() -> int:
    rs, stray = rows()
    print(json.dumps({"ops": rs, "undeclared": stray}, indent=1) if "--json" in sys.argv else table(rs, stray))
    return 0


if __name__ == "__main__":
    sys.exit(main())
