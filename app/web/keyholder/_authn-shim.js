/* ═══════════════════════════════════════════════════════════════════════════
   _authn-shim.js — a virtual authenticator, for the smoke run only.

   TEST CODE, ON THE KEYHOLDER'S ORIGIN, and it must never be loaded by
   index.html. It is served from here because a WebAuthn shim has to be in the
   same document as the code it stands behind, and that code is keyholder.js.
   The leading underscore is the convention this harness uses for files that are
   not the product; _link-frame.html is the only page that pulls it in.

   WHY IT EXISTS. keyholder.js derives the device-sync channel from a passkey's
   `prf` extension. Nothing about that can be automated with a real
   authenticator — Touch ID needs a person — so the platforms provide virtual
   ones for exactly this (Chrome via CDP, Safari via WebDriver). Neither is
   available to a headless WKWebView, so this is that, in the page.

   WHAT IS REAL. The signature is a genuine ECDSA P-256 signature over
   authData || SHA-256(clientDataJSON), made with WebCrypto, and the auth
   service verifies it with py_webauthn and no idea anything is unusual. The PRF
   is a genuine HMAC over the spec's salt derivation. What is NOT real is where
   the key came from: it is handed in, because the whole point is that this
   browser holds THE SAME passkey as the Python side — which is what a
   credential synced through iCloud Keychain is.

   THE ONE THING THAT IS EASY TO GET WRONG. WebCrypto returns an ECDSA
   signature as raw r||s (IEEE P1363). WebAuthn carries DER. A real
   authenticator emits DER, so the shim has to convert — and without it every
   assertion is refused with a signature error that looks like a key mismatch.
   ═══════════════════════════════════════════════════════════════════════════ */
(function () {
  'use strict';

  var enc = new TextEncoder();

  function unb64(s) {
    var raw = atob(String(s).replace(/-/g, '+').replace(/_/g, '/'));
    var u = new Uint8Array(raw.length);
    for (var i = 0; i < raw.length; i++) u[i] = raw.charCodeAt(i);
    return u;
  }
  function b64u(buf) {
    var b = new Uint8Array(buf), s = '';
    for (var i = 0; i < b.length; i++) s += String.fromCharCode(b[i]);
    return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }
  function cat() {
    var n = 0, i;
    for (i = 0; i < arguments.length; i++) n += arguments[i].length;
    var out = new Uint8Array(n), at = 0;
    for (i = 0; i < arguments.length; i++) { out.set(arguments[i], at); at += arguments[i].length; }
    return out;
  }

  /* r||s → DER SEQUENCE { INTEGER r, INTEGER s }. Integers are signed, so a
     leading byte ≥ 0x80 needs a 0x00 in front or it reads as negative. */
  function derSig(raw) {
    var u = new Uint8Array(raw);
    function int(b) {
      var i = 0;
      while (i < b.length - 1 && b[i] === 0) i++;      /* drop leading zeros */
      b = b.slice(i);
      if (b[0] & 0x80) b = cat(new Uint8Array([0]), b);
      return cat(new Uint8Array([0x02, b.length]), b);
    }
    var body = cat(int(u.slice(0, 32)), int(u.slice(32)));
    return cat(new Uint8Array([0x30, body.length]), body);
  }

  function sha256(bytes) {
    return crypto.subtle.digest('SHA-256', bytes).then(function (d) { return new Uint8Array(d); });
  }

  /* WebAuthn L3 §10.1.4: the browser hashes a fixed prefix with the caller's
     salt before it reaches CTAP2's hmac-secret, so a site cannot choose the raw
     HMAC input. Reproduced exactly, or a value derived here would not equal one
     a real authenticator produces. */
  /* The last byte is NUL. Spelled out rather than escaped into the string,
     because a literal control character in source is invisible to a reader
     and turns the file into something `file` reports as binary data. */
  var PRF_PREFIX = cat(enc.encode('WebAuthn PRF'), new Uint8Array([0]));

  /* The credential this browser holds, handed in as JSON:
       { cred_id, pkcs8, cred_random, rp_id, user_handle }  (all base64url)   */
  window.installVirtualAuthenticator = function (cred) {
    var credId = unb64(cred.cred_id);
    var handle = unb64(cred.user_handle);
    var random = unb64(cred.cred_random);
    var rpId = cred.rp_id;

    var signKey = crypto.subtle.importKey(
      'pkcs8', unb64(cred.pkcs8).buffer,
      { name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign']);

    var prfKey = crypto.subtle.importKey(
      'raw', random.buffer, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);

    function prf(salt) {
      return sha256(cat(PRF_PREFIX, salt)).then(function (h) {
        return prfKey.then(function (k) {
          return crypto.subtle.sign('HMAC', k, h);
        });
      });
    }

    /* Some engines make navigator.credentials properties non-writable;
       defineProperty is the fallback where assignment silently does nothing. */
    var impl = function (opts) {
      var pk = opts.publicKey;
      var clientData = enc.encode(JSON.stringify({
        type: 'webauthn.get',
        challenge: b64u(pk.challenge),
        origin: location.origin,
        crossOrigin: false
      }));

      /* UP | UV | BE | BS. The counter stays 0: a SYNCED credential cannot keep
         a monotonic counter across copies, so it reports 0 forever and the
         server skips the check — which is exactly what lets two devices share
         one passkey. An incrementing counter here makes the second device look
         like a clone and be refused, correctly. */
      var authData;
      var want = (pk.extensions && pk.extensions.prf && pk.extensions.prf.eval &&
                  pk.extensions.prf.eval.first) || null;

      return sha256(enc.encode(rpId)).then(function (rpHash) {
        authData = cat(rpHash, new Uint8Array([0x01 | 0x04 | 0x08 | 0x10]),
                       new Uint8Array([0, 0, 0, 0]));
        return sha256(clientData);
      }).then(function (cdHash) {
        return signKey.then(function (k) {
          return crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, k,
                                    cat(authData, cdHash));
        });
      }).then(function (raw) {
        var sig = derSig(raw);
        return (want ? prf(new Uint8Array(want)) : Promise.resolve(null))
          .then(function (first) {
            return {
              id: b64u(credId), rawId: credId.buffer, type: 'public-key',
              response: {
                clientDataJSON: clientData.buffer,
                authenticatorData: authData.buffer,
                signature: sig.buffer,
                userHandle: handle.buffer
              },
              getClientExtensionResults: function () {
                return first ? { prf: { results: { first: first } } } : {};
              }
            };
          });
      });
    };
    try { navigator.credentials.get = impl; } catch (e) {}
    if (navigator.credentials.get !== impl) {
      Object.defineProperty(navigator.credentials, 'get', { value: impl, configurable: true });
    }

    return signKey.then(function () { return true; });
  };
})();
