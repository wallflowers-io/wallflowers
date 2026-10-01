/* pow-worker.js — the proof of work a sign-up or sign-in without a kiosk claim pays
   before it starts (D-55). The window runs it in a Worker during consent, so the
   page stays live; the Door checks it (app/door/src/pow.rs).

   The contract, stated too in pow.rs:
     challenge  v1.<seed>.<issued>.<bits>.<mac>, opaque to the window but for seed
                and bits; seed is 16 bytes, base64url without padding; the MAC binds
                the start it was issued for, which the text does not carry
     nonce      the count 0, 1, 2, … as ASCII decimal, no leading zeros
     work       SHA-256(seed bytes ‖ nonce digits) has `bits` leading zero bits

   postMessage({challenge}) answers postMessage({nonce}), the first count that works.
   SHA-256 is written out: crypto.subtle is a promise per hash, too slow for a
   million of them. One file for the Worker and for node, so a test solves exactly
   as the window does. */
(function (root) {
  'use strict';
  var IV = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
  var K = new Int32Array([
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2]);
  var W = new Int32Array(64);

  /* One 64-byte block of `m` at `off` into the state `st`. */
  function compress(st, m, off) {
    var i, x, y;
    for (i = 0; i < 16; i++, off += 4) W[i] = m[off] << 24 | m[off + 1] << 16 | m[off + 2] << 8 | m[off + 3];
    for (i = 16; i < 64; i++) {
      x = W[i - 15];
      y = W[i - 2];
      W[i] = ((x >>> 7 | x << 25) ^ (x >>> 18 | x << 14) ^ x >>> 3) + W[i - 7] +
             ((y >>> 17 | y << 15) ^ (y >>> 19 | y << 13) ^ y >>> 10) + W[i - 16] | 0;
    }
    var a = st[0], b = st[1], c = st[2], d = st[3], e = st[4], f = st[5], g = st[6], h = st[7];
    for (i = 0; i < 64; i++) {
      x = h + ((e >>> 6 | e << 26) ^ (e >>> 11 | e << 21) ^ (e >>> 25 | e << 7)) + (e & f ^ ~e & g) + K[i] + W[i] | 0;
      y = ((a >>> 2 | a << 30) ^ (a >>> 13 | a << 19) ^ (a >>> 22 | a << 10)) + (a & b ^ a & c ^ b & c) | 0;
      h = g; g = f; f = e; e = d + x | 0; d = c; c = b; b = a; a = x + y | 0;
    }
    st[0] = st[0] + a | 0; st[1] = st[1] + b | 0; st[2] = st[2] + c | 0; st[3] = st[3] + d | 0;
    st[4] = st[4] + e | 0; st[5] = st[5] + f | 0; st[6] = st[6] + g | 0; st[7] = st[7] + h | 0;
  }

  /* SHA-256 of any bytes, as a Uint8Array: what the test holds against node's. */
  function sha256(bytes) {
    var n = bytes.length, m = new Uint8Array((n + 72) & ~63), h = new Int32Array(IV), i;
    m.set(bytes);
    m[n] = 0x80;
    m[m.length - 5] = n / 0x20000000;
    m[m.length - 4] = n >>> 21; m[m.length - 3] = n >>> 13; m[m.length - 2] = n >>> 5; m[m.length - 1] = n << 3;
    for (i = 0; i < m.length; i += 64) compress(h, m, i);
    var out = new Uint8Array(32);
    for (i = 0; i < 32; i++) out[i] = h[i >> 2] >>> (24 - 8 * (i & 3));
    return out;
  }

  function zeros(h) {
    for (var n = 0, i = 0; i < 8; i++) {
      var z = Math.clz32(h[i]);
      n += z;
      if (z < 32) break;
    }
    return n;
  }

  function unb64u(text) {
    var s = atob(text.replace(/-/g, '+').replace(/_/g, '/'));
    var out = new Uint8Array(s.length);
    for (var i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
    return out;
  }

  /* The first nonce that does the challenge's work. Seed and digits never pass 55
     bytes, so every try is one block, built in place. */
  function solve(challenge) {
    var p = String(challenge).split('.');
    var seed = p.length === 5 && p[0] === 'v1' ? unb64u(p[1]) : [];
    var bits = +p[3];
    if (seed.length !== 16 || !(bits >= 0 && bits <= 256)) throw new Error('not a v1 challenge');
    var m = new Uint8Array(64), h = new Int32Array(8), s, n, i;
    m.set(seed);
    for (var nonce = 0; ; nonce++) {
      s = String(nonce);
      n = 16 + s.length;
      for (i = 0; i < s.length; i++) m[16 + i] = s.charCodeAt(i);
      m[n] = 0x80;
      m[62] = n >>> 5;
      m[63] = n << 3;
      h.set(IV);
      compress(h, m, 0);
      if (zeros(h) >= bits) return s;
    }
  }

  var api = { sha256: sha256, solve: solve };
  if (typeof module !== 'undefined') module.exports = api;
  if (typeof importScripts === 'function') {
    root.onmessage = function (e) { root.postMessage({ nonce: solve(e.data.challenge) }); };
  }
})(globalThis);
