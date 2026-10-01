/* ═══════════════════════════════════════════════════════════════════════════
   webapp.js — the interior, and the way to it.

   THE BROWSER HOLDS A SESSION AND NOTHING ELSE. Ruled 23 September 2026: the
   door (app/door) holds the seed and runs the native `Node`; this page holds an
   HttpOnly cookie it cannot read. The words cross once, to the door.

   WHAT THIS FILE DOES NOT DO.  It performs no cryptography and models no state.
   The door folds every object through `Node::object_view`; this file asks and
   draws. Where it cannot get an answer it draws that too — see `problems`.
   ═══════════════════════════════════════════════════════════════════════════ */
(function () {
  'use strict';

  var $ = function (id) { return document.getElementById(id); };
  var doc = document.documentElement;
  /* Each step a person waits on, timed in this tab (the Door's perf.js); a stub where it did not load. */
  var T = window.WallFlowersPerf || { begin: function () { return null; }, end: function () {}, shown: function () {} };

  /* ── the way in ────────────────────────────────────────────────────────────
     A SIGN-IN PORTAL, and nothing else signed out (Ralph, 28 Sep: "its only purpose
     is as a sign in portal ... take the user straight to the signin page"). Signed in
     it is the interior; signed out, the Door's window (DR-4), on the Door's origin, where
     the passkey and its PRF never touch this page; it comes back to this address, hash
     and all. A kiosk's visitor (#join) and www's draft (#register=) are
     new people: the window opens on the account it would make. */
  function toWindow(fresh) {
    doc.removeAttribute('data-state');    // nothing drawn while the window loads
    var here = DOOR === location.origin ? location.pathname + location.search + location.hash : '/';
    location.assign(DOOR + '/signin?' + (fresh ? 'new&' : '') + 'return=' + encodeURIComponent(here));
  }
  function signIn() { toWindow(/^#(join$|register=)/.test(location.hash || '')); }

  /* ── the door ──────────────────────────────────────────────────────────── */
  /* The door: door-origin.js (SEC-9). */
  var DOOR = doorOrigin(location);

  function door(path, opts) {
    opts = opts || {};
    var t0 = T.begin(), step = (opts.method || 'GET') + ' ' + path;
    return fetch(DOOR + path, {
      method: opts.method || 'GET',
      credentials: 'include',
      headers: opts.body ? { 'content-type': 'application/json' } : {},
      body: opts.body ? JSON.stringify(opts.body) : undefined
    }).then(function (r) {
      return r.text().then(function (t) {
        T.end(step, t0, { status: r.status, bytes: t.length });
        if (r.status === 401) ended();
        if (!r.ok) { var e = new Error(t || r.status); e.status = r.status; throw e; }
        return t ? JSON.parse(t) : null;
      });
    });
  }

  /* A SESSION THE DOOR HAS ENDED (idle, or signed out elsewhere) answers 401 (NC-91): the
     page goes to the window, as signing out does, and never sits on a view it can no
     longer write to. */
  function ended() {
    if (doc.getAttribute('data-state') !== 'inside') return;
    if (events) { events.close(); events = null; }
    M = null; S = { site: null, tab: 'feed', view: 'feed', sel: null };
    toWindow(false);
  }

  /* SEVERAL WRITES, ONE REQUEST (/v2/batch): the steps in order, a later one naming an
     earlier mint as {$step: i}. Answers what was made, one result a step, and the step
     refused if one was: a refusal part-way still says what stands. */
  function batch(steps) {
    var t0 = T.begin();
    return fetch(DOOR + '/v2/batch', {
      method: 'POST', credentials: 'include',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ steps: steps })
    }).then(function (r) {
      return r.text().then(function (t) {
        T.end('POST /v2/batch', t0, { status: r.status, bytes: t.length, steps: steps.length });
        if (r.status === 401) ended();
        var o = null;
        try { o = JSON.parse(t); } catch (e) { o = null; }
        if (!o || !Array.isArray(o.made)) throw new Error(t || r.status);
        return o;
      });
    });
  }

  function busy(el, on) {
    el.disabled = on;
    el.setAttribute('aria-busy', on ? 'true' : 'false');
  }

  /* ── the interior ────────────────────────────────────────────────────────
     THE MODEL IS /v2/graph, arranged, never extended. The door folds every
     object through `Node::object_view`; this only decides where each one is
     drawn:
       the self record   spine index 0 — you, not a site
       a part            an object another is made of (`view.parts`): a
                         site's channel, a post's comments, a Host
       a site            a group that is neither
       a connection      the door's `role: channel`; its chat is `role: chat`
     An object that will not fold is drawn at the top of the feed in the core's
     words. Nothing here is stored: a reload asks the door again. */
  var M = null;
  var S = { site: null, tab: 'feed', view: 'feed', sel: null };

  function strip(pk) { return String(pk || '').replace(/^ed25519:/, ''); }

  function model(g) {
    var m = { me: { pk: strip(g.me && g.me.pk), name: (g.me && g.me.display_name) || '' },
              byId: {}, host: {}, origin: {}, names: {}, sites: [], feed: [], conns: [], chats: {}, problems: [],
              self: null };
    var spine = (g.spine || []).slice().sort(function (a, b) { return a.index - b.index; });
    m.self = spine.length ? spine[0].object : null;
    (g.objects || []).forEach(function (o) { m.byId[o.id] = o; });
    (g.objects || []).forEach(function (o) {
      ((o.view && o.view.parts) || []).forEach(function (p) { m.host[p.part] = o.id; });
      if (o.kind === 'group') ((o.view && o.view.affiliations) || []).forEach(function (a) {
        if (a.rel === 'created') m.origin[a.peer] = o.id;
      });
    });
    m.names[m.me.pk] = m.me.name;
    (g.objects || []).forEach(function (o) {
      if (!o.folds) { m.problems.push(o); return; }
      if (o.role === 'channel') {
        m.names[o.peer] = o.name;
        m.conns.push({ pk: o.peer, name: o.name, id: o.id });
        return;
      }
      if (o.role === 'chat') { m.chats[o.peer] = o.id; return; }
      if (o.id === m.self) return;
      var h = m.host[o.id];
      if (o.kind === 'group' && !h) { m.sites.push(o); return; }
      if (o.kind === 'forum' && h && m.byId[h] && m.byId[h].kind !== 'group') return;   /* comments */
      m.feed.push(o);
    });
    return m;
  }

  /* `#site=<object id>` — where the embed's anchor points. Held until the door
     has answered, then dropped from the URL. */
  var WANT = (/site=([0-9a-f]{64})/.exec(location.hash) || [])[1] || null;
  /* #you opens Your card, where a Site's own page sends a member to edit theirs (Ralph,
     29 Sep: egregores-echoes.com's profile icon). Signed out, the window returns here. A
     claim's visitor (#join) is given their first card by entry(), once the join answers. */
  var YOU = location.hash === '#you', JOINS = location.hash === '#join';

  /* THE MODEL ITSELF, from the door: which ops a link writes, what they take,
     and who may write them. Read, never restated. */
  var ICD = null;
  /* By its hash where the Door's page names it: cached for good. A site's page names none. */
  var ICD_AT = (function (m) {
    var h = m && m.getAttribute('content');
    return /^[0-9a-f]{64}$/.test(h || '') ? '/v2/icd/' + h : '/v2/icd';
  })($('wallflowers-icd'));
  function icd() {
    return ICD ? Promise.resolve(ICD) : door(ICD_AT).then(function (d) { ICD = d; return d; });
  }
  function opDecl(name) {
    var found = null;
    Object.keys(ICD.kinds).forEach(function (k) { var o = (ICD.kinds[k].ops || {})[name]; if (o) found = o; });
    Object.keys(ICD.facets).forEach(function (f) { var o = (ICD.facets[f].ops || {})[name]; if (o) found = o; });
    return found;
  }
  function op0(kind) {
    var ops = (ICD.kinds[kind] || {}).ops || {}, found = null;
    Object.keys(ops).forEach(function (n) { if (ops[n].op === 0) found = ops[n]; });
    return found;
  }

  var landed = false;
  function load() {
    var t0 = T.begin();
    return Promise.all([door('/v2/graph'), icd().catch(function () { return null; }), door('/v2/me').catch(function () { return null; })]).then(function (both) {
      var g = both[0], t = T.begin();
      M = model(g);
      siteItems(true);
      if (both[2] && typeof both[2].display_name === 'string' && both[2].display_name) { M.me.name = both[2].display_name; M.names[M.me.pk] = M.me.name; }
      CARD_NOTE = (both[2] && both[2].card_note) || null;
      T.end('model', t, { objects: (g.objects || []).length });
      /* RX.3's report: what this session holds, or its spine names, that will not fold. */
      ((both[2] && both[2].noncompliant) || []).forEach(function (n) {
        if (!M.problems.some(function (p) { return p.id === n.object; })) M.problems.push({ id: n.object, kind: n.kind, why: n.why });
      });
      if (WANT && M.byId[WANT]) S.site = WANT;
      WANT = null;
      if (JOINING && M.byId[JOINING]) { S.site = JOINING; JOINING = null; T.shown('join', joinT); joinT = null; }
      if (REFUSED) M.problems.push({ id: '', why: REFUSED });
      NOTED.forEach(function (n) { M.problems.push(n); });
      render();
      siteItems(false);
      T.shown('graph', t0);
      if (!landed) {
        landed = true; T.shown('landing', 0);
        if (!M.me.name && !JOINS && $('sheet').hidden) yourCard(true);
        else if (YOU) yourCard(false);
        if (YOU) { YOU = false; history.replaceState(null, '', location.pathname + location.search); }
      }
    },
      function (e) { M = M || model({}); M.problems = [{ id: '', why: String(e && e.message || e) }]; render(); });
  }

  /* W-98, THE ROSTER GAP (MANAGE's (b)): a Site's events its member does not hold, from the Site's
     public items (GET /v2/site/:site/items), made read-only events by the events page and drawn by
     its hook. Kept a Site at a time, so a reload draws them at once and asks again after. */
  var ITEMS = {};
  function siteItems(cached) {
    var E = eventsModule('EventsPage');
    if (!E || !E.fromItems || !M) return;
    M.sites.forEach(function (s) {
      if (cached) {
        if (ITEMS[s.id]) E.fromItems(s, ITEMS[s.id], M).forEach(function (o) { if (!M.byId[o.id]) { M.byId[o.id] = o; M.feed.push(o); } });
        return;
      }
      var unheld = ((s.view && s.view.affiliations) || []).some(function (a) { return a.rel === 'created' && (!M.byId[a.peer] || M.byId[a.peer].item); });
      if (!unheld) return;
      door('/v2/site/' + s.id + '/items').then(function (r) {
        var was = JSON.stringify(ITEMS[s.id] || null);
        ITEMS[s.id] = r;
        if (M && was !== JSON.stringify(r || null)) { load(); }
      }, function () {});
    });
  }

  /* THE DOOR SAYS WHEN ANYTHING CHANGED — a write here, or what its account's
     background sync brought in from the relay — and the page asks again. */
  var events = null, again = null, backoff = 0, heard = null, opened = false;
  function listen() {
    if (events || typeof EventSource === 'undefined') return;
    events = new EventSource(DOOR + '/v2/events', { withCredentials: true });
    events.addEventListener('changed', function () {
      if (heard == null) heard = T.begin();   // the first change not yet drawn
      clearTimeout(again);
      again = setTimeout(function () {
        var t = heard;
        heard = null;
        load().then(function () { T.shown('changed', t); });
      }, 150);
    });
    /* The Door sends nothing on open, so a stream that opens again (the browser's own
       reconnect, or a new one after a failure) reads what changed while it was down: a
       reply during a blip is drawn (NC-134). The first open does not: the landing has just
       read the graph. */
    events.onopen = function () { backoff = 0; if (opened) load(); opened = true; };
    /* A stream the browser will not reopen (a 401 is one) asks after the session once: an
       ended one goes to the door, a live one listens again, after a wait that doubles from
       2 s to a minute, so a route that keeps closing it is not asked as fast as it answers. */
    events.onerror = function () {
      if (!events || events.readyState !== 2) return;
      events = null;
      backoff = Math.min(Math.max(backoff * 2, 2000), 60000);
      door('/v2/me').then(function () { setTimeout(listen, backoff); }, function () {});
    };
  }

  function arrive() {
    doc.setAttribute('data-state', 'inside');
    var d = registration();
    if (d && d.refused) NOTED.push({ kind: 'register', id: '', why: d.refused });
    load();
    listen();
    if (location.hash === '#join') join();
    else {
      joinedDone();   // no join: a first card, saved, goes straight on
      if (d && !d.refused) register(d);
    }
  }

  /* A KIOSK'S VISITOR (A-3): the Door holds their claim; signed in, they present it.
     The Site comes by the relay, and opens when it lands. */
  /* A room the node could not join yet is named, and asked for once more: the Door
     kept the claim, and a retry completes the rooms without a second spend (D-58). */
  var JOINING = null, REFUSED = null, RETRIED = false, joinT = null;
  /* J-A (Ralph, 29 Sep): a claim's visitor joins, makes their card, and is handed to the
     Site's own site. The Door names where (`home`, the landing URL of the client registered
     for the joined Site, held to its registered origins); none, and they stay here as before. The handoff waits for the join to
     settle (its one retry included: a site's session cannot join) and for the card. */
  var HOME = null, CARDED = false, CARD_OPEN = false, joinedDone = null;
  var joined = new Promise(function (ok) { joinedDone = ok; });
  function nextUrl(u) {
    try { var x = new URL(u); return /^https?:$/.test(x.protocol) ? x.href : null; } catch (e) { return null; }
  }
  function join() {
    if (joinT == null) joinT = T.begin();
    door('/v2/join', { method: 'POST' }).then(function (r) {
      JOINING = JOINING || r.site || null;
      HOME = HOME || nextUrl(r.home);
      if (location.hash === '#join') {
        var at = ['joined'];
        if (r.c) at.push('c=' + encodeURIComponent(r.c));
        if (r.a) at.push('a=' + encodeURIComponent(r.a));
        history.replaceState(null, '', location.pathname + location.search + '#' + at.join('&'));
      }
      var left = r.unjoined || [];
      REFUSED = left.length ? left.map(function (u) { return u.room + ': ' + u.why; }).join('\n') : null;
      if (left.length && !RETRIED) { RETRIED = true; setTimeout(join, 5000); } else joinedDone();
      load().then(entry);
    }, function (e) { REFUSED = String(e && e.message || e); joinedDone(); load(); });
  }

  /* YOUR CARD, once, on entry (O-77; "All members of a community are required to publish a
     profile card on entry, which includes Name, profile icon and public key"): the name
     and a picture, written on the person's own record (group.setProfile), which core
     publishes into every group they hold. The picture is made small enough for the card
     (a 128 px square, under 11,000 characters of base64) until rich media (2.3.0). */
  function entry() {
    if (CARDED) return;
    CARDED = true;
    door('/v2/me').then(function (me) { return me && me.display_name ? joined.then(handoff) : yourCard(true); }, function () { yourCard(true); });
  }
  /* The person's own record held (M.self): now, or after reading again once a second, for up
     to 30 seconds. */
  function whenSelf() {
    return new Promise(function (ok, no) {
      var tries = 0;
      (function check() {
        if (M && M.self) return ok();
        if (++tries > 30) return no(new Error('Your own record has not arrived. Check your connection and press Save again.'));
        setTimeout(function () { load().then(check, check); }, 1000);
      })();
    });
  }
  function handoff() {
    if (!HOME) return load();
    doc.removeAttribute('data-state');
    $('boot').textContent = 'Taking you to ' + new URL(HOME).host + '…';
    $('boot').hidden = false;
    location.assign(HOME);
  }

  /* A SITE TO REGISTER (O-58). www's form hands its answers over as `#register=<draft>`:
     base64url, unpadded, of UTF-8 JSON {kind, name, purpose, slug, pname, face}, the
     face a JSON string or null. Signed out it rides the window and back; signed in it
     is shown, and made only on Register. HOSTILE DATA (Software Security): drawn as
     text, and its face held to the Face's rules before it is set. */
  var SHAPE = { Business: 'organisation', Community: 'community', Fund: 'organisation' };
  var ADDRESS = 'wallflowers.io/';
  /* Derived from the address above, so there is one spelling of the domain here: the Arc that
     serves a Face, and the site a dev's signup page is on. The www origin is FIXED, never taken
     from the draft, so a crafted `#register` cannot aim this page anywhere (W-103). */
  var ARC = 'https://arc.' + ADDRESS.replace(/\/$/, '');
  var WWW = 'https://' + ADDRESS.replace(/\/$/, '');
  /* Keys come bare (owners, rosters, a publisher) or as ed25519:<hex> (/v2/graph's me), and a
     publisher is compared and written bare — hack-seoul.js 6f0320a0, where a Site read as not
     its owner's for want of this. */
  function bareKey(k) { return String(k || '').replace(/^ed25519:/, '').toLowerCase(); }
  function b64u(text) {
    var b = new TextEncoder().encode(text), bin = '';
    for (var i = 0; i < b.length; i++) bin += String.fromCharCode(b[i]);
    return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }
  /* The dev step's fields as the contract carries them (team-registration.md; 5173 and "/" are
     MANAGE's defaults, 1 Oct): three keys, echoed back to the page that sent them. Hostile data
     like the rest of the draft, so what goes back is these three or nothing. */
  function devApp(a) {
    if (!a || typeof a !== 'object' || Array.isArray(a)) return null;
    var port = typeof a.port === 'number' && a.port > 0 && a.port < 65536 ? Math.floor(a.port) : 5173;
    var dep = typeof a.deployed === 'string' && /^https:\/\/[^\s"'<>]+$/.test(a.deployed) ? a.deployed : null;
    var path = typeof a.path === 'string' && /^\/[^\s"'<>]*$/.test(a.path) ? a.path : '/';
    return { port: port, deployed: dep, path: path };
  }
  var NOTED = [];      /* what a Register could not set: said at the top of the feed */
  var MADE = null;     /* what a Register has made so far: a retry makes nothing twice */

  var DRAFT_MAX = 32768;   /* the encoded draft, as www caps it: refused unread above */
  function registration() {
    var h = location.hash || '';
    if (!/^#register=/.test(h)) return null;
    if (h.length - 10 > DRAFT_MAX) return { refused: 'the draft is over 32 KiB' };
    var m = /^#register=([A-Za-z0-9_-]+)$/.exec(h);
    if (!m) return { refused: 'not a draft' };
    try {
      var bin = atob(m[1].replace(/-/g, '+').replace(/_/g, '/')), bytes = new Uint8Array(bin.length);
      for (var i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      var d = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
      if (!d || typeof d !== 'object' || typeof d.name !== 'string' || !d.name.trim()) return { refused: 'the draft names no site' };
      return d;
    } catch (e) { return { refused: 'not a draft: ' + (e && e.message || e) }; }
  }

  /* The face as core caps it (group.setFace: a JSON object, at most 16384 bytes) and the
     Arc's renderer draws it (arc/lib/face-render, look.rs): hex colours, the styles it
     knows, the fonts it has, and no picture fetched from anywhere. Null when it may be
     set; else why not. */
  var ONE_OF = {
    wallpaper: { style: ['fill', 'gradient', 'blur', 'pattern', 'image'], angle: ['down', 'diagonal', 'radial'],
                 pattern: ['dots', 'grid', 'lines', 'waves', 'checks', 'plus'] },
    blocks: { style: ['solid', 'glass', 'outline'], shadow: ['none', 'soft', 'strong', 'hard'] }
  };
  var IMAGE = /^(?!\/\/)[A-Za-z0-9._~\/-]+$|^data:image\/(png|jpeg|gif|webp);base64,[A-Za-z0-9+\/=]+$/;

  /* THE PICTURES: the mark, the cover (the header's banner) and the logo are the Host's media,
     set by host.setMedia one per slot, and drawn from there (face-render); none stays in the
     face. www's Face.document() writes a description in a picture's place ('data URL, <n>
     chars'), and that was stored as a Site's mark. A picture the draft carries, as `media` or
     in the face, is set on the Host if it is one host.setMedia takes; any other is named. The
     wallpaper's image stays the look's, which draws it (faceRefusal holds it). */
  var MEDIA = /^data:(image\/(?:png|jpeg|webp));base64,([A-Za-z0-9+\/]+={0,2})$/;
  var MEDIA_MAX = 156000;   /* host.setMedia's cap: base64 bytes */
  function pictures(d) {
    var out = { face: d.face, media: [], left: [] }, from = {}, f = null;
    var sent = d.media && typeof d.media === 'object' && !Array.isArray(d.media) ? d.media : {};
    ['mark', 'cover', 'logo', 'wallpaper'].forEach(function (s) { if (sent[s] != null && sent[s] !== '') from[s] = sent[s]; });
    try { f = typeof d.face === 'string' ? JSON.parse(d.face) : null; } catch (e) { f = null; }
    if (f && typeof f === 'object' && !Array.isArray(f)) {
      var h = f.header && typeof f.header === 'object' && !Array.isArray(f.header) ? f.header : null, moved = false;
      [[f, 'mark', 'mark'], [h, 'logo', 'logo'], [h, 'banner', 'cover']].forEach(function (x) {
        if (!x[0] || x[0][x[1]] == null || x[0][x[1]] === '') return;
        if (from[x[2]] == null) from[x[2]] = x[0][x[1]];
        x[0][x[1]] = '';
        moved = true;
      });
      if (moved) out.face = JSON.stringify(f);
    }
    ['mark', 'cover', 'logo', 'wallpaper'].forEach(function (slot) {
      if (from[slot] == null) return;
      var m = typeof from[slot] === 'string' ? MEDIA.exec(from[slot]) : null;
      if (m && m[2].length <= MEDIA_MAX) out.media.push({ slot: slot, mime: m[1], data: m[2] });
      else out.left.push(slot);
    });
    return out;
  }

  function faceRefusal(face) {
    if (typeof face !== 'string') return 'not a JSON string';
    if (new TextEncoder().encode(face).length > 16384) return 'over 16384 bytes';
    var f;
    try { f = JSON.parse(face); } catch (e) { return 'not JSON'; }
    if (!f || typeof f !== 'object' || Array.isArray(f)) return 'not a JSON object';
    var lk = f.look, bad = [];
    if (lk == null) return null;
    if (typeof lk !== 'object' || Array.isArray(lk)) return 'its look is not an object';
    function part(k) {
      var v = lk[k];
      if (v == null) return {};
      if (typeof v !== 'object' || Array.isArray(v)) { bad.push(k); return {}; }
      return v;
    }
    var hex = /^#[\da-f]{6}$/i, c = part('colours'), wp = part('wallpaper'), b = part('blocks'), t = part('text');
    Object.keys(c).forEach(function (k) { if (!hex.test(c[k])) bad.push('colours.' + k); });
    ['colour2', 'colour3'].forEach(function (k) { if (wp[k] != null && !hex.test(wp[k])) bad.push('wallpaper.' + k); });
    Object.keys(ONE_OF).forEach(function (p) {
      var v = p === 'wallpaper' ? wp : b;
      Object.keys(ONE_OF[p]).forEach(function (k) { if (v[k] != null && ONE_OF[p][k].indexOf(v[k]) < 0) bad.push(p + '.' + k); });
    });
    if (wp.soften != null && !(typeof wp.soften === 'number' && wp.soften >= 0 && wp.soften <= 100)) bad.push('wallpaper.soften');
    if (b.corners != null && [0, 1, 2, 3].indexOf(b.corners) < 0) bad.push('blocks.corners');
    // The renderer's own three, and whatever vendored fonts look.js lists where it is loaded.
    var L = window.WallFlowers && window.WallFlowers.Look;
    var fonts = ['marker', 'system-sans', 'system-serif'].concat(((L && L.FONTS) || []).map(function (x) { return x.id; }));
    ['font', 'title'].forEach(function (k) { if (t[k] != null && t[k] !== '' && fonts.indexOf(t[k]) < 0) bad.push('text.' + k); });
    // look.js writes it into url("…") as it stands: a plain path on this origin, or an
    // image's own data, and nothing that could close the quote (Software Security).
    if (wp.image != null && wp.image !== '' && !(typeof wp.image === 'string' && IMAGE.test(wp.image))) bad.push('wallpaper.image');
    return bad.length ? 'its look draws nothing at ' + bad.join(', ') : null;
  }

  function register(d) {
    var name = d.name.trim(), slug = String(d.slug || '').trim(), box = $('sheetFields');
    $('sheetTitle').textContent = name;
    box.textContent = '';
    if (slug) box.appendChild(el('dl', { class: 'kv' }, [el('dt', { text: 'address' }), el('dd', { text: ADDRESS + slug })]));
    $('sheetWhy').textContent = '';
    $('sheetGo').textContent = 'Register';
    $('sheet').hidden = false;
    $('sheetForm').onsubmit = function (ev) { ev.preventDefault(); made(d, name, slug); };
  }

  /* As www's form made it (signup.html, doorSubmit): the Site, its Host, the edge from
     both ends, the face, in one request, then the address, which the Site stands without.
     Each step once, however often Register is pressed. Then the address bar is cleared and
     the Site opens. */
  function made(d, name, slug) {
    var P = MADE = MADE || { at: Date.now() }, t0 = T.begin();
    var pics = pictures(d), face = pics.face;
    var faceWhy = face == null ? null : faceRefusal(face);
    function post(path, body) { return door(path, { method: 'POST', body: body }); }
    var steps = [], keys = [];
    function step(k, s) { if (!P[k]) { keys.push(k); steps.push(s); } }
    function ref(k) { return P[k] || { $step: keys.indexOf(k) }; }
    step('site', { do: 'mint', kind: 'group', draft: { name: name, shape: SHAPE[d.kind] || 'community' } });
    step('host', { do: 'mint', kind: 'host', draft: { name: name } });
    step('part', { do: 'apply', object: ref('site'), op: 'base.setPart', args: { part: ref('host'), role: 'host', at: P.at } });
    step('parent', { do: 'apply', object: ref('host'), op: 'base.setParent', args: { parent: ref('site'), role: 'host', at: P.at } });
    if (face != null && !faceWhy) step('face', { do: 'apply', object: ref('site'), op: 'group.setFace', args: { face: face } });
    pics.media.forEach(function (m) {
      step('media:' + m.slot, { do: 'apply', object: ref('host'), op: 'host.setMedia', args: { slot: m.slot, media: m.data, mediaMime: m.mime } });
    });
    busy($('sheetGo'), true);
    $('sheetWhy').textContent = '';
    (steps.length ? batch(steps) : Promise.resolve({ made: [] }))
      .then(function (r) {
        keys.forEach(function (k, i) { if (i < r.made.length) P[k] = r.made[i]; });
        if (r.refused) throw new Error(r.refused.why);
      })
      .then(function () {
        return slug ? post('/v2/site/address', { slug: slug, host: P.host }).then(function () { return null; }, function (e) { return String(e && e.message || e); }) : null;
      })
      /* THE PAGE GOES LIVE (W-103, Ralph's P1: "Right now, people will struggle"). Until this,
         runbook § 3b did it by hand, as one console paste: the Arc added to the Host with a
         fresh bundle, the Site's face copied onto it, the Host published at the address with
         the Arc as its publisher (hack-seoul.js d0efef5d, the sequence it proved). Each step
         once, like every other step here, so Register pressed again finishes what is left.
         The Arc's refusal, or the Arc not answering, is said where a refused address is. */
      .then(function (unaddressed) {
        if (unaddressed || !slug) return unaddressed;
        /* NOT A WALL (MANAGE, 1 Oct). The Site is made and the address is claimed before this
           runs, so a page that did not go live is SAID, where an unset face is said, and the
           person goes on. The Arc away, or its /v1/bundle answering without the CORS header —
           which a browser reports as "Failed to fetch", words none of ours — must not hold
           someone at a sheet whose work is already done. */
        return live().then(function () { return null; }, function (e) {
          NOTED.push({ kind: 'page', id: slug, why: String(e && e.message || e) });
          return null;
        });
      })
      .then(function (unaddressed) {
        /* AN ADDRESS TAKEN since www checked it (UX-C's sign-up map, 29 Sep): the Site
           stands, and the sheet stays with the address to change. Register again claims
           that address alone, since MADE keeps what was made; closed, the Site opens
           without it, and the feed says so. */
        if (unaddressed) return askAddress(unaddressed);
        return finish(null);
      }, function (e) {
        $('sheetWhy').textContent = String(e && e.message || e);
        if (P.site) load();
      })
      .then(function () { busy($('sheetGo'), false); });

    /* The Arc reads a Face off the Host, never off the Site (the Arc is in no group of the
       Site's): so the public bundle is copied across, and the Arc is given a seat on the Host
       alone. A bundle is used once, so it is fetched fresh here. */
    function live() {
      var now = Date.now();
      var body = JSON.stringify({
        v: 1,
        profile: { displayName: name, card: { note: '', urls: [] } },
        face: face != null && !faceWhy ? face : null
      });
      return Promise.resolve()
        .then(function () {
          if (P.arc) return null;
          return fetch(ARC + '/v1/bundle', { credentials: 'omit' }).then(function (r) {
            return r.text().then(function (t) {
              if (!r.ok) throw new Error(t || 'the Arc answered ' + r.status);
              /* The route answers the bundle itself, or an object carrying it. */
              var b = String(t).trim();
              try { var j = JSON.parse(b); if (j && typeof j.bundle === 'string') b = j.bundle.trim(); } catch (e) { /* the bundle itself */ }
              if (!b) throw new Error('the Arc answered no bundle');
              return post('/v2/add', { object: P.host, bundle: b }).then(function (a) { P.arc = bareKey(a && a.member); });
            });
          });
        })
        .then(function () {
          if (P.faceOn) return null;
          return post('/v2/apply', { object: P.host, op: 'host.hydrate', args: { key: 'face', payload: body, fetchedAt: now, rev: now } })
            .then(function () { P.faceOn = true; });
        })
        .then(function () {
          if (P.published === slug) return null;
          return post('/v2/apply', { object: P.host, op: 'base.publish', args: { slug: slug, publisher: P.arc } })
            .then(function () { P.published = slug; });
        });
    }
    function finish(unaddressed) {
      UNADDRESSED = null;
      $('sheetX').textContent = 'Cancel';
      if (faceWhy) NOTED.push({ kind: 'face', id: P.site, why: faceWhy });
      if (pics.left.length) NOTED.push({ kind: 'face', id: P.site, why: 'not carried: ' + pics.left.join(', ') });
      if (unaddressed) NOTED.push({ kind: 'address', id: slug, why: unaddressed });
      MADE = null;
      $('sheet').hidden = true;
      history.replaceState(null, '', location.pathname + location.search);
      /* The owner's first card, now the Site is theirs: the landing let the draft's
         sheet go first, and the name they gave on www comes with it. */
      /* BACK TO THE PAGE THEY CAME FROM (W-103): a dev who began on the signup page gets their
         Site id and address there, to copy. The address rides only when the page is live, so
         what they are shown is what serves. A fragment, so no request carries it. */
      var dev = devApp(d.app);
      return load()
        .then(function () { open('feed', null, P.site); T.shown('register', t0); if (!M.me.name) yourCard(true, d.pname); })
        /* The signup's name, saved (HACK_USER): before the return to the dev page, so a dev
           sent back leaves named; the first card above is already open with it. */
        .then(function () { return nameFromDraft(d.pname); })
        /* LAST, and only here: a navigation begun earlier would abandon whatever this chain
           still owes — the Site's own load, and anything another step writes at Register.
           Anything added to finish() belongs BEFORE this step, never after it. */
        .then(function () {
          if (!dev) return null;
          var back = { site: P.site, app: dev };
          if (P.published) back.slug = P.published;
          location.assign(WWW + '/signup#registered=' + b64u(JSON.stringify(back)));
          return null;
        });
    }
    function askAddress(why) {
      var box = $('sheetFields'), field = el('input', { id: 'regSlug', type: 'text', maxlength: '32', value: slug,
        autocapitalize: 'off', autocorrect: 'off', spellcheck: 'false', 'aria-label': 'Address' });
      box.textContent = '';
      box.appendChild(el('label', { class: 'fld' }, [el('span', { text: ADDRESS }), field]));
      $('sheetWhy').textContent = /taken|held|409/.test(why) ? ADDRESS + slug + ' is taken. Choose another.' : ADDRESS + slug + ': ' + why;
      $('sheetGo').textContent = 'Use this address';
      UNADDRESSED = function () { return finish(why); };
      $('sheetX').textContent = 'Later';   // the Site is made: closing opens it, without the address
      $('sheetForm').onsubmit = function (ev) {
        ev.preventDefault();
        var v = String(field.value || '').trim().toLowerCase();
        if (v.length < 2 || !/^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(v)) { $('sheetWhy').textContent = 'Letters, numbers and hyphens, two or more, not starting or ending with one.'; return; }
        made(d, name, v);
      };
      try { field.focus(); } catch (e) { /* none */ }
    }
  }
  /* The draft's sheet closed while its address is asked again: the Site opens without it. */
  var UNADDRESSED = null;
  function closeSheet() {
    $('sheet').hidden = true;
    if (UNADDRESSED) UNADDRESSED();
  }

  /* ── drawing ───────────────────────────────────────────────────────────── */
  var HUES = ['#E57D8B', '#EE826E', '#E79D51', '#BEB34D', '#469C48', '#418CB2', '#0D6AAE', '#CF84A8'];
  function hue(s) {
    var h = 0, str = String(s || '');
    for (var i = 0; i < str.length; i++) h = (h * 31 + str.charCodeAt(i)) >>> 0;
    return HUES[h % HUES.length];
  }
  function count(n, one, many) { return n + ' ' + (n === 1 ? one : many); }
  function el(tag, attrs, kids) {
    var n = document.createElement(tag);
    Object.keys(attrs || {}).forEach(function (k) {
      if (k === 'class') n.className = attrs[k];
      else if (k === 'text') n.textContent = attrs[k];
      else if (k === 'html') n.innerHTML = attrs[k];
      else if (k.slice(0, 2) === 'on') n.addEventListener(k.slice(2), attrs[k]);
      else if (attrs[k] != null) n.setAttribute(k, attrs[k]);
    });
    (kids || []).forEach(function (c) { if (c) n.appendChild(typeof c === 'string' ? document.createTextNode(c) : c); });
    return n;
  }
  var ICON = {
    feed: '<path d="M4 5h16v6H4zM4 13h16v6H4z"/>',
    members: '<circle cx="9" cy="8" r="3.2"/><path d="M3.5 19c.6-3.2 2.8-5 5.5-5s4.9 1.8 5.5 5M16 5.2a3 3 0 010 5.6M17.5 14.2c1.7.6 2.7 2.3 3 4.8"/>',
    hash: '<path d="M9 4L7 20M17 4l-2 16M4 9h17M3 15h17"/>',
    search: '<circle cx="11" cy="11" r="6.5"/><path d="M20 20l-4.2-4.2"/>',
    menu: '<path d="M4 7h16M4 12h16M4 17h16"/>',
    back: '<path d="M15 5l-7 7 7 7"/>',
    send: '<path d="M4 12l16-7-6 16-3-6.5z"/>',
    out: '<path d="M14 4h5v16h-5M10 8l-4 4 4 4M6 12h10"/>',
    doc: '<path d="M7 3h7l4 4v14H7zM14 3v4h4M10 12h5M10 16h5"/>',
    cal: '<rect x="4" y="5" width="16" height="15" rx="2"/><path d="M4 10h16M9 3v4M15 3v4"/>',
    tag: '<path d="M3 12V4h8l10 10-8 8z"/><circle cx="7.5" cy="8.5" r="1.3"/>',
    fund: '<ellipse cx="12" cy="6.5" rx="7" ry="2.5"/><path d="M5 6.5v5c0 1.4 3.1 2.5 7 2.5s7-1.1 7-2.5v-5M5 11.5v5c0 1.4 3.1 2.5 7 2.5s7-1.1 7-2.5v-5"/>',
    chevL: '<path d="M14 6l-6 6 6 6"/>',
    chevR: '<path d="M10 6l6 6-6 6"/>',
    chevD: '<path d="M7 10l5 5 5-5"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    rail: '<rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><path d="M9.5 4.5v15"/>',
    home: '<path d="M4 11l8-6 8 6v8.5H4z"/>',
    person: '<circle cx="12" cy="9" r="3.4"/><path d="M5.5 19.5c.9-3.4 3.4-5.3 6.5-5.3s5.6 1.9 6.5 5.3"/>',
    /* MEET, Ralph's glyph (28 Sep, drawn in marker on squared paper): a ring, and four legs
       leaving it about 26° either side of the vertical, as long as it is wide. */
    meet: '<circle cx="12" cy="12" r="5.2"/><path d="M9.72 7.33L7.45 2.87M14.28 7.33l2.27-4.46M9.72 16.67l-2.27 4.46M14.28 16.67l2.27 4.46"/>',
    close: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
    smile: '<circle cx="12" cy="12" r="8.5"/><path d="M8.6 14.2c.9 1.3 2 1.9 3.4 1.9s2.5-.6 3.4-1.9"/><circle cx="9.2" cy="10" r=".6" fill="currentColor"/><circle cx="14.8" cy="10" r=".6" fill="currentColor"/>',
    reply: '<path d="M9.5 7L4.5 12l5 5"/><path d="M5 12h9.5a5 5 0 015 5v1"/>',
    image: '<rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><circle cx="9" cy="10" r="1.8"/><path d="M20.5 16l-5-5L5 19.5"/>',
    link: '<path d="M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1"/>',
    zin: '<circle cx="11" cy="11" r="6.5"/><path d="M20 20l-4.2-4.2M11 8v6M8 11h6"/>',
    zout: '<circle cx="11" cy="11" r="6.5"/><path d="M20 20l-4.2-4.2M8 11h6"/>',
    info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5.5M12 8v.01"/>',
    copy: '<rect x="8.5" y="8.5" width="11" height="11" rx="2.5"/><path d="M15.5 8.5V6a1.5 1.5 0 00-1.5-1.5H6A1.5 1.5 0 004.5 6v8A1.5 1.5 0 006 15.5h2.5"/>',
    trash: '<path d="M4.5 7h15M9.5 7V5h5v2M6.5 7l1 12.5h9l1-12.5M10.5 11v5M13.5 11v5"/>',
    face: '<rect x="6" y="2.5" width="12" height="19" rx="2.5"/><circle cx="12" cy="8" r="2.2"/><path d="M9 12.5h6M9 15.5h6"/>',
    crown: '<path d="M4 17.5L3 8l5 4 4-7 4 7 5-4-1 9.5z"/>',
    shield: '<path d="M12 3l7 3v5.5c0 4.3-3 7.7-7 9.5-4-1.8-7-5.2-7-9.5V6z"/>',
    eye: '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/>',
    ticket: '<path d="M4 7h16v3a2 2 0 000 4v3H4v-3a2 2 0 000-4z"/>',
    key: '<circle cx="8" cy="15" r="3.8"/><path d="M10.8 12.2L20 3M16.5 6.5l3 3"/>',
    link: '<path d="M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1"/>'
  };
  function icon(name) {
    return '<span class="ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" ' +
           'stroke-linecap="round" stroke-linejoin="round">' + ICON[name] + '</svg></span>';
  }
  function ic(name) { return el('span', { html: icon(name) }).firstChild; }
  /* A name, or nobody: never a key (Ralph on O-77: a name, and until then a hue and a mark). */
  function named(pk) { var p = profileOf(pk); return (p && typeof p.name === 'string' && p.name.trim()) || M.names[pk] || ''; }
  function who(pk) { return named(pk) || (pk === M.me.pk ? 'You' : 'New member'); }
  /* A MEMBER'S CARD, which every member publishes on entry (Ralph, 28 Sep: "Name, profile
     icon and public key"). The op is designed and not built (base.publishProfile, the
     `profiles` facet; MANAGE 28 Sep), so a card is drawn from what the Door serves today:
     the key from the roster, a name where a connection carries one, the standing and offices
     from the Site. The group view's `profiles`, {<member>: {name, icon: {mime, data}, gen}},
     is read the day it is served. */
  /* A Site's cards as core serves them (BW-D, 28 Sep): the group's and the forum's view
     carry `profiles` as [[member, {displayName, icon}]], the icon a data: URL, as `roles`
     are pairs. Read into {member: {name, icon}}; a map, or {name, icon: {mime, data}}, is
     read the same way. An icon is drawn only as a still picture's own data. */
  var CARDS = typeof WeakMap === 'function' ? new WeakMap() : null;
  function iconUrl(i) {
    if (typeof i === 'string') return /^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+\/]+={0,2}$/.test(i) ? i : null;
    return i && i.data && /^image\/(png|jpeg|webp)$/.test(i.mime || '') ? 'data:' + i.mime + ';base64,' + i.data : null;
  }
  function cardsOf(o) {
    if (CARDS && CARDS.has(o)) return CARDS.get(o);
    var raw = o && o.view && o.view.profiles, out = {};
    function put(pk, c) {
      if (!c || typeof c !== 'object' || typeof pk !== 'string') return;
      var n = typeof c.displayName === 'string' ? c.displayName : typeof c.name === 'string' ? c.name : '';
      out[strip(pk)] = { name: n.trim(), icon: iconUrl(c.icon) };
    }
    if (Array.isArray(raw)) raw.forEach(function (x) { if (Array.isArray(x)) put(x[0], x[1]); });
    else if (raw && typeof raw === 'object') Object.keys(raw).forEach(function (k) { put(k, raw[k]); });
    if (CARDS && o) CARDS.set(o, out);
    return out;
  }
  function profileOf(pk) {
    var sites = S.site && M.byId[S.site] ? [M.byId[S.site]].concat(M.sites) : M.sites;
    for (var i = 0; i < sites.length; i++) { var p = cardsOf(sites[i])[pk]; if (p) return p; }
    if (pk === M.me.pk) { var c = selfCard(); if (c) return { name: M.me.name, icon: c.photo ? iconUrl({ mime: c.photo_mime, data: c.photo }) : null }; }
    return null;
  }
  /* One's own card is the self record's (group.setProfile's card, REQUIREMENTS.md § 3). */
  function selfCard() {
    var s = M.self && M.byId[M.self], c = s && s.view && s.view.card;
    if (typeof c === 'string') { try { c = JSON.parse(c); } catch (e) { c = null; } }
    return c && typeof c === 'object' ? c : null;
  }
  function iconOf(pk) { var p = profileOf(pk); return (p && p.icon) || null; }
  function personCard(pk) {
    var site = S.site && M.byId[S.site], box = $('personBody'), p = profileOf(pk);
    box.textContent = '';
    var big = avatar(pk); big.classList.add('big');
    box.appendChild(el('div', { class: 'pc-head' }, [big, el('div', { class: 'grow' }, [
      el('h3', { text: who(pk) }),
      el('p', { text: pk === M.me.pk ? 'You' : M.chats[pk] ? 'Connected' : 'Not connected' })])]));
    var kv = el('dl', { class: 'kv' });
    function row(k, v) { kv.appendChild(el('dt', { text: k })); kv.appendChild(v); }
    if (site) {
      var r = pk === site.owner ? 'owner' : (((site.view && site.view.roles) || []).filter(function (x) { return x[0] === pk; })[0] || [])[1] || ((site.members || []).indexOf(pk) >= 0 ? 'member' : null);
      if (r && ROLE[r]) row('In ' + titleOf(site), el('dd', { class: 'pc-role' }, [el('span', { class: 'role', style: '--r:' + ROLE[r].hue,
        html: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round">' + ICON[ROLE[r].glyph] + '</svg>' }), el('span', { text: ROLE[r].say })]));
      var offs = ((site.view && site.view.offices) || []).filter(function (o) { return o.holder === pk; }).map(function (o) { return o.office; });
      if (offs.length) row('Titles', el('dd', { text: offs.join(', ') }));
    }
    var key = 'ed25519:' + pk;
    row('Public key', el('dd', { class: 'pc-key' }, [el('code', { text: pk.slice(0, 8) + '…' + pk.slice(-8), title: key }),
      el('button', { class: 'tb', type: 'button', 'aria-label': 'Copy the key', title: 'Copy', html: icon('copy'), onclick: function (e) {
        var b = e.currentTarget;
        (navigator.clipboard ? navigator.clipboard.writeText(key) : Promise.reject()).then(function () { b.classList.add('done'); setTimeout(function () { b.classList.remove('done'); }, 1200); }, function () {});
      } })]));
    box.appendChild(kv);
    if (!named(pk) && !p) box.appendChild(el('p', { class: 'mnote', text: 'No card yet. Every member publishes one on entry: their name, their picture and this key.' }));
    if (M.chats[pk] && pk !== M.me.pk) box.appendChild(el('button', { class: 'pc-go', type: 'button', text: 'Message', onclick: function () { closeCard(); open('dm', pk); } }));
    /* The owner gives and takes the admin role here (REQUIREMENTS.md § 4). */
    if (site && site.owner === M.me.pk && pk !== M.me.pk && (site.members || []).indexOf(pk) >= 0) {
      var admin = roleOfMember(site, pk) === 'admin';
      box.appendChild(el('button', { class: 'pc-alt', type: 'button', text: admin ? 'Remove admin' : 'Make admin',
        onclick: function (e) { (admin ? unmakeAdmin : makeAdmin)(site, pk, e.currentTarget); } }));
      /* The Door adds someone to the page (the Host) only if they are your contact (BW-B). */
      box.appendChild(el('p', { class: 'why', id: 'pcWhy',
        text: !admin && !M.chats[pk] ? 'To change the pictures too, they need to be connected with you first.' : '' }));
    }
    $('person').hidden = false;
  }
  function closeCard() { $('person').hidden = true; }
  function roleOfMember(site, pk) {
    if (pk === site.owner) return 'owner';
    var r = ((site.view && site.view.roles) || []).filter(function (x) { return x[0] === pk; })[0];
    return r ? r[1] : ((site.members || []).indexOf(pk) >= 0 ? 'member' : null);
  }
  /* MAKE ADMIN, three steps on the owner's device (MANAGE, 28 Sep): the role on the Site
     (base.setRole), the person added to the Site's Host (/v2/add, by their key: REQUIREMENTS.md
     § 4), and the role on the Host too, without which an admin cannot set the Site's pictures.
     Each step says where it got to; a refusal is the Door's own words. */
  function makeAdmin(site, pk, btn) {
    var host = partsIn(site, 'host')[0], hostObj = host && M.byId[host.part], say = function (s) { var w = $('pcWhy'); if (w) w.textContent = s; };
    function setRole(obj) { return door('/v2/apply', { method: 'POST', body: { object: obj, op: 'base.setRole', args: { member: pk, role: 'admin' } } }); }
    busy(btn, true);
    say('Making ' + who(pk) + ' an admin of ' + titleOf(site) + '…');
    var siteDone = false;
    setRole(site.id).then(function () {
      siteDone = true;
      if (!host) return 'no Host';
      var onHost = hostObj && (hostObj.members || []).indexOf(pk) >= 0;
      say('Adding them to the page (the Host)…');
      /* A role overlays a member of that object (roles.rs), so the Host's role waits on the add. */
      return (onHost ? Promise.resolve() : door('/v2/add', { method: 'POST', body: { object: host.part, member: pk } }))
        .then(function () { say('Making them an admin of the page…'); return setRole(host.part); });
    }).then(function () {
      /* And of the Site's rooms they are in: a room's admin is its own roles' (W-98 Rooms). */
      return Promise.all(roomsOf(site).filter(function (r) { return !r.missing && (r.members || []).indexOf(pk) >= 0; }).map(function (r) { return setRole(r.id); }));
    }).then(function () { return load().then(function () { personCard(pk); }); },
      function (e) {
        var why = String(e && e.message || e);
        load().then(function () {
          personCard(pk);
          say(siteDone ? who(pk) + ' is an admin of ' + titleOf(site) + ', but not yet of its page, so not of its pictures: ' + why : 'Stopped: ' + why);
        });
      })
      .then(function () { busy(btn, false); });
  }
  function unmakeAdmin(site, pk, btn) {
    var host = partsIn(site, 'host')[0];
    function clear(obj) { return door('/v2/apply', { method: 'POST', body: { object: obj, op: 'base.clearRole', args: { member: pk } } }); }
    busy(btn, true);
    clear(site.id).then(function () { return host ? clear(host.part).catch(function () {}) : null; })
      .then(function () {
        return Promise.all(roomsOf(site).filter(function (r) { return ((r.view && r.view.roles) || []).some(function (x) { return x[0] === pk; }); })
          .map(function (r) { return clear(r.id).catch(function () {}); }));
      })
      .then(function () { return load().then(function () { personCard(pk); }); },
        function (e) { var w = $('pcWhy'); if (w) w.textContent = 'Stopped: ' + String(e && e.message || e); })
      .then(function () { busy(btn, false); });
  }

  /* YOUR CARD (O-77; REQUIREMENTS.md § 3): a name and a picture, asked once on arrival when
     the Door knows no name, and changed from the profile menu. Written where it is the
     person's own, the self record: group.setProfile {displayName, shape, card: {photo,
     photo_mime}}, an op that exists. Core publishes it into every community held, at entry
     and on each change; the picture is made small here, inside the card's 500,000 cap. */
  /* The picture a card carries. Full size waits on rich media, ICD 2.3.0 (O-79: R2 offload,
     REQUIREMENTS.md § 9); until then it is made to fit the published card's 16,384 bytes
     whole (Ralph, 29 Sep: "if we can fit smaller profile pictures, that'd be great"): 128 px,
     webp where the browser writes it and jpeg where not, under 11,000 characters of base64.
     If core must still leave it out, /v2/me's card_note says so and Your card shows it. */
  var PHOTO_MAX = 11000, CARD_NOTE = null;
  /* `o` (a listing's photo, trade.js): {max, side, square: false} for the whole picture at
     most `side` on its long edge, within `max`; none is the card's. */
  function shrinkPhoto(file, o) {
    o = o || {};
    var MAX = o.max || PHOTO_MAX, SQUARE = o.square !== false, TYPE = o.type || 'image/webp';
    return new Promise(function (ok, no) {
      var img = new Image(), url = URL.createObjectURL(file);
      img.onload = function () {
        URL.revokeObjectURL(url);
        var side = o.side || 128, q = 0.82, out = null;
        for (var tries = 0; tries < 12; tries++) {
          // square, from the middle: a face is a circle
          var sq = Math.min(img.width, img.height), c = document.createElement('canvas');
          if (SQUARE) {
            c.width = c.height = Math.min(side, sq);
            c.getContext('2d').drawImage(img, (img.width - sq) / 2, (img.height - sq) / 2, sq, sq, 0, 0, c.width, c.height);
          } else {
            var k = Math.min(1, side / Math.max(img.width, img.height));
            c.width = Math.round(img.width * k); c.height = Math.round(img.height * k);
            c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
          }
          out = c.toDataURL(TYPE, q);
          if (out.indexOf('data:' + TYPE) !== 0) out = c.toDataURL('image/jpeg', q);
          if (out.length - out.indexOf(',') - 1 <= MAX) return ok(out);
          if (q > 0.6) q -= 0.08; else side = Math.round(side * 0.85);
        }
        no(new Error('that picture will not come small enough'));
      };
      img.onerror = function () { URL.revokeObjectURL(url); no(new Error('not a picture this browser can read')); };
      img.src = url;
    });
  }
  /* YOUR CARD: from the profile menu, #you, and on entry. The first (`first`: a member with
     no name yet, or a claim's visitor after the join) cannot be put off, since every member
     publishes one (O-77), and once saved goes on to the Site's home (J-A). Save waits for the
     person's own record, which on a fresh device can arrive a moment after they do, and a
     write met by it settling is tried once more. A picture not changed is not sent: core
     keeps the one held (group.rs OP_SET_PROFILE). */
  function closeYou() { if (!CARD_OPEN) $('you').hidden = true; }
  /* THE SIGNUP'S NAME, SAVED AT REGISTER (HACK_USER): the name given on www (the draft's
     pname) is written as Your card's Save writes it, group.setProfile on the person's own
     record, so they are named everywhere at once rather than "New member" until they press
     Save. Only for a person with no name yet, and only with their record held; a refusal
     leaves the name to the card, as before. */
  function nameFromDraft(pname) {
    var n = typeof pname === 'string' ? pname.trim().slice(0, 48) : '';
    if (!n || !M.self || M.me.name) return Promise.resolve();
    var self = M.byId[M.self], args = { displayName: n, shape: (self && self.view && self.view.shape) || 'individual' };
    return door('/v2/apply', { method: 'POST', body: { object: M.self, op: 'group.setProfile', args: args } })
      .then(function () { M.me.name = n; M.names[M.me.pk] = n; }, function () {});
  }
  function yourCard(first, suggest) {
    var box = $('youBody'), c = selfCard() || {}, pic = c.photo ? 'data:' + (c.photo_mime || 'image/jpeg') + ';base64,' + c.photo : null, picked = false;
    box.textContent = '';
    var face = el('label', { class: 'you-pic', style: '--t:' + hue(M.me.pk) });
    function showPic() { face.textContent = ''; if (pic) face.appendChild(el('img', { src: pic, alt: '' })); else face.appendChild(el('span', { html: icon('person') })); face.appendChild(file); face.appendChild(el('span', { class: 'you-edit', text: pic ? 'Change' : 'Add a picture' })); }
    var file = el('input', { type: 'file', accept: 'image/*', 'aria-label': 'Your picture' });
    file.onchange = function () {
      if (!file.files[0]) return;
      $('youWhy').textContent = '';
      shrinkPhoto(file.files[0]).then(function (u) { pic = u; picked = true; showPic(); }, function (e) { $('youWhy').textContent = String(e.message || e); });
    };
    showPic();
    var name = el('input', { id: 'youName', type: 'text', maxlength: '48', autocomplete: 'name', value: M.me.name || (typeof suggest === 'string' ? suggest.trim().slice(0, 48) : ''), placeholder: 'Your name' });
    box.appendChild(el('h3', { text: 'Your card' }));
    box.appendChild(face);
    box.appendChild(el('label', { class: 'fld' }, [el('span', { text: 'Name' }), name]));
    box.appendChild(el('p', { class: 'you-key', text: 'ed25519:' + M.me.pk.slice(0, 12) + '…' + M.me.pk.slice(-8) }));
    box.appendChild(el('p', { class: 'why', id: 'youWhy', text: CARD_NOTE || '' }));
    var go = el('button', { class: 'pc-go', id: 'youGo', type: 'button', text: 'One moment…' });
    busy(go, true);
    function ready() { go.textContent = 'Save'; busy(go, false); }
    whenSelf().then(ready, ready);
    go.onclick = function () {
      if (go.disabled) return;
      var n = name.value.trim();
      if (!n) { $('youWhy').textContent = 'A name, please: it is how people here will know you.'; return; }
      busy(go, true);
      $('youWhy').textContent = '';
      function write() {
        var self = M.self && M.byId[M.self], args = { displayName: n, shape: (self && self.view && self.view.shape) || 'individual' };
        var m = picked && /^data:(image\/[a-z]+);base64,(.*)$/.exec(pic), card = selfCard() || {};
        if (m) { card.photo = m[2]; card.photo_mime = m[1]; args.card = JSON.stringify(card); }
        return door('/v2/apply', { method: 'POST', body: { object: M.self, op: 'group.setProfile', args: args } });
      }
      whenSelf().then(function () {
        return write().catch(function () { return new Promise(function (ok) { setTimeout(ok, 1500); }).then(load).then(write); });
      })
        .then(function () {
          CARD_OPEN = false;
          $('you').hidden = true;
          return first ? joined.then(handoff) : load();
        }, function (e) { $('youWhy').textContent = String(e && e.message || e); })
        .then(function () { busy(go, false); });
    };
    box.appendChild(go);
    if (!first) box.appendChild(contactCode());
    CARD_OPEN = !!first;
    $('youX').hidden = !!first;
    $('you').hidden = false;
    if (!M.me.name) name.focus();
  }
  /* YOUR CONTACT CODE (W-96): what an owner pastes to add this person to a community and up
     to five of its rooms. The Door's POST /v2/bundle answers six fresh key packages under the
     person's identity, kept by the Door (so the owner may add them hours later): one for the
     community, one a room. The code is `wf1.` + base64url of that JSON array. Each is for one
     community: the owner's webapp refuses a code it has used. */
  var CODE_PREFIX = 'wf1.', USED_KEY = 'wallflowers.codes.used';
  function b64urlOf(s) { return btoa(unescape(encodeURIComponent(s))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''); }
  function fromB64url(s) { s = s.replace(/-/g, '+').replace(/_/g, '/'); while (s.length % 4) s += '='; return decodeURIComponent(escape(atob(s))); }
  function packCode(bundles) { return CODE_PREFIX + b64urlOf(JSON.stringify(bundles)); }
  function unpackCode(code) {
    var c = String(code || '').replace(/\s+/g, '');
    if (c.indexOf(CODE_PREFIX) !== 0) return null;
    try { var a = JSON.parse(fromB64url(c.slice(CODE_PREFIX.length))); return Array.isArray(a) && a.length && a.every(function (x) { return typeof x === 'string' && x; }) ? a : null; } catch (e) { return null; }
  }
  /* A code's mark, to know it again: not a secret, only a name for it in this browser. */
  function codeMark(code) { var h = 5381, c = String(code).replace(/\s+/g, ''); for (var i = 0; i < c.length; i++) h = ((h * 33) ^ c.charCodeAt(i)) >>> 0; return c.length + ':' + h.toString(36); }
  function usedCodes() { try { return JSON.parse(localStorage.getItem(USED_KEY) || '[]'); } catch (e) { return []; } }
  function markUsed(code) { try { localStorage.setItem(USED_KEY, JSON.stringify(usedCodes().concat([codeMark(code)]).slice(-200))); } catch (e) { /* not kept */ } }
  function contactCode() {
    var wrap = el('div', { class: 'you-code' }), out = el('textarea', { id: 'codeOut', readonly: 'readonly', rows: '3', hidden: 'hidden', 'aria-label': 'Your contact code' });
    var note = el('p', { class: 'mnote' });
    var b = el('button', { class: 'pc-go ghost', id: 'codeGo', type: 'button', text: 'Copy a new code' });
    b.onclick = function () {
      busy(b, true);
      door('/v2/bundle', { method: 'POST' }).then(function (r) {
        var bundles = (r && r.bundles) || [];
        if (!bundles.length) throw new Error('No code came back. Try again.');
        var code = packCode(bundles);
        out.value = code; out.hidden = false;
        try { out.select(); } catch (e) { /* none */ }
        return (navigator.clipboard ? navigator.clipboard.writeText(code) : Promise.reject()).then(
          function () { note.textContent = 'Copied.'; }, function () { note.textContent = ''; });
      }).then(null, function (e) { note.textContent = String(e && e.message || e); }).then(function () { busy(b, false); });
    };
    wrap.appendChild(el('h4', { text: 'Your contact code' }));
    wrap.appendChild(b); wrap.appendChild(out); wrap.appendChild(note);
    return wrap;
  }
  /* ADD BY CONTACT CODE (W-96), the owner's: the code's first key makes the person a member of
     the community (POST /v2/add {object, bundle} → {member}); each next key, of one of its
     rooms, in order, up to five. A room past five needs another code, and is named. */
  function addByCode(site) {
    var box = $('sheetFields'), input = el('textarea', { id: 'codeIn', rows: '4', spellcheck: 'false', autocapitalize: 'off', 'aria-label': 'Contact code' });
    box.textContent = '';
    box.appendChild(el('label', { class: 'fld' }, [el('span', { text: 'Contact code' }), input]));
    $('sheetTitle').textContent = 'Add by contact code';
    $('sheetWhy').textContent = '';
    $('sheetGo').textContent = 'Add';
    $('sheet').hidden = false;
    $('sheetForm').onsubmit = function (ev) {
      ev.preventDefault();
      var code = String(input.value || '').trim(), bundles = unpackCode(code);
      if (!code) { $('sheetWhy').textContent = 'Paste their contact code.'; return; }
      if (!bundles) { $('sheetWhy').textContent = 'Not a contact code.'; return; }
      if (usedCodes().indexOf(codeMark(code)) >= 0) { $('sheetWhy').textContent = 'Used already. Ask them for a new one.'; return; }
      var go = $('sheetGo'), rooms = partsIn(site, 'room').map(function (p) { return p.part; }), missed = [];
      var name = function (id) { return (M.byId[id] && titleOf(M.byId[id])) || 'a room'; };
      busy(go, true);
      $('sheetWhy').textContent = '';
      door('/v2/add', { method: 'POST', body: { object: site.id, bundle: bundles[0] } }).then(function (r) {
        if (!r || !r.member) throw new Error('The code was taken, but no member came back.');
        markUsed(code);
        return rooms.reduce(function (p, room, i) {
          return p.then(function () {
            if (!bundles[i + 1]) { missed.push(name(room) + ': needs another code'); return; }
            return door('/v2/add', { method: 'POST', body: { object: room, bundle: bundles[i + 1] } }).then(null, function (e) {
              missed.push(name(room) + ': ' + String(e && e.message || e));
            });
          });
        }, Promise.resolve()).then(function () {
          $('sheet').hidden = true;
          toast('Added to ' + titleOf(site) + (missed.length ? ', not to ' + missed.length + ' room' + (missed.length === 1 ? '' : 's') : ''));
          if (missed.length) NOTED.push({ kind: 'add', id: r.member, why: missed.join('\n') });
          return load();
        });
      }).then(null, function (e) { $('sheetWhy').textContent = String(e && e.message || e); })
        .then(function () { busy(go, false); });
    };
    try { input.focus(); } catch (e) { /* none */ }
  }
  function avatar(pk) {
    var n = named(pk), pic = iconOf(pk), a = el('span', { class: 'av' + (n || pic ? '' : ' anon'), style: '--t:' + hue(pk), 'aria-hidden': 'true' });
    if (pic) a.appendChild(el('img', { src: pic, alt: '' }));
    else if (n) a.textContent = n.slice(0, 1).toUpperCase(); else a.innerHTML = icon('person');
    return a;
  }
  function titleOf(o) { var v = o.view || {}; return v.title || v.display_name || o.name || o.kind; }
  function msgs(id) { var o = id && M.byId[id]; return (o && o.view && o.view.messages) || []; }
  function partsIn(o, role) {
    return ((o.view && o.view.parts) || []).filter(function (p) { return p.role === role; });
  }
  function commentsOf(o) {
    var p = partsIn(o, 'comments')[0];
    return p ? p.part : null;
  }
  function siteOf(o) { var h = M.host[o.id] || M.origin[o.id]; return h && M.byId[h]; }
  function when(ts) {
    if (!ts) return '';
    var d = Date.now() - ts, m = 60000;
    if (d < m) return 'now';
    if (d < 60 * m) return Math.floor(d / m) + 'm';
    if (d < 24 * 60 * m) return Math.floor(d / 3600000) + 'h';
    if (d < 7 * 24 * 60 * m) return Math.floor(d / 86400000) + 'd';
    return new Date(ts).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' });
  }

  /* A SITE'S MARK: the Host's own picture (host.setMedia 'mark'), else its initial in its
     Face's button colours, else in its hue. Home is WallFlowers'. */
  function markOf(site) {
    if (!site) {
      var h = el('span', { class: 'mark home' });
      h.appendChild(el('img', { src: 'brand/wallflowers-icon.svg', alt: '' }));
      return h;
    }
    var host = partsIn(site, 'host')[0], hv = host && M.byId[host.part] && M.byId[host.part].view;
    var pic = hv && hv.media && hv.media.mark;
    /* In the Site's own accent from its palette, which always reads, never its raw button
       colours (Tulip's are white on white). */
    var look = site.view && site.view.face && site.view.face.look, pal = look && paletteOf(look);
    var bg = pal ? pal.vars['--accent'] : hue(site.id), fg = pal ? pal.vars['--on-accent'] : '#fff';
    var m = el('span', { class: 'mark', style: '--t:' + bg + ';--tf:' + fg, 'aria-hidden': 'true' });
    if (pic && pic.data && /^image\/(png|jpeg|webp)$/.test(pic.mime || '')) m.appendChild(el('img', { src: 'data:' + pic.mime + ';base64,' + pic.data, alt: '' }));
    else m.textContent = titleOf(site).trim().slice(0, 1).toUpperCase();
    return m;
  }

  /* YOUR STANDING IN A SITE, as the Door serves it (/v2/graph), never guessed: the Site's
     `owner` is the object's owner, the one who may write its owner-only ops (mayLink reads
     the same field); else your entry in its view's `roles`, the overlay `base.setRole`
     writes (owner | admin | member | viewer | guest | admitter; GroupRole also has peer);
     else a member of its roster. Offices (`group.setOffice`: chair, webmaster, …) are
     titles that grant nothing, so they are named in the tooltip, never drawn as the role. */
  var ROLE = {
    owner:    { hue: '#E79D51', glyph: 'crown',  say: 'Owner' },
    admin:    { hue: '#0D6AAE', glyph: 'shield', say: 'Admin' },
    member:   { hue: '#469C48', glyph: 'person', say: 'Member' },
    viewer:   { hue: '#8D939A', glyph: 'eye',    say: 'Viewer' },
    guest:    { hue: '#A58BD6', glyph: 'ticket', say: 'Guest' },
    admitter: { hue: '#418CB2', glyph: 'key',    say: 'Admitter' },
    peer:     { hue: '#CF84A8', glyph: 'link',   say: 'Peer' }
  };
  function roleIn(site) {
    var me = M.me.pk;
    if (site.owner === me) return 'owner';
    var r = ((site.view && site.view.roles) || []).filter(function (x) { return x[0] === me; })[0];
    if (r && ROLE[r[1]]) return r[1];
    return (site.members || []).indexOf(me) >= 0 ? 'member' : null;
  }
  function roleBadge(site) {
    var r = site && roleIn(site);
    if (!r) return null;
    var offices = ((site.view && site.view.offices) || []).filter(function (o) { return o.holder === M.me.pk; })
      .map(function (o) { return o.office + (o.status === 'pending' ? ' (offered)' : ''); });
    var say = 'Your role: ' + ROLE[r].say + (offices.length ? ' · ' + offices.join(', ') : '');
    return el('span', { class: 'role', style: '--r:' + ROLE[r].hue, title: say, role: 'img', 'aria-label': say,
      html: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round">' + ICON[ROLE[r].glyph] + '</svg>' });
  }

  /* ── SITE SETTINGS: what the role button opens. What is offered is what the ICD lets
     your standing write: every op that changes a Site (its face, rooms, roles, offices,
     links) is owner-only (ego: owner), so an admin is offered the face to work on and told
     plainly the Door will not take it from them (Ralph, 28 Sep: owners and admins). */
  function hostMedia(site) {
    var host = partsIn(site, 'host')[0], v = host && M.byId[host.part] && M.byId[host.part].view, out = {};
    Object.keys((v && v.media) || {}).forEach(function (slot) {
      var m = v.media[slot];
      if (m && m.data && /^image\/(png|jpeg|webp|gif)$/.test(m.mime || '')) out[slot] = 'data:' + m.mime + ';base64,' + m.data;
    });
    return out;
  }
  function opt(ic_, title, sub, go, off) {
    return el('button', { class: 'opt', type: 'button', disabled: off ? 'disabled' : null, onclick: go },
      [el('span', { class: 'oi', html: icon(ic_) }), el('span', { class: 'grow' }, [el('b', { text: title }), sub ? el('span', { class: 's', text: sub }) : null])]);
  }
  function settings() {
    var site = S.site && M.byId[S.site];
    if (!site) return;
    var r = roleIn(site), owner = r === 'owner';
    var head = $('manageHead'); head.textContent = '';
    var badge = roleBadge(site), say = badge ? badge.getAttribute('aria-label').replace(/^Your role: /, '') : '';
    head.appendChild(markOf(site));
    head.appendChild(el('div', { class: 'grow' }, [el('h3', { id: 'manageTitle', text: titleOf(site) }),
      el('p', {}, [badge, el('span', { text: say })])]));
    head.appendChild(el('button', { class: 'tb', type: 'button', 'aria-label': 'Close', html: icon('close'), onclick: closeSettings }));
    var body = $('manageBody'); body.textContent = '';
    if (owner || r === 'admin') {
      body.appendChild(opt('face', 'Edit the face', owner
        ? 'Its page and its look, in the Face editor. This room takes its colours from it.'
        : 'Its page and its look, in the Face editor.',
        function () { closeSettings(); editFace(site); }));
    }
    SECTIONS.forEach(function (sec) {
      if (mayLink(site, sec)) body.appendChild(opt(sec.icon, 'New ' + singular(sec), null, function () { closeSettings(); newIn(site, sec); }));
    });
    body.appendChild(opt('members', 'Members', (site.members || []).length + ' in it' + (owner ? '. Roles are yours to give.' : ''),
      function () { closeSettings(); if (narrowNow()) $('app').classList.add('roster'); else if (R.rf) fold('r'); }));
    if (owner) body.appendChild(opt('person', 'Add by contact code', null,
      function () { closeSettings(); addByCode(site); }));
    if (!owner) body.appendChild(el('p', { class: 'mnote', text: 'The owner changes this community: its face, its rooms, and who holds which role.' }));
    $('manage').hidden = false;
    $('siteRole').setAttribute('aria-expanded', 'true');
    var first = body.querySelector('.opt:not(:disabled)'); if (first) first.focus();
  }
  function closeSettings() { $('manage').hidden = true; $('siteRole').setAttribute('aria-expanded', 'false'); }

  /* ── THE FACE EDITOR, wallflowers.io/signup's own: the website's assets/face/face.js and
     its stylesheets, loaded here once they are first wanted, the Site's face put into it
     (Face.load) and what it makes written back through the Door: group.setFace on the Site,
     and its pictures as the Host's media (host.setMedia), never as words in the face. */
  var FACE = { css: ['assets/face/face.css', 'assets/face/vendor/coloris/coloris.min.css', 'assets/face/face-editor.css'],
               js: ['assets/face/vendor/coloris/coloris.min.js', 'assets/face/vendor/qrcode/qrcode.js', 'assets/face/face.js'] };
  var faceReady = null, faceMounted = false;
  function faceEditor() {
    if (faceReady) return faceReady;
    window.WallFlowers = window.WallFlowers || {};
    window.WallFlowers.FaceIcd = DOOR + '/v2/icd';
    FACE.css.forEach(function (href) { document.head.appendChild(el('link', { rel: 'stylesheet', href: href })); });
    faceReady = FACE.js.reduce(function (p, src) {
      return p.then(function () {
        return new Promise(function (ok, no) {
          var s = el('script', { src: src });
          s.onload = ok; s.onerror = function () { no(new Error('the Face editor could not load ' + src)); };
          document.body.appendChild(s);
        });
      });
    }, Promise.resolve()).then(function () {
      var F = window.WallFlowers.Face;
      if (!F || !F.load) throw new Error('this Face editor cannot open an existing face (it has no Face.load)');
      return F;
    });
    faceReady.catch(function () { faceReady = null; });
    return faceReady;
  }
  function editFace(site) {
    var owner = roleIn(site) === 'owner';
    $('feTitle').textContent = 'The face of ' + titleOf(site);
    $('feSub').textContent = 'What a visitor meets at its address, and the colours this room is made from.';
    $('feWhy').textContent = '';
    $('faceEd').hidden = false;
    busy($('feSave'), true);
    faceEditor().then(function (F) {
      if (!faceMounted) { F.mount($('feMount')); faceMounted = true; }
      F.site({ name: titleOf(site), purpose: '', slug: '', kind: '' });
      F.load(site.view && site.view.face, hostMedia(site));
      busy($('feSave'), false);
      $('feSave').onclick = function () { saveFace(site, F); };
    }, function (e) {
      $('feWhy').textContent = String(e && e.message || e);
      busy($('feSave'), false);
      $('feSave').disabled = true;   // no editor, nothing to save; Cancel closes
    });
  }
  function closeFace() { $('faceEd').hidden = true; }
  function saveFace(site, F) {
    var doc = F.document(), pics = F.media(), had = hostMedia(site);
    doc.mark = '';
    if (doc.header) { doc.header.logo = ''; doc.header.banner = ''; }
    if (doc.look && doc.look.wallpaper) doc.look.wallpaper.image = '';
    var face = JSON.stringify(doc), why = faceRefusal(face);
    if (why) { $('feWhy').textContent = 'The face cannot be set: ' + why; return; }
    /* ICD 2.1.0 (BW-B, 29 Sep): the owner and the Site's admins alike write the face with
       group.editFace and its pictures with host.editMedia, both commutative; an empty
       picture clears its slot. group.setFace and host.setMedia stay only for old Sites. */
    var steps = [{ do: 'apply', object: site.id, op: 'group.editFace', args: { face: face } }], left = [];
    var host = partsIn(site, 'host')[0], t0 = T.begin();
    ['mark', 'cover', 'logo', 'wallpaper'].forEach(function (slot) {
      var u = pics[slot];
      if (u === had[slot]) return;
      if (!u) { if (host && had[slot]) steps.push({ do: 'apply', object: host.part, op: 'host.editMedia', args: { slot: slot, media: '', mediaMime: '' } }); return; }
      var m = MEDIA.exec(u);
      if (host && m && m[2].length <= MEDIA_MAX) steps.push({ do: 'apply', object: host.part, op: 'host.editMedia', args: { slot: slot, media: m[2], mediaMime: m[1] } });
      else left.push(slot);
    });
    busy($('feSave'), true);
    $('feWhy').textContent = '';
    batch(steps).then(function (r) {
      if (r.refused) throw new Error(r.refused.why);
      if (left.length) NOTED.push({ kind: 'face', id: site.id, why: 'not carried: ' + left.join(', ') + ' (a picture over ' + MEDIA_MAX + ' bytes, or not PNG, JPEG or WebP)' });
      closeFace();
      return load().then(function () { T.shown('face', t0); });
    }).catch(function (e) { $('feWhy').textContent = String(e && e.message || e); })
      .then(function () { busy($('feSave'), false); });
  }

  /* ── THE TABS: the boards, in the deck's words, and the kind each lists ── */
  var BOARDS = window.Kenjin && window.Kenjin.Boards;
  function boardName(b, fallback) { return BOARDS ? BOARDS.NAMES[b] : fallback; }
  var TABS = [
    { id: 'feed',     board: 'communities',  label: 'Feed' },
    { id: 'forum',    board: 'forums',       label: 'Rooms',  kind: 'forum' },   // Ralph, 28 Sep: "just Rooms" in the top bar
    { id: 'event',    board: 'events',       label: boardName('events', 'Events'),      kind: 'event' },
    { id: 'post',     board: 'publications', label: 'Resources', kind: 'post' },   // Ralph, 28 Sep: Articles are Resources
    { id: 'thing',    board: 'marketplace',  label: boardName('marketplace', 'Trade'),  kind: 'thing' },
    { id: 'treasury', board: 'treasury',     label: boardName('treasury', 'Treasury'),  kind: 'treasury' }
  ];
  function tabOf(id) { return TABS.filter(function (t) { return t.id === id; })[0] || TABS[0]; }
  function tabOfKind(kind) { return TABS.filter(function (t) { return t.kind === kind; })[0] || TABS[0]; }

  function inScope(o) { return !S.site || M.host[o.id] === S.site || M.origin[o.id] === S.site; }
  function scopeSites() { return S.site ? [M.byId[S.site]] : M.sites; }
  function ofKind(kind) { return M.feed.filter(function (o) { return o.kind === kind && inScope(o) && matches(o); }); }
  function roomsOf(site) {
    return partsIn(site, 'room').map(function (p) { return M.byId[p.part] || { id: p.part, name: p.role, kind: 'forum', missing: true }; });
  }
  function lastTs(id) { var ms = msgs(id); return ms.length ? ms[ms.length - 1].ts || 0 : 0; }

  function open(view, sel, site) {
    S.view = view; S.sel = sel || null;
    if (site !== undefined) S.site = site;
    var o = sel && M && M.byId[sel];
    if (view === 'dm') S.tab = 'forum';
    else if (o && (view === 'channel' || view === 'object' || view === 'event-edit')) S.tab = tabOfKind(o.kind).id;
    $('app').classList.remove('drawer', 'roster');
    if (MEETING) meet(false, { quiet: true });
    $('refused').hidden = true;
    render();
    $('feed').scrollTop = view === 'channel' || view === 'dm' ? 1e9 : 0;
  }
  function tab(id) { S.tab = id; open('feed', null); }

  /* THE THEME IS THE COMMUNITY'S FACE, made quieter: its look's colours become a palette
     of many shades (palette.js) written on #app in the properties the webapp's own look
     uses, and its type is taken as it is (the Face editor's own fonts, look.js). Outside a
     community, or in one with no look, the Face editor's first theme, Paper. */
  var themed = [], PALETTES = {};
  function paletteOf(look) {
    var P = window.WallFlowers && window.WallFlowers.Palette;
    if (!P) return null;
    var key = look && typeof look === 'object' ? JSON.stringify([look.colours || null, look.wallpaper && look.wallpaper.colour2]) : '';
    if (!PALETTES[key]) {
      try { PALETTES[key] = P.from(key ? look : null); }
      catch (e) { console.warn('this community\'s Face has colours this build cannot read — Paper is used', e); PALETTES[key] = P.from(null); }
    }
    return PALETTES[key];
  }
  function theme(site) {
    var app = $('app'), look = site && site.view && site.view.face && site.view.face.look;
    var L = window.WallFlowers && window.WallFlowers.Look, P = window.WallFlowers && window.WallFlowers.Palette;
    themed.forEach(function (k) { app.style.removeProperty(k); });
    themed = [];
    app.classList.remove('faced', 'dark');
    if (!P) return;
    function set(k, v) { app.style.setProperty(k, v); themed.push(k); }
    var key = look && typeof look === 'object' ? 'x' : '', p = paletteOf(key ? look : null);
    Object.keys(p.vars).forEach(function (k) { set(k, p.vars[k]); });
    app.classList.toggle('dark', p.dark);
    if (key && L) {
      var tx = look.text || {};
      set('--font', L.fontOf(tx.font).stack);
      set('--name-font', L.fontOf(tx.title || tx.font).stack);
      app.classList.add('faced');
    }
    var meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute('content', p.vars['--s1']);
  }

  /* Which room a Chat Rooms tab opens on: the one spoken in last. */
  function settle() {
    if (S.site && !M.byId[S.site]) S.site = null;
    if ((S.view === 'channel' || S.view === 'object' || (S.view === 'event-edit' && S.sel)) && !M.byId[S.sel]) { S.view = 'feed'; S.sel = null; }
    if (S.tab === 'forum' && S.view === 'feed') {
      var best = null;
      scopeSites().forEach(function (s) {
        roomsOf(s).forEach(function (r) { if (!r.missing && (!best || lastTs(r.id) > lastTs(best.id))) best = r; });
      });
      if (best) { S.view = 'channel'; S.sel = best.id; }
    }
  }

  function render() {
    if (!M) return;
    var t = T.begin();
    settle();
    theme(S.site && M.byId[S.site]);
    rails();
    drawTop(); drawLeft(); drawPane(); drawPeople();
    /* The kept widths land without gliding; only a deliberate change animates (Pacific's guard). */
    var app = $('app');
    if (app.classList.contains('still')) requestAnimationFrame(function () { requestAnimationFrame(function () { app.classList.remove('still'); }); });
    T.end('render', t);
  }

  /* ── the top bar ───────────────────────────────────────────────────────── */
  function drawTop() {
    var site = S.site && M.byId[S.site], b = $('siteBtn');
    b.textContent = '';
    b.appendChild(markOf(site));
    b.appendChild(el('span', { class: 'nm', text: site ? titleOf(site) : 'WallFlowers' }));
    b.appendChild(ic('chevD')).classList.add('chev');
    var role = $('siteRole'), badge = roleBadge(site);
    role.textContent = '';
    if (badge) { role.appendChild(badge); role.setAttribute('aria-label', badge.getAttribute('aria-label') + ': this community\'s settings'); }
    role.hidden = !badge;
    b.setAttribute('aria-label', (site ? titleOf(site) : 'All communities') + ': switch community');
    var tabs = $('tabs'); tabs.textContent = '';
    TABS.forEach(function (t) {
      tabs.appendChild(el('button', { class: 'tab', type: 'button', 'aria-current': S.tab === t.id ? 'page' : null,
        text: t.label, onclick: function () { tab(t.id); } }));
    });
    placeMeet();
    var me = $('meBtn'), own = iconOf(M.me.pk);
    me.style.setProperty('--t', hue(M.me.pk));
    me.textContent = own ? '' : (M.me.name || '').slice(0, 1).toUpperCase();
    if (own) me.appendChild(el('img', { src: own, alt: '' }));
    me.title = M.me.name || 'You';
  }

  /* THE MENUS: one open at a time; a click outside or Escape closes it. */
  var MENUS = [['siteBtn', 'siteMenu', fillSites], ['meBtn', 'meMenu', fillMe]];
  function closeMenus() {
    MENUS.forEach(function (m) { $(m[1]).hidden = true; $(m[0]).setAttribute('aria-expanded', 'false'); });
  }
  function toggleMenu(i) {
    var m = MENUS[i], was = !$(m[1]).hidden;
    closeMenus();
    if (was || !M) return;
    var box = $(m[1]); box.textContent = '';
    m[2](box);
    box.hidden = false;
    $(m[0]).setAttribute('aria-expanded', 'true');
    var first = box.querySelector('.mi'); if (first) first.focus();
  }
  function mi(kids, go, cls) {
    return el('button', { class: 'mi' + (cls ? ' ' + cls : ''), type: 'button', role: 'menuitem',
      onclick: function () { closeMenus(); go(); } }, kids);
  }
  function fillSites(box) {
    box.appendChild(mi([markOf(null), el('span', { class: 'grow', text: 'All communities' })],
      function () { open('feed', null, null); }, S.site ? '' : 'on'));
    if (M.sites.length) box.appendChild(el('div', { class: 'sep' }));
    M.sites.forEach(function (s) {
      box.appendChild(mi([markOf(s), el('span', { class: 'grow nmrole' }, [el('span', { class: 'nmt', text: titleOf(s) }), roleBadge(s)]),
        el('span', { class: 'ct', text: String((s.members || []).length) })],
        function () { open('feed', null, s.id); }, S.site === s.id ? 'on' : ''));
    });
  }
  function fillMe(box) {
    var b = el('span', { class: 'meBtn', style: '--t:' + hue(M.me.pk), text: (M.me.name || '').slice(0, 1).toUpperCase() });
    var own = iconOf(M.me.pk); if (own) { b.textContent = ''; b.appendChild(el('img', { src: own, alt: '' })); }
    box.appendChild(el('div', { class: 'who' }, [b, el('div', {}, [el('b', { text: M.me.name || 'You' }),
      el('span', { text: M.sites.length + (M.sites.length === 1 ? ' community · ' : ' communities · ') + M.conns.length + ' connections' })])]));
    box.appendChild(el('div', { class: 'sep' }));
    box.appendChild(mi([ic('person'), el('span', { class: 'grow', text: 'Your card' })], function () { yourCard(false); }));
    box.appendChild(mi([ic('out'), el('span', { class: 'grow', text: 'Sign out' })], signOut, 'out'));
  }
  function singular(sec) {
    return { forum: 'room', post: 'resource', event: 'event', thing: 'listing' }[sec.kind] || sec.label;
  }


  /* ── MEET: the search (Ralph, 28 Sep). Its glyph sits where the hamburger was; opened,
     it slides to the middle of the top bar and widens into a bar across the centre
     column, the tabs fading under it, while the profile glides to the middle of the right
     rail's column. Narrow, it takes the top row beside the profile. Everything it finds is
     what the pane, the index and the people already hold: `matches`, nothing fetched. */
  var MEETING = false, meetT = null;
  function narrowNow() { return window.matchMedia('(max-width:760px)').matches; }
  /* THE SITE'S NAME AND YOUR ROLE: the role sits at the left rail's inner edge; a longer
     name pushes it on over the centre, and only a name that would bring it against the
     first tab ("Feed") is shortened (Ralph, 28 Sep). */
  function placeBrand() {
    var brand = $('brand'), b = brand.getBoundingClientRect(), stop;
    if (!b.width && !b.height) return;
    if (narrowNow()) stop = $('meBtn').getBoundingClientRect().left - 8;
    else { var first = $('tabs').firstElementChild; stop = first ? first.getBoundingClientRect().left - 6 : b.right; }
    brand.style.maxWidth = Math.max(0, stop - b.left) + 'px';
  }
  /* Where Meet is: a circle in the slot, closed; a bar across the centre column, open. The
     element covers both, and both shapes are set as clip-paths and glyph offsets on it, so
     opening and closing only swap which applies: nothing is laid out while it moves. */
  function placeMeet() {
    placeBrand();
    var top = $('top').getBoundingClientRect(), slot = $('meetSlot').getBoundingClientRect(), m = $('meet'), me = $('meBtn');
    if (!top.width) return;
    var c = { l: slot.left - top.left, t: slot.top - top.top, w: slot.width, h: slot.height };
    var h = 42, o = { t: c.t + (c.h - h) / 2, h: h };
    if (narrowNow()) {
      /* A phone: the whole top row; the Site's name fades and the profile slides off to the
         right (Ralph, 28 Sep). */
      var meLeft = $('util').getBoundingClientRect().left + me.offsetLeft;
      me.style.transform = MEETING ? 'translateX(' + (top.right - meLeft + 4) + 'px)' : '';
      o.l = 10; o.w = top.width - 2 * o.l;
    } else {
      var mid = $('tabs').getBoundingClientRect(), from = Math.max(mid.left, $('brand').getBoundingClientRect().right), PAD = 20;
      o.l = from - top.left + PAD; o.w = mid.right - from - 2 * PAD;
      var util = $('util');
      me.style.transform = MEETING ? 'translateX(' + (util.clientWidth / 2 - (me.offsetLeft + me.offsetWidth / 2)) + 'px)' : '';
    }
    o.w = Math.max(120, o.w);
    var u = { l: Math.min(c.l, o.l), t: Math.min(c.t, o.t) };
    u.r = Math.max(c.l + c.w, o.l + o.w); u.b = Math.max(c.t + c.h, o.t + o.h);
    function inset(b, d, r) {
      return 'inset(' + [b.t - u.t + d, u.r - (b.l + b.w) + d, u.b - (b.t + b.h) + d, b.l - u.l + d].map(function (x) { return x.toFixed(2) + 'px'; }).join(' ') +
             ' round ' + Math.max(0, r - d) + 'px)';
    }
    var s = m.style;
    s.left = u.l + 'px'; s.top = u.t + 'px'; s.width = (u.r - u.l) + 'px'; s.height = (u.b - u.t) + 'px';
    s.setProperty('--c0', inset(c, 0, c.h / 2)); s.setProperty('--f0', inset(c, 1, c.h / 2));
    s.setProperty('--c1', inset(o, 0, 14)); s.setProperty('--f1', inset(o, 1, 14));
    s.setProperty('--gx0', (c.l - u.l + c.w / 2 - 17) + 'px'); s.setProperty('--gy0', (c.t - u.t + c.h / 2 - 17) + 'px');
    s.setProperty('--gx1', (o.l - u.l + o.w / 2 - 17) + 'px'); s.setProperty('--gy1', (o.t - u.t + o.h / 2 - 17) + 'px');
    s.setProperty('--ox', (o.l - u.l) + 'px'); s.setProperty('--oy', (o.t - u.t) + 'px');
    s.setProperty('--ow', o.w + 'px'); s.setProperty('--oh', o.h + 'px');
  }
  function filter() { if (M) { drawLeft(); drawPane(); drawPeople(); } }
  /* THE GLYPH, drawn so it can turn: a ring, and four legs each held to it at its own angle
     and rotated about its centre, so a leg never leaves the ring. Closed they stand as Ralph
     drew them, 26° off the vertical (NNE, NNW, SSE, SSW); open they swing round to 26° off
     the horizontal (ENE, WNW, ESE, WSW): the same glyph, on its side. */
  var LEGS = [[26, 64], [-26, -64], [154, 116], [-154, -116]];
  function glyph() {
    var legs = LEGS.map(function (a) {
      return '<g class="leg" style="--from:' + a[0] + 'deg;--to:' + a[1] + 'deg"><path d="M12 6.8V1.9"/></g>';
    }).join('');
    return '<svg class="glyph" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" aria-hidden="true">' +
      '<circle cx="12" cy="12" r="5.2"/>' + legs + '</svg>';
  }
  /* Open: the box grows round the glyph, which stays at its centre; nothing is typed and
     nothing is focused until the box is pressed, which hides the glyph. Any press outside,
     or leaving it by keyboard, closes it and the whole of it runs backwards. */
  function meet(on, opts) {
    opts = opts || {};
    if (on === MEETING) { if (on && opts.focus) $('q').focus({ preventScroll: true }); return; }
    MEETING = on;
    var m = $('meet');
    m.classList.add('anim');
    m.classList.toggle('open', on);
    if (!on) m.classList.remove('typing');
    $('top').classList.toggle('meeting', on);
    $('meetGo').setAttribute('aria-expanded', String(on));
    $('meetGo').tabIndex = on ? -1 : 0;
    $('q').tabIndex = on ? 0 : -1;
    closeMenus();
    placeMeet();
    clearTimeout(meetT);
    meetT = setTimeout(function () { m.classList.remove('anim'); }, 560);
    if (on && opts.focus) $('q').focus({ preventScroll: true });
    if (!on) {
      if (document.activeElement === $('q')) $('q').blur();
      if ($('q').value) { $('q').value = ''; if (!opts.quiet) filter(); }
    }
  }

  /* ── THE RAILS: Pacific's. Drag the inner edge (200px to 30% of the window);
     click it and it matches the other rail; fold it to 48px. Kept in this browser. */
  var MIN_W = 200, FOLDED = 48, KEY = 'wallflowers.rails';
  var R = { lw: 256, rw: 256, lf: false, rf: false };
  try { var kept = JSON.parse(localStorage.getItem(KEY) || 'null'); if (kept) Object.keys(R).forEach(function (k) { if (typeof kept[k] === typeof R[k]) R[k] = kept[k]; }); } catch (e) { /* none kept */ }
  function keep() { try { localStorage.setItem(KEY, JSON.stringify(R)); } catch (e) { /* not kept */ } }
  function clampW(w) { return Math.min(Math.max(MIN_W, Math.floor(window.innerWidth * 0.3)), Math.max(MIN_W, Math.round(w))); }
  function rails() {
    var app = $('app');
    R.lw = clampW(R.lw); R.rw = clampW(R.rw);
    app.style.setProperty('--lw', (R.lf ? FOLDED : R.lw) + 'px');
    app.style.setProperty('--rw', (R.rf ? FOLDED : R.rw) + 'px');
    app.style.setProperty('--rwh', R.rf ? 'max-content' : R.rw + 'px');
    $('lrail').classList.toggle('folded', R.lf);
    $('people').classList.toggle('folded', R.rf);
    $('lFold').innerHTML = icon(R.lf ? 'chevR' : 'chevL');
    $('rFold').innerHTML = icon(R.rf ? 'chevL' : 'chevR');
    $('lFold').setAttribute('aria-label', R.lf ? 'Show the index' : 'Hide the index');
    $('rFold').setAttribute('aria-label', R.rf ? 'Show people' : 'Hide people');
    placeMeet();
  }
  function fold(side) { R[side + 'f'] = !R[side + 'f']; keep(); rails(); }
  function grip(side) {
    var g = $(side + 'Grip'), other = side === 'l' ? 'r' : 'l';
    g.addEventListener('pointerdown', function (e) {
      if (e.button !== 0) return;
      e.preventDefault();
      var x0 = e.clientX, dragged = false, app = $('app');
      g.setPointerCapture(e.pointerId);
      function move(ev) {
        if (!dragged) {
          if (Math.abs(ev.clientX - x0) < 4) return;
          dragged = true;
          app.classList.add('resizing');
          document.body.style.cursor = 'ew-resize';
        }
        R[side + 'w'] = clampW(side === 'l' ? ev.clientX : window.innerWidth - ev.clientX);
        rails();
      }
      function up() {
        g.removeEventListener('pointermove', move);
        g.removeEventListener('pointerup', up);
        g.removeEventListener('pointercancel', up);
        app.classList.remove('resizing');
        document.body.style.cursor = '';
        if (!dragged) { R[other + 'w'] = R[side + 'w']; R[other + 'f'] = false; }   // a click: the other rail takes this one's width
        keep(); rails();
      }
      g.addEventListener('pointermove', move);
      g.addEventListener('pointerup', up);
      g.addEventListener('pointercancel', up);
    });
  }

  /* ── the left rail: the index of the tab ─────────────────────────────── */
  function railTitle(t) {
    var h = $('lTitle'); h.textContent = '';
    var mark = boardMark(t.board);
    if (mark.length) { h.appendChild(mark[0]); }
    h.appendChild(el('span', { text: t.label }));
  }
  function item(kids, on, go, label) {
    return el('button', { class: 'it' + (on ? ' on' : ''), type: 'button', onclick: go, 'aria-label': label || null }, kids);
  }
  function lines(t, s) {
    return el('span', { class: 'grow' }, [el('span', { class: 't', text: t }), s ? el('span', { class: 's', text: s }) : null]);
  }
  function drawLeft() {
    var t = tabOf(S.tab), body = $('lbody'), site = S.site && M.byId[S.site];
    railTitle(t);
    body.textContent = '';
    var sec = SECTIONS.filter(function (x) { return x.kind === t.kind; })[0];
    if (site && sec && mayLink(site, sec)) {
      body.appendChild(el('button', { class: 'newbtn', type: 'button', onclick: function () { newIn(site, sec); } },
        [ic('plus'), el('span', { text: 'New ' + singular(sec) })]));
    }
    ({ feed: leftFeed, forum: leftRooms, event: leftEvents, post: leftArticles, thing: leftTrade, treasury: leftTreasury })[t.id](body, site);
  }
  /* The Feed's index: what the feed holds, the rooms spoken in last first. Never another
     community: the Site is changed from the top left and nowhere else (Ralph, 28 Sep). */
  var KIND_ICON = { post: 'doc', event: 'cal', thing: 'tag', treasury: 'fund' };
  function leftFeed(body) {
    var items = M.feed.filter(function (o) { return o.kind !== 'host' && inScope(o) && matches(o); });
    var rooms = items.filter(function (o) { return o.kind === 'forum'; }).sort(function (a, b) { return lastTs(b.id) - lastTs(a.id); });
    rooms.concat(items.filter(function (o) { return o.kind !== 'forum'; })).forEach(function (o) {
      var v = o.view || {}, ms = msgs(o.id), last = ms[ms.length - 1], s = siteOf(o);
      var sub = o.kind === 'forum' ? (last ? who(last.author) + ': ' + last.text : '')
        : o.kind === 'event' ? [evDates(o), v.venue].filter(Boolean).join(' · ')
        : o.kind === 'thing' ? [v.posture, v.price].filter(Boolean).join(' · ')
        : o.kind === 'treasury' ? money(v)
        : !S.site && s ? titleOf(s) : tabOfKind(o.kind).label;
      body.appendChild(item([o.kind === 'forum' ? el('span', { class: 'hash', text: '#' }) : ic(KIND_ICON[o.kind] || 'feed'),
        lines(o.kind === 'forum' ? o.name || 'room' : titleOf(o), sub), last ? el('span', { class: 'ct', text: when(last.ts) }) : null],
        S.sel === o.id, function () { open(o.kind === 'forum' ? 'channel' : 'object', o.id); }));
    });
    if (!items.length) body.appendChild(el('div', { class: 'none', text: 'Nothing here yet.' }));
  }
  function leftRooms(body) {
    var sites = scopeSites().filter(function (s) { return roomsOf(s).length; });
    sites.forEach(function (s) {
      if (!S.site) body.appendChild(el('div', { class: 'sec', text: titleOf(s) }));
      roomsOf(s).forEach(function (r) {
        var ms = msgs(r.id), last = ms[ms.length - 1];
        body.appendChild(item([el('span', { class: 'hash', text: '#' }),
          lines(r.name || 'room', last ? who(last.author) + ': ' + last.text : ''),
          last ? el('span', { class: 'ct', text: when(last.ts) }) : null],
          S.view === 'channel' && S.sel === r.id, function () { if (!r.missing) open('channel', r.id); }));
      });
    });
    if (!sites.length) body.appendChild(el('div', { class: 'none', text: 'No rooms yet.' }));
    if (M.conns.length) {
      body.appendChild(el('div', { class: 'sec', text: 'Direct messages' }));
      M.conns.forEach(function (c) {
        var ms = msgs(M.chats[c.pk]), last = ms[ms.length - 1];
        body.appendChild(item([avatar(c.pk), lines(c.name || 'New member', last ? last.text : ''),
          last ? el('span', { class: 'ct', text: when(last.ts) }) : null],
          S.view === 'dm' && S.sel === c.pk, function () { open('dm', c.pk); }));
      });
    }
  }
  function dateBlock(ms) {
    var d = new Date(ms);
    return el('span', { class: 'date' }, [el('i', { text: d.toLocaleDateString('en-GB', { weekday: 'short' }) }),
      el('b', { text: String(d.getDate()) }), el('i', { text: d.toLocaleDateString('en-GB', { month: 'short' }) })]);
  }
  function startOf(o) { return (o.view && o.view.start_ms) || 0; }
  function endOf(o) { return (o.view && o.view.end_ms) || 0; }
  /* An event given by its dates alone is held as whole days, from midnight to 23:59 on its
     last day (event.setProfile has startMs and endMs, no all-day flag). It reads as its
     dates, a range over several days, and never as a time. */
  var DAY_FMT = { weekday: 'short', day: 'numeric', month: 'short' };
  function allDay(o) {
    var s = new Date(startOf(o)), e = new Date(endOf(o));
    return !!endOf(o) && s.getHours() + s.getMinutes() === 0 && e.getHours() === 23 && e.getMinutes() === 59;
  }
  function manyDays(o) { return allDay(o) && new Date(endOf(o)).toDateString() !== new Date(startOf(o)).toDateString(); }
  function evDates(o) {
    var a = new Date(startOf(o)).toLocaleDateString('en-GB', DAY_FMT);
    return manyDays(o) ? a + ' – ' + new Date(endOf(o)).toLocaleDateString('en-GB', DAY_FMT) : a;
  }
  function evWhen(o) {
    return allDay(o) ? evDates(o) : new Date(startOf(o)).toLocaleString('en-GB', { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
  }
  function leftEvents(body) {
    var now = Date.now(), all = ofKind('event').sort(function (a, b) { return startOf(a) - startOf(b); });
    var soon = all.filter(function (o) { return (endOf(o) || startOf(o) + 6 * 3600000) >= now; }), past = all.filter(function (o) { return soon.indexOf(o) < 0; });
    function row(o) {
      var v = o.view || {};
      var at = manyDays(o) ? 'to ' + new Date(endOf(o)).toLocaleDateString('en-GB', DAY_FMT)
        : allDay(o) ? '' : new Date(startOf(o)).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
      body.appendChild(item([dateBlock(startOf(o)), lines(titleOf(o), [v.venue, at].filter(Boolean).join(' · '))],
        S.sel === o.id, function () { open('object', o.id); }));
    }
    soon.forEach(row);
    if (past.length) { body.appendChild(el('div', { class: 'sec', text: 'Past' })); past.reverse().forEach(row); }
    if (!all.length) body.appendChild(el('div', { class: 'none', text: 'Nothing on yet.' }));
  }
  var RES_ICON = { pdf: 'doc', picture: 'image', video: 'image', link: 'link', article: 'doc' };
  function leftArticles(body) {
    var all = ofKind('post');
    all.forEach(function (o) {
      var s = siteOf(o), k = resKind(o);
      body.appendChild(item([ic(RES_ICON[k]), lines(titleOf(o), [KIND_WORD[k], !S.site && s ? titleOf(s) : ''].filter(Boolean).join(' · '))],
        S.sel === o.id, function () { open('object', o.id); if (k === 'pdf') viewPdf(o); }));
    });
    var pubs = publications();
    pubs.forEach(function (p) { body.appendChild(pubRow(p)); });
    if (!all.length && !pubs.length) body.appendChild(el('div', { class: 'none', text: 'No resources yet.' }));
  }
  function leftTrade(body) {
    var all = ofKind('thing');
    all.forEach(function (o) {
      var v = o.view || {};
      body.appendChild(item([v.posture ? el('span', { class: 'pill', text: v.posture }) : null,
        lines(titleOf(o), [v.price, v.area].filter(Boolean).join(' · '))], S.sel === o.id, function () { open('object', o.id); }));
    });
    if (!all.length) body.appendChild(el('div', { class: 'none', text: 'Nothing listed yet.' }));
  }
  function money(v) {
    if (!v || v.balance == null) return '';
    try { return new Intl.NumberFormat('en-GB', { style: 'currency', currency: v.currency || 'EUR', maximumFractionDigits: 0 }).format(v.balance); }
    catch (e) { return v.balance + ' ' + (v.currency || ''); }
  }
  function leftTreasury(body) {
    var all = ofKind('treasury');
    all.forEach(function (o) {
      var s = siteOf(o);
      body.appendChild(item([ic('fund'), lines(s ? titleOf(s) : titleOf(o), money(o.view))], S.sel === o.id, function () { open('object', o.id); }));
    });
    if (!all.length) body.appendChild(el('div', { class: 'none', text: S.site ? 'This community has no treasury yet.' : 'No treasuries yet.' }));
  }

  /* ── the right rail: who is here ─────────────────────────────────────── */
  function person(pk, sub, go) {
    return (go ? item : function (k) { return el('div', { class: 'it' }, k); })(
      [avatar(pk), lines(who(pk), sub)], go && S.view === 'dm' && S.sel === pk, go);
  }
  function drawPeople() {
    var body = $('rbody'), site = S.site && M.byId[S.site];
    body.textContent = '';
    if (site) {
      /* W-98 Members: the rail lists and counts as GET /v2/members does: an admitter (the Arc's node) is not a person, unless they are the owner. */
      var people = (site.members || []).filter(function (pk) { return pk === site.owner || !((site.view && site.view.roles) || []).some(function (r) { return r[0] === pk && r[1] === 'admitter'; }); });
      var ms = people.filter(function (pk) { return matches(who(pk)); });
      var rank = function (pk) { return pk === site.owner ? 0 : pk === M.me.pk ? 1 : named(pk) ? 2 : 3; };
      ms.sort(function (a, b) { return rank(a) - rank(b); });
      /* W-98 Members: with members.js loaded, the heading is the Members page's control. */
      var MP = window.WallFlowers && window.WallFlowers.Members, mh = 'Members — ' + people.length;
      body.appendChild(MP ? el('button', { class: 'sec go', type: 'button', 'aria-current': S.view === 'members' ? 'page' : null, text: mh,
        onclick: function () { open('members', null); } }) : el('div', { class: 'sec', text: mh }));
      ms.forEach(function (pk) {
        body.appendChild(person(pk, pk === site.owner ? 'owner' : pk === M.me.pk ? 'you' : '', function () { personCard(pk); }));
      });
    }
    body.appendChild(el('div', { class: 'sec', text: 'Connections — ' + M.conns.length }));
    M.conns.forEach(function (c) { body.appendChild(person(c.pk, '', function () { personCard(c.pk); })); });
    if (!M.conns.length) body.appendChild(el('div', { class: 'none', text: 'Nobody yet.' }));
  }

  /* ── the pane ──────────────────────────────────────────────────────────── */
  function head(title, sub) {
    var h = $('pttl'); h.textContent = '';
    h.appendChild(el('h2', { text: title }));
    if (sub) h.appendChild(el('p', { text: sub }));
  }
  function matches(x) {
    var q = $('q').value.trim().toLowerCase();
    return !q || JSON.stringify(x).toLowerCase().indexOf(q) >= 0;
  }
  function drawPane() {
    /* W-98 Members: members.js draws its page, and keeps it as it is while an editor is open. */
    var MP = window.WallFlowers && window.WallFlowers.Members, mp = S.view === 'members' && S.site && M.byId[S.site];
    if (MP && mp) return MP.draw($('feed'), mp, { door: door, el: el, icon: icon, iconUrl: iconUrl, role: ROLE, head: head, titleOf: titleOf,
      composeFor: composeFor, busy: busy, toast: toast, model: M, redraw: function () { if (M && S.view === 'members') drawPane(); } });
    if (S.view === 'members') S.view = 'feed';
    var f = $('feed'); f.textContent = '';
    var site = S.site && M.byId[S.site], t = tabOf(S.tab);
    composeFor(null);
    if (S.view === 'channel') return drawChannel(f, S.sel);
    if (S.view === 'dm') return drawChannel(f, M.chats[S.sel], S.sel);
    if (S.view === 'object') return drawObject(f, M.byId[S.sel]);
    if (S.view === 'event-edit' && eventsModule('EventsEdit')) { head(site ? titleOf(site) : ''); return eventsModule('EventsEdit').draw(f, S.sel, eventsCtx()); }
    var col = el('div', { class: 'col' }); f.appendChild(col);
    if (t.id === 'feed') head(site ? titleOf(site) : 'Everything', site ? count((site.members || []).length, 'member', 'members') : count(M.sites.length, 'community', 'communities'));
    else head(t.label, site ? titleOf(site) : 'All communities');
    if (t.id === 'feed') M.problems.forEach(function (o) {
      col.appendChild(el('div', { class: 'miss' }, [el('b', { text: (o.kind || 'object') + ' · ' + String(o.id).slice(0, 12) }),
        el('p', { text: o.why || '' })]));
    });
    var items = t.id === 'feed'
      ? M.feed.filter(function (o) { return o.kind !== 'host' && inScope(o) && matches(o); })
      : ofKind(t.kind);
    if (t.id === 'treasury' && items.length === 1) return drawObject(f, items[0]);
    if (t.id === 'thing' && TRADE) return drawTrade(col, items);
    if (t.id === 'post') return drawPublications(col, items);
    if (!items.length) { col.appendChild(el('div', { class: 'none', text: t.id === 'forum' ? 'No rooms yet.' : 'Nothing here yet.' })); return; }
    items.forEach(function (o) { col.appendChild(card(o)); });
  }

  /* W-98 RESOURCES: a post is written in the Door's editor (/v2/resources.js, registered as
     WallFlowers.Resources), over the page. "+" and a post's Edit open it; every write is the
     model's, through /v2/batch. Without it, "+" is the sheet. */
  var resEd = null;
  function resources() { var R = window.WallFlowers && window.WallFlowers.Resources; return R && R.mount ? R : null; }
  function newIn(site, sec) { if (sec.kind === 'post' && resources()) editResource(site, null); else sheet(site, sec); }
  function editResource(site, post) {
    var box = $('resEd'), saved = post || null;
    box.hidden = false;
    resEd = resources().mount(box, {
      fetch: function (path, init) {
        init = init || {};
        return fetch(DOOR + path, { method: init.method || 'GET', credentials: 'include',
          headers: init.body ? { 'content-type': 'application/json' } : {}, body: init.body })
          .then(function (r) { if (r.status === 401) { closeResource(); ended(); } return r; });
      },
      site: site.id, post: saved, icd: ICD,
      onSaved: function (r) { saved = r.post; load(); },
      onClose: function () { closeResource(); if (saved) load().then(function () { open('object', saved); }); }
    });
  }
  function closeResource() { if (resEd) resEd.close(); resEd = null; $('resEd').hidden = true; }

  /* THE PUBLICATIONS BOARD: a Site's posts are created objects their authors hold, not its
     members. What this member does not hold, the Site names (its `created` affiliations) and
     its Face carries (/v2/site/:site/items, the Arc's items and the slug): listed by title and
     excerpt, newest first, each opening its public page. Asked once a minute a Site. */
  var PUBS = {};
  function pubsOf(site) {
    var c = PUBS[site.id];
    if (!c || (c.r && Date.now() - c.t > 60000)) {
      c = PUBS[site.id] = { t: Date.now(), r: null };
      door('/v2/site/' + site.id + '/items').then(function (r) { c.r = r || {}; }, function () { c.r = {}; })
        .then(function () { if (M && S.tab === 'post') render(); });
    }
    var got = c.r || {}, items = got.items || {};
    if (!got.slug) return [];
    return ((site.view && site.view.affiliations) || []).filter(function (a) {
      var v = items['post:' + a.peer];
      return a.rel === 'created' && !M.byId[a.peer] && v && typeof v.title === 'string' && v.title.trim();
    }).map(function (a) {
      return { id: a.peer, site: site, view: items['post:' + a.peer], url: 'https://' + ADDRESS + got.slug + '/post/' + a.peer };
    });
  }
  function publications() {
    var out = [];
    scopeSites().forEach(function (s) { if (s) out = out.concat(pubsOf(s)); });
    return out.filter(function (p) { return matches(p.view); }).sort(function (a, b) { return (b.view.at || 0) - (a.view.at || 0); });
  }
  function pubExcerpt(v) { return v.excerpt || plain(v.body); }
  function openPub(p) { window.open(p.url, '_blank', 'noopener'); }
  function pubRow(p) {
    return item([ic(RES_ICON[resKind(p)]), lines(p.view.title, pubExcerpt(p.view))], false, function () { openPub(p); });
  }
  function pubCard(p) {
    var c = el('button', { class: 'card', type: 'button', onclick: function () { openPub(p); } }), meta = el('div', { class: 'meta' });
    meta.appendChild(markOf(p.site));
    meta.appendChild(el('span', { class: 'site', text: titleOf(p.site) }));
    meta.appendChild(el('span', { text: 'Resources · ' + KIND_WORD[resKind(p)] }));
    c.appendChild(meta);
    c.appendChild(el('h3', { text: p.view.title }));
    var ex = pubExcerpt(p.view);
    if (ex) c.appendChild(el('p', { class: 'ex', text: ex }));
    return c;
  }
  /* The Resources pane: the posts this member holds, as today, then the Site's others. */
  function drawPublications(col, held) {
    var pubs = publications();
    if (!held.length && !pubs.length) { col.appendChild(el('div', { class: 'none', text: 'Nothing here yet.' })); return; }
    held.forEach(function (o) { col.appendChild(card(o)); });
    pubs.forEach(function (p) { col.appendChild(pubCard(p)); });
  }

  /* ── RESOURCES: what a Post is, drawn by what it holds (post.setProfile's form, fold.rs).
     A PDF is form `pdf`, its link the document (ICD 2.1.0), or an older link post to a .pdf;
     the file store is unbuilt. A picture at full size is a link to the picture, as a Post's
     own banner is a few hundred KB of base64 at most. An article is words, and its banner. */
  var PDF = /\.pdf(?:[?#]|$)/i, PICTURE = /\.(?:jpe?g|png|webp|gif|avif)(?:[?#]|$)/i, VIDEO = /\.(?:mp4|webm|mov|m4v)(?:[?#]|$)/i;
  function resKind(o) {
    var v = o.view || {}, link = typeof v.link === 'string' ? v.link : '';
    if (v.form === 'pdf' || (v.form === 'link' && PDF.test(link))) return 'pdf';
    if (v.form === 'link' && PICTURE.test(link)) return 'picture';
    if (v.form === 'link' && VIDEO.test(link)) return 'video';
    if (v.form === 'image') return 'picture';
    return v.form === 'link' ? 'link' : 'article';
  }
  var KIND_WORD = { pdf: 'PDF', picture: 'Picture', video: 'Film', link: 'Link', article: 'Article' };
  /* A Post's banner is base64 with no type beside it: its first bytes say which. */
  function bannerOf(o) {
    var b = o.view && o.view.banner;
    if (!b || typeof b !== 'string') return null;
    var mime = /^\/9j\//.test(b) ? 'image/jpeg' : /^iVBOR/.test(b) ? 'image/png' : /^UklGR/.test(b) ? 'image/webp' : /^R0lGOD/.test(b) ? 'image/gif' : null;
    return mime ? 'data:' + mime + ';base64,' + b : null;
  }
  function pictureOf(o) { var full = webUrl(o.view.link); return resKind(o) === 'picture' && full && PICTURE.test(full) ? full : bannerOf(o); }
  function hostOf(u) { try { return new URL(u, location.href).host; } catch (e) { return ''; } }
  /* A member's link made a link only as a web address (http or https, made absolute), never
     `javascript:` or `data:`: core checks a post's link for length alone (SECURITY, 29 Sep). */
  function webUrl(u) { if (!u) return null; try { var x = new URL(u, location.href); return /^https?:$/.test(x.protocol) ? x.href : null; } catch (e) { return null; } }

  /* The words of an article, as plain text with the few marks the organisers' copy uses:
     "## " a heading, "- " a list item, **bold**, *italic*; a blank line between paragraphs.
     Built as nodes, never as HTML: the body is a member's writing. */
  function inline(text) {
    var out = [], re = /(\*\*[^*]+\*\*|\*[^*\s][^*]*\*)/g, at = 0, m;
    while ((m = re.exec(text))) {
      if (m.index > at) out.push(document.createTextNode(text.slice(at, m.index)));
      var s = m[0];
      out.push(s.slice(0, 2) === '**' ? el('strong', { text: s.slice(2, -2) }) : el('em', { text: s.slice(1, -1) }));
      at = m.index + s.length;
    }
    if (at < text.length) out.push(document.createTextNode(text.slice(at)));
    return out;
  }
  /* An article's words for a card's excerpt: prose()'s marks taken out. */
  function plain(body) {
    return String(body || '').replace(/^#{1,6}\s+/gm, '').replace(/^- /gm, '').replace(/\*\*([^*]+)\*\*|\*([^*]+)\*/g, '$1$2').replace(/\s+/g, ' ').trim();
  }
  /* A `markdown` body (post.setProfile's bodyFormat), as the editor reads it: elements, never
     HTML; its pictures the post's own assets. */
  function markdownOf(o) {
    var v = o.view, assets = {}, box = el('div', { class: 'prose' });
    (v.assets || []).forEach(function (a) { if (a.data) assets[a.id] = { src: 'data:' + a.mime + ';base64,' + a.data, width: a.width, height: a.height }; });
    box.appendChild(resources().markdown.render(v.body, document, { assets: assets, read: true }));
    return box;
  }
  function prose(body) {
    var box = el('div', { class: 'prose' }), list = null;
    String(body || '').split(/\n/).forEach(function (line) {
      var l = line.trim();
      if (!l) { list = null; return; }
      if (/^##\s+/.test(l)) { list = null; box.appendChild(el('h3', {}, inline(l.replace(/^##\s+/, '')))); return; }
      if (/^[-•]\s+/.test(l)) { if (!list) box.appendChild(list = el('ul')); list.appendChild(el('li', {}, inline(l.replace(/^[-•]\s+/, '')))); return; }
      list = null;
      box.appendChild(el('p', {}, inline(l)));
    });
    return box;
  }

  /* ── THE PDF VIEWER: every page, drawn by PDF.js (vendor/pdfjs, served with the webapp)
     onto canvases as they come into view, at the screen's own density; the same in Safari,
     on the phone and in Chrome. A PDF that will not load says why, and offers itself. */
  var pdfjs = null;
  function pdfLib() {
    if (!pdfjs) pdfjs = import(new URL('vendor/pdfjs/pdf.min.mjs', location.href).href).then(function (lib) {
      lib.GlobalWorkerOptions.workerSrc = new URL('vendor/pdfjs/pdf.worker.min.mjs', location.href).href;
      return lib;
    }, function (e) { pdfjs = null; throw e; });
    return pdfjs;
  }
  /* A post's PDF: the one it holds (post.setDocument), else the one it links. */
  function pdfSrc(o) {
    var d = o.view && o.view.document, src = { isEvalSupported: false };
    if (d && d.data) {
      var bin = atob(d.data), b = new Uint8Array(bin.length);
      for (var i = 0; i < bin.length; i++) b[i] = bin.charCodeAt(i);
      src.data = b;
    } else src.url = o.view.link;
    return src;
  }
  var viewing = null;
  function viewPdf(o) {
    var url = o.view.link, box = $('pdfPages'), zoom = 1;
    closePdf();
    $('pdfTitle').textContent = titleOf(o);
    $('pdfSub').textContent = 'Loading…';
    var open = webUrl(url);
    if (open) $('pdfOpen').href = open; else $('pdfOpen').removeAttribute('href');
    $('pdfOpen').hidden = !open;
    $('pdfv').hidden = false;
    box.textContent = '';
    var mine = viewing = { destroyed: false, task: null, io: null };
    pdfLib().then(function (lib) { mine.task = lib.getDocument(pdfSrc(o)); return mine.task.promise; }).then(function (doc) {
      if (mine.destroyed) return mine.task.destroy();
      mine.doc = doc;
      var held = o.view.document && o.view.document.data;
      $('pdfSub').textContent = doc.numPages + (doc.numPages === 1 ? ' page' : ' pages') + ' · ' + (held ? o.view.document.name || 'PDF' : hostOf(url));
      var pages = [];
      function width() { return Math.min(box.clientWidth - 32, 1100) * zoom; }
      mine.io = new IntersectionObserver(function (es) {
        es.forEach(function (e) { if (e.isIntersecting) draw(+e.target.dataset.n); });
      }, { root: box, rootMargin: '600px 0px' });
      function draw(n) {
        var slot = pages[n - 1];
        if (!slot || slot.drawn === width()) return;
        slot.drawn = width();
        doc.getPage(n).then(function (page) {
          if (mine.destroyed) return;
          var base = page.getViewport({ scale: 1 }), scale = slot.drawn / base.width, dpr = Math.min(window.devicePixelRatio || 1, 3);
          var vp = page.getViewport({ scale: scale * dpr }), c = slot.canvas;
          c.width = Math.floor(vp.width); c.height = Math.floor(vp.height);
          c.style.width = Math.floor(vp.width / dpr) + 'px'; c.style.height = Math.floor(vp.height / dpr) + 'px';
          page.render({ canvasContext: c.getContext('2d'), viewport: vp });
        });
      }
      function layout() {
        return doc.getPage(1).then(function (p1) {
          var r = p1.getViewport({ scale: 1 }), w = width();
          pages.forEach(function (s) {
            s.box.style.width = Math.floor(w) + 'px';
            s.box.style.height = Math.floor(w * r.height / r.width) + 'px';
            s.drawn = 0;
          });
          pages.forEach(function (s, i) { var b = s.box.getBoundingClientRect(), v = box.getBoundingClientRect(); if (b.bottom > v.top - 600 && b.top < v.bottom + 600) draw(i + 1); });
        });
      }
      for (var n = 1; n <= doc.numPages; n++) {
        var slot = { box: el('div', { class: 'pg', 'data-n': String(n) }), canvas: el('canvas'), drawn: 0 };
        slot.box.appendChild(slot.canvas);
        slot.box.appendChild(el('span', { class: 'pgn', text: n + ' / ' + doc.numPages }));
        box.appendChild(slot.box);
        pages.push(slot);
        mine.io.observe(slot.box);
      }
      mine.zoom = function (z) { zoom = Math.max(.5, Math.min(3, z === 0 ? 1 : zoom * z)); layout(); };
      mine.relayout = layout;
      return layout();
    }).catch(function (e) {
      if (mine.destroyed) return;
      $('pdfSub').textContent = 'This PDF could not be shown here (' + String(e && e.message || e) + '). Open it where it lives.';
    });
  }
  function closePdf() {
    /* PDF.js 6: the loading task, not the document, is what is torn down. */
    if (viewing) { viewing.destroyed = true; if (viewing.io) viewing.io.disconnect(); if (viewing.task) viewing.task.destroy().catch(function () {}); viewing = null; }
    $('pdfv').hidden = true;
    $('pdfPages').textContent = '';
  }
  function openResource(o) { if (resKind(o) === 'pdf') viewPdf(o); }
  /* A PDF's first page, small, for its card: drawn once it is on screen. */
  var thumbs = window.IntersectionObserver ? new IntersectionObserver(function (es) {
    es.forEach(function (e) {
      if (!e.isIntersecting) return;
      thumbs.unobserve(e.target);
      var c = e.target, o = M && M.byId[c.dataset.pdf];
      var task = null;
      if (!o) return;
      pdfLib().then(function (lib) { task = lib.getDocument(pdfSrc(o)); return task.promise; })
        .then(function (doc) { return doc.getPage(1).then(function (p) {
          var w = c.parentNode.clientWidth || 320, base = p.getViewport({ scale: 1 }), dpr = Math.min(window.devicePixelRatio || 1, 2);
          var vp = p.getViewport({ scale: w / base.width * dpr });
          c.width = vp.width; c.height = vp.height;
          return p.render({ canvasContext: c.getContext('2d'), viewport: vp }).promise.then(function () { c.parentNode.classList.add('drawn'); task.destroy(); });
        }); })
        .catch(function () { c.parentNode.classList.add('failed'); });
    });
  }, { rootMargin: '200px 0px' }) : null;
  function preview(o) {
    var k = resKind(o), pic = pictureOf(o), fig = el('div', { class: 'thumb ' + k });
    if (k === 'pdf') {
      var c = el('canvas', { 'data-pdf': o.id });
      fig.appendChild(c);
      fig.appendChild(el('span', { class: 'badge', text: 'PDF' }));
      if (thumbs) thumbs.observe(c);
    } else if (pic) {
      fig.appendChild(el('img', { src: pic, alt: '', loading: 'lazy', decoding: 'async' }));
    } else if (k === 'video') {
      if (webUrl(o.view.link)) fig.appendChild(el('video', { src: webUrl(o.view.link) + '#t=0.5', muted: '', playsinline: '', preload: 'metadata' }));
      fig.appendChild(el('span', { class: 'badge', text: 'FILM' }));
    } else return null;
    return fig;
  }

  /* ── TRADE (trade.js; W-98): the Site's board of listings, and a Thing's listing page. What
     trade.js reads of this page is this, and nothing else. */
  var TRADE = window.WallFlowers && window.WallFlowers.Trade;
  function tradeCtx() {
    return { M: M, icd: ICD, el: el, door: door, load: load, open: open, busy: busy, who: who,
             titleOf: titleOf, roleIn: roleIn, siteOf: siteOf, shrink: shrinkPhoto };
  }
  /* The board, then any Thing in scope that no listing names, as a card, so none goes missing. */
  function drawTrade(col, things) {
    var sites = scopeSites().filter(Boolean), listed = {};
    var n = TRADE.draw.board(tradeCtx(), col, sites);
    sites.forEach(function (s) { ((s.view && s.view.listings) || []).forEach(function (l) { listed[l.thingId] = true; }); });
    var rest = things.filter(function (o) { return !listed[o.id]; });
    rest.forEach(function (o) { col.appendChild(card(o)); });
    if (!n && !rest.length) col.appendChild(el('div', { class: 'none', text: 'Nothing here yet.' }));
  }
  function card(o) {
    var site = siteOf(o), v = o.view || {};
    var c = el('button', { class: 'card', type: 'button',
      onclick: function () { o.kind === 'forum' ? open('channel', o.id) : open('object', o.id); } });
    var meta = el('div', { class: 'meta' });
    if (site) { var m = markOf(site); meta.appendChild(m); meta.appendChild(el('span', { class: 'site', text: titleOf(site) })); }
    meta.appendChild(el('span', { text: o.kind === 'forum' ? '# room' : tabOfKind(o.kind).label }));
    c.appendChild(meta);
    if (o.kind === 'forum') {
      var ms = msgs(o.id), last = ms[ms.length - 1];
      c.appendChild(el('h3', { text: o.name || 'room' }));
      if (last) c.appendChild(el('div', { class: 'quote' }, [avatar(last.author),
        el('div', {}, [el('span', { class: 'who', text: who(last.author) }), el('p', { text: last.text })])]));
      c.appendChild(el('div', { class: 'foot' }, [el('span', { text: ms.length + (ms.length === 1 ? ' message' : ' messages') }),
        last ? el('span', { text: when(last.ts) }) : null]));
      return c;
    }
    if (o.kind === 'post') {
      var fig = preview(o);
      if (fig) { c.classList.add('res'); c.insertBefore(fig, c.firstChild); }
      meta.lastChild.textContent = 'Resources · ' + KIND_WORD[resKind(o)];
      if (resKind(o) === 'pdf') c.addEventListener('click', function () { viewPdf(o); });   // after the card's own open
    }
    c.appendChild(el('h3', { text: titleOf(o) }));
    var ex = o.kind === 'post' && v.excerpt ? v.excerpt : o.kind === 'post' && resKind(o) !== 'article' ? (v.body || hostOf(v.link)) : o.kind === 'event' ? [evWhen(o), v.venue].filter(Boolean).join(' · ')
      : o.kind === 'thing' ? [v.posture, v.price, v.area].filter(Boolean).join(' · ')
      : o.kind === 'treasury' ? money(v)
      : (o.kind === 'post' ? plain(v.body) : v.body) || v.descriptor || v.link || '';
    if (ex) c.appendChild(el('p', { class: 'ex', text: ex }));
    var com = commentsOf(o);
    if (com) { var n = msgs(com).length; c.appendChild(el('div', { class: 'foot', text: n + (n === 1 ? ' comment' : ' comments') })); }
    return c;
  }

  /* REPLIES AND REACTIONS, written to the webapp's spec (REQUIREMENTS.md § 1), which is
     what the forum reducer already reads (coordinator.rs FORUM_POST, FORUM_REACT): a reply
     names its parent by reply_author and reply_gen, a post carries its ts, a reaction names
     its message by target_author and target_gen and is set (active 1) or taken back (0).
     Core aligns the ICD to it (NC-9; Ralph, 28 Sep: "Do not cripple the build by waiting
     for the ICD"). Whether a reaction is one's own is the view's to say (§ 2): a third
     element on each [emoji, count], read wherever it is served. */
  var QUICK = ['❤️', '👍', '😂', '🔥', '🙏', '🌱'];
  var replying = null;   // {forum, author, gen, text}
  function mineOf(m, emoji) {
    return (m.reactions || []).some(function (x) { return x[0] === emoji && x[2] === true; });
  }
  function react(forum, m, emoji) {
    door('/v2/apply', { method: 'POST', body: { object: forum, op: 'forum.react',
      args: { target_author: m.author, target_gen: m.gen, emoji: emoji, active: mineOf(m, emoji) ? 0 : 1 } } })
      .then(load, function (e) { note(String(e && e.message || e)); });
  }
  /* TAKEN BACK (W-98): forum.retract, which the fold lets a message's author write
     (coordinator.rs FORUM_RETRACT). The Door adds its gen. */
  function retract(forum, m) {
    door('/v2/apply', { method: 'POST', body: { object: forum, op: 'forum.retract', args: { target_author: m.author, target_gen: m.gen } } })
      .then(load, function (e) { note(String(e && e.message || e)); });
  }
  /* THE EMOJI PICKER: categorised, many to choose from, the ones used here first. Above the
     React button on a wide screen, a sheet from the bottom on a phone. Pressing anywhere
     else closes it, and any message's opened controls with it (Ralph, 29 Sep). What was
     chosen lately is kept in this browser only. */
  var EMOJI = [
    ['Smileys', '😀', '😀 😃 😄 😁 😆 😅 🤣 😂 🙂 🙃 😉 😊 😇 🥰 😍 🤩 😘 😗 😚 😙 🥲 😋 😛 😜 🤪 😝 🤑 🤗 🤭 🤫 🤔 🫡 🤐 🤨 😐 😑 😶 🫥 😏 😒 🙄 😬 🤥 😌 😔 😪 🤤 😴 😷 🤒 🤕 🤢 🤮 🥵 🥶 🥴 😵 🤯 🤠 🥳 🥸 😎 🤓 🧐 😕 🫤 😟 🙁 😮 😯 😲 😳 🥺 🥹 😦 😧 😨 😰 😥 😢 😭 😱 😖 😣 😞 😓 😩 😫 🥱 😤 😡 😠 🤬 😈 👿 💀 👻 👽 🤖 💩 🙈 🙉 🙊'],
    ['People', '👋', '👋 🤚 🖐️ ✋ 🖖 🫱 🫲 👌 🤌 🤏 ✌️ 🤞 🫰 🤟 🤘 🤙 👈 👉 👆 👇 ☝️ 🫵 👍 👎 ✊ 👊 🤛 🤜 👏 🙌 🫶 👐 🤲 🤝 🙏 ✍️ 💅 💪 🦾 🧠 🫀 👀 👁️ 👂 👃 👄 🫦 🧒 🧑 🧓 👶 🧑‍🎤 🧑‍🎨 🧑‍🌾 🧑‍🍳 🧑‍🏫 🧑‍🔬 🧑‍💻 🧑‍🔧 🧘 🏃 💃 🕺 🧗 🚶 🧎 👯 🫂 👥'],
    ['Hearts', '❤️', '❤️ 🧡 💛 💚 💙 🩵 💜 🤎 🖤 🩶 🤍 🩷 💔 ❤️‍🔥 ❤️‍🩹 💕 💞 💓 💗 💖 💘 💝 💟 ❣️ 💌 💋 🫶 💯 💢 💥 💫 💦 💨 🕳️ 💬 🗨️ 💭 💤'],
    ['Nature', '🌱', '🌱 🌿 ☘️ 🍀 🌾 🌵 🌲 🌳 🌴 🪴 🍃 🍂 🍁 🌷 🌹 🥀 🌺 🌸 🌼 🌻 🪷 💐 🍄 🪸 🐚 🪨 🌍 🌏 🌎 🌙 🌕 🌑 ⭐ 🌟 ✨ ⚡ 🔥 🌈 ☀️ 🌤️ ⛅ 🌧️ ⛈️ ❄️ 🌊 💧 🐝 🦋 🐞 🐌 🐛 🪲 🐢 🐍 🦎 🐙 🐬 🐳 🐟 🐠 🦀 🐦 🕊️ 🦉 🦅 🐓 🦆 🐺 🦊 🐻 🐼 🐨 🐯 🦁 🐮 🐷 🐸 🐵 🐶 🐱 🐰 🦔 🦌 🐘'],
    ['Food', '🍲', '🍏 🍎 🍐 🍊 🍋 🍌 🍉 🍇 🍓 🫐 🍈 🍒 🍑 🥭 🍍 🥥 🥝 🍅 🍆 🥑 🥦 🥬 🥒 🌶️ 🫑 🌽 🥕 🧄 🧅 🥔 🍠 🥐 🍞 🥖 🧀 🥚 🍳 🥞 🧇 🥓 🍗 🌮 🌯 🥙 🧆 🥗 🍝 🍜 🍲 🍛 🍣 🍱 🥟 🍙 🍚 🍘 🍥 🥮 🍡 🍧 🍨 🍦 🥧 🧁 🍰 🎂 🍮 🍭 🍬 🍫 🍯 🥛 ☕ 🫖 🍵 🧉 🍶 🍺 🍻 🥂 🍷 🥃 🍸 🍹'],
    ['Activities', '🎉', '🎉 🎊 🎈 🎁 🎀 🪅 🎗️ 🏆 🥇 🥈 🥉 🏅 🎖️ ⚽ 🏀 🏐 🎾 🏓 🏸 🥊 🛹 🏄 🚴 🧘 🎯 🎲 🧩 ♟️ 🎨 🖌️ 🖍️ 🧵 🧶 🎭 🎬 🎤 🎧 🎼 🎵 🎶 🎹 🥁 🪘 🎷 🎺 🎸 🪕 🎻 🪗 📷 📸 🎥 📽️'],
    ['Places', '🏡', '🏡 🏠 🏘️ 🏚️ 🏗️ 🏢 🏛️ ⛪ 🕌 🛕 🕍 ⛩️ 🏰 🗼 🗽 ⛲ ⛺ 🏕️ 🏞️ 🏜️ 🏝️ 🏔️ ⛰️ 🌋 🗻 🌅 🌄 🌇 🌆 🌃 🌉 🎡 🎢 🚂 🚆 🚇 🚊 🚌 🚲 🛴 🛵 🚗 🚕 ⛵ 🛶 🚤 ⛴️ 🚢 ✈️ 🛫 🛬 🚀 🛸 🗺️ 🧭 📍'],
    ['Objects', '💡', '💡 🔦 🕯️ 🪔 📱 💻 ⌨️ 🖥️ 🖨️ 📡 🔋 🔌 💾 💿 📼 📺 📻 🎙️ ⏰ ⌛ ⏳ 📚 📖 📝 ✏️ 🖊️ 📎 📌 📐 ✂️ 🗂️ 📁 📅 📆 🗒️ 📋 📦 ✉️ 📨 📮 🏷️ 🔑 🗝️ 🔒 🔓 🔨 🪛 🔧 🪚 🧰 🪜 🧲 🧪 🔬 🔭 🩺 💊 🪴 🧺 🧹 🪣 🧼 🪞 🛋️ 🪑 🛏️ 🧸 🪆 🖼️ 💰 💳 💎 ⚖️ 🔔 🎐'],
    ['Symbols', '✅', '✅ ☑️ ✔️ ❌ ❎ ➕ ➖ ➗ ✖️ ♾️ ‼️ ⁉️ ❓ ❔ ❗ ❕ 〰️ ⚠️ 🚫 ⛔ ♻️ 🔁 🔄 ⏩ ⏪ ⬆️ ⬇️ ⬅️ ➡️ ↩️ ↪️ 🔝 🆕 🆗 🆒 🆙 🆓 🔴 🟠 🟡 🟢 🔵 🟣 🟤 ⚫ ⚪ 🟥 🟧 🟨 🟩 🟦 🟪 🔺 🔻 🔸 🔹 ☮️ ☯️ ✴️ ✳️ ❇️ 🌀 ♀️ ♂️ ⚧️ 🏳️‍🌈 🏳️‍⚧️ 🏴 🏳️']
  ].map(function (c) { return { name: c[0], tab: c[1], all: c[2].split(' ') }; });
  var RECENT_KEY = 'wallflowers.emoji.recent', pickingFor = null;
  function recent() {
    try { var r = JSON.parse(localStorage.getItem(RECENT_KEY) || '[]'); return Array.isArray(r) ? r.slice(0, 24) : []; } catch (e) { return []; }
  }
  function used(e) {
    try { localStorage.setItem(RECENT_KEY, JSON.stringify([e].concat(recent().filter(function (x) { return x !== e; })).slice(0, 24))); } catch (x) { /* not kept */ }
  }
  function picker(btn, forum, m) {
    var box = $('emo');
    if (!box.hidden && pickingFor === m) return closePicker();
    pickingFor = m;
    var cats = (recent().length ? [{ name: 'Used here', tab: '🕘', all: recent() }] : []).concat(
      [{ name: 'Quick', tab: '⭐', all: QUICK }]).concat(EMOJI);
    var tabs = el('div', { class: 'emo-tabs', role: 'tablist' }), grid = el('div', { class: 'emo-grid' });
    function show(i) {
      grid.textContent = '';
      tabs.querySelectorAll('button').forEach(function (b, j) { b.setAttribute('aria-selected', String(i === j)); });
      grid.appendChild(el('div', { class: 'emo-cat', text: cats[i].name }));
      var cells = el('div', { class: 'emo-cells' });
      cats[i].all.forEach(function (e) {
        if (!e) return;
        cells.appendChild(el('button', { type: 'button', class: mineOf(m, e) ? 'on' : null, text: e, 'aria-label': e,
          onclick: function () { used(e); closePicker(); closeActs(); react(forum, m, e); } }));
      });
      grid.appendChild(cells);
      grid.scrollTop = 0;
    }
    cats.forEach(function (c, i) {
      tabs.appendChild(el('button', { type: 'button', role: 'tab', title: c.name, 'aria-label': c.name, text: c.tab, onclick: function () { show(i); } }));
    });
    box.textContent = '';
    box.appendChild(tabs);
    box.appendChild(grid);
    box.hidden = false;
    show(0);
    if (narrowNow() || !btn) { box.classList.add('bottom'); box.style.left = box.style.top = ''; return; }
    box.classList.remove('bottom');
    var r = btn.getBoundingClientRect(), w = box.offsetWidth, h = box.offsetHeight;
    var x = Math.max(8, Math.min(window.innerWidth - w - 8, r.right - w));
    var y = r.top - h - 8 < 8 ? r.bottom + 8 : r.top - h - 8;
    box.style.left = x + 'px'; box.style.top = y + 'px';
  }
  function closePicker() { $('emo').hidden = true; pickingFor = null; }

  /* A MESSAGE'S MENU, as WhatsApp and Signal have it (Ralph, 29 Sep): hold a message on a
     phone (right-click on a desktop) and it lifts, the reactions in a row above it, Reply,
     Copy and Info below. On a desktop React and Reply are also on the row, on hover. */
  var HOLD_MS = 420, menuAt = 0;
  function holdable(node, fire) {
    var timer = null, x0 = 0, y0 = 0;
    function cancel() { if (timer) { clearTimeout(timer); timer = null; } }
    node.addEventListener('pointerdown', function (e) {
      if (e.pointerType !== 'touch' || e.target.closest('button,a,input')) return;
      x0 = e.clientX; y0 = e.clientY;
      timer = setTimeout(function () { timer = null; fire(); }, HOLD_MS);
    });
    node.addEventListener('pointermove', function (e) { if (timer && Math.hypot(e.clientX - x0, e.clientY - y0) > 10) cancel(); });
    node.addEventListener('pointerup', cancel);
    node.addEventListener('pointercancel', cancel);
    node.addEventListener('contextmenu', function (e) {
      if (e.target.closest('a,input')) return;
      e.preventDefault(); cancel(); fire();
    });
  }
  function messageMenu(forum, m, row, n) {
    closePicker(); closeActs();
    var box = $('mmenu'), r = row.getBoundingClientRect();
    box.textContent = '';
    row.classList.add('held');
    var col = el('div', { class: 'mm-col' });
    var er = el('div', { class: 'mm-emo', role: 'menu', 'aria-label': 'React' });
    QUICK.forEach(function (e) {
      er.appendChild(el('button', { type: 'button', role: 'menuitem', class: mineOf(m, e) ? 'on' : null, text: e, 'aria-label': 'React ' + e,
        onclick: function () { used(e); closeMenu(); react(forum, m, e); } }));
    });
    er.appendChild(el('button', { type: 'button', role: 'menuitem', class: 'more', 'aria-label': 'More reactions', html: icon('plus'),
      onclick: function () { closeMenu(); picker(null, forum, m); } }));
    var shown = row.cloneNode(true);
    shown.removeAttribute('id');
    shown.className = 'msg mm-msg';
    shown.style.removeProperty('--d');
    shown.querySelectorAll('.acts,.fold-t,.rail-d').forEach(function (x) { x.remove(); });
    var acts = el('div', { class: 'mm-acts', role: 'menu', 'aria-label': 'Message' });
    [['reply', 'Reply', function () { replyTo(forum, m); }],
     ['copy', 'Copy', function () { copyText(m.text); }],
     ['info', 'Info', function () { messageInfo(forum, m, n); }]].forEach(function (a) {
      acts.appendChild(el('button', { type: 'button', role: 'menuitem', onclick: function () { closeMenu(); a[2](); } },
        [el('span', { text: a[1] }), el('span', { html: icon(a[0]) }).firstChild]));
    });
    /* Your own message, asked once: a retracted message stays retracted. */
    if (m.author === M.me.pk) acts.appendChild(el('button', { type: 'button', role: 'menuitem', class: 'del', onclick: function () {
      acts.textContent = '';
      acts.appendChild(el('button', { type: 'button', role: 'menuitem', class: 'del', text: 'Delete for everyone', onclick: function () { closeMenu(); retract(forum, m); } }));
      acts.appendChild(el('button', { type: 'button', role: 'menuitem', text: 'Cancel', onclick: closeMenu }));
      acts.firstChild.focus();
    } }, [el('span', { text: 'Delete' }), el('span', { html: icon('trash') }).firstChild]));
    col.appendChild(er); col.appendChild(shown); col.appendChild(acts);
    box.appendChild(col);
    box.hidden = false;
    menuAt = Date.now();
    /* The message stays where it was when it can, the row above it and the menu below on screen. */
    var above = er.offsetHeight + 8, h = col.offsetHeight;
    col.style.top = Math.max(12, Math.min(window.innerHeight - h - 12, r.top - above)) + 'px';
    var first = er.querySelector('button'); if (first) first.focus({ preventScroll: true });
  }
  function closeMenu() {
    $('mmenu').hidden = true;
    document.querySelectorAll('.msg.held').forEach(function (x) { x.classList.remove('held'); });
  }
  function toast(text) {
    var t = $('toast');
    t.textContent = text; t.hidden = false;
    clearTimeout(toast.t); toast.t = setTimeout(function () { t.hidden = true; }, 1600);
  }
  function copyText(text) {
    (navigator.clipboard ? navigator.clipboard.writeText(text) : Promise.reject()).then(function () { toast('Copied'); }, function () { toast('Could not copy'); });
  }
  /* What is known of a message, from the view: who, when, what it answers, what answers it. */
  function messageInfo(forum, m, n) {
    var box = $('personBody'), byRef = {};
    msgs(forum).forEach(function (x) { byRef[mref(x)] = x; });
    var parent = m.reply_to && byRef[m.reply_to.author + ':' + m.reply_to.gen];
    box.textContent = '';
    var big = avatar(m.author); big.classList.add('big');
    box.appendChild(el('div', { class: 'pc-head' }, [big, el('div', { class: 'grow' }, [el('h3', { text: who(m.author) }), el('p', { text: 'Message' })])]));
    box.appendChild(el('p', { class: 'mi-text', text: m.text }));
    var kv = el('dl', { class: 'kv' });
    function row(k, v) { kv.appendChild(el('dt', { text: k })); kv.appendChild(el('dd', { text: v })); }
    row('Sent', m.ts ? new Date(m.ts).toLocaleString('en-GB', { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' }) : 'No time recorded');
    if (m.reply_to) row('In reply to', parent ? who(parent.author) + ': ' + parent.text : 'a message not held here');
    if (n && n.descendants) row('Replies', String(n.descendants));
    var rs = (m.reactions || []).filter(function (x) { return x[1] > 0; });
    if (rs.length) row('Reactions', rs.map(function (x) { return x[0] + ' ' + x[1] + (x[2] === true ? ' (yours)' : ''); }).join('   '));
    if (m.up || m.down) row('Votes', '▲ ' + (m.up || 0) + '   ▼ ' + (m.down || 0));
    box.appendChild(kv);
    box.appendChild(el('button', { class: 'pc-alt', type: 'button', text: 'About ' + who(m.author), onclick: function () { personCard(m.author); } }));
    $('person').hidden = false;
  }
  function closeActs() { document.querySelectorAll('.msg.on').forEach(function (n) { n.classList.remove('on'); }); }
  function replyTo(forum, m) {
    replying = { forum: forum, author: m.author, gen: m.gen, text: m.text };
    drawReplying();
    $('say').focus();
  }
  function drawReplying() {
    var bar = $('replying');
    bar.hidden = !replying || replying.forum !== composeTo;
    if (bar.hidden) return;
    bar.textContent = '';
    bar.appendChild(el('span', { class: 'rt' }, [el('b', { text: 'Replying to ' + who(replying.author) }), el('span', { text: replying.text })]));
    bar.appendChild(el('button', { class: 'tb', type: 'button', 'aria-label': 'Not a reply', html: icon('close'),
      onclick: function () { replying = null; drawReplying(); } }));
  }
  function note(text) { $('refused').textContent = text; $('refused').hidden = false; }
  /* THREADS, as on a subreddit (Ralph, 29 Sep): each reply under what it answers, not in
     the order it arrived. Core's own projection, ForumState::thread() (coordinator.rs
     ~690), done here the same way until the view serves it (REQUIREMENTS.md § 8): a
     reply's parent is its reply_to when that names another message held here, else it is
     a root; siblings in causal (gen, author) order; pre-order with depth and a count of
     descendants; a post caught in a parent cycle surfaces as a root; every message once. */
  function mref(m) { return m.author + ':' + m.gen; }
  function thread(ms) {
    var byRef = {}, kids = {}, roots = [], out = [], seen = {};
    ms.forEach(function (m) { byRef[mref(m)] = m; });
    function parentOf(m) {
      var r = m.reply_to, k = r && r.author + ':' + r.gen;
      return k && k !== mref(m) && byRef[k] ? k : null;
    }
    function causal(a, b) { return a.gen - b.gen || (a.author < b.author ? -1 : a.author > b.author ? 1 : 0); }
    ms.forEach(function (m) { var p = parentOf(m); if (p) (kids[p] = kids[p] || []).push(m); else roots.push(m); });
    roots.sort(causal);
    Object.keys(kids).forEach(function (k) { kids[k].sort(causal); });
    function walk(start) {
      var stack = [{ m: start, d: 0 }];
      while (stack.length) {
        var s = stack.pop();
        if (s.fin != null) { out[s.fin].descendants = out.length - s.fin - 1; continue; }
        var k = mref(s.m);
        if (seen[k]) continue;
        seen[k] = true;
        var i = out.length;
        out.push({ m: s.m, depth: s.d, descendants: 0, orphan: !!s.m.reply_to && !parentOf(s.m) });
        stack.push({ fin: i });
        (kids[k] || []).slice().reverse().forEach(function (c) { stack.push({ m: c, d: s.d + 1 }); });
      }
    }
    roots.forEach(walk);
    ms.slice().sort(causal).forEach(function (m) { if (!seen[mref(m)]) walk(m); });
    return out;
  }
  var folded = {};   // a thread folded shut, by its message's ref: this tab only
  function messageList(f, ms, forum) {
    var byRef = {}, shut = null;
    ms.forEach(function (m) { byRef[mref(m)] = m; });
    var q = $('q').value.trim();
    thread(ms).forEach(function (n) {
      var m = n.m;
      if (shut && n.depth > shut.depth) return;   // inside a folded thread
      shut = null;
      if (q && !matches(m)) return;
      if (folded[mref(m)] && n.descendants) shut = n;
      var r = (m.reactions || []).filter(function (x) { return x[1] > 0; });
      var parent = m.reply_to && byRef[m.reply_to.author + ':' + m.reply_to.gen];
      var row = el('div', { class: 'msg' + (n.depth ? ' reply' : ''), id: 'm-' + m.author.slice(0, 16) + '-' + m.gen,
        style: '--d:' + Math.min(n.depth, narrowNow() ? 4 : 8) });
      /* The thread's line down the left: a press folds or opens what hangs below. */
      for (var d = 0; d < Math.min(n.depth, narrowNow() ? 4 : 8); d++) row.appendChild(el('span', { class: 'rail-d', style: '--i:' + d }));
      var body = el('div', { class: 'mb' }), tog = null;
      if (n.orphan) {
        body.appendChild(el('button', { class: 'rq', type: 'button', onclick: function () {
          var at = parent && $('m-' + parent.author.slice(0, 16) + '-' + parent.gen);
          if (at) { at.scrollIntoView({ block: 'center', behavior: 'smooth' }); at.classList.add('lit'); setTimeout(function () { at.classList.remove('lit'); }, 1200); }
        } }, [el('span', { class: 'rq-who', text: parent ? who(parent.author) : 'A message' }),
              el('span', { class: 'rq-text', text: parent ? parent.text : 'not held here' })]));
      }
      if (n.descendants) {
        var k = mref(m), shutNow = !!folded[k];
        tog = el('button', { type: 'button', class: 'fold-t', text: shutNow ? '+ ' + n.descendants + (n.descendants === 1 ? ' reply' : ' replies') : '– hide ' + (n.descendants === 1 ? 'reply' : n.descendants + ' replies'),
          'aria-expanded': String(!shutNow), onclick: function () { folded[k] = !folded[k]; var at = $('feed').scrollTop; drawPane(); $('feed').scrollTop = at; } });
      }
      body.appendChild(el('div', {}, [el('button', { class: 'who', type: 'button', text: who(m.author), onclick: function () { personCard(m.author); } }),
        el('span', { class: 'when', text: when(m.ts) })]));
      body.appendChild(el('p', { text: m.text }));
      if (r.length || m.up || m.down) {
        var rx = el('div', { class: 'rx' });
        r.forEach(function (x) { rx.appendChild(el('button', { type: 'button', class: x[2] === true ? 'mine' : null, text: x[0] + ' ' + x[1],
          title: x[2] === true ? 'Take back ' + x[0] : 'React ' + x[0], onclick: function () { react(forum, m, x[0]); } })); });
        if (m.up) rx.appendChild(el('span', { class: 'votes', text: '▲ ' + m.up + (m.down ? ' ▼ ' + m.down : '') }));
        body.appendChild(rx);
      }
      if (forum) {
        /* Two controls, each its own (Ralph, 29 Sep): React opens the picker; Reply replies. */
        var acts = el('div', { class: 'acts' });
        acts.appendChild(el('button', { type: 'button', class: 'act react', title: 'React', 'aria-label': 'React', html: icon('smile'),
          onclick: function (e) { e.stopPropagation(); picker(e.currentTarget, forum, m); } }));
        acts.appendChild(el('button', { type: 'button', class: 'act reply', title: 'Reply', 'aria-label': 'Reply', html: icon('reply'),
          onclick: function (e) { e.stopPropagation(); closeActs(); replyTo(forum, m); } }));
        body.appendChild(acts);
      }
      if (tog) body.appendChild(tog);
      var av = avatar(m.author);
      row.appendChild(el('button', { class: 'avb', type: 'button', 'aria-label': who(m.author), onclick: function () { personCard(m.author); } }, [av]));
      row.appendChild(body);
      if (forum) holdable(row, function () { messageMenu(forum, m, row, n); });
      f.appendChild(row);
    });
  }

  function drawChannel(f, id, peer) {
    var o = id && M.byId[id], site = o && siteOf(o);
    var name = peer ? who(peer) : (o && o.name) || 'room';
    head(peer ? name : '# ' + name, peer ? 'Direct message' : site ? titleOf(site) + ' · ' + count((o.members || []).length, 'member', 'members') : '');
    var col = el('div', { class: 'col' }); f.appendChild(col);
    if (!o) { col.appendChild(el('div', { class: 'none', text: 'Nothing here yet.' })); return; }
    col.appendChild(el('div', { class: 'welcome' }, [peer ? avatar(peer) : el('div', { class: 'big', text: '#' }),
      el('h1', { text: peer ? name : name }),
      el('p', { text: peer ? 'The start of your conversation.' : 'The start of #' + name + '.' })]));
    if (!peer && window.WallFlowers && window.WallFlowers.Rooms) window.WallFlowers.Rooms.header(col, o, { door: door, load: load, el: el, op: opDecl, me: M.me.pk, note: note });
    messageList(col, msgs(id), id);
    composeFor(id, peer ? name : '#' + name);
  }

  /* An object's own view: what its lens folded, labelled by the view's own keys.
     A post's comments are its hosted forum, and the composer writes to it. */
  /* W-98 EVENTS (UX's seam, pdr/w98-integration.md): an event's editor and its page are their own
     modules (events-edit.js, events-page.js, over events-icd.js), each drawn by one hook, and
     eventsCtx() is all of the page they read. Without the modules the page is as it was. */
  function eventsModule(name) { return window.WallFlowers && window.WallFlowers[name]; }
  function eventsCtx() {
    return { door: door, batch: batch, load: load, open: open, el: el, toast: toast, M: M, S: S, icd: ICD };
  }
  function drawObject(f, o) {
    if (!o) return open('feed');
    var site = siteOf(o);
    head(titleOf(o), [tabOfKind(o.kind).label, site && titleOf(site)].filter(Boolean).join(' · '));
    if (o.kind === 'event' && eventsModule('EventsPage')) return eventsModule('EventsPage').draw(f, o, eventsCtx());
    var v = o.view || {}, wrap = el('div', { class: 'col obj' });
    if (o.kind === 'treasury') {
      wrap.appendChild(el('div', { class: 'fund' }, [el('div', { class: 'sub', text: 'Balance' }),
        el('div', { class: 'bal', text: money(v) || '—' }),
        el('div', { class: 'sub', text: (v.deposits || []).length + ' deposits · ' + (v.settlements || []).length + ' paid out' })]));
    } else if (o.kind === 'post') {
      var k = resKind(o), pic = pictureOf(o);
      wrap.classList.add('resource', k);
      var eSite = site || (S.site && M.byId[S.site]), eOp = ICD && opDecl('post.setProfile');
      if (resources() && eSite && eOp && (eOp.ego !== 'owner' || o.owner === M.me.pk))
        wrap.appendChild(el('button', { class: 'resedit', type: 'button', text: 'Edit', onclick: function () { editResource(eSite, o.id); } }));
      wrap.appendChild(el('span', { class: 'pill', text: KIND_WORD[k] }));
      wrap.appendChild(el('h1', { text: titleOf(o) }));
      if (k === 'pdf') {
        var cover = el('button', { class: 'pdfcover', type: 'button', onclick: function () { viewPdf(o); } });
        var fig = preview(o); if (fig) cover.appendChild(fig);
        cover.appendChild(el('span', { class: 'read', text: 'Read' }));
        wrap.appendChild(cover);
      } else if (k === 'video') {
        if (webUrl(o.view.link)) wrap.appendChild(el('video', { class: 'film', src: webUrl(o.view.link), controls: '', playsinline: '', preload: 'metadata' }));
      } else if (k === 'picture' && pic) {
        var big = el('img', { src: pic, alt: titleOf(o), decoding: 'async' });
        wrap.appendChild(webUrl(pic) ? el('a', { class: 'bigpic', href: webUrl(pic), target: '_blank', rel: 'noopener' }, [big]) : el('div', { class: 'bigpic' }, [big]));
      } else if (pic) wrap.appendChild(el('img', { class: 'banner', src: pic, alt: '' }));
      if (v.body) wrap.appendChild(v.body_format === 'markdown' && resources() ? markdownOf(o) : prose(v.body));
      var go = webUrl(v.link);
      if (go && k !== 'pdf') wrap.appendChild(el('a', { class: 'golink', href: go, target: '_blank', rel: 'noopener', text: 'Open at ' + hostOf(go) }));
      if (go && k === 'pdf') wrap.appendChild(el('a', { class: 'golink', href: go, target: '_blank', rel: 'noopener', text: 'Open the PDF at ' + hostOf(go) }));
    } else {
      if (v.posture) wrap.appendChild(el('span', { class: 'pill', text: v.posture }));
      wrap.appendChild(el('h1', { text: titleOf(o) }));
      if (v.body) wrap.appendChild(el('div', { class: 'body', text: v.body }));
      if (o.kind === 'thing' && TRADE) TRADE.draw.thing(tradeCtx(), wrap, o);
    }
    var kv = el('dl', { class: 'kv' });
    Object.keys(v).forEach(function (k) {
      var x = v[k];
      if (['title', 'name', 'display_name', 'body', 'form', 'parts', 'messages', 'posture', 'open', 'balance', 'link', 'banner', 'icon', 'retracted'].indexOf(k) >= 0) return;
      if (o.kind === 'thing' && TRADE && (k === 'condition' || k === 'description')) return;
      if (o.kind === 'post') return;
      if (x == null || x === '' || typeof x === 'object') return;
      if (/_ms$/.test(k) && typeof x === 'number') x = new Date(x).toLocaleString('en-GB', { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });
      kv.appendChild(el('dt', { text: k.replace(/_ms$/, '').replace(/_/g, ' ') })); kv.appendChild(el('dd', { text: String(x) }));
    });
    if (Array.isArray(v.lineup) && v.lineup.length) { kv.appendChild(el('dt', { text: 'lineup' })); kv.appendChild(el('dd', { text: v.lineup.join(', ') })); }
    if (kv.childNodes.length) wrap.appendChild(kv);
    f.appendChild(wrap);
    var com = commentsOf(o);
    if (com) { messageList(wrap, msgs(com), com); composeFor(com, 'Comment'); }
  }

  /* ── what a site links ────────────────────────────────────────────────────
     Product words over kinds, and the ICD op each link writes. Which args that
     op takes, and who may write it, are the ICD's. */
  /* Each section is a BOARD (website/site/assets/tiles/boards.js): its picture, its
     colours and its name are the boards', shared with the Instagram grid and the
     how page. What is this page's is only which kind it makes and how it links. */
  var SECTIONS = [
    { board: 'forums',       kind: 'forum', op: 'base.setPart', role: 'room', icon: 'hash' },
    { board: 'publications', kind: 'post',  op: 'group.setAffiliation', rel: 'created', icon: 'doc' },
    { board: 'events',       kind: 'event', op: 'group.setAffiliation', rel: 'created', icon: 'cal' },
    { board: 'marketplace',  kind: 'thing', op: 'group.setAffiliation', rel: 'created', icon: 'tag' }
  ];
  SECTIONS.forEach(function (sec) { sec.label = BOARDS ? BOARDS.NAMES[sec.board] : sec.board; });

  /* A board and its name panel, as the grid draws them: the board at its own
     resolution, the panel one canvas pixel to an art pixel, both scaled by CSS. */
  var boardCanvases = {};
  function boardMark(key) {
    if (!BOARDS || !window.Kenjin.Tile) return [];
    var c = boardCanvases[key];
    if (!c) c = boardCanvases[key] = BOARDS.paint(window.Kenjin.Tile.canvas(), key);
    var tile = document.createElement('canvas');
    tile.width = c.width; tile.height = c.height; tile.className = 'tile';
    tile.getContext('2d').drawImage(c, 0, 0);
    var art = BOARDS.bannerArt(key), name = document.createElement('canvas');
    name.width = art.width; name.height = art.height; name.className = 'name';
    name.getContext('2d').drawImage(art, 0, 0);
    name.setAttribute('aria-label', BOARDS.NAMES[key]);
    return [tile, name];
  }


  /* `ego` is what the actor must BE to the site, in the ICD's word. */
  function mayLink(site, sec) {
    var d = ICD && opDecl(sec.op);
    if (!d) return false;
    if (d.ego === 'owner') return site.owner === M.me.pk;
    return (site.members || []).indexOf(M.me.pk) >= 0 || site.owner === M.me.pk;
  }

  /* The link's args: the one that carries the reference is the new object,
     then `name`, `at`, `rel` and `role` — every other required arg is a refusal. */
  function linkArgs(sec, id, name) {
    var d = opDecl(sec.op), out = {}, missing = [];
    Object.keys(d.args || {}).forEach(function (k) {
      var spec = d.args[k];
      if ((spec.rel || []).length) out[k] = id;
      else if (k === 'name') out[k] = name;
      else if (k === 'at') out[k] = Date.now();
      else if (k === 'rel' && sec.rel) out[k] = sec.rel;
      else if (k === 'role' && sec.role) out[k] = sec.role;
      else if (spec.required) missing.push(k);
    });
    return missing.length ? Promise.reject(new Error(sec.op + ' needs ' + missing.join(', '))) : Promise.resolve(out);
  }

  /* THE SHEET: what the mint carries is the door's answer (/v2/draft), what op 0
     requires is the ICD's. */
  function sheet(site, sec) {
    if (sec.kind === 'event' && eventsModule('EventsEdit')) return open('event-edit', null, site.id);
    door('/v2/draft/' + sec.kind).then(function (d) {
      var req = {}, o0 = op0(sec.kind);
      Object.keys((o0 && o0.args) || {}).forEach(function (k) { if (o0.args[k].required) req[k] = true; });
      var box = $('sheetFields'); box.textContent = '';
      var inputs = [];
      function field(label, id, control, required) {
        box.appendChild(el('label', { class: 'fld' }, [
          el('span', {}, [label, required ? el('b', { text: ' *' }) : null]), control]));
        inputs.push({ id: id });
      }
      if (d.name_only) field('name', 'f_name', el('input', { id: 'f_name', type: 'text', maxlength: '80' }), true);
      var fields = (d.fields || []).slice();
      /* The spec's own (REQUIREMENTS.md § 5, § 6), where the Door's draft does not yet list them. */
      if (sec.kind === 'event' && !fields.some(function (f) { return f.arg === 'endMs'; })) fields.splice(2, 0, { arg: 'endMs', draft: 'end_ms', kind: 'int' });   // the Door's draft field, snake_case
      fields.forEach(function (f) { if (sec.kind === 'post' && f.arg === 'form' && f.values && f.values.indexOf('pdf') < 0) f.values = f.values.concat(['pdf']); });
      fields.filter(function (f) { return f.draft; }).forEach(function (f, i) {
        var id = 'f_' + i, c;
        if (f.values) c = el('select', { id: id }, f.values.map(function (v) { return el('option', { value: v, text: v }); }));
        else if (f.kind === 'int' && /Ms$/.test(f.arg)) c = el('input', { id: id, type: 'datetime-local' });
        else c = el('input', { id: id, type: f.kind === 'int' ? 'number' : 'text' });
        field(f.arg, id, c, !!req[f.arg]);
        inputs[inputs.length - 1] = { id: id, draft: f.draft, arg: f.arg, kind: f.kind, date: f.kind === 'int' && /Ms$/.test(f.arg), req: !!req[f.arg] };
      });
      if (d.name_only) inputs[0] = { id: 'f_name', draft: 'name', arg: 'name', req: true };
      /* The board's picture, and the tab's own word (the boards' lettering says Articles and Chat Rooms). */
      var t = $('sheetTitle'), mark = boardMark(sec.board);
      t.textContent = '';
      if (mark.length) t.appendChild(mark[0]);
      t.appendChild(el('span', { text: 'New ' + singular(sec) }));
      $('sheetWhy').textContent = '';
      $('sheetGo').textContent = 'Create';
      $('sheet').hidden = false;
      var first = box.querySelector('input,select'); if (first) first.focus();
      $('sheetForm').onsubmit = function (ev) {
        ev.preventDefault();
        var draft = {}, missing = null;
        inputs.forEach(function (f) {
          var v = $(f.id).value;
          if (f.req && !v) missing = missing || f.arg;
          if (!v) return;
          draft[f.draft] = f.date ? Date.parse(v) : f.kind === 'int' ? parseInt(v, 10) : v;
        });
        if (missing) { $('sheetWhy').textContent = missing + ' is required'; return; }
        create(site, sec, draft);
      };
    }, function (e) { $('refused').textContent = String(e.message || e); $('refused').hidden = false; });
  }

  /* MINT, THEN LINK, FROM BOTH ENDS, at one moment. The link is the site's half; the
     object writes its own: a part names its parent (`base.setParent`), and what the
     site created names it back (`base.setBacklink {rel: created}`, NC-81), without
     which it is not the site's and stays off its Face (O-48). If a link is refused
     the object exists on the owner's spine, and the refusal says how far it got. */
  function create(site, sec, draft) {
    busy($('sheetGo'), true);
    var made = [], t0 = T.begin();
    linkArgs(sec, { $step: 0 }, draft.name || '')
      .then(function (args) {
        var at = args.at || Date.now();
        var back = sec.role
          ? { do: 'apply', object: { $step: 0 }, op: 'base.setParent', args: { parent: site.id, role: sec.role, at: at } }
          : { do: 'apply', object: { $step: 0 }, op: 'base.setBacklink', args: { object: site.id, rel: sec.rel, at: at } };
        return batch([{ do: 'mint', kind: sec.kind, draft: draft }, { do: 'apply', object: site.id, op: sec.op, args: args }, back]);
      })
      .then(function (r) {
        made = r.made;
        if (r.refused) throw new Error(r.refused.why);
        $('sheet').hidden = true;
        return load().then(function () { open(sec.kind === 'forum' ? 'channel' : 'object', made[0]); T.shown('create', t0); });
      })
      .then(null, function (e) {
        $('sheetWhy').textContent = (made.length > 1 ? 'the Site names it, and it does not name the Site: ' : made.length ? 'minted, not linked: ' : '') + String(e && e.message || e);
        if (made.length) load();
      })
      .then(function () { busy($('sheetGo'), false); });
  }

  /* THE COMPOSER WRITES `forum.post`, the ICD's op, through the door's one write
     path. A refusal is the core's words, under the field. */
  var composeTo = null;
  function composeFor(id, label) {
    composeTo = id;
    $('compose').hidden = !id;
    if (id) $('say').placeholder = label ? 'Message ' + label : 'Message';
    drawReplying();
  }
  function say(ev) {
    ev.preventDefault();
    var text = $('say').value.trim();
    if (!text || !composeTo) return;
    busy($('send'), true);
    var t0 = T.begin();
    /* The spec's post (REQUIREMENTS.md § 1): its time, and a reply's parent. */
    var args = { text: text };
    if (replying && replying.forum === composeTo) { args.reply_author = replying.author; args.reply_gen = replying.gen; }
    args.ts = Date.now();
    door('/v2/apply', { method: 'POST', body: { object: composeTo, op: 'forum.post', args: args } })
      .then(function () {
        $('say').value = ''; $('refused').hidden = true; replying = null; drawReplying();
        var sent = args;
        return load().then(function () {
          var mine = msgs(composeTo).filter(function (m) { return m.author === M.me.pk && m.text === sent.text; }).pop();
          var at = mine && $('m-' + mine.author.slice(0, 16) + '-' + mine.gen);
          if (at && sent.reply_author) { at.scrollIntoView({ block: 'center' }); at.classList.add('lit'); setTimeout(function () { at.classList.remove('lit'); }, 1400); }
          else $('feed').scrollTop = 1e9;
          T.shown('post', t0);
        });
      }, function (e) {
        $('refused').textContent = String(e && e.message || e); $('refused').hidden = false;
      })
      .then(function () { busy($('send'), false); $('say').focus(); });
  }

  function signOut() {
    if (events) { events.close(); events = null; }
    door('/v2/signout', { method: 'POST' }).catch(function () {}).then(function () {
      M = null; S = { site: null, tab: 'feed', view: 'feed', sel: null };
      toWindow(false);
    });
  }

  /* ── wiring ────────────────────────────────────────────────────────────── */
  $('q').oninput = filter;
  $('compose').onsubmit = say;
  $('send').innerHTML = icon('send');
  $('meetGo').innerHTML = glyph();
  $('siteRole').onclick = function () { $('manage').hidden ? settings() : closeSettings(); };
  $('manage').onclick = function (e) { if (e.target.id === 'manage') closeSettings(); };
  $('feCancel').onclick = closeFace;
  $('person').onclick = function (e) { if (e.target.id === 'person') closeCard(); };
  $('personX').onclick = closeCard;
  $('personX').innerHTML = icon('close');
  $('youX').onclick = closeYou;
  $('youX').innerHTML = icon('close');
  $('mmenu').addEventListener('click', function (e) { if (e.target === $('mmenu') && Date.now() - menuAt > 350) closeMenu(); });
  $('pdfX').innerHTML = icon('close'); $('pdfIn').innerHTML = icon('zin'); $('pdfOut').innerHTML = icon('zout');
  $('pdfX').onclick = closePdf;
  $('pdfIn').onclick = function () { if (viewing && viewing.zoom) viewing.zoom(1.25); };
  $('pdfOut').onclick = function () { if (viewing && viewing.zoom) viewing.zoom(0.8); };
  $('pdfFit').onclick = function () { if (viewing && viewing.zoom) viewing.zoom(0); };
  window.addEventListener('resize', function () { if (viewing && viewing.relayout) viewing.relayout(); });
  /* A press opens it and leaves the glyph showing; a keyboard opens it ready to type. */
  $('meetGo').onclick = function (e) { meet(true, { focus: e.detail === 0 }); };
  $('q').addEventListener('focus', function () { $('meet').classList.add('typing'); });
  $('meet').addEventListener('focusout', function (e) {
    if (MEETING && !(e.relatedTarget && $('meet').contains(e.relatedTarget)) && !pressing) meet(false);
  });
  /* The bar, the tabs' column and the brand move with every rail change, animated or dragged:
     measure again whenever any of them changes size, and once a column change has settled. */
  if (window.ResizeObserver) {
    var ro = new ResizeObserver(function () { placeMeet(); });
    ro.observe($('top')); ro.observe($('tabs')); ro.observe($('util'));
  }
  $('top').addEventListener('transitionend', function (e) { if (e.target === $('top')) placeMeet(); });
  $('toDrawer').innerHTML = icon('rail');
  $('toPeople').innerHTML = icon('members');
  $('toDrawer').onclick = function () { $('app').classList.toggle('drawer'); };
  $('toPeople').onclick = function () { $('app').classList.toggle('roster'); };
  $('shade').onclick = function () { $('app').classList.remove('drawer', 'roster'); };
  $('lFold').onclick = function () { fold('l'); };
  $('rFold').onclick = function () { fold('r'); };
  grip('l'); grip('r');
  MENUS.forEach(function (m, i) { $(m[0]).onclick = function () { toggleMenu(i); }; });
  var pressing = false;   // a press inside Meet moves focus through it; that is not leaving
  $('meet').addEventListener('pointerdown', function () { pressing = true; setTimeout(function () { pressing = false; }, 0); });
  document.addEventListener('pointerdown', function (e) {
    if (!e.target.closest) return;
    if (!$('emo').hidden && !e.target.closest('#emo,.act.react')) closePicker();
    if (!e.target.closest('.msg.on,#emo')) closeActs();
    if (!e.target.closest('.menu,#siteBtn,#meBtn')) closeMenus();
    if (MEETING && e.target.closest && !e.target.closest('#meet')) meet(false);
  });
  window.addEventListener('resize', function () { rails(); closePicker(); });
  $('feed').addEventListener('scroll', function () { if (!$('emo').classList.contains('bottom')) closePicker(); }, { passive: true });
  $('sheetX').onclick = closeSheet;
  $('sheet').onclick = function (e) { if (e.target.id === 'sheet') closeSheet(); };
  $('you').onclick = function (e) { if (e.target.id === 'you') closeYou(); };
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') { if (!$('mmenu').hidden) { closeMenu(); return; } if (!$('emo').hidden) { closePicker(); return; } closeActs(); if (!$('pdfv').hidden) return closePdf(); if (!$('sheet').hidden) closeSheet(); closeMenus(); meet(false); if (!$('manage').hidden) closeSettings(); closeCard(); closeYou(); }
    else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k' && M) { e.preventDefault(); meet(true, { focus: true }); }
  });
  rails();

  /* A live cookie is a session: straight in. None, and the window. Any other answer is
     the Door's own words, where the page would have been: not a round trip back to it. */
  door('/v2/me').then(function () {
    $('boot').hidden = true;
    arrive();
    T.shown('first-render', 0);
  }, function (e) {
    if (e && e.status === 401) return signIn();
    $('boot').textContent = String(e && e.message || e);
    T.shown('first-render', 0);
  });
})();
