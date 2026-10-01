/* seal.js — the passkey's PRF output, sealed to the one session process that asked
   for it (SEC-37; mdr/door.md §4 step 2). The Door's edge and supervisor carry it
   and cannot read it.

   The construction, pinned in ICD-4:
     recipient  X25519 public key, 32 bytes, made by the session process for this
                attempt only
     shared     X25519(ephemeral secret, recipient)
     key        HKDF-SHA256(salt "wallflowers/door/prf/v1", ikm shared,
                            info ephemeral public ‖ recipient), 32 bytes
     seal       AES-256-GCM, a random 12-byte iv, AAD the attempt id in UTF-8
     wire       {epk, iv, ct}, each base64url without padding; ct carries its tag

   One file for the window and for node, so a test seals exactly as DR-4 does. */
(function (root) {
  'use strict';
  var SALT = 'wallflowers/door/prf/v1';
  var enc = new TextEncoder();
  var subtle = root.crypto.subtle;

  function b64u(bytes) {
    var s = '';
    new Uint8Array(bytes).forEach(function (b) { s += String.fromCharCode(b); });
    return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }

  function unb64u(text) {
    var s = atob(text.replace(/-/g, '+').replace(/_/g, '/'));
    var out = new Uint8Array(s.length);
    for (var i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
    return out;
  }

  /* recipient: the session key, base64url. secret: the bytes to seal (the 32-byte
     PRF output). attempt: the id the supervisor gave this sign-in. */
  async function seal(recipient, secret, attempt) {
    var them = unb64u(recipient);
    if (them.length !== 32) throw new Error('a session key is 32 bytes');
    var eph = await subtle.generateKey({ name: 'X25519' }, true, ['deriveBits']);
    var epk = new Uint8Array(await subtle.exportKey('raw', eph.publicKey));
    var peer = await subtle.importKey('raw', them, { name: 'X25519' }, false, []);
    var shared = await subtle.deriveBits({ name: 'X25519', public: peer }, eph.privateKey, 256);
    var info = new Uint8Array(64);
    info.set(epk, 0);
    info.set(them, 32);
    var ikm = await subtle.importKey('raw', shared, 'HKDF', false, ['deriveKey']);
    var key = await subtle.deriveKey(
      { name: 'HKDF', hash: 'SHA-256', salt: enc.encode(SALT), info: info },
      ikm, { name: 'AES-GCM', length: 256 }, false, ['encrypt']);
    var iv = root.crypto.getRandomValues(new Uint8Array(12));
    var ct = await subtle.encrypt({ name: 'AES-GCM', iv: iv, additionalData: enc.encode(attempt) }, key, secret);
    return { epk: b64u(epk), iv: b64u(iv), ct: b64u(ct) };
  }

  var api = { seal: seal, b64u: b64u, unb64u: unb64u };
  root.WallFlowersSeal = api;
  if (typeof module !== 'undefined') module.exports = api;
})(globalThis);
