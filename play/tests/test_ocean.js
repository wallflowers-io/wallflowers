#!/usr/bin/env node
/*
 * The sea, the wind and the boat (assets/play/ocean.js): that the picture's sea
 * is the boat's sea, that the waves run as deep water's do, and that a boat
 * made of forces does what a boat does without being told to.
 *
 * Run: node play/tests/test_ocean.js
 */
'use strict';
var path = require('path');
var assert = require('assert');
var Ocean = require(path.join(__dirname, '..', 'assets', 'play', 'ocean.js'));
var DEG = Ocean.DEG, DT = 1 / 120;

/* ── the sea ── */
var sea = Ocean.sea({ seed: 7, from: 225, speed: 8 });
var again = Ocean.sea({ seed: 7, from: 225, speed: 8 }), other = Ocean.sea({ seed: 8, from: 225, speed: 8 });
assert.strictEqual(sea.height(12.5, -40, 31.4), again.height(12.5, -40, 31.4), 'the same seed at the same moment is the same sea');
assert.notStrictEqual(sea.height(12.5, -40, 31.4), other.height(12.5, -40, 31.4), 'another seed is another sea');

// deep water: a train's length sets how fast it runs, and the long ones run faster
sea.trains.forEach(function (w, i) {
  assert.ok(Math.abs(w.w * w.w - Ocean.G * w.k) < 1e-9, 'a train is off deep water\'s rule');
  assert.ok(w.a * w.k <= 0.3 + 1e-9, 'a train is steeper than a wave can stand');
  if (i) assert.ok(w.w / w.k < sea.trains[i - 1].w / sea.trains[i - 1].k, 'a shorter train outruns a longer one');
});
// together they stand as high as the wind and the fetch say: four times the surface's spread
var n = 0, sum = 0, sq = 0;
for (var z = -400; z < 400; z += 7.3) for (var x = -400; x < 400; x += 7.3) { var y = sea.height(x, z, 3); n++; sum += y; sq += y * y; }
var spread = Math.sqrt(sq / n - sum * sum / n / n);
assert.ok(Math.abs(4 * spread - sea.hs) < 0.15 * sea.hs, 'the sea stands ' + (4 * spread).toFixed(2) + ' m, and was to stand ' + sea.hs.toFixed(2));
// more wind, and more water for it to work on, is a bigger and a longer sea
assert.ok(Ocean.sea({ seed: 7, from: 0, speed: 12 }).hs > sea.hs && Ocean.sea({ seed: 7, from: 0, speed: 12 }).tp > sea.tp);
assert.ok(Ocean.sea({ seed: 7, from: 0, speed: 8, fetch: 2000 }).hs < sea.hs);

// the slope and the height are one surface
var S = {}, e = 1e-4;
[[0, 0, 0], [13.7, -2.2, 5], [-80, 41, 99.5]].forEach(function (p) {
  sea.surface(p[0], p[1], p[2], S);
  assert.ok(Math.abs(S.y - sea.height(p[0], p[1], p[2])) < 1e-12);
  assert.ok(Math.abs(S.sx - (sea.height(p[0] + e, p[1], p[2]) - sea.height(p[0] - e, p[1], p[2])) / (2 * e)) < 1e-6, 'the slope east is not the height\'s');
  assert.ok(Math.abs(S.sz - (sea.height(p[0], p[1] + e, p[2]) - sea.height(p[0], p[1] - e, p[2])) / (2 * e)) < 1e-6, 'the slope south is not the height\'s');
});
// a hull feels the long waves and not the short ones
var whole = sea.surface(3, 4, 5, {}), felt = sea.surface(3, 4, 5, {}, 400);
assert.ok(Math.abs(felt.y) < 1e-6 && Math.abs(felt.sx) < 1e-6 && Math.abs(whole.y) > 1e-3);

// THE PICTURE'S SEA IS THE BOAT'S: every cell of the square is the height at its middle, to the art pixel
var N = 64, CELL = 0.5, out = new Uint8Array(N * N * 4), worst = 0, level = 0, checked = 0;
sea.sample(out, N, CELL, -20, 35, 12.25);
for (var j = 0; j < N; j++) for (var i = 0; i < N; i++) {
  var p = (j * N + i) * 4, far = Math.hypot(i + 0.5 - N / 2, j + 0.5 - N / 2);
  if (far < N / 2 * 0.6) {
    checked++;
    worst = Math.max(worst, Math.abs(out[p] - 128 - sea.height(-20 + (i + 0.5) * CELL, 35 + (j + 0.5) * CELL, 12.25) * Ocean.PX));
  }
  if (far >= N / 2 - 1 && (out[p] !== 128 || out[p + 1] !== 0 || out[p + 2] !== 128)) level++;
}
assert.ok(checked > 1000 && worst <= 0.5 + 1e-6, 'the picture is ' + worst.toFixed(2) + ' art pixels off the sea');
assert.strictEqual(level, 0, 'the square does not go level and plain at its edge');
// crests break in a breeze and not in an air
function foam(speed) {
  var s = Ocean.sea({ seed: 7, from: 225, speed: speed }), o = new Uint8Array(N * N * 4), c = 0;
  for (var t = 0; t < 20; t += 1.7) { s.sample(o, N, CELL, 0, 0, t); for (var q = 1; q < o.length; q += 4) if (o[q] > 0) c++; }
  return c;
}
assert.strictEqual(foam(3), 0, 'an air does not break a wave');
assert.ok(foam(13) > foam(7) && foam(7) > 0, 'more wind does not break more waves');

/* ── the wind ── */
var W = {}, lo = 1e9, hi = 0, mean = 0, swung = 0, c = 0;
for (var t = 0; t < 600; t += 3.1) for (x = -300; x <= 300; x += 150) {
  sea.wind(x, x * 0.7, t, W);
  lo = Math.min(lo, W.speed); hi = Math.max(hi, W.speed); mean += W.speed; c++;
  swung = Math.max(swung, Math.abs(Ocean.turn(225 * DEG, W.from)));
}
assert.ok(lo >= 8 * 0.72 - 1e-9 && hi <= 8 * 1.28 + 1e-9 && hi - lo > 1, 'the puffs are out of their bounds, or there are none');
assert.ok(Math.abs(mean / c - 8) < 0.4, 'the wind does not average what it was set to');
assert.ok(swung > 5 * DEG && swung < 14 * DEG, 'the wind swings ' + (swung / DEG).toFixed(1) + ' degrees');

/* ── the boat: steady wind from the north, flat water, so what is seen is the boat ── */
function flat(speed) { return Ocean.sea({ seed: 1, from: 0, speed: speed, fetch: 1, gust: 0, swing: 0 }); }
/* Held to a course by a hand on the tiller for a while; what she settles to. */
function sail(speed, course, sheet, secs, fly) {
  var s = flat(speed), b = Ocean.boat({ hdg: course, sheet: sheet }), helm = { rudder: 0, sheet: sheet, fly: fly }, t = 0, m = 0;
  var r = { u: 0, upwind: 0, leeway: 0, heel: 0, over: false, most: 0 };
  b.vx = Math.sin(course * DEG) * 1.5; b.vz = -Math.cos(course * DEG) * 1.5;
  for (var k = 0; k < secs / DT; k++, t += DT) {
    helm.rudder = Math.max(-1, Math.min(1, 2.5 * Ocean.turn(b.hdg, course * DEG) - 1.2 * b.r));
    Ocean.step(b, s, helm, DT, t);
    if (b.over > 0) r.over = true;
    r.most = Math.max(r.most, Math.abs(b.heel));
    if (t > secs - 8) { m++; r.u += b.speed; r.upwind += -b.vz; r.leeway += b.leeway; r.heel += b.heel; }
  }
  r.u /= m; r.upwind /= m; r.leeway /= m; r.heel /= m; r.boat = b;
  return r;
}
function best(speed, course) {                  // the sheet that suits the course
  var top = null;
  for (var sh = 0; sh <= 1.001; sh += 0.1) { var r = sail(speed, course, sh, 35); if (!r.over && (!top || r.u > top.u)) { top = r; top.sheet = sh; } }
  return top;
}
var hauled = best(6.5, 50), reach = best(6.5, 90), run = best(6.5, 180), pinch = best(6.5, 25);

// she cannot sail into the wind: pointed at it the sail only holds her back and her way runs off, and close to it she barely moves
var irons = Ocean.boat({ hdg: 0, sheet: 0.2 }), pulled = 0, still = flat(6.5);
irons.vz = -2;
for (var k = 0; k < 5 / DT; k++) { Ocean.step(irons, still, { rudder: 0, sheet: 0.2 }, DT, k * DT); if (Math.abs(irons.twa) < 15 * DEG && irons.drive >= 0) pulled++; }
assert.strictEqual(pulled, 0, 'a sail drew within fifteen degrees of the wind');
assert.ok(irons.speed < 0.5, 'head to wind five seconds she still makes ' + irons.speed.toFixed(2) + ' m/s');
assert.ok(pinch.upwind < 0.7 * hauled.upwind, 'pinching makes ground to windward as well as sailing does');
// but off the wind a little, she works up into it
assert.ok(hauled.upwind > 1 && hauled.u > 2, 'close hauled she makes ' + hauled.upwind.toFixed(2) + ' m/s to windward');
// a reach is the fast way, and a run is slower than a reach
assert.ok(reach.u > hauled.u && reach.u > run.u, 'reach ' + reach.u.toFixed(2) + ', hauled ' + hauled.u.toFixed(2) + ', run ' + run.u.toFixed(2));
// the sheet matters: in hard for the wind ahead, out for the wind behind
assert.ok(hauled.sheet < 0.3 && run.sheet > 0.6, 'hauled at ' + hauled.sheet.toFixed(1) + ', running at ' + run.sheet.toFixed(1));
assert.ok(sail(6.5, 180, 0, 35).u < 0.75 * run.u, 'a sail hard in on a run pulls as well as one squared off');
assert.ok(sail(6.5, 90, 1, 35).u < 0.5 * reach.u, 'a sail let right out on a reach pulls as well as one trimmed');
// she slides to leeward, a little at speed and a lot when slow
assert.ok(hauled.leeway > 1 * DEG && hauled.leeway < 8 * DEG, 'close hauled she makes ' + (hauled.leeway / DEG).toFixed(1) + ' degrees of leeway');
assert.ok(pinch.leeway > 2 * hauled.leeway, 'slow, she slides no more than fast');
// the wind over the port side lays her to starboard, and the crew sit to port
assert.ok(hauled.heel > 0 && hauled.boat.hike < 0 && hauled.boat.boom > 0 && hauled.boat.awa < 0);
var port = sail(6.5, 310, hauled.sheet, 35);
assert.ok(port.heel < 0 && port.boat.hike > 0 && port.boat.boom < 0 && port.boat.awa > 0, 'the other tack is not the mirror of this one');
assert.ok(Math.abs(port.u - hauled.u) < 0.05, 'one tack is faster than the other');
// more wind than she can carry puts her over; letting the sheet fly keeps her up
assert.ok(sail(16, 60, 0.1, 30).over, 'she carries full sail in a gale');
var flown = sail(16, 60, 0.1, 30, true);
assert.ok(!flown.over && flown.most < 30 * DEG, 'with the sheet let fly she still goes ' + (flown.most / DEG).toFixed(0) + ' degrees over');
assert.ok(!sail(6.5, 60, 0.1, 30).over, 'she goes over in a breeze');

// going astern the rudder works backwards
var b = Ocean.boat({ hdg: 0, sheet: 1 }), s = flat(6), tt = 0;
b.vz = 1.5;
for (i = 0; i < 2 / DT; i++, tt += DT) Ocean.step(b, s, { rudder: 1, sheet: 1, fly: true }, DT, tt);
assert.ok(b.speed < 0 && Ocean.turn(0, b.hdg) < -5 * DEG, 'astern, helm to starboard took her bow ' + (Ocean.turn(0, b.hdg) / DEG).toFixed(0));
b = Ocean.boat({ hdg: 90, sheet: 0.4 }); b.vx = 3; tt = 0;
for (i = 0; i < 2 / DT; i++, tt += DT) Ocean.step(b, s, { rudder: 1, sheet: 0.4 }, DT, tt);
assert.ok(Ocean.turn(90 * DEG, b.hdg) > 20 * DEG, 'ahead, helm to starboard did not take her bow to starboard');

// a tack: from close hauled on one side, through the wind, to sailing on the other
b = sail(6.5, 50, 0.1, 25).boat; tt = 25;
for (i = 0; i < 12 / DT; i++, tt += DT)
  Ocean.step(b, s = flat(6.5), { rudder: Math.max(-1, Math.min(1, 2.5 * Ocean.turn(b.hdg, 310 * DEG) - 1.2 * b.r)), sheet: 0.1 }, DT, tt);
assert.ok(Math.abs(Ocean.turn(b.hdg, 310 * DEG)) < 6 * DEG && b.speed > 1.5 && b.boom < 0, 'she did not come through the wind and sail on');

// a hull with no sail is rocked by the waves and not carried off by them
var bare = Ocean.boat({ spec: Object.assign({}, Ocean.DINGHY, { sail: 0, windage: 0 }) }), swell = Ocean.sea({ seed: 3, from: 300, speed: 9 }), fast = 0;
for (i = 0, tt = 0; i < 120 / DT; i++, tt += DT) { Ocean.step(bare, swell, { rudder: 0, sheet: 1 }, DT, tt); fast = Math.max(fast, Math.hypot(bare.vx, bare.vz)); }
assert.ok(fast > 0.2 && Math.hypot(bare.x, bare.z) < 20, 'adrift two minutes she is ' + Math.hypot(bare.x, bare.z).toFixed(1) + ' m off');

/* ── finding the way ── */
assert.strictEqual(Ocean.bearing(0, 0, 0, -10), 0, 'north');
assert.ok(Math.abs(Ocean.bearing(0, 0, 10, 0) - 90 * DEG) < 1e-12 && Math.abs(Ocean.bearing(0, 0, 0, 10) - 180 * DEG) < 1e-12 && Math.abs(Ocean.bearing(0, 0, -10, 0) - 270 * DEG) < 1e-12);
assert.ok(Math.abs(Ocean.turn(350 * DEG, 10 * DEG) - 20 * DEG) < 1e-12 && Math.abs(Ocean.turn(10 * DEG, 350 * DEG) + 20 * DEG) < 1e-12, 'the least turn goes through north');
var marks = Ocean.course(40 * DEG, [[0, 100], [120, 60]], 5, 5);
assert.ok(Math.abs(Ocean.bearing(5, 5, marks[0].x, marks[0].z) - 40 * DEG) < 1e-9, 'the windward mark is not in the wind\'s eye');
assert.ok(Math.abs(Ocean.bearing(5, 5, marks[1].x, marks[1].z) - 160 * DEG) < 1e-9);
var T = Ocean.to({ x: 0, z: 0, vx: 3, vz: 0, hdg: 90 * DEG }, 100, -100, {});
assert.ok(Math.abs(T.range - Math.sqrt(2) * 100) < 1e-9 && Math.abs(T.bearing - 45 * DEG) < 1e-9 && Math.abs(T.off + 45 * DEG) < 1e-9);
assert.ok(Math.abs(T.made - 3 * Math.SQRT1_2) < 1e-9, 'how fast she closes a mark is not how fast she goes');
assert.deepStrictEqual([0, 45, 70, 90, 130, 180].map(function (a) { return Ocean.point(a * DEG); }),
  ['in irons', 'close hauled', 'close reach', 'beam reach', 'broad reach', 'running']);

console.log('ok: ocean.js  (6.5 m/s: close hauled ' + hauled.u.toFixed(1) + ' m/s and ' + hauled.upwind.toFixed(1) + ' to windward, reach ' + reach.u.toFixed(1) + ', run ' + run.u.toFixed(1) + ')');
