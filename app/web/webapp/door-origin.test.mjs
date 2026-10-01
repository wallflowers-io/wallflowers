/* door-origin.test.mjs — SEC-9, srr/vv.md DT-27.

   "The webapp shall send sign-in credentials only to the Door origin fixed in its
   build, never to an origin taken from the page's address." A link cannot choose
   where the recovery words go.

   Run: node --test app/web/webapp/door-origin.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const { doorOrigin } = createRequire(import.meta.url)('./door-origin.js');
const at = (href) => doorOrigin(new URL(href));

test('a served page talks to its own origin, whatever ?door= names', () => {
  for (const href of [
    'https://app.wallflowers.io/',
    'https://app.wallflowers.io/?door=https://evil.example',
    'https://app.wallflowers.io/?door=http://127.0.0.1:8233',
    'https://app.wallflowers.io/?door=javascript:alert(1)',
    'https://app.wallflowers.io/#door=https://evil.example',
  ]) {
    assert.equal(at(href), 'https://app.wallflowers.io', href);
  }
});

test('a loopback page may name another loopback door', () => {
  assert.equal(at('http://127.0.0.1:8231/?door=http://127.0.0.1:9999'), 'http://127.0.0.1:9999');
  assert.equal(at('http://localhost:8231/?door=http://localhost:9999'), 'http://localhost:9999');
  assert.equal(at('http://[::1]:8231/?door=http://[::1]:9999'), 'http://[::1]:9999');
});

// A loopback page with no valid ?door= talks to its own origin: the Door serves the
// webapp same-origin (ba54d39). What matters is that no link can name the origin.
test('a loopback page cannot be pointed anywhere else', () => {
  for (const asked of [
    'https://evil.example',
    'http://localhost@evil.example',
    'http://127.0.0.1.evil.example',
    'http://evil.example/127.0.0.1',
    'javascript:alert(1)',
    'data:text/html,x',
    'not a url',
    '',
  ]) {
    const href = 'http://127.0.0.1:8231/?door=' + encodeURIComponent(asked);
    assert.equal(at(href), 'http://127.0.0.1:8231', asked);
    assert.notEqual(new URL(at(href)).hostname, 'evil.example', asked);
  }
});

test('a loopback page with no ?door= talks to its own origin', () => {
  assert.equal(at('http://127.0.0.1:8231/'), 'http://127.0.0.1:8231');
});
