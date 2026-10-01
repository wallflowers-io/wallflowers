/* events-page.js — W-98 Events: an event's page (EVENTS-UI-B): its cover, when and where, the
   host, the RSVP bar on the Site, the lineup, photos, clip and links, the wall, the status
   badge, and Manage for its owner and co-hosts. Drawn by webapp.js's drawObject hook with
   eventsCtx() {door, batch, load, open, el, toast, M, S, icd}, reading the ICD through
   events-icd.js; nothing here states what the ICD holds.

   The host is the owner (R3: a Site's events are `created`, so the roster holds only its
   creator). The answers are the Site's (group.rsvp, on the group whose `created` edge names
   the event); declined is a count, never a list. An act is confirmed where the view says so;
   its performer confirms by writing their own half of performs_at (group.setAffiliation) on
   their own record, M.self, or on a group they own. An op the served ICD does not declare is
   not drawn; `gen` is the Door's to add; a refusal is shown as the Door words it. */
(function (root) {
  'use strict';
  var RSVP = 'group.rsvp', AFFIL = 'group.setAffiliation';
  var CREATED = 'created', PERFORMS_AT = 'performs_at', GOING = 'going';
  /* Faces for these answers alone: a status the ICD adds later is counted, never listed. */
  var FACES = [GOING, 'maybe'];
  var LABEL = { going: 'Going', maybe: 'Maybe', declined: "Can't go" };
  /* ASSURANCE's two lines (assurance.md:182), each one string, here alone. */
  var SEEN_ANSWER = 'Members see your answer.', SEEN_NAMES = 'Members see these names.';
  var HUES = ['rose', 'red', 'amber', 'yellow', 'green', 'cyan', 'blue', 'magenta'];
  var DAY = { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' }, TIME = { hour: '2-digit', minute: '2-digit' };
  var LATEST = 3, MAX_FACES = 8;

  function X() { return root.WallFlowers.EventsIcd; }

  /* ── readers the page needs and events-icd.js does not hold ─────────────── */
  /* The role of the event's forum part (kinds.event.parts): its wall. */
  function wallRole(icd) {
    var parts = (icd && icd.kinds && icd.kinds.event && icd.kinds.event.parts) || [];
    for (var i = 0; i < parts.length; i++) if (parts[i].kind === 'forum') return parts[i].role;
    return null;
  }
  /* The default guests a going answer may bring: group.setRegistration's guestsMax, which the
     ICD states in its summary alone ("default 10"); null where it cannot be read. */
  function guestsDefault(icd) {
    var s = ((X().args(icd, 'group.setRegistration').guestsMax) || {}).summary || '';
    var m = /default (\d+)/.exec(s);
    return m ? Number(m[1]) : null;
  }
  function performsAt(icd) {
    var rel = X().args(icd, AFFIL).rel;
    return !!(rel && rel.vocabulary && Object.prototype.hasOwnProperty.call(rel.vocabulary, PERFORMS_AT));
  }
  function vocab(icd, op, arg) { return ((X().args(icd, op)[arg] || {}).vocabulary) || {}; }

  /* ── the graph ────────────────────────────────────────────────────────── */
  function edge(g, peer, rel) {
    return !!g && ((g.view && g.view.affiliations) || []).some(function (a) { return a.peer === peer && a.rel === rel; });
  }
  /* The Site: the open one where it created the event, else the group that did. */
  function siteOf(ctx, o) {
    var M = ctx.M, open = ctx.S && ctx.S.site && M.byId[ctx.S.site];
    if (open && open.kind === 'group' && edge(open, o.id, CREATED)) return open;
    var ids = Object.keys(M.byId);
    for (var i = 0; i < ids.length; i++) {
      var g = M.byId[ids[i]];
      if (g && g.kind === 'group' && edge(g, o.id, CREATED)) return g;
    }
    return null;
  }
  function wallOf(ctx, o) {
    var role = wallRole(ctx.icd), M = ctx.M;
    if (!role) return null;
    var ids = Object.keys(M.byId);
    for (var i = 0; i < ids.length; i++) {
      var f = M.byId[ids[i]], p = f && f.kind === 'forum' && f.view && f.view.parent;
      if (p && p.parent === o.id && p.role === role) return f;
    }
    return null;
  }
  function iconUrl(i) {
    if (typeof i === 'string') return /^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+\/]+={0,2}$/.test(i) ? i : null;
    return i && i.data && /^image\/(png|jpeg|webp)$/.test(i.mime || '') ? 'data:' + i.mime + ';base64,' + i.data : null;
  }
  /* A Site's cards: its view's `profiles`, pairs [[member, {displayName, icon}]] or a map. */
  function cardsOf(g) {
    var raw = g && g.view && g.view.profiles, out = {};
    function put(pk, c) {
      if (!c || typeof c !== 'object' || typeof pk !== 'string') return;
      var n = typeof c.displayName === 'string' ? c.displayName : typeof c.name === 'string' ? c.name : '';
      out[pk.replace(/^ed25519:/, '')] = { name: n.trim(), icon: iconUrl(c.icon) };
    }
    if (Array.isArray(raw)) raw.forEach(function (x) { if (Array.isArray(x)) put(x[0], x[1]); });
    else if (raw && typeof raw === 'object') Object.keys(raw).forEach(function (k) { put(k, raw[k]); });
    return out;
  }
  /* A person: the event's Site's card, else another Site's, else the names the page holds.
     A name, or nobody: never a key (O-77). */
  function person(ctx, site, pk) {
    var M = ctx.M, sites = [site].concat(M.sites || []);
    for (var i = 0; i < sites.length; i++) {
      var c = sites[i] && cardsOf(sites[i])[pk];
      if (c && (c.name || c.icon)) return { name: c.name || M.names[pk] || '', icon: c.icon };
    }
    return { name: M.names[pk] || '', icon: null };
  }
  function groupName(g) { var v = (g && g.view) || {}; return v.display_name || (g && g.name) || ''; }
  function hue(s) {
    var h = 0, str = String(s || '');
    for (var i = 0; i < str.length; i++) h = (h * 31 + str.charCodeAt(i)) >>> 0;
    return 'var(--' + HUES[h % HUES.length] + ')';
  }
  function face(el, key, who, cls) {
    var f = el('span', { class: 'ev-face' + (cls ? ' ' + cls : '') + (who.name || who.icon ? '' : ' anon'), style: '--t:' + hue(key),
      title: who.name || null, 'aria-label': who.name || null });
    if (who.icon) f.appendChild(el('img', { src: who.icon, alt: '' }));
    else if (who.name) f.textContent = who.name.slice(0, 1).toUpperCase();
    return f;
  }

  /* ── values ───────────────────────────────────────────────────────────── */
  function zoneOf(tz) {
    if (!tz) return null;
    try { new Intl.DateTimeFormat('en-GB', { timeZone: tz }); return tz; } catch (e) { return null; }
  }
  function fmt(ms, tz, opts) {
    var o = {};
    Object.keys(opts).forEach(function (k) { o[k] = opts[k]; });
    if (tz) o.timeZone = tz;
    return new Intl.DateTimeFormat('en-GB', o).format(new Date(ms));
  }
  function zoneName(ms, tz) {
    try {
      var parts = new Intl.DateTimeFormat('en-GB', { timeZone: tz, timeZoneName: 'short' }).formatToParts(new Date(ms));
      for (var i = 0; i < parts.length; i++) if (parts[i].type === 'timeZoneName') return parts[i].value;
    } catch (e) { /* no name */ }
    return tz;
  }
  function here() { try { return new Intl.DateTimeFormat().resolvedOptions().timeZone; } catch (e) { return null; } }
  function whenOf(v) {
    var tz = zoneOf(v.tz), s = v.start_ms, e = v.end_ms;
    if (typeof s !== 'number') return '';
    var d0 = fmt(s, tz, DAY), later = typeof e === 'number' && e > s;
    if (v.all_day === 1) {
      var d1 = later ? fmt(e, tz, DAY) : d0;
      return d1 !== d0 ? d0 + ' – ' + d1 : d0;
    }
    var t = d0 + ', ' + fmt(s, tz, TIME);
    if (later) t += ' – ' + (fmt(e, tz, DAY) === d0 ? '' : fmt(e, tz, DAY) + ', ') + fmt(e, tz, TIME);
    if (tz && tz !== here()) t += ' ' + zoneName(s, tz);
    return t;
  }
  /* The rule as the grammar reads it; the raw rule where it cannot. */
  function repeatOf(icd, v) {
    if (!v.recurrence) return '';
    var c = X().parseRule(icd, v.recurrence);
    if (!c) return String(v.recurrence);
    var once = X().parseRule(icd, 'FREQ=' + c.freq), out = [c.freq.charAt(0) + c.freq.slice(1).toLowerCase()];
    if (once && c.interval !== once.interval) out.push('every ' + c.interval);
    if (c.byday && c.byday.length) { var names = X().weekdayNames(icd); out.push(c.byday.map(function (d) { return names[d] || d; }).join(' ')); }
    if (c.count != null) out.push(c.count + '×');
    if (c.until != null) out.push('until ' + fmt(c.until, zoneOf(v.tz), DAY));
    return out.join(' · ');
  }
  function setOf(a, v) {
    if (a.start == null) return '';
    var tz = zoneOf(v.tz), day = typeof v.start_ms === 'number' && fmt(a.start, tz, DAY) !== fmt(v.start_ms, tz, DAY);
    var t = (day ? fmt(a.start, tz, { weekday: 'short' }) + ' ' : '') + fmt(a.start, tz, TIME);
    return a.end != null && a.end > a.start ? t + '–' + fmt(a.end, tz, TIME) : t;
  }
  function web(u) { return typeof u === 'string' && /^https?:\/\/[^\s"'<>]+$/i.test(u) ? u : null; }
  function hostOf(u) { var m = /^https?:\/\/([^\/?#]+)/i.exec(u); return m ? m[1].replace(/^www\./i, '') : u; }
  /* A still or a clip as a data: URL, from the view's {mime, data}; none for a detached ref. */
  function dataUrl(m, type) {
    if (!m || !m.data || typeof m.mime !== 'string' || m.mime.indexOf(type + '/') !== 0) return null;
    if (!/^[A-Za-z0-9+\/]+={0,2}$/.test(m.data) || !/^[a-z0-9.+-]+\/[a-z0-9.+-]+$/i.test(m.mime)) return null;
    return 'data:' + m.mime + ';base64,' + m.data;
  }

  /* ── the write ────────────────────────────────────────────────────────── */
  /* The op's declared args alone; a refusal in the Door's words under the control. */
  function send(ctx, object, op, a, why, controls) {
    var declared = X().args(ctx.icd, op), args = {};
    Object.keys(a).forEach(function (k) { if (Object.prototype.hasOwnProperty.call(declared, k) && a[k] != null) args[k] = a[k]; });
    controls.forEach(function (b) { b.disabled = true; });
    why.textContent = ''; why.hidden = true;
    return ctx.door('/v2/apply', { method: 'POST', body: { object: object, op: op, args: args } })
      .then(function () { return ctx.load(); }, function (e) {
        controls.forEach(function (b) { b.disabled = false; });
        why.textContent = String((e && e.message) || e); why.hidden = false;
      });
  }
  function whyBox(el) { var w = el('p', { class: 'ev-why', role: 'alert' }); w.hidden = true; return w; }

  /* ── the parts of the page ────────────────────────────────────────────── */
  function cover(el, v) {
    var still = dataUrl(v.banner, 'image');
    if (still) return el('div', { class: 'ev-cover' }, [el('img', { src: still, alt: '' })]);
    var clip = dataUrl(v.clip, 'video');
    if (!clip) return null;
    var vid = el('video', { src: clip, muted: '', autoplay: '', loop: '', playsinline: '', controls: '', preload: 'metadata' });
    vid.muted = true;
    return el('div', { class: 'ev-cover' }, [vid]);
  }

  function facts(ctx, o, v, site) {
    var el = ctx.el, dl = el('dl', { class: 'ev-facts' });
    function row(k, dd) { dl.appendChild(el('dt', { text: k })); dl.appendChild(dd); }
    function link(u) { return el('a', { class: 'ev-link', href: u, target: '_blank', rel: 'noopener', text: hostOf(u) }); }
    var when = whenOf(v);
    if (when) row('When', el('dd', { text: when }));
    var rep = repeatOf(ctx.icd, v);
    if (rep) row('Repeats', el('dd', { text: rep }));
    if (v.venue) row('Where', el('dd', { text: v.venue }));
    if (web(v.online)) row('Online', el('dd', {}, [link(web(v.online))]));
    if (o.owner) {
      var who = person(ctx, site, o.owner);
      row('Host', el('dd', { class: 'ev-hosts' }, [face(el, o.owner, who), who.name ? el('span', { text: who.name }) : null]));
    }
    if (web(v.ticket_url)) row('Tickets', el('dd', {}, [link(web(v.ticket_url))]));
    return dl;
  }

  function rsvp(ctx, o, site) {
    var el = ctx.el, icd = ctx.icd, E = X(), me = ctx.M.me.pk;
    var box = el('section', { class: 'ev-rsvp' });
    var reg = E.registrationOf(icd, site, o.id) || {}, answers = E.rsvpsOf(icd, site, o.id), mine = answers[me] || null;
    var words = vocab(icd, RSVP, 'status'), statuses = E.rsvpStatuses(icd);
    function label(s) { return LABEL[s] || words[s] || s; }
    /* A status the registration switches off (its `maybe` 0) is not offered. */
    var offered = statuses.filter(function (s) { return reg[s] !== 0; });
    var mayAnswer = E.opServed(icd, RSVP) && E.may(icd, RSVP, site, me);
    if (mayAnswer) {
      var why = whyBox(el), controls = [];
      var row = el('div', { class: 'ev-choices', role: 'group' });
      offered.forEach(function (s) {
        var on = !!mine && mine.status === s;
        var b = el('button', { type: 'button', class: 'ev-choice' + (on ? ' on' : ''), 'data-status': s, 'aria-pressed': on ? 'true' : 'false', text: label(s),
          onclick: function () { if (!on) answer(s, 0, []); } });
        controls.push(b);
        row.appendChild(b);
      });
      var pend = pending(mine, reg);
      if (pend) row.appendChild(el('span', { class: 'ev-pending', text: pend }));
      box.appendChild(row);
      box.appendChild(el('p', { class: 'ev-seen', text: SEEN_ANSWER }));
      var cap = reg.guestsMax != null ? reg.guestsMax : guestsDefault(icd);
      if (mine && mine.status === GOING && cap !== 0 && E.args(icd, RSVP).guests) box.appendChild(guests(mine.guests || 0));
      box.appendChild(why);
    }
    function answer(status, n, names) {
      var a = { event: o.id, status: status, at: Date.now() };
      if (status === GOING) {
        a.guests = n;
        var given = names.map(function (x) { return String(x || '').trim(); }).filter(Boolean);
        if (given.length) a.guestNames = JSON.stringify(given);
      }
      return send(ctx, site.id, RSVP, a, why, controls);
    }
    /* The guests stepper, 0 to the cap, and a name for each guest. */
    function guests(held) {
      var n = held, names = heldNames(mine), wrap = el('div', { class: 'ev-guests' }), saveBtn = null;
      var was = JSON.stringify(heldNames(mine).slice(0, held));
      function changed() { return n !== held || JSON.stringify(names.slice(0, n).map(function (x) { return String(x || '').trim(); })) !== was; }
      function draw() {
        wrap.textContent = '';
        var less = el('button', { type: 'button', class: 'ev-step', text: '−', onclick: function () { if (n > 0) { n--; names.length = n; draw(); } } });
        var more = el('button', { type: 'button', class: 'ev-step', text: '+', onclick: function () { if (cap == null || n < cap) { n++; draw(); } } });
        less.disabled = n <= 0;
        more.disabled = cap != null && n >= cap;
        wrap.appendChild(el('div', { class: 'ev-stepper' }, [el('span', { class: 'ev-k', text: 'Guests' }), less, el('b', { class: 'ev-n', text: String(n) }), more]));
        if (n > 0) {
          wrap.appendChild(el('p', { class: 'ev-seen', text: SEEN_NAMES }));
          var list = el('div', { class: 'ev-names' });
          for (var i = 0; i < n; i++) (function (i) {
            var inp = el('input', { type: 'text', class: 'ev-guest', 'aria-label': 'Guest ' + (i + 1), autocomplete: 'off' });
            inp.value = names[i] || '';
            inp.addEventListener('input', function () { names[i] = inp.value; if (saveBtn) saveBtn.disabled = !changed(); });
            list.appendChild(inp);
          })(i);
          wrap.appendChild(list);
        }
        var save = el('button', { type: 'button', class: 'ev-save', text: 'Save', onclick: function () {
          controls.push(save);
          answer(GOING, n, names.slice(0, n));
        } });
        save.disabled = !changed();
        wrap.appendChild(save);
        saveBtn = save;
      }
      draw();
      return wrap;
    }
    box.appendChild(tallies(ctx, o, site, reg, answers, statuses, label));
    return box;
  }
  /* The guests' names my answer holds (FOLDS-CORE's rsvp view: a list). */
  function heldNames(mine) {
    var g = mine && mine.guestNames;
    if (typeof g === 'string') { try { g = JSON.parse(g); } catch (e) { g = []; } }
    return Array.isArray(g) ? g.slice() : [];
  }
  /* My answer's standing: the fold's `place` (FOLDS-CORE: going | waitlisted | pending |
     declined), Pending while a host has yet to decide, else the ICD's decision word; without a
     place, as approval and the decision say. */
  function pending(mine, reg) {
    if (mine && mine.place) {
      if (mine.place === 'pending') return 'Pending';
      if (mine.place === GOING || (mine.place === 'declined' && mine.status !== GOING)) return '';
      return mine.decision || mine.place;
    }
    if (!mine || mine.status !== GOING || reg.approval !== 1 || mine.decision === 'approved') return '';
    return mine.decision || 'Pending';
  }
  /* An answer counted where its status says: going only once it has its place. */
  function counted(a, s, reg) {
    if (!a || a.status !== s) return false;
    if (s !== GOING) return true;
    return a.place ? a.place === GOING : reg.approval !== 1 || a.decision === 'approved';
  }
  function tallies(ctx, o, site, reg, answers, statuses, label) {
    var el = ctx.el, counts = X().countsOf(ctx.icd, site, o.id), box = el('div', { class: 'ev-tallies' });
    statuses.forEach(function (s) {
      var n = counts[s] || 0;
      if (!n) return;
      var t = el('div', { class: 'ev-tally', 'data-status': s }, [el('b', { class: 'ev-n', text: String(n) }), el('span', { class: 'ev-k', text: label(s) })]);
      if (s === GOING && counts.guests) t.appendChild(el('span', { class: 'ev-plus', text: '+' + counts.guests }));
      if (FACES.indexOf(s) >= 0) {
        var who = Object.keys(answers).filter(function (pk) {
          return counted(answers[pk], s, reg);
        });
        var faces = el('span', { class: 'ev-faces' });
        who.slice(0, MAX_FACES).forEach(function (pk) { faces.appendChild(face(el, pk, person(ctx, site, pk))); });
        if (who.length > MAX_FACES) faces.appendChild(el('span', { class: 'ev-more', text: '+' + (who.length - MAX_FACES) }));
        if (who.length) t.appendChild(faces);
      }
      box.appendChild(t);
    });
    return box;
  }

  /* An act's own record, where the viewer may write its half: M.self for their own act, a
     group they own for an organisation's. */
  function recordOf(ctx, a) {
    var M = ctx.M, me = M.me.pk, g;
    if (a.member) {
      if (a.member !== me) return null;
      g = M.self ? M.byId[M.self] : null;
      return { g: g && X().may(ctx.icd, AFFIL, g, me) ? g : null };   // no record held: drawn disabled
    }
    g = a.object && M.byId[a.object];
    return g && g.kind === 'group' && X().may(ctx.icd, AFFIL, g, me) ? { g: g } : null;
  }
  function lineup(ctx, o, v, site) {
    var el = ctx.el, E = X(), acts = E.actsOf(v), box = el('section', { class: 'ev-lineup' });
    var old = Array.isArray(v.lineup) ? v.lineup.filter(Boolean) : [];
    if (!acts.length && !old.length) return null;
    box.appendChild(el('h3', { class: 'ev-h', text: 'Lineup' }));
    var rw = (((E.args(ctx.icd, 'event.setLineup').acts || {}).items || {}).role || {}).vocabulary || {};
    var canConfirm = E.opServed(ctx.icd, AFFIL) && performsAt(ctx.icd);
    acts.forEach(function (a) {
      var key = a.member || a.object, who = a.member ? person(ctx, site, a.member) : { name: groupName(ctx.M.byId[a.object]), icon: null };
      var meta = el('div', { class: 'ev-meta' }, [el('span', { class: 'ev-role', text: rw[a.role] || a.role }),
        setOf(a, v) ? el('span', { class: 'ev-set', text: setOf(a, v) }) : null,
        a.confirmed ? null : el('span', { class: 'ev-unconfirmed', text: 'Unconfirmed' })]);
      var card = el('div', { class: 'ev-act' }, [face(el, key, who, 'big'),
        el('div', { class: 'ev-who' }, [who.name ? el('b', { text: who.name }) : null, meta])]);
      var rec = !a.confirmed && canConfirm ? recordOf(ctx, a) : null;
      if (rec && !(rec.g && edge(rec.g, o.id, PERFORMS_AT))) {
        var why = whyBox(el);
        var b = el('button', { type: 'button', class: 'ev-confirm', text: 'Confirm', onclick: function () {
          send(ctx, rec.g.id, AFFIL, { peer: o.id, rel: PERFORMS_AT, name: v.title || o.name, at: Date.now() }, why, [b]);
        } });
        if (!rec.g) b.disabled = true;
        card.appendChild(b);
        card.appendChild(why);
      }
      box.appendChild(card);
    });
    old.forEach(function (n) { box.appendChild(el('div', { class: 'ev-act' }, [face(el, n, { name: n }, 'big'), el('div', { class: 'ev-who' }, [el('b', { text: n })])])); });
    return box;
  }

  /* The photos, the clip where the banner is the cover, and the video's link. */
  function media(ctx, v) {
    var el = ctx.el, M = X().mediaOf(v), kids = [];
    var shots = M.photos.map(function (p) { return dataUrl(p, 'image'); }).filter(Boolean);
    if (shots.length) kids.push(el('div', { class: 'ev-photos' }, shots.map(function (u) { return el('img', { src: u, alt: '', loading: 'lazy' }); })));
    var clip = dataUrl(M.clip, 'video');
    if (clip && dataUrl(M.banner, 'image')) {
      var vid = el('video', { class: 'ev-clip', src: clip, muted: '', controls: '', playsinline: '', preload: 'metadata' });
      vid.muted = true;
      kids.push(vid);
    }
    if (web(M.video)) kids.push(el('a', { class: 'ev-card', href: web(M.video), target: '_blank', rel: 'noopener' }, [
      el('span', { class: 'ev-k', text: 'Video' }), el('b', { text: hostOf(web(M.video)) })]));
    return kids.length ? el('section', { class: 'ev-media' }, kids) : null;
  }

  function wall(ctx, o, site) {
    var f = wallOf(ctx, o);
    if (!f) return null;
    var el = ctx.el, ms = ((f.view && f.view.messages) || []).slice(-LATEST);
    var box = el('section', { class: 'ev-wall' });
    box.appendChild(el('button', { type: 'button', class: 'ev-h ev-open', text: 'Wall', onclick: function () { ctx.open('channel', f.id); } }));
    ms.forEach(function (m) {
      var who = person(ctx, site, m.author);
      box.appendChild(el('div', { class: 'ev-post' }, [face(el, m.author, who),
        el('div', { class: 'ev-who' }, [who.name ? el('b', { text: who.name }) : null, el('p', { text: m.text || '' })])]));
    });
    /* A member posts here as in a room: forum.post, its text and time (the Door adds gen). */
    var me = ctx.M.me.pk;
    if (X().opServed(ctx.icd, 'forum.post') && (f.members || []).indexOf(me) >= 0) {
      var why = whyBox(el);
      var say = el('input', { type: 'text', class: 'ev-say', name: 'wall-say', autocomplete: 'off', 'aria-label': 'Wall' });
      var go = el('button', { type: 'button', class: 'febtn go ev-send', 'data-act': 'wall-send', text: 'Send', onclick: function () {
        var text = String(say.value || '').trim();
        if (text) send(ctx, f.id, 'forum.post', { text: text, ts: Date.now() }, why, [go]);
      } });
      say.addEventListener('keydown', function (e) { if (e && e.key === 'Enter') { e.preventDefault && e.preventDefault(); go.onclick(); } });
      box.appendChild(el('div', { class: 'ev-compose' }, [say, go]));
      box.appendChild(why);
    }
    return box;
  }

  function draw(f, o, ctx) {
    if (!f || !o || o.kind !== 'event' || !ctx || !ctx.M) return;
    f.textContent = '';
    var el = ctx.el, icd = ctx.icd, E = X(), v = o.view || {}, me = ctx.M.me.pk, site = siteOf(ctx, o);
    var words = vocab(icd, 'event.setProfile', 'status'), statuses = E.statuses(icd);
    /* The vocabulary's first word is the one an absent status reads as ("absent is scheduled"). */
    var badge = v.status && statuses.indexOf(v.status) > 0 ? el('span', { class: 'ev-badge', 'data-status': v.status, text: words[v.status] }) : null;
    var head = el('header', { class: 'ev-head' }, [el('h1', { class: 'ev-title' }, [v.title || o.name || '', badge ? ' ' : null, badge])]);
    if (!o.item && E.editOp(icd, o, me)) head.appendChild(el('button', { type: 'button', class: 'ev-manage', text: 'Manage', onclick: function () { ctx.open('event-edit', o.id); } }));
    f.appendChild(el('article', { class: 'ev' }, [cover(el, v), head, facts(ctx, o, v, site), site ? rsvp(ctx, o, site) : null,
      v.descriptor ? el('p', { class: 'ev-about', text: v.descriptor }) : null,
      lineup(ctx, o, v, site), media(ctx, v), o.item ? null : wall(ctx, o, site)]));
  }

  root.WallFlowers = root.WallFlowers || {};
  /* THE ROSTER GAP (MANAGE's (b), 1 Oct): an event its Site created that this member does not
     hold. The Site's `created` names it, and its public item (GET /v2/site/:site/items, the Arc's
     face_items body) gives face_items' args by ICD name, taken here as the view's own keys. It is
     drawn read-only: no owner, roster or parts are known, so no Host, Manage or wall; its acts are
     the confirmed ones. A created event with no item is not drawn. */
  function snake(k) { return k.replace(/[A-Z]/g, function (c) { return '_' + c.toLowerCase(); }); }
  function fromItems(site, answer, M) {
    var items = (answer && answer.items) || {}, out = [];
    ((site && site.view && site.view.affiliations) || []).forEach(function (a) {
      var held = M.byId[a.peer];
      if (a.rel !== 'created' || (held && !held.item)) return;
      var it = items['event:' + a.peer];
      if (!it || typeof it !== 'object' || !it.title) return;
      var v = {};
      Object.keys(it).forEach(function (k) { v[snake(k)] = it[k]; });
      v.lineup = typeof it.lineup === 'string' && it.lineup ? it.lineup.split('\n') : [];
      v.acts = (Array.isArray(it.acts) ? it.acts : []).map(function (x) { var c = {}; Object.keys(x).forEach(function (k) { c[k] = x[k]; }); c.confirmed = true; return c; });
      v.roles = {}; v.photos = []; v.banner = null; v.clip = null;
      out.push({ id: a.peer, kind: 'event', name: it.title, owner: null, members: [], folds: true, item: true, view: v });
    });
    return out;
  }

  /* The page's own words for the editor's preview and its Guests: one formatter, one label. */
  root.WallFlowers.EventsPage = { draw: draw, whenOf: whenOf, repeatOf: repeatOf, fromItems: fromItems, label: function (s) { return LABEL[s] || s; } };
})(typeof window !== 'undefined' ? window : this);
