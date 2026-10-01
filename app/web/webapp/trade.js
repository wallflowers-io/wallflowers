/* trade.js — the Trade board and a listing's page (W-98 Trade; ICD 2.3.1).

     WallFlowers.Trade.board(site, me)          the Site's live listings, newest first
     WallFlowers.Trade.draw.board(ctx, col, sites)   rows drawn
     WallFlowers.Trade.draw.thing(ctx, wrap, o)

   A member lists one of their Things on a Site's board (group.publishListing, a snapshot of
   the Thing) and withdraws it (the same op, `withdrawn` 1); the Site's owner or an admin
   removes one (group.removeListing). A Thing's page carries its condition and description
   (thing.setProfile), its photos (thing.addPhoto, removePhoto) and whether it is still for
   sale (thing.setDisposition). Every arg, word and cap is the ICD's, read from the one the
   webapp is served (trade.test.mjs holds the builders to it). The drawing takes the
   webapp's ctx (webapp.js tradeCtx) and writes through the Door's /v2/apply; a refusal is
   shown in the Door's words. */
(function (root) {
  'use strict';

  function arg(icd, kind, op, name) { return icd.kinds[kind].ops[op].args[name] || {}; }
  function words(icd, kind, op, name) { return Object.keys(arg(icd, kind, op, name).vocabulary || {}); }

  /* ── what is written ─────────────────────────────────────────────────────────── */

  /* A Thing listed on a Site: the snapshot publishListing carries, its first photo as the
     row's picture. A Thing with no posture has nothing to list. */
  function listArgs(o, now) {
    var v = o.view || {};
    if (!v.posture) return null;
    var a = { thingId: o.id, posture: v.posture, title: v.name || '', reach: v.reach || 'network', rev: now };
    if (v.descriptor) a.descriptor = v.descriptor;
    if (v.price) a.price = v.price;
    if (v.deadline) a.deadline = v.deadline;
    if (v.area) a.area = v.area;
    var p = (v.photos || [])[0];
    if (p && p.data) { a.photo = p.data; a.photoMime = p.mime; }
    return a;
  }

  /* Taken down: the listing restated, withdrawn, at a later rev. */
  function withdrawArgs(l, now) {
    return { thingId: l.thingId, posture: l.posture, title: l.title, reach: l.reach, rev: Math.max(now, (l.rev || 0) + 1), withdrawn: 1 };
  }

  function removeArgs(l) { return { author: l.author, thingId: l.thingId }; }
  function mayRemove(role) { return role === 'owner' || role === 'admin'; }

  function board(site, me) {
    return ((site.view && site.view.listings) || []).slice()
      .sort(function (a, b) { return (b.gen || 0) - (a.gen || 0); })
      .map(function (l) { return Object.assign({ mine: l.author === me }, l); });
  }
  function listedOn(site, thingId, me) {
    return board(site, me).filter(function (l) { return l.mine && l.thingId === thingId; })[0];
  }

  /* thing.setProfile is the whole profile: what the Thing is now, with the edits over it. */
  function profileArgs(v, edits, icd) {
    var a = { name: v.name };
    ['descriptor', 'category', 'condition', 'description'].forEach(function (k) {
      var x = k in edits ? edits[k] : v[k];
      if (x != null && x !== '') a[k] = x;
    });
    if (a.condition && conditions(icd).indexOf(a.condition) < 0) throw new Error('condition: ' + a.condition);
    var max = arg(icd, 'thing', 'thing.setProfile', 'description').maxLength;
    if (a.description && max && a.description.length > max) throw new Error('description: over ' + max);
    return a;
  }
  function conditions(icd) { return words(icd, 'thing', 'thing.setProfile', 'condition'); }

  /* A still, inline within the ICD's ceiling, keyed by 16 hex of the client's choosing. A still
     is png or jpeg to core (pacific-media MediaKind::Still; webp and gif are an animation). */
  var STILL = /^data:(image\/(?:png|jpeg));base64,([A-Za-z0-9+/]+={0,2})$/;
  function hex16() {
    var b = new Uint8Array(8);
    (root.crypto || globalThis.crypto).getRandomValues(b);
    return Array.prototype.map.call(b, function (x) { return (x < 16 ? '0' : '') + x.toString(16); }).join('');
  }
  function photoArgs(dataUrl, icd, now) {
    var m = STILL.exec(dataUrl || ''), max = arg(icd, 'thing', 'thing.addPhoto', 'photo').maxLength;
    if (!m || (max && m[2].length > max)) return null;
    return { id: hex16(), photo: m[2], photoMime: m[1], at: now };
  }
  function removePhotoArgs(id) { return { id: id }; }
  function roomForPhotos(v, icd) { return (v.photos || []).length < (icd.kinds.thing.ops['thing.addPhoto'].maxLive || 0); }

  function dispositions(icd) { return words(icd, 'thing', 'thing.setDisposition', 'state'); }
  function dispositionArgs(state, now, forPk, until, icd) {
    if (dispositions(icd).indexOf(state) < 0) throw new Error('state: ' + state);
    var a = { state: state, at: now };
    if (state === 'reserved') {
      if (!forPk || !until) throw new Error('reserved: for whom, and until when');
      a.for = forPk; a.until = until;
    }
    return a;
  }

  /* ── what is drawn ───────────────────────────────────────────────────────────── */

  /* A listing's snapshot photo (`photo`, `photoMime`, as core serves a SiteListing), or none. */
  function picture(l) { return l && l.photoMime && l.photo ? 'data:' + l.photoMime + ';base64,' + l.photo : null; }

  /* One write through the Door: busy while it goes, the model reloaded after, a refusal in
     the Door's words beside the button. */
  function write(ctx, btn, why, object, op, args) {
    ctx.busy(btn, true);
    why.textContent = '';
    return ctx.door('/v2/apply', { method: 'POST', body: { object: object, op: op, args: args } })
      .then(function () { return ctx.load(); }, function (e) { why.textContent = String(e && e.message || e); })
      .then(function () { ctx.busy(btn, false); });
  }
  function action(ctx, label, object, op, args, cls) {
    var why = ctx.el('span', { class: 'why' });
    var b = ctx.el('button', { type: 'button', class: 'tbtn' + (cls ? ' ' + cls : ''), text: label });
    b.onclick = function (e) {
      e.stopPropagation();
      var a = typeof args === 'function' ? args() : args;
      if (a) write(ctx, b, why, object, op, a);
    };
    return ctx.el('span', { class: 'tact' }, [b, why]);
  }

  function row(ctx, site, l) {
    var held = ctx.M.byId[l.thingId], pic = picture(l);
    var r = ctx.el('div', { class: 'card listing' + (held ? '' : ' nohold') }, [
      pic ? ctx.el('img', { class: 'lthumb', src: pic, alt: '' }) : ctx.el('span', { class: 'lthumb' }),
      ctx.el('div', { class: 'ltxt' }, [
        ctx.el('h3', { text: l.title }),
        ctx.el('p', { class: 'ex', text: [l.posture, l.price, l.area].filter(Boolean).join(' · ') }),
        ctx.el('p', { class: 'lwho', text: l.site ? ctx.titleOf(site) : ctx.who(l.author) })
      ])
    ]);
    if (held) r.onclick = function () { ctx.open('object', l.thingId); };
    if (l.mine) r.appendChild(action(ctx, 'Withdraw', site.id, 'group.publishListing', function () { return withdrawArgs(l, Date.now()); }));
    else if (mayRemove(ctx.roleIn(site))) r.appendChild(action(ctx, 'Remove', site.id, 'group.removeListing', removeArgs(l)));
    return r;
  }

  /* The rows, each Site's under its name where there are several; how many were drawn. */
  function drawBoard(ctx, col, sites) {
    var n = 0;
    sites.forEach(function (site) {
      var rows = board(site, ctx.M.me.pk);
      if (!rows.length) return;
      n += rows.length;
      if (sites.length > 1) col.appendChild(ctx.el('div', { class: 'lsite', text: ctx.titleOf(site) }));
      rows.forEach(function (l) { col.appendChild(row(ctx, site, l)); });
    });
    return n;
  }

  /* A Thing is its lister's own object: its condition, description, photos and disposition
     reach its lister alone, so they are drawn on the lister's own page and nowhere else. What
     other members see is the listing's snapshot on the board. */
  function drawThing(ctx, wrap, o) {
    var v = o.view || {}, me = ctx.M.me.pk, mine = o.owner === me, icd = ctx.icd;
    if (!mine) return;
    var d = v.disposition;
    if (d && d.state && d.state !== 'available') wrap.insertBefore(ctx.el('span', { class: 'pill disp ' + d.state, text: d.state }), wrap.firstChild);

    var gal = ctx.el('div', { class: 'gallery' });
    (v.photos || []).forEach(function (p) {
      var f = ctx.el('figure', {}, [ctx.el('img', { src: picture({ photoMime: p.mime, photo: p.data }), alt: '' })]);
      if (mine) f.appendChild(action(ctx, '×', o.id, 'thing.removePhoto', removePhotoArgs(p.id), 'x'));
      gal.appendChild(f);
    });
    if (mine && icd && roomForPhotos(v, icd)) {
      var why = ctx.el('span', { class: 'why' });
      var file = ctx.el('input', { type: 'file', accept: 'image/*', class: 'addpic' });
      file.onchange = function () {
        var f = file.files && file.files[0];
        if (!f) return;
        var max = arg(icd, 'thing', 'thing.addPhoto', 'photo').maxLength;
        ctx.shrink(f, { max: max, side: 1080, square: false, type: 'image/jpeg' }).then(function (url) {
          var a = photoArgs(url, icd, Date.now());
          if (!a) throw new Error('too large');
          return write(ctx, file, why, o.id, 'thing.addPhoto', a);
        }).catch(function (e) { why.textContent = String(e && e.message || e); });
      };
      gal.appendChild(ctx.el('label', { class: 'addfig' }, [ctx.el('span', { text: '+' }), file, why]));
    }
    if (gal.childNodes.length) wrap.appendChild(gal);

    if (mine && icd) {
      wrap.appendChild(editor(ctx, o, v, icd));
      wrap.appendChild(listing(ctx, o));
      return;
    }
    if (v.condition) {
      var said = icd && arg(icd, 'thing', 'thing.setProfile', 'condition').vocabulary;
      var c = (said && said[v.condition]) || v.condition;
      wrap.appendChild(ctx.el('p', { class: 'cond', text: c.charAt(0).toUpperCase() + c.slice(1) }));
    }
    if (v.description) wrap.appendChild(ctx.el('div', { class: 'desc', text: v.description }));
  }

  /* The owner's edits: condition and description, and whether it is still for sale. */
  function editor(ctx, o, v, icd) {
    var box = ctx.el('div', { class: 'tedit' });
    var cond = ctx.el('select', { 'aria-label': 'Condition' });
    [''].concat(conditions(icd)).forEach(function (w) {
      var opt = ctx.el('option', { value: w, text: w ? arg(icd, 'thing', 'thing.setProfile', 'condition').vocabulary[w] : '—' });
      if (w === (v.condition || '')) opt.selected = true;
      cond.appendChild(opt);
    });
    cond.value = v.condition || '';
    var desc = ctx.el('textarea', { rows: 5, 'aria-label': 'Description', maxlength: arg(icd, 'thing', 'thing.setProfile', 'description').maxLength || null });
    desc.value = v.description || '';
    box.appendChild(cond);
    box.appendChild(desc);
    box.appendChild(action(ctx, 'Save', o.id, 'thing.setProfile', function () {
      return profileArgs(v, { condition: cond.value, description: desc.value }, icd);
    }, 'go'));

    var disp = ctx.el('select', { 'aria-label': 'For sale' }), cur = (v.disposition && v.disposition.state) || 'available';
    dispositions(icd).forEach(function (w) {
      var opt = ctx.el('option', { value: w, text: w.charAt(0).toUpperCase() + w.slice(1) });
      if (w === cur) opt.selected = true;
      disp.appendChild(opt);
    });
    disp.value = cur;
    var site = ctx.siteOf(o), forWho = ctx.el('select', { 'aria-label': 'Reserved for' }), until = ctx.el('input', { type: 'date', 'aria-label': 'Until' });
    var held = v.disposition && v.disposition.state === 'reserved' ? v.disposition : null;
    ((site && site.members) || []).filter(function (pk) { return pk !== ctx.M.me.pk; }).forEach(function (pk) {
      forWho.appendChild(ctx.el('option', { value: pk, text: ctx.who(pk) }));
    });
    if (held && held.for) forWho.value = held.for;
    if (held && held.until) until.value = new Date(held.until).toISOString().slice(0, 10);
    var hold = ctx.el('span', { class: 'hold' }, [forWho, until]);
    hold.hidden = disp.value !== 'reserved';
    disp.onchange = function () { hold.hidden = disp.value !== 'reserved'; };
    box.appendChild(ctx.el('div', { class: 'trow' }, [disp, hold, action(ctx, 'Set', o.id, 'thing.setDisposition', function () {
      var end = until.value ? new Date(until.value + 'T23:59:59').getTime() : null;
      return dispositionArgs(disp.value, Date.now(), disp.value === 'reserved' ? forWho.value : null, disp.value === 'reserved' ? end : null, icd);
    })]));
    return box;
  }

  /* On each Site you are in: listed there, and withdrawn from there. */
  function listing(ctx, o) {
    var box = ctx.el('div', { class: 'tlist' }), me = ctx.M.me.pk;
    if (o.owner !== me) return box;
    ctx.M.sites.filter(function (s) { return ctx.roleIn(s); }).forEach(function (site) {
      var l = listedOn(site, o.id, me);
      if (!l && !(o.view || {}).posture) return;
      box.appendChild(l
        ? action(ctx, 'Withdraw from ' + ctx.titleOf(site), site.id, 'group.publishListing', function () { return withdrawArgs(l, Date.now()); })
        : action(ctx, 'List on ' + ctx.titleOf(site), site.id, 'group.publishListing', function () { return listArgs(o, Date.now()); }, 'go'));
    });
    return box;
  }

  var api = {
    listArgs: listArgs, withdrawArgs: withdrawArgs, removeArgs: removeArgs, mayRemove: mayRemove,
    board: board, listedOn: listedOn, picture: picture, profileArgs: profileArgs, conditions: conditions,
    photoArgs: photoArgs, removePhotoArgs: removePhotoArgs, roomForPhotos: roomForPhotos,
    dispositions: dispositions, dispositionArgs: dispositionArgs,
    draw: { board: drawBoard, thing: drawThing }
  };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else { root.WallFlowers = root.WallFlowers || {}; root.WallFlowers.Trade = api; }
})(typeof self !== 'undefined' ? self : this);
