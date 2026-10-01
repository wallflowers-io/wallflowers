import fs from 'node:fs'; import assert from 'node:assert/strict'; import { fileURLToPath } from 'node:url';
// The snippet beside this file, or one named on the command line.
const raw = fs.readFileSync(process.argv[2] || fileURLToPath(new URL('./hr-arc-history.js', import.meta.url)), 'utf8');
assert.ok(raw.includes("const BUNDLE = '<the Arc>';"), 'the BUNDLE line is the one the test replaces');
const src = raw.replace("const BUNDLE = '<the Arc>';", "const BUNDLE = 'b-arc';");
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c', HR = 'e'.repeat(64), R = 'r'.repeat(64), ARC = 'a'.repeat(64), ME = 'c'.repeat(64);
// Production's shape after rooms-unmarked.js: three rooms the Arc admits to; Healing Resistance
// a room with no choice, the Arc not in it. A room's view carries its roles (W-98 Rooms, R3).
const fresh = () => ({
  members: { [SITE]: [ME, ARC], [HR]: [ME], [R]: [ME, ARC] },
  roles: { [SITE]: [[ARC, 'admitter']], [HR]: [], [R]: [[ARC, 'admitter']] },
  names: { [HR]: 'Healing Resistance', [R]: 'Resources' },
  parts: [{ part: HR, role: 'room', at: 2 }, { part: R, role: 'room', at: 3 }] });
let world = fresh();
const REPORT = { object: HR, spine: 3, records: 2, posts: 40, left: 0, unsent: null, sealed: 90000, max: 150000 };
async function run({ stopAt, report = REPORT, grantTakes = true, roomRoles = true } = {}) {
  const calls = [], out = [];
  console.log = (...a) => out.push(a);
  globalThis.fetch = async (p, o) => {
    const b = o.body && JSON.parse(o.body); calls.push([p, b]);
    if (stopAt && calls.length === stopAt) return { ok: false, status: 503, text: async () => 'the relay is away' };
    const ok = (j) => ({ ok: true, json: async () => j });
    if (p === '/v2/graph') return ok({ objects: [
      { id: SITE, members: world.members[SITE], view: { roles: world.roles[SITE], parts: world.parts.map(x => ({ ...x })) } },
      ...[HR, R].map(id => ({ id, name: world.names[id], members: world.members[id], view: roomRoles ? { roles: world.roles[id] } : { messages: [] } }))] });
    if (p === '/v2/history') return ok({ ...report, dry: b.dry });
    if (p === '/v2/add') { world.members[b.object].push(ARC); return ok({ member: ARC }); }
    if (p === '/v2/apply' && b.op === 'base.setRole' && grantTakes) world.roles[b.object].push([b.args.member, b.args.role]);
    return ok({ delta: 'd' });
  };
  let err = null;
  try { await eval(src); } catch (e) { err = e; }
  return { calls, err, out };
}
const seq = (calls) => calls.map(([p, b]) => p + (b ? ' ' + (b.op || (p === '/v2/history' ? (b.dry ? 'dry' : 'send') : b.object)) : ''));
// ONE RUN, in order: the dry run first, the add, the history, the grant LAST (no claim reaches the
// room before its history), the graph read again. Only Healing Resistance is written to.
{
  const one = await run();
  assert.equal(one.err, null, String(one.err));
  assert.deepEqual(seq(one.calls), ['/v2/graph', '/v2/history dry', '/v2/add ' + HR, '/v2/history send', '/v2/apply base.setRole', '/v2/graph']);
  assert.ok(one.calls.filter(([, b]) => b).every(([, b]) => b.object === HR), 'only Healing Resistance written to');
  assert.deepEqual(one.calls.filter(([p]) => p !== '/v2/graph').map(([, b]) => b.bundle || b.args.member), ['b-arc', 'b-arc', 'b-arc', ARC], 'one bundle, the Arc\'s');
  assert.deepEqual(one.calls.find(([, b]) => b && b.op)[1].args, { member: ARC, role: 'admitter' });
  const said = one.out.at(-1)[0];
  assert.deepEqual([said.room, said.arc, said.spine, said.posts, said.sealed, said.max], [HR, ARC, 3, 40, 90000, 150000]);
}
// AGAIN: the add and the grant skipped; the history sealed again, harmless.
{
  const two = await run();
  assert.equal(two.err, null, String(two.err));
  assert.deepEqual(seq(two.calls), ['/v2/graph', '/v2/history dry', '/v2/history send', '/v2/graph']);
  assert.ok(two.out.some(a => a[0] === 'already: the Arc in Healing Resistance'));
  assert.ok(two.out.some(a => a[0] === 'already: the Arc Healing Resistance\'s admitter'));
}
// OVER ONE BLOB: the dry run stops it, and nothing is written.
{
  world = fresh();
  const over = await run({ report: { ...REPORT, unsent: 'its spine alone is 160000 characters sealed, over the 150000 one blob holds', sealed: 0 } });
  assert.match(String(over.err), /the dry run: its spine alone is 160000/);
  assert.ok(!over.calls.some(([p, b]) => p === '/v2/add' || p === '/v2/apply' || (b && b.dry === false)), 'nothing written');
}
// A STOP names its step, and a run after it finishes what is left.
{
  world = fresh();
  const one = await run({ stopAt: 4 });
  assert.match(String(one.err), /Healing Resistance's history sealed to the Arc/, 'a stop names its step');
  assert.deepEqual(world.roles[HR], [], 'no grant before the history');
  const two = await run();
  assert.equal(two.err, null, String(two.err));
  assert.deepEqual(seq(two.calls), ['/v2/graph', '/v2/history dry', '/v2/history send', '/v2/apply base.setRole', '/v2/graph']);
}
// A GRANT THAT DID NOT TAKE fails by name, never reports done.
{
  world = fresh();
  const r = await run({ grantTakes: false });
  assert.match(String(r.err), /the Arc is not Healing Resistance's admitter after its grant/);
}
// A DOOR WHOSE ROOM VIEWS CARRY NO ROLES (before W-98's Rooms; TEST, door-test at 46c268c2): the
// grant is written and said to be unread, never a failure after a good run; the summary still comes.
{
  world = fresh();
  const r = await run({ roomRoles: false });
  assert.equal(r.err, null, String(r.err));
  assert.deepEqual(seq(r.calls), ['/v2/graph', '/v2/history dry', '/v2/add ' + HR, '/v2/history send', '/v2/apply base.setRole', '/v2/graph']);
  assert.ok(r.out.some(a => a[0] === "the grant written; this Door shows no room's roles to read it back"));
  assert.equal(r.out.at(-1)[0].room, HR, 'the summary still comes');
}
// THE BUNDLE NOT FILLED IN: stop before anything.
{
  world = fresh();
  let err = null;
  globalThis.fetch = async () => { throw new Error('no call expected'); };
  try { await eval(raw); } catch (e) { err = e; }
  assert.match(String(err), /BUNDLE: the Arc's/);
}
// SAFARI: one classic script, no top-level await.
{
  const vm = await import('node:vm');
  assert.doesNotThrow(() => new vm.Script(src), 'a classic script: no top-level await');
}
process.stdout.write('stub: dry run, add, history, grant last, graph again; a re-run skips the add and the grant; a Door with no room roles: the grant written, unread, no failure; over one blob nothing written; a stop names its step and a re-run finishes; a grant that did not take fails by name; the bundle unfilled stops first; Safari: one classic script\n');
