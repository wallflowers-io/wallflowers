#!/usr/bin/env node
// THE BROWSER AS AN MLS MEMBER — keyholder.js's group methods, in two real
// browser origins, over a real relay.
//
// WHAT THIS IS THE BROWSER HALF OF. `smoke/relay-pair.mjs` proves the sequence
// under Node: create → key package → add → join → encrypt → relay → decrypt,
// with the delta byte-identical at the far end. It loads the same wasm the page
// loads, but it is not the page: it has no IndexedDB, no sealed snapshot, no
// MessagePort boundary and no second origin. This drives the same sequence
// through keyholder.js's RPC surface instead, so what passes here is what a
// member's browser actually does.
//
//   ./run.sh                                   (8100 docs, 8103 + 8104 keyholders)
//   RELAY=ws://127.0.0.1:8080/v1/relay node smoke/group-pair.mjs
//
// TWO ORIGINS ARE TWO DEVICES. :8103 and :8104 serve the SAME keyholder from two
// ports, which is two origins, two IndexedDB stores, two device keys and two MLS
// clients. The driver page is on :8100 — the origin `ALLOWED` names for this
// site — and frames both, exactly as docs/pacific-two-devices.html does.
//
// ── THE ONE FIXTURE, AND IT ANNOUNCES ITSELF ─────────────────────────────────
// `leaf()` refuses to stand up without an account seed, and rightly: a browser
// that minted its own would be a person who does not exist. Earning one needs a
// passkey ceremony against a live auth service, which is `smoke/browser.py`'s
// job and not this one. So this PLANTS a seed directly into each keyholder
// origin's IndexedDB, sealed under that origin's own `mls-wrap` key, from a
// page on that origin — the same bytes `adopt()` would have written. Everything
// after that point is keyholder.js unmodified.
//
// THAT MEANS: a pass here says the GROUP path works. It says nothing whatever
// about sign-in, and must never be read as saying so.
import { chromium } from 'playwright-core';

const RELAY = process.env.RELAY || 'ws://127.0.0.1:8080/v1/relay';
const DOCS  = process.env.DOCS  || 'http://localhost:8100';
const KH_A  = process.env.KH_A  || 'http://localhost:8103';
const KH_B  = process.env.KH_B  || 'http://localhost:8104';
const SITE  = 'cambridge-dd';
/* A VISIBLE BROWSER, never headless: a run that decides something is one a person
   can watch it decide. So this is the full Chrome for Testing, headed, and not the
   headless shell it used to launch. */
const EXE = process.env.CHROME ||
  process.env.HOME + '/Library/Caches/ms-playwright/chromium-1234/chrome-mac-arm64/' +
  'Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';

let failures = 0;
const ok   = (c, m) => { console.log((c ? '  ok    ' : '  FAIL  ') + m); if (!c) failures++; };
const head = (m) => console.log('\n' + m);

/* ── the fixture, run on the keyholder's OWN origin ───────────────────────── */
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
  /* THE WRAP KEY IS RACED FOR, and losing that race is what this loop is for.
     MEASURED: this document is the keyholder's own origin, so loading it also
     runs `door()`, which calls `account.status` → `sealedGet('account')` →
     `wrapKey()`. Both sides then read 'mls-wrap', both find nothing, both
     `generateKey`, and the LAST write wins — so the seed was sealed under a key
     the keyholder no longer has and every MLS call afterwards died on a bare
     `OperationError` two origins away from the cause.

     `wrapKey()` only ever mints when the slot is EMPTY, so the fix is to seal,
     read the key back, and check it still opens what was just written. Once the
     door has settled there is exactly one key and the second pass sticks. */
  const seal = async () => {
    let wrap = await tx('readonly', (s) => s.get('mls-wrap'));
    if (!wrap) {
      wrap = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false,
                                             ['encrypt', 'decrypt']);
      await tx('readwrite', (s) => s.put(wrap, 'mls-wrap'));
    }
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const pt = new TextEncoder().encode(JSON.stringify({ seed: seedHex }));
    const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, wrap, pt));
    const out = new Uint8Array(12 + ct.length);
    out.set(iv, 0); out.set(ct, 12);
    let s = ''; for (const b of out) s += String.fromCharCode(b);
    await tx('readwrite', (st) => st.put(btoa(s), 'mls-seed'));
  };
  /* Exactly what `sealedGet('mls-seed')` will do, with the key that is in the
     store NOW rather than the one this function happens to be holding. */
  const opens = async () => {
    const [key, blob] = await Promise.all([
      tx('readonly', (s) => s.get('mls-wrap')),
      tx('readonly', (s) => s.get('mls-seed'))
    ]);
    if (!key || !blob) return false;
    const raw = atob(blob), u = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) u[i] = raw.charCodeAt(i);
    try {
      const pt = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: u.slice(0, 12) },
                                             key, u.slice(12));
      return JSON.parse(new TextDecoder().decode(pt)).seed === seedHex;
    } catch (e) { return false; }
  };

  let planted = false;
  for (let i = 0; i < 20 && !planted; i++) {
    await seal();
    await new Promise((r) => setTimeout(r, 150));   /* let the door finish minting */
    planted = await opens();
  }
  /* A snapshot from an earlier run belongs to an earlier seed, and `mls_init`
     over a stranger's state is the reset lib.rs warns about. */
  await tx('readwrite', (st) => st.delete('mls-state'));
  db.close();
  if (!planted) throw new Error('the seed would not stay sealed under this origin’s wrap key');
  return true;
}

/* ── the embedder's half of the port protocol, injected ───────────────────── */
async function embed([which, origin, site]) {
  window.__kh = window.__kh || {};
  const f = document.createElement('iframe');
  f.style.cssText = 'position:absolute;width:0;height:0;border:0;visibility:hidden';
  f.src = origin + '/index.html';
  const ready = new Promise((res, rej) => {
    const timer = setTimeout(() => rej(new Error(which + ': the keyholder did not answer')), 20000);
    f.onload = () => {
      const ch = new MessageChannel();
      let n = 0;
      const waiting = {}, events = [];
      ch.port1.onmessage = (e) => {
        const d = e.data || {};
        if (d.t === 'pacific.denied') { clearTimeout(timer); return rej(new Error(d.why)); }
        if (d.t === 'pacific.ready') {
          clearTimeout(timer);
          window.__kh[which] = {
            pk: d.pk,
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
        d.ok ? w[0](d.v) : w[1](Object.assign(new Error(d.e), { refusal: d.refusal || null }));
      };
      f.contentWindow.postMessage({ t: 'pacific.init', site }, new URL(origin).origin, [ch.port2]);
    };
  });
  document.body.appendChild(f);
  return ready;
}

/* ── go ───────────────────────────────────────────────────────────────────── */
const browser = await chromium.launch({ executablePath: EXE, headless: false, args: ['--no-sandbox'] });
const ctx = await browser.newContext();
const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('  [pageerror] ' + e.message));
page.on('console', (m) => { if (m.type() === 'error') console.log('  [console] ' + m.text()); });

try {
  head('the fixture — an account seed, planted on each keyholder origin');
  const seeds = { A: 'a1'.repeat(32), B: 'b2'.repeat(32) };
  for (const [name, origin] of [['A', KH_A], ['B', KH_B]]) {
    await page.goto(origin + '/', { waitUntil: 'load' });
    ok(await page.evaluate(plant, seeds[name]), name + ': seed sealed into ' + origin + "'s store (FIXTURE)");
  }

  head('two devices, one relay: ' + RELAY);
  await page.goto(DOCS + '/', { waitUntil: 'load' });
  const pkA = await page.evaluate(embed, ['A', KH_A, SITE]);
  const pkB = await page.evaluate(embed, ['B', KH_B, SITE]);
  ok(!!pkA && !!pkB && pkA !== pkB, 'two keyholders answered, with two device keys');

  const call = (which, m, a) =>
    page.evaluate(([w, mm, aa]) => window.__kh[w].call(mm, aa), [which, m, a || {}]);
  const events = (which) => page.evaluate((w) => window.__kh[w].events, which);
  /* A call whose REFUSAL is the thing under test. page.evaluate carries an
     Error's message across and drops everything else, so the structured
     refusal comes back as data. */
  const tryCall = (which, m, a) =>
    page.evaluate(([w, mm, aa]) => window.__kh[w].call(mm, aa).then(
      (v) => ({ ok: true, v }),
      (e) => ({ ok: false, e: e.message, refusal: e.refusal || null })), [which, m, a || {}]);

  head('A creates a group');
  const g = await call('A', 'group.create', { name: 'Relay pair, in the browser', relay: RELAY });
  ok(/^[0-9a-f]{32,}$/.test(g.group_id || ''), 'group.create → ' + String(g.group_id).slice(0, 16) + '… at epoch ' + g.epoch);
  ok(g.tag && !('secret' in g), 'it returns the TAG and never the epoch seal secret');
  ok(typeof g.spine_index === 'number', "and it is NAMED on A's spine, at index " + g.spine_index);

  head('B mints a KeyPackage, A adds it');
  const kp = await call('B', 'group.keyPackage', {});
  ok(!!kp.key_package, 'group.keyPackage → ' + kp.key_package.length / 2 + ' bytes');
  const inv = await call('A', 'group.invite', { group_id: g.group_id, key_package: kp.key_package, relay: RELAY });
  ok(inv.committed === true, 'group.invite published the commit and the relay ACCEPTED it (Ack ok)');
  ok(inv.epoch === g.epoch + 1, 'A advanced to epoch ' + inv.epoch);

  head('B joins from the Welcome');
  const j = await call('B', 'group.join', { welcome: inv.welcome, relay: RELAY });
  ok(j.group_id === g.group_id, 'B joined the group A created, at epoch ' + j.epoch);
  ok(j.tag === inv.tag, 'both sides derived the SAME mailbox — ' + String(j.tag).slice(0, 12) + '…');
  ok(j.named === true && typeof j.spine_index === 'number', "and B's spine names it, at index " + j.spine_index);

  head('A opens its receive arm');
  const w = await call('A', 'group.watch', { relay: RELAY });
  ok((w.watching || []).some((x) => x.group_id === g.group_id), 'group.watch is listening on ' + (w.watching || []).length + ' group(s)');

  head('THE DIRECTION UNDER TEST — B → A, through the keyholder');
  const sent = await call('B', 'group.send', { group_id: g.group_id, text: 'Hello from the browser', gen: 1, relay: RELAY });
  ok(sent.published === true, 'group.send authored a core delta and the relay stored it (' + sent.delta.length / 2 + ' bytes)');

  let got = null;
  for (let i = 0; i < 60 && !got; i++) {
    await new Promise((r) => setTimeout(r, 250));
    got = (await events('A')).find((e) => e.t === 'pacific.group.delta');
  }
  ok(!!got, 'A received a delta on the group tag' + (got ? ' at epoch ' + got.epoch : ' — NOTHING ARRIVED'));
  if (got) {
    ok(got.delta === sent.delta, 'the delta is byte-identical to what B authored');
    ok(got.group === g.group_id, 'attributed to the group A created');
    ok(/^[0-9a-f]{64}$/.test(got.sender || ''), "attributed to B's leaf (" + String(got.sender).slice(0, 16) + '…), not carried in the payload');
  }

  head('THE CORE FOLDS IT — object.fold, on both devices');
  // A received the message and B authored it. Both folds go through the core's
  // fold_object, so they must agree: the same view, the same digest.
  const fa = await call('A', 'object.fold', { group: g.group_id });
  const ga = (fa.groups || [])[0];
  ok((fa.rejected || []).length === 0,
     'A folds the group with nothing refused' + ((fa.rejected || []).length ? ' — ' + fa.rejected[0].why : ''));
  ok(!!ga && ga.kind === 'forum', 'the kind is read from the log, not remembered: ' + (ga && ga.kind));
  const ma = (ga && ga.view && ga.view.messages) || [];
  ok(ma.length === 1 && ma[0].text === 'Hello from the browser',
     "A's fold holds B's message: " + JSON.stringify(ma.map((m) => m.text)));
  ok(!!got && !!ma[0] && ma[0].author === got.sender, 'attributed to B, as MLS said on arrival');
  ok(!!ga && /^[0-9a-f]{64}$/.test(fa.exported_by || '') && ga.owner === fa.exported_by,
     'the owner is A, who made it, as the group\'s own MLS context says');
  const fb = await call('B', 'object.fold', { group: g.group_id });
  const gb = (fb.groups || [])[0];
  ok(!!gb && !!ga && JSON.stringify(gb.view) === JSON.stringify(ga.view),
     "B's fold of its own message is the SAME view A folded");
  ok(!!gb && !!ga && gb.digest === ga.digest, 'and the same digest: ' + String(ga && ga.digest).slice(0, 12) + '…');

  head('what this producer says it can do');
  const cap = await call('A', 'ops.capabilities', {});
  ok(cap.writes === true, 'this driver origin may write (WRITERS); production origins may not until it is ruled');
  const fp = cap.ops && cap.ops['forum.post'];
  ok(!!fp && fp.reachable === true && /gen floor/.test(fp.needs || ''),
     'forum.post is reachable, and says what it needs: ' + String(fp && fp.needs).slice(0, 40) + '…');
  const mj = cap.ops && cap.ops['base.memberJoined'];
  ok(!!mj && mj.reachable === false && /MLS doors/.test(mj.why || ''), 'membership is refused, in words: ' + (mj && mj.why));
  // `on` is what the door will accept, asked of the same check build makes: the
  // ratify ops are on every object, and a note is not written onto a Site's group.
  const rv = cap.ops && cap.ops['ratify.vote'];
  ok(!!rv && rv.reachable === true && rv.on.includes('forum') && rv.on.includes('group'),
     'the ratify ops are on every kind, as build accepts them: ' + (rv && rv.on.length) + ' kinds');
  const nw = cap.ops && cap.ops['base.noteWrite'];
  ok(!!nw && JSON.stringify(nw.on) === '["notebook","note"]', 'a note op is offered only on notebooks: ' + JSON.stringify(nw && nw.on));

  head('THE WRITES — object.mint and object.author, through the core\'s one door');
  const ev = await call('A', 'object.mint', { kind: 'event', relay: RELAY, draft: {
    name: 'Prodigal at the Hall', start_ms: 1760000000000, venue: 'The Hall', lineup: 'Prodigal\nSupport act' } });
  ok(/^[0-9a-f]{32,}$/.test(ev.group_id || ''), 'A minted an event: ' + String(ev.group_id).slice(0, 12) + '…');
  const tk = await call('A', 'object.author', { group: ev.group_id, op: 'event.setTickets', relay: RELAY,
    // `rev` is optional in the ICD and REQUIRED by the reducer: drift, reported 22 Sep.
    // Passed here because both sides accept it.
    args: { priceCents: 0, currency: 'gbp', capacity: 150, open: 1, rev: 1 } });
  ok(/^[0-9a-f]{64}$/.test(tk.deltaId || ''), 'and set its tickets: delta ' + String(tk.deltaId).slice(0, 12) + '…');
  const fe = (await call('A', 'object.fold', { group: ev.group_id })).groups[0] || {};
  ok(fe.kind === 'event' && fe.view && fe.view.title === 'Prodigal at the Hall', 'the fold reads it back: ' + (fe.view && fe.view.title));
  ok(!!fe.view && JSON.stringify(fe.view.lineup) === '["Prodigal","Support act"]' && fe.view.tickets && fe.view.tickets.capacity === 150,
     'with its lineup and its 150 tickets');
  // The external ticket link (ICD, approved 22 Sep). setProfile replaces the whole
  // profile, so the lineup is written again beside it.
  await call('A', 'object.author', { group: ev.group_id, op: 'event.setProfile', relay: RELAY,
    args: { title: 'Prodigal at the Hall', startMs: 1760000000000, venue: 'The Hall',
            lineup: 'Prodigal\nSupport act', ticketUrl: 'https://tickets.example/prodigal' } });
  const fl = (await call('A', 'object.fold', { group: ev.group_id })).groups[0] || {};
  ok(!!fl.view && fl.view.ticket_url === 'https://tickets.example/prodigal', 'and a ticket link, read back: ' + (fl.view && fl.view.ticket_url));
  const js = await tryCall('A', 'object.author', { group: ev.group_id, op: 'event.setProfile', relay: RELAY,
    args: { title: 'Prodigal at the Hall', startMs: 1760000000000, ticketUrl: 'javascript:alert(1)' } });
  ok(!js.ok && js.refusal && js.refusal.by === 'core', 'a ticket link that is not a web address is refused by the core: ' + String(js.refusal && js.refusal.why).slice(0, 60));

  const fo = await call('A', 'object.mint', { kind: 'forum', draft: { name: 'Hall chat' }, relay: RELAY });
  const empty = (await call('A', 'object.fold', { group: fo.group_id })).groups[0] || {};
  ok(empty.kind === 'forum' && (empty.view.messages || []).length === 0,
     'a forum is minted name-only, and folds as an empty forum rather than as nothing');
  await call('A', 'object.author', { group: fo.group_id, op: 'forum.post', args: { text: 'first post' }, relay: RELAY });
  const posted = (await call('A', 'object.fold', { group: fo.group_id })).groups[0] || {};
  ok(JSON.stringify((posted.view.messages || []).map((m) => m.text)) === '["first post"]', 'and the first post is in it');

  head('THE SPINE — what a device that has never seen the account would find');
  const sa = await call('A', 'spine.read', { relay: RELAY });
  const named = (s, id) => (s.objects || []).find((o) => o.group_id === id);
  ok(!!named(sa, g.group_id) && named(sa, g.group_id).first_epoch === 0, "A's spine names the group A made, from epoch 0");
  ok(!!named(sa, ev.group_id) && !!named(sa, fo.group_id), "and the event and the forum A minted in the browser");
  ok(sa.recovery && sa.recovery.tail === 'unknown', 'with tail "unknown": no head advances yet, and it says so');
  const sb = await call('B', 'spine.read', { relay: RELAY });
  ok(!!named(sb, g.group_id) && named(sb, g.group_id).first_epoch === j.epoch,
     "B's spine names the group B joined, from the epoch B joined it (" + j.epoch + ')');
  ok(!named(sb, ev.group_id), "and not A's event: each person's spine is their own");

  head('the door refuses, and says who said no');
  const before = (await call('A', 'object.fold', {})).groups.length;
  const inert = await tryCall('A', 'object.mint', { kind: 'event', draft: { name: 'When?' }, relay: RELAY });
  ok(!inert.ok && inert.refusal && inert.refusal.by === 'core', 'an event with no start: refused by the core — ' + String(inert.refusal && inert.refusal.why).slice(0, 60));
  ok((await call('A', 'object.fold', {})).groups.length === before, 'and NOTHING was made: both or neither');
  const typo = await tryCall('A', 'object.mint', { kind: 'event', draft: { name: 'Gig', startMs: 1 }, relay: RELAY });
  ok(!typo.ok && typo.refusal && typo.refusal.by === 'core' && /startMs/.test(typo.refusal.why), 'a misspelt draft field is refused, not dropped');
  const gen = await tryCall('A', 'object.author', { group: fo.group_id, op: 'forum.post', args: { text: 'x', gen: 7 }, relay: RELAY });
  ok(!gen.ok && gen.refusal && gen.refusal.by === 'binding', 'a caller-supplied gen: refused by the binding — the door assigns it');
  // B joined the pair group through a RAW Welcome, which carries no floor: G1.
  const noFloor = await tryCall('B', 'object.author', { group: g.group_id, op: 'forum.post', args: { text: 'from B' }, relay: RELAY });
  ok(!noFloor.ok && noFloor.refusal && noFloor.refusal.by === 'core' && /floor/.test(noFloor.refusal.why),
     'B has no gen floor for a group it joined by a raw Welcome: refused by the core, not collided — ' + String(noFloor.refusal && noFloor.refusal.why).slice(0, 50) + '…');
  const aPost = await tryCall('A', 'object.author', { group: g.group_id, op: 'forum.post', args: { text: 'from A' }, relay: RELAY });
  ok(aPost.ok, 'A, who made that group, has floor 0 and may post in it');
  // A handler's own failure goes out STRUCTURED, so a consumer's R1 decides it and
  // R4's text match (meant for connect failures) never mislabels it transport.
  const plain = await tryCall('A', 'group.invite', {});
  ok(!plain.ok && plain.refusal && plain.refusal.by === 'keyholder' && /wants group_id/.test(plain.refusal.why),
     "a handler's plain error arrives as {by: keyholder}: " + (plain.refusal && plain.refusal.why));
  const unknown = await tryCall('A', 'no.such.method', {});
  ok(!unknown.ok && !unknown.refusal && /no such method/.test(unknown.e),
     'an unknown method stays bare, so R3 answers it in its own words');

  head('G1 — B joins the forum by an INTRO, arrives with a floor, and posts');
  // B hands over its signed card, as a phone would. A adds B by it: the card
  // carries the KeyPackage AND the intro tag, so the Welcome goes to B's intro
  // mailbox sealed with the floor A's log shows.
  const card = await call('B', 'device.bundle', { name: 'B' });
  ok(typeof card.bundle === 'string' && card.bundle.length > 100, "B's contact card, signed by B's person");
  const byCard = await call('A', 'group.invite', { group_id: fo.group_id, bundle: card.bundle, name: 'A', relay: RELAY });
  ok(byCard.committed === true && byCard.delivered === true,
     'A added B by the card and delivered the intro to B\'s mailbox');
  ok(typeof byCard.floor === 'number' && byCard.floor >= 1,
     'the floor A handed is past the post A already made there: ' + byCard.floor);
  const inbox = await call('B', 'group.inbox', { relay: RELAY });
  const joinedFo = (inbox.joined || []).find((j) => j.group_id === fo.group_id);
  ok(!!joinedFo, 'B drained its intro mailbox and joined the forum' +
     ((inbox.refused || []).length ? ' (' + inbox.refused.length + ' older intro(s) refused, by name)' : ''));
  ok(!!joinedFo && joinedFo.floor === byCard.floor && joinedFo.kind === 'forum' && joinedFo.owner === fa.exported_by,
     'with the floor A sent, as a forum, owned by A — what the intro said, checked against the roster');
  const bPost = await tryCall('B', 'object.author', { group: fo.group_id, op: 'forum.post',
    args: { text: 'B, from its own browser' }, relay: RELAY });
  ok(bPost.ok, 'B may post in what it joined: the core positioned it above the floor' +
     (bPost.ok ? '' : ' — refused: ' + (bPost.refusal ? bPost.refusal.why : bPost.e)));
  let heard = null;
  for (let i = 0; i < 60 && !heard; i++) {
    await new Promise((r) => setTimeout(r, 250));
    heard = (await events('A')).find((e) => e.t === 'pacific.group.delta' && e.group === fo.group_id);
  }
  ok(!!heard, 'A received B\'s post on the forum\'s tag');
  const both = (await call('A', 'object.fold', { group: fo.group_id }));
  const texts = (((both.groups || [])[0] || {}).view || {}).messages || [];
  ok((both.rejected || []).length === 0 &&
     JSON.stringify(texts.map((m) => m.text)) === '["first post","B, from its own browser"]',
     "A's fold holds both posts, in order, nothing refused: " + JSON.stringify(texts.map((m) => m.text)));

  head('THE CLIENT — as another site loads it: beside pacific.js, over pacific.connect');
  // What egregore, thedoor and cyp3 import. Loaded from this origin the way they
  // load it from theirs, and asked through the real connect, not the smoke's frame.
  const viaClient = await page.evaluate(async ([kh, site, forum]) => {
    const load = (src) => new Promise((res, rej) => {
      const s = document.createElement('script'); s.src = src; s.onload = res; s.onerror = () => rej(new Error(src + ' did not load'));
      document.head.appendChild(s);
    });
    for (const f of ['/pacific.js', '/wallflowers-fold.js', '/wallflowers-client.js']) await load(f);
    const api = window.WallflowersClient.socialApi(() => window.Pacific.connect({ keyholder: kh, site }));
    const cap = await api.capabilities();
    const model = await api.fold(forum);
    const wrote = await api.author(forum, 'forum.post', { text: 'through the shared client' });
    const again = await api.fold(forum);
    const nope = await api.author(forum, 'forum.post', { text: 'x', gen: 3 });
    return { writes: cap.writes, reachable: cap.ops && cap.ops['forum.post'] && cap.ops['forum.post'].reachable,
             before: model.groups && model.groups[0] && model.groups[0].messages.length,
             owner: model.groups && model.groups[0] && model.groups[0].owner, fixture: model.fixture,
             wrote, after: again.groups && again.groups[0] && again.groups[0].messages.map((m) => m.text), nope };
  }, [KH_A, SITE, fo.group_id]);
  ok(viaClient.writes === true && viaClient.reachable === true, 'capabilities through the client: this origin writes, forum.post is reachable');
  ok(viaClient.before >= 2 && /^[0-9a-f]{64}$/.test(viaClient.owner || '') && viaClient.fixture === false,
     'fold through the client, read by the producer\'s normalise: ' + viaClient.before + ' posts, the owner kept, not a fixture');
  ok(viaClient.wrote && viaClient.wrote.ok === true && /^[0-9a-f]{64}$/.test(viaClient.wrote.deltaId || ''),
     'author through the client: a delta id ' + String(viaClient.wrote && viaClient.wrote.deltaId).slice(0, 12) + '…');
  ok(Array.isArray(viaClient.after) && viaClient.after[viaClient.after.length - 1] === 'through the shared client',
     'and the fold reads it back');
  ok(viaClient.nope && viaClient.nope.ok === false && viaClient.nope.by === 'binding',
     'a refusal through the client says whose it is: ' + (viaClient.nope && viaClient.nope.by));

  const simmed = await page.evaluate(async () => {
    const answer = await (await fetch('/wallflowers-sim.json')).json();
    const api = window.WallflowersClient.socialApi(window.WallflowersClient.sim(answer));
    const m = await api.fold();
    const w = await api.author(m.groups[0].id, 'forum.post', { text: 'x' });
    return { fixture: m.fixture, n: m.groups.length, problems: m.problems.length, refused: w.ok === false && /fixture/.test(w.why) };
  });
  ok(simmed.fixture === true && simmed.n > 0 && simmed.problems > 0 && simmed.refused,
     'the served sim answer draws through the client, says it is a fixture, and refuses writes: ' +
     simmed.n + ' objects, ' + simmed.problems + ' refused');

  const siteSim = await page.evaluate(async () => {
    await new Promise((res, rej) => { const s = document.createElement('script'); s.src = '/wallflowers-sim-site.js';
      s.onload = res; s.onerror = () => rej(new Error('wallflowers-sim-site.js did not load')); document.head.appendChild(s); });
    const m = await window.WallflowersClient.socialApi(window.WallflowersClient.sim(self.WallflowersSimAnswer)).fold();
    const site = m.groups.find((g) => g.kind === 'group' && g.view && Array.isArray(g.view.offices) && g.view.offices.length);
    return { fixture: m.fixture, problems: m.problems.length, owned: !!site && site.owner === m.identity,
             offices: site ? site.view.offices.length : 0, forum: site && site.view.forums[0] && site.view.forums[0].name,
             rooms: (m.groups.find((g) => g.name === 'guestbook') || {}).view?.rooms?.length || 0 };
  });
  ok(siteSim.fixture === true && siteSim.problems === 0 && siteSim.owned && siteSim.offices === 4 &&
     siteSim.forum === 'guestbook' && siteSim.rooms === 1,
     'the Site fixture loads as a script and draws: owned by the viewer, 4 offices, a guestbook with a room: ' + JSON.stringify(siteSim));

  head('what is NOT here');
  console.log('  note  writes answer only to this driver origin. Which member sites may write is not');
  console.log('        ruled; group.send, used above, is the older path and is not narrowed.');
  console.log('  note  the log object.fold reads is THIS DEVICE\'s copy, on its own origin. Another');
  console.log('        device gets the content back through the archive and the spine, not from here.');
} catch (e) {
  ok(false, String(e && e.message || e));
} finally {
  await browser.close();
}

console.log('\n' + (failures ? failures + ' FAILED' : 'all passed — the browser is an MLS member: it creates, adds, joins, encrypts, relays and decrypts, and the core folds what it holds'));
process.exit(failures ? 1 : 0);
