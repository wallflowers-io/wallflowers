/* rooms.js — W-98 Rooms: a room's description, forum.editDescription's, under the room's name.
   Drawn by one hook in webapp.js (drawChannel), which hands over the room and
   {door, load, el, op, me, note}; `op` is the served ICD's op by name.

   Who may edit is the op's ego, asked of THIS room: `owner` its owner, `role:<r>` its view's
   `roles`, `member` its roster. The cap is its `description.maxBytes`: over it, Save waits and
   says how far over, rather than send what the Door will refuse. `gen` is the Door's to add. */
(function (root) {
  var OP = 'forum.editDescription';
  var PEN = '<svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 19.5l1-4 10-10a2.1 2.1 0 013 3l-10 10-4 1zM13.5 7.5l3 3"/></svg>';
  var CSS = '.room-about{display:flex;align-items:flex-start;gap:6px;margin:-2px 0 12px}' +
    '.room-desc{flex:1;margin:0;font-size:15px;line-height:1.45;white-space:pre-wrap;word-break:break-word;color:var(--ink)}' +
    '.room-about .tb{margin-left:auto}' +
    '.room-edit{flex:1;display:flex;flex-direction:column;gap:8px}' +
    '.room-edit textarea{width:100%;box-sizing:border-box;min-height:84px;padding:10px 12px;border:1px solid var(--hair);border-radius:12px;' +
    'background:var(--s0);color:var(--ink);font:15px/1.45 var(--sans);resize:vertical}' +
    '.room-edit .row{display:flex;align-items:center;gap:8px}' +
    '.room-edit .row button{height:36px;padding:0 16px;border-radius:10px;font-weight:600;border:1px solid var(--hair);color:var(--ink)}' +
    '.room-edit .row .save{background:var(--ink);color:var(--s0);border-color:var(--ink)}' +
    '.room-edit .row .save[disabled]{opacity:.4}' +
    '.room-edit .over{margin-left:auto;font-size:12px;color:var(--red)}';

  function styled() {
    var d = root.document;
    if (!d || !d.head || d.getElementById('rooms-css')) return;
    var s = d.createElement('style');
    s.id = 'rooms-css';
    s.textContent = CSS;
    d.head.appendChild(s);
  }

  /* The ego, standing by standing, against this room. */
  function may(room, ctx) {
    var d = ctx.op && ctx.op(OP);
    if (!d) return false;
    var roles = (room.view && room.view.roles) || [];
    return String(d.ego || '').split('|').some(function (e) {
      if (e === 'owner') return room.owner === ctx.me;
      if (e === 'member') return (room.members || []).indexOf(ctx.me) >= 0;
      if (e.indexOf('role:') === 0) return roles.some(function (r) { return r[0] === ctx.me && r[1] === e.slice(5); });
      return false;
    });
  }

  function bytes(s) {
    return typeof TextEncoder === 'function' ? new TextEncoder().encode(s).length : unescape(encodeURIComponent(s)).length;
  }

  function header(col, room, ctx) {
    styled();
    var el = ctx.el, text = (room.view && room.view.description) || '', mine = may(room, ctx);
    if (!text && !mine) return;
    var d = ctx.op && ctx.op(OP), max = d && d.args && d.args.description && d.args.description.maxBytes;
    var box = el('div', { class: 'room-about' });
    function show() {
      box.textContent = '';
      if (text) box.appendChild(el('p', { class: 'room-desc', text: text }));
      if (mine) box.appendChild(el('button', { type: 'button', class: 'tb', title: 'Edit description', 'aria-label': 'Edit description', html: PEN, onclick: edit }));
    }
    function edit() {
      box.textContent = '';
      var area = el('textarea', { rows: '3', 'aria-label': 'Description' });
      area.value = text;
      var over = el('span', { class: 'over' });
      var save = el('button', { type: 'button', class: 'save', text: 'Save', onclick: function () { send(area.value.trim()); } });
      area.addEventListener('input', function () {
        var n = bytes(area.value.trim());
        save.disabled = !!max && n > max;
        over.textContent = save.disabled ? n + ' / ' + max : '';
      });
      box.appendChild(el('div', { class: 'room-edit' }, [area,
        el('div', { class: 'row' }, [save, el('button', { type: 'button', text: 'Cancel', onclick: show }), over])]));
      area.focus();
    }
    function send(next) {
      ctx.door('/v2/apply', { method: 'POST', body: { object: room.id, op: OP, args: { description: next } } })
        .then(ctx.load, function (e) { show(); ctx.note(String(e && e.message || e)); });
    }
    show();
    col.appendChild(box);
  }

  root.WallFlowers = root.WallFlowers || {};
  root.WallFlowers.Rooms = { header: header, may: may };
})(typeof window !== 'undefined' ? window : this);
