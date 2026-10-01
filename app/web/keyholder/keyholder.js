/* ═══════════════════════════════════════════════════════════════════════════
   THE KEYHOLDER — Pacific's origin, and the only place a device key exists.

   The web client's UI is embedded in the member's own site, which means it runs
   on the member's origin, beside whatever else that site loads. A shadow root
   isolates CSS. It does not isolate scripts: same origin is same origin, and
   any script on that page can reach the same IndexedDB and use the same
   non-extractable CryptoKey handles. Non-extractable stops exfiltration. It
   does not stop USE.

   So the key does not live there. It lives here, on an origin Pacific controls,
   in a document the host page can hold a reference to and never read into. The
   same-origin policy does the work — not a convention, not a wrapper, the
   browser's own boundary. This is the shape Stripe Elements uses for card
   fields, and for the same reason.

   WHAT CROSSES THE BOUNDARY. A MessagePort, handed over once at init, and typed
   requests over it. The port is unforgeable: a script that did not receive it
   cannot obtain one, so authorisation happens exactly once instead of on every
   message. What never crosses: the private key, in any form. There is no method
   that returns it, because a method that returned it would make this file
   decorative.

   WHAT IS STILL MISSING. The origin allowlist below is a constant. In
   production it is the site's registered origins, served from Pacific's own
   directory, because otherwise any page anywhere embeds pacific.js with
   data-site="cambridge-dd" and gets a live session as whoever is signed in.
   That is the whole reason this file authorises at all, so the constant is a
   placeholder for a lookup, not a simplification of one.
   ═══════════════════════════════════════════════════════════════════════════ */
(function () {
  'use strict';

  /* ── who may ask ─────────────────────────────────────────────────────────
     Deny by default. A site handle maps to the origins allowed to embed it —
     the member's own site, and Pacific's own pages for the standalone client. */
  var ALLOWED = {
    'cambridge-dd': ['http://localhost:8100'],
    'stoma':        ['http://localhost:8100', 'http://localhost:8101'],
    'example':    ['http://localhost:8100', 'http://localhost:8102'],
    /* The door's outside consumers, in DEV only, and READ-only: `WRITERS` below does
       not name them, so object.mint and object.author still refuse them until Ralph
       rules on member-site writes. No production origin: thedoor's host is not
       named yet, and a row for one would be a guess. Added 22 Sep 2026. */
    'thedoor':      ['http://localhost:8107'],
    'cyp3':         ['http://localhost:8117']
  };
  /* No production origin. `https://app.wallflowers.io` was listed per site from
     16 Sep 2026; that name is the Door's now (O-53), and a Door page is not a
     keyholder's client. Never a wildcard: a page must be NAMED to ask, or any
     subdomain anyone stands up could. */
  /* The two-device harness drives both keyholders from one page on :8100, so
     that page's origin is what asks on behalf of each. */
  function permitted(site, origin) {
    var list = ALLOWED[site];
    return !!list && list.indexOf(origin) >= 0;
  }

  /* ── the device key ──────────────────────────────────────────────────────
     Ed25519, because that is what the core's identity is: `identity.rs` holds an
     Ed25519 signing key whose public half is the stable peer id, and an MLS leaf
     carries exactly those 32 bytes as its BasicCredential id. So a browser
     device is not a different kind of identity wearing a translation layer — it
     is the same kind, minted here.

     `extractable: false` means the private half cannot be read out by anything,
     including this file. It is stored as a live CryptoKey handle: IndexedDB
     structured-clones CryptoKeys, so the key survives a reload without ever
     having existed as bytes in JavaScript. */
  var DB = 'pacific.keyholder', STORE = 'device', LOG = 'deltas', VER = 3;

  /* WHICH SITE. One device holds many sites' graphs — that is what "a device
     holds many groups" has always meant — so the log and the checkpoint are
     partitioned by site handle while the DEVICE KEY is not: the key is the
     device's, one per origin, shared across every site it carries. Before this,
     two sites on one keyholder origin silently shared a world; the second to
     load simply read the first's. */
  var site_ = null;
  /* The origin that opened this port, for the one question `permitted` does not
     answer: whether it may WRITE through object.* (see WRITERS). */
  var origin_ = null;
  var cpKey = function () { return 'cp:' + site_; };

  function idb() {
    return new Promise(function (res, rej) {
      var r = indexedDB.open(DB, VER);
      r.onupgradeneeded = function () {
        var db = r.result;
        if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE);
        /* Keyed (author, seq) the way `delta_log` is keyed (group_id, delta_id):
           an append-only log per author, so a delta from another device slots in
           beside ours instead of overwriting anything. */
        if (db.objectStoreNames.contains(LOG)) db.deleteObjectStore(LOG);
        db.createObjectStore(LOG, { keyPath: ['site', 'a', 'seq'] });
        /* v1 kept a single folded `world`; v2 kept one log and one checkpoint for
           whatever site got there first. Neither is migrated — a snapshot has no
           authors to turn into deltas, and an unpartitioned log cannot be split
           after the fact because nothing in it says which site it belonged to.
           The seed re-establishes genesis per site on next load. */
        if (db.objectStoreNames.contains(STORE) && r.transaction) {
          var s = r.transaction.objectStore(STORE);
          try { s.delete('world'); s.delete('checkpoint'); } catch (e) {}
        }
      };
      r.onsuccess = function () {
        var db = r.result;
        /* MEASURED, 10 Sep 26: this origin was found holding `pacific.keyholder`
           at version 3 with NO object stores at all — an upgrade that recorded
           its version and not its schema, presumably a run killed inside the
           transaction. Every call after that fails with "one of the specified
           object stores was not found", forever, because the version is already
           current so onupgradeneeded never runs again to repair it.

           A database missing a store it was opened for holds nothing that could
           be read, so deleting it costs nothing and is the only way back. It is
           narrow on purpose: a store that is merely EMPTY is untouched, because
           an empty log is the normal state of a new device and wiping the device
           key over one would be the worse failure by far. */
        if (db.objectStoreNames.contains(STORE) && db.objectStoreNames.contains(LOG)) {
          return res(db);
        }
        console.warn('[keyholder] schema incomplete at v' + db.version +
                     ' — rebuilding: [' + Array.prototype.join.call(db.objectStoreNames, ',') + ']');
        db.close();
        var del = indexedDB.deleteDatabase(DB);
        del.onsuccess = function () { idb().then(res, rej); };
        del.onerror = function () { rej(del.error); };
        del.onblocked = function () { rej(new Error('another tab is holding the keyholder database open')); };
      };
      r.onerror = function () { rej(r.error); };
    });
  }
  /* `fn` returns an IDBRequest and the transaction resolves with its result —
     which is `undefined` for a key that is not there, and that is a real answer
     rather than a missing one. An earlier version tried to be clever about the
     difference and resolved with the IDBRequest itself when the result was
     undefined, so the first load of a fresh device handed an IDBRequest to
     postMessage and the whole mount died on a DataCloneError two origins away
     from the mistake. */
  function req(mode, fn, which) {
    return idb().then(function (db) {
      return new Promise(function (res, rej) {
        var t = db.transaction(which || STORE, mode), r = fn(t.objectStore(which || STORE));
        t.oncomplete = function () { res(r.result); };
        t.onerror = function () { rej(t.error); };
        r.onerror = function () { rej(r.error); };
      });
    });
  }
  var get = function (k) { return req('readonly', function (s) { return s.get(k); }); };
  var put = function (k, v) {
    return req('readwrite', function (s) { return s.put(v, k); }).then(function () { return v; });
  };
  var del = function (k) { return req('readwrite', function (s) { return s.delete(k); }); };

  /* EACH GROUP'S LOG, as this device received it: one row per application
     message, {author, envelope}, exactly what `mls_decrypt` hands back (`sender`
     and `delta`). `object.fold` gives these rows to the core's `fold_object` and
     nothing here decodes a delta. That is the whole reason the row is kept raw.

     WHAT THIS IS AND IS NOT. It is this device's copy on its own origin, the
     same boundary the MLS snapshot already lives behind. It is NOT how the
     content survives: another device, or this one after its storage is cleared,
     gets it back through the archive and the spine, never from here. A row is
     only ever what MLS authenticated on arrival, or what this device itself
     authored and the relay accepted.

     Appends are queued per group, so two messages arriving together cannot
     read the same old list and each write it back without the other. A
     replayed message is the same envelope, so it is not appended twice. */
  var logKey = function (gid) { return 'glog:' + gid; };
  var logQueue = {};
  function appendLog(gid, row) {
    var next = (logQueue[gid] || Promise.resolve()).then(function () {
      return get(logKey(gid)).then(function (rows) {
        rows = Array.isArray(rows) ? rows : [];
        for (var i = 0; i < rows.length; i++) if (rows[i].envelope === row.envelope) return rows;
        rows.push({ author: row.author, envelope: row.envelope });
        return put(logKey(gid), rows);
      });
    });
    logQueue[gid] = next.catch(function () {});
    return next;
  }
  function readLog(gid) {
    return (logQueue[gid] || Promise.resolve()).then(function () { return get(logKey(gid)); })
      .then(function (rows) { return Array.isArray(rows) ? rows : []; });
  }

  var hex = function (b) {
    return Array.prototype.map.call(new Uint8Array(b), function (x) {
      return ('0' + x.toString(16)).slice(-2);
    }).join('');
  };

  /* Mint once, then never again. The public half is exported to bytes — that is
     the peer id and it is meant to travel; the private half never is. */
  function device() {
    return get('self').then(function (rec) {
      if (rec && rec.keys) return rec;
      return crypto.subtle.generateKey({ name: 'Ed25519' }, false, ['sign', 'verify'])
        .then(function (kp) {
          return crypto.subtle.exportKey('raw', kp.publicKey).then(function (raw) {
            var fresh = {
              keys: kp,
              pk: new Uint8Array(raw),
              created: Date.now(),
              /* A device says what it is so a member can recognise it in the list
                 they are about to remove something from. */
              agent: navigator.userAgent.indexOf('Mac') >= 0 ? 'browser · mac' : 'browser'
            };
            return put('self', fresh);
          });
        });
    });
  }

  /* ── the log ─────────────────────────────────────────────────────────────
     What the browser holds is a FEED, not a fold. It used to hold a fold: every
     commit mutated one `world` blob and threw the op away, which is the thing
     `membership.rs` condemns in its own domain — "a correct cache and a terrible
     history… no author, no timestamp, no reason, and nothing for any consumer to
     observe". It is also unmergeable: two replicas of one graph cannot converge
     from snapshots, because the last writer overwrites the other's whole world.

     So a commit APPENDS. The world is derived, and derived cheaply, because a
     checkpoint sits under the tail — the same `.feed` / `.state` split the object
     model already has, for the same reason: 20k deltas replay in ~370 ms here,
     a million would take ~19 s, and start-up cannot be allowed to become that.

     ORDERING IS A PLACEHOLDER AND IS MARKED AS ONE. Deltas fold in
     (at, author, seq) order — deterministic, so two replicas holding the same
     set agree, which is the property that matters today. It is NOT the core's
     ordering: owner-sequenced ops belong on a spine and commutative ops carry
     their own LWW key. `coordination/delta-fold.icd.json` is where the real rule
     gets pinned, and this function is what it will pin. */

  /* DeltaId is sha256 over canonical bytes in the core. Here it is sha256 over
     key-sorted JSON — the same idea with a different serialiser, and it must
     become canonical CBOR before an id crosses the wire, or the two sides will
     compute different ids for the same delta. */
  function canon(v) {
    if (v === null || typeof v !== 'object') return JSON.stringify(v);
    if (Array.isArray(v)) return '[' + v.map(canon).join(',') + ']';
    return '{' + Object.keys(v).sort().map(function (k) {
      return JSON.stringify(k) + ':' + canon(v[k]);
    }).join(',') + '}';
  }
  function deltaId(d) {
    return crypto.subtle.digest('SHA-256', new TextEncoder().encode(canon(d))).then(hex);
  }

  /* IDBKeyRange over the compound key: everything from [site] to [site, ∞]. */
  var logAll = function () {
    return req('readonly', function (s) {
      return s.getAll(IDBKeyRange.bound([site_], [site_, []], false, false));
    }, LOG);
  };
  var logPut = function (d) {
    d.site = site_;
    return req('readwrite', function (s) { return s.put(d); }, LOG);
  };

  /* The vector a checkpoint covers: author → highest seq folded into it. A delta
     is in the tail if its seq is above its author's mark, so a replica can hold a
     checkpoint from one device and still replay another's arrivals over it. */
  function tail(deltas, at) {
    return deltas.filter(function (d) { return d.seq > (at[d.a] || -1); });
  }
  function order(ds) {
    return ds.slice().sort(function (x, y) {
      return (x.at - y.at) || (x.a < y.a ? -1 : x.a > y.a ? 1 : 0) || (x.seq - y.seq);
    });
  }
  function fold(base, deltas) {
    var w = JSON.parse(JSON.stringify(base));
    order(deltas).forEach(function (d) { applyOp(w, d.op); });
    return w;
  }

  /* Replay is bounded by the checkpoint, and the checkpoint is rewritten once the
     tail gets long enough to be worth collapsing. CHECKPOINT_EVERY is the only
     tuning knob: too small and every commit rewrites the world, too large and a
     cold start pays for it. */
  var CHECKPOINT_EVERY = 200;

  function state() {
    return Promise.all([get(cpKey()), logAll()]).then(function (r) {
      var cp = r[0] || { at: {}, world: null }, ds = r[1] || [];
      if (!cp.world) return { cp: cp, deltas: ds, world: null };
      return { cp: cp, deltas: ds, world: fold(cp.world, tail(ds, cp.at)) };
    });
  }

  function append(op, author) {
    return state().then(function (st) {
      if (!st.cp.world) throw new Error('no world — load with a seed first');
      var seq = 0;
      st.deltas.forEach(function (d) { if (d.a === author && d.seq >= seq) seq = d.seq + 1; });
      var d = { a: author, seq: seq, at: Date.now(), op: op };
      return deltaId({ a: d.a, seq: d.seq, at: d.at, op: d.op }).then(function (id) {
        d.id = id;
        return logPut(d).then(function () {
          var ds = st.deltas.concat([d]);
          var t = tail(ds, st.cp.at);
          var world = fold(st.cp.world, t);
          if (t.length < CHECKPOINT_EVERY) return { world: world, delta: d };
          /* Collapse: the fold becomes the new base and the vector moves up to
             every author's head. The deltas stay — they are the thing that syncs. */
          var at = {};
          ds.forEach(function (x) { if (x.seq > (at[x.a] || -1)) at[x.a] = x.seq; });
          return put(cpKey(), { at: at, world: world }).then(function () {
            return { world: world, delta: d };
          });
        });
      });
    });
  }

  /* ── the session ─────────────────────────────────────────────────────────
     WHO YOU ARE, AND WHICH ARC IS YOURS.

     ONE DOOR (launch decision, 11 Sep 2026): a passkey, minted at the Arc the
     person chose. Sign in with Apple, AI Passport and OreCloud are gone — the
     auth service 404s their routes — so there is nothing here that links to a
     provider and nothing that waits for one to redirect back. A WebAuthn
     ceremony happens in the page, because only the browser can talk to an
     authenticator, and the page it happens in is this one opened as a window
     (see `door()` at the bottom): the same origin the device key lives on,
     which is also the origin the passkey's RP ID names.

     What this adds is the one thing the interior cannot do for itself. The
     interior runs on the MEMBER'S origin; the account's seed lives on THIS one,
     and must never cross. So the keyholder is the only place in the system that
     can answer "is anyone signed in, and where do they sync" without handing
     over the thing that makes the answer true.

     THERE IS NO COOKIE TO READ ANY MORE. This paragraph used to describe
     `GET /auth/session` -> { uid, providers[], arc, store }, with 401 meaning
     signed out. That route is gone with the rest of the session subsystem: what
     signs a person in is a seed, and no server can hand one back. The question
     is answered from this origin's own database instead, and the Arc is asked
     against a SIGNATURE when its view is wanted (see `askArc`).

     What crosses to the interior: signed-in or not, the identity key, the Arc,
     and the sites held. Never the seed, and there is no method that returns
     it. */

  /* Same origin as this document by default — in production the keyholder is
     served from kenjin.cc, so /auth is already here. Overridable because a
     local harness serves the keyholder from a port of its own. */
  var AUTH = '';

  /* ── who this browser is ─────────────────────────────────────────────────
     THERE IS NO SESSION TO ASK ABOUT ANY MORE. This used to be a fetch of
     `/auth/session`, which answered from an httpOnly cookie; the service has
     neither route nor cookie now. What signs a person in is a SEED, and a seed
     is not something a server can hand back — so the question "is anyone signed
     in here" is answered locally, by whether this origin holds one.

     THIS IS THE ORIGIN THAT HOLDS IT, deliberately and by design: the document
     is framed by a member's page precisely so the member's page cannot read
     what is in here. The seed rests sealed (sealedPut, like every other record)
     and is used without leaving.

     `arc` still comes from the service, because which Arc holds a person's
     mailboxes is a property of the ACCOUNT rather than of this browser — but it
     is now fetched against a signature rather than read out of a cookie, and
     only when a seed is actually held. */
  function accountRec() { return sealedGet('account'); }

  function session() {
    return accountRec().then(function (rec) {
      if (!rec || !rec.seed) return { signedIn: false };
      return { signedIn: true, uid: rec.pk, arc: rec.arc || null, store: true };
    });
  }

  /* The Arc's own view, which needs the key. Separate from `session` because it
     is a round trip and most callers only want to know whether anyone is here.

     arc_relay_url FIRST, and the distinction is not pedantic. arc_url addresses
     the Arc over https — discovery, signup, the rail; the blind relay is a
     websocket endpoint the Arc advertises separately (wss://…/v1/relay).
     Opening a socket on arc_url is a category error that went unnoticed only
     because one deployment set ARC_URL to the relay address itself, so the two
     happened to be the same string. An older account that predates the column
     has no relay url recorded, and falls back to what it does have. */
  function askArc(rec) {
    return pacific().then(function (a) {
      return a.identity(unhex(rec.seed)).then(function (id) { return a.account(id); });
    }).then(function (body) {
      var arc = body.arc || null;
      return {
        signedIn: true, uid: rec.pk, store: true,
        arc: (arc && (arc.arc_relay_url || arc.arc_url)) || null,
        sites: body.sites || [], history: body.history || null,
        created_at: body.created_at || null
      };
    }, function (e) {
      /* A door that cannot be reached is not a door that refused. The interior
         needs to tell those apart — one says "sign in", the other says "we
         could not ask". Being unable to reach the Arc does not sign anyone out:
         the seed is here and still opens everything local. */
      return { signedIn: true, uid: rec.pk, arc: rec.arc || null, store: true,
               unreachable: String(e && e.message || e) };
    });
  }

  /* ── becoming the person ─────────────────────────────────────────────────
     THE POINT OF ALL OF THIS. A browser that has adopted a seed is not "a
     browser with a session"; it is that person's device, and the MLS leaf it
     stands up carries their identity key as its credential — which is what
     makes a delta it authors theirs, and what makes every peer's pin match.

     WHAT WAS HERE BEFORE. The keyholder minted a SECOND random seed for MLS and
     a third WebCrypto key for the device, so a person signed in on their phone
     and on the web was two or three different identities to everyone they had
     ever met. `mls-seed` is now the account's seed. The per-install MLS SIGNING
     key (sk/pk) stays per-install and is not derived from it — two live devices
     must be two leaves — but both leaves now say the same person.

     A DIFFERENT ACCOUNT RESETS THE LEAF, loudly rather than by accident.
     `mls_init` is a reset (lib.rs says so), and restoring a stranger's snapshot
     over a fresh identity is how a browser ends up holding ratchets it can
     never use. So when the seed changes, the MLS state goes with it. */
  function adopt(a, got, words) {
    /* REFUSE A SEED THAT IS NOT ONE. This is not defensiveness for its own sake:
       `create` used to zero the seed before returning it, so this stored the
       empty string, and the result was an account that registered successfully,
       reported itself signed in, and could never sign in again — every symptom
       pointing somewhere other than the cause. Anything but 32 bytes is a bug
       upstream, and it stops here rather than being written to disk. */
    if (!got || !got.seed || got.seed.length !== 32) {
      throw new Error('refusing to adopt a ' + ((got && got.seed && got.seed.length) || 0) +
                      '-byte seed — the account flow returned no usable key');
    }
    var rec = { seed: hex(got.seed), pk: got.pk, arc: null, at: Date.now() };
    var body = got.account || {};
    var arc = body.arc || null;
    rec.arc = (arc && (arc.arc_relay_url || arc.arc_url)) || null;

    return sealedGet('account').then(function (prev) {
      var changed = !prev || prev.pk !== rec.pk;
      /* The seed is the leaf's seed now. Writing it as `mls-seed` keeps leaf()
         unchanged: it reads that record and always has. */
      var work = [sealedPut('account', rec), sealedPut('mls-seed', { seed: rec.seed })];
      if (changed) {
        _leaf = null;
        chan = null;
        if (sock) { try { sock.close(); } catch (e) {} sock = null; }
        work.push(del('mls-state'), del('channel'), del('channel-addr'));
      }
      return Promise.all(work);
    }).then(function () {
      /* The seed is in the database now; the copy the caller handed over is
         not needed and does not get to linger in a closure. */
      PacificAccount.forget(got.seed);
      return {
        signedIn: true, uid: rec.pk, arc: rec.arc, store: true,
        created_at: body.created_at || null,
        sites: body.sites || [],
        words: words || null          /* only ever set on create */
      };
    });
  }

  /* The account module, shared with kenjin.cc and staged beside this file.
     Nothing cryptographic about accounts is written in this file — the
     derivation, the wrap and the challenge signature have one definition. */
  var _pac = null;
  function pacific() {
    if (_pac) return _pac;
    if (!window.PacificAccount) {
      return Promise.reject(new Error('pacific-account.js is not loaded — run app/web/account/stage.sh'));
    }
    _pac = Promise.resolve(new window.PacificAccount({
      corePath: 'core/core_wasm_bg.wasm',
      authOrigin: AUTH
    }));
    return _pac;
  }

  /* ── the channel between a person's own devices ──────────────────────────
     The narrow thing that makes "send on one device, see it on another" true,
     without either device being able to read the other's private key and without
     the relay learning anything at all.

     PAIRING is one X25519 exchange. Each device holds a channel keypair; an
     OFFER carries the public half (this is the QR's payload). Accepting an offer
     derives the shared secret, and HKDF splits it into two independent keys:

       tag key  → the mailbox address both devices publish to and drain from
       seal key → AES-GCM over the delta

     The tag is derived, never transmitted. The relay sees an opaque 32 bytes and
     a sealed blob, which is all `pacific-wire` says it may ever see: "the relay
     never sees identities, content, the MLS group_id, or the social graph".

     WHAT THIS IS NOT. It is not MLS and it is not a group. It is a pairwise
     channel between two devices that already trust each other because a person
     held both. That is deliberately the smaller primitive —
     docs/multi-device-plan.md calls the same shape "hand an encrypted archive
     to a device that cannot decrypt the past" and builds history transfer on
     it. */

  /* The domain strings now live in `identity.rs` (ACCOUNT_CHANNEL_DOMAIN and
     DEVICE_CHANNEL_DOMAIN) so no client can retype one and differ by a byte.
     PRF below is still needed HERE because the authenticator evaluates its
     secret over that salt before the core ever sees it — same bytes, and the
     one place they are still stated twice. */

  /* MEASURED, 9 Sep 26: an X25519 CryptoKeyPair does NOT survive IndexedDB in
     WebKit. The put completes, the transaction fires oncomplete, and the record
     reads back as undefined — a silent loss, with no error anywhere. Ed25519,
     ECDH P-256, ECDSA and AES-GCM all round-trip correctly, so this is specific
     to X25519 and not to CryptoKey.

     It does not need a workaround, because it was the wrong thing to persist.
     The exchange key is ephemeral by nature — you show a code and scan the reply
     in one sitting — and what has to survive a reload is the DERIVED channel key,
     which is AES-GCM and stores fine. So the pair lives in memory for the length
     of a pairing and nothing more. */
  var pairing = null;

  function x25519() {
    return crypto.subtle.generateKey({ name: 'X25519' }, false, ['deriveBits']);
  }
  function b64(buf) {
    var b = new Uint8Array(buf), s = '';
    for (var i = 0; i < b.length; i++) s += String.fromCharCode(b[i]);
    return btoa(s);
  }
  function unb64(s) {
    var raw = atob(s), u = new Uint8Array(raw.length);
    for (var i = 0; i < raw.length; i++) u[i] = raw.charCodeAt(i);
    return u;
  }
  function unhex(s) {
    var u = new Uint8Array(s.length / 2);
    for (var i = 0; i < u.length; i++) u[i] = parseInt(s.substr(i * 2, 2), 16);
    return u;
  }

  /* One secret in, two unrelated keys out. Domain-separated by info string, the
     way `atrest.rs` separates its own — so the address cannot be derived from
     the cipher key or the other way about. */
  /* ── the core, as wasm ───────────────────────────────────────────────────
     THE CHANNEL DERIVATION IS THE CORE'S, ON EVERY PLATFORM.

     This used to be fifteen lines of WebCrypto HKDF here, and iOS was about to
     grow its own in Swift. That is a worse failure than it looks: a wrong
     signature throws and a wrong hash mismatches, but a wrong TAG raises
     nothing at all. Two devices simply derive different mailboxes, sit on them,
     and never hear each other — with no error anywhere to find. Silence is the
     entire failure mode.

     So both halves come from `pacific-core` compiled to wasm, and the vector in
     `identity::account_channel_matches_webcrypto` pins the Rust against the
     WebCrypto bytes this file used to compute, so the retirement changed
     nothing about what the browser derives.

     Loaded lazily and once: most sessions never touch a channel.

     WHICH BUILD. There is only one now — `build-wasm.sh`, the
     wasm-bindgen layout: `core/core_wasm_bg.wasm` beside its `core_wasm.js`
     glue, copied here from docs/core. The keyholder needs the MLS surface (a
     pairing bundle carries a KeyPackage) and the archive fold, and the
     arithmetic-only module has neither.

     THE GLUE IS NOT LOADED, and that is a decision rather than a shortcut. It
     is an ES module that imports the bare specifier "env", which only an
     import map in the document can resolve — and this file is a classic script
     on purpose, being the frame and the door at once (see the bottom of the
     file). What the glue actually does is small and entirely visible in its
     `__wbg_get_imports`: satisfy three host functions and run
     `__wbindgen_start`. So the module's OWN import list is read, and each
     import is satisfied BY NAME from the table below. An import not in the
     table refuses to instantiate — a build that needs something new fails
     here with the name, not later with a wrong key. */
  var CORE_URL = 'core/core_wasm_bg.wasm';
  var HOST = {
    /* The CSPRNG, as a declared import: a host that cannot supply one cannot
       instantiate the module, which is the property worth having. */
    pacific_fill_random: function (at) {
      return function (ptr, len) {
        crypto.getRandomValues(new Uint8Array(at().exports.memory.buffer, ptr, len));
      };
    },
    /* wasm-bindgen's externref table seed, verbatim from the glue it emits. */
    __wbindgen_init_externref_table: function (at) {
      return function () {
        var t = at().exports.__wbindgen_externrefs, off = t.grow(4);
        t.set(0, undefined);
        t.set(off + 0, undefined); t.set(off + 1, null);
        t.set(off + 2, true); t.set(off + 3, false);
      };
    },
    /* mls-rs-core's one inline snippet: the clock, for KeyPackage lifetimes. */
    date_now: function () { return function () { return Date.now(); }; }
  };
  var _core = null;
  function core() {
    if (_core) return _core;
    _core = fetch(CORE_URL).then(function (r) {
      if (!r.ok) {
        throw new Error(CORE_URL + ' ' + r.status + ' — build it with app/web/build-wasm.sh ' +
                        'and put docs/core/ beside this file');
      }
      return r.arrayBuffer();
    }).then(function (b) { return WebAssembly.compile(b); }).then(function (mod) {
      /* The module's memory is not reachable until it is instantiated, so each
         import closes over a getter the instance fills in a moment later. */
      var inst = null, at = function () { return inst; }, imports = {};
      WebAssembly.Module.imports(mod).forEach(function (i) {
        var make = HOST[i.name];
        if (!make) {
          throw new Error('core_wasm imports ' + i.module + '.' + i.name +
                          ', which this keyholder does not supply');
        }
        (imports[i.module] = imports[i.module] || {})[i.name] = make(at);
      });
      return WebAssembly.instantiate(mod, imports).then(function (w) {
        inst = w;
        if (w.exports.__wbindgen_start) w.exports.__wbindgen_start();
        return w.exports;
      });
    });
    /* A failed load is not remembered as one: the next call fetches again, so
       a server that came up late does not mean a page that must be reloaded.
       The failure itself still reaches whoever asked. */
    _core.catch(function () { _core = null; });
    return _core;
  }

  /* alloc → write → call → read: the ABI every export shares. The buffer is
     re-read after `alloc` because growing linear memory detaches the old one,
     and a refusal puts its reason in the same out buffer with `erred()` set —
     so it comes back as an Error with the core's own words, never as bytes
     that happen to be empty. */
  var utf8 = function (s) { return new TextEncoder().encode(s); };
  var text = function (b) { return new TextDecoder().decode(b); };
  function call(e, fn, bytes) {
    var ptr = e.alloc(bytes.length);
    new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
    var n = fn(bytes.length);
    if (e.erred()) throw new Error(text(new Uint8Array(e.memory.buffer, e.out_ptr(), e.out_len())));
    return new Uint8Array(e.memory.buffer, e.out_ptr(), n).slice();
  }
  function callJson(e, fn, obj) { return JSON.parse(text(call(e, fn, utf8(JSON.stringify(obj))))); }

  /* Call a 32-bytes-in, 64-bytes-out channel export and wrap the halves the way
     the rest of this file wants them: the tag as hex to address a mailbox, the
     seal as a live AES-GCM CryptoKey that never exists as bytes again. */
  function channel(fn, secret) {
    return core().then(function (e) {
      var input = new Uint8Array(secret);
      if (input.length !== 32) throw new Error('channel secret must be 32 bytes');
      var out = call(e, e[fn], input);
      /* The first half is the channel's relay-address SEED (since 18 Sep 2026):
         the relay refuses a pub its tag did not sign, so the tag is the PUBLIC key
         of that seed, asked of the core, and the seed stays to sign with. */
      var addr = out.slice(0, 32), seal = out.slice(32, 64);
      var tag = call(e, e.relay_address, addr);
      return crypto.subtle.importKey('raw', seal, 'AES-GCM', false, ['encrypt', 'decrypt'])
        .then(function (k) { return { tag: hex(tag), seal: k, addr: hex(addr) }; });
    });
  }

  /* The channel's write seed is a secret, and the channel record holds a live
     CryptoKey the sealed store cannot carry — so the seed is sealed on its own,
     beside it. A channel stored before signing has no seed and its tag is the old,
     unsigned kind: it is not loaded, and is derived again on the next link. */
  function putChannel(ch, extra) {
    var rec = { tag: ch.tag, seal: ch.seal };
    for (var k in (extra || {})) rec[k] = extra[k];
    return Promise.all([put('channel', rec), sealedPut('channel-addr', { tag: ch.tag, addr: ch.addr })]);
  }
  function loadChannel() {
    return Promise.all([get('channel'), sealedGet('channel-addr').catch(function () { return null; })])
      .then(function (r) {
        var c = r[0], a = r[1];
        return c && a && a.tag === c.tag ? { tag: c.tag, seal: c.seal, addr: a.addr, from: c.from } : null;
      });
  }

  var accountChannelFrom = function (prf) { return channel('account_channel', prf); };
  var deviceChannelFrom = function (shared) { return channel('device_channel', shared); };

  function seal(ch, obj) {
    var iv = crypto.getRandomValues(new Uint8Array(12));
    var pt = new TextEncoder().encode(JSON.stringify(obj));
    return crypto.subtle.encrypt({ name: 'AES-GCM', iv: iv }, ch.seal, pt).then(function (ct) {
      var out = new Uint8Array(12 + ct.byteLength);
      out.set(iv, 0); out.set(new Uint8Array(ct), 12);
      return b64(out);
    });
  }
  function unseal(ch, blob) {
    var u = unb64(blob);
    return crypto.subtle.decrypt({ name: 'AES-GCM', iv: u.slice(0, 12) }, ch.seal, u.slice(12))
      .then(function (pt) { return JSON.parse(new TextDecoder().decode(pt)); });
  }

  /* ── the account channel ─────────────────────────────────────────────────
     PAIRING above asks a person to be holding both devices at once. That is the
     right primitive and the wrong ceremony for the ordinary case, where the two
     devices are already the same account — so the channel is derived instead of
     read off a screen.

     IT USED TO BE DERIVED FROM THE PASSKEY, and ~110 lines lived here to do it:
     a WebAuthn assertion evaluating two PRF salts, one for this channel and one
     for the backup seal, plus the credential-JSON encoder and the two
     /auth/passkey/* calls it posted to. All of it is gone, for two reasons.

     The routes went first — the service has no WebAuthn endpoints any more —
     but the reasoning went too, and that is the part worth recording. The
     design rested on the passkey's PRF secret SYNCING between a person's
     devices, and whether it does is up to the platform. Where it does not, two
     devices derived different channels and never met, silently, both of them
     apparently working. The seed does not have that hole: every device that can
     open the account has it, and one that cannot has no business on the
     channel. The backup seal moved for the same reason — `history_key(seed)`,
     in the core, so twenty-four words are enough to restore.

     What survives is `channel()` above, unchanged, and its domain separation:
     `account_channel` mixes its own domain in, so a channel key can never be
     confused with a wrap key or a seal. */

  /* THE CHANNEL COMES FROM THE SEED NOW, NOT FROM THE PASSKEY, and this is a bug
     fix as much as a simplification.

     It used to be derived from the passkey's PRF, on the reasoning that two
     devices holding the same synced passkey would derive the same channel and
     so find each other. That reasoning has a hole in it that only showed up
     when it was tested on real hardware: whether a platform carries a
     credential's PRF between a person's devices is up to the platform, and
     where it does not, the two devices derived DIFFERENT channels and simply
     never met — silently, with both of them apparently working.

     The seed does not have that problem. Every device that can open the account
     has it, by construction, and a device that cannot open the account has no
     business on the channel. So the rendezvous is derived from the seed, which
     also means it needs no ceremony at all: signing in IS linking, and the
     separate "link this device" passkey prompt is gone.

     Same domain separation as before — `account_channel` mixes its own domain
     in, so a channel key can never be confused with a wrap key or a seal. */
  function accountChannel() {
    return sealedGet('account').then(function (rec) {
      if (!rec || !rec.seed) throw new Error('no account on this device — sign in first');
      return accountChannelFrom(unhex(rec.seed)).then(function (ch) {
        chan = ch;
        arcUrl = rec.arc || arcUrl;
        return putChannel(ch, { from: 'account' })
          .then(function () { return { tag: ch.tag, arc: arcUrl, uid: rec.pk }; });
      });
    });
  }

  /* ── the relay ───────────────────────────────────────────────────────────
     `pacific-wire`'s Frame vocabulary, in the browser: pub/sub by tag, JSON text
     frames, `since` for replay. The relay is the same semaphore the app dials;
     nothing here is a stand-in for it. */
  var sock = null;

  /* ONE SOCKET, SEVERAL CONVERSATIONS. The account channel subscribes once and
     listens forever; a rekey (below) publishes into a group's commit slot and
     has to READ THE VERDICT — `Ack{ok:false}` is the relay saying the slot was
     already taken, and a client that ignores it has forked. So the socket
     carries a small router rather than one callback:

       acks   — the relay answers every `pub` with one `ack`, in order, and an
                Ack names no tag; so the waiters are a queue, and EVERY pub goes
                through `pub()` below, even the ones whose verdict nobody reads.
                A pub sent around the queue would hand its ack to the wrong
                waiter, and a rekey would apply a commit that lost.
       eose   — one per `sub`, in order, for the same reason.
       tags   — a listener per subscribed tag, so a drained epoch tag's blobs
                do not land in the account channel's handler. A blob on a tag
                nobody is listening to is dropped, and said. */
  /* NEVER SILENT, and to the right audience. The console is for whoever is
     holding the devtools; the interior is the only surface the MEMBER is looking
     at, so anything that changes what they can believe about their own data goes
     to both. `importArchive` already did this by hand — now everything on the
     wire does. */
  function say(where, why) {
    console.warn('[pacific] ' + where + ': ' + why);
    if (port_) port_.postMessage({ t: 'pacific.error', where: where, why: String(why) });
  }

  function attach(ws) {
    ws._acks = [];
    ws._eose = [];
    ws._tags = {};
    ws.onmessage = function (e) {
      var f;
      /* A frame this build cannot PARSE is not nothing. `pacific-wire` makes an
         unknown frame fatal on the Rust side for exactly this reason: the
         alternative is a client that quietly stops understanding its relay. */
      try { f = JSON.parse(e.data); } catch (x) {
        return say('relay', 'a frame that is not JSON was dropped (' +
                            String(e.data).slice(0, 60) + '…)');
      }
      if (f.t === 'ack') { var a = ws._acks.shift(); if (a) a(f); return; }
      if (f.t === 'eose') { var r = ws._eose.shift(); if (r) r(); return; }
      if (f.t === 'gap') return say('relay', 'the relay announced a hole on ' +
                                    String(f.tag).slice(0, 16) + '… below seq ' + f.floor);
      if (f.t === 'msg') {
        var fn = ws._tags[f.tag];
        if (fn) return fn(f.blob, f.seq);
        return say('relay', 'a blob arrived on ' + String(f.tag).slice(0, 16) +
                            '…, which nothing is listening to — dropped');
      }
      /* THE DEFAULT BRANCH. There was none, so a frame this build does not know
         — a new relay-to-client variant, a media frame, a typo — fell
         straight through and left no trace anywhere. A client that has stopped
         understanding its relay is the one failure that must never be quiet:
         everything downstream of it looks exactly like "nothing to say". */
      say('relay', 'an unknown frame t=' + JSON.stringify(f.t) +
                   ' was dropped — this build does not know it. Sub declares ' +
                   'v:1 (Msg/Ack/Eose/Gap); a relay sending more than that has ' +
                   'moved ahead of this client.');
    };
    return ws;
  }
  function open(url) {
    return new Promise(function (res, rej) {
      var ws = attach(new WebSocket(url));
      ws.onopen = function () { res(ws); };
      ws.onerror = function () { rej(new Error('relay unreachable at ' + url)); };
      ws.onclose = function () { if (sock === ws) sock = null; };
    });
  }
  /* Subscribe on an open socket; resolves at the end of the stored backlog.
     v:1 so the relay may tell us the backlog was incomplete. Acting on a Gap is
     a separate step; parsing it is free and refusing it is fatal. */
  function sub(ws, tag, onBlob) {
    ws._tags[tag] = onBlob;
    return new Promise(function (res) {
      ws._eose.push(res);
      ws.send(JSON.stringify({ t: 'sub', tags: [tag], since: 0, v: 1 }));
    });
  }
  /* Publish, and resolve with the relay's verdict. `commit` claims the tag's
     one commit slot — `pacific-wire`'s blind sequencer — and is sent only when
     true, so a plain pub is byte-identical to what it always was. */
  /* `to` is anything that can sign for its address: a channel ({tag, addr}) or a
     group epoch's keys ({tag, address_seed}). The relay refuses a pub its tag did
     not sign, and the signature is the core's — `relay_sign_pub` — never made here. */
  function pub(ws, to, blob, commit) {
    var seed = to.addr || to.address_seed;
    if (!seed) return Promise.reject(new Error('no key for ' + String(to.tag).slice(0, 16) +
                                               '… — the relay would refuse an unsigned pub'));
    return core().then(function (e) {
      var s = callJson(e, e.relay_sign_pub, { seed: seed, blob: blob });
      if (s.tag !== to.tag) throw new Error('the key does not name this address — refusing to publish');
      return new Promise(function (res) {
        ws._acks.push(res);
        var f = { t: 'pub', tag: s.tag, blob: blob, sig: s.sig };
        if (commit) f.commit = true;
        ws.send(JSON.stringify(f));
      });
    });
  }
  /* Publish and READ THE VERDICT. `pub` had no callers at all: every real
     publish in this file was a raw `ws.send`, which is two bugs rather than one.
     The verdict was discarded — so an `Ack{ok:false}`, the relay saying a
     commit slot was already taken, was invisible — and the send went AROUND
     the ack queue, so the next waiter that did exist would have been handed
     somebody else's answer.

     A refused COMMIT throws. It means this device lost the epoch race: its MLS
     client has already advanced and the group has not, so everything it
     encrypts from here is addressed to an epoch nobody is in. That is not
     something to log and carry on from.

     A refused plain pub is said out loud and returns false. The delta is already
     in this device's own log; failing the member's commit because the OTHER
     device did not receive it would be the same lie pointing the other way. */
  function pubOK(ws, to, blob, commit, what) {
    return pub(ws, to, blob, commit).then(function (ack) {
      if (!ack || ack.ok !== false) return true;
      var where = String(to.tag).slice(0, 16) + '…';
      /* A REASON means a refusal — a signature, a limit, a full store — and never a
         lost race. Only a commit refused WITHOUT one lost its slot. */
      if (ack.reason) {
        if (commit) throw new Error('the relay refused the commit on ' + where + ': ' + ack.reason);
        say(what, 'the relay refused the publish on ' + where + ': ' + ack.reason);
        return false;
      }
      if (commit) {
        throw new Error('the relay refused the commit on ' + where + ' — this epoch’s ' +
                        'one commit slot was already taken. This device LOST the race and its ' +
                        'group state is now ahead of the group; it has to rebase onto the ' +
                        'winning commit, and that is not built here yet.');
      }
      say(what, 'the relay refused the publish on ' + where +
                ' — it was not stored and not fanned out');
      return false;
    });
  }
  function relay(url, tag, onDelta) {
    return open(url).then(function (ws) {
      return sub(ws, tag, onDelta).then(function () { return ws; });
    });
  }

  /* WHICH RELAY, and why this is not a constant. An explicit override wins
     because a harness has to be able to say so; otherwise it is the Arc the
     SESSION names, and only if there is no session at all does it fall back to
     the local one. A device that silently synced from the wrong relay would
     look exactly like a device that had nothing to sync — the same empty screen
     for "we are not talking" and "there is nothing to say" — so the Arc is
     resolved here rather than assumed to have been. */
  function relayUrl(override) {
    if (override) return Promise.resolve(override);
    if (arcUrl) return Promise.resolve(arcUrl);
    return session().then(function (s) {
      arcUrl = s.arc || arcUrl;
      return arcUrl || 'ws://localhost:8787';
    });
  }
  /* The socket, opened if it is not. Opening one does not subscribe the account
     channel — `sync.start` does that, and checks the listener rather than the
     socket to know whether it already has. */
  function socket(override) {
    if (sock && sock.readyState === 1) return Promise.resolve(sock);
    return relayUrl(override).then(open).then(function (ws) { sock = ws; return ws; });
  }

  /* ── the archive ─────────────────────────────────────────────────────────
     HISTORY, HANDED OVER. MLS forward secrecy means a leaf added at epoch N
     cannot read anything before N, so a new device — this browser — gets its
     past OUT OF BAND: a device that has it exports `archive.rs`'s Archive,
     seals it, and sends it down the account channel like any other delta. It
     arrives here, folds through the core, and the interior opens to the
     conversations it already had.

     THE FOLD IS THE CORE'S. `fold_archive` in core-wasm runs
     `archive::fold_group` over every group — byte-for-byte the fold the phone
     runs, because `object_store::folded` delegates to the same function — and
     hands back one JSON document: per group, its kind, roster, digest and the
     folded view. Two devices holding the same archive compute the same digest,
     so "did the history transfer intact" is one hash; nothing in this file
     derives a message from a delta. `world.js` then puts that document in the
     interior's vocabulary, and that mapping is all the JavaScript there is.

     WHAT BECOMES OF THE WORLD. The folded world becomes the CHECKPOINT for
     this site, at every author's current head — so `store.load` returns it on
     the next visit and anything committed after it folds over it, exactly as a
     seed's genesis does. It is also kept whole under 'archive:<site>' with who
     exported it and when, so the record of where this device's history came
     from outlives the next checkpoint collapse. A device that folded an
     archive and then reloaded into the seed would have silently lost the one
     thing it was handed; rewriting the checkpoint is what prevents that.

     A group the core would not fold is reported, never fatal: the archive
     still lands, minus that group, and the reply says which one and why. */
  var archiveKey = function () { return 'archive:' + site_; };

  /* `world.js` is the pure mapping from the fold to the interior's world — no
     DOM, no globals of its own, so `world.test.cjs` can hold it to the shape
     under node; the same file runs here. Loaded lazily, like the core: most
     sessions never import an archive. */
  var _world = null;
  function world() {
    if (_world) return _world;
    _world = new Promise(function (res, rej) {
      if (window.PacificWorld) return res(window.PacificWorld);
      var s = document.createElement('script');
      s.src = 'world.js';
      s.onload = function () {
        if (window.PacificWorld) res(window.PacificWorld);
        else rej(new Error('world.js loaded and defined nothing'));
      };
      s.onerror = function () { rej(new Error('world.js did not load')); };
      document.head.appendChild(s);
    });
    _world.catch(function () { _world = null; });
    return _world;
  }

  function importArchive(cbor_b64, me) {
    if (typeof cbor_b64 !== 'string' || !cbor_b64) {
      return Promise.reject(new Error('archive.import wants cbor_b64 — the archive as Archive::to_cbor encodes it, base64'));
    }
    return Promise.all([core(), world()]).then(function (r) {
      var e = r[0], W = r[1];
      if (!e.fold_archive) throw new Error('this core_wasm has no fold_archive — rebuild it with app/web/build-wasm.sh');
      var fold = JSON.parse(text(call(e, e.fold_archive, unb64(cbor_b64))));
      // W.FOLD_V, not a literal: the version is world.js's to state (it is the
      // thing that does the mapping), and a second copy here is how this guard
      // came to read v1 for two days after the core moved to v2.
      if (fold.v !== W.FOLD_V) {
        throw new Error('archive fold v' + fold.v + ' — this keyholder reads v' + W.FOLD_V);
      }
      (fold.rejected || []).forEach(function (x) {
        console.warn('[pacific] archive group ' + String(x.group).slice(0, 16) + '… did not fold: ' + x.why);
      });
      /* WHO IS "ME". The archive is its exporter's history — the conversations
         are theirs, and their side of each is the side to draw as ours — so the
         exporter is `me` unless the caller says otherwise. This device's own
         key authored nothing in a history it has only just received. */
      var w = W.worldFromFold(fold, me || fold.exported_by);
      return logAll().then(function (ds) {
        var at = {};
        ds.forEach(function (x) { if (x.seq > (at[x.a] || -1)) at[x.a] = x.seq; });
        /* THE BYTES ARE KEPT, not only the fold. `backup.export` puts the
           archive in the backup verbatim — `Backup.archive` is the core's
           `Archive`, and the core decodes it before sealing — so a browser that
           held only the folded world would have nothing a backup could carry.
           This is the one copy of the phone's history this device has; the
           record it sits in already outlives checkpoint collapses. */
        return put(archiveKey(), { by: fold.exported_by, at: Date.now(), groups: fold.groups.length, world: w, cbor_b64: cbor_b64 })
          .then(function () { return put(cpKey(), { at: at, world: w }); });
      }).then(function () {
        if (port_) port_.postMessage({ t: 'pacific.push', v: w });
        return {
          from: fold.exported_by,
          groups: fold.groups.length,
          threads: w.threads.length,
          convs: w.convs.length,
          events: w.events.length,
          people: Object.keys(w.people).length,
          unsupported: fold.groups.filter(function (g) { return g.view && g.view.unsupported; }).length,
          rejected: fold.rejected || []
        };
      });
    });
  }

  /* ── the MLS leaf, and the person behind it ──────────────────────────────
     WHAT A BUNDLE IS. `handshake.rs`: the identity pubkey, a fresh one-time
     KeyPackage, the intro mailbox tag, the display name and the next-key
     commitment, self-signed by the Ed25519 IDENTITY. Scanning it is how another
     person adds this device to a group, and the identity key in it is what they
     pin — so it must be the person's key, and core-wasm refuses to sign one
     with anything else.

     WHY THAT IS NOT THE DEVICE KEY ABOVE, said plainly because it is a wart.
     The core derives an identity from a 32-byte SEED (`identity::keys_from_seed`
     — HKDF under the core's own salt, a current key and a next one) and signs
     with what it derived. The device key was minted by WebCrypto with
     `extractable: false`: there is no seed behind it that anything can read,
     and WebCrypto cannot be asked to run the core's derivation. So the MLS
     identity cannot be that key, however much this file would prefer one key
     per device.

     SO THERE IS A SECOND SEED, held under 'mls-seed'. 32 bytes from
     `crypto.getRandomValues`, and the identity the core derives from it is the
     one a pairing bundle names. It is NOT the `pk` this keyholder reports in
     `pacific.ready` and `device.identity`: that remains the WebCrypto key, which
     signs `device.sign` and nothing else. Folding the two into one would mean
     either an extractable WebCrypto key — whose only protection would then be
     the origin boundary, which is exactly this seed's position — or the core's
     derivation done a second time in JavaScript, which is the drift
     `identity.rs` exists to prevent. The honest option is the one that exists,
     and it is recorded here rather than papered over.

     HOW IT IS KEPT. The seed, and everything the wasm hands back to persist —
     the MLS signing key it mints, the intro tag, the group-state snapshot that
     `lib.rs` calls SECRET IN FULL — are AES-GCM sealed under 'mls-wrap', a
     non-extractable CryptoKey stored the way the channel key is. Be exact about
     what that buys: the plaintext must exist in this script and in wasm linear
     memory for every MLS call, so this is protection for the database at rest,
     not the "never bytes in JavaScript" property the device key has. A script
     on this origin could unseal it — and a script on this origin is the thing
     the whole file is built to keep out.

     THE CONTRACT WITH core-wasm, in one place so a mismatch is found here:
       mls_init            {"seed"[,"cred","sk","pk","intro"]} → {"cred","pk","sk","intro"}
                           `cred` is the seed's identity key. Only the core can
                           derive it, so the FIRST stand-up sends the seed alone
                           and every later one sends the reply back verbatim.
       mls_contact_bundle  the display name, UTF-8 → {"bundle"}
       mls_snapshot / mls_restore   CBOR bytes, as lib.rs documents them. */
  function wrapKey() {
    return get('mls-wrap').then(function (k) {
      if (k) return k;
      return crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt'])
        .then(function (fresh) { return put('mls-wrap', fresh); });
    });
  }
  /* `seal`/`unseal` take a channel — anything with a `.seal` CryptoKey — so a
     record sealed under the wrap key goes through the same two functions the
     wire uses, and there is no second AEAD in this file. */
  function sealedGet(k) {
    return Promise.all([wrapKey(), get(k)]).then(function (r) {
      return r[1] ? unseal({ seal: r[0] }, r[1]) : null;
    });
  }
  function sealedPut(k, v) {
    return wrapKey().then(function (wk) { return seal({ seal: wk }, v); })
      .then(function (blob) { return put(k, blob); })
      .then(function () { return v; });
  }

  /* Stand the leaf up: the identity from the seed, the MLS signing key and
     intro tag the core minted last time (or fresh ones, persisted), and the
     group state it held. `mls_init` is a reset — restoring over a live client
     strands what it held, lib.rs says — so this runs once per page and every
     MLS call goes through what it produced. */
  var _leaf = null;
  function leaf() {
    if (_leaf) return _leaf;
    _leaf = Promise.all([core(), sealedGet('mls-seed'), sealedGet('mls-state')]).then(function (r) {
      var e = r[0], seedRec = r[1], st = r[2] || {};
      if (!e.mls_init || !e.mls_contact_bundle) {
        throw new Error('this core_wasm has no MLS surface — rebuild it with app/web/build-wasm.sh');
      }
      /* NO SECOND IDENTITY. This used to mint a random seed here when none was
         stored, which meant a browser stood up an MLS leaf under a credential
         belonging to nobody — the person was one identity on their phone and a
         different one on the web, to everyone they had ever met. The seed is
         the ACCOUNT's now (adopt() writes it), and a browser with no account
         has no leaf rather than a fictional one. Refusing here is the fix: it
         is better for the interior to say "sign in" than to quietly become
         someone who does not exist. */
      if (!seedRec || !seedRec.seed) {
        throw new Error('no account on this device — sign in before standing up a leaf');
      }
      var minted = Promise.resolve(seedRec);
      return minted.then(function (rec) {
        var init = { seed: rec.seed };
        if (st.cred) init.cred = st.cred;
        if (st.sk && st.pk) { init.sk = st.sk; init.pk = st.pk; }
        if (st.intro) init.intro = st.intro;
        var id = callJson(e, e.mls_init, init);
        if (st.snap) call(e, e.mls_restore, unb64(st.snap));
        var next = { cred: id.cred, pk: id.pk, sk: id.sk, intro: id.intro, snap: st.snap || null,
                     groups: st.groups || [], meta: st.meta || {} };
        var same = st.cred === next.cred && st.sk === next.sk && st.pk === next.pk && st.intro === next.intro;
        return (same ? Promise.resolve(next) : sealedPut('mls-state', next))
          .then(function (state) { return { e: e, state: state }; });
      });
    });
    _leaf.catch(function () { _leaf = null; });
    return _leaf;
  }

  /* After any MLS mutation. The snapshot is the only copy of a KeyPackage's
     private half and of every ratchet; a bundle whose private half stayed in a
     page that has since reloaded is a bundle nobody can ever accept. */
  function checkpointLeaf(L) {
    L.state.snap = b64(call(L.e, L.e.mls_snapshot, new Uint8Array(0)));
    return sealedPut('mls-state', L.state);
  }

  /* ── the group, in the browser ────────────────────────────────────────────
     Transcribed from smoke/relay-pair.mjs, which has been putting exactly this
     sequence through a real relay the whole time this file could not:
     create → key package → add → join → encrypt → relay → decrypt, with the
     delta byte-identical at the far end. None of it is new; it is the working
     path moved inside the origin that holds the key.

     THE HELPERS, and why each is here rather than inlined. `mls_epoch_keys`,
     `mls_create_group` and `mls_join` take a bare string rather than JSON, so
     `callJson` is wrong for them and a JSON-wrapped group id silently addresses
     nothing.

     THE SEAL SECRET NEVER LEAVES THIS FILE. `mls_epoch_keys` returns a tag and a
     secret, and lib.rs is explicit that the secret is LIVE KEY MATERIAL: anyone
     with it and the tag can strip the seal off a captured relay blob. So a group
     method returns the TAG (an address, which the relay sees anyway) and never
     the secret, the same way `device.export` refuses. */
  var mlsText = function (L, fn, s) {
    return text(call(L.e, fn, utf8(s == null ? '' : String(s))));
  };
  var mlsJson = function (L, fn, s) { return JSON.parse(mlsText(L, fn, s)); };
  var epochKeys = function (L, gid) { return mlsJson(L, L.e.mls_epoch_keys, gid); };

  /* `seal::seal` then base64 — the relay's blob, exactly as relay-pair.mjs
     builds it. The metadata seal is the CORE's, under this epoch's tag and
     secret, so the relay sees 32 opaque bytes of address and a sealed blob and
     nothing else. */
  function sealTo(L, keys, innerHex) {
    return b64(unhex(mlsText(L, L.e.seal_seal, JSON.stringify({
      inner: innerHex, tag: keys.tag, secret: keys.secret
    }))));
  }
  function openFrom(L, keys, blob) {
    return mlsText(L, L.e.seal_open, JSON.stringify({
      blob: hex(unb64(blob)), tag: keys.tag, secret: keys.secret
    }));
  }

  /* THE GROUP INDEX, and what it is not. The MLS snapshot is the authority —
     it carries every group and every ratchet — but core-wasm has no export that
     ENUMERATES it, so the ids are kept beside the snapshot as they are created
     and joined. Two consequences, said rather than papered over: a snapshot
     restored from another device brings groups this index does not name, and
     the index is not a Delta, so it is scaffolding around real state and not
     state itself. The remedy is an `mls_groups` export, not a local table. */
  /* WHAT THIS DEVICE MUST REMEMBER ABOUT A GROUP, because nothing it can read
     carries it. The KIND: the MLS context holds a name, an owner and a version and
     nothing else, and a Forum is minted name-only, so its empty log names no kind
     either. The GEN FLOOR: the one number a Welcome carries so that a device with
     an empty log (forward secrecy) does not re-mint a gen its person already used.
     0 for an object this device minted, which has seen all of it; the Welcome's
     mark for one it joined; ABSENT for a raw-Welcome join, which carried none,
     and then the core refuses every commutative op on it. Max'd, as
     `raise_gen_floor` is natively, so a replayed Welcome only moves it forward.

     Kept in the leaf state, which the checkpoint seals on this origin. That is
     local. A fresh device recovering the account learns neither from the spine
     today (a spine entry names a group and a floor epoch, not a kind), and that
     is a gap in the model, reported rather than papered over. */
  function groupMeta(L, gid) { return (L.state.meta || {})[gid] || {}; }
  function noteMeta(L, gid, m) {
    var all = {}, k;
    for (k in (L.state.meta || {})) all[k] = L.state.meta[k];
    var was = all[gid] || {}, now = {};
    for (k in was) now[k] = was[k];
    if (m.kind) now.kind = m.kind;
    if (typeof m.floor === 'number') now.floor = Math.max(typeof was.floor === 'number' ? was.floor : 0, m.floor);
    all[gid] = now;
    L.state.meta = all;
    return checkpointLeaf(L);
  }

  function noteGroup(L, gid) {
    L.state.groups = (L.state.groups || []).slice();
    if (L.state.groups.indexOf(gid) < 0) L.state.groups.push(gid);
    return checkpointLeaf(L);
  }

  /* Listen on a group's CURRENT epoch tag. Every epoch has its own mailbox, so
     advancing an epoch means MOVING — which is why this re-subscribes itself
     from inside the handshake arm rather than being set up once and forgotten.
     A client that kept draining the old tag would simply go quiet, with no
     error anywhere, which is the failure mode this whole file is arranged
     against. */
  function watchGroup(ws, gid) {
    return leaf().then(function (L) {
      var keys = epochKeys(L, gid);
      return sub(ws, keys.tag, function (blob) { onGroupBlob(ws, gid, keys, blob); })
        .then(function () { return { epoch: keys.epoch, tag: keys.tag }; });
    });
  }

  /* THE RECEIVE ARM. Unseal, `mls_decrypt`, and hand the interior what came
     back. This is the half `relay-pair.mjs` calls "the direction under test",
     and the half this file did not have. */
  function onGroupBlob(ws, gid, keys, blob) {
    return leaf().then(function (L) {
      var inner;
      try { inner = openFrom(L, keys, blob); }
      catch (e) {
        return say('group.receive', 'a blob on ' + keys.tag.slice(0, 16) +
                   '… would not unseal — ' + (e && e.message || e));
      }
      var got;
      try { got = callJson(L.e, L.e.mls_decrypt, { group_id: gid, message: inner }); }
      catch (e) {
        return say('group.receive', 'MLS refused a message on group ' + gid.slice(0, 12) +
                   '… — ' + (e && e.message || e));
      }
      /* Our own send, echoed back by the relay's fan-out. Not an event. */
      if (got.kind === 'own') return;
      /* REMOVED (membership-through-mls.md §8.3, §8.4). A commit took this device
         out of the group — its own leave completing, or the owner removing it.
         mls-rs left the group at its old epoch and nothing after this commit is
         readable here, so: stop listening, forget the group's MLS state, and say
         so to the interior. Never quietly: a group that vanished without a word
         is the failure this file is arranged against. */
      if (got.kind === 'removed') return forgetGroup(ws, L, gid, keys, got);
      /* The ratchet moved, whatever the message was. A snapshot that does not
         include it is a snapshot that cannot read the next one. */
      return checkpointLeaf(L).then(function () {
        /* A PROPOSAL (§7). It is cached, the epoch did not move, and until
           somebody commits it this device cannot send — RFC 9420 §12.4, which
           mls-rs enforces. Commit it: that is what completes someone else's leave.
           Unless it removes THIS person: then another member commits it, and a
           commit from here could only drop it and restart the leave. */
        if (got.kind === 'proposal') {
          if (got.mine) {
            console.info('[pacific] group.proposal: ' + gid.slice(0, 12) +
                         '… — this person is leaving; another member completes it');
            return;
          }
          return commitPending(ws, gid);
        }
        if (got.kind === 'handshake') {
          return watchGroup(ws, gid).then(function (now) {
            if (port_) port_.postMessage({ t: 'pacific.group.epoch', group: gid,
                                           epoch: now.epoch, tag: now.tag });
          });
        }
        /* AN APPLICATION MESSAGE: the canonical-CBOR Delta the sender authored,
           byte for byte, attributed to THEIR leaf rather than to anything inside
           the payload — which is the property MLS is here for.

           KEPT, and folded by the CORE on request: the row goes into this
           group's log, and `object.fold` hands the log to `fold_object`. It is
           still NOT handed to `applyOp`, which reduces ten invented ops that are
           in no ICD. Wiring a real delta into that would be inventing a reducer,
           the one thing that must not happen here. */
        return appendLog(gid, { author: got.sender, envelope: got.delta }).then(function () {
          if (port_) port_.postMessage({ t: 'pacific.group.delta', group: gid,
                                         sender: got.sender, delta: got.delta,
                                         epoch: got.epoch });
        });
      });
    }).catch(function (e) { say('group.receive', String(e && e.message || e)); });
  }

  /* FORGET A GROUP this device was removed from (§8.3, §8.4): drop the listener on
     its tag, take it out of the index, delete its MLS state (`mls_forget_group`),
     checkpoint, and tell the interior who did it — `left` is true when the removal
     was this person's own leave completing. */
  function forgetGroup(ws, L, gid, keys, got) {
    delete ws._tags[keys.tag];
    L.state.groups = (L.state.groups || []).filter(function (g) { return g !== gid; });
    var forgot;
    try { forgot = mlsJson(L, L.e.mls_forget_group, gid); }
    catch (e) { return say('group.removed', 'could not delete the MLS state of ' + gid.slice(0, 12) + '… — ' + (e && e.message || e)); }
    return checkpointLeaf(L).then(function () {
      console.info('[pacific] group.removed: ' + gid.slice(0, 12) + '… — ' +
                   (got.left ? 'this person left' : 'removed by ' + String(got.by).slice(0, 12) + '…') +
                   '; its MLS state is ' + (forgot && forgot.forgotten ? 'deleted' : 'already gone'));
      if (port_) port_.postMessage({ t: 'pacific.group.removed', group: gid, by: got.by, left: !!got.left });
    });
  }

  /* COMMIT WHAT OTHERS PROPOSED (§7), in the browser. Stage (`mls_rekey` sweeps the
     cache through the same commit rules the phone runs), seal to the CURRENT epoch's
     tag, claim that epoch's one commit slot, and read the verdict:
       won   → `mls_rekey_apply`, checkpoint, move to the new epoch's tag;
       lost  → `mls_rekey_drop`; the winner's commit is already on this tag and
               arrives through the ordinary receive arm, clearing the cache;
       refused with a reason → drop, and say why.
     Nothing here is new machinery: the three exports and `pub()` existed; nothing
     connected them. */
  function commitPending(ws, gid) {
    return leaf().then(function (L) {
      var keys = epochKeys(L, gid);
      var st;
      try { st = mlsJson(L, L.e.mls_rekey, gid); }
      catch (e) { return say('group.commit', 'could not stage the pending commit on ' + gid.slice(0, 12) + '… — ' + (e && e.message || e)); }
      var blob = sealTo(L, keys, st.commit);
      return pub(ws, keys, blob, true).then(function (ack) {
        if (ack && ack.ok === false) {
          try { mlsText(L, L.e.mls_rekey_drop, gid); } catch (e) {}
          if (ack.reason) return say('group.commit', 'the relay refused the commit on ' + gid.slice(0, 12) + '…: ' + ack.reason);
          console.info('[pacific] group.commit: lost the slot on ' + gid.slice(0, 12) + '… — folding the winner');
          return;
        }
        var applied = mlsJson(L, L.e.mls_rekey_apply, gid);
        return checkpointLeaf(L).then(function () {
          console.info('[pacific] group.commit: committed the waiting proposals on ' + gid.slice(0, 12) + '…; epoch ' + applied.epoch);
          return watchGroup(ws, gid).then(function (now) {
            if (port_) port_.postMessage({ t: 'pacific.group.epoch', group: gid, epoch: now.epoch, tag: now.tag });
          });
        });
      });
    }).catch(function (e) { say('group.commit', String(e && e.message || e)); });
  }

  /* Every known group, one subscription at a time. A group that will not open is
     said and skipped: one broken group must not cost the member the others. */
  function watchAllGroups(ws) {
    return leaf().then(function (L) {
      return (L.state.groups || []).reduce(function (chain, gid) {
        return chain.then(function (acc) {
          return watchGroup(ws, gid).then(function (k) {
            acc.push({ group_id: gid, epoch: k.epoch, tag: k.tag });
            return acc;
          }, function (e) {
            say('group.watch', 'group ' + gid.slice(0, 12) + '… did not open — ' +
                (e && e.message || e));
            return acc;
          });
        });
      }, Promise.resolve([]));
    });
  }

  /* ── what object.* stands on ─────────────────────────────────────────────── */

  /* A refusal that knows who said no. The port reply carries it as `refusal`
     beside the flat `e`, and a consumer takes `by` from there and nowhere else
     (shared/refusal-cases.json): core | binding | keyholder. */
  function refusal(by, why, op) {
    var e = new Error(why);
    e.refusal = { by: by, why: why, op: op };
    return e;
  }
  /* A core call whose refusal is the CORE's, in its words. */
  function coreJson(L, fn, obj, op) {
    try { return callJson(L.e, fn, obj); }
    catch (e) { throw refusal('core', String(e && e.message || e), op); }
  }
  /* Encrypt a built delta for the group, publish it, and only once the relay has
     accepted it, add it to this device's log. The ratchet moved when it
     encrypted, so the checkpoint comes before the wire. */
  function sendDelta(L, ws, gid, deltaHex, what) {
    var keys = epochKeys(L, gid);
    var msg = callJson(L.e, L.e.mls_encrypt, { group_id: gid, delta: deltaHex }).message;
    return checkpointLeaf(L)
      .then(function () { return pubOK(ws, keys, sealTo(L, keys, msg), false, what); })
      .then(function (ok) {
        if (!ok) throw refusal('keyholder', 'the relay did not accept it, so nothing was written', what);
        return appendLog(gid, { author: L.state.cred, envelope: deltaHex });
      });
  }

  /* ── THE SPINE, from the browser ──────────────────────────────────────────
     An account names its objects in one chain of sealed entries, at addresses
     derived from the seed, so a device that has never seen the account finds them
     from the 24 words alone (spine_addresses / spine_place / spine_read). The
     native mint queues an entry and its drain publishes it. The browser wrote
     none, so everything it minted could not be named on recovery, and nothing
     said so (chain, 22 Sep). This is that step, in the browser.

     THE INDEX, until indices are claimed account-wide: the first index whose tag
     holds nothing. Two of a person's devices writing at the same moment can land
     on one index, and spine_read tolerates two blobs there. It never writes over
     an occupied tag, because the walk stops only at an empty one. There is no
     head to trust (none advances yet), so the walk does not ask for one. */
  var SPINE_GEN = 0, SPINE_BATCH = 32, SPINE_MAX = 4096;
  function spineSeed() {
    return sealedGet('mls-seed').then(function (rec) {
      if (!rec || !rec.seed) {
        throw refusal('keyholder', 'no account seed on this device, so its spine cannot be read or written', 'spine');
      }
      return rec.seed;
    });
  }
  /* Every entry the relay holds for this account, index by index, stopping at the
     first index with nothing at it. Every blob at a tag is kept, not only the
     first: until indices are claimed, one index can hold two. */
  function spineWalk(L, ws, seed) {
    var fetched = [];
    function batch(from) {
      if (from >= SPINE_MAX) return Promise.resolve(from);
      var addrs = callJson(L.e, L.e.spine_addresses, {
        seed: seed, gen: SPINE_GEN, from: from, count: SPINE_BATCH
      }).addresses;
      function one(i) {
        if (i >= addrs.length) return batch(from + addrs.length);
        var a = addrs[i], blobs = [];
        return sub(ws, a.tag, function (blob) { blobs.push(blob); }).then(function () {
          if (!blobs.length) return a.index;
          fetched.push({ index: a.index, blobs: blobs });
          return one(i + 1);
        });
      }
      return one(0);
    }
    return batch(0).then(function (next) { return { next: next, fetched: fetched }; });
  }
  /* Name a group on this account's spine, ONCE. A group the spine already names is
     left alone: a second device of the same person joining a group the account is
     already in adds nothing, as natively. */
  function nameOnSpine(L, ws, gid, firstEpoch, what) {
    return spineSeed().then(function (seed) {
      return spineWalk(L, ws, seed).then(function (w) {
        var named = callJson(L.e, L.e.spine_read, { seed: seed, head: null, fetched: w.fetched }).objects || [];
        if (named.some(function (o) { return o.group_id === gid; })) return { index: null, already: true };
        var p = callJson(L.e, L.e.spine_place, {
          seed: seed, gen: SPINE_GEN, index: w.next,
          body: { group: { group_id: gid, first_epoch: firstEpoch } }
        });
        return pubOK(ws, { tag: p.tag, address_seed: p.address_seed }, p.blob, false, what).then(function (ok) {
          if (!ok) {
            throw refusal('keyholder', 'the relay did not take the spine entry, so this object could not ' +
                          'be named from the recovery words', what);
          }
          return { index: p.index, already: false };
        });
      });
    });
  }
  /* A mint that could not be named is undone, before anything of it was published:
     the MLS group existed only here, so forgetting it leaves nothing anywhere. */
  function unmint(L, gid) {
    L.state.groups = (L.state.groups || []).filter(function (g) { return g !== gid; });
    if (L.state.meta && L.state.meta[gid]) { var m = {}, k; for (k in L.state.meta) if (k !== gid) m[k] = L.state.meta[k]; L.state.meta = m; }
    try { mlsJson(L, L.e.mls_forget_group, gid); } catch (e) {}
    return checkpointLeaf(L);
  }

  /* JOIN FROM AN INTRO: the way the phone joins (`process_intro_blob`), and the
     only join that arrives with a GEN FLOOR. The adder sealed the Welcome to this
     device's intro tag together with the object's owner, its kind and the mark its
     log showed; `mls_join_intro` opens it and checks the sender and the owner
     against the roster it joined. The floor is kept per group (noteMeta takes the
     max across re-adds), and `object.author` hands it to the core as the
     watermark, so commutative ops on a joined group are no longer refused.
     Named on the spine at the joined epoch, as a raw join is; a naming failure is
     said and not undone, because the join already happened in MLS. */
  function joinFromIntro(L, ws, blobB64, what) {
    var j = mlsJson(L, L.e.mls_join_intro, JSON.stringify({ blob_b64: blobB64 }));
    var m = { kind: j.kind };
    if (typeof j.gen_watermark === 'number') m.floor = j.gen_watermark;
    var spine = null;
    return noteGroup(L, j.group_id)
      .then(function () { return noteMeta(L, j.group_id, m); })
      .then(function () {
        return nameOnSpine(L, ws, j.group_id, j.epoch, what).then(function (s) { spine = s; },
          function (e) { say(what, String(e && e.message || e)); });
      })
      .then(function () { return watchGroup(ws, j.group_id); })
      .then(function (k) {
        return { group_id: j.group_id, name: j.name, kind: j.kind, owner: j.owner, from: j.scanner,
                 epoch: k.epoch, tag: k.tag,
                 floor: typeof j.gen_watermark === 'number' ? j.gen_watermark : null,
                 spine_index: spine ? spine.index : null, named: !!spine };
      });
  }

  /* WHO MAY WRITE through object.*: the local driver origin the smokes use, and
     nothing in production, until Ralph rules (see THE WRITES). Reads (object.fold,
     ops.capabilities) answer every origin `ALLOWED` names. The older group.*
     methods are not narrowed here: they are what the smokes and the pages use
     today, and narrowing them is part of the same ruling, not a side effect of
     this one. */
  var WRITERS = { 'http://localhost:8100': true };
  var WRITES = { 'object.mint': true, 'object.author': true };

  /* ── the wire ────────────────────────────────────────────────────────────
     One request, one reply, matched by id. `subscribe` is the exception: it
     replies once and then pushes, which is what a replica converging from
     somewhere else looks like from in here. */
  var METHODS = {
    'device.identity': function () {
      return device().then(function (d) {
        return { pk: hex(d.pk), created: d.created, agent: d.agent };
      });
    },
    /* Proof the key is usable without being readable: the host can get a
       signature and can never get the thing that made it. */
    'device.sign': function (a) {
      return device().then(function (d) {
        var msg = new TextEncoder().encode(String(a && a.message || ''));
        return crypto.subtle.sign({ name: 'Ed25519' }, d.keys.privateKey, msg);
      }).then(function (sig) { return { sig: hex(sig) }; });
    },
    'store.load': function (a) {
      return state().then(function (st) {
        if (st.world) return st.world;
        /* First run on this device: the seed is genesis, so it becomes the
           checkpoint at an empty vector rather than a delta nobody authored. */
        if (a && a.seed) return put(cpKey(), { at: {}, world: a.seed }).then(function (cp) { return cp.world; });
        return null;
      });
    },
    'store.commit': function (a) {
      return device().then(function (d) { return append(a.op, hex(d.pk)); })
        .then(function (r) { return publish(r.delta).then(function () { return r.world; }); });
    },
    /* What sync will ship. Not wired to a relay yet; it is here because a log
       whose contents cannot be enumerated is a log that cannot converge, and
       building it without the reader is how that gets discovered late. */
    'store.deltas': function (a) {
      return logAll().then(function (ds) {
        var since = (a && a.since) || {};
        return order(tail(ds, since)).map(function (d) {
          return { id: d.id, a: d.a, seq: d.seq, at: d.at, op: d.op };
        });
      });
    },

    /* ── pairing ───────────────────────────────────────────────────────────
       Two calls. `pair.offer` on the device you are holding, `pair.accept` on
       the one you are adding — the offer string is what a QR carries. Both ends
       derive the same tag and the same seal key and neither ever sends one. */
    'pair.offer': function () {
      if (pairing) return { offer: hex(pairing.pub) };
      return x25519().then(function (kp) {
        return crypto.subtle.exportKey('raw', kp.publicKey).then(function (raw) {
          pairing = { keys: kp, pub: new Uint8Array(raw) };
          return { offer: hex(pairing.pub) };
        });
      });
    },
    'pair.accept': function (a) {
      if (!a || !a.offer) throw new Error('no offer');
      return Promise.resolve(METHODS['pair.offer']()).then(function () {
        return crypto.subtle.importKey('raw', unhex(a.offer), { name: 'X25519' }, false, []);
      }).then(function (theirs) {
        return crypto.subtle.deriveBits({ name: 'X25519', public: theirs }, pairing.keys.privateKey, 256);
      }).then(deviceChannelFrom).then(function (ch) {
        pairing = null;                 /* the exchange key has done its one job */
        chan = ch;
        /* The derived key is stored as a live CryptoKey, the same way the device
           key is: it survives a reload without ever being bytes in JavaScript. */
        return putChannel(ch).then(function () { return { tag: ch.tag }; });
      });
    },

    /* Derive this device's channel from the account's passkey. Callable from
       the frame, but a cross-origin frame is at the browser's discretion for
       WebAuthn and the gesture does not cross a MessagePort — so the reliable
       caller is the window this file opens as, below. */
    'account.link': function () { return accountChannel(); },
    /* Re-read rather than report what init found. The door that agrees a
       channel is a WINDOW on this origin, and it writes to this database after
       this frame has already loaded — so a frame that answered from memory
       would say "not linked" for the rest of its life about a device that is. */
    'account.linked': function () {
      return loadChannel().then(function (c) {
        if (c) chan = c;
        return { linked: !!c, from: (c && c.from) || null, tag: (c && c.tag) || null };
      });
    },

    /* ── sync ──────────────────────────────────────────────────────────────
       Subscribe to the channel, unseal what arrives, and put it in the log. A
       delta from the other device carries ITS author and ITS seq, so it slots in
       beside ours rather than over anything — which is the whole reason the log
       is keyed (author, seq) and not by arrival. */
    'sync.start': function (a) {
      if (!chan) throw new Error('not paired');
      if (sock) return { tag: chan.tag, already: true };
      /* WHICH RELAY, and why this is not a constant. An explicit override wins
         because a harness has to be able to say so; otherwise it is the Arc the
         SESSION names, and only if there is no session at all does it fall back
         to the local one. A device that silently synced from the wrong relay
         would look exactly like a device that had nothing to sync — the same
         empty screen for "we are not talking" and "there is nothing to say" —
         so the Arc is resolved here rather than assumed to have been. */
      return ((a && a.relay) ? Promise.resolve(a.relay)
             : arcUrl ? Promise.resolve(arcUrl)
             : session().then(function (s) {
                 arcUrl = s.arc || arcUrl;
                 return arcUrl || 'ws://localhost:8787';
               })
      ).then(function (url) { return dial(url); });

      function dial(url) {
      return relay(url, chan.tag, function (blob) {
        unseal(chan, blob).then(function (d) {
          /* AN ARCHIVE, NOT A DELTA. The other device's whole history rides the
             same channel as its deltas — sealed the same way, addressed the same
             way — and is told apart by its `t`. It folds through the core and
             becomes the world; it never enters the log, which holds this
             interior's ops and not the core's envelopes. A fold that fails is
             said out loud with its reason: it is the most interesting thing this
             channel can carry and the last thing to drop quietly. */
          if (d && d.t === 'archive') {
            return importArchive(d.cbor_b64).catch(function (e) {
              console.error('[pacific] the archive did not import: ' + String(e && e.message || e));
              /* NEVER SILENT. An archive that failed to land is history the member
                 believes they now have; a console line is not a signal. Tell the
                 interior, which is the only surface they are looking at. */
              if (port_) port_.postMessage({ t: 'pacific.error', where: 'archive.import',
                                             why: String(e && e.message || e) });
            });
          }
          return logAll().then(function (ds) {
            var have = ds.some(function (x) { return x.a === d.a && x.seq === d.seq; });
            if (have) return;                        /* idempotent re-delivery */
            /* Refold with the arrival included and hand the interior the result.
               The other device's delta lands in the same fold ours do — there is
               no separate "remote" path, which is the point of a log. */
            return logPut(d).then(state).then(function (st) {
              if (port_ && st.world) port_.postMessage({ t: 'pacific.push', v: st.world });
            });
          });
        }).catch(function (e) { console.warn('[pacific] undecryptable blob dropped', e.name); });
      }).then(function (ws) {
        sock = ws;
        /* Everything this device has already authored, so a device paired later
           catches up. Bounded by the log; the archive handover is the answer for
           a log that has outgrown a re-publish. */
        /* ONE AT A TIME, AND THROUGH `pub`. The relay answers every publish
           with exactly one ack, in order, and an Ack names no tag — so the
           waiters are a queue. A `Promise.all` of raw sends put N blobs on the
           wire with no waiters behind them at all, which is how the verdict on
           a member's whole backlog came to be thrown away. */
        return logAll().then(function (ds) {
          var refused = 0;
          return ds.reduce(function (chain, d) {
            return chain.then(function () {
              return seal(chan, d)
                .then(function (b) { return pubOK(ws, chan, b, false, 'sync.start'); })
                .then(function (ok) { if (!ok) refused++; });
            });
          }, Promise.resolve()).then(function () {
            return { tag: chan.tag, relay: url, published: ds.length - refused,
                     refused: refused };
          });
        }).then(function (out) {
          /* THE OTHER ARM. The account channel above carries this person's own
             devices. A GROUP carries everybody else, and it is the arm this file
             did not have: a listener on each group's current epoch tag that
             unseals, calls `mls_decrypt` and reports the delta. A browser with no
             account has no leaf and therefore no groups — that is not an error,
             and must not take the device channel down with it. */
          return watchAllGroups(ws).then(function (gs) {
            out.groups = gs;
            return out;
          }, function (e) {
            say('sync.start', 'the group arm did not start — ' + (e && e.message || e));
            out.groups = [];
            return out;
          });
        });
      });
      }
    },
    /* Who is signed in, and where their Arc is. Never the cookie. */
    'session.get': function () {
      return session().then(function (s) {
        arcUrl = s.arc || arcUrl;
        return s;
      });
    },
    /* ── signing out ───────────────────────────────────────────────────────
       WHAT BELONGS TO THE ACCOUNT AND WHAT BELONGS TO THE DEVICE. Sign-out is
       only meaningful if that line is drawn, and it is not obvious:

       THE COOKIES ARE THE ACCOUNT'S. Only the service can drop them — they are
       httpOnly, which is the whole point of them, so `document.cookie` cannot
       see them and this cannot clear them locally. It has to ask, and if the
       ask fails it must say so rather than draw a signed-out screen over a live
       session. That is why a failed POST throws here instead of being smoothed
       over: an interface that says "signed out" while the cookie still opens
       the account is worse than one that says the sign-out did not work.

       THE CHANNEL IS THE ACCOUNT'S TOO, and this is the part worth stating.
       `account.link` derives it from the account's passkey, so it is a
       credential of that account wearing a different shape. Leaving it behind
       would mean the next person to sign in on this browser inherits the last
       one's sync channel and starts publishing their deltas into a stranger's
       mailbox. It goes. Re-linking is one ceremony.

       THE DEVICE KEY STAYS, and deliberately. The Ed25519 public half IS the
       identity — `identity.rs` holds the same key and an MLS leaf carries those
       32 bytes as its BasicCredential id — so discarding it would not sign a
       person out, it would make them a different person to every peer they have
       ever met, with no way to prove the two were ever the same. That is device
       destruction, and it is a different button with a different warning on it.

       THE LOG STAYS for the same reason in a plainer form: those deltas are the
       member's own writing, this device may hold the only copy, and nobody asks
       to be signed out meaning "and delete what I wrote". */
    'session.signout': function () {
      /* Stop talking first. A socket that outlived the session would keep
         draining a mailbox for an account that is no longer here. */
      if (sock) { try { sock.close(); } catch (e) {} sock = null; }
      /* NOTHING TO ASK PERMISSION FOR ANY MORE. This used to POST /auth/signout
         and refuse to draw a signed-out screen unless the service said the
         cookie was dropped — correct then, and meaningless now: there is no
         cookie, so what signs this browser out is dropping the seed, and only
         this browser can do that. It cannot fail and it cannot be refused.

         THE SEED GOES, and with it the channel derived from it. The MLS state
         goes too, and that is the part worth stating: a leaf is bound to the
         identity that made it, so leaving it behind would mean the next person
         to sign in on this browser inherits a stranger's ratchet.

         THE LOG STAYS. Those deltas are the member's own writing, this device
         may hold the only copy, and nobody asks to be signed out meaning "and
         delete what I wrote". */
      chan = null;
      arcUrl = null;
      _leaf = null;
      return Promise.all([del('account'), del('channel'), del('channel-addr'), del('mls-state'), del('mls-seed')])
        .then(function () { return { signedIn: false }; });
    },

    /* ── the account, on the origin that holds it ──────────────────────────
       These three are the whole of signing in. They run HERE rather than on
       the member's page because the seed must land on this origin and nowhere
       else — and because a WebAuthn ceremony needs a first-party context, which
       a framed document does not have. The door (below) calls them with a user
       gesture behind it; the RPC surface exposes them so a harness can too. */
    'account.status': function () {
      return accountRec().then(function (rec) {
        return rec ? askArc(rec) : { signedIn: false };
      });
    },

    'account.create': function () {
      return pacific().then(function (a) {
        return a.create({}).then(function (made) { return adopt(a, made, made.words); });
      });
    },

    'account.signin': function () {
      return pacific().then(function (a) {
        return a.signIn({}).then(function (got) { return adopt(a, got); });
      });
    },

    'account.restore': function (arg) {
      return pacific().then(function (a) {
        return a.restore(String(arg && arg.words || '')).then(function (got) {
          return adopt(a, got);
        });
      });
    },

    /* The local harness serves the keyholder from its own port, so the auth
       service is not same-origin there. Production leaves this unset. */
    'session.origin': function (a) {
      AUTH = (a && a.origin) || '';
      return { origin: AUTH || 'same-origin' };
    },

    'sync.status': function () {
      return { paired: !!chan, connected: !!sock, tag: chan ? chan.tag : null };
    },

    /* ── the archive ───────────────────────────────────────────────────────
       `archive.import` is the receiving end: callable directly — a harness, a
       blob a member fetched from storage under a key they hold — and reached
       by the channel above when a sealed archive arrives. `archive.export` is
       the sending end for a device that holds one: sealed with the channel key
       and published on the tag, in the SAME envelope a phone sends,
       `{t:'archive', cbor_b64}`, so the receiver has one shape to recognise
       whoever sent it. This browser does not build an Archive of its own — its
       log holds the interior's ops, not the core's envelopes — so the bytes
       come from the caller, and a caller with none is told so. */
    'archive.import': function (a) { return importArchive(a && a.cbor_b64, a && a.me); },
    'archive.export': function (a) {
      if (!a || typeof a.cbor_b64 !== 'string' || !a.cbor_b64) {
        throw new Error('archive.export wants cbor_b64 — the archive as Archive::to_cbor encodes it, base64');
      }
      if (!chan) throw new Error('not paired — there is no channel to seal to');
      if (!sock) throw new Error('not syncing — call sync.start first');
      /* THROUGH `pub`, and the verdict is the answer. A handover the relay
         refused is a history the other device will never receive, and returning
         `{sealed: n}` regardless said the opposite. */
      return seal(chan, { t: 'archive', cbor_b64: a.cbor_b64 }).then(function (b) {
        return pubOK(sock, chan, b, false, 'archive.export').then(function (ok) {
          if (!ok) throw new Error('the relay refused the archive — it was not stored, ' +
                                   'so the other device will not receive this history');
          return { tag: chan.tag, sealed: b.length, published: true };
        });
      });
    },

    /* ── pairing, as a person ──────────────────────────────────────────────
       The contact bundle a peer scans to add this device — see the note on
       the leaf for whose key signs it. Each call mints a fresh one-time
       KeyPackage, so each call is a new bundle, and the state is checkpointed
       after it or the bundle could never be accepted. `name` is what the
       scanner will see, and it has no default: a bundle with a blank name is a
       card with no name on it. */
    'device.bundle': function (a) {
      var name = a && typeof a.name === 'string' ? a.name.trim() : '';
      if (!name) throw new Error('device.bundle wants a name — the one the scanner will see');
      return leaf().then(function (L) {
        var out = JSON.parse(text(call(L.e, L.e.mls_contact_bundle, utf8(name))));
        if (!out.bundle) throw new Error('mls_contact_bundle returned no bundle');
        return checkpointLeaf(L).then(function () {
          return { bundle: out.bundle, identity: L.state.cred, intro: L.state.intro, name: name };
        });
      });
    },

    /* ── the group, as RPC ─────────────────────────────────────────────────
       WHAT IS REAL HERE. `group.send` puts a canonical-CBOR Delta the CORE
       authored inside an MLS application message and onto the epoch's mailbox;
       the arm in `sync` decrypts one and attributes it to the sender's leaf.
       That is the information architecture, not a stand-in for it: a second
       device folding the same group reaches the same state, because the bytes
       are the same bytes.

       WHAT IS NOT. Nothing folds a received Forum delta into the interior's
       world. The interior draws from `applyOp` and ten invented ops that are in
       no ICD and have no reducer in the core; those two vocabularies are not the
       same vocabulary and joining them is a separate piece of work. A delta that
       arrives is reported, and stops there.

       THE TWO HALVES OF AN INVITE RUN ON TWO DEVICES. The newcomer mints a
       KeyPackage with `group.keyPackage` on THEIR device — its private half
       lives in THEIR snapshot and nowhere else — and the inviter passes it to
       `group.invite`. That is why they are two methods and not one. */
    'group.list': function () {
      return leaf().then(function (L) {
        return { groups: (L.state.groups || []).map(function (gid) {
          try {
            var k = epochKeys(L, gid);
            return { group_id: gid, epoch: k.epoch, tag: k.tag };
          } catch (e) { return { group_id: gid, error: String(e && e.message || e) }; }
        }) };
      });
    },

    'group.create': function (a) {
      var name = a && typeof a.name === 'string' ? a.name.trim() : '';
      if (!name) throw new Error('group.create wants a name — a group with no name on it is a card with no name on it');
      return Promise.all([leaf(), socket(a && a.relay)]).then(function (r) {
        var L = r[0], ws = r[1];
        var gid = mlsJson(L, L.e.mls_create_group, name).group_id;
        /* Checkpointed before anything else can happen to it: the snapshot is the
           only copy of the group's ratchet, and a group whose state died with the
           page is a group nobody can ever speak in. Its gen floor is 0: the device
           that made it has seen all of it (see noteMeta). And it is NAMED on the
           spine, or undone: a group the seed cannot name returns on no other
           device (see THE SPINE). */
        return noteGroup(L, gid)
          .then(function () { return noteMeta(L, gid, { floor: 0 }); })
          .then(function () { return nameOnSpine(L, ws, gid, 0, 'group.create'); })
          .then(null, function (e) { return unmint(L, gid).then(function () { throw e; }); })
          .then(function (s) {
            var k = epochKeys(L, gid);
            return { group_id: gid, name: name, epoch: k.epoch, tag: k.tag, spine_index: s.index };
          });
      });
    },

    /* The newcomer's half. Each call mints a fresh one-time KeyPackage whose
       private half is in the snapshot, so the checkpoint is not optional — a
       bundle whose private half was in a page that has since reloaded is a
       bundle nobody can ever accept. */
    'group.keyPackage': function () {
      return leaf().then(function (L) {
        var kp = mlsJson(L, L.e.mls_key_package, '').key_package;
        return checkpointLeaf(L).then(function () { return { key_package: kp }; });
      });
    },

    /* BY BUNDLE, and then the newcomer arrives with a floor. `bundle` is the
       newcomer's contact card (`device.bundle` on THEIR device, or a phone's). It
       carries the KeyPackage to add AND the intro tag to deliver to, so once the
       commit is accepted the Welcome goes to their intro mailbox sealed with the
       owner, the kind and the gen floor this device's log shows (`intro_seal`) —
       what the phone's `group_add_member` sends. The newcomer joins it with
       `group.join {intro}` or `group.inbox`.

       BY KEY_PACKAGE, the older way, the Welcome comes BACK raw and delivery is
       the caller's. It carries no floor, so the newcomer can write no commutative
       op in the group; `floor: null` in the answer says so. */
    'group.invite': function (a) {
      var gid = a && a.group_id, kp = a && a.key_package, bundle = a && a.bundle;
      if (!gid) throw new Error('group.invite wants group_id');
      if (!kp && !bundle) {
        throw new Error('group.invite wants bundle — the newcomer\'s contact card, from ' +
                        'device.bundle on THEIR device — or key_package for a raw Welcome ' +
                        'that carries no gen floor');
      }
      return Promise.all([leaf(), socket(a && a.relay)]).then(function (r) {
        var L = r[0], ws = r[1];
        var peer = bundle ? callJson(L.e, L.e.bundle_read, { bundle: bundle }) : null;
        var meta = groupMeta(L, gid);
        var kind = meta.kind || (a && typeof a.kind === 'string' ? a.kind : '');
        /* The kind rides in the intro and becomes the newcomer's record of what this
           is. A guess would be worse than the question: an object this device did
           not mint through object.mint may have no kind on record. */
        if (peer && !kind) {
          throw new Error('group.invite: this device holds no kind for ' + gid.slice(0, 12) +
                          '… — pass kind, or the newcomer is told nothing true about what it joined');
        }
        if (peer) kp = peer.key_package;
        /* READ THE KEYS BEFORE THE ADD. The commit has to land on the tag every
           OTHER member is still draining, and `mls_add_member` advances this
           client's epoch — so keys read afterwards would address a mailbox only
           this device is in, and the add would be invisible to the group. */
        var before = epochKeys(L, gid);
        var out = callJson(L.e, L.e.mls_add_member, { group_id: gid, key_package: kp });
        return checkpointLeaf(L).then(function () {
          /* RFC 9420 §14, and the entire reason `pub`'s commit flag exists: the
             commit claims the tag's ONE slot, and the Welcome is only valid once
             that commit is accepted. `pubOK` throws if it was not — which is the
             hole this file had, where a lost epoch race was silent. */
          return pubOK(ws, before, sealTo(L, before, out.commit), true, 'group.invite');
        }).then(function () {
          return watchGroup(ws, gid);
        }).then(function (now) {
          if (!peer) {
            /* Raw: the Welcome comes BACK, and delivery is the caller's ceremony. */
            return { group_id: gid, welcome: out.welcome, epoch: now.epoch, tag: now.tag,
                     committed: true, floor: null };
          }
          /* After the commit was accepted, never before (RFC 9420 §14). */
          return readLog(gid).then(function (log) {
            var ro = JSON.parse(text(call(L.e, L.e.mls_roster, utf8(gid))));
            var intro = callJson(L.e, L.e.intro_seal, {
              bundle: bundle, welcome: out.welcome, kind: kind, owner: ro.owner,
              name: (a && typeof a.name === 'string') ? a.name : '',
              log: log, floor: typeof meta.floor === 'number' ? meta.floor : null
            });
            return pubOK(ws, { tag: intro.tag, address_seed: intro.address_seed }, intro.blob, false,
                         'group.invite').then(function (ok) {
              if (!ok) {
                throw new Error('the relay did not take the intro, so ' + String(peer.display_name) +
                                ' is in the group and has not been told — the add stands, the ' +
                                'Welcome is not delivered');
              }
              return { group_id: gid, epoch: now.epoch, tag: now.tag, committed: true,
                       delivered: true, to: peer.identity, floor: intro.gen_watermark };
            });
          });
        });
      });
    },

    /* `intro` (the sealed blob a bundle invite delivered) joins WITH the floor;
       `welcome` (a raw Welcome) joins without one. */
    'group.join': function (a) {
      var w = a && a.welcome, intro = a && a.intro;
      if (!w && !intro) {
        throw new Error('group.join wants intro — the blob group.invite delivered to this device\'s ' +
                        'intro mailbox — or welcome, the raw hex, which carries no gen floor');
      }
      if (intro) {
        return Promise.all([leaf(), socket(a && a.relay)]).then(function (r) {
          return joinFromIntro(r[0], r[1], String(intro).trim(), 'group.join');
        });
      }
      return Promise.all([leaf(), socket(a && a.relay)]).then(function (r) {
        var L = r[0], ws = r[1];
        var joined = mlsJson(L, L.e.mls_join, String(w).trim());
        var spine = null;
        return noteGroup(L, joined.group_id)
          /* Named on the spine at the epoch it was joined: that is the floor of
             what this account may read. A group the account is already named in
             (a second device of the same person) adds nothing. A failure here is
             SAID and not undone: the join already happened in MLS, and the other
             members hold the commit that made it. */
          .then(function () {
            return nameOnSpine(L, ws, joined.group_id, joined.epoch, 'group.join').then(function (s) { spine = s; },
              function (e) { say('group.join', String(e && e.message || e)); });
          })
          .then(function () { return watchGroup(ws, joined.group_id); })
          .then(function (k) {
            return { group_id: joined.group_id, name: joined.name,
                     epoch: k.epoch, tag: k.tag,
                     spine_index: spine ? spine.index : null, named: !!spine };
          });
      });
    },

    /* THIS DEVICE'S INTRO MAILBOX: where an adder holding this device's bundle
       delivers a Welcome, whether a phone's `group_add_member` or another
       browser's `group.invite`. Drained from the start (the relay keeps
       everything), joined in order, and then LEFT OPEN, so an add that lands
       later is joined as it arrives, as the phone's sync does. A seq already
       handled is skipped. One that will not join (a forgery, a KeyPackage minted
       by another device, a replay) is reported by name and not retried. */
    'group.inbox': function (a) {
      return Promise.all([leaf(), socket(a && a.relay), core()]).then(function (r) {
        var L = r[0], ws = r[1], e = r[2];
        if (!L.state.intro) throw new Error('this device has no intro tag — mls_init mints one');
        var tag = hex(call(e, e.relay_address, unhex(L.state.intro)));
        if (ws._tags[tag]) return { tag: tag, already: true, joined: [], refused: [] };
        var handled = function (seq) {
          L.state.introSeq = Math.max(L.state.introSeq || 0, seq || 0);
          return checkpointLeaf(L);
        };
        var take = function (blob, seq) {
          if (typeof seq === 'number' && seq <= (L.state.introSeq || 0)) return Promise.resolve(null);
          return joinFromIntro(L, ws, blob, 'group.inbox').then(
            function (j) { return handled(seq).then(function () { return { joined: j }; }); },
            function (err) {
              return handled(seq).then(function () { return { refused: { seq: seq, why: String(err && err.message || err) } }; });
            });
        };
        var backlog = [];
        return sub(ws, tag, function (blob, seq) { backlog.push({ blob: blob, seq: seq }); }).then(function () {
          var joined = [], refused = [];
          return backlog.reduce(function (p, m) {
            return p.then(function () {
              return take(m.blob, m.seq).then(function (o) {
                if (o && o.joined) joined.push(o.joined);
                if (o && o.refused) refused.push(o.refused);
              });
            });
          }, Promise.resolve()).then(function () {
            ws._tags[tag] = function (blob, seq) {
              take(blob, seq).then(function (o) {
                if (o && o.joined && port_) port_.postMessage({ t: 'pacific.group.joined', joined: o.joined });
                if (o && o.refused) say('group.inbox', 'an intro at seq ' + o.refused.seq + ' did not join: ' + o.refused.why);
              });
            };
            return { tag: tag, joined: joined, refused: refused };
          });
        });
      });
    },

    'group.send': function (a) {
      var gid = a && a.group_id;
      if (!gid) throw new Error('group.send wants group_id');
      return Promise.all([leaf(), socket(a && a.relay)]).then(function (r) {
        var L = r[0], ws = r[1];
        var keys = epochKeys(L, gid);
        /* THE DELTA IS THE CORE'S. `forum_post` is the app's own constructor: it
           stamps the type_id and puts `gen` INSIDE the args as well as on the
           envelope, which the ICD's payload schema does not say and a
           hand-assembled delta therefore gets wrong — encoding to something the
           fold accepts and then ignores. core-wasm's own comment records that
           bug. A pre-authored `delta` (hex canonical CBOR) is taken as-is for a
           caller that has one. */
        var deltaHex;
        if (typeof a.delta === 'string' && a.delta) {
          deltaHex = a.delta;
        } else {
          if (typeof a.text !== 'string' || !a.text) {
            throw new Error('group.send wants text, or a pre-authored delta as hex canonical CBOR');
          }
          deltaHex = hex(call(L.e, L.e.forum_post, utf8(JSON.stringify({
            text: a.text,
            /* LAMPORT, not a counter: 1 + the highest gen this author has seen.
               The caller knows the transcript; this file does not, so it is an
               argument with an honest default rather than an invention. */
            gen: typeof a.gen === 'number' ? a.gen : 1,
            epoch: keys.epoch
          }))));
        }
        var msg = callJson(L.e, L.e.mls_encrypt, { group_id: gid, delta: deltaHex }).message;
        /* The ratchet moved when it encrypted. Checkpoint before the wire, so a
           page that dies mid-publish is not holding a stale ratchet. */
        return checkpointLeaf(L)
          .then(function () { return pubOK(ws, keys, sealTo(L, keys, msg), false, 'group.send'); })
          .then(function (ok) {
            /* Our own message does not come back to us as an application message
               (it arrives as `own`), so it joins our log here, and ONLY once the
               relay accepted it. A delta that never reached the group was never
               in it, and a fold that showed it would be showing a message nobody
               else will ever see. Authored by this leaf's credential, which is
               what `mls_decrypt` reports as `sender` on every other device. */
            var kept = ok ? appendLog(gid, { author: L.state.cred, envelope: deltaHex }) : Promise.resolve();
            return kept.then(function () {
              return { group_id: gid, epoch: keys.epoch, tag: keys.tag,
                       delta: deltaHex, published: ok };
            });
          });
      });
    },

    /* THE FOLD, of the groups this device holds live, by the CORE. Each group's
       roster, owner and name come from its own MLS state (`mls_roster`), its log
       from what this device received, and `fold_object` does the rest: the same
       fold_one `fold_archive` runs, so a live screen and a restored one cannot
       disagree. The answer is `fold_archive`'s shape, {groups, rejected}, so
       the shared fold's normalise() takes it unchanged. (Named that way, not by
       file: stage.sh stages a shared file wherever its name appears.)

       A group that will not fold is not dropped. It is in `rejected` in the
       core's own words, and one bad group does not blank the others: the skip
       is the bug, not the failure. A group with nothing in its log yet cannot
       say what kind it is, and it says exactly that rather than being guessed.

       A READ, so it is served to every allowed origin. Writes (object.mint,
       object.author) are a different question, and it is Ralph's (see the
       object.* note below). */
    'object.fold': function (a) {
      var only = a && typeof a.group === 'string' && a.group ? a.group : null;
      /* EVERY OBJECT THIS DEVICE HOLDS is every object of the PERSON, across every
         site they belong to: another site's groups, their private conversations.
         A member site's page is not offered that list. It names the group it
         wants. Listing all of them answers only WRITERS, the first-party and
         dev origins. */
      if (!only && !WRITERS[origin_]) {
        throw refusal('keyholder', 'listing every object this device holds is not offered to a member site; ' +
                      'name the group', 'object.fold');
      }
      return leaf().then(function (L) {
        var gids = only ? [only] : (L.state.groups || []).slice();
        return Promise.all(gids.map(function (gid) {
          return readLog(gid).then(function (rows) { return [gid, rows]; });
        })).then(function (pairs) {
          var groups = [], rejected = [];
          pairs.forEach(function (p) {
            var gid = p[0];
            try {
              var r = JSON.parse(text(call(L.e, L.e.mls_roster, utf8(gid))));
              var input = { group_id: gid, owner: r.owner, members: r.members, name: r.name, log: p[1] };
              /* The kind this device recorded, when it has one: a Forum is minted
                 name-only, so its log is empty and names no kind of its own. */
              var kind = groupMeta(L, gid).kind;
              if (kind) input.kind = kind;
              groups.push(callJson(L.e, L.e.fold_object, input));
            } catch (e) {
              rejected.push({ group: gid, why: String(e && e.message || e) });
            }
          });
          return { exported_by: L.state.cred, groups: groups, rejected: rejected };
        });
      });
    },

    /* THIS ACCOUNT'S SPINE, as a device that has never seen it would read it:
       spine_read over every entry the relay holds. It is the account's whole list
       of objects, so it answers only WRITERS (see object.fold). */
    'spine.read': function (a) {
      if (!WRITERS[origin_]) throw refusal('keyholder', 'the spine lists every object of this account; ' +
                                           'it is not offered to a member site', 'spine.read');
      return Promise.all([leaf(), socket(a && a.relay), spineSeed()]).then(function (r) {
        var L = r[0], ws = r[1], seed = r[2];
        return spineWalk(L, ws, seed).then(function (w) {
          return callJson(L.e, L.e.spine_read, { seed: seed, head: null, fetched: w.fetched });
        });
      });
    },

    /* WHAT THIS PRODUCER CAN DO, per op, and nothing transcribed. The MODEL's
       facts (op id, authority, fold, fields) come from the generated table; which
       KINDS an op may be written onto comes from the core's authoring catalogue
       (`ops_on`), asked of the same check `authoring::build` makes, so a consumer
       holding `on` knows what the door will accept. It used to come from
       `ops_catalogue`, which says which op TABLES declare an op: that put note ops
       on a Site's `group`, which the door refuses, and left the ratify ops, which
       every object carries, on nothing (thedoor, 22 Sep). An op no kind declares,
       or one the table refuses (membership, recorded by the MLS doors), is
       refused with its reason. A commutative op is reachable but carries `needs`:
       the core refuses it on any object this device holds no gen floor for, and a
       consumer that knows that draws the right thing instead of a button that
       fails. `writes` says whether THIS origin may write at all (WRITERS). */
    'ops.capabilities': function () {
      var O = self.WallflowersOps;
      if (!O) {
        throw new Error('the generated op table did not load (wallflowers-ops.js is not staged beside this ' +
                        'page; app/web/build-wasm.sh stages it)');
      }
      return leaf().then(function (L) {
        var n = L.e.ops_on();
        var on = JSON.parse(text(new Uint8Array(L.e.memory.buffer, L.e.out_ptr(), n).slice()));
        var writes = !!WRITERS[origin_];
        var ops = {};
        Object.keys(O.OPS).forEach(function (k) {
          var o = O.OPS[k], kinds = on[k] || [];
          var why = !o.reachable ? o.why
                  : !kinds.length ? 'no kind in this core may carry ' + k
                  : null;
          ops[k] = {
            opId: o.opId, authority: o.authority, fold: o.fold, fields: o.fields,
            on: kinds, reachable: !why, why: why,
            needs: !why && o.fold === 'commutative'
              ? 'a gen floor on the object: 0 if this device minted it, the Welcome\'s mark if it joined through one'
              : null
          };
        });
        return { writes: writes, ops: ops };
      });
    },

    /* ── THE WRITES: object.mint and object.author ─────────────────────────────
       The one generic door (Ralph, 22 Sep: one authoring API), through the core's
       `authoring::build` (e0a9bf3) by way of core-wasm's `mint_ops` and
       `author_build`. The core refuses what it refuses, in its words: an op the
       kind does not declare, owner-only from a non-owner, a commutative op with
       no floor, and anything its probe says would do nothing. This file only
       holds the log and the MLS state, encrypts, and publishes.

       A write is DONE when the relay accepts it, and only then does it join this
       device's log. A delta that never reached the group was never in it.

       WHO MAY CALL THEM is not settled. A member site's port is usable by every
       script on that site, so a write offered there is a write every such script
       has. That is Ralph's to rule, with options put to him on 22 Sep (a gesture
       inside this frame, a per-origin scope, or both). Until then these answer
       only to WRITERS. */
    'object.mint': function (a) {
      var kind = a && typeof a.kind === 'string' ? a.kind : '';
      var draft = (a && a.draft) || {};
      var what = (kind || '?') + '.mint';
      if (!kind) throw refusal('binding', 'object.mint wants {kind, draft}', what);
      var name = typeof draft.name === 'string' ? draft.name.trim() : '';
      if (!name) throw refusal('binding', 'a draft needs a name: every kind has one', what);
      return Promise.all([leaf(), socket(a.relay)]).then(function (r) {
        var L = r[0], ws = r[1], me = L.state.cred;
        var ops = coreJson(L, L.e.mint_ops, { kind: kind, draft: draft }, what).ops;
        /* BOTH OR NEITHER. Every op is built, and so PROBED, against the log it
           will have, before the group exists: the object is new, so its epoch is
           0, its roster is this person and its owner is this person. A refused
           op 0 therefore makes nothing, rather than an empty object that says
           nothing about why. */
        var log = [], built = [];
        ops.forEach(function (op) {
          var b = coreJson(L, L.e.author_build, {
            kind: kind, op_id: op.op_id, args: op.args, me: me, epoch: 0,
            members: [me], owners: [[0, me]], log: log, watermark: 0
          }, what);
          built.push(b);
          log.push({ author: me, envelope: b.delta });
        });
        var gid = mlsJson(L, L.e.mls_create_group, name).group_id;
        var spine = null;
        return noteGroup(L, gid)
          .then(function () { return noteMeta(L, gid, { kind: kind, floor: 0 }); })
          /* NAMED FIRST, before anything of it is published. An object the seed
             cannot name comes back on no other device, so a mint that could not
             name it is undone, and at this point it has been nowhere but here. */
          .then(function () { return nameOnSpine(L, ws, gid, 0, what); })
          .then(function (s) { spine = s; }, function (e) {
            return unmint(L, gid).then(function () { throw e; });
          })
          .then(function () {
            return built.reduce(function (p, b) {
              return p.then(function () { return sendDelta(L, ws, gid, b.delta, what); });
            }, Promise.resolve());
          })
          .then(function () { return { group_id: gid, spine_index: spine.index }; });
      });
    },

    'object.author': function (a) {
      var gid = a && typeof a.group === 'string' ? a.group : '';
      var op = a && typeof a.op === 'string' ? a.op : '';
      if (!gid || !op) throw refusal('binding', 'object.author wants {group, op, args}', op || '?');
      var O = self.WallflowersOps, spec = O && O.OPS[op];
      if (!spec) throw refusal('binding', 'no op ' + op + ' in the ICD', op);
      var given = a.args || {}, args, k;
      /* GEN IS THE DOOR'S. `authoring::build` derives it from the log and the floor
         and writes it into args["gen"] itself, because a caller that computed gen
         is how the gen-0 collision happened. So a supplied gen is REFUSED, not
         quietly overwritten, and where the ICD lists gen as required it is filled
         in only so the schema check passes; the core replaces it. */
      if (spec.fold === 'commutative' && given.gen !== undefined) {
        throw refusal('binding', op + ': gen is assigned by the core from the log and the floor; do not supply it', op);
      }
      var shaped = {};
      for (k in given) shaped[k] = given[k];
      if (spec.fold === 'commutative' && spec.fields.some(function (f) { return f.name === 'gen'; })) shaped.gen = 0;
      try { args = O.check(spec, shaped); } catch (e) { throw refusal('binding', String(e.message || e), op); }
      return Promise.all([leaf(), socket(a.relay)]).then(function (r) {
        var L = r[0], ws = r[1];
        var ro;
        try { ro = JSON.parse(text(call(L.e, L.e.mls_roster, utf8(gid)))); }
        catch (e) { throw refusal('keyholder', 'this device holds no group ' + gid.slice(0, 12) + '…: ' + String(e.message || e), op); }
        var meta = groupMeta(L, gid);
        return readLog(gid).then(function (log) {
          var req = {
            op_id: spec.opId, args: args, me: L.state.cred, epoch: ro.epoch,
            members: ro.members,
            /* The owner the group's context names NOW. Owner-sequenced deltas an
               earlier owner wrote before a handover fold as a non-owner's here,
               which the core tolerates in history and does not refuse. */
            owners: [[0, ro.owner]],
            log: log,
            watermark: typeof meta.floor === 'number' ? meta.floor : null
          };
          if (meta.kind) req.kind = meta.kind;
          var b = coreJson(L, L.e.author_build, req, op);
          return sendDelta(L, ws, gid, b.delta, op).then(function () { return { deltaId: b.delta_id }; });
        });
      });
    },

    /* Start the receive arm without pairing. `sync.start` does this too, but it
       needs a device channel first, and a browser that is an MLS member need not
       have a second device of its own. */
    'group.watch': function (a) {
      return socket(a && a.relay).then(function (ws) {
        return watchAllGroups(ws).then(function (gs) { return { watching: gs }; });
      });
    },

    /* Deliberately present and deliberately impossible. Named so that anyone
       reaching for it finds the reason rather than a missing method. */
    'device.export': function () {
      throw new Error('the private key does not leave this origin — that is the point');
    }
  };

  /* THE op reducer — no longer a duplicate of one. The store is the authority on
     what a commit means, and a keyholder that applied whatever the host page sent
     would be a database with extra steps; now that `apply` has gone from
     pacific.js there is no second copy to keep in step, and the interior cannot
     apply an op it does not like the look of.

     That makes this the one place a new op must land. An op the interior authors
     and this switch does not know is not an error anywhere — it falls through,
     the world comes back unchanged, and the member watches their ballot vanish on
     the next redraw. When the fold moves to wasm, this is the function it
     replaces, and `wallet.rs` in pacific-core is what it replaces it with. */
  function applyOp(w, op) {
    var byId = function (l, id) {
      for (var i = 0; i < l.length; i++) if (l[i].id === id) return l[i];
      return null;
    };
    switch (op && op.t) {
      case 'post': {
        var th = byId(w.threads, op.thread); if (!th) break;
        th.posts.push({ by: op.by, at: op.at || 'now', s: op.s });
        th.n++; th.when = 'now'; break;
      }
      case 'conv':
        if (!byId(w.convs, op.id) && !w.convs.filter(function (c) { return c.with === op.with; })[0])
          w.convs.push({ id: op.id, with: op.with, unread: 0, msgs: [] });
        break;
      case 'say': {
        var c = byId(w.convs, op.conv); if (c) c.msgs.push({ me: 1, s: op.s }); break;
      }
      case 'read': {
        var r = byId(w.convs, op.conv); if (r) r.unread = 0; break;
      }
      case 'rsvp':   w.rsvp[op.event] = !!op.going; break;
      case 'invite': if (w.pending.indexOf(op.email) < 0) w.pending.push(op.email); break;

      /* The wallet. Decisions (propose/ballot/close) and the one statement about
         money that actually moved (settle) — the same four the interior authors,
         because a store that knew fewer ops than its client would drop ballots
         silently and the member would watch their vote vanish on redraw. */
      case 'propose': {
        if (!w.wallet || byId(w.wallet.proposals, op.id)) break;
        var pb0 = {};
        pb0[op.by] = 1;               /* the proposer approves their own */
        w.wallet.proposals.unshift({
          id: op.id, by: op.by, amount: op.amount, payee: op.payee, what: op.what,
          thread: op.thread || null, at: op.at || 'now', closed: null, ballots: pb0
        });
        break;
      }
      case 'ballot': {
        if (!w.wallet) break;
        var pb = byId(w.wallet.proposals, op.proposal);
        if (!pb || pb.closed) break;  /* a closed poll is sealed */
        pb.ballots[op.by] = op.b;     /* LWW per voter */
        break;
      }
      case 'close': {
        if (!w.wallet) break;
        var pc = byId(w.wallet.proposals, op.proposal);
        if (pc && !pc.closed) pc.closed = op.at || 'now';
        break;
      }
      case 'settle': {
        if (!w.wallet || byId(w.wallet.settlements, op.id)) break;
        w.wallet.settlements.unshift({
          id: op.id, proposal: op.proposal || null, amount: op.amount,
          at: op.at || 'now', by: op.by, ref: op.ref || '—', what: op.what || ''
        });
        break;
      }
    }
    return w;
  }

  /* ── init ────────────────────────────────────────────────────────────────
     The host page opens the channel; this side decides whether to speak. The
     port is taken exactly once — a second init on the same document is a page
     trying to get a channel it was not given, and is ignored. */
  var opened = false, chan = null, port_ = null, arcUrl = null;

  /* Ship one delta to the paired device. Silent when unpaired — a person with
     one device is the normal case, not an error. */
  function publish(d) {
    if (!chan || !sock) return Promise.resolve();
    /* Through `pub`, so the ack queue stays in lockstep and a refusal reaches
       the member. It does NOT throw: the delta is in this device's log either
       way, and failing `store.commit` over the paired device's copy would tell
       the member their own writing did not happen. `pubOK` says so instead. */
    return seal(chan, d).then(function (b) {
      return pubOK(sock, chan, b, false, 'store.commit');
    });
  }

  window.addEventListener('message', function (e) {
    var d = e.data;
    if (!d || d.t !== 'pacific.init' || opened) return;
    var port = e.ports && e.ports[0];
    if (!port) return;

    if (!permitted(d.site, e.origin)) {
      /* Refused loudly on the channel it arrived on, and not a byte more: the
         refusal must not say whether the site exists or whether anyone is
         signed in to it. */
      port.postMessage({ t: 'pacific.denied', why: 'origin not registered for this site' });
      port.close();
      return;
    }
    opened = true;
    site_ = d.site;
    origin_ = e.origin;
    port_ = port;

    /* A channel agreed on a previous run comes back with the device. */
    loadChannel().then(function (c) { if (c) chan = c; });

    port.onmessage = function (m) {
      var req = m.data || {}, fn = METHODS[req.m];
      Promise.resolve()
        .then(function () {
          if (!fn) throw new Error('no such method: ' + req.m);
          if (WRITES[req.m] && !WRITERS[origin_]) {
            throw refusal('keyholder', 'this origin may read but not write: which member sites may write ' +
                          'through object.* is not yet ruled', req.m);
          }
          return fn(req.a || {});
        })
        .then(function (v) { port.postMessage({ id: req.id, ok: true, v: v }); })
        /* Name the method in the failure. A bare "cannot clone" from behind an
           origin boundary is the least debuggable error there is. */
        .catch(function (err) {
          var out = { id: req.id, ok: false,
            e: (req.m || '?') + ': ' + String(err && err.message || err) };
          /* The structured refusal, when the thrower knew who said no. pacific.js
             (7b9b3f8) carries it onto the Error, and a consumer takes `by` from
             here and nowhere else (shared/refusal-cases.json). */
          if (err && err.refusal) out.refusal = err.refusal;
          /* And when it did not, a handler's own failure is still THIS FILE saying
             no, so it goes out structured and R1 decides it. Left bare, a consumer
             falls through to R4's text match, which exists for pacific.js's connect
             failures: "the generated op table did not load" matched "did not load"
             and was reported as transport (thedoor, 22 Sep). No `op`: the caller's op
             is the right one, and R1 falls back to it. An UNKNOWN method stays bare,
             so R3 still answers it in its own words. */
          else if (fn) out.refusal = { by: 'keyholder', why: String(err && err.message || err) };
          port.postMessage(out);
        });
    };

    /* Ask to survive eviction. Browsers grant this to engaged origins and
       refuse it to drive-by ones, so it is a request, not a guarantee — and the
       answer is worth telling the host, because a device that can be evicted is
       a device that will need re-pairing. */
    var persisted = navigator.storage && navigator.storage.persist
      ? navigator.storage.persist().catch(function () { return false; })
      : Promise.resolve(false);

    Promise.all([device(), persisted]).then(function (r) {
      port.postMessage({ t: 'pacific.ready', pk: hex(r[0].pk), agent: r[0].agent, persisted: !!r[1] });
    });
  });

  /* ── the same file, opened as a window ───────────────────────────────────
     Framed, this document is an origin and shows nothing. Opened as a window it
     is a door, and it has to be THIS document rather than a page of its own for
     two reasons that are really one: a WebAuthn ceremony wants a first-party
     context and a user gesture, neither of which survives a cross-origin frame
     or a MessagePort — and whatever it derives has to land in the IndexedDB the
     frame will look in, which is this origin's.

     So the popup is the keyholder, unframed. It signs in and it links, and the
     frame finds a channel waiting for it on the next load. */
  if (window.top === window) door();

  function door() {
    var esc = function (s) {
      return String(s).replace(/[&<>"]/g, function (c) {
        return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c];
      });
    };

    document.title = 'Pacific';
    var css = document.createElement('style');
    css.textContent =
      ':root{--ink:#111;--ink-2:#555;--ink-3:#8a8a8a;--hair:#e3e3e3;--bg:#fbfbfa}' +
      'html,body{margin:0;height:100%;background:var(--bg)}' +
      'body{display:flex;align-items:center;justify-content:center;' +
      'font:14px/1.55 ui-sans-serif,-apple-system,"Helvetica Neue",sans-serif;color:var(--ink)}' +
      '.d{width:280px;padding:26px 0}' +
      'h1{margin:0 0 22px;font-size:11px;letter-spacing:.34em;text-transform:uppercase;' +
      'font-weight:500;color:var(--ink-3)}' +
      'dl{margin:0 0 20px}dt{font-size:10px;letter-spacing:.2em;text-transform:uppercase;' +
      'color:var(--ink-3);padding-top:9px}' +
      'dd{margin:0;font-size:12px;word-break:break-all}dd.m{color:var(--ink-3)}' +
      'button,a.b{display:block;width:100%;box-sizing:border-box;padding:11px 14px;' +
      'border:1px solid var(--ink);background:transparent;border-radius:2px;color:var(--ink);' +
      'font:inherit;font-size:13px;text-align:center;text-decoration:none;cursor:pointer}' +
      'button:hover,a.b:hover{background:var(--ink);color:#fff}' +
      'button[disabled]{opacity:.45;cursor:default}' +
      'button.q{border-color:transparent;color:var(--ink-3);margin-top:6px}' +
      'button.q:hover{background:transparent;color:#a3251f;border-color:#a3251f}' +
      'p{margin:0 0 18px;color:var(--ink-2);font-size:13px}' +
      'p.e{color:#a3251f}' +
      'textarea{width:100%;box-sizing:border-box;font:inherit;font-size:13px;line-height:1.7;padding:9px 11px;border:1px solid var(--hair);border-radius:2px;margin:0 0 8px;resize:vertical}' +
      '.w{font:13px/1.9 ui-monospace,Menlo,monospace;background:#fff;border:1px solid var(--hair);padding:12px 13px;margin:0 0 16px;word-spacing:.35em;-webkit-user-select:all;user-select:all}';
    document.head.appendChild(css);

    var d = document.createElement('div');
    d.className = 'd';
    document.body.appendChild(d);

    var say = function (html) { d.innerHTML = '<h1>Pacific</h1>' + html; };
    var on = function (id, fn) {
      var el = document.getElementById(id);
      if (el) el.addEventListener('click', fn);
    };

    function rows(s) {
      var sites = s.sites || [];
      return '<dl>' +
        '<dt>Account</dt><dd>' + esc((s.uid || '').slice(0, 24)) + '…</dd>' +
        '<dt>Arc</dt><dd class="' + (s.arc ? '' : 'm') + '">' +
          esc(s.arc || 'not recorded') + '</dd>' +
        '<dt>Sites</dt><dd class="' + (sites.length ? '' : 'm') + '">' +
          (sites.length ? sites.map(function (x) { return esc(x.slug || x); }).join(', ')
                        : 'none held') + '</dd>' +
        '</dl>';
    }

    function draw() {
      /* Something on the screen before the round trip finishes. A door that is
         blank while it thinks is a door that looks broken. */
      say('<p>…</p>');
      METHODS['account.status']().then(function (s) {
        if (!s.signedIn) return offer();
        arrived(s);
      }, function (e) {
        say('<p class="e">' + esc(e && e.message || e) + '</p>' +
            '<button id="again">Try again</button>');
        on('again', draw);
      });
    }

    /* TWO WAYS IN, AND A THIRD BEHIND RECOVERY. The page runs all of them
       itself: it used to send people to the auth service's own /signin and wait
       for a cookie to appear; there is no cookie now, and what a person needs at
       the end of this is a SEED on this origin — which only a document on this
       origin can put here.

       THE WORDS ARE NOT A SIGN-IN METHOD, and offering them as one was a
       mistake worth naming (16 Sep 2026). Three buttons of equal weight asked a
       person to CHOOSE between a passkey and typing twenty-four words, and the
       words look like the reliable option — no ceremony, nothing to go wrong —
       so the more cautious the person, the more likely they are to pick the one
       that makes them handle their own root secret in a textarea. They are the
       recovery path: the thing you reach for when the passkey is gone, which is
       exactly when you need them and never before.

       They are still ONE CLICK AWAY and always will be. `docs/prf-portability.html`
       names the case they exist for — a platform that does not carry a passkey's
       PRF secret between devices, where the wrap simply cannot open on device two
       — and `run()` below already sends a person here by name when it sees that
       failure. Burying them would break the only door that works in that case. */
    function offer() {
      say('<p>This browser holds no account yet.</p>' +
          '<button id="new">Create an account</button>' +
          '<button id="in">Sign in with a passkey</button>' +
          '<button class="q" id="recover">Account recovery</button>');
      on('new', function () { run('new', METHODS['account.create'](), true); });
      on('in', function () { run('in', METHODS['account.signin']()); });
      on('recover', wordsForm);
    }

    /* Reached from ACCOUNT RECOVERY, and from `run()` when a passkey opens
       nothing. It says what it is for, because someone arriving here has either
       lost something or been sent by a failure, and neither person should have
       to guess whether they are in the right place. */
    function wordsForm() {
      say('<p>Account recovery.</p>' +
          '<p class="q">Your twenty-four words bring your account back on any ' +
          'device — they ARE your account, not a copy of it. Use them when your ' +
          'passkey is gone, or when it will not open your key.</p>' +
          '<textarea id="w" rows="4" spellcheck="false" autocapitalize="none" ' +
          'autocomplete="off" aria-label="Your twenty-four words"></textarea>' +
          '<button id="go">Recover my account</button>' +
          '<button class="q" id="back">Back</button>');
      document.getElementById('w').focus();
      on('go', function () {
        run('go', METHODS['account.restore']({ words: document.getElementById('w').value }));
      });
      on('back', draw);
    }

    function run(id, work, isNew) {
      var b = document.getElementById(id);
      if (b) { b.disabled = true; b.textContent = 'Waiting…'; }
      work.then(function (s) {
        if (isNew && s.words) return showWords(s);
        arrived(s);
      }, function (e) {
        var msg = String(e && e.message || e);
        /* Two causes, one symptom, and this page cannot tell them apart: another
           device enrolled since (one wrap per account, so that revoked this
           one), or this platform does not carry the passkey secret across
           devices. Say both and open the door that works either way. */
        if (/wrap open failed/.test(msg)) {
          msg = 'That passkey works, but it does not open your key — either another device ' +
                'has enrolled since, or this platform does not carry the passkey secret ' +
                'between devices. Your words will work.';
        }
        say('<p class="e">' + esc(msg) + '</p>' +
            '<button id="again">Back</button>');
        on('again', draw);
      });
    }

    /* THE ONE IRREVERSIBLE MOMENT, and the only screen in this door that a
       person must not be allowed to skip past by closing the window. */
    function showWords(s) {
      say('<p><b>Write these down.</b> They are the only way back to this account, ' +
          'they are not stored anywhere, and this is the only time they are shown.</p>' +
          '<div class="w">' + esc(s.words) + '</div>' +
          '<button id="ok">I have written them down</button>');
      on('ok', function () { arrived(s); });
    }

    function arrived(s) {
      say(rows(s) +
          (s.unreachable ? '<p class="e">The Arc could not be reached: ' +
                           esc(s.unreachable) + '</p>' : '') +
          '<button id="x">Close</button>' +
          '<button class="q" id="out">Sign out</button>');
      on('x', function () { window.close(); });
      on('out', out);
    }

    function out() {
      var b = document.getElementById('out');
      b.disabled = true;
      METHODS['session.signout']().then(draw, function (e) {
        b.disabled = false;
        say('<p class="e">' + esc(e && e.message || e) + '</p>' +
            '<button id="again">Back</button>');
        on('again', draw);
      });
    }

    draw();
  }
})();
