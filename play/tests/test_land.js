#!/usr/bin/env node
/*
 * /map's generator (assets/play/land.js) and its geohashes (geo.js): the same
 * places make the same land, every chosen place can be walked to, nothing is
 * built on a road, and every material is made of colours ceremony.js has.
 *
 * Run: node play/tests/test_land.js
 */
'use strict';
var fs = require('fs');
var path = require('path');
var assert = require('assert');
var play = path.join(__dirname, '..', 'assets', 'play');
var Geo = require(path.join(play, 'geo.js')), Land = require(path.join(play, 'land.js'));

// geohash: the published vectors, and a cell holds the point it was made from
assert.strictEqual(Geo.encode(57.64911, 10.40744, 11), 'u4pruydqqvj');
assert.strictEqual(Geo.encode(42.6, -5.6, 5), 'ezs42');
assert.strictEqual(Geo.encode(-33.8688, 151.2093, 5), 'r3gx2');
var cell = Geo.bounds('wydm6uwg2');
assert.ok(cell.n - cell.s < 0.00005 && cell.e - cell.w < 0.00005, 'nine characters is under five metres');
assert.strictEqual(Geo.encode.apply(null, Geo.centre('wydm6uwg2').concat(9)), 'wydm6uwg2');
assert.strictEqual(Geo.bounds('wydm6uwgA'), null, 'a, i, l, o and capitals are not in the alphabet');

// real corners of Gangnam, as /map would hand them over
function sites(list) { return list.map(function (p) { return { id: Geo.encode(p[0], p[1], 9), lat: p[0], lng: p[1], name: p[2], kind: p[3] }; }); }
var FOUR = [[37.5126, 127.0590, 'A', 'mall'], [37.5172, 127.0473, 'B', 'park'], [37.5045, 127.0490, 'C', 'cafe'], [37.4979, 127.0276, 'D', 'station']];
var SETS = {
  two: FOUR.slice(0, 2), four: FOUR,
  eight: FOUR.concat([[37.5219, 127.0228, 'E', 'office'], [37.5300, 127.0400, 'F', 'bar'], [37.4900, 127.0700, 'G', 'school'], [37.5100, 127.0200, 'H', 'garden']]),
  onTopOfEachOther: [[37.5126, 127.0590, 'A', 'mall'], [37.5127, 127.0591, 'B', 'cafe'], [37.51265, 127.05905, 'C', 'bar']],
  twoDoorsAndAFarOne: [[37.5126, 127.0590, 'A', 'mall'], [37.51262, 127.05905, 'B', 'cafe'], [37.4900, 127.0100, 'C', 'park'], [37.5350, 127.0850, 'D', 'bar']],
  one: FOUR.slice(0, 1)
};
function make(list) { var names = []; return Land.generate(list, { stuff: function (name) { names.push(name); return names.length; } }); }
function bytes(L) {
  var all = [];
  L.world.chunks.forEach(function (c, i) { if (c) all.push(i + ':' + Buffer.from(c.buffer).toString('base64')); });
  return all.join('|');
}

Object.keys(SETS).forEach(function (key) {
  var list = sites(SETS[key]), L = make(list), N = L.size;

  // the same places are the same land, in whatever order they were chosen
  assert.strictEqual(bytes(make(list)), bytes(L), key + ': generated twice, it differs');
  assert.strictEqual(bytes(make(list.slice().reverse())), bytes(L), key + ': chosen in another order, it differs');
  assert.deepStrictEqual(L.spots.map(function (s) { return s.name; }), list.map(function (s) { return s.name; }), key + ': the places come back in the order chosen');

  // as the map has them: who is west of whom, who is north of whom
  L.spots.forEach(function (a, i) { L.spots.forEach(function (b, j) {
    if (i >= j) return;
    if (Math.abs(list[i].lng - list[j].lng) > 0.01) assert.strictEqual(a.x < b.x, list[i].lng < list[j].lng, key + ': ' + a.name + ' and ' + b.name + ' have changed sides');
    if (Math.abs(list[i].lat - list[j].lat) > 0.01) assert.strictEqual(a.z < b.z, list[i].lat > list[j].lat, key + ': ' + a.name + ' and ' + b.name + ' have changed ends');
  }); });
  L.spots.forEach(function (a, i) { L.spots.forEach(function (b, j) { if (i < j) assert.ok(Math.hypot(a.x - b.x, a.z - b.z) >= 30, key + ': ' + a.name + ' stands on ' + b.name); }); });

  // every place can be walked to from the first, a block up at a time, never through a wall
  var seen = new Uint8Array(N * N), queue = [[L.spots[0].x, L.spots[0].z]];
  seen[L.spots[0].z * N + L.spots[0].x] = 1;
  while (queue.length) {
    var c = queue.pop();
    [[1, 0], [-1, 0], [0, 1], [0, -1]].forEach(function (d) {
      var x = c[0] + d[0], z = c[1] + d[1];
      if (x < 1 || z < 1 || x >= N - 1 || z >= N - 1 || seen[z * N + x] || !L.open(x, z) || L.height(x, z) - L.height(c[0], c[1]) > 1) return;
      seen[z * N + x] = 1; queue.push([x, z]);
    });
  }
  L.spots.forEach(function (s) {
    assert.ok(L.open(s.x, s.z), key + ': ' + s.name + ' has something built on its forecourt');
    assert.ok(seen[s.z * N + s.x], key + ': ' + s.name + ' cannot be walked to');
    assert.ok(L.open(s.board[0], s.board[1]), key + ': ' + s.name + "'s board stands in a wall");
  });

  // a street runs on the square, and straight: a step at a time east-west or north-south, and few corners
  L.paths.forEach(function (p) {
    var turns = 0;
    p.forEach(function (q, i) {
      if (!i) return;
      assert.strictEqual(Math.abs(q[0] - p[i - 1][0]) + Math.abs(q[1] - p[i - 1][1]), 1, key + ': a road cuts a corner at ' + q);
      if (i > 1 && (q[0] - p[i - 1][0] !== p[i - 1][0] - p[i - 2][0] || q[1] - p[i - 1][1] !== p[i - 1][1] - p[i - 2][1])) turns++;
    });
    assert.ok(turns <= 8, key + ': a road with ' + turns + ' corners in ' + p.length + ' blocks is a lane, not a street');
  });
  // a road is never built on, stays dry, and climbs a block at a time
  L.paths.forEach(function (p) {
    p.forEach(function (q, i) {
      assert.ok(L.open(q[0], q[1]), key + ': a building on the road at ' + q);
      assert.ok(L.height(q[0], q[1]) > L.sea, key + ': the road is under water at ' + q);
      if (i) assert.ok(Math.abs(L.height(q[0], q[1]) - L.height(p[i - 1][0], p[i - 1][1])) <= 1, key + ': the road steps more than a block at ' + q);
    });
  });
  /* a building stands on its own ground: not on another, not on a landmark, not in water */
  var stands = L.lots.map(function (l) { return l.slice(0, 4); }).concat(L.spots.filter(function (s) { return s.lot; }).map(function (s) { return s.lot; }));
  stands.forEach(function (a, i) {
    for (var z = a[1]; z < a[1] + a[3]; z++) for (var x = a[0]; x < a[0] + a[2]; x++) {
      assert.ok(!L.open(x, z), key + ': a lot with a hole in it');
      assert.ok(L.height(x, z) >= L.sea, key + ': a building in the water at ' + [x, z]);
      assert.strictEqual(L.height(x, z), L.height(a[0], a[1]), key + ': a building on a slope at ' + [x, z]);
    }
    stands.forEach(function (b, j) {
      if (i < j) assert.ok(a[0] + a[2] <= b[0] || b[0] + b[2] <= a[0] || a[1] + a[3] <= b[1] || b[1] + b[3] <= a[1], key + ': two buildings on one lot, at ' + a + ' and ' + b);
    });
  });
  if (list.length > 1) assert.ok(L.lots.length >= 4 * (list.length - 1), key + ': ' + L.lots.length + ' buildings between ' + list.length + ' places is a field, not a street');
  // the sun's heights and the radar's map are of this land: as big as it, and a building on the map is a wall in the block
  assert.strictEqual(L.tops.length, N * N); assert.strictEqual(L.map.length, N * N);
  L.lots.forEach(function (l) { assert.strictEqual(L.map[l[1] * N + l[0]], 3); assert.ok(L.tops[l[1] * N + l[0]] > L.height(l[0], l[1]) + 3, key + ': a building no taller than its ground'); });
  L.paths.forEach(function (p) { var mid = p[p.length >> 1]; assert.ok(L.map[mid[1] * N + mid[0]] === 1 || L.map[mid[1] * N + mid[0]] === 2, key + ': a road the radar does not show'); });
});

// what kind of landmark a place gets
assert.strictEqual(Land.landmark('cafe'), 'shop');
assert.strictEqual(Land.landmark('park'), 'green');
assert.strictEqual(Land.landmark('place_of_worship'), 'hall');
assert.strictEqual(Land.landmark(undefined), 'tower');

// every material is made of colours ceremony.js has: its palette is read here, not restated
var src = fs.readFileSync(path.join(play, '..', 'how', 'ceremony.js'), 'utf8');
var table = /var P = \{\}, SRC = \{([\s\S]*?)\n  \};/.exec(src);
assert.ok(table, "ceremony.js's palette was not found where it was");
var has = {};
table[1].replace(/([A-Za-z][A-Za-z0-9]*): '#[0-9A-Fa-f]{6}'/g, function (m, name) { has[name] = true; return m; });
assert.ok(has.grass && has.paper && Object.keys(has).length > 60);
assert.ok(has.panelFill2, 'a name with a digit in it was not read');
Object.keys(Land.STUFF).concat(['awning0', 'awning6', 'car0', 'car6']).forEach(function (name) {
  var spec = Land.stuffOf(name);
  assert.strictEqual(spec.colours.length, 4, name);
  spec.colours.forEach(function (c) { assert.ok(/^(gem:[0-6]|sky:[0-5])$/.test(c) || has[c], name + ' is made of ' + c + ', which ceremony.js does not have'); });
});

console.log('ok: geo.js, land.js');
