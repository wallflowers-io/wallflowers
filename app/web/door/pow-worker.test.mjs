/* pow-worker.test.mjs — D-55. The window's proof of work hashes what the Door's
   check hashes: SHA-256(seed bytes ‖ ASCII decimal nonce), and answers the first
   count that has `bits` leading zero bits.

   VECTOR is pinned in app/door/src/pow.rs too: the Door signs it with the test key for
   a sign-up, and accepts NONCE, which its own count from 0 also finds first.

   Run: node --test app/web/door/pow-worker.test.mjs */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createHash, createHmac, randomBytes } from 'node:crypto';

const { sha256, solve } = createRequire(import.meta.url)('./pow-worker.js');

const KEY = 'wallflowers/door/pow/test-vector';
const VECTOR = 'v1.AAECAwQFBgcICQoLDA0ODw.1790000000.16.y-rbL9BGUNtPum8ZPfzsZ0MOqzmk-lXx9Gh-0CkCCm0';
const NONCE = '34416';

const digest = (seed, nonce) => createHash('sha256').update(seed).update(nonce, 'ascii').digest();
const zeros = (d) => {
  let n = 0;
  for (const b of d) {
    n += Math.clz32(b) - 24;
    if (b) break;
  }
  return n;
};

test('sha256 is SHA-256, across the block boundaries', () => {
  for (const n of [0, 1, 3, 31, 32, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 1000]) {
    const b = randomBytes(n);
    assert.deepEqual(Buffer.from(sha256(b)), createHash('sha256').update(b).digest(), `${n} bytes`);
  }
  assert.equal(Buffer.from(sha256(new Uint8Array(0))).toString('hex'),
    'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
});

test('solve answers the first nonce that does the work', () => {
  for (const bits of [0, 1, 8, 12]) {
    const seed = randomBytes(16);
    const nonce = solve(`v1.${seed.toString('base64url')}.0.${bits}.mac`);
    assert.match(nonce, /^(0|[1-9][0-9]*)$/);
    assert.ok(zeros(digest(seed, nonce)) >= bits, `bits ${bits}`);
    for (let n = 0; n < +nonce; n++) assert.ok(zeros(digest(seed, String(n))) < bits, `${n} came first`);
  }
});

test('the vector the Door pins: its MAC, and the nonce both sides find', () => {
  const [v, seed, issued, bits, mac] = VECTOR.split('.');
  assert.equal(v, 'v1');
  assert.equal(mac, createHmac('sha256', KEY).update(`v1.${seed}.${issued}.${bits}.signup`).digest('base64url'));
  assert.equal(solve(VECTOR), NONCE);
  assert.ok(zeros(digest(Buffer.from(seed, 'base64url'), NONCE)) >= +bits);
});

test('not a v1 challenge, refused', () => {
  for (const c of ['', 'v2.AAECAwQFBgcICQoLDA0ODw.0.8.m', 'v1.AAEC.0.8.m', 'v1.AAECAwQFBgcICQoLDA0ODw.0.x.m']) {
    assert.throws(() => solve(c), /not a v1 challenge/, JSON.stringify(c));
  }
});
