/* pacific-account.js — the browser half of the account contract.
 *
 * THE SEED IS THE ROOT. 32 bytes the person owns. A passkey WRAPS it; a passkey
 * is never the account. The arc authenticates the identity public key by
 * challenge–response and never sees the passkey at all. Everything below is in
 * service of those three sentences.
 *
 * WHY THIS FILE EXISTS. The flow was written inline in the test page, and then
 * two more surfaces needed it: kenjin.cc's signup, and the keyholder that holds
 * the seed for an interior embedded in a member's own site. Three inline copies
 * of a key derivation is how two of them drift and nobody notices until a person
 * cannot sign in on their second device. So it lives once, and is STAGED beside
 * the wasm it calls (see stage.sh) rather than imported across origins — the
 * keyholder is deliberately its own origin and can fetch nothing from the site.
 *
 * WHAT IS NOT HERE: UI, storage, and routing. A surface decides where a seed
 * rests and what a person sees; this decides what the bytes are. The one
 * exception is `forget`, because zeroing a seed is a correctness matter rather
 * than a presentation one.
 *
 * ES5 on purpose. The keyholder and the gated site pages are both plain scripts
 * with no build step between them and the browser.
 */
(function (root) {
  'use strict';

  /* ── where the Arc is ───────────────────────────────────────────────────
     ONE RULE, NOT THREE CONSTANTS. The Arc moved to its own hostname on 18 Sep
     2026, which means the page's origin and the Arc's origin are different
     strings for the first time — and THREE separate things are downstream of
     getting that right:

       · the audience inside every signature (`expectedAudience` below),
       · the host bound into the wrap's AAD, which decides whether a wrap opens,
       · and which origin the fetches go to at all.

     Three copies of one hostname is how two of them drift, and the failure is
     silent in the worst way: a wrap sealed to the page's host instead of the
     Arc's still opens, because both halves use the same wrong value — right up
     until the account is carried to a device that does it correctly.

     LOCALHOST IS SAME-ORIGIN, deliberately. The gatetest stack proxies /auth/
     through its own nginx, so there is no second origin there and `''` is not a
     placeholder — it is the truth on that surface. It is also what keeps the
     audience check meaningful rather than something to switch off locally. */
  function hostOf(origin) {
    var a = document.createElement('a');
    a.href = origin;
    return a.host;
  }

  function arcOriginFor(hostname) {
    return hostname === 'localhost' || hostname === '127.0.0.1'
      ? '' : 'https://arc.wallflowers.io';
  }

  Account.arcOrigin = function () { return arcOriginFor(location.hostname); };

  /* ── the core's own constants ───────────────────────────────────────────
     Drift here is SILENT: the bytes simply stop matching what pacific-core
     derives, on a surface that still looks like it is working. Each of these
     has exactly one definition in Rust, named in the comment beside it. */
  var SEED_SALT   = 'pacific/identity/seed/v1';     // identity.rs SEED_SALT
  var SEED_INFO   = 'pacific/identity/current/v1';  // identity.rs SEED_INFO_CURRENT
  var WRAP_DOMAIN = 'pacific/wrap/v1';              // wrap.rs WRAP_DOMAIN, and the PRF salt
  var AUTH_DOMAIN = 'pacific-auth:v1';              // identity.rs auth_payload
  var KEY_PREFIX  = 'ed25519:';                     // identity.rs IDENTITY_KEY_PREFIX

  /* The fixed 16-byte PKCS#8 wrapper around a raw Ed25519 seed. WebCrypto has
     no "import raw private key" for this curve, so the raw scalar is dressed as
     PKCS#8 to get it in. */
  var PKCS8_ED25519 = '302e020100300506032b657004220420';

  var enc = new TextEncoder();
  var dec = new TextDecoder();

  function hex(b) {
    return Array.prototype.map.call(new Uint8Array(b), function (x) {
      return ('0' + x.toString(16)).slice(-2);
    }).join('');
  }
  function unhex(s) {
    var out = new Uint8Array(s.length / 2);
    for (var i = 0; i < out.length; i++) out[i] = parseInt(s.substr(i * 2, 2), 16);
    return out;
  }
  function b64urlToBytes(s) {
    s = s.replace(/-/g, '+').replace(/_/g, '/');
    var raw = atob(s + '==='.slice((s.length + 3) % 4));
    var out = new Uint8Array(raw.length);
    for (var i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
    return out;
  }

  /* ── the wasm ───────────────────────────────────────────────────────────
     pacific-core compiled for the browser. This module does NOT reimplement the
     wrap: XChaCha20-Poly1305 is not in WebCrypto, and a second implementation of
     the one thing that must never drift is the wrong kind of saving.

     The imports are supplied by hand rather than by the wasm-bindgen glue,
     because the glue assumes a module loader and these are plain scripts. Three
     imports, all trivial; `pacific_fill_random` is the one that matters — it is
     the ONLY way randomness enters the module (see the getrandom guard in
     build-wasm.sh). */
  var HOSTFNS = {
    pacific_fill_random: function (at) {
      return function (ptr, len) {
        crypto.getRandomValues(new Uint8Array(at().exports.memory.buffer, ptr, len));
      };
    },
    __wbindgen_init_externref_table: function (at) {
      return function () {
        var t = at().exports.__wbindgen_externrefs, off = t.grow(4);
        t.set(0, undefined);
        t.set(off + 0, undefined); t.set(off + 1, null);
        t.set(off + 2, true); t.set(off + 3, false);
      };
    },
    date_now: function () { return function () { return Date.now(); }; }
  };

  function Account(opts) {
    opts = opts || {};
    /* Where the module is staged. Every consumer stages it beside itself, so a
       relative path is right by default and overridable for the odd harness. */
    this.corePath = opts.corePath || 'core/core_wasm_bg.wasm';
    /* Where the arc's /auth routes are. Same-origin in production; a full origin
       in the local harness, where the service is on another port. */
    /* The passkey's RP ID — wallflowers.io, since 15 Sep 2026.
       A CREDENTIAL IS BOUND TO THIS FOR LIFE. Not for the session, not until the
       next deploy: a passkey minted under one RP ID can never be presented under
       another, and there is no migration. So this is the one constant in the
       system that must be right BEFORE anybody makes an account, and changing it
       strands every credential made before the change.
       It is the REGISTRABLE DOMAIN and not the origin, so wallflowers.io covers
       dev.wallflowers.io, www, and every other subdomain — a credential made on
       the dev host is the same credential on the apex.
       It read `kenjin.cc` until 15 Sep 2026, which was right while kenjin.cc was
       the deployment target and is now the opposite of right: kenjin.cc is being
       retired as a domain we control, and a credential bound to a domain somebody
       else may one day hold is worse than no credential. `docs/signup-plan.html`
       D2 ("one RP ID… kenjin.cc for every user") is superseded by that decision,
       not by a change of mind about having one RP ID — there is still exactly one.
       localhost stays a special case: WebAuthn permits it as a secure context, and
       a localhost credential is deliberately STRANDED — it can never be presented
       at wallflowers.io and no second device will ever see it. That makes a
       localhost run useless for testing interdevice sync, which is what
       `site/auth/dev/live.sh` exists to avoid. */
    this.rpId = opts.rpId
      || (location.hostname === 'localhost' ? 'localhost' : 'wallflowers.io');
    /* The Arc, unless a caller names one. `''` means same-origin. */
    this.authOrigin = opts.authOrigin !== undefined
      ? opts.authOrigin : arcOriginFor(location.hostname);
    /* What the wrap is bound to in its AAD: THE ARC THAT SERVES IT, which since
       18 Sep 2026 is not the page's host. Taking `location.host` here was right
       only while the two were the same string. */
    this.host = opts.host
      || (this.authOrigin ? hostOf(this.authOrigin) : location.host);
    /* What the passkey SAVE PROMPT calls this. The person reads it in a system
       sheet at the one moment they are deciding whether to trust us, so it is
       the product's name and not the codebase's: 'wallflowers' since 15 Sep 2026.
       (Pacific stays the name of the protocol and the iOS bundle id.) */
    this.rpName = opts.rpName || 'WallFlowers';
    this._core = null;
  }

  Account.prototype.core = function () {
    var self = this;
    if (this._core) return this._core;
    this._core = fetch(this.corePath)
      .then(function (r) {
        if (!r.ok) {
          throw new Error(self.corePath + ' ' + r.status +
            ' — the core is not staged here; run app/web/account/stage.sh');
        }
        return r.arrayBuffer();
      })
      .then(function (b) { return WebAssembly.compile(b); })
      .then(function (mod) {
        var inst = null, at = function () { return inst; }, imports = {};
        WebAssembly.Module.imports(mod).forEach(function (i) {
          var make = HOSTFNS[i.name];
          if (!make) throw new Error('unsupplied wasm import ' + i.module + '.' + i.name);
          (imports[i.module] = imports[i.module] || {})[i.name] = make(at);
        });
        return WebAssembly.instantiate(mod, imports).then(function (w) {
          inst = w;
          if (w.exports.__wbindgen_start) w.exports.__wbindgen_start();
          return w.exports;
        });
      });
    this._core.catch(function () { self._core = null; });   // a failed load may be retried
    return this._core;
  };

  /* alloc → write → call → read: the ABI every export shares. `erred()` is the
     module's way of saying the out buffer holds a message rather than a result. */
  function callWasm(e, fn, bytes) {
    var ptr = e.alloc(bytes.length);
    new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
    var n = fn(bytes.length);
    if (e.erred()) {
      throw new Error(dec.decode(new Uint8Array(e.memory.buffer, e.out_ptr(), e.out_len())));
    }
    return new Uint8Array(e.memory.buffer, e.out_ptr(), n).slice();
  }

  /* ── identity ───────────────────────────────────────────────────────────
     seed --HKDF--> 32 bytes --Ed25519--> the key the arc knows you by. This is
     WebCrypto rather than the wasm on purpose: HKDF-SHA256 and Ed25519 are both
     native here, and matching RFC 5869 over the same salt and info is exactly
     what pacific-core's `keys_from_seed` does. `verify` below proves it rather
     than asserting it. */
  Account.prototype.identity = function (seed) {
    return crypto.subtle.importKey('raw', seed, 'HKDF', false, ['deriveBits'])
      .then(function (ikm) {
        return crypto.subtle.deriveBits({
          name: 'HKDF', hash: 'SHA-256',
          salt: enc.encode(SEED_SALT), info: enc.encode(SEED_INFO)
        }, ikm, 256);
      })
      .then(function (scalar) {
        var raw = unhex(PKCS8_ED25519);
        var pkcs8 = new Uint8Array(raw.length + 32);
        pkcs8.set(raw, 0);
        pkcs8.set(new Uint8Array(scalar), raw.length);
        return crypto.subtle.importKey('pkcs8', pkcs8, { name: 'Ed25519' }, true, ['sign']);
      })
      .then(function (priv) {
        /* The public half comes back in the JWK; WebCrypto will not hand it over
           any other way for a private key. */
        return crypto.subtle.exportKey('jwk', priv).then(function (jwk) {
          return { priv: priv, pk: hex(b64urlToBytes(jwk.x)) };
        });
      });
  };

  /* The cross-check worth having, and cheap: the core derives an identity from
     the same seed, in Rust, and the two must agree byte for byte. A surface that
     skips this can ship a subtly different derivation and only find out when a
     person's second device derives a stranger. */
  Account.prototype.verify = function (seed, id) {
    return this.core().then(function (e) {
      /* `identity_key`, not `mls_init`: pure, 32 bytes in and 32 out, no group
         state stood up and no intro tag minted as a side effect of asking a
         question about a key. */
      if (hex(callWasm(e, e.identity_key, seed)) !== id.pk) {
        throw new Error('this browser derives a different identity than the core does');
      }
      return id;
    });
  };

  /* ── proving the account to the arc ─────────────────────────────────────
     The arc issues a single-use nonce; the device signs
     `pacific-auth:v1 \n audience \n ed25519:<pk> \n nonce` and sends the
     signature in headers. No cookie, no session, nothing to steal at rest —
     which is why the whole session subsystem could be deleted. */
  Account.prototype.authPayload = function (audience, pk, nonce) {
    return enc.encode(AUTH_DOMAIN + '\n' + audience + '\n' + KEY_PREFIX + pk + '\n' + nonce);
  };

  /* THE HOST THIS DEVICE IS ACTUALLY TALKING TO. The audience is inside the
     signed bytes so that a signature captured by one arc cannot be replayed at
     another — and the arc DECLARES its own audience in the challenge. Signing
     whatever it says makes the whole property rest on the arc being honest about
     its own name: a hostile origin declares `audience: "arc.wallflowers.io"`,
     collects a signature over it, and replays that at the real arc.

     So the device decides what it expects and checks. It was not checkable
     before — while the pages and the arc shared an origin, audience and page
     host were the same string by accident. Giving the arc its own hostname is
     what makes the two separable, and therefore what makes this a real check
     rather than a tautology. */
  Account.prototype.expectedAudience = function () {
    return this.authOrigin ? hostOf(this.authOrigin) : location.host;
  };

  Account.prototype.headers = function (id) {
    var self = this;
    var want = this.expectedAudience();
    return fetch(this.authOrigin + '/auth/challenge')
      .then(function (r) {
        if (!r.ok) throw new Error('the arc issued no challenge (HTTP ' + r.status + ')');
        return r.json();
      })
      .then(function (c) {
        /* REFUSE TO SIGN FOR SOMEBODY ELSE. Nothing downstream can catch this:
           the signature would be perfectly valid, just valid somewhere the
           person never meant to authorise. */
        if (c.audience !== want) {
          throw new Error(
            'this arc calls itself ' + JSON.stringify(c.audience) + ' but it is ' +
            JSON.stringify(want) + ' — refusing to sign, because a signature for ' +
            'another arc is one this arc could replay there');
        }
        return crypto.subtle.sign('Ed25519', id.priv, self.authPayload(c.audience, id.pk, c.nonce))
          .then(function (sig) {
            return { 'X-Pacific-Challenge': c.nonce, 'X-Pacific-Signature': hex(sig) };
          });
      });
  };

  /* ── the passkey ────────────────────────────────────────────────────────
     THE HANDLE CARRIES THE ACCOUNT: `public key ‖ arc host`. Those are exactly
     the two things a fresh device cannot derive for itself — the address IS the
     public key, and the public key is downstream of the seed it is fetching. */
  Account.prototype.createPasskey = function (pk, label) {
    var host = enc.encode(this.host);
    var handle = new Uint8Array(32 + host.length);
    handle.set(unhex(pk), 0);
    handle.set(host, 32);
    return navigator.credentials.create({
      publicKey: {
        /* Not verified by any server: this arc runs no WebAuthn ceremony. The
           passkey is a local unlock, so the challenge is ceremony, not proof. */
        challenge: crypto.getRandomValues(new Uint8Array(32)),
        rp: { id: this.rpId, name: this.rpName },
        user: {
          /* `name` ALWAYS CARRIES THE ARC HOST, and a label never displaces it.
             The authenticator stores this string, syncs it between the owner's
             devices and shows it in the picker, so it is the one human-visible
             cue about WHICH arc a credential belongs to — and a passkey minted
             at one arc does not open an account at another. An earlier version
             let a caller's label replace it, which meant signup.html (passing
             the site's name) minted credentials that said nothing about where
             they worked.

             The local part is the public key's first 8 hex, never anything
             personal: it is already public, it is stable, and it distinguishes
             two accounts in the same picker. The label, if given, is for
             `displayName` only. */
          id: handle,
          name: pk.slice(0, 8) + '@' + this.host,
          displayName: label || 'WallFlowers account'
        },
        pubKeyCredParams: [{ type: 'public-key', alg: -8 },
                           { type: 'public-key', alg: -7 },
                           { type: 'public-key', alg: -257 }],
        authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
        attestation: 'none',
        extensions: { prf: {} }
      }
    });
  };

  /* PRF COMES FROM get(), NEVER create(): several shipping providers return
     nothing at create time and the full 32 bytes at get. Pass `allow` (a rawId)
     to name a credential; pass nothing for the discoverable path, where the
     person chooses and the handle says who they are. */
  Account.prototype.prf = function (allow) {
    var req = {
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      rpId: this.rpId,
      userVerification: 'required',
      extensions: { prf: { eval: { first: enc.encode(WRAP_DOMAIN) } } }
    };
    if (allow) req.allowCredentials = [{ type: 'public-key', id: allow }];
    return navigator.credentials.get({ publicKey: req }).then(function (assertion) {
      var ext = assertion.getClientExtensionResults();
      if (!ext.prf || !ext.prf.results || !ext.prf.results.first) {
        throw new Error('this authenticator returned no PRF — it cannot hold an account');
      }
      return { prf: new Uint8Array(ext.prf.results.first), assertion: assertion };
    });
  };

  /* What a handle says. Anything shorter than 33 bytes is not one of ours: the
     key alone is 32 and the host is at least one character. */
  Account.prototype.readHandle = function (userHandle) {
    var h = new Uint8Array(userHandle);
    if (h.length <= 32) throw new Error('this passkey carries no host in its handle');
    return { pk: hex(h.slice(0, 32)), host: dec.decode(h.slice(32)) };
  };

  /* ── the two artefacts ──────────────────────────────────────────────────
     THE WRAP is the seed under the passkey's PRF, bound to the arc host, and is
     the only thing served to anyone who asks by public key — which is safe
     precisely because its key never leaves an authenticator.
     THE HISTORY (archive + MLS state) is sealed under a SEED-derived key and
     served only against a signature. Splitting them is what lets the wrap be
     public without handing a stranger a person's history. */
  Account.prototype.wrapSeed = function (prf, seed, host) {
    var self = this;
    return this.core().then(function (e) {
      return unhex(dec.decode(callWasm(e, e.wrap_seal, enc.encode(JSON.stringify({
        prf: hex(prf), seed: hex(seed), host: host || self.host
      })))));
    });
  };

  Account.prototype.openWrap = function (prf, blob, host) {
    var self = this;
    return this.core().then(function (e) {
      return unhex(dec.decode(callWasm(e, e.wrap_open, enc.encode(JSON.stringify({
        prf: hex(prf), blob: hex(blob), host: host || self.host
      })))));
    });
  };

  /* ── the head ───────────────────────────────────────────────────────────
     THE ANCHOR, and the third artefact an account has. The wrap says what the
     seed is. The history says what the account held. The head says HOW MUCH OF
     THE CHAIN THERE IS — a position and the hash of the last entry's ciphertext,
     and nothing else, because every field beyond those is a field an arc gets to
     observe the length of.

     WHY IT CANNOT BE SKIPPED. A chain cannot prove it is complete: a prefix of a
     valid chain is a valid chain, so an arc serving three of five entries is
     indistinguishable from an arc that has three. Hash-linking catches
     modification and reordering and is silent about truncation. The head is the
     only thing that catches it.

     SEALED UNDER THE SEED'S STORAGE ROOT, like the history and unlike the wrap.
     The words door has to reach it: someone restoring on a borrowed laptop who has
     never had a passkey is exactly the person most in need of knowing whether they
     were shown everything, and a PRF-derived key would make that a passkey-only
     feature without anything failing to say so. The core derives the root from
     the seed; this file never computes a key.

     HEAD v2 (18 Sep 2026): `chain_gen` names which chain the head anchors, so a
     chain can be restarted at a fresh address; `updated_at` is gone — nothing read
     it, and the arc's own column is what a witness watches. */
  Account.prototype.sealHead = function (seed, head) {
    return this.core().then(function (e) {
      return unhex(dec.decode(callWasm(e, e.head_seal, enc.encode(JSON.stringify({
        seed: hex(seed),
        arc: head.arc,
        position: head.position,
        tail: head.tail || '',          /* empty at position 0: no entries, no tail */
        chain_gen: head.chain_gen
      })))));
    });
  };

  /* `declaredPosition` is the arc's X-Chain-Position header, and it is not
     optional in practice. The arc orders writes on a number it cannot decrypt,
     so the header and the sealed body are two separate claims and a broken or
     dishonest arc can serve position 41 in one and the ciphertext of 17 in the
     other. The core refuses the pair when they disagree; the cost is one
     comparison and it removes the whole class. */
  Account.prototype.openHead = function (seed, blob, declaredPosition) {
    var body = { seed: hex(seed), blob: hex(new Uint8Array(blob)) };
    if (declaredPosition !== null && declaredPosition !== undefined) {
      body.declared_position = declaredPosition;
    }
    return this.core().then(function (e) {
      return JSON.parse(dec.decode(callWasm(e, e.head_open, enc.encode(JSON.stringify(body)))));
    });
  };

  /* Four answers, not two. `head_behind` is NOT an alarm — it is what a device
     that appended and then died before updating its head leaves behind, and the
     response is to advance the head rather than to distrust the arc. Reporting
     it as an attack would train people to ignore the one that is. */
  Account.prototype.checkChain = function (head, walkedPosition, walkedTail) {
    return this.core().then(function (e) {
      return JSON.parse(dec.decode(callWasm(e, e.head_check, enc.encode(JSON.stringify({
        position: head.position, tail: head.tail || '',
        walked_position: walkedPosition, walked_tail: walkedTail || ''
      })))));
    });
  };

  /* Store the head. Signed, and compare-and-set on the position: the arc refuses
     anything that does not move the chain forward and says which of the two
     events it was — `behind` (read the stored one and catch up) or `fork` (two
     writers appended against one predecessor, and neither can be believed). */
  Account.prototype.putHead = function (id, blob, position) {
    var self = this;
    return this.headers(id).then(function (h) {
      h['Content-Type'] = 'application/octet-stream';
      h['X-Chain-Position'] = String(position);
      return fetch(self.authOrigin + '/auth/users/' + id.pk + '/head', {
        method: 'PUT', headers: h, body: blob
      });
    }).then(readJson('the arc refused the head'));
  };

  /* Fetch the head. UNAUTHENTICATED, like the wrap and for a sharper reason: an
     anchor nobody can watch is an anchor an arc can move. This is ciphertext
     under a key the arc has never held, so serving it to anyone costs nothing and
     lets a witness — a monitor, a mirror, the person's own second device — see a
     position go backwards. Null means there is no head yet, which is a state and
     not an error: an account created before this existed has none. */
  Account.prototype.fetchHead = function (pk) {
    return fetch(this.authOrigin + '/auth/users/' + pk + '/head').then(function (r) {
      if (r.status === 404) return null;
      if (!r.ok) throw new Error('could not fetch the head (HTTP ' + r.status + ')');
      var position = r.headers.get('X-Chain-Position');
      var updated = r.headers.get('X-Head-Updated-At');
      return r.arrayBuffer().then(function (blob) {
        return { blob: new Uint8Array(blob),
                 position: position === null ? null : Number(position),
                 updatedAt: updated === null ? null : Number(updated) };
      });
    });
  };

  /* The first head: position 0, no tail. Written at account creation, and by a
     sign-in that finds none — which is how every account made before this
     existed gets one.

     THIS IS NOT BOOKKEEPING. Position 0 is the floor the arc's compare-and-set
     enforces from then on. An account that has never written a head has no floor,
     so the first head it ever holds is whatever reaches the route first, and
     there is nothing to compare that against. */
  Account.prototype.writeFirstHead = function (id, seed) {
    var self = this;
    var head = { arc: this.host, position: 0, tail: '', chain_gen: 0 };
    return this.sealHead(seed, head).then(function (blob) {
      return self.putHead(id, blob, 0).then(function (meta) {
        return { head: head, meta: meta, bytes: blob.length };
      });
    });
  };

  /* Read the anchor, or write the first one. The call a returning device makes
     before it trusts anything it walked.

     A FAILURE HERE IS REPORTED, NOT THROWN. The account already exists by this
     point — the wrap is stored and the seed is in hand — and refusing the sign-in
     because an anchor could not be written would cost the person an account over
     a thing that can be retried on the next visit. So the shape is always
     `{head, wrote, error}` and a caller that does not look at `error` is the one
     being quiet, not this. */
  Account.prototype.anchor = function (id, seed) {
    var self = this;
    return this.fetchHead(id.pk).then(function (got) {
      if (got) {
        return self.openHead(seed, got.blob, got.position).then(function (head) {
          return { head: head, wrote: false, error: null };
        });
      }
      return self.writeFirstHead(id, seed).then(function (a) {
        return { head: a.head, wrote: true, error: null };
      });
    }).catch(function (e) {
      return { head: null, wrote: false, error: e && e.message || String(e) };
    });
  };

  /* ── the words ──────────────────────────────────────────────────────────
     The account's other door, and the one that does not depend on a passkey
     syncing. Whether a platform carries a credential's PRF to a person's next
     device is up to that platform; the words follow the person. */
  Account.prototype.words = function (seed) {
    return this.core().then(function (e) {
      return dec.decode(callWasm(e, e.recovery_words, seed));
    });
  };

  Account.prototype.seedFromWords = function (phrase) {
    var normalised = String(phrase).trim().toLowerCase().replace(/\s+/g, ' ');
    return this.core().then(function (e) {
      return callWasm(e, e.recovery_seed, enc.encode(normalised));
    });
  };

  /* ── the arc's doors ────────────────────────────────────────────────────── */

  /* Store the wrap. Signed, because only the account may write its own. */
  Account.prototype.register = function (id, wrap) {
    var self = this;
    return this.headers(id).then(function (h) {
      h['Content-Type'] = 'application/octet-stream';
      return fetch(self.authOrigin + '/auth/users/' + id.pk, {
        method: 'PUT', headers: h, body: wrap
      });
    }).then(readJson('the arc refused the registration'));
  };

  /* Fetch a wrap. UNAUTHENTICATED, deliberately: this read is how a new device
     gets the key it would otherwise have to authenticate with. */
  Account.prototype.fetchWrap = function (pk) {
    return fetch(this.authOrigin + '/auth/users/' + pk).then(function (r) {
      if (r.status === 404) throw new Error('no account is stored under that key');
      if (!r.ok) throw new Error('could not fetch the wrap (HTTP ' + r.status + ')');
      return r.arrayBuffer();
    });
  };

  Account.prototype.account = function (id) {
    var self = this;
    return this.headers(id).then(function (h) {
      return fetch(self.authOrigin + '/auth/users/' + id.pk + '/account', { headers: h });
    }).then(readJson('the arc refused the signature'));
  };

  function readJson(what) {
    return function (r) {
      return r.json().then(function (body) {
        if (!r.ok) {
          throw new Error((body && body.error ? body.error : what) +
                          (body && body.message ? ' — ' + body.message : ''));
        }
        return body;
      }, function () {
        throw new Error(what + ' (HTTP ' + r.status + ', and the reply was not JSON)');
      });
    };
  }

  /* ── the whole flows ────────────────────────────────────────────────────
     Two calls, because these are the two things a person actually does. Each
     reports progress through `step` so a surface can narrate without knowing
     the order. */

  /* Mint an account. The seed is minted HERE, in the page, and the arc never
     sees it — only 76 bytes it cannot read. */
  Account.prototype.create = function (o) {
    o = o || {};
    var step = o.step || function () {};
    var self = this, seed = crypto.getRandomValues(new Uint8Array(32));
    var id, words;

    step('seed', 'minting a seed — 32 bytes, this device only');
    return this.identity(seed)
      .then(function (got) { id = got; return self.verify(seed, id); })
      .then(function () {
        step('identity', 'identity ' + id.pk.slice(0, 16) + '… · matches the core exactly');
        return self.words(seed);
      })
      .then(function (w) {
        words = w;
        step('passkey', 'creating a passkey — approve the prompt');
        return self.createPasskey(id.pk, o.label);
      })
      .then(function (cred) {
        step('prf', 'asking it for PRF — approve once more');
        return self.prf(cred.rawId);
      })
      .then(function (got) { return self.wrapSeed(got.prf, seed, self.host); })
      .then(function (wrap) {
        step('wrapped', 'seed wrapped under the passkey · ' + wrap.length + ' bytes of ciphertext');
        return self.register(id, wrap);
      })
      .then(function (body) {
        step('registered', 'registered · the arc stored the wrap and nothing it can read');
        /* THE ANCHOR, written while the seed is still in hand and while the arc
           will accept a position 0 — it refuses one once a head is held. Written
           rather than read: `anchor` below probes first because it serves a
           device that may be returning to an account it did not make, and this
           account was made three lines ago.

           The failure is REPORTED rather than thrown, which is why this can sit
           inside this handler without disarming the forget below: by
           this line the account EXISTS, the wrap is stored and the words are
           real, and throwing away all of that over an anchor that the next
           sign-in will write is the expensive way to handle a retryable error.
           An un-anchored account is a real state and it says so. */
        return self.writeFirstHead(id, seed).then(function (a) {
          return { head: a.head, error: null };
        }, function (e) {
          return { head: null, error: e && e.message || String(e) };
        }).then(function (a) {
          step(a.error ? 'unanchored' : 'anchored',
               a.error ? 'THE ACCOUNT IS NOT ANCHORED — ' + a.error +
                         ' · signing in again writes it'
                       : 'anchor written · position 0, the floor every later head ' +
                         'is compared against');
          /* THE SEED IS RETURNED, NOT DESTROYED, and the caller decides. This
             used to call forget(seed) here, which was right for a signup page —
             its job is done the moment the wrap is stored — and silently wrong
             for the keyholder, whose entire purpose is to KEEP the seed: it
             adopted a zeroed one, stored an empty string, and produced an
             account that looked created and could never sign in.

             So all three flows return the seed and none of them destroy it. A
             caller that does not want it calls PacificAccount.forget; a caller
             that does is not quietly robbed of it. `signIn` and `restore` always
             worked this way — `create` was the odd one out. */
          return { id: id, pk: id.pk, seed: seed, words: words, account: body,
                   head: a.head, anchored: !a.error, anchorError: a.error };
        });
      }, function (e) {
        forget(seed);      /* nothing to hand back: the account was not made */
        throw e;
      });
  };

  /* Sign in with a passkey: discoverable, so nothing is typed and no account is
     named. The handle says who, the wrap says what, the signature proves it. */
  Account.prototype.signIn = function (o) {
    o = o || {};
    var step = o.step || function () {};
    var self = this, who, prf;

    step('passkey', 'asking for your passkey — no account named, the credential knows');
    return this.prf(null)
      .then(function (got) {
        prf = got.prf;
        who = self.readHandle(got.assertion.response.userHandle);
        step('handle', 'the handle says ' + who.pk.slice(0, 16) + '… at ' + who.host);
        return self.fetchWrap(who.pk);
      })
      .then(function (blob) { return self.openWrap(prf, blob, who.host); })
      .then(function (seed) {
        step('unwrapped', 'wrap opened · the seed is back on this device');
        return self.identity(seed).then(function (id) {
          if (id.pk !== who.pk) {
            forget(seed);
            throw new Error('the seed does not derive the key it was stored under');
          }
          return { id: id, seed: seed };
        });
      })
      .then(function (got) {
        return self.account(got.id).then(function (body) {
          step('in', 'signed in · the arc verified a signature, not a password and not a cookie');
          /* READ THE ANCHOR, or write it. A device that has just recovered a seed
             is about to walk a chain, and the position it walks to means nothing
             without something to check it against. An account made before heads
             existed has none, and this is where it gets one. */
          return self.anchor(got.id, got.seed).then(function (a) {
            step('anchor', a.error ? 'no anchor — ' + a.error
                 : a.wrote ? 'anchor written · this account had none until now'
                 : 'anchor read · the chain ends at position ' + a.head.position);
            return { id: got.id, pk: got.id.pk, seed: got.seed, host: who.host,
                     account: body, head: a.head, anchored: !a.error,
                     anchorError: a.error, anchorWritten: a.wrote };
          });
        });
      });
  };

  /* Sign in with the words. No passkey is involved at all, which is what makes
     this the door that always opens: after a lost device, on a platform whose
     PRF does not travel, or on someone else's browser. */
  Account.prototype.restore = function (phrase, o) {
    o = o || {};
    var step = o.step || function () {};
    var self = this, id, seed;

    step('words', 'reading your words — the checksum holds or it does not');
    return this.seedFromWords(phrase)
      .then(function (s) {
        seed = s;
        step('checked', 'words check out · they carry the 32-byte seed');
        return self.identity(seed);
      })
      .then(function (got) {
        id = got;
        step('identity', 'identity ' + id.pk.slice(0, 16) + '… · derived from the words alone');
        return self.account(id);
      })
      .then(function (body) {
        step('in', 'restored · no passkey touched, and the arc still only saw a signature');
        /* The same anchor read as the passkey door, and the reason the head key
           comes off the SEED rather than the PRF: this door never touches a
           passkey, and it still has to be able to tell a whole chain from a
           truncated one. */
        return self.anchor(id, seed).then(function (a) {
          step('anchor', a.error ? 'no anchor — ' + a.error
               : a.wrote ? 'anchor written · this account had none until now'
               : 'anchor read · the chain ends at position ' + a.head.position);
          return { id: id, pk: id.pk, seed: seed, account: body, head: a.head,
                   anchored: !a.error, anchorError: a.error, anchorWritten: a.wrote };
        });
      });
  };

  /* Enrol a passkey against a seed already in hand — the second half of a
     words restore, and the way a second device gets its own local unlock.
     ONE WRAP PER ACCOUNT is what makes this revocation as well as enrolment:
     storing a new wrap makes the credential it replaces useless. */
  Account.prototype.enrolPasskey = function (seed, id, o) {
    o = o || {};
    var step = o.step || function () {};
    var self = this;
    step('passkey', 'creating a passkey on this device — approve the prompt');
    return this.createPasskey(id.pk, o.label)
      .then(function (cred) {
        step('prf', 'asking it for PRF — approve once more');
        return self.prf(cred.rawId);
      })
      .then(function (got) { return self.wrapSeed(got.prf, seed, self.host); })
      .then(function (wrap) { return self.register(id, wrap); })
      .then(function (body) {
        step('enrolled', 'this device can now open the account on its own');
        return body;
      });
  };

  /* Zeroing a seed is not theatre, but it is not a guarantee either: a JS engine
     may have copied the bytes during GC and nothing here can reach those. It
     removes the one copy we hold, which is the part we control. */
  function forget(seed) {
    try { seed.fill(0); } catch (e) {}
  }

  Account.hex = hex;
  Account.unhex = unhex;
  Account.forget = forget;
  Account.KEY_PREFIX = KEY_PREFIX;
  Account.WRAP_DOMAIN = WRAP_DOMAIN;

  root.PacificAccount = Account;
})(typeof self !== 'undefined' ? self : this);
