/* icd.test.mjs — the model by its hash (UX's perf audit): where the Door's page names the ICD's
   sha256 in <meta id="wallflowers-icd">, the webapp fetches /v2/icd/<hash>, which is cached for
   good; a page that names none, or not a hash, fetches /v2/icd. webapp.js runs as the page runs
   it, over a stub page and a stub Door.

   Run: node --test app/web/webapp/icd.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./webapp.js', import.meta.url), 'utf8');
const ORIGIN = 'https://app.wallflowers.io';
const HASH = 'ab'.repeat(32);

/* A signed-in page whose head names `content`, or nothing: the paths it fetched. */
function page(content) {
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
  if (content !== undefined) $('wallflowers-icd').setAttribute('content', content);
  const fetched = [];
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => JSON.stringify(body) });
  const fetch = async (url) => {
    const path = url.slice(ORIGIN.length);
    fetched.push(path);
    if (path === '/v2/me') return reply(200, { pk: 'ed25519:' + 'c'.repeat(64) });
    if (path === '/v2/graph') return reply(200, { objects: [], spine: [] });
    if (path.startsWith('/v2/icd')) return reply(200, { kinds: {}, facets: {} });
    return reply(200, {});
  };
  class EventSource {
    constructor() { this.readyState = 1; }
    addEventListener() {}
    close() {}
  }
  const ctx = {
    document: { getElementById: $, documentElement: new Node('html'), createElement: (t) => new Node(t), createTextNode: (t) => ({ textContent: String(t) }), addEventListener() {} },
    location: { hash: '', pathname: '/', search: '', origin: ORIGIN, assign() {} }, history: { replaceState() {} }, fetch, EventSource, doorOrigin: () => ORIGIN,
    addEventListener() {}, scrollTo() {},
    innerWidth: 1280, matchMedia: () => ({ matches: false, addEventListener() {} }), requestAnimationFrame: (f) => setImmediate(f),
    atob, TextEncoder, TextDecoder, Uint8Array, JSON, Promise, String, Error, Object, Array, Date, Math, console,
    setTimeout: (f) => setImmediate(f), clearTimeout() {}
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  return fetched;
}

const settle = async () => { for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r)); };
const models = (fetched) => fetched.filter((p) => p.startsWith('/v2/icd'));

test('a page naming the hash fetches the model by it', async () => {
  const fetched = page(HASH);
  await settle();
  assert.deepEqual(models(fetched), ['/v2/icd/' + HASH]);
});

test('a page naming none, or not a hash, fetches /v2/icd', async () => {
  for (const content of [undefined, '', 'AB'.repeat(32), HASH.slice(1), HASH + '/../x']) {
    const fetched = page(content);
    await settle();
    assert.deepEqual(models(fetched), ['/v2/icd'], String(content));
  }
});
