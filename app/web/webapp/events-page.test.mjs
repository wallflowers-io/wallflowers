/* events-page.test.mjs — W-98 Events, the event's page (EVENTS-UI-B, W-99): events-page.js,
   drawn by webapp.js's drawObject hook with eventsCtx(), reading the ICD through events-icd.js.

   The cover, the title and its status badge, when (in the event's own zone) and where, the
   host (the owner: a Site's events are `created`, so the roster holds only its creator), the
   RSVP bar on the Site that created the event (group.rsvp, its args read from the ICD, no gen:
   the Door's), the tallies (going and maybe with faces, declined a count only), your own answer
   Pending while approval holds it, the lineup (an unconfirmed act shown, never dropped; Confirm
   for its performer alone, writing their half of performs_at on their own record), the media,
   the wall, and Manage where the op's ego holds. An op the served ICD does not declare is not
   drawn. A refusal is shown as the Door words it. webapp.js runs as the page runs it, over a
   stub page, the real ICD and a stub Door.

   Run: node --test app/web/webapp/events-page.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const EICD = readFileSync(new URL('./events-icd.js', import.meta.url), 'utf8');
const EPAGE = readFileSync(new URL('./events-page.js', import.meta.url), 'utf8');
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));
const ORIGIN = 'https://app.wallflowers.io';
const ME = 'c'.repeat(64), OWNER = 'b'.repeat(64), SITE = 'a'.repeat(64), OTHER = '8'.repeat(64), EVENT = 'e'.repeat(64),
  SELF = '5'.repeat(64), BAND = '6'.repeat(64), THEIRS = '9'.repeat(64), FORUM = 'f'.repeat(64), POST = 'd'.repeat(64),
  GITA = '1'.repeat(64), MO = '2'.repeat(64), DEE = '3'.repeat(64), PIA = '4'.repeat(64), NOBODY = '7'.repeat(64), UNLISTED = '0'.repeat(64);
const START = Date.UTC(2026, 9, 3, 18, 0), END = Date.UTC(2026, 9, 3, 21, 0), TZ = 'America/New_York';
const GR = ICD.kinds.group.ops, EV = ICD.kinds.event.ops;
const RSVP = GR['group.rsvp'], AFFIL = GR['group.setAffiliation'];
const [RK, CK] = Object.keys(RSVP.view), [GK] = Object.keys(GR['group.setRegistration'].view);
const hhmm = (ms, timeZone) => new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit', ...(timeZone ? { timeZone } : {}) }).format(ms);

/* The graph: the Site that created the event, a Site that did not, the viewer's own record
   (spine index 0), a band the viewer owns and one they do not, the event, a post, and the
   event's wall when asked. */
function world(o = {}) {
  const owner = o.owner || OWNER;
  const event = { id: EVENT, kind: 'event', name: 'Dig day', owner, members: [owner], folds: true,
    view: { title: 'Dig day', descriptor: 'Bring gloves', start_ms: START, end_ms: END, tz: TZ, venue: 'Plot 9', status: 'scheduled',
      roles: {}, acts: [], banner: null, clip: null, photos: [], lineup: [], recurrence: '', ticket_url: null, video_url: null,
      online: null, all_day: 0, ...o.event } };
  const site = { id: SITE, kind: 'group', name: 'Mill Road Allotments', owner: OWNER, members: [OWNER, ME, GITA, MO, DEE, PIA], folds: true,
    view: { display_name: 'Mill Road Allotments', affiliations: o.uncreated ? [] : [{ peer: EVENT, rel: 'created', name: 'Dig day', tether: null, at: 1 },
      ...(o.unheld ? [{ peer: UNLISTED, rel: 'created', name: 'Unlisted', tether: null, at: 2 }] : [])],
      parts: [], roles: [],
      profiles: [[OWNER, { displayName: 'Olu' }], [GITA, { displayName: 'Gita' }], [MO, { displayName: 'Mo' }], [DEE, { displayName: 'Dee' }], [PIA, { displayName: 'Pia' }]],
      [RK]: { [EVENT]: o.rsvps || {} }, [CK]: { [EVENT]: o.counts || { going: 0, maybe: 0, declined: 0, guests: 0 } },
      ...(o.registration ? { [GK]: { [EVENT]: o.registration } } : {}) } };
  const group = (id, name, gowner, affiliations = []) => ({ id, kind: 'group', name, owner: gowner, members: [gowner], folds: true,
    view: { display_name: name, affiliations, parts: [], roles: [] } });
  const objects = [site, group(OTHER, 'Canal Club', ME), group(SELF, 'Cal', ME, o.selfAffiliations), group(BAND, 'The Diggers', ME),
    group(THEIRS, 'Hoe Down', PIA), ...(o.unheld ? [] : [event]),
    { id: POST, kind: 'post', name: 'Seed swap', owner: OWNER, members: [OWNER], folds: true, view: { title: 'Seed swap', body: 'Bring seeds' } }];
  if (o.wall) objects.push({ id: FORUM, kind: 'forum', name: 'comments', owner, members: o.wallMembers || [owner, ME, GITA], folds: true,
    view: { messages: o.wall, parts: [], parent: { parent: EVENT, role: 'comments', at: 1 } } });
  return { me: { pk: 'ed25519:' + ME, display_name: 'Cal' }, objects, spine: o.noSelf ? [] : [{ index: 0, object: SELF, kind: 'group' }] };
}

/* The page, over world(o). `drawn` records each draw the hook hands the event page: the
   object and eventsCtx(). */
function page(o = {}, answers = {}, modules = {}) {
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
    get childNodes() { return this.children; }
    insertBefore(c, ref) { const i = this.children.indexOf(ref); this.children.splice(i < 0 ? this.children.length : i, 0, c); return c; }
    querySelector() { return null; }
  }
  const byId = {};
  const $ = (id) => (byId[id] ||= new Node('#' + id));
  for (const id of ['sheet', 'side', 'refused', 'status', 'compose']) $(id).hidden = true;
  const calls = [];
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const graph = world(o);
  const fetch = async (url, init = {}) => {
    const path = url.slice(ORIGIN.length), body = init.body ? JSON.parse(init.body) : null;
    calls.push({ path, body });
    const a = answers[path];
    if (typeof a === 'function') { const r = a(body); if (r) return reply(r[0], r[1]); }
    if (path === '/v2/me') return reply(200, { pk: 'ed25519:' + ME, display_name: 'Cal' });
    if (path === '/v2/graph') return reply(200, graph);
    if (path === '/v2/icd') return reply(200, o.icd || ICD);
    if (path === `/v2/site/${SITE}/items`) return reply(200, { slug: 'mill-road', items: o.items || {} });
    return reply(200, {});
  };
  // With no Site that created the event, it is in no Site's scope: opened from Everything.
  const loc = { hash: o.uncreated ? '' : '#site=' + SITE, pathname: '/', search: '', origin: ORIGIN, assign() {} };
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
  vm.runInContext(EICD, ctx);
  vm.runInContext(EPAGE, ctx);
  const drawn = [], EP = ctx.WallFlowers.EventsPage;
  if (EP && typeof EP.draw === 'function') {
    const real = EP.draw;
    EP.draw = function (f, x, c) { drawn.push({ x, ctx: c }); return real.call(this, f, x, c); };
  }
  vm.runInContext(SRC, ctx);
  return { $, calls, win: ctx, drawn, EP, blank: () => new Node('div') };
}

const settle = async () => { for (let i = 0; i < 40; i++) await new Promise((r) => setImmediate(r)); };
function find(n, pred) {
  if (pred(n)) return n;
  for (const c of n.children || []) { const f = find(c, pred); if (f) return f; }
  return null;
}
function all(n, pred, out = []) {
  if (pred(n)) out.push(n);
  for (const c of n.children || []) all(c, pred, out);
  return out;
}
const cls = (n, c) => typeof n.className === 'string' && n.className.split(/\s+/).includes(c);
const button = (n, text) => find(n, (x) => x.tag === 'button' && x.textContent === text);
const buttons = (n, text) => all(n, (x) => x.tag === 'button' && x.textContent === text);
/* Every word a person can read on a node: its text, its attributes, its markup. */
const everywhere = (n) => all(n, () => true).map((x) => [x.textContent || '', ...Object.values(x.attrs || {}), x._html || ''].join(' ')).join(' ');
const applies = (p, op) => p.calls.filter((c) => c.path === '/v2/apply' && (!op || c.body.op === op));
/* Every arg sent is one the ICD's op declares; the gen is the Door's to add. */
function declared(args, op) {
  for (const k of Object.keys(args)) assert.ok(k in op.args, k + ' is an arg the ICD declares');
  assert.ok(!('gen' in args), 'no gen: the Door adds it');
}

/* The event, as a member opens it: its card, in the Site's feed. */
async function openEvent(p) {
  await settle();
  assert.ok(p.EP, 'WallFlowers.EventsPage is registered: the stub page draws nothing');
  const card = find(p.$('feed'), (n) => n.tag === 'button' && cls(n, 'card') && n.textContent.includes('Dig day'));
  assert.ok(card, "the event's card");
  card.onclick();
  await settle();
  return pageOf(p);
}
function pageOf(p) {
  const root = find(p.$('feed'), (n) => cls(n, 'ev'));
  assert.ok(root, "the event's page is drawn");
  return root;
}
const choice = (root, status) => find(root, (n) => n.tag === 'button' && cls(n, 'ev-choice') && n.attrs['data-status'] === status);
const tally = (root, status) => find(root, (n) => cls(n, 'ev-tally') && n.attrs['data-status'] === status);
const actsIn = (root) => all(root, (n) => cls(n, 'ev-act'));
const actOf = (root, name) => actsIn(root).find((n) => everywhere(n).includes(name));

test("the page: title, when in the event's zone, where, the owner as host", async () => {
  const p = page({ event: { roles: { [MO]: 'admin' }, recurrence: 'FREQ=WEEKLY;BYDAY=SA' } });
  const root = await openEvent(p);
  assert.equal(p.drawn.at(-1).x.id, EVENT, 'handed the event');
  const text = root.textContent;
  assert.ok(text.includes('Dig day'), 'the title');
  assert.ok(text.includes(hhmm(START, TZ)) && text.includes(hhmm(END, TZ)), "start and end in the event's zone");
  assert.ok(!text.includes(hhmm(START, 'UTC')), 'not in UTC');
  assert.ok(text.includes('Plot 9'), 'where');
  assert.ok(text.includes('Bring gloves'), 'the descriptor');
  assert.ok(!text.includes('FREQ='), 'an expressible rule is read, not shown raw');
  const sat = new Intl.DateTimeFormat(undefined, { weekday: 'short', timeZone: 'UTC' }).format(new Date(Date.UTC(2024, 0, 13)));
  assert.ok(text.includes(sat) && !/\bSA\b/.test(text), "the day as the reader's own name, not the grammar's token");
  const hosts = find(root, (n) => cls(n, 'ev-hosts'));
  assert.ok(hosts && everywhere(hosts).includes('Olu'), 'the owner hosts');
  assert.ok(!everywhere(hosts).includes('Mo'), 'an admin of the event is not drawn as a host (R3: the roster holds only its creator)');
});

test('another kind is not the event page: the hook keeps a post, and the page draws nothing for one', async () => {
  const p = page();
  await openEvent(p);
  const n = p.drawn.length;
  p.drawn.at(-1).ctx.open('object', POST);
  await settle();
  assert.equal(p.drawn.length, n, 'a post is not handed over');
  assert.ok(!find(p.$('feed'), (x) => cls(x, 'ev')), 'no event page');
  assert.ok(p.$('feed').textContent.includes('Bring seeds'), "the post, by webapp.js's own drawObject");
  const f = p.blank();
  p.EP.draw(f, { id: POST, kind: 'post', view: { title: 'Seed swap' } }, p.drawn.at(-1).ctx);
  assert.equal(f.children.length, 0, 'handed a post, it draws nothing');
});

test("an unknown or empty zone falls back to the viewer's; all day draws no times; a rule the grammar refuses is shown raw", async () => {
  for (const tz of ['', 'Nowhere/Land']) {
    const root = await openEvent(page({ event: { tz } }));
    assert.ok(root.textContent.includes(hhmm(START)), `tz ${JSON.stringify(tz)}: the viewer's zone`);
  }
  const allDay = await openEvent(page({ event: { all_day: 1 } }));
  assert.ok(!allDay.textContent.includes(hhmm(START, TZ)), 'all day: no times');
  const raw = await openEvent(page({ event: { recurrence: 'FREQ=WEEKLY;BYHOUR=9' } }));
  assert.ok(raw.textContent.includes('FREQ=WEEKLY;BYHOUR=9'), 'not expressible: the raw rule');
});

test("RSVP writes group.rsvp on the Site that created the event, the ICD's args only, no gen, guests only when going", async () => {
  const p = page({ rsvps: { [ME]: { status: 'going', guests: 0, at: 5 } } });
  let root = await openEvent(p);
  assert.ok(root.textContent.includes('Members see your answer.'));
  const words = Object.keys(RSVP.args.status.vocabulary);
  for (const s of words) assert.ok(choice(root, s), 'the choice ' + s);
  assert.deepEqual(words.map((s) => choice(root, s).textContent), ['Going', 'Maybe', "Can't go"]);

  choice(root, 'maybe').onclick();
  await settle();
  let w = applies(p).at(-1);
  assert.ok(w, 'written');
  assert.equal(w.body.object, SITE, "on the Site's log");
  assert.equal(w.body.op, 'group.rsvp');
  declared(w.body.args, RSVP);
  assert.equal(w.body.args.event, EVENT);
  assert.equal(w.body.args.status, 'maybe');
  assert.ok(w.body.args.at > 0, 'at');
  assert.ok(!('guests' in w.body.args) && !('guestNames' in w.body.args), 'no guests unless going');
  assert.ok(p.calls.slice(p.calls.indexOf(w)).some((c) => c.path === '/v2/graph'), 'read again after the write');

  root = pageOf(p);
  const plus = button(root, '+');
  assert.ok(plus, 'going: the guests stepper');
  plus.onclick();
  root = pageOf(p);
  assert.ok(root.textContent.includes('Members see these names.'));
  const names = all(root, (n) => n.tag === 'input' && cls(n, 'ev-guest'));
  assert.equal(names.length, 1, 'a name for each guest');
  names[0].value = 'Ann';
  if (names[0].oninput) names[0].oninput();
  const save = button(root, 'Save');
  assert.ok(save, 'Save');
  save.onclick();
  await settle();
  w = applies(p).at(-1);
  declared(w.body.args, RSVP);
  assert.equal(w.body.args.status, 'going');
  assert.equal(w.body.args.guests, 1);
  assert.deepEqual(JSON.parse(w.body.args.guestNames), ['Ann']);

  choice(pageOf(p), 'declined').onclick();
  await settle();
  w = applies(p).at(-1);
  declared(w.body.args, RSVP);
  assert.equal(w.body.args.status, 'declined');
  assert.ok(!('guests' in w.body.args) && !('guestNames' in w.body.args), 'no guests unless going');

  const before = applies(p).length;
  choice(pageOf(p), 'going').onclick();
  await settle();
  assert.equal(applies(p).length, before, 'the answer already held: nothing written');

  const q = page();
  choice(await openEvent(q), 'going').onclick();
  await settle();
  w = applies(q).at(-1);
  declared(w.body.args, RSVP);
  assert.equal(w.body.object, SITE);
  assert.equal(w.body.args.status, 'going');
  assert.equal(w.body.args.guests, 0, 'going: its guests');
  assert.ok(!('guestNames' in w.body.args), 'no names given, none sent');
});

test('the Site is found by its created edge when another Site is open; with none, no RSVP bar', async () => {
  const p = page();
  await openEvent(p);
  p.drawn.at(-1).ctx.open('object', EVENT, OTHER);
  await settle();
  assert.equal(p.drawn.at(-1).ctx.S.site, OTHER, 'another Site open');
  choice(pageOf(p), 'going').onclick();
  await settle();
  assert.equal(applies(p, 'group.rsvp').at(-1).body.object, SITE, 'the Site that created it');

  const none = await openEvent(page({ uncreated: true }));
  assert.ok(!find(none, (n) => cls(n, 'ev-choice')), 'no Site: no RSVP bar');
  assert.ok(none.textContent.includes('Dig day'), 'the page still drawn');
});

test('registration: Maybe hidden when it is off; guests stop at guestsMax', async () => {
  const root = await openEvent(page({ registration: { maybe: 0, guestsMax: 1 }, rsvps: { [ME]: { status: 'going', guests: 0, at: 5 } } }));
  assert.ok(!choice(root, 'maybe'), 'no Maybe');
  assert.ok(choice(root, 'going') && choice(root, 'declined'));
  const p = page({ registration: { guestsMax: 1 }, rsvps: { [ME]: { status: 'going', guests: 0, at: 5 } } });
  await openEvent(p);
  button(pageOf(p), '+').onclick();
  const plus = button(pageOf(p), '+');
  if (!plus.disabled) plus.onclick();
  assert.equal(all(pageOf(p), (n) => n.tag === 'input' && cls(n, 'ev-guest')).length, 1, 'at most guestsMax');
  assert.equal(button(pageOf(p), '+').disabled, true, '+ stops at guestsMax');
});

test('declined members are counted, never listed; going and maybe show faces', async () => {
  const p = page({ rsvps: { [GITA]: { status: 'going', guests: 2, at: 5 }, [MO]: { status: 'maybe', guests: 0, at: 6 },
    [DEE]: { status: 'declined', guests: 0, at: 7 } }, counts: { going: 1, maybe: 1, declined: 1, guests: 2 } });
  const root = await openEvent(p);
  const going = tally(root, 'going'), maybe = tally(root, 'maybe'), declined = tally(root, 'declined');
  assert.ok(going && maybe && declined, 'a tally for each');
  assert.ok(all(going, (n) => cls(n, 'ev-face')).some((n) => everywhere(n).includes('Gita')), "going: Gita's face");
  assert.ok(all(maybe, (n) => cls(n, 'ev-face')).some((n) => everywhere(n).includes('Mo')), "maybe: Mo's face");
  assert.ok(declined.textContent.includes('1'), 'declined: counted');
  assert.equal(all(declined, (n) => cls(n, 'ev-face')).length, 0, 'declined: no faces');
  assert.ok(!everywhere(root).includes('Dee'), 'who declined is nowhere on the page');
});

test('"Pending" shows for my answer while approval holds it undecided', async () => {
  const mine = (x) => ({ [ME]: { status: 'going', guests: 0, at: 5, ...x } });
  const pending = await openEvent(page({ registration: { approval: 1 }, rsvps: mine({}) }));
  assert.ok(find(pending, (n) => cls(n, 'ev-pending') && n.textContent === 'Pending'), 'Pending');
  const approved = await openEvent(page({ registration: { approval: 1 }, rsvps: mine({ decision: 'approved' }) }));
  assert.ok(!approved.textContent.includes('Pending'), 'approved: not pending');
  const open = await openEvent(page({ registration: { approval: 0 }, rsvps: mine({}) }));
  assert.ok(!open.textContent.includes('Pending'), 'no approval: not pending');
});

test("the fold's place wins (FOLDS-CORE): Pending while a host decides, going faces only once placed, my guests' names held", async () => {
  const p = page({ registration: { approval: 0, guestsMax: 3 }, rsvps: {
    [ME]: { status: 'going', guests: 1, at: 5, place: 'pending', guestNames: ['Ana'] },
    [GITA]: { status: 'going', guests: 0, at: 6, place: 'waitlisted' },
    [MO]: { status: 'going', guests: 0, at: 7, place: 'going' } }, counts: { going: 1, maybe: 0, declined: 0, guests: 0 } });
  const root = await openEvent(p);
  assert.ok(find(root, (n) => cls(n, 'ev-pending') && n.textContent === 'Pending'), 'place pending: Pending, whatever approval says');
  const faces = all(tally(root, 'going'), (n) => cls(n, 'ev-face')).map(everywhere).join(' ');
  assert.ok(faces.includes('Mo') && !faces.includes('Gita'), 'going faces: placed answers only');
  assert.deepEqual(all(root, (n) => cls(n, 'ev-guest')).map((n) => n.value), ['Ana'], "my guests' names, as the fold holds them");
});

test('lineup: an unconfirmed act is listed as "Unconfirmed"; only its performer sees "Confirm", which writes performs_at on their own record', async () => {
  const lineup = [{ member: PIA, role: 'headliner', start: START, end: START + 3600000, confirmed: false },
    { member: ME, role: 'dj', start: null, end: null, confirmed: false },
    { member: GITA, role: 'support', start: null, end: null, confirmed: true },
    { member: NOBODY, role: 'mc', start: null, end: null, confirmed: false }];
  const p = page({ event: { acts: lineup } });
  const root = await openEvent(p);
  assert.equal(actsIn(root).length, 4, 'every act, the unresolved one too: never dropped');
  const pia = actOf(root, 'Pia');
  assert.ok(pia.textContent.includes('headliner') && pia.textContent.includes(hhmm(START, TZ)), 'its role and set time');
  assert.ok(pia.textContent.includes('Unconfirmed'), 'unconfirmed');
  assert.ok(!actOf(root, 'Gita').textContent.includes('Unconfirmed'), 'confirmed');
  assert.equal(buttons(root, 'Confirm').length, 1, 'one Confirm');
  const mine = actOf(root, 'Cal');
  const confirm = button(mine, 'Confirm');
  assert.ok(confirm && !confirm.disabled, "on the performer's own act");
  confirm.onclick();
  await settle();
  const w = applies(p).at(-1);
  assert.equal(w.body.object, SELF, "the performer's own record");
  assert.equal(w.body.op, 'group.setAffiliation');
  declared(w.body.args, AFFIL);
  for (const k of Object.keys(AFFIL.args).filter((k) => AFFIL.args[k].required)) assert.ok(k in w.body.args, 'required ' + k);
  assert.equal(w.body.args.peer, EVENT);
  assert.equal(w.body.args.rel, 'performs_at');

  const held = await openEvent(page({ event: { acts: lineup }, selfAffiliations: [{ peer: EVENT, rel: 'performs_at', name: 'Dig day', tether: null, at: 2 }] }));
  assert.equal(buttons(held, 'Confirm').length, 0, 'the half already held: no Confirm');
  const noSelf = await openEvent(page({ event: { acts: lineup }, noSelf: true }));
  assert.equal(button(noSelf, 'Confirm').disabled, true, 'no own record held: Confirm drawn disabled');
});

test("an organisation's act: Confirm for its owner, writing performs_at on its own group", async () => {
  const p = page({ event: { acts: [{ object: BAND, role: 'performer', start: null, end: null, confirmed: false },
    { object: THEIRS, role: 'support', start: null, end: null, confirmed: false }] } });
  const root = await openEvent(p);
  assert.ok(actOf(root, 'The Diggers') && actOf(root, 'Hoe Down'), "named by the group's own profile");
  assert.equal(buttons(actOf(root, 'Hoe Down'), 'Confirm').length, 0, "another's group: no Confirm");
  button(actOf(root, 'The Diggers'), 'Confirm').onclick();
  await settle();
  const w = applies(p).at(-1);
  assert.equal(w.body.object, BAND);
  assert.equal(w.body.op, 'group.setAffiliation');
  declared(w.body.args, AFFIL);
  assert.equal(w.body.args.rel, 'performs_at');
  assert.equal(w.body.args.peer, EVENT);
});

test('an op the served ICD does not declare is not drawn', async () => {
  const lineup = [{ member: ME, role: 'dj', start: null, end: null, confirmed: false }];
  const noRsvp = structuredClone(ICD);
  delete noRsvp.kinds.group.ops['group.rsvp'];
  const a = await openEvent(page({ icd: noRsvp, event: { acts: lineup } }));
  assert.ok(!find(a, (n) => cls(n, 'ev-choice')), 'no group.rsvp: no RSVP bar');
  assert.ok(button(a, 'Confirm'), 'the rest drawn');
  const noHalf = structuredClone(ICD);
  delete noHalf.kinds.group.ops['group.setAffiliation'].args.rel.vocabulary.performs_at;
  const b = await openEvent(page({ icd: noHalf, event: { acts: lineup } }));
  assert.ok(!button(b, 'Confirm'), 'no performs_at: no Confirm');
  assert.ok(b.textContent.includes('Unconfirmed'), 'the act still shown');
});

test('Manage: for the owner and a co-host, not a member; it opens event-edit on the event', async () => {
  const EventsEdit = { drawn: [], draw(f, x) { this.drawn.push(x); } };
  const p = page({ owner: ME }, {}, { EventsEdit });
  const mine = await openEvent(p);
  const manage = button(mine, 'Manage');
  assert.ok(manage, 'the owner');
  manage.onclick();
  await settle();
  assert.equal(EventsEdit.drawn.at(-1), EVENT, 'event-edit, on the event');
  assert.ok(button(await openEvent(page({ event: { roles: { [ME]: 'admin' } } })), 'Manage'), 'a co-host');
  assert.ok(!button(await openEvent(page()), 'Manage'), 'a member: no Manage');
});

test("a refusal is shown as the Door words it", async () => {
  const WHY = 'Answers to Dig day closed at 21:00.';
  const p = page({ event: { acts: [{ member: ME, role: 'dj', start: null, end: null, confirmed: false }] } },
    { '/v2/apply': () => [422, WHY] });
  choice(await openEvent(p), 'going').onclick();
  await settle();
  assert.ok(pageOf(p).textContent.includes(WHY), 'the RSVP refused, verbatim');
  const q = page({ event: { acts: [{ member: ME, role: 'dj', start: null, end: null, confirmed: false }] } },
    { '/v2/apply': () => [422, 'Not your record.'] });
  button(await openEvent(q), 'Confirm').onclick();
  await settle();
  assert.ok(pageOf(q).textContent.includes('Not your record.'), 'Confirm refused, verbatim');
});

test('the badge shows for postponed and cancelled, in the ICD\'s word, not for scheduled', async () => {
  const words = EV['event.setProfile'].args.status.vocabulary;
  for (const s of ['postponed', 'cancelled']) {
    const root = await openEvent(page({ event: { status: s } }));
    const badge = find(root, (n) => cls(n, 'ev-badge'));
    assert.ok(badge, s + ': a badge');
    assert.equal(badge.textContent, words[s]);
  }
  assert.ok(!find(await openEvent(page()), (n) => cls(n, 'ev-badge')), 'scheduled: none');
});

test('the media: the banner as cover, else the clip muted; photos; the video, tickets and online links; the wall', async () => {
  const banner = { mime: 'image/png', data: 'iVBORw0KGgo=', width: 1, height: 1 };
  const clip = { mime: 'video/mp4', data: 'AAAAIGZ0eXA=', width: 1, height: 1, duration_ms: 1000 };
  const msg = (author, gen, text) => ({ author, gen, text, ts: 1759200000000 + gen, reply_to: null, up: 0, down: 0, reactions: [] });
  const p = page({ event: { banner, clip, photos: [{ id: '0123456789abcdef', mime: 'image/jpeg', data: '/9j/', at: 1, width: 1, height: 1 }],
    video_url: 'https://video.example/v', ticket_url: 'https://tix.example/t', online: 'https://meet.example/x' },
  wall: [msg(GITA, 1, 'See you there')] });
  const root = await openEvent(p);
  const cover = find(root, (n) => cls(n, 'ev-cover'));
  assert.ok(find(cover, (n) => n.tag === 'img' && n.attrs.src === 'data:image/png;base64,' + banner.data), 'the banner');
  assert.ok(find(root, (n) => n.tag === 'img' && n.attrs.src === 'data:image/jpeg;base64,/9j/'), 'a photo');
  const video = find(root, (n) => n.tag === 'video');
  assert.ok(video && video.attrs.src === 'data:video/mp4;base64,' + clip.data && 'muted' in video.attrs && 'controls' in video.attrs, 'the clip, muted, with controls');
  for (const href of ['https://video.example/v', 'https://tix.example/t', 'https://meet.example/x'])
    assert.ok(find(root, (n) => n.tag === 'a' && n.attrs.href === href), href);
  assert.ok(root.textContent.includes('See you there'), "the wall's latest");
  button(root, 'Wall').onclick();
  await settle();
  assert.ok(p.$('pttl').textContent.includes('comments'), 'the wall opens as its channel');

  const bare = await openEvent(page({ event: { clip, ticket_url: 'javascript:alert(1)' } }));
  const v = find(find(bare, (n) => cls(n, 'ev-cover')), (n) => n.tag === 'video');
  assert.ok(v && 'muted' in v.attrs, 'no banner: the clip, muted, is the cover');
  assert.ok(!find(bare, (n) => n.tag === 'a' && /^javascript:/.test(n.attrs.href || '')), 'only http and https are links');
  assert.ok(!button(bare, 'Wall'), 'no wall: none drawn');
});

test('the wall: a member posts there as in a room, forum.post on the comments part, the ICD\'s args, no gen', async () => {
  const p = page({ wall: [{ author: GITA, gen: 1, text: 'Bring a trowel', ts: 1 }] });
  const root = await openEvent(p);
  const say = find(root, (n) => n.attrs && n.attrs.name === 'wall-say');
  assert.ok(say, 'the composer');
  say.value = 'Soup is on me';
  find(root, (n) => n.attrs && n.attrs['data-act'] === 'wall-send').onclick();
  await settle();
  const w = applies(p, 'forum.post');
  assert.equal(w.length, 1, 'posted once');
  assert.equal(w[0].body.object, FORUM, 'on the comments part');
  assert.equal(w[0].body.args.text, 'Soup is on me');
  const declared = Object.keys(ICD.kinds.forum.ops['forum.post'].args).filter((a) => a !== 'gen');
  for (const k of Object.keys(w[0].body.args)) assert.ok(declared.includes(k), k);
});

test("the wall's composer is for its members alone: off the comments part's roster, none is drawn", async () => {
  const root = await openEvent(page({ wall: [{ author: GITA, gen: 1, text: 'Bring a trowel', ts: 1 }], wallMembers: [OWNER, GITA] }));
  assert.ok(root.textContent.includes('Bring a trowel'), 'the wall is read');
  assert.equal(find(root, (n) => n.attrs && n.attrs.name === 'wall-say'), null, 'no composer');
  assert.equal(find(root, (n) => n.attrs && n.attrs['data-act'] === 'wall-send'), null, 'no Send');
});

/* THE ROSTER GAP (MANAGE's (b), BUILD 1 Oct): an event its Site created that this member does not
   hold. The Site's `created` names it; its public item (GET /v2/site/:site/items, the Arc's
   face_items body) says what the public may see. The board lists it and the page draws it read-
   only from the item, with the RSVP bar on the Site, which the member holds. */
const ITEM = { title: 'Dig day', startMs: START, endMs: END, venue: 'Plot 9', tz: TZ, status: 'postponed', recurrence: 'FREQ=WEEKLY;BYDAY=SA',
  acts: [{ member: MO, role: 'headliner' }], at: 1 };
test("an event the member does not hold: listed from the Site's created edge and its item, read-only, RSVP on the Site", async () => {
  const p = page({ unheld: true, items: { [`event:${EVENT}`]: ITEM }, wall: [{ author: GITA, gen: 1, text: 'Bring a trowel', ts: 1 }] });
  const root = await openEvent(p);
  assert.ok(p.calls.some((c) => c.path === `/v2/site/${SITE}/items`), "the Site's items asked for");
  const text = root.textContent;
  assert.ok(text.includes('Dig day') && text.includes(hhmm(START, TZ)) && text.includes('Plot 9'), "title, when in the event's zone, where");
  assert.ok(find(root, (n) => cls(n, 'ev-badge') && n.attrs['data-status'] === 'postponed'), 'the status');
  assert.ok(actOf(root, 'Mo'), 'the act the item carries');
  assert.ok(!everywhere(root).includes('Unconfirmed') && !button(root, 'Confirm'), 'confirmed only, no Confirm');
  assert.ok(!button(root, 'Manage'), 'no Manage');
  assert.equal(find(root, (n) => n.attrs && n.attrs.name === 'wall-say'), null, 'no wall composer');
  assert.ok(!root.textContent.includes('Bring a trowel'), 'no wall, even where a comments part is held');
  choice(root, 'going').onclick();
  await settle();
  const w = applies(p, 'group.rsvp');
  assert.equal(w.length, 1, 'an answer');
  assert.deepEqual([w[0].body.object, w[0].body.args.event, w[0].body.args.status], [SITE, EVENT, 'going'], 'on the Site, for the event');
  const board = find(p.$('lbody'), (n) => n.tag === 'button' && n.textContent.includes('Unlisted'));
  assert.equal(board, null, 'an event without an item is not listed');
});
