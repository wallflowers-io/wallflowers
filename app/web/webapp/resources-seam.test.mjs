/* resources-seam.test.mjs — W-98 Resources in the webapp: the Door's editor (/v2/resources.js)
   is what "+" on Resources and a post's Edit open, and a `markdown` body is read as it reads it.
   webapp.js runs as the page runs it, over a stub page, the real ICD, a stub Door and a stub
   editor that records what it is given.

   Run: node --test app/web/webapp/resources-seam.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const HTML = readFileSync(new URL('./index.html', import.meta.url), 'utf8');
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));
const ORIGIN = 'https://app.wallflowers.io';
const ME = 'c'.repeat(64), SITE = 'a'.repeat(64), MINE = 'd'.repeat(64), THEIRS = 'e'.repeat(64), OTHER = 'f'.repeat(64);
const UNHELD = '9'.repeat(64), UNHELD2 = '8'.repeat(64), STRANGER = '7'.repeat(64);

function page({ editor = true, pubs } = {}) {
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
    get firstChild() { return this.children[0] || (this._html ? (this._parsed ||= new Node('svg')) : null); }
    get lastChild() { return this.children.at(-1) || null; }
    get childNodes() { return this.children; }
    insertBefore(c, ref) { const i = this.children.indexOf(ref); this.children.splice(i < 0 ? this.children.length : i, 0, c); return c; }
    querySelector() { return null; }
  }
  const byId = {};
  const $ = (id) => (byId[id] ||= new Node('#' + id));
  for (const id of ['sheet', 'side', 'refused', 'status', 'compose', 'resEd']) $(id).hidden = true;
  const calls = [];
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)), json: async () => body });
  const post = (id, owner, view) => ({ id, kind: 'post', name: view.title, owner, members: [owner], folds: true,
    view: Object.assign({ title: '', body: '', form: 'article', body_format: 'plain', backlinks: [{ object: SITE, rel: 'created', at: 1 }], parts: [] }, view) });
  const site = { id: SITE, kind: 'group', name: 'Mill Road Allotments', owner: ME, members: [ME, OTHER], folds: true,
    view: { display_name: 'Mill Road Allotments', parts: [], affiliations: [
      { peer: MINE, rel: 'created', name: 'Seed swap', at: 1 }, { peer: THEIRS, rel: 'created', name: 'Rota', at: 2 },
      { peer: UNHELD, rel: 'created', name: 'Water rota', at: 9 }, { peer: UNHELD2, rel: 'created', name: 'Tool library', at: 8 }] } };
  const graph = { me: { pk: 'ed25519:' + ME }, spine: [], objects: [site,
    post(MINE, ME, { title: 'Seed swap', body: 'Hello **all**.\n\n![rows](asset:00000000000000a1)', body_format: 'markdown',
      assets: [{ id: '00000000000000a1', mime: 'image/jpeg', data: 'AAAA', width: 4, height: 3, alt: 'rows', at: 1 }] }),
    post(THEIRS, OTHER, { title: 'Rota', body: 'plain words' })] };
  const fetch = async (url, init = {}) => {
    const path = url.slice(ORIGIN.length);
    calls.push({ path, init });
    if (path === '/v2/me') return reply(200, { pk: 'ed25519:' + ME, display_name: 'Me' });
    if (path === '/v2/graph') return reply(200, graph);
    if (path === '/v2/icd') return reply(200, ICD);
    if (path.startsWith('/v2/draft/')) return reply(200, { name_only: true });
    if (path === '/v2/site/' + SITE + '/items') return pubs ? reply(200, pubs) : reply(404, 'no such route');
    return reply(200, {});
  };
  const mounted = [], rendered = [];
  const R = {
    mount(host, o) { const m = { host, o, closed: 0 }; mounted.push(m); return { close() { m.closed++; }, get dirty() { return false; } }; },
    markdown: { render(text, doc, o) { rendered.push({ text, o }); const n = doc.createElement('md'); n.textContent = 'MD:' + text; return n; } },
  };
  const opened = [];
  const loc = { hash: '#site=' + SITE, pathname: '/', search: '', origin: ORIGIN, href: ORIGIN + '/#site=' + SITE, assign() {} };
  const ctx = {
    document: { getElementById: $, documentElement: new Node('html'), createElement: (t) => new Node(t),
      createTextNode: (t) => ({ textContent: String(t) }), addEventListener() {}, querySelector: () => null },
    location: loc, history: { replaceState() {} }, fetch, doorOrigin: () => ORIGIN,
    addEventListener() {}, scrollTo() {},
    innerWidth: 1280, matchMedia: () => ({ matches: false, addEventListener() {} }), requestAnimationFrame: (f) => setImmediate(f),
    atob, URL, TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, console,
    setTimeout: (f) => setImmediate(f), clearTimeout() {}, open: (...a) => { opened.push(a); return null; }
  };
  ctx.window = ctx;
  if (editor) ctx.WallFlowers = { Resources: R };
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  return { $, calls, mounted, rendered, opened };
}

const settle = async () => { for (let i = 0; i < 40; i++) await new Promise((r) => setImmediate(r)); };
function find(n, pred) {
  if (pred(n)) return n;
  for (const c of n.children || []) { const f = find(c, pred); if (f) return f; }
  return null;
}
async function resources(p) {
  await settle();
  find(p.$('tabs'), (n) => n.tag === 'button' && n.textContent === 'Resources').onclick();
  await settle();
}
async function view(p, title) {
  await resources(p);
  const row = find(p.$('lbody'), (n) => n.tag === 'button' && n.textContent.startsWith(title));
  assert.ok(row, title + ' in the rail');
  row.onclick();
  await settle();
}
const editButton = (p) => find(p.$('feed'), (n) => n.tag === 'button' && n.className === 'resedit');

test('the page loads the Door\'s editor, and has where it opens', () => {
  const scripts = [...HTML.matchAll(/<script src="([^"]+)"/g)].map((m) => m[1]);
  assert.ok(scripts.includes('/v2/resources.js'), scripts.join());
  assert.ok(scripts.indexOf('palette.js') < scripts.indexOf('/v2/resources.js'), 'after the page has its WallFlowers');
  assert.ok(scripts.indexOf('/v2/resources.js') < scripts.indexOf('webapp.js'));
  assert.match(HTML, /id="resEd"/);
});

test('"+" on Resources opens the editor for a new post of this Site, with the model the page holds', async () => {
  const p = page();
  await resources(p);
  find(p.$('lbody'), (n) => n.tag === 'button' && n.textContent === 'New resource').onclick();
  await settle();
  assert.equal(p.mounted.length, 1);
  const { host, o } = p.mounted[0];
  assert.equal(host, p.$('resEd'));
  assert.equal(p.$('resEd').hidden, false);
  assert.equal(p.$('sheet').hidden, true, 'not the sheet');
  assert.ok(!p.calls.some((c) => c.path.startsWith('/v2/draft/')));
  assert.equal(o.site, SITE);
  assert.equal(o.post, null);
  assert.equal(JSON.stringify(o.icd), JSON.stringify(ICD));
  /* Its requests are the Door's, with the session. */
  await o.fetch('/v2/batch', { method: 'POST', body: '{"steps":[]}' });
  const last = p.calls.at(-1);
  assert.equal(last.path, '/v2/batch');
  assert.equal(last.init.credentials, 'include');
  assert.equal(last.init.method, 'POST');
  /* Closed, it is gone. */
  o.onClose();
  assert.equal(p.mounted[0].closed, 1);
  assert.equal(p.$('resEd').hidden, true);
});

test('without the editor, "+" on Resources is the sheet it was', async () => {
  const p = page({ editor: false });
  await resources(p);
  find(p.$('lbody'), (n) => n.tag === 'button' && n.textContent === 'New resource').onclick();
  await settle();
  assert.equal(p.$('sheet').hidden, false);
});

test('a post its author may write offers Edit, which opens it in the editor; another\'s does not', async () => {
  const p = page();
  await view(p, 'Seed swap');
  const b = editButton(p);
  assert.ok(b, 'Edit on my post');
  assert.equal(b.textContent, 'Edit');
  b.onclick();
  await settle();
  assert.equal(p.mounted.length, 1);
  assert.deepEqual([p.mounted[0].o.site, p.mounted[0].o.post], [SITE, MINE]);

  const q = page();
  await view(q, 'Rota');
  assert.equal(editButton(q), null, 'post.setProfile is owner-only: no Edit on another\'s post');
});

test('a markdown body is read as the editor reads it, its pictures the post\'s own; a plain one is not', async () => {
  const p = page();
  await view(p, 'Seed swap');
  assert.equal(p.rendered.length, 1);
  assert.equal(p.rendered[0].text, 'Hello **all**.\n\n![rows](asset:00000000000000a1)');
  assert.equal(p.rendered[0].o.read, true);
  assert.equal(p.rendered[0].o.assets['00000000000000a1'].src, 'data:image/jpeg;base64,AAAA');
  assert.equal(find(p.$('feed'), (n) => n.className === 'golink'), null, 'an article with no link offers no Open');

  const q = page();
  await view(q, 'Rota');
  assert.equal(q.rendered.length, 0);
});

/* ── THE PUBLICATIONS BOARD: what a member does not hold, from the Site's public items ── */

/* GET /v2/site/:site/items (BW-A; webapp cookie, members only): the Arc's /v1/face/:slug/items
   body and the slug. Its post keys are core face_items' `post:<id>`, each post.setProfile's args
   the Face carries (title, body, form, link, bodyFormat, excerpt) and `at`. */
const PUBS = {
  slug: 'mill-road',
  items: {
    ['post:' + UNHELD]: { title: 'Water rota', excerpt: 'Who waters when.', body: 'Long words.', form: 'article', at: 9 },
    ['post:' + UNHELD2]: { title: 'Tool library', body: 'Forks in the **green box**.', bodyFormat: 'markdown', form: 'pdf', at: 8 },
    ['post:' + MINE]: { title: 'Seed swap', excerpt: 'held', at: 1 },
    ['post:' + STRANGER]: { title: 'Not this Site\'s', at: 3 },
    ['event:' + 'b'.repeat(64)]: { title: 'Dig day', startMs: 1, at: 4 },
  },
};
const cards = (p) => { const out = []; (function walk(n) { if (n.tag === 'button' && n.className && n.className.split(' ').includes('card')) out.push(n); (n.children || []).forEach(walk); })(p.$('feed')); return out; };
const rows = (p) => { const out = []; (function walk(n) { if (n.tag === 'button' && /^it\b/.test(n.className || '')) out.push(n); (n.children || []).forEach(walk); })(p.$('lbody')); return out; };

test('a member sees the Site\'s published posts they do not hold, by title and excerpt; each opens its public page in a new tab', async () => {
  const p = page({ pubs: PUBS });
  await resources(p);
  assert.equal(p.calls.filter((c) => c.path === '/v2/site/' + SITE + '/items').length, 1, 'asked once');
  const titles = cards(p).map((c) => c.textContent);
  assert.equal(titles.filter((t) => t.includes('Seed swap')).length, 1, 'a held post is today\'s card, once');
  assert.ok(titles.some((t) => t.includes('Rota') && !t.includes('Water')), 'held');
  const water = cards(p).find((c) => c.textContent.includes('Water rota'));
  assert.ok(water, 'an unheld post the Site names and its Face carries');
  assert.ok(water.textContent.includes('Who waters when.'), 'its excerpt');
  const tools = cards(p).find((c) => c.textContent.includes('Tool library'));
  assert.ok(tools && tools.textContent.includes('Forks in the green box.'), 'no excerpt: the body, marks let go (the ICD\'s row text)');
  assert.ok(!titles.some((t) => /Not this Site|Dig day/.test(t)), 'only posts the Site names as created');
  assert.ok(cards(p).indexOf(water) < cards(p).indexOf(tools), 'newest first');
  water.onclick();
  assert.deepEqual(p.opened, [['https://wallflowers.io/mill-road/post/' + UNHELD, '_blank', 'noopener']]);
  const row = rows(p).find((r) => r.textContent.includes('Water rota'));
  assert.ok(row && row.textContent.includes('Who waters when.'), 'in the rail too');
  row.onclick();
  assert.equal(p.opened.length, 2);
  assert.equal(p.opened[1][0], 'https://wallflowers.io/mill-road/post/' + UNHELD);
  /* Drawn again, it is not asked again. */
  find(p.$('tabs'), (n) => n.tag === 'button' && n.textContent === 'Resources').onclick();
  await settle();
  assert.equal(p.calls.filter((c) => c.path === '/v2/site/' + SITE + '/items').length, 1);
});

test('without the Site\'s items, the board is the posts this member holds', async () => {
  const p = page();
  await resources(p);
  const titles = cards(p).map((c) => c.textContent);
  assert.ok(titles.some((t) => t.includes('Seed swap')) && titles.some((t) => t.includes('Rota')));
  assert.ok(!titles.some((t) => t.includes('Water rota')));
  assert.equal(p.opened.length, 0);
});
