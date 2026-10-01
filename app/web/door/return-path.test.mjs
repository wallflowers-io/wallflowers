/* return-path.test.mjs — SEC-41, srr/vv.md DT-28 (CS-39).

   "The sign-in window shall return only to a path on its own origin, judged by the
   URL it resolves to, not by its spelling." The property tested is where the browser
   goes: the value `returnPath` gives, navigated to from /signin, stays on the Door.

   Run: node --test app/web/door/return-path.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const { returnPath } = createRequire(import.meta.url)('./return-path.js');
const DOOR = 'https://app.wallflowers.io';
/* Where `location.replace(value)` on /signin actually lands. */
const lands = (r) => new URL(returnPath(r, DOOR), DOOR + '/signin');

test('a hostile return never leaves the Door', () => {
  for (const r of [
    '//evil.example',
    '/\\evil.example',
    '/\t/evil.example',
    '/\n/evil.example',
    '/\r/evil.example',
    '/.//evil.example',
    '/..//evil.example',
    '/x/..//evil.example',
    '/%2e//evil.example',
    '/./\t/evil.example',
    '///evil.example',
    'https://evil.example/',
    'javascript:alert(1)',
    'data:text/html,x',
    '\t//evil.example',
  ]) {
    assert.equal(lands(r).origin, DOOR, JSON.stringify(r));
  }
});

test('a missing or broken return goes to the Door\'s root', () => {
  for (const r of [null, undefined, '', 'http://[::1']) {
    assert.equal(lands(r).href, DOOR + '/', String(r));
  }
});

test('a path on the Door is kept, with its query and fragment', () => {
  assert.equal(lands('/x?y=1#z').href, DOOR + '/x?y=1#z');
  assert.equal(lands('/%2F%2Fevil.example').origin, DOOR);
});
