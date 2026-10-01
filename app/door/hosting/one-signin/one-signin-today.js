/* ONE SIGN-IN FOR ENTRY, BEFORE ICD 2.1.0 (Door door.1 1691bd4 or door.2 887d8ae0, Arc fd81568; runbook § 3 and § 3b), in the
   console of https://app.wallflowers.io, signed in as the Site's owner, after Ralph's
   ARC_CLAIM_KEYS line and the arc-node restart:
   § 3   the Arc as the Site's admitter, and the kiosk's claim key: a claim admits to the Site;
   rooms ROOMS below, Ralph's choice. 'none': no room is touched, and a visitor joins the Site
         only. 'three': the three rooms named after the answers are made, the Arc their admitter,
         and until ICD 2.1.0 EVERY visitor joins all three (today's Arc admits to every room it
         is admitter of, D-58); 2.1.0's snippet then marks each with its choice, not making them
         again. Healing Resistance, and every other existing room, is never touched either way;
   face  the Site's face with no picture described in it (the "data URL, 46303 chars" text out);
   § 3b  the Arc on the Host, the mark set by host.setMedia, the face hydrated, the Face published.
   BUNDLES: each fetched fresh just before this runs (a key package is used once), each from
     curl -s https://arc.wallflowers.io/v1/bundle
   two for 'none' (the Site's, the Host's); five for 'three' (then one per room, in ROOM_NAMES'
   order). The mark: chosen in the file picker it puts at the top left (png, jpeg or webp).
   Each step says what it did, and a failure stops it, naming the step. Run it again after a
   stop: a room made before, the adds, the Site's admitter grant and the publish are skipped
   ("already"); the claim key, each room's grant (a room's view carries no roles), the face, the
   mark and the hydrate are applied again, harmless, one Delta each. */
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c';
const SLUG = 'egregore';
const NOTE = '';   // the Face's one line, if Ralph gives one
const ROOMS = 'none';   // or 'three': Ralph's choice
const ROOM_NAMES = ['Resources', 'Skills, Time & Services', 'Financial Support'];
const BUNDLES = ['<the Site>', '<the Host>'];   // 'three': add '<Resources>', '<Skills, Time & Services>', '<Financial Support>'
const KID = '8d9436e9fe3efd2b', KEY = '_i2yUcFtAhDzTnhSsRhx-W4sALQ8VzBbFNvqdSpP2Uw';

const call = (p, b) => fetch(p, { method: b ? 'POST' : 'GET', credentials: 'include', headers: { 'content-type': 'application/json' }, body: b && JSON.stringify(b) })
  .then(r => r.ok ? r.json() : r.text().then(t => { throw new Error(p + ' ' + r.status + ' ' + t); }));
const step = async (what, f) => { try { const r = await f(); console.log('✓ ' + what, r ?? ''); return r; } catch (e) { throw new Error(what + ': ' + (e && e.message || e)); } };

// The mark, picked by hand. A picture over host.setMedia's 156,000 base64 bytes is drawn smaller.
const pick = () => new Promise(res => {
  const i = document.createElement('input');
  i.type = 'file'; i.accept = 'image/png,image/jpeg,image/webp';
  i.style = 'position:fixed;top:8px;left:8px;z-index:99999;background:#fff';
  i.onchange = () => { res(i.files[0]); i.remove(); };
  document.body.appendChild(i);
});
const read = f => new Promise((res, rej) => { const r = new FileReader(); r.onload = () => res(r.result); r.onerror = rej; r.readAsDataURL(f); });
const smaller = (url, side) => new Promise(res => {
  const img = new Image();
  img.onload = () => {
    const k = Math.min(1, side / Math.max(img.naturalWidth, img.naturalHeight)), c = document.createElement('canvas');
    c.width = Math.round(img.naturalWidth * k); c.height = Math.round(img.naturalHeight * k);
    c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
    res(c.toDataURL('image/webp', 0.85));
  };
  img.src = url;
});

const g = await step('the graph', () => call('/v2/graph'));
const S = g.objects.find(o => o.id === SITE);
if (!S) throw new Error('this account holds no Site ' + SITE);
const HOST = S.view.parts.find(p => p.role === 'host').part;
if (ROOMS !== 'none' && ROOMS !== 'three') throw new Error("ROOMS is 'none' or 'three'");
const WANT = ROOMS === 'three' ? ROOM_NAMES : [];
if (BUNDLES.length !== 2 + WANT.length) throw new Error('BUNDLES needs ' + (2 + WANT.length) + ': the Site, the Host' + (WANT.length ? ', and one per room' : ''));
const nameOf = id => (g.objects.find(o => o.id === id) || {}).name;
const holds = id => (g.objects.find(o => o.id === id) || {}).members || [];
// The Arc's key, if a run before this one made it the Site's admitter.
let N = ((S.view.roles || []).find(r => r[1] === 'admitter') || [])[0];
// Added unless it is already a member; its bundle is spent only by an add that runs.
const add = async (what, object, bundle) => {
  if (N && holds(object).includes(N)) { console.log('already: ' + what); return N; }
  return (await step(what, () => call('/v2/add', { object, bundle }))).member;
};

console.log('the mark: choose it in the file picker at the top left');
let url = await read(await pick());
for (const side of [1024, 768, 512, 384]) {
  if ((url.split(',')[1] || '').length <= 156000) break;
  url = await smaller(url, side);
}
const [, MIME, MARK] = /^data:(image\/(?:png|jpeg|webp));base64,(.*)$/.exec(url) || [];
if (!MARK || MARK.length > 156000) throw new Error('the mark: not png, jpeg or webp within 156,000 base64 bytes');

// § 3: the Arc as the Site's admitter; the kiosk's claim key.
N = await add('the Arc added to the Site', SITE, BUNDLES[0]);
if ((S.view.roles || []).some(r => r[0] === N && r[1] === 'admitter')) console.log('already: the Arc the Site\'s admitter');
else await step('the Arc the Site\'s admitter', () => call('/v2/apply', { object: SITE, op: 'base.setRole', args: { member: N, role: 'admitter' } }));
await step('the claim key', () => call('/v2/apply', { object: SITE, op: 'group.setClaimIssuer', args: { kid: KID, key: KEY } }));
// The rooms ('three' only): made as the webapp makes a room (the mint, the Site's half, the
// room's), unless a room of that name is already a part; then the Arc added and made admitter
// (a grant to a node not yet a member is refused). No other room is touched.
for (const [i, name] of WANT.entries()) {
  let room = (S.view.parts.find(p => p.role === 'room' && nameOf(p.part) === name) || {}).part;
  if (room) console.log('already: room ' + name);
  else {
    const at = Date.now();
    const r = await step('room ' + name + ' made', () => call('/v2/batch', { steps: [
      { do: 'mint', kind: 'forum', draft: { name } },
      { do: 'apply', object: SITE, op: 'base.setPart', args: { part: { $step: 0 }, role: 'room', at } },
      { do: 'apply', object: { $step: 0 }, op: 'base.setParent', args: { parent: SITE, role: 'room', at } }] }));
    if (r.refused) throw new Error('room ' + name + ' made: ' + r.refused.why + ' (made: ' + (r.made || []).join(', ') + ')');
    room = r.made[0];
  }
  await add('the Arc added to room ' + name, room, BUNDLES[2 + i]);
  await step('the Arc room ' + name + '\'s admitter', () => call('/v2/apply', { object: room, op: 'base.setRole', args: { member: N, role: 'admitter' } }));
}
// The face, with no picture described in it: the pictures are the Host's media. The view carries
// it parsed (fold.rs, "THE FACE, parsed rather than escaped"; null when none), a string read too.
const raw = S.view.face, face = typeof raw === 'string' ? JSON.parse(raw) : Object.assign({}, raw || {});
face.mark = '';
if (face.header) face.header = Object.assign({}, face.header, { logo: '', banner: '' });
await step('the Site\'s face, no picture described', () => call('/v2/apply', { object: SITE, op: 'group.setFace', args: { face: JSON.stringify(face) } }));
// § 3b: the Arc on the Host, the mark, the face, publish.
await add('the Arc added to the Host', HOST, BUNDLES[1]);
await step('the mark', () => call('/v2/apply', { object: HOST, op: 'host.setMedia', args: { slot: 'mark', media: MARK, mediaMime: MIME } }));
const B = JSON.stringify({ v: 1, profile: { displayName: S.view.display_name || S.name, card: { note: NOTE, urls: [] } }, face });
if (new TextEncoder().encode(B).length > 16384) throw new Error('the face is over 16,384 bytes');
await step('the Face hydrated', () => call('/v2/apply', { object: HOST, op: 'host.hydrate', args: { key: 'face', payload: B, fetchedAt: Date.now(), rev: Date.now() } }));
const pub = (g.objects.find(o => o.id === HOST) || {}).view?.publication;
if (pub && pub.slug === SLUG && pub.publisher === N) console.log('already: published at ' + SLUG);
else await step('published at ' + SLUG, () => call('/v2/apply', { object: HOST, op: 'base.publish', args: { slug: SLUG, publisher: N } }));
console.log({ SITE, HOST, ROOMS, N, mark: MIME + ', ' + MARK.length + ' base64 bytes' });
