(async () => { // one async function: Safari's console takes no top-level await
/* EVERY CLAIM INTO EVERY ROOM (Ralph, 30 Sep: "Everyone joining (paid or unpaid) should be put into
   all groups"), in the console of https://app.wallflowers.io, signed in as the Site's owner.
   Each room carrying a claim's choice is set again with none (base.setPart, the part replaced in
   place, its `at` kept). With no room carrying a claim's `c`, the Arc admits every claim to every
   room it admits to (admit_by_claim's fallback); it reads the marks at each admission, so the next
   claim joins them all. The choice stays recorded on the spend (base.claimSpent). Healing
   Resistance is not touched: the Arc is not in it, and a room it is not in admits no claim.
   Each step says what it did, and a failure stops it, naming the step. Run it again after a stop:
   a room already unmarked is skipped. */
const SITE = '71c104edcc747ef54bf2bf16f770ea2b1f7f60deb4b9262a0d11ae808b886a6c';

const call = (p, b) => fetch(p, { method: b ? 'POST' : 'GET', credentials: 'include', headers: { 'content-type': 'application/json' }, body: b && JSON.stringify(b) })
  .then(r => r.ok ? r.json() : r.text().then(t => { throw new Error(p + ' ' + r.status + ' ' + t); }));
const step = async (what, f) => { try { const r = await f(); console.log('✓ ' + what, r ?? ''); return r; } catch (e) { throw new Error(what + ': ' + (e && e.message || e)); } };

const graph = async (what) => {
  const g = await step(what, () => call('/v2/graph'));
  const S = g.objects.find(o => o.id === SITE);
  if (!S) throw new Error('this account holds no Site ' + SITE);
  return { g, S };
};
let { g, S } = await graph('the graph');
const N = ((S.view.roles || []).find(r => r[1] === 'admitter') || [])[0];
if (!N) throw new Error('the Site has no admitter: the Arc admits no one');
const nameOf = id => (g.objects.find(o => o.id === id) || {}).name || id.slice(0, 8);
const rooms = () => S.view.parts.filter(p => p.role === 'room');
const marked = rooms().filter(p => p.choice);
if (!marked.length) console.log('already: no room carries a choice');
for (const p of marked) {
  await step('room ' + nameOf(p.part) + ' unmarked (was ' + p.choice + ')', () =>
    call('/v2/apply', { object: SITE, op: 'base.setPart', args: { part: p.part, role: 'room', at: p.at ?? Date.now() } }));
}
({ g, S } = await graph('the graph, again'));
const left = rooms().filter(p => p.choice);
if (left.length) throw new Error('still marked: ' + left.map(p => nameOf(p.part) + ' (' + p.choice + ')').join(', '));
const holds = id => ((g.objects.find(o => o.id === id) || {}).members || []).includes(N);
console.log({ every_claim_joins: rooms().filter(p => holds(p.part)).map(p => nameOf(p.part)), not_the_arcs: rooms().filter(p => !holds(p.part)).map(p => nameOf(p.part)) });
})();
