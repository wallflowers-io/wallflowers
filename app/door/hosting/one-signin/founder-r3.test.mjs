import fs from 'node:fs'; import assert from 'node:assert/strict'; import { fileURLToPath } from 'node:url';
// The snippet beside this file, or one named on the command line.
const src = fs.readFileSync(process.argv[2] || fileURLToPath(new URL('./founder-r3.js', import.meta.url)), 'utf8');
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c', HOST = 'h'.repeat(64), HR = 'e'.repeat(64), ARC = 'a'.repeat(64), ME = 'c'.repeat(64);
const [R, K, F] = ['r', 's', 'f'].map(c => c.repeat(64));
const ARC_BUNDLE = 'https://arc.wallflowers.io/v1/bundle';
// Production's shape after one-signin-2.1.0.js: three rooms marked by choice, the Arc in each and the
// Site's admitter; Healing Resistance a room with no choice, the Arc not in it.
const fresh = () => ({
  members: { [SITE]: [ME, ARC], [HOST]: [ME, ARC], [HR]: [ME], [R]: [ME, ARC], [K]: [ME, ARC], [F]: [ME, ARC] },
  names: { [HR]: 'Healing Resistance', [R]: 'Resources', [K]: 'Skills, Time & Services', [F]: 'Financial Support' },
  parts: [{ part: HOST, role: 'host', at: 1 }, { part: HR, role: 'room', at: 2 },
    { part: R, role: 'room', at: 3, choice: 'resources' }, { part: K, role: 'room', at: 4, choice: 'skills' }, { part: F, role: 'room', at: 5, choice: 'financial' }],
  siteRoles: [[ARC, 'admitter']], roomRoles: { [HR]: [], [R]: [[ARC, 'admitter']], [K]: [[ARC, 'admitter']], [F]: [[ARC, 'admitter']] } });
let world = fresh();
const REPORT = { spine: 3, records: 2, posts: 40, left: 0, unsent: null, sealed: 90000, max: 150000 };
async function run({ report = REPORT, grantTakes = true, roomRoles = true, bundle = [200, 'b-arc\n'], stopAt } = {}) {
  const calls = [], out = [];
  console.log = (...a) => out.push(a);
  globalThis.fetch = async (p, o = {}) => {
    const b = o.body && JSON.parse(o.body); calls.push([p, b]);
    if (stopAt && calls.length === stopAt) return { ok: false, status: 503, text: async () => 'the relay is away' };
    if (p === ARC_BUNDLE) return { ok: bundle[0] === 200, status: bundle[0], text: async () => bundle[1] };
    const ok = (j) => ({ ok: true, json: async () => j });
    if (p === '/v2/graph') return ok({ objects: [
      { id: SITE, members: world.members[SITE], view: { roles: world.siteRoles, parts: world.parts.map(x => ({ ...x })) } },
      ...[HR, R, K, F].map(id => ({ id, name: world.names[id], members: world.members[id], view: roomRoles ? { roles: world.roomRoles[id] } : { messages: [] } }))] });
    if (p === '/v2/history') return ok({ ...report, dry: b.dry });
    if (p === '/v2/add') { world.members[b.object].push(ARC); return ok({ member: ARC }); }
    if (b && b.op === 'base.setPart') {
      const i = world.parts.findIndex(q => q.part === b.args.part);
      world.parts[i] = { part: b.args.part, role: b.args.role, at: b.args.at, ...(b.args.choice ? { choice: b.args.choice } : {}) };
    }
    if (b && b.op === 'base.setRole' && grantTakes) world.roomRoles[b.object].push([b.args.member, b.args.role]);
    return ok({ delta: 'd' });
  };
  let err = null;
  try { await eval(src); } catch (e) { err = e; }
  return { calls, err, out };
}
const seq = (calls) => calls.map(([p, b]) => p === ARC_BUNDLE ? 'arc bundle' : p + (b ? ' ' + (b.op ? b.op + ' ' + (b.args.part || b.object).slice(0, 1) : p === '/v2/history' ? (b.dry ? 'dry' : 'send') : b.object.slice(0, 1)) : ''));
// ONE RUN, IN ORDER: the three rooms unmarked (their `at` kept), the Arc's bundle fetched (trimmed),
// the dry run, the add, the history, the grant LAST, the graph again. Nothing to fill in.
{
  const one = await run();
  assert.equal(one.err, null, String(one.err));
  assert.deepEqual(seq(one.calls), ['/v2/graph', '/v2/apply base.setPart r', '/v2/apply base.setPart s', '/v2/apply base.setPart f',
    'arc bundle', '/v2/history dry', '/v2/add e', '/v2/history send', '/v2/apply base.setRole e', '/v2/graph']);
  assert.deepEqual(one.calls.filter(([, b]) => b && b.op === 'base.setPart').map(([, b]) => b.args), [
    { part: R, role: 'room', at: 3 }, { part: K, role: 'room', at: 4 }, { part: F, role: 'room', at: 5 }], 'no choice, `at` kept');
  assert.deepEqual(one.calls.filter(([p, b]) => b && b.bundle).map(([, b]) => b.bundle), ['b-arc', 'b-arc', 'b-arc'], "the Arc's bundle, trimmed");
  assert.deepEqual(world.parts.filter(p => p.choice), []);
  const said = one.out.at(-1)[0];
  assert.deepEqual(said.every_claim_joins, ['Healing Resistance', 'Resources', 'Skills, Time & Services', 'Financial Support']);
  assert.equal(said.history.posts, 40);
}
// AGAIN: unmarks, the add and the grant skipped; the history sealed again, harmless.
{
  const two = await run();
  assert.equal(two.err, null, String(two.err));
  assert.deepEqual(seq(two.calls), ['/v2/graph', 'arc bundle', '/v2/history dry', '/v2/history send', '/v2/graph']);
}
// THE ARC'S BUNDLE UNREACHABLE: the rooms are unmarked, and nothing is written to Healing Resistance.
{
  world = fresh();
  const r = await run({ bundle: [502, 'bad gateway'] });
  assert.match(String(r.err), /the Arc's bundle: .*502/);
  assert.ok(!r.calls.some(([p, b]) => p === '/v2/add' || p === '/v2/history' || (b && b.op === 'base.setRole')), 'nothing for Healing Resistance');
  assert.deepEqual(world.parts.filter(p => p.choice), [], 'the rooms are unmarked already');
}
// OVER ONE BLOB: the dry run stops it, and nothing is written to Healing Resistance.
{
  world = fresh();
  const r = await run({ report: { ...REPORT, unsent: 'its spine alone is 160000 characters sealed, over the 150000 one blob holds', sealed: 0 } });
  assert.match(String(r.err), /the dry run: its spine alone is 160000/);
  assert.ok(!r.calls.some(([p, b]) => p === '/v2/add' || (b && b.dry === false) || (b && b.op === 'base.setRole')));
}
// A STOP names its step; a re-run finishes what is left.
{
  world = fresh();
  const one = await run({ stopAt: 8 });
  assert.match(String(one.err), /Healing Resistance's history sealed to the Arc/);
  assert.deepEqual(world.roomRoles[HR], [], 'no grant before the history');
  const two = await run();
  assert.equal(two.err, null, String(two.err));
  assert.deepEqual(seq(two.calls), ['/v2/graph', 'arc bundle', '/v2/history dry', '/v2/history send', '/v2/apply base.setRole e', '/v2/graph']);
}
// A GRANT THAT DID NOT TAKE fails by name.
{
  world = fresh();
  const r = await run({ grantTakes: false });
  assert.match(String(r.err), /the Arc is not Healing Resistance's admitter after its grant/);
}
// A DOOR WHOSE ROOM VIEWS CARRY NO ROLES: the grant written and said unread, never a failure.
{
  world = fresh();
  const r = await run({ roomRoles: false });
  assert.equal(r.err, null, String(r.err));
  assert.ok(r.out.some(a => a[0] === "the grant written; this Door shows no room's roles to read it back"));
}
// NO ADMITTER: stop before anything.
{
  world = fresh(); world.siteRoles = [];
  const r = await run();
  assert.match(String(r.err), /the Site has no admitter/);
  assert.ok(!r.calls.some(([p]) => p !== '/v2/graph'));
}
// THE ARC'S BUNDLE INLINED (founder-r3.sh's file): used as written, and the Arc is never asked.
{
  world = fresh();
  const inlined = src.replace("const BUNDLE_SET = '<the Arc>';", 'const BUNDLE_SET = "b-inline";');
  assert.notEqual(inlined, src, 'the BUNDLE_SET line is the one the generator fills');
  const calls = []; const out = [];
  console.log = (...x) => out.push(x);
  const real = globalThis.fetch;
  const r = await (async () => {
    const saved = src;
    let err = null;
    globalThis.fetch = async (p, o = {}) => { calls.push(p); if (p === ARC_BUNDLE) throw new Error('the Arc was asked'); return real(p, o); };
    try { await eval(inlined); } catch (e) { err = e; }
    return { err };
  })();
  globalThis.fetch = real;
  assert.equal(r.err, null, String(r.err));
  assert.ok(!calls.includes(ARC_BUNDLE), 'the Arc never asked');
  assert.ok(out.some(x => x[0] === "✓ the Arc's bundle, inlined"));
}
// founder-r3.sh against a stand-in on loopback (never production's Arc): the paste it writes carries
// the bundle as the Arc sent it, quoted; a failing or empty bundle stops it, named.
{
  const { execFile } = await import('node:child_process');
  const http = await import('node:http');
  const os = await import('node:os'); const path = await import('node:path');
  const script = fileURLToPath(new URL('./founder-r3.sh', import.meta.url));
  const serve = (code, body) => new Promise(res => { const s = http.createServer((q, w) => { w.writeHead(code, { 'content-type': 'text/plain' }); w.end(body); }); s.listen(0, '127.0.0.1', () => res(s)); });
  const gen = async (code, body) => {
    const s = await serve(code, body);
    const out = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'founder-')), 'paste.js');
    try {
      return await new Promise(res => execFile('bash', [script, out], { env: { ...process.env, ARC_BUNDLE_URL: `http://127.0.0.1:${s.address().port}/v1/bundle` } },
        (e, _o, err) => res({ err: e ? String(err) : null, out: e ? null : fs.readFileSync(out, 'utf8') })));
    } finally { s.close(); }
  };
  const ok = await gen(200, 'b1 "quoted" \\ back\n');
  assert.equal(ok.err, null, ok.err);
  assert.ok(ok.out.includes('const BUNDLE_SET = "b1 \\"quoted\\" \\\\ back";'), 'the bundle inlined, quoted as the Arc sent it');
  assert.ok(!ok.out.includes("const BUNDLE_SET = '<the Arc>';"), 'the placeholder line is filled');
  const vm = await import('node:vm');
  assert.doesNotThrow(() => new vm.Script(ok.out), 'the written paste is one classic script');
  assert.match((await gen(500, 'down')).err, /did not answer 200/);
  assert.match((await gen(200, '  \n')).err, /the bundle is empty/);
}
// SAFARI: one classic script, no top-level await.
{
  const vm = await import('node:vm');
  assert.doesNotThrow(() => new vm.Script(src), 'a classic script: no top-level await');
}
process.stdout.write("stub: rooms unmarked, the Arc's bundle fetched, dry run, add, history, grant last, graph again; a re-run skips what is done; the bundle unreachable or over one blob writes nothing for Healing Resistance; a stop names its step; a grant that did not take fails by name; a Door with no room roles is no failure; no admitter, no write; the bundle inlined is used and the Arc never asked; founder-r3.sh writes it, quoted, and stops on a failing or empty bundle; Safari: one classic script\n");
