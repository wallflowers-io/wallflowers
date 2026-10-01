/* events-edit.test.mjs — W-98 Events, EVENTS-UI-A: the event editor (events-edit.js), for a new
   event and for Manage, in place of the Site's sheet (UX's spec, pdr/w98-integration.md).

   A new event is one /v2/batch: the mint, event.setProfile whole, the Site's
   group.setAffiliation {rel: created} and the event's base.setBacklink at one moment (NC-81, as
   create.test.mjs holds the sheet to; its event case and "a refused backlink" are re-homed here
   with the same assertions), then the media, the lineup and the Site's registration. Repeat
   writes only a rule the grammar allows. The owner saves with setProfile carrying every held
   field; a co-host with editProfile. The lineup writes only {member | object, role, start, end}.
   A picture over the ICD's cap is refused before any write, and event.setTickets is never
   written. webapp.js, events-icd.js and events-edit.js run as the page runs them, over a stub
   page, the real ICD and a stub Door.

   Run: node --test app/web/webapp/events-edit.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const ICDJS = readFileSync(new URL('./events-icd.js', import.meta.url), 'utf8');
const EDITJS = readFileSync(new URL('./events-edit.js', import.meta.url), 'utf8');
const PAGEJS = readFileSync(new URL('./events-page.js', import.meta.url), 'utf8');
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));
const ORIGIN = 'https://app.wallflowers.io';
const ME = 'c'.repeat(64), SITE = 'a'.repeat(64), MADE = 'd'.repeat(64), EVENT = 'e'.repeat(64), OTHER = '7'.repeat(64), PERF = '8'.repeat(64);
const EV = ICD.kinds.event.ops;
const EVENT_VIEW = { title: 'Dig day', descriptor: 'Bring gloves', start_ms: 1790000000000, end_ms: 1790003600000, venue: 'Plot 9',
  recurrence: null, tz: 'Europe/London', online: null, status: 'scheduled', video_url: null, descriptor_format: 'plain', all_day: 0,
  ticket_url: null, lineup: [], acts: [], visibility: 'private', banner: null, clip: null, photos: [] };
/* The args a site sends of `op`: the ICD's own, less the gen the Door adds to a commutative op. */
const argsOf = (op) => Object.keys(EV[op].args).filter((a) => !(a === 'gen' && EV[op].fold === 'commutative'));

function page({ event: held = EVENT_VIEW, owner = ME, roles = {}, answers = {}, site: siteView = {} } = {}) {
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
  const event = { id: EVENT, kind: 'event', name: 'Dig day', owner, members: [ME, OTHER, PERF], folds: true, view: { ...held, roles } };
  site.view.profiles = { [PERF]: { name: 'Mo', gen: 1 }, [OTHER]: { name: 'Jo', gen: 1 } };
  site.members = [ME, OTHER, PERF];
  Object.assign(site.view, siteView);
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
  /* A picked file, as the browser reads it: its data URL. No canvas here, so nothing is drawn smaller. */
  ctx.FileReader = class { readAsDataURL(f) { this.result = f.url; setImmediate(() => this.onload && this.onload()); } };
  ctx.window = ctx;
  vm.createContext(ctx);
  // As index.html loads them: the readers and the editor, then the page.
  vm.runInContext(ICDJS, ctx);
  vm.runInContext(EDITJS, ctx);
  // The event's page, as index.html loads it before the editor: where a saved event opens, and
  // whose formatter and labels the editor's preview and Guests use.
  vm.runInContext(PAGEJS, ctx);
  // What the page hands the editor, kept, so a test can open it on an event as Manage does.
  const handed = {};
  const E = ctx.WallFlowers && ctx.WallFlowers.EventsEdit;
  if (E) { const draw = E.draw; E.draw = (f, x, c) => { handed.ctx = c; return draw.call(E, f, x, c); }; }
  vm.runInContext(SRC, ctx);
  return { $, calls, handed, vm: ctx };
}

const settle = async () => { for (let i = 0; i < 60; i++) await new Promise((r) => setImmediate(r)); };
function find(n, pred) {
  if (pred(n)) return n;
  for (const c of n.children || []) { const f = find(c, pred); if (f) return f; }
  return null;
}
const named = (p, name) => find(p.$('feed'), (n) => n.attrs && n.attrs.name === name);
const act = (p, name) => find(p.$('feed'), (n) => n.attrs && n.attrs['data-act'] === name);
function set(p, name, value) {
  const n = named(p, name);
  assert.ok(n, `the editor's ${name}`);
  n.value = value;
  (n.oninput || n.onchange || (() => {}))({ target: n });
}
const writes = (p) => p.calls.filter((c) => c.batched || c.path === '/v2/apply').map((c) => c.body);
const batches = (p) => p.calls.filter((c) => c.path === '/v2/batch');
async function newEvent(p) {
  await settle();
  find(p.$('tabs'), (n) => n.tag === 'button' && n.textContent === 'Events').onclick();
  await settle();
  find(p.$('lbody'), (n) => n.tag === 'button' && n.textContent === 'New event').onclick();
  await settle();
}
/* Manage, as the event's page opens it: the editor on the event, by its id. */
async function manage(p) {
  await newEvent(p);
  assert.ok(p.handed.ctx, 'the editor was drawn');
  p.handed.ctx.open('event-edit', EVENT);
  await settle();
}
async function publish(p) {
  const go = act(p, 'publish') || act(p, 'save');
  assert.ok(go, 'Publish or Save, always there');
  go.onclick({ preventDefault() {} });
  await settle();
}

test('New event opens the editor, not the sheet: Publish always there, the rail basics first', async () => {
  const p = page();
  await newEvent(p);
  assert.equal(p.$('sheet').hidden, true, 'no sheet');
  assert.ok(act(p, 'publish'), 'Publish');
  const rail = find(p.$('feed'), (n) => n.attrs && n.attrs['data-rail'] != null);
  assert.ok(rail, 'the rail');
  const steps = [];
  (function walk(n) { if (n.attrs && n.attrs['data-step']) steps.push(n.attrs['data-step']); (n.children || []).forEach(walk); })(rail);
  assert.equal(steps[0], 'basics', 'basics first');
  assert.ok(!steps.includes('hosts') && !steps.includes('guests') && !steps.includes('status'), 'Manage-only sections are not for a new event');
});

test("event: made from the editor, declared from both ends at one moment, in one request (create.test.mjs's case, re-homed)", async () => {
  const p = page();
  await newEvent(p);
  set(p, 'title', 'Seed swap');
  set(p, 'startMs', '2026-10-04T10:00');
  await publish(p);
  const w = writes(p);
  assert.deepEqual(w.slice(0, 4).map((b) => b.kind || b.op), ['event', 'event.setProfile', 'group.setAffiliation', 'base.setBacklink']);
  const [mint, prof, aff, back] = w;
  assert.equal(mint.draft.name, 'Seed swap');
  assert.equal(prof.object, MADE);
  assert.equal(prof.args.title, 'Seed swap');
  assert.equal(prof.args.startMs, Date.parse('2026-10-04T10:00'));
  assert.deepEqual([aff.object, aff.args.peer, aff.args.rel, aff.args.name], [SITE, MADE, 'created', 'Seed swap'], "the Site's half");
  assert.deepEqual([back.object, back.args.object, back.args.rel], [MADE, SITE, 'created'], "the object's half");
  assert.equal(back.args.at, aff.args.at, 'one moment from both ends');
  assert.deepEqual(p.calls.filter((c) => !c.batched && ['/v2/batch', '/v2/mint', '/v2/apply'].includes(c.path)).map((c) => c.path), ['/v2/batch'], 'one request');
  for (const b of w.filter((b) => b.op && b.op.startsWith('event.'))) {
    for (const k of Object.keys(b.args)) assert.ok(argsOf(b.op).includes(k), `${b.op}: ${k} is an arg the ICD declares`);
  }
  assert.ok(!w.some((b) => b.op === 'event.setTickets'), 'setTickets is never written');
});

test("the preview draws the time with the page's own formatter, in the event's zone", async () => {
  const p = page();
  await manage(p);
  const when = find(p.$('feed'), (n) => n.attrs && n.attrs['data-preview'] === 'when');
  assert.ok(when, 'the preview says when');
  const view = { start_ms: EVENT_VIEW.start_ms, end_ms: EVENT_VIEW.end_ms, tz: EVENT_VIEW.tz, all_day: EVENT_VIEW.all_day };
  assert.equal(when.textContent, p.vm.WallFlowers.EventsPage.whenOf(view), "the page's words");
  const tz = named(p, 'tz');
  assert.equal(tz.tag, 'select', 'a zone is chosen, never typed');
  assert.equal(tz.value, 'Europe/London', 'the held zone');
  assert.ok(tz.children.length > 100 && tz.children.every((o) => o.attrs.value), "Intl's zones");
});

test("a refused backlink is said, with how far the link got (create.test.mjs's case, re-homed)", async () => {
  const p = page({ answers: { '/v2/apply': (b) => (b.op === 'base.setBacklink' ? [403, 'not yours to write'] : null) } });
  await newEvent(p);
  set(p, 'title', 'Dig day');
  set(p, 'startMs', '2026-10-04T10:00');
  await publish(p);
  const why = find(p.$('feed'), (n) => n.attrs && n.attrs['data-why'] != null);
  assert.equal(why && why.textContent, 'the Site names it, and it does not name the Site: not yours to write');
  assert.ok(act(p, 'publish'), 'the editor stays, with the refusal on it');
});

test('Repeat writes only a rule the grammar allows; a held rule it cannot express stays as it was', async () => {
  const p = page();
  await newEvent(p);
  set(p, 'title', 'Dig day');
  set(p, 'startMs', '2026-10-04T10:00');
  const freq = named(p, 'freq');
  assert.deepEqual(freq.children.map((o) => o.attrs.value), [''].concat(ICD.kinds.event.ops['event.setProfile'].args.recurrence.grammar.FREQ.oneOf), 'none, then the grammar\'s');
  set(p, 'freq', 'WEEKLY');
  await settle();
  set(p, 'interval', '2');
  for (const d of ['MO', 'WE']) { const b = act(p, 'day-' + d); assert.ok(b, d); b.onclick({ preventDefault() {} }); }
  await publish(p);
  assert.equal(writes(p)[1].args.recurrence, 'FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE');

  const q = page({ event: { ...EVENT_VIEW, recurrence: 'FREQ=WEEKLY;BYHOUR=9' } });
  await manage(q);
  const f = named(q, 'freq');
  assert.equal(f.value, 'FREQ=WEEKLY;BYHOUR=9', 'held, selected');
  assert.equal(f.attrs.disabled, 'true', 'read-only');
  await publish(q);
  assert.equal(writes(q).find((b) => b.op === 'event.setProfile').args.recurrence, 'FREQ=WEEKLY;BYHOUR=9', 'kept');
});

test('the owner saves with setProfile carrying every held field; a co-host with editProfile', async () => {
  const held = { ...EVENT_VIEW, all_day: 1, descriptor_format: 'markdown', video_url: 'https://v.example/1' };
  const p = page({ event: held });
  await manage(p);
  set(p, 'title', 'Dig day, again');
  await publish(p);
  const prof = writes(p).find((b) => b.op === 'event.setProfile');
  assert.ok(prof, 'setProfile');
  assert.deepEqual([prof.args.title, prof.args.allDay, prof.args.descriptorFormat, prof.args.videoUrl, prof.args.tz, prof.args.venue],
    ['Dig day, again', 1, 'markdown', 'https://v.example/1', 'Europe/London', 'Plot 9'], 'every held field, those not shown too');
  assert.ok(!writes(p).some((b) => b.op === 'event.editProfile'));

  const q = page({ event: held, owner: OTHER, roles: { [ME]: 'admin' } });
  await manage(q);
  set(q, 'title', 'Dig day, again');
  await publish(q);
  const ed = writes(q).find((b) => b.op === 'event.editProfile');
  assert.ok(ed, 'editProfile');
  assert.equal(ed.args.gen, undefined, "gen is the Door's to add");
  assert.deepEqual([ed.args.title, ed.args.allDay, ed.args.descriptorFormat], ['Dig day, again', 1, 'markdown']);
  assert.ok(!writes(q).some((b) => b.op === 'event.setProfile'), 'a co-host does not write setProfile');
});

test('the lineup picks profiles only and writes {member | object, role, start, end}', async () => {
  const p = page();
  await manage(p);
  const pick = named(p, 'act-pick');
  assert.ok(pick.children.every((o) => o.attrs.value === '' || /^[0-9a-f]{64}$/.test(o.attrs.value)), 'profiles only, no free-text names');
  assert.ok(pick.children.some((o) => o.attrs.value === PERF), "the Site's profiles");
  set(p, 'act-pick', PERF);
  const add = act(p, 'act-add');
  assert.ok(add, 'add an act');
  add.onclick({ preventDefault() {} });
  await settle();
  set(p, 'act-role-0', 'headliner');
  await publish(p);
  const l = writes(p).find((b) => b.op === 'event.setLineup');
  assert.ok(l, 'setLineup');
  assert.deepEqual(JSON.parse(l.args.acts), [{ member: PERF, role: 'headliner' }]);
  assert.deepEqual(Object.keys(l.args), ['acts'], "acts alone; gen is the Door's");
});

test("a picture over the ICD's cap is refused before any write", async () => {
  const cap = EV['event.addPhoto'].args.photo.maxLength;
  const p = page();
  await manage(p);
  const input = named(p, 'photo');
  assert.ok(input, 'add a photo');
  input.files = [{ name: 'big.jpg', type: 'image/jpeg', url: 'data:image/jpeg;base64,' + 'A'.repeat(cap + 4) }];
  input.onchange({ target: input });
  await settle();
  const why = find(p.$('feed'), (n) => n.attrs && n.attrs['data-why-media'] != null);
  assert.ok(why && why.textContent.includes(String(cap)), 'refused, with the cap');
  await publish(p);
  assert.ok(!writes(p).some((b) => b.op === 'event.addPhoto'), 'not written');
});

test("Manage's Guests: the Site's counts, declined a count only, and a pending answer decided on the Site", async () => {
  const [rk, ck] = Object.keys(ICD.kinds.group.ops['group.rsvp'].view), [gk] = Object.keys(ICD.kinds.group.ops['group.setRegistration'].view);
  const p = page({ site: {
    [rk]: { [EVENT]: { [PERF]: { status: 'going', guests: 0, at: 5, place: 'pending' }, [OTHER]: { status: 'declined', guests: 0, at: 6, place: 'declined' } } },
    [ck]: { [EVENT]: { going: 0, maybe: 0, declined: 1, guests: 0 } }, [gk]: { [EVENT]: { approval: 1, waitlist: 1 } } } });
  await manage(p);
  const step = find(p.$('feed'), (n) => n.attrs && n.attrs['data-step'] === 'guests');
  assert.ok(step, 'Guests, in Manage');
  step.onclick();
  await settle();
  const sec = find(p.$('feed'), (n) => n.attrs && n.attrs['data-sec'] === 'guests');
  assert.ok(sec.textContent.includes('Pending'), 'the pending answer');
  assert.ok(sec.textContent.includes('Mo'), 'by name');
  assert.ok(!sec.textContent.includes('Jo'), 'who declined is not listed');
  assert.ok(sec.textContent.includes("1 Can't go"), 'declined, a count, in the page\'s word');
  assert.deepEqual(['approved', 'waitlisted', 'declined'].map((d) => act(p, `decide-${d}-${PERF}`).textContent), ['Approve', 'Waitlist', 'Decline'], 'verbs, not the enum');
  act(p, 'decide-approved-' + PERF).onclick();
  await settle();
  const d = writes(p).find((b) => b.op === 'group.rsvpDecide');
  assert.ok(d, 'decided');
  assert.deepEqual([d.object, d.args.event, d.args.member, d.args.decision], [SITE, EVENT, PERF, 'approved'], 'on the Site');
  const declared = Object.keys(ICD.kinds.group.ops['group.rsvpDecide'].args).filter((a) => a !== 'gen');
  for (const k of Object.keys(d.args)) assert.ok(declared.includes(k), k);
});

test("times are the event's own: its zone's wall clock shown, and read back in that zone", async () => {
  const p = page();
  await manage(p);
  const w = (ms, timeZone) => new Intl.DateTimeFormat('sv-SE', { timeZone, year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }).format(ms).replace(' ', 'T');
  const start = named(p, 'startMs');
  assert.equal(start.value || start.attrs.value, w(EVENT_VIEW.start_ms, 'Europe/London'), "the start as London's clock reads it");
  set(p, 'startMs', '2026-10-04T10:00');
  await publish(p);
  const prof = writes(p).find((b) => b.op === 'event.setProfile');
  assert.equal(prof.args.startMs, Date.UTC(2026, 9, 4, 9, 0), '10:00 in London in October (BST) is 09:00 UTC');
});

test("one event, one time: held in London at 10:00 UTC, 11:00 in the field, the preview and the page; saved unchanged, the same instant", async () => {
  const start = Date.UTC(2026, 9, 11, 10, 0), end = Date.UTC(2026, 9, 11, 14, 0);
  const held = { ...EVENT_VIEW, tz: 'Europe/London', start_ms: start, end_ms: end };
  const p = page({ event: held });
  await manage(p);
  const field = named(p, 'startMs');
  assert.equal(field.value || field.attrs.value, '2026-10-11T11:00', "the field: London's clock");
  const EP = p.vm.WallFlowers.EventsPage, pageWhen = EP.whenOf(held);
  assert.ok(pageWhen.includes('11:00') && pageWhen.includes('15:00'), `the page: ${pageWhen}`);
  const preview = find(p.$('feed'), (n) => n.attrs && n.attrs['data-preview'] === 'when');
  assert.equal(preview.textContent, pageWhen, 'the preview: the page\'s words');
  await publish(p);
  const prof = writes(p).find((b) => b.op === 'event.setProfile');
  assert.deepEqual([prof.args.startMs, prof.args.endMs, prof.args.tz], [start, end, 'Europe/London'], 'saved unchanged: the same instants and zone');
});
