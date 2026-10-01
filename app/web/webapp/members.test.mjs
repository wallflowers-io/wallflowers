/* members.test.mjs — W-98 Members (Ralph, 30 Sep: "New Members page accessed by clicking the
   'MEMBERS — x' control on the right rail. Ability to post a developed, site scoped bio and
   info. Site owner/admin can define free questions and multiselect polls."; modelled on
   node.cyp3.xyz's Node Grove: SEEK, OFFER (multiselect tags), IMAGINE).

   The page is members.js, drawn through two guarded hooks in webapp.js: the right rail's
   "Members — n" becomes a button that opens it in the pane, and the pane hands it S.view
   'members'. It reads GET /v2/members/<site id> (the common route's contract, ICD 2.3.1
   draft) through the webapp's own Door session, and draws every member in the route's order:
   icon or a neutral placeholder, name or "New member" (never a key), ROLE's word, and their
   intentions as labels. A selected member shows their about, their answers and intentions.
   Writes are POST /v2/apply on the Site: base.publishAbout (the whole record), and
   base.answerQuestion, base.defineQuestion and base.retireQuestion; links, options and choices
   are JSON array text; a question is named by the author and gen of its id; no gen is ever
   sent. Caps are counted in bytes of UTF-8 before sending; a refusal is the Door's sentence
   as it stands; after a 200 the route is read again. Pages carry labels only (BUILD, 1 Oct):
   UX's note (pdr/w98-mockup.html) says who sees what I write, drawn once, at the head of what
   it covers: "Members see your answer." heading my answers; nothing on what is only read. The
   about editor has none: "Members see your bio and links." was new copy, and Ralph had not
   approved it by 20:15Z (UX, 1 Oct), so ASSURANCE's rule stays a residual there for his
   morning. An editor that is open is never
   rebuilt under the member typing in it, whatever the Door reports meanwhile.

   members.js and webapp.js run as index.html loads them, over a stub page and a stub Door,
   with the route's answer from fixture/members.json.

   Run: node --test app/web/webapp/members.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const MEMBERS_AT = new URL('./members.js', import.meta.url);
const MEMBERS = existsSync(MEMBERS_AT) ? readFileSync(MEMBERS_AT, 'utf8') : '';
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));
const FIXTURE = JSON.parse(readFileSync(new URL('./fixture/members.json', import.meta.url), 'utf8'));
const ORIGIN = 'https://app.wallflowers.io';
const SITE = FIXTURE.site;
const [OWNER, ADMIN, KOFI, ANON, BLANK] = FIXTURE.members.map((m) => m.key);
const Q = Object.fromEntries(FIXTURE.questions.map((q) => [q.text, q]));
const ref = (q) => { const [a, g] = q.id.split(':'); return { target_author: a, target_gen: Number(g) }; };
const clone = (x) => JSON.parse(JSON.stringify(x));
const SEE_ANSWER = 'Members see your answer.';
const SEE_ABOUT = 'Members see your bio and links.';

/* A signed-in page on the Site. `members`: what /v2/members/<site> answers (a function of the
   call's number, an object, or [status, body]); `apply`: what /v2/apply answers, [status, body];
   `bare`: index.html without members.js. */
function page({ members = FIXTURE, apply = () => [200, {}], me = FIXTURE.me, bare = false,
  roster = [OWNER, ADMIN, KOFI, ANON, BLANK], roles = [[ADMIN, 'admin']] } = {}) {
  const writes = { html: [] };
  class Node {
    constructor(tag) {
      Object.assign(this, { tag, children: [], _text: '', hidden: false, disabled: false, checked: false, value: '', className: '', attrs: {}, scrollTop: 0 });
      this.style = { setProperty() {}, removeProperty() {} };
      this.classList = { add() {}, remove() {}, toggle() {}, contains() { return false; } };
    }
    get textContent() { return this._text + this.children.map((c) => c.textContent || '').join(''); }
    set textContent(v) { this._text = String(v); this.children = []; }
    set innerHTML(v) { writes.html.push(String(v)); this._html = String(v); }
    get innerHTML() { return this._html || ''; }
    set outerHTML(v) { writes.html.push(String(v)); }
    setAttribute(k, v) { this.attrs[k] = String(v); }
    getAttribute(k) { return this.attrs[k] ?? null; }
    removeAttribute(k) { delete this.attrs[k]; }
    appendChild(c) { this.children.push(c); return c; }
    addEventListener(t, f) { this['on' + t] = f; }
    focus() {}
    blur() {}
    getBoundingClientRect() { return { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 }; }
    querySelectorAll() { return []; }
    querySelector() { return null; }
    closest() { return null; }
    contains(n) { return n === this || this.children.some((c) => c.contains && c.contains(n)); }
    remove() {}
    scrollIntoView() {}
    get firstChild() { return this.children[0] || (this._html ? (this._parsed ||= new Node('svg')) : null); }
    get lastChild() { return this.children.at(-1) || null; }
    get childNodes() { return this.children; }
    insertBefore(c, ref) { const i = this.children.indexOf(ref); this.children.splice(i < 0 ? this.children.length : i, 0, c); return c; }
  }
  const byId = {};
  const $ = (id) => (byId[id] ||= new Node('#' + id));
  for (const id of ['sheet', 'side', 'refused', 'status', 'compose', 'siteMenu', 'meMenu', 'you', 'youX', 'pdfv', 'person', 'manage']) $(id).hidden = true;
  const calls = [];
  let asked = 0;
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const site = { id: SITE, kind: 'group', name: 'Node', owner: OWNER, members: roster, folds: true,
    view: { title: 'Node', roles, parts: [] } };
  const fetch = async (url, init = {}) => {
    const path = url.slice(ORIGIN.length), body = init.body ? JSON.parse(init.body) : null;
    calls.push({ path, body, method: init.method || 'GET', headers: init.headers || {}, credentials: init.credentials });
    if (path === '/v2/me') return reply(200, { pk: 'ed25519:' + me, display_name: 'Kofi' });
    if (path === '/v2/graph') return reply(200, { me: { pk: 'ed25519:' + me, display_name: 'Kofi' }, objects: [site], spine: [] });
    if (path.startsWith('/v2/icd')) return reply(200, { kinds: {}, facets: {} });
    if (path === '/v2/members/' + SITE) {
      const a = typeof members === 'function' ? members(asked++) : members;
      return Array.isArray(a) ? reply(a[0], a[1]) : reply(200, a);
    }
    if (path.startsWith('/v2/members')) return reply(404, 'no such route');
    if (path === '/v2/apply') { const [s, b] = apply(body); return reply(s, b); }
    return reply(200, {});
  };
  const streams = [];
  class EventSource {
    constructor() { this.readyState = 1; this.on = {}; streams.push(this); }
    addEventListener(t, f) { this.on[t] = f; }
    close() {}
  }
  const loc = { hash: '#site=' + SITE, pathname: '/', search: '', origin: ORIGIN, assign() {} };
  const ctx = {
    document: { getElementById: $, documentElement: new Node('html'), createElement: (t) => new Node(t), createTextNode: (t) => ({ textContent: String(t) }),
      querySelector: () => null, querySelectorAll: () => [], addEventListener() {} },
    location: loc, history: { replaceState(s, t, u) { loc.hash = u.includes('#') ? u.slice(u.indexOf('#')) : ''; } }, fetch, EventSource, doorOrigin: () => ORIGIN,
    addEventListener() {}, scrollTo() {}, URL,
    innerWidth: 1280, matchMedia: () => ({ matches: false, addEventListener() {} }), requestAnimationFrame: (f) => setImmediate(f),
    atob, TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, Number, console,
    setTimeout: (f, ms) => (ms >= 1000 ? 0 : setImmediate(f)), clearTimeout() {}
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  if (!bare && MEMBERS) vm.runInContext(MEMBERS, ctx);
  vm.runInContext(SRC, ctx);
  /* The Door reports a change (/v2/events 'changed'): the page loads the graph again and redraws. */
  const changed = () => streams.forEach((s) => s.on.changed && s.on.changed());
  return { $, calls, writes, changed };
}

const settle = async () => { for (let i = 0; i < 40; i++) await new Promise((r) => setImmediate(r)); };
const has = (n, c) => typeof n.className === 'string' && n.className.split(/\s+/).includes(c);
const all = (n, pred, out = []) => { if (!n) return out; if (pred(n)) out.push(n); (n.children || []).forEach((c) => all(c, pred, out)); return out; };
const one = (n, pred) => all(n, pred)[0];
const attr = (k, v) => (n) => n.attrs && n.attrs[k] === v;
const visible = (n) => !n.hidden;

/* The rail's control, and the page it opens. */
const control = (p) => one(p.$('rbody'), (n) => /^Members — \d+$/.test(n.textContent) && has(n, 'sec'));
const pageOf = (p) => one(p.$('feed'), (n) => has(n, 'members'));
async function opened(opts) {
  const p = page(opts);
  await settle();
  control(p).onclick();
  await settle();
  return p;
}
const cards = (p) => all(pageOf(p), (n) => n.tag === 'button' && has(n, 'mem'));
const detail = (p) => one(pageOf(p), (n) => has(n, 'mem-detail') && visible(n));
const sec = (p, name) => one(pageOf(p), attr('data-sec', name));
const field = (box, f) => one(box, attr('data-f', f));
const fields = (box, f) => all(box, attr('data-f', f));
const go = (box, g) => one(box, attr('data-go', g));
const why = (box) => one(box, (n) => has(n, 'why'));
const form = (box) => one(box, attr('data-edit', 'define'));
/* A question's row, by its words: a question's id carries its author's key, which the page never shows. */
const question = (box, text) => one(box, (n) => has(n, 'mem-q') && (one(n, (m) => has(m, 'qt')) || {}).textContent === text);
const chips = (box) => all(box, (n) => n.tag === 'button' && has(n, 'chip'));
const pressed = (box) => chips(box).map((c) => c.attrs['aria-pressed']);
const applied = (p) => p.calls.filter((c) => c.path === '/v2/apply').map((c) => c.body);
const reads = (p) => p.calls.filter((c) => c.path.startsWith('/v2/members')).length;
/* UX's labels: a note by an editable field, saying who sees what is written there. */
const notes = (box) => all(box, (n) => has(n, 'note')).map((n) => n.textContent);
const labels = (box) => all(box, (n) => has(n, 'intent')).map((n) => n.textContent);
async function editing(p, name) {
  const box = sec(p, name);
  go(box, 'edit').onclick();
  return box;
}
async function answering(p, text) {
  const row = question(sec(p, 'answers'), text);
  go(row, 'edit').onclick();
  return row;
}
const asAdmin = () => ({ members: { ...clone(FIXTURE), me: ADMIN, can_define: true }, me: ADMIN });

test('the rail\'s "Members — n" is a button, and it opens the Members page for that Site from /v2/members/<site id>', async () => {
  const p = page();
  await settle();
  const c = control(p);
  assert.ok(c, 'the rail\'s heading');
  assert.equal(c.tag, 'button', 'a button');
  assert.equal(c.textContent, 'Members — 5');
  assert.equal(pageOf(p), undefined, 'not open before it is pressed');
  c.onclick();
  await settle();
  const read = p.calls.filter((x) => x.path.startsWith('/v2/members'));
  assert.deepEqual(read.map((x) => x.path), ['/v2/members/' + SITE], 'the path names the Site: no ?site=');
  assert.equal(read[0].method, 'GET');
  assert.equal(read[0].credentials, 'include', 'through the webapp\'s own Door session');
  assert.match(p.$('pttl').textContent, /^Members/);
  assert.ok(pageOf(p), 'the page is drawn in the pane');
  assert.equal(p.$('compose').hidden, true, 'no composer on it');
  assert.equal(control(p).attrs['aria-current'], 'page', 'the control says the page is open');
});

test('a page without members.js still works: the heading is words, and nothing asks for /v2/members', async () => {
  const p = page({ bare: true });
  await settle();
  const c = control(p);
  assert.ok(c, 'the heading is there');
  assert.notEqual(c.tag, 'button', 'but it is not a control');
  assert.equal(c.textContent, 'Members — 5');
  assert.ok(p.$('feed').children.length, 'the pane is drawn');
  assert.equal(reads(p), 0);
});

/* The rail's people under "Members — n" (the harness has no connections, so every one is a member). */
const railPeople = (p) => all(p.$('rbody'), (n) => n.tag === 'button' && has(n, 'it'));
test('the rail\'s "Members — n" counts what the page lists: an admitter (the Arc) is left out, unless they are the owner', async () => {
  /* As GET /v2/members does (members.rs, BUILD 1 Oct): the role `admitter` is the Arc's node, not a person. */
  const ARC = 'e7'.repeat(32);
  const roles = [[ADMIN, 'admin'], [ARC, 'admitter']];
  for (const bare of [false, true]) {
    const p = page({ bare, roster: [OWNER, ADMIN, KOFI, ARC], roles });
    await settle();
    assert.equal(control(p).textContent, 'Members — 3', bare ? 'without members.js too' : 'owner, admin and member; not the admitter');
    assert.equal(railPeople(p).length, 3, 'and the people listed beneath it agree');
  }
  const p = page({ roster: [OWNER, ADMIN, KOFI, ARC], roles: [...roles, [OWNER, 'admitter']] });
  await settle();
  assert.equal(control(p).textContent, 'Members — 3', 'the owner keeps their place whatever else they hold');
  assert.equal(railPeople(p).length, 3);
  assert.match(railPeople(p)[0].textContent, /owner$/, 'the owner, listed first');
});

test('every member in the route\'s order, as it comes; a name or "New member", never a key', async () => {
  /* The route orders them (owner, admins, the rest by name, nameless last); the page keeps its order. */
  const scrambled = { ...clone(FIXTURE), members: [2, 0, 4, 1, 3].map((i) => clone(FIXTURE.members[i])) };
  const p = await opened({ members: scrambled });
  const cs = cards(p);
  assert.equal(cs.length, 5);
  assert.deepEqual(cs.map((c) => one(c, (n) => has(n, 't')).textContent), ['Kofi', 'Hana', 'New member', 'Ade', 'New member']);
  assert.deepEqual(cs.map((c) => one(c, (n) => has(n, 'mrole')).textContent), ['Member', 'Owner', 'Member', 'Admin', 'Member'], 'ROLE\'s word');
  assert.ok(one(cs[1], (n) => n.tag === 'img' && /^data:image\/webp;base64,/.test(n.attrs.src)), 'Hana\'s icon');
  assert.equal(one(cs[3], (n) => n.tag === 'img'), undefined, 'no icon: a placeholder');
  assert.ok(one(cs[3], (n) => has(n, 'av')), 'the placeholder');
  assert.match(cs[0].textContent, /You/, 'I am marked');
  for (const c of cs) c.onclick();
  const text = pageOf(p).textContent;
  for (const k of [OWNER, ADMIN, KOFI, ANON, BLANK, SITE]) {
    assert.ok(!text.includes(k.slice(0, 8)), 'no key, nor its start, in the page\'s words');
    assert.ok(!all(pageOf(p), (n) => Object.values(n.attrs || {}).some((v) => v.includes(k.slice(0, 8)))).length, 'nor in its attributes');
  }
});

test('intentions are labels: Resources, Skills, Financial Support; nothing else, and never a sentence', async () => {
  const p = await opened();
  const cs = cards(p);
  assert.deepEqual(cs.map(labels), [['Financial Support'], [], ['Resources', 'Skills'], ['Skills'], []], 'an unknown choice is not drawn');
  cs[2].onclick();
  assert.deepEqual(labels(detail(p)), ['Resources', 'Skills'], 'and in the selected member\'s detail');
});

test('a selected member: their about as text, https links alone as links; their answers by label', async () => {
  const p = await opened();
  const hana = cards(p)[0];
  assert.equal(detail(p), undefined, 'nothing open at first');
  hana.onclick();
  const d = detail(p);
  assert.ok(d, 'her detail opens');
  assert.equal(hana.attrs['aria-expanded'], 'true');
  assert.match(d.textContent, /Sound artist and organiser\.\n<img src=x onerror=alert\(1\)>/, 'the bio as it was written');
  assert.ok(!p.writes.html.some((h) => h.includes('onerror')), 'never as HTML');
  const links = all(d, (n) => n.tag === 'a');
  assert.deepEqual(links.map((a) => a.attrs.href), ['https://hana.example/work'], 'only the https link is a link');
  assert.equal(links[0].attrs.rel, 'noopener noreferrer');
  assert.match(d.textContent, /javascript:alert\(1\)/, 'another is shown as text');
  assert.match(d.textContent, /http:\/\/plain\.example\//);
  assert.match(d.textContent, /SEEK/);
  assert.match(d.textContent, /Collaborators for a sound walk/);
  assert.match(d.textContent, /OFFER/);
  assert.match(d.textContent, /Field recording/);
  assert.deepEqual(all(d, (n) => has(n, 'chip')).map((c) => c.textContent), ['TECH / AI', 'SOUND'], 'the choices by their labels, only the ones chosen');
  assert.ok(!/an answer to a retired question/.test(d.textContent), 'a retired question\'s answer is not shown');
  assert.deepEqual(notes(d), [], 'another member\'s detail is only read: no label');
  cards(p)[1].onclick();
  assert.equal(all(pageOf(p), (n) => has(n, 'mem-detail') && visible(n)).length, 1, 'one open at a time');
  assert.equal(hana.attrs['aria-expanded'], 'false');
});

test('no connect or withdraw control, anywhere on the page', async () => {
  const p = await opened(asAdmin());
  for (const c of cards(p)) c.onclick();
  assert.equal(all(pageOf(p), (n) => n.tag === 'button' && /connect|withdraw/i.test(n.textContent)).length, 0);
});

test('my about: base.publishAbout on the Site, the whole record, links as JSON array text, no gen', async () => {
  const p = await opened();
  const box = sec(p, 'about');
  assert.ok(box, 'my about is on the page');
  assert.match(box.textContent, /Carpenter\./, 'as it stands');
  assert.deepEqual(notes(box), [], 'closed, nothing to label');
  assert.equal(field(box, 'bio'), undefined, 'closed until I edit it');
  go(box, 'edit').onclick();
  assert.equal(field(box, 'bio').value, 'Carpenter.', 'my bio as it stands');
  assert.deepEqual(fields(box, 'link').map((f) => f.value), ['https://kofi.example/', '', ''], 'three links at most');
  assert.deepEqual(notes(box), [], 'open, no label: the about label waits on Ralph (UX, 20:15Z)');
  const order = all(box, (n) => has(n, 'note') || n.attrs['data-f'] === 'bio' || n.attrs['data-f'] === 'link');
  assert.equal(order[0], field(box, 'bio'), 'the editor opens on the bio');
  field(box, 'bio').value = '  Carpenter and luthier.\nBrussels.  ';
  fields(box, 'link')[2].value = ' https://kofi.example/guitars ';
  const before = reads(p);
  go(box, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p), [{ object: SITE, op: 'base.publishAbout',
    args: { bio: 'Carpenter and luthier.\nBrussels.', links: '["https://kofi.example/","https://kofi.example/guitars"]' } }]);
  const post = p.calls.find((c) => c.path === '/v2/apply');
  assert.equal(post.method, 'POST');
  assert.equal(post.headers['content-type'], 'application/json');
  assert.equal(reads(p), before + 1, 'after a 200, the route is read again');
  assert.equal(field(sec(p, 'about'), 'bio'), undefined, 'and the editor closes');
});

test('an about emptied is sent whole: bio "" and links "[]"', async () => {
  const p = await opened();
  const box = await editing(p, 'about');
  field(box, 'bio').value = '   ';
  fields(box, 'link')[0].value = '';
  go(box, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p), [{ object: SITE, op: 'base.publishAbout', args: { bio: '', links: '[]' } }]);
});

test('my about\'s caps, before sending: 4096 bytes of UTF-8 for the bio, https links of 2048 characters', async () => {
  const p = await opened();
  const box = await editing(p, 'about');
  field(box, 'bio').value = 'é'.repeat(2048) + 'a';   // 2049 characters, 4097 bytes
  go(box, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p), [], 'over by one byte: not sent');
  assert.ok(why(box).textContent, 'and said why');
  field(box, 'bio').value = 'é'.repeat(2048);   // 4096 bytes
  fields(box, 'link')[1].value = 'http://kofi.example/';
  go(box, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p), [], 'a link that is not https: not sent');
  assert.ok(why(box).textContent);
  fields(box, 'link')[1].value = 'https://kofi.example/' + 'x'.repeat(2048 - 20);   // 2049 characters
  go(box, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p), [], 'a link over 2048 characters: not sent');
  fields(box, 'link')[1].value = '';
  go(box, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p).map((b) => b.args.bio.length), [2048], 'at the cap exactly: sent');
});

test('my answer to a poll: base.answerQuestion, the author and gen from the question\'s id, choices within max, as JSON array text', async () => {
  const p = await opened();
  const box = sec(p, 'answers');
  assert.ok(box, 'my answers are on the page');
  assert.deepEqual(notes(box), [SEE_ANSWER], 'once, for all my answers');
  const heads = all(box, (n) => has(n, 'note') || has(n, 'mem-q'));
  assert.equal(heads[0].textContent, SEE_ANSWER, 'at the head of my answers, above the first question');
  assert.equal(question(box, 'An old question'), undefined, 'a retired question takes no new answers');
  assert.deepEqual(all(box, (n) => has(n, 'mem-q')).map((r) => one(r, (m) => has(m, 'qt')).textContent), ['SEEK', 'OFFER', 'IMAGINE', 'Which night?']);
  const offer = await answering(p, 'OFFER');
  assert.match(offer.textContent, /Up to two/, 'the hint');
  assert.deepEqual(notes(offer), [], 'not by each answer field: once, at the head');
  assert.deepEqual(chips(offer).map((c) => c.textContent), Q.OFFER.options);
  assert.deepEqual(pressed(offer), ['false', 'true', 'false', 'false'], 'my choice as it stands');
  assert.equal(field(offer, 'text').value, 'Joinery');
  chips(offer)[0].onclick();
  chips(offer)[3].onclick();   // a third: max is 2
  assert.deepEqual(pressed(offer), ['true', 'true', 'false', 'false'], 'no more than max');
  assert.ok(why(offer).textContent, 'and said why');
  field(offer, 'text').value = ' Joinery, and repairs ';
  go(offer, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p), [{ object: SITE, op: 'base.answerQuestion',
    args: { ...ref(Q.OFFER), text: 'Joinery, and repairs', choices: '[0,1]' } }]);
  assert.equal(typeof applied(p)[0].args.target_gen, 'number');
});

test('a poll that is not multi takes one choice, and carries no text when it is not free', async () => {
  const p = await opened();
  const night = await answering(p, 'Which night?');
  assert.equal(field(night, 'text'), undefined, 'not free: no text field');
  assert.deepEqual(pressed(night), ['false', 'true']);
  chips(night)[0].onclick();
  assert.deepEqual(pressed(night), ['true', 'false'], 'one replaces the other');
  go(night, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p), [{ object: SITE, op: 'base.answerQuestion', args: { ...ref(Q['Which night?']), choices: '[0]' } }]);
});

test('a free question carries text alone, within its textMax in bytes; an empty answer clears it', async () => {
  const p = await opened();
  const seek = await answering(p, 'SEEK');
  assert.match(seek.textContent, /What are you looking for\?/, 'the hint');
  assert.equal(chips(seek).length, 0, 'no options');
  field(seek, 'text').value = 'é'.repeat(90) + 'a';   // 91 characters, 181 bytes: SEEK's textMax is 180
  go(seek, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p), [], 'over its textMax: not sent');
  assert.ok(why(seek).textContent, 'and said why');
  field(seek, 'text').value = 'é'.repeat(90);
  go(seek, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p), [{ object: SITE, op: 'base.answerQuestion', args: { ...ref(Q.SEEK), text: 'é'.repeat(90) } }], 'at its textMax: sent');

  const p2 = await opened();
  const imagine = await answering(p2, 'IMAGINE');
  field(imagine, 'text').value = 'x'.repeat(4097);   // no textMax: 4096
  go(imagine, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p2), []);
  field(imagine, 'text').value = '';
  go(imagine, 'save').onclick();
  await settle();
  assert.deepEqual(applied(p2), [{ object: SITE, op: 'base.answerQuestion', args: { ...ref(Q.IMAGINE), text: '' } }], 'empty clears');
});

test('a refusal is the Door\'s sentence as it stands; the editor stays, and nothing is read again', async () => {
  const p = await opened({ apply: () => [422, 'base.answerQuestion: that question is retired'] });
  const seek = await answering(p, 'SEEK');
  field(seek, 'text').value = 'x';
  const before = reads(p);
  go(seek, 'save').onclick();
  await settle();
  const row = question(sec(p, 'answers'), 'SEEK');
  assert.equal(why(row).textContent, 'base.answerQuestion: that question is retired');
  assert.equal(field(row, 'text').value, 'x', 'what I wrote is still there');
  assert.equal(reads(p), before);
});

test('the route\'s own refusal is shown in its words', async () => {
  const p = await opened({ members: [404, 'that Site is not in this session\'s reach'] });
  assert.match(pageOf(p).textContent, /that Site is not in this session's reach/);
  assert.equal(cards(p).length, 0);
  assert.equal(sec(p, 'about'), undefined);
});

test('a member who is not the owner or an admin sees no question controls: can_define alone decides', async () => {
  const p = await opened();
  assert.equal(sec(p, 'questions'), undefined);
  for (const g of ['retire', 'new', 'define']) assert.equal(all(pageOf(p), attr('data-go', g)).length, 0);
  /* An admin by role whom the route does not let define: still none. */
  const p2 = await opened({ members: { ...clone(FIXTURE), me: ADMIN, can_define: false }, me: ADMIN });
  assert.equal(sec(p2, 'questions'), undefined);
  const p3 = await opened({ members: { ...clone(FIXTURE), can_define: 'yes' } });
  assert.equal(sec(p3, 'questions'), undefined, 'only true');
});

test('the owner or an admin defines a poll: text, hint, options one per line, multi, max, free, textMax', async () => {
  const p = await opened(asAdmin());
  const box = sec(p, 'questions');
  assert.ok(box, 'can_define: the questions are theirs to set');
  go(box, 'new').onclick();
  field(box, 'qtext').value = '  What do you bring?  ';
  field(box, 'hint').value = ' Up to two ';
  field(box, 'options').value = 'Tools\n\n  Time \nSpace\n';
  field(box, 'multi').checked = true;
  field(box, 'max').value = '2';
  field(box, 'free').checked = true;
  field(box, 'textMax').value = '120';
  const before = reads(p);
  go(box, 'define').onclick();
  await settle();
  assert.deepEqual(applied(p), [{ object: SITE, op: 'base.defineQuestion',
    args: { text: 'What do you bring?', hint: 'Up to two', options: '["Tools","Time","Space"]', multi: 1, max: 2, free: 1, textMax: 120 } }]);
  assert.equal(reads(p), before + 1, 'read again after the 200');
});

test('a question without options is free: text, and its hint and textMax where given; multi and max are not sent', async () => {
  const p = await opened(asAdmin());
  const box = sec(p, 'questions');
  go(box, 'new').onclick();
  field(box, 'qtext').value = 'IMAGINE';
  field(box, 'multi').checked = true;
  field(box, 'max').value = '3';
  field(box, 'textMax').value = '500';
  go(box, 'define').onclick();
  await settle();
  assert.deepEqual(applied(p), [{ object: SITE, op: 'base.defineQuestion', args: { text: 'IMAGINE', free: 1, textMax: 500 } }]);
});

test('a poll with one choice and no free text: multi 0, free 0, and neither max nor textMax', async () => {
  const p = await opened(asAdmin());
  const box = sec(p, 'questions');
  go(box, 'new').onclick();
  field(box, 'qtext').value = 'Which night?';
  field(box, 'options').value = 'Friday\nSaturday';
  field(box, 'max').value = '2';
  field(box, 'textMax').value = '100';
  go(box, 'define').onclick();
  await settle();
  assert.deepEqual(applied(p), [{ object: SITE, op: 'base.defineQuestion', args: { text: 'Which night?', options: '["Friday","Saturday"]', multi: 0, free: 0 } }]);
});

test('a question\'s caps, before sending: text 1024 bytes, hint 200 bytes, 2 to 24 distinct options of 80 characters, max, textMax', async () => {
  const p = await opened(asAdmin());
  const box = sec(p, 'questions');
  go(box, 'new').onclick();
  const tries = [];
  const attempt = (set) => {
    field(box, 'qtext').value = 'Q'; field(box, 'hint').value = ''; field(box, 'options').value = 'A\nB';
    field(box, 'multi').checked = false; field(box, 'max').value = ''; field(box, 'free').checked = false; field(box, 'textMax').value = '';
    set();
    go(box, 'define').onclick();
    tries.push(why(form(box)).textContent);
  };
  attempt(() => { field(box, 'qtext').value = '   '; });
  attempt(() => { field(box, 'qtext').value = '€'.repeat(341) + 'ab'; });   // 1025 bytes
  attempt(() => { field(box, 'hint').value = 'é'.repeat(100) + 'a'; });    // 201 bytes
  attempt(() => { field(box, 'options').value = 'Only one'; });
  attempt(() => { field(box, 'options').value = Array.from({ length: 25 }, (_, i) => 'o' + i).join('\n'); });
  attempt(() => { field(box, 'options').value = 'Same\nSame'; });
  attempt(() => { field(box, 'options').value = 'A\n' + 'x'.repeat(81); });
  attempt(() => { field(box, 'multi').checked = true; field(box, 'max').value = '3'; });   // more than the options
  attempt(() => { field(box, 'multi').checked = true; field(box, 'max').value = '1'; });
  attempt(() => { field(box, 'free').checked = true; field(box, 'textMax').value = '4097'; });
  attempt(() => { field(box, 'free').checked = true; field(box, 'textMax').value = '0'; });
  await settle();
  assert.deepEqual(applied(p), [], 'none of them sent');
  assert.ok(tries.every(Boolean), 'each said why');
  attempt(() => {
    field(box, 'qtext').value = '€'.repeat(341) + 'a';                        // 1024 bytes
    field(box, 'hint').value = 'é'.repeat(100);                              // 200 bytes
    field(box, 'options').value = 'é'.repeat(80) + '\nB';                     // 80 characters, 160 bytes
    field(box, 'multi').checked = true; field(box, 'max').value = '2';
    field(box, 'free').checked = true; field(box, 'textMax').value = '4096';
  });
  await settle();
  assert.equal(applied(p).length, 1, 'at every cap exactly: sent');
});

test('the owner or an admin sees each question\'s tally, and retires an open one, after asking once', async () => {
  const p = await opened(asAdmin());
  const box = sec(p, 'questions');
  const offer = question(box, 'OFFER');
  assert.deepEqual(all(offer, (n) => has(n, 'tally')).map((t) => t.textContent), ['TECH / AI1', 'MAKING / PRODUCTION1', 'SOUND1', 'CARE0']);
  const old = question(box, 'An old question');
  assert.ok(old, 'a retired question is listed');
  assert.match(old.textContent, /Retired/);
  assert.equal(go(old, 'retire'), undefined, 'a retired one is not retired again');
  go(offer, 'retire').onclick();
  assert.deepEqual(applied(p), [], 'asked first');
  go(question(sec(p, 'questions'), 'OFFER'), 'retire-yes').onclick();
  await settle();
  assert.deepEqual(applied(p), [{ object: SITE, op: 'base.retireQuestion', args: ref(Q.OFFER) }]);
});

test('every write is the ICD 2.3.1 draft\'s op, with only its declared args, the required ones, and never a gen', async () => {
  const p = await opened(asAdmin());
  const about = await editing(p, 'about');
  field(about, 'bio').value = 'b';
  go(about, 'save').onclick();
  await settle();
  const offer = question(sec(p, 'answers'), 'OFFER');
  go(offer, 'edit').onclick();
  go(offer, 'save').onclick();
  await settle();
  const qs = sec(p, 'questions');
  go(qs, 'new').onclick();
  field(qs, 'qtext').value = 'Q'; field(qs, 'options').value = 'A\nB\nC'; field(qs, 'multi').checked = true; field(qs, 'max').value = '2';
  field(qs, 'hint').value = 'h'; field(qs, 'free').checked = true; field(qs, 'textMax').value = '9';
  go(qs, 'define').onclick();
  await settle();
  go(question(sec(p, 'questions'), 'SEEK'), 'retire').onclick();
  go(question(sec(p, 'questions'), 'SEEK'), 'retire-yes').onclick();
  await settle();
  const sent = applied(p);
  assert.deepEqual(sent.map((b) => b.op), ['base.publishAbout', 'base.answerQuestion', 'base.defineQuestion', 'base.retireQuestion']);
  const decl = (name) => Object.values(ICD.facets).map((f) => (f.ops || {})[name]).find(Boolean);
  for (const b of sent) {
    const d = decl(b.op);
    assert.ok(d, b.op + ' is the ICD\'s');
    assert.equal(b.object, SITE);
    assert.ok(!('gen' in b.args), b.op + ': no gen');
    for (const [k, v] of Object.entries(b.args)) {
      assert.ok(d.args[k], b.op + ': ' + k + ' is declared');
      assert.equal(typeof v, d.args[k].type === 'integer' ? 'number' : 'string', b.op + ': ' + k + ' is its type');
    }
    for (const [k, a] of Object.entries(d.args)) if (a.required && k !== 'gen') assert.ok(k in b.args, b.op + ': ' + k + ' is sent');
  }
});

test('an open editor is not rebuilt under a member typing when the Door reports a change; closed, the page shows the change', async () => {
  const next = clone(FIXTURE);
  next.members[0].about.bio = 'Hana, a new bio';
  const p = await opened({ members: (n) => (n === 0 ? FIXTURE : next) });
  const page0 = pageOf(p);
  const box = await editing(p, 'about');
  field(box, 'bio').value = 'half-typed';
  const before = reads(p);
  p.changed();
  await settle();
  assert.ok(reads(p) > before, 'the change is read');
  assert.equal(pageOf(p), page0, 'the same page');
  assert.equal(sec(p, 'about'), box, 'the same editor');
  assert.equal(field(sec(p, 'about'), 'bio').value, 'half-typed', 'with what I typed');
  /* Nor is a question's answer, or the define form. */
  const seek = await answering(p, 'SEEK');
  field(seek, 'text').value = 'still typing';
  p.changed();
  await settle();
  assert.equal(field(question(sec(p, 'answers'), 'SEEK'), 'text').value, 'still typing');
  go(question(sec(p, 'answers'), 'SEEK'), 'cancel').onclick();
  assert.equal(field(sec(p, 'about'), 'bio').value, 'half-typed', 'one editor closed, another still open: still kept');
  go(sec(p, 'about'), 'cancel').onclick();
  await settle();
  assert.notEqual(pageOf(p), page0, 'every editor closed: drawn again');
  cards(p)[0].onclick();
  assert.match(detail(p).textContent, /Hana, a new bio/, 'with what the Door now says');
});

test('with no editor open, a change the Door reports is drawn, and the selected member stays selected', async () => {
  const next = clone(FIXTURE);
  next.members[0].about.bio = 'Hana, a new bio';
  const p = await opened({ members: (n) => (n === 0 ? FIXTURE : next) });
  cards(p)[0].onclick();
  p.changed();
  await settle();
  assert.match(detail(p).textContent, /Hana, a new bio/);
});

test('the admin\'s define form is not rebuilt under them either', async () => {
  const p = await opened(asAdmin());
  go(sec(p, 'questions'), 'new').onclick();
  field(sec(p, 'questions'), 'qtext').value = 'half a question';
  p.changed();
  await settle();
  assert.equal(field(sec(p, 'questions'), 'qtext').value, 'half a question');
});

test('labels only: UX\'s note, drawn once at the head of my answers, and the old sentence nowhere', async () => {
  const p = await opened(asAdmin());
  const about = await editing(p, 'about');
  assert.deepEqual(notes(about), [], 'the about editor carries none (it waits on Ralph)');
  const rows = all(sec(p, 'answers'), (n) => has(n, 'mem-q'));
  assert.equal(rows.length, 4);
  for (const row of rows) go(row, 'edit').onclick();
  assert.deepEqual(notes(sec(p, 'answers')), [SEE_ANSWER], 'heading my answers, once, with every answer editor open');
  for (const row of all(sec(p, 'answers'), (n) => has(n, 'mem-q'))) {
    const words = one(row, (m) => has(m, 'qt')).textContent;
    assert.deepEqual(notes(row), [], words + ': no label by its own field');
  }
  go(sec(p, 'questions'), 'new').onclick();
  for (const c of cards(p)) { c.onclick(); assert.deepEqual(notes(detail(p)), [], 'another member, only read: no label'); }
  const text = pageOf(p).textContent;
  assert.ok(!/can read/i.test(text), 'the old sentence appears nowhere on the page');
  assert.deepEqual(notes(pageOf(p)), [SEE_ANSWER], 'one label, once: nothing else labelled');
  assert.ok(!/can read this/.test(MEMBERS), 'nor in members.js\'s words');
  assert.ok(!MEMBERS.includes(SEE_ABOUT), 'nor the about label, until Ralph says yes');
});
