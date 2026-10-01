/* ═══════════════════════════════════════════════════════════════════════════
   THE CLIENT, HELD TO THE CONTRACT ITS CONSUMERS AGREED.

   Every case in refusal-cases.json goes through refusalOf and must return its
   `expect` exactly; every answer shape goes through the client itself. Beyond
   the file: a connection that fails to open is transport whatever its words, the
   fold is read through the producer's own normalise, and a fixture says so.

   Run:  node app/web/shared/wallflowers-client.test.mjs
   ═══════════════════════════════════════════════════════════════════════════ */
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { createRequire } from 'module';

const here = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const Fold = require(path.join(here, 'wallflowers-fold.js'));
const C = require(path.join(here, 'wallflowers-client.js'));
const contract = JSON.parse(fs.readFileSync(path.join(here, 'refusal-cases.json'), 'utf8'));

let fails = 0;
const ok = (what, cond, detail) => {
  if (!cond) fails++;
  console.log(`${cond ? '  ok  ' : ' FAIL '} ${what}${cond || !detail ? '' : `\n         ${detail}`}`);
};
/* Equal as values: the same keys and the same values, in any key order. */
const canon = (v) => Array.isArray(v) ? v.map(canon)
  : v && typeof v === 'object' ? Object.keys(v).sort().reduce((o, k) => (o[k] = canon(v[k]), o), {}) : v;
const same = (what, got, want) => ok(what, JSON.stringify(canon(got)) === JSON.stringify(canon(want)),
  `got ${JSON.stringify(got)}\n         want ${JSON.stringify(want)}`);

/* A port whose every call answers `v`, or throws the envelope an error. */
const port = (answer) => () => Promise.resolve({
  call: (method, args) => (answer.ok ? Promise.resolve(typeof answer.v === 'function' ? answer.v(method, args) : answer.v)
    : Promise.reject(Object.assign(new Error(answer.e), answer.refusal ? { refusal: answer.refusal } : {})))
});

console.log('\nrefusal-cases.json — every case, through refusalOf');
for (const c of contract.cases) {
  const err = c.envelope
    ? Object.assign(new Error(c.envelope.e), c.envelope.refusal ? { refusal: c.envelope.refusal } : {})
    : new Error(c.error);
  same(c.name, C.refusalOf(c.op, c.method, err), c.expect);
}

console.log('\nrefusal-cases.json — every answer shape, through the client');
for (const c of contract.answer_shape) {
  const api = C.socialApi(port(c.answer), { normalise: Fold.normalise });
  const got = c.method === 'object.author' ? await api.author('g1', c.op, {})
            : c.method === 'object.mint' ? await api.mint(c.kind, {})
            : null;
  same(c.name, got, c.expect);
}

console.log('\nwhat the client owns, beyond the file');
{
  // R4 by structure: a connect failure whose words match nothing is still transport.
  const api = C.socialApi(() => Promise.reject(new Error('the page has no keyholder frame yet')));
  const r = await api.author('g1', 'forum.post', { text: 'x' });
  same('a connection that will not open is transport, whatever its words', r,
       { ok: false, op: 'forum.post', by: 'transport', why: 'the page has no keyholder frame yet' });
  // …and the next call asks again rather than repeating the old failure.
  let n = 0;
  const flaky = C.socialApi(() => (++n === 1 ? Promise.reject(new Error('not yet')) : port({ ok: true, v: { deltaId: 'd1' } })()));
  await flaky.author('g1', 'forum.post', {});
  same('a failed connection is forgotten, so the next call opens a new one',
       await flaky.author('g1', 'forum.post', {}), { ok: true, deltaId: 'd1', group: 'g1' });
}
{
  const api = C.socialApi(port({ ok: true, v: { writes: 'yes', ops: { 'forum.post': { reachable: true, needs: 'a gen floor' }, junk: {} } } }));
  const on = C.socialApi(port({ ok: true, v: { writes: true, ops: { 'base.noteWrite': { reachable: true, on: ['notebook', 'note', 7] } } } }));
  same('capabilities carry `on`, the kinds the door will write an op onto (and nothing that is not a kind)',
       (await on.capabilities()).ops['base.noteWrite'].on, ['notebook', 'note']);
  same('capabilities: anything but a plain true is not permission, and a malformed row is left out',
       await api.capabilities(), { writes: false, ops: { 'forum.post': { reachable: true, why: null, needs: 'a gen floor', on: [] } } });
}
{
  const raw = {
    v: 1, exported_by: 'ab'.repeat(32), display_name: 'Ada', peers: [],
    groups: [{ id: 'cd'.repeat(16), kind: 'forum', name: 'Hall chat', owner: 'ab'.repeat(32), members: ['ab'.repeat(32)],
               view: { messages: [], rooms: [] },
               recovery: { named: true, speakable: false, first_epoch: 0, current_epoch: 3, unreadable_epochs: [0, 1, 2, 3] } }],
    rejected: [], recovery: { tail: 'sideways', holes: [1, 'x'], extra: 0, found: 2 }
  };
  const model = await C.socialApi(port({ ok: true, v: raw }), { normalise: Fold.normalise }).fold();
  ok('the fold is read through the producer\'s normalise, which keeps the roster',
     model.groups[0].owner === 'ab'.repeat(32) && model.groups[0].members.length === 1 && model.groups[0].id === 'cd'.repeat(16));
  same('the per-object recovery record rides beside its group', model.groups[0].recovery,
       { named: true, speakable: false, first_epoch: 0, current_epoch: 3, unreadable_epochs: [0, 1, 2, 3] });
  same('an account record with an unknown tail reads as unknown, never intact', model.recovery,
       { tail: 'unknown', holes: [1], extra: 0, found: 2 });
  ok('a live answer is not a fixture', model.fixture === false);
  same('without a normalise the fold refuses, rather than reading an answer it cannot',
       await C.socialApi(port({ ok: true, v: raw })).fold(),
       { ok: false, op: 'object.fold', by: 'site', why: "The producer's fold (wallflowers-fold.js) did not load here, so an answer could not be read." });

  const sim = C.socialApi(C.sim(raw), { normalise: Fold.normalise });
  ok('a sim model says it is a fixture', (await sim.fold()).fixture === true);
  const w = await sim.author('cd'.repeat(16), 'forum.post', { text: 'x' });
  same('and refuses every write as the SITE, since nothing was asked of WallFlowers', w,
       { ok: false, op: 'forum.post', by: 'site', why: 'this is a fixture — nothing is written, and nothing here is a member' });
  ok('and says it may not write', (await sim.capabilities()).writes === false);
}

console.log(fails ? `\n${fails} FAILED` : '\nall passed — the client meets the contract its consumers agreed');
process.exit(fails ? 1 : 0);
