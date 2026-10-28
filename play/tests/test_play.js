#!/usr/bin/env node
/*
 * /play's block of voxels (assets/play/vox.js): what the mesher draws, which
 * way each face looks, and what stops a body.
 *
 * Run: node play/tests/test_play.js
 */
'use strict';
var path = require('path');
var assert = require('assert');
var Vox = require(path.join(__dirname, '..', 'assets', 'play', 'vox.js'));

var NORMAL = [[-1, 0, 0], [1, 0, 0], [0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1]];
function quads(m) { return m.i.length / 6; }
/* Every triangle's own normal is the way its vertices say they face. */
function facesOut(m) {
  for (var t = 0; t < m.i.length; t += 3) {
    var a = m.i[t] * 6, b = m.i[t + 1] * 6, c = m.i[t + 2] * 6, v = m.v;
    var u = [v[b] - v[a], v[b + 1] - v[a + 1], v[b + 2] - v[a + 2]], w = [v[c] - v[a], v[c + 1] - v[a + 1], v[c + 2] - v[a + 2]];
    var n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]], want = NORMAL[v[a + 4]];
    assert.ok(n[0] * want[0] + n[1] * want[1] + n[2] * want[2] > 0, 'a triangle faces away from its own normal');
    assert.strictEqual(Math.abs(n[0] * want[1]) + Math.abs(n[1] * want[2]) + Math.abs(n[2] * want[0]), 0, 'a triangle is off its axis');
  }
}
function area(m) {                              // unit faces drawn, whatever they were merged into
  var sum = 0;
  for (var t = 0; t < m.i.length; t += 3) {
    var a = m.i[t] * 6, b = m.i[t + 1] * 6, c = m.i[t + 2] * 6, v = m.v;
    var u = [v[b] - v[a], v[b + 1] - v[a + 1], v[b + 2] - v[a + 2]], w = [v[c] - v[a], v[c + 1] - v[a + 1], v[c + 2] - v[a + 2]];
    sum += Math.hypot(u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]) / 2;
  }
  return sum;
}
function box(w, x0, y0, z0, x1, y1, z1, id) {
  for (var y = y0; y < y1; y++) for (var z = z0; z < z1; z++) for (var x = x0; x < x1; x++) w.set(x, y, z, id);
}

// a solid box is six rectangles, however big
var w = new Vox.World(64, 64, 64);
box(w, 10, 10, 10, 20, 22, 25, 1);
var m = Vox.meshAll(w);
assert.strictEqual(quads(m), 6);
assert.strictEqual(area(m), 2 * (10 * 12 + 10 * 15 + 12 * 15));
facesOut(m);

// a box across a chunk's edge: each chunk draws its own part, and nothing between them
w = new Vox.World(96, 64, 64);
box(w, 20, 4, 4, 44, 8, 8, 1);
m = Vox.meshAll(w);
assert.strictEqual(quads(m), 10);
assert.strictEqual(area(m), 2 * (24 * 4 + 24 * 4 + 4 * 4));
facesOut(m);

// two stuffs side by side do not merge, and the face between them is not drawn
w = new Vox.World(32, 32, 32);
box(w, 0, 0, 0, 4, 4, 4, 1); box(w, 4, 0, 0, 8, 4, 4, 2);
m = Vox.meshAll(w);
assert.strictEqual(quads(m), 10);
assert.strictEqual(area(m), 2 * (8 * 4 + 8 * 4 + 4 * 4));
var seen = {};
for (var k = 3; k < m.v.length; k += 6) seen[m.v[k]] = true;
assert.deepStrictEqual(Object.keys(seen).sort(), ['1', '2']);

// a voxel on a chunk's edge is the next chunk's business too
w = new Vox.World(64, 64, 64);
w.set(32, 5, 5, 1);
assert.strictEqual(w.dirty[w.index(1, 0, 0)], 1);
assert.strictEqual(w.dirty[w.index(0, 0, 0)], 1);
assert.strictEqual(w.dirty[w.index(1, 1, 0)], 0);
assert.strictEqual(w.chunks[w.index(0, 0, 0)], undefined, 'air is not stored');

// what stops a body: the block's sides and floor, and anything in it that is not soft
w = new Vox.World(64, 64, 64);
box(w, 0, 0, 0, 64, 8, 64, 1);
w.soft[7] = 1; box(w, 30, 8, 30, 31, 12, 31, 7);
assert.ok(w.hits(10, 7.5, 10, 12, 20, 12), 'the ground');
assert.ok(!w.hits(10, 8, 10, 12, 20, 12), 'standing on it');
assert.ok(!w.hits(29, 8, 29, 32, 20, 32), 'grass is walked through');
assert.ok(w.hits(-0.5, 8, 10, 1, 20, 12) && w.hits(10, 8, 63, 12, 20, 64.5), 'the sides are walls');
assert.ok(!w.hits(10, 60, 10, 12, 90, 12), 'the sky is open');
assert.strictEqual(w.floorUnder(30.5, 40, 30.5), 8, 'a shadow lies on the ground, not on the grass');

// cubes one at a time, 36 indices each, in the order given
m = Vox.meshCubes([5, 5, 5, 3, 9, 9, 9, 4]);
assert.strictEqual(m.i.length, 72);
assert.strictEqual(m.v[m.i[0] * 6 + 3], 3);
assert.strictEqual(m.v[m.i[36] * 6 + 3], 4);
facesOut(m);

console.log('ok: vox.js');
