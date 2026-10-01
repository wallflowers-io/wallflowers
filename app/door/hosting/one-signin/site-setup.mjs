#!/usr/bin/env node
/* site-setup.mjs: the Site's one sign-in (one-signin-2.1.0.js) as a build step, confirmed by the
   owner's passkey on this Mac (W-90; Ralph, 29 Sep: "bundled as a build step which requires my
   touch id, rather than handling browser terminals").

     make site-setup SITE=egregore MARK=<png, jpeg or webp>

   The five bundles from bundles.sh; then the Door's window in the default browser for the Site's
   setup client, wallflowers-setup-<SITE> (clients.stand-in.json), with an S256 challenge and a
   state, answered on that client's one loopback callback (its port taken, this stops: the Door
   redirects nowhere else); the code traded at /v2/token with a DPoP proof by a key made for this
   run; then the snippet, verbatim but for its BUNDLES line, each call to the Door with the token
   and a fresh proof, its file picker MARK, drawn smaller by sips where the snippet draws it
   smaller. Signed out at exit. The token and the key never leave this process; the only files
   written are the mark's copies for sips, in a private directory removed at exit.

   A RE-RUN ONLY: a site's token adds no one (account.rs), so on a Site the console's run has not
   set up it stops at the Site's add, before any write.

   DOOR (https://app.wallflowers.io), ARC_BUNDLE_URL (bundles.sh's), SITE_SETUP_OPEN (open) and
   SITE_SETUP_CLIENTS (clients.stand-in.json) name others, for the tests. */
import crypto from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { execFileSync, spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const DOOR = (process.env.DOOR || 'https://app.wallflowers.io').replace(/\/+$/, '');
const CLIENTS = process.env.SITE_SETUP_CLIENTS || path.join(here, '../../clients.stand-in.json');
const OPEN = process.env.SITE_SETUP_OPEN || 'open';
// The snippet's BUNDLES line as it ships, the one line replaced (as its stub test replaces it).
const PLACEHOLDER = "const BUNDLES = ['<the Site>', '<the Host>', '<Resources>', '<Skills, Time & Services>', '<Financial Support>'];";
const WINDOW_WAIT = 5 * 60 * 1000;
const CALL_WAIT = 2 * 60 * 1000;

const b64u = (b) => Buffer.from(b).toString('base64url');
const sha256 = (s) => crypto.createHash('sha256').update(s).digest();

let token = null, key = null, scratch = null, server = null;

// RFC 9449: a proof for one request, by this run's key; `ath` once there is a token.
function proof(htm, htu, t) {
  const { x, y } = key.publicKey.export({ format: 'jwk' });
  const claims = { htm, htu, jti: b64u(crypto.randomBytes(18)), iat: Math.floor(Date.now() / 1000) };
  if (t) claims.ath = b64u(sha256(t));
  const input = b64u(JSON.stringify({ typ: 'dpop+jwt', alg: 'ES256', jwk: { kty: 'EC', crv: 'P-256', x, y } })) + '.' + b64u(JSON.stringify(claims));
  return input + '.' + b64u(crypto.sign('sha256', Buffer.from(input), { key: key.privateKey, dsaEncoding: 'ieee-p1363' }));
}

// The snippet's fetch: a Door route, with the token and a fresh proof; no cookie.
function doorFetch(p, o = {}) {
  if (!/^\/v2\/[a-z/]+$/.test(p)) return Promise.reject(new Error(p + ' is not a Door route'));
  const method = o.method || 'GET', url = DOOR + p;
  return fetch(url, { method, body: o.body, signal: AbortSignal.timeout(CALL_WAIT),
    headers: { ...o.headers, authorization: 'DPoP ' + token, dpop: proof(method, url, token) } });
}

// THE FILE PICKER: MARK, read from disk; the picture drawn smaller by sips, not a canvas.
const mimeOf = (b) =>
  b.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])) ? 'image/png'
  : b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff ? 'image/jpeg'
  : b.subarray(0, 4).toString('latin1') === 'RIFF' && b.subarray(8, 12).toString('latin1') === 'WEBP' ? 'image/webp'
  : 'application/octet-stream';
let copies = 0;
const copy = (bytes, ext) => { const f = path.join(scratch, `${++copies}.${ext}`); fs.writeFileSync(f, bytes, { mode: 0o600 }); return f; };
const sips = (...a) => execFileSync('sips', a, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
function shims(markFile) {
  const mark = { bytes: fs.readFileSync(markFile) };
  const document = {
    body: { appendChild(i) { setImmediate(() => { i.files = [mark]; i.onchange(); }); } },
    createElement(tag) {
      if (tag === 'input') return { style: '', remove() {} };
      if (tag === 'canvas') return canvas();
      throw new Error('no <' + tag + '> here');
    },
  };
  class FileReader {
    readAsDataURL(f) { this.result = 'data:' + mimeOf(f.bytes) + ';base64,' + f.bytes.toString('base64'); setImmediate(() => this.onload()); }
  }
  // Loaded at once: a picture sips cannot read throws here, which rejects the snippet's promise.
  class Image {
    set src(url) {
      const m = /^data:[^;]*;base64,(.*)$/.exec(url);
      this.file = copy(Buffer.from(m ? m[1] : '', 'base64'), 'img');
      let got;
      try { got = sips('-g', 'pixelWidth', '-g', 'pixelHeight', '-g', 'hasAlpha', this.file); } catch { throw new Error('the mark: not a picture sips reads'); }
      [this.naturalWidth, this.naturalHeight] = [/pixelWidth: (\d+)/, /pixelHeight: (\d+)/].map(r => +(r.exec(got) || [])[1]);
      if (!this.naturalWidth || !this.naturalHeight) throw new Error('the mark: not a picture sips reads');
      this.alpha = /hasAlpha: yes/.test(got);
      this.onload();
    }
  }
  // A JPEG when one is asked for. sips writes no webp: asked for one, a PNG where the picture has
  // alpha (as Safari gives; the snippet then asks for a JPEG if it is over), else a JPEG.
  const canvas = () => ({
    width: 0, height: 0,
    getContext() { const c = this; return { drawImage(img) { c.img = img; } }; },
    toDataURL(type, quality) {
      const [fmt, mime] = type !== 'image/jpeg' && this.img.alpha ? ['png', 'image/png'] : ['jpeg', 'image/jpeg'];
      const out = path.join(scratch, `${++copies}.${fmt}`);
      sips('-z', String(this.height), String(this.width), '-s', 'format', fmt, ...(fmt === 'jpeg' ? ['-s', 'formatOptions', String(Math.round(quality * 100))] : []), this.img.file, '--out', out);
      return 'data:' + mime + ';base64,' + fs.readFileSync(out).toString('base64');
    },
  });
  // The steps, as the snippet says them; its console-only lines (the picker's) are not.
  const console_ = { log: (...a) => {
    if (typeof a[0] === 'string' && /^(✓ |already: )/.test(a[0])) console.log(a[0]);
    else if (a.length === 1 && a[0] && typeof a[0] === 'object') console.log(a[0]);
  } };
  return { document, FileReader, Image, console: console_ };
}

// The window's answer on the callback: the code, once, for this run's state.
function answer(cb, state) {
  return new Promise((res, rej) => {
    const late = setTimeout(() => rej(new Error('the window did not answer within 5 minutes')), WINDOW_WAIT);
    server.on('request', (q, r) => {
      const u = new URL(q.url, cb);
      if (q.method !== 'GET' || u.pathname !== cb.pathname) { r.writeHead(404).end(); return; }
      const end = (code, text) => { r.writeHead(code, { 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'no-store', connection: 'close' }); r.end(text); };
      const p = u.searchParams;
      clearTimeout(late);
      if (p.get('state') !== state) { end(400, 'Refused.'); rej(new Error('the callback carries another state; nothing traded')); }
      else if (p.get('error')) { end(200, 'Cancelled.'); rej(new Error('the window answered ' + p.get('error'))); }
      else if (!p.get('code')) { end(400, 'Refused.'); rej(new Error('the callback carries no code')); }
      else { end(200, 'Signed in.'); res(p.get('code')); }
    });
  });
}

async function main([site, markFile]) {
  if (!site || !markFile) throw new Error('make site-setup SITE=<slug> MARK=<picture>');
  const client = 'wallflowers-setup-' + site;
  const reg = JSON.parse(fs.readFileSync(CLIENTS, 'utf8'))[client];
  if (!reg) throw new Error(`no client ${client} in ${CLIENTS}`);
  if ((reg.callbacks || []).length !== 1) throw new Error(`${client}: one callback, not ${(reg.callbacks || []).length}`);
  const callback = reg.callbacks[0], cb = new URL(callback);
  if (cb.protocol !== 'http:' || cb.hostname !== '127.0.0.1' || !cb.port) throw new Error(`${client}: ${callback} is not a loopback callback`);
  let src = fs.readFileSync(path.join(here, 'one-signin-2.1.0.js'), 'utf8');
  const SITE = (/^const SITE = '([0-9a-f]{64})';$/m.exec(src) || [])[1], SLUG = (/^const SLUG = '([^']*)';$/m.exec(src) || [])[1];
  if (SLUG !== site) throw new Error(`the snippet is ${SLUG}'s, not ${site}'s`);
  if (SITE !== reg.site) throw new Error(`${client} is registered for ${reg.site}, the snippet's Site is ${SITE}`);
  if (src.split(PLACEHOLDER).length !== 2) throw new Error("the snippet's BUNDLES line is not the one this replaces");
  const shim = shims(markFile);
  if (mimeOf(fs.readFileSync(markFile)) === 'application/octet-stream') throw new Error(`${markFile}: not png, jpeg or webp`);

  server = http.createServer();
  await new Promise((res, rej) => {
    server.once('error', (e) => rej(e.code === 'EADDRINUSE' ? new Error(`${cb.host} is taken: ${client}'s one callback is ${callback}`) : e));
    server.listen(Number(cb.port), cb.hostname, res);
  });

  const line = execFileSync('bash', [path.join(here, 'bundles.sh')], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
  src = src.replace(PLACEHOLDER, () => line.trim());

  const verifier = b64u(crypto.randomBytes(32)), state = b64u(crypto.randomBytes(16));
  const window = `${DOOR}/signin?` + new URLSearchParams({ client, redirect_uri: callback, code_challenge: b64u(sha256(verifier)), code_challenge_method: 'S256', state });
  const code = answer(cb, state);
  console.error('the window: ' + window);
  spawn(OPEN, [window], { stdio: 'ignore', detached: true }).on('error', () => {}).unref();
  const got = await code;
  server.close(); server.closeAllConnections(); server = null;

  key = crypto.generateKeyPairSync('ec', { namedCurve: 'P-256' });
  const tokenUrl = DOOR + '/v2/token';
  const r = await fetch(tokenUrl, { method: 'POST', signal: AbortSignal.timeout(CALL_WAIT),
    headers: { 'content-type': 'application/json', dpop: proof('POST', tokenUrl) },
    body: JSON.stringify({ code: got, code_verifier: verifier, client, redirect_uri: callback }) });
  if (!r.ok) throw new Error('/v2/token ' + r.status + ' ' + await r.text());
  const t = await r.json();
  token = t.access_token;
  if (t.token_type !== 'DPoP' || t.scope !== SITE) throw new Error(`the token is ${t.token_type} for ${t.scope}, not DPoP for ${SITE}`);
  console.log('✓ signed in');

  scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'site-setup-'));
  // The snippet is one async function called at once: its promise is returned, or nothing waits for it.
  if (!src.startsWith('(async () => {')) throw new Error('the snippet is not one async function');
  const run = new (async () => {}).constructor('fetch', 'document', 'FileReader', 'Image', 'console', 'return ' + src);
  await run(doorFetch, shim.document, shim.FileReader, shim.Image, shim.console);
}

// Signed out whatever happened: the session ends at the Door and its backup runs.
async function leave() {
  if (scratch) fs.rmSync(scratch, { recursive: true, force: true });
  if (server && server.listening) server.close();
  if (!token) return true;
  const t = token, url = DOOR + '/v2/signout';
  token = null;
  try {
    const r = await fetch(url, { method: 'POST', signal: AbortSignal.timeout(CALL_WAIT), headers: { authorization: 'DPoP ' + t, dpop: proof('POST', url, t) } });
    if (r.ok) { console.log('✓ signed out'); return true; }
    console.error('site-setup: the sign-out: ' + r.status + ' ' + await r.text());
  } catch (e) { console.error('site-setup: the sign-out: ' + e.message); }
  return false;
}

process.once('SIGINT', () => leave().finally(() => process.exit(130)));
let ok = true;
try { await main(process.argv.slice(2)); } catch (e) { ok = false; console.error('site-setup: ' + (e && e.message || e)); }
process.exit((await leave()) && ok ? 0 : 1);
