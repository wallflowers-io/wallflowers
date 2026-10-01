/* register.test.mjs — O-58: a Site registered from www's `#register=<draft>`.

   The draft arrives at the Door's own origin from a link anyone can craft, so it is
   hostile data (Software Security): drawn as text only, its face held to the Face's
   rules before it is set, the whole refused unread above 32 KiB, and a Site made only
   on Register. webapp.js runs as the page runs it, over a stub page and a stub Door.

   Run: node --test app/web/webapp/register.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const ORIGIN = 'https://app.wallflowers.io';
const SITE = 'a'.repeat(64), HOST = 'b'.repeat(64), SELF = 'e'.repeat(64);
const ARC = 'https://arc.wallflowers.io', WWW = 'https://wallflowers.io';
const BUNDLE = 'wf1' + 'e'.repeat(60), ARCKEY = 'f'.repeat(64);

const b64u = (s) => Buffer.from(s, 'utf8').toString('base64url');
const draft = (d) => b64u(JSON.stringify({ kind: 'Community', name: 'Riverside', purpose: '', slug: 'riverside', pname: '', face: null, ...d }));
const LOOK = { colours: { background: '#FAF8F3', text: '#14181E', cards: '#FFFFFF', buttons: '#546CAC', buttonText: '#FFFFFF', title: '#14181E' },
  wallpaper: { style: 'fill', colour2: '#FAF8F3', colour3: '#FAF8F3', angle: 'down', pattern: 'dots', image: '', soften: 30 },
  blocks: { style: 'solid', corners: 3, shadow: 'soft' }, text: { font: 'system-sans', title: 'marker' } };

/* A page: every element the script names, and what was written into it. */
function page({ hash, signedIn = true, answers = {}, look = true, name = '', bundle = BUNDLE, self = false }) {
  const writes = { html: [], attrs: [], styles: [] };
  class Node {
    constructor(tag) {
      Object.assign(this, { tag, children: [], _text: '', hidden: false, disabled: false, value: '', className: '', attrs: {}, scrollTop: 0 });
      const self = this;
      this.style = { setProperty(k, v) { writes.styles.push(String(v)); }, removeProperty() {} };
      this.classList = { add() {}, remove() {}, toggle() {}, contains() { return false; } };
      void self;
    }
    get textContent() { return this._text + this.children.map((c) => c.textContent).join(''); }
    set textContent(v) { this._text = String(v); this.children = []; }
    set innerHTML(v) { writes.html.push(String(v)); this._html = String(v); }
    get innerHTML() { return this._html || ''; }
    set outerHTML(v) { writes.html.push(String(v)); }
    setAttribute(k, v) { this.attrs[k] = String(v); writes.attrs.push(String(v)); }
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
  // Hidden as the markup has them, so "shown" is something the script did.
  for (const id of ['sheet', 'side', 'refused', 'status', 'compose', 'you', 'youX']) $(id).hidden = true;
  const calls = [];
  let assigned = null;
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const graph = { objects: [], spine: [], ...(name ? { me: { pk: 'ed25519:' + 'c'.repeat(64), display_name: name } } : {}) };
  // The person's own record (spine index 0), when the page holds one; a group.setProfile on
  // it names them, as the Door's graph then answers.
  if (self) {
    graph.spine.push({ object: SELF });
    graph.objects.push({ id: SELF, kind: 'group', name: 'you', folds: true, view: { display_name: name, shape: 'individual' } });
  }
  const fetch = async (url, init = {}) => {
    /* The Arc's own origin (W-103): a fresh contact bundle, over CORS. */
    if (url.startsWith(ARC)) {
      calls.push({ path: url.slice(ARC.length), arc: true });
      return typeof bundle === 'function' ? bundle() : reply(200, bundle);
    }
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
        if (s.do === 'mint') {
          const id = s.kind === 'group' ? SITE : HOST;
          graph.objects.push({ id, kind: s.kind, name: s.draft.name, folds: true, view: { title: s.draft.name } });
          got.push(id);
        } else got.push(b.object);
      }
      return reply(200, { made: got, refused: null });
    }
    const a = answers[path];
    if (typeof a === 'function') { const r = a(body); if (r) return reply(r[0], r[1]); }
    if (path === '/v2/apply' && body.object === SELF && body.op === 'group.setProfile') {
      graph.me = { pk: 'ed25519:' + 'c'.repeat(64), display_name: body.args.displayName };
      return reply(200, { delta: 'd'.repeat(64) });
    }
    if (path === '/v2/me') return signedIn ? reply(200, { pk: 'ed25519:' + 'c'.repeat(64) }) : reply(401, 'not signed in');
    if (path === '/v2/graph') return reply(200, graph);
    if (path === '/v2/icd') return reply(200, { kinds: {}, facets: {} });
    if (path === '/v2/add') return reply(200, { member: ARCKEY });
    if (path === '/v2/mint') {
      const id = body.kind === 'group' ? SITE : HOST;
      graph.objects.push({ id, kind: body.kind, name: body.draft.name, folds: true, view: { title: body.draft.name } });
      return reply(200, { object_id: id });
    }
    return reply(200, {});
  };
  const loc = { hash, pathname: '/', search: '', origin: ORIGIN, assign(u) { assigned = u; calls.push({ path: 'assign', body: u }); } };
  const history = { states: [], replaceState(_, __, u) { this.states.push(u); const i = u.indexOf('#'); loc.hash = i < 0 ? '' : u.slice(i); } };
  const ctx = {
    document: {
      getElementById: $, documentElement: new Node('html'),
      createElement: (t) => new Node(t), createTextNode: (t) => ({ textContent: String(t) }),
      addEventListener() {}
    },
    location: loc, history, fetch, doorOrigin: () => ORIGIN,
    addEventListener() {}, scrollTo() {},
    innerWidth: 1280, matchMedia: () => ({ matches: false, addEventListener() {} }), requestAnimationFrame: (f) => setImmediate(f),
    WallFlowers: look ? { Look: { FONTS: [{ id: 'marker' }, { id: 'system-sans' }, { id: 'system-serif' }], apply() { return {}; } } } : undefined,
    atob, btoa, TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, console,
    setTimeout: (f) => setImmediate(f), clearTimeout() {}
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  return { $, calls, writes, assigned: () => assigned, history, loc, ctx };
}

const settle = async () => { for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r)); };
const made = (p) => p.calls.filter((c) => c.path === '/v2/mint' || c.path === '/v2/apply' || c.path === '/v2/site/address');
const press = async (p) => { p.$('sheetForm').onsubmit({ preventDefault() {} }); await settle(); };
const leaked = (p, needle) => [...p.writes.html, ...p.writes.attrs, ...p.writes.styles].filter((w) => w.includes(needle));

test('signed in: the draft is shown, name and address, and nothing is made until Register', async () => {
  const p = page({ hash: '#register=' + draft({}) });
  await settle();
  assert.equal(p.$('sheet').hidden, false);
  assert.equal(p.$('sheetTitle').textContent, 'Riverside');
  assert.match(p.$('sheetFields').textContent, /address\s*wallflowers\.io\/riverside/);
  assert.equal(p.$('sheetGo').textContent, 'Register');
  assert.deepEqual(made(p), [], 'a link prefills and never creates');
});

test('Register makes the Site as www did, then clears the address bar and opens it', async () => {
  const face = JSON.stringify({ look: LOOK });
  const p = page({ hash: '#register=' + draft({ kind: 'Business', name: '  Riverside  ', slug: ' riverside ', face }) });
  await settle();
  await press(p);
  assert.deepEqual(made(p).map((c) => [c.path, c.body.kind || c.body.op || c.body.slug]), [
    ['/v2/mint', 'group'], ['/v2/mint', 'host'], ['/v2/apply', 'base.setPart'], ['/v2/apply', 'base.setParent'],
    ['/v2/apply', 'group.setFace'], ['/v2/site/address', 'riverside'],
    ['/v2/apply', 'host.hydrate'], ['/v2/apply', 'base.publish']
  ]);
  const [site, host, part, parent, setFace, address] = made(p).map((c) => c.body);
  assert.deepEqual(site.draft, { name: 'Riverside', shape: 'organisation' });
  assert.deepEqual(host.draft, { name: 'Riverside' });
  assert.deepEqual([part.object, part.args.part, part.args.role], [SITE, HOST, 'host']);
  assert.deepEqual([parent.object, parent.args.parent, parent.args.role], [HOST, SITE, 'host']);
  assert.equal(part.args.at, parent.args.at, 'the edge is one moment from both ends');
  assert.deepEqual([setFace.object, setFace.args.face], [SITE, face]);
  assert.deepEqual(address, { slug: 'riverside', host: HOST });
  const requests = p.calls.filter((c) => !c.batched && ['/v2/batch', '/v2/mint', '/v2/apply', '/v2/add', '/v2/site/address'].includes(c.path));
  assert.deepEqual(requests.map((c) => c.path), ['/v2/batch', '/v2/site/address', '/v2/add', '/v2/apply', '/v2/apply'],
    'the Site in one request, its address, then the Arc and the page live');
  assert.equal(p.loc.hash, '', 'the draft leaves the address bar');
  assert.equal(p.$('sheet').hidden, true);
  assert.match(p.$('siteBtn').textContent, /Riverside/, 'the new Site is open');
});

test('a refused address, or a face that fails, is said, and the Site stands', async () => {
  const bad = JSON.stringify({ look: { ...LOOK, colours: { ...LOOK.colours, background: 'red;} body{display:none' } } });
  const p = page({
    hash: '#register=' + draft({ face: bad }),
    answers: { '/v2/site/address': () => [409, 'riverside is taken'] }
  });
  await settle();
  await press(p);
  assert.ok(!made(p).some((c) => c.body.op === 'group.setFace'), 'a face that fails is not set');
  // the address is asked again (below); closed, the Site opens without it, and says so
  p.$('sheetX').onclick();
  await settle();
  const feed = p.$('feed').textContent;
  assert.match(feed, /address · riverside/);
  assert.match(feed, /riverside is taken/);
  assert.match(feed, /face ·/);
  assert.match(feed, /colours\.background/);
  assert.match(p.$('siteBtn').textContent, /Riverside/, 'the Site stands, opened');
});

test('with no renderer loaded, a face in its built-in fonts is still set; an unknown font is not', async () => {
  for (const [font, set] of [['system-serif', true], ['a-font-nobody-has', false]]) {
    const face = JSON.stringify({ look: { ...LOOK, text: { font, title: '' } } });
    const p = page({ hash: '#register=' + draft({ face }), look: false });
    await settle();
    await press(p);
    assert.equal(made(p).some((c) => c.body.op === 'group.setFace'), set, font);
  }
});

test('a Register pressed again after a failure makes nothing twice', async () => {
  let fail = true;
  const p = page({
    hash: '#register=' + draft({}),
    answers: { '/v2/apply': (b) => (b.op === 'base.setParent' && fail ? (fail = false, [503, 'the relay is away']) : null) }
  });
  await settle();
  await press(p);
  assert.equal(p.$('sheetWhy').textContent, 'the relay is away');
  assert.equal(p.$('sheet').hidden, false);
  await press(p);
  const ops = made(p).map((c) => c.body.kind || c.body.op || 'address');
  assert.deepEqual(ops, ['group', 'host', 'base.setPart', 'base.setParent', 'base.setParent', 'address', 'host.hydrate', 'base.publish']);
  assert.equal(p.$('sheet').hidden, true);
});

test('a hostile draft is drawn as text and reaches no markup, attribute or style', async () => {
  const name = '<img src=x onerror=alert(1)>';
  const face = JSON.stringify({ look: { ...LOOK, wallpaper: { ...LOOK.wallpaper, style: 'image', image: 'javascript:alert(2)' } },
    widgets: [{ type: 'text', text: '<script>alert(3)</script>' }] });
  const p = page({ hash: '#register=' + draft({ name, slug: '"><svg onload=alert(4)>', face }) });
  await settle();
  assert.equal(p.$('sheetTitle').textContent, name, 'the name, as text');
  for (const needle of ['onerror', '<script', 'javascript:', 'onload', 'alert(']) {
    assert.deepEqual(leaked(p, needle), [], `${needle} reached markup, an attribute or a style`);
  }
  await press(p);
  assert.ok(!made(p).some((c) => c.body.op === 'group.setFace'), 'a face whose picture is fetched from a URL is not set');
  assert.match(p.$('feed').textContent, /wallpaper\.image/);
  // Made, the name is the Site's, as the Door folds it, drawn as any Site's name is:
  // text, and setAttribute for a title, which no parser reads. Never markup or a style.
  const markup = (n) => [...p.writes.html, ...p.writes.styles].filter((w) => w.includes(n));
  for (const needle of ['onerror', '<script', 'javascript:', 'onload', 'alert(']) {
    assert.deepEqual(markup(needle), [], `${needle} reached markup or a style after Register`);
  }
});

test('a wallpaper image is a plain path here or an image data URL, and nothing that closes its quote', async () => {
  for (const [image, set] of [
    ['a"), url("https://tracker.example/x', false],
    ['https://tracker.example/x.png', false],
    ['//tracker.example/x.png', false],
    ['data:image/svg+xml;base64,PHN2Zz4=', false],
    ['data URL, 1234 chars', false],
    ['assets/face/wall.webp', true],
    ['data:image/png;base64,iVBORw0KGgo=', true],
    ['', true]
  ]) {
    const face = JSON.stringify({ look: { ...LOOK, wallpaper: { ...LOOK.wallpaper, style: 'image', image } } });
    const p = page({ hash: '#register=' + draft({ face }) });
    await settle();
    await press(p);
    assert.equal(made(p).some((c) => c.body.op === 'group.setFace'), set, image);
    if (!set) assert.match(p.$('feed').textContent, /wallpaper\.image/, image);
  }
});

test('an oversized draft is refused unread, and a face over 16 KiB is not set', async () => {
  const huge = page({ hash: '#register=' + 'A'.repeat(32769) });
  await settle();
  assert.equal(huge.$('sheet').hidden, true, 'no Register for a refused draft');
  assert.match(huge.$('feed').textContent, /register ·.*over 32 KiB/s);
  assert.deepEqual(made(huge), []);

  const fat = JSON.stringify({ look: LOOK, pad: 'x'.repeat(16400) });
  const p = page({ hash: '#register=' + draft({ face: fat }) });
  await settle();
  await press(p);
  assert.ok(!made(p).some((c) => c.body.op === 'group.setFace'));
  assert.match(p.$('feed').textContent, /over 16384 bytes/);
});

test('not a draft is said, and nothing is offered', async () => {
  for (const h of ['#register=@@@', '#register=' + b64u('[1,2]'), '#register=' + b64u(JSON.stringify({ name: '   ' }))]) {
    const p = page({ hash: h });
    await settle();
    assert.equal(p.$('sheet').hidden, true, h);
    assert.match(p.$('feed').textContent, /register ·/, h);
    assert.deepEqual(made(p), [], h);
  }
});

test('signed out: straight to the window, on a new account, and it returns to the draft', async () => {
  const d = draft({});
  const p = page({ hash: '#register=' + d, signedIn: false });
  await settle();
  assert.notEqual(p.ctx.document.documentElement.getAttribute('data-state'), 'inside');
  const u = new URL(p.assigned());
  assert.equal(u.origin + u.pathname, ORIGIN + '/signin');
  assert.ok(u.searchParams.has('new'));
  assert.equal(u.searchParams.get('return'), '/#register=' + d, 'the window comes back to the draft');
  assert.deepEqual(made(p), []);
});

// The pictures are the Host's media (host.setMedia), never text in the face. www's
// Face.document() wrote a description in a picture's place, and that was stored as a mark.
const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
const WEBP = 'data:image/webp;base64,UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==';
const setMedia = (p) => made(p).filter((c) => c.body.op === 'host.setMedia').map((c) => [c.body.object, c.body.args.slot, c.body.args.mediaMime, c.body.args.media]);

test('a picture described, not carried, leaves the face and is named, and nothing is set in its slot', async () => {
  const face = JSON.stringify({ look: LOOK, mark: 'data URL, 46303 chars', header: { layout: 'hero', title: 'text', logo: 'data URL, 812 chars', banner: '' } });
  const p = page({ hash: '#register=' + draft({ face }) });
  await settle();
  await press(p);
  const set = JSON.parse(made(p).find((c) => c.body.op === 'group.setFace').body.args.face);
  assert.equal(set.mark, '', 'the description is not stored as the mark');
  assert.equal(set.header.logo, '');
  assert.deepEqual(set.look, LOOK, 'the rest of the face is kept');
  assert.deepEqual(setMedia(p), [], 'no picture was carried, so none is set');
  assert.match(p.$('feed').textContent, /not carried: mark, logo/);
});

test('a picture carried is set on the Host by host.setMedia, one per slot, the draft\'s own first', async () => {
  const face = JSON.stringify({ look: LOOK, mark: '', header: { layout: 'banner', title: 'text', logo: '', banner: WEBP } });
  const p = page({ hash: '#register=' + draft({ face, media: { mark: PNG, cover: PNG } }) });
  await settle();
  await press(p);
  assert.deepEqual(setMedia(p), [
    [HOST, 'mark', 'image/png', PNG.split(',')[1]],
    [HOST, 'cover', 'image/png', PNG.split(',')[1]]
  ], 'the draft\'s cover is set; the face\'s banner is not set twice');
  const set = JSON.parse(made(p).find((c) => c.body.op === 'group.setFace').body.args.face);
  assert.equal(set.header.banner, '', 'the banner leaves the face for its slot');
  const requests = p.calls.filter((c) => !c.batched && ['/v2/batch', '/v2/mint', '/v2/apply', '/v2/add', '/v2/site/address'].includes(c.path));
  assert.deepEqual(requests.map((c) => c.path), ['/v2/batch', '/v2/site/address', '/v2/add', '/v2/apply', '/v2/apply'],
    'the pictures ride the one batch, and the page goes live after the address');
  assert.doesNotMatch(p.$('feed').textContent, /not carried/);
});

// (A picture over host.setMedia's 156,000 cannot reach here: the draft is refused unread over
// 32 KiB, a stricter cap. MEDIA_MAX holds for any other way a picture comes.)
test('a gif, a path, an SVG or a description is not set, and is named', async () => {
  const media = { mark: 'data:image/gif;base64,R0lGODlhAQABAAAAACw=', cover: '/assets/cover.png', logo: 'data:image/svg+xml;base64,PHN2Zz4=', wallpaper: 'data URL, 5 chars' };
  const p = page({ hash: '#register=' + draft({ face: JSON.stringify({ look: LOOK }), media }) });
  await settle();
  await press(p);
  assert.deepEqual(setMedia(p), []);
  assert.match(p.$('feed').textContent, /not carried: mark, cover, logo, wallpaper/);
});

/* The owner's card (O-77) once their Site is made: the landing lets the draft's sheet go first,
   so Register ends on Your card, the first one (no close), with the name they gave on www. */
test("Register ends on the owner's first card, with the name given on www", async () => {
  const p = page({ hash: '#register=' + draft({ pname: '  Ana Kim  ' }) });
  await settle();
  assert.equal(p.$('you').hidden, true, 'not over the draft');
  await press(p);
  assert.equal(p.$('you').hidden, false, 'Your card, after Register');
  assert.equal(p.$('youX').hidden, true, 'the first card: no close');
  const find = (n, pred) => (pred(n) ? n : (n.children || []).map((c) => find(c, pred)).find(Boolean));
  assert.equal(find(p.$('youBody'), (n) => n.attrs && n.attrs.id === 'youName').attrs.value, 'Ana Kim');
});

test('someone who already has a name is not asked again after Register', async () => {
  const p = page({ hash: '#register=' + draft({ pname: 'Ana Kim' }), name: 'Ana' });
  await settle();
  await press(p);
  assert.equal(p.$('you').hidden, true);
});

/* An address taken between www's check and Register (UX-C's sign-up map, 29 Sep): the Site
   stands, and the sheet stays with the address to change. Register again claims that address
   and nothing else, since what was made is kept. Closed instead, the Site opens without it. */
const findIn = (n, pred) => (pred(n) ? n : (n.children || []).map((c) => findIn(c, pred)).find(Boolean));
test('a refused address keeps the sheet, and Register again tries the address alone', async () => {
  let asked = 0;
  const p = page({
    hash: '#register=' + draft({}),
    answers: { '/v2/site/address': () => (++asked === 1 ? [409, 'the auth service refused the address (409 Conflict): {"error":"held"}'] : null) }
  });
  await settle();
  await press(p);
  assert.equal(p.$('sheet').hidden, false, 'the sheet stays');
  assert.match(p.$('sheetWhy').textContent, /wallflowers\.io\/riverside is taken/);
  const field = findIn(p.$('sheetFields'), (n) => n.attrs && n.attrs.id === 'regSlug');
  assert.ok(field, 'the address, to change');
  assert.equal(p.$('sheetX').textContent, 'Later', 'closing now opens the Site without it');
  const before = made(p).length;
  field.value = 'riverside-two';
  await press(p);
  const again = made(p).slice(before);
  assert.deepEqual(again.map((c) => [c.path, c.body.slug || c.body.op]), [
    ['/v2/site/address', 'riverside-two'], ['/v2/apply', 'host.hydrate'], ['/v2/apply', 'base.publish']
  ], 'the address alone, then the page live at it');
  assert.equal(p.$('sheet').hidden, true);
  assert.match(p.$('siteBtn').textContent, /Riverside/, 'the Site opens');
  assert.doesNotMatch(p.$('feed').textContent, /address ·/, 'no refusal left to say');
});

test('an address that is no shape is said before anything is asked', async () => {
  const p = page({ hash: '#register=' + draft({}), answers: { '/v2/site/address': () => [409, 'taken'] } });
  await settle();
  await press(p);
  const field = findIn(p.$('sheetFields'), (n) => n.attrs && n.attrs.id === 'regSlug');
  assert.ok(field, 'the address, to change');
  const before = made(p).length;
  field.value = '-no-';
  await press(p);
  assert.equal(made(p).length, before, 'nothing asked');
  assert.match(p.$('sheetWhy').textContent, /letters, numbers and hyphens/i);
});

/* W-103, Ralph's P1: Register puts the page live itself. Runbook § 3b was a console paste —
   the Arc added to the Host with a fresh bundle, the Site's face copied onto it, the Host
   published at the address with the Arc as its publisher (hack-seoul.js d0efef5d). Each step
   once, as the rest of Register is, and a refusal is the Door's or the Arc's own words. */
const b64decode = (s) => JSON.parse(Buffer.from(s, 'base64url').toString('utf8'));
const APP = { port: 5173, deployed: null, path: '/' };
const liveCalls = (p) => p.calls.filter((c) => c.arc || c.path === '/v2/add'
  || (c.path === '/v2/apply' && ['host.hydrate', 'base.publish'].includes(c.body.op)));

test('Register puts the page live: the Arc on the Host, the face onto it, published at the address', async () => {
  const face = JSON.stringify({ look: LOOK });
  const p = page({ hash: '#register=' + draft({ face }) });
  await settle();
  await press(p);
  assert.deepEqual(liveCalls(p).map((c) => c.path === '/v2/apply' ? c.body.op : c.path),
    ['/v1/bundle', '/v2/add', 'host.hydrate', 'base.publish'], 'the sequence, in order');
  const [, add, hydrate, publish] = liveCalls(p).map((c) => c.body);
  assert.deepEqual(add, { object: HOST, bundle: BUNDLE }, 'the Arc joins the Host by its fresh bundle');
  assert.equal(hydrate.object, HOST);
  assert.equal(hydrate.args.key, 'face');
  assert.deepEqual(JSON.parse(hydrate.args.payload), { v: 1, profile: { displayName: 'Riverside', card: { note: '', urls: [] } }, face },
    'the public bundle the renderer reads');
  assert.deepEqual([publish.object, publish.args.slug, publish.args.publisher], [HOST, 'riverside', ARCKEY]);
  // After the address, never before it: there is nothing to publish at until it is claimed.
  const paths = p.calls.filter((c) => !c.batched).map((c) => c.path);
  assert.ok(paths.indexOf('/v2/site/address') < paths.indexOf('/v2/add'), 'the address first');
  assert.equal(p.$('sheet').hidden, true);
});

test('a Site with no face publishes with its profile', async () => {
  const p = page({ hash: '#register=' + draft({ face: null }) });
  await settle();
  await press(p);
  const hydrate = liveCalls(p).find((c) => c.body && c.body.op === 'host.hydrate');
  assert.equal(JSON.parse(hydrate.body.args.payload).face, null, 'no face, and the profile still goes');
  assert.ok(liveCalls(p).some((c) => c.body && c.body.op === 'base.publish'), 'and it is published');
});

test('the dev page gets its Site back; without app, nothing returns', async () => {
  const p = page({ hash: '#register=' + draft({ app: APP }) });
  await settle();
  await press(p);
  const u = p.assigned();
  assert.ok(u && u.startsWith(WWW + '/signup#registered='), 'back to the page it came from: ' + u);
  assert.deepEqual(b64decode(u.split('#registered=')[1]), { site: SITE, slug: 'riverside', app: APP });
  /* LAST: a navigation begun before the chain is done abandons what it still owes (the Site's
     load, and whatever else Register writes — BW-E's name among them, 1 Oct). */
  const order = p.calls.map((c) => c.path), at = order.indexOf('assign');
  assert.ok(order.indexOf('/v2/graph') >= 0 && order.indexOf('/v2/graph') < at, 'the Site is loaded before the page is left');
  const writes = ['/v2/batch', '/v2/mint', '/v2/apply', '/v2/add', '/v2/site/address'];
  assert.deepEqual(order.slice(at + 1).filter((x) => writes.includes(x)), [], 'nothing is written after it');

  const q = page({ hash: '#register=' + draft({}) });
  await settle();
  await press(q);
  assert.equal(q.assigned(), null, 'no app in the draft is today behaviour exactly');
});

/* A PAGE THAT DID NOT GO LIVE IS NOTED, NOT A WALL (MANAGE, 1 Oct). The Arc away — or its
   /v1/bundle without the CORS header, which a browser reports as "Failed to fetch" with no
   words of ours in it — must not hold a person at a sheet whose Site is already made and whose
   address is already claimed. It is said where an unset face is said, and they go on. */
test('the Arc away is noted, and the Site stands at its address', async () => {
  const p = page({
    hash: '#register=' + draft({ app: APP }),
    answers: { '/v2/apply': (b) => (b.op === 'base.publish' ? [503, 'the Arc is away'] : null) }
  });
  await settle();
  await press(p);
  assert.equal(p.$('sheet').hidden, true, 'not a wall: the sheet closes');
  assert.match(p.$('feed').textContent, /page · riverside/, 'the page is named');
  assert.match(p.$('feed').textContent, /the Arc is away/, "in the Arc's own words");
  assert.ok(made(p).some((c) => c.path === '/v2/site/address'), 'the address was still claimed');
  assert.match(p.$('siteBtn').textContent, /Riverside/, 'and the Site stands, opened');
  const back = b64decode(p.assigned().split('#registered=')[1]);
  assert.deepEqual(back, { site: SITE, app: APP }, 'the dev gets their Site id, and no address: the page is not live');
});

test('the bundle unreachable is noted the same way, and nothing is published at the address', async () => {
  const p = page({ hash: '#register=' + draft({}), bundle: () => { throw new TypeError('Failed to fetch'); } });
  await settle();
  await press(p);
  assert.equal(p.$('sheet').hidden, true);
  assert.match(p.$('feed').textContent, /page · riverside/);
  assert.ok(!liveCalls(p).some((c) => c.body && c.body.op === 'base.publish'), 'nothing is published without the Arc');
  assert.match(p.$('siteBtn').textContent, /Riverside/);
});

/* HACK_USER (R3.1 gap): the signup's name (the draft's pname) was shown on Your card and kept
   only if the person pressed Save there; /v2/graph's me.display_name stayed "", so builders
   read as "New member" everywhere. Register now writes it as Your card's Save does
   (group.setProfile {displayName, shape} on the person's own record), right after the Site. */
test('Register saves the signup\'s name on the person\'s own record, as Your card\'s Save writes it', async () => {
  const p = page({ hash: '#register=' + draft({ pname: '  Hana  ' }), self: true });
  await settle();
  await press(p);
  await settle();
  const named = p.calls.filter((c) => c.path === '/v2/apply' && c.body && c.body.op === 'group.setProfile');
  assert.equal(named.length, 1, 'one write of the name');
  assert.deepEqual(named[0].body, { object: SELF, op: 'group.setProfile', args: { displayName: 'Hana', shape: 'individual' } });
  const order = p.calls.map((c) => c.path).filter((x) => x === '/v2/batch' || x === '/v2/apply');
  assert.ok(order.indexOf('/v2/batch') < order.lastIndexOf('/v2/apply'), 'after the Site is made');
  assert.equal(p.$('you').hidden, false, 'the first card still opens after Register, to add a photo');
  const find = (n, pred) => (pred(n) ? n : (n.children || []).map((c) => find(c, pred)).find(Boolean));
  assert.equal(find(p.$('youBody'), (n) => n.attrs && n.attrs.id === 'youName').attrs.value, 'Hana', 'with the name, saved and shown');
});

test('with no name in the draft, nothing is written and the first card asks, as before', async () => {
  const p = page({ hash: '#register=' + draft({ pname: '' }), self: true });
  await settle();
  await press(p);
  await settle();
  assert.equal(p.calls.filter((c) => c.path === '/v2/apply' && c.body && c.body.op === 'group.setProfile').length, 0);
  assert.equal(p.$('you').hidden, false, 'the first card asks for a name');
});
