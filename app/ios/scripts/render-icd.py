#!/usr/bin/env python3
"""Render coordination/delta-graph.icd.json to a readable page.

The JSON is the source of truth (it is what pacific-core/src/icd.rs asserts
against). This script only reshapes it, so the page can never disagree with the
document — regenerate rather than hand-edit the HTML.

    python3 scripts/render-icd.py
"""
import json, os, html, collections

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "coordination", "delta-graph.icd.json")
OUT = os.path.join(os.path.dirname(ROOT), "docs", "delta-graph-icd.html")

d = json.load(open(SRC))
msgs = d["components"]["messages"]
e = html.escape


def para(text):
    """The description block is prose with blank-line paragraphs and indented
    literal blocks; keep both readable."""
    out = []
    for block in text.split("\n\n"):
        lines = block.split("\n")
        if all(l.startswith("  ") or not l.strip() for l in lines):
            out.append(f"<pre class='lit'>{e(block)}</pre>")
        else:
            out.append(f"<p>{e(' '.join(l.strip() for l in lines))}</p>")
    return "\n".join(out)


def effect_cells(g):
    """The graph clause as badges — the column a reader actually came for."""
    bits = []
    for n in g["node"]:
        bits.append(f"<span class='b node'>node.{e(n)}</span>")
    for ed in g["edges"]:
        act = ed["action"]
        cls = {"mint": "mint", "invalidate": "inval", "annotate": "annot"}[act]
        arrow = "&rarr;"
        bits.append(
            f"<span class='b {cls}' title='{e(ed.get('note',''))}'>"
            f"{e(ed['from'])} {arrow} <b>{e(ed['rel'])}</b> {arrow} {e(ed['to'])}"
            f"<i>{e(act)}</i></span>"
        )
    if g["embed"]:
        bits.append("<span class='b embed'>re-embed</span>")
    if g.get("episode"):
        bits.append("<span class='b epi'>episode</span>")
    if g.get("exclude"):
        bits.append(f"<span class='b excl'>excluded</span>")
        bits.append(f"<span class='why'>{e(g['exclude'])}</span>")
    return "".join(bits) or "<span class='b none'>—</span>"


rows_html = []
for key, ch in d["channels"].items():
    x = ch["x-object"]
    ent = x["entityType"]
    ent_badge = (
        f"<span class='ent'>{e(ent)} node</span>"
        if ent
        else "<span class='ent no'>no node</span>"
    )
    rows = []
    for name in ch["messages"]:
        m = msgs[name]
        xd = m["x-delta"]
        g = m["x-graph"]
        op_id = xd["opId"]
        idtxt = f"0x{op_id:X}" if op_id >= 0xF000_0000 else str(op_id)
        args = m["payload"].get("properties", {})
        req = set(m["payload"].get("required", []))
        argtxt = (
            " ".join(
                f"<code class='{'req' if a in req else 'opt'}'>{e(a)}</code>" for a in args
            )
            or "<span class='dim'>no args</span>"
        )
        spec = g["status"] == "specified"
        rows.append(
            f"<tr class='{'spec' if spec else ''}'>"
            f"<td class='op'><code>{e(name)}</code>"
            f"{'<span class=todo>specified</span>' if spec else ''}</td>"
            f"<td class='id'>{idtxt}</td>"
            f"<td class='au'>{e(xd['authority'])}<br><i>{e(xd['fold'])}</i></td>"
            f"<td class='ar'>{argtxt}</td>"
            f"<td class='fx'>{effect_cells(g)}</td>"
            f"</tr>"
        )
    rows_html.append(
        f"""
<section id="{e(key)}">
  <h2>{e(key)} <span class="tid">type_id {x['typeId']}</span> {ent_badge}</h2>
  <p class="chsum">{e(ch['summary'])}</p>
  <div class="tw"><table>
    <thead><tr><th>op</th><th>id</th><th>authority</th><th>args</th>
    <th>graph effect</th></tr></thead>
    <tbody>{''.join(rows)}</tbody>
  </table></div>
</section>"""
    )

n_ops = sum(len(c["messages"]) for c in d["channels"].values())
n_embed = sum(1 for m in msgs.values() if m["x-graph"]["embed"])
n_excl = sum(1 for m in msgs.values() if m["x-graph"].get("exclude"))
n_edge = sum(len(m["x-graph"]["edges"]) for m in msgs.values())
n_spec = sum(1 for m in msgs.values() if m["x-graph"]["status"] == "specified")
vocab = d["info"]["x-vocabulary"]
built = set(vocab["implemented"])
vocab_html = "".join(
    f"<span class='b {'mint' if r in built else 'annot'}'>{e(r)}"
    f"{'' if r in built else '<i>specified</i>'}</span>"
    for r in vocab["relations"]
)

page = f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pacific — delta &rarr; graph ICD</title>
<style>
  :root{{
    --bg:#0b0d10; --surface:#14171c; --surface-2:#1b1f26; --line:#272c35;
    --ink:#e8ecf2; --ink-2:#9aa4b2; --ink-3:#6b7482;
    --accent:#6fd3c7; --accent-2:#e0b062; --accent-3:#8f9ee8; --warn:#e0796a;
    --mono:ui-monospace,SFMono-Regular,"SF Mono",Menlo,monospace;
  }}
  *{{box-sizing:border-box}}
  body{{margin:0;background:var(--bg);color:var(--ink);
    font:16px/1.65 ui-sans-serif,-apple-system,"SF Pro Text",Inter,system-ui,sans-serif;
    -webkit-font-smoothing:antialiased}}
  .wrap{{max-width:1180px;margin:0 auto;padding:56px 24px 120px}}
  header{{margin-bottom:40px;border-bottom:1px solid var(--line);padding-bottom:30px}}
  h1{{font-size:34px;line-height:1.15;margin:0 0 10px;letter-spacing:-.02em;font-weight:650}}
  .sub{{color:var(--ink-2);font-size:17px;margin:0;max-width:74ch}}
  .meta{{color:var(--ink-3);font-size:13px;margin-top:18px;font-family:var(--mono)}}
  h2{{font-size:23px;margin:56px 0 4px;letter-spacing:-.01em;font-weight:640;
     display:flex;align-items:center;gap:12px;flex-wrap:wrap}}
  .tid{{font-family:var(--mono);font-size:12px;color:var(--ink-3);font-weight:400}}
  .ent{{font-size:11px;font-weight:600;letter-spacing:.04em;text-transform:uppercase;
    color:#0b0d10;background:var(--accent);padding:3px 9px;border-radius:99px}}
  .ent.no{{background:transparent;color:var(--ink-3);border:1px solid var(--line)}}
  .chsum{{color:var(--ink-2);margin:8px 0 18px;max-width:80ch}}
  p{{margin:12px 0}}  .intro p{{max-width:80ch;color:var(--ink-2)}}
  .intro strong{{color:var(--ink)}}
  pre.lit{{font-family:var(--mono);font-size:12.5px;line-height:1.6;color:var(--ink-2);
    background:var(--surface);border:1px solid var(--line);border-left:3px solid var(--accent-3);
    border-radius:0 8px 8px 0;padding:14px 18px;overflow-x:auto;white-space:pre}}
  code{{font-family:var(--mono);font-size:.86em;background:var(--surface-2);
    padding:2px 6px;border-radius:4px;color:#cfe6e2;border:1px solid var(--line)}}
  code.opt{{color:var(--ink-3);border-style:dashed}}
  .tw{{overflow-x:auto;border:1px solid var(--line);border-radius:12px;background:var(--surface)}}
  table{{border-collapse:collapse;width:100%;font-size:13.5px;min-width:900px}}
  th{{text-align:left;font-size:11px;letter-spacing:.08em;text-transform:uppercase;
    color:var(--ink-3);font-weight:600;padding:12px 14px;border-bottom:1px solid var(--line);
    background:var(--surface-2)}}
  td{{padding:12px 14px;border-bottom:1px solid rgba(39,44,53,.6);vertical-align:top}}
  tr:last-child td{{border-bottom:none}}
  tr.spec{{background:rgba(224,176,98,.035)}}
  td.op code{{background:none;border:none;padding:0;color:var(--ink);font-size:13px}}
  td.id{{font-family:var(--mono);color:var(--ink-3);white-space:nowrap}}
  td.au{{color:var(--ink-2);white-space:nowrap;font-size:12.5px}}
  td.au i{{color:var(--ink-3);font-style:normal}}
  td.ar{{max-width:230px;line-height:2.1}}
  td.fx{{line-height:2.2;min-width:330px}}
  .dim{{color:var(--ink-3)}}
  .todo{{font-size:10px;letter-spacing:.06em;text-transform:uppercase;color:var(--accent-2);
    border:1px solid rgba(224,176,98,.4);border-radius:99px;padding:1px 7px;margin-left:8px;
    vertical-align:middle}}
  .b{{display:inline-block;font-family:var(--mono);font-size:11.5px;padding:3px 8px;
    border-radius:6px;margin:0 5px 3px 0;border:1px solid var(--line);white-space:nowrap}}
  .b i{{font-style:normal;opacity:.55;margin-left:7px;font-size:10px;text-transform:uppercase;
    letter-spacing:.05em}}
  .b b{{font-weight:650}}
  .node{{color:var(--accent-3);border-color:rgba(143,158,232,.35);background:rgba(143,158,232,.08)}}
  .mint{{color:var(--accent);border-color:rgba(111,211,199,.35);background:rgba(111,211,199,.08)}}
  .inval{{color:var(--warn);border-color:rgba(224,121,106,.35);background:rgba(224,121,106,.08)}}
  .annot{{color:var(--ink-2);border-color:var(--line);background:var(--surface-2)}}
  .embed{{color:var(--accent-2);border-color:rgba(224,176,98,.4);background:rgba(224,176,98,.1)}}
  .epi{{color:var(--accent-3);border-color:rgba(143,158,232,.3);background:transparent}}
  .excl{{color:var(--ink-3);border-style:dashed}}
  .none{{color:var(--ink-3);border:none;background:none}}
  .why{{display:block;color:var(--ink-3);font-size:12px;line-height:1.5;max-width:52ch;
    margin-top:2px;font-style:italic}}
  .strip{{display:flex;gap:10px;flex-wrap:wrap;margin:26px 0 8px}}
  .stat{{background:var(--surface);border:1px solid var(--line);border-radius:10px;
    padding:12px 16px;min-width:104px;flex:1}}
  .stat b{{display:block;font-size:24px;font-weight:650;letter-spacing:-.02em}}
  .stat span{{color:var(--ink-3);font-size:11.5px;letter-spacing:.04em;text-transform:uppercase}}
  .legend{{background:var(--surface);border:1px solid var(--line);border-radius:12px;
    padding:18px 20px;margin:26px 0}}
  .legend h4{{margin:0 0 12px;font-size:12px;letter-spacing:.08em;text-transform:uppercase;
    color:var(--ink-3);font-weight:600}}
  .legend div+h4{{margin-top:18px}}
  .legend p{{margin:8px 0 0;color:var(--ink-3);font-size:13px}}
  nav{{display:flex;gap:8px;flex-wrap:wrap;margin:22px 0 0}}
  nav a{{font-family:var(--mono);font-size:12.5px;color:var(--ink-2);text-decoration:none;
    border:1px solid var(--line);border-radius:99px;padding:5px 13px;background:var(--surface)}}
  nav a:hover{{color:var(--accent);border-color:rgba(111,211,199,.4)}}
  footer{{margin-top:72px;padding-top:26px;border-top:1px solid var(--line);
    color:var(--ink-3);font-size:13px;font-family:var(--mono)}}
  @media (max-width:640px){{ .wrap{{padding:34px 16px 80px}} h1{{font-size:27px}} }}
</style>
</head>
<body><div class="wrap">
<header>
  <h1>Delta &rarr; graph ICD</h1>
  <p class="sub">What every op on every GroupObject does to the lodedb-graph read
  model: which node fields it writes, which edges it mints or invalidates, and
  whether it bursts the embedding cache.</p>
  <p class="meta">AsyncAPI {d['asyncapi']} &nbsp;·&nbsp; v{d['info']['version']}
  &nbsp;·&nbsp; source: coordination/delta-graph.icd.json
  &nbsp;·&nbsp; conformance: {d['info']['x-conformance']['test']}</p>
  <nav>{''.join(f'<a href="#{e(k)}">{e(k)}</a>' for k in d['channels'])}</nav>
</header>

<div class="strip">
  <div class="stat"><b>{n_ops}</b><span>ops</span></div>
  <div class="stat"><b>{len(d['channels'])}</b><span>object kinds</span></div>
  <div class="stat"><b>{n_edge}</b><span>edge clauses</span></div>
  <div class="stat"><b>{n_embed}</b><span>re-embed</span></div>
  <div class="stat"><b>{n_excl}</b><span>excluded</span></div>
  <div class="stat"><b>{n_spec}</b><span>specified, not built</span></div>
</div>

<div class="intro">{para(d['info']['description'])}</div>

<div class="legend">
  <h4>Reading the graph-effect column</h4>
  <div>
    <span class="b node">node.field</span>
    <span class="b mint">from &rarr; <b>rel</b> &rarr; to<i>mint</i></span>
    <span class="b inval">from &rarr; <b>rel</b> &rarr; to<i>invalidate</i></span>
    <span class="b annot">from &rarr; <b>rel</b> &rarr; to<i>annotate</i></span>
    <span class="b embed">re-embed</span>
    <span class="b epi">episode</span>
    <span class="b excl">excluded</span>
  </div>
  <p><b>invalidate</b> sets <code>invalid_at</code>; it never deletes. <b>annotate</b>
  writes a property onto an edge the roster already justified. <b>re-embed</b> fires
  when <code>fold_digest()</code> moves, so a sync landing forty deltas re-embeds once.
  A row tinted amber is <b>specified</b> — the document is ahead of the projector, and
  that is the work list.</p>
  <h4>Relation vocabulary</h4>
  <div>{vocab_html}</div>
  <p>{e(vocab['note'])}</p>
</div>

{''.join(rows_html)}

<footer>
  Generated from coordination/delta-graph.icd.json by scripts/render-icd.py —
  regenerate, do not hand-edit.
</footer>
</div></body></html>
"""

os.makedirs(os.path.dirname(OUT), exist_ok=True)
open(OUT, "w").write(page)
print(f"wrote {OUT}  ({len(page):,} bytes, {n_ops} ops across {len(d['channels'])} kinds)")
