(async () => { // one async function: Safari's console takes no top-level await
/* R3, THE FOUNDER'S ONE PASTE (Ralph, 30 Sep: "Everyone joining (paid or unpaid) should be put
   into all groups, including healing resistance"), in the console of https://app.wallflowers.io,
   signed in as the Site's owner, once R3's Arc and Door are live. Nothing to fill in: founder-r3.sh
   writes this file with the Arc's fresh bundle inlined (a bundle is used once, so one file per run).
     1. every room carrying a claim's choice set again with none (base.setPart, its `at` kept):
        every claim then joins every room the Arc admits to;
     2. Healing Resistance: the Arc's bundle (inlined; else fetched from the Arc); a dry run of the room's history
        against one relay blob (over it, nothing written); the Arc added; the history sealed to it;
        the Arc the room's admitter LAST, so no claim reaches the room before its history;
     3. the graph read again: no room marked, the Arc in Healing Resistance and its admitter.
   Each step says what it did, and a failure stops it, naming the step. Run it again after a stop:
   what is done is skipped ("already"); the history is sealed again, harmless. */
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c';
const ROOM = 'Healing Resistance';
const ARC_BUNDLE = 'https://arc.wallflowers.io/v1/bundle';
const BUNDLE_SET = '<the Arc>'; // founder-r3.sh writes the Arc's fresh bundle here

const call = (p, b) => fetch(p, { method: b ? 'POST' : 'GET', credentials: 'include', headers: { 'content-type': 'application/json' }, body: b && JSON.stringify(b) })
  .then(r => r.ok ? r.json() : r.text().then(t => { throw new Error(p + ' ' + r.status + ' ' + t); }));
const step = async (what, f) => { try { const r = await f(); console.log('✓ ' + what, r ?? ''); return r; } catch (e) { throw new Error(what + ': ' + (e && e.message || e)); } };

const graph = async (what) => {
  const g = await step(what, () => call('/v2/graph'));
  const S = g.objects.find(o => o.id === SITE);
  if (!S) throw new Error('this account holds no Site ' + SITE);
  return { g, S };
};
let { g, S } = await graph('the graph');
const N = ((S.view.roles || []).find(r => r[1] === 'admitter') || [])[0];
if (!N) throw new Error('the Site has no admitter: the Arc admits no one');
const nameOf = id => (g.objects.find(o => o.id === id) || {}).name || id.slice(0, 8);
const rooms = () => S.view.parts.filter(p => p.role === 'room');

// 1. Every claim into every room.
const marked = rooms().filter(p => p.choice);
if (!marked.length) console.log('already: no room carries a choice');
for (const p of marked) {
  await step('room ' + nameOf(p.part) + ' unmarked (was ' + p.choice + ')', () =>
    call('/v2/apply', { object: SITE, op: 'base.setPart', args: { part: p.part, role: 'room', at: p.at ?? Date.now() } }));
}

// 2. Healing Resistance: the room's history to the Arc, then the Arc its admitter.
const hr = () => rooms().map(p => g.objects.find(o => o.id === p.part)).find(o => o && o.name === ROOM);
let room = hr();
if (!room) throw new Error('no room named ' + ROOM + ' on the Site');
// A room's roles are in its view on a Door with W-98's Rooms; on an older Door they are unknown.
const shown = r => Array.isArray((r.view || {}).roles);
const admits = r => shown(r) && r.view.roles.some(x => x[0] === N && x[1] === 'admitter');
const BUNDLE = BUNDLE_SET !== '<the Arc>' ? (console.log('✓ the Arc\'s bundle, inlined'), BUNDLE_SET.trim())
  : await step('the Arc\'s bundle', () => fetch(ARC_BUNDLE, { cache: 'no-store' })
    .then(r => r.ok ? r.text() : Promise.reject(new Error(ARC_BUNDLE + ' ' + r.status))).then(t => t.trim()).then(t => t || Promise.reject(new Error('empty'))));
const dry = await step('the dry run: ' + ROOM + '\'s history against one relay blob', () => call('/v2/history', { object: room.id, bundle: BUNDLE, dry: true }));
if (dry.unsent) throw new Error('the dry run: ' + dry.unsent + '; nothing written for ' + ROOM);
if ((room.members || []).includes(N)) console.log('already: the Arc in ' + ROOM);
else await step('the Arc added to ' + ROOM, () => call('/v2/add', { object: room.id, bundle: BUNDLE }));
const sent = await step(ROOM + '\'s history sealed to the Arc', () => call('/v2/history', { object: room.id, bundle: BUNDLE, dry: false }));
if (sent.unsent) throw new Error(ROOM + '\'s history sealed to the Arc: ' + sent.unsent);
if (admits(room)) console.log('already: the Arc ' + ROOM + '\'s admitter');
else await step('the Arc ' + ROOM + '\'s admitter', () => call('/v2/apply', { object: room.id, op: 'base.setRole', args: { member: N, role: 'admitter' } }));

// 3. Read back.
({ g, S } = await graph('the graph, again'));
const left = rooms().filter(p => p.choice);
if (left.length) throw new Error('still marked: ' + left.map(p => nameOf(p.part) + ' (' + p.choice + ')').join(', '));
room = hr();
if (!(room.members || []).includes(N)) throw new Error('the Arc is not in ' + ROOM + ' after its add');
if (shown(room) && !admits(room)) throw new Error('the Arc is not ' + ROOM + '\'s admitter after its grant');
if (!shown(room)) console.log('the grant written; this Door shows no room\'s roles to read it back');
const holds = id => ((g.objects.find(o => o.id === id) || {}).members || []).includes(N);
console.log({ every_claim_joins: rooms().filter(p => holds(p.part)).map(p => nameOf(p.part)), history: { spine: sent.spine, records: sent.records, posts: sent.posts, left: sent.left, sealed: sent.sealed, max: sent.max } });
})();
