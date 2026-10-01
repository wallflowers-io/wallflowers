/* DR-4, the sign-in window (mdr/door.md §4). The passkey's PRF output leaves this
   page only sealed to the session process that asked for it (seal.js, SEC-37).
   The recovery words are shown once, after the account exists, and never sent
   back (SEC-6). */
(function () {
  'use strict';
  var S = window.WallFlowersSeal;
  /* Each step a person waits on, timed in this tab (perf.js); a stub where it did not load. */
  var T = window.WallFlowersPerf || { begin: function () { return null; }, end: function () {}, shown: function () {} };
  var rpId = document.querySelector('meta[name="rp-id"]').content;
  var WRAP = new TextEncoder().encode('pacific/wrap/v1');   // wrap.rs WRAP_DOMAIN
  var params = new URLSearchParams(location.search);
  var back = window.returnPath(params.get('return'), location.origin);   // return-path.js
  /* A PUBLIC SITE'S SIGN-IN (RD.4): the Door answers with a redirect to the
     callback that site registered, carrying a code; the page follows it as is. */
  var client = params.get('client') ? {
    client: params.get('client'), redirect_uri: params.get('redirect_uri'),
    code_challenge: params.get('code_challenge'), state: params.get('state')
  } : undefined;
  /* A DEVICE KNOWN HERE (Ralph, 30 Sep: one prompt): once a passkey has signed in or been made
     in this browser, a Site's window asks it at once; before, it asks a name. Only that this
     browser has been here: no account, no key. */
  var KNOWN = 'wallflowers.known';
  function known() { try { return window.localStorage.getItem(KNOWN) === '1'; } catch (e) { return false; } }
  function land(done) {
    try { window.localStorage.setItem(KNOWN, '1'); } catch (e) {}
    location.replace(done && done.redirect || back);
  }
  var pending = null;
  /* A Site's new member's name, given on the one page with the passkey (O-77's card on entry;
     the user, 30 Sep: the name on the one page): the account is made under it (/v2/signup's
     name). WallFlowers' own window does not ask it. */
  var NAME = null;

  function $(id) { return document.getElementById(id); }

  function answer(r) {
    return r.text().then(function (t) {
      if (!r.ok) {
        var e = new Error(t || String(r.status));
        e.status = r.status;
        throw e;
      }
      return t ? JSON.parse(t) : null;
    });
  }

  function post(path, body) {
    return fetch(path, {
      method: 'POST', credentials: 'same-origin',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body || {})
    }).then(answer);
  }

  /* D-55: the work a start pays, when the Door says one is due, solved in a Worker so
     the page stays live. The Door keeps a challenge 120 s; one begun at load
     and older than 90 s is not used. */
  var ready = {};
  function solve(kind) {
    return fetch('/v2/work?for=' + kind, { credentials: 'same-origin' }).then(answer).then(function (o) {
      if (!o.challenge) return undefined;
      return new Promise(function (ok, no) {
        var w = new Worker('/door/pow-worker.js');
        w.onmessage = function (e) { w.terminate(); ok({ challenge: o.challenge, nonce: e.data.nonce }); };
        w.onerror = function (e) { w.terminate(); no(new Error(e.message || 'pow-worker.js failed')); };
        w.postMessage({ challenge: o.challenge });
      });
    });
  }
  function prepare(kind) {
    ready[kind] = { at: Date.now(), work: solve(kind) };
    ready[kind].work.catch(function () {});
  }
  function work(kind) {
    var r = ready[kind];
    delete ready[kind];
    return r && Date.now() - r.at < 90000 ? r.work : solve(kind);
  }
  /* A start, with its work. Owed none when asked, and owed some by the time it
     starts (the pool crossed half, or the join expired): asked again, once. */
  var OWED = 'this start needs its proof of work';   // main.rs
  function begin(kind, path, extra) {
    var t = T.begin();
    var body = function (w) { var b = { work: w }; for (var k in extra || {}) b[k] = extra[k]; return b; };
    return work(kind).then(function (w) { T.end(kind + ':work', t); return post(path, body(w)); }).catch(function (e) {
      if (e.message !== OWED) throw e;
      return solve(kind).then(function (w) { return post(path, body(w)); });
    }).then(function (a) { T.end(kind + ':start', t); return a; });
  }

  function say(text) { $('status').textContent = text || ''; $('status').hidden = !text; }
  function busy(on) { ['in', 'new', 'saved', 'deny', 'done'].forEach(function (id) { $(id).disabled = on; }); }
  function fail(e) { say(e && e.message || String(e)); busy(false); if (client) offer(); }
  function random(n) { return crypto.getRandomValues(new Uint8Array(n)); }

  /* `allow` names the credential; nothing, and the person chooses. `quiet`, an AbortSignal: the
     passkeys offered in the name field (conditional mediation), no prompt until one is picked. */
  function assert(allow, quiet) {
    var req = { challenge: random(32), rpId: rpId, userVerification: 'required',
                extensions: { prf: { eval: { first: WRAP } } } };
    if (allow) req.allowCredentials = [{ type: 'public-key', id: allow }];
    return navigator.credentials.get(quiet ? { publicKey: req, mediation: 'conditional', signal: quiet } : { publicKey: req });
  }

  function prfOf(assertion) {
    var ext = assertion.getClientExtensionResults();
    if (!ext.prf || !ext.prf.results || !ext.prf.results.first) throw new Error('This passkey has no PRF.');
    return new Uint8Array(ext.prf.results.first);
  }

  /* A prompt the person dismissed, or one the platform would not show (no passkey here, or
     no tap to show it on): not a failure to report. */
  function dismissed(e) { return !!e && (e.name === 'NotAllowedError' || e.name === 'AbortError'); }

  /* `allow`: the credential to sign in with; none, and the person chooses. `quiet`: asked
     unbidden, as a Site's window asks a known device at once; dismissed, a Site's window shows
     the name, for a passkey made here. `picked`: a passkey already picked in the name field. */
  /* THE PASSKEY FIRST, THEN THE ATTEMPT (TEST, run 110): the PRF is read and checked before an
     attempt is opened, so a PRF the check refuses opens none, and a retry never meets the Door's
     429 in place of the sentence; and the attempt's minute starts after the prompt, not before
     it. The challenge is this window's own, so nothing ties the prompt to the attempt. */
  function signIn(allow, quiet, picked) {
    busy(true); say(''); withdraw();
    var attempt, handle, prf, t0 = T.begin(), t = T.begin();
    (picked ? Promise.resolve(picked) : assert(allow || null))
      .then(function (g) {
        T.end('signin:passkey', t); t = T.begin();
        /* The handle, then nothing more of the assertion is kept. */
        handle = S.b64u(g.response.userHandle);
        prf = prfOf(g);
        return verify(g.rawId, prf);
      })
      .then(function () { return begin('signin', '/v2/signin'); })
      .then(function (a) { attempt = a; t = T.begin(); return S.seal(attempt.key, prf, attempt.attempt); })
      .then(function (s) { prf.fill(0); T.end('signin:seal', t); return s; }, function (e) { if (prf) prf.fill(0); throw e; })
      .then(function (sealed) {
        t = T.begin();
        return finish({ attempt: attempt.attempt, handle: handle, sealed: sealed, client: client }, 3);
      })
      .then(function (done) { T.end('signin:finish', t); T.end('signin', t0); land(done); }, function (e) {
        if (!quiet || !dismissed(e)) return fail(e);
        busy(false);
        if (client) { $('name').hidden = false; offer(); }
      });
  }

  /* A PASSKEY THIS BROWSER ALREADY HOLDS (synced from another device, or made before the device
     was known here), offered in the name field where the platform can: picked, it signs in, and
     no second account is made. Withdrawn before any other passkey request, which the platform
     would otherwise refuse. */
  var offered = null;
  function offer() {
    withdraw();
    var P = window.PublicKeyCredential;
    if (!P || typeof P.isConditionalMediationAvailable !== 'function' || typeof AbortController !== 'function') return;
    var c = offered = new AbortController();
    Promise.resolve(P.isConditionalMediationAvailable()).then(function (can) {
      if (!can || offered !== c) return;
      return assert(null, c.signal).then(function (g) {
        if (offered !== c) return;
        offered = null;
        signIn(null, false, g);
      });
    }).catch(function () { if (offered === c) offered = null; });
  }
  function withdraw() { var c = offered; offered = null; if (c) c.abort(); }

  /* A person's process that cannot open for now answers 503 and keeps the attempt:
     the same proof is sent again, after 1, 2 and 3 s, inside the attempt's minute. */
  function finish(body, left) {
    return post('/v2/signin/finish', body).catch(function (e) {
      if (e.status !== 503 || !left) throw e;
      return new Promise(function (ok) { setTimeout(ok, 1000 * (4 - left)); }).then(function () { return finish(body, left - 1); });
    });
  }

  /* ONE TAP (Ralph, 29 Sep: "immediately taking you to the passkey"): the account's start,
     then the passkey at once. Where the platform will not ask without a tap of its own (the
     one that started this was spent on the network), or the prompt was dismissed, the
     passkey's own button is shown. */
  function signUp() {
    if (client) {
      NAME = ($('name').value || '').trim();
      if (!NAME) { say('Your name'); $('name').focus(); return; }
    }
    busy(true); say(''); withdraw();
    var t = T.begin();
    begin('signup', '/v2/signup', client ? { name: NAME } : undefined).then(function (p) {
      pending = p;
      T.shown('signup:open', t);
      enrol(true);
    }, fail);
  }

  /* ONE PAGE FOR A SITE (the user, 30 Sep: "exactly one app.wallflowers.io page, which serves
     only the passkey. Egregore branded screen, and immediately back"; recovery words dropped on
     this path, the user's ruling). The account made, straight on to the Site's code, in its Face.
     MADE, NOT SAVED (NC-89): no words either; the passkey just made signs in, which opens the
     account afresh, and goes back the same way. */
  function onward(done, made) {
    done.words = '';
    if (done.saved === false) return signIn(made.rawId);
    if (!done.continue) return land(done);
    var t = T.begin();
    return post('/v2/signup/continue', { continue: done.continue }).then(function (d) { T.end('signup:continue', t); land(d); }, fail);
  }

  /* The words, once the account they recover exists. MADE, NOT SAVED (NC-89): the Door
     could not keep it, and says so with `saved: false`. The words are shown all the same,
     with one next step, a sign-in with the passkey just made: never a second create(), and
     nothing here times out while they are copied. The words are in the window's own look:
     a Site's Face comes off for them. */
  function show(done, made) {
    document.documentElement.removeAttribute('data-face');
    var list = $('list');
    list.textContent = '';
    (done.words || '').split(' ').forEach(function (w) {
      var li = document.createElement('li');
      li.textContent = w;
      list.appendChild(li);
    });
    done.words = '';
    $('start').hidden = true;
    $('words').hidden = true;
    $('shown').hidden = false;
    if (done.saved === false) {
      say('Your account is made and not yet saved.');
      $('done').textContent = 'Sign in with this passkey';
      $('done').onclick = function () { signIn(made.rawId); };
      $('done').focus();
      return;
    }
    // A site's code is minted when the words are kept, not before: a minute is too
    // short to write twenty-four words down (NC-53).
    $('done').onclick = function () {
      list.textContent = '';
      if (!done.continue) return land(done);
      busy(true);
      var t = T.begin();
      post('/v2/signup/continue', { continue: done.continue }).then(function (d) { T.end('signup:continue', t); land(d); }, fail);
    };
    $('done').focus();
  }

  /* NO PRF, NO ACCOUNT (NC-83). A passkey whose platform says at create() that it holds
     no PRF, or says nothing, or whose get() gives none, can never open the account: it
     is refused before the finish, so no account exists, and the platform is told the
     credential is unknown here so it can drop the orphan (where it can be told). */
  var NO_PRF = 'No account was made: this passkey has no PRF.';
  function noPrf() { var e = new Error(NO_PRF); e.noPrf = true; return e; }
  function forget(cred) {
    try {
      var P = window.PublicKeyCredential;
      if (P && typeof P.signalUnknownCredential === 'function') {
        Promise.resolve(P.signalUnknownCredential({ rpId: rpId, credentialId: S.b64u(cred.rawId) })).catch(function () {});
      }
    } catch (e) {}
  }

  function hex(b) { return Array.prototype.map.call(new Uint8Array(b), function (x) { return (x < 16 ? '0' : '') + x.toString(16); }).join(''); }
  /* A byte view of what the platform gave (an ArrayBuffer, or a view), so zeroing it zeroes the original. */
  function bytes(b) { return ArrayBuffer.isView(b) ? new Uint8Array(b.buffer, b.byteOffset, b.byteLength) : new Uint8Array(b); }

  /* create()'s PRF ONLY WHERE IT IS WITNESSED (ASSURANCE's (ii), 30 Sep): a PRF result from create()
     makes the account only for a provider TEST has watched open that account again, in the browser
     it watched it in; anywhere else one get() reads the PRF, and create()'s is zeroed unused. The
     provider is the passkey's AAGUID, in its authenticator data (flags at 32, AT 0x40; the AAGUID
     at 37-53). A pair joins this list once it is witnessed. */
  var WITNESSED = [
    ['fbfc3007154e4ecc8c0b6e020557d7bd', function (ua) { return ios() || macSafari(ua); }],   // iCloud Keychain, in WebKit
    ['ea9b8d664d011d213ce4b6b48cb575d4', desktopChrome]                                       // Google Password Manager
  ];
  function macSafari(ua) { return /Macintosh/.test(ua) && /Version\/[\d.]+ Safari\//.test(ua) && !/Chrome|Chromium|Firefox|Edg|OPR/.test(ua); }
  function desktopChrome(ua) { return /Chrome\/\d/.test(ua) && !/Android|Mobile|Edg\/|OPR\/|SamsungBrowser|YaBrowser/.test(ua); }
  function witnessed(cred) {
    var d;
    try { d = new Uint8Array(cred.response.getAuthenticatorData()); } catch (e) { return false; }
    if (d.length < 53 || !(d[32] & 0x40)) return false;
    var id = hex(d.subarray(37, 53)), ua = navigator.userAgent || '';
    return WITNESSED.some(function (w) { return w[0] === id && w[1](ua); });
  }

  /* A CHECK ON create()'s PRF (ASSURANCE, 30 Sep): where the account was made with create()'s PRF,
     a check value of it is kept on this device under the passkey's id, and the first get() here
     compares its own, so a witnessed provider that later changes its PRF fails by name, before
     anything is sent. A hash of the PRF under its own domain, never the PRF; dropped once a get()
     agrees. On any other device the Door's wrap, an AEAD under the PRF, refuses a wrong one. */
  var CHECK = 'wallflowers.prf-check.';
  var CHECK_DOMAIN = new TextEncoder().encode('pacific/prf-check/v1\0');
  var PRF_CHANGED = 'This passkey no longer gives the key it was made with, so nothing was opened.';
  function checkOf(prf) {
    var b = new Uint8Array(CHECK_DOMAIN.length + prf.length);
    b.set(CHECK_DOMAIN); b.set(prf, CHECK_DOMAIN.length);
    return crypto.subtle.digest('SHA-256', b).then(function (h) { b.fill(0); return hex(new Uint8Array(h).subarray(0, 16)); });
  }
  function verify(rawId, prf) {
    var key = CHECK + hex(rawId), kept = null;
    try { kept = window.localStorage.getItem(key); } catch (e) {}
    if (!kept) return Promise.resolve();
    return checkOf(prf).then(function (c) {
      if (c !== kept) { var e = new Error(PRF_CHANGED); e.prfChanged = true; throw e; }
      try { window.localStorage.removeItem(key); } catch (e) {}
    });
  }

  /* ON AN iPHONE OR iPAD, THE DEVICE'S OWN PASSKEY (NC-84). With no attachment WebKit
     also builds a security-key request and, for `prf`, calls setPrf: on it, an iOS 26.4
     API: Safari on iOS 18.7.10 crashes in create() (WebKit bug 324148). 'platform'
     leaves that request out. Only here: elsewhere it would take away a desktop's passkey
     on a phone over a QR code, and Windows without Hello. On an iPhone or iPad the device
     is the platform, and what it gives up is a hardware security key. Every iOS browser
     is WebKit; an iPad asks as a Mac with a touch screen. Not by version: Safari on iOS
     26 reports its OS as 18, and the other iOS browsers carry no Safari version. */
  function ios() {
    var ua = navigator.userAgent || '';
    return /iPhone|iPad|iPod/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1);
  }

  /* ONE PROMPT (Ralph, 30 Sep: "Creating the passkey should sign them in"): the PRF is asked
     inside create(), and read there where the platform gives it and the provider is witnessed;
     elsewhere one get() reads it. */
  function enrol(first) {
    var p = pending, made = null, check = null, t0 = T.begin(), t = t0;
    busy(true); say(''); withdraw();
    navigator.credentials.create({ publicKey: {
      challenge: random(32),
      rp: { id: rpId, name: 'WallFlowers' },
      user: { id: S.unb64u(p.handle), name: p.pk.replace(/^ed25519:/, '').slice(0, 8) + '@' + p.host, displayName: NAME || 'WallFlowers account' },
      pubKeyCredParams: [{ type: 'public-key', alg: -8 }, { type: 'public-key', alg: -7 }, { type: 'public-key', alg: -257 }],
      authenticatorSelection: ios()
        ? { authenticatorAttachment: 'platform', residentKey: 'required', userVerification: 'required' }
        : { residentKey: 'required', userVerification: 'required' },
      attestation: 'none',
      extensions: { prf: { eval: { first: WRAP } } }
    } })
      .then(function (cred) {
        made = cred;
        var ext = (cred.getClientExtensionResults() || {}).prf;
        var given = ext && ext.results && ext.results.first ? bytes(ext.results.first) : null;
        if (given && witnessed(cred)) {
          return checkOf(given).then(function (c) { check = c; return given; });
        }
        if (given) given.fill(0);
        if (!ext || ext.enabled !== true) throw noPrf();
        return assert(cred.rawId).then(function (g) { try { return prfOf(g); } catch (e) { throw noPrf(); } });
      })
      .then(function (prf) {
        T.end('signup:passkey', t); t = T.begin();
        return S.seal(p.key, prf, p.attempt).then(function (s) { prf.fill(0); T.end('signup:seal', t); return s; });
      })
      .then(function (sealed) { t = T.begin(); return post('/v2/signup/finish', { attempt: p.attempt, sealed: sealed, client: client }); })
      .then(function (done) {
        T.end('signup:finish', t);
        pending = null;
        if (check) { try { window.localStorage.setItem(CHECK + hex(made.rawId), check); } catch (e) {} }
        if (client) return onward(done, made);
        busy(false);
        show(done, made);
        T.shown('signup:words', t0);
      }, function (e) {
        if (e && e.noPrf && made) forget(made);
        if (first === true && !made && dismissed(e)) {
          busy(false);
          $('start').hidden = true;
          $('words').hidden = false;
          $('saved').focus();
          return;
        }
        fail(e);
      });
  }

  $('in').onclick = function () { signIn(); };
  $('new').onclick = signUp;
  $('saved').onclick = function () { enrol(false); };
  /* Drawn disabled: a tap before this line would find no handler (UX-B, 29 Sep). */
  ['in', 'new', 'saved'].forEach(function (id) { $(id).disabled = false; });

  /* A SITE'S SIGN-IN, ONE STEP, ONE BUTTON (Ralph, 29 and 30 Sep): the Site's Face and name on
     the one screen the passkey is asked from (§5 step 3's consent). SIGN IN, which makes a
     passkey if there is none: a device known here is asked its passkey at once; a device new
     here gives a name, and SIGN IN makes the passkey under it, the passkeys it already holds
     offered in the name field. A known device's SIGN IN with no name asks its passkey again.
     Cancel goes back to the callback the Door checked, with the state. */
  if (client) {
    prepare('signin');
    prepare('signup');
    $('in').textContent = 'SIGN IN';
    $('saved').textContent = 'SIGN IN';
    $('in').onclick = function () {
      if (known() && !($('name').value || '').trim()) return signIn();
      signUp();
    };
    $('new').hidden = true;
    $('deny').hidden = false;
    $('deny').onclick = function () {
      var sep = client.redirect_uri.indexOf('?') < 0 ? '?' : '&';
      location.replace(client.redirect_uri + sep + 'error=access_denied&state=' + encodeURIComponent(client.state));
    };
    $('deny').disabled = false;
    if (known()) { $('in').focus(); signIn(null, true); } else { $('name').hidden = false; $('name').focus(); offer(); }
  } else if (params.has('new')) {
    /* A first-timer's window: Create an account is the one main button, and first; Sign in stays,
       quiet, for someone who has an account (MANAGE, 29 Sep). */
    $('new').className = 'primary';
    $('in').className = 'quiet';
    $('start').insertBefore($('new'), $('in'));
    prepare('signup');
    $('new').focus();
  } else $('in').focus();
  T.shown('first-render', 0);
})();
