// WEBAPP REQUIREMENTS § 1: a commutative op's `gen` is the author's own, stamped by
// authoring::build over whatever is sent. The binding never asks for it, and refuses one.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const ops = createRequire(import.meta.url)('./wallflowers-ops.js');

test('a commutative op is drafted without gen', () => {
  const asking = Object.values(ops.OPS).filter((o) => o.fold === 'commutative' && o.fields.some((f) => f.name === 'gen'));
  assert.deepEqual(asking.map((o) => o.name), []);
  assert.deepEqual(ops.forumPost({ text: 'hi' }).args, { text: 'hi' });
  assert.deepEqual(ops.forumReact({ target_author: 'ad'.repeat(32), target_gen: 1, emoji: '👍', active: 1, target: 'ad'.repeat(32) }).args.gen, undefined);
  assert.throws(() => ops.forumPost({ text: 'hi', gen: 3 }), /gen is not an argument/);
});

test('a sequenced op keeps what the ICD declares', () => {
  assert.ok(ops.OPS['event.setProfile'].fields.some((f) => f.name === 'endMs'));
});
