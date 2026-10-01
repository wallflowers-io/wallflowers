/* pdf.test.mjs — the PDF module (W-98 Resources, part B), as the Door serves it: the bundle
   main.rs's RESOURCES_JS names, over a small stand-in document. A post's document is a PDF the
   post holds (post.setDocument); what fits one Delta is the model's, the editor says how much.

   Run: node --test app/web/resources/pdf.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = new URL('../../door/src/', import.meta.url);
function bundle() {
  const main = readFileSync(new URL('main.rs', SRC), 'utf8');
  const m = /const RESOURCES_JS: &str = concat!\(([\s\S]*?)\);/.exec(main);
  return [...m[1].matchAll(/include_str!\("([^"]+)"\)/g)].map((x) => readFileSync(new URL(x[1], SRC), 'utf8')).join('');
}

/* ── a stand-in document ─────────────────────────────────────────────── */
class El {
  constructor(doc, tag) {
    Object.assign(this, { ownerDocument: doc, tagName: tag.toUpperCase(), childNodes: [], attributes: {}, parentNode: null, listeners: {}, style: {}, hidden: false, _text: '' });
    const cls = new Set();
    this.classList = { add: (c) => cls.add(c), remove: (c) => cls.delete(c), contains: (c) => cls.has(c), toggle: (c, on) => (on ?? !cls.has(c)) ? cls.add(c) : cls.delete(c) };
  }
  appendChild(c) { if (c.parentNode) c.parentNode.removeChild(c); c.parentNode = this; this.childNodes.push(c); return c; }
  removeChild(c) { this.childNodes = this.childNodes.filter((k) => k !== c); c.parentNode = null; return c; }
  remove() { if (this.parentNode) this.parentNode.removeChild(this); }
  setAttribute(k, v) { this.attributes[k] = String(v); }
  getAttribute(k) { return k in this.attributes ? this.attributes[k] : null; }
  addEventListener(t, f) { (this.listeners[t] ||= []).push(f); }
  removeEventListener(t, f) { this.listeners[t] = (this.listeners[t] || []).filter((g) => g !== f); }
  fire(t, ev = {}) { for (const f of this.listeners[t] || []) f(Object.assign({ type: t, target: this, preventDefault() {}, stopPropagation() {} }, ev)); }
  click() { this.fire('click'); }
  get textContent() { return this._text + this.childNodes.map((c) => c.textContent).join(''); }
  set textContent(v) { this.childNodes = []; this._text = String(v ?? ''); }
  getContext() { return { fillRect() {}, drawImage() {} }; }
}
function page() {
  const doc = { head: null, createElement: (t) => new El(doc, t), getElementById: () => null };
  doc.head = new El(doc, 'head');
  const urls = { made: [], revoked: [] };
  const U = class extends URL {};
  U.createObjectURL = (b) => { urls.made.push(b); return 'blob:' + urls.made.length; };
  U.revokeObjectURL = (u) => urls.revoked.push(u);
  const window = {};
  const ctx = vm.createContext({ window, document: doc, URL: U, Blob, Uint8Array, TextDecoder, crypto: globalThis.crypto, btoa, atob, setTimeout, Promise,
    define: Object.assign(() => { throw new Error('AMD'); }, { amd: true }) });
  vm.runInContext(bundle(), ctx);
  return { R: window.WallFlowersResources, doc, urls };
}
function all(n, out = []) { for (const c of n.childNodes || []) { out.push(c); all(c, out); } return out; }
const settle = async () => { for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r)); };

/* A PDF's bytes, as a file would be: n pages, the header first. */
function pdfBytes(n, junk = '') {
  const kids = Array.from({ length: n }, (_, i) => `${i + 3} 0 R`).join(' ');
  let s = junk + '%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [' + kids + '] /Count ' + n + ' >> endobj\n';
  for (let i = 0; i < n; i++) s += `${i + 3} 0 obj << /Type /Page /Parent 2 0 R >> endobj\n`;
  return new Uint8Array(Buffer.from(s + '%%EOF\n', 'latin1'));
}
function file(name, bytes, type = 'application/pdf') {
  return { name, type, size: bytes.length, arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length) };
}

test('a dropped file is taken only if its bytes are a PDF, and only within the bytes one Delta holds', async () => {
  const { R, doc } = page();
  const el = doc.createElement('div'), got = [], no = [];
  R.pdf.zone(el, { maxBytes: 400, onFile: (f) => got.push(f), onRefuse: (w) => no.push(w) });
  const drop = async (f) => { el.fire('drop', { dataTransfer: { files: [f] } }); await settle(); };
  await drop(file('notes.pdf', new Uint8Array(Buffer.from('<html>not a pdf</html>'))));
  await drop(file('big.pdf', pdfBytes(40)));
  await drop(file('photo.jpg', new Uint8Array([0xff, 0xd8, 0xff]), 'image/jpeg'));
  await drop(file('rota.pdf', pdfBytes(2)));
  await drop(file('rota-2.pdf', pdfBytes(1, '\n\n')));
  assert.deepEqual(no.slice(0, 1), ['not a PDF']);
  assert.match(no[1], /^[\d.]+ (bytes|KB|MB) is over 400 bytes$/);
  assert.equal(no[2], 'not a PDF');
  assert.deepEqual(got.map((f) => [f.name, f.mime, f.bytes.length]), [['rota.pdf', 'application/pdf', pdfBytes(2).length], ['rota-2.pdf', 'application/pdf', pdfBytes(1, '\n\n').length]]);
  assert.equal(got[0].bytes.constructor.name, 'Uint8Array');
});

test('the zone says a file is over it while it is dragged, and lets go after', () => {
  const { R, doc } = page();
  const el = doc.createElement('div');
  const z = R.pdf.zone(el, { maxBytes: 0, onFile() {}, onRefuse() {} });
  el.fire('dragover', { dataTransfer: { types: ['Files'] } });
  assert.ok(el.classList.contains('over'));
  el.fire('dragleave', { relatedTarget: null });
  assert.ok(!el.classList.contains('over'));
  el.fire('dragover', { dataTransfer: { types: ['Files'] } });
  el.fire('drop', { dataTransfer: { files: [] } });
  assert.ok(!el.classList.contains('over'));
  z.destroy();
  assert.equal((el.listeners.drop || []).length, 0);
});

test('a preview names the file, its pages and its size, and opens it; closed, it lets its bytes go', async () => {
  const { R, doc, urls } = page();
  const el = doc.createElement('div');
  const p = R.pdf.preview(el, { name: 'rota.pdf', bytes: pdfBytes(3), lib: () => Promise.reject(new Error('no pdf.js here')) });
  await settle();
  const text = el.textContent;
  assert.match(text, /rota\.pdf/);
  assert.match(text, /3 pages/);
  assert.match(text, /\d+ (bytes|KB)/);
  const open = all(el).find((n) => n.tagName === 'A');
  assert.ok(open, 'Open');
  assert.equal(open.textContent, 'Open');
  assert.equal(open.getAttribute('href'), 'blob:1');
  assert.equal(open.getAttribute('target'), '_blank');
  p.destroy();
  assert.deepEqual(urls.revoked, ['blob:1']);
  assert.equal(el.childNodes.length, 0);
});

test('with PDF.js, the preview draws the first page, and the page count is its own', async () => {
  const { R, doc } = page();
  const drawn = [];
  const lib = async () => ({
    GlobalWorkerOptions: {},
    getDocument: (src) => ({ promise: Promise.resolve({ numPages: 5, getPage: async (n) => ({ getViewport: ({ scale }) => ({ width: 612 * scale, height: 792 * scale }), render: (r) => { drawn.push([n, r.viewport.width]); return { promise: Promise.resolve() }; } }) }), destroy: async () => {} }),
  });
  const el = doc.createElement('div');
  R.pdf.preview(el, { name: 'rota.pdf', bytes: pdfBytes(3), lib });
  await settle();
  assert.equal(drawn.length, 1);
  assert.equal(drawn[0][0], 1);
  assert.ok(all(el).some((n) => n.tagName === 'CANVAS'));
  assert.match(el.textContent, /5 pages/);
});
