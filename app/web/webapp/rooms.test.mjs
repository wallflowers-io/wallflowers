/* rooms.test.mjs — W-98 Rooms: a member takes back their own message in a room; a room's
   description, shown to its members and edited by its owner or an admin of the room.

   The op is the ICD's forum.retract, its args read from the ICD; `gen` is the Door's to add, as
   for every commutative op. The fold lets a message's author take it back (coordinator.rs
   FORUM_RETRACT), so the page offers it on the member's own messages, in the message's menu,
   and asks once before it writes: a retracted message stays retracted. webapp.js runs as the
   page runs it, over a stub page, the real ICD and a stub Door.

   The description is forum.editDescription's, its args read from the ICD. Who may edit is the
   ICD's ego, "owner|role:admin", asked of THIS room: its owner, or its view's `roles`. The
   description's UI is rooms.js, drawn through one hook in the room's welcome; Make admin and
   Remove admin set and clear the role on the Site's rooms too, where the person is a member.

   Run: node --test app/web/webapp/rooms.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const ROOMS = readFileSync(new URL('./rooms.js', import.meta.url), 'utf8');
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));
const ORIGIN = 'https://app.wallflowers.io';
const ME = 'c'.repeat(64), OWNER = 'b'.repeat(64), SITE = 'a'.repeat(64), ROOM = 'e'.repeat(64), HOST = 'f'.repeat(64);
const RETRACT = ICD.kinds.forum.ops['forum.retract'];
const DESCRIBE = ICD.kinds.forum.ops['forum.editDescription'];
/* The args a site sends: the ICD's required ones, less the gen the Door adds to a commutative op. */
const sent = (op) => Object.keys(op.args).filter((a) => op.args[a].required && !(a === 'gen' && op.fold === 'commutative')).sort();

/* `opts`: who owns the Site and the room, the room's roles and description, the Site's roles. */
function page(answers = {}, opts = {}) {
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
    getBoundingClientRect() { return { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 }; }
    querySelectorAll() { return []; }
    closest() { return null; }
    remove() {}
    cloneNode() { return new Node(this.tag); }
    scrollIntoView() {}
    get firstChild() { return this.children[0] || (this._html ? (this._parsed ||= new Node('svg')) : null); }
    get lastChild() { return this.children.at(-1) || null; }
    insertBefore(c, ref) { const i = this.children.indexOf(ref); this.children.splice(i < 0 ? this.children.length : i, 0, c); return c; }
    querySelector() { return null; }
  }
  const byId = {};
  const $ = (id) => (byId[id] ||= new Node('#' + id));
  for (const id of ['sheet', 'side', 'refused', 'status', 'compose', 'mmenu', 'emo']) $(id).hidden = true;
  const calls = [];
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const site = { id: SITE, kind: 'group', name: 'Mill Road Allotments', owner: opts.siteOwner || OWNER, members: [OWNER, ME], folds: true,
    view: { display_name: 'Mill Road Allotments', affiliations: [], roles: opts.siteRoles || [],
      parts: [{ part: ROOM, role: 'room' }, { part: HOST, role: 'host' }] } };
  const host = { id: HOST, kind: 'host', name: 'page', owner: opts.siteOwner || OWNER, members: [OWNER, ME], folds: true, view: { roles: opts.siteRoles || [] } };
  const msg = (author, gen, text) => ({ author, gen, text, ts: 1759200000000 + gen, reply_to: null, up: 0, down: 0, reactions: [] });
  const room = { id: ROOM, kind: 'forum', name: 'general', owner: opts.roomOwner || OWNER, members: [OWNER, ME], folds: true,
    view: { messages: [msg(ME, 3, 'mine'), msg(OWNER, 4, 'theirs')], parts: [], parent: null,
      description: opts.description ?? null, roles: opts.roomRoles || [] } };
  const graph = { me: { pk: 'ed25519:' + ME }, objects: [site, host, room], spine: [] };
  const fetch = async (url, init = {}) => {
    const path = url.slice(ORIGIN.length), body = init.body ? JSON.parse(init.body) : null;
    calls.push({ path, body });
    const a = answers[path];
    if (typeof a === 'function') { const r = a(body); if (r) return reply(r[0], r[1]); }
    if (path === '/v2/me') return reply(200, { pk: 'ed25519:' + ME, display_name: 'Me' });
    if (path === '/v2/graph') return reply(200, graph);
    if (path === '/v2/icd') return reply(200, ICD);
    return reply(200, {});
  };
  const loc = { hash: '#site=' + SITE, pathname: '/', search: '', origin: ORIGIN, assign() {} };
  const ctx = {
    document: { getElementById: $, documentElement: new Node('html'), createElement: (t) => new Node(t),
      createTextNode: (t) => ({ textContent: String(t) }), addEventListener() {}, querySelectorAll: () => [] },
    location: loc, history: { replaceState() {} }, fetch, doorOrigin: () => ORIGIN,
    addEventListener() {}, scrollTo() {},
    innerWidth: 1280, innerHeight: 800, matchMedia: () => ({ matches: false, addEventListener() {} }), requestAnimationFrame: (f) => setImmediate(f),
    atob, TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, console,
    setTimeout: (f) => setImmediate(f), clearTimeout() {}
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(ROOMS, ctx);
  vm.runInContext(SRC, ctx);
  return { $, calls };
}

const settle = async () => { for (let i = 0; i < 40; i++) await new Promise((r) => setImmediate(r)); };
function find(n, pred) {
  if (pred(n)) return n;
  for (const c of n.children || []) { const f = find(c, pred); if (f) return f; }
  return null;
}
const button = (n, text) => find(n, (x) => x.tag === 'button' && x.textContent === text);

/* The room, as a member opens it: the Rooms tab opens on the room spoken in last. */
async function openRoom(p) {
  await settle();
  const tab = button(p.$('tabs'), 'Rooms');
  assert.ok(tab, 'the Rooms tab');
  tab.onclick();
  await settle();
}
/* The room, and a message's menu, as a right-click opens it. */
async function menuOf(p, author, gen) {
  await openRoom(p);
  const row = find(p.$('feed'), (n) => n.attrs && n.attrs.id === 'm-' + author.slice(0, 16) + '-' + gen);
  assert.ok(row, 'the message');
  row.oncontextmenu({ target: { closest: () => null }, preventDefault() {} });
  assert.equal(p.$('mmenu').hidden, false, "the message's menu");
  return p.$('mmenu');
}
const retracts = (p) => p.calls.filter((c) => c.path === '/v2/apply' && c.body.op === 'forum.retract');

test('your own message: Delete, asked once, writes forum.retract with the ICD\'s args', async () => {
  const p = page();
  const menu = await menuOf(p, ME, 3);
  const del = button(menu, 'Delete');
  assert.ok(del, 'your own message offers Delete');
  del.onclick();
  await settle();
  assert.equal(retracts(p).length, 0, 'asked before it writes: a retracted message stays retracted');
  const sure = button(p.$('mmenu'), 'Delete for everyone');
  assert.ok(sure, 'asked once');
  sure.onclick();
  await settle();
  const [w] = retracts(p);
  assert.ok(w, 'forum.retract written');
  assert.equal(w.body.object, ROOM);
  const sent = Object.keys(RETRACT.args).filter((a) => RETRACT.args[a].required && !(a === 'gen' && RETRACT.fold === 'commutative'));
  assert.deepEqual(Object.keys(w.body.args).sort(), sent.sort(), "the ICD's args, gen the Door's");
  assert.deepEqual([w.body.args.target_author, w.body.args.target_gen], [ME, 3]);
  assert.ok(p.calls.slice(p.calls.indexOf(w)).some((c) => c.path === '/v2/graph'), 'the room is read again');
});

test("someone else's message offers no Delete", async () => {
  const p = page();
  const menu = await menuOf(p, OWNER, 4);
  assert.ok(button(menu, 'Reply'), 'the menu is there');
  assert.equal(button(menu, 'Delete'), null);
});

test("a refusal is said, in the Door's words", async () => {
  const p = page({ '/v2/apply': (b) => (b.op === 'forum.retract' ? [400, 'coordinator: op 6 was refused by the reducer: Unauthorized'] : null) });
  const del = button(await menuOf(p, ME, 3), 'Delete');
  assert.ok(del, 'your own message offers Delete');
  del.onclick();
  await settle();
  const sure = button(p.$('mmenu'), 'Delete for everyone');
  assert.ok(sure, 'asked once');
  sure.onclick();
  await settle();
  assert.equal(p.$('refused').hidden, false);
  assert.equal(p.$('refused').textContent, 'coordinator: op 6 was refused by the reducer: Unauthorized');
});

/* ── THE ROOM'S DESCRIPTION ─────────────────────────────────────────────────────────────── */

const describes = (p) => p.calls.filter((c) => c.path === '/v2/apply' && c.body.op === 'forum.editDescription');
const labelled = (n, label) => find(n, (x) => x.tag === 'button' && x.attrs && x.attrs['aria-label'] === label);
const shows = (p, text) => !!find(p.$('feed'), (n) => n._text === text);

test('the room shows its description to a member, who may not edit it', async () => {
  const p = page({}, { description: 'Seed swaps on Saturdays' });
  await openRoom(p);
  assert.ok(shows(p, 'Seed swaps on Saturdays'), 'shown in the room');
  assert.equal(labelled(p.$('feed'), 'Edit description'), null, 'a member is not the owner or an admin of the room');
});

for (const [who, opts] of [['its owner', { roomOwner: ME }], ['an admin of the room', { roomRoles: [[ME, 'admin']] }]]) {
  test(`${who} edits it: forum.editDescription with the ICD's args`, async () => {
    const p = page({}, { ...opts, description: 'Seed swaps on Saturdays' });
    await openRoom(p);
    const edit = labelled(p.$('feed'), 'Edit description');
    assert.ok(edit, `${who} is offered the edit`);
    edit.onclick();
    await settle();
    const box = find(p.$('feed'), (n) => n.tag === 'textarea');
    assert.ok(box, 'the description, to edit');
    assert.equal(box.value, 'Seed swaps on Saturdays', 'starting from what it says');
    box.value = 'Seed swaps on Sundays';
    button(p.$('feed'), 'Save').onclick();
    await settle();
    const [w] = describes(p);
    assert.ok(w, 'forum.editDescription written');
    assert.equal(w.body.object, ROOM);
    assert.deepEqual(Object.keys(w.body.args).sort(), sent(DESCRIBE), "the ICD's args, gen the Door's");
    assert.equal(w.body.args.description, 'Seed swaps on Sundays');
    assert.ok(p.calls.slice(p.calls.indexOf(w)).some((c) => c.path === '/v2/graph'), 'the room is read again');
  });
}

test('a room with none: its owner adds one; Cancel writes nothing', async () => {
  const p = page({}, { roomOwner: ME });
  await openRoom(p);
  const edit = labelled(p.$('feed'), 'Edit description');
  assert.ok(edit, 'its owner is offered the edit');
  edit.onclick();
  await settle();
  assert.equal(find(p.$('feed'), (n) => n.tag === 'textarea').value, '');
  button(p.$('feed'), 'Cancel').onclick();
  await settle();
  assert.equal(describes(p).length, 0);
});

test("a refused description is said, in the Door's words", async () => {
  const p = page({ '/v2/apply': (b) => (b.op === 'forum.editDescription' ? [400, 'coordinator: op 7 would not fold: MalformedArgs'] : null) }, { roomOwner: ME });
  await openRoom(p);
  const edit = labelled(p.$('feed'), 'Edit description');
  assert.ok(edit, 'its owner is offered the edit');
  edit.onclick();
  await settle();
  find(p.$('feed'), (n) => n.tag === 'textarea').value = 'x';
  button(p.$('feed'), 'Save').onclick();
  await settle();
  assert.equal(p.$('refused').hidden, false);
  assert.equal(p.$('refused').textContent, 'coordinator: op 7 would not fold: MalformedArgs');
});

/* ── MAKE ADMIN COVERS THE SITE'S ROOMS ─────────────────────────────────────────────────── */

const roles = (p, op) => p.calls.filter((c) => c.path === '/v2/apply' && c.body.op === op).map((c) => c.body.object);

/* The person card of the room's other member, as its owner opens it from a message. */
async function cardOf(p) {
  await openRoom(p);
  const row = find(p.$('feed'), (n) => n.attrs && n.attrs.id === 'm-' + OWNER.slice(0, 16) + '-4');
  find(row, (n) => n.tag === 'button' && n.className === 'who').onclick();
  await settle();
  return p.$('personBody');
}

test("Make admin gives the role on the Site's rooms too", async () => {
  const p = page({}, { siteOwner: ME, roomOwner: ME });
  const make = button(await cardOf(p), 'Make admin');
  assert.ok(make, 'the Site owner is offered Make admin');
  make.onclick({ currentTarget: make });
  await settle();
  assert.deepEqual(roles(p, 'base.setRole').sort(), [HOST, ROOM, SITE].sort(), 'the Site, its page, and its room');
  const room = p.calls.find((c) => c.body && c.body.op === 'base.setRole' && c.body.object === ROOM);
  assert.deepEqual([room.body.args.member, room.body.args.role], [OWNER, 'admin']);
});

test("Remove admin takes it back on the Site's rooms too", async () => {
  const p = page({}, { siteOwner: ME, roomOwner: ME, siteRoles: [[OWNER, 'admin']], roomRoles: [[OWNER, 'admin']] });
  const remove = button(await cardOf(p), 'Remove admin');
  assert.ok(remove, 'the Site owner is offered Remove admin');
  remove.onclick({ currentTarget: remove });
  await settle();
  assert.deepEqual(roles(p, 'base.clearRole').sort(), [HOST, ROOM, SITE].sort(), 'the Site, its page, and its room');
});
