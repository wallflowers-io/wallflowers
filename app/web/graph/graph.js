/* ═══════════════════════════════════════════════════════════════════════════
   graph.js — the WallFlowers graph, drawn from what the core folded, and grown
   by what the core mints.

   WHAT IS REAL HERE. Every node, every edge, every action, every refusal and
   every minted object is the core's, read or authored through `core-wasm` at
   runtime:

     the door   GET /v1/personas  the persona store, in name order
                GET /v1/kinds     which kinds the core will mint, and which it will not
                GET /v1/graph     what an account holds: each object folded through its
                                  own lens by `Node::object_view`, its roster, its owner,
                                  and the spine's own list
                POST /v1/mint     `Node::mint` — the MLS group, the kind's op 0 through
                                  its own door, delta_sig, the vertebra, and the publish
     the core   ops_on / WallflowersOps   which kinds take which op, and the authority

   NO ARCHIVE. It was deleted from the core on 23 September 2026: a mint or a join
   puts the object on the spine, the spine is reachable from the seed, and shipping
   a backup blob to a page that wanted a list of names was never the way to read.

   So this page states no kind, no op, no opId and no authority of its own.

   WHAT IS NOT REAL, and says so in the header, on the node and in the rail:
   NOTHING IS PUBLISHED. There is no relay here and no arc: a minted object
   lives in this tab's memory, its deltas are real and go nowhere, and a reload
   ends it. That is a fact about this page, not about the objects — the same
   calls against a relay would carry them.

   ZERO IS A NUMBER. With nothing loaded and nobody minted this draws no nodes
   and says the graph holds none, because that is the true count.

   SELECTION IS EGO AND TARGET. The first selection is the Ego and is always a
   person: every action belongs to somebody. The second is the Target and may be
   a person, an object or an edge. What the Ego may do to the Target is decided
   by the op's authority against their standing in it, never by this file.
   ═══════════════════════════════════════════════════════════════════════════ */
(function () {
  'use strict';

  var $ = function (id) { return document.getElementById(id); };
  var dec = new TextDecoder(), enc = new TextEncoder();

  /* ── the core ─────────────────────────────────────────────────────────────
     PacificAccount is the loader: it supplies the three declared imports, of
     which `pacific_fill_random` is the one that matters, and hands back the
     instance's exports. The core is the one staged for the other surfaces —
     this page is served from `app/web`, so it reads that copy rather than
     adding a fourth staging target. */
  /* THE MODULE IS FETCHED PAST THE CACHE. A browser that cached the wasm under
     heuristic freshness keeps serving it whatever the server says NEXT, so a
     rebuilt export reads as "fn is not a function" until someone thinks to hard
     reload. A dev tool reloads against a core that changed minutes ago; paying
     for the fetch is cheaper than the confusion. */
  var account = new PacificAccount({
    corePath: '../docs/core/core_wasm_bg.wasm?b=' + Date.now()
  });
  var CORE = null;

  /* THE LOG IS NOT A NARRATION. One line per crossing into the core through the
     raw ptr/len ABI, with the real input and output lengths and whatever the
     module put in the out buffer. A refusal is a line like any other, in the
     core's own words. */
  var LOG = [], T0 = Date.now();

  function logline(name, inN, outN, ok, rs) {
    LOG.push({ t: Date.now() - T0, name: name, inN: inN, outN: outN, ok: ok, rs: rs || '' });
    if (LOG.length > 500) LOG.shift();
    renderLog();
  }

  /* A short read of what came back, taken from the bytes themselves. */
  function summarise(name, out) {
    if (!out.length) return '';
    if (out[0] !== 0x7b) return out.length + ' bytes';       /* not JSON: CBOR, words, a tag */
    try {
      var v = JSON.parse(dec.decode(out)), bits = [];
      ['group_id', 'delta_id', 'cred', 'op_id', 'members', 'groups', 'ops', 'id', 'kind'].forEach(function (k) {
        if (v[k] === undefined) return;
        var x = v[k];
        bits.push(k + '=' + (typeof x === 'string' ? (x.length > 12 ? '\u00b7' + x.slice(-6) : x)
                 : Array.isArray(x) ? x.length : x));
      });
      return bits.join(' ');
    } catch (e) { return out.length + ' bytes'; }
  }

  /* alloc → write → call → read, the ABI every export shares. `erred()` means
     the out buffer holds a message rather than a result. */
  function callBytes(name, fn, bytes) {
    bytes = bytes || new Uint8Array(0);
    var ptr = CORE.alloc(bytes.length);
    if (bytes.length) new Uint8Array(CORE.memory.buffer, ptr, bytes.length).set(bytes);
    var n = fn(bytes.length);
    /* Read the buffer AFTER the call: serving one can grow the module's memory,
       and growing detaches every view taken beforehand. */
    if (CORE.erred()) {
      var why = dec.decode(new Uint8Array(CORE.memory.buffer, CORE.out_ptr(), CORE.out_len()));
      logline(name, bytes.length, 0, false, why);
      throw new Error(why);
    }
    var out = new Uint8Array(CORE.memory.buffer, CORE.out_ptr(), n).slice();
    logline(name, bytes.length, n, true, summarise(name, out));
    return out;
  }
  function call(name, fn, bytes) { return dec.decode(callBytes(name, fn, bytes)); }
  function callJson(name, fn, obj) { return JSON.parse(call(name, fn, enc.encode(JSON.stringify(obj)))); }

  function renderLog() {
    var box = $('loglines');
    if (!box) return;
    box.textContent = '';
    LOG.forEach(function (l) {
      var d = document.createElement('div');
      d.className = 'ln' + (l.ok ? '' : ' err');
      d.innerHTML = '<span class="t">' + (l.t / 1000).toFixed(2) + 's</span>' +
                    '<span class="fn">' + esc(l.name) + '</span>' +
                    '<span class="io">' + l.inN + ' \u2192 ' + l.outN + ' B</span>' +
                    '<span class="rs">' + esc(l.rs) + '</span>';
      box.appendChild(d);
    });
    box.scrollTop = box.scrollHeight;
    var n = $('logn');
    if (n) n.textContent = LOG.length + ' calls';
  }

  /* ── state ──────────────────────────────────────────────────────────────── */
  var M = {
    source: null,        // where an opened archive came from
    fold: null,          // what /v1/graph returned for one door
    objects: [],         // GroupObjects: folded, refused, or minted here
    people: [],
    edges: [],
    byId: {},
    me: null,            // who this page is acting as
    session: null,       // which door it acts through
    sessions: [],        // every door the image is serving
    personas: [],        // every persona the door's store holds
    folds: [],           // one fold per door
    kindRows: [],        // every kind the core knows, as /v1/kinds answered
    mintable: {},        // kind -> may the core mint it, from /v1/kinds
    kindWhy: {},         // kind -> the core's word for why not (mint::NoMint)
    opsOn: {},           // op name -> [kind, …], from the core
    icd: null,           // the model itself, as the door serves it
    lens: {},            // kind string -> ICD channel, from authoring::channel_of
    kinds: [],
    colour: {},
    ego: null,           // the person acting. Always a person.
    target: null         // {kind:'node', id} | {kind:'edge', i} | null
  };

  /* Seeds and devices of people minted here. The seed never enters the model or
     the DOM, so no render can put it on the page by accident. */
  /* THE KINDS TAKE THE BRAND'S COLOURS. The doorways palette is declared in the
     stylesheet as --k1…--k8 and read from there, so the livery owns the colours
     and this file owns none. */
  function palette() {
    var css = getComputedStyle(document.documentElement), out = [];
    for (var i = 1; i <= 8; i++) {
      var v = css.getPropertyValue('--k' + i).trim();
      if (v) out.push(v);
    }
    return out.length ? out : ['#0D6AAE'];
  }
  var PALETTE = palette();
  var UNKNOWN = (getComputedStyle(document.documentElement)
                  .getPropertyValue('--mute') || '#9AA1A9').trim();

  function tail(id) { return id ? id.slice(-6) : '······'; }
  function hex(b) {
    var s = '';
    for (var i = 0; i < b.length; i++) s += (b[i] < 16 ? '0' : '') + b[i].toString(16);
    return s;
  }
  function rand32() { return hex(crypto.getRandomValues(new Uint8Array(32))); }
  /* NO STATUS LINE. A failure is logged as what it is — the call that failed and
     the words it failed with — and nothing on this page narrates itself. */
  function fail(what, why) { logline(what, 0, 0, false, why); }
  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"]/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c];
    });
  }
  function nameOf(id) { var p = M.byId[id]; return (p && p.name) || ''; }

  /* ═════════════════════════════════════════════════════ THE DOOR ══════════
     Everything that writes goes through here. The page never sees a seed, a
     signing key or an MLS snapshot — it asks the door, and the door answers with
     the core's own words when it refuses.

     The create buttons are product words over the core's kinds. WHICH kinds may
     be minted is the CORE's answer (`mint::classify`, served at /v1/kinds), so a
     kind it refuses cannot be offered and a kind it gains appears here. */
  var DOOR = 'http://127.0.0.1:8233';
  /* WHICH KINDS CAN BE BROUGHT INTO EXISTENCE IS THE CORE'S ANSWER — `/v1/kinds`,
     which is `mint::classify` — so a kind it gains appears here and a kind it
     loses goes, with no edit. These are product WORDS over those kinds and
     nothing else: a kind with no word here shows its own name rather than being
     left out, which is how a new kind announces itself. */
  var LABEL = {
    group: 'Site', forum: 'ChatRoom', post: 'Article', thing: 'Listing',
    event: 'Event', project: 'Project', place: 'Place',
    contact: 'Connection', conversation: 'Conversation'
  };

  /* HOW a kind comes into existence, in `mint::NoMint`'s own words. `internal`
     and `unbuilt` are not acts a person performs, so they are not offered. */
  function creatable() {
    return (M.kindRows || []).filter(function (k) {
      return k.mintable || k.no_mint === 'paired' || k.no_mint === 'derived';
    }).map(function (k) {
      /* WHAT IT MAKES, from the model's own `parts`: minting an Article brings a
         comments Forum into being with it, and a button that implies one object
         when the act makes two is the shorter-list lie in miniature. */
      var shape = (M.icd && M.icd.kinds[k.kind]) || {};
      return { kind: k.kind, label: LABEL[k.kind] || k.kind,
               parts: (shape.parts || []).map(function (p) { return p.role; }),
               how: k.mintable ? 'mint' : k.no_mint };
    }).sort(function (a, b) {
      /* What one person can make, first. The order inside each half is the
         core's — `ObjectKind::ALL`, which is type-id order. */
      return (a.how === 'mint' ? 0 : 1) - (b.how === 'mint' ? 0 : 1);
    });
  }

  function door(path, opts) {
    var t0 = Date.now(), body = opts && opts.body, h = {};
    if (body) h['content-type'] = 'application/json';
    /* WHICH DOOR. The image runs one per Ego and routes by session. A browser sends
       one cookie per origin, so this dev tool names the session it means on every
       call; a shipped client holds one session and sends the cookie. */
    var sess = (opts && opts.session) || M.session;
    if (sess) h['x-door-session'] = sess;
    return fetch(DOOR + path, {
      method: (opts && opts.method) || 'GET',
      headers: h,
      body: body ? JSON.stringify(body) : undefined
    }).then(function (r) {
      var cbor = (r.headers.get('content-type') || '').indexOf('cbor') >= 0;
      return (cbor ? r.arrayBuffer() : r.text()).then(function (v) {
        var n = cbor ? v.byteLength : v.length;
        logline('door ' + path, body ? JSON.stringify(body).length : 0, n, r.ok,
                r.ok ? (Date.now() - t0) + ' ms' : String(v).slice(0, 110));
        if (!r.ok) throw new Error(v);
        return cbor ? new Uint8Array(v) : (v ? JSON.parse(v) : null);
      });
    }, function () {
      logline('door ' + path, 0, 0, false, 'unreachable — is the door running?');
      throw new Error('the door is not answering on ' + DOOR);
    });
  }

  /* THE DOORS THE IMAGE IS SERVING. One account per process, because
     `paths::state_dir()` is read per call and two accounts in one process would
     share a directory. So a person is a door, and the Ego is whichever door this
     page is acting as. */
  function hello() {
    return door('/v1/personas', { session: null }).then(function (list) {
      M.personas = list || [];
      M.sessions = M.personas.filter(function (p) { return p.open; })
        .map(function (p) { return { session: p.session, pk: p.pk, name: p.name }; });
      /* THE RAIL IS THE STORE, not the open doors. With personas held and none
         open — every door closed, or the image just restarted — a bare `render`
         left the rail empty and nothing to click to open one. */
      if (!M.sessions.length) { rebuild(true); return null; }
      if (!M.session || !M.sessions.some(function (s) { return s.session === M.session; })) {
        use(M.sessions[0]);
      }
      return door('/v1/kinds').then(function (kinds) {
        M.kindRows = kinds || [];
        M.mintable = {}; M.kindWhy = {};
        M.kindRows.forEach(function (k) {
          M.mintable[k.kind] = k.mintable;
          M.kindWhy[k.kind] = k.no_mint || null;
        });
        return refresh();
      });
    });
  }

  /* A PERSONA IS A ROW IN THE DOOR'S STORE. Opening one starts its account
     process if it is not already running; the words that rebuild it never leave
     the door. */
  function actAs(persona) {
    if (persona.open && persona.session) {
      use({ session: persona.session, pk: persona.pk, name: persona.name });
      return refresh();
    }
    return door('/v1/session', { method: 'POST', session: null, body: { pk: persona.pk } })
      .then(function (s) { return hello().then(function () { use(s); return refresh(); }); })
      .catch(function (e) { fail('door', e.message); });
  }

  function use(s) {
    M.session = s.session;
    M.ego = (s.pk || '').replace(/^ed25519:/, '');
    M.me = { pk: s.pk, display_name: s.name };
  }

  /* A NEW PERSONA. The door mints the words, keeps them, and stands the account
     up. Today it asks for nothing but a name — the passkey ceremony is the next
     piece, and until it exists this is why the image binds loopback. */
  function newPersona(name, done) {
    return door('/v1/persona', { method: 'POST', session: null, body: { name: name } })
      .then(function (s) {
        return hello().then(function () {
          use(s);
          return refresh().then(function () { if (done) done(null); });
        });
      })
      .catch(function (e) { if (done) done(e.message); else fail('door', e.message); });
  }

  function closeDoor(s) {
    return door('/v1/session', { method: 'DELETE', session: s.session })
      .then(function () {
        M.sessions = M.sessions.filter(function (x) { return x.session !== s.session; });
        if (M.session === s.session) {
          M.session = null; M.ego = null;
          if (M.sessions.length) use(M.sessions[0]);
        }
        return refresh();
      })
      .catch(function (e) { fail('door', e.message); });
  }

  /* The archive is the door's answer to "what does this account have". The page
     each object folded by `Node::object_view` in the door, through the core's one
     renderer — the same one the wasm serves. */
  /* EVERY OPEN DOOR'S ARCHIVE, folded by the core and unioned. Each account holds
     its own; an object two people are both in appears in both, and the same id is
     the same node. That union IS the graph this page is for. */
  function refresh() {
    if (!M.sessions.length) { M.folds = []; rebuild(true); return Promise.resolve(); }
    return Promise.all(M.sessions.map(function (s) {
      return door('/v1/graph', { session: s.session })
        .then(function (g) { return { s: s, fold: g }; })
        .catch(function () { return { s: s, fold: null }; });
    })).then(function (all) {
      M.folds = all.filter(function (a) { return a.fold; });
      rebuild(true);
    });
  }

  /* THE MINT SHEET. What a kind's mint carries is the door's answer (/v1/draft),
     discovered from `profile_args` rather than listed anywhere. Which of those
     arguments the op REQUIRES is the ICD's answer, through the generated bindings.
     Neither list is kept in this file. */
  var DRAFTS = {};

  function op0(kind) {
    var OPS = window.WallflowersOps.OPS;
    var found = null;
    Object.keys(OPS).forEach(function (n) {
      if (OPS[n].prefix === kind && OPS[n].opId === 0) found = OPS[n];
    });
    return found;
  }

  /* The same sheet, for a person. One field, because that is all the door asks
     for today; when the ceremony lands this is where it goes. */
  function openPersonSheet() {
    $('sheetfields').innerHTML =
      '<div class="fld"><label><b>name</b><span class="req">required</span></label>' +
      '<input id="p_name" type="text" maxlength="60"></div>';
    $('sheettitle').textContent = 'New person';
    $('sheetwhy').textContent = '';
    $('sheet').classList.add('person');
    $('modal').hidden = false;
    var el = $('p_name');
    if (el) el.focus();
    $('sheet').onsubmit = function (ev) {
      ev.preventDefault();
      var nm = ($('p_name').value || '').trim();
      if (!nm) { $('sheetwhy').textContent = 'name is required'; return; }
      $('sheetmint').disabled = true;
      newPersona(nm, function (why) {
        $('sheetmint').disabled = false;
        if (why) { $('sheetwhy').textContent = why; return; }
        closeSheet();
      });
    };
  }

  function openSheet(spec) {
    var get = DRAFTS[spec.kind]
      ? Promise.resolve(DRAFTS[spec.kind])
      : door('/v1/draft/' + spec.kind).then(function (d) { DRAFTS[spec.kind] = d; return d; });
    get.then(function (d) { sheet(spec, d); })
       .catch(function (e) { fail('door', e.message); });
  }

  function sheet(spec, d) {
    var box = $('sheetfields'), op = op0(spec.kind) || { fields: [] };
    var required = {};
    (op.fields || []).forEach(function (f) { if (f.required) required[f.name] = true; });
    box.textContent = '';
    $('sheettitle').textContent = spec.label + ' \u00b7 ' + spec.kind;
    $('sheetwhy').textContent = '';

    var inputs = [];
    if (d.name_only) {
      /* A Forum carries no profile op: its name rides the MLS group context, and
         authoring op 0 would post a message. So one field, and it is not an arg. */
      var w = document.createElement('div');
      w.className = 'fld';
      w.innerHTML = '<label><b>name</b></label>' +
                    '<input id="f_name" type="text" maxlength="80">' +
                    '<div class="hint">name only \u2014 the MLS group carries it</div>';
      box.appendChild(w);
      inputs.push({ draft: 'name', el: 'f_name', kind: 'text' });
    }
    (d.fields || []).forEach(function (f, i) {
      var id = 'f_' + i, carried = !!f.draft;
      var w = document.createElement('div');
      w.className = 'fld' + (carried ? '' : ' off');
      var control;
      if (f.values) {
        control = '<select id="' + id + '">' + f.values.map(function (v) {
          return '<option value="' + esc(v) + '">' + esc(v) + '</option>';
        }).join('') + '</select>';
      } else if (f.kind === 'int' && /Ms$/.test(f.arg)) {
        /* An epoch in milliseconds is the wire's unit, not a person's. The field
           takes a date and time and the page converts; nothing else is inferred. */
        control = '<input id="' + id + '" type="datetime-local">';
      } else {
        control = '<input id="' + id + '" type="' + (f.kind === 'int' ? 'number' : 'text') +
                  '"' + (carried ? '' : ' disabled') + '>';
      }
      w.innerHTML = '<label><b>' + esc(f.arg) + '</b>' +
        (required[f.arg] ? '<span class="req">required</span>' : '') + '</label>' + control +
        (carried ? '' : '<div class="hint">a mint does not carry this</div>');
      box.appendChild(w);
      if (carried) inputs.push({ draft: f.draft, el: id, kind: f.kind, arg: f.arg,
                                 date: f.kind === 'int' && /Ms$/.test(f.arg) });
    });

    $('sheet').classList.remove('person');
    $('modal').hidden = false;
    var first = box.querySelector('input,select');
    if (first) first.focus();

    $('sheet').onsubmit = function (ev) {
      ev.preventDefault();
      var draft = {}, missing = null;
      inputs.forEach(function (f) {
        var el = $(f.el);
        if (!el) return;
        var v = el.value;
        if (f.date) { draft[f.draft] = v ? Date.parse(v) : 0; }
        else if (f.kind === 'int') { draft[f.draft] = v ? parseInt(v, 10) : 0; }
        else if (v) { draft[f.draft] = v; }
        if (f.arg && required[f.arg] && !v) missing = f.arg;
      });
      if (missing) { $('sheetwhy').textContent = missing + ' is required'; return; }
      $('sheetmint').disabled = true;
      mintObject(spec, draft, function (why) {
        $('sheetmint').disabled = false;
        if (why) { $('sheetwhy').textContent = why; return; }
        $('modal').hidden = true;
      });
    };
  }

  function closeSheet() {
    $('modal').hidden = true;
    $('sheet').classList.remove('person');
  }

  function mintObject(spec, draft, done) {
    door('/v1/mint', { method: 'POST', body: { kind: spec.kind, draft: draft } })
      .then(function (got) {
        return refresh().then(function () {
          M.target = { kind: 'node', id: got.object_id };
          render();
          if (done) done(null);
        });
      })
      .catch(function (e) {
        if (done) done(e.message); else fail('door /v1/mint', e.message);
      });
  }


  /* PAIRING. The image drives both doors: the peer's bundle, the Ego's scan
     (group, Welcome, vertebra), the peer's drain and accept. */
  function connectTo(to, done) {
    door('/v1/connect', { method: 'POST', body: { pk: to.pk } })
      .then(function (got) {
        return refresh().then(function () {
          if (got && got.object) M.target = { kind: 'node', id: got.object };
          render();
          if (done) done(null);
        });
      })
      .catch(function (e) {
        if (done) done(e.message); else fail('door /v1/connect', e.message);
      });
  }

  /* ── loading an archive ─────────────────────────────────────────────────── */

  /* CLEAR IS A WIPE, not a redraw. Every account process killed, every account
     directory and the persona store deleted, and the relay restarted — it holds
     its blobs in memory, so a restart is the only wipe there is. */
  function clearAll() {
    door('/v1/state', { method: 'DELETE', session: null })
      .then(function () {
        M.folds = []; M.personas = []; M.sessions = [];
        M.session = null; M.ego = null; M.me = null; M.target = null;
        M.objects = []; M.people = []; M.edges = []; M.byId = {};
        render();
        return hello();
      })
      .catch(function (e) { fail('door /v1/state', e.message); });
  }

  /* ── the graph ──────────────────────────────────────────────────────────── */
  function person(id, name, extra) {
    if (!id) return null;
    var p = M.byId[id];
    if (!p) {
      p = { id: id, type: 'person', name: name || '', sub: extra || '', member: {}, x: 0, y: 0 };
      M.byId[id] = p; M.people.push(p);
    }
    if (name && !p.name) p.name = name;
    if (extra && !p.sub) p.sub = extra;
    return p;
  }

  function edge(from, to, label, cls) {
    if (!from || !to || from === to) return;
    M.edges.push({ from: from, to: to, label: label, cls: cls || '' });
  }

  function addObject(o) {
    var old = wasAt[o.id];
    if (old) { o.x = old.x; o.y = old.y; o.fixed = old.fixed; }
    M.byId[o.id] = o; M.objects.push(o); return o;
  }
  var wasAt = {};

  function build() {
    /* The rail lists the store, not only what folded: a persona whose door is
       closed still exists, and saying otherwise is the shorter-list lie. */
    /* A rebuild makes new node objects; the field should not jump because of it,
       and a node somebody dragged must stay where they put it. */
    var was = M.byId;
    var minted = M.people.filter(function (p) { return p.minted; });
    var keep = {};
    minted.forEach(function (p) { keep[p.id] = { name: p.name, sub: p.sub }; });
    wasAt = was || {};
    M.objects = []; M.people = []; M.edges = []; M.byId = {};
    minted.forEach(function (p) { p.member = {}; M.byId[p.id] = p; M.people.push(p); });

    (M.personas || []).forEach(function (s) {
      person((s.pk || '').replace(/^ed25519:/, ''), s.name, s.open ? 'a door' : 'closed');
    });
    (M.folds || []).forEach(function (entry) {
      var f = entry.fold, pk = ((f.me && f.me.pk) || '').replace(/^ed25519:/, '');
      var me = person(pk, (f.me && f.me.display_name) || '', 'a door');
      me.minted = true;
      var spine = {};
      (f.spine || []).forEach(function (v) { spine[v.object] = v.index; });
      (f.objects || []).forEach(function (o) {
        if (M.byId[o.id]) return;            /* two doors in one object: one node */
        addObject({ id: o.id, type: 'object', kind: o.kind, name: o.name, owner: o.owner,
                    members: o.members || [], view: o.view, folded: !!o.folds,
                    peer: o.peer || null, role: o.role || null,
                    why: o.why, spineIndex: spine[o.id], x: 0, y: 0 });
      });
    });

    M.objects.forEach(function (o) {
      if (!o.folded) return;
      person(o.owner);
      edge(o.owner, o.id, o.spineOf ? 'their own record' : 'owns', 'owns');
      o.members.forEach(function (m) {
        person(m);
        if (m !== o.owner) edge(m, o.id, 'member', 'member');
        if (M.byId[m]) M.byId[m].member[o.id] = true;
      });
      if (M.byId[o.owner]) M.byId[o.owner].member[o.id] = true;

      var v = o.view || {};
      (v.affiliations || []).forEach(function (a) { edge(o.id, a.peer, a.rel || 'affiliation', 'aff'); });
      (v.parts || []).forEach(function (p) { edge(o.id, p.part, p.role, 'aff'); });
      (v.offices || []).forEach(function (of) {
        person(of.holder);
        edge(of.holder, o.id, of.office + (of.status ? ' · ' + of.status : ''), 'office');
      });
      /* The vertebrae. A spine's `joined` is the person's own enumeration of
         their graph, and it is the edge that makes an object reachable from
         nothing but their words. */
      (v.joined || []).forEach(function (j) {
        edge(o.id, j.object, 'joined · ' + (j.kind || '') + ' · from the fold', 'spine');
      });
    });

    M.edges.forEach(function (e) {
      [e.from, e.to].forEach(function (id) {
        if (!M.byId[id]) {
          addObject({ id: id, type: 'object', kind: '', name: '', owner: '', members: [],
                      digest: '', view: null, folded: false, absent: true,
                      why: 'named by an edge, not in anything loaded here', x: 0, y: 0 });
        }
      });
    });

    if (M.ego && !M.byId[M.ego]) M.ego = null;
    if (M.target && M.target.kind === 'node' && !M.byId[M.target.id]) M.target = null;
  }

  function rebuild(relayout) {
    build();
    if (relayout) layout();
    render();
    applyT();
  }

  /* ── layout: the spine, in order, as a grid ─────────────────────────────
     The Ego's spine IS the arrangement. It is an ordered list — index 0 is
     their own record, and every object they hold hangs off a later vertebra —
     so it is laid left to right in that order and wrapped, rather than let a
     force find an order of its own. Anything the field holds that is NOT on
     this Ego's spine goes in a second band below: it belongs to another door,
     or it was named by an edge and is not held here. */
  var CELL = { w: 178, h: 128 };

  /* The active Ego's spine, in index order, as object ids. */
  function spineOrder() {
    var out = [];
    if (!M.ego) return out;
    (M.folds || []).forEach(function (entry) {
      var f = entry.fold;
      if (((f.me && f.me.pk) || '').replace(/^ed25519:/, '') !== M.ego) return;
      (f.spine || []).slice().sort(function (a, b) { return a.index - b.index; })
        .forEach(function (v) { out.push(v.object); });
    });
    return out;
  }

  /* THE TWO BANDS. `mine` is the Ego's spine in index order; `rest` is what the
     field holds that is not on it — another door's objects, or an id an edge
     named and nobody here holds. */
  function bands() {
    var order = spineOrder(), rank = {};
    order.forEach(function (id, i) { rank[id] = i; });
    var mine = [], rest = [];
    M.objects.forEach(function (o) { (rank[o.id] === undefined ? rest : mine).push(o); });
    mine.sort(function (a, b) { return rank[a.id] - rank[b.id]; });
    rest.sort(function (a, b) {
      var an = (a.kind || '') + (a.name || '') + a.id, bn = (b.kind || '') + (b.name || '') + b.id;
      return an < bn ? -1 : an > bn ? 1 : 0;
    });
    return { mine: mine, rest: rest };
  }

  function layout() {
    /* Only objects are laid out. A person is not a dot in the field — they are
       their row in the rail, and every edge they are in is anchored to it.
       ONE ROW PER BAND, never wrapped: the spine is an ordered list and a wrap
       puts vertebra 7 under vertebra 1, which reads as a second beginning. It
       runs off the panel instead, and the panel scrolls. */
    var b = bands();
    place(b.mine, 0);
    place(b.rest, CELL.h * 1.4);
  }

  function place(band, y0) {
    band.forEach(function (o, i) {
      if (o.fixed) return;                    /* dragged: it stays where it was put */
      o.x = i * CELL.w + CELL.w / 2;
      o.y = y0;
    });
  }

  /* The width the field needs at this zoom, and the height its bands occupy. */
  function extent() {
    var b = bands();
    var cols = Math.max(b.mine.length, b.rest.length, 1);
    return { w: cols * CELL.w + CELL.w, h: b.rest.length ? CELL.h * 1.4 : 0 };
  }

  /* WHERE THE SPINE SITS: level with the Ego's row in the rail, so the row, its
     anchor and its first vertebra are one horizontal line. It follows the row
     when the rail scrolls. */
  function egoRowY() {
    var wrap = $('canvasWrap').getBoundingClientRect();
    var row = M.ego && $('plist').querySelector('.row[data-id="' + M.ego + '"]');
    var y = row
      ? row.getBoundingClientRect().top + row.getBoundingClientRect().height / 2 - wrap.top
      : wrap.height / 3;
    var below = extent().h * T.k + CELL.h * 0.6;
    return Math.max(CELL.h * 0.5, Math.min(wrap.height - below, y));
  }

  /* ── render ─────────────────────────────────────────────────────────────── */
  var SVGNS = 'http://www.w3.org/2000/svg';
  function el(name, attrs) {
    var e = document.createElementNS(SVGNS, name);
    for (var a in attrs) if (attrs.hasOwnProperty(a)) e.setAttribute(a, attrs[a]);
    return e;
  }

  function kindColour(kind) {
    if (!kind) return UNKNOWN;
    if (M.colour[kind]) return M.colour[kind];
    var i = M.kinds.indexOf(kind);
    return (i >= 0) ? PALETTE[i % PALETTE.length] : UNKNOWN;
  }

  function targetId() { return M.target && M.target.kind === 'node' ? M.target.id : null; }

  function inSubgraph(nodeId) {
    if (!M.ego) return true;
    var p = M.byId[M.ego];
    if (!p) return true;
    var o = M.byId[nodeId];
    if (!o) return false;
    if (o.type === 'person') return o.id === p.id;
    return !!p.member[nodeId];
  }

  function render() {
    var edges = $('edges'), nodes = $('nodes');
    edges.textContent = ''; nodes.textContent = '';

    var tid = targetId();
    M.edges.forEach(function (e, i) {
      var a = M.byId[e.from], b = M.byId[e.to];
      if (!a || !b) return;
      if (a.type === 'person' || b.type === 'person') return;      /* pinned layer */
      var lit = (tid && (e.from === tid || e.to === tid)) ||
                (M.target && M.target.kind === 'edge' && M.target.i === i);
      var out = !(inSubgraph(e.from) && inSubgraph(e.to));
      var g = el('g', { 'class': 'edgehit' });
      var ln = el('line', {
        x1: a.x, y1: a.y, x2: b.x, y2: b.y,
        'class': 'edge ' + (e.cls ? e.cls + ' ' : '') + (lit ? 'lit ' : '') + (out ? 'out' : '')
      });
      var hit = el('line', { x1: a.x, y1: a.y, x2: b.x, y2: b.y, 'class': 'hit' });
      e._ln = ln; e._hit = hit;
      g.appendChild(ln); g.appendChild(hit);
      g.appendChild(el('title', {})).textContent = e.label;
      g.addEventListener('click', function (ev) {
        ev.stopPropagation();
        M.target = (M.target && M.target.kind === 'edge' && M.target.i === i)
          ? null : { kind: 'edge', i: i };
        render();
      });
      edges.appendChild(g);
    });

    M.objects.forEach(function (o) { nodes.appendChild(objectNode(o)); });

    renderPeople();
    renderPinned();
    renderFold();
    renderCreate();
    renderActions();
    renderLegend();
    renderSelection();
  }

  /* A PERSON AND THEIR ROW ARE ONE THING, whoever the Ego is. People are not
     dots floating in the field: each is the row on the left, anchored at the
     rail's edge level with that row, and every edge they are in starts there.
     This layer is OUTSIDE the graph transform and in screen coordinates, which
     is what keeps each anchor on its row while the field pans and zooms under
     it, and what makes the rail's scroll move them. */
  function anchors() {
    var out = {}, wrap = $('canvasWrap').getBoundingClientRect();
    var rail = $('people').getBoundingClientRect();
    Array.prototype.forEach.call($('plist').querySelectorAll('.row'), function (row) {
      var r = row.getBoundingClientRect();
      var mid = r.top + r.height / 2;
      out[row.dataset.id] = {
        x: 0,
        y: Math.max(8, Math.min(wrap.height - 8, mid - wrap.top)),
        /* a row scrolled out of the rail keeps its anchor at the edge it went
           out of, so the edge still points the right way rather than vanishing */
        off: mid < rail.top + 4 || mid > rail.bottom - 4
      };
    });
    return out;
  }

  /* A node's place on the overlay: the field's transform, less the scroll. */
  function onScreen(o) {
    return { x: T.x + o.x * T.k - scroller.scrollLeft, y: T.y + o.y * T.k };
  }

  function renderPinned() {
    var layer = $('pinned');
    if (!layer) return;
    layer.textContent = '';
    var A = anchors(), tid = targetId();

    M.edges.forEach(function (e, i) {
      var a = M.byId[e.from], b = M.byId[e.to];
      if (!a || !b) return;
      var pa = a.type === 'person' ? A[a.id] : null, pb = b.type === 'person' ? A[b.id] : null;
      if (!pa && !pb) return;
      /* The overlay does not scroll and the field does, so a node's end takes
         the scroll off and a rail anchor's does not. */
      var x1, y1, x2, y2;
      if (pa) { x1 = pa.x; y1 = pa.y; } else { x1 = onScreen(a).x; y1 = onScreen(a).y; }
      if (pb) { x2 = pb.x; y2 = pb.y; } else { x2 = onScreen(b).x; y2 = onScreen(b).y; }
      var lit = (tid && (e.from === tid || e.to === tid)) ||
                (M.target && M.target.kind === 'edge' && M.target.i === i);
      var out = !(inSubgraph(e.from) && inSubgraph(e.to)) ||
                (pa && pa.off) || (pb && pb.off);
      var g = el('g', { 'class': 'edgehit' });
      g.appendChild(el('line', { x1: x1, y1: y1, x2: x2, y2: y2,
        'class': 'edge ' + (e.cls ? e.cls + ' ' : '') + (lit ? 'lit ' : '') + (out ? 'out' : '') }));
      g.appendChild(el('line', { x1: x1, y1: y1, x2: x2, y2: y2, 'class': 'hit' }));
      g.appendChild(el('title', {})).textContent = e.label;
      g.addEventListener('click', function (ev) {
        ev.stopPropagation();
        M.target = (M.target && M.target.kind === 'edge' && M.target.i === i)
          ? null : { kind: 'edge', i: i };
        render();
      });
      layer.appendChild(g);
    });

    M.people.forEach(function (p) {
      var a = A[p.id];
      if (!a) return;
      var cls = 'anchor' + (p.id === M.ego ? ' ego' : '') + (targetId() === p.id ? ' target' : '') +
                (a.off ? ' off' : '');
      var c = el('circle', { cx: a.x, cy: a.y, r: p.id === M.ego ? 6.5 : 5, 'class': cls });
      c.appendChild(el('title', {})).textContent = (p.name || 'unnamed') + ' ' + p.id;
      c.addEventListener('click', function (ev) { ev.stopPropagation(); pick(p.id); });
      layer.appendChild(c);
    });
  }

  /* PICKING IS TARGETING. The Ego is not a selection — it is whose door the page
     is acting through — so it is set by the rail's single select and nowhere
     else. Selecting a person here makes them the Target. */
  function pick(id) {
    if (!M.byId[id]) return;
    M.target = (targetId() === id) ? null : { kind: 'node', id: id };
    render();
  }

  function objectNode(o) {
    var sel = targetId() === o.id;
    var g = el('g', { 'class': 'node' + (inSubgraph(o.id) ? '' : ' out'),
                      transform: 'translate(' + o.x + ',' + o.y + ')' });
    /* AN ENTITY IS DRAWN LARGER THAN PLUMBING. `kinds[k].node` is the model's
       own word for it: a forum, a connection, a conversation and the system kind
       are `false` — real objects, but not things the graph projects a node for. */
    var shape = (M.icd && M.icd.kinds[M.lens[o.kind]]) || null;
    var plumbing = shape ? !shape.node : false;
    var r = (plumbing ? 8 : 11) + Math.min(8, (o.members || []).length * 1.6);
    if (sel) g.appendChild(el('circle', { r: r + 5, 'class': 'ring target' }));
    if (o.folded) {
      g.appendChild(el('circle', { r: r, 'class': 'obj' + (o.local ? ' local' : ''),
                                   fill: kindColour(o.kind) }));
    } else {
      g.appendChild(el('circle', { r: r, 'class': 'bad' }));
    }
    var label = objectLabel(o) || (o.folded ? o.kind : (o.absent ? 'not held' : 'will not fold'));
    var t1 = el('text', { 'class': 'lbl', y: r + 14 }); t1.textContent = label;
    var t2 = el('text', { 'class': 'lbl small', y: r + 26 });
    t2.textContent = (o.folded ? o.kind + ' · ' : '') + tail(o.id) +
                     (o.spineOf ? ' · spine' : '');
    g.appendChild(t1); g.appendChild(t2);
    g.appendChild(el('title', {})).textContent = o.folded
      ? o.kind + ' ' + o.id : (o.why || '') + ' — ' + o.id;
    o._g = g;
    draggable(g, o);
    g.addEventListener('click', function (ev) {
      ev.stopPropagation();
      if (g._moved) { g._moved = false; return; }
      pick(o.id);
    });
    return g;
  }

  /* Dragging a node. The capture goes on the NODE, so its own click still lands
     — capturing on the <svg> is what made nothing selectable before. Movement is
     divided by the zoom, so a node follows the pointer at any scale. */
  function draggable(g, o) {
    var from = null;
    g.addEventListener('pointerdown', function (ev) {
      ev.stopPropagation();
      from = { x: ev.clientX, y: ev.clientY, ox: o.x, oy: o.y };
      g._moved = false;
      g.setPointerCapture(ev.pointerId);
    });
    g.addEventListener('pointermove', function (ev) {
      if (!from) return;
      var dx = ev.clientX - from.x, dy = ev.clientY - from.y;
      if (Math.abs(dx) > 2 || Math.abs(dy) > 2) g._moved = true;
      o.x = from.ox + dx / T.k;
      o.y = from.oy + dy / T.k;
      o.fixed = true;
      g.setAttribute('transform', 'translate(' + o.x + ',' + o.y + ')');
      refreshEdges();
    });
    ['pointerup', 'pointercancel'].forEach(function (name) {
      g.addEventListener(name, function (ev) {
        from = null;
        try { g.releasePointerCapture(ev.pointerId); } catch (e) {}
      });
    });
  }

  /* Move the lines, do not rebuild them. */
  function refreshEdges() {
    M.edges.forEach(function (e) {
      if (!e._ln) return;
      var a = M.byId[e.from], b = M.byId[e.to];
      if (!a || !b) return;
      [e._ln, e._hit].forEach(function (ln) {
        ln.setAttribute('x1', a.x); ln.setAttribute('y1', a.y);
        ln.setAttribute('x2', b.x); ln.setAttribute('y2', b.y);
      });
    });
    renderPinned();
  }

  function renderLegend() {
    var box = $('legend'); box.textContent = '';
    var seen = {};
    M.objects.forEach(function (o) { if (o.folded) seen[o.kind] = true; });
    Object.keys(seen).sort().forEach(function (kind) {
      var s = document.createElement('span');
      var i = document.createElement('i'); i.style.background = kindColour(kind);
      s.appendChild(i); s.appendChild(document.createTextNode(kind));
      box.appendChild(s);
    });
    if (M.objects.some(function (o) { return !o.folded; })) {
      var s2 = document.createElement('span');
      var i2 = document.createElement('i');
      i2.style.background = 'transparent'; i2.style.border = '1.4px dashed #E5788A';
      s2.appendChild(i2); s2.appendChild(document.createTextNode('will not fold'));
      box.appendChild(s2);
    }
  }

  function renderSelection() {
    var e = M.ego ? (nameOf(M.ego) || 'unnamed') + ' ·' + tail(M.ego) : 'none';
    var t = 'none';
    if (M.target && M.target.kind === 'node') {
      var o = M.byId[M.target.id];
      t = (o.type === 'person' ? (o.name || 'unnamed') : (o.name || o.kind || 'object')) +
          ' ·' + tail(o.id);
    } else if (M.target && M.target.kind === 'edge') {
      var ed = M.edges[M.target.i];
      t = 'edge · ' + (ed ? ed.label : '');
    }
    $('sel').innerHTML = '<b>Ego</b> ' + esc(e) + ' &nbsp; <b>Target</b> ' + esc(t);
  }

  /* ── left rail: people ──────────────────────────────────────────────────── */
  function renderPeople() {
    var list = $('plist'); list.textContent = '';
    if (!M.people.length) {
      list.innerHTML = '<div class="empty">none</div>';
      return;
    }
    var persona = {};
    (M.personas || []).forEach(function (s) {
      persona[(s.pk || '').replace(/^ed25519:/, '')] = s;
    });
    /* ALPHABETICAL, ALWAYS. A list that reorders itself when you pick a row is a
       list you cannot point at twice. */
    M.people.slice().sort(function (a, b) {
      var an = (a.name || '\uffff' + a.id).toLowerCase(), bn = (b.name || '\uffff' + b.id).toLowerCase();
      return an < bn ? -1 : an > bn ? 1 : 0;
    }).forEach(function (p) {
      var row = document.createElement('div');
      row.dataset.id = p.id;
      row.className = 'row' + (M.ego === p.id ? ' on' : '') + (targetId() === p.id ? ' tgt' : '');
      var n = Object.keys(p.member).length;
      var pr = persona[p.id];
      var mine = M.ego === p.id;

      /* THE EGO IS A SINGLE SELECT. One of these is on; picking another opens
         that persona's door and closes nothing else. */
      var sel = document.createElement('button');
      sel.className = 'ego';
      sel.type = 'button';
      sel.setAttribute('role', 'radio');
      sel.setAttribute('aria-checked', mine ? 'true' : 'false');
      sel.setAttribute('aria-label', (p.name || 'unnamed') + ' as ego');
      sel.disabled = !pr;
      sel.addEventListener('click', function (ev) {
        ev.stopPropagation();
        if (pr) actAs(pr);
      });

      var who = document.createElement('button');
      who.className = 'who';
      who.type = 'button';
      who.innerHTML =
        '<div><span class="' + (p.name ? 'nm' : 'nm anon') + '">' + esc(p.name || 'unnamed') +
          '</span>' + (pr && !pr.open ? '<span class="badge">closed</span>' : '') + '</div>' +
        '<div class="tail">·' + esc(tail(p.id)) + '</div>' +
        '<div class="sub">' + n + ' object' + (n === 1 ? '' : 's') +
          (pr ? '' : p.sub ? ' · ' + esc(p.sub) : '') + '</div>';
      who.addEventListener('click', function () { pick(p.id); });

      row.appendChild(sel); row.appendChild(who);
      list.appendChild(row);
    });
  }

  /* ── right rail: the fold ───────────────────────────────────────────────── */
  function renderFold() {
    var body = $('foldbody'), whose = $('foldwhose');
    var me = M.ego;
    whose.textContent = me ? ('as ·' + tail(me)) : '';

    if (M.target && M.target.kind === 'edge') {
      var e = M.edges[M.target.i];
      if (e) {
        body.innerHTML =
          '<dl class="kv"><dt>edge</dt><dd>' + esc(e.label.split(' · from ')[0]) + '</dd>' +
          '<dt>from</dt><dd>' + esc(labelFor(e.from)) + '</dd>' +
          '<dt>to</dt><dd>' + esc(labelFor(e.to)) + '</dd>' +
          '<dt>source</dt><dd>' + (/authored delta/.test(e.label)
            ? 'author_build · probed, not folded' : 'fold_object') + '</dd></dl>';
        return;
      }
    }

    var o = M.byId[targetId()] || M.byId[M.ego];
    if (!o) {
      body.innerHTML = '<div class="empty">nothing selected</div>';
      return;
    }
    if (o.type === 'person') {
      var mine = Object.keys(o.member);
      body.innerHTML =
        '<dl class="kv"><dt>person</dt><dd>' + esc(o.name || 'unnamed') + '</dd>' +
        '<dt>key</dt><dd>' + esc(o.id) + '</dd>' +
        '<dt>objects</dt><dd>' + mine.length + '</dd></dl>' +
        (o.id === M.ego ? '<div class="note">this page is acting as them</div>' : '') +
        '<div class="sect"><div class="cap">member of</div><pre>' +
        esc(mine.map(function (id) {
          var g = M.byId[id];
          return (g.folded ? g.kind : 'unfolded') + '  ' + (g.name || '') + '  ·' + tail(id);
        }).join('\n') || '—') + '</pre></div>';
      return;
    }
    if (!o.folded) {
      body.innerHTML =
        '<dl class="kv"><dt>object</dt><dd>' + esc(o.id) + '</dd>' +
        '<dt>state</dt><dd class="refusal">' + (o.absent ? 'not held here' : 'will not fold') +
        '</dd></dl>' +
        '<div class="sect"><div class="cap">refusal</div>' +
        '<pre class="refusal">' + esc(o.why || 'refused') + '</pre></div>';
      return;
    }
    body.innerHTML =
      '<dl class="kv">' +
      '<dt>kind</dt><dd>' + esc(o.kind) + (o.spineOf ? ' · a spine' : '') + '</dd>' +
      '<dt>name</dt><dd>' + esc(o.name || '—') + '</dd>' +
      '<dt>object</dt><dd>' + esc(o.id) + '</dd>' +
      '<dt>owner</dt><dd>' + esc((nameOf(o.owner) || '') + ' ·' + tail(o.owner)) + '</dd>' +
      '<dt>roster</dt><dd>' + o.members.length + '</dd>' +
      (o.local ? '<dt>deltas</dt><dd>' + o.deltas + '</dd><dt>published</dt><dd>no</dd>' : '') +
      (o.wallet ? '<dt>wallet</dt><dd>' + esc(o.wallet) + '</dd>' : '') +
      '</dl>' +

      '<div class="sect"><div class="cap">the lens view</div><pre>' +
      esc(JSON.stringify(o.view, null, 1)) + '</pre></div>';
  }

  /* A CONNECTION IS NAMED FROM WHERE YOU STAND. Each door answers with the other
     party as IT sees them, and the union keeps whichever door answered first —
     so Bo's connection with Ada was labelled "Bo". Name it by the member who is
     not the Ego. */
  function objectLabel(o) {
    if (o.peer && M.ego) {
      var other = null;
      (o.members || []).forEach(function (m) { if (m !== M.ego) other = m; });
      if (other) return nameOf(other) || tail(other);
    }
    return o.name;
  }

  function labelFor(id) {
    var n = M.byId[id];
    if (!n) return tail(id);
    if (n.type === 'person') return (n.name || 'unnamed') + ' ·' + tail(id);
    return (objectLabel(n) || n.kind || 'object') + ' ·' + tail(id);
  }

  /* THE PAIRWISE LOG between two people, if they hold one. Found by the `peer`
     the core's connection reader names, not by a kind string kept here. */
  /* THE PAIRWISE LOG between two people, if they hold one. A pair holds two: the
     CHANNEL carries their prekeys and cards, the CHAT carries what they said. The
     chat is what one person does to another, so it wins; the channel is still its
     own node and can be targeted directly. */
  function pairObject(a, b) {
    var channel = null, chat = null;
    M.objects.forEach(function (o) {
      if (!o.peer) return;
      var m = o.members || [];
      if (m.indexOf(a) < 0 || m.indexOf(b) < 0) return;
      if (o.role === 'chat') chat = o; else channel = o;
    });
    return chat || channel;
  }

  /* Who the Ego may pair with right now, or why not. */
  function pairing() {
    var t = M.byId[targetId()];
    if (!t || t.type !== 'person' || t.id === M.ego) return { why: 'a person' };
    if (pairObject(M.ego, t.id)) return { why: 'connected' };
    var row = null;
    (M.personas || []).forEach(function (r) {
      if ((r.pk || '').replace(/^ed25519:/, '') === t.id) row = r;
    });
    return row ? { to: row, name: t.name || tail(t.id) } : { why: 'not a door here' };
  }

  /* ── bottom rail, left: what the Ego may create ─────────────────────────── */
  function renderCreate() {
    var box = $('creates'); box.textContent = '';
    var who = $('createwho');
    if (!M.ego) {
      box.innerHTML = '<div class="empty">no ego</div>';
      if (who) who.textContent = '';
      return;
    }
    if (who) who.textContent = 'as ·' + tail(M.ego);
    var specs = creatable();
    if (!specs.length) { box.innerHTML = '<div class="empty">no kinds</div>'; return; }
    specs.forEach(function (spec) {
      var b = document.createElement('button');
      b.className = 'mk';
      var ok = false, why = spec.kind + (spec.parts.length ? ' + ' + spec.parts.join(' ') : '');
      if (spec.how === 'mint') {
        ok = true;
      } else if (spec.how === 'paired') {
        /* Two people make it, so it takes the Target rather than a sheet. */
        var pair = pairing();
        ok = !!pair.to;
        why = spec.kind + ' · ' + (pair.to ? pair.name : 'paired · ' + pair.why);
      } else {
        /* `derived` — the core says how it is formed and no door does it yet. */
        why = spec.kind + ' · ' + spec.how + ' · no door';
      }
      b.disabled = !ok;
      b.innerHTML = '<span class="op">+ ' + esc(spec.label) + '</span>' +
                    '<span class="why">' + esc(why) + '</span>';
      b.addEventListener('click', function () {
        if (spec.how === 'paired') connectTo(pairing().to);
        else if (spec.how === 'mint') openSheet(spec);
      });
      box.appendChild(b);
    });
  }

  /* ── bottom rail, right: what the Ego may do to the Target ──────────────── */
  function opKinds(name) {
    if (M.opsOn[name]) return M.opsOn[name];
    var bare = name.indexOf('.') >= 0 ? name.slice(name.indexOf('.') + 1) : name;
    if (M.opsOn[bare]) return M.opsOn[bare];
    return null;
  }

  /* THE MODEL, INDEXED. `channels[<channel>].messages` names the ops a channel
     carries; `components.messages[<op>]` is the op — `x-delta` gives the opId,
     the authority and the fold, and `payload.required` gives the args it will
     not go without. Nothing here is restated: this reads the document. */
  function indexIcd(doc) {
    var out = { kinds: {}, ops: {}, relations: doc.relations || {} };

    /* AN OP IS DECLARED WHERE IT BELONGS. A kind's own ops sit under the kind;
       the ops several kinds share sit once under their facet, which names the
       kinds that carry it. Reading both into one table is all the page needs to
       know about the difference — and the FACET is what it groups by, because
       "hosting" and "wallet" say something and "base" never did. */
    function take(name, m, group) {
      out.ops[name] = {
        name: name,
        group: group,
        opId: m.op,
        ego: m.ego,                 /* owner | member — what the Ego must be */
        fold: m.fold,
        args: m.args || {},
        required: Object.keys(m.args || {}).filter(function (a) {
          return (m.args[a] || {}).required;
        }),
        summary: m.summary || ''
      };
    }

    Object.keys(doc.kinds || {}).forEach(function (k) {
      var kind = doc.kinds[k] || {};
      out.kinds[k] = { node: !!kind.node, parts: kind.parts || [], ops: [] };
      Object.keys(kind.ops || {}).forEach(function (name) {
        take(name, kind.ops[name], k);
        out.kinds[k].ops.push(name);
      });
    });
    Object.keys(doc.facets || {}).forEach(function (f) {
      var facet = doc.facets[f] || {};
      Object.keys(facet.ops || {}).forEach(function (name) {
        take(name, facet.ops[name], f);
        (facet.on || []).forEach(function (k) {
          if (out.kinds[k]) out.kinds[k].ops.push(name);
        });
      });
    });
    Object.keys(out.kinds).forEach(function (k) { out.kinds[k].ops.sort(); });
    return out;
  }

  /* THE OBJECT THE ACTIONS ARE ABOUT. A person Target resolves to the pairwise
     log the two of them hold: between two people there is no other log to write
     to, and the ops legal on it are the ops legal between them. */
  function actionObject() {
    var t = M.byId[targetId()];
    if (!t) return null;
    if (t.type === 'object') return t;
    if (t.type === 'person' && M.ego && t.id !== M.ego) return pairObject(M.ego, t.id);
    return null;
  }

  /* ONLY WHAT IS LEGAL, read off the ICD.
     The channel comes from the core (`authoring::channel_of`), because the model
     is keyed by channel and the directory speaks kind strings. Everything else —
     which ops the channel carries, each one's authority and fold — is the
     document's own answer. Two gates are the Ego's: the roster, and the owner on
     an owner-sequenced op. One fact the ICD does not carry is whether any door
     writes the op at all, so an op the core would refuse is SHOWN AS REFUSED
     rather than dropped — the document and the code disagreeing is a thing to
     see, not to hide. */
  function renderActions() {
    var box = $('acts'), count = $('actcount');
    box.textContent = ''; count.textContent = '';
    function none(why) { box.innerHTML = '<div class="empty">' + esc(why) + '</div>'; }

    if (!M.icd) { none('no icd'); return; }
    var p = M.ego && M.byId[M.ego];
    if (!p) { none('no ego'); return; }
    var t = M.byId[targetId()];
    if (!t) { none('no target'); return; }
    var o = actionObject();
    if (!o) { none(t.type === 'person' ? 'no shared object' : 'no target'); return; }
    if (!o.folded) { none(o.absent ? 'not held here' : 'will not fold'); return; }

    var lens = M.lens[o.kind];
    var k = M.icd.kinds[lens];
    if (!k) { none('the model names no kind ' + (lens || o.kind)); return; }

    var isOwner = p.id === o.owner, isMember = o.members.indexOf(p.id) >= 0 || isOwner;
    count.textContent = o.kind + ' \u2192 ' + lens + ' \u00b7' + tail(o.id);
    if (!isMember) { none('not a member'); return; }

    var by = {}, n = 0, refused = 0;
    k.ops.forEach(function (name) {
      var op = M.icd.ops[name];
      if (!op) return;
      /* EGO AND TARGET, as the model now says it: `ego` is what the actor must
         BE to the target. The page invented these two words; the document speaks
         them, so this is a read rather than a translation. */
      if (op.ego === 'owner' && !isOwner) return;
      /* The one fact the model does not state: `authoring::catalogue` leaves out
         every op no door writes. */
      var kinds = opKinds(name);
      op.writable = !!kinds && kinds.indexOf(o.kind) >= 0;
      if (op.writable) { n++; } else { refused++; }
      (by[op.group] || (by[op.group] = [])).push(op);
    });
    if (!n && !refused) { none('none legal on ' + o.kind); return; }

    /* GROUPED BY FACET, not by the op name's prefix. `hosting` and `wallet` say
       what a group of ops is FOR; `base` — which is what the prefix gave — said
       only that several kinds share them. */
    Object.keys(by).sort().forEach(function (group) {
      var h = document.createElement('div');
      h.className = 'grp';
      h.textContent = group;
      box.appendChild(h);
      by[group].forEach(function (op) {
        var d = document.createElement('div');
        d.className = 'act' + (op.writable ? '' : ' refused');
        d.title = op.summary;
        var short = op.name.indexOf('.') > 0 ? op.name.slice(op.name.indexOf('.') + 1) : op.name;
        /* WHAT IT DOES TO THE GRAPH, from the arg that carries the reference. The
           model declares the relation on the arg, so this is read rather than
           guessed — and it is known BEFORE the op is authored, which folding
           could never tell you. */
        var rels = [];
        Object.keys(op.args).forEach(function (a) {
          ((op.args[a] || {}).rel || []).forEach(function (r) {
            if (rels.indexOf(r.name) < 0) rels.push(r.name);
          });
        });
        d.innerHTML = '<span class="op">' + esc(short) + '</span>' +
          '<span class="why">' + esc(op.ego + ' \u00b7 ' + op.fold +
            (op.required.length ? ' \u00b7 ' + op.required.join(' ') : '') +
            (rels.length ? ' \u00b7 ' + rels.join(' ') : '') +
            (op.writable ? '' : ' \u00b7 no door writes it')) + '</span>';
        box.appendChild(d);
      });
    });
    count.textContent = n + (refused ? ' of ' + (n + refused) : '') +
      ' \u00b7 ' + o.kind + ' \u2192 ' + lens;
  }

  /* ── the panel scrolls ──────────────────────────────────────────────────
     X BELONGS TO THE SCROLLBAR. The field is as wide as the spine needs and the
     panel scrolls it; the overlay that holds the rail anchors does not scroll,
     so an anchor stays on its row while the vertebrae move under it.
     Y is the Ego's row, not a pan — see `egoRowY`. */
  var T = { x: CELL.w * 0.15, y: 0, k: 1 };
  var wrap = $('canvasWrap'), scroller = $('scroller');

  function applyT() {
    T.y = egoRowY();
    $('view').setAttribute('transform',
      'translate(' + T.x + ',' + T.y + ') scale(' + T.k + ')');
    var e = extent();
    $('g').setAttribute('width', Math.max(wrap.clientWidth, e.w * T.k + T.x));
    renderPinned();
  }

  /* Fit zooms out until the whole field is on screen; at k = 1 it scrolls. */
  function fit() {
    var e = extent();
    T.k = Math.min(1, (wrap.clientWidth - T.x) / Math.max(e.w, 1));
    scroller.scrollLeft = 0;
    applyT();
  }

  var svg = $('g'), drag = null, moved = false;
  /* NO POINTER CAPTURE. Capturing the pointer on the <svg> retargets everything
     that follows to the <svg> — including the CLICK — so a node's own handler
     never runs and nothing is selectable. `moved` tells a drag from a click. */
  svg.addEventListener('pointerdown', function (e) {
    drag = { x: e.clientX, left: scroller.scrollLeft }; moved = false;
    svg.classList.add('drag');
  });
  svg.addEventListener('pointermove', function (e) {
    if (!drag) return;
    var dx = e.clientX - drag.x;
    if (Math.abs(dx) > 3) moved = true;
    scroller.scrollLeft = drag.left - dx;
  });
  ['pointerup', 'pointercancel', 'pointerleave'].forEach(function (t) {
    svg.addEventListener(t, function () { drag = null; svg.classList.remove('drag'); });
  });
  /* A plain wheel scrolls — that is the panel's own gesture now. Zoom is the
     browser's modifier, so a trackpad swipe is never swallowed. */
  svg.addEventListener('wheel', function (e) {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
    T.k = Math.max(0.2, Math.min(3, T.k * Math.exp(-e.deltaY * 0.0016)));
    applyT();
  }, { passive: false });
  scroller.addEventListener('scroll', renderPinned);

  /* Background click drops the Target. Not the Ego: that is an open door, and
     the rail's single select is the only thing that moves it. */
  svg.addEventListener('click', function () {
    if (moved) { moved = false; return; }
    if (M.target) { M.target = null; render(); }
  });

  /* ── wiring ─────────────────────────────────────────────────────────────── */
  $('mint').addEventListener('click', openPersonSheet);
  $('clear').addEventListener('click', clearAll);
  $('sheetcancel').addEventListener('click', closeSheet);
  $('modal').addEventListener('click', function (e) { if (e.target.id === 'modal') closeSheet(); });
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape' && !$('modal').hidden) closeSheet();
  });
  $('fit').addEventListener('click', fit);
  window.addEventListener('resize', applyT);
  /* The rail scrolls the Ego's row, and the spine is level with it. */
  $('people').addEventListener('scroll', applyT);

  /* the right rail's two tabs: the fold, and every crossing into the core */
  function tab(which) {
    var core = which === 'core';
    $('tabcore').classList.toggle('on', core);
    $('tabfold').classList.toggle('on', !core);
    $('logpane').hidden = !core;
    $('foldbody').hidden = core;
    if (core) renderLog();
  }
  $('tabfold').addEventListener('click', function () { tab('fold'); });
  $('tabcore').addEventListener('click', function () { tab('core'); });

  /* ── boot ───────────────────────────────────────────────────────────────── */
  /* ONE FAILURE IS ONE FAILURE. Each of these is asked for separately and logged
     where it breaks: a missing export used to take the whole boot with it, and
     the page came up with no colours, no model and nothing saying why. */
  function ask(name, fn) {
    try {
      if (typeof fn !== 'function') throw new Error(name + ' is not exported by this module');
      return JSON.parse(call(name, fn));
    } catch (e) {
      fail('core-wasm ' + name, e.message);
      return null;
    }
  }

  account.core().then(function (exports) {
    CORE = exports;
    M.opsOn = ask('ops_on', CORE.ops_on) || {};
    /* The map between the ICD's channels and the directory's kind strings, from
       `authoring::channel_of` — the same dispatch the door makes. */
    M.lens = ask('lenses', CORE.lenses) || {};
    var kinds = {};
    Object.keys(M.opsOn).forEach(function (op) {
      (M.opsOn[op] || []).forEach(function (k) { kinds[k] = true; });
    });
    M.kinds = Object.keys(kinds).sort();
    M.kinds.forEach(function (k, i) { M.colour[k] = PALETTE[i % PALETTE.length]; });
    render();
    /* THE MODEL, fetched. The panel reads the ICD itself rather than a build-time
       copy of it, so what it offers is what the document says today. */
    return door('/v1/icd', { session: null })
      .then(function (icd) { M.icd = indexIcd(icd); render(); })
      .catch(function (e) { fail('door /v1/icd', e.message); })
      .then(function () { return hello().catch(function (e) { fail('door', e.message); }); });
  }).catch(function (e) {
    fail('core-wasm', e.message);
  });
})();
