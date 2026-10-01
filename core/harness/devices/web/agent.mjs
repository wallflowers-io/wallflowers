// agent.mjs — one browser device, driven over stdin/stdout.
//
// ONE PROCESS IS ONE DEVICE. It launches its own headless Chromium with its own
// context, so the keyholder origin's IndexedDB it writes is this device's and
// nobody else's. Two agents on two keyholder ports are two devices with two keys
// and two logs, which is what app/web/README.md says two origins are.
//
// TWO PAGES, both on the member's origin:
//
//   keyholder   a blank page that frames a keyholder and holds the port — the
//               same handshake pacific.js's keyholderPort() does, and the one
//               docs/pacific-two-devices.html does by hand. A device with no UI.
//   embed       a member's own page, deliberately hostile CSS and all, with
//               pacific.js mounted into it. The interior is driven through its
//               open shadow root the way a person would — the mark, the discs,
//               the composer — and its world is read back through the api
//               Pacific.mount returns.
//
// WHY THE MEMBER'S ORIGIN IS ROUTED, NOT SERVED. keyholder.js refuses any page
// not on an origin registered for the site, and for cambridge-dd that is
// http://localhost:8100. Something may well be listening there already — run.sh
// and smoke/run.sh both start a docs server on it, and `run.sh stop` kills
// whatever holds the port. So this agent never touches :8100 on the network: it
// answers every request for that origin itself, from app/web/docs on disk,
// read-only. The browser sees the registered origin; no server is involved.
// The KEYHOLDER is a real server on a port of its own, because its origin is
// the device boundary and has to be a real one.
//
//   → {"id": 1, "verb": "open", "args": {"page": "keyholder", "keyholder": "http://localhost:51234"}}
//   ← {"id": 1, "ok": true, "v": {"pk": "…", "origin": "http://localhost:8100"}}

import { chromium } from 'playwright-core';
import { readFile } from 'node:fs/promises';
import { readdirSync, existsSync } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import readline from 'node:readline';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const WEB = process.env.PACIFIC_WEB || path.resolve(HERE, '../../../../app/web');
const DOCS = path.join(WEB, 'docs');
/* Registered for cambridge-dd in keyholder.js's ALLOWED. Not a choice. */
const MEMBER = 'http://localhost:8100';
const HOST = 'div[data-pacific]';

const out = (o) => process.stdout.write(JSON.stringify(o) + '\n');
const note = (s) => process.stderr.write(String(s) + '\n');

/* THE CHROMIUM IS PINNED, because the versions do not line up on their own.
   The cached playwright-core asks for a full Chrome build that is not installed;
   its matching headless shell is. multi-platform-harness.html §07 says to pin
   this rather than rediscover it, so: the headless shell whose revision is the
   one playwright-core wants, else the newest one present, else a loud failure. */
function pinnedChromium() {
  if (process.env.HARNESS_CHROMIUM) return process.env.HARNESS_CHROMIUM;
  const cache = path.join(os.homedir(), 'Library', 'Caches', 'ms-playwright');
  let wanted = '';
  try { wanted = (chromium.executablePath().match(/chromium-(\d+)/) || [])[1] || ''; } catch (e) {}
  const shells = readdirSync(cache)
    .filter((d) => d.startsWith('chromium_headless_shell-'))
    .map((d) => ({ d, rev: d.split('-').pop() }))
    .sort((a, b) => Number(b.rev) - Number(a.rev));
  const pick = shells.find((s) => s.rev === wanted) || shells[0];
  if (!pick) throw new Error('no chromium_headless_shell under ' + cache);
  const inner = readdirSync(path.join(cache, pick.d)).find((d) => d.startsWith('chrome-headless-shell'));
  const exe = path.join(cache, pick.d, inner || '', 'chrome-headless-shell');
  if (!existsSync(exe)) throw new Error('headless shell missing at ' + exe);
  return exe;
}

const MIME = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.mjs': 'text/javascript',
  '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.woff2': 'font/woff2'
};

const PAGES = {
  '/__harness/keyholder.html': `<!doctype html>
<meta charset="utf-8"><title>harness · keyholder device</title>
<body><script>
(function () {
  var q = new URLSearchParams(location.search), KH = q.get('kh'), SITE = q.get('site');
  window.__pushes = [];
  window.__errors = [];
  window.__ready = new Promise(function (res, rej) {
    var f = document.createElement('iframe');
    f.style.cssText = 'position:absolute;width:0;height:0;border:0;visibility:hidden';
    f.src = KH.replace(/\\/$/, '') + '/index.html';
    var t = setTimeout(function () { rej(new Error('keyholder did not answer in 15s')); }, 15000);
    f.onload = function () {
      var ch = new MessageChannel(), n = 0, waiting = {};
      ch.port1.onmessage = function (e) {
        var d = e.data || {};
        if (d.t === 'pacific.ready') {
          clearTimeout(t);
          window.__call = function (m, a) {
            return new Promise(function (rs, rj) {
              var id = ++n; waiting[id] = [rs, rj];
              ch.port1.postMessage({ id: id, m: m, a: a || {} });
            });
          };
          return res({ pk: d.pk, agent: d.agent, persisted: !!d.persisted });
        }
        if (d.t === 'pacific.denied') { clearTimeout(t); return rej(new Error('denied: ' + d.why)); }
        if (d.t === 'pacific.push') { window.__pushes.push(d.v); return; }
        if (d.t === 'pacific.error') { window.__errors.push(d.where + ': ' + d.why); return; }
        if (d.id && waiting[d.id]) {
          var w = waiting[d.id]; delete waiting[d.id];
          return d.ok ? w[0](d.v) : w[1](new Error(d.e));
        }
      };
      f.contentWindow.postMessage({ t: 'pacific.init', site: SITE }, new URL(KH).origin, [ch.port2]);
    };
    document.body.appendChild(f);
  });
})();
</script>`,

  /* A member's page that does everything a host page is entitled to do to
     break an embedded component: a content-box reset, buttons unset, huge
     headings, letter-spacing on the body, a transform on an ancestor. None of
     it may reach the interior, which is the claim the embed makes. */
  '/__harness/embed.html': `<!doctype html>
<meta charset="utf-8"><title>A member's own site</title>
<style>
  *, *::before, *::after { box-sizing: content-box !important; }
  body { font: 19px/1.1 "Comic Sans MS", cursive; letter-spacing: .2em; background: #fdf6e3; margin: 40px; }
  button { all: unset; font-family: cursive; }
  h1, h2 { font-size: 60px; }
  .row { transform: rotate(2deg); }
  main { transform: translateZ(0); }
</style>
<main><h1>Cambridge Digital Democracy</h1><p class="row">Their words, their CSS, their domain.</p></main>
<script src="/pacific.js"></script>
<script>
(function () {
  var q = new URLSearchParams(location.search);
  window.__pushes = [];
  window.__ready = Pacific.mount({ site: q.get('site'), keyholder: q.get('kh'), seed: Pacific.seed })
    .then(function (api) {
      window.__api = api;
      window.__call = function (m, a) { return api.rpc.call(m, a || {}); };
      api.rpc.onPush(function (v) { window.__pushes.push(v); });
      var d = api.device || {};
      return { pk: d.pk, agent: d.agent, persisted: !!d.persisted };
    });
})();
</script>`
};

async function member(route) {
  const url = new URL(route.request().url());
  if (PAGES[url.pathname]) {
    return route.fulfill({ status: 200, contentType: MIME['.html'], body: PAGES[url.pathname] });
  }
  const file = path.normalize(path.join(DOCS, decodeURIComponent(url.pathname)));
  if (!file.startsWith(DOCS + path.sep)) return route.fulfill({ status: 403, body: 'outside app/web/docs' });
  try {
    return route.fulfill({
      status: 200, body: await readFile(file),
      contentType: MIME[path.extname(file)] || 'application/octet-stream'
    });
  } catch (e) {
    return route.fulfill({ status: 404, body: 'not in app/web/docs: ' + url.pathname });
  }
}

let browser = null, context = null, page = null, mode = null;
const logs = [];
function remember(kind, text) {
  logs.push({ kind, text: String(text).slice(0, 2000) });
  if (logs.length > 2000) logs.shift();
}
function need(kind) {
  if (!page) throw new Error('no page open — call open first');
  if (kind && mode !== kind) throw new Error('this verb needs the ' + kind + ' page, and this device opened ' + mode);
}
const screen = () => page.locator(HOST).getAttribute('data-screen');
const deltaCount = () => page.evaluate(() => window.__call('store.deltas', {}).then((d) => d.length));
/* Wait on a predicate over the device's state. The interval only stops the loop
   spinning; what ends the wait is the state, or a timeout that says what it was
   waiting for. */
async function until(pred, timeout, what) {
  const t0 = Date.now();
  while (!(await pred())) {
    if (Date.now() - t0 > timeout) throw new Error('timed out after ' + timeout + 'ms waiting for ' + what);
    await new Promise((r) => setTimeout(r, 100));
  }
}

const V = {
  async open({ page: kind = 'keyholder', keyholder, site = 'cambridge-dd', width = 1280, height = 800 }) {
    if (!keyholder) throw new Error('open wants a keyholder origin');
    if (!PAGES['/__harness/' + kind + '.html']) throw new Error('no page called ' + kind);
    if (page) await page.close();
    page = await context.newPage();
    await page.setViewportSize({ width, height });
    page.on('console', (m) => remember('console.' + m.type(), m.text()));
    page.on('pageerror', (e) => remember('pageerror', e.message));
    mode = kind;
    await page.goto(MEMBER + '/__harness/' + kind + '.html?' + new URLSearchParams({ kh: keyholder, site }));
    const v = await page.evaluate(() => window.__ready);
    return { ...v, origin: MEMBER, page: kind, keyholder };
  },

  /* Any keyholder method, over the port the page holds. On the embed page this
     is the SAME port the interior's store uses — one channel, as pacific.js
     insists — so a harness call and an interior commit are indistinguishable to
     the keyholder. */
  async rpc({ m, a = {} }) {
    need();
    if (!m) throw new Error('rpc wants m, the keyholder method');
    return page.evaluate(([m, a]) => window.__call(m, a), [m, a]);
  },

  /* Several requests on the port at once — how two quick actions by one person,
     or one site open in two tabs, arrive at a keyholder. Each answer comes back
     in order with its own ok, so one failure does not hide the others. */
  async rpc_many({ calls = [] }) {
    need();
    return page.evaluate((calls) => Promise.all(calls.map((c) =>
      window.__call(c.m, c.a || {}).then(
        (v) => ({ ok: true, v: v }),
        (e) => ({ ok: false, e: String((e && e.message) || e) })))), calls);
  },

  async pushes() { need(); return page.evaluate(() => window.__pushes.length); },

  /* Waits in the page on the push counter — the keyholder telling the host a
     change arrived from somewhere else. State, not a sleep. */
  async push_wait({ after = 0, timeout = 20000 }) {
    need();
    await page.waitForFunction((n) => window.__pushes.length > n, after, { timeout, polling: 100 });
    return page.evaluate(() => ({ count: window.__pushes.length }));
  },

  /* What the device folds. On the embed page, the world the interior is
     actually drawing — not a second fetch of it. */
  async world() {
    need();
    return page.evaluate(() => (window.__api ? window.__api.world() : window.__call('store.load', {})));
  },

  async seed() { need('embed'); return page.evaluate(() => window.Pacific.seed); },

  /* The host page's own view of itself: which databases IT can see, whether
     any of the interior leaked into its light DOM, and what it frames. */
  async host() {
    need();
    return page.evaluate(async () => {
      const dbs = indexedDB.databases ? (await indexedDB.databases()).map((d) => d.name) : null;
      return {
        origin: location.origin,
        databases: dbs,
        hosts: [...document.querySelectorAll('[data-pacific]')].map((h) => ({
          site: h.getAttribute('data-pacific'), win: h.dataset.win, screen: h.dataset.screen || null,
          shadow: !!h.shadowRoot, device: h.dataset.device || null
        })),
        interiorInLightDom: !!document.querySelector('#ch-in, #logo, #rail, #discs'),
        frames: [...document.querySelectorAll('iframe')].map((f) => new URL(f.src).origin),
        errors: window.__errors || []
      };
    });
  },

  /* ── the interior, through its shadow root ─────────────────────────────── */

  async ui_open() {
    need('embed');
    const host = page.locator(HOST);
    if ((await host.getAttribute('data-win')) === 'closed') await page.locator(HOST + ' #logo').click();
    await page.waitForFunction((sel) => document.querySelector(sel).dataset.win !== 'closed', HOST);
    return { win: await host.getAttribute('data-win') };
  },

  async ui_view({ view }) {
    need('embed');
    await V.ui_open();
    if (view === 'notes') {
      throw new Error("the note disc's tap begins a note and its hold opens the list — not a view to switch to");
    }
    if (view === 'chat' || view === 'life') {
      /* A tap on the disc of the screen you are ALREADY on does something else
         (chat toggles rooms/chats, life goes to the feed), so only tap to arrive. */
      if ((await screen()) !== view) await page.locator(HOST + ' [data-slot="' + view + '"]').click();
      await page.waitForFunction(([sel, v]) => document.querySelector(sel).dataset.screen === v, [HOST, view]);
    } else {
      await page.locator(HOST + ' #views button[data-view="' + view + '"]').click();
      await page.waitForFunction(([sel, v]) => {
        const b = document.querySelector(sel).shadowRoot.querySelector('#views button[data-view="' + v + '"]');
        return !!b && b.classList.contains('on');
      }, [HOST, view]);
    }
    return { screen: await screen() };
  },

  /* Type into CHAT's composer and send, as a person would. Returns once the
     message is in the world the interior draws — which means the keyholder
     folded the commit and handed the world back. */
  async ui_chat_say({ conv, text }) {
    need('embed');
    if (!text) throw new Error('ui_chat_say wants text');
    await V.ui_view({ view: 'chat' });
    const input = page.locator(HOST + ' #ch-in');
    if ((await input.getAttribute('placeholder')) !== 'Message') {
      throw new Error('CHAT is showing rooms, not conversations');
    }
    if (conv) {
      /* Opening a conversation commits a `read`. A person's click has landed
         before they start typing; a driver that types in the same millisecond
         issues two commits at once, and keyholder.js's append() gives both the
         same seq, losing one (case W4). So the click is let land first, waited
         on in the keyholder's own log rather than slept on. */
      const n = await deltaCount();
      await page.locator(HOST + ' #chlist [data-conv="' + conv + '"]').click();
      await until(async () => (await deltaCount()) > n, 20000, 'the conversation click to commit its read');
    }
    const chosen = conv || await page.locator(HOST + ' #chlist [data-conv].on').first().getAttribute('data-conv');
    if (!chosen) throw new Error('no conversation is open — this world has no convs');
    await input.fill(text);
    await input.press('Enter');
    await page.waitForFunction(([c, t]) => {
      const x = (window.__api.world().convs || []).find((k) => k.id === c);
      return !!x && x.msgs.some((m) => m.s === t);
    }, [chosen, text], { timeout: 20000, polling: 100 });
    return { conv: chosen, rendered: await page.locator(HOST + ' #chlog').innerText() };
  },

  async ui_text({ selector }) {
    need('embed');
    return page.locator(HOST + ' ' + selector).first().innerText();
  },

  async screenshot({ path: file, full = false }) {
    need();
    await page.screenshot({ path: file, fullPage: !!full });
    return file;
  },

  async console({ since = 0 }) { return { next: logs.length, lines: logs.slice(since) }; }
};

async function main() {
  const exe = pinnedChromium();
  browser = await chromium.launch({ executablePath: exe, headless: true });
  context = await browser.newContext();
  /* LOCAL NETWORK ACCESS, granted to the member's origin and nothing else.
     A document this agent answers from disk has no IP address behind it, so
     Chromium files it as PUBLIC — and a public page framing a keyholder on
     loopback is exactly what Local Network Access blocks, with
     ERR_BLOCKED_BY_LOCAL_NETWORK_ACCESS_CHECKS and a frame whose origin is
     'null'. That is an artefact of routing, not of Pacific: run.sh serves the
     member page from a real loopback server and never meets it, and production
     frames a public keyholder from a public page and never meets it either.
     Granting the permission is what a person clicking Allow does. The check
     stays on for every other origin; it is not disabled. */
  await context.grantPermissions(['local-network-access'], { origin: MEMBER });
  await context.route(MEMBER + '/**', member);
  out({ t: 'ready', agent: 'web', pid: process.pid, chromium: exe, version: browser.version(), docs: DOCS });

  const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of rl) {
    if (!line.trim()) continue;
    let req;
    try { req = JSON.parse(line); } catch (e) { note('bad request line: ' + line.slice(0, 200)); continue; }
    try {
      const fn = V[req.verb];
      if (!fn) throw new Error('no verb ' + req.verb + ' (has: ' + Object.keys(V).join(', ') + ')');
      out({ id: req.id, ok: true, v: await fn(req.args || {}) });
    } catch (e) {
      /* Playwright appends a call log; the first lines carry the cause. */
      const msg = String((e && e.message) || e).split('\n').slice(0, 3).join(' | ');
      out({ id: req.id, ok: false, e: msg.slice(0, 1200) });
    }
  }
  await browser.close();
}

main().catch(async (e) => {
  note('agent.mjs died: ' + ((e && e.stack) || e));
  try { if (browser) await browser.close(); } catch (x) {}
  process.exit(1);
});
