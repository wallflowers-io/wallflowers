#!/usr/bin/env node
/* face-fixtures.mjs: faces as the webapp's editor writes them, for face.schema.json's tests.

     node core/coordination/face-fixtures.mjs           write core/pacific-core/tests/fixtures/faces/
     node core/coordination/face-fixtures.mjs --check   exit 1 if they are stale

   face.js runs as the page loads it (no DOM: document() and load() read only its state), and
   each fixture is what its document() answers after load(), and published/ what it publishes
   from it: the editor's own default, every theme, and one face using every widget, sticker kind, question kind and door. So a change to
   the editor that the schema does not allow fails core's test, not a visitor's page. */
import { readFileSync, writeFileSync, mkdirSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import vm from 'node:vm';

const here = path.dirname(fileURLToPath(import.meta.url));
const FACE = path.join(here, '../../app/web/webapp/assets/face');
const OUT = path.join(here, '../pacific-core/tests/fixtures/faces');
const SCHEMA = JSON.parse(readFileSync(path.join(here, 'face.schema.json'), 'utf8'));

function editor() {
  const window = { document: { createElement() { return {}; }, head: {}, addEventListener() {} },
                   addEventListener() {}, fetch() { return new Promise(() => {}); } };
  window.window = window;
  const ctx = vm.createContext(window);
  for (const f of ['vendor/face-assets.js', 'look.js', 'face.js']) vm.runInContext(readFileSync(path.join(FACE, f), 'utf8'), ctx, { filename: f });
  return window.WallFlowers;
}

/* The working copy (document()) and what face.js publishes from it (deltas(): faceDoc() in
   group.setFace, the social links in the card's urls, the location in base.setLocation). */
function written(doc) {
  const F = editor().Face;
  if (doc) F.load(doc, {});
  const d = F.deltas(), arg = (op) => (d.find((x) => x.op === op) || {}).args;
  const loc = arg('base.setLocation');
  return { working: F.document(), published: { face: JSON.parse(arg('group.setFace').face),
    urls: JSON.parse(arg('group.setProfile').card).urls, location: loc ? JSON.parse(loc.source) : null } };
}

const W = editor(), assets = W.FaceAssets;
const make = (key, size) => ({ ...SCHEMA['x-wf-widgets'].find((w) => w.key === key).make, size });
const faces = { 'default': written(null) };
for (const p of SCHEMA['x-wf-presets']) faces['preset-' + p.id] = written({ look: p.look });
faces['every-part'] = written({
  look: { ...SCHEMA['x-wf-presets'][3].look, text: { font: assets.fonts[0].id, title: '' } },
  header: { layout: 'hero', title: 'logo' },
  show: { members: false, address: true, purpose: true, share: false },
  door: { access: 'open', label: 'Come in',
          questions: [{ kind: 'short', prompt: 'Your name?', required: true, options: [] },
                      { kind: 'long', prompt: 'Why?', required: false, options: [] },
                      { kind: 'choice', prompt: 'Which night?', required: true, options: ['Fri', 'Sat'] },
                      { kind: 'agree', prompt: 'I agree to the rules', required: true, options: [] }] },
  notice: { on: true, kind: 'custom', text: 'Mind the step.' },
  sharing: { listed: false },
  stickers: [{ id: 'doodle-heart', x: 0.2, y: 0.4, size: 0.3, rot: -8, flip: false, colour: 'title' },
             { id: assets.stickers[0].id, x: 0.8, y: 1.6, size: 0.24, rot: 12, flip: true }],
  blocks: { root: { children: [
    ...SCHEMA['x-wf-widgets'].map((w) => make(w.key, w.sizes[w.sizes.length - 1])),
    { ...make('link', 's'), title: 'Tickets', url: 'https://example.org', colours: { bg: '#CD5B7A', fg: '#FFFFFF' } },
    { ...make('social', 'm'), links: [{ platform: 'instagram', handle: 'riverside' }, { platform: 'email', handle: 'hi@example.org' }] },
    { ...make('rooms', 'l'), empty: 'note', emptyNote: 'Nothing yet' }
  ] } }
});

const want = {};
for (const [k, v] of Object.entries(faces)) {
  want[k + '.json'] = JSON.stringify(v.working, null, 1) + '\n';
  want['published/' + k + '.json'] = JSON.stringify(v.published, null, 1) + '\n';
}
if (process.argv.includes('--check')) {
  const have = Object.fromEntries(['', 'published/'].flatMap((d) => readdirSync(path.join(OUT, d)).filter((f) => f.endsWith('.json')).map((f) => [d + f, readFileSync(path.join(OUT, d, f), 'utf8')])));
  const stale = [...new Set([...Object.keys(want), ...Object.keys(have)])].filter((f) => want[f] !== have[f]);
  if (stale.length) { console.error('face fixtures stale: ' + stale.join(', ')); process.exit(1); }
} else {
  mkdirSync(path.join(OUT, 'published'), { recursive: true });
  for (const [f, t] of Object.entries(want)) writeFileSync(path.join(OUT, f), t);
  console.log(Object.keys(faces).length + ' faces → ' + path.relative(process.cwd(), OUT));
}
