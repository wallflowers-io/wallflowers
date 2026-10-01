/* perf.test.mjs — what a person waits on, timed (perf.js; Ralph, 28 Sep: latency first).

   perf.js over a stub clock: each step one `wf:` measure, the summary's percentiles by
   nearest rank, nothing sent. Then the window and the webapp as they run, over stub
   pages and a stub Door: the steps a sign-in, a sign-up and a landing record, in order.

   Run: node --test app/web/door/perf.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const PERF = readFileSync(new URL('./perf.js', import.meta.url), 'utf8');
const SIGNIN = readFileSync(new URL('./signin.js', import.meta.url), 'utf8');
const WEBAPP = readFileSync(new URL('../webapp/webapp.js', import.meta.url), 'utf8');

/* A Performance with a clock the test moves, and the measures it was given. */
function clock() {
  const P = { t: 0, measures: [], timeOrigin: 1000 };
  P.now = () => P.t;
  P.measure = (name, o) => { P.measures.push({ name, duration: o.end - o.start, start: o.start, detail: o.detail }); };
  P.getEntriesByType = (type) => (type === 'measure' ? P.measures : []);
  return P;
}

function load(ctx) {
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(PERF, ctx);
  return ctx;
}

const steps = (P) => P.measures.map((m) => m.name);
const settle = async () => { for (let i = 0; i < 40; i++) await new Promise((r) => setImmediate(r)); };

test('a step is one wf: measure, from its start to now, with its detail', () => {
  const P = clock();
  const W = load({ performance: P, location: { pathname: '/' } }).WallFlowersPerf;
  P.t = 10;
  const t = W.begin();
  P.t = 250;
  W.end('GET /v2/graph', t, { bytes: 3 });
  assert.deepEqual(P.measures, [{ name: 'wf:GET /v2/graph', duration: 240, start: 10, detail: { bytes: 3 } }]);
});

test('shown waits for the frame after the paint; with no frames, at once', async () => {
  const P = clock();
  const frames = [];
  const ctx = load({ performance: P, location: { pathname: '/' }, requestAnimationFrame: (f) => frames.push(f), setTimeout: (f) => setImmediate(f) });
  ctx.WallFlowersPerf.shown('landing', 0);
  P.t = 40;
  assert.equal(P.measures.length, 0, 'nothing before the frame');
  frames.shift()();
  P.t = 55;
  await settle();
  assert.deepEqual(steps(P), ['wf:landing']);
  assert.equal(P.measures[0].duration, 55);

  const Q = clock();
  load({ performance: Q, location: { pathname: '/' } }).WallFlowersPerf.shown('x', 0);
  assert.deepEqual(steps(Q), ['wf:x']);
});

test('the summary: n, p50, p95 and max per step by nearest rank, the page, and nothing else', () => {
  const P = clock();
  P.measures.push(...[5, 1, 4, 2, 3, 100, 6, 7, 8, 9].map((d) => ({ name: 'wf:post', duration: d })));
  P.measures.push({ name: 'something else', duration: 1 });
  const logged = [];
  const W = load({ performance: P, location: { pathname: '/' }, console: { table: (x) => logged.push(x) } }).WallFlowersPerf;
  const s = W.summary();
  // Made in the page's realm: compared by value.
  assert.deepEqual(JSON.parse(JSON.stringify(s.steps)), { post: { n: 10, p50: 5, p95: 100, max: 100, last: 9 } });
  assert.equal(s.timeOrigin, 1000);
  assert.equal(logged.length, 1);
  assert.equal(logged[0], s.steps, 'printed');
  W.summary(true);
  assert.equal(logged.length, 1, 'quiet');
});

test('no Performance: the calls do nothing, and the summary is null', () => {
  const W = load({ location: { pathname: '/' } }).WallFlowersPerf;
  W.end('x', W.begin());
  W.shown('y', 0);
  assert.equal(W.summary(), null);
});

/* ── the window ───────────────────────────────────────────────────────────── */
function windowPage(search) {
  const P = clock();
  const els = {};
  const el = (id) => (els[id] ||= { id, hidden: false, disabled: false, textContent: '', focus() {}, appendChild() {}, insertBefore() {} });
  for (const id of ['words', 'shown']) el(id).hidden = true;
  let landed = null;
  const reply = (body) => ({ ok: true, status: 200, text: async () => JSON.stringify(body) });
  // The Door answers after 5 ms of the clock each time: every step has a duration.
  const fetch = async (path) => {
    P.t += 5;
    if (path.startsWith('/v2/work')) return reply({ challenge: null });
    if (path === '/v2/signin') return reply({ attempt: 'a1', key: 'k1' });
    if (path === '/v2/signup') return reply({ attempt: 'a1', key: 'k1', handle: 'AQID', pk: 'ed25519:' + 'c'.repeat(64), host: 'h' });
    if (path === '/v2/signup/finish') return reply({ pk: 'c'.repeat(64), words: 'one two' });
    return reply({ redirect: '/' });
  };
  const prf = { prf: { results: { first: new Uint8Array(32) }, enabled: true } };
  const ctx = load({
    performance: P, location: { search, pathname: '/signin', origin: 'https://door', replace(u) { landed = u; } },
    document: { documentElement: { removeAttribute() {} }, getElementById: el, querySelector: (q) => ({ content: q.includes('rp-id') ? 'door' : '' }), createElement: () => ({}) },
    navigator: { userAgent: '', credentials: {
      create: async () => { P.t += 5; return { rawId: new Uint8Array([9]).buffer, getClientExtensionResults: () => prf }; },
      get: async () => { P.t += 5; return { response: { userHandle: new Uint8Array([1]) }, getClientExtensionResults: () => prf }; } } },
    fetch, setTimeout: (f) => setImmediate(f), URLSearchParams, TextEncoder, crypto: globalThis.crypto, Promise, JSON, Error, Uint8Array, String,
  });
  ctx.WallFlowersSeal = { b64u: () => 'AQID', unb64u: () => new Uint8Array([1]), seal: async () => { P.t += 5; return { epk: 'e', iv: 'i', ct: 'c' }; } };
  ctx.returnPath = () => '/';
  vm.runInContext(SIGNIN, ctx);
  return { P, el, landed: () => landed };
}

test('the window: a sign-in records each step it waits on, then the whole, before it lands', async () => {
  const w = windowPage('');
  w.el('in').onclick();
  for (let i = 0; i < 20 && !w.landed(); i++) await settle();
  assert.equal(w.landed(), '/');
  // the passkey first, then the attempt (TEST, run 110: a refused check opens none)
  assert.deepEqual(steps(w.P), ['wf:first-render', 'wf:signin:passkey', 'wf:signin:work', 'wf:signin:start', 'wf:signin:seal',
    'wf:signin:finish', 'wf:signin']);
  assert.ok(w.P.measures.slice(1).every((m) => m.duration > 0), 'each step took the clock');
});

test('the window: a sign-up records its start, then passkey, seal, finish and the words', async () => {
  const w = windowPage('?new');
  w.el('new').onclick();   // one tap: the passkey at once (Ralph, 29 Sep)
  for (let i = 0; i < 20 && w.el('shown').hidden; i++) await settle();
  await settle();
  assert.equal(w.el('shown').hidden, false);
  assert.deepEqual(steps(w.P), ['wf:first-render', 'wf:signup:work', 'wf:signup:start', 'wf:signup:open', 'wf:signup:passkey',
    'wf:signup:seal', 'wf:signup:finish', 'wf:signup:words']);
});

/* ── the webapp ───────────────────────────────────────────────────────────── */
function webappPage() {
  const P = clock();
  class Node {
    constructor() {
      Object.assign(this, { children: [], hidden: false, disabled: false, value: '', className: '', attrs: {}, scrollTop: 0 });
      this.style = { setProperty() {}, removeProperty() {} };
      this.classList = { add() {}, remove() {}, toggle() {}, contains() { return false; } };
    }
    set textContent(v) { this.children = []; }
    get textContent() { return ''; }
    setAttribute(k, v) { this.attrs[k] = v; }
    getAttribute(k) { return this.attrs[k] ?? null; }
    appendChild(c) { this.children.push(c); return c; }
    addEventListener(t, f) { this['on' + t] = f; }
    focus() {}
    querySelector() { return null; }
    /* The layout a browser always has; the page measures its top bar and rails. */
    getBoundingClientRect() { return { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 }; }
    querySelectorAll() { return []; }
    closest() { return null; }
    remove() {}
    set innerHTML(v) { this._html = String(v); }
    get innerHTML() { return this._html || ''; }
    get firstChild() { return this.children[0] || (this._html ? (this._parsed ||= new Node()) : null); }
    get lastChild() { return this.children.at(-1) || null; }
    insertBefore(c, ref) { const i = this.children.indexOf(ref); this.children.splice(i < 0 ? this.children.length : i, 0, c); return c; }
  }
  const byId = {};
  const $ = (id) => (byId[id] ||= new Node());
  const reply = (body) => ({ ok: true, status: 200, text: async () => JSON.stringify(body) });
  const forum = 'f'.repeat(64);
  const graph = { objects: [{ id: forum, kind: 'forum', name: 'general', folds: true, view: { messages: [] } }], spine: [] };
  const fetch = async (url, init = {}) => {
    P.t += 5;
    const path = url.slice('https://door'.length);
    if (path === '/v2/apply') graph.objects[0].view.messages.push({ author: 'c', text: JSON.parse(init.body).args.text });
    if (path === '/v2/graph') return reply(graph);
    if (path === '/v2/icd') return reply({ kinds: {}, facets: {} });
    return reply({ pk: 'ed25519:' + 'c'.repeat(64) });
  };
  const ctx = load({
    performance: P,
    document: { getElementById: $, documentElement: new Node(), createElement: () => new Node(), createTextNode: () => ({}), addEventListener() {}, querySelectorAll: () => [] },
    location: { hash: '', pathname: '/', search: '', origin: 'https://door' }, history: { replaceState() {} },
    fetch, doorOrigin: () => 'https://door', addEventListener() {}, scrollTo() {},
    innerWidth: 1280, matchMedia: () => ({ matches: false, addEventListener() {} }), requestAnimationFrame: (f) => setImmediate(f),
    TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, console,
    setTimeout: (f) => setImmediate(f), clearTimeout() {},
  });
  vm.runInContext(WEBAPP, ctx);
  return { P, $, graph };
}

test('the webapp: a landing records each fetch, the model, the render, the graph and the landing', async () => {
  const p = webappPage();
  await settle();
  const names = steps(p.P);
  for (const s of ['wf:GET /v2/me', 'wf:first-render', 'wf:GET /v2/graph', 'wf:GET /v2/icd', 'wf:model', 'wf:render', 'wf:graph', 'wf:landing']) {
    assert.ok(names.includes(s), `${s} in ${names.join(', ')}`);
  }
  assert.equal(names.filter((n) => n === 'wf:landing').length, 1, 'landing once');
  const graph = p.P.measures.find((m) => m.name === 'wf:GET /v2/graph');
  assert.equal(graph.detail.status, 200);
  assert.ok(graph.detail.bytes > 0);
});

test('the webapp: a post is timed from submit to the graph that shows it', async () => {
  const p = webappPage();
  await settle();
  // Open the room from its card in the feed (the feed's column holds the cards), and say
  // something, as a person does.
  const card = (n) => (typeof n.onclick === 'function' ? n : (n.children || []).map(card).find(Boolean));
  card(p.$('feed')).onclick();
  const before = p.P.measures.length;
  p.$('say').value = 'hello';
  p.$('compose').onsubmit({ preventDefault() {} });
  await settle();
  assert.deepEqual(p.graph.objects[0].view.messages.map((m) => m.text), ['hello']);
  const after = steps(p.P).slice(before);
  const at = (n) => after.indexOf(n);
  assert.ok(at('wf:POST /v2/apply') >= 0 && at('wf:POST /v2/apply') < at('wf:GET /v2/graph'), `the write, then the graph: ${after.join(', ')}`);
  assert.ok(at('wf:graph') < at('wf:post'), `the graph drawn, then the post: ${after.join(', ')}`);
  const post = p.P.measures.find((m) => m.name === 'wf:post');
  const apply = p.P.measures.find((m) => m.name === 'wf:POST /v2/apply');
  assert.ok(post.duration > apply.duration, 'the post waits on the write and the graph after it');
});
