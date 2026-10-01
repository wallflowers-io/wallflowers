(async () => { // one async function: Safari's console takes no top-level await
/* ONE SIGN-IN (runbook § 3 and § 3b), in the console of https://app.wallflowers.io, signed in as
   the Site's owner, once ICD 2.1.0 is on door-01 and kenjin-01 (base.setPart's `choice`):
   § 3   the Arc as the Site's admitter, and the kiosk's claim key;
   rooms the three rooms by choice (P1, Ralph's names), each marked with its choice, the Arc its
         admitter, so a kiosk visitor lands in the room of the message they chose. Healing
         Resistance, and any room without a choice, is not touched: claim visitors never join it;
   face  the Site's face with no picture described in it (the "data URL, 46303 chars" text out);
   § 3b  the Arc on the Host, the mark set by host.setMedia, the face hydrated, the Face published.
   BUNDLES: five, each fetched fresh just before this runs (a key package is used once): the
   Site's, the Host's, then one per room in CHOICE_ROOMS' order, each from
     curl -s https://arc.wallflowers.io/v1/bundle
   or all five at once: app/door/hosting/one-signin/bundles.sh prints the finished BUNDLES line.
   The mark: chosen in the file picker it puts at the top left (png, jpeg or webp; Ralph's own
   picture, or Egregore's site/app/icon.png). Each step says what it did, and a failure stops it, naming the step. Run it again after a
   stop: a room already marked with its choice, the adds, the Site's admitter grant and the
   publish are skipped where a run before did them ("already"); the claim key, each room's grant
   (a room's view carries no roles), the face, the mark and the hydrate are applied again,
   harmless, one Delta each. A room of the same name made before 2.1.0 (one-signin-today.js,
   'three') is marked with its choice, not made again. A room made but refused its mark is
   said, and not made again until its forum is removed. */
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c';
const SLUG = 'egregore';
const NOTE = '';   // the Face's one line, if Ralph gives one
const BUNDLES = ['<the Site>', '<the Host>', '<Resources>', '<Skills, Time & Services>', '<Financial Support>'];
const KID = '8d9436e9fe3efd2b', KEY = '_i2yUcFtAhDzTnhSsRhx-W4sALQ8VzBbFNvqdSpP2Uw';
// [the claim's `c`, the room's name] (Ralph, 28 Sep: "Name them after the answers").
const CHOICE_ROOMS = [['resources', 'Resources'], ['skills', 'Skills, Time & Services'], ['financial', 'Financial Support']];

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
const CAP = 156000, b64 = u => (u.split(',')[1] || '').length;
const read = f => new Promise((res, rej) => { const r = new FileReader(); r.onload = () => res(r.result); r.onerror = rej; r.readAsDataURL(f); });
const smaller = (url, side) => new Promise(res => {
  const img = new Image();
  img.onload = () => {
    const k = Math.min(1, side / Math.max(img.naturalWidth, img.naturalHeight)), c = document.createElement('canvas');
    c.width = Math.round(img.naturalWidth * k); c.height = Math.round(img.naturalHeight * k);
    c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
    // Safari encodes no webp and answers PNG: kept within the cap, else drawn as JPEG.
    const w = c.toDataURL('image/webp', 0.85);
    res(!w.startsWith('data:image/png') || b64(w) <= CAP ? w : c.toDataURL('image/jpeg', 0.85));
  };
  img.src = url;
});

const g = await step('the graph', () => call('/v2/graph'));
const S = g.objects.find(o => o.id === SITE);
if (!S) throw new Error('this account holds no Site ' + SITE);
const HOST = S.view.parts.find(p => p.role === 'host').part;
if (BUNDLES.length !== 2 + CHOICE_ROOMS.length) throw new Error('BUNDLES needs ' + (2 + CHOICE_ROOMS.length) + ': the Site, the Host, and one per room');
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
  if (b64(url) <= CAP) break;
  url = await smaller(url, side);
}
const [, MIME, MARK] = /^data:(image\/(?:png|jpeg|webp));base64,(.*)$/.exec(url) || [];
if (!MARK || MARK.length > CAP) throw new Error('the mark: not png, jpeg or webp within 156,000 base64 bytes');

// § 3: the Arc as the Site's admitter; the kiosk's claim key.
N = await add('the Arc added to the Site', SITE, BUNDLES[0]);
if ((S.view.roles || []).some(r => r[0] === N && r[1] === 'admitter')) console.log('already: the Arc the Site\'s admitter');
else await step('the Arc the Site\'s admitter', () => call('/v2/apply', { object: SITE, op: 'base.setRole', args: { member: N, role: 'admitter' } }));
await step('the claim key', () => call('/v2/apply', { object: SITE, op: 'group.setClaimIssuer', args: { kid: KID, key: KEY } }));
// The rooms by choice: made and marked unless a run before did, then the Arc added and made
// admitter (a grant to a node not yet a member is refused). Made as the webapp makes a room:
// the mint, the Site's half (base.setPart, here with its choice), the room's (base.setParent).
const nameOf = id => (g.objects.find(o => o.id === id) || {}).name;
for (const [i, [choice, name]] of CHOICE_ROOMS.entries()) {
  let room = (S.view.parts.find(p => p.role === 'room' && p.choice === choice) || {}).part;
  // A room of that name made before 2.1.0 (one-signin-today.js, 'three') is marked, not made again.
  const unmarked = room ? null : (S.view.parts.find(p => p.role === 'room' && !p.choice && nameOf(p.part) === name) || {}).part;
  if (room) console.log('already: room ' + name + ', for ' + choice);
  else if (unmarked) {
    room = unmarked;
    await step('room ' + name + ' marked, for ' + choice, () => call('/v2/apply', { object: SITE, op: 'base.setPart', args: { part: room, role: 'room', choice, at: Date.now() } }));
  } else {
    const at = Date.now();
    const r = await step('room ' + name + ' made, for ' + choice, () => call('/v2/batch', { steps: [
      { do: 'mint', kind: 'forum', draft: { name } },
      { do: 'apply', object: SITE, op: 'base.setPart', args: { part: { $step: 0 }, role: 'room', choice, at } },
      { do: 'apply', object: { $step: 0 }, op: 'base.setParent', args: { parent: SITE, role: 'room', at } }] }));
    if (r.refused) throw new Error('room ' + name + ' made, for ' + choice + ': ' + r.refused.why + ' (made: ' + (r.made || []).join(', ') + ')');
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
console.log({ SITE, HOST, N, mark: MIME + ', ' + MARK.length + ' base64 bytes' });
})();
