#!/usr/bin/env python3
"""Emit the web's op bindings FROM THE ICD. Never by hand.

    gen-web-ops.py            write app/web/shared/wallflowers-ops.js
    gen-web-ops.py --check    exit 1 if the emitted file is stale

WHY THIS EXISTS.  The audit (core/docs/icd-audit.html) found four
hand-maintained layers between the model and a screen, no two of which agreed:
eleven ObjectKinds against eleven different ICD prefixes against nine
catalogue kinds against nine author doors, 233 FFI functions over 95 ops, and
a `status` field wrong forty-five times.  Every one of those is a
transcription.  The doctrine's rule is that a transcribed number is "drift
with a delay on it" — so the web's bindings are generated, and the generator
is the only thing that reads the ICD.

WHY IT IS SMALL.  The ICD turns out to be machine-complete: 95 messages, all
with a payload, 90 with properties and 5 legitimately nullary, and every one
of the 261 properties is `string` or `integer` — which is exactly the core's
`ArgVal::Text | Int`.  There is no type system to build here, only two types.

WHAT IT DOES NOT DO.  It does not decide whether an op can be authored on a
given object.  Since e0a9bf3 that is the core's one authoring door
(`authoring::build`, which core-wasm serves as `author_build`): it refuses an op
the object's kind does not declare, owner-only from a non-owner, a commutative
op on an object with no gen floor, and anything its probe says would do
nothing, per object, in its own words.  So this table marks only what is
refused EVERYWHERE: the membership ops, which a `memberJoined` with no MLS
operation behind it would forge, and which are recorded by the MLS doors.
Which KINDS declare an op is the core's to say too (`ops_catalogue`), and the
keyholder's `ops.capabilities` reads it there rather than from here.

WHAT IT USED TO DO, and why it stopped (22 Sep 2026).  It refused ops whose
prefix had no `<kind>_author` door in node.rs, and every commutative op behind
a watermark gate.  Both were true of the web until the web had a door; with
one, both became second statements of rules the core now enforces per object,
and a second statement is a copy that drifts.
"""
import json
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
ICD = HERE / "delta-graph.icd.json"
# Derived from this file's own location, never written down: the restructure
# broke every literal path in the tree at once.
OUT = HERE.parents[1] / "app" / "web" / "shared" / "wallflowers-ops.js"


# The core's door refuses these on every kind (`authoring::Refusal::ByTheMlsDoors`):
# membership is recorded by the MLS doors, alongside the commit that makes it
# true, never by an authored delta.
MEMBERSHIP_REFUSED = ("base.memberJoined", "base.memberLeft", "base.ownerHandover")


def camel(s: str) -> str:
    return re.sub(r"[._-]([a-z])", lambda m: m.group(1).upper(), s)


def js_string(s: str) -> str:
    return json.dumps(s, ensure_ascii=False)


def every_op(icd: dict) -> dict:
    """Every op the model declares: a kind's own, plus the facet ops it shares.

    There is no `components.messages` any more — an op is declared where it
    belongs, and a facet is declared once beside the kinds that carry it. Reading
    both into one map is the whole of the difference for this generator.
    """
    out = {}
    for kind in icd["kinds"].values():
        out.update(kind["ops"])
    for facet in icd["facets"].values():
        out.update(facet["ops"])
    return out


def authority(name: str, ego: str) -> str:
    """The ego as the core names it: owner, anyMember, or a principal form the ICD's
    `principals` defines (`role:<name>`, `a|b`), verbatim. Anything else stops the build."""
    legacy = {"owner": "owner", "member": "anyMember"}
    if ego in legacy:
        return legacy[ego]
    parts = ego.split("|")
    if all(p in legacy or (p.startswith("role:") and len(p) > 5) for p in parts):
        return ego
    sys.exit(f"{name}: ego {ego!r} is no principal the ICD defines (owner, member, role:<name>, a|b)")


def build() -> str:
    icd = json.loads(ICD.read_text())
    msgs = every_op(icd)

    ops = []
    for name in sorted(msgs):
        m = msgs[name]
        prefix = name.split(".", 1)[0]
        # The op IS its own declaration now: `op`, `ego` and `fold` are fields on
        # it rather than an `x-delta` block, and `args` replaces a JSON-Schema
        # payload whose `required` list lived somewhere else again.
        delta = {"opId": m["op"], "authority": authority(name, m["ego"]), "fold": m["fold"]}
        props = m.get("args") or {}
        required = {a for a, spec in props.items() if spec.get("required")}

        fields = []
        for pname in sorted(props):
            # A commutative op's `gen` is the author's own: authoring::build stamps it over
            # whatever is sent, so no caller writes it.
            if pname == "gen" and m["fold"] == "commutative":
                continue
            t = (props[pname] or {}).get("type")
            if t not in ("string", "integer"):
                sys.exit(
                    f"{name}.{pname}: type {t!r}. The core's ArgVal is Text|Int only — "
                    f"a third type needs a decision in the core before it can be generated."
                )
            fields.append({"name": pname, "type": t, "required": pname in required})

        # Refused everywhere, so refused here. Everything else is the core's to
        # decide, per object, when it is authored (authoring::build).
        if name in MEMBERSHIP_REFUSED:
            reach, why = False, "recorded by the MLS doors, never authored as a delta"
        else:
            reach, why = True, None

        ops.append({
            "name": name,
            "fn": camel(name.replace(".", "_")),
            "prefix": prefix,
            "opId": delta.get("opId"),
            "authority": delta.get("authority"),
            "fold": delta.get("fold"),
            "fields": fields,
            "reachable": reach,
            "why": why,
        })

    reachable = [o for o in ops if o["reachable"]]
    blocked = [o for o in ops if not o["reachable"]]

    L = []
    w = L.append
    w("/* GENERATED FROM core/coordination/delta-graph.icd.json — DO NOT EDIT.")
    w(" *")
    w(" *   regenerate:  core/coordination/gen-web-ops.py")
    w(" *   check:       core/coordination/gen-web-ops.py --check")
    w(" *")
    w(" * Every op the model defines, as a function that builds its args and hands them")
    w(" * to the core. Nothing here is transcribed: the names, the op ids, the")
    w(" * authority, the commutativity and the argument types all come out of the ICD,")
    w(" * so an op added there appears here on the next build and an op removed breaks")
    w(" * it. That is the whole reason this file is generated — the audit found four")
    w(" * hand-maintained layers between the model and a screen and no two agreed.")
    w(" *")
    w(f" * {len(ops)} ops · {len(reachable)} reachable · {len(blocked)} not, each with its reason.")
    w(" *")
    w(" * AN UNREACHABLE OP IS STILL LISTED. It has no author function, and asking for")
    w(" * one throws with the reason. A client that cannot see what it cannot do will")
    w(" * show a member a button that fails.")
    w(" */")
    w("(function (root, factory) {")
    w("  if (typeof module === 'object' && module.exports) module.exports = factory();")
    w("  else root.WallflowersOps = factory();")
    w("}(typeof self !== 'undefined' ? self : this, function () {")
    w("  'use strict';")
    w("")
    w("  /* The catalogue, as the ICD has it. */")
    w("  var OPS = {")
    for o in ops:
        f = ", ".join(
            "{name:%s,type:%s,required:%s}" % (js_string(x["name"]), js_string(x["type"]),
                                               "true" if x["required"] else "false")
            for x in o["fields"]
        )
        w("    %s: {name:%s, prefix:%s, opId:%s, authority:%s, fold:%s, reachable:%s, why:%s, fields:[%s]},"
          % (js_string(o["name"]), js_string(o["name"]), js_string(o["prefix"]), o["opId"],
             js_string(o["authority"]), js_string(o["fold"]),
             "true" if o["reachable"] else "false",
             js_string(o["why"]) if o["why"] else "null", f))
    w("  };")
    w("")
    w("  /* Validate against the ICD's own schema before the core is troubled. The")
    w("     core validates authoritatively — this is only so a caller finds out in")
    w("     the same tick, with the field named. */")
    w("  function check(spec, args) {")
    w("    args = args || {};")
    w("    var out = {};")
    w("    for (var i = 0; i < spec.fields.length; i++) {")
    w("      var f = spec.fields[i], v = args[f.name];")
    w("      if (v === undefined || v === null) {")
    w("        if (f.required) throw new Error(spec.name + ': ' + f.name + ' is required');")
    w("        continue;")
    w("      }")
    w("      if (f.type === 'integer') {")
    w("        if (typeof v !== 'number' || !Number.isInteger(v)) {")
    w("          throw new Error(spec.name + ': ' + f.name + ' must be an integer (the core\\'s ArgVal has no float)');")
    w("        }")
    w("      } else if (typeof v !== 'string') {")
    w("        throw new Error(spec.name + ': ' + f.name + ' must be a string');")
    w("      }")
    w("      out[f.name] = v;")
    w("    }")
    w("    for (var k in args) {")
    w("      if (Object.prototype.hasOwnProperty.call(args, k) && !(k in out)) {")
    w("        throw new Error(spec.name + ': ' + k + ' is not an argument of this op ' +")
    w("          '(the ICD sets additionalProperties:false)');")
    w("      }")
    w("    }")
    w("    return out;")
    w("  }")
    w("")
    w("  /* One draft: {op, opId, args}. The core resolves the object's kind and")
    w("     picks the door; this never guesses which one. */")
    w("  function draft(name, args) {")
    w("    var spec = OPS[name];")
    w("    if (!spec) throw new Error('no op ' + name + ' in the ICD');")
    w("    if (!spec.reachable) throw new Error(name + ' cannot be authored: ' + spec.why);")
    w("    return { op: spec.name, opId: spec.opId, args: check(spec, args) };")
    w("  }")
    w("")
    w("  var api = { OPS: OPS, draft: draft, check: check,")
    w("              names: Object.keys(OPS),")
    w("              reachable: Object.keys(OPS).filter(function (k) { return OPS[k].reachable; }),")
    w("              blocked: Object.keys(OPS).filter(function (k) { return !OPS[k].reachable; }) };")
    w("")
    w("  /* One function per op, named for it. Unreachable ops get a function that")
    w("     throws the reason rather than no function at all — a missing name reads")
    w("     as a typo; a refusal reads as the truth. */")
    for o in ops:
        w("  api.%s = function (a) { return draft(%s, a); };" % (o["fn"], js_string(o["name"])))
    w("")
    w("  return api;")
    w("}));")
    return "\n".join(L) + "\n"


def main() -> int:
    text = build()
    if "--check" in sys.argv:
        if not OUT.exists():
            print(f"{OUT} does not exist — run gen-web-ops.py", file=sys.stderr)
            return 1
        if OUT.read_text() != text:
            print(f"{OUT} is stale — run gen-web-ops.py", file=sys.stderr)
            return 1
        print(f"{OUT.name} matches the ICD")
        return 0
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(text)
    print(f"wrote {OUT.relative_to(HERE.parents[1])} — {len(every_op(json.loads(ICD.read_text())))} ops")
    return 0


if __name__ == "__main__":
    sys.exit(main())
