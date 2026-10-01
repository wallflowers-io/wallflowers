import fs from 'node:fs'; import assert from 'node:assert/strict'; import { fileURLToPath } from 'node:url';
// The snippet beside this file, or one named on the command line.
const src = fs.readFileSync(process.argv[2] || fileURLToPath(new URL('./rooms-unmarked.js', import.meta.url)), 'utf8');
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c', HOST = 'h'.repeat(64), HR = 'e'.repeat(64), ARC = 'a'.repeat(64), ME = 'c'.repeat(64);
const [R, K, F] = ['r', 's', 'f'].map(c => c.repeat(64));
// Production's shape after one-signin-2.1.0.js: three rooms marked by choice, the Arc in each and the
// Site's admitter; Healing Resistance a room with no choice, the Arc not in it.
const fresh = () => ({
  members: { [SITE]: [ME, ARC], [HOST]: [ME, ARC], [HR]: [ME], [R]: [ME, ARC], [K]: [ME, ARC], [F]: [ME, ARC] },
  names: { [HR]: 'Healing Resistance', [R]: 'Resources', [K]: 'Skills, Time & Services', [F]: 'Financial Support' },
  parts: [{ part: HOST, role: 'host', at: 1 }, { part: HR, role: 'room', at: 2 },
    { part: R, role: 'room', at: 3, choice: 'resources' }, { part: K, role: 'room', at: 4, choice: 'skills' }, { part: F, role: 'room', at: 5, choice: 'financial' }],
  roles: [[ARC, 'admitter']] });
let world = fresh();
async function run(stopAt, refuse) {
  const calls = [], out = [];
  console.log = (...a) => out.push(a);
  globalThis.fetch = async (p, o) => {
    const b = o.body && JSON.parse(o.body); calls.push([p, b]);
    if (stopAt && calls.length === stopAt) return { ok: false, status: 503, text: async () => 'the relay is away' };
    const ok = (j) => ({ ok: true, json: async () => j });
    if (p === '/v2/graph') return ok({ objects: [
      { id: SITE, members: world.members[SITE], view: { roles: world.roles, parts: world.parts.map(x => ({ ...x })) } },
      ...Object.keys(world.members).filter(id => id !== SITE).map(id => ({ id, name: world.names[id], members: world.members[id], view: {} }))] });
    if (b.op === 'base.setPart' && b.object === SITE && !refuse) {
      const i = world.parts.findIndex(q => q.part === b.args.part);
      world.parts[i] = { part: b.args.part, role: b.args.role, at: b.args.at, ...(b.args.choice ? { choice: b.args.choice } : {}) };
    }
    return ok({ delta: 'd' });
  };
  let err = null;
  try { await eval(src); } catch (e) { err = e; }
  return { calls, err, out };
}
const touchesHR = (calls) => calls.some(([, b]) => b && (b.object === HR || (b.args && b.args.part === HR)));
// ONE RUN: each marked room set again as a room with no choice and its own `at`; nothing else
// written; Healing Resistance untouched; the graph read again and no room left marked.
{
  const one = await run();
  assert.equal(one.err, null, String(one.err));
  const writes = one.calls.filter(([p]) => p !== '/v2/graph');
  assert.deepEqual(writes.map(([p, b]) => [p, b.object, b.op, b.args]), [
    ['/v2/apply', SITE, 'base.setPart', { part: R, role: 'room', at: 3 }],
    ['/v2/apply', SITE, 'base.setPart', { part: K, role: 'room', at: 4 }],
    ['/v2/apply', SITE, 'base.setPart', { part: F, role: 'room', at: 5 }]], 'three unmarks, no choice, `at` kept');
  assert.deepEqual(one.calls.filter(([p]) => p === '/v2/graph').length, 2, 'the graph read again after');
  assert.ok(!touchesHR(one.calls), 'Healing Resistance untouched');
  assert.deepEqual(world.parts.filter(p => p.choice), [], 'no room marked');
  const said = one.out.at(-1)[0];
  assert.deepEqual(said.every_claim_joins, ['Resources', 'Skills, Time & Services', 'Financial Support']);
  assert.deepEqual(said.not_the_arcs, ['Healing Resistance'], "a room the Arc is not in is named");
}
// AGAIN: nothing to do, nothing written.
{
  const two = await run();
  assert.equal(two.err, null, String(two.err));
  assert.ok(!two.calls.some(([p]) => p === '/v2/apply'), 'no write repeats');
  assert.ok(two.out.some(a => a[0] === 'already: no room carries a choice'));
}
// A STOP names its step, and a run after it finishes what is left.
{
  world = fresh();
  const one = await run(3);
  assert.match(String(one.err), /room Skills, Time & Services unmarked \(was skills\)/, 'a stop names its step');
  const two = await run();
  assert.equal(two.err, null, String(two.err));
  assert.deepEqual(two.calls.filter(([p]) => p === '/v2/apply').map(([, b]) => b.args.part), [K, F], 'the rest, once');
  assert.deepEqual(world.parts.filter(p => p.choice), []);
}
// A WRITE THAT DID NOT TAKE (the graph still shows a mark) fails by name, never reports done.
{
  world = fresh();
  const r = await run(0, true);
  assert.match(String(r.err), /still marked: Resources \(resources\), Skills, Time & Services \(skills\), Financial Support \(financial\)/);
}
// NO ADMITTER: the Arc admits no one, so unmarking changes nothing; stop before writing.
{
  world = fresh(); world.roles = [];
  const r = await run();
  assert.match(String(r.err), /the Site has no admitter/);
  assert.ok(!r.calls.some(([p]) => p === '/v2/apply'));
}
// SAFARI: one classic script, no top-level await.
{
  const vm = await import('node:vm');
  assert.doesNotThrow(() => new vm.Script(src), 'a classic script: no top-level await');
}
process.stdout.write('stub: every marked room unmarked, `at` kept; Healing Resistance untouched and named as not the Arc\'s; a re-run writes nothing; a stop names its step; a write that did not take fails by name; no admitter, no write; Safari: one classic script\n');
