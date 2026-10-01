/* ═══════════════════════════════════════════════════════════════════════════
   wallflowers-fold.js — ONE fold, two clients.

   THE PROBLEM THIS EXISTS TO END. The mobile page and the desktop interior each
   knew how to turn an account into a screen, and they knew different things.
   The desktop's world was folded from ten invented ops
   (post·conv·say·read·rsvp·invite·propose·ballot·close·settle): none is in the
   ICD, none has a reducer, none carries an opId, authority or fold. The mobile
   page folds real ICD Deltas through the core's `fold_archive`. Two clients
   showing the same account different things is not a layout difference; it is
   two answers to "what does this person have", and at most one of them is true.

   So the fold lives here, once, and both clients consume it. pacific.js already
   said this was where it would land — beside its orecloud adapter: "applying an
   op locally needs the reducer, and the reducer is deliberately not in this
   file. When the fold moves to wasm this is where it comes back — ONE
   implementation, called here and in the keyholder both." It has moved. This is
   that one implementation.

   WHAT IS SHARED IS BEHAVIOUR, NOT LAYOUT. The two clients look nothing alike
   and should not: a phone is not a shadow-rooted panel over someone's website.
   What they may not differ on is what they believe — which groups exist, what
   is in them, and what could not be read. Those three come from here.

   THREE SURFACES, ONE SOURCE:

     fold(core, bytes)  -> the model. What the core folded, plus what it refused.
     toWorld(model)     -> pacific.js's world shape, for the desktop interior.
     store(opts)        -> {load, commit, subscribe}, pacific.js's store seam,
                           so the desktop takes this by injection with no edit
                           to its 169 KB.

   `commit` REFUSES. Writing needs the MLS send path and that is not built.
   A store that accepted an op and dropped it would make the interior lie the
   moment someone typed, so it throws with the reason instead.
   ═══════════════════════════════════════════════════════════════════════════ */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.WallflowersFold = factory();
}(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var dec = new TextDecoder();

  /* ── the fold ──────────────────────────────────────────────────────────────
     `core` is core-wasm's exports. `bytes` is a canonical `Archive` CBOR.

     READ memory.buffer AFTER THE CALL, never before. The module grows its
     memory to serve a fold and growing DETACHES every view taken beforehand;
     a `Uint8Array` captured on the line above throws when the call returns.
     This cost a debugging round the first time and is invisible until the
     archive is big enough to need the growth. */
  function fold(core, bytes) {
    var ptr = core.alloc(bytes.length);
    new Uint8Array(core.memory.buffer, ptr, bytes.length).set(bytes);
    var n = core.fold_archive(bytes.length);
    /* A REFUSAL RETURNS 0 and leaves its words in the out buffer, so they are
       read by out_len(), not by n. Read by n, every refusal became an Error
       with nothing in it: a member told something failed and not what. */
    if (core.erred()) {
      throw new Error(dec.decode(new Uint8Array(core.memory.buffer, core.out_ptr(), core.out_len())) ||
                      'the core refused the archive and gave no reason');
    }
    var text = dec.decode(new Uint8Array(core.memory.buffer, core.out_ptr(), n));
    return normalise(JSON.parse(text));
  }

  /* The model. One shape, whatever the client draws with it. */
  function normalise(res) {
    var groups = (res.groups || []).map(function (g) {
      var v = g.view || {};
      return {
        /* `id` is what fold_one emits. This read `group_id`, which it never
           has, so every object came out with an empty id and the desktop keyed
           its sites by NAME: two groups with one name became one site. */
        id:       g.id || '',
        kind:     g.kind || 'object',
        /* TRUE when the core read the kind off the log's type id instead of
           being told it. That is a lens, not a kind: a notebook folds as
           "group" and a connection as "forum". Show the kind as unknown, and
           never take the lens's name for it. */
        kindInferred: !!g.kind_inferred,
        name:     g.name || defaultName(g.kind),
        /* THE ROSTER, as the core folded it: who owns the object and who is in
           it. Dropping these cost a page its member count and the band its
           owner's mark (thedoor, 22 Sep). `null` and `[]` when the core said
           nothing, so an archive without them reads as unknown, not empty. */
        owner:    g.owner || null,
        members:  Array.isArray(g.members) ? g.members.slice() : [],
        digest:   g.digest || null,
        messages: (v.messages || []).map(function (m) {
          return {
            author: m.author || '',
            text:   m.text == null ? '' : String(m.text),
            ts:     m.ts || 0,
            gen:    m.gen || 0
          };
        }),
        view: v
      };
    });

    /* EVERY OBJECT THE CORE REFUSED, carried out in the coordinator's own
       words. This is the list a client may not quietly drop: an object that
       will not fold must be indistinguishable from nothing only if the member
       is told, and the whole point of returning it beside the groups is that
       there is no way to render one without having been handed the other. */
    var problems = (res.rejected || []).map(function (r) {
      return {
        what: 'An object in your archive will not fold',
        why:  (r.why || 'the reducer refused it'),
        id:   r.group || ''
      };
    });

    return {
      me:       res.display_name || null,
      identity: res.exported_by || null,
      peers:    res.peers || [],
      groups:   groups,
      problems: problems,
      version:  res.v
    };
  }

  function defaultName(kind) {
    return ({
      conversation: 'A conversation', group: 'Your own record', place: 'A place',
      event: 'An event', post: 'A post', forum: 'A forum'
    })[kind] || (kind || 'Object');
  }

  /* ── the desktop's world shape ─────────────────────────────────────────────
     pacific.js renders {site, me, sites, people, threads, events, rsvp, convs,
     links, pending, wallet, listings, posts}. Every key it reads must exist or
     it throws on a `.length` — so this returns all of them, and returns them
     EMPTY where the fold has nothing rather than inventing filler. An empty
     events list is a true statement about an account with no events; a
     fabricated one is the invented-ops problem coming back by another door. */
  function toWorld(model, opts) {
    opts = opts || {};
    var sites = {}, people = {}, threads = [], events = [];

    model.groups.forEach(function (g) {
      sites[g.id || g.name] = {
        name: g.name,
        kind: capitalise(g.kind),
        disc: 'chat'
      };

      /* A group with messages is a thread. The core decided what a message is;
         this only arranges them. */
      if (g.messages.length) {
        threads.push({
          id: g.id || g.name,
          title: g.name,
          by: shortId(g.messages[0].author),
          when: ago(g.messages[g.messages.length - 1].ts),
          n: g.messages.length,
          posts: g.messages.map(function (m) {
            return { by: shortId(m.author), at: clock(m.ts), s: m.text };
          })
        });
      }

      if (g.kind === 'event') {
        var v = g.view || {};
        events.push({
          id: g.id || g.name,
          title: v.title || g.name,
          when: v.start_ms ? new Date(v.start_ms).toDateString() : '',
          where: v.venue || '',
          by: model.me || ''
        });
      }

      g.messages.forEach(function (m) {
        var who = shortId(m.author);
        if (who && !people[who]) {
          people[who] = { n: who, home: g.id || g.name, role: 'Member', joined: '', on: 0 };
        }
      });
    });

    var me = model.me ? String(model.me).toLowerCase() : 'you';

    /* `site` MUST be a key of `sites`: pacific.js reads w.sites[w.site].name
       without guarding, so a site id that names nothing is an immediate throw.
       An account with no groups is a legitimate state, so it gets one entry
       standing for the account itself — that is not inventing content, it is
       the one thing such an account definitely has. The coupling test pins
       this, because it is exactly the invariant that broke the first time the
       desktop was handed a real fold. */
    if (!Object.keys(sites).length) {
      sites[me] = { name: model.me || 'Your account', kind: 'Account', disc: 'chat' };
    }
    var site = (opts.site && sites[opts.site]) ? opts.site : Object.keys(sites)[0];

    if (!people[me]) people[me] = { n: me, home: site, role: 'Member', joined: '', on: 1 };

    return {
      site: site,
      me: me,
      sites: sites,
      people: people,
      threads: threads,
      events: events,
      rsvp: {},
      convs: [],
      links: [],
      pending: [],
      wallet: null,
      listings: [],
      posts: [],
      /* Carried into the world so the desktop can draw what it could not read.
         Not part of pacific.js's original shape, and deliberately additive:
         a renderer that ignores it shows a shorter list, which is the failure,
         so the coupling test asserts the desktop surfaces it. */
      problems: model.problems
    };
  }

  function capitalise(s) { return String(s || '').charAt(0).toUpperCase() + String(s || '').slice(1); }
  function shortId(a) { return a ? String(a).slice(0, 8) : ''; }
  function clock(ts) {
    if (!ts) return '';
    var d = new Date(ts > 1e12 ? ts : ts * 1000);
    return ('0' + d.getHours()).slice(-2) + ':' + ('0' + d.getMinutes()).slice(-2);
  }
  function ago(ts) {
    if (!ts) return '';
    var h = Math.floor((Date.now() - new Date(ts > 1e12 ? ts : ts * 1000)) / 3600000);
    if (h < 1) return 'now';
    if (h < 24) return h + 'h';
    return Math.floor(h / 24) + 'd';
  }

  /* ── the desktop's store seam ──────────────────────────────────────────────
     pacific.js takes `opts.store` and never asks where it came from, so the
     desktop is coupled to this fold by INJECTION — no edit to its 169 KB, and
     no second copy of the mapping. */
  function store(opts) {
    var world = null;
    return {
      load: function () {
        return Promise.resolve(opts.archive()).then(function (bytes) {
          world = toWorld(fold(opts.core, bytes), opts);
          return world;
        });
      },
      /* REFUSES, on purpose. Writing a delta needs the MLS send path, which is
         not built. Accepting an op and dropping it would put a message on
         screen that no other device will ever see — the exact shape of lie
         this whole layer exists to refuse. */
      commit: function () {
        return Promise.reject(new Error(
          'This interior is read-only: writing a delta needs the MLS send path, ' +
          'which is not built. Nothing was posted.'));
      },
      subscribe: function () { return function () {}; }
    };
  }

  return { fold: fold, toWorld: toWorld, store: store, normalise: normalise };
}));
