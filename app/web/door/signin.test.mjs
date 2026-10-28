/* signin.test.mjs — the sign-in window's finish, retried (D-34 (c)). A person's process
   that cannot open for now answers 503 and the Door keeps the attempt; the window sends
   the same proof again, and lands when it opens. Any other refusal is shown, not retried.

   The page, the passkey and the Door are stubs; signin.js runs as the window runs it.
   Run: node --test app/web/door/signin.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./signin.js', import.meta.url), 'utf8');
/* The window's own markup: each button's class, and #start's buttons in order. */
const HTML = readFileSync(new URL('./signin.html', import.meta.url), 'utf8');
const CLASS = Object.fromEntries([...HTML.matchAll(/<button id="(\w+)"(?: class="([^"]*)")?/g)].map(([, id, c]) => [id, c || '']));
const START = [.../<section id="start">([\s\S]*?)<\/section>/.exec(HTML)[1].matchAll(/id="(\w+)"/g)].map((m) => m[1]);
const WORDS = [.../<section id="words"[^>]*>([\s\S]*?)<\/section>/.exec(HTML)[1].matchAll(/<button id="(\w+)"/g)].map((m) => m[1]);
const DISABLED = Object.fromEntries([...HTML.matchAll(/<button id="(\w+)"[^>]*>/g)].map(([tag, id]) => [id, /\sdisabled[\s>]/.test(tag)]));

/* One window: `finishes` are the statuses /v2/signin/finish answers, in turn; `search`, its query. */
function window(finishes, search = '') {
  const els = {};
  const el = (id) => (els[id] ||= { id, hidden: false, disabled: DISABLED[id] || false, textContent: '', className: CLASS[id] || '', focus() {}, appendChild() {} });
  const order = START.slice();
  els.start = { id: 'start', order, insertBefore: (a, b) => { order.splice(order.indexOf(a.id), 1); order.splice(order.indexOf(b.id), 0, a.id); } };
  const sent = [];
  let landed = null;
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const fetch = async (path, init) => {
    sent.push({ path, body: init && init.body ? JSON.parse(init.body) : null });
    if (path.startsWith('/v2/work')) return reply(200, { challenge: null });
    if (path === '/v2/signin') return reply(200, { attempt: 'a1', key: 'k1' });
    if (path === '/v2/signin/finish') {
      const status = finishes.shift();
      return status === 200 ? reply(200, { redirect: 'https://www.wallflowers.io/' }) : reply(status, 'the Door is at its session limit; try again shortly');
    }
    throw new Error(`unexpected ${path}`);
  };
  const ctx = {
    document: {
      getElementById: el,
      querySelector: (q) => ({ content: q.includes('rp-id') ? 'wallflowers.io' : '' }),
      createElement: () => ({}),
    },
    location: { search, origin: 'https://app.wallflowers.io', replace: (u) => { landed = u; } },
    navigator: {
      credentials: {
        get: async () => ({
          response: { userHandle: new Uint8Array([1, 2, 3]) },
          getClientExtensionResults: () => ({ prf: { results: { first: new Uint8Array(32) } } }),
        }),
      },
    },
    fetch,
    setTimeout: (f) => setImmediate(f),
    URLSearchParams, TextEncoder, crypto: globalThis.crypto, Promise, JSON, Error, Uint8Array, String,
  };
  ctx.window = ctx;
  ctx.WallFlowersSeal = { b64u: () => 'AQID', seal: async () => ({ epk: 'e', iv: 'i', ct: 'c' }) };
  ctx.returnPath = () => '/';
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  return { els, sent, landed: () => landed };
}

const settle = () => new Promise((r) => setTimeout(r, 50));

test('a 503 at the finish is sent again, the same proof, and lands', async () => {
  const w = window([503, 503, 200]);
  w.els.in.onclick();
  for (let i = 0; i < 40 && !w.landed(); i++) await settle();
  assert.equal(w.landed(), 'https://www.wallflowers.io/');
  const finishes = w.sent.filter((s) => s.path === '/v2/signin/finish');
  assert.equal(finishes.length, 3);
  assert.ok(finishes.every((f) => JSON.stringify(f.body) === JSON.stringify(finishes[0].body)), 'the same proof each time');
  assert.equal(w.sent.filter((s) => s.path === '/v2/signin').length, 1, 'one attempt: no new assertion');
});

test('the retries stop, and the refusal is shown', async () => {
  const w = window([503, 503, 503, 503, 200]);
  w.els.in.onclick();
  for (let i = 0; i < 40 && !w.els.status.textContent; i++) await settle();
  assert.equal(w.landed(), null);
  assert.equal(w.sent.filter((s) => s.path === '/v2/signin/finish').length, 4, 'the first and three more');
  assert.match(w.els.status.textContent, /session limit/);
});

test('any other refusal is shown at once, not sent again', async () => {
  const w = window([401]);
  w.els.in.onclick();
  for (let i = 0; i < 40 && !w.els.status.textContent; i++) await settle();
  assert.equal(w.sent.filter((s) => s.path === '/v2/signin/finish').length, 1);
  assert.equal(w.landed(), null);
});

/* ── NC-83: no PRF, no account ─────────────────────────────────────────────────
   A sign-up window: `atCreate` is what create() reports for prf (undefined: nothing),
   `atGet` whether get() gives a PRF, `signal` the platform's signalUnknownCredential
   (undefined: not supported). */
function signupWindow({ atCreate, atGet = true, signal, ua = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', touch = 0, saved,
                       search = '?new', refuseFirst = false, getRefuse = 0, face = false, finishRefused,
                       known = false, conditional = false, aaguid, at = true, keep = {} } = {}) {
  const els = {};
  const el = (id) => (els[id] ||= { id, hidden: false, disabled: false, textContent: '', value: '', items: [], focus() {}, appendChild(c) { this.items.push(c); }, insertBefore() {} });
  for (const id of ['words', 'shown', 'name']) el(id).hidden = true;   // as the markup has them
  // The window's root, as the Door serves it: a Site's with its Face wears data-face.
  const attrs = new Set(face ? ['data-site', 'data-face'] : []);
  const html = { hasAttribute: (k) => attrs.has(k), removeAttribute: (k) => attrs.delete(k) };
  const sent = [], created = [], signalled = [], asked = [];
  const stored = new Map([...(known ? [['wallflowers.known', '1']] : []), ...Object.entries(keep)]);
  const pick = { resolve: null, aborted: false };
  const refused = (what) => { const e = new Error(what); e.name = 'NotAllowedError'; return e; };
  let replaced = null;
  let gets = 0, landed = null;
  const reply = (status, body) => ({ ok: status < 300, status, text: async () => (typeof body === 'string' ? body : JSON.stringify(body)) });
  const fetch = async (path, init) => {
    sent.push({ path, body: init && init.body ? JSON.parse(init.body) : null });
    if (path.startsWith('/v2/work')) return reply(200, { challenge: null });
    if (path === '/v2/signup') return reply(200, { attempt: 'a1', key: 'k1', handle: 'AQID', pk: 'ed25519:' + 'c'.repeat(64), host: 'app.wallflowers.io' });
    if (path === '/v2/signup/finish' && finishRefused) return reply(...finishRefused);
    if (path === '/v2/signup/finish') return reply(200, saved === false ? { pk: 'c'.repeat(64), words: 'one two three', saved: false } : { pk: 'c'.repeat(64), words: 'one two three', ...(search.includes('client=') ? { continue: 'h1' } : {}) });
    if (path === '/v2/signup/continue') return reply(200, { redirect: 'https://egregores-echoes.com/signin/callback?code=c1&state=s1' });
    if (path === '/v2/signin') return reply(200, { attempt: 'a2', key: 'k2' });
    if (path === '/v2/signin/finish') return reply(200, { redirect: 'https://app.wallflowers.io/' });
    throw new Error(`unexpected ${path}`);
  };
  const ext = atCreate === undefined ? {} : { prf: atCreate };
  const ctx = {
    document: { documentElement: html, getElementById: el, querySelector: (q) => ({ content: q.includes('rp-id') ? 'app.wallflowers.io' : '' }), createElement: () => ({}) },
    location: { search, origin: 'https://app.wallflowers.io', replace(u) { landed = u; replaced = u; } },
    navigator: {
      userAgent: ua, maxTouchPoints: touch,
      credentials: {
        create: async (o) => {
          created.push(o.publicKey);
          if (refuseFirst && created.length === 1) throw refused('not without a tap of its own');
          // the passkey's provider, as its authenticator data names it (AAGUID); none, as a platform without getAuthenticatorData
          const response = aaguid === undefined ? {} : { getAuthenticatorData: () => authData(aaguid, at) };
          return { rawId: new Uint8Array([9, 9, 9]).buffer, response, getClientExtensionResults: () => ext };
        },
        get: async (o) => {
          asked.push({ ...o.publicKey, mediation: o.mediation });
          // the name field's offer (conditional mediation): no prompt; it waits for a pick, or its abort
          if (o.mediation === 'conditional') {
            return new Promise((ok, no) => {
              pick.resolve = () => ok({ response: { userHandle: new Uint8Array([1, 2, 3]) }, getClientExtensionResults: () => ({ prf: { results: { first: new Uint8Array(32) } } }) });
              o.signal?.addEventListener('abort', () => { pick.aborted = true; const e = new Error('aborted'); e.name = 'AbortError'; no(e); });
            });
          }
          gets++;
          if (gets <= getRefuse) throw refused('dismissed, or no passkey here');
          return { rawId: new Uint8Array([9, 9, 9]).buffer, response: { userHandle: new Uint8Array([1, 2, 3]) },
            getClientExtensionResults: () => (atGet ? { prf: { results: { first: new Uint8Array(32) } } } : {}) };
        },
      },
    },
    PublicKeyCredential: {
      ...(signal === undefined ? {} : { signalUnknownCredential: (o) => { signalled.push(o); return signal(o); } }),
      ...(conditional ? { isConditionalMediationAvailable: async () => true } : {}),
    },
    localStorage: { getItem: (k) => stored.get(k) ?? null, setItem: (k, v) => stored.set(k, String(v)), removeItem: (k) => stored.delete(k) },
    AbortController,
    fetch, setTimeout: (f) => setImmediate(f),
    URLSearchParams, TextEncoder, crypto: globalThis.crypto, Promise, JSON, Error, Uint8Array, String,
  };
  ctx.window = ctx;
  ctx.WallFlowersSeal = { b64u: () => 'CQkJ', unb64u: () => new Uint8Array([1, 2, 3]), seal: async () => ({ epk: 'e', iv: 'i', ct: 'c' }) };
  ctx.returnPath = () => '/';
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  return { el, html, sent, created, signalled, asked, stored, pick, gets: () => gets, landed: () => landed, replaced: () => replaced };
}
/* Authenticator data: rpIdHash (32), flags (1; AT is 0x40), signCount (4), then the AAGUID (16). */
function authData(aaguid, at) {
  const d = new Uint8Array(37 + 16 + 2);
  d[32] = at ? 0x45 : 0x05;
  const hex = aaguid.replace(/-/g, '');
  for (let i = 0; i < 16; i++) d[37 + i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  return d.buffer;
}
/* One tap: Create an account asks the passkey at once (Ralph, 29 Sep). */
async function signUp(w) {
  w.el('new').onclick();
  for (let i = 0; i < 60 && !w.el('status').textContent && w.el('shown').hidden !== false && w.el('words').hidden !== false; i++) await settle();
}
const finished = (w) => w.sent.filter((s) => s.path === '/v2/signup/finish').length;
const ok = async () => undefined;
const plain = (v) => JSON.parse(JSON.stringify(v));   // out of the page's realm

for (const [what, atCreate] of [['reports prf disabled', { enabled: false }], ['reports nothing', undefined], ['reports prf without enabled', {}]]) {
  test(`a passkey whose create() ${what} makes no account, and the platform is told`, async () => {
    const w = signupWindow({ atCreate, signal: ok });
    await signUp(w);
    assert.equal(w.el('status').textContent, 'No account was made: this passkey has no PRF.');
    assert.equal(finished(w), 0, 'no finish: no account');
    assert.equal(w.gets(), 0, 'no second prompt');
    assert.deepEqual(plain(w.signalled), [{ rpId: 'app.wallflowers.io', credentialId: 'CQkJ' }]);
    assert.notEqual(w.el('shown').hidden, false, 'no words shown');
  });
}

test('prf enabled at create() but none at get(): no account, and the platform is told', async () => {
  const w = signupWindow({ atCreate: { enabled: true }, atGet: false, signal: ok });
  await signUp(w);
  assert.equal(w.el('status').textContent, 'No account was made: this passkey has no PRF.');
  assert.equal(finished(w), 0);
  assert.equal(w.signalled.length, 1);
});

test('where the platform cannot be told, or its telling fails, the refusal stands', async () => {
  for (const signal of [undefined, async () => { throw new Error('NotSupportedError'); }, () => { throw new Error('sync'); }]) {
    const w = signupWindow({ atCreate: { enabled: false }, signal });
    await signUp(w);
    assert.equal(w.el('status').textContent, 'No account was made: this passkey has no PRF.');
    assert.equal(finished(w), 0);
  }
});

test('prf enabled and a PRF at get(): the account is made, the words shown, nothing signalled', async () => {
  const w = signupWindow({ atCreate: { enabled: true }, signal: ok });
  await signUp(w);
  assert.equal(finished(w), 1);
  assert.equal(w.el('status').textContent, '');
  assert.equal(w.el('shown').hidden, false);
  assert.equal(w.signalled.length, 0);
  const first = w.created[0].extensions.prf.eval.first;
  assert.equal(new TextDecoder().decode(first), 'pacific/wrap/v1', "the PRF evaluated at create(), on the wrap's input");
  assert.deepEqual(plain(w.created[0].extensions), { prf: { eval: { first: plain(first) } } }, 'prf is asked for at create()');
  assert.equal(w.gets(), 1, 'no PRF given at create(): one get() reads it');
});

/* ── NC-84: on an iPhone or iPad, the device's own passkey ─────────────────────── */
const UAS = {
  iphone: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.7 Mobile/15E148 Safari/604.1',
  chromeOnIphone: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/140.0.0.0 Mobile/15E148 Safari/604.1',
  ipadAsMac: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.7 Safari/605.1.15',
  windows: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36',
  android: 'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Mobile Safari/537.36',
};
for (const [who, ua, touch, platform] of [
  ['an iPhone', UAS.iphone, 5, true],
  ['Chrome on an iPhone (WebKit too)', UAS.chromeOnIphone, 5, true],
  ['an iPad, which asks as a Mac with a touch screen', UAS.ipadAsMac, 5, true],
  ['a Mac', UAS.ipadAsMac, 0, false],
  ['Windows', UAS.windows, 0, false],
  ['Android', UAS.android, 5, false],
]) {
  test(`create() on ${who}: ${platform ? "the device's own passkey only" : 'any authenticator, a phone over a QR code included'}`, async () => {
    const w = signupWindow({ atCreate: { enabled: true }, ua, touch });
    await signUp(w);
    const sel = plain(w.created[0].authenticatorSelection);
    assert.equal(sel.authenticatorAttachment, platform ? 'platform' : undefined);
    assert.equal(sel.residentKey, 'required');
    assert.equal(sel.userVerification, 'required');
    assert.equal(finished(w), 1, 'and the account is made');
  });
}

/* ── NC-132: nothing made ─────────────────────────────────────────────────────────
   A Door with no room to keep the account refuses the finish before one exists (503): the
   refusal is said in the status line, no words are shown, the buttons come back, and a
   sign-up can be begun again. */
test('a sign-up this Door cannot keep is refused, said, and can be begun again', async () => {
  const why = 'No account was made: this Door cannot keep it (its seal could not be written).';
  const w = signupWindow({ atCreate: { enabled: true }, finishRefused: [503, why] });
  await signUp(w);
  assert.equal(w.el('status').textContent, why, 'the refusal, in its own words');
  assert.equal(w.el('shown').hidden, true, 'no words for an account that does not exist');
  for (const id of ['in', 'new']) assert.equal(w.el(id).disabled, false, `${id} is back`);
  const begun = () => w.sent.filter((s) => s.path === '/v2/signup').length;
  const before = begun();
  w.el('status').textContent = '';
  await signUp(w);
  assert.equal(begun(), before + 1, 'a sign-up begun again');
  assert.equal(finished(w), 2, 'and finished again, never retried on its own');
});

/* ── NC-89: made, not saved ───────────────────────────────────────────────────────
   The Door made the account and could not keep it: it answers the words with
   `saved: false`. They are shown, with one next step, a sign-in with this passkey. */
test('made and not saved: the words are shown, and the one next step signs in with the same passkey', async () => {
  const w = signupWindow({ atCreate: { enabled: true }, saved: false });
  await signUp(w);
  assert.equal(w.el('shown').hidden, false, 'the words are shown');
  assert.deepEqual(w.el('list').items.map((li) => li.textContent), ['one', 'two', 'three']);
  assert.equal(w.el('status').textContent, 'Your account is made and not yet saved.');
  assert.equal(w.el('done').textContent, 'Sign in with this passkey');
  assert.equal(w.landed(), null, 'nothing lands, and nothing counts down, while the words are copied');
  const getsBefore = w.gets();
  w.el('done').onclick();
  for (let i = 0; i < 40 && !w.landed(); i++) await settle();
  assert.equal(w.created.length, 1, 'never a second create()');
  assert.equal(w.gets(), getsBefore + 1, 'one sign-in');
  const allow = w.asked.at(-1).allowCredentials;
  assert.equal(allow.length, 1, 'the passkey just made, and no other');
  assert.deepEqual([...new Uint8Array(allow[0].id)], [9, 9, 9]);
  assert.equal(w.sent.filter((s) => s.path === '/v2/signin/finish').length, 1);
  assert.equal(w.landed(), 'https://app.wallflowers.io/', 'and it lands');
});

test('saved, as ever: the words and Continue, no sign-in', async () => {
  const w = signupWindow({ atCreate: { enabled: true } });
  await signUp(w);
  assert.equal(w.el('status').textContent, '');
  assert.notEqual(w.el('done').textContent, 'Sign in with this passkey');
  assert.equal(w.sent.filter((s) => s.path === '/v2/signin').length, 0);
});

/* ── One step (Ralph, 29 Sep: "consolidated into one step, immediately taking you to the passkey") ── */
test('one tap: Create an account asks the passkey at once, and the step before it is never shown', async () => {
  const w = signupWindow({ atCreate: { enabled: true } });
  await signUp(w);
  assert.equal(w.created.length, 1, 'the passkey was asked');
  assert.equal(w.el('words').hidden, true, 'no "Create the passkey" step');
  assert.equal(w.el('shown').hidden, false, 'the words');
  assert.equal(finished(w), 1);
});

test('a platform that will not ask without a tap of its own: the passkey\'s button, and it asks', async () => {
  const w = signupWindow({ atCreate: { enabled: true }, refuseFirst: true });
  await signUp(w);
  assert.equal(w.el('words').hidden, false, 'the passkey\'s own button');
  assert.equal(w.el('status').textContent, '', 'nothing said: nothing failed');
  assert.equal(finished(w), 0);
  w.el('saved').onclick();
  for (let i = 0; i < 60 && w.el('shown').hidden !== false; i++) await settle();
  assert.equal(w.created.length, 2);
  assert.equal(w.el('shown').hidden, false, 'the words');
  assert.equal(finished(w), 1, 'one account');
});

const SITE = '?' + new URLSearchParams({ client: 'egregores-echoes.com', redirect_uri: 'https://egregores-echoes.com/signin/callback',
  code_challenge: 'x', code_challenge_method: 'S256', state: 's1' });
const until = async (f) => { for (let i = 0; i < 60 && !f(); i++) await settle(); };

/* ONE BUTTON, ONE PROMPT (Ralph, 30 Sep ~13:00Z, after walking a real visitor through it: "At the
   moment, the user is forced through 3 face-id steps. this should be 1. Creating the passkey
   should sign them in. We dont need separate controls for create passkey/sign in with passkey. It
   should just be 'SIGN IN', which creates a passkey *if* one doesnt exist."). A modal prompt is
   a get() without mediation, or a create(); the name field's offer (conditional) is not one. */
const prompts = (w) => w.gets() + w.created.length;
const named = (w, name) => { w.el('name').value = name; };
const PRF_AT_CREATE = { enabled: true, results: { first: new Uint8Array(32) } };
const ICLOUD = 'fbfc3007-154e-4ecc-8c0b-6e020557d7bd', GPM = 'ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4';
const MAC_SAFARI = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Safari/605.1.15';
const MAC_CHROME = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36';

test("a Site's window on a device that has signed in here: the passkey at once, one prompt, and back", async () => {
  const w = signupWindow({ search: SITE, known: true });
  await until(() => w.landed());
  assert.equal(prompts(w), 1, 'one prompt');
  assert.equal(w.landed(), 'https://app.wallflowers.io/');
  assert.equal(w.el('name').hidden, true, 'no name to give');
});

test("a Site's window shows one button, SIGN IN, beside the name; Create an account is gone", async () => {
  const w = signupWindow({ search: SITE, atCreate: PRF_AT_CREATE });
  await settle();
  assert.equal(w.el('in').textContent, 'SIGN IN');
  assert.equal(w.el('in').hidden, false);
  assert.equal(w.el('new').hidden, true, 'no second control');
  assert.equal(w.el('name').hidden, false);
  assert.equal(prompts(w), 0, 'nothing asked before the tap on a device new here');
  assert.equal(w.el('deny').hidden, false, 'and Cancel, quiet');
});

test("a Site's new member: a name, SIGN IN, one prompt (create, with its PRF), and back signed in", async () => {
  const w = signupWindow({ search: SITE, atCreate: PRF_AT_CREATE, face: true, ua: MAC_SAFARI, aaguid: ICLOUD });
  await settle();
  named(w, 'Hana');
  w.el('in').onclick();
  await until(() => w.landed());
  assert.equal(prompts(w), 1, 'one prompt: the create');
  assert.equal(w.created.length, 1);
  assert.deepEqual(plain(w.created[0].extensions), { prf: { eval: { first: plain(w.created[0].extensions.prf.eval.first) } } }, 'PRF asked at create');
  assert.equal(w.landed(), 'https://egregores-echoes.com/signin/callback?code=c1&state=s1');
  assert.equal(w.stored.get('wallflowers.known'), '1', 'the device is known here from now on');
});

test("where the platform gives no PRF at create, one get() reads it: two prompts, never more", async () => {
  const w = signupWindow({ search: SITE, atCreate: { enabled: true }, face: true });
  await settle();
  named(w, 'Hana');
  w.el('in').onclick();
  await until(() => w.landed());
  assert.equal(prompts(w), 2);
});

/* PRF AT create() ONLY WHERE IT IS WITNESSED (ASSURANCE's (ii), through SCM, 30 Sep): create()'s PRF
   result is used only for a provider TEST witnesses, in the browser it witnesses it in: iCloud
   Keychain on an iPhone or iPad and in Mac Safari, Google Password Manager in desktop Chrome.
   Everywhere else one get() reads the PRF, as before, and create()'s result is zeroed unused. The
   provider is the passkey's AAGUID, in its authenticator data. */
const UNKNOWN = 'bada5566-a7aa-401f-bd96-45619a55120d', NONE = '00000000-0000-0000-0000-000000000000';
const WINDOWS_CHROME = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36';
const EDGE = WINDOWS_CHROME + ' Edg/151.0.0.0';
const ANDROID_CHROME = 'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Mobile Safari/537.36';
const MAC_FIREFOX = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:143.0) Gecko/20100101 Firefox/143.0';
const IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.7 Mobile/15E148 Safari/604.1';
const CHROME_ON_IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/140.0.0.0 Mobile/15E148 Safari/604.1';
async function newMember(o) {
  const first = new Uint8Array(32).fill(7);
  const w = signupWindow({ search: SITE, face: true, atCreate: { enabled: true, results: { first } }, ...o });
  await settle();
  named(w, 'Hana');
  w.el('in').onclick();
  await until(() => w.landed());
  return { w, first };
}
for (const [what, o] of [
  ['iCloud Keychain on an iPhone', { ua: IPHONE, touch: 5, aaguid: ICLOUD }],
  ['iCloud Keychain in Chrome on an iPhone (WebKit, as every iOS browser)', { ua: CHROME_ON_IPHONE, touch: 5, aaguid: ICLOUD }],
  ['iCloud Keychain in Mac Safari', { ua: MAC_SAFARI, aaguid: ICLOUD }],
  ['Google Password Manager in Chrome on a Mac', { ua: MAC_CHROME, aaguid: GPM }],
  ['Google Password Manager in Chrome on Windows', { ua: WINDOWS_CHROME, aaguid: GPM }],
]) {
  test(`witnessed, ${what}: create()'s PRF is used, one prompt`, async () => {
    const { w } = await newMember(o);
    assert.equal(w.created.length, 1);
    assert.equal(w.gets(), 0, 'no get()');
  });
}
for (const [what, o] of [
  ['iCloud Keychain in Chrome on a Mac', { ua: MAC_CHROME, aaguid: ICLOUD }],
  ['Google Password Manager in Chrome on Android', { ua: ANDROID_CHROME, touch: 5, aaguid: GPM }],
  ['Google Password Manager named in Edge', { ua: EDGE, aaguid: GPM }],
  ['Google Password Manager named in Firefox', { ua: MAC_FIREFOX, aaguid: GPM }],
  ['another provider in Mac Safari', { ua: MAC_SAFARI, aaguid: UNKNOWN }],
  ['a provider that names none (zero AAGUID) on an iPhone', { ua: IPHONE, touch: 5, aaguid: NONE }],
  ['no authenticator data to read', { ua: MAC_SAFARI }],
  ['authenticator data without the credential (no AT flag)', { ua: MAC_SAFARI, aaguid: ICLOUD, at: false }],
]) {
  test(`not witnessed, ${what}: one get() reads the PRF, and create()'s is zeroed unused`, async () => {
    const { w, first } = await newMember(o);
    assert.equal(w.created.length, 1);
    assert.equal(w.gets(), 1, 'one get()');
    assert.deepEqual([...new Set(first)], [0], "create()'s PRF result zeroed");
  });
}

/* A CHECK ON create()'s PRF (ASSURANCE, through SCM, 30 Sep): where the account was made with the
   PRF create() gave, a check value of it is kept on this device under the passkey's id, and the
   first get() here compares its own, so a witnessed provider that later changes its PRF fails by
   name, before anything is sent. The check is a hash of the PRF under its own domain, never the
   PRF. The Door's wrap (an AEAD under the PRF) still refuses a wrong PRF on any other device. */
const checkOf = (prf) => createHash('sha256').update(Buffer.concat([Buffer.from('pacific/prf-check/v1\0'), Buffer.from(prf)])).digest('hex').slice(0, 32);
const CHECK_KEY = 'wallflowers.prf-check.090909';   // the passkey's id, [9, 9, 9], in hex
const checks = (w) => [...w.stored.keys()].filter((k) => k.startsWith('wallflowers.prf-check.'));

test("witnessed: a check value of create()'s PRF is kept here under the passkey's id, never the PRF", async () => {
  const { w } = await newMember({ ua: MAC_SAFARI, aaguid: ICLOUD });
  assert.deepEqual(checks(w), [CHECK_KEY]);
  assert.equal(w.stored.get(CHECK_KEY), checkOf(new Uint8Array(32).fill(7)));
  assert.ok(![...w.stored.values()].some((v) => v.includes('0707070707')), 'not the PRF');
});

test('not witnessed: no check value, the get() having read the PRF the account is made with', async () => {
  const { w } = await newMember({ ua: MAC_CHROME, aaguid: ICLOUD });
  assert.deepEqual(checks(w), []);
});

test("the first get() here that agrees signs in, and the check is dropped", async () => {
  const w = signupWindow({ search: SITE, known: true, keep: { [CHECK_KEY]: checkOf(new Uint8Array(32)) } });   // the harness's get() PRF: zeros
  await until(() => w.landed());
  assert.equal(w.landed(), 'https://app.wallflowers.io/');
  assert.deepEqual(checks(w), []);
});

test("a first get() whose PRF is not create()'s fails by name: nothing sent, nothing opened, the check kept", async () => {
  const w = signupWindow({ search: SITE, known: true, keep: { [CHECK_KEY]: checkOf(new Uint8Array(32).fill(7)) } });
  await until(() => w.el('status').textContent);
  assert.equal(w.el('status').textContent, 'This passkey no longer gives the key it was made with, so nothing was opened.');
  assert.equal(w.landed(), null);
  assert.equal(w.sent.filter((s) => s.path === '/v2/signin/finish').length, 0, 'nothing sent');
  assert.deepEqual(checks(w), [CHECK_KEY]);
});

/* TEST, run 110 (1 Oct): each retry after that sentence opened a /v2/signin attempt before the
   check refused it, so a person who tried a few times met the Door's 429 ("too many sign-ins
   are open from here") in place of the sentence. The passkey is read and checked first, and an
   attempt is opened only for a PRF that will be sent. */
test("a PRF the check refuses opens no sign-in attempt, however often it is tried", async () => {
  const w = signupWindow({ search: SITE, known: true, keep: { [CHECK_KEY]: checkOf(new Uint8Array(32).fill(7)) } });
  await until(() => w.el('status').textContent);
  for (let i = 0; i < 3; i++) {
    w.el('status').textContent = '';
    w.el('in').onclick();
    await until(() => w.el('status').textContent);
  }
  assert.equal(w.el('status').textContent, 'This passkey no longer gives the key it was made with, so nothing was opened.');
  assert.equal(w.sent.filter((s) => s.path === '/v2/signin').length, 0, 'no attempt opened');
});

test("a sign-in reads the passkey first, then opens its attempt, then seals to it", async () => {
  const w = signupWindow({ search: SITE, known: true });
  await until(() => w.landed());
  const order = w.sent.map((s) => s.path).filter((p) => p === '/v2/signin' || p === '/v2/signin/finish');
  assert.deepEqual(order, ['/v2/signin', '/v2/signin/finish']);
  assert.equal(w.gets(), 1);
});

test("a passkey synced here from elsewhere is offered in the name field, signs in from it, and makes no second account", async () => {
  const w = signupWindow({ search: SITE, atCreate: PRF_AT_CREATE, conditional: true });
  await until(() => w.pick.resolve);
  assert.equal(w.asked[0].mediation, 'conditional', 'the offer rides the name field');
  assert.match(HTML, /<input id="name"[^>]*autocomplete="username webauthn"/, 'a field the platform offers passkeys in');
  w.pick.resolve();
  await until(() => w.landed());
  assert.equal(w.created.length, 0, 'no create');
  assert.equal(w.gets(), 0, 'no modal prompt beyond the pick');
  assert.equal(w.sent.filter((s) => s.path === '/v2/signin/finish').length, 1);
});

test("SIGN IN on a new name withdraws the name field's offer before it creates", async () => {
  const w = signupWindow({ search: SITE, atCreate: PRF_AT_CREATE, conditional: true, ua: MAC_SAFARI, aaguid: ICLOUD });
  await until(() => w.pick.resolve);
  named(w, 'Hana');
  w.el('in').onclick();
  await until(() => w.landed());
  assert.equal(w.pick.aborted, true, 'the offer withdrawn');
  assert.equal(prompts(w), 1);
});

test("a known device whose passkey is gone: dismissed, then the name and SIGN IN make one", async () => {
  const w = signupWindow({ search: SITE, known: true, getRefuse: 1, atCreate: PRF_AT_CREATE });
  await until(() => w.gets() >= 1 && !w.el('in').disabled);
  assert.equal(w.landed(), null);
  assert.equal(w.el('status').textContent, '', 'a dismissed prompt is not an error');
  assert.equal(w.el('name').hidden, false, 'the name, for a new passkey');
  named(w, 'Hana');
  w.el('in').onclick();
  await until(() => w.landed());
  assert.equal(w.created.length, 1);
});

test("a Site's Cancel goes to its registered callback with access_denied and the state", async () => {
  const w = signupWindow({ search: SITE });
  await settle();
  w.el('deny').onclick();
  const u = new URL(w.replaced());
  assert.equal(u.origin + u.pathname, 'https://egregores-echoes.com/signin/callback');
  assert.equal(u.searchParams.get('error'), 'access_denied');
  assert.equal(u.searchParams.get('state'), 's1');
  assert.equal(u.searchParams.has('code'), false);
});

/* ONE PAGE FOR A SITE'S NEW MEMBER (30 Sep; Antoine: "I would like this small interface of
   login to still be on Egregore's"; the user: "exactly one app.wallflowers.io page, which
   serves only the passkey. Egregore branded screen, and immediately back"). In a Site's
   window a new member gives a name and makes a passkey, in the Site's Face, and goes straight
   back to the Site: no recovery words on this path (the user's ruling: "Drop them entirely"),
   and the name on the one page (O-77's card on entry, the user's ruling). */
test("a Site's window asks a new member's name beside the passkey; WallFlowers' own window does not", async () => {
  assert.match(HTML, /<input id="name"[^>]*\shidden/, 'in the markup, hidden until a Site asks');
  const site = signupWindow({ search: SITE, atCreate: { enabled: true }, face: true });
  await settle();
  assert.equal(site.el('name').hidden, false, "shown in a Site's window");
  const own = signupWindow({ atCreate: { enabled: true } });
  await settle();
  assert.equal(own.el('name').hidden, true, "not in WallFlowers' own window");
});

test("a Site's new account: the name goes with it, no words, the Face stays, and straight back to the Site", async () => {
  const w = signupWindow({ search: SITE, atCreate: { enabled: true }, face: true });
  await settle();
  named(w, '  Hana  ');
  w.el('in').onclick();
  await until(() => w.landed());
  const start = w.sent.find((s) => s.path === '/v2/signup');
  assert.equal(start.body.name, 'Hana', 'the account made under the name, trimmed');
  assert.deepEqual(plain(w.sent.find((s) => s.path === '/v2/signup/continue').body), { continue: 'h1' }, 'continued at once');
  assert.equal(w.landed(), 'https://egregores-echoes.com/signin/callback?code=c1&state=s1');
  assert.notEqual(w.el('shown').hidden, false, 'no recovery words');
  assert.equal(w.el('list').items.length, 0, 'not one word drawn');
  assert.equal(w.html.hasAttribute('data-face'), true, "the Site's Face throughout");
});

test("a Site's new account needs a name: none, and no passkey is made", async () => {
  const w = signupWindow({ search: SITE, atCreate: { enabled: true }, face: true });
  await settle();
  named(w, '   ');
  w.el('in').onclick();
  await settle();
  assert.equal(w.created.length, 0, 'no create()');
  assert.equal(finished(w), 0);
  assert.equal(w.el('status').textContent, 'Your name');
});

test("a Site's account made and not saved: no words, the same passkey signs in, and back to the Site", async () => {
  const w = signupWindow({ search: SITE, atCreate: { enabled: true }, face: true, saved: false });
  await settle();
  named(w, 'Hana');
  w.el('in').onclick();
  await until(() => w.landed());
  assert.notEqual(w.el('shown').hidden, false, 'no recovery words');
  assert.equal(w.sent.filter((s) => s.path === '/v2/signin/finish').length, 1, 'signed in with the passkey just made');
  assert.equal(w.created.length, 1, 'never a second create()');
});

/* Every element the script reaches is in the page it runs in: a stub makes any id on
   demand, so only this can catch one the markup has lost. */
test('every id the window script uses is in the window', () => {
  const html = readFileSync(new URL('./signin.html', import.meta.url), 'utf8');
  const ids = new Set([...html.matchAll(/\bid="([^"]+)"/g)].map((m) => m[1]));
  const used = new Set([...SRC.matchAll(/\$\('([a-z]+)'\)/g)].map((m) => m[1]));
  for (const m of SRC.matchAll(/\[((?:'[a-z]+',?\s*)+)\]\.forEach\(function \(id\)/g)) {
    for (const q of m[1].matchAll(/'([a-z]+)'/g)) used.add(q[1]);
  }
  assert.ok(used.size > 5, 'the script names its elements');
  assert.deepEqual([...used].filter((id) => !ids.has(id)), []);
});

/* A Face's buttons in its window as on its own page (face.css .wf-btn: --btn-bg, --btn-fg),
   each label readable on its button. Egregore's look, as production's /brand served it,
   sets --accent and --on-accent both white: the passkey button was a blank white pill
   (EGREGORE, 29 Sep). Its button over the look's --bg, text 3:1 at least. */
test("a Face's buttons are readable in its window", () => {
  const css = readFileSync(new URL('./signin.css', import.meta.url), 'utf8');
  const decls = (sel) => {
    const at = css.indexOf(sel + '{');
    assert.ok(at >= 0, sel);
    const body = css.slice(at + sel.length + 1, css.indexOf('}', at));
    return Object.fromEntries(body.split(';').filter(Boolean).map((d) => [d.slice(0, d.indexOf(':')), d.slice(d.indexOf(':') + 1)]));
  };
  const resolve = (v, vars) => {
    const m = /^var\((--[a-z-]+)(?:,(.*))?\)$/.exec(v.trim());
    return !m ? v.trim() : m[1] in vars ? vars[m[1]] : resolve(m[2] ?? '', vars);
  };
  const rgba = (c) => {
    const h = /^#([0-9a-f]{6})$/i.exec(c);
    if (h) return [0, 2, 4].map((i) => parseInt(h[1].slice(i, i + 2), 16)).concat(1);
    const f = /^rgba?\(([^)]*)\)$/.exec(c)[1].split(',').map(Number);
    return f.length === 3 ? f.concat(1) : f;
  };
  const over = (top, under) => top.slice(0, 3).map((x, i) => x * top[3] + under[i] * (1 - top[3]));
  const lum = (c) => c.map((x) => x / 255).map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4)).reduce((a, x, i) => a + x * [0.2126, 0.7152, 0.0722][i], 0);
  const looks = {
    egregore: '--btn-bg:rgba(255,255,255,0.26);--btn-fg:#FFFFFF;--card-bg:rgba(24,20,20,0.18);--bg:#243819;--fg:#FFFFFF;--surface:#181414;--accent:#FFFFFF;--on-accent:#FFFFFF',
    paper: '--btn-bg:#546CAC;--btn-fg:#FFFFFF;--card-bg:#FFFFFF;--bg:#FAF8F3;--fg:#14181E;--surface:#FFFFFF;--accent:#546CAC;--on-accent:#FFFFFF',
  };
  for (const [name, look] of Object.entries(looks)) {
    const vars = Object.fromEntries(look.split(';').map((d) => d.split(':')));
    for (const which of ['button', 'button.primary']) {
      const d = { ...decls('html[data-face] button'), ...(which === 'button' ? {} : decls('html[data-face] button.primary')) };
      const page = rgba(vars['--bg']);
      const bg = over(rgba(resolve(d.background, vars)), page), fg = over(rgba(resolve(d.color, vars)), bg);
      const [hi, lo] = [lum(bg), lum(fg)].sort((a, b) => b - a);
      assert.ok((hi + 0.05) / (lo + 0.05) >= 3, `${name}'s ${which}: ${d.background} behind ${d.color}`);
    }
  }
});

/* ?new is a first-timer's window (every QR visitor's, every wallflowers.io sign-up): Create an account
   is its one main button, and first; Sign in stays, quiet and second, for someone who has an account
   (MANAGE, 29 Sep: the bright Sign in sent first-timers to iOS's "no passkeys saved"). */
test('with ?new, Create an account is the main button and first; Sign in is quiet, second', () => {
  const w = window([], '?new');
  assert.equal(w.els.new.className, 'primary');
  assert.equal(w.els.in.className, 'quiet');
  assert.deepEqual(w.els.start.order.filter((id) => id === 'new' || id === 'in'), ['new', 'in']);
});

test('without ?new, the window keeps Sign in as its main button', () => {
  const w = window([]);
  assert.equal(w.els.in.className, 'primary');
  assert.notEqual(w.els.new.className, 'primary');
  assert.deepEqual(w.els.start.order.filter((id) => id === 'new' || id === 'in'), ['in', 'new']);
});

/* A tap before the window's script has bound its buttons did nothing, and said nothing: at a venue's
   130 ms, signin.js arrives a round trip after the page (UX-B, 29 Sep). So they are drawn disabled,
   and the script enables each one once it has bound it (UX, 29 Sep). */
test("the window's first buttons start disabled in its HTML", () => {
  // the buttons: #start also holds a Site's name field, an input a tap cannot reach too early
  for (const id of [...START.filter((id) => id in CLASS), ...WORDS]) assert.equal(DISABLED[id], true, `#${id} starts disabled`);
});

test('the script enables every button it binds, and only once bound', () => {
  for (const search of ['', '?new']) {
    const w = window([], search);
    for (const id of ['in', 'new', 'saved']) {
      assert.equal(typeof w.els[id].onclick, 'function', `#${id} bound (${search || 'plain'})`);
      assert.equal(w.els[id].disabled, false, `#${id} enabled (${search || 'plain'})`);
    }
    // Untouched by the script, #deny is as the HTML draws it.
    assert.equal(w.els.deny ? w.els.deny.disabled : DISABLED.deny, true, `#deny, bound only for a Site, stays disabled (${search || 'plain'})`);
  }
});

/* The recovery words stand alone: neither way in stays live above them, or a tap there
   starts a second account (UX-C, 29 Sep: #start shown and #new enabled on the words). */
// WallFlowers' own window: a Site's path shows no words at all (the user, 30 Sep; tests above)
test('the recovery words stand alone: no Sign in, no second Create an account above them', async () => {
  for (const [search, saved] of [['?new', undefined], ['?new', false]]) {
    const w = signupWindow({ search, saved, atCreate: { enabled: true } });
    await signUp(w);
    assert.equal(w.el('shown').hidden, false, 'the words');
    assert.equal(w.el('start').hidden, true, `${search} ${saved}: #start hidden with the words`);
  }
});
