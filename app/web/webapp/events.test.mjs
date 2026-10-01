/* events.test.mjs — W-98 Events, the seam (UX's, pdr/w98-integration.md § Lanes and seams):
   webapp.js hands an event's pages to their own modules through three hooks and one interface.
   "New event" opens the editor (WallFlowers.EventsEdit) in place of the Site's sheet, Manage
   opens it on the event, and an event opens on its page (WallFlowers.EventsPage), each drawn
   with eventsCtx(): {door, batch, load, open, el, toast, M, S, icd}, the only thing either
   module reads of the page. Without the modules, the page is as it was. webapp.js runs as the
   page runs it, over a stub page, the real ICD and a stub Door.

   Run: node --test app/web/webapp/events.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));
const ORIGIN = 'https://app.wallflowers.io';
const ME = 'c'.repeat(64), SITE = 'a'.repeat(64), MADE = 'd'.repeat(64), EVENT = 'e'.repeat(64);
const CTX = ['M', 'S', 'batch', 'door', 'el', 'icd', 'load', 'open', 'toast'];

function page(modules = {}, answers = {}) {
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
  for (const id of ['sheet', 'side', 'refused', 'status', 'compose']) $(id).hidden = true;
  const calls = [];
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const site = { id: SITE, kind: 'group', name: 'Mill Road Allotments', owner: ME, members: [ME], folds: true,
    view: { display_name: 'Mill Road Allotments', affiliations: [{ peer: EVENT, rel: 'created' }], parts: [] } };
  const event = { id: EVENT, kind: 'event', name: 'Dig day', owner: ME, members: [ME], folds: true, view: { title: 'Dig day', start_ms: 1, roles: {}, acts: [] } };
  const graph = { me: { pk: 'ed25519:' + ME }, objects: [site, event], spine: [] };
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
    atob, TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, console,
    setTimeout: (f) => setImmediate(f), clearTimeout() {}
  };
  ctx.window = ctx;
  ctx.WallFlowers = { ...modules };
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  return { $, calls };
}

const settle = async () => { for (let i = 0; i < 40; i++) await new Promise((r) => setImmediate(r)); };
function find(n, pred) {
  if (pred(n)) return n;
  for (const c of n.children || []) { const f = find(c, pred); if (f) return f; }
  return null;
}
/* A module that records each draw: what it was handed. */
function recorder() {
  const drawn = [];
  return { drawn, draw(f, x, ctx) { drawn.push({ f, x, ctx }); } };
}
async function newEvent(p) {
  await settle();
  find(p.$('tabs'), (n) => n.tag === 'button' && n.textContent === 'Events').onclick();
  await settle();
  const add = find(p.$('lbody'), (n) => n.tag === 'button' && n.textContent === 'New event');
  assert.ok(add, 'the Events section offers a new one');
  add.onclick();
  await settle();
}

test('New event opens the editor on a new event, with eventsCtx, not the sheet', async () => {
  const EventsEdit = recorder();
  const p = page({ EventsEdit });
  await newEvent(p);
  assert.equal(EventsEdit.drawn.length, 1, 'the editor drawn once');
  const [{ f, x, ctx }] = EventsEdit.drawn;
  assert.equal(x, null, 'a new event: no id');
  assert.equal(f, p.$('feed'), 'in the pane');
  assert.deepEqual(Object.keys(ctx).sort(), CTX, 'the one interface');
  assert.equal(ctx.S.site, SITE, "on this Site");
  assert.equal(ctx.icd.version, ICD.version, 'the served ICD');
  assert.equal(p.$('sheet').hidden, true, 'no sheet');
  assert.ok(!p.calls.some((c) => c.path.startsWith('/v2/draft/')), 'the sheet asked nothing of the Door');
});

test('Manage: the editor on an event, by its id', async () => {
  const EventsEdit = recorder();
  const p = page({ EventsEdit });
  await newEvent(p);
  EventsEdit.drawn[0].ctx.open('event-edit', EVENT);
  await settle();
  assert.equal(EventsEdit.drawn.at(-1).x, EVENT);
});

test('an event opens on its page, handed the event and eventsCtx', async () => {
  const EventsPage = recorder();
  // The editor, as Publish ends it: the new event opened as an object.
  const p = page({ EventsPage, EventsEdit: { draw(f, x, ctx) { if (!EventsPage.drawn.length) ctx.open('object', EVENT); } } });
  await newEvent(p);
  await settle();
  const last = EventsPage.drawn.at(-1);
  assert.ok(last, 'the page drawn');
  assert.equal(last.x.id, EVENT, 'the event, as the graph holds it');
  assert.equal(last.f, p.$('feed'), 'in the pane');
  assert.ok(p.$('pttl').textContent.includes('Dig day'), "the pane's title is the event's");
  assert.deepEqual(Object.keys(last.ctx).sort(), CTX);
});

test('without the modules the page is as it was: the sheet', async () => {
  const p = page();
  await newEvent(p);
  assert.equal(p.$('sheet').hidden, false, 'the sheet opens');
});
