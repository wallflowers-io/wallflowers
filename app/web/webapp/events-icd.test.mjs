/* events-icd.test.mjs — W-98 Events: events-icd.js, pure readers of the served ICD and of the
   views, which both Events pages use and nothing else restates: the recurrence grammar, the
   media args' caps and suffixes, the acts' vocabulary and shape, who may write an op (its ego,
   asked of the object), the event's profile as setProfile's args, and the Site's answers under
   the keys group.rsvp's and group.setRegistration's own `view` names.

   Run: node --test app/web/webapp/events-icd.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const SRC = readFileSync(new URL('./events-icd.js', import.meta.url), 'utf8');
const ICD = JSON.parse(readFileSync(new URL('../../../core/coordination/delta-graph.icd.json', import.meta.url), 'utf8'));
const OWNER = 'b'.repeat(64), ADMIN = 'c'.repeat(64), MEMBER = 'd'.repeat(64), E = 'e'.repeat(64), P = 'f'.repeat(64), G = '9'.repeat(64);
const EV = ICD.kinds.event.ops, GR = ICD.kinds.group.ops;
const GRAMMAR = EV['event.setProfile'].args.recurrence.grammar;

const ctx = { JSON, Object, Array, String, Number, Math, Date, Error, Intl };
ctx.window = ctx;
vm.createContext(ctx);
vm.runInContext(SRC, ctx);
const X = ctx.WallFlowers.EventsIcd;
const plain = (v) => JSON.parse(JSON.stringify(v));

test('Repeat offers the grammar\'s frequencies and nothing else', () => {
  assert.deepEqual(plain(X.frequencies(ICD)), GRAMMAR.FREQ.oneOf);
  assert.deepEqual(plain(X.weekdays(ICD)), GRAMMAR.BYDAY.subsetOf);
  const names = plain(X.weekdayNames(ICD));
  assert.deepEqual(Object.keys(names), GRAMMAR.BYDAY.subsetOf, 'a name for each token');
  const intl = (i) => new Intl.DateTimeFormat(undefined, { weekday: 'short', timeZone: 'UTC' }).format(new Date(Date.UTC(2024, 0, 8 + i)));
  assert.deepEqual(['MO', 'TU', 'SA', 'SU'].map((d) => names[d]), [intl(0), intl(1), intl(5), intl(6)], "the reader's own day names");
});

test('a rule is built only as the grammar allows, and read back the same', () => {
  const ok = [
    [{ freq: 'WEEKLY' }, 'FREQ=WEEKLY'],
    [{ freq: 'WEEKLY', interval: 2, byday: ['MO', 'WE'] }, 'FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE'],
    [{ freq: 'MONTHLY', count: 6 }, 'FREQ=MONTHLY;COUNT=6'],
    [{ freq: 'DAILY', until: 1790000000000 }, 'FREQ=DAILY;UNTIL=1790000000000'],
  ];
  for (const [choice, rule] of ok) {
    assert.equal(X.ruleFrom(ICD, choice), rule);
    assert.deepEqual(plain(X.parseRule(ICD, rule)), { interval: 1, byday: [], ...choice }, rule);
  }
  assert.equal(X.ruleFrom(ICD, null), '', 'none: no rule');
  const refused = [
    { freq: 'HOURLY' },                                  // not in FREQ's oneOf
    { freq: 'DAILY', byday: ['MO'] },                    // BYDAY only with WEEKLY
    { freq: 'WEEKLY', byday: ['XX'] },                   // not a weekday
    { freq: 'WEEKLY', interval: 0 },                     // below integerMin
    { freq: 'WEEKLY', count: 3, until: 1790000000000 },  // COUNT excludes UNTIL
    { freq: 'WEEKLY', count: 0 },
  ];
  for (const c of refused) assert.throws(() => X.ruleFrom(ICD, c), undefined, JSON.stringify(c));
  assert.equal(X.parseRule(ICD, 'FREQ=WEEKLY;BYHOUR=9'), null, 'a part the grammar refuses: not expressible, held as it is');
  assert.equal(X.parseRule(ICD, ''), null);
});

test("a media arg's cap and its suffixes are the ICD's", () => {
  for (const [op, prefix] of [['event.setBanner', 'banner'], ['event.addPhoto', 'photo'], ['event.setClip', 'clip']]) {
    const args = EV[op].args;
    const m = plain(X.mediaCaps(ICD, op, prefix));
    assert.equal(m.maxLength, args[prefix].maxLength, op);
    assert.deepEqual(m.suffixes, Object.keys(args).filter((k) => k.startsWith(prefix) && k !== prefix).map((k) => k.slice(prefix.length)), op);
  }
});

test("the acts' roles, cap and shape: {member | object, role, start, end}, nothing else", () => {
  const acts = EV['event.setLineup'].args.acts;
  assert.deepEqual(plain(X.actRoles(ICD)), Object.keys(acts.items.role.vocabulary));
  assert.equal(X.actsMax(ICD), acts.maxItems);
  const arg = X.actsArg(ICD, [
    { member: P, role: 'headliner', start: 10, end: 20, confirmed: true, name: 'Mo' },
    { object: G, role: 'support' },
  ]);
  assert.deepEqual(JSON.parse(arg), [{ member: P, role: 'headliner', start: 10, end: 20 }, { object: G, role: 'support' }]);
  assert.throws(() => X.actsArg(ICD, [{ member: P, object: G, role: 'dj' }]), undefined, 'both');
  assert.throws(() => X.actsArg(ICD, [{ role: 'dj' }]), undefined, 'neither');
  assert.throws(() => X.actsArg(ICD, [{ member: P, role: 'juggler' }]), undefined, 'a role the vocabulary lacks');
  assert.throws(() => X.actsArg(ICD, [{ member: P, role: 'dj', start: 20, end: 10 }]), undefined, 'end before start');
  assert.throws(() => X.actsArg(ICD, [{ member: P, role: 'dj' }, { member: P, role: 'mc' }]), undefined, 'twice');
  assert.deepEqual(plain(X.actsOf({ acts: [{ member: P, role: 'dj', start: null, end: null, confirmed: false }] })),
    [{ member: P, object: null, role: 'dj', start: null, end: null, confirmed: false }]);
});

test('who may write is the op\'s ego, asked of this event; the owner saves whole, an admin edits', () => {
  const event = { id: E, kind: 'event', owner: OWNER, members: [OWNER, ADMIN, MEMBER], view: { roles: { [ADMIN]: 'admin' } } };
  assert.equal(X.may(ICD, 'event.setLineup', event, OWNER), true);
  assert.equal(X.may(ICD, 'event.setLineup', event, ADMIN), true);
  assert.equal(X.may(ICD, 'event.setLineup', event, MEMBER), false);
  assert.equal(X.may(ICD, 'event.setTickets', event, ADMIN), false, "an owner's op");
  assert.equal(X.editOp(ICD, event, OWNER), 'event.setProfile');
  assert.equal(X.editOp(ICD, event, ADMIN), 'event.editProfile');
  assert.equal(X.editOp(ICD, event, MEMBER), null);
  assert.deepEqual(plain(X.hostsOf(event)), [OWNER, ADMIN]);
  assert.equal(X.opServed(ICD, 'event.setBanner'), true);
  assert.equal(X.opServed(ICD, 'event.setGuestList'), false);
  assert.deepEqual(plain(X.visibilities(ICD)), Object.keys(ICD.facets.visibility.ops['base.setVisibility'].args.visibility.vocabulary));
  assert.deepEqual(plain(X.statuses(ICD)), Object.keys(EV['event.setProfile'].args.status.vocabulary));
  assert.equal(X.visibilityDefault(ICD, 'event'), ICD.facets.visibility.defaults.event, "the facet's default for an event");
});

test("the event's profile, as setProfile's args: every one the view holds", () => {
  const view = { title: 'Dig day', start_ms: 100, end_ms: 200, descriptor: 'Bring gloves', venue: 'Plot 9', recurrence: 'FREQ=WEEKLY',
    ticket_url: null, tz: 'Europe/London', online: 'https://meet.example/x', status: 'scheduled', video_url: null,
    descriptor_format: 'plain', all_day: 0, lineup: ['Mo', 'Jo'], acts: [], roles: {} };
  const held = plain(X.heldProfile(ICD, view));
  assert.deepEqual(held, { title: 'Dig day', startMs: 100, endMs: 200, descriptor: 'Bring gloves', venue: 'Plot 9', recurrence: 'FREQ=WEEKLY',
    tz: 'Europe/London', online: 'https://meet.example/x', status: 'scheduled', descriptorFormat: 'plain', allDay: 0, lineup: 'Mo\nJo' });
  for (const k of Object.keys(held)) assert.ok(k in EV['event.setProfile'].args, k + ' is a setProfile arg');
});

test("the Site's answers, under the keys the ICD's views name", () => {
  const [rk, ck] = Object.keys(GR['group.rsvp'].view), [gk] = Object.keys(GR['group.setRegistration'].view);
  const site = { view: { [rk]: { [E]: { [MEMBER]: { status: 'going', guests: 1, at: 5 } } }, [ck]: { [E]: { going: 1, maybe: 0, declined: 2, guests: 1 } },
    [gk]: { [E]: { capacity: 10, approval: 1 } } } };
  assert.deepEqual(plain(X.rsvpsOf(ICD, site, E)), { [MEMBER]: { status: 'going', guests: 1, at: 5 } });
  assert.deepEqual(plain(X.countsOf(ICD, site, E)), { going: 1, maybe: 0, declined: 2, guests: 1 });
  assert.deepEqual(plain(X.registrationOf(ICD, site, E)), { capacity: 10, approval: 1 });
  assert.deepEqual(plain(X.countsOf(ICD, { view: {} }, E)), { going: 0, maybe: 0, declined: 0, guests: 0 }, 'none yet');
  assert.deepEqual(plain(X.rsvpStatuses(ICD)), Object.keys(GR['group.rsvp'].args.status.vocabulary));
});
