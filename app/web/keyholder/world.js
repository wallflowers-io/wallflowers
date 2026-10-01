/* ═══════════════════════════════════════════════════════════════════════════
   THE WORLD, FROM AN ARCHIVE — the core's fold, as the interior draws it.

   `archive.rs` hands a new device its history as CBOR; `core-wasm`'s
   `fold_archive` runs THE fold over it — the same `fold_group` the phone runs —
   and emits one JSON document: per group, its kind, its roster and the folded
   view through the lens that kind folds through. That document is the core's
   read model. It is not what `pacific.js` draws.

   What `pacific.js` draws is the world `docs/seeds/*.js` shaped: a site, people
   keyed by handle, threads, conversations, events. This file is the mapping
   between the two, and it is the WHOLE of it — nothing here folds, nothing here
   decides what a message means, nothing here reads a delta. It takes the fold's
   answer and puts it in the interior's vocabulary.

   PURE, ON PURPOSE. No DOM, no globals of its own, no IndexedDB, no clock it
   was not handed: the same file runs in the keyholder frame and under node, and
   `world.test.cjs` is what holds it to the shape. The shim at the bottom exports
   it both ways.

   TWO RULES THE MAPPING KEEPS.

   It never invents content. A forum with no posts is a thread with no posts. A
   person the archive names but says nothing more about gets the blanks the seed
   itself uses for "not known" — `joined: '—'`, `on: 0`, `presence:
   'undeclared'` — rather than a plausible value. Empty arrays are fine; a
   fabricated date is not.

   It fails loud on a fold it does not understand. The JSON shape is a contract
   between two tracks built in parallel; a document at the wrong version, a key
   that is not 64 hex, a view without the messages its kind promises — each of
   those throws with the field named, because a world quietly drawn from a
   misread fold is exactly the failure that would go unnoticed.
   ═══════════════════════════════════════════════════════════════════════════ */
(function (root, factory) {
  'use strict';
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.PacificWorld = factory();
}(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var HEX64 = /^[0-9a-f]{64}$/;
  var isHex64 = function (s) { return typeof s === 'string' && HEX64.test(s); };
  function need(cond, why) { if (!cond) throw new Error('worldFromFold: ' + why); }

  /* ── time, in the seed's own words ───────────────────────────────────────
     The seeds carry dates as the short strings a member reads — `at: '09:12'`,
     `when: '40m'`, `d: '19', m: 'Sep'`, `start: '2026-09-19T19:30'` — and
     `pacific.js` stamps what it authors the same way (its `today()`), so a
     fold's millisecond timestamps are rendered into that vocabulary here and
     nowhere else. Local time, because that is what the interior uses for its
     own stamps and a thread should not show two clocks. A `ts` of 0 is the
     core's "absent" and renders as '' — not as 1970. */
  var MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  var two = function (n) { return ('0' + n).slice(-2); };
  function clock(ms) {
    var d = new Date(ms);
    return two(d.getHours()) + ':' + two(d.getMinutes());
  }
  function iso(ms) {
    var d = new Date(ms);
    return d.getFullYear() + '-' + two(d.getMonth() + 1) + '-' + two(d.getDate()) + 'T' + clock(ms);
  }
  function ago(ms, now) {
    var s = Math.max(0, now - ms) / 1000;
    if (s < 60) return 'now';
    if (s < 3600) return Math.floor(s / 60) + 'm';
    if (s < 86400) return Math.floor(s / 3600) + 'h';
    if (s < 7 * 86400) return Math.floor(s / 86400) + 'd';
    return Math.floor(s / (7 * 86400)) + 'w';
  }

  /* ── handles ─────────────────────────────────────────────────────────────
     The interior keys people by a short handle — `axel`, `szonja` — and uses
     it everywhere a person appears: as the `by` on a post, the `with` on a
     conversation, the key into `people`. The archive keys people by their
     32-byte identity. This is the one function that turns the second into the
     first, so every appearance of a key agrees.

     A name becomes a handle the way a name becomes a slug: lower-cased, spaces
     to hyphens, anything that is not a letter, digit or one of `._-` dropped.
     The person's REAL name is kept on `people[h].n`; the handle is the key.

     Two people with the same name must not share a handle, or their posts
     merge. So a name that is not unique among the peers is not used as a
     handle at all — both fall back to the first eight hex characters of their
     key, which is unique by construction and is what a person with no name at
     all gets anyway. */
  /* The seed stamps a note `12 Aug · 09:12` (pacific.js `keepNote`), and a
     ledger row `03 Sep`. Both are built from the same MON table above, so a
     fold's milliseconds are rendered into the interior's words here and nowhere
     else. A `0` is the core's "absent" and renders as '', not as 1970. */
  function dayMon(ms) {
    if (!ms) return '';
    var d = new Date(ms);
    return two(d.getDate()) + ' ' + MON[d.getMonth()];
  }
  function noteStamp(ms) {
    if (!ms) return '';
    return dayMon(ms) + ' · ' + clock(ms);
  }

  /* WHAT A CURRENCY IS WORTH IN MINOR UNITS, AND WHAT IT LOOKS LIKE — asked of
     the currency itself rather than of a table here.

     Won, yen and friends have no minor unit: 5000 KRW is ₩5,000, not ₩50.
     `pacific.js`'s `money()` divided by 100 unconditionally, which is the same
     defect `wallet.rs` spends a paragraph refusing ("amounts are integer minor
     units, never floats") — so the mapping carries the divisor with the
     currency instead of letting the renderer assume one.

     Intl holds both facts and is in every browser this runs in; a hand-kept list
     of zero-decimal codes here would be a second, staler copy. The locale is
     fixed so two devices render one string, and an unknown code falls back to
     its own letters and to 100, which is the majority case and the one the
     renderer already assumed. */
  function currencyFacts(ccy) {
    var code = String(ccy || '').toUpperCase();
    var out = { ccy: code, sym: code, minor: 100 };
    if (!code) return out;
    try {
      var f = new Intl.NumberFormat('en', { style: 'currency', currency: code });
      var digits = f.resolvedOptions().maximumFractionDigits;
      if (typeof digits === 'number') out.minor = Math.pow(10, digits);
      f.formatToParts(0).forEach(function (part) {
        if (part.type === 'currency') out.sym = part.value;
      });
    } catch (e) { /* not a currency Intl knows: its own letters will do */ }
    return out;
  }

  function slug(name) {
    return String(name == null ? '' : name)
      .trim().toLowerCase()
      .replace(/\s+/g, '-')
      .replace(/[^\p{L}\p{N}._-]/gu, '');
  }
  function handleFor(hex, peers) {
    need(isHex64(hex), 'a key must be 64 hex characters, got ' + JSON.stringify(hex));
    var byId = {}, count = {};
    (peers || []).forEach(function (p) {
      if (!p || !isHex64(p.id) || byId[p.id] !== undefined) return;
      var h = slug(p.name);
      if (!h) return;
      byId[p.id] = h;
      count[h] = (count[h] || 0) + 1;
    });
    var h = byId[hex];
    return (h && count[h] === 1) ? h : hex.slice(0, 8);
  }

  /* The lens each kind folds through, as `archive::lens_for` decides it. Only
     the kinds the interior has a surface for are mapped; the fold marks the rest
     `{unsupported: true}` and they are counted, not drawn. */
  var THREAD_KINDS = { forum: 1 };
  var CONV_KINDS = { connection: 1, conversation: 1 };

  /* `sites[].kind` is the interior's word for what a site IS — signup's
     vocabulary, and the second column of the Network table. GroupShape has
     exactly three values, so this table is total; an unknown shape is a fold
     this file does not understand and is refused rather than defaulted. */
  var SITE_KIND = { individual: 'Individual', team: 'Team', organisation: 'Community' };

  function messagesOf(g) {
    need(g.view && Array.isArray(g.view.messages),
         g.kind + ' ' + g.id.slice(0, 16) + '… has no messages array in its view');
    g.view.messages.forEach(function (m, i) {
      need(m && isHex64(m.author), 'message ' + i + ' of ' + g.id.slice(0, 16) + '… has no 64-hex author');
      need(typeof m.text === 'string', 'message ' + i + ' of ' + g.id.slice(0, 16) + '… has no text');
    });
    return g.view.messages;
  }
  var lastOf = function (msgs) {
    return msgs.reduce(function (t, m) { return Math.max(t, m.ts || 0); }, 0);
  };
  /* Newest first, and stable — two threads with no timestamps keep the archive's
     own order rather than being shuffled by the sort. */
  function byRecency(list) {
    return list.map(function (x, i) { return { x: x, i: i }; })
      .sort(function (a, b) { return (b.x._last - a.x._last) || (a.i - b.i); })
      .map(function (w) { delete w.x._last; return w.x; });
  }

  /* THE FOLD VERSION THIS MAPPING READS — `pacific_core::archive::ARCHIVE_VERSION`.
     ONE PLACE. keyholder.js imports it rather than repeating the number, and
     world.test.cjs asserts the REAL core emits it, so the constant cannot drift
     from the Rust without a test naming it.

     It read 1 until 15 Sep 2026, by which time the core had been emitting 2 for
     two days — bumped when `LogEntry::sig` arrived — so every archive handover
     threw 'fold v2 — this mapping reads v1' and the only real fold→interior path
     in the browser was silently shut. Two tests were red in the tree the whole
     time.

     NOT "accept 1 or 2". archive.rs is explicit that the bump is "a SIGNAL rather
     than a barrier: a v2 reader knows authorship proofs are possible and can
     insist on them where the source is untrusted", and that nothing in the wild
     writes v1 because no surface has ever stored a history. Accepting v1 here
     would take an archive whose entries cannot carry `sig` and read it as though
     they could. */
  var FOLD_V = 2;

  /* `worldFromFold(fold, me, opts)`
       fold  the JSON `fold_archive` emits
       me    the 64-hex identity whose history this is — decides which side of
             a conversation is "me". Ordinarily `fold.exported_by`.
       opts  { now } — the clock, for the tests; `Date.now()` otherwise. */
  /* ═══ THE TWO FACETS THE SITE'S OWN GROUP CARRIES ═══════════════════════════

     `group.rs` folds a notebook, a wallet and a publication onto every
     Group-typed object, and `core-wasm`'s `group_view` emits all three. Until
     those keys existed this file returned `wallet: null` unconditionally and had
     never heard of a note, so the core folded a treasury and a knowledge base
     and the interior threw both away.

     A fold that carries neither behaves exactly as it did — the same promise
     `site_group` makes above — because the hand-written folds in the test
     suites, and any wasm built before the emitter, legitimately predate them.
     What is NOT tolerated is a key that is present and the wrong shape: that is
     a contract between two tracks, and it fails loud with the field named. */

  /* THE SITE'S KNOWLEDGE BASE.

     One row per note, already reduced to a winner by `NoteBook::current` in
     Rust: which of two co-authored versions IS the document is the fold's
     decision, and deciding it a second time here is exactly what this file
     exists not to do. `versions` says a note is co-edited without carrying every
     draft, and a note whose every entry is retracted never arrives at all.

     The interior's own note row reads `n.s` and `n.at` (`drawNotes`), so those
     two are in its vocabulary; the rest is carried under the fold's own names
     for a surface that wants more than the first line. */
  function notesFromFold(view, handle) {
    var nb = view && view.notebook;
    if (!nb) return [];
    need(Array.isArray(nb.notes), 'notebook.notes must be an array');
    var comments = {}, reacted = {};
    (nb.comments || []).forEach(function (c) {
      comments[c.note] = (comments[c.note] || 0) + 1;
    });
    /* An EMPTY emoji is the cleared state, not a reaction — `note.rs` keeps the
       row so the clearing is itself a fact, and counting it would inflate the
       number a member sees. */
    (nb.reactions || []).forEach(function (r) {
      if (r.emoji) reacted[r.note] = (reacted[r.note] || 0) + 1;
    });
    return nb.notes.map(function (n) {
      need(typeof n.note === 'string' && n.note, 'a note needs an id');
      return {
        id: n.note,
        t: n.title || '',
        s: n.text || '',
        by: handle(n.by),
        at: noteStamp(n.at),
        versions: n.versions,
        comments: comments[n.note] || 0,
        react: reacted[n.note] || 0,
        retracted: false
      };
    });
  }

  /* THE SITE'S WALLET, in the interior's vocabulary (`docs/seeds/stoma-flat.js`
     WALLET is the shape).

     `null` is "no treasury opened", which hides the Money tab, and it is a
     different statement from a treasury holding nothing — `group.rs` says so on
     the field itself and `pacific.js` reads it that way.

     TWO THINGS THE FOLD DOES NOT CARRY, and neither is filled in here:

     - A BAND'S LABEL. The seed's are editorial ("two-thirds", "any Admin,
       logged"); the log holds a rule and a quorum, so the band wears its rule's
       own word and nothing is invented to sit beside it.
     - A PROPOSAL'S AMOUNT AND PAYEE. RATIFY carries an opaque `payload` whose
       format is not pinned, so how much a decision authorised is genuinely not
       in the log. The key is therefore ABSENT rather than zero: `band()` reads
       an absent amount into the strictest band, which is the safe direction, and
       the divergence check for overpayment cannot fire on a comparison against
       nothing. `pacific.js` renders the missing amount as unknown rather than as
       a number it does not have. */
  var BALLOT_CODE = { approve: 1, reject: 2, abstain: 3 };

  function walletFromFold(view, handle) {
    var w = view && view.wallet;
    if (!w) return null;
    need(typeof w.currency === 'string', 'wallet.currency must be a string');
    need(Array.isArray(w.bands) && w.bands.length, 'wallet.bands must be a non-empty array');
    var facts = currencyFacts(w.currency);
    var row = function (r, id) {
      return { id: id, ref: id, amount: r.amount, at: dayMon(r.at), by: handle(r.by) };
    };
    /* The last count any member attested, which is `Wallet::latest_count`'s own
       rule — a maximum by `at`, not a second opinion about what a balance is. */
    var counts = (w.counts || []).slice().sort(function (a, b) { return a.at - b.at; });
    var last = counts.length ? counts[counts.length - 1] : null;

    return {
      ccy: facts.ccy,
      sym: facts.sym,
      /* What one major unit is worth in the minor units the log stores. */
      minor: facts.minor,
      account: w.account || '',
      disclosure: w.disclosure || 'full',
      cooloff: w.cooloff_hours,
      balance: w.balance,
      bands: w.bands.map(function (b) {
        return { upto: b.ceiling === undefined ? null : b.ceiling, rule: b.rule,
                 quorum: b.quorum, label: b.rule };
      }),
      deposits: (w.deposits || []).map(function (d) {
        var out = row(d, d.reference);
        out.source = d.source || '';
        return out;
      }),
      settlements: (w.settlements || []).map(function (s) {
        var out = row(s, s.reference);
        out.proposal = s.proposal === undefined ? null : s.proposal;
        out.what = s.memo || '';
        return out;
      }),
      proposals: (view.decisions || []).map(function (d) {
        var ballots = {};
        (d.ballots || []).forEach(function (b) {
          var code = BALLOT_CODE[b.ballot];
          if (code) ballots[handle(b.by)] = code;
        });
        return {
          id: d.id,
          by: handle(d.proposer),
          what: d.payload || '',
          /* RATIFY stamps a Lamport `gen`, not a wall clock, so there is no date
             to render — the seed's own blank for a time it does not know. */
          at: '',
          closed: !!d.closed,
          outcome: d.outcome,
          ballots: ballots
        };
      }),
      attested: last ? { amount: last.amount, at: dayMon(last.at), by: handle(last.by) } : null
    };
  }

  function worldFromFold(fold, me, opts) {
    opts = opts || {};
    var now = typeof opts.now === 'number' ? opts.now : Date.now();

    need(fold && typeof fold === 'object', 'no fold');
    need(fold.v === FOLD_V,
         'fold v' + fold.v + ' — this mapping reads v' + FOLD_V);
    need(isHex64(fold.exported_by), 'exported_by must be 64 hex');
    need(Array.isArray(fold.groups), 'groups must be an array');
    need(Array.isArray(fold.peers), 'peers must be an array');
    need(isHex64(me), 'me must be 64 hex');
    fold.peers.forEach(function (p, i) {
      need(p && isHex64(p.id), 'peer ' + i + ' has no 64-hex id');
    });
    fold.groups.forEach(function (g, i) {
      need(g && typeof g.id === 'string' && /^[0-9a-f]+$/.test(g.id), 'group ' + i + ' has no hex id');
      need(typeof g.kind === 'string', 'group ' + g.id.slice(0, 16) + '… has no kind');
      need(isHex64(g.owner), 'group ' + g.id.slice(0, 16) + '… has no 64-hex owner');
      need(Array.isArray(g.members), 'group ' + g.id.slice(0, 16) + '… has no members');
      need(g.view && typeof g.view === 'object', 'group ' + g.id.slice(0, 16) + '… has no view');
    });

    var individuals = fold.groups.filter(function (g) {
      return g.kind === 'group' && g.view.shape === 'individual';
    });

    /* WHO HAS A NAME. Three places the archive puts a name against a key, in
       order of authority: a person's own Group says what they call themselves
       (the profile the phone shows), the exporter's own `display_name` says
       what they call themselves, and the peers list says what THIS device had
       recorded for them — the pairing-bundle name, which `contact.rs` treats
       as the fallback when there is no profile. First entry per key wins in
       `handleFor`, so the order here is the precedence. */
    /* WHOSE RECORD an individual Group is comes from an EDGE the archive
       carries — `peers[].identity_group`, and `own_group` for the exporter —
       never from `owner`. Owner is who minted the record, and for a contact's
       record that is the exporter, which is exactly the wrong answer. A record
       no edge claims has no subject here and names nobody. */
    var subjectOf = {};
    (fold.peers || []).forEach(function (p) {
      if (p.identity_group) subjectOf[p.identity_group] = p.id;
    });
    if (fold.own_group) subjectOf[fold.own_group] = fold.exported_by;

    var named = [];
    individuals.forEach(function (g) {
      var who = subjectOf[g.id];
      if (who && g.view.display_name) named.push({ id: who, name: g.view.display_name });
    });
    if (fold.display_name) named.push({ id: fold.exported_by, name: fold.display_name });
    fold.peers.forEach(function (p) { named.push({ id: p.id, name: p.name }); });
    var H = function (key) { return handleFor(key, named); };

    /* THE SITE. The interior draws one site and everyone's `home` points at
       it. There are two ways an archive can say which one, and only two.

       IT CAN DECLARE IT. `site_group` names the Group this archive is the
       history OF — a community's archive, whose site is an ORGANISATION or a
       team and which no individual Group can stand in for. That is an EDGE,
       exactly like `own_group` and `peers[].identity_group`, and it is read the
       same way: the fold says which group, this file does not guess. A
       `site_group` naming a group the fold does not carry, or naming something
       that is not a Group, is a misread document rather than a missing field,
       so it throws with the id in the message.

       OR IT CAN SAY NOTHING, and the site is the exporter's own Group (a person
       IS a Group, shape individual) when the archive carries one, and a
       synthetic 'home' when it does not — synthetic in the sense of having no
       group id, not in the sense of invented content: its name is the archive's
       own `display_name`. That is the personal archive, unchanged.

       A declared site is keyed by its name the way a person is: the same slug,
       so `sites` and `people` are keyed out of one vocabulary and a site with
       no name falls back to its id the way a person with no name falls back to
       their key. */
    var declared = null;
    if (fold.site_group) {
      declared = fold.groups.filter(function (g) { return g.id === fold.site_group; })[0] || null;
      need(declared, 'site_group ' + String(fold.site_group).slice(0, 16) + '… names no group in this fold');
      need(declared.kind === 'group',
           'site_group ' + declared.id.slice(0, 16) + '… is a ' + declared.kind + ', and a site is a group');
      need(SITE_KIND[declared.view.shape],
           'site_group ' + declared.id.slice(0, 16) + '… has shape ' +
           JSON.stringify(declared.view.shape) + ', which is not a GroupShape');
    }
    var own = individuals.filter(function (g) { return fold.own_group && g.id === fold.own_group; })[0] || null;
    var shape = declared ? String(declared.view.shape) : 'individual';
    var site = declared ? (slug(declared.view.display_name || declared.name) || declared.id.slice(0, 8))
             : own ? H(fold.exported_by)
             : 'home';
    var sites = {};
    sites[site] = {
      name: declared ? (declared.view.display_name || declared.name || site)
          : (own && own.view.display_name) || fold.display_name || (own ? site : 'Home'),
      kind: SITE_KIND[shape], disc: 'chat', shape: shape
    };
    /* The roles the SITE's own group declares for its members — the one roster
       whose roles are about THIS site rather than about someone else's. Which
       group that is is the question above: the declared one when there is one,
       and the exporter's own otherwise. */
    var siteGroup = declared || own;
    var roleOf = {};
    if (siteGroup && Array.isArray(siteGroup.view.roles)) {
      siteGroup.view.roles.forEach(function (r) {
        need(Array.isArray(r) && r.length === 2 && isHex64(r[0]), 'a role entry must be [key, role]');
        roleOf[r[0]] = String(r[1]);
      });
    }

    /* THE PEOPLE. Everyone the archive mentions is in the roster; what it does
       not say about them is left as the seed's own blanks. `person` is the one
       way in, so a key seen on a post, a conversation and the peers list is one
       entry, not three. */
    var people = {};
    function person(key) {
      var h = H(key);
      if (!people[h]) {
        people[h] = { n: h, home: site, role: roleOf[key] || '', joined: '—', on: 0, presence: 'undeclared' };
      }
      return h;
    }
    individuals.forEach(function (g) {
      var who = subjectOf[g.id];
      if (!who) return;                 /* no edge claims it: it is nobody's profile here */
      var h = person(who);
      if (g.view.display_name) people[h].n = g.view.display_name;
      if (g.view.presence) people[h].presence = String(g.view.presence);
    });
    var meHandle = person(me);
    if (me === fold.exported_by && fold.display_name && people[meHandle].n === meHandle) {
      people[meHandle].n = fold.display_name;
    }
    fold.peers.forEach(function (p) {
      var h = person(p.id);
      if (p.name && people[h].n === h) people[h].n = p.name;
    });

    /* THREADS — one per forum. `by` is who opened it: the first post's author,
       or the group's owner for a forum nobody has posted in yet. `channel` is the
       Forum hub's key in the seed; the fold does not carry it, so it is blank
       rather than guessed. A forum with no name is called by its id, the way a
       person with no name is called by their key. */
    var threads = fold.groups.filter(function (g) { return THREAD_KINDS[g.kind]; }).map(function (g) {
      var msgs = messagesOf(g), last = lastOf(msgs);
      return {
        id: g.id,
        channel: '',
        title: g.name || g.id.slice(0, 8),
        by: person(msgs.length ? msgs[0].author : g.owner),
        when: last ? ago(last, now) : '',
        n: msgs.length,
        posts: msgs.map(function (m) {
          return { by: person(m.author), at: m.ts ? clock(m.ts) : '', s: m.text };
        }),
        _last: last
      };
    });

    /* CONVERSATIONS — one per connection or conversation. `with` is the other
       member; a conversation with more than two members has no place to put
       the rest in this shape, so the first other member names it and the
       roster is still the group's. `unread` is per-device state the archive
       does not carry — it is a fresh device, and nothing here is unread on it
       any more than everything is. */
    var convs = fold.groups.filter(function (g) { return CONV_KINDS[g.kind]; }).map(function (g) {
      var msgs = messagesOf(g);
      g.members.forEach(function (k, i) {
        need(isHex64(k), 'member ' + i + ' of ' + g.id.slice(0, 16) + '… is not 64 hex');
      });
      var others = g.members.filter(function (k) { return k !== me; });
      var other = others.length ? others[0] : (g.owner !== me ? g.owner : null);
      return {
        id: g.id,
        with: other ? person(other) : '',
        unread: 0,
        msgs: msgs.map(function (m) { return { me: m.author === me ? 1 : 0, s: m.text }; }),
        _last: lastOf(msgs)
      };
    });

    /* EVENTS. `host` must be a key into `sites` — the interior reads
       `sites[e.host].name` — and the archive gives one site, so every event is
       hosted here. `going`/`more` are the prototype's attendance, which the
       model does not keep on the event (the seed says so at length); empty is
       the truth. `start_ms` of 0 is "not yet real" in `event.rs`, so it renders
       as no date rather than as the epoch. */
    var events = fold.groups.filter(function (g) { return g.kind === 'event'; }).map(function (g) {
      var v = g.view;
      need(typeof v.title === 'string' && typeof v.venue === 'string',
           'event ' + g.id.slice(0, 16) + '… view lacks title/venue');
      var s = typeof v.start_ms === 'number' ? v.start_ms : 0;
      var d = s ? new Date(s) : null;
      return {
        id: g.id,
        d: d ? two(d.getDate()) : '',
        m: d ? MON[d.getMonth()] : '',
        t: v.title || g.name || g.id.slice(0, 8),
        at: [d ? clock(s) : '', v.venue].filter(Boolean).join(' · '),
        host: site,
        going: [],
        more: 0,
        start: d ? iso(s) : null,
        end: (typeof v.end_ms === 'number' && v.end_ms) ? iso(v.end_ms) : null,
        venue: v.venue,
        _start: s
      };
    }).sort(function (a, b) { return (a._start || Infinity) - (b._start || Infinity); });
    events.forEach(function (e) { delete e._start; });

    /* The keys `pacific.js` reads and the archive has nothing for. Present and
       empty, because the interior indexes into `rsvp` and maps over `pending`
       and `links`.

       `notes` and `wallet` are no longer among them: they come off the SITE's
       own group, which is the object whose notebook and treasury they are. A
       site with no declared group and no own group has neither, and `wallet:
       null` still means "no treasury opened" — which hides the Money tab, and
       is a different statement from an empty one, as the seed explains. */
    var siteView = siteGroup ? siteGroup.view : null;
    return {
      site: site,
      me: meHandle,
      sites: sites,
      people: people,
      threads: byRecency(threads),
      events: events,
      rsvp: {},
      convs: byRecency(convs),
      links: [],
      pending: [],
      notes: notesFromFold(siteView, person),
      wallet: walletFromFold(siteView, person),
      listings: [],
      posts: []
    };
  }

  return { worldFromFold: worldFromFold, handleFor: handleFor, FOLD_V: FOLD_V };
}));
