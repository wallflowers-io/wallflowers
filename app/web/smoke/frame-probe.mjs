#!/usr/bin/env node
// THE INBOUND DISPATCH, PROVED LOUD — keyholder.js's socket router, against a
// relay that says things this build does not know.
//
// WHY THIS IS SEPARATE FROM group-pair.mjs. That one needs the REAL relay, and
// `sock` in keyholder.js is one socket per origin: whichever URL opens it first
// keeps it. So a bogus-frame probe cannot share a page with the real run — it
// would either steal the socket or never get one. It gets its own page, its own
// keyholder origin and its own throwaway relay.
//
//   ./run.sh                          (8100 docs, 8103 keyholder)
//   node smoke/frame-probe.mjs
//
// WHAT IS UNDER TEST. `attach()` used to route `ack`, `eose`, `gap` and `msg`
// and then simply END — so a frame this build does not know fell straight
// through and left no trace anywhere, which is the one failure a client must
// never have: a browser that has stopped understanding its relay looks exactly
// like a browser with nothing to say. There is a default branch now, and a
// `msg` on a tag nobody subscribed to is said too. Both report on the port the
// interior is holding, not only to a console nobody is reading.
//
// THE RELAY HERE IS A PROP and says so: forty lines of RFC 6455 that answer the
// handshake, ack every pub, eose every sub, and then say three things on
// purpose. It proves the CLIENT's router. It proves nothing about the relay,
// which is what group-pair.mjs is for.
import net from 'node:net';
import crypto from 'node:crypto';
import { chromium } from 'playwright-core';

const DOCS = process.env.DOCS || 'http://localhost:8100';
const KH   = process.env.KH   || 'http://localhost:8103';
const SITE = 'cambridge-dd';
const EXE = process.env.CHROME_HEADLESS_SHELL ||
  process.env.HOME + '/Library/Caches/ms-playwright/chromium_headless_shell-1243/' +
  'chrome-headless-shell-mac-arm64/chrome-headless-shell';

let failures = 0;
const ok   = (c, m) => { console.log((c ? '  ok    ' : '  FAIL  ') + m); if (!c) failures++; };
const head = (m) => console.log('\n' + m);

/* ── forty lines of relay, enough to be wrong on purpose ──────────────────── */
const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const frame = (s) => {                       /* server→client text, unmasked */
  const b = Buffer.from(s);
  const hdr = b.length < 126
    ? Buffer.from([0x81, b.length])
    : Buffer.concat([Buffer.from([0x81, 126]), Buffer.from([b.length >> 8, b.length & 0xff])]);
  return Buffer.concat([hdr, b]);
};
const said = [];
const server = net.createServer((sock) => {
  let up = false, buf = Buffer.alloc(0);
  sock.on('data', (d) => {
    buf = Buffer.concat([buf, d]);
    if (!up) {
      const end = buf.indexOf('\r\n\r\n');
      if (end < 0) return;
      const key = /sec-websocket-key:\s*(\S+)/i.exec(buf.toString('utf8', 0, end));
      const accept = crypto.createHash('sha1').update(key[1] + GUID).digest('base64');
      sock.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n' +
                 'Connection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + '\r\n\r\n');
      up = true; buf = Buffer.alloc(0);
      /* THE THREE THINGS, sent the moment the socket is up. The client has not
         asked for any of them, which is the point: every one of them used to
         vanish. */
      sock.write(frame(JSON.stringify({ t: 'quorum', v: 2 })));   /* a frame from a newer relay */
      sock.write(frame('{not json at all'));                      /* a frame that will not parse */
      sock.write(frame(JSON.stringify({ t: 'msg', tag: 'ff'.repeat(32),
                                        seq: 1, blob: 'AAAA' })));/* a tag nobody subscribed to */
      return;
    }
    buf = Buffer.alloc(0);                   /* inbound is masked and unread — a prop */
  });
  sock.on('error', () => {});
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const RELAY = 'ws://127.0.0.1:' + server.address().port + '/v1/relay';

/* ── the fixture, and it announces itself ─────────────────────────────────── */
async function plant(seedHex) {
  const DB = 'pacific.keyholder', STORE = 'device', LOG = 'deltas', VER = 3;
  const db = await new Promise((res, rej) => {
    const r = indexedDB.open(DB, VER);
    r.onupgradeneeded = () => {
      const d = r.result;
      if (!d.objectStoreNames.contains(STORE)) d.createObjectStore(STORE);
      if (d.objectStoreNames.contains(LOG)) d.deleteObjectStore(LOG);
      d.createObjectStore(LOG, { keyPath: ['site', 'a', 'seq'] });
    };
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
  });
  const tx = (mode, fn) => new Promise((res, rej) => {
    const t = db.transaction(STORE, mode), q = fn(t.objectStore(STORE));
    t.oncomplete = () => res(q.result);
    t.onerror = () => rej(t.error);
  });
  /* Raced against the door's own `wrapKey()`, exactly as group-pair.mjs is —
     see the note there. Seal, read the key back, check it still opens. */
  const seal = async () => {
    let wrap = await tx('readonly', (s) => s.get('mls-wrap'));
    if (!wrap) {
      wrap = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false,
                                             ['encrypt', 'decrypt']);
      await tx('readwrite', (s) => s.put(wrap, 'mls-wrap'));
    }
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, wrap,
      new TextEncoder().encode(JSON.stringify({ seed: seedHex }))));
    const out = new Uint8Array(12 + ct.length);
    out.set(iv, 0); out.set(ct, 12);
    let s = ''; for (const b of out) s += String.fromCharCode(b);
    await tx('readwrite', (st) => st.put(btoa(s), 'mls-seed'));
  };
  const opens = async () => {
    const [key, blob] = await Promise.all([
      tx('readonly', (s) => s.get('mls-wrap')),
      tx('readonly', (s) => s.get('mls-seed'))
    ]);
    if (!key || !blob) return false;
    const raw = atob(blob), u = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) u[i] = raw.charCodeAt(i);
    try {
      const pt = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: u.slice(0, 12) }, key, u.slice(12));
      return JSON.parse(new TextDecoder().decode(pt)).seed === seedHex;
    } catch (e) { return false; }
  };
  let planted = false;
  for (let i = 0; i < 20 && !planted; i++) {
    await seal();
    await new Promise((r) => setTimeout(r, 150));
    planted = await opens();
  }
  await tx('readwrite', (st) => st.delete('mls-state'));
  db.close();
  if (!planted) throw new Error('the seed would not stay sealed under this origin’s wrap key');
  return true;
}

/* ── the embedder's half of the port protocol ─────────────────────────────── */
async function embed([origin, site]) {
  const f = document.createElement('iframe');
  f.style.cssText = 'position:absolute;width:0;height:0;border:0;visibility:hidden';
  f.src = origin + '/index.html';
  const ready = new Promise((res, rej) => {
    const timer = setTimeout(() => rej(new Error('the keyholder did not answer')), 20000);
    f.onload = () => {
      const ch = new MessageChannel();
      let n = 0;
      const waiting = {}, events = [];
      ch.port1.onmessage = (e) => {
        const d = e.data || {};
        if (d.t === 'pacific.denied') { clearTimeout(timer); return rej(new Error(d.why)); }
        if (d.t === 'pacific.ready') {
          clearTimeout(timer);
          window.__kh = {
            events,
            call: (m, a) => new Promise((y, n2) => {
              const id = ++n; waiting[id] = [y, n2];
              ch.port1.postMessage({ id, m, a: a || {} });
            })
          };
          return res(d.pk);
        }
        if (d.t) { events.push(d); return; }
        const w = waiting[d.id];
        if (!w) return;
        delete waiting[d.id];
        d.ok ? w[0](d.v) : w[1](new Error(d.e));
      };
      f.contentWindow.postMessage({ t: 'pacific.init', site }, new URL(origin).origin, [ch.port2]);
    };
  });
  document.body.appendChild(f);
  return ready;
}

const browser = await chromium.launch({ executablePath: EXE, args: ['--no-sandbox'] });
const ctx = await browser.newContext();
const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('  [pageerror] ' + e.message));
page.on('console', (m) => { if (m.type() === 'warning') said.push(m.text()); });

try {
  head('the fixture — an account seed, planted on the keyholder origin');
  await page.goto(KH + '/', { waitUntil: 'load' });
  ok(await page.evaluate(plant, 'c3'.repeat(32)), 'seed sealed into ' + KH + "'s store (FIXTURE)");

  head('a relay that says three things this build does not know: ' + RELAY);
  await page.goto(DOCS + '/', { waitUntil: 'load' });
  ok(!!(await page.evaluate(embed, [KH, SITE])), 'the keyholder answered');

  /* `group.watch` is the cheapest method that opens a socket: no groups, so no
     subscriptions, so nothing but the router is under test. */
  const w = await page.evaluate(([r]) => window.__kh.call('group.watch', { relay: r }), [RELAY]);
  ok((w.watching || []).length === 0, 'group.watch opened the socket with no groups to watch');

  let evs = [];
  for (let i = 0; i < 40; i++) {
    await new Promise((r) => setTimeout(r, 150));
    evs = await page.evaluate(() => window.__kh.events);
    if (evs.filter((e) => e.t === 'pacific.error').length >= 3) break;
  }
  const errs = evs.filter((e) => e.t === 'pacific.error');
  const why = errs.map((e) => e.why).join(' | ');

  head('every one of them reached the interior, not just a console');
  ok(errs.some((e) => /unknown frame t="quorum"/.test(e.why)),
     'an unknown frame is named and refused — ' +
     (errs.find((e) => /unknown frame/.test(e.why)) || {}).why);
  ok(errs.some((e) => /not JSON/.test(e.why)),
     'a frame that will not parse is said, with what it was');
  ok(errs.some((e) => /nothing is listening to/.test(e.why)),
     'a blob on an unsubscribed tag is said rather than dropped in silence');
  ok(errs.every((e) => e.where === 'relay'), 'all three are attributed to the relay');
  if (!errs.length) console.log('  saw: ' + JSON.stringify(evs) + ' | console: ' + said.join(' // '));
  else console.log('  and the member is told, in the interior: ' + why.slice(0, 180) + '…');
} catch (e) {
  ok(false, String(e && e.message || e));
} finally {
  await browser.close();
  server.close();
}

console.log('\n' + (failures ? failures + ' FAILED'
  : 'all passed — a client that has stopped understanding its relay says so'));
process.exit(failures ? 1 : 0);
