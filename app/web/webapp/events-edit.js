/* events-edit.js — W-98 Events: an event's editor (EVENTS-UI-A), for a new event and for Manage,
   in place of the Site's sheet. Drawn by webapp.js's render hook (S.view 'event-edit', S.sel the
   event or null) with eventsCtx(), and reading the ICD through events-icd.js.

   Ralph's editor rule: a rail of numbered sections, basics first, each opening its detail beside a
   live preview; on a phone the rail is a stepper and the preview a toggle; Publish (new) or Save
   (Manage) always in reach.

   WRITES. A new event is one /v2/batch: the mint, event.setProfile whole, the Site's
   group.setAffiliation {rel: created} and the event's base.setBacklink at one moment (NC-81, as
   the sheet did), then its media, lineup, place and the Site's registration. The
   owner saves with event.setProfile carrying every held field, those not shown too; a co-host with
   event.editProfile. `gen` is the Door's to add to a commutative op. Tickets are a link
   (ticketUrl), never event.setTickets. A refusal is the Door's own sentence. The decisions on
   pending answers are written as they are chosen. No visibility and no co-hosts in R3 (MANAGE's
   go, 1 Oct): an event's roster is its creator alone, so neither has a route; every event is the
   facet's default. */
(function (root) {
  var X = function () { return root.WallFlowers.EventsIcd; };
  var drafts = {};
  var SECTIONS = [
    ['basics', 'Basics'], ['when', 'When'], ['where', 'Where'], ['details', 'Details'], ['lineup', 'Lineup'],
    ['media', 'Media'], ['tickets', 'Tickets'], ['registration', 'Registration'],
    ['status', 'Status', true], ['guests', 'Guests', true]
  ];
  var WORD = { DAILY: 'Daily', WEEKLY: 'Weekly', MONTHLY: 'Monthly', YEARLY: 'Yearly' };

  function hex16() {
    var a = new Uint8Array(8), c = root.crypto;
    if (c && c.getRandomValues) c.getRandomValues(a); else for (var i = 0; i < 8; i++) a[i] = Math.floor(Math.random() * 256);
    return Array.prototype.map.call(a, function (b) { return (b < 16 ? '0' : '') + b.toString(16); }).join('');
  }
  /* A time as its wall clock reads in zone `tz` (the event's), and back: the fields say the
     event's own time, wherever the editor is. No zone, or one Intl does not know: the reader's. */
  function wall(ms, tz) {
    var o = { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }, p = {};
    try { if (tz) o.timeZone = tz; new Intl.DateTimeFormat('en-US', o).formatToParts(new Date(ms)).forEach(function (x) { p[x.type] = x.value; }); }
    catch (e) { return null; }
    return { y: +p.year, m: +p.month, d: +p.day, h: +p.hour % 24, mi: +p.minute };
  }
  function localInput(ms, tz) {
    if (ms == null || ms === '') return '';
    var w = wall(ms, tz) || wall(ms), z = function (n) { return (n < 10 ? '0' : '') + n; };
    return w.y + '-' + z(w.m) + '-' + z(w.d) + 'T' + z(w.h) + ':' + z(w.mi);
  }
  function parseInput(v, tz) {
    var m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/.exec(v || '');
    if (!m) return null;
    var want = Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]);
    if (!tz || !wall(want, tz)) { var t = Date.parse(v); return isNaN(t) ? null : t; }
    /* The instant whose wall clock in tz reads `want`: the offset there, twice for a DST edge. */
    var at = want;
    for (var i = 0; i < 2; i++) { var w = wall(at, tz); at = want - (Date.UTC(w.y, w.m - 1, w.d, w.h, w.mi) - at); }
    return at;
  }
  function localTz() { try { return Intl.DateTimeFormat().resolvedOptions().timeZone || ''; } catch (e) { return ''; } }
  function zones(held) {
    var z = [];
    try { z = Intl.supportedValuesOf ? Intl.supportedValuesOf('timeZone').slice() : []; } catch (e) { z = []; }
    if (held && z.indexOf(held) < 0) z.unshift(held);
    return z;
  }
  function cap(w) { w = String(w == null ? '' : w); return w.charAt(0).toUpperCase() + w.slice(1); }
  /* The page's own words, where the page is loaded: its formatter and its labels. */
  function page() { return root.WallFlowers && root.WallFlowers.EventsPage; }
  function whenText(p) {
    if (!p.startMs) return '';
    var o = { dateStyle: 'medium', timeStyle: 'short' };
    try { if (p.tz) o.timeZone = p.tz; return new Intl.DateTimeFormat(undefined, o).format(new Date(p.startMs)); }
    catch (e) { return new Date(p.startMs).toLocaleString(); }
  }

  /* The Site this event is made on: the page's, or the one whose `created` names it. */
  function siteFor(ctx, ev) {
    var M = ctx.M, s = ctx.S.site && M.byId[ctx.S.site];
    if (!ev) return s || null;
    var names = function (o) { return ((o.view || {}).affiliations || []).some(function (a) { return a.peer === ev.id && a.rel === 'created'; }); };
    if (s && names(s)) return s;
    return Object.keys(M.byId).map(function (k) { return M.byId[k]; }).filter(function (o) { return o.kind === 'group' && names(o); })[0] || s || null;
  }

  /* What the editor holds: a new event's blanks, or Manage's copy of the event as folded. */
  function init(icd, ev, site) {
    var v = (ev && ev.view) || {};
    var profile = ev ? X().heldProfile(icd, v) : { tz: localTz() };
    if (!profile.tz) profile.tz = localTz();
    var reg = site && ev ? X().registrationOf(icd, site, ev.id) : null;
    var m = X().mediaOf(v);
    return {
      step: 'basics', preview: false, why: '', mediaWhy: '',
      profile: profile, rule: profile.recurrence || '',
      choice: X().parseRule(icd, profile.recurrence || '') || { freq: '', interval: 1, byday: [] },
      ends: v.recurrence && /COUNT=/.test(v.recurrence) ? 'count' : v.recurrence && /UNTIL=/.test(v.recurrence) ? 'until' : 'never',
      acts: X().actsOf(v), actsChanged: false,
      banner: null, photos: m.photos.map(function (p) { return { id: p.id, mime: p.mime, data: p.data, held: true }; }), removed: [], clip: null,
      reg: reg ? JSON.parse(JSON.stringify(reg)) : {}, regHeld: reg ? JSON.stringify(reg) : '{}',
      place: ''
    };
  }

  /* ── media: read, drawn smaller in the browser to the ICD's cap, or refused ───────── */
  function readFile(file) {
    return new Promise(function (ok, no) {
      var r = new root.FileReader();
      r.onload = function () { ok(r.result); };
      r.onerror = function () { no(r.error); };
      r.readAsDataURL(file);
    });
  }
  function smaller(url, cap) {
    var b64 = function (u) { return String(u).split(',')[1] || ''; };
    if (b64(url).length <= cap || !root.Image || !root.document || !/^data:image\//.test(url)) return Promise.resolve(url);
    return new Promise(function (ok) {
      var img = new root.Image();
      img.onload = function () {
        var side = Math.max(img.naturalWidth, img.naturalHeight), out = url;
        [2048, 1600, 1280, 1024, 768, 512].some(function (s) {
          var k = Math.min(1, s / side), c = root.document.createElement('canvas');
          c.width = Math.round(img.naturalWidth * k); c.height = Math.round(img.naturalHeight * k);
          c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
          out = c.toDataURL('image/jpeg', 0.85);
          return b64(out).length <= cap;
        });
        ok(out);
      };
      img.onerror = function () { ok(url); };
      img.src = url;
    });
  }
  function takeMedia(icd, op, prefix, file) {
    var cap = X().mediaCaps(icd, op, prefix).maxLength;
    return readFile(file).then(function (url) { return smaller(url, cap); }).then(function (url) {
      var m = /^data:([^;,]+);base64,(.*)$/.exec(url) || [], data = m[2] || '';
      if (!data) throw new Error(file.name);
      if (cap != null && data.length > cap) throw new Error(file.name + ': ' + data.length + ' > ' + cap);
      return { mime: m[1], data: data };
    });
  }

  /* ── the writes ─────────────────────────────────────────────────────────── */
  function profileArgs(icd, st) {
    var a = {}, p = st.profile, declared = X().args(icd, 'event.setProfile');
    Object.keys(p).forEach(function (k) { if (k in declared && p[k] !== '' && p[k] != null) a[k] = p[k]; });
    var rule = st.rule;
    if (st.choice && st.choice.freq && !st.held) {
      var c = { freq: st.choice.freq, interval: st.choice.interval, byday: st.choice.freq === 'WEEKLY' ? st.choice.byday : [] };
      if (st.ends === 'count') c.count = st.choice.count;
      if (st.ends === 'until') c.until = st.choice.until;
      rule = X().ruleFrom(icd, c);
    } else if (!st.held) rule = '';
    if (rule) a.recurrence = rule; else delete a.recurrence;
    return a;
  }
  function mediaSteps(icd, st, object) {
    var steps = [], now = Date.now();
    if (st.banner) steps.push({ do: 'apply', object: object, op: 'event.setBanner', args: { banner: st.banner.data, bannerMime: st.banner.mime } });
    st.photos.filter(function (p) { return !p.held; }).forEach(function (p) {
      steps.push({ do: 'apply', object: object, op: 'event.addPhoto', args: { id: p.id, photo: p.data, photoMime: p.mime, at: now } });
    });
    st.removed.forEach(function (id) { steps.push({ do: 'apply', object: object, op: 'event.removePhoto', args: { id: id } }); });
    if (st.clip) steps.push({ do: 'apply', object: object, op: 'event.setClip', args: { clip: st.clip.data, clipMime: st.clip.mime } });
    return steps.filter(function (s) { return X().opServed(icd, s.op); });
  }
  function lineupStep(icd, st, object) {
    if (!st.actsChanged || !X().opServed(icd, 'event.setLineup')) return [];
    return [{ do: 'apply', object: object, op: 'event.setLineup', args: { acts: X().actsArg(icd, st.acts) } }];
  }
  function regStep(icd, st, site, event) {
    if (!site || JSON.stringify(st.reg) === st.regHeld || !X().opServed(icd, 'group.setRegistration')) return [];
    var a = { event: event }, declared = X().args(icd, 'group.setRegistration');
    Object.keys(st.reg).forEach(function (k) { if (k in declared && k !== 'event' && k !== 'gen' && st.reg[k] !== '' && st.reg[k] != null) a[k] = st.reg[k]; });
    return [{ do: 'apply', object: site.id, op: 'group.setRegistration', args: a }];
  }
  function missing(icd, args, op) {
    var d = X().args(icd, op);
    return Object.keys(d).filter(function (k) { return d[k].required && k !== 'gen' && (args[k] == null || args[k] === ''); });
  }

  function publish(ctx, st, site, redraw) {
    var icd = ctx.icd, prof;
    try { prof = profileArgs(icd, st); } catch (e) { st.why = String(e.message || e); return redraw(); }
    var need = missing(icd, prof, 'event.setProfile');
    if (need.length) { st.why = need.join(', '); return redraw(); }
    var at = Date.now(), me = { $step: 0 };
    var aff = {}, declared = X().args(icd, 'group.setAffiliation');
    Object.keys(declared).forEach(function (k) {
      if ((declared[k].rel || []).length) aff[k] = me;
      else if (k === 'rel') aff[k] = 'created';
      else if (k === 'name') aff[k] = prof.title;
      else if (k === 'at') aff[k] = at;
    });
    var steps = [
      { do: 'mint', kind: 'event', draft: { name: prof.title, start_ms: prof.startMs } },
      { do: 'apply', object: me, op: 'event.setProfile', args: prof },
      { do: 'apply', object: site.id, op: 'group.setAffiliation', args: aff },
      { do: 'apply', object: me, op: 'base.setBacklink', args: { object: site.id, rel: 'created', at: at } }
    ];
    var linked = steps.length;
    try { steps = steps.concat(mediaSteps(icd, st, me), lineupStep(icd, st, me), regStep(icd, st, site, me)); }
    catch (e) { st.why = String(e.message || e); return redraw(); }
    return go(ctx, st, steps, redraw, function (made, refused) {
      if (!refused) return null;
      var why = String(refused.why || '');
      if (refused.step === linked - 1) return 'the Site names it, and it does not name the Site: ' + why;
      if (refused.step >= 1 && refused.step < linked - 1) return 'minted, not linked: ' + why;
      return why;
    });
  }
  function save(ctx, st, ev, site, redraw) {
    var icd = ctx.icd, me = ctx.M.me.pk, op = X().editOp(icd, ev, me), prof;
    if (!op) return;
    try { prof = profileArgs(icd, st); } catch (e) { st.why = String(e.message || e); return redraw(); }
    var need = missing(icd, prof, op);
    if (need.length) { st.why = need.join(', '); return redraw(); }
    var steps = [{ do: 'apply', object: ev.id, op: op, args: prof }];
    if (st.place && X().may(icd, 'event.setVenue', ev, me)) {
      var pl = ctx.M.byId[st.place];
      steps.push({ do: 'apply', object: ev.id, op: 'event.setVenue', args: { place: st.place, name: (pl && pl.name) || '', at: Date.now() } });
    }
    try { steps = steps.concat(mediaSteps(icd, st, ev.id), lineupStep(icd, st, ev.id), regStep(icd, st, site, ev.id)); }
    catch (e) { st.why = String(e.message || e); return redraw(); }
    return go(ctx, st, steps, redraw, function (made, refused) { return refused ? String(refused.why || '') : null; });
  }
  function go(ctx, st, steps, redraw, said) {
    st.busy = true; st.why = ''; redraw();
    return ctx.batch(steps).then(function (r) {
      var why = said(r.made, r.refused);
      if (why) { st.busy = false; st.why = why; if (r.made && r.made.length) ctx.load(); return redraw(); }
      var id = r.made[0];
      Object.keys(drafts).forEach(function (k) { if (drafts[k] === st) delete drafts[k]; });
      return ctx.load().then(function () { ctx.open('object', id); });
    }, function (e) { st.busy = false; st.why = String(e && e.message || e); redraw(); });
  }
  /* A write chosen in a Manage section, at once: its refusal is the Door's sentence. */
  function now(ctx, st, object, op, args, redraw) {
    st.why = '';
    return ctx.door('/v2/apply', { method: 'POST', body: { object: object, op: op, args: args } })
      .then(function () { return ctx.load(); }, function (e) { st.why = String(e && e.message || e); redraw(); });
  }

  /* ── drawing ────────────────────────────────────────────────────────────── */
  function draw(f, id, ctx) {
    var el = ctx.el, icd = ctx.icd, M = ctx.M, me = M.me.pk;
    var ev = id ? M.byId[id] : null, site = siteFor(ctx, ev);
    var key = id || 'new:' + (site ? site.id : '');
    var st = drafts[key] || (drafts[key] = init(icd, ev, site));
    st.held = !!st.rule && !X().parseRule(icd, st.rule);
    var manage = !!ev, redraw = function () { draw(f, id, ctx); };
    f.textContent = '';
    if (!site && !manage) return;
    var p = st.profile, sections = SECTIONS.filter(function (s) { return manage || !s[2]; });

    function field(label, control) { return el('label', { class: 'ee-f' }, [el('span', { text: label }), control]); }
    function input(name, value, type, onv) {
      return el('input', { name: name, type: type || 'text', value: value == null ? '' : String(value),
        oninput: function (e) { onv(e.target.value); preview(); } });
    }
    function text(name, label, arg, type) {
      var d = X().args(icd, 'event.setProfile')[arg] || {};
      var n = input(name, p[arg], type, function (v) { p[arg] = v; });
      if (d.maxLength) n.setAttribute('maxlength', String(d.maxLength));
      return field(label, n);
    }
    function when(name, label, arg) {
      return field(label, input(name, localInput(p[arg], p.tz), 'datetime-local', function (v) { p[arg] = parseInput(v, p.tz); }));
    }
    function select(name, options, value, onc, disabled) {
      var s = el('select', { name: name, disabled: disabled ? 'true' : null, onchange: function (e) { onc(e.target.value); } },
        options.map(function (o) { return el('option', { value: o[0], text: o[1], selected: o[0] === value ? 'true' : null }); }));
      s.value = value == null ? '' : value;
      return s;
    }
    function pick(name, accept, multiple, label, onfile) {
      var inp = el('input', { name: name, type: 'file', accept: accept, multiple: multiple ? 'true' : null, class: 'ee-file', onchange: onfile });
      return el('span', { class: 'ee-pick' }, [inp, el('button', { type: 'button', class: 'febtn', 'data-act': 'pick-' + name, text: label,
        onclick: function () { if (inp.click) inp.click(); } })]);
    }
    function still(m) { return m && m.data ? el('img', { class: 'ee-still', src: 'data:' + m.mime + ';base64,' + m.data, alt: '' }) : null; }
    function nameOf(hex) {
      var prof = ((site && site.view && site.view.profiles) || {})[hex];
      var o = M.byId[hex];
      return (prof && prof.name) || M.names[hex] || (o && ((o.view || {}).display_name || o.name)) || hex.slice(0, 8);
    }

    var body = {};
    body.basics = [
      text('title', 'Title', 'title'),
      field('Cover', el('div', { class: 'ee-cover' }, [still(st.banner || (ev && ev.view && ev.view.banner)),
        pick('banner', 'image/*', false, st.banner || (ev && ev.view && ev.view.banner && ev.view.banner.data) ? 'Replace' : 'Choose', function (e) {
          var file = e.target.files && e.target.files[0]; if (!file) return;
          takeMedia(icd, 'event.setBanner', 'banner', file).then(function (m) { st.banner = m; st.mediaWhy = ''; redraw(); },
            function (err) { st.mediaWhy = String(err.message || err); redraw(); });
        })])),
      st.mediaWhy && st.step === 'basics' ? el('p', { class: 'ee-why', 'data-why-media': '', text: st.mediaWhy }) : null
    ];
    var freqs = [['', 'None']].concat(X().frequencies(icd).map(function (fq) { return [fq, WORD[fq] || fq]; }));
    if (st.held) freqs.push([st.rule, st.rule]);
    body.when = [
      when('startMs', 'Start', 'startMs'), when('endMs', 'End', 'endMs'),
      zones(p.tz).length ? field('Time zone', select('tz', zones(p.tz).map(function (z) { return [z, z.replace(/_/g, ' ')]; }), p.tz, function (v) {
        ['startMs', 'endMs'].forEach(function (k) { if (p[k] != null) p[k] = parseInput(localInput(p[k], p.tz), v); });
        p.tz = v; redraw(); })) : text('tz', 'Time zone', 'tz'),
      field('Repeat', select('freq', freqs, st.held ? st.rule : st.choice.freq || '', function (v) { st.choice.freq = v; redraw(); }, st.held))
    ];
    if (st.choice.freq && !st.held) {
      body.when.push(field('Every', input('interval', st.choice.interval, 'number', function (v) { st.choice.interval = parseInt(v, 10) || 1; })));
      if (st.choice.freq === 'WEEKLY') {
        var days = X().weekdayNames(icd);
        body.when.push(el('div', { class: 'ee-days' }, X().weekdays(icd).map(function (d) {
          var on = st.choice.byday.indexOf(d) >= 0;
          return el('button', { type: 'button', 'data-act': 'day-' + d, 'aria-pressed': on ? 'true' : 'false', class: on ? 'on' : '', text: days[d] || d,
            onclick: function () { var i = st.choice.byday.indexOf(d); if (i >= 0) st.choice.byday.splice(i, 1); else st.choice.byday.push(d); redraw(); } });
        })));
      }
      body.when.push(field('Ends', select('ends', [['never', 'Never'], ['count', 'After'], ['until', 'On']], st.ends, function (v) { st.ends = v; redraw(); })));
      if (st.ends === 'count') body.when.push(field('Times', input('count', st.choice.count, 'number', function (v) { st.choice.count = parseInt(v, 10); })));
      if (st.ends === 'until') body.when.push(field('Until', input('until', localInput(st.choice.until, p.tz), 'datetime-local', function (v) { st.choice.until = parseInput(v, p.tz); })));
    }
    var places = Object.keys(M.byId).map(function (k) { return M.byId[k]; }).filter(function (o) { return o.kind === 'place'; });
    body.where = [text('venue', 'Venue', 'venue'), text('online', 'Online', 'online', 'url'),
      manage && places.length && X().opServed(icd, 'event.setVenue') ? field('Place', select('place', [['', '—']].concat(places.map(function (o) { return [o.id, o.name || o.id.slice(0, 8)]; })), st.place, function (v) { st.place = v; })) : null];
    body.details = [field('Description', el('textarea', { name: 'descriptor', rows: '6', text: p.descriptor || '', oninput: function (e) { p.descriptor = e.target.value; } }))];

    /* The lineup: profiles only, people from the Site's cards, groups by their own object. */
    var people = Object.keys((site && site.view && site.view.profiles) || {});
    var groups = Object.keys(M.byId).filter(function (k) { var o = M.byId[k]; return o.kind === 'group' && (!site || k !== site.id) && people.indexOf(k) < 0; });
    var pickable = people.concat(groups).filter(function (h) { return !st.acts.some(function (a) { return (a.member || a.object) === h; }); });
    var roles = X().actRoles(icd).map(function (r) { return [r, cap(r)]; });
    body.lineup = st.acts.map(function (a, i) {
      return el('div', { class: 'ee-act' }, [
        el('div', { class: 'r1' }, [
          el('span', { class: 'nm', text: nameOf(a.member || a.object) }),
          a.confirmed ? null : el('span', { class: 'ee-tag', text: 'Unconfirmed' })
        ]),
        el('div', { class: 'r3' }, [
          select('act-role-' + i, roles, a.role, function (v) { a.role = v; st.actsChanged = true; }),
          el('button', { type: 'button', 'data-act': 'act-remove-' + i, class: 'x', text: '×', onclick: function () { st.acts.splice(i, 1); st.actsChanged = true; redraw(); } })
        ]),
        el('div', { class: 'r2' }, [
          input('act-start-' + i, localInput(a.start, p.tz), 'datetime-local', function (v) { a.start = parseInput(v, p.tz); st.actsChanged = true; }),
          input('act-end-' + i, localInput(a.end, p.tz), 'datetime-local', function (v) { a.end = parseInput(v, p.tz); st.actsChanged = true; })
        ])
      ]);
    });
    var max = X().actsMax(icd);
    if (X().opServed(icd, 'event.setLineup') && (max == null || st.acts.length < max)) {
      var actPick = select('act-pick', [['', '—']].concat(pickable.map(function (h) { return [h, nameOf(h)]; })), st.pick || '', function (v) { st.pick = v; });
      body.lineup.push(el('div', { class: 'ee-add' }, [actPick, el('button', { type: 'button', 'data-act': 'act-add', text: 'Add', onclick: function () {
        var h = st.pick; if (!h) return;
        var a = people.indexOf(h) >= 0 ? { member: h } : { object: h };
        a.role = roles.some(function (r) { return r[0] === 'performer'; }) ? 'performer' : roles[0][0];
        a.start = null; a.end = null; a.confirmed = false;
        st.acts.push(a); st.pick = ''; st.actsChanged = true; redraw();
      } })]));
    }

    body.media = [
      field('Photos', pick('photo', 'image/*', true, 'Add', function (e) {
        var files = Array.prototype.slice.call(e.target.files || []);
        Promise.all(files.map(function (file) {
          return takeMedia(icd, 'event.addPhoto', 'photo', file).then(function (m) { st.photos.push({ id: hex16(), mime: m.mime, data: m.data }); },
            function (err) { st.mediaWhy = String(err.message || err); });
        })).then(redraw);
      })),
      el('div', { class: 'ee-photos' }, st.photos.map(function (ph, i) {
        return el('figure', {}, [el('img', { src: 'data:' + ph.mime + ';base64,' + ph.data, alt: '' }),
          el('button', { type: 'button', 'data-act': 'photo-remove-' + i, class: 'x', text: '×', onclick: function () {
            if (ph.held) st.removed.push(ph.id); st.photos.splice(i, 1); redraw();
          } })]);
      })),
      field('Clip', el('div', { class: 'ee-cover' }, [
        (function (c) { return c && c.data ? el('video', { class: 'ee-still', src: 'data:' + c.mime + ';base64,' + c.data, muted: 'true', controls: 'true', playsinline: 'true' }) : null; })(st.clip || (ev && ev.view && ev.view.clip)),
        pick('clip', 'video/*', false, 'Choose', function (e) {
          var file = e.target.files && e.target.files[0]; if (!file) return;
          takeMedia(icd, 'event.setClip', 'clip', file).then(function (m) { st.clip = m; st.mediaWhy = ''; redraw(); },
            function (err) { st.mediaWhy = String(err.message || err); redraw(); });
        })])),
      text('videoUrl', 'Video link', 'videoUrl', 'url'),
      st.mediaWhy && st.step !== 'basics' ? el('p', { class: 'ee-why', 'data-why-media': '', text: st.mediaWhy }) : null
    ];
    body.tickets = [text('ticketUrl', 'Tickets link', 'ticketUrl', 'url')];

    var regArgs = X().args(icd, 'group.setRegistration');
    var flag = function (k, label) {
      return regArgs[k] ? el('label', { class: 'ee-flag' }, [el('input', { name: k, type: 'checkbox', checked: st.reg[k] ? 'true' : null,
        onchange: function (e) { st.reg[k] = e.target.checked ? 1 : 0; } }), el('span', { text: label })]) : null;
    };
    var num = function (k, label) { return regArgs[k] ? field(label, input(k, st.reg[k], 'number', function (v) { st.reg[k] = v === '' ? null : parseInt(v, 10); })) : null; };
    body.registration = site && X().opServed(icd, 'group.setRegistration') ? [
      num('capacity', 'Capacity'), flag('waitlist', 'Waitlist'), flag('approval', 'Approval'), num('guestsMax', 'Guests each'), flag('maybe', 'Maybe'),
      regArgs.closesMs ? field('Closes', input('closesMs', localInput(st.reg.closesMs, p.tz), 'datetime-local', function (v) { st.reg.closesMs = parseInput(v, p.tz); })) : null,
      regArgs.location ? field('Location', select('location', Object.keys(regArgs.location.vocabulary || {}).map(function (k) { return [k, cap(k)]; }),
        st.reg.location || Object.keys(regArgs.location.vocabulary || {})[0], function (v) { st.reg.location = v; })) : null
    ] : [];

    if (manage) {
      body.status = [field('Status', select('status', X().statuses(icd).map(function (s) { return [s, cap(s)]; }), p.status || X().statuses(icd)[0], function (v) { p.status = v; }))];
      var rsvps = site ? X().rsvpsOf(icd, site, ev.id) : {}, counts = site ? X().countsOf(icd, site, ev.id) : null;
      var decide = site && X().may(icd, 'group.rsvpDecide', site, me);
      /* An answer waiting on a host is the fold's `place` pending (FOLDS-CORE); without one, going
         under approval and undecided. */
      var pending = Object.keys(rsvps).filter(function (m) {
        var a = rsvps[m];
        return a.place ? a.place === 'pending' : a.status === 'going' && st.reg.approval && !a.decision;
      });
      var word = function (s) { return page() && page().label ? page().label(s) : cap(s); };
      body.guests = counts ? [el('div', { class: 'ee-counts' }, X().rsvpStatuses(icd).map(function (s) {
        return el('span', {}, [el('b', { text: String(counts[s] || 0) }), ' ' + word(s)]);
      }))].concat(pending.map(function (m) {
        return el('div', { class: 'ee-host' }, [el('span', { class: 'nm', text: nameOf(m) }), el('span', { class: 'ee-tag', text: 'Pending' }),
          el('span', { class: 'ee-acts' }, !decide ? [] : [['approved', 'Approve'], st.reg.waitlist ? ['waitlisted', 'Waitlist'] : null, ['declined', 'Decline']]
            .filter(Boolean).map(function (d) {
              return el('button', { type: 'button', 'data-act': 'decide-' + d[0] + '-' + m, text: d[1], onclick: function () {
                now(ctx, st, site.id, 'group.rsvpDecide', { event: ev.id, member: m, decision: d[0] }, redraw); } });
            }))]);
      })) : [];
    }

    /* The rail, the sections (each drawn, the chosen one shown), the preview, and Publish. */
    var rail = el('nav', { class: 'ee-rail', 'data-rail': '' }, sections.map(function (s, i) {
      return el('button', { type: 'button', 'data-step': s[0], class: st.step === s[0] ? 'on' : '', 'aria-current': st.step === s[0] ? 'step' : null,
        onclick: function () { st.step = s[0]; redraw(); } }, [el('b', { text: String(i + 1) }), el('span', { text: s[1] })]);
    }));
    var detail = el('div', { class: 'ee-detail' }, sections.map(function (s) {
      var sec = el('section', { 'data-sec': s[0], 'aria-label': s[1] }, body[s[0]] || []);
      sec.hidden = st.step !== s[0];
      return sec;
    }));
    var card = el('div', { class: 'ee-card' });
    function preview() {
      card.textContent = '';
      var cover = st.banner || (ev && ev.view && ev.view.banner);
      if (cover && cover.data) card.appendChild(el('img', { class: 'cov', src: 'data:' + cover.mime + ';base64,' + cover.data, alt: '' }));
      card.appendChild(el('h3', { text: p.title || '' }));
      var P = page(), v0 = { start_ms: p.startMs, end_ms: p.endMs, tz: p.tz, all_day: p.allDay }, rule = '';
      try { rule = profileArgs(icd, st).recurrence || ''; } catch (e) { rule = st.rule; }
      card.appendChild(el('p', { 'data-preview': 'when', text: P && P.whenOf ? P.whenOf(v0) : whenText(p) }));
      if (rule && P && P.repeatOf) card.appendChild(el('p', { 'data-preview': 'repeat', text: P.repeatOf(icd, { recurrence: rule, tz: p.tz }) }));
      card.appendChild(el('p', { text: p.venue || p.online || '' }));
    }
    preview();
    var go_ = el('button', { type: 'button', class: 'ee-go', 'data-act': manage ? 'save' : 'publish', text: manage ? 'Save' : 'Publish',
      disabled: st.busy ? 'true' : null, onclick: function () {
        if (st.busy) return;
        if (manage) save(ctx, st, ev, site, redraw); else publish(ctx, st, site, redraw);
      } });
    var why = st.why ? el('p', { class: 'ee-why', 'data-why': '', text: st.why }) : null;
    var top = el('div', { class: 'ee-top' }, [
      el('h1', { text: manage ? (p.title || '') : 'New event' }),
      el('button', { type: 'button', class: 'ee-pv', 'data-act': 'preview-toggle', text: 'Preview', 'aria-pressed': st.preview ? 'true' : 'false',
        onclick: function () { st.preview = !st.preview; redraw(); } }),
      go_
    ]);
    f.appendChild(el('div', { class: 'ee' + (st.preview ? ' pv' : '') }, [top, why, el('div', { class: 'ee-body' }, [rail, detail, el('aside', { class: 'ee-prev' }, [card])])]));
    /* The stepper keeps the chosen step in view. */
    var on = rail.querySelector && rail.querySelector('.on');
    if (on && on.scrollIntoView) on.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }

  root.WallFlowers = root.WallFlowers || {};
  root.WallFlowers.EventsEdit = { draw: draw };
})(window);
