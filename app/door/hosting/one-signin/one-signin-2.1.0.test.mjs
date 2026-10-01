import fs from 'node:fs'; import assert from 'node:assert/strict'; import { fileURLToPath } from 'node:url';
// The snippet beside this file, or one named on the command line.
const src = fs.readFileSync(process.argv[2] || fileURLToPath(new URL('./one-signin-2.1.0.js', import.meta.url)), 'utf8').replace(
  "const BUNDLES = ['<the Site>', '<the Host>', '<Resources>', '<Skills, Time & Services>', '<Financial Support>'];",
  "const BUNDLES = ['b-site', 'b-host', 'b-resources', 'b-skills', 'b-financial'];");
assert.ok(src.includes("'b-resources'"), 'the BUNDLES line is the one the test replaces');
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c', HOST = 'h'.repeat(64), HR = 'e'.repeat(64), ARC = 'a'.repeat(64), ME = 'c'.repeat(64);
const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
globalThis.document = { createElement: () => ({ style: '', remove() {} }), body: { appendChild(i) { setTimeout(() => { i.files = [{}]; i.onchange(); }, 0); } } };
let picked = PNG;
globalThis.FileReader = class { readAsDataURL() { this.result = picked; setTimeout(() => this.onload(), 0); } };
// A Door that remembers: members, the Site's parts and roles, the Host's publication.
// Healing Resistance is a room with no choice, and the Arc is not in it.
let world = { members: { [SITE]: [ME], [HOST]: [ME], [HR]: [ME] }, roles: [], pub: null, names: { [HR]: 'Healing Resistance' },
  parts: [{ part: HOST, role: 'host' }, { part: HR, role: 'room' }] };
async function run(faceAsString, stopAt) {
  const calls = [];
  globalThis.fetch = async (p, o) => {
    const b = o.body && JSON.parse(o.body); calls.push([p, b]);
    if (stopAt && calls.length === stopAt) return { ok: false, status: 503, text: async () => 'the relay is away' };
    const ok = (j) => ({ ok: true, json: async () => j });
    const face = { look: { colours: { background: '#FAF8F3' } }, mark: 'data URL, 46303 chars', header: { layout: 'hero', title: 'text', logo: 'data URL, 9 chars', banner: '' } };
    if (p === '/v2/graph') return ok({ objects: [
      { id: SITE, members: world.members[SITE], view: { display_name: "Egregore's Echoes", roles: world.roles, parts: world.parts.map(x => ({ ...x })), face: faceAsString ? JSON.stringify(face) : face } },
      { id: HOST, members: world.members[HOST], view: { publication: world.pub } },
      ...Object.keys(world.members).filter(id => id !== SITE && id !== HOST).map(id => ({ id, name: (world.names || {})[id], members: world.members[id], view: {} }))] });
    if (p === '/v2/add') { world.members[b.object].push(ARC); return ok({ member: ARC }); }
    if (p === '/v2/batch') {
      const [mint, part, parent] = b.steps;
      assert.deepEqual([mint.do, mint.kind, part.op, part.object, part.args.role, parent.op, parent.args.parent, parent.args.role],
        ['mint', 'forum', 'base.setPart', SITE, 'room', 'base.setParent', SITE, 'room'], 'a room made as the webapp makes one');
      const id = part.args.choice[0].repeat(64);
      world.members[id] = [ME]; world.parts.push({ part: id, role: 'room', choice: part.args.choice });
      return ok({ made: [id, 'd1', 'd2'] });
    }
    if (b.op === 'base.setPart' && b.object === SITE) { const x = world.parts.find(q => q.part === b.args.part); x.choice = b.args.choice; }
    if (b.op === 'base.setRole' && b.object === SITE) world.roles.push([b.args.member, b.args.role]);
    if (b.op === 'base.publish') world.pub = { slug: b.args.slug, publisher: b.args.publisher };
    return ok({ delta: 'd' });
  };
  let err = null;
  try { await eval(src); } catch (e) { err = e; }
  return { calls, err };
}
console.log = () => {};
const R = 'r'.repeat(64), S = 's'.repeat(64), F = 'f'.repeat(64);
// bundles.sh, against a stand-in on loopback (never production's Arc): five distinct bundles,
// one of them holding a quote and a backslash, come out as one BUNDLES line the snippet takes;
// a failing, empty or repeated bundle stops it.
{
  const { execFile } = await import('node:child_process');
  const run_ = (env) => new Promise(res => execFile('bash', [script], { env }, (e, out, err) => res(e ? { out: '', err: String(err) } : { out, err: null })));
  const http = await import('node:http');
  const script = fileURLToPath(new URL('./bundles.sh', import.meta.url));
  const serve = (answers) => new Promise(res => {
    let i = 0;
    const s = http.createServer((q, r) => { const [code, body] = answers[Math.min(i++, answers.length - 1)]; r.writeHead(code, { 'content-type': 'text/plain' }); r.end(body); });
    s.listen(0, '127.0.0.1', () => res(s));
  });
  const bundlesFrom = async (answers) => {
    const s = await serve(answers);
    try {
      return await run_({ ...process.env, ARC_BUNDLE_URL: `http://127.0.0.1:${s.address().port}/v1/bundle` });
    } finally { s.close(); }
  };
  const five = ['b1-site', 'b2-host', 'b3 "quoted" \\ back', 'b4-skills', 'b5-financial'];
  const ok = await bundlesFrom(five.map(b => [200, b]));
  assert.equal(ok.err, null, ok.err);
  assert.match(ok.out, /^const BUNDLES = \[.*\];\n$/, 'one line');
  assert.deepEqual(new Function(ok.out + 'return BUNDLES;')(), five, 'quoted as the Arc sent them');
  const line = fs.readFileSync(fileURLToPath(new URL('./one-signin-2.1.0.js', import.meta.url)), 'utf8').split('\n').find(l => l.startsWith('const BUNDLES = '));
  assert.ok(line && new Function(ok.out + 'return BUNDLES.length')() === 5, "the snippet's BUNDLES line is the one it replaces, five long");
  assert.match((await bundlesFrom([[200, 'a'], [500, 'down']])).err, /bundle 2 of 5: .* did not answer 200/);
  assert.match((await bundlesFrom([[200, 'a'], [200, '']])).err, /bundle 2 of 5 is empty/);
  assert.match((await bundlesFrom([[200, 'a'], [200, 'b'], [200, 'a']])).err, /bundle 3 of 5 repeats an earlier one/);
}
// FROM SCRATCH, ONE RUN (Ralph's morning, the snippet for entry never having run): the Arc the
// Site's admitter, the claim key, the three rooms made and marked, the Arc their admitter, the
// face, the Host, the mark, the Face published; in that order, each once, Healing Resistance untouched.
{
  const fresh = await run(false);
  assert.equal(fresh.err, null, String(fresh.err));
  const seq = fresh.calls.map(([p, b]) => p + ' ' + (b ? (b.op || (b.steps ? b.steps[1].args.choice : b.object)) : ''));
  assert.deepEqual(seq, ['/v2/graph ', '/v2/add ' + SITE, '/v2/apply base.setRole', '/v2/apply group.setClaimIssuer',
    '/v2/batch resources', '/v2/add ' + R, '/v2/apply base.setRole',
    '/v2/batch skills', '/v2/add ' + S, '/v2/apply base.setRole',
    '/v2/batch financial', '/v2/add ' + F, '/v2/apply base.setRole',
    '/v2/apply group.setFace', '/v2/add ' + HOST, '/v2/apply host.setMedia', '/v2/apply host.hydrate', '/v2/apply base.publish'], 'one run does it all');
  assert.deepEqual(world.parts.filter(p => p.choice).map(p => [p.choice, p.part]), [['resources', R], ['skills', S], ['financial', F]]);
  assert.deepEqual(world.roles, [[ARC, 'admitter']], "the Arc the Site's admitter");
  assert.deepEqual(fresh.calls.filter(([, b]) => b && b.op === 'group.setClaimIssuer').map(([, b]) => b.args.kid), ['8d9436e9fe3efd2b'], 'the kiosk key');
  assert.ok(!fresh.calls.some(([, b]) => b && (b.object === HR || (b.args && b.args.part === HR))), 'Healing Resistance untouched');
  assert.deepEqual(world.pub, { slug: 'egregore', publisher: ARC });
  // Back to the start for the stop-and-resume runs below.
  world = { members: { [SITE]: [ME], [HOST]: [ME], [HR]: [ME] }, roles: [], pub: null, names: { [HR]: 'Healing Resistance' },
    parts: [{ part: HOST, role: 'host' }, { part: HR, role: 'room' }] };
}
// First run stops at the second room's making (the 8th call): the Site and Resources done.
const one = await run(false, 8);
assert.match(String(one.err), /room Skills, Time & Services made, for skills/, 'a stop names its step');
// Second run, the face now as a string: what was done is skipped, the rest done.
const two = await run(true);
assert.equal(two.err, null, String(two.err));
const seq = two.calls.map(([p, b]) => p + ' ' + (b ? (b.op || (b.steps ? b.steps[1].args.choice : b.object)) : ''));
assert.deepEqual(seq, ['/v2/graph ', '/v2/apply group.setClaimIssuer',
  '/v2/apply base.setRole',                                               // Resources: already made, already a member; the grant again
  '/v2/batch skills', '/v2/add ' + S, '/v2/apply base.setRole',
  '/v2/batch financial', '/v2/add ' + F, '/v2/apply base.setRole',
  '/v2/apply group.setFace', '/v2/add ' + HOST, '/v2/apply host.setMedia', '/v2/apply host.hydrate', '/v2/apply base.publish']);
assert.deepEqual([two.calls[4][1].bundle, two.calls[7][1].bundle, two.calls[10][1].bundle], ['b-skills', 'b-financial', 'b-host'], 'each room its own bundle');
const face = JSON.parse(two.calls[9][1].args.face);
assert.deepEqual([face.mark, face.header.logo, face.look.colours.background], ['', '', '#FAF8F3'], 'a string face is read, and cleaned');
assert.deepEqual(world.parts.filter(p => p.choice).map(p => [p.choice, p.part]), [['resources', R], ['skills', S], ['financial', F]], 'three rooms, each marked once');
// Third run: everything that can be skipped is.
const three = await run(false);
assert.equal(three.err, null, String(three.err));
assert.ok(!three.calls.some(([p]) => p === '/v2/add' || p === '/v2/batch'), 'no add or room repeats');
assert.ok(!three.calls.some(([, b]) => b && b.op === 'base.publish'), 'no publish repeats');
// Healing Resistance is never touched: no add, no grant, no mark.
const all = [one.calls, two.calls, three.calls].flat();
assert.ok(!all.some(([, b]) => b && (b.object === HR || (b.args && b.args.part === HR))), 'Healing Resistance untouched');
assert.deepEqual(world.members[HR], [ME], 'the Arc is not in Healing Resistance');
assert.ok(!JSON.stringify(all).includes('data URL,'), 'no description stored');
// Rooms made before 2.1.0 by one-signin-today.js ('three', no choice, the Arc already in them)
// are marked with their choice by name: no room made again, no add, Healing Resistance untouched.
{
  const T = ['7', '8', '9'].map(c => c.repeat(64));
  world = { members: { [SITE]: [ME, ARC], [HOST]: [ME, ARC], [HR]: [ME], [T[0]]: [ME, ARC], [T[1]]: [ME, ARC], [T[2]]: [ME, ARC] },
    roles: [[ARC, 'admitter']], pub: { slug: 'egregore', publisher: ARC },
    names: { [HR]: 'Healing Resistance', [T[0]]: 'Resources', [T[1]]: 'Skills, Time & Services', [T[2]]: 'Financial Support' },
    parts: [{ part: HOST, role: 'host' }, { part: HR, role: 'room' }, ...T.map(part => ({ part, role: 'room' }))] };
  const four = await run(false);
  assert.equal(four.err, null, String(four.err));
  assert.ok(!four.calls.some(([p]) => p === '/v2/batch' || p === '/v2/add'), 'nothing made or added again');
  const marks = four.calls.filter(([, b]) => b && b.op === 'base.setPart').map(([, b]) => [b.args.part, b.args.role, b.args.choice]);
  assert.deepEqual(marks, [[T[0], 'room', 'resources'], [T[1], 'room', 'skills'], [T[2], 'room', 'financial']], 'each marked by name');
  assert.ok(!four.calls.some(([, b]) => b && (b.object === HR || (b.args && b.args.part === HR))), 'Healing Resistance untouched');
}
// SAFARI (Ralph's run, 29 Sep): its console takes no top-level await, and its canvas has no webp
// encoder, so toDataURL('image/webp') answers PNG. The snippet is one classic script; a picture whose
// PNG stays over the cap is drawn as JPEG, and one within it stays PNG.
{
  const vm = await import('node:vm');
  assert.doesNotThrow(() => new vm.Script(src), 'a classic script: no top-level await');
  const BIG = 'A'.repeat(200000);
  picked = 'data:image/png;base64,' + BIG;
  globalThis.Image = class { set src(u) { this.naturalWidth = 1230; this.naturalHeight = 1186; setTimeout(() => this.onload(), 0); } };
  const safari = (png) => (tag) => tag === 'canvas'
    ? { getContext: () => ({ drawImage() {} }), toDataURL: t => t === 'image/jpeg' ? 'data:image/jpeg;base64,/9j/' : 'data:image/png;base64,' + png }
    : { style: '', remove() {} };
  const markOf = (r) => { assert.equal(r.err, null, String(r.err)); const a = r.calls.find(([, b]) => b && b.op === 'host.setMedia')[1].args; return [a.mediaMime, a.media]; };
  document.createElement = safari(BIG);
  assert.deepEqual(markOf(await run(false)), ['image/jpeg', '/9j/'], 'a PNG over the cap, drawn as JPEG');
  document.createElement = safari('iVBO');
  assert.deepEqual(markOf(await run(false)), ['image/png', 'iVBO'], 'a PNG within the cap stays PNG');
}
process.stdout.write('stub: rooms made before 2.1.0 are marked, not made again; three rooms by choice, each once; Healing Resistance untouched; a stop names its step; a re-run skips what is done; no description stored; Safari: one classic script, a PNG over the cap drawn as JPEG\n');
