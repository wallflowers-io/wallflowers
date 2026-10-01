import test from 'node:test';
import assert from 'node:assert/strict';
import { diff, check } from './icd-changelog.mjs';

const model = (ops, extra = {}) => ({ version: 'x', kinds: { forum: { typeId: 1, ops } }, facets: {}, ...extra });
const op = (id, ego = 'member', args = {}) => ({ op: id, ego, fold: 'commutative', args });

test('an op added, retired, renamed in place, or changed is named, and nothing else is', () => {
  const before = model({ 'forum.post': op(0, 'member', { text: { type: 'string', required: true } }), 'forum.old': op(1), 'forum.gone': op(2) });
  const after = model({ 'forum.post': op(0, 'owner', { text: { type: 'string', required: true, maxLength: 5 }, media: { type: 'string' } }), 'forum.new': op(1), 'forum.retract': op(6) });
  assert.deepEqual(diff(before, after), [
    'added: forum.retract (forum, op 6, member, commutative)',
    'changed: forum.post: ego member → owner; +media (string); text: string required → string required max 5',
    'renamed: forum.old → forum.new',
    'retired: forum.gone (forum, op 2)',
  ]);
});

test('a release that moves no op says which of the model\'s other sections moved', () => {
  const ops = { 'forum.post': op(0) };
  assert.deepEqual(diff(model(ops, { mls: { history: 'none' } }), model(ops, { mls: { history: 'welcome' } })), ['model sections changed besides ops: mls']);
  assert.deepEqual(diff(model(ops), model(ops)), []);
});

test('every icd/* tag has its entry, each entry is what the releases compute, and the docs describe a release', () => {
  assert.equal(check(), 0);
});
