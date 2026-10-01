/* ═══════════════════════════════════════════════════════════════════════════
   couple.test.mjs — the two clients may not disagree about an account.

   Sharing a file is an intention. This is the enforcement: one archive goes in,
   both surfaces come out, and the test fails if they report different facts.

   WHAT IT HOLDS THEM TO. Not pixels — the phone and the shadow-rooted desktop
   panel look nothing alike and should. What they may not differ on is:

     · which groups exist, and what they are called
     · how many messages are in each, and what the latest one says
     · WHAT COULD NOT BE READ

   The third is the one worth a test of its own. An object that will not fold is
   the easiest thing in the world to lose in a mapping layer, and losing it is
   indistinguishable — to the member — from the object never having existed.
   The desktop's world shape had no field for it, so `toWorld` adds one; this
   asserts it survives the trip, because a field nothing checks is a field that
   will be dropped in the next refactor.

   Run:  node app/web/shared/couple.test.mjs
   ═══════════════════════════════════════════════════════════════════════════ */
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { createRequire } from 'module';

const here = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const Fold = require(path.join(here, 'wallflowers-fold.js'));

const CORE = path.join(here, '..', 'docs', 'core', 'core_wasm_bg.wasm');
/* The committed archive, held to core-wasm's hand_built() by
   the_sim_archive_is_the_hand_built_one. It used to read an untracked copy of the
   same bytes under mobile/fixture/, which a fresh checkout does not have. */
const FIXTURE = path.join(here, 'wallflowers-sim.cbor');

let fails = 0;
const ok = (what, cond, detail) => {
  if (!cond) fails++;
  console.log(`${cond ? '  ok  ' : ' FAIL '} ${what}${cond || !detail ? '' : `\n         ${detail}`}`);
};
const same = (what, a, b) => ok(what, JSON.stringify(a) === JSON.stringify(b),
  `phone  ${JSON.stringify(a)}\n         desktop ${JSON.stringify(b)}`);

for (const p of [CORE, FIXTURE]) {
  if (!fs.existsSync(p)) {
    console.error(`missing ${p}\n  core:    run app/web/build-wasm.sh\n  archive: WALLFLOWERS_SIM_WRITE=1 ` +
      `cargo test -p core-wasm the_sim_archive_is_the_hand_built_one`);
    process.exit(2);
  }
}

/* The real core, with the host entropy it declares as an import. */
const mod = await WebAssembly.compile(fs.readFileSync(CORE));
let inst;
const imports = {};
for (const i of WebAssembly.Module.imports(mod)) {
  (imports[i.module] ||= {})[i.name] = i.name === 'pacific_fill_random'
    ? (ptr, len) => { crypto.getRandomValues(new Uint8Array(inst.exports.memory.buffer, ptr, len)); return 0; }
    : () => 0;
}
inst = await WebAssembly.instantiate(mod, imports);
const core = inst.exports;

/* One archive. One fold. Two surfaces. */
const bytes = new Uint8Array(fs.readFileSync(FIXTURE));
const model = Fold.fold(core, bytes);           /* what the phone draws */
const world = Fold.toWorld(model, { site: 'test' });  /* what the desktop draws */

console.log('\nthe fold ran, and ran for real');
ok('groups came back', model.groups.length > 0, 'the fixture folds five');
ok('refusals came back', model.problems.length > 0, 'the fixture refuses two on purpose');
ok('the refusals are the coordinator\'s words, not ours',
   model.problems.every(p => p.why && !/could not|unknown error/i.test(p.why)),
   JSON.stringify(model.problems.map(p => p.why)));

ok('every object keeps its id', model.groups.every(g => /^[0-9a-f]{16,}$/.test(g.id)),
   JSON.stringify(model.groups.map(g => g.id)));
ok('every object keeps its roster: the owner and the members the core folded',
   model.groups.every(g => /^[0-9a-f]{64}$/.test(g.owner || '') && Array.isArray(g.members) && g.members.length > 0),
   JSON.stringify(model.groups.map(g => [String(g.owner).slice(0, 8), g.members.length])));
same('and the desktop keys its sites by those ids, not by name',
     Object.keys(world.sites).sort(), model.groups.map(g => g.id).sort());

console.log('\nboth clients name the same groups');
const phoneNames = model.groups.map(g => g.name).sort();
const deskNames = Object.values(world.sites).map(s => s.name).sort();
same('the same group names', phoneNames, deskNames);
same('the same number of groups', model.groups.length, Object.keys(world.sites).length);

console.log('\nboth clients hold the same conversations');
const phoneThreads = model.groups.filter(g => g.messages.length)
  .map(g => ({ name: g.name, n: g.messages.length, last: g.messages[g.messages.length - 1].text }))
  .sort((a, b) => a.name.localeCompare(b.name));
const deskThreads = world.threads
  .map(t => ({ name: t.title, n: t.n, last: t.posts[t.posts.length - 1].s }))
  .sort((a, b) => a.name.localeCompare(b.name));
same('the same threads, counts and latest message', phoneThreads, deskThreads);

console.log('\nWHAT COULD NOT BE READ SURVIVES THE TRIP TO THE DESKTOP');
ok('the desktop world carries the refusals at all', Array.isArray(world.problems),
   'toWorld dropped them — a member would see a shorter list and be told nothing');
same('the same refusals, in the same words',
     model.problems.map(p => p.why).sort(),
     (world.problems || []).map(p => p.why).sort());

console.log('\nthe desktop world is complete enough to render');
for (const k of ['site', 'me', 'sites', 'people', 'threads', 'events',
                 'rsvp', 'convs', 'links', 'pending', 'listings', 'posts']) {
  ok(`world.${k} exists`, world[k] !== undefined);
}
/* The invariant that broke the desktop the first time it was handed a real
   fold: pacific.js reads w.sites[w.site].name unguarded. */
ok('world.site names a site that exists', !!world.sites[world.site],
   `site=${JSON.stringify(world.site)} keys=${JSON.stringify(Object.keys(world.sites))}`);
const empty = Fold.toWorld({ me: null, groups: [], problems: [] }, { site: 'nothing-here' });
ok('and still does for an account with no groups at all', !!empty.sites[empty.site],
   `site=${JSON.stringify(empty.site)} keys=${JSON.stringify(Object.keys(empty.sites))}`);

ok('and invents nothing where the fold had nothing',
   world.events.length === model.groups.filter(g => g.kind === 'event').length &&
   world.listings.length === 0 && world.posts.length === 0);

console.log('\nan archive the core refuses is refused in the core\'s words');
/* A refusal returns 0 and leaves its reason in the out buffer. Read by the
   returned length, every refusal was an Error with nothing in it. */
let why = null;
try { Fold.fold(core, new Uint8Array([1, 2, 3, 4])); } catch (e) { why = e.message; }
ok('it throws', why !== null);
ok('with the core\'s reason in it', !!why && !/gave no reason/.test(why), JSON.stringify(why));

console.log('\nthe shared store refuses to write, rather than pretending');
const st = Fold.store({ core, archive: () => bytes, site: 'test' });
const loaded = await st.load();
ok('load gives the desktop its world', loaded && loaded.sites && Object.keys(loaded.sites).length > 0);
let refused = null;
await st.commit({ t: 'post', s: 'hello' }).then(() => {}, e => { refused = e; });
ok('commit refuses', !!refused, 'a store that accepts and drops makes the interior lie');
ok('and says why', refused && /MLS send path/.test(refused.message), refused && refused.message);

console.log('\nthe standard sim answer, through the client a site imports');
{
  const { simAnswer } = await import('./gen-sim.mjs');
  const C = require(path.join(here, 'wallflowers-client.js'));
  const answer = await simAnswer(CORE);
  const sim = await C.socialApi(C.sim(answer), { normalise: Fold.normalise }).fold();
  ok('the sim answer is the core\'s own fold, and says it is a fixture', sim.fixture === true && answer.fixture === true);
  same('it folds the same objects the phone does', sim.groups.map(g => g.id).sort(), model.groups.map(g => g.id).sort());
  ok('and carries the same refusals, in the core\'s words', sim.problems.length === model.problems.length && sim.problems.length > 0);
  const w = await C.socialApi(C.sim(answer), { normalise: Fold.normalise }).author(sim.groups[0].id, 'forum.post', { text: 'x' });
  ok('a write to it is refused, and says why', w.ok === false && /fixture/.test(w.why), JSON.stringify(w));
  const cap = await C.socialApi(C.sim(answer), { normalise: Fold.normalise }).capabilities();
  same('and its capabilities carry the core\'s `on`, so a site\'s kind checks run on a fixture',
       cap.ops['base.noteWrite'] && cap.ops['base.noteWrite'].on, ['notebook', 'note']);
}

console.log(fails ? `\n${fails} FAILED — the two clients disagree\n` : '\nboth clients agree\n');
process.exit(fails ? 1 : 0);
