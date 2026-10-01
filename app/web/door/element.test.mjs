/* element.test.mjs — Sign in with WallFlowers' change stream (element.js, session.events), NC-133.
   A stream ends on a Door restart, a proxy's idle cut or a network change; one that ends or
   fails, not by stop(), opens again after a wait that doubles from 2 s to a minute, reset once
   one opens, as the webapp's listen() waits. A reopened stream says `changed` once: the Door
   says nothing on open, and a change may have come while it was down. A 401 is the session
   over: not asked again, and onEnded has the refusal. stop() ends it for good.

   The Door, the page's IndexedDB and its timers are stubs; element.js runs as a page runs it,
   with this runtime's WebCrypto for its DPoP proofs.
   Run: node --test app/web/door/element.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./element.js', import.meta.url), 'utf8');
const DOOR = 'https://door.example.test';
const CLIENT = 'site.example.test';

/* Until `ok()` holds, or fail naming `what`: the element's work is WebCrypto and streams. */
async function until(ok, what) {
  for (const end = Date.now() + 3000; !ok(); ) {
    if (Date.now() > end) assert.fail('never: ' + what);
    await new Promise((r) => setTimeout(r, 5));
  }
}

/* One page holding a session. `plan`: what each /v2/events answers in turn, 'open' (a stream the
   test feeds), 'network' (the fetch rejects) or a status; past its end, 'open'. */
async function page(plan = []) {
  const keys = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']);
  const record = { client: CLIENT, site: 's'.repeat(64), token: 'the-token', expires: Date.now() + 3600e3, keys };
  const indexedDB = {
    open() {
      const r = {};
      setImmediate(() => {
        r.result = { transaction() { const t = { objectStore: () => ({ get() { setImmediate(() => t.oncomplete()); return { result: record }; } }) }; return t; } };
        r.onsuccess();
      });
      return r;
    },
  };
  // The page's timers, held: each wait is read, and run when the test says.
  const timers = [];
  const waiting = () => timers.filter((t) => !t.cleared && !t.ran);
  const fire = () => { const t = waiting()[0]; assert.ok(t, 'a reopen is waiting'); t.ran = true; t.fn(); return t.ms; };
  // The Door's /v2/events.
  const calls = [], streams = [];
  const fetch = async (url, init) => {
    calls.push({ url, auth: init.headers.get('authorization'), dpop: init.headers.get('dpop'), credentials: init.credentials });
    const next = plan.length ? plan.shift() : 'open';
    if (next === 'network') throw new TypeError('Failed to fetch');
    if (typeof next === 'number') return new Response(next === 401 ? 'no such session' : 'the Door is restarting', { status: next });
    const s = { cancelled: false };
    streams.push(s);
    return new Response(new ReadableStream({ start(c) { s.c = c; }, cancel() { s.cancelled = true; } }), { status: 200, headers: { 'content-type': 'text/event-stream' } });
  };
  const send = (i, v) => streams[i].c.enqueue(new TextEncoder().encode(`event: changed\ndata: ${v}\n\n`));
  const ctx = {
    document: { currentScript: { src: DOOR + '/v2/signin.js' } },
    TextEncoder, TextDecoder, crypto, btoa, URL, URLSearchParams, Headers, fetch, indexedDB,
    setTimeout: (fn, ms) => timers.push({ fn, ms }) , clearTimeout: (id) => { if (timers[id - 1]) timers[id - 1].cleared = true; },
    sessionStorage: { getItem: () => null, setItem() {}, removeItem() {} }, location: {}, history: {},
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  const session = await ctx.WallFlowers.current({ client: CLIENT });
  assert.ok(session, 'the page holds a session');
  return { session, calls, streams, send, waiting, fire };
}

test('a stream that ends opens again after 2 s, says changed once, and delivers the next change', async () => {
  const p = await page();
  const got = [];
  const stop = p.session.events((v) => got.push(v));
  await until(() => p.streams.length === 1, 'the stream opened');
  p.send(0, '1');
  await until(() => got.length === 1, 'the first change');
  p.streams[0].c.close();
  await until(() => p.waiting().length === 1, 'a reopen waits');
  assert.equal(p.fire(), 2000, 'after 2 s');
  await until(() => p.streams.length === 2 && got.length === 2, 'the stream opened again, and said so');
  p.send(1, '2');
  await until(() => got.length === 3, 'the next change, on the new stream');
  assert.deepEqual(got, ['1', '', '2'], 'the change, once for the reopen, the next');
  for (const c of p.calls) {
    assert.deepEqual([c.url, c.auth, c.credentials], [DOOR + '/v2/events', 'DPoP the-token', 'omit']);
    assert.match(c.dpop, /^[\w-]+\.[\w-]+\.[\w-]+$/, 'a proof on each');
  }
  assert.notEqual(p.calls[0].dpop, p.calls[1].dpop, 'a fresh proof for the reopen');
  stop();
});

test('the wait doubles from 2 s to a minute, and is reset once a stream opens', async () => {
  const p = await page([503, 'network', 503, 503, 503, 503, 503]);
  const got = [];
  p.session.events((v) => got.push(v));
  const waits = [];
  for (let i = 0; i < 7; i++) {
    await until(() => p.waiting().length === 1, `wait ${i + 1}`);
    waits.push(p.fire());
  }
  await until(() => p.streams.length === 1, 'a stream opened at last');
  // A read that fails, as a network change fails it: the stream opens again, from 2 s.
  p.streams[0].c.error(new TypeError('network changed'));
  await until(() => p.waiting().length === 1, 'a reopen after the failed read');
  waits.push(p.fire());
  assert.deepEqual(waits, [2000, 4000, 8000, 16000, 32000, 60000, 60000, 2000]);
  await until(() => p.streams.length === 2, 'and it opened');
});

test('stop() ends it for good: the open stream cancelled, a waiting reopen cleared', async () => {
  // While a stream is open.
  let p = await page();
  let stop = p.session.events(() => {});
  await until(() => p.streams.length === 1, 'the stream opened');
  stop();
  await until(() => p.streams[0].cancelled, 'the stream cancelled');
  await new Promise((r) => setTimeout(r, 50));
  assert.deepEqual([p.waiting().length, p.calls.length], [0, 1], 'no reopen');
  // While a reopen waits.
  p = await page();
  stop = p.session.events(() => {});
  await until(() => p.streams.length === 1, 'the stream opened');
  p.streams[0].c.close();
  await until(() => p.waiting().length === 1, 'a reopen waits');
  stop();
  assert.equal(p.waiting().length, 0, 'the waiting reopen cleared');
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(p.calls.length, 1, 'and never asked');
  // Before the first answer comes.
  p = await page();
  const got = [];
  stop = p.session.events((v) => got.push(v));
  stop();
  await until(() => p.streams.length === 1 && p.streams[0].cancelled, 'the answer that came after stop() cancelled');
  assert.deepEqual([got, p.waiting().length], [[], 0]);
});

test('a 401 is the session over: not asked again, and onEnded has the refusal', async () => {
  const p = await page([401]);
  const ended = [];
  p.session.events(() => assert.fail('no change'), (e) => ended.push(e));
  await until(() => ended.length === 1, 'onEnded');
  assert.deepEqual([ended[0].message, ended[0].status], ['no such session', 401], "the Door's refusal, and its status");
  await new Promise((r) => setTimeout(r, 50));
  assert.deepEqual([p.waiting().length, p.calls.length, ended.length], [0, 1, 1], 'not asked again');
  // After a reopen too; and a page that gave no onEnded is not thrown at.
  const q = await page(['open', 401]);
  q.session.events(() => {});
  await until(() => q.streams.length === 1, 'the stream opened');
  q.streams[0].c.close();
  await until(() => q.waiting().length === 1, 'a reopen waits');
  q.fire();
  await until(() => q.calls.length === 2, 'the reopen asked');
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(q.waiting().length, 0, 'the 401 ends it');
});
