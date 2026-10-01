/* world.js under node: the fold → world mapping, held to the interior's shape.

   Run:  node world.test.cjs        (or: node --test world.test.cjs)

   Three sources, in order of how much they prove.

   A HAND-WRITTEN fold in the exact JSON `fold_archive` emits — the contract both
   tracks build to — with an assertion on every field the mapping fills.

   The wasm track's fixture, when it is there: core/coordination/
   archive-fixture.expected.json is a SUMMARY of the fixture archive (per group:
   kind, digest, delta and message counts), not the fold itself, so a synthetic
   fold is built from it with the counts it states and message text of this
   file's own — enough to hold the mapping to "one thread per forum, one
   conversation per connection, every message in order".

   And the fixture archive itself, archive-fixture.cbor, folded through the real
   core — the wasm-bindgen build under docs/core, stood up under node with the
   same loader shape keyholder.js uses — when that build carries `fold_archive`.
   That is the end-to-end check: the core's own fold, the summary's own counts
   and digests, this mapping in between. Its absence is reported as a skip with
   the reason, never as a pass. */
'use strict';
/* The mapping renders times in local time, as the interior does. The
   assertions below are exact, so the clock is pinned before any Date exists. */
process.env.TZ = 'UTC';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const { worldFromFold, handleFor, FOLD_V } = require('./world.js');

const key = (c) => c.repeat(64);          // a readable 64-hex identity
const gid = (c) => c.repeat(64);          // group ids are hex of any length; 32 bytes here
const ME = key('a'), BOB = key('b'), CAROL = key('c'), DAN = key('d'), EVE = key('e');
const NOW = Date.UTC(2026, 8, 10, 12, 0, 0);   // 10 Sep 2026 12:00Z
const min = 60 * 1000, hour = 60 * min, day = 24 * hour;

const msg = (author, gen, text, ts) => ({
  author, gen, text, ts, reply_to: null, up: 0, down: 0, reactions: []
});

/* The shape from the task, exactly. Bob is named twice on purpose — 'bob' in
   the peers list (the pairing-bundle name this device recorded) and 'Bob B' by
   his own Group (his profile) — because the mapping has to choose, and the
   profile is what he calls himself now. */
function fixture() {
  return {
    v: FOLD_V,
    exported_by: ME,
    display_name: 'Ada',
    /* The edges the archive carries — a record's subject is never inferred from
       its owner (Ada minted Bob's record too, and would otherwise be renamed). */
    own_group: gid('6'),
    peers: [
      { id: BOB, name: 'bob', status: 'connected', identity_group: gid('7') },
      { id: CAROL, name: 'Carol Díaz', status: 'connected' },
      { id: DAN, name: '', status: 'pending' }
    ],
    groups: [
      { id: gid('1'), kind: 'forum', name: 'Cookbook', owner: ME, members: [ME, BOB, CAROL],
        digest: key('0'),
        view: { messages: [
          msg(BOB, 1, 'Printer quoted 300.', NOW - 3 * hour),
          msg(ME, 2, 'Split it 200/100.', NOW - 2 * hour),
          msg(CAROL, 3, 'Xalapa can place 120.', NOW - 40 * min)
        ] } },
      { id: gid('2'), kind: 'forum', name: '', owner: BOB, members: [ME, BOB],
        digest: key('0'),
        view: { messages: [ msg(BOB, 1, 'Long table seats 40.', NOW - 2 * day) ] } },
      { id: gid('3'), kind: 'forum', name: 'Empty room', owner: ME, members: [ME],
        digest: key('0'), view: { messages: [] } },
      { id: gid('4'), kind: 'connection', name: '', owner: ME, members: [ME, BOB],
        digest: key('0'),
        view: { messages: [
          msg(BOB, 1, 'Plates are held until Friday.', NOW - 5 * hour),
          msg(ME, 2, 'Spanish first?', NOW - 4 * hour)
        ] } },
      { id: gid('5'), kind: 'conversation', name: 'Kitchen', owner: CAROL, members: [CAROL, ME, DAN],
        digest: key('0'),
        view: { messages: [ msg(ME, 1, 'Two burners only.', NOW - 10 * min) ] } },
      { id: gid('6'), kind: 'group', name: 'Ada', owner: ME, members: [ME, BOB],
        digest: key('0'),
        view: { display_name: 'Ada Lovelace', shape: 'individual', presence: 'onPlatform',
                roles: [[BOB, 'Member'], [ME, 'Owner']] } },
      { id: gid('7'), kind: 'group', name: 'Bob', owner: BOB, members: [BOB],
        digest: key('0'),
        view: { display_name: 'Bob B', shape: 'individual', presence: 'offPlatform', roles: [] } },
      { id: gid('8'), kind: 'group', name: 'Las 3000', owner: BOB, members: [BOB, ME],
        digest: key('0'),
        view: { display_name: 'Las 3000', shape: 'team', presence: 'onPlatform', roles: [] } },
      { id: gid('9'), kind: 'event', name: 'Launch', owner: ME, members: [ME, BOB],
        digest: key('0'),
        view: { title: 'Cookbook Zine 2 — launch', descriptor: 'Void Space, 18:00',
                start_ms: Date.UTC(2026, 8, 26, 18, 0), end_ms: Date.UTC(2026, 8, 26, 21, 0),
                venue: 'Void Space' } },
      { id: gid('f'), kind: 'event', name: 'Undated', owner: ME, members: [ME],
        digest: key('0'),
        view: { title: 'Sometime', descriptor: '', start_ms: 0, end_ms: null, venue: '' } },
      { id: gid('e'), kind: 'place', name: 'The yard', owner: ME, members: [ME],
        digest: key('0'), view: { unsupported: true } }
    ],
    rejected: [ { group: gid('d'), why: 'a foreign type_id on the log' } ]
  };
}

test('handleFor: a peer name becomes a slug, no name becomes eight hex', () => {
  const peers = [{ id: BOB, name: 'bob' }, { id: CAROL, name: 'Carol Díaz' }, { id: DAN, name: '' }];
  assert.equal(handleFor(BOB, peers), 'bob');
  assert.equal(handleFor(CAROL, peers), 'carol-díaz');
  assert.equal(handleFor(DAN, peers), 'dddddddd');
  assert.equal(handleFor(EVE, peers), 'eeeeeeee');           // not a peer at all
  assert.equal(handleFor(EVE, []), 'eeeeeeee');
  assert.equal(handleFor(EVE), 'eeeeeeee');
});

test('handleFor: two people with one name do not share a handle', () => {
  const peers = [{ id: BOB, name: 'sam' }, { id: CAROL, name: 'Sam ' }];
  assert.equal(handleFor(BOB, peers), 'bbbbbbbb');
  assert.equal(handleFor(CAROL, peers), 'cccccccc');
});

test('handleFor: refuses a key that is not 64 hex', () => {
  assert.throws(() => handleFor('abc', []), /64 hex/);
  assert.throws(() => handleFor(undefined, []), /64 hex/);
});

test('the site is the exporter\'s own individual group, and ME is its handle', () => {
  const w = worldFromFold(fixture(), ME, { now: NOW });
  assert.equal(w.site, 'ada-lovelace');
  assert.equal(w.me, 'ada-lovelace');
  assert.deepEqual(w.sites, {
    'ada-lovelace': { name: 'Ada Lovelace', kind: 'Individual', disc: 'chat', shape: 'individual' }
  });
});

test('threads: one per forum, posts in order, text preserved, newest first', () => {
  const w = worldFromFold(fixture(), ME, { now: NOW });
  assert.equal(w.threads.length, 3);
  assert.deepEqual(w.threads.map((t) => t.id), [gid('1'), gid('2'), gid('3')]);

  const t = w.threads[0];
  assert.equal(t.title, 'Cookbook');
  assert.equal(t.by, 'bob-b');                     // the first post's author opened it
  assert.equal(t.n, 3);
  assert.equal(t.when, '40m');                     // the newest post, relative to `now`
  assert.equal(t.channel, '');
  assert.deepEqual(t.posts, [
    { by: 'bob-b',        at: '09:00', s: 'Printer quoted 300.' },
    { by: 'ada-lovelace', at: '10:00', s: 'Split it 200/100.' },
    { by: 'carol-díaz',   at: '11:20', s: 'Xalapa can place 120.' }
  ]);

  assert.equal(w.threads[1].title, '22222222');    // unnamed: called by its id
  assert.equal(w.threads[1].when, '2d');
  const empty = w.threads[2];
  assert.equal(empty.n, 0);
  assert.deepEqual(empty.posts, []);
  assert.equal(empty.by, 'ada-lovelace');          // nobody posted: the owner
  assert.equal(empty.when, '');
});

test('convs: one per connection/conversation, with = the other member, me = author == me', () => {
  const w = worldFromFold(fixture(), ME, { now: NOW });
  assert.equal(w.convs.length, 2);
  const kitchen = w.convs[0], bob = w.convs[1];     // newest first
  assert.equal(kitchen.id, gid('5'));
  assert.equal(kitchen.with, 'carol-díaz');         // first member who is not me
  assert.deepEqual(kitchen.msgs, [{ me: 1, s: 'Two burners only.' }]);
  assert.equal(bob.id, gid('4'));
  assert.equal(bob.with, 'bob-b');
  assert.equal(bob.unread, 0);
  assert.deepEqual(bob.msgs, [
    { me: 0, s: 'Plates are held until Friday.' },
    { me: 1, s: 'Spanish first?' }
  ]);
});

test('people: everyone mentioned, individual groups and peers fill in, nothing invented', () => {
  const w = worldFromFold(fixture(), ME, { now: NOW });
  assert.deepEqual(Object.keys(w.people).sort(),
    ['ada-lovelace', 'bob-b', 'carol-díaz', 'dddddddd']);
  assert.deepEqual(w.people['ada-lovelace'],
    { n: 'Ada Lovelace', home: 'ada-lovelace', role: 'Owner', joined: '—', on: 0, presence: 'onPlatform' });
  assert.deepEqual(w.people['bob-b'],
    { n: 'Bob B', home: 'ada-lovelace', role: 'Member', joined: '—', on: 0, presence: 'offPlatform' });
  assert.deepEqual(w.people['carol-díaz'],
    { n: 'Carol Díaz', home: 'ada-lovelace', role: '', joined: '—', on: 0, presence: 'undeclared' });
  assert.deepEqual(w.people.dddddddd,
    { n: 'dddddddd', home: 'ada-lovelace', role: '', joined: '—', on: 0, presence: 'undeclared' });
  assert.equal(w.people.bob, undefined);           // the profile name won; no second entry
  assert.equal(w.people['las-3000'], undefined);   // a team is not a person
});

test('events: dated ones render the seed\'s fields, undated ones render blanks, hosted here', () => {
  const w = worldFromFold(fixture(), ME, { now: NOW });
  assert.equal(w.events.length, 2);
  const e = w.events[0];
  assert.ok(w.sites[e.host], 'host must index into sites or the interior throws');
  assert.deepEqual(e, {
    id: gid('9'), d: '26', m: 'Sep', t: 'Cookbook Zine 2 — launch', at: '18:00 · Void Space',
    host: 'ada-lovelace', going: [], more: 0,
    start: '2026-09-26T18:00', end: '2026-09-26T21:00', venue: 'Void Space'
  });
  const u = w.events[1];
  assert.deepEqual(u, {
    id: gid('f'), d: '', m: '', t: 'Sometime', at: '', host: 'ada-lovelace', going: [], more: 0,
    start: null, end: null, venue: ''
  });
});

test('the keys pacific.js reads are all present, and empty rather than missing', () => {
  const w = worldFromFold(fixture(), ME, { now: NOW });
  assert.deepEqual(Object.keys(w).sort(),
    ['convs', 'events', 'links', 'listings', 'me', 'notes', 'pending', 'people',
     'posts', 'rsvp', 'site', 'sites', 'threads', 'wallet'].sort());
  assert.deepEqual(w.rsvp, {});
  assert.deepEqual(w.pending, []);
  assert.deepEqual(w.links, []);
  assert.deepEqual(w.listings, []);
  assert.deepEqual(w.posts, []);
  /* This fixture's groups carry no `notebook` and no `wallet` \u2014 they are
     hand-written folds that predate the facets, exactly like a wasm built before
     `group_view` emitted them. Absent stays absent: an empty notebook, and a
     wallet that is null because no treasury was opened, not because the mapping
     could not read one. */
  assert.deepEqual(w.notes, []);
  assert.equal(w.wallet, null);
});

test('no individual group for the exporter: a synthetic home named by the archive', () => {
  const f = fixture();
  f.groups = f.groups.filter((g) => g.id !== gid('6'));
  const w = worldFromFold(f, ME, { now: NOW });
  assert.equal(w.site, 'home');
  assert.deepEqual(w.sites.home, { name: 'Ada', kind: 'Individual', disc: 'chat', shape: 'individual' });
  assert.equal(w.me, 'ada');
  assert.equal(w.people.ada.n, 'Ada');
  assert.equal(w.people.ada.role, '');             // no own group, no roles to read
  assert.equal(w.people['bob-b'].home, 'home');
});

test('me on the other side: the same conversation flips', () => {
  const w = worldFromFold(fixture(), BOB, { now: NOW });
  const bob = w.convs.find((c) => c.id === gid('4'));
  assert.equal(bob.with, 'ada-lovelace');
  assert.deepEqual(bob.msgs.map((m) => m.me), [1, 0]);
  assert.equal(w.me, 'bob-b');
  assert.equal(w.site, 'ada-lovelace');            // the site is still the EXPORTER's
});

test('refuses what it does not understand, by name', () => {
  assert.throws(() => worldFromFold(null, ME), /no fold/);
  // A version this mapping does not read, expressed RELATIVE to the one it does.
  // This said `v: 2` literally, from when the mapping read v1 — so the day the
  // core moved to v2 the test went on asserting that the current version must be
  // refused, which is precisely what the code was wrongly doing.
  const NEXT = FOLD_V + 1;
  assert.throws(() => worldFromFold({ v: NEXT, exported_by: ME, groups: [], peers: [] }, ME),
                new RegExp('v' + NEXT));
  assert.throws(() => worldFromFold({ v: FOLD_V, exported_by: 'nope', groups: [], peers: [] }, ME), /exported_by/);
  assert.throws(() => worldFromFold({ v: FOLD_V, exported_by: ME, groups: [], peers: [] }, 'me'), /me must be 64 hex/);
  assert.throws(() => worldFromFold({ v: FOLD_V, exported_by: ME, groups: {}, peers: [] }, ME), /groups/);
  const noMsgs = { v: FOLD_V, exported_by: ME, peers: [], groups: [
    { id: gid('1'), kind: 'forum', name: 'x', owner: ME, members: [ME], view: {} } ] };
  assert.throws(() => worldFromFold(noMsgs, ME), /no messages array/);
  const badAuthor = { v: FOLD_V, exported_by: ME, peers: [], groups: [
    { id: gid('1'), kind: 'forum', name: 'x', owner: ME, members: [ME],
      view: { messages: [{ author: 'bob', gen: 1, text: 'hi', ts: 0 }] } } ] };
  assert.throws(() => worldFromFold(badAuthor, ME), /64-hex author/);
});

/* ── the wasm track's fixture ─────────────────────────────────────────────── */
const COORD = path.resolve(__dirname, '../../../core/coordination');
const SUMMARY = path.join(COORD, 'archive-fixture.expected.json');
const CBOR = path.join(COORD, 'archive-fixture.cbor');
const WASM = path.resolve(__dirname, '../docs/core/core_wasm_bg.wasm');

const MESSAGE_KINDS = ['forum', 'connection', 'conversation'];
/* The summary shape, and WHY each clause. `v` was a hardcoded 1 here — a fourth
   copy of the number, and the one that outlived the other three, because when it
   failed it blamed the KEYS: the fixture's keys were fine and the version was
   not, so the message sent the reader looking in the wrong place. It reports the
   failing clause by name now. */
function summaryFault(d) {
  if (!d || typeof d !== 'object') return 'not an object';
  if (d.v !== FOLD_V) return 'v' + d.v + ', but this reads v' + FOLD_V;
  if (!d.groups || Array.isArray(d.groups)) return 'groups must be an object keyed by id';
  if (typeof d.exported_by !== 'string') return 'no exported_by';
  return '';
}

/* A fold with the summary's shape: its groups, kinds, digests and message
   COUNTS, with text and members of this file's own — synthetic in exactly that
   sense, and no other. */
function foldFromSummary(s) {
  const groups = Object.keys(s.groups).map((id) => {
    const g = s.groups[id];
    let view;
    if (MESSAGE_KINDS.includes(g.kind)) {
      const n = g.messages || 0;
      view = { messages: Array.from({ length: n }, (_, i) =>
        msg(i % 2 ? s.exported_by : BOB, i + 1, 'message ' + (i + 1) + ' of ' + id.slice(0, 8),
            NOW - (n - i) * min)) };
    } else if (g.kind === 'group') {
      view = { display_name: '', shape: 'team', presence: 'undeclared', roles: [] };
    } else if (g.kind === 'event') {
      view = { title: '', descriptor: '', start_ms: 0, end_ms: null, venue: '' };
    } else {
      view = { unsupported: true };
    }
    return { id, kind: g.kind, name: '', owner: s.exported_by, members: [s.exported_by, BOB],
             digest: g.digest, view };
  });
  return { v: s.v, exported_by: s.exported_by, display_name: s.display_name,
           peers: [{ id: BOB, name: 'bea', status: 'connected' }], groups, rejected: [] };
}

/* What every fold — synthetic or real — must map to. */
function assertMapsFaithfully(fold, summary) {
  const w = worldFromFold(fold, fold.exported_by, { now: NOW });
  const kinds = (...ks) => fold.groups.filter((g) => ks.includes(g.kind));
  assert.equal(w.threads.length, kinds('forum').length, 'one thread per forum');
  assert.equal(w.convs.length, kinds('connection', 'conversation').length, 'one conv per tether');
  assert.equal(w.events.length, kinds('event').length, 'one event per event');
  for (const g of kinds('forum')) {
    const t = w.threads.find((x) => x.id === g.id);
    assert.ok(t, 'thread for ' + g.id);
    assert.deepEqual(t.posts.map((p) => p.s), g.view.messages.map((m) => m.text), 'text, in order');
    assert.equal(t.n, g.view.messages.length);
  }
  for (const g of kinds('connection', 'conversation')) {
    const c = w.convs.find((x) => x.id === g.id);
    assert.ok(c, 'conv for ' + g.id);
    assert.deepEqual(c.msgs.map((m) => m.s), g.view.messages.map((m) => m.text), 'text, in order');
    assert.deepEqual(c.msgs.map((m) => m.me),
      g.view.messages.map((m) => (m.author === fold.exported_by ? 1 : 0)), 'me = author == me');
  }
  assert.ok(w.sites[w.site], 'the site indexes into sites');
  if (summary) {
    for (const g of fold.groups) {
      const s = summary.groups[g.id];
      assert.ok(s, 'summary knows ' + g.id);
      assert.equal(g.kind, s.kind);
      if (s.messages !== null) assert.equal(g.view.messages.length, s.messages, 'message count of ' + g.id.slice(0, 8));
    }
  }
  return w;
}

test('the fixture summary builds a synthetic fold that maps with the right counts',
  { skip: fs.existsSync(SUMMARY) ? false : 'not present: ' + SUMMARY },
  () => {
    const doc = JSON.parse(fs.readFileSync(SUMMARY, 'utf8'));
    const fault = summaryFault(doc);
    assert.equal(fault, '', 'archive-fixture.expected.json is not the summary shape: ' + fault);
    const fold = foldFromSummary(doc);
    const w = assertMapsFaithfully(fold, doc);
    const total = Object.values(doc.groups).reduce((n, g) => n + (g.messages || 0), 0);
    const mapped = w.threads.reduce((n, t) => n + t.posts.length, 0)
                 + w.convs.reduce((n, c) => n + c.msgs.length, 0);
    assert.equal(mapped, total, 'every message the summary counts is drawn somewhere');
    console.log('  summary: %d groups → %d threads, %d convs, %d events, %d messages',
      fold.groups.length, w.threads.length, w.convs.length, w.events.length, mapped);
  });

/* The core, stood up under node the way keyholder.js stands it up: the
   module's imports satisfied BY NAME from one small table, `__wbindgen_start`
   run once, the alloc → write → call → read ABI on top. */
async function loadCore() {
  const HOST = {
    pacific_fill_random: (at) => (ptr, len) => {
      globalThis.crypto.getRandomValues(new Uint8Array(at().exports.memory.buffer, ptr, len));
    },
    __wbindgen_init_externref_table: (at) => () => {
      const t = at().exports.__wbindgen_externrefs, off = t.grow(4);
      t.set(0, undefined);
      t.set(off + 0, undefined); t.set(off + 1, null);
      t.set(off + 2, true); t.set(off + 3, false);
    },
    date_now: () => () => Date.now()
  };
  const mod = await WebAssembly.compile(fs.readFileSync(WASM));
  let inst = null; const at = () => inst; const imports = {};
  for (const i of WebAssembly.Module.imports(mod)) {
    const make = HOST[i.name];
    if (!make) throw new Error('core_wasm imports ' + i.module + '.' + i.name + ', which this loader does not supply');
    (imports[i.module] = imports[i.module] || {})[i.name] = make(at);
  }
  inst = await WebAssembly.instantiate(mod, imports);
  const e = inst.exports;
  if (e.__wbindgen_start) e.__wbindgen_start();
  const call = (fn, bytes) => {
    const ptr = e.alloc(bytes.length);
    new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
    const n = fn(bytes.length);
    if (e.erred()) throw new Error(new TextDecoder().decode(new Uint8Array(e.memory.buffer, e.out_ptr(), e.out_len())));
    return new Uint8Array(e.memory.buffer, e.out_ptr(), n).slice();
  };
  return { e, call };
}

test('the fixture archive folds through the real core and maps to the summary\'s counts and digests',
  { skip: !fs.existsSync(SUMMARY) ? 'not present: ' + SUMMARY
        : !fs.existsSync(CBOR) ? 'not present: ' + CBOR
        : !fs.existsSync(WASM) ? 'not built: ' + WASM + ' (run app/web/build-wasm.sh)'
        : false },
  async (t) => {
    const { e, call } = await loadCore();
    if (typeof e.fold_archive !== 'function') {
      return t.skip('core_wasm_bg.wasm has no fold_archive — rebuild with app/web/build-wasm.sh');
    }
    const summary = JSON.parse(fs.readFileSync(SUMMARY, 'utf8'));
    const fold = JSON.parse(new TextDecoder().decode(call(e.fold_archive, fs.readFileSync(CBOR))));
    // THE PIN. This is the only place the real core's version is compared to the
    // one world.js reads, so it is what stops the two drifting again — a bump in
    // `pacific_core::archive::ARCHIVE_VERSION` fails HERE and names the constant
    // to change, instead of silently shutting the archive handover in the browser.
    assert.equal(fold.v, FOLD_V,
      'the core emits fold v' + fold.v + ' and world.js reads v' + FOLD_V +
      ' — update FOLD_V in keyholder/world.js');
    assert.equal(fold.exported_by, summary.exported_by);
    assert.equal(fold.display_name, summary.display_name);
    assert.equal(fold.peers.length, summary.peers);
    assert.equal(fold.groups.length + fold.rejected.length, Object.keys(summary.groups).length,
      'every archived group is either folded or rejected by name');
    for (const g of fold.groups) {
      assert.equal(g.digest, summary.groups[g.id].digest, 'fold digest of ' + g.id.slice(0, 8));
    }
    const w = assertMapsFaithfully(fold, summary);
    console.log('  real fold: %d groups (%d rejected) → %d threads, %d convs, %d events, %d people',
      fold.groups.length, fold.rejected.length, w.threads.length, w.convs.length,
      w.events.length, Object.keys(w.people).length);
  });

/* ═══ THE SITE'S KNOWLEDGE BASE AND THE SITE'S WALLET ═════════════════════════

   The two facets `group_view` emits off the site's own group. Before these keys
   crossed, the core folded a notebook and a treasury and this mapping returned
   `wallet: null` and no notes at all — a green test asserted the null, while
   16 green tests in `wallet.rs` asserted the four ops fold. Both were true and
   they were about the same four ops.

   The fold shapes below are `core-wasm`'s output verbatim: snake_case, integer
   minor units, hex identities. */

const NOTE_AT = Date.UTC(2026, 7, 12, 9, 12);    // 12 Aug 2026 09:12Z
const DEP_AT = Date.UTC(2026, 8, 3, 10, 0);      // 03 Sep
const SET_AT = Date.UTC(2026, 8, 6, 11, 0);      // 06 Sep
const PID = `${ME}:5`;

function siteFold() {
  return {
    v: FOLD_V,
    exported_by: ME,
    display_name: 'Ada',
    own_group: gid('6'),
    site_group: gid('8'),
    peers: [{ id: BOB, name: 'bob', status: 'connected', identity_group: gid('7') }],
    groups: [
      { id: gid('6'), kind: 'group', name: 'Ada', owner: ME, members: [ME], digest: key('0'),
        view: { display_name: 'Ada Lovelace', shape: 'individual', presence: 'onPlatform',
                roles: [], notebook: { notes: [], comments: [], reactions: [] },
                wallet: null, decisions: [], publication: null } },
      { id: gid('7'), kind: 'group', name: 'Bob', owner: BOB, members: [BOB], digest: key('0'),
        view: { display_name: 'Bob B', shape: 'individual', presence: 'onPlatform',
                roles: [], notebook: { notes: [], comments: [], reactions: [] },
                wallet: null, decisions: [], publication: null } },
      { id: gid('8'), kind: 'group', name: 'STOMA', owner: ME, members: [ME, BOB], digest: key('0'),
        view: {
          display_name: 'STOMA', shape: 'organisation', presence: 'onPlatform',
          roles: [[ME, 'Owner'], [BOB, 'Member']],
          notebook: {
            notes: [{ note: 'house-rules', title: 'House rules', text: 'Everyone cooks.',
                      at: NOTE_AT, created: NOTE_AT, source: '', by: ME, versions: 2 }],
            comments: [{ note: 'house-rules', comment: 'c1', text: 'and washes up',
                         at: NOTE_AT, by: BOB }],
            /* One real reaction and one CLEARED one — `note.rs` keeps the empty
               row so the clearing is itself a fact. The count is of reactions,
               so it is 1. */
            reactions: [{ note: 'house-rules', emoji: '\u{1F331}', at: NOTE_AT, by: BOB },
                        { note: 'house-rules', emoji: '', at: NOTE_AT, by: ME }]
          },
          wallet: {
            open: true, currency: 'krw', account: 'Triodos 2291', disclosure: 'full',
            cooloff_hours: 24, balance: 9000000,
            bands: [{ ceiling: 10000, rule: 'petty', quorum: 0 },
                    { ceiling: null, rule: 'consent', quorum: 7 }],
            deposits: [{ reference: 'FC-2026-114', amount: 9120000, at: DEP_AT,
                         source: 'Fundacion Cadiz', by: ME }],
            settlements: [{ reference: 'TR-88214', amount: 120000, at: SET_AT,
                            proposal: PID, memo: 'print run', by: ME }],
            counts: [{ by: ME, amount: 9000000, at: SET_AT }]
          },
          decisions: [{ id: PID, proposer: ME, gen: 5, payload: 'Imprenta Ruiz — print run',
                        rule: 'owner', outcome: 'passed', closed: true,
                        ballots: [{ by: ME, ballot: 'approve' }] }],
          publication: { slug: 'stoma', publisher: BOB }
        } }
    ]
  };
}

test('the site’s notebook becomes the interior’s notes', () => {
  const w = worldFromFold(siteFold(), ME, { now: NOW });
  assert.equal(w.notes.length, 1, 'one live note');
  const n = w.notes[0];
  assert.equal(n.id, 'house-rules');
  assert.equal(n.t, 'House rules');
  assert.equal(n.s, 'Everyone cooks.');
  assert.equal(n.by, w.me, 'the author is a handle, not a key');
  assert.equal(n.at, '12 Aug · 09:12', 'the stamp pacific.js’s keepNote writes');
  assert.equal(n.versions, 2, 'co-edited, without carrying every draft');
  assert.equal(n.comments, 1);
  assert.equal(n.react, 1, 'the cleared reaction is not a reaction');
});

test('the notes come off the SITE’s group, not the exporter’s own', () => {
  const f = siteFold();
  /* Ada's own record carries a note of her own. It is not the site's. */
  f.groups[0].view.notebook.notes = [{ note: 'private', title: 'Mine', text: 'x',
                                       at: NOTE_AT, created: NOTE_AT, source: '', by: ME, versions: 1 }];
  const w = worldFromFold(f, ME, { now: NOW });
  assert.deepEqual(w.notes.map((n) => n.id), ['house-rules']);
});

test('the site’s wallet crosses, in the units the log actually stores', () => {
  const w = worldFromFold(siteFold(), ME, { now: NOW });
  const W = w.wallet;
  assert.ok(W, 'a treasury was opened, so it is not null');
  assert.equal(W.ccy, 'KRW');
  assert.equal(W.sym, '₩');
  /* THE ONE THAT MATTERS. Won has no minor unit: 9120000 is ₩9,120,000, not
     ₩91,200.00. `money()` divided by 100 unconditionally, which is the same
     defect the egg’s website carries, and the divisor now rides with the
     currency instead of being assumed by the renderer. */
  assert.equal(W.minor, 1, 'KRW is zero-decimal');
  assert.equal(W.balance, 9000000, 'deposits less settlements, derived by the core');
  assert.equal(W.cooloff, 24);
  assert.equal(W.disclosure, 'full');
  assert.deepEqual(W.bands[0], { upto: 10000, rule: 'petty', quorum: 0, label: 'petty' });
  assert.equal(W.bands[1].upto, null, 'the top band is open');
  assert.equal(W.deposits[0].id, 'FC-2026-114');
  assert.equal(W.deposits[0].at, '03 Sep');
  assert.equal(W.deposits[0].by, w.me);
  assert.equal(W.settlements[0].what, 'print run');
  assert.equal(W.settlements[0].proposal, PID, 'the payment names its decision');
  assert.deepEqual(W.attested, { amount: 9000000, at: '06 Sep', by: w.me });
});

test('a euro-denominated wallet keeps the divisor the renderer assumed', () => {
  const f = siteFold();
  f.groups[2].view.wallet.currency = 'eur';
  const W = worldFromFold(f, ME, { now: NOW }).wallet;
  assert.equal(W.ccy, 'EUR');
  assert.equal(W.sym, '€');
  assert.equal(W.minor, 100);
});

test('the decisions cross with the money, so no payment is orphaned', () => {
  const W = worldFromFold(siteFold(), ME, { now: NOW }).wallet;
  assert.equal(W.proposals.length, 1);
  const p = W.proposals[0];
  assert.equal(p.id, PID);
  assert.equal(p.what, 'Imprenta Ruiz — print run');
  assert.equal(p.closed, true);
  assert.equal(p.outcome, 'passed');
  assert.deepEqual(p.ballots, { [W.settlements[0].by]: 1 }, 'ballots are keyed by handle');
  /* A settlement whose proposal resolves is NOT an unauthorised release. That is
     the whole reason the decisions are carried: divergence() looks its proposal
     up by id, and an empty list would have it accuse a named member. */
  assert.ok(W.proposals.some((q) => q.id === W.settlements[0].proposal));
  /* And the amount a decision authorised is genuinely not in the log — RATIFY
     carries an opaque payload — so the key is absent rather than zero. */
  assert.equal('amount' in p, false, 'an amount the log does not hold is not invented');
});

test('a wallet that was never opened stays null, and hides the Money tab', () => {
  const f = siteFold();
  f.groups[2].view.wallet = null;
  const w = worldFromFold(f, ME, { now: NOW });
  assert.equal(w.wallet, null);
  assert.equal(w.notes.length, 1, 'the notebook is unaffected');
});
