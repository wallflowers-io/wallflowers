#!/usr/bin/env node
// Smoke test for the MLS build of core-wasm, under Node, loaded THE WAY A PAGE
// LOADS IT: through docs/core/core_wasm.js — the wasm-bindgen glue — with the
// one import the module declares, `env.pacific_fill_random`, supplied by the
// host. Nothing here reaches around the glue; if the glue cannot stand the module
// up, neither can the page, and that is the first thing to know.
//
// `env` is a bare specifier the glue imports, and a page satisfies it with an
// import map. Node has no import map, so a resolve hook maps it to a data: URL
// module whose one export forwards to a global this script binds once the
// memory exists — the same "closes over a getter the instance fills in later"
// shape docs/pacific-wasm-test.html uses.
//
//   node smoke/core-wasm.mjs                       (b) pairing + seal + join
//   WASM_ARCHIVE_FIXTURE_DIR=… node smoke/core-wasm.mjs
//        also folds the hand-built archive core-wasm's native test wrote there
//        (`ARCHIVE_FIXTURE_DIR=… cargo test -p core-wasm`) and requires the wasm
//        fold to be BYTE-IDENTICAL to the native one.
//   The coordination fixture, core/coordination/archive-fixture.cbor, is folded
//   and checked against its .expected.json whenever it exists.

import { registerHooks } from 'node:module';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import { webcrypto } from 'node:crypto';

const here = path.dirname(fileURLToPath(import.meta.url));
const coreDir = path.resolve(here, '..', 'docs', 'core');
const coordFixture = path.resolve(here, '..', '..', '..', 'core', 'coordination', 'archive-fixture.cbor');

// ── the env import ───────────────────────────────────────────────────────────
const ENV_URL = 'data:text/javascript,' + encodeURIComponent(
  'export function pacific_fill_random(ptr, len) { globalThis.__pacific_fill_random(ptr, len); }'
);
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === 'env') return { url: ENV_URL, shortCircuit: true };
    return next(specifier, context);
  },
});
// Entropy before the memory is bound is a bug, not a fallback.
globalThis.__pacific_fill_random = () => { throw new Error('entropy asked for before the module was bound'); };

let failures = 0;
function ok(cond, what) {
  console.log((cond ? '  ok    ' : '  FAIL  ') + what);
  if (!cond) failures++;
}
function head(s) { console.log('\n' + s); }

// ── a device = one instance of the module ────────────────────────────────────
// The glue caches its instance per JS module, so a second DEVICE is a second
// import of the same glue under a different URL. Each gets its own memory, its
// own DEVICE static, its own entropy binding.
let entropyCalls = 0;
async function device(label) {
  const glue = await import(pathToFileURL(path.join(coreDir, 'core_wasm.js')).href + '?device=' + label);
  const wasm = glue.initSync({ module: readFileSync(path.join(coreDir, 'core_wasm_bg.wasm')) });
  const fill = (ptr, len) => {
    entropyCalls++;
    webcrypto.getRandomValues(new Uint8Array(wasm.memory.buffer, ptr, len));
  };
  // One global, many devices: dispatch on which memory the pointer belongs to is
  // not possible, so the binding is swapped around each call instead.
  const call = (name, input) => {
    const bytes = typeof input === 'string' ? new TextEncoder().encode(input) : input;
    const ptr = wasm.alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
    const prev = globalThis.__pacific_fill_random;
    globalThis.__pacific_fill_random = fill;
    try { wasm[name](bytes.length); } finally { globalThis.__pacific_fill_random = prev; }
    // memory may have grown during the call, so the buffer is re-read after it.
    const out = new Uint8Array(wasm.memory.buffer, wasm.out_ptr(), wasm.out_len()).slice();
    if (wasm.erred()) throw new Error(name + ': ' + new TextDecoder().decode(out));
    return out;
  };
  const json = (name, input) => JSON.parse(new TextDecoder().decode(call(name, input)));
  const text = (name, input) => new TextDecoder().decode(call(name, input));
  return { wasm, call, json, text, exports: Object.keys(wasm) };
}

const hex = (u8) => Array.from(u8, (b) => b.toString(16).padStart(2, '0')).join('');
const unhex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));

// ── a minimal CBOR encoder, for the IntroPayload the scanner seals ────────────
// ciborium reads a struct from a text-keyed map in any order; optional fields
// left out decode as None. Only the shapes the payload needs.
function cbor(v) {
  const out = [];
  const hdr = (major, n) => {
    if (n < 24) out.push((major << 5) | n);
    else if (n < 0x100) out.push((major << 5) | 24, n);
    else if (n < 0x10000) out.push((major << 5) | 25, n >> 8, n & 0xff);
    else out.push((major << 5) | 26, (n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
  };
  const enc = (x) => {
    if (x instanceof Uint8Array) { hdr(2, x.length); out.push(...x); }
    else if (typeof x === 'string') { const b = new TextEncoder().encode(x); hdr(3, b.length); out.push(...b); }
    else if (x && typeof x === 'object') {
      const keys = Object.keys(x);
      hdr(5, keys.length);
      for (const k of keys) { enc(k); enc(x[k]); }
    } else throw new Error('cbor: unsupported ' + typeof x);
  };
  enc(v);
  return Uint8Array.from(out);
}

// ── go ───────────────────────────────────────────────────────────────────────
head('module');
const A = await device('a');
const B = await device('b');
for (const name of ['fold_archive', 'mls_init', 'mls_contact_bundle', 'mls_join_intro', 'seal_open', 'seal_seal']) {
  ok(typeof A.wasm[name] === 'function', 'export ' + name);
}

// ── (a) the archive ──────────────────────────────────────────────────────────
head('fold_archive');
try { A.call('fold_archive', new TextEncoder().encode('not an archive')); ok(false, 'garbage is refused'); }
catch (e) { ok(/archive decode/.test(e.message), 'garbage is refused: ' + e.message); }

const mine = process.env.WASM_ARCHIVE_FIXTURE_DIR;
if (mine && existsSync(path.join(mine, 'wasm-archive-fixture.cbor'))) {
  const cborBytes = readFileSync(path.join(mine, 'wasm-archive-fixture.cbor'));
  const expected = readFileSync(path.join(mine, 'wasm-archive-fixture.expected.json'), 'utf8');
  const got = A.text('fold_archive', cborBytes);
  ok(got === expected, 'hand-built archive: wasm fold is byte-identical to the native fold (' + got.length + ' bytes)');
  const v = JSON.parse(got);
  ok(v.groups.length === 5 && v.rejected.length === 2, `groups ${v.groups.length}, rejected ${v.rejected.length}`);
  for (const g of v.groups) {
    const n = g.view.messages ? g.view.messages.length : '-';
    console.log(`         ${g.kind.padEnd(13)} ${g.id.slice(0, 8)}… digest ${g.digest.slice(0, 16)}… messages ${n}`);
  }
  for (const r of v.rejected) console.log(`         rejected ${r.group.slice(0, 8)}…: ${r.why}`);
} else {
  console.log('  note  WASM_ARCHIVE_FIXTURE_DIR not set or empty — hand-built archive not checked');
}

if (existsSync(coordFixture)) {
  // core/coordination/archive-fixture.expected.json: what the phone's fold said
  // about the same bytes — per group id: kind, delta count, digest, and the
  // message count (null where the lens has no messages).
  const expPath = coordFixture.replace(/\.cbor$/, '.expected.json');
  const got = JSON.parse(A.text('fold_archive', readFileSync(coordFixture)));
  console.log(`  coordination fixture: ${got.groups.length} groups, ${got.rejected.length} rejected`);
  if (existsSync(expPath)) {
    const exp = JSON.parse(readFileSync(expPath, 'utf8'));
    ok(got.v === exp.v && got.exported_by === exp.exported_by && got.display_name === exp.display_name,
       `archive v${got.v}, exported by ${got.exported_by.slice(0, 12)}… "${got.display_name}"`);
    ok(got.peers.length === exp.peers, `peers ${got.peers.length} (expected ${exp.peers})`);
    ok(got.rejected.length === 0, 'nothing rejected');
    const ids = Object.keys(exp.groups);
    ok(ids.length > 0 && got.groups.length === ids.length, `groups ${got.groups.length} (expected ${ids.length})`);
    for (const id of ids) {
      const eg = exp.groups[id];
      const g = got.groups.find((x) => x.id === id);
      ok(!!g, `${(eg.kind || '?').padEnd(11)} ${id.slice(0, 12)}… folded`);
      if (!g) continue;
      ok(g.kind === eg.kind, `            kind ${g.kind}`);
      ok(g.digest === eg.digest, `            digest ${g.digest.slice(0, 16)}… matches the phone's`);
      const n = Array.isArray(g.view.messages) ? g.view.messages.length : null;
      ok(n === eg.messages, `            messages ${n} (expected ${eg.messages})`);
    }
  } else {
    console.log('  note  no archive-fixture.expected.json beside it — nothing to check against');
  }
} else {
  console.log('  note  core/coordination/archive-fixture.cbor is not there yet');
}

// ── (b) pairing ──────────────────────────────────────────────────────────────
head('mls_init with a seed');
// The seed's identity key is not computable here without the core — so the
// contract is: hand over the SEED and read the cred back. (It used to be learned
// from a refusal message; that was the wrong contract, and is why a seed-only
// stand-up was being refused.)
const seedA = 'a1'.repeat(32), seedB = 'b2'.repeat(32);
function person(dev, seed) {
  const r = dev.json('mls_init', JSON.stringify({ seed }));
  ok(/^[0-9a-f]{64}$/.test(r.cred) && /^[0-9a-f]{64}$/.test(r.intro),
     'seed-only init derives cred ' + r.cred.slice(0, 12) + '… and mints a 32-byte intro tag');
  let refused = false;
  try { dev.json('mls_init', JSON.stringify({ cred: '00'.repeat(32), seed })); }
  catch (e) { refused = /does not match/.test(e.message); }
  ok(refused, 'a cred that is not the seed\'s identity key is refused');
  const again = dev.json('mls_init', JSON.stringify({ cred: r.cred, seed, sk: r.sk, pk: r.pk, intro: r.intro }));
  ok(again.cred === r.cred, 'and the derived cred is accepted back explicitly');
  return { cred: r.cred, intro: r.intro, sk: r.sk, pk: r.pk };
}
const a = person(A, seedA);
const b = person(B, seedB);
const again = A.json('mls_init', JSON.stringify({ cred: a.cred, seed: seedA, sk: a.sk, pk: a.pk, intro: a.intro }));
ok(again.intro === a.intro && again.pk === a.pk, 'an accepted intro tag and keys come back verbatim');

head('mls_contact_bundle');
const bundle = A.json('mls_contact_bundle', 'Ada').bundle;
ok(typeof bundle === 'string' && bundle.length > 0, 'bundle is a non-empty string (' + bundle.length + ' chars)');
const raw = Buffer.from(bundle, 'base64');
ok(raw.length > 0 && (raw[0] >> 5) === 5, 'bundle is base64 of a CBOR map (' + raw.length + ' bytes)');
ok(Buffer.from(raw).includes(Buffer.from(unhex(a.intro))), 'bundle carries the intro tag');
ok(entropyCalls > 0, 'entropy came from the host (' + entropyCalls + ' fills so far)');
console.log('         bundle: ' + bundle.slice(0, 72) + '…');

head('seal_seal / seal_open');
const tag = 'aa'.repeat(32), secret = 'bb'.repeat(32), inner = hex(new TextEncoder().encode('inner MLS bytes'));
const blob = A.text('seal_seal', JSON.stringify({ inner, tag, secret }));
ok(A.text('seal_open', JSON.stringify({ blob, tag, secret })) === inner, 'round trip');
try { A.text('seal_open', JSON.stringify({ blob, tag, secret: 'cc'.repeat(32) })); ok(false, 'wrong secret refused'); }
catch (e) { ok(/seal open failed/.test(e.message), 'wrong secret refused: ' + e.message); }

head('mls_join_intro (A scans B\'s bundle, B joins from the sealed Welcome)');
const bBundle = B.json('mls_contact_bundle', 'Bea').bundle;
// B's fresh KeyPackage is inside the bundle; a page would parse_and_verify. Here
// the KeyPackage is taken straight from the device, which is the same bytes.
const kp = B.json('mls_key_package', '').key_package;
const gid = A.json('mls_create_group', 'Tuesday club').group_id;
const add = A.json('mls_add_member', JSON.stringify({ group_id: gid, key_package: kp }));
const payload = cbor({ scanner_pk: unhex(a.cred), scanner_name: 'Ada', welcome: unhex(add.welcome), kind: 'forum', owner: unhex(a.cred) });
const sealed = A.text('seal_seal', JSON.stringify({ inner: hex(payload), tag: b.intro, secret: b.intro }));
const joined = B.json('mls_join_intro', JSON.stringify({ blob_b64: Buffer.from(unhex(sealed)).toString('base64') }));
ok(joined.group_id === gid, 'joined the group A created');
ok(joined.name === 'Tuesday club' && joined.kind === 'forum', `name "${joined.name}", kind ${joined.kind}`);
ok(joined.owner === a.cred && joined.scanner === a.cred && joined.scanner_name === 'Ada', 'owner and scanner are A, by the roster');
const roster = B.json('mls_roster', gid).members;
ok(roster.includes(a.cred) && roster.includes(b.cred), 'B\'s roster holds both');
ok(bBundle.length > 0, 'B\'s bundle also minted');

// forgery: a sender the roster does not contain.
const kp2 = B.json('mls_key_package', '').key_package;
const gid2 = A.json('mls_create_group', 'forged').group_id;
const add2 = A.json('mls_add_member', JSON.stringify({ group_id: gid2, key_package: kp2 }));
const forged = cbor({ scanner_pk: unhex('55'.repeat(32)), scanner_name: 'Mallory', welcome: unhex(add2.welcome), kind: 'forum', owner: unhex(a.cred) });
const sealed2 = A.text('seal_seal', JSON.stringify({ inner: hex(forged), tag: b.intro, secret: b.intro }));
try { B.json('mls_join_intro', JSON.stringify({ blob_b64: Buffer.from(unhex(sealed2)).toString('base64') })); ok(false, 'forged sender refused'); }
catch (e) { ok(/possible forgery/.test(e.message), 'forged sender refused: ' + e.message); }
// wrong mailbox: sealed to a tag that is not B's.
const sealed3 = A.text('seal_seal', JSON.stringify({ inner: hex(payload), tag: '99'.repeat(32), secret: '99'.repeat(32) }));
try { B.json('mls_join_intro', JSON.stringify({ blob_b64: Buffer.from(unhex(sealed3)).toString('base64') })); ok(false, 'wrong tag refused'); }
catch (e) { ok(/seal open failed/.test(e.message), 'wrong tag refused: ' + e.message); }

console.log('\n' + (failures ? `${failures} FAILED` : 'all passed'));
process.exit(failures ? 1 : 0);
