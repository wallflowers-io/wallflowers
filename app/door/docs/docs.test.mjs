/* docs.test.mjs: the Community API's documentation says what the Door serves, and is built.

   Every route the routers serve for sites is documented, every documented route is one they serve
   for sites, every other audience is explained, and dist/ is what the sources build (so what
   docs.wallflowers.io serves is never behind the Door). The Door's settings (idle time, session
   cap, lifetimes) are stated from its configuration, never written into the prose.

   Run: node --test app/door/docs/docs.test.mjs */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { build, problems, inventory, sri, door, icd, RELEASE, preview, devClient, reloadEvery } from './build.mjs';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.join(HERE, 'dist');

test('every route for sites is documented, and nothing documented is not served', () => {
  assert.deepEqual(problems(), []);
});

test('a route the routers serve for sites, with no page, is caught', () => {
  const inv = inventory().concat([{ method: 'GET', path: '/v2/brand-new', audience: 'site', auth: 'session', errors: [] }]);
  assert.match(problems(inv).join('\n'), /GET \/v2\/brand-new for sites, and no page says what it is/);
});

test('dist/ is what the sources build', () => {
  const { out } = build();
  for (const [f, s] of Object.entries(out)) {
    assert.ok(existsSync(path.join(DIST, f)), `${f} is built`);
    assert.equal(readFileSync(path.join(DIST, f), 'utf8'), s, `${f} is current: run node app/door/docs/build.mjs`);
  }
});

test('the sign-in script is pinned by its own hash, everywhere it is named', () => {
  const pin = sri();
  const { out } = build();
  for (const f of ['signin.html', 'quickstart.html', 'agents.md', 'starter/app.js', 'routes.json']) assert.ok(out[f].includes(pin), `${f} names ${pin}`);
});

test('the Door\'s settings come from its configuration, and the deployment\'s values win', () => {
  const d = door({});
  assert.equal(d.DOOR_IDLE_SECS, 900, 'main.rs\'s default');
  assert.equal(d.DOOR_MAX_SESSIONS, 64, 'main.rs\'s default');
  const { out } = build({ door: { DOOR_IDLE_SECS: 300, DOOR_MAX_SESSIONS: 200, DOOR_TOKEN_SECS: 7200 } });
  for (const f of ['agents.md', 'quickstart.html', 'session.html', 'limits.html']) {
    assert.match(out[f], /5 minutes/, `${f}: the deployed idle time`);
    assert.match(out[f], /200 sessions|200, for all of WallFlowers/, `${f}: the deployed cap`);
  }
  assert.match(out['session.html'], /2 hours/);
  assert.match(out['routes.html'], /&quot;expires_in&quot;: 7200/);
});

test('no page writes a Door setting into its prose', () => {
  const src = readdirSync(path.join(HERE, 'pages')).map((f) => readFileSync(path.join(HERE, 'pages', f), 'utf8'))
    .concat(readFileSync(path.join(HERE, 'routes.mjs'), 'utf8')).join('\n');
  for (const w of [/\b15 minutes\b/, /\bone hour\b/, /\b64 sessions\b/, /"expires_in": \d/, /\b60 s to finish\b/]) assert.doesNotMatch(src, w);
});

test('a GET lists no write refusal, and the model page no internal note', () => {
  const { out } = build();
  for (const sec of out['routes.html'].split('<section class="route"').slice(1).filter((s) => /<span class="m">GET</.test(s))) {
    assert.doesNotMatch(sec, /a write from an origin|the body is over/, sec.slice(0, 80));
  }
  assert.doesNotMatch(out['model.html'], /RULED|DOCUMENTED|ON THE WIRE|::|\.rs\b|harness|\bMLS\b/);
});

test('the baseline is stated once, from the Door release: the model production serves and the sign-in script', () => {
  const { out } = build();
  const b = JSON.parse(out['baseline.json']);
  const product = path.resolve(HERE, '../../..');
  const file = RELEASE ? execFileSync('git', ['-C', product, 'show', `${RELEASE}:core/coordination/delta-graph.icd.json`]) : readFileSync(path.join(product, 'core/coordination/delta-graph.icd.json'));
  assert.equal(b.icd.sha256, createHash('sha256').update(file).digest('hex'), 'the ICD at door-release');
  assert.equal(b.icd.version, JSON.parse(file.toString('utf8')).version);
  assert.equal(b.icd.sha256, icd().sha256);
  assert.equal(b.door, RELEASE);
  assert.equal(b.signin_sri, sri());
  assert.ok(out['model.html'].includes(b.icd.sha256), 'the model page names it');
  assert.ok(out['agents.md'].includes(b.icd.sha256), 'agents.md names it');
});

test("a preview of a model draft says so on every page, tabulates it against production's, and leaves the published build alone", () => {
  const published = build().out;
  preview(RELEASE || 'HEAD');
  let out;
  try { out = build().out; } finally { preview(null); }
  const pages = Object.keys(out).filter((f) => f.endsWith('.html') && !f.includes('/'));   // starter/ is the app, not a page
  for (const f of pages) assert.match(out[f], /Preview, not published/, `${f} says it is a preview`);
  assert.ok(out['draft.html'], 'the draft page is built');
  const base = JSON.parse(released_icd());
  const n = Object.values(base.kinds || {}).concat(Object.values(base.facets || {})).reduce((k, v) => k + Object.keys(v.ops || {}).length, 0);
  assert.match(out['draft.html'], new RegExp(`Ops</td><td>${n} \\(was ${n}\\): 0 new, 0 changed, 0 removed`), 'the same model against itself: nothing moves');
  assert.equal(build().out['index.html'], published['index.html'], 'the published build is as it was');
  assert.doesNotMatch(published['index.html'], /Preview, not published/);
  assert.equal(existsSync(path.join(DIST, 'draft.html')), false, 'no draft in dist/');
});

function released_icd() {
  return RELEASE ? execFileSync('git', ['-C', path.join(HERE, '../../..'), 'show', `${RELEASE}:core/coordination/delta-graph.icd.json`]).toString('utf8')
    : readFileSync(path.join(HERE, '../../../core/coordination/delta-graph.icd.json'), 'utf8');
}

test('registration as the Door release has it: the dev client from its registry, and how soon a registration counts', () => {
  const { out } = build();
  const reg = out['register.html'], qs = out['quickstart.html'], dev = devClient(), every = reloadEvery();
  if (dev) {
    assert.match(reg, /On your machine: nothing to register/);
    assert.ok(reg.includes(`<code>${dev.id}</code>`), 'the dev client is named as registered');
    for (const p of dev.paths) assert.ok(reg.includes(`:&lt;port&gt;${p}</code>`), `its callback path ${p}`);
    assert.ok(reg.includes(String(dev.ports[0])) && reg.includes(String(dev.ports[dev.ports.length - 1])), 'its ports');
    assert.equal(qs.includes(dev.id), dev.paths.includes('/'), "the quickstart's shortcut only when the starter's own page can be the callback");
  } else {
    assert.doesNotMatch(reg, /On your machine/);
    assert.doesNotMatch(qs, /skip this step/);
  }
  if (every) assert.ok(reg.includes(`every ${every} s, with no restart`));
  else assert.match(reg, /adds registrations in batches/);
  assert.doesNotMatch(reg + qs, /\{\{[A-Z_]+\}\}/, 'nothing left unfilled');
});
