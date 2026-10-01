/* DR-5, Sign in with WallFlowers (RD.4; mdr/door.md §5). A public site loads this
   from the Door, pinned by SRI. It sends the person to the Door's window, takes the
   code at the site's registered callback, trades it with its PKCE verifier for the
   session token, and spends the token with a P-256 key the page cannot export
   (DPoP, RFC 9449; SEC-39). The key and the token live in this origin's IndexedDB
   until the token expires or the person signs out; there is no refresh token.

     WallFlowers.signIn({client, callback})   navigates to the Door
     WallFlowers.finish({client, callback})   on the callback page → session, or null if cancelled
     WallFlowers.current({client})            → session, or null
     session.fetch(path, init)  session.events(onChanged, onEnded) → stop  session.signOut() */
(function (root) {
  'use strict';
  var DOOR = new URL(document.currentScript.src).origin;
  var enc = new TextEncoder();
  var subtle = crypto.subtle;
  var DB = 'wallflowers-door', STORE = 'session';

  function b64u(bytes) {
    var s = '';
    new Uint8Array(bytes).forEach(function (b) { s += String.fromCharCode(b); });
    return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }
  function random(n) { return b64u(crypto.getRandomValues(new Uint8Array(n))); }
  function sha256(text) { return subtle.digest('SHA-256', enc.encode(text)).then(b64u); }

  /* ── this origin's IndexedDB: one record, the key and the token ─────────── */
  function db() {
    return new Promise(function (ok, no) {
      var r = indexedDB.open(DB, 1);
      r.onupgradeneeded = function () { r.result.createObjectStore(STORE); };
      r.onsuccess = function () { ok(r.result); };
      r.onerror = function () { no(r.error); };
    });
  }
  function tx(mode, f) {
    return db().then(function (d) {
      return new Promise(function (ok, no) {
        var t = d.transaction(STORE, mode), req = f(t.objectStore(STORE));
        t.oncomplete = function () { ok(req && req.result); };
        t.onerror = function () { no(t.error); };
      });
    });
  }
  var keep = function (v) { return tx('readwrite', function (s) { return s.put(v, 'current'); }); };
  var read = function () { return tx('readonly', function (s) { return s.get('current'); }); };
  var forget = function () { return tx('readwrite', function (s) { return s.delete('current'); }); };

  /* ── a DPoP proof: ES256 over {jti, htm, htu, iat, ath?} with the jwk in the header */
  function proof(keys, method, url, token) {
    return subtle.exportKey('jwk', keys.publicKey).then(function (jwk) {
      var header = { typ: 'dpop+jwt', alg: 'ES256', jwk: { kty: jwk.kty, crv: jwk.crv, x: jwk.x, y: jwk.y } };
      var claims = { jti: random(16), htm: method, htu: url.split(/[?#]/)[0], iat: Math.floor(Date.now() / 1000) };
      return (token ? sha256(token).then(function (ath) { claims.ath = ath; }) : Promise.resolve()).then(function () {
        var input = b64u(enc.encode(JSON.stringify(header))) + '.' + b64u(enc.encode(JSON.stringify(claims)));
        return subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, keys.privateKey, enc.encode(input))
          .then(function (sig) { return input + '.' + b64u(sig); });
      });
    });
  }

  function refusal(r) { return r.text().then(function (t) { throw new Error(t || String(r.status)); }); }

  function session(s) {
    function call(path, init) {
      init = init || {};
      var url = DOOR + path, method = (init.method || 'GET').toUpperCase();
      return proof(s.keys, method, url, s.token).then(function (p) {
        var h = new Headers(init.headers || {});
        h.set('authorization', 'DPoP ' + s.token);
        h.set('dpop', p);
        if (init.body && !h.has('content-type')) h.set('content-type', 'application/json');
        return fetch(url, Object.assign({}, init, { method: method, headers: h, credentials: 'omit' }));
      });
    }
    return {
      client: s.client,
      site: s.site,
      fetch: call,
      /* The change stream, by fetch: EventSource cannot carry a token (DV-9). One that ends
         or fails, not by stop(), opens again after a wait that doubles from 2 s to a minute,
         reset once one opens (NC-133; the webapp's listen()). The Door says nothing on open,
         so a reopened stream says `changed` once for what came while it was down, its version
         '': "caught up after a gap", no version of the Door's. A 401 is
         the session over: not asked again, and onEnded, if given, has the refusal. */
      events: function (onChanged, onEnded) {
        var stop = false, reader = null, wait = null, backoff = 0, opened = false;
        function tell(v) { try { onChanged(v); } catch (e) { Promise.reject(e); } }
        function again() {
          if (stop) return;
          backoff = Math.min(Math.max(backoff * 2, 2000), 60000);
          wait = setTimeout(open, backoff);
        }
        function open() {
          wait = null;
          call('/v2/events').then(function (r) {
            if (stop) { if (r.body) r.body.cancel(); return; }
            if (r.status === 401) {
              stop = true;
              return refusal(r).catch(function (e) { e.status = 401; if (onEnded) onEnded(e); });
            }
            if (!r.ok) { if (r.body) r.body.cancel(); return again(); }
            backoff = 0;
            if (opened) tell('');
            opened = true;
            reader = r.body.getReader();
            var dec = new TextDecoder(), buf = '';
            (function pump() {
              reader.read().then(function (x) {
                if (stop) return;
                if (x.done) return again();
                buf += dec.decode(x.value, { stream: true });
                var at;
                while ((at = buf.indexOf('\n\n')) >= 0) {
                  var ev = buf.slice(0, at); buf = buf.slice(at + 2);
                  var m = /^data: (.*)$/m.exec(ev);
                  if (/^event: changed$/m.test(ev) && m) tell(m[1]);
                }
                pump();
              }, again);
            })();
          }, again);
        }
        open();
        return function () {
          stop = true;
          clearTimeout(wait);
          if (reader) reader.cancel().catch(function () {});
        };
      },
      signOut: function () {
        return call('/v2/signout', { method: 'POST' }).catch(function () {}).then(forget);
      }
    };
  }

  /* ── the flow ──────────────────────────────────────────────────────────── */
  function signIn(o) {
    var verifier = random(32), state = random(16);
    sessionStorage.setItem('wallflowers.flow', JSON.stringify({ verifier: verifier, state: state, client: o.client }));
    return sha256(verifier).then(function (challenge) {
      location.assign(DOOR + '/signin?' + new URLSearchParams({
        client: o.client, redirect_uri: o.callback,
        code_challenge: challenge, code_challenge_method: 'S256', state: state
      }));
    });
  }

  function finish(o) {
    var q = new URLSearchParams(location.search);
    var flow = JSON.parse(sessionStorage.getItem('wallflowers.flow') || 'null');
    sessionStorage.removeItem('wallflowers.flow');
    history.replaceState(null, '', location.pathname);           // the code, off the address
    if (!flow || flow.client !== o.client || q.get('state') !== flow.state) {
      return Promise.reject(new Error('This sign-in was not started here.'));
    }
    if (q.get('error')) return Promise.resolve(null);             // cancelled at the Door
    var code = q.get('code');
    return subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify'])
      .then(function (keys) {
        return proof(keys, 'POST', DOOR + '/v2/token').then(function (p) {
          return fetch(DOOR + '/v2/token', {
            method: 'POST', credentials: 'omit',
            headers: { 'content-type': 'application/json', dpop: p },
            body: JSON.stringify({ code: code, code_verifier: flow.verifier, client: o.client, redirect_uri: o.callback })
          });
        }).then(function (r) { return r.ok ? r.json() : refusal(r); })
          .then(function (t) {
            var s = { client: o.client, site: t.scope, token: t.access_token, expires: Date.now() + t.expires_in * 1000, keys: keys };
            return keep(s).then(function () { return session(s); });
          });
      });
  }

  function current(o) {
    return read().then(function (s) {
      if (!s || s.client !== o.client) return null;
      if (s.expires <= Date.now()) return forget().then(function () { return null; });
      return session(s);
    }, function () { return null; });
  }

  root.WallFlowers = { signIn: signIn, finish: finish, current: current };
})(window);
