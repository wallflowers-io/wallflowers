/* create.test.mjs — NC-81: what a Site creates is declared from both ends.

   A post, an event or a thing made from a Site's sheet is the Site's own only when the
   Site's `group.setAffiliation {rel: created}` and the object's own `base.setBacklink
   {object: site, rel: created}` both say so (O-48's face_items reads both). webapp.js
   runs as the page runs it, over a stub page, the real ICD and a stub Door.

   Run: node --test app/web/webapp/create.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));
const ORIGIN = 'https://app.wallflowers.io';
const ME = 'c'.repeat(64), SITE = 'a'.repeat(64), MADE = 'd'.repeat(64);

function page(answers = {}, { rooms = [], store = {} } = {}) {
  class Node {
    constructor(tag) {
      Object.assign(this, { tag, children: [], _text: '', hidden: false, disabled: false, value: '', className: '', attrs: {}, scrollTop: 0 });
      this.style = { setProperty() {}, removeProperty() {} };
      this.classList = { add() {}, remove() {}, toggle() {}, contains() { return false; } };
    }
    get textContent() { return this._text + this.children.map((c) => c.textContent || '').join(''); }
    set textContent(v) { this._text = String(v); this.children = []; }
    set innerHTML(v) { this._html = String(v); }
    get innerHTML() { return this._html || ''; }
    setAttribute(k, v) { this.attrs[k] = String(v); }
    getAttribute(k) { return this.attrs[k] ?? null; }
    removeAttribute(k) { delete this.attrs[k]; }
    appendChild(c) { this.children.push(c); return c; }
    addEventListener(t, f) { this['on' + t] = f; }
    focus() {}
    /* The layout a browser always has; the page measures its top bar and rails. */
    getBoundingClientRect() { return { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 }; }
    querySelectorAll() { return []; }
    closest() { return null; }
    remove() {}
    get firstChild() { return this.children[0] || (this._html ? (this._parsed ||= new Node('svg')) : null); }
    get lastChild() { return this.children.at(-1) || null; }
    insertBefore(c, ref) { const i = this.children.indexOf(ref); this.children.splice(i < 0 ? this.children.length : i, 0, c); return c; }
    querySelector() { return null; }
  }
  const byId = {};
  const $ = (id) => (byId[id] ||= new Node('#' + id));
  for (const id of ['sheet', 'side', 'refused', 'status', 'compose', 'manage']) $(id).hidden = true;
  const calls = [];
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const site = { id: SITE, kind: 'group', name: 'Mill Road Allotments', owner: ME, members: [ME], folds: true,
    view: { display_name: 'Mill Road Allotments', affiliations: [], parts: [] } };
  site.view.parts = rooms.map((r, i) => ({ part: r, role: 'room', at: i }));
  const graph = { me: { pk: 'ed25519:' + ME }, objects: [site, ...rooms.map((r, i) => ({ id: r, kind: 'forum', name: 'Room ' + (i + 1), owner: ME, members: [ME], folds: true, view: { messages: [] } }))], spine: [] };
  const fetch = async (url, init = {}) => {
    const path = url.slice(ORIGIN.length), body = init.body ? JSON.parse(init.body) : null;
    calls.push({ path, body });
    if (path === '/v2/batch') {
      // Each step as the call it stands for, its $steps resolved: what each route would get.
      const got = [];
      for (const [i, s] of body.steps.entries()) {
        const r = (x) => (x && typeof x === 'object' && '$step' in x ? got[x.$step] : x);
        const sub = s.do === 'mint' ? '/v2/mint' : '/v2/apply';
        const b = s.do === 'mint' ? { kind: s.kind, draft: s.draft }
          : { object: r(s.object), op: s.op, args: Object.fromEntries(Object.entries(s.args || {}).map(([k, v]) => [k, r(v)])) };
        calls.push({ path: sub, body: b, batched: true });
        const a = answers[sub];
        const no = typeof a === 'function' ? a(b) : null;
        if (no) return reply(422, { made: got, refused: { step: i, why: no[1] } });
        got.push(s.do === 'mint' ? MADE : b.object);
      }
      return reply(200, { made: got, refused: null });
    }
    const a = answers[path];
    if (typeof a === 'function') { const r = a(body); if (r) return reply(r[0], r[1]); }
    if (path === '/v2/me') return reply(200, { pk: 'ed25519:' + ME });
    if (path === '/v2/graph') return reply(200, graph);
    if (path === '/v2/icd') return reply(200, ICD);
    if (path.startsWith('/v2/draft/')) return reply(200, { name_only: true });
    if (path === '/v2/mint') return reply(200, { object_id: MADE });
    return reply(200, {});
  };
  const loc = { hash: '#site=' + SITE, pathname: '/', search: '', origin: ORIGIN, assign() {} };
  const ctx = {
    document: { getElementById: $, documentElement: new Node('html'), createElement: (t) => new Node(t),
      createTextNode: (t) => ({ textContent: String(t) }), addEventListener() {} },
    location: loc, history: { replaceState() {} }, fetch, doorOrigin: () => ORIGIN,
    addEventListener() {}, scrollTo() {},
    innerWidth: 1280, matchMedia: () => ({ matches: false, addEventListener() {} }), requestAnimationFrame: (f) => setImmediate(f),
    btoa, localStorage: { getItem: (k) => store[k] ?? null, setItem: (k, v) => { store[k] = String(v); } },
    atob, TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, console,
    setTimeout: (f) => setImmediate(f), clearTimeout() {}
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  return { $, calls };
}

const settle = async () => { for (let i = 0; i < 40; i++) await new Promise((r) => setImmediate(r)); };
const writes = (p) => p.calls.filter((c) => c.path === '/v2/mint' || c.path === '/v2/apply');
function find(n, pred) {
  if (pred(n)) return n;
  for (const c of n.children || []) { const f = find(c, pred); if (f) return f; }
  return null;
}
/* A Site's sheet for `label`, as a person opens it: the Site, its tab along the top, the
   tab's New in the left rail, a name. */
const TAB = { forums: ['Rooms', 'New room'], publications: ['Resources', 'New resource'],
              events: ['Events', 'New event'], marketplace: ['Trade', 'New listing'] };
async function make(p, label, name) {
  await settle();
  const [tabName, newName] = TAB[label];
  const tab = find(p.$('tabs'), (n) => n.tag === 'button' && n.textContent === tabName);
  assert.ok(tab, `the ${tabName} tab`);
  tab.onclick();
  await settle();
  const add = find(p.$('lbody'), (n) => n.tag === 'button' && n.textContent === newName);
  assert.ok(add, `the Site's ${label} section offers a new one`);
  add.onclick();
  await settle();
  p.$('f_name').value = name;
  p.$('sheetForm').onsubmit({ preventDefault() {} });
  await settle();
}

for (const [label, kind] of [['publications', 'post'], ['events', 'event'], ['marketplace', 'thing']]) {
  test(`${kind}: made from the Site's sheet, declared from both ends at one moment`, async () => {
    const p = page();
    await make(p, label, 'Seed swap');
    const w = writes(p).map((c) => c.body);
    assert.deepEqual(w.map((b) => b.kind || b.op), [kind, 'group.setAffiliation', 'base.setBacklink']);
    const [, aff, back] = w;
    assert.deepEqual([aff.object, aff.args.peer, aff.args.rel, aff.args.name], [SITE, MADE, 'created', 'Seed swap'], "the Site's half");
    assert.deepEqual([back.object, back.args.object, back.args.rel], [MADE, SITE, 'created'], "the object's half");
    assert.equal(back.args.at, aff.args.at, 'one moment from both ends');
    assert.deepEqual(p.calls.filter((c) => !c.batched && ['/v2/batch', '/v2/mint', '/v2/apply'].includes(c.path)).map((c) => c.path), ['/v2/batch'], 'one request');
    assert.equal(p.$('sheet').hidden, true);
  });
}

test("a refused backlink is said, with how far the link got", async () => {
  const p = page({ '/v2/apply': (b) => (b.op === 'base.setBacklink' ? [403, 'not yours to write'] : null) });
  await make(p, 'events', 'Dig day');
  assert.equal(p.$('sheetWhy').textContent, 'the Site names it, and it does not name the Site: not yours to write');
  assert.equal(p.$('sheet').hidden, false, 'the sheet stays, with the refusal on it');
});

test('a part names its parent, and writes no backlink', async () => {
  const p = page();
  await make(p, 'forums', 'General');
  const w = writes(p).map((c) => c.body);
  assert.deepEqual(w.map((b) => b.kind || b.op), ['forum', 'base.setPart', 'base.setParent']);
  assert.equal(w[2].args.at, w[1].args.at);
});

/* ADD BY CONTACT CODE (W-96): the owner pastes a person's code; its first key adds them to the
   community, each next key to one of its rooms, in order. */
const MEMBER = 'e'.repeat(64), ROOMS = ['1'.repeat(64), '2'.repeat(64), '3'.repeat(64)];
const code = (bundles) => 'wf1.' + Buffer.from(JSON.stringify(bundles)).toString('base64url');
const adds = (p) => p.calls.filter((c) => c.path === '/v2/add').map((c) => c.body);
async function paste(p, text) {
  await settle();
  p.$('siteRole').onclick();
  const opt = find(p.$('manageBody'), (n) => n.tag === 'button' && /Add by contact code/.test(n.textContent));
  assert.ok(opt, "the owner's settings offer Add by contact code");
  opt.onclick();
  find(p.$('sheetFields'), (n) => n.attrs && n.attrs.id === 'codeIn').value = text;
  p.$('sheetForm').onsubmit({ preventDefault() {} });
  await settle();
}
const answers = { '/v2/add': (b) => (b.object === SITE ? [200, { member: MEMBER }] : null) };

test('a contact code adds its person to the community, then a room a key', async () => {
  const p = page(answers, { rooms: ROOMS.slice(0, 2) });
  await paste(p, code(['b0', 'b1', 'b2']));
  assert.deepEqual(adds(p), [{ object: SITE, bundle: 'b0' }, { object: ROOMS[0], bundle: 'b1' }, { object: ROOMS[1], bundle: 'b2' }]);
  assert.equal(p.$('sheet').hidden, true);
});

test('a room past the code\'s keys is named, not added', async () => {
  const p = page(answers, { rooms: ROOMS });
  await paste(p, code(['b0', 'b1']));
  assert.deepEqual(adds(p).map((b) => b.object), [SITE, ROOMS[0]]);
  assert.match(p.$('toast').textContent, /^Added to Mill Road Allotments, not to 2 rooms$/);
});

test('a code used once is refused the second time, and nothing is sent', async () => {
  const store = {};
  const first = page(answers, { rooms: [], store });
  await paste(first, code(['b0']));
  assert.equal(adds(first).length, 1);
  const again = page(answers, { rooms: [], store });
  await paste(again, code(['b0']));
  assert.deepEqual(adds(again), []);
  assert.match(again.$('sheetWhy').textContent, /^Used already\. Ask them for a new one\.$/);
});

test('what is not a contact code is said, and nothing is sent', async () => {
  const p = page(answers);
  await paste(p, 'hello there');
  assert.deepEqual(adds(p), []);
  assert.match(p.$('sheetWhy').textContent, /^Not a contact code\.$/);
});
