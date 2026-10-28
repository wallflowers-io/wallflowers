/* app.test.mjs — the Members and events example, run as a page runs it: signin.js's session, the
   Door and the public page stubbed. It lists the upcoming events (public and this session's), each
   member's own answer pressed, and answers with group.rsvp on the community; an unpublished
   community's public page (a 404, or a failed fetch) leaves the session's events.
   Run: node --test app/door/docs/examples/members-events/app.test.mjs */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./app.js', import.meta.url), 'utf8').replace('export const CONFIG', 'const CONFIG')
  .replace("address: 'your-community-address'", "address: 'mill-road'");
const SITE = 'a'.repeat(64), OWNER = 'b'.repeat(64), BO = 'c'.repeat(64);
const HELD = 'e'.repeat(64), PUB = 'f'.repeat(64), PAST = '1'.repeat(64), OFF = '2'.repeat(64), LOOSE = '3'.repeat(64);
const DAY = 86_400_000, NOW = Date.now();

function page({ me = BO, published = true } = {}) {
  const els = {};
  const el = (tag) => ({ tag, hidden: false, textContent: '', children: [], attrs: {},
    replaceChildren(...k) { this.children = k; }, append(...k) { this.children.push(...k); }, setAttribute(k, v) { this.attrs[k] = v; } });
  const node = (id) => (els[id] ||= el(id));
  const sent = [];
  const reply = (status, body) => ({ ok: status < 300, status, json: async () => body, text: async () => JSON.stringify(body) });
  const session = {
    site: SITE,
    events: () => () => {},
    signOut: async () => {},
    fetch: async (path, init = {}) => {
      sent.push({ path, body: init.body ? JSON.parse(init.body) : null });
      if (path === '/v2/members') return reply(200, { site: SITE, me, members: [{ key: OWNER, name: 'Hana', role: 'owner' }, { key: BO, name: null, role: 'member' }] });
      if (path === '/v2/graph') return reply(200, { objects: [
        { id: SITE, kind: 'group', owner: OWNER, view: {
          display_name: 'Mill Road',
          affiliations: [HELD, PAST, OFF].map((peer) => ({ peer, rel: 'created' })),
          rsvps: { [HELD]: { [BO]: { status: 'going' } }, [PUB]: { [OWNER]: { status: 'declined' } } },
          rsvp_counts: { [HELD]: { going: 1 }, [PUB]: { going: 0 } } } },
        { id: HELD, kind: 'event', view: { title: 'Seed swap', start_ms: NOW + 2 * DAY, venue: 'Plot 9', status: 'scheduled' } },
        { id: PAST, kind: 'event', view: { title: 'Last spring', start_ms: NOW - 9 * DAY, end_ms: NOW - 8 * DAY } },
        { id: OFF, kind: 'event', view: { title: 'Rained off', start_ms: NOW + 3 * DAY, status: 'cancelled' } },
        { id: LOOSE, kind: 'event', view: { title: 'Not the community\'s', start_ms: NOW + DAY } }] });
      if (path === '/v2/apply') return reply(200, { ok: true });
      return reply(404, 'no such route');
    },
  };
  const ctx = {
    document: { getElementById: node, createElement: el, head: { append() {} } },
    window: { WallFlowers: { current: async () => session, finish: async () => session, signIn() {} } },
    location: { origin: 'http://localhost:5173', pathname: '/', search: '', reload() {} },
    // An unpublished community's 404 carries no CORS header: the browser's fetch rejects.
    fetch: async (url) => {
      sent.push({ path: url });
      if (published === 'no-cors') throw new TypeError('Failed to fetch');
      if (!published) return reply(404, 'no such page');
      return reply(200, { items: { [`event:${PUB}`]: { title: 'Dig day', startMs: NOW + DAY, venue: 'Hall', at: 1 } } });
    },
    URLSearchParams, Map, Set, Date, JSON, Object, Promise, Error, TypeError, String, console,
    setTimeout: (f) => setImmediate(f), clearTimeout() {},
  };
  vm.createContext(ctx);
  vm.runInContext(SRC, ctx);
  return { node, sent };
}
const settle = async () => { for (let i = 0; i < 30; i++) await new Promise((r) => setImmediate(r)); };
const rows = (p) => p.node('events').children.map((li) => ({
  text: li.textContent + li.children.filter((c) => typeof c === 'string').join(''),
  buttons: li.children.flatMap((c) => c.children ?? []).filter((c) => c.tag === 'button').map((b) => `${b.textContent}${b.attrs['aria-pressed'] === 'true' ? '*' : ''}`),
  li,
}));

test('upcoming events, soonest first: the public page\'s and the community\'s own; past and unlinked ones left out', async () => {
  const p = page();
  await settle();
  assert.equal(p.node('status').textContent, '', 'no error');
  assert.deepEqual(p.node('members').children.map((c) => c.textContent), ['Hana · owner', 'New member · member'], 'a name, never a key');
  const r = rows(p);
  assert.deepEqual(r.map((x) => x.text.split(' · ')[1]), ['Dig day', 'Seed swap', 'Rained off'], JSON.stringify(r.map((x) => x.text)));
  assert.ok(r[0].text.includes('Hall') && r[0].text.includes('0 going'));
  assert.ok(r[1].text.includes('Plot 9') && r[1].text.includes('1 going'));
  assert.deepEqual(r[0].buttons, ['Going', 'Not going'], 'unanswered: nothing pressed (another member\'s answer is theirs)');
  assert.deepEqual(r[1].buttons, ['Going*', 'Not going'], 'this member\'s answer, pressed');
  assert.deepEqual(r[2].buttons, [], 'cancelled: no answer');
  assert.ok(r[2].text.includes('cancelled'));
  assert.ok(p.sent.some((s) => s.path === '{{ARC}}/v1/face/mill-road/items'), 'the public page, by its address');
});

test('an answer is group.rsvp on the community, naming the event; no gen sent', async () => {
  const p = page();
  await settle();
  const notGoing = rows(p)[0].li.children.flatMap((c) => c.children ?? []).find((c) => c.textContent === 'Not going');
  await notGoing.onclick();
  const a = p.sent.find((s) => s.path === '/v2/apply').body;
  assert.equal(a.object, SITE);
  assert.equal(a.op, 'group.rsvp');
  assert.deepEqual([a.args.event, a.args.status], [PUB, 'declined']);
  assert.ok(a.args.at > 0);
  assert.ok(!('gen' in a.args), 'WallFlowers adds the gen');
});

for (const published of [false, 'no-cors']) {
  test(`a community not yet published (${published ? 'a 404 without CORS: a failed fetch' : '404'}): the community's own events still show`, async () => {
    const p = page({ published });
    await settle();
    assert.equal(p.node('status').textContent, '', 'an unpublished page is no error');
    assert.equal(p.node('app').hidden, false);
    assert.deepEqual(rows(p).map((x) => x.text.split(' · ')[1]), ['Seed swap', 'Rained off']);
  });
}
