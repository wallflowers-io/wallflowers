/* hud.js — what /play and /map put over the picture: the words, and what a
 * thumb needs. On /play it also starts the story.
 *
 * THE WORDS ARE CEREMONY.JS'S, in its panel and its font: two lines where the
 * window allows, and one size for every beat, so the panel never changes shape
 * under the reader. Small: the picture is the page, and they sit under it. They are said again, off screen,
 * for a reader that cannot see a canvas.
 *
 * A STICK AND A BUTTON, on a screen that is touched; nothing on one that has
 * keys. They are boxes of the panel's own make.
 */
(function (root) {
  'use strict';
  var K = root.Kenjin, doc = root.document, A = K.Ceremony.art, P = A.P;
  function $(id) { return doc.getElementById(id); }
  var TAMA_V = '61b27f81b2';                    // site.html's: one cast, one cache entry
  var ap = 2, said = '', game = null;

  function show(canvas, art) {
    canvas.width = art.c.width; canvas.height = art.c.height;
    canvas.getContext('2d').drawImage(art.c, 0, 0);
    canvas.style.width = art.c.width * ap + 'px'; canvas.style.height = art.c.height * ap + 'px';
  }
  function words(text) {
    said = text;
    ap = root.innerWidth >= 900 ? 3 : 2;
    var avail = Math.floor(Math.min(root.innerWidth - 16, 1180) / ap), s = 1, pad = 3 + s;
    var w = Math.min(avail, Math.ceil(A.BEATS.reduce(function (m, t) { return Math.max(m, A.twoWidth(t, s)); }, 0)) + 2 * (pad + 3 * s));
    var tw = w - 2 * (pad + 3 * s), lineH = 10 * s;
    var most = A.BEATS.reduce(function (m, t) { return Math.max(m, A.wrapText(t, tw, s).length); }, 2);
    var h = most * lineH + 2 * pad + 2 * s, art = A.art(w, h), lines = A.wrapText(text, tw, s), x, y;
    A.panelBox(art, 0, 0, w, h, false);
    for (x = 3; x < w - 3; x++) { art.put(x, 3, P.panelLine); art.put(x, h - 4, P.panelLine); }
    for (y = 3; y < h - 3; y++) { art.put(3, y, P.panelLine); art.put(w - 4, y, P.panelLine); }
    var y0 = Math.round((h - (lines.length * lineH - 2 * s)) / 2);
    lines.forEach(function (line, j) {
      A.putText(art, line, pad + 3 * s + Math.round((tw - A.textW(line, s)) / 2), y0 + j * lineH, s, P.panelText, P.panelShade);
    });
    show($('words'), art);
    $('line').textContent = text;
    pads(h * ap);
  }

  /* ── the stick and the button ── */
  var BASE = 40, KNOB = 14, REACH = 13, FIELD = 60, knob = [0, 0];
  function stick() {
    var art = A.art(FIELD, FIELD), o = (FIELD - BASE) / 2;
    A.panelBox(art, o, o, BASE, BASE, true);
    A.panelBox(art, Math.round(FIELD / 2 - KNOB / 2 + knob[0]), Math.round(FIELD / 2 - KNOB / 2 + knob[1]), KNOB, KNOB, false);
    show($('stick'), art);
  }
  /* A key: a box with a triangle in it, pointing the way it sends you. */
  function key(canvas, way, down) {
    var S = 34, art = A.art(S, S), half = 6;
    A.panelBox(art, 0, 0, S, S, down);
    for (var k = 0; k <= half; k++) for (var j = -k; j <= k; j++) {
      var along = k - half / 2 - 1;
      art.put(S / 2 + (way === 'up' ? j : way === 'right' ? -along - 1 : along), S / 2 + (way === 'up' ? along : j), P.panelArrow);
    }
    show(canvas, art);
  }
  function button(down) { key($('jump'), 'up', down); }
  function pads(panelH) {
    var foot = panelH + 26;
    $('stick').style.bottom = foot - (FIELD - BASE) / 2 * ap + 'px'; $('jump').style.bottom = foot + 'px';
    stick(); button(false);
  }
  function hold(el, on, off) {
    var id = null;
    el.addEventListener('pointerdown', function (e) { id = e.pointerId; try { el.setPointerCapture(id); } catch (err) { /* nothing to capture */ } on(e); e.preventDefault(); });
    el.addEventListener('pointermove', function (e) { if (e.pointerId === id) on(e); });
    function up(e) { if (e.pointerId !== id) return; id = null; off(); }
    el.addEventListener('pointerup', up); el.addEventListener('pointercancel', up);
  }
  hold($('stick'), function (e) {
    var r = $('stick').getBoundingClientRect(), dx = (e.clientX - r.left - r.width / 2) / ap, dy = (e.clientY - r.top - r.height / 2) / ap;
    var far = Math.hypot(dx, dy), k = far > REACH ? REACH / far : 1;
    knob = [dx * k, dy * k]; stick();
    if (game) { game.input.x = knob[0] / REACH; game.input.y = -knob[1] / REACH; }
  }, function () { knob = [0, 0]; stick(); if (game) game.input.x = game.input.y = 0; });
  hold($('jump'), function () { button(true); if (game) game.input.jump = true; }, function () { button(false); if (game) game.input.jump = false; });

  root.addEventListener('resize', function () { if (said) words(said); });
  function cast() {
    return root.fetch('assets/tiles/tama/cast.json?v=' + TAMA_V)
      .then(function (r) { if (!r.ok) throw new Error('cast.json ' + r.status); return r.json(); })
      .then(function (j) { return j.cast; });
  }
  /* ── THE RADAR: San Andreas's. Round, the land under you turning with the
     view so that up is the way you are looking; a blip on it for every place,
     held at the rim when it is further off than the radar shows, so the way to
     each is always there. A block of the land to a pixel. ── */
  var RAD = 24, INK = [P.grass, P.panelHi, P.pebbleLight, P.boneShade, A.SKY[1], P.bushDark];
  function radar(d) {
    var c = $('radar'), S = RAD * 2 + 1, g = c.getContext('2d'), co = Math.cos(d.yaw), si = Math.sin(d.yaw), x, y;
    if (c.width !== S) { c.width = c.height = S; }
    var scale = root.innerWidth >= 700 ? 3 : 2, img = g.createImageData(S, S), px = img.data;
    c.style.width = c.style.height = S * scale + 'px';
    function put(x2, y2, col) {
      x2 = Math.round(x2); y2 = Math.round(y2);
      if (x2 < 0 || y2 < 0 || x2 >= S || y2 >= S) return;
      px.set([col[0], col[1], col[2], 255], (y2 * S + x2) * 4);
    }
    function dot(x2, y2, r, col) { for (var j = -r; j <= r; j++) for (var i = -r; i <= r; i++) put(x2 + i, y2 + j, Math.abs(i) === r || Math.abs(j) === r ? P.panelEdge : col); }
    for (y = -RAD; y <= RAD; y++) for (x = -RAD; x <= RAD; x++) {
      var r2 = x * x + y * y;
      if (r2 > RAD * RAD) continue;
      if (r2 > (RAD - 2) * (RAD - 2)) { put(x + RAD, y + RAD, r2 > (RAD - 1) * (RAD - 1) ? P.panelEdge : P.panelHi); continue; }
      var wx = Math.floor(d.x + co * x + si * y), wz = Math.floor(d.z - si * x + co * y);      // right and down on the radar, in the land
      put(x + RAD, y + RAD, wx < 0 || wz < 0 || wx >= d.size || wz >= d.size ? P.panelFill : INK[d.map[wz * d.size + wx]]);
    }
    function onRadar(dx, dz, reach) {                       // a point of the land, on the radar, kept inside its rim
      var rx = co * dx - si * dz, ry = si * dx + co * dz, far = Math.hypot(rx, ry);
      return far > reach ? [RAD + rx / far * reach, RAD + ry / far * reach] : [RAD + rx, RAD + ry];
    }
    var n = onRadar(0, -1e6, RAD - 5), blink = Math.floor(Date.now() / 300) & 1;
    [[-1, -2], [-1, -1], [-1, 0], [-1, 1], [-1, 2], [0, -1], [0, 0], [1, -2], [1, -1], [1, 0], [1, 1], [1, 2]].forEach(function (k) { put(n[0] + k[0], n[1] + k[1], P.white); });   // N
    d.spots.forEach(function (s) { var at = onRadar(s.x - d.x, s.z - d.z, RAD - 4); dot(at[0], at[1], s.next && blink ? 3 : 2, s.seen ? P.grassTip : P.key); });
    var nose = onRadar(d.hx * 4, d.hz * 4, 4);
    dot(RAD, RAD, 2, P.white); put(nose[0], nose[1], P.white); put((nose[0] + RAD) / 2, (nose[1] + RAD) / 2, P.white);
    g.putImageData(img, 0, 0);
  }
  /* Where you have just arrived, said for a moment, as a district's name comes up in San Andreas. */
  var saying = 0;
  function say(text) {
    var el = $('say');
    el.textContent = text; el.hidden = false;
    root.clearTimeout(saying);
    saying = root.setTimeout(function () { el.hidden = true; }, 3200);
  }

  /* Whose controls these are, and — with no words to stand over — where they sit. */
  K.Hud = { words: words, cast: cast, key: key, radar: radar, say: say, bind: function (g) { game = g; if (!said) pads(0); } };
  if (doc.body.getAttribute('data-play') === 'story') cast().then(function (c) {
    game = K.Play.now = K.Play.start(c, { canvas: $('world'), mark: $('mark'), words: words,
                                          as: new root.URLSearchParams(root.location.search).get('as') });
  });
})(typeof self !== 'undefined' ? self : this);
