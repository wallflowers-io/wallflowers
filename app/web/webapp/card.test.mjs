/* card.test.mjs — J-A: a claim's visitor joins, makes their card, and is handed to the Site's
   own site (Ralph, 29 Sep).

   On a join the page asks /v2/me. A person with a name goes on; one without is shown Your
   card, which cannot be dismissed. An empty name is refused before anything is written. A
   name is written once, as group.setProfile on the person's own record (spine 0). Save
   waits for that record on a fresh device, and a first write met by the record settling is
   tried once more. Then the page goes to /v2/join's `home`, only an http(s) address, and
   only once the join has settled, its one retry included (a site's session cannot join).
   No home, and the person stays. #you (a Site's own page, egregores-echoes.com's profile
   icon) opens the same card to change it: dismissable, the card as it stands, a picture
   not changed not sent (core keeps the one held), and no handoff. The card is the
   webapp's Your card (#you: #youName, #youGo, #youWhy, #youX). webapp.js runs as the page
   runs it, over a stub page and a stub Door, whose elements start hidden as index.html has
   them; waits of a second or more are held until the test lets them run.

   Run: node --test app/web/webapp/card.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
/* The elements index.html starts hidden: the page's own starting state, not a list kept here. */
const HIDDEN = [...readFileSync(new URL('./index.html', import.meta.url), 'utf8').matchAll(/<[a-z0-9]+\s[^>]*\bid="([^"]+)"[^>]*\shidden[\s>]/g)].map((m) => m[1]);
const ORIGIN = 'https://app.wallflowers.io';
const HOME = 'https://egregores-echoes.com/community';
const SELF = 'f'.repeat(64);

/* A page arrived by a claim (#join). `name` is /v2/me's display_name; `joins` the Door's
   answers to /v2/join, in turn; `self` whether the graph holds the person's own record yet;
   `refuse` how many writes the Door refuses before it takes one. */
function page({ name = '', joins = [{ site: 'a'.repeat(64), home: HOME }], self = true, refuse = 0, hash = '#join', card = null } = {}) {
  const state = { self, refuse, joins: joins.slice(), joined: 0 };
  class Node {
    constructor(tag) {
      Object.assign(this, { tag, children: [], _text: '', hidden: false, disabled: false, value: '', className: '', attrs: {}, scrollTop: 0 });
      this.style = { setProperty() {}, removeProperty() {} };
      this.classList = { add() {}, remove() {}, toggle() {}, contains() { return false; } };
    }
    get textContent() { return this._text + this.children.map((c) => c.textContent).join(''); }
    set textContent(v) { this._text = String(v); this.children = []; }
    set innerHTML(v) { this._html = String(v); }
    get innerHTML() { return this._html || ''; }
    setAttribute(k, v) { this.attrs[k] = String(v); }
    getAttribute(k) { return this.attrs[k] ?? null; }
    removeAttribute(k) { delete this.attrs[k]; }
    appendChild(c) { this.children.push(c); return c; }
    addEventListener(t, f) { this['on' + t] = f; }
    focus() {}
    querySelector() { return null; }
    /* The layout a browser always has; the page measures its top bar and rails. */
    getBoundingClientRect() { return { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 }; }
    querySelectorAll() { return []; }
    closest() { return null; }
    remove() {}
    get firstChild() { return this.children[0] || (this._html ? (this._parsed ||= new Node('svg')) : null); }
    get lastChild() { return this.children.at(-1) || null; }
    insertBefore(c, ref) { const i = this.children.indexOf(ref); this.children.splice(i < 0 ? this.children.length : i, 0, c); return c; }
  }
  const byId = {};
  const $ = (id) => (byId[id] ||= new Node('#' + id));
  for (const id of HIDDEN) $(id).hidden = true;
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const writes = [];
  const fetch = async (url, opts = {}) => {
    const path = url.slice(ORIGIN.length);
    if (path === '/v2/me') return reply(200, { pk: 'ed25519:' + 'c'.repeat(64), ...(name ? { display_name: name } : {}) });
    if (path === '/v2/graph') {
      return reply(200, state.self
        ? { me: { pk: 'ed25519:' + 'c'.repeat(64), display_name: name }, objects: [{ id: SELF, kind: 'group', folds: true, view: { shape: 'individual', card } }], spine: [{ index: 0, object: SELF }] }
        : { objects: [], spine: [] });
    }
    if (path === '/v2/icd') return reply(200, { kinds: {}, facets: {} });
    if (path === '/v2/bundle') return reply(200, { bundles: ['k0', 'k1'] });
    if (path === '/v2/join') state.joined++;
    if (path === '/v2/join') return reply(200, state.joins.length > 1 ? state.joins.shift() : state.joins[0]);
    if (path === '/v2/apply') {
      writes.push(JSON.parse(opts.body));
      if (state.refuse > 0) { state.refuse--; return reply(409, 'the record is still settling'); }
      return reply(200, {});
    }
    return reply(200, {});
  };
  class EventSource { constructor() { this.readyState = 1; } addEventListener() {} close() {} }
  const held = [];
  const keys = [];
  const html = new Node('html');
  const sent = [];
  const loc = { hash, pathname: '/', search: '', origin: ORIGIN, assign(u) { sent.push(u); } };
  const ctx = {
    document: {
      getElementById: $, documentElement: html, createElement: (t) => new Node(t), createTextNode: (t) => ({ textContent: String(t) }),
      querySelectorAll: () => [],
      addEventListener(t, f) { if (t === 'keydown') keys.push(f); }
    },
    location: loc, history: { replaceState(s, t, u) { loc.hash = u.includes('#') ? u.slice(u.indexOf('#')) : ''; } }, fetch, EventSource, doorOrigin: () => ORIGIN,
    addEventListener() {}, scrollTo() {}, URL,
    innerWidth: 390, matchMedia: () => ({ matches: false, addEventListener() {} }), requestAnimationFrame: (f) => setImmediate(f),
    btoa,
    atob, TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, console,
    setTimeout: (f, ms) => (ms >= 1000 ? held.push(f) : setImmediate(f)), clearTimeout() {}
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  /* Let every held wait run once. */
  const tick = () => held.splice(0).forEach((f) => f());
  /* Your card's own controls, as drawn into #youBody. */
  const find = (n, pred) => (pred(n) ? n : (n.children || []).map((c) => find(c, pred)).find(Boolean));
  const byAttr = (id) => find($('youBody'), (n) => n.attrs && n.attrs.id === id);
  const go = () => byAttr('youGo');
  const title = () => (find($('youBody'), (n) => n.tag === 'h3') || {}).textContent;
  const nameShown = () => byAttr('youName').attrs.value;
  const save = (value) => { byAttr('youName').value = value; go().onclick(); };
  return { state, writes, sent, keys, $, loc, tick, save, go, title, nameShown };
}

const settle = async () => { for (let i = 0; i < 60; i++) await new Promise((r) => setImmediate(r)); };

test('a person with a name is handed to the site, with no card asked for', async () => {
  const p = page({ name: 'Ana' });
  await settle();
  assert.deepEqual(p.sent, [HOME]);
  assert.equal(p.$('you').hidden, true);
  assert.deepEqual(p.writes, []);
});

test('no name: Your card, which cannot be dismissed', async () => {
  const p = page();
  await settle();
  assert.equal(p.$('you').hidden, false);
  assert.equal(p.title(), 'Your card');
  assert.equal(p.$('youX').hidden, true, 'no close button');
  p.$('you').onclick({ target: { id: 'you' } });
  p.keys.forEach((f) => f({ key: 'Escape' }));
  assert.equal(p.$('you').hidden, false, 'neither the backdrop nor Escape closes it');
  assert.deepEqual(p.sent, [], 'and nobody is sent on');
});

test('an empty name is refused, and nothing is written', async () => {
  const p = page();
  await settle();
  p.save('   ');
  await settle();
  assert.deepEqual(p.writes, []);
  assert.match(p.$('youWhy').textContent, /A name, please/);
  assert.equal(p.$('you').hidden, false);
  assert.deepEqual(p.sent, []);
});

test('a name is written once, on the person\'s own record, then the handoff', async () => {
  const p = page();
  await settle();
  p.save('  Ana  ');
  await settle();
  assert.equal(p.writes.length, 1);
  const w = p.writes[0];
  assert.equal(w.object, SELF);
  assert.equal(w.op, 'group.setProfile');
  assert.deepEqual(w.args, { displayName: 'Ana', shape: 'individual' }, 'no picture chosen, no card sent');
  assert.equal(p.$('you').hidden, true);
  assert.deepEqual(p.sent, [HOME]);
});

test('on a fresh device, Save waits for the person\'s own record', async () => {
  const p = page({ self: false });
  await settle();
  assert.equal(p.go().disabled, true);
  assert.equal(p.go().textContent, 'One moment…');
  p.save('Ana');
  await settle();
  assert.deepEqual(p.writes, [], 'a press while it waits writes nothing');
  p.state.self = true;
  p.tick();
  await settle();
  assert.equal(p.go().disabled, false);
  assert.equal(p.go().textContent, 'Save');
  p.save('Ana');
  await settle();
  assert.equal(p.writes.length, 1);
  assert.equal(p.writes[0].object, SELF);
  assert.deepEqual(p.sent, [HOME]);
});

test('a write met by the record settling is tried once more; a second refusal is shown', async () => {
  const once = page({ refuse: 1 });
  await settle();
  once.save('Ana');
  await settle();
  assert.equal(once.writes.length, 1);
  assert.deepEqual(once.sent, [], 'not yet');
  once.tick();
  await settle();
  assert.equal(once.writes.length, 2);
  assert.deepEqual(once.sent, [HOME]);

  const twice = page({ refuse: 2 });
  await settle();
  twice.save('Ana');
  await settle();
  twice.tick();
  await settle();
  assert.equal(twice.writes.length, 2, 'once more, not a loop');
  assert.match(twice.$('youWhy').textContent, /still settling/);
  assert.equal(twice.$('you').hidden, false);
  assert.equal(twice.go().disabled, false, 'Save can be pressed again');
  assert.deepEqual(twice.sent, []);
});

test('home is only an http(s) address, and none means the person stays', async () => {
  for (const home of [undefined, '', 'javascript:alert(1)', 'data:text/html,x', 'not a url']) {
    const p = page({ name: 'Ana', joins: [{ site: 'a'.repeat(64), home }] });
    await settle();
    assert.deepEqual(p.sent, [], String(home));
  }
});

test('the handoff waits for the join to settle, its one retry included', async () => {
  for (const name of ['Ana', '']) {
    const p = page({ name, joins: [{ site: 'a'.repeat(64), home: HOME, unjoined: [{ room: 'Chat', why: 'not yet' }] }, { site: 'a'.repeat(64), home: HOME }] });
    await settle();
    if (!name) { p.save('Ana'); await settle(); }
    assert.deepEqual(p.sent, [], `${name || 'a new card'}: not before the retry`);
    p.tick();
    await settle();
    assert.deepEqual(p.sent, [HOME], name || 'a new card');
  }
});

test('#you opens the card as it stands, and it can be closed', async () => {
  const p = page({ name: 'Ana', hash: '#you' });
  await settle();
  assert.equal(p.$('you').hidden, false);
  assert.equal(p.title(), 'Your card');
  assert.equal(p.nameShown(), 'Ana', 'the name as it stands');
  assert.equal(p.$('youX').hidden, false, 'a close button');
  assert.equal(p.loc.hash, '', 'a reload does not open it again');
  p.$('you').onclick({ target: { id: 'you' } });
  assert.equal(p.$('you').hidden, true, 'the backdrop closes it');
  const esc = page({ name: 'Ana', hash: '#you' });
  await settle();
  esc.keys.forEach((f) => f({ key: 'Escape' }));
  assert.equal(esc.$('you').hidden, true, 'and so does Escape');
  assert.deepEqual([...p.writes, ...esc.writes], [], 'closed, nothing written');
});

test('#you: a new name alone keeps the picture, and nobody is sent anywhere', async () => {
  const p = page({ name: 'Ana', hash: '#you', card: { photo: 'c3RpbGw=', photo_mime: 'image/webp' } });
  await settle();
  p.save('Ana Kim');
  await settle();
  assert.equal(p.writes.length, 1);
  const w = p.writes[0];
  assert.deepEqual([w.object, w.op, w.args.displayName], [SELF, 'group.setProfile', 'Ana Kim']);
  assert.ok(!('card' in w.args), 'no card sent: core keeps the picture held');
  assert.equal(p.$('you').hidden, true);
  assert.deepEqual(p.sent, [], 'no handoff from #you');
  assert.equal(p.state.joined, 0, 'and no join was asked for');
});

/* YOUR CONTACT CODE (W-96): on the card from the profile menu or #you, not on the first. */
test('Your card makes a contact code from the Door\'s keys; the first card has none', async () => {
  const p = page({ name: 'Ana', hash: '#you' });
  await settle();
  const find = (n, pred) => (pred(n) ? n : (n.children || []).map((x) => find(x, pred)).find(Boolean));
  const go = find(p.$('youBody'), (n) => n.attrs && n.attrs.id === 'codeGo');
  assert.ok(go, 'a contact code offered');
  go.onclick();
  await settle();
  const out = find(p.$('youBody'), (n) => n.attrs && n.attrs.id === 'codeOut');
  assert.equal(out.hidden, false);
  assert.ok(out.value.startsWith('wf1.'));
  assert.deepEqual(JSON.parse(Buffer.from(out.value.slice(4), 'base64url').toString()), ['k0', 'k1']);
  const first = page();
  await settle();
  assert.ok(!find(first.$('youBody'), (n) => n.attrs && n.attrs.id === 'codeGo'), 'not on the first card');
});
