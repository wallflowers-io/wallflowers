#!/usr/bin/env node
/*
 * /map's land in shapes (assets/play/townmesh.js) and the looks it can wear
 * (skins.js): every look colours every stuff, the shapes are of the land
 * land.js made, and every chosen place can still be walked to now that walls,
 * hedges and cars are what stop you and blocks are not.
 *
 * Run: node play/tests/test_town.js
 */
'use strict';
var path = require('path');
var assert = require('assert');
var play = path.join(__dirname, '..', 'assets', 'play');
var Geo = require(path.join(play, 'geo.js')), Land = require(path.join(play, 'land.js')), Skins = require(path.join(play, 'skins.js')), TM = require(path.join(play, 'townmesh.js'));
var G = TM.G, GEMS = [[139, 161, 200], [234, 221, 62], [215, 147, 181], [220, 118, 41], [84, 108, 172], [205, 91, 122], [166, 200, 224]];

// ── the looks ──
assert.ok(Skins.MATS.length <= 48, 'more stuff than the shader has room for');
assert.strictEqual(new Set(Skins.MATS).size, Skins.MATS.length);
Skins.MATS.forEach(function (name) { assert.ok(/^gem[0-6]$/.test(name) || /^#[0-9A-F]{6}$/i.test(Skins.BASE[name]), name + ' has no plain colour'); });
assert.ok(Skins.LIST.length >= 4);
assert.strictEqual(new Set(Skins.LIST.map(function (s) { return s.id; })).size, Skins.LIST.length);
var seen = {};
Skins.LIST.forEach(function (s) {
  var r = Skins.resolve(s.id, GEMS), key = '';
  Object.keys(s.own || {}).concat(s.glow || [], s.keep || []).forEach(function (name) { assert.ok(Skins.index[name] !== undefined, s.id + ' names ' + name + ', which is no stuff'); });
  for (var i = 0; i < Skins.MATS.length * 3; i++) { assert.ok(r.pal[i] >= 0 && r.pal[i] <= 1, s.id + ': a colour out of range'); key += r.pal[i].toFixed(2); }
  assert.ok(Math.abs(Math.hypot(r.sun[0], r.sun[1], r.sun[2]) - 1) < 1e-6 && r.sun[1] > 0, s.id + ': the sun is not up');
  assert.ok(!seen[key + r.lines + r.cel + r.ink], s.id + ' looks exactly like ' + seen[key + r.lines + r.cel + r.ink]);
  seen[key + r.lines + r.cel + r.ink] = s.id;
  if (!r.lines) assert.ok(Math.hypot(r.pal[0] - r.sky[0][0], r.pal[1] - r.sky[0][1], r.pal[2] - r.sky[0][2]) > 0.08, s.id + ': the grass is the colour of the sky');
});
assert.strictEqual(Skins.resolve('no such look', GEMS).id, Skins.LIST[0].id, 'a look that is not there is the first');
// the wordmark's colours are handed in, not written down: a gem is whatever it is given
assert.deepStrictEqual(Array.from(Skins.resolve('dollhouse', [[10, 20, 30]].concat(GEMS.slice(1))).pal.slice(Skins.index.gem0 * 3, Skins.index.gem0 * 3 + 3)).map(function (v) { return Math.round(v * 255) > 0; }), [true, true, true]);
var clay = Skins.resolve('maquette', GEMS);
assert.deepStrictEqual(Array.from(clay.pal.slice(Skins.index.brick * 3, Skins.index.brick * 3 + 3)), Array.from(clay.pal.slice(Skins.index.grass * 3, Skins.index.grass * 3 + 3)), 'a maquette is one clay');
assert.notDeepStrictEqual(Array.from(clay.pal.slice(Skins.index.mark * 3, Skins.index.mark * 3 + 3)), Array.from(clay.pal.slice(Skins.index.brick * 3, Skins.index.brick * 3 + 3)), 'but a chosen place stands out of it');

// ── the shapes ──
function sites(list) { return list.map(function (p) { return { id: Geo.encode(p[0], p[1], 9), lat: p[0], lng: p[1], name: p[2], kind: p[3] }; }); }
var FOUR = [[37.5126, 127.0590, 'A', 'mall'], [37.5172, 127.0473, 'B', 'park'], [37.5045, 127.0490, 'C', 'cafe'], [37.4979, 127.0276, 'D', 'station']];
var SETS = { two: FOUR.slice(0, 2), four: FOUR, eight: FOUR.concat([[37.5219, 127.0228, 'E', 'office'], [37.5300, 127.0400, 'F', 'bar'], [37.4900, 127.0700, 'G', 'school'], [37.5100, 127.0200, 'H', 'garden']]) };
Object.keys(SETS).forEach(function (key) {
  var n = 0, L = Land.generate(sites(SETS[key]), { stuff: function () { return ++n; } }), T = TM.build(L), N = L.size, i;
  assert.strictEqual(T.mesh.p.length, T.mesh.n * 3); assert.strictEqual(T.mesh.m.length, T.mesh.n * 4); assert.strictEqual(T.mesh.n % 3, 0);
  for (i = 0; i < T.mesh.p.length; i++) assert.ok(isFinite(T.mesh.p[i]), key + ': a corner is nowhere');
  for (i = 0; i < T.mesh.m.length; i += 4) assert.ok(T.mesh.m[i] < Skins.MATS.length, key + ': a triangle made of stuff number ' + T.mesh.m[i]);
  for (i = 0; i < T.mesh.p.length; i += 3) assert.ok(T.mesh.p[i] >= -30 && T.mesh.p[i] <= N * G + 30 && T.mesh.p[i + 1] >= -9 && T.mesh.p[i + 1] <= L.tall * G && T.mesh.p[i + 2] >= -30 && T.mesh.p[i + 2] <= N * G + 30, key + ': a corner off the land');
  assert.strictEqual(T.tops.length, N * N);

  // the ground that is drawn is the ground land.js levelled: flat across a road, a pavement, a forecourt
  L.paths.forEach(function (p) { p.forEach(function (q) { assert.ok(Math.abs(T.heightAt((q[0] + 0.5) * G, (q[1] + 0.5) * G) - L.height(q[0], q[1]) * G) < 4.1, key + ': the road at ' + q.slice(0, 2) + ' is not where its blocks were'); }); });
  L.spots.forEach(function (s) { assert.strictEqual(T.heightAt((s.x + 0.5) * G, (s.z + 0.5) * G), s.level * G, key + ': ' + s.name + "'s forecourt is not level"); });
  // where anything paved or built touches a corner of the ground, the corner is at its level: the land does the sloping, not the street
  for (var cz = 1; cz < N; cz++) for (var cx = 1; cx < N; cx++) {
    var hard = -1;
    [[-1, -1], [0, -1], [-1, 0], [0, 0]].forEach(function (d) { var f = (cz + d[1]) * N + cx + d[0]; if (L.map[f] >= 1 && L.map[f] <= 3) hard = Math.max(hard, L.heights[f]); });
    if (hard >= 0) assert.strictEqual(T.heightAt(cx * G, cz * G), hard * G, key + ': a kerb that sags at ' + [cx, cz]);
  }
  // a building is a wall you cannot walk through, and stands as tall as its blocks did
  L.builds.forEach(function (b) {
    assert.ok(T.inTheWay((b.x0 + b.w / 2) * G, (b.z0 + b.d / 2) * G, 1), key + ': a building you can stand inside');
    assert.ok(T.tops[b.z0 * N + b.x0] >= b.top * 4, key + ': a building that casts no shadow');
  });
  assert.strictEqual(T.rects.length, L.builds.length + L.hedges.length + L.parked.length);

  // every place can be walked to from the first: now by its walls, hedges, cars and trunks, half a block at a time
  var R = 4.5, C = 4, W = Math.floor(N * G / C), reach = new Uint8Array(W * W), s0 = L.spots[0], start = [Math.round((s0.x - 1.5) * G / C), Math.round((s0.z + 1.5) * G / C)], queue = [start];
  assert.ok(!T.inTheWay(start[0] * C, start[1] * C, R), key + ': you come down inside something');
  reach[start[1] * W + start[0]] = 1;
  while (queue.length) {
    var c = queue.pop();
    [[1, 0], [-1, 0], [0, 1], [0, -1]].forEach(function (d) {
      var x = c[0] + d[0], z = c[1] + d[1];
      if (x < 0 || z < 0 || x >= W || z >= W || reach[z * W + x] || T.inTheWay(x * C, z * C, R)) return;
      reach[z * W + x] = 1; queue.push([x, z]);
    });
  }
  L.spots.forEach(function (s) {                 // within arm's reach of its sign, which is where arriving is counted from
    var bx = (s.board[0] + 0.5) * G, bz = (s.board[1] + 0.5) * G, got = false;
    for (var z = 0; z < W && !got; z++) for (var x = 0; x < W && !got; x++) if (reach[z * W + x] && Math.hypot(x * C - bx, z * C - bz) < 30) got = true;
    assert.ok(got, key + ': ' + s.name + ' cannot be walked to');
  });
});

// the parts that stand by themselves
[TM.sign(0), TM.sign(9), TM.diamond('green'), TM.diamond('gold')].forEach(function (m) { assert.ok(m.n >= 24 && m.n % 3 === 0); });
// a figure's own colours ride in the corners, and the shader is told so by stuff number 255
var one = new TM.Mesh(); one.tri([0, 0, 0], [1, 0, 0], [0, 1, 0], [12, 34, 56]);
assert.deepStrictEqual(Array.from(one.done().m.slice(0, 4)), [255, 12, 34, 56]);
assert.ok(Skins.MATS.length < 255);

console.log('ok: skins.js, townmesh.js');
