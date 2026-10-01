#!/usr/bin/env node
/* THE ICD CHANGELOG, held to the releases (Ralph, 30 Sep: "both COMMUNITY_API and docs.wallflowers.io are
   always baselined on ICD releases, and ... a changelog is maintained"). deploy.md § 10 is the method.

     node app/door/docs/icd-changelog.mjs                 print each release's computed changes
     node app/door/docs/icd-changelog.mjs --write         rewrite the generated block of each entry
     node app/door/docs/icd-changelog.mjs --check         exit 1 unless every icd/* tag has an entry naming its
                                                          version and pin, each entry's generated block is what
                                                          the releases compute, and the docs' baseline (the ICD
                                                          at door-release) is a released ICD
     node app/door/docs/icd-changelog.mjs --served        exit 1 unless production's GET /v2/icd is the docs'
                                                          baseline and the served docs' signin.js integrity is
                                                          production's /v2/signin.js (after every U6 docs deploy)
     node app/door/docs/icd-changelog.mjs --served --dist the same for dist/, the docs about to ship (before it)

   The releases are the icd/* tags; an entry's ops come from the ICD at its tag against the one before, so
   nothing here is typed twice. A draft (DRAFTS below) is the same, from a named commit, until it is tagged.
   What a person adds by hand sits outside the markers: departures, and for each new feature the route a
   site uses and its docs page. */
import { readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const PRODUCT = path.resolve(HERE, '../../..');
const LOG = path.join(HERE, 'icd-changelog.md');
const ICD = 'core/coordination/delta-graph.icd.json';
const DOOR = 'https://app.wallflowers.io';
const DOCS = 'https://docs.wallflowers.io';
// A draft stays here until its icd/* tag exists; then its tag is the release and the draft line goes.
const DRAFTS = [];

const git = (...a) => execFileSync('git', ['-C', PRODUCT, ...a], { encoding: 'utf8', maxBuffer: 64 << 20 });
const sha256 = (s) => createHash('sha256').update(s).digest('hex');
const at = (ref, p) => git('show', `${ref}:${p}`);
const vkey = (v) => v.split('.').map(Number).reduce((a, n) => a * 1000 + n, 0);

/** The released ICDs, oldest first: { version, tag, ref, sha, date, doc }. */
export function releases() {
  return git('tag', '-l', 'icd/*').split('\n').filter(Boolean).map((tag) => {
    const bytes = at(tag, ICD);
    const doc = JSON.parse(bytes);
    const date = git('log', '-1', '--format=%cs', `${tag}^{commit}`).trim();
    return { version: doc.version, tag, ref: tag, sha: sha256(bytes), date, doc };
  }).sort((a, b) => vkey(a.version) - vkey(b.version));
}

export function drafts(rel) {
  const released = new Set(rel.map((r) => r.version));
  return DRAFTS.filter((d) => !released.has(d.version)).map((d) => {
    const bytes = at(d.ref, ICD);
    const doc = JSON.parse(bytes);
    return { version: doc.version, tag: null, ref: d.ref, sha: sha256(bytes), date: null, doc, note: d.note };
  });
}

/** Every op the model declares, by name: where it lives, its id, who may write it, how it folds, its args. */
export function ops(doc) {
  const out = new Map();
  const take = (where, o) => {
    for (const [name, op] of Object.entries(o || {})) {
      const args = Object.fromEntries(Object.entries(op.args || {}).map(([a, s]) =>
        [a, [s.type, s.required ? 'required' : '', s.maxLength != null ? `max ${s.maxLength}` : ''].filter(Boolean).join(' ')]));
      out.set(name, { where, id: op.op, ego: op.ego, fold: op.fold, args });
    }
  };
  for (const [k, v] of Object.entries(doc.kinds || {})) take(k, v.ops);
  for (const [f, v] of Object.entries(doc.facets || {})) take(`facet ${f}`, v.ops);
  return out;
}

/** What changed between two models: kinds, and ops added, retired, renamed (same place and id) or changed. */
export function diff(before, after) {
  const a = ops(before), b = ops(after), lines = [];
  const kinds = (d) => new Set(Object.keys(d.kinds || {}));
  for (const k of kinds(after)) if (!kinds(before).has(k)) lines.push(`kind added: ${k}`);
  for (const k of kinds(before)) if (!kinds(after).has(k)) lines.push(`kind retired: ${k}`);
  const place = (m) => new Map([...m].map(([n, o]) => [`${o.where}#${o.id}`, n]));
  const pa = place(a), pb = place(b), renamed = new Map();
  for (const [n, o] of b) if (!a.has(n)) { const was = pa.get(`${o.where}#${o.id}`); if (was && !b.has(was)) renamed.set(was, n); }
  for (const [was, now] of renamed) lines.push(`renamed: ${was} → ${now}`);
  for (const [n, o] of b) if (!a.has(n) && ![...renamed.values()].includes(n)) lines.push(`added: ${n} (${o.where}, op ${o.id}, ${o.ego}, ${o.fold})`);
  for (const [n, o] of a) if (!b.has(n) && !renamed.has(n)) lines.push(`retired: ${n} (${o.where}, op ${o.id})`);
  for (const [n, o] of b) {
    const p = a.get(n) || a.get([...renamed].find(([, now]) => now === n)?.[0]);
    if (!p) continue;
    const said = [];
    for (const f of ['where', 'id', 'ego', 'fold']) if (p[f] !== o[f]) said.push(`${f} ${p[f]} → ${o[f]}`);
    const args = new Set([...Object.keys(p.args), ...Object.keys(o.args)]);
    for (const x of [...args].sort()) {
      if (!(x in p.args)) said.push(`+${x} (${o.args[x]})`);
      else if (!(x in o.args)) said.push(`-${x}`);
      else if (p.args[x] !== o.args[x]) said.push(`${x}: ${p.args[x] || 'optional'} → ${o.args[x] || 'optional'}`);
    }
    if (said.length) lines.push(`changed: ${n}: ${said.join('; ')}`);
  }
  // Everything else the model states (its rules, facets' fields, the matrix), named by section when it moved.
  const rest = (d, k) => JSON.stringify(k === 'kinds' || k === 'facets'
    ? Object.fromEntries(Object.entries(d[k] || {}).map(([n, v]) => [n, { ...v, ops: undefined }])) : d[k]);
  const sections = [...new Set([...Object.keys(before), ...Object.keys(after)])]
    .filter((k) => k !== 'version' && rest(before, k) !== rest(after, k)).sort();
  if (sections.length) lines.push(`model sections changed besides ops: ${sections.join(', ')}`);
  return lines.sort();
}

const BEGIN = (v) => `<!-- generated ${v}: icd-changelog.mjs --write; do not edit between the markers -->`;
const END = (v) => `<!-- end generated ${v} -->`;

export function block(r, prev) {
  const head = r.tag
    ? `- Release: \`${r.tag}\`, ${r.date}; the pin \`${r.sha}\``
    : `- Draft (${r.note}), from \`${r.ref}\`; its ICD \`${r.sha}\`, not a release until its icd/* tag`;
  const since = prev ? `- Against ${prev.version} (\`${prev.sha.slice(0, 16)}…\`):` : '- The first release: every op is new.';
  const body = prev ? diff(prev.doc, r.doc).map((l) => `  - ${l}`) : [];
  return [BEGIN(r.version), head, since, ...(prev && !body.length ? ['  - no op or kind changed'] : body), END(r.version)].join('\n');
}

function entries() {
  const rel = releases(), all = [...rel, ...drafts(rel)];
  return all.map((r, i) => ({ r, prev: all[i - 1], text: block(r, all[i - 1]) }));
}

function baseline() {
  const release = readFileSync(path.join(HERE, 'door-release'), 'utf8').trim().split(/\s/)[0];
  const sha = sha256(at(release, ICD));
  return { release, sha };
}

export function check() {
  const bad = [], log = readFileSync(LOG, 'utf8');
  for (const { r, text } of entries()) {
    if (!log.includes(`## ${r.version}`)) { bad.push(`${r.version}: no "## ${r.version}" entry`); continue; }
    if (r.tag && !log.includes(r.sha)) bad.push(`${r.version}: its entry does not name its pin ${r.sha}`);
    const m = log.match(new RegExp(`${BEGIN(r.version).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}[\\s\\S]*?${END(r.version).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`));
    if (!m) bad.push(`${r.version}: no generated block (run --write)`);
    else if (m[0] !== text) bad.push(`${r.version}: the generated block is not what the releases compute (run --write)`);
  }
  const b = baseline(), rel = releases();
  const hit = rel.find((r) => r.sha === b.sha);
  if (!hit) bad.push(`the docs' baseline, the ICD at door-release ${b.release.slice(0, 8)} (${b.sha.slice(0, 16)}…), is no released ICD`);
  for (const l of bad) console.error(`✗ icd-changelog: ${l}`);
  if (!bad.length) console.log(`icd-changelog: ${rel.length} release(s) logged and current; the docs describe ${hit.version} (${hit.tag})`);
  return bad.length ? 1 : 0;
}

async function served(local) {
  const bad = [], b = baseline();
  const icd = sha256(Buffer.from(await (await fetch(`${DOOR}/v2/icd`)).arrayBuffer()));
  if (icd !== b.sha) bad.push(`production /v2/icd is ${icd.slice(0, 16)}…; the docs describe ${b.sha.slice(0, 16)}…`);
  const js = Buffer.from(await (await fetch(`${DOOR}/v2/signin.js`)).arrayBuffer());
  const sri = 'sha384-' + createHash('sha384').update(js).digest('base64');
  for (const p of ['signin.html', 'quickstart.html', 'starter/app.js', 'agents.md']) {
    const text = local ? readFileSync(path.join(HERE, 'dist', p), 'utf8') : await (await fetch(`${process.env.DOCS_URL || DOCS}/${p}`)).text();
    const found = [...new Set(text.match(/sha384-[A-Za-z0-9+/=]{64}/g) || [])];
    if (!found.length) bad.push(`${p}: names no integrity`);
    for (const h of found) if (h !== sri) bad.push(`${p}: integrity ${h.slice(0, 20)}…, production's signin.js is ${sri.slice(0, 20)}…`);
  }
  for (const l of bad) console.error(`✗ icd-changelog --served: ${l}`);
  if (!bad.length) console.log(`icd-changelog --served${local ? ' --dist' : ''}: production serves the ${local ? 'built' : 'served'} docs' ICD and signin.js`);
  return bad.length ? 1 : 0;
}

function write() {
  let log = readFileSync(LOG, 'utf8');
  for (const { r, text } of entries()) {
    const re = new RegExp(`${BEGIN(r.version).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}[\\s\\S]*?${END(r.version).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`);
    if (re.test(log)) log = log.replace(re, text);
    else console.error(`icd-changelog: no block for ${r.version}; add "## ${r.version}" and its markers by hand, then --write`);
  }
  writeFileSync(LOG, log);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const a = process.argv.slice(2);
  if (a.includes('--check')) process.exit(check());
  else if (a.includes('--served')) process.exit(await served(a.includes('--dist')));
  else if (a.includes('--write')) write();
  else for (const { text } of entries()) console.log(text + '\n');
}
