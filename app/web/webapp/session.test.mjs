/* session.test.mjs — a sign-in portal, and a session that ends (NC-91).

   Signed out, the page is the Door's window and nothing else (Ralph, 28 Sep: "its only
   purpose is as a sign in portal ... take the user straight to the signin page"): it goes
   there at once, and the window comes back to the same address, hash and all. A kiosk's
   visitor and www's draft open it on a new account. Any answer but 401 is shown, not
   followed back to the window.

   The Door ends a session that has been idle (DOOR_IDLE_SECS), or signed out elsewhere, and
   answers its cookie 401. The page, left open, goes to the window as signing out does: on a
   read that answers 401, and on its change stream failing for good with the session gone.
   A stream that fails while the session lives listens again. webapp.js runs as the page
   runs it, over a stub page, a stub Door and a stub EventSource.

   Run: node --test app/web/webapp/session.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const ORIGIN = 'https://app.wallflowers.io';

/* A page, its Door (`live` says whether the session stands; `me` what /v2/me answers
   otherwise), every stream it opened, and where it was sent. */
function page({ live = true, hash = '', me = 401, name = '', objects = [] } = {}) {
  const state = { live };
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
    set outerHTML(v) {}
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
  for (const id of ['sheet', 'side', 'refused', 'status', 'compose', 'siteMenu', 'meMenu', 'you', 'pdfv']) $(id).hidden = true;
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const fetch = async (url) => {
    const path = url.slice(ORIGIN.length);
    if (!state.live && path !== '/v2/icd') return me === 401 ? reply(401, 'no such session') : reply(me, 'the Door could not open it');
    const pk = 'ed25519:' + 'c'.repeat(64);
    if (path === '/v2/me') return reply(200, { pk, ...(name ? { display_name: name } : {}) });
    if (path === '/v2/graph') { state.graphs = (state.graphs || 0) + 1; return reply(200, { me: { pk, display_name: name }, objects, spine: [] }); }
    if (path === '/v2/icd') return reply(200, { kinds: {}, facets: {} });
    return reply(200, {});
  };
  const streams = [];
  class EventSource {
    constructor(url) { this.url = url; this.readyState = 1; this.listeners = {}; this.closed = false; streams.push(this); }
    addEventListener(t, f) { this.listeners[t] = f; }
    close() { this.closed = true; this.readyState = 2; }
    /* The browser's end of a stream it will not reopen: a non-200 answer. */
    fail() { this.readyState = 2; this.onerror && this.onerror({}); }
  }
  const waits = [];
  const html = new Node('html');
  const sent = [];
  const loc = { hash, pathname: '/', search: '', origin: ORIGIN, assign(u) { sent.push(u); }, get href() { return ORIGIN + this.pathname + this.search + this.hash; } };
  const ctx = {
    document: { getElementById: $, documentElement: html, createElement: (t) => new Node(t), createTextNode: (t) => ({ textContent: String(t) }), addEventListener() {} },
    location: loc, history: { replaceState(s, t, u) { loc.hash = u.includes('#') ? u.slice(u.indexOf('#')) : ''; } }, fetch, EventSource, doorOrigin: () => ORIGIN,
    addEventListener() {}, scrollTo() {},
    innerWidth: 1280, matchMedia: () => ({ matches: false, addEventListener() {} }), requestAnimationFrame: (f) => setImmediate(f),
    atob, URL, TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, console,
    setTimeout: (f, ms) => { waits.push(ms); return setImmediate(f); }, clearTimeout() {}
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  return { state, streams, waits, sent, $, loc, at: () => html.getAttribute('data-state') };
}

const settle = async () => { for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r)); };

/* Where the page sent the person: the window, and what it returns to. */
function windowOf(u) {
  const w = new URL(u);
  return { at: w.origin + w.pathname, fresh: w.searchParams.has('new'), back: w.searchParams.get('return') };
}

test('signed out, straight to the window: nothing drawn, and it comes back here', async () => {
  for (const hash of ['', '#signin', '#site=' + 'a'.repeat(64), '#you']) {
    const p = page({ live: false, hash });
    await settle();
    assert.equal(p.sent.length, 1, hash);
    assert.deepEqual(windowOf(p.sent[0]), { at: ORIGIN + '/signin', fresh: false, back: '/' + hash }, hash);
    assert.equal(p.at(), null, 'no face, no door: nothing but the boot line');
  }
});

test("a kiosk's visitor and www's draft open the window on a new account", async () => {
  for (const hash of ['#join', '#register=eyJuYW1lIjoiWCJ9']) {
    const p = page({ live: false, hash });
    await settle();
    assert.deepEqual(windowOf(p.sent[0]), { at: ORIGIN + '/signin', fresh: true, back: '/' + hash }, hash);
  }
});

test('any answer but 401 is the Door\'s own words, not a round trip back to the window', async () => {
  const p = page({ live: false, me: 503 });
  await settle();
  assert.deepEqual(p.sent, [], 'not sent away');
  assert.match(p.$('boot').textContent, /could not open it/);
});

test('signing out goes to the window', async () => {
  const p = page();
  await settle();
  // Sign out is in the profile menu, top right: opened, found there and pressed.
  p.$('meBtn').onclick();
  const find = (n) => (n.tag === 'button' && n.textContent === 'Sign out' ? n : (n.children || []).map(find).find(Boolean));
  find(p.$('meMenu')).onclick();
  await settle();
  assert.equal(p.sent.length, 1);
  assert.equal(windowOf(p.sent[0]).at, ORIGIN + '/signin');
  assert.ok(p.streams.every((s) => s.closed), 'its stream closed');
});

test('an ended session, met by a read, goes to the window', async () => {
  const p = page();
  await settle();
  assert.equal(p.at(), 'inside', 'a live cookie is straight in');
  p.state.live = false;
  p.streams.at(-1).listeners.changed();
  await settle();
  assert.equal(p.sent.length, 1, 'the 401 is the window, not a page that cannot write');
  assert.deepEqual(windowOf(p.sent[0]), { at: ORIGIN + '/signin', fresh: false, back: '/' });
  assert.notEqual(p.at(), 'inside');
  assert.ok(p.streams.every((s) => s.closed), 'and its stream is closed');
});

test('an ended session, met by its stream, goes to the window', async () => {
  const p = page();
  await settle();
  p.state.live = false;
  p.streams.at(-1).fail();
  await settle();
  assert.equal(p.sent.length, 1);
  assert.equal(windowOf(p.sent[0]).at, ORIGIN + '/signin');
  assert.notEqual(p.at(), 'inside');
});

test('a stream that fails while the session lives listens again', async () => {
  const p = page();
  await settle();
  const first = p.streams.length;
  p.streams.at(-1).fail();
  await settle();
  assert.equal(p.at(), 'inside', 'still in');
  assert.equal(p.streams.length, first + 1, 'a new stream');
});

test('a stream that keeps failing is asked after less and less often, to a minute', async () => {
  const p = page();
  await settle();
  const relisten = [];
  for (let i = 0; i < 7; i++) {
    p.waits.length = 0;
    p.streams.at(-1).fail();
    await settle();
    relisten.push(Math.max(...p.waits));
  }
  assert.deepEqual(relisten, [2000, 4000, 8000, 16000, 32000, 60000, 60000]);
  // Once a stream opens, the next failure waits the least again.
  p.streams.at(-1).onopen();
  p.waits.length = 0;
  p.streams.at(-1).fail();
  await settle();
  assert.equal(Math.max(...p.waits), 2000);
});

test('#you opens Your card, once; signed out, the window returns to it', async () => {
  const p = page({ name: 'Ana', hash: '#you' });
  await settle();
  assert.equal(p.$('you').hidden, false, 'a Site\'s page sends a member to their card');
  assert.equal(p.loc.hash, '', 'and a reload does not open it again');
  const plain = page({ name: 'Ana' });
  await settle();
  assert.equal(plain.$('you').hidden, true, 'without #you, a named member lands as ever');
  const out = page({ live: false, hash: '#you' });
  await settle();
  assert.deepEqual(windowOf(out.sent[0]), { at: ORIGIN + '/signin', fresh: false, back: '/#you' }, 'the same account, back to the card');
});

/* A member's link is a link only as a web address (SECURITY, 29 Sep): core checks a post's
   link for length alone, and an href is where a `javascript:` link would run as this origin. */
test("a member's link that is not http(s) is never made a link", async () => {
  const post = (id, form, link) => ({ id: id.repeat(64), kind: 'post', name: form + ' post', folds: true,
    view: { form, title: form + ' post', link, body: '', icon: '', banner: '', parts: [] } });
  const tree = (n, out = []) => { if (n.attrs && n.attrs.href) out.push(n.attrs.href); (n.children || []).forEach((c) => tree(c, out)); return out; };
  // and a picture or a film only from a web address, or the webapp's own data:image
  const media = (n, out = []) => { if (n.attrs && n.attrs.src && !/^(https?:|data:image\/)/i.test(n.attrs.src)) out.push(n.attrs.src); (n.children || []).forEach((c) => media(c, out)); return out; };
  const find = (n, pred) => (pred(n) ? n : (n.children || []).map((c) => find(c, pred)).find(Boolean));
  const evil = 'javascript:alert(document.cookie)//';
  for (const [form, link] of [['image', evil + 'x.png'], ['link', evil], ['pdf', evil + 'x.pdf'], ['link', 'JavaScript:alert(1)'], ['image', 'data:text/html,<script>alert(1)</script>'], ['link', evil + 'x.mp4']]) {
    const p = page({ name: 'Ana', objects: [post('b', form, link)] });
    await settle();
    // its own page, from the left rail
    find(p.$('lbody'), (n) => typeof n.onclick === 'function').onclick();
    await settle();
    assert.deepEqual(tree(p.$('feed')).filter((h) => !/^https?:/i.test(h)), [], `${form} ${link}: no link but a web address`);
    assert.deepEqual(media(p.$('feed')), [], `${form} ${link}: no picture or film from it`);
    if (form === 'pdf') {
      // the viewer, from the Resources tab's rail
      find(p.$('tabs'), (n) => n.tag === 'button' && n.textContent === 'Resources').onclick();
      await settle();
      find(p.$('lbody'), (n) => typeof n.onclick === 'function' && n.className !== 'newbtn').onclick();
      await settle();
      assert.equal(p.$('pdfv').hidden, false, 'the viewer opened');
      assert.ok(!/^\s*javascript:/i.test(p.$('pdfOpen').href || ''), 'the viewer offers no such link');
      assert.equal(p.$('pdfOpen').hidden, true, 'nor its Open button');
    }
  }
  const good = page({ name: 'Ana', objects: [post('d', 'link', 'https://egregores-echoes.com/healing')] });
  await settle();
  find(good.$('lbody'), (n) => typeof n.onclick === 'function').onclick();
  await settle();
  assert.ok(tree(good.$('feed')).includes('https://egregores-echoes.com/healing'), 'a web address is still a link');
});

/* NC-134: the Door sends nothing when a stream opens, so what changed while it was down is read
   once it is back: a reply that came during a blip is drawn, not left for the next change. The
   first open reads nothing more: the landing has just read the graph. */
test('a stream that reopens reads what it missed; the first open reads nothing more', async () => {
  const objects = [];
  const p = page({ name: 'Ana', objects });
  await settle();
  const landed = p.state.graphs;
  p.streams.at(-1).onopen();
  await settle();
  assert.equal(p.state.graphs, landed, 'the first open: no second read');
  objects.push({ id: 'e'.repeat(64), kind: 'post', name: 'Missed while down', folds: true,
    view: { form: 'article', title: 'Missed while down', body: '', parts: [] } });
  p.streams.at(-1).fail();
  await settle();
  const reopened = p.streams.at(-1);
  reopened.onopen();
  await settle();
  assert.match(p.$('feed').textContent, /Missed while down/, 'drawn once the stream is back');
  // and the browser's own reconnect, the same stream opening again, reads once too
  const before = p.state.graphs;
  reopened.onopen();
  await settle();
  assert.equal(p.state.graphs, before + 1, 'a reconnect reads once');
});
