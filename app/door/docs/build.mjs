#!/usr/bin/env node
/* THE COMMUNITY API'S DOCUMENTATION, built from what the Door serves, for docs.wallflowers.io.

     node app/door/docs/build.mjs           writes app/door/docs/dist/
     node app/door/docs/build.mjs --check   fails if the Door serves a route these pages do not
                                            account for, a page names a route it does not serve,
                                            or dist/ is not what the sources build

   Nothing that the Door or the model states is written here twice. The routes are the routers'
   (app/door/routes.json, held to them by the Door's tests); the ops are the model's (the ICD);
   the sign-in script's integrity hash is element.js's own sha384; the latencies are measured
   (latency/, one file a run). What is written here is what a person needs besides: what a route
   is for, what it answers, how to use it.

   door-release names the Door commit production serves; the routes, the sign-in script and the
   Door's configuration are read from it (git show), so the pages say what production does even
   while the tree is ahead of it. Without the file, the tree's own. */
import { readFileSync, writeFileSync, mkdirSync, readdirSync, existsSync, rmSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { ROUTES, AUDIENCES } from './routes.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const PRODUCT = path.resolve(HERE, '../../..');
const DIST = path.join(HERE, 'dist');
const read = (p) => readFileSync(path.join(PRODUCT, p), 'utf8');
const exists = (p) => existsSync(path.join(PRODUCT, p));

/* THE DOOR RELEASE THESE PAGES DESCRIBE (door-release: one commit, the Door production serves).
   Its sign-in script, routes and configuration come from that commit, not the tree, which can be
   ahead of production: on 30 Sep the tree's element.js was the next release's, and the integrity
   these pages published blocked the sign-in script for anyone who copied it. The model stays the
   tree's (the Arc serves it). Without the file, everything is the tree's. */
const RELEASE_FILE = path.join(HERE, 'door-release');
export const RELEASE = existsSync(RELEASE_FILE) ? readFileSync(RELEASE_FILE, 'utf8').trim().split(/\s+/)[0] : '';
/* A file the release does not have (routes.json came after 322da7f7) is the tree's: RELEASED
   records which came from where, for the pages' own line. */
export const RELEASED = {};
export const released = (p) => {
  if (RELEASE) {
    try {
      const b = execFileSync('git', ['-C', PRODUCT, 'show', `${RELEASE}:${p}`], { maxBuffer: 64 << 20, stdio: ['ignore', 'pipe', 'ignore'] });
      RELEASED[p] = true;
      return b;
    } catch (e) { /* not in the release: the tree's */ }
  }
  RELEASED[p] = false;
  return readFileSync(path.join(PRODUCT, p));
};
const readDoor = (p) => released(p).toString('utf8');

export const SITE = 'https://docs.wallflowers.io';
export const DOOR = 'https://app.wallflowers.io';
export const ARC = 'https://arc.wallflowers.io';

/* ── the sources ─────────────────────────────────────────────────────────── */

/* The Door's routes: app/door/routes.json, which the Door's tests hold to the three
   routers (the Door's, its per-person process's, the Arc's gateway). One row per method and path:
   where a path is on two routers, the row for sites wins. */
export function inventory() {
  const rows = JSON.parse(readDoor('app/door/routes.json'));
  const by = new Map();
  for (const r of rows) {
    const k = r.method + ' ' + r.path, had = by.get(k);
    if (!had || (r.audience === 'site' && had.audience !== 'site')) by.set(k, r);
  }
  return [...by.values()].sort((a, b) => (a.path + a.method).localeCompare(b.path + b.method));
}

/* The limits the code states, from the inventory (the routes' own `limits`): the largest body a
   route reads whole, a batch's steps, a ceremony's proof of work. */
export function limits(inv = inventory()) {
  const all = JSON.parse(readDoor('app/door/routes.json')).map((r) => r.limits || {});
  const max = (k) => Math.max(0, ...all.map((l) => l[k] || 0));
  return { body: max('body_bytes'), steps: max('steps_max'), pow: max('pow_bits') };
}
const mib = (n) => (n >= 1048576 && n % 1048576 === 0 ? n / 1048576 + ' MiB' : n + ' bytes');

/* PREVIEW: a model draft, built into a folder of its own for a person to inspect, never
   into dist/ and never published (--preview <ref> --out <dir>). Every page says so. */
let PREVIEW = null;
export function preview(ref) { PREVIEW = ref ? { ref } : null; }
const ICD_PATH = 'core/coordination/delta-graph.icd.json';
const icdAt = (ref) => execFileSync('git', ['-C', PRODUCT, 'show', `${ref}:${ICD_PATH}`], { maxBuffer: 64 << 20 });

/* The model production's Door serves at /v2/icd: the ICD at door-release (SCM, 1 Oct), whose
   sha256 is the docs' baseline, and which SCM's check holds to a released icd/* pin. */
export function icd() {
  const raw = PREVIEW ? icdAt(PREVIEW.ref) : released(ICD_PATH);
  const d = JSON.parse(raw.toString('utf8'));
  const kinds = [];
  for (const [k, v] of Object.entries(d.kinds || {})) kinds.push({ kind: k, summary: v.summary || '', ops: v.ops || {}, facets: v.facets || [] });
  const facets = [];
  for (const [f, v] of Object.entries(d.facets || {})) facets.push({ facet: f, summary: v.summary || '', on: v.on || [], ops: v.ops || {} });
  return { version: d.version || (d.info && d.info.version) || '', sha256: createHash('sha256').update(raw).digest('hex'), kinds, facets };
}

/* The sign-in script's integrity: /v2/signin.js is element.js, served as it is (main.rs,
   element_js), so its hash is the file's. */
/* A served script's integrity: the sha384 of its bytes, at the Door release. /v2/signin.js is
   element.js, served as it is (main.rs, element_js); /v2/resources.js is RESOURCES_JS, its parts in
   the order main.rs concatenates them (the Resources lane, BUILD 9bcf144a), read from main.rs. */
export function sri(files = ['app/web/door/element.js']) {
  const h = createHash('sha384');
  for (const f of files) h.update(released(f));
  return 'sha384-' + h.digest('base64');
}
export function resourcesParts() {
  const m = /const RESOURCES_JS: &str = concat!\(([\s\S]*?)\);/.exec(readDoor('app/door/src/main.rs'));
  if (!m) return [];
  return [...m[1].matchAll(/include_str!\("([^"]+)"\)/g)].map((x) => path.relative(PRODUCT, path.resolve(PRODUCT, 'app/door/src', x[1])));
}

/* The measurements, one file a run (latency/): warm n/p50/p95 at the top of each row. */
export function latency() {
  const dir = path.join(HERE, 'latency');
  const runs = [];
  if (existsSync(dir)) {
    for (const f of readdirSync(dir).filter((n) => n.endsWith('.json')).sort()) {
      try { runs.push({ file: f, ...JSON.parse(readFileSync(path.join(dir, f), 'utf8')) }); } catch (e) { /* a run being written */ }
    }
  }
  const by = {};
  for (const run of runs) {
    for (const r of run.routes || []) {
      if (!r.n) continue;   // not measured on this link: the run's note says why, and another run has it
      const k = String(r.method).toUpperCase() + ' ' + r.path;
      (by[k] ||= []).push({ where: run.where, link: run.link || r.link || run.file.replace(/\.json$/, ''), from: run.from, door: run.door, at: run.at,
        n: r.n, p50: r.p50_ms, p95: r.p95_ms, cold: r.cold && r.cold.n ? r.cold.p50_ms : null, note: r.note || '' });
    }
  }
  return { runs: runs.map((r) => ({ file: r.file, door: r.door, where: r.where, from: r.from, at: r.at, link: r.link || r.file.replace(/\.json$/, ''), detail: r.link_detail || '' })), by };
}

export function event() {
  const f = path.join(HERE, 'event.json');
  return existsSync(f) ? JSON.parse(readFileSync(f, 'utf8')) : {};
}
export const REPO = event().repository || '';   // the public repository, once named

/* The Door's configuration: each setting's default as main.rs reads it (env("DOOR_…", "n")),
   and over it the value the Door is deployed with, where event.json's `door` names one. The
   pages state these numbers only from here. */
/* THE DEV CLIENT (R3.1 (C)): a client registered with "site": "*" has no fixed Site; the person
   picks one of theirs at sign-in. Read from the registry at the Door release, so its id, ports and
   paths are stated as registered, never by hand. None: the pages say nothing of it. */
export function devClient() {
  let clients;
  try { clients = JSON.parse(readDoor('app/door/clients.stand-in.json')); } catch { return null; }
  const found = Object.entries(clients).find(([, c]) => c && c.site === '*');
  if (!found) return null;
  const [id, c] = found;
  const urls = (c.callbacks || []).map((u) => new URL(u));
  const ports = [...new Set(urls.map((u) => Number(u.port)))].sort((a, b) => a - b);
  const paths = [...new Set(urls.map((u) => u.pathname))];
  const hosts = [...new Set(urls.map((u) => u.hostname))];
  // Each path's own ports, per host: a path need not be registered on every port.
  const portsOf = (host, path) => [...new Set(urls.filter((u) => u.hostname === host && u.pathname === path).map((u) => Number(u.port)))].sort((a, b) => a - b);
  const byPath = paths.map((p) => ({ path: p, ports: portsOf(hosts[0], p) }));
  const sameOnAll = hosts.every((h) => paths.every((p) => portsOf(h, p).join() === portsOf(hosts[0], p).join()));
  return { id, name: c.name || id, ports, paths, hosts, byPath, sameOnAll, portsOf, callbacks: c.callbacks || [] };
}
/* W-102 (Ralph, 1 Oct): wallflowers.io/signup is the only way to make a community, and its account
   step leads with "Create an account". Someone who already has an account must choose "Sign in",
   or they make a second account, which owns no community. Said wherever the docs send people there. */
const SIGNUP_NOTE = 'Already have a WallFlowers account? Press <strong>Create my account</strong>, then, on the next page, choose <strong>Sign in with a passkey</strong>, so the community is made for the account you already have.';

const span_ports = (ps) => {
  const runs = [];
  for (const p of ps) { const r = runs[runs.length - 1]; if (r && p === r[1] + 1) r[1] = p; else runs.push([p, p]); }
  return runs.map(([a, b]) => (a === b ? String(a) : `${a}–${b}`)).join(', ');
};

/* HOW SOON A REGISTRATION COUNTS: the Door reads its registry again when the file changes
   (R3.1 (B), token::RELOAD_EVERY), or only at a restart before it. From the release. */
export function reloadEvery() {
  const m = /RELOAD_EVERY: Duration = Duration::from_secs\((\d+)\)/.exec(readDoor('app/door/src/token.rs'));
  return m ? Number(m[1]) : null;
}

export function door(ev = event()) {
  const out = {};
  for (const [, k, v] of readDoor('app/door/src/main.rs').matchAll(/env\("(DOOR_[A-Z_]+)", "(\d+)"\)/g)) out[k] = Number(v);
  const skew = /const PROOF_SKEW: u64 = (\d+);/.exec(readDoor('app/door/src/token.rs'));
  if (skew) out.PROOF_SKEW = Number(skew[1]);
  for (const [k, v] of Object.entries(ev.door || {})) out[k] = Number(v);
  return out;
}
const span = (s) => (s % 3600 === 0 ? (s === 3600 ? 'one hour' : s / 3600 + ' hours')
  : s % 60 === 0 ? (s === 60 ? '1 minute' : s / 60 + ' minutes') : s + ' seconds');

/* ── the check ───────────────────────────────────────────────────────────── */

/* Every route for sites is documented here, and every route documented here is one the routers
   serve for sites. The rest are listed from the inventory by audience. */
export function problems(inv = inventory()) {
  const site = new Set(inv.filter((r) => r.audience === 'site').map((r) => r.method + ' ' + r.path));
  const told = new Set(ROUTES.map((r) => r.method + ' ' + r.path));
  const out = [];
  for (const k of site) if (!told.has(k)) out.push(`the routers serve ${k} for sites, and no page says what it is`);
  for (const k of told) if (!site.has(k)) out.push(`${k} is documented here, and no router serves it for sites`);
  for (const r of inv) if (r.audience !== 'site' && !AUDIENCES[r.audience]) out.push(`${r.method} ${r.path}: audience "${r.audience}" is not explained`);
  return out;
}

/* ── the pages ───────────────────────────────────────────────────────────── */

const esc = (s) => String(s == null ? '' : s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
export const anchor = (r) => (r.method + '-' + r.path).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

/* The navigation, in reading order. `file` is the page's source in pages/ or a generator. */
/* The ICD's changelog (SCM's, icd-changelog.md beside this file): a page when it is there. */
const CHANGELOG = path.join(HERE, 'icd-changelog.md');
export const NAV = [
  ['Start', [['index', 'Overview'], ['start', 'Start here'], ['quickstart', 'Quickstart'], ['register', 'Register your app'], ['signin', 'Sign in'], ['session', 'The session']]],
  ['Read and write', [['routes', 'Routes'], ['views', 'What you read'], ['model', 'The model: kinds and ops']]],
  ['Recipes', [['rooms', 'Rooms and threads'], ['members', 'Members and roles'], ['decisions', 'Decisions'], ['page', 'The page and its pictures'], ['public', 'Events and resources'], ['resources', 'The Resources editor'], ['trade', 'The listings board'], ['client', 'A client that holds up']]],
  ['Reference', [['errors', 'Errors'], ['limits', 'Limits'], ...(existsSync(CHANGELOG) ? [['changelog', 'Model changelog']] : []), ['agents', 'For AI agents']]]
];

/* Markdown enough for the changelog: headings, paragraphs, lists, tables, fenced code, and
   inline code, bold and links. Everything is escaped first; nothing in it runs. */
export function markdown(md) {
  md = String(md).replace(/<!--[\s\S]*?-->/g, '');   // a source's comments are for its writer, not the page
  const inline = (t) => esc(t).replace(/`([^`]+)`/g, '<code>$1</code>').replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+|[a-z0-9./#_-]+)\)/gi, '<a href="$2">$1</a>');
  const lines = md.replace(/\r\n?/g, '\n').split('\n');
  let html = '', i = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (/^```/.test(l)) { let j = i + 1, code = []; while (j < lines.length && !/^```/.test(lines[j])) code.push(lines[j++]); html += `<pre><code>${esc(code.join('\n'))}</code></pre>`; i = j + 1; continue; }
    const h = /^(#{1,4})\s+(.*)$/.exec(l);
    if (h) { const n = Math.min(h[1].length, 3); html += `<h${n}>${inline(h[2])}</h${n}>`; i++; continue; }
    if (/^\s*\|/.test(l)) {
      const rows = []; while (i < lines.length && /^\s*\|/.test(lines[i])) rows.push(lines[i++]);
      const cells = (r) => r.trim().replace(/^\||\|$/g, '').split('|').map((c) => c.trim());
      const body = rows.filter((r, k) => !(k === 1 && /^[\s|:-]+$/.test(r)));
      html += '<table><thead><tr>' + cells(body[0]).map((c) => `<th>${inline(c)}</th>`).join('') + '</tr></thead><tbody>' +
        body.slice(1).map((r) => '<tr>' + cells(r).map((c) => `<td>${inline(c)}</td>`).join('') + '</tr>').join('') + '</tbody></table>';
      continue;
    }
    if (/^\s*([-*]|\d+\.)\s+/.test(l)) {
      const ordered = /^\s*\d+\./.test(l), items = [];
      while (i < lines.length && /^\s*([-*]|\d+\.)\s+/.test(lines[i])) { let item = lines[i++].replace(/^\s*([-*]|\d+\.)\s+/, ''); while (i < lines.length && /^\s{2,}\S/.test(lines[i]) && !/^\s*([-*]|\d+\.)\s+/.test(lines[i])) item += ' ' + lines[i++].trim(); items.push(item); }
      html += `<${ordered ? 'ol' : 'ul'}>` + items.map((t) => `<li>${inline(t)}</li>`).join('') + `</${ordered ? 'ol' : 'ul'}>`;
      continue;
    }
    if (!l.trim()) { i++; continue; }
    const para = []; while (i < lines.length && lines[i].trim() && !/^(#{1,4}\s|```|\s*\||\s*([-*]|\d+\.)\s+)/.test(lines[i])) para.push(lines[i++].trim());
    html += `<p>${inline(para.join(' '))}</p>`;
  }
  return html;
}
const TITLES = Object.fromEntries(NAV.flatMap(([, items]) => items));

/* A page about ops the model doesn't have yet stays out until it does (the listings board, 2.3.1). */
const NEEDS = { trade: 'group.publishListing', resources: 'GET /v2/resources.js' };
let MODEL_OPS = null;   // the model's ops and the Door's routes, at the release
const shown = (items) => items.filter(([s]) => !NEEDS[s] || !MODEL_OPS || MODEL_OPS.has(NEEDS[s]));
const navGroups = () => NAV.map(([g, items]) => [g, shown(PREVIEW && g === 'Read and write' ? [...items, ['draft', 'Draft: what changes']] : items)]);

function layout(slug, title, body, facts) {
  const nav = navGroups().map(([group, items]) => `<p class="ng">${esc(group)}</p><ul>` + items.map(([s, t]) =>
    `<li><a href="${s === 'index' ? (PREVIEW ? 'index.html' : './') : s + (s === 'agents' ? '.md' : '.html')}"${s === slug ? ' aria-current="page"' : ''}>${esc(t)}</a></li>`).join('') + '</ul>').join('');
  if (PREVIEW) body = `<p style="border:2px solid #b8860b;border-radius:8px;padding:10px 14px;margin:0 0 18px;background:rgba(184,134,11,.08)"><strong>Preview, not published.</strong> The model on these pages is the draft <code>${esc(PREVIEW.version)}</code> (sha256 <code>${esc(PREVIEW.sha256.slice(0, 16))}…</code>, git <code>${esc(PREVIEW.ref)}</code>), not pinned. Production serves <code>${esc(PREVIEW.baseVersion)}</code>; the live docs are ${SITE}.</p>` + body;
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)} · WallFlowers Community API</title>
<meta name="description" content="Build on a WallFlowers community: sign in with a passkey, read its rooms and people, write as the member.">
<link rel="stylesheet" href="docs.css">
<link rel="icon" href="data:,">
<link rel="alternate" type="text/markdown" href="agents.md" title="For AI coding agents">
</head>
<body>
<header class="top"><a class="brand" href="./">WallFlowers <span>Community API</span></a>${REPO ? `<a class="gh" href="${REPO}">Source</a>` : ''}</header>
<div class="wrap">
<nav class="side" aria-label="Pages">${nav}</nav>
<main>
${body}
<footer class="facts">${facts}</footer>
</main>
</div>
</body>
</html>
`;
}

const CSS = `:root{--bg:#FBFAF7;--ink:#16191F;--i2:#4A5160;--i3:#7A8190;--hair:#E3E1DA;--code:#F1EFE8;--accent:#3E5BA9;--ok:#2F7D4F;--warn:#9B5A12}
@media (prefers-color-scheme:dark){:root{--bg:#121418;--ink:#ECEDEF;--i2:#B4B9C3;--i3:#8A909C;--hair:#2A2E36;--code:#1B1E24;--accent:#8FA6E8;--ok:#6FC38F;--warn:#E0A55A}}
*{box-sizing:border-box}html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--ink);font:16px/1.6 -apple-system,BlinkMacSystemFont,"Segoe UI",Inter,Roboto,sans-serif}
a{color:var(--accent)}a:hover{text-decoration-thickness:2px}
.top{display:flex;align-items:center;justify-content:space-between;padding:14px 24px;border-bottom:1px solid var(--hair);position:sticky;top:0;background:var(--bg);z-index:2}
.brand{font-weight:700;color:var(--ink);text-decoration:none}.brand span{font-weight:500;color:var(--i2)}.gh{font-size:14px}
.wrap{display:grid;grid-template-columns:240px minmax(0,1fr);max-width:1180px;margin:0 auto}
.side{padding:20px 16px 40px 24px;border-right:1px solid var(--hair);font-size:14px;position:sticky;top:53px;align-self:start;max-height:calc(100vh - 53px);overflow:auto}
.side ul{list-style:none;margin:0 0 14px;padding:0}.side li a{display:block;padding:4px 8px;border-radius:6px;color:var(--i2);text-decoration:none}
.side li a[aria-current]{background:var(--code);color:var(--ink);font-weight:600}.ng{margin:0 0 4px;font-size:12px;letter-spacing:.06em;text-transform:uppercase;color:var(--i3)}
main{padding:28px 40px 60px;min-width:0}
h1{font-size:32px;line-height:1.2;margin:0 0 12px}h2{font-size:22px;margin:40px 0 10px;padding-top:8px;border-top:1px solid var(--hair)}h3{font-size:17px;margin:26px 0 8px}
.lede{font-size:18px;color:var(--i2);margin:0 0 20px}
code{font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace;background:var(--code);padding:1px 5px;border-radius:4px}
pre{background:var(--code);padding:14px 16px;border-radius:8px;overflow:auto;font:13.5px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace}pre code{background:none;padding:0}
table{border-collapse:collapse;width:100%;margin:10px 0 18px;font-size:14.5px;display:block;overflow-x:auto}th,td{text-align:left;vertical-align:top;padding:7px 10px;border-bottom:1px solid var(--hair)}th{font-weight:600;color:var(--i2);white-space:nowrap}
.route{border:1px solid var(--hair);border-radius:10px;padding:4px 18px 10px;margin:18px 0}.route h3{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:16px}
.m{display:inline-block;min-width:52px;font-weight:700;color:var(--accent)}.tag{display:inline-block;font-size:12px;padding:1px 8px;border-radius:999px;background:var(--code);color:var(--i2);margin-left:6px;font-family:inherit;font-weight:500}
.note{border-left:3px solid var(--accent);padding:6px 14px;background:var(--code);border-radius:0 8px 8px 0}.warn{border-left-color:var(--warn)}
.facts{margin-top:48px;padding-top:14px;border-top:1px solid var(--hair);font-size:13px;color:var(--i3)}
.facts code{word-break:break-all}
@media (max-width:820px){.wrap{grid-template-columns:1fr}.side{position:static;max-height:none;border-right:0;border-bottom:1px solid var(--hair);padding:12px 16px}.side ul{display:flex;flex-wrap:wrap;gap:2px 6px}.ng{margin-top:6px}main{padding:22px 16px 48px}h1{font-size:26px}}
`;

/* Whether a route needs the member's session: the inventory says. `none` needs nothing, and
   `dpop` (/v2/token) is the sign-in script's own proof, made before there is a session. */
function session(r, inv) {
  const row = inv.find((x) => x.method === r.method && x.path === r.path);
  return row ? row.auth === 'session' : true;
}
function ms(v) { return v == null || Number.isNaN(Number(v)) ? '–' : Math.round(Number(v)) + ' ms'; }
function latencyRows(k, lat) {
  const rows = lat.by[k] || [];
  if (!rows.length) return '<p class="lat"><strong>Latency:</strong> not measured yet.</p>';
  return '<p><strong>Latency</strong> (<a href="#latency">how it was measured</a>)</p><table class="lat"><thead><tr><th>Over</th><th>n</th><th>p50</th><th>p95</th><th>First call</th><th></th></tr></thead><tbody>' +
    rows.map((r) => `<tr><td>${esc(r.link)}</td><td>${esc(r.n)}</td><td>${ms(r.p50)}</td><td>${ms(r.p95)}</td><td>${ms(r.cold)}</td><td>${esc(r.note)}</td></tr>`).join('') + '</tbody></table>';
}
/* The runs, once, at the head of Routes: what each link is, which Door, when. */
function latencyRuns(lat) {
  if (!lat.runs.length) return '';
  return '<h2 id="latency">How latency was measured</h2><p>Each route was called many times over each link below, warm (its connection already open) and first (a new connection: TCP and TLS first). p50 is the median, p95 the time 19 calls in 20 beat.</p>' +
    '<table><thead><tr><th>Over</th><th>What</th><th>The Door</th><th>When</th></tr></thead><tbody>' +
    lat.runs.map((r) => `<tr><td>${esc(r.link)}</td><td>${esc(r.detail || r.where)}${r.detail && r.where ? '; ' + esc(r.where) : ''}</td><td>${/^[0-9a-f]{40}$/.test(r.door || '') ? '<code>' + esc(r.door.slice(0, 8)) + '</code>' : esc(r.door)}</td><td>${esc(String(r.at || '').replace('T', ' ').slice(0, 16))} UTC</td></tr>`).join('') + '</tbody></table>';
}

const STATUS = { BAD_REQUEST: 400, UNAUTHORIZED: 401, FORBIDDEN: 403, NOT_FOUND: 404, METHOD_NOT_ALLOWED: 405, CONFLICT: 409, GONE: 410,
  PAYLOAD_TOO_LARGE: 413, UNPROCESSABLE_ENTITY: 422, TOO_MANY_REQUESTS: 429, INTERNAL_SERVER_ERROR: 500, BAD_GATEWAY: 502, SERVICE_UNAVAILABLE: 503 };
const WRITE_ONLY = /^a write from an origin|^the body is over/;
/* A refusal's words are the Door's, with its placeholders shown as <name>; its own process
   failures (a thread gone or panicked, no answer) are one line, since a site can only try again. */
const PROCESS_FAULT = /thread (panicked|is gone)|^no answer$/;
function refusals(row, extra) {
  const seen = new Set(), out = [];
  for (const e of (row && row.errors) || []) {
    if (row.method === 'GET' && e.words && WRITE_ONLY.test(e.words)) continue;   // a GET sends no body and writes nothing
    const code = STATUS[e.status] || e.status;
    // The Door's own failures (its process, its thread) say nothing a site can act on: one line.
    const words = e.words && PROCESS_FAULT.test(e.words) ? '' : (e.words || '').replace(/\{(\w+)\}/g, '<$1>');
    const k = code + words;
    if (seen.has(k)) continue;
    seen.add(k);
    out.push([String(code) + (words ? ` "${words}"` : ''), words ? '' : (e.words ? 'WallFlowers\' own failure: try again shortly' : 'with WallFlowers\' own sentence')]);
  }
  return out.concat(extra || []);
}

function routesPage(lat, inv) {
  const groups = [...new Set(ROUTES.map((r) => r.group))];
  let body = `<h1>Routes</h1><p class="lede">Every route a site calls: WallFlowers' Door at <code>${DOOR}</code>, through <code>session.fetch</code>, which signs each request, and the public page at <code>${ARC}</code>. The ones marked <em>no session</em> need nothing.</p>`;
  body += '<table><thead><tr><th>Route</th><th>For</th><th>Session</th></tr></thead><tbody>' + ROUTES.map((r) =>
    `<tr><td><a href="#${anchor(r)}"><code>${r.method} ${r.origin === 'arc' ? ARC.replace('https://', '') : ''}${esc(r.path)}</code></a></td><td>${esc(r.summary)}</td><td>${session(r, inv) ? 'member' : 'no session'}</td></tr>`).join('') + '</tbody></table>';
  body += latencyRuns(lat);
  for (const g of groups) {
    body += `<h2>${esc(g)}</h2>`;
    for (const r of ROUTES.filter((x) => x.group === g)) {
      const k = r.method + ' ' + r.path;
      const row = inv.find((x) => x.method === r.method && x.path === r.path), errs = refusals(row, r.errors);
      body += `<section class="route" id="${anchor(r)}"><h3><span class="m">${r.method}</span>${r.origin === 'arc' ? '<span style="color:var(--i3)">' + ARC.replace('https://', '') + '</span>' : ''}${esc(r.path)}${session(r, inv) ? '' : '<span class="tag">no session</span>'}</h3>` +
        `<p>${r.doc}</p>` +
        (r.request ? `<p><strong>Request</strong></p><pre><code>${esc(r.request)}</code></pre>` : '') +
        (r.response ? `<p><strong>Answers</strong></p><pre><code>${esc(r.response)}</code></pre>` : '') +
        (errs.length ? '<p><strong>Refusals</strong></p><table><tbody>' + errs.map(([s, w]) => `<tr><td><code>${esc(s)}</code></td><td>${w}</td></tr>`).join('') + '</tbody></table>' : '') +
        ((row && row.limits && Object.keys(row.limits).length && r.method !== 'GET') || r.limits ? `<p><strong>Limits</strong> ${[row && row.limits && row.limits.body_bytes && r.method !== 'GET' ? 'A body of at most ' + mib(row.limits.body_bytes) + '.' : '', row && row.limits && row.limits.steps_max ? 'At most ' + row.limits.steps_max + ' steps.' : '', r.limits || ''].filter(Boolean).join(' ')}</p>` : '') +
        latencyRows(k, lat) + '</section>';
    }
  }
  body += `<h2 id="not-for-sites">Routes that are not for sites</h2><p>The routers serve these too. They belong to WallFlowers' own sign-in window, app and services; a site does not call them, and many refuse a site's session. This list is the routers', so it is complete.</p>`;
  for (const [aud, what] of Object.entries(AUDIENCES)) {
    const rows = inv.filter((r) => r.audience === aud);
    if (!rows.length) continue;
    body += `<h3>${esc(aud)}</h3><p>${what}</p><p>` + rows.map((r) => `<code>${r.method} ${esc(r.path)}</code>`).join(' · ') + '</p>';
  }
  return body;
}

/* A summary from the model, if it reads as public text: one that names a person, a file, an
   internal document or record, or an old name is left out rather than rewritten. */
const INTERNAL = /Ralph|Pacific|\b[A-Z]{1,3}-\d+\b|\.md\b|\brow \d+|kiosk|episode|spine|\bArc\b|§|\bNC\b|MANAGE|BUILD|TBD|RULED|DOCUMENTED|ON THE WIRE|::|\.rs\b|\.py\b|harness|\bMLS\b|\bM\d\b|\d{1,2} Sep|September|\bslice\b|\bfold|\bLWW\b|\bdelta|wire id/i;
/* Its first sentence: the rest is the model's reasoning, for its authors. */
const first = (t) => String(t || '').split(/(?<=[.!?])\s+(?=[A-Z`"(])/)[0];
const clean = (t) => { const f = first(t); return f && !INTERNAL.test(f) ? f : ''; };
/* Arguments a site never sends: gen (WallFlowers adds it), one the model marks unread (nothing
   reads it: forum.react's target), and an attachment's parts (media*, not yet open to sites). */
const HIDDEN_ARG = (a, spec) => a === 'gen' || (spec && spec.unread) || /^media/.test(a);
/* The kinds a site's recipes use, first: the community, its rooms, its public page. */
const SITE_KINDS = ['group', 'forum', 'host'];

/* What the model declares and this release does not open (MANAGE and BUILD, 1 Oct, for R3), by
   model version. The model can't say what a release's routes allow, so it's said here, by hand;
   a new model version starts with none, until someone looks again. */
const NOT_OPEN = {
  '2.3.1': {
    'kind:event': 'Every event is public in this release: leave its visibility at <code>network</code>, the default. The community\'s members see it in WallFlowers as its public page shows it, and can RSVP there. Members-only events and co-hosts are not yet available: no route puts another member on an event\'s roster, so private or connections would reach only its creator.',
    'kind:post': 'Every post is public in this release: leave its visibility at <code>network</code>, the default. The community\'s members see its newest 8 posts in WallFlowers, as its public page lists them. Members-only posts are not yet available: no route puts another member on a post\'s roster, so private or connections would reach only its creator.',
    'kind:contact': 'Connecting two members is not yet available. The model has the link (<code>contact.setLink</code>: one side\'s 1 is an invitation, both sides\' 1 a link, either side\'s 0 ends it; a card reaches a connection only after its own side\'s 1), but no route in this release lets a site, or WallFlowers\' app, reach another member to start one.',
    'kind:transaction': 'Not yet available: no deal can be opened in this release, and every deal op is refused. Trade in this release is the <a href="trade.html">listings board</a>.',
    'facet:visibility': 'On an event or a post, only <code>network</code> is available in this release: <code>private</code> and <code>connections</code> would reach only its creator.',
  },
};
const notOpen = (version, key) => (NOT_OPEN[version] || {})[key];
const notOpenNote = (html) => (html ? `<p style="border-left:3px solid #b8860b;padding:2px 0 2px 12px"><strong>In this release:</strong> ${html}</p>` : '');

function modelPage(m) {
  let body = `<h1>The model: kinds and ops</h1><p class="lede">Everything in a community is an object of some kind, changed by ops. This page is generated from the model itself, version <code>${esc(m.version)}</code>, sha256 <code>${esc(m.sha256)}</code>: the same document <code>GET /v2/icd</code> serves.</p>`;
  body += '<p><strong>Who may write:</strong> <code>owner</code> means the object\'s owner; <code>member</code> means anyone on its roster. WallFlowers refuses the rest. Arguments are text or integers. WallFlowers adds <code>gen</code> itself, so it is not listed; <code>GET /v2/icd</code> lists it as required, but leave it out.</p>';
  body += '<p><strong>What a site reaches:</strong> its community, the community\'s parts (its rooms, its public page), and the objects its own session created. Anything else of the member is refused with <code>… is outside this site\'s scope</code>. The kinds below the first three are listed for completeness: a site meets them only as a part of its community, or by creating one.</p>';
  body += '<table><thead><tr><th>You say</th><th>The model says</th></tr></thead><tbody><tr><td>community</td><td><code>group</code>. The API calls it the Site: <code>session.site</code> is its id, and a site\'s scope is it and its parts.</td></tr><tr><td>room</td><td><code>forum</code> (a part of the community with role <code>room</code>)</td></tr><tr><td>public page</td><td><code>host</code> (a part with role <code>host</code>)</td></tr><tr><td>message</td><td>an entry in a forum\'s <code>messages</code>, made by <code>forum.post</code></td></tr><tr><td>thread</td><td>a message and the replies under it (<a href="rooms.html#threads">Rooms and threads</a>)</td></tr></tbody></table>';
  const opTable = (ops) => '<table><thead><tr><th>Op</th><th>Arguments</th><th>Who</th><th></th></tr></thead><tbody>' + Object.entries(ops).map(([name, op]) => {
    const args = Object.entries(op.args || {}).filter(([a, spec]) => !HIDDEN_ARG(a, spec)).map(([a, s]) => `<code>${esc(a)}</code>${s.required ? '' : '<sup>?</sup>'} <span style="color:var(--i3)">${esc(s.type || '')}</span>`).join('<br>');
    return `<tr><td><code>${esc(name)}</code></td><td>${args || '–'}</td><td>${esc(op.ego || op.who || '')}</td><td>${esc(clean(op.summary))}</td></tr>`;
  }).join('') + '</tbody></table>';
  const kinds = SITE_KINDS.map((n) => m.kinds.find((k) => k.kind === n)).filter(Boolean).concat(m.kinds.filter((k) => !SITE_KINDS.includes(k.kind)));
  kinds.forEach((k, i) => { if (i === SITE_KINDS.length) body += '<h2 id="other-kinds">The rest of the model</h2><p>Reached only as a part of the community, or by creating one.</p>'; body += `<h2 id="kind-${esc(k.kind)}">${esc(k.kind)}</h2>` + (clean(k.summary) ? `<p>${esc(clean(k.summary))}</p>` : '') + notOpenNote(notOpen(m.version, 'kind:' + k.kind)) + (Object.keys(k.ops).length ? opTable(k.ops) : ''); });
  body += '<h2 id="facets">Shared ops (facets)</h2><p>Ops several kinds share. <code>on</code> lists the kinds that carry them.</p>';
  for (const f of m.facets) body += `<h3 id="facet-${esc(f.facet)}">${esc(f.facet)} <span class="tag">on ${esc(f.on.join(', '))}</span></h3>` + (clean(f.summary) ? `<p>${esc(clean(f.summary))}</p>` : '') + notOpenNote(notOpen(m.version, 'facet:' + f.facet)) + opTable(f.ops);
  body += '<p><sup>?</sup> optional.</p>';
  return body;
}

/* ── the draft page (preview only): every op of the draft against production's model ── */
const AREAS = ['Rooms', 'Events', 'Resources', 'Members', 'Links', 'Trade', 'Shared and other'];
function areaOf(where, owner, name) {
  if (where === 'facet') return { about: 'Members', questions: 'Members' }[owner] || 'Shared and other';
  if (owner === 'forum' || (owner === 'conversation' && name === 'forum.post')) return 'Rooms';
  if (owner === 'event') return 'Events';
  if (owner === 'post' || name === 'host.hydrate') return 'Resources';
  if (owner === 'contact') return 'Links';
  if (owner === 'transaction' || owner === 'thing') return 'Trade';
  if (owner === 'group') return { 'group.rsvp': 'Events', 'group.setRegistration': 'Events', 'group.rsvpDecide': 'Events', 'group.setAffiliation': 'Events', 'group.publishListing': 'Trade', 'group.removeListing': 'Trade' }[name] || 'Shared and other';
  return 'Shared and other';
}
/* The routes a site calls for each: null in the middle is filled from the draft (the area's
   new and changed ops for /v2/apply, its new view fields for /v2/graph). The state is by hand. */
const AREA_ROUTES = {
  Rooms: [['POST /v2/apply', null, 'live; the ops wait for the Door to serve the model'], ['GET /v2/graph', null, 'live; the fields with the model']],
  Events: [['POST /v2/apply', null, 'live; the ops wait for the model'], ['GET /v2/graph', null, 'live; the fields with the model'], [`GET ${ARC}/v1/face/<address>/items`, 'public events', 'live']],
  Resources: [['POST /v2/apply', null, 'live; the ops wait for the model'], ['GET /v2/resources.js', 'the editor bundle, SRI-pinned', 'on a branch (w98/resources-editor)'], [`GET ${ARC}/v1/face/<address>/items`, "a post's public page", 'live']],
  Members: [['POST /v2/apply', null, 'live; the ops wait for the model'], ['GET /v2/members', 'the members list', 'on a branch (w98/members), not green']],
  Links: [['POST /v2/apply', null, 'live; but no route reaches another member\'s connection'], ["the Door's link routes", 'starting a connection with a member', 'none in this release: reaching a member waits on D-59']],
  Trade: [['POST /v2/apply', null, 'live; the listings board and thing ops with the model; deal ops refused (the next release)'], ['GET /v2/graph', null, 'live; the fields with the model']],
  'Shared and other': [['POST /v2/apply', null, 'live'], ['GET /v2/graph', 'every view', 'live']],
};
const hexId = (n) => (typeof n === 'number' && n > 0xffff ? '0x' + n.toString(16).toUpperCase() : String(n ?? ''));
function capsOf(a) {
  const c = [];
  if (a.required) c.push('required');
  for (const k of ['maxLength', 'maxBytes', 'maxItems']) if (a[k] != null) c.push(`${k} ${a[k]}`);
  if (a.pattern) c.push(`pattern <code>${esc(a.pattern)}</code>`);
  const keys = (v) => (Array.isArray(v) ? v : v && typeof v === 'object' ? Object.keys(v) : [v]);
  if (a.vocabulary) c.push(`one of ${keys(a.vocabulary).map((v) => `<code>${esc(v)}</code>`).join(', ')}`);
  if (a.mediaRef) c.push(`a media reference: <code>${esc(a.mediaRef.prefix || '')}</code> with ${esc(keys(a.mediaRef.suffixes || []).filter(Boolean).join(', '))}`);
  if (a.default != null) c.push(`default ${esc(JSON.stringify(a.default))}`);
  if (a.rel) c.push(`edges ${[...new Set([].concat(a.rel).map((r) => (r && typeof r === 'object' ? `${r.name} (${r.action || ''} ${r.dir || ''})`.replace(' ()', '') : String(r))))].map((r) => `<code>${esc(r)}</code>`).join(', ')}`);
  if (a.grammar) c.push(`grammar: ${keys(a.grammar).map((v) => `<code>${esc(v)}</code>`).join(', ')}`);
  if (a.items && typeof a.items === 'object') c.push(`each item: ${Object.entries(a.items).map(([k, v]) => `<code>${esc(k)}</code> ${esc((v && v.type) || '')}`).join(', ')}`);
  return c.join('; ');
}
function argsCell(op, baseOp) {
  const rows = Object.entries(op.args || {}).map(([n, a]) => {
    const was = baseOp && (baseOp.args || {})[n];
    const mark = !baseOp ? '' : !was ? ' <span class="tag">new</span>' : JSON.stringify(was) !== JSON.stringify(a) ? ' <span class="tag">changed</span>' : '';
    const caps = capsOf(a);
    return `<code>${esc(n)}</code> <span style="color:var(--i3)">${esc(a.type || '')}</span>${mark}${caps ? `<br><small>${caps}</small>` : ''}`;
  });
  const gone = baseOp ? Object.keys(baseOp.args || {}).filter((n) => !(n in (op.args || {}))).map((n) => `<code>${esc(n)}</code> <span class="tag">removed</span>`) : [];
  return rows.concat(gone).join('<br>') || '–';
}
function changesOf(op, was) {
  if (!was) return 'new';
  const f = [];
  for (const k of ['ego', 'fold']) if (op[k] !== was[k]) f.push(`${k}: ${esc(was[k] || '–')} → ${esc(op[k] || '–')}`);
  if (JSON.stringify(op.args || {}) !== JSON.stringify(was.args || {})) f.push('args');
  if (JSON.stringify(op.view || {}) !== JSON.stringify(was.view || {})) f.push('view');
  for (const k of ['maxLive', 'episode', 'stub']) if (JSON.stringify(op[k]) !== JSON.stringify(was[k])) f.push(k);
  if ((op.summary || op.description) !== (was.summary || was.description)) f.push('wording');
  return f.length ? 'changed: ' + f.join('; ') : 'unchanged';
}
function allOps(d) {
  const out = [];
  for (const [k, v] of Object.entries(d.kinds || {})) for (const [n, op] of Object.entries(v.ops || {})) out.push({ where: 'kind', owner: k, name: n, op });
  for (const [f, v] of Object.entries(d.facets || {})) for (const [n, op] of Object.entries(v.ops || {})) out.push({ where: 'facet', owner: f, name: n, op });
  return out;
}
export function draftPage(pv) {
  const d = pv.draft, b = pv.base;
  const baseOps = new Map(allOps(b).map((o) => [o.where + ' ' + o.owner + ' ' + o.name, o.op]));
  const rows = allOps(d).map((o) => { const was = baseOps.get(o.where + ' ' + o.owner + ' ' + o.name); return { ...o, was, area: areaOf(o.where, o.owner, o.name), change: changesOf(o.op, was) }; });
  const removed = [...baseOps.keys()].filter((k) => !rows.some((o) => o.where + ' ' + o.owner + ' ' + o.name === k));
  const count = (area, st) => rows.filter((o) => o.area === area && (st === 'new' ? o.change === 'new' : st === 'changed' ? o.change.startsWith('changed') : o.change === 'unchanged')).length;
  const newFacets = Object.keys(d.facets || {}).filter((f) => !(f in (b.facets || {})));
  const newRels = Object.keys(d.relations || {}).filter((r) => !(r in (b.relations || {})));
  const newKinds = Object.keys(d.kinds || {}).filter((k) => !(k in (b.kinds || {})));
  let h = `<h1>Draft ${esc(pv.version)}: what changes</h1><p class="lede">Every kind and op of the draft model, against the model production serves (${esc(pv.baseVersion)}), grouped by W-98's six. Generated from the draft itself: nothing on this page is written by hand except the routes' state.</p>`;
  h += `<table><tbody><tr><td>Draft</td><td><code>${esc(pv.version)}</code>, sha256 <code>${esc(pv.sha256)}</code> (git <code>${esc(pv.ref)}</code>). Not pinned: the pin is Ralph's, by a W- row naming the final hash.</td></tr>`;
  h += `<tr><td>Against</td><td><code>${esc(pv.baseVersion)}</code>, sha256 <code>${esc(pv.baseSha256)}</code>: what production's Door serves at <code>/v2/icd</code>.</td></tr>`;
  h += `<tr><td>Ops</td><td>${rows.length} (was ${baseOps.size}): ${rows.filter((o) => o.change === 'new').length} new, ${rows.filter((o) => o.change.startsWith('changed')).length} changed, ${removed.length} removed</td></tr>`;
  h += `<tr><td>New in the model</td><td>${newKinds.length ? 'kinds ' + newKinds.map((k) => `<code>${esc(k)}</code>`).join(', ') + '; ' : ''}facets ${newFacets.map((f) => `<code>${esc(f)}</code>`).join(', ') || '–'}; relations ${newRels.map((r) => `<code>${esc(r)}</code>`).join(', ') || '–'}</td></tr></tbody></table>`;
  const claim = rows.find((o) => o.name === 'base.claimSpent');
  if (claim && claim.was && claim.op.ego !== claim.was.ego) h += `<p><strong>NC-135, the one departure from ${esc(pv.baseVersion)}</strong> (Gate 1): <code>base.claimSpent</code>'s ego is stated as it is enforced, <code>${esc(claim.was.ego)}</code> → <code>${esc(claim.op.ego)}</code>. Every other change adds.</p>`;
  h += '<h2 id="summary">By area</h2><table><thead><tr><th>Area</th><th>New</th><th>Changed</th><th>Unchanged</th></tr></thead><tbody>' +
    AREAS.map((a) => `<tr><td><a href="#${a.toLowerCase().replace(/\W+/g, '-')}">${a}</a></td><td>${count(a, 'new')}</td><td>${count(a, 'changed')}</td><td>${count(a, 'unchanged')}</td></tr>`).join('') + '</tbody></table>';
  const opRow = (o) => `<tr><td><code>${esc(o.name)}</code>${o.where === 'facet' ? `<br><small>facet ${esc(o.owner)}</small>` : o.owner !== o.name.split('.')[0] ? `<br><small>on ${esc(o.owner)}</small>` : ''}</td><td>${esc(hexId(o.op.op))}</td><td>${esc(o.op.ego || '')}</td><td>${esc(o.op.fold || '')}${o.op.maxLive ? `<br><small>maxLive ${o.op.maxLive}</small>` : ''}${o.op.stub ? '<br><small>stub</small>' : ''}</td><td>${argsCell(o.op, o.was)}</td><td>${o.op.view ? Object.keys(o.op.view).map((v) => `<code>${esc(v)}</code>`).join('<br>') : '–'}</td><td>${esc(o.change)}</td></tr>`;
  const head = '<table><thead><tr><th>Op</th><th>Id</th><th>Ego</th><th>Fold</th><th>Arguments and caps</th><th>View</th><th>Against ' + esc(pv.baseVersion) + '</th></tr></thead><tbody>';
  for (const a of AREAS) {
    const mine = rows.filter((o) => o.area === a), moved = mine.filter((o) => o.change !== 'unchanged'), still = mine.filter((o) => o.change === 'unchanged');
    h += `<h2 id="${a.toLowerCase().replace(/\W+/g, '-')}">${a}</h2>`;
    for (const key of [...new Set(mine.map((o) => (o.where === 'facet' ? 'facet:' : 'kind:') + o.owner))]) h += notOpenNote(notOpen(pv.version, key));
    const fresh = (r) => r.startsWith('POST') ? [...new Set(moved.map((o) => o.name))].map((n) => `<code>${esc(n)}</code>`).join(', ') || 'no new or changed op'
      : [...new Set(moved.flatMap((o) => Object.keys(o.op.view || {}).filter((v) => !(o.was && o.was.view && v in o.was.view))))].map((v) => `<code>${esc(v)}</code>`).join(', ') || 'no new view field';
    h += '<table><thead><tr><th>Route a site calls</th><th>For</th><th>State on production</th></tr></thead><tbody>' + (AREA_ROUTES[a] || []).map(([r, f, st]) => `<tr><td><code>${esc(r)}</code></td><td>${f === null ? fresh(r) : esc(f)}</td><td>${esc(st)}</td></tr>`).join('') + '</tbody></table>';
    if (moved.length) h += head + moved.map(opRow).join('') + '</tbody></table>';
    else h += '<p>Nothing new or changed.</p>';
    if (still.length) h += `<details><summary>${still.length} unchanged op${still.length === 1 ? '' : 's'}</summary>` + head + still.map(opRow).join('') + '</tbody></table></details>';
  }
  if (removed.length) h += '<h2>Removed</h2><ul>' + removed.map((k) => `<li><code>${esc(k)}</code></li>`).join('') + '</ul>';
  return h;
}

export function agentsMd(ctx) {
  const dev = devClient();
  const root = dev ? (dev.byPath.find((b) => b.path === '/') || { ports: [] }).ports : [];
  const local = dev ? `1. On the person's own machine, use the shared client \`${dev.id}\`: nothing to register. Callbacks, matched exactly: ${dev.byPath.map(({ path, ports }) => `\`http://${dev.hosts[0]}:<port>${path}\` on ${span_ports(ports)}`).join('; ')}${dev.hosts.length > 1 ? ` (the same on ${dev.hosts.slice(1).join(', ')})` : ''}. The person must own or administer a community first (made at https://wallflowers.io/signup); at sign-in they pick it, and the session reaches only it. A site on the web gets registered (${SITE}/register.html).`
    : `1. Get your site registered first (${SITE}/register.html). Your callback URL is matched exactly, including the port and the trailing slash.`;
  const port = root.length ? (root.includes(5173) ? 5173 : root[0]) : 5173;
  return `# WallFlowers Community API: for AI coding agents

You are building a web app that signs a person in with WallFlowers and reads and writes their community.
Read the linked pages before writing code; they are generated from the running system and are the truth.

- Docs: ${SITE}/  (quickstart: ${SITE}/quickstart.html, routes: ${SITE}/routes.html, model: ${SITE}/model.html)
- API origin: ${DOOR}  (every call is \`session.fetch('/v2/...')\`, JSON in and out)
- Sign-in script: \`<script src="${DOOR}/v2/signin.js" integrity="${ctx.sri}" crossorigin="anonymous"></script>\`
- Machine-readable routes: ${SITE}/routes.json · the data model: ${DOOR}/v2/icd (no session needed; from a browser, only on an origin WallFlowers allows)
- Baseline these docs describe: model ${ctx.icd} (sha256 ${ctx.icdSha})${ctx.door ? ', the Door at ' + ctx.door.slice(0, 8) : ''}; machine-readable: ${SITE}/baseline.json
- A working starter to begin from (two files, no build): ${SITE}/starter/index.html and ${SITE}/starter/app.js
${REPO ? '- Source: ' + REPO + '\n' : ''}
## Do

${local}
2. Load the sign-in script once per page with the integrity attribute above. Never copy it, bundle it, or load it unpinned.
3. On the callback page call \`WallFlowers.finish({client, callback})\`; on every other page \`WallFlowers.current({client})\`. \`null\` means signed out: show a button that calls \`WallFlowers.signIn({client, callback})\`.
4. Read everything with \`GET /v2/graph\` and draw from it. Write one change with \`POST /v2/apply {object, op, args}\`, several in order with \`POST /v2/batch\`.
5. Open one change stream per page (\`session.events\`), and on \`changed\` re-read \`/v2/graph\` about 300 ms after the last one. Re-read after your own writes too.
6. Treat a \`401\`, or a \`400\` saying \`not signed in\`, as signed out: show your Sign in button again (it calls \`WallFlowers.signIn\`). If you send the person to sign in yourself, at most once per page view, and never in a loop. Show any other refusal's sentence to the person; do not parse it.
7. Sign out every session you open: \`session.signOut()\` (\`POST /v2/signout\`) when done, and in scripts and tests at exit. An abandoned session is held until it has been idle for ${ctx.idle}, and once WallFlowers holds ${ctx.sessions} sessions, everyone's sign-in is refused (\`503 the Door is at its session limit\`).
8. Name people from \`profiles\` on the community's or the room's view. Never show a key as a name; say "New member".
9. Take arguments from the model: \`GET /v2/icd\` (or ${SITE}/model.html). Arguments are text or integers. Never send \`gen\`: the model lists it as required, and WallFlowers fills it in.
10. The community is \`session.site\`; its rooms are its view's \`parts\` with role \`room\`. A room's messages are its view's \`messages\` (\`author\`, \`text\`, \`ts\`, \`gen\`, …: ${SITE}/views.html); a message has no length of its own beyond the request body (${SITE}/limits.html). Events and resources your site's session created are in its reach, in \`/v2/graph\`; others are not. The community's published ones are at \`${ARC}/v1/face/<address>/items\` (no session)${ctx.registerPublishes ? '; a community is published as it is made at signup (one made before 1 October, 14:00 KST, may not be yet, and answers 404 there)' : ', but only once WallFlowers has published the community: a new one answers 404 there'}. \`<address>\` (\`:slug\` on the routes page) is the community's address, as in wallflowers.io/<address>. A member's session can't read it: put it in your app's configuration, as signup showed it or as it was chosen at Register. Read it on load and when something changes, never faster than once every 30 s, never in a loop.
11. Creating many objects: one \`POST /v2/batch\` (up to ${ctx.steps} steps), or send them concurrently. Never one awaited mint after another: each mint gets slower as the account holds more.
12. Serve your page from the exact origin and callback you use, e.g. \`python3 -m http.server ${port} --bind 127.0.0.1\` (the bind keeps it to this machine), and open exactly \`http://localhost:${port}/\`.

## Don't

- Don't store or send passwords, passkeys or keys: WallFlowers' window does the passkey; your page never sees it.
- Don't poll faster than every few seconds, and don't open a stream per component.
- Don't put third-party scripts on pages where a member is signed in: anything on the page can use the session.
- Don't guess routes. If it is not on ${SITE}/routes.html, it is not for sites.
`;
}

function llmsTxt() {
  return `# WallFlowers Community API\n\n> Build on a WallFlowers community: sign in with a passkey through WallFlowers' window, read the community's rooms and people, and write as the member.\n\n` +
    navGroups().map(([g, items]) => `## ${g}\n\n` + items.map(([s, t]) => `- [${t}](${SITE}/${s === 'index' ? '' : s + (s === 'agents' ? '.md' : '.html')})`).join('\n')).join('\n\n') + '\n';
}

/* Teams, once contact codes are open (event.json contact_codes): how a community's owner adds a
   teammate, who then signs in to the same community on the site. */
const teamsHtml = (cfg) => `<h2 id="teams">Teams: adding a teammate</h2>
<p>A community has one owner. To build together, the owner adds each teammate to it:</p>
<ol>
<li>The teammate makes a WallFlowers account at <code>${DOOR}</code> (Create an account), then opens <em>Your card</em> (the profile menu, or <code>${DOOR}/#you</code>) and, under <em>Your contact code</em>, presses <strong>Copy a new code</strong>. They send the code to the owner.</li>
<li>The owner opens the community in WallFlowers, then its settings (the round badge beside its name), <strong>Add by contact code</strong>, pastes it into <em>Contact code</em>, and presses <strong>Add</strong>. The teammate joins the community and up to five of its rooms, in the community's order; for each room past five, the owner's app says "<em>room</em>: needs another code".</li>
<li>The teammate signs in on your site: they now reach the same community.${devClient() ? ` On your own machine, with <code>${esc(devClient().id)}</code>, the sign-in offers only communities the person owns or administers: the owner first makes the teammate an admin (their card in WallFlowers, <strong>Make admin</strong>).` : ''}</li>
</ol>
<p>A code works once${cfg.DOOR_CODE_SECS ? `, for ${span(cfg.DOOR_CODE_SECS)}` : ''}: "Used already" or "the contact code has expired" means ask for a new one.</p>`;

/* ── build ───────────────────────────────────────────────────────────────── */

export function build(ev = event()) {
  const inv = inventory(), m = icd(), lat = latency(), lim = limits(inv), cfg = door(ev);
  MODEL_OPS = new Set([...m.kinds, ...m.facets].flatMap((k) => Object.keys(k.ops || {})).concat(inv.map((r) => r.method + ' ' + r.path)));
  const parts = resourcesParts();
  const ctx = { registerPublishes: !!ev.register_publishes, sri: sri(), resourcesSri: parts.length ? sri(parts) : '', icd: m.version, icdSha: m.sha256, door: RELEASE, ev, steps: lim.steps, idle: span(cfg.DOOR_IDLE_SECS), sessions: String(cfg.DOOR_MAX_SESSIONS) };
  const fromRelease = Object.entries(RELEASED).filter(([, v]) => v).map(([k]) => ({ 'app/web/door/element.js': 'its sign-in script', 'app/door/src/main.rs': 'its settings', 'app/door/routes.json': 'its routes', 'core/coordination/delta-graph.icd.json': 'its model' })[k]).filter(Boolean);
  const facts = `Generated from ${RELEASE && fromRelease.length ? `the Door at <code>${esc(RELEASE.slice(0, 8))}</code>, the one production serves (${fromRelease.join(', ')}), ` : ''}the routers' inventory (app/door/routes.json), the model version ${esc(m.version)} (sha256 <code>${esc(m.sha256.slice(0, 16))}…</code>), and ` +
    (lat.runs.length ? `latencies measured ${[...new Set(lat.runs.map((r) => esc(String(r.at || '').slice(0, 10))))].join(', ')}` : 'no latency measurements yet') +
    `. Sign-in script integrity <code>${esc(ctx.sri)}</code>.` + (REPO ? ` Source: <a href="${REPO}">${REPO.replace('https://', '')}</a>, app/door/docs.` : ' The source will be public with WallFlowers\' open-source release.');
  const dev = devClient(), every = reloadEvery();
  const devHtml = dev ? `<h2 id="local">On your machine: nothing to register</h2>
<p>For local development, every team uses one shared client, <code>${esc(dev.id)}</code>. Serve your page on ${dev.hosts.map((h) => `<code>http://${esc(h)}:&lt;port&gt;</code>`).join(' or ')}, and use one of these as your callback, character for character:</p>
<ul>${dev.byPath.map(({ path, ports }) => `<li><code>http://${esc(dev.hosts[0])}:&lt;port&gt;${esc(path)}</code>, for port ${esc(span_ports(ports))}</li>`).join('')}</ul>${dev.hosts.length > 1 ? (dev.sameOnAll ? `<p>The same on <code>${esc(dev.hosts.slice(1).join(', '))}</code>. They are different origins: a member signed in on one is not signed in on the other.</p>`
  : dev.hosts.slice(1).map((h) => `<p>On <code>${esc(h)}</code>: ${dev.paths.map((p) => `<code>${esc(p)}</code> for port ${esc(span_ports(dev.portsOf(h, p)))}`).join('; ')}. A different origin from <code>${esc(dev.hosts[0])}</code>: a member signed in on one is not signed in on the other.</p>`).join('')) : ''}
<p>When a member signs in, WallFlowers asks which of their communities your page may use: the ones they own or administer. The session reaches that community and nothing else. Someone who owns or administers none is told so ("you are the owner or an admin of no Site"), and must make one first at <a href="https://wallflowers.io/signup">wallflowers.io/signup</a>. ${SIGNUP_NOTE} Make a test community for the hackathon and choose that one, not a community with real members: your page acts there with your full standing.</p>
<p>This client works only on your own machine. To put your site on the web, register it as below.</p>` : '';
  // The starter's callback is its own page, the origin's root: the shortcut holds only if "/" is registered.
  const rootPorts = dev ? (dev.byPath.find((b) => b.path === '/') || { ports: [] }).ports : [];
  const qsDev = rootPorts.length ? dev : null;
  const qsPort = qsDev ? String(rootPorts.includes(5173) ? 5173 : rootPorts[0]) : '5173';
  const devQs = qsDev ? `<p><strong>Make your community first</strong>, at <a href="https://wallflowers.io/signup">wallflowers.io/signup</a>, before you run the starter: you will own it. Signing up from the starter instead makes an account with no community, and its sign-in stops at "you are the owner or an admin of no Site". If that happened, make the community at signup, press <strong>Create my account</strong>, and on the next page choose <strong>Sign in with a passkey</strong> with that account.</p><p>Nothing to register on your machine: use the client <code>${esc(qsDev.id)}</code>, and serve on port ${esc(qsPort)}. When you sign in, WallFlowers asks which of your communities the page may use: pick your test community (<a href="register.html#local">On your machine</a>). To put your site on the web later, <a href="register.html">register it</a>.</p>`
    : `<p>You need a <strong>client id</strong> and an exact <strong>callback URL</strong> registered for your community first: see <a href="register.html">Register your app</a>. For this quickstart, register <code>http://localhost:${qsPort}/</code> as your callback.</p>`;
  const regTiming = every ? `A registration counts within seconds of WallFlowers adding it: the Door reads its list of sites again every ${every} s, with no restart, so no one is signed out. It does not change the sign-in script or its hash.`
    : 'WallFlowers adds registrations in batches, so send yours early. It does not change the sign-in script or its hash.';
  const fill = (html) => html.replace(/\{\{(\w+)\}\}/g, (_, k) => ({ SRI: ctx.sri, RESOURCES_SRI: ctx.resourcesSri, DOOR, ARC, SITE, REPO, ICD: esc(m.version),
    IDLE: ctx.idle, SESSIONS: ctx.sessions, TOKEN: span(cfg.DOOR_TOKEN_SECS), TOKEN_SECS: String(cfg.DOOR_TOKEN_SECS),
    PER_ADDR: String(cfg.DOOR_ATTEMPTS_PER_ADDR), ATTEMPT: span(cfg.DOOR_ATTEMPT_SECS), SIGNUP: span(cfg.DOOR_SIGNUP_SECS),
    // Where a registration goes: event.json's register_html, set for the event without a Door
    // release (MANAGE, 1 Oct: the organisers' channel, Ralph's to name); until then, plainly, the organisers.
    REGISTER: ev.register_html || '<p>Send these to the hackathon\'s organisers.</p>', EVENT: ev.event_html || '', DEV: devHtml, DEV_QS: devQs, QS_PORT: qsPort, QS_STEP1: qsDev ? '1. Your client id' : '1. Get your site registered', QS_CLIENT: qsDev ? `<code>${esc(qsDev.id)}</code>` : 'your client id', QS_CLIENT_JS: qsDev ? esc(qsDev.id) : 'your-client-id', REG_TIMING: regTiming, SIGNUP_NOTE,
    ORIGIN_EXAMPLES: qsDev ? `a <code>localhost</code> port the shared client doesn't cover, a preview address, a site not registered yet` : `<code>127.0.0.1</code> for <code>localhost</code>, another port, a preview address`,
    START_LOCAL: qsDev ? `<p><strong>Make your community first</strong>, at <a href="https://wallflowers.io/signup">wallflowers.io/signup</a>, before you run the starter: you will own it. Signing up from the starter instead makes an account with no community, and its sign-in stops at "you are the owner or an admin of no Site". If that happened, make the community at signup, press <strong>Create my account</strong>, and on the next page choose <strong>Sign in with a passkey</strong> with that account.</p><p>Then nothing to register: use the shared client <code>${esc(qsDev.id)}</code> on <code>localhost</code>, and follow the <a href="quickstart.html">Quickstart</a>. When you sign in, pick your community (<a href="register.html#local">On your machine</a>).</p>`
      : `<p>Register <code>http://localhost:${qsPort}/</code> as your callback (<a href="register.html">Register your app</a>), then follow the <a href="quickstart.html">Quickstart</a>.</p>`,
    STARTER_ORIGIN_FIX: qsDev ? `on localhost, use client ${qsDev.id} on a port it covers; any other address must be registered (Register your app, on the docs).` : 'it must be registered (Register your app, on the docs).',
    QS_ORIGIN_FIX: qsDev ? `on <code>localhost</code>, use the client <code>${esc(qsDev.id)}</code> on a port it covers; any other address must be registered (<a href="register.html">Register your app</a>)` : `it must be registered first (<a href="register.html">Register your app</a>)`,
    MEMBERS_ADD: ev.contact_codes ? 'In WallFlowers, the owner adds a teammate by their contact code: see <a href="register.html#teams">Teams: adding a teammate</a>.'
      : 'In WallFlowers, adding someone needs them to be your connection first, and connecting is not yet available (<a href="model.html">the model</a>, <code>contact</code>). So for now, test with an account that is already a member: the community\'s owner, or someone who joined with its QR code.',
    // W-103 (website signup, 1 Oct): an optional "Your app" step, and after Register the Site id and the
    // request, each with Copy. Said only when event.json's signup_app says the website serves it.
    SIGNUP_APP_STEP: ev.signup_app ? 'then, if you are building an app, <strong>Your app</strong> (its local port, callback path and deployed URL; or <em>Skip</em>), ' : '',
    SIGNUP_APP_AFTER: ev.signup_app ? '<p>With <strong>Your app</strong> filled in, signup brings you back after you register and lists your community\'s Site id, Address, Client and Callback and, for a deployed URL, the request to send, each with <strong>Copy</strong>.</p>' : '',
    SIGNUP_APP_ID: ev.signup_app ? ' If you filled in <strong>Your app</strong>, signup shows its Site id afterwards, with <strong>Copy</strong>, and the request for a deployed URL. Without it:' : '',
    // R3.2 (HACK_USER, 05:26Z): Register publishes the page in about 24 s; communities made before 14:00 KST may not be.
    PUBLISHED_INDEX: ev.register_publishes ? '' : ', once the community is published',
    PUBLISHED_NOTE: ev.register_publishes ? '<strong>Published at signup.</strong> A community made at <a href="https://wallflowers.io/signup">wallflowers.io/signup</a> is published as it is made: its page and these routes answer within a minute. One made before 1 October, 14:00 KST, may still be unpublished and answer <code>404</code> here: ask the organisers to publish it. Signed in, your page also reads, in <code>/v2/graph</code>, the events and resources its own session created.'
      : '<strong>Published communities only.</strong> WallFlowers publishes a community\'s public page; a community made today isn\'t published yet, and answers <code>404</code> here. Until it is, your signed-in page reads, in <code>/v2/graph</code>, the events and resources its own session created.',
    PUBLISHED_ROUTE: ev.register_publishes ? 'A community made at signup is published as it is made; one made before 1 October, 14:00 KST, may not be yet, and answers <code>404</code> (ask the organisers).'
      : 'Only a published community has one: a community made today isn\'t published yet, and answers <code>404</code>; signed in, your page reads the events and resources its own session created in <code>/v2/graph</code>.',
    SIGNUP_APP_QS: ev.signup_app ? '<p>Made your community with <strong>Your app</strong>? Signup\'s last page lists its Site id, Address, Client and Callback: the starter needs only Client and Callback (<code>CONFIG.client</code>, <code>CONFIG.callback</code>); keep the Address for the public page.</p>' : '',
    START_DEV_SCOPE: qsDev ? ` <code>${esc(qsDev.id)}</code> covers only your own machine.` : '',
    BODY: mib(lim.body), SKEW: String(cfg.PROOF_SKEW ?? 60), STEPS: String(lim.steps), POW: String(lim.pow), TEAMS: ev.contact_codes ? teamsHtml(cfg) : '' })[k] ?? `{{${k}}}`);
  const out = {};
  if (PREVIEW) {
    const base = released(ICD_PATH);
    Object.assign(PREVIEW, { version: m.version, sha256: m.sha256, draft: JSON.parse(icdAt(PREVIEW.ref).toString('utf8')),
      base: JSON.parse(base.toString('utf8')), baseVersion: JSON.parse(base.toString('utf8')).version, baseSha256: createHash('sha256').update(base).digest('hex') });
  }
  for (const [, items] of navGroups()) {
    for (const [slug, title] of items) {
      if (slug === 'agents') continue;
      let body;
      if (slug === 'routes') body = fill(routesPage(lat, inv));
      else if (slug === 'draft') body = draftPage(PREVIEW);
      else if (slug === 'model') body = modelPage(m);
      else if (slug === 'changelog') body = markdown(readFileSync(CHANGELOG, 'utf8'));
      else body = fill(readFileSync(path.join(HERE, 'pages', slug + '.html'), 'utf8'));
      out[slug === 'index' ? 'index.html' : slug + '.html'] = layout(slug, title, body, facts);
    }
  }
  out['docs.css'] = CSS;
  for (const f of readdirSync(path.join(HERE, 'starter'))) out['starter/' + f] = fill(readFileSync(path.join(HERE, 'starter', f), 'utf8'));
  out['agents.md'] = agentsMd(ctx);
  out['llms.txt'] = llmsTxt();
  out['baseline.json'] = JSON.stringify({ icd: { version: m.version, sha256: m.sha256 }, door: RELEASE, signin_sri: ctx.sri, ...(ctx.resourcesSri ? { resources_sri: ctx.resourcesSri } : {}) }, null, 2) + '\n';
  out['routes.json'] = JSON.stringify({
    door: DOOR, model: m.version, signin_integrity: ctx.sri, ...(ctx.resourcesSri ? { resources_integrity: ctx.resourcesSri } : {}),
    routes: ROUTES.map((r) => ({ method: r.method, origin: r.origin === 'arc' ? ARC : DOOR, path: r.path, session: session(r, inv), summary: r.summary, docs: `${SITE}/routes.html#${anchor(r)}`,
      latency: (lat.by[r.method + ' ' + r.path] || []).map((l) => ({ link: l.link, n: l.n, p50_ms: l.p50, p95_ms: l.p95, first_call_p50_ms: l.cold })) })),
    not_for_sites: inv.filter((r) => r.audience !== 'site').map((r) => ({ method: r.method, path: r.path, audience: r.audience }))
  }, null, 2) + '\n';
  return { out, problems: problems(inv) };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1]) && process.argv.includes('--preview')) {
  const at = (flag) => process.argv[process.argv.indexOf(flag) + 1];
  const ref = at('--preview'), dir = process.argv.includes('--out') ? path.resolve(at('--out')) : '';
  if (!ref || !dir || dir === DIST || dir.startsWith(DIST + path.sep)) { console.error('usage: build.mjs --preview <git ref of the draft> --out <a folder that is not dist/>'); process.exit(2); }
  preview(ref);
  const { out } = build();
  rmSync(dir, { recursive: true, force: true });
  for (const [f, s] of Object.entries(out)) { mkdirSync(path.dirname(path.join(dir, f)), { recursive: true }); writeFileSync(path.join(dir, f), s); }
  console.log(`docs: a preview of the model at ${ref} in ${dir} (${Object.keys(out).length} files; open ${path.join(dir, 'draft.html')})`);
} else if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  const { out, problems: bad } = build();
  if (process.argv.includes('--check')) {
    const stale = Object.entries(out).filter(([f, s]) => !existsSync(path.join(DIST, f)) || readFileSync(path.join(DIST, f), 'utf8') !== s).map(([f]) => f);
    const extra = existsSync(DIST) ? readdirSync(DIST, { recursive: true }).filter((f) => !(f in out) && !Object.keys(out).some((k) => k.startsWith(f + '/'))) : [];
    for (const p of bad) console.error('  ' + p);
    if (stale.length) console.error('  dist/ is not what the sources build: ' + stale.join(', ') + ' (run node app/door/docs/build.mjs)');
    if (extra.length) console.error('  dist/ holds files the sources do not build: ' + extra.join(', '));
    if (bad.length || stale.length || extra.length) process.exit(1);
    console.log(`docs: ${Object.keys(out).length} files current; every route the Door serves accounted for`);
  } else {
    if (bad.length) { for (const p of bad) console.error('  ' + p); process.exit(1); }
    rmSync(DIST, { recursive: true, force: true });
    mkdirSync(DIST, { recursive: true });
    for (const [f, s] of Object.entries(out)) { mkdirSync(path.dirname(path.join(DIST, f)), { recursive: true }); writeFileSync(path.join(DIST, f), s); }
    console.log(`docs: wrote ${Object.keys(out).length} files to app/door/docs/dist`);
  }
}
