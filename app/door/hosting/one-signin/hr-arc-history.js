(async () => { // one async function: Safari's console takes no top-level await
/* HEALING RESISTANCE, EVERY CLAIM INTO IT TOO (Ralph, 30 Sep: "Everyone joining (paid or unpaid)
   should be put into all groups, including healing resistance"), in the console of
   https://app.wallflowers.io, signed in as the Site's owner, once the Door serves POST /v2/history
   (R3). The Arc holds only rows it has seen, so the room's owner seals it the room's history
   before it admits anyone there:
     1. a dry run: the room's history against one relay blob; over it, stop, nothing written;
     2. the Arc added to the room (its bundle, fresh from `curl -s https://arc.wallflowers.io/v1/bundle`);
     3. the room's history sealed to the Arc;
     4. the Arc the room's admitter, last, so no claim reaches the room before its history does;
     5. the graph read again: the Arc in the room and its admitter, or the step named.
   Each step says what it did, and a failure stops it, naming the step. Run it again after a
   stop: the add and the grant are skipped where done ("already"); the history is sealed again,
   harmless (the Arc keeps each row once). */
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c';
const ROOM = 'Healing Resistance';
const BUNDLE = '<the Arc>';

const call = (p, b) => fetch(p, { method: b ? 'POST' : 'GET', credentials: 'include', headers: { 'content-type': 'application/json' }, body: b && JSON.stringify(b) })
  .then(r => r.ok ? r.json() : r.text().then(t => { throw new Error(p + ' ' + r.status + ' ' + t); }));
const step = async (what, f) => { try { const r = await f(); console.log('✓ ' + what, r ?? ''); return r; } catch (e) { throw new Error(what + ': ' + (e && e.message || e)); } };

if (BUNDLE.startsWith('<')) throw new Error('BUNDLE: the Arc\'s, from curl -s https://arc.wallflowers.io/v1/bundle');
const graph = async (what) => {
  const g = await step(what, () => call('/v2/graph'));
  const S = g.objects.find(o => o.id === SITE);
  if (!S) throw new Error('this account holds no Site ' + SITE);
  const room = S.view.parts.filter(p => p.role === 'room').map(p => g.objects.find(o => o.id === p.part)).find(o => o && o.name === ROOM);
  if (!room) throw new Error('no room named ' + ROOM + ' on the Site');
  return { S, room };
};
let { S, room } = await graph('the graph');
const N = ((S.view.roles || []).find(r => r[1] === 'admitter') || [])[0];
if (!N) throw new Error('the Site has no admitter: the Arc admits no one');
// A room's roles are in its view on a Door with W-98's Rooms; on an older Door they are unknown.
const shown = r => Array.isArray((r.view || {}).roles);
const admits = r => shown(r) && r.view.roles.some(x => x[0] === N && x[1] === 'admitter');

const dry = await step('the dry run: ' + ROOM + '\'s history against one relay blob', () => call('/v2/history', { object: room.id, bundle: BUNDLE, dry: true }));
if (dry.unsent) throw new Error('the dry run: ' + dry.unsent + '; nothing written');
if (dry.sealed > dry.max) throw new Error('the dry run: ' + dry.sealed + ' sealed, over the ' + dry.max + ' one blob holds; nothing written');

if ((room.members || []).includes(N)) console.log('already: the Arc in ' + ROOM);
else await step('the Arc added to ' + ROOM, () => call('/v2/add', { object: room.id, bundle: BUNDLE }));
const sent = await step(ROOM + '\'s history sealed to the Arc', () => call('/v2/history', { object: room.id, bundle: BUNDLE, dry: false }));
if (sent.unsent) throw new Error(ROOM + '\'s history sealed to the Arc: ' + sent.unsent);
if (admits(room)) console.log('already: the Arc ' + ROOM + '\'s admitter');
else await step('the Arc ' + ROOM + '\'s admitter', () => call('/v2/apply', { object: room.id, op: 'base.setRole', args: { member: N, role: 'admitter' } }));

({ S, room } = await graph('the graph, again'));
if (!(room.members || []).includes(N)) throw new Error('the Arc is not in ' + ROOM + ' after its add');
if (shown(room) && !admits(room)) throw new Error('the Arc is not ' + ROOM + '\'s admitter after its grant');
if (!shown(room)) console.log('the grant written; this Door shows no room\'s roles to read it back');
console.log({ room: room.id, arc: N, spine: sent.spine, records: sent.records, posts: sent.posts, left: sent.left, sealed: sent.sealed, max: sent.max });
})();
