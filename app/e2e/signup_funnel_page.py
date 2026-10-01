"""signup_funnel.py's page, core/docs/launch/pdr/signup-funnel.html, self-contained: UX-B's
egg-journeys cards and edges, the paths, where people stop, the defects, the removals ranked."""
from __future__ import annotations

import html
import json
import statistics

E = html.escape
# (id, title, where, row, column, kind): page, step (inside a page), transit (hands on by itself), os (the device's).
NODES = [("home", "wallflowers.io", "/ · entrance", 0, 1, "page"), ("site", "wallflowers.io/site", "/site · every back-link", 0, 2, "page"),
         ("signup·site", "Register your site", "www /signup · 1 of 3", 1, 1, "step"), ("www·signin", "www /signin", "hands on to the Door", 1, 3, "transit"),
         ("signup·face", "Your public face", "www /signup · 2 of 3", 2, 1, "step"), ("signup·account", "Your account", "www /signup · 3 of 3", 3, 1, "step"),
         ("window·new", "The Door's window, ?new", "app /signin?new", 4, 1, "page"), ("window·signin", "The Door's window", "app /signin", 4, 3, "page"),
         ("os·passkey", "Passkey sheets", "create, then get (PRF)", 5, 1, "os"),
         ("window·passkey", "Create the passkey", "app /signin · a sheet dismissed", 5, 2, "step"),
         ("window·words", "Recovery words", "app /signin · 24 words", 6, 1, "step"),
         ("webapp·register", "Register", "app /#register= · the draft's sheet", 7, 1, "page"),
         ("webapp·card", "Your card", "app / · the first card", 8, 1, "step"),
         ("webapp·feed", "The Site's feed", "graph drawn, Site registered, card made", 9, 1, "step"),
         ("webapp·home", "Home, no Site", "an account with no site", 9, 2, "step")]
# (from, to, label, style): tap; auto, moves on by itself; back.
EDGES = [("home", "signup·site", "Sign up", "tap"), ("home", "www·signin", "Sign in", "tap"), ("site", "signup·site", "Sign up · the mark", "tap"),
         ("site", "www·signin", "Sign in", "tap"), ("signup·site", "signup·face", "Continue", "tap"),
         ("signup·site", "signup·account", "Skip to account", "tap"), ("signup·face", "signup·account", "Continue", "tap"),
         ("signup·face", "signup·site", "Back", "back"), ("signup·account", "window·new", "Create my account", "tap"),
         ("signup·account", "signup·face", "Back", "back"), ("www·signin", "window·signin", "", "auto"),
         ("window·new", "os·passkey", "Create an account", "tap"), ("os·passkey", "window·words", "both passed", "auto"),
         ("os·passkey", "window·passkey", "dismissed", "auto"), ("window·passkey", "os·passkey", "Create the passkey", "back"),
         ("window·words", "webapp·register", "Continue · a draft", "tap"), ("window·words", "webapp·card", "Continue · no draft", "tap"),
         ("webapp·register", "webapp·card", "Register", "tap"), ("webapp·card", "webapp·feed", "Save", "tap"),
         ("webapp·card", "webapp·home", "Save · no draft", "tap")]
STUBS = {"window·new": "Sign in with a passkey → a sheet with no passkey", "webapp·register": "Cancel → the webapp, no Site",
         "window·signin": "Sign in with a passkey → the webapp"}
PATHS = [("Sign up", ["home", "signup·site", "signup·face", "signup·account", "window·new", "os·passkey", "window·words",
                      "webapp·register", "webapp·card", "webapp·feed"]),
         ("Skip to account", ["home", "signup·site", "signup·account", "window·new", "os·passkey", "window·words", "webapp·card", "webapp·home"]),
         ("Sign in", ["home", "www·signin", "window·signin"])]
SKIP = {"signup·account", "window·new", "window·words", "webapp·card"}   # met by via="skip" on that path
# Production's screens are timed live from this host; the account's and the webapp's on the Stack with
# production's distances (far), or without (stack). A phone's from iOS Safari first, where it ran.
LIVE = {"home", "site", "signup·site", "signup·face", "signup·account", "window·new", "window·signin"}
# MANAGE, 29 Sep: a phone figure from Chrome's emulation is tagged provisional until Safari's replaces it.
PROV = False
TAG = ' <span class="prov">provisional</span>'


def visits(d: dict, node: str, view: str, via: str | None = None) -> list[dict]:
    engines = ("safari", "chrome") if view == "phone" else ("chrome",)
    for leg in (("live",) if node in LIVE else ("far", "stack")):
        for eng in engines:
            vs = [v for v in d["visits"] if v["node"] == node and v["view"] == view and v["leg"] == leg and v.get("engine", "chrome") == eng]
            got = [v for v in vs if v.get("via") == via and (via or not v.get("warm"))] or ([v for v in vs if not v.get("warm")] or vs if via is None else [])
            if got:
                return sorted(got, key=lambda v: v.get("run", 0))
    return []


visit = lambda d, node, view, via=None: (visits(d, node, view, via) or [None])[0]
fastest = lambda v: round(v["held"] + v["leave"] - v["tap"]) if v.get("leave") and v.get("tap") else v["held"]


def med(d, node, view, f, via=None):
    xs = [f(v) for v in visits(d, node, view, via)]
    return round(statistics.median(xs)) if xs else None


held = lambda d, node, view, via=None: med(d, node, view, lambda v: v["held"], via)
fast = lambda d, node, view, via=None: med(d, node, view, fastest, via)


pv = lambda text, v: text + TAG if PROV and (v or {}).get("engine", "chrome") != "safari" else text
num = lambda x: "—" if x is None else f"{x / 1000:.2f} s" if x >= 1000 else f"{round(x)} ms"


def derived(d: dict) -> dict:
    """What no one screen holds: www's hand-off to the Door, and the passkey sheets."""
    out = {}
    for view in ("phone", "desk"):
        a = next((v for v in d["visits"] if v["node"] == "site" and v["view"] == view and v.get("via") == "again"), None)
        w, words = visit(d, "window·signin", view), visit(d, "window·words", view) or {}
        s = (words.get("perf") or {}).get("steps") or {}
        ms = {k: (s.get("signup:" + k) or {}).get("last") for k in ("passkey", "work", "start", "seal", "finish")}
        out[("www·signin", view)] = {"held": round(w["enter"] - a["tap"]) if a and w and a.get("tap") else None, "v": a}
        out[("os·passkey", view)] = {"held": ms["passkey"], **ms, "sheets": words.get("sheets"), "v": words}
    return out


def card(d: dict, der: dict, seen: dict, n: tuple) -> str:
    nid, title, where, row, col, kind = n
    ph, dk = visit(d, nid, "phone"), visit(d, nid, "desk")
    shots = "".join(f'<button class="shot {c}" data-full="{v["view"]}·{E(nid)}" aria-label="{E(title)}, {v["view"]}"><img alt="" src="data:image/jpeg;base64,{v["shot"]}"></button>'
                    if v and v.get("shot") else f'<span class="shot {c} none">{"device" if kind == "os" else "no frame"}</span>'
                    for v, c in ((ph, "ph"), (dk, "dk")))
    two = lambda f, g=str: f"{pv(g(f('phone')), ph)} · {g(f('desk'))}"
    if nid in ("os·passkey", "www·signin"):
        a, b = der[(nid, "phone")], der[(nid, "desk")]
        rows = [("held", f'{pv(num(a["held"]), a["v"])} · {num(b["held"])}')]
        if kind == "os":
            rows += [("sheets", E(a["sheets"] or "—")), ("work · start", f'{num(a["work"])} · {num(a["start"])}'),
                     ("seal · finish", f'{num(a["seal"])} · {num(a["finish"])}'), ("taps", "2 (device)")]
    else:
        v = ph or dk or {}
        rows = [("held", two(lambda w: held(d, nid, w), num)), ("fastest", two(lambda w: fast(d, nid, w), num)),
                ("taps", pv(str(v.get("taps", "—")), ph)), ("fields", f"{len(v['fields'])} shown · {v.get('required', 0)} required" if v.get("fields") else "0")]
        rows += [("typed", f"{v['typed']} chars")] if v.get("typed") else []
        rows += [("scroll", two(lambda w: (visit(d, nid, w) or {}).get("scroll", "—")))]
        if ph and ph.get("way_bottom") and ph["way_bottom"] > ph["vh"]:
            rows.append(("way on", pv(f"{ph['way_bottom']:,} px, fold {ph['vh']}", ph)))
        for k in ("words", "sheets"):
            rows += [(k, E(str(v[k])))] if v.get(k) else []
        if nid == "webapp·card" and ph:
            rows.append(("name", E(ph.get("prefilled") or "empty")))
        if (ph or {}).get("mark_turn"):
            rows.append(("the mark", pv(f"live {num(ph.get('mark_live'))} · turn {num(ph['mark_turn'])}", ph)))
        if nid == "window·new":
            rows += [("on main", "Create an account first and primary · 3d69c93a"),
                     ("held, return=/", two(lambda w: held(d, nid, w, "skip"), num))]
        rows += [(E(k), E(str(x))) for k, x in (v.get("extra") or {}).items()]
    ctl = (ph or dk or {}).get("controls") or []
    fields = (ph or dk or {}).get("fields") or []
    badge = f'<span class="seen">seen {seen[nid]}</span>' if nid in seen else '<span class="dark">dark</span>'
    return (f'<article class="node {kind}{"" if nid in seen else " is-dark"}" id="n-{E(nid)}" style="grid-row:{row + 1};grid-column:{col}">'
            f'<header><h3>{E(title)}</h3>{badge}</header><p class="where">{E(where)}</p><div class="shots">{shots}</div>'
            f'<dl>{"".join(f"<dt>{k}</dt><dd>{x}</dd>" for k, x in rows)}</dl>'
            + (f'<p class="f">{E(" · ".join(fields))}</p>' if fields else "")
            + (f'<details><summary>{(ph or dk)["n_controls"]} other controls</summary><p>{E(" · ".join(ctl))}</p></details>' if ctl else "")
            + (f'<p class="stub">↳ {E(STUBS[nid])}</p>' if nid in STUBS else "") + "</article>")


def path_row(d: dict, der: dict, name: str, nodes: list[str]) -> str:
    tot, taps, typed, fields, req, prov = {"phone": 0, "desk": 0}, 0, 0, 0, 0, False
    for nid in nodes:
        via = "skip" if name == "Skip to account" and nid in SKIP else None
        for view in tot:   # the sheets' time is inside Recovery words' held
            tot[view] += (der[(nid, view)]["held"] or 0) if nid == "www·signin" else 0 if nid == "os·passkey" else fast(d, nid, view, via) or 0
        v = visit(d, nid, "phone") or {}
        prov |= bool(pv("", v))
        taps += 2 if nid == "os·passkey" else 0 if nid == "www·signin" else v.get("taps", 0)
        typed, fields, req = typed + v.get("typed", 0), fields + len(v.get("fields", [])), req + v.get("required", 0)
    t = TAG if prov else ""
    return (f"<tr><th>{E(name)}</th><td>{len(nodes)}</td><td>{taps}{t}</td><td>{req} of {fields}</td><td>{typed}</td>"
            f"<td>{num(tot['phone'])}{t}</td><td>{num(tot['desk'])}</td></tr>")


def ranked(d: dict, der: dict) -> list[tuple]:
    """The intermediate steps by expected effect per cost: (change, removes, saves on a phone, its source
    screen, effect, cost, needs)."""
    ph = lambda n: fast(d, n, "phone")
    face, mark = visit(d, "signup·face", "phone") or {}, next((v for v in d["visits"] if v.get("mark_turn")), {})
    entr, site = held(d, "home", "phone"), held(d, "site", "phone")
    return [
        ("Your account · www 3 of 3", "1 screen · 2 taps · Banner, Photo, About (saved nowhere) · Name (asked again on Your card)",
         f"{num(ph('signup·account'))} + typing", "signup·account", "every www sign-up", "www: signup.html", "134ca736 live: Your card takes the name"),
        ("The entrance on /", "the wait before any way in is live", num(entr - site if entr and site else None), "home",
         "every first visit", "www: site.html, one condition", "Ralph"),
        ("The window's choice, ?new", "1 screen · 2 ways, 1 of them wrong for a new person", "1 tap where the platform asks without its own tap",
         None, "every sign-up; 2 of 3 who reached it on 29 Sep left there", "Door: signin.js · a Door deploy",
         "3d69c93a (main) is half of it · Safari's tap rule measured"),
        ("wallflowers.io → www 301 on Sign up", "1 redirect", "1 round trip", None, "every sign-up from the apex", "www: /signup's link absolute", "—"),
        ("www /signin", "1 document", num(der[("www·signin", "phone")]["held"]), "site", "every sign-in from www", "www: #signin to the Door", "—"),
        ("Register · the draft's sheet", "1 screen · 1 tap", num(ph("webapp·register")), "webapp·register", "every www sign-up with a site",
         "webapp + Door · a Door deploy", "the draft bound to the sign-up it rode: a link alone makes no Site"),
        ("The mark's turn", "the turn before /signup", num(mark.get("mark_turn")), "site", "the mark's taps", "www: site.html", "Ralph"),
        ("The second passkey sheet · get() for PRF", "1 device sheet", "1 Face ID or Touch ID", None,
         "every sign-up, where the platform gives PRF at create()", "Door: signin.js · SEC-37 review", "PRF at create() measured per platform"),
        ("Your public face · www 2 of 3", f"1 screen · 1 tap · {face.get('n_controls', '—')} controls · Continue at {face.get('way_bottom', 0):,} px, fold {face.get('vh', '—')}",
         num(ph("signup·face")), "signup·face", "every www sign-up with a site", "webapp: a Face editor (none; runbook § 3b is by console)",
         "Ralph: the Face sells (26 Sep)"),
        ("Recovery words", "1 screen · 1 tap · 24 words", "reading 24 words", None, "every sign-up", "Door + webapp",
         "a ruling: the words shown once (SEC-6, NC-53)")]


def page(d: dict, evidence: dict | None = None) -> str:
    global PROV
    ev = evidence or {}
    PROV, der, seen = bool(ev.get("phone_provisional")), derived(d), ev.get("seen", {})
    tr = lambda rows: "".join("<tr>" + "".join(f"<{'th' if i == 0 else 'td'}>{E(str(x))}</{'th' if i == 0 else 'td'}>" for i, x in enumerate(r)) + "</tr>" for r in rows)
    rank = "".join(f'<tr><td class="r">{i}</td><th>{E(c)}</th><td>{E(rm)}{TAG if src == "signup·face" and pv("", visit(d, src, "phone")) else ""}</td>'
                   f'<td>{E(sv)}{pv("", visit(d, src, "phone")) if src else ""}</td><td>{E(ef)}</td><td>{E(co)}</td><td>{E(nd)}</td></tr>'
                   for i, (c, rm, sv, src, ef, co, nd) in enumerate(ranked(d, der), 1))
    shots = {f'{v["view"]}·{n[0]}': v["shot"] for n in NODES for v in (visit(d, n[0], "phone"), visit(d, n[0], "desk")) if v and v.get("shot")}
    fill = {"META": "".join(f"<div><dt>{E(k)}</dt><dd>{E(str(v))}</dd></div>" for k, v in ev.get("meta", [])),
            "STRIP": "".join(f"<li>{E(x)}</li>" for x in ev.get("strip", [])), "PATHS": "".join(path_row(d, der, n, p) for n, p in PATHS),
            "EVIDENCE": tr(ev.get("rows", [])), "ENDED": tr(ev.get("ended", [])), "DEFECTS": tr(ev.get("defects", [])), "RANK": rank,
            "CARDS": "".join(card(d, der, seen, n) for n in NODES), "EDGES": json.dumps([dict(zip("abls", e)) for e in EDGES]), "SHOTS": json.dumps(shots)}
    out = TEMPLATE
    for k, v in fill.items():
        out = out.replace(f"__{k}__", v)
    return out


TEMPLATE = """<!doctype html>
<html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>WallFlowers sign-up funnel</title>
<style>
:root { --bg:#f7f6f2; --fg:#1b1c1e; --mute:#6b6d72; --card:#fff; --line:#d9d7d0; --lit:#2f8f5b; --dark:#c0392b; --os:#7a6fb0; --edge:#8b8d93; --chip:#eeece6; --warn:#b26b00; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { --bg:#121315; --fg:#e8e6e1; --mute:#9a9ca2; --card:#1c1d20; --line:#34363b; --lit:#57c08a; --dark:#e5766a; --os:#a79be0; --edge:#6d7077; --chip:#26282c; --warn:#e0a24a; } }
:root[data-theme="dark"] { --bg:#121315; --fg:#e8e6e1; --mute:#9a9ca2; --card:#1c1d20; --line:#34363b; --lit:#57c08a; --dark:#e5766a; --os:#a79be0; --edge:#6d7077; --chip:#26282c; --warn:#e0a24a; }
* { box-sizing:border-box; } body { margin:0; background:var(--bg); color:var(--fg); font:14px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, sans-serif; }
main { max-width:1180px; margin:0 auto; padding:24px 16px 64px; } h1 { font-size:22px; margin:0 0 12px; letter-spacing:-.01em; }
h2 { font-size:15px; margin:32px 0 10px; text-transform:uppercase; letter-spacing:.06em; color:var(--mute); }
dl.meta { display:grid; grid-template-columns:repeat(auto-fill,minmax(210px,1fr)); gap:6px 16px; margin:0; } dl.meta div { background:var(--card); border:1px solid var(--line); border-radius:8px; padding:6px 10px; }
dl.meta dt, thead th { font-size:11px; color:var(--mute); text-transform:uppercase; letter-spacing:.05em; font-weight:600; } dl.meta dd { margin:0; }
ul.strip { list-style:none; display:flex; flex-wrap:wrap; gap:6px; padding:0; margin:0; } ul.strip li { background:var(--chip); border-radius:999px; padding:3px 10px; font-size:12px; }
.tables { display:grid; grid-template-columns:repeat(auto-fit,minmax(340px,1fr)); gap:16px; } .scroll, .chartwrap { overflow-x:auto; }
table { border-collapse:collapse; width:100%; background:var(--card); border:1px solid var(--line); border-radius:8px; font-variant-numeric:tabular-nums; }
th, td { text-align:left; padding:6px 10px; border-bottom:1px solid var(--line); vertical-align:top; font-size:13px; } td.r { font-weight:700; font-size:16px; }
.chartwrap { border:1px solid var(--line); border-radius:12px; } .chart { position:relative; display:grid; grid-template-columns:repeat(3,304px); gap:84px 48px; padding:32px 24px 48px 112px; width:max-content; margin:0 auto; }
svg.edges { position:absolute; inset:0; width:100%; height:100%; pointer-events:none; overflow:visible; } svg.edges path { fill:none; stroke:var(--edge); stroke-width:1.6; }
svg.edges path.auto { stroke-dasharray:6 5; } svg.edges path.back { stroke-dasharray:2 4; } .elabel { position:absolute; z-index:2; transform:translate(-50%,-50%); background:var(--bg); font-size:11.5px; padding:1px 6px; border-radius:6px; border:1px solid var(--line); white-space:nowrap; }
.node { position:relative; z-index:1; background:var(--card); border:1.5px solid var(--lit); border-radius:12px; padding:10px 12px 8px; }
.node.is-dark { border-color:var(--dark); } .node.os { border-style:dashed; border-color:var(--os); } .node.transit { border-style:dashed; }
.node header { display:flex; justify-content:space-between; align-items:baseline; gap:8px; } .node h3 { margin:0; font-size:14px; }
.node .where, .node .f, .node details { margin:0 0 8px; color:var(--mute); font-size:12px; } .node .f, .node details { margin:6px 0 0; } .node details p { margin:4px 0 0; }
.seen, .dark { font-size:11px; border-radius:999px; padding:1px 8px; white-space:nowrap; } .seen { color:var(--lit); border:1px solid; } .dark { color:var(--dark); border:1px solid; }
.shots { display:flex; gap:8px; align-items:flex-end; margin-bottom:8px; } .shot img { display:block; width:100%; height:auto; }
.shot { padding:0; border:1px solid var(--line); border-radius:6px; overflow:hidden; background:var(--chip); cursor:zoom-in; display:block; }
.shot.ph { width:88px; } .shot.dk { width:190px; } .shot.ph.none { height:190px; } .shot.dk.none { height:125px; } .shot.none { cursor:default; display:flex; align-items:center; justify-content:center; color:var(--mute); font-size:11px; }
.node dl { display:grid; grid-template-columns:auto 1fr; gap:1px 10px; margin:0; font-size:12.5px; font-variant-numeric:tabular-nums; } .node dt { color:var(--mute); } .node dd { margin:0; }
.node .stub { margin:6px 0 0; font-size:12px; color:var(--warn); } .prov { font-size:10px; color:var(--warn); border:1px solid currentColor; border-radius:4px; padding:0 4px; margin-left:2px; white-space:nowrap; vertical-align:1px; }
.legend { display:flex; flex-wrap:wrap; gap:14px; font-size:12px; color:var(--mute); margin:8px 0; } dialog { border:0; padding:0; background:transparent; } dialog::backdrop { background:rgba(0,0,0,.72); } dialog img { display:block; max-width:96vw; max-height:94vh; border-radius:8px; }
</style></head>
<body><main><h1>WallFlowers sign-up funnel</h1><dl class="meta">__META__</dl>
<h2>Ways out</h2><ul class="strip">__STRIP__</ul>
<h2>Paths</h2><div class="scroll"><table><thead><tr><th>path</th><th>screens</th><th>taps</th><th>fields req.</th><th>typed</th><th>fastest · phone</th><th>fastest · desk</th></tr></thead><tbody>__PATHS__</tbody></table></div>
<h2>Where people stop</h2><div class="tables"><div class="scroll"><table><thead><tr><th>evidence</th><th>n</th><th>source</th></tr></thead><tbody>__EVIDENCE__</tbody></table></div>
<div class="scroll"><table><thead><tr><th>ended at</th><th>n</th><th>source</th></tr></thead><tbody>__ENDED__</tbody></table></div></div>
<h2>Screens</h2><div class="legend"><span>── tap</span><span>- - moves on by itself</span><span>··· back</span><span>green: seen in a log</span><span>red: dark</span><span>dashed: the device's</span><span>phone · desk</span></div>
<div class="chartwrap"><div class="chart" id="chart"><svg class="edges" id="edges"></svg>__CARDS__</div></div>
<h2>Defects on the path</h2><div class="scroll"><table><thead><tr><th>where</th><th>what</th><th>since</th></tr></thead><tbody>__DEFECTS__</tbody></table></div>
<h2>Removals, ranked</h2><div class="scroll"><table><thead><tr><th>#</th><th>change</th><th>removes</th><th>saves · phone</th><th>effect</th><th>cost</th><th>needs</th></tr></thead><tbody>__RANK__</tbody></table></div>
</main><dialog id="big"><img alt=""></dialog>
<script>
(function () {
  var EDGES = __EDGES__, SHOTS = __SHOTS__, chart = document.getElementById('chart'), svg = document.getElementById('edges');
  function box(id) {
    var e = document.getElementById('n-' + id), c = chart.getBoundingClientRect(), r = e.getBoundingClientRect();
    return { l: r.left - c.left, t: r.top - c.top, r: r.right - c.left, b: r.bottom - c.top, cx: (r.left + r.right) / 2 - c.left };
  }
  function draw() {
    svg.innerHTML = ''; [].forEach.call(chart.querySelectorAll('.elabel'), function (x) { x.remove(); });
    var fan = 0;
    EDGES.forEach(function (e) {
      var a = box(e.a), b = box(e.b), d, mx, my, x1, x2, y1, y2;
      if (Math.abs(a.t - b.t) < 4) {                       // same row: side to side, back a line lower
        var right = b.cx > a.cx; x1 = right ? a.r : a.l; x2 = right ? b.l : b.r; y1 = a.t + (e.s === 'back' ? 96 : 40);
        d = 'M' + x1 + ',' + y1 + ' C' + (x1 + (right ? 30 : -30)) + ',' + y1 + ' ' + (x2 + (right ? -30 : 30)) + ',' + y1 + ' ' + x2 + ',' + y1;
        mx = (x1 + x2) / 2; my = y1 - 10;
      } else if (b.t < a.t) {                              // up: out to the left
        var out = Math.min(a.l, b.l) - 46 - (fan += 18); y1 = a.t + 60; y2 = b.b - 40;
        d = 'M' + a.l + ',' + y1 + ' C' + out + ',' + y1 + ' ' + out + ',' + y2 + ' ' + b.l + ',' + y2; mx = out + 10; my = (y1 + y2) / 2;
      } else if (Math.abs(a.cx - b.cx) < 4 && b.t - a.b > 200) {   // down past a node: out to the right
        var xr = a.r + 26; y1 = a.b - 30; y2 = b.t + 30;
        d = 'M' + a.r + ',' + y1 + ' C' + xr + ',' + y1 + ' ' + xr + ',' + y2 + ' ' + b.r + ',' + y2; mx = xr + 4; my = (y1 + y2) / 2;
      } else {
        x1 = a.cx + (b.cx - a.cx) * 0.25; x2 = b.cx - (b.cx - a.cx) * 0.15; y1 = a.b; y2 = b.t;
        var bend = Math.max(40, (y2 - y1) / 2);
        d = 'M' + x1 + ',' + y1 + ' C' + x1 + ',' + (y1 + bend) + ' ' + x2 + ',' + (y2 - bend) + ' ' + x2 + ',' + y2; mx = (x1 + x2) / 2; my = (y1 + y2) / 2;
      }
      var p = document.createElementNS('http://www.w3.org/2000/svg', 'path'), l = document.createElement('div');
      p.setAttribute('d', d); p.setAttribute('class', e.s); svg.appendChild(p);
      if (e.l) { l.className = 'elabel'; l.textContent = e.l; l.style.left = mx + 'px'; l.style.top = my + 'px'; chart.appendChild(l); }
    });
  }
  window.addEventListener('load', draw); window.addEventListener('resize', draw);
  var dlg = document.getElementById('big'), img = dlg.querySelector('img');
  chart.addEventListener('click', function (e) {
    var b = e.target.closest('.shot[data-full]'); if (!b) return;
    img.src = 'data:image/jpeg;base64,' + SHOTS[b.getAttribute('data-full')]; dlg.showModal();
  });
  dlg.addEventListener('click', function () { dlg.close(); });
}());
</script></body></html>
"""
