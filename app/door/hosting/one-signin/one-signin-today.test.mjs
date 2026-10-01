import fs from 'node:fs'; import assert from 'node:assert/strict'; import { fileURLToPath } from 'node:url';
// The snippet beside this file, or one named on the command line.
const raw = fs.readFileSync(process.argv[2] || fileURLToPath(new URL('./one-signin-today.js', import.meta.url)), 'utf8');
for (const line of ["const ROOMS = 'none';", "const BUNDLES = ['<the Site>', '<the Host>'];"]) assert.ok(raw.includes(line), 'the snippet has ' + line);
const variant = (rooms) => raw.replace("const ROOMS = 'none';", `const ROOMS = '${rooms}';`).replace(
  /const BUNDLES = \['<the Site>', '<the Host>'\];[^\n]*/,
  rooms === 'three' ? "const BUNDLES = ['b-site', 'b-host', 'b-r1', 'b-r2', 'b-r3'];" : "const BUNDLES = ['b-site', 'b-host'];");
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c', HOST = 'h'.repeat(64), HR = 'e'.repeat(64), ARC = 'a'.repeat(64), ME = 'c'.repeat(64);
const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
globalThis.document = { createElement: () => ({ style: '', remove() {} }), body: { appendChild(i) { setTimeout(() => { i.files = [{}]; i.onchange(); }, 0); } } };
globalThis.FileReader = class { readAsDataURL() { this.result = PNG; setTimeout(() => this.onload(), 0); } };
console.log = () => {};
function fresh() {
  return { members: { [SITE]: [ME], [HOST]: [ME], [HR]: [ME] }, names: { [SITE]: "Egregore's Echoes", [HOST]: "Egregore's Echoes", [HR]: 'Healing Resistance' },
    roles: [], pub: null, parts: [{ part: HOST, role: 'host' }, { part: HR, role: 'room' }], made: 0 };
}
async function run(src, world, stopAt) {
  const calls = [];
  globalThis.fetch = async (p, o) => {
    const b = o.body && JSON.parse(o.body); calls.push([p, b]);
    if (stopAt && calls.length === stopAt) return { ok: false, status: 503, text: async () => 'the relay is away' };
    const ok = (j) => ({ ok: true, json: async () => j });
    const face = { mark: 'data URL, 46303 chars', header: { logo: 'data URL, 9 chars', banner: '' } };
    if (p === '/v2/graph') return ok({ objects: Object.keys(world.members).map(id => ({ id, name: world.names[id], members: world.members[id],
      view: id === SITE ? { roles: world.roles, parts: world.parts.map(x => ({ ...x })), face } : id === HOST ? { publication: world.pub } : {} })) });
    if (p === '/v2/add') { world.members[b.object].push(ARC); return ok({ member: ARC }); }
    if (p === '/v2/batch') {
      const [mint, part] = b.steps;
      assert.equal(part.args.choice, undefined, "today's build: no choice arg");
      const id = String(++world.made).repeat(64);
      world.members[id] = [ME]; world.names[id] = mint.draft.name; world.parts.push({ part: id, role: 'room' });
      return ok({ made: [id, 'd1', 'd2'] });
    }
    if (b.op === 'base.setRole' && b.object === SITE) world.roles.push([b.args.member, b.args.role]);
    if (b.op === 'base.publish') world.pub = { slug: b.args.slug, publisher: b.args.publisher };
    return ok({ delta: 'd' });
  };
  let err = null;
  try { await eval('(async () => {' + src + '})()'); } catch (e) { err = e; }
  return { calls, err };
}
const ops = (calls) => calls.map(([p, b]) => p + ' ' + (b ? (b.op || (b.steps ? b.steps[0].draft.name : b.object)) : ''));
const untouched = (calls) => !calls.some(([, b]) => b && (b.object === HR || (b.args && b.args.part === HR)));

// 'none', the default: the Site's admitter and the claim key; no room touched.
{
  const w = fresh(); const r = await run(variant('none'), w);
  assert.equal(r.err, null, String(r.err));
  assert.deepEqual(ops(r.calls), ['/v2/graph ', '/v2/add ' + SITE, '/v2/apply base.setRole', '/v2/apply group.setClaimIssuer',
    '/v2/apply group.setFace', '/v2/add ' + HOST, '/v2/apply host.setMedia', '/v2/apply host.hydrate', '/v2/apply base.publish']);
  assert.ok(untouched(r.calls), 'none: Healing Resistance untouched');
  assert.deepEqual(w.members[HR], [ME]);
  const again = await run(variant('none'), w);
  assert.equal(again.err, null, String(again.err));
  assert.ok(!again.calls.some(([p]) => p === '/v2/add'), 'none: a re-run adds nothing');
}
// 'three': the three rooms made once, the Arc their admitter; a stop names its step; a re-run skips.
{
  const w = fresh();
  const one = await run(variant('three'), w, 8);   // stops at the second room's making
  assert.match(String(one.err), /room Skills, Time & Services made/);
  const two = await run(variant('three'), w);
  assert.equal(two.err, null, String(two.err));
  assert.deepEqual(ops(two.calls), ['/v2/graph ', '/v2/apply group.setClaimIssuer',
    '/v2/apply base.setRole',
    '/v2/batch Skills, Time & Services', '/v2/add ' + '2'.repeat(64), '/v2/apply base.setRole',
    '/v2/batch Financial Support', '/v2/add ' + '3'.repeat(64), '/v2/apply base.setRole',
    '/v2/apply group.setFace', '/v2/add ' + HOST, '/v2/apply host.setMedia', '/v2/apply host.hydrate', '/v2/apply base.publish']);
  assert.deepEqual(w.parts.filter(p => p.part !== HOST && p.part !== HR).map(p => w.names[p.part]), ['Resources', 'Skills, Time & Services', 'Financial Support'], 'three rooms, once each');
  const three = await run(variant('three'), w);
  assert.equal(three.err, null, String(three.err));
  assert.ok(!three.calls.some(([p]) => p === '/v2/add' || p === '/v2/batch'), 'three: a re-run makes and adds nothing');
  assert.ok(untouched([...one.calls, ...two.calls, ...three.calls]), 'three: Healing Resistance untouched');
  assert.deepEqual(w.members[HR], [ME], 'the Arc is not in Healing Resistance');
}
process.stdout.write("stub (today's build): 'none' touches no room; 'three' makes the three once, the Arc their admitter; Healing Resistance untouched; stops name their step; re-runs skip\n");
