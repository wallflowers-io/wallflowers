/* resources.test.mjs — the Resources editor (W-98; RESOURCES-A), as the Door serves it.

   The bundle is /v2/resources.js: the files main.rs's RESOURCES_JS concatenates, read from
   main.rs itself, so what runs here is what a site loads. It runs over a small stand-in
   document; the model is the ICD this tree carries.

   Run: node --test app/web/resources/resources.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import vm from 'node:vm';

const HERE = new URL('./', import.meta.url);
const SRC = new URL('../../door/src/', import.meta.url);
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));

/* RESOURCES_JS's parts, in order, as main.rs names them. */
function parts() {
  const main = readFileSync(new URL('main.rs', SRC), 'utf8');
  const m = /const RESOURCES_JS: &str = concat!\(([\s\S]*?)\);/.exec(main);
  assert.ok(m, 'main.rs declares RESOURCES_JS');
  return [...m[1].matchAll(/include_str!\("([^"]+)"\)/g)].map((x) => new URL(x[1], SRC));
}
function bundle() { return parts().map((u) => readFileSync(u, 'utf8')).join(''); }

/* ── a stand-in document: elements, text, fragments ─────────────────────── */
class N {
  constructor(type, name, value) {
    Object.assign(this, { nodeType: type, tagName: name, nodeName: name, nodeValue: value ?? null, childNodes: [], attributes: {}, parentNode: null });
  }
  get firstChild() { return this.childNodes[0] || null; }
  get lastChild() { return this.childNodes.at(-1) || null; }
  appendChild(c) {
    if (c.nodeType === 11) { for (const k of [...c.childNodes]) this.appendChild(k); return c; }
    if (c.parentNode) c.parentNode.removeChild(c);
    c.parentNode = this;
    this.childNodes.push(c);
    return c;
  }
  removeChild(c) { this.childNodes = this.childNodes.filter((k) => k !== c); c.parentNode = null; return c; }
  setAttribute(k, v) { this.attributes[k] = String(v); }
  getAttribute(k) { return k in this.attributes ? this.attributes[k] : null; }
  removeAttribute(k) { delete this.attributes[k]; }
  get textContent() { return this.nodeType === 3 ? this.nodeValue : this.childNodes.map((c) => c.textContent).join(''); }
  set textContent(v) { this.childNodes = []; if (v !== '' && v != null) this.appendChild(new N(3, '#text', String(v))); }
}
const doc = {
  createElement: (t) => new N(1, t.toUpperCase()),
  createTextNode: (v) => new N(3, '#text', String(v)),
  createDocumentFragment: () => new N(11, '#fragment'),
};
function all(n, out = []) { for (const c of n.childNodes) { out.push(c); all(c, out); } return out; }

function load() {
  const window = {};
  const ctx = vm.createContext({ window, crypto: globalThis.crypto, URL, btoa, atob, setTimeout, define: Object.assign(() => { throw new Error('a page\'s AMD was called'); }, { amd: true }) });
  vm.runInContext(bundle(), ctx);
  return window.WallFlowersResources;
}
const R = load();
const md = new Proxy({}, { get: (_, k) => R.markdown[k] });
/* The bundle runs in its own realm: its arrays and objects, as plain data, to compare. */
const J = (x) => JSON.parse(JSON.stringify(x));

/* The reference parser's HTML for a text, whitespace between and around tags let go. */
function html(text) {
  const cm = R.commonmark;
  return new cm.HtmlRenderer({ safe: true }).render(new cm.Parser().parse(text)).replace(/\s+/g, ' ').replace(/> </g, '><').trim();
}
function through(text) {
  const root = doc.createElement('div');
  root.appendChild(md.render(text, doc, { assets: {} }));
  return md.write(root);
}

test('the vendored parser is the bytes vendor/SOURCES names, and the bundle sets one global, calling no AMD', () => {
  const src = readFileSync(new URL('vendor/SOURCES', HERE), 'utf8');
  for (const [, sha, file] of src.matchAll(/^([0-9a-f]{64})\s+(\S+)/gm)) {
    assert.equal(createHash('sha256').update(readFileSync(new URL('vendor/' + file, HERE))).digest('hex'), sha, file);
  }
  const window = {};
  const ctx = vm.createContext({ window, crypto: globalThis.crypto, URL, btoa, atob, setTimeout, define: Object.assign(() => { throw new Error('AMD'); }, { amd: true }) });
  vm.runInContext(bundle(), ctx);
  assert.deepEqual(Object.keys(window), ['WallFlowersResources']);
  assert.deepEqual(Object.keys(window.WallFlowersResources).sort(), ['commonmark', 'fromView', 'limits', 'markdown', 'model', 'mount', 'pdf', 'plan', 'save', 'standing']);
});

test('on a page that has WallFlowers (the webapp; a site, after the sign-in script), the bundle is WallFlowers.Resources too, and sets no other global', () => {
  const window = { WallFlowers: { Palette: {} } };
  const ctx = vm.createContext({ window, crypto: globalThis.crypto, URL, btoa, atob, setTimeout, define: Object.assign(() => { throw new Error('AMD'); }, { amd: true }) });
  vm.runInContext(bundle(), ctx);
  assert.deepEqual(Object.keys(window).sort(), ['WallFlowers', 'WallFlowersResources']);
  assert.equal(window.WallFlowers.Resources, window.WallFlowersResources);
  assert.deepEqual(Object.keys(window.WallFlowers).sort(), ['Palette', 'Resources']);
});

test('what the editor writes, the reference parser reads as what was written', () => {
  for (const text of [
    '# Title\n\nPara with **bold**, *em*, `code` and [a link](https://example.org/a%20b).',
    '## Sub\n\n> quoted **text**\n> and more\n\n- one\n- two\n  - nested\n- three',
    '1. first\n2. second\n\n- after',
    '3. starts at three\n4. four',
    '```js\nconst x = `y`;\n\n  indented\n```',
    '---',
    '![a cat, sitting](asset:0123456789abcdef)',
    'line one\\\nline two',
    '***both***',
    '**_x_ y**',
    'text with \\*escaped\\* stars, a_b_c and 5 * 3 < 4 & 1 > 0',
    '- a\n\n  b\n\n- c',
    '> - quoted list\n> - two',
    '- one\n- two\n\n* three\n* four',
    '`` a ` tick ``',
    'Head\n===\n\nand setext',
    '#### four\n\n###### six',
    'a [link *with* marks](mailto:a@b.org) here',
  ]) {
    assert.equal(html(through(text)), html(text), JSON.stringify(text) + ' wrote ' + JSON.stringify(through(text)));
  }
});

test('text that looks like CommonMark is kept as text', () => {
  for (const s of ['# not a heading', '1. not a list', '- not a list', '+ nor this', '> not a quote', '*not em*', '_not em_',
    '<b>not html</b>', '[not](https://a.link)', '&amp; stays', 'back\\slash', '`tick`', '---', '===', '~~~', '2024. a year',
    '![not](asset:0123456789abcdef)', '#hashtag', '    indented', 'a <https://auto.link> b']) {
    const root = doc.createElement('div'), p = doc.createElement('p');
    p.appendChild(doc.createTextNode(s));
    root.appendChild(p);
    const out = md.write(root), tree = md.parse(out), first = tree.firstChild;
    assert.equal(first.type, 'paragraph', `${JSON.stringify(s)} wrote ${JSON.stringify(out)}`);
    assert.equal(first.next, null, s);
    const inl = [];
    for (let k = first.firstChild; k; k = k.next) inl.push(k);
    assert.equal(inl.every((k) => k.type === 'text') ? inl.map((k) => k.literal).join('') : 'marks', s.trim(), `${JSON.stringify(s)} wrote ${JSON.stringify(out)}`);
  }
});

test('a member\'s markup never becomes elements: raw HTML is text, a script link is text, an image from elsewhere is a link', () => {
  const root = doc.createElement('div');
  root.appendChild(md.render('<script>alert(1)</script>\n\npara <b>x</b> [go](javascript:alert(1)) ![cat](https://x.org/c.png)', doc, { assets: {} }));
  const tags = all(root).filter((n) => n.nodeType === 1).map((n) => n.tagName);
  assert.ok(!tags.includes('SCRIPT') && !tags.includes('B') && !tags.includes('IMG'), tags.join());
  const links = all(root).filter((n) => n.tagName === 'A');
  assert.deepEqual(links.map((a) => a.getAttribute('href')), ['https://x.org/c.png']);
  assert.match(root.textContent, /<script>alert\(1\)<\/script>/);
  assert.match(root.textContent, /<b>x<\/b>/);
  const again = md.parse(md.write(root));
  const w = again.walker(); let e, kinds = [];
  while ((e = w.next())) if (e.entering) kinds.push(e.node.type);
  assert.ok(!kinds.includes('html_block') && !kinds.includes('html_inline'), kinds.join());
});

test('a body\'s pictures are its own assets, in order, once each', () => {
  assert.deepEqual(J(md.assetsOf('![a](asset:00000000000000aa)\n\ntext ![b](asset:00000000000000bb) ![a](asset:00000000000000aa) ![c](https://x.org/c.png)')), ['00000000000000aa', '00000000000000bb']);
});

/* ── what a save writes, held to the model ─────────────────────────────── */

const SITE = 'a'.repeat(64), POST = 'd'.repeat(64);
let model;
const M = () => (model ||= R.model(ICD));
const A1 = '00000000000000a1', A2 = '00000000000000a2';
function fresh(over = {}) {
  return Object.assign(R.fromView({}), {
    title: 'Seed swap', excerpt: 'Bring what you grew.', bodyFormat: 'markdown', form: 'pdf', body: `Hello **all**.\n\n![rows](asset:${A1})\n\n![beans](asset:${A2})\n`,
    banner: 'AAAA', bannerMime: 'image/jpeg', bannerAlt: 'the plot',
    document: { data: 'JVBERi0=', mime: 'application/pdf', name: 'rota.pdf' },
    assets: { [A1]: { data: 'AAAA', mime: 'image/jpeg', width: 4, height: 3, alt: 'rows', at: 1 }, [A2]: { data: 'BBBB', mime: 'image/jpeg', width: 4, height: 3, alt: '', at: 2 } },
  }, over);
}

test('publishing a new post: its words, cover, document and pictures first, then the Site\'s created from both ends; every step is the model\'s', () => {
  const steps = J(R.plan(M(), { was: null, now: fresh(), post: null, site: SITE, publish: true, at: 7 }));
  assert.deepEqual(steps.map((s) => s.do === 'mint' ? 'mint ' + s.kind : s.op),
    ['mint post', 'post.setProfile', 'post.setMedia', 'post.setDocument', 'post.addAsset', 'post.addAsset', 'group.setAffiliation', 'base.setBacklink']);
  const profile = steps[1].args;
  assert.equal(profile.bodyFormat, 'markdown');
  assert.ok('markdown' in ICD.kinds.post.ops['post.setProfile'].args.bodyFormat.vocabulary, 'the model has markdown');
  assert.equal(profile.form, 'pdf');
  assert.deepEqual(steps[6], { do: 'apply', object: SITE, op: 'group.setAffiliation', args: { peer: 'POST', rel: 'created', name: 'Seed swap', at: 7 } });
  assert.deepEqual(steps[7], { do: 'apply', object: 'POST', op: 'base.setBacklink', args: { object: SITE, rel: 'created', at: 7 } });
  for (const s of steps.filter((s) => s.do === 'apply')) {
    assert.equal(M().check({ ...s, args: Object.fromEntries(Object.entries(s.args).map(([k, v]) => [k, v === 'POST' ? POST : v])) }), null, s.op);
  }
});

test('a draft is the post alone: no created from either end', () => {
  const steps = J(R.plan(M(), { was: null, now: fresh(), post: null, site: SITE, publish: false }));
  assert.ok(!steps.some((s) => s.op === 'group.setAffiliation' || s.op === 'base.setBacklink'));
});

test('editing a published post writes only what changed; a new title renames the Site\'s row at its first time', () => {
  const was = fresh();
  const now = fresh({ title: 'Seed swap, Sunday', body: `Hello **all**.\n\n![rows](asset:${A1})\n`, assets: { ...was.assets } });
  const steps = J(R.plan(M(), { was, now, post: POST, site: SITE, published: true, publishedAt: 5, at: 9 }));
  assert.deepEqual(steps.map((s) => s.op), ['post.setProfile', 'post.removeAsset', 'group.setAffiliation']);
  assert.deepEqual(steps[1].args, { id: A2 });
  assert.equal(steps[2].args.at, 5);
  assert.deepEqual(J(R.plan(M(), { was, now: fresh(), post: POST, site: SITE, published: true })), []);
});

test('unpublishing takes the Site\'s created back from both ends', () => {
  const steps = J(R.plan(M(), { was: fresh(), now: fresh(), post: POST, site: SITE, published: true, unpublish: true }));
  assert.deepEqual(steps.map((s) => [s.object, s.op]), [[SITE, 'group.clearAffiliation'], ['POST', 'base.clearBacklink']]);
});

test('a cover change keeps the post\'s icon: post.setMedia is both, whole', () => {
  const was = fresh({ icon: 'ICON', iconMime: 'image/png' });
  const steps = J(R.plan(M(), { was, now: { ...was, banner: 'CCCC' }, post: POST, site: SITE, published: false }));
  assert.deepEqual(steps.map((s) => s.op), ['post.setMedia']);
  assert.deepEqual(steps[0].args, { icon: 'ICON', iconMime: 'image/png', banner: 'CCCC', bannerMime: 'image/jpeg', bannerAlt: 'the plot' });
});

test('caps and vocabularies are the model\'s: an arg over its maxLength, or outside its vocabulary, is refused before it is sent', () => {
  const icd = structuredClone(ICD);
  icd.kinds.post.ops['post.setProfile'].args.excerpt.maxLength = 300;
  const mm = R.model(icd);
  const long = { do: 'apply', object: POST, op: 'post.setProfile', args: { title: 't', excerpt: 'x'.repeat(301) } };
  assert.equal(mm.check(long), 'excerpt is over 300 characters');
  assert.equal(mm.check({ ...long, args: { title: 't', excerpt: 'x'.repeat(300) } }), null);
  assert.match(M().check({ do: 'apply', object: POST, op: 'post.setProfile', args: { title: 't', bodyFormat: 'html' } }), /^bodyFormat is not one of/);
  assert.equal(M().check({ do: 'apply', object: POST, op: 'post.setProfile', args: { title: 't', colour: 'red' } }), 'post.setProfile has no arg colour');
  assert.equal(M().check({ do: 'apply', object: POST, op: 'post.setProfile', args: {} }), 'post.setProfile needs title');
});

test('saving: the mint and what names it go in the first batch, by $step; later batches name the post; the Door\'s refusal is said', async () => {
  const sent = [];
  const send = async (path, init) => {
    const body = JSON.parse(init.body);
    sent.push({ path, steps: body.steps });
    const made = body.steps.map((s, i) => (s.do === 'mint' ? POST : 'delta' + i));
    return { ok: true, status: 200, text: async () => JSON.stringify({ made }) };
  };
  const now = fresh();
  for (let i = 0; i < 20; i++) now.assets['00000000000001' + i.toString(16).padStart(2, '0')] = { data: 'A', mime: 'image/jpeg', at: i };
  now.body += Object.keys(now.assets).map((id) => `![x](asset:${id})`).join('\n\n');
  const steps = J(R.plan(M(), { was: null, now, post: null, site: SITE, publish: true, at: 3 }));
  assert.equal(await R.save(M(), send, steps, null), POST);
  assert.ok(sent.length > 1 && sent.every((b) => b.path === '/v2/batch' && b.steps.length <= 16));
  assert.equal(sent[0].steps[0].do, 'mint');
  assert.ok(sent[0].steps.slice(1).every((s) => JSON.stringify(s.object) === '{"$step":0}'));
  const last = sent.at(-1).steps;
  assert.deepEqual(last.slice(-2).map((s) => [s.object, s.op, s.args.peer || s.args.object]), [[SITE, 'group.setAffiliation', POST], [POST, 'base.setBacklink', SITE]]);

  const refusing = async () => ({ ok: false, status: 422, text: async () => JSON.stringify({ made: [POST], refused: { step: 1, why: "coordinator: 'post.setProfile' is owner-only, and the author is not the owner" } }) });
  await assert.rejects(R.save(M(), refusing, R.plan(M(), { was: null, now: fresh(), post: null, site: SITE }), null), /owner-only/);
});

test('a post as the Door folds it: snake_case or camelCase, and whether the Site names it from both ends', () => {
  const v = R.fromView({ title: 'T', body: 'b', body_format: 'markdown', banner_mime: 'image/png', banner_alt: 'alt', assets: [{ id: A1, data: 'x', mime: 'image/jpeg', alt: 'a', at: 1 }], document: { data: 'JVBE', mime: 'application/pdf', name: 'n.pdf' } });
  assert.equal(v.bodyFormat, 'markdown');
  assert.equal(v.bannerAlt, 'alt');
  assert.equal(v.assets[A1].alt, 'a');
  assert.equal(v.document.name, 'n.pdf');
  const site = { id: SITE, view: { affiliations: [{ peer: POST, rel: 'created', at: 4 }] } };
  assert.deepEqual(J(R.standing(site, { view: { backlinks: [{ object: SITE, rel: 'created', at: 4 }] } }, POST)), { published: true, at: 4, half: false });
  assert.deepEqual(J(R.standing(site, { view: { backlinks: [] } }, POST)), { published: false, at: 4, half: true });
});

/* ── the post's fields as EVENTS-CORE folds them, and the model's ceilings (W-98 2.3.1) ── */

test('a picture set\'s ceiling is the model\'s maxLive, as the served ICD says it', () => {
  const live = ICD.kinds.post.ops['post.addAsset'].maxLive;
  assert.ok(Number.isInteger(live), 'the ICD states post.addAsset\'s maxLive');
  assert.equal(M().max('post.addAsset'), live);
  const icd = structuredClone(ICD);
  icd.kinds.post.ops['post.addAsset'].maxLive = live + 3;
  assert.equal(R.model(icd).max('post.addAsset'), live + 3);
  /* More live pictures than that is refused before anything is sent. */
  const now = fresh();
  for (let i = 0; i <= live; i++) {
    const id = '000000000000f' + i.toString(16).padStart(3, '0');
    now.assets[id] = { data: 'A', mime: 'image/jpeg', at: i + 10 };
    now.body += `\n\n![x](asset:${id})`;
  }
  assert.match(R.limits(M(), now), new RegExp('at most ' + live + ' pictures'));
  assert.equal(R.limits(M(), fresh()), null);
});

test('an arg\'s pattern is the model\'s, and a length is counted as the fold counts it, in characters', () => {
  assert.match(M().check({ do: 'apply', object: POST, op: 'post.removeAsset', args: { id: 'not-an-id' } }), /^id is not/);
  assert.equal(M().check({ do: 'apply', object: POST, op: 'post.removeAsset', args: { id: A1 } }), null);
  const cap = ICD.kinds.post.ops['post.setProfile'].args.excerpt.maxLength;
  assert.equal(M().check({ do: 'apply', object: POST, op: 'post.setProfile', args: { title: 't', excerpt: '🌱'.repeat(cap) } }), null);
  assert.equal(M().check({ do: 'apply', object: POST, op: 'post.setProfile', args: { title: 't', excerpt: '🌱'.repeat(cap + 1) } }), `excerpt is over ${cap} characters`);
});

test('a cover fits one Delta: the banner\'s cap and the carriage ceiling, less the icon it travels with', () => {
  const media = ICD.kinds.post.ops['post.setMedia'].args, one = ICD.kinds.post.ops['post.addAsset'].args.asset.maxLength;
  const room = Math.min(media.banner.maxLength, one);
  assert.equal(M().cover(''), room);
  assert.equal(M().cover('x'.repeat(1000)), room - 1000);
});

test('a document is the post view\'s {mime, data, name}; post.setDocument writes it as a MediaRef, and an empty ref clears it', () => {
  const v = R.fromView({ title: 'T', form: 'pdf', document: { mime: 'application/pdf', width: 0, height: 0, duration_ms: 0, data: 'JVBE', bytes: 3, name: 'rota.pdf' } });
  assert.deepEqual(J(v.document), { data: 'JVBE', mime: 'application/pdf', name: 'rota.pdf' });
  const set = J(R.plan(M(), { was: fresh({ document: null }), now: fresh(), post: POST, site: SITE })).filter((s) => s.op === 'post.setDocument');
  assert.equal(set.length, 1);
  const decl = ICD.kinds.post.ops['post.setDocument'].args;
  const ref = decl.document.mediaRef;
  for (const k of Object.keys(set[0].args)) assert.ok(k === 'name' || k.startsWith(ref.prefix) && ref.suffixes.includes(k.slice(ref.prefix.length)), k);
  assert.equal(M().check({ ...set[0], object: POST }), null);
  const clear = J(R.plan(M(), { was: fresh(), now: fresh({ document: null }), post: POST, site: SITE })).filter((s) => s.op === 'post.setDocument');
  assert.deepEqual(clear[0].args, { document: '', documentMime: '' });
});

test('a refusal after the mint names the post it made, so the next save writes to it and mints nothing', async () => {
  let n = 0;
  const send = async (path, init) => {
    const steps = JSON.parse(init.body).steps;
    n++;
    return { ok: false, status: 422, text: async () => JSON.stringify({ made: steps[0].do === 'mint' ? [POST] : [], refused: { step: 1, why: 'no' } }) };
  };
  const err = await R.save(M(), send, R.plan(M(), { was: null, now: fresh(), post: null, site: SITE }), null).then(() => null, (e) => e);
  assert.ok(err, 'refused');
  assert.equal(err.post, POST);
  assert.equal(n, 1, 'nothing after a refusal');
});

test('a link, picture or linked PDF post keeps its form and its link when its words are edited', () => {
  for (const form of ['link', 'image', 'pdf']) {
    const was = R.fromView({ title: 'Old', body: '', form, link: 'https://example.org/x' });
    assert.equal(was.link, 'https://example.org/x');
    const now = { ...was, title: 'New', document: null };
    const steps = J(R.plan(M(), { was, now, post: POST, site: SITE }));
    assert.deepEqual(steps.map((s) => s.op), ['post.setProfile']);
    assert.equal(steps[0].args.form, form);
    assert.equal(steps[0].args.link, 'https://example.org/x');
    assert.equal(M().check({ ...steps[0], object: POST }), null);
  }
  /* An article has no link: the fold refuses one. */
  const art = J(R.plan(M(), { was: null, now: fresh({ document: null, form: 'article', link: 'https://x.org' }), post: null, site: SITE }));
  assert.equal(art[1].args.form, 'article');
  assert.ok(!('link' in art[1].args));
});

test('publishing writes no visibility: a post is the ICD\'s default (MANAGE, R3)', () => {
  for (const o of [{ publish: true }, { publish: false }, { published: true, unpublish: true, was: fresh() }]) {
    const steps = J(R.plan(M(), Object.assign({ was: null, now: fresh(), post: null, site: SITE }, o)));
    assert.ok(!steps.some((s) => /visibility/i.test(s.op || '')), JSON.stringify(o));
  }
});

test('a PDF\'s ceiling in bytes is post.setDocument\'s document maxLength, as base64 decodes it', () => {
  const cap = ICD.kinds.post.ops['post.setDocument'].args.document.maxLength;
  assert.ok(Number.isInteger(cap), 'the ICD states it');
  assert.equal(M().bytes('post.setDocument', 'document'), Math.floor(cap / 4) * 3);
  const icd = structuredClone(ICD);
  icd.kinds.post.ops['post.setDocument'].args.document.maxLength = 4000;
  assert.equal(R.model(icd).bytes('post.setDocument', 'document'), 3000);
});
