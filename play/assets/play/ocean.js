/* ocean.js — the sea past the land's edge: its waves, its wind, a boat on it,
 * and finding the way.
 *
 * land.js stops at the water. This is what is beyond it. Given a wind (where
 * it blows from, how hard, over how much open water) it says how high the sea
 * stands at any place and moment, what the wind is doing there, and what a
 * sailing boat does about both. Nothing is drawn here; sail.js draws it.
 *
 * THE SEA IS A SUM OF WAVE TRAINS, each a Stokes wave of the second order: a
 * cosine, and a second harmonic that sharpens the crest and flattens the
 * trough. How long a train is sets how fast it runs (deep water: ω² = gk), so
 * the long ones overtake the short ones and the surface does not repeat. How
 * high they stand together is set by the wind and its fetch.
 *
 * ONE FUNCTION IS THE SEA. `surface` is what a boat floats on and `sample` is
 * the square of it a picture is made from. tests/test_ocean.js holds the second
 * to the first, so what is seen is what is sailed.
 *
 * THE SAME SEED AT THE SAME MOMENT IS THE SAME SEA, on every device, with
 * nothing sent: every choice below is drawn from the seed, and the time is
 * the caller's.
 *
 * THE WIND IS NAMED BY WHERE IT COMES FROM, as a sailor names it. It swings a
 * few degrees either side of its mean, slowly, and it comes in puffs that
 * drift down wind. A puff shows on the water before it arrives (`sample`).
 *
 * THE BOAT IS FORCES, NOT A TABLE OF SPEEDS. A sail lifts and drags by the
 * angle the wind meets it at; a board and a rudder are foils in the water; the
 * hull drags; the crew's weight and the hull's shape hold her up. That she
 * cannot sail into the wind, that she slides sideways when slow, that she
 * turns the wrong way going astern, that a puff lays her over until the sheet
 * is eased: none of it is written down here. It is what the forces do.
 *
 * UNITS: metres, seconds, radians. x is east and z is south, as geo.js's plan
 * has them; a bearing is clockwise from north. PX art pixels to a metre is the
 * scale the picture is drawn at, and a body in engine.js is 26 of them tall.
 *
 * MADE UP, AND NOT KEPT. The numbers make a small dinghy that behaves; they are
 * not measurements of one. Nothing here is WallFlowers state: no Delta carries
 * a sea, a wind, a boat or a course.
 *
 * NOTHING HERE TOUCHES THE DOM, so node can run it.
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else (root.Kenjin = root.Kenjin || {}).Ocean = factory();
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  var G = 9.81, AIR = 1.225, WATER = 1025;         // gravity; what a cubic metre of each weighs
  var TAU = Math.PI * 2, DEG = Math.PI / 180, KNOT = 0.5144;
  var PX = 16;

  function clamp(v, a, b) { return v < a ? a : v > b ? b : v; }
  function lerp(a, b, t) { return a + (b - a) * t; }
  /* The least turn that takes one bearing to another: clockwise is positive. */
  function turn(from, to) {
    var d = (to - from) % TAU;
    return d > Math.PI ? d - TAU : d <= -Math.PI ? d + TAU : d;
  }
  /* How b bears from a, 0 to 2π. */
  function bearing(ax, az, bx, bz) { var b = Math.atan2(bx - ax, az - bz); return b < 0 ? b + TAU : b; }

  /* A train's period over the peak's, and its share of the height. */
  var TRAINS = [[1.25, 0.5], [1, 1], [0.8, 0.8], [0.62, 0.55], [0.47, 0.35], [0.34, 0.2]];
  var COARSE = 8;                                  // the wind on the water is worked out every so many cells

  /* o: { seed, from (degrees), speed (m/s), fetch (m), swing (degrees), gust (a share of the speed) } */
  function sea(o) {
    var seed = o.seed >>> 0, U = Math.max(0.5, o.speed), from0 = o.from * DEG, F = o.fetch || 20000;
    var swing = (o.swing === undefined ? 9 : o.swing) * DEG, gust = o.gust === undefined ? 0.28 : o.gust;
    function h(a, b) {
      var x = (Math.imul(a | 0, 374761393) + Math.imul(b | 0, 668265263) + seed) | 0;
      x = Math.imul(x ^ (x >>> 13), 1274126177);
      return ((x ^ (x >>> 16)) >>> 0) / 4294967296;
    }
    function ease(t) { return t * t * (3 - 2 * t); }
    function noise(x, z, period, salt) {
      var i = Math.floor(x / period), j = Math.floor(z / period), fx = ease(x / period - i), fz = ease(z / period - j);
      return lerp(lerp(h(i * 31 + salt, j), h((i + 1) * 31 + salt, j), fx), lerp(h(i * 31 + salt, j + 1), h((i + 1) * 31 + salt, j + 1), fx), fz);
    }

    /* How high and how long the waves are: they grow with the wind and with
       the water it has had to work on, up to a sea that can grow no more. The
       curve is the fetch-limited one of the US Army's Shore Protection Manual
       (1984) AS REMEMBERED: its coefficients have not been checked against the
       manual, so this is a shape that behaves, not a forecast. */
    var hs = Math.min(0.0016 * U * Math.sqrt(F / G), 0.2433 * U * U / G);
    var tp = Math.min(0.2857 * Math.cbrt(U * F) / Math.pow(G, 2 / 3), 8.134 * U / G);
    var toward = from0 + Math.PI, dx0 = Math.sin(toward), dz0 = -Math.cos(toward), share = 0, steep = 0;
    TRAINS.forEach(function (s) { share += s[1] * s[1]; });
    var trains = TRAINS.map(function (s, i) {
      var w = TAU / (tp * s[0]), k = w * w / G;
      var a = Math.min(hs / (4 * Math.sqrt(share / 2)) * s[1], 0.3 / k);      // together they stand hs high; none steeper than a wave can be
      var dir = toward + (h(i, 11) - 0.5) * (0.5 + 0.9 * (1 - s[0]));         // the long ones hold the wind's line; the short ones wander
      steep += a * k;
      return { a: a, b: 0.5 * k * a * a, k: k, w: w, dx: Math.sin(dir), dz: -Math.cos(dir), kx: k * Math.sin(dir), kz: -k * Math.cos(dir), ph: h(i, 12) * TAU };
    });
    /* Where crests meet they break, once there is wind enough to break them. */
    var white = clamp((U - 4) / 10, 0, 1), breakAt = steep * (0.92 - 0.42 * white);

    function height(x, z, t) {
      for (var y = 0, i = 0; i < trains.length; i++) {
        var w = trains[i], c = Math.cos(w.kx * x + w.kz * z - w.w * t + w.ph);
        y += w.a * c + w.b * (2 * c * c - 1);
      }
      return y;
    }
    /* The surface under a point: its height, how it slopes east and south, and
       how the water itself is moving (forward on a crest, back in a trough).
       With a `span`, it is the surface as a hull that long feels it: a wave
       shorter than the hull lifts one end as it drops the other, and comes to
       nothing. */
    function surface(x, z, t, out, span) {
      var y = 0, sx = 0, sz = 0, ux = 0, uz = 0;
      for (var i = 0; i < trains.length; i++) {
        var w = trains[i], th = w.kx * x + w.kz * z - w.w * t + w.ph, c = Math.cos(th), s = Math.sin(th);
        var felt = span ? Math.exp(-w.k * w.k * span * span / 24) : 1;
        var d = -(w.a * s + 4 * w.b * s * c) * felt, q = w.a * w.w * c * felt;
        y += (w.a * c + w.b * (2 * c * c - 1)) * felt;
        sx += d * w.kx; sz += d * w.kz; ux += q * w.dx; uz += q * w.dz;
      }
      out.y = y; out.sx = sx; out.sz = sz; out.ux = ux; out.uz = uz;
      return out;
    }

    var period = 70 + h(1, 21) * 80, phase = h(1, 22) * TAU;
    function wind(x, z, t, out) {
      var ax = x - dx0 * U * 0.6 * t, az = z - dz0 * U * 0.6 * t;             // a puff drifts down wind, slower than the wind
      var puff = (noise(ax, az, 75, 31) * 0.65 + noise(ax, az, 26, 32) * 0.35 - 0.5) * 2 * gust;
      out.speed = U * (1 + puff);
      out.from = from0 + swing * Math.sin(TAU * t / period + phase) + puff * 0.25;
      out.puff = gust ? puff / gust : 0;
      return out;
    }

    /* A square of the sea for a picture: n cells a side, each `cell` metres,
       its corner at x0, z0. Four bytes to a cell: the height in art pixels
       (128 is level), how far it is breaking, the wind on it (128 is the
       mean), and one left for whoever draws it. Toward its edge the square
       goes level and plain, so it can be set into a flat sea. */
    var wins = {}, rows = {}, W = {};
    function fade(n) {
      if (wins[n]) return wins[n];
      var f = wins[n] = new Float32Array(n * n), R = n / 2;
      for (var j = 0; j < n; j++) for (var i = 0; i < n; i++)
        f[j * n + i] = ease(clamp((R - 1 - Math.hypot(i + 0.5 - R, j + 0.5 - R)) / (0.38 * R), 0, 1));
      return f;
    }
    function sample(out, n, cell, x0, z0, t) {
      var f = fade(n), row = rows[n] || (rows[n] = [new Float64Array(n), new Float64Array(n)]), y = row[0], q = row[1], i, j, k;
      var m = Math.ceil(n / COARSE) + 1, puffs = new Float32Array(m * m);
      for (j = 0; j < m; j++) for (i = 0; i < m; i++)
        puffs[j * m + i] = wind(x0 + (i * COARSE + 0.5) * cell, z0 + (j * COARSE + 0.5) * cell, t, W).puff;
      for (j = 0; j < n; j++) {
        y.fill(0); q.fill(0);
        for (k = 0; k < trains.length; k++) {
          var w = trains[k], th = w.kx * (x0 + 0.5 * cell) + w.kz * (z0 + (j + 0.5) * cell) - w.w * t + w.ph;
          var c = Math.cos(th), s = Math.sin(th), cd = Math.cos(w.kx * cell), sd = Math.sin(w.kx * cell), nc, ak = w.a * w.k;
          for (i = 0; i < n; i++) {                 // one cell east is one turn of the same angle: no cosine to a cell
            y[i] += w.a * c + w.b * (2 * c * c - 1); q[i] += ak * c;
            nc = c * cd - s * sd; s = s * cd + c * sd; c = nc;
          }
        }
        var gj = j / COARSE, j0 = Math.floor(gj), fj = gj - j0;
        for (i = 0; i < n; i++) {
          var p = (j * n + i) * 4, e = f[j * n + i], gi = i / COARSE, i0 = Math.floor(gi), fi = gi - i0, g = j0 * m + i0;
          var puff = lerp(lerp(puffs[g], puffs[g + 1], fi), lerp(puffs[g + m], puffs[g + m + 1], fi), fj);
          out[p] = clamp(Math.round(y[i] * PX * e) + 128, 0, 255);
          out[p + 1] = white ? clamp(Math.round((q[i] - breakAt) / (steep - breakAt) * 255 * e), 0, 255) : 0;
          out[p + 2] = clamp(Math.round(puff * 127 * e) + 128, 0, 255);
          out[p + 3] = 0;
        }
      }
      return out;
    }

    return { seed: seed, from: from0, speed: U, fetch: F, hs: hs, tp: tp, trains: trains,
             height: height, surface: surface, wind: wind, sample: sample };
  }

  /* ── THE BOAT ── */
  /* A dinghy with one sail: about fourteen feet, one person aboard. Lengths in
     metres from the middle of the boat, forward positive; areas in square
     metres; `ce` is how far the sail's centre of effort stands above the
     board's; `hike` is the most the crew's weight can hold her up by, N·m. */
  var DINGHY = {
    mass: 165, length: 4.2, beam: 1.4, draft: 0.9,
    sail: 7.1, boom: 2.6, mast: 1.0, ce: 2.4,
    board: 0.36, boardX: 0.12, rudder: 0.13, rudderX: -1.9, lateral: 1.2,
    drag: 0.034, windage: 1.1, hike: 780, gm: 0.62, yawI: 330, rollI: 150, helm: 35 * DEG
  };
  /* How a soft sail lifts and drags by the angle the wind meets it at:
     degrees, lift, drag. The SHAPE of a sail's curves (lift rising to a stall
     near 25°, drag taking over by 90°, a flag's drag at none, and the wind
     coming at it from behind past 90°), not a measured sail. */
  var SAIL = [[0, 0, 0.1], [8, 0.55, 0.13], [16, 1.1, 0.19], [24, 1.45, 0.28], [32, 1.4, 0.42], [45, 1.15, 0.72],
              [60, 0.85, 1.0], [75, 0.5, 1.18], [90, 0.12, 1.25], [105, 0.2, 1.2], [135, 0.7, 0.75], [165, 0.5, 0.2], [180, 0, 0.1]];
  function sailAt(deg, out) {
    for (var i = 1; i < SAIL.length - 1 && SAIL[i][0] < deg; i++);
    var a = SAIL[i - 1], b = SAIL[i], t = clamp((deg - a[0]) / (b[0] - a[0]), 0, 1);
    out[0] = lerp(a[1], b[1], t); out[1] = lerp(a[2], b[2], t);
    return out;
  }
  var FOIL = 3.6, BLADE = 3.0, STALL = 1.1;        // a foil's lift to the radian (board, rudder), and where it lets go
  var YAW = [40, 60, 120], ROLL = 420;             // what resists a turn (still, moving, hard over); what steadies a roll
  var TRIM = 0.85;                                 // how much of the sail's push the crew sit out, while they can
  var BY_LEE = 168 * DEG;                          // the wind this far round, and no further, before the boom comes over
  var OVER = 1.0, DOWN = 5;                        // past this she is over; this long before she is up again

  /* o: { x, z, hdg (degrees), sheet (0 hard in … 1 right out), spec } */
  function boat(o) {
    return { spec: o.spec || DINGHY, x: o.x || 0, z: o.z || 0, vx: 0, vz: 0, hdg: (o.hdg || 0) * DEG, r: 0,
             heel: 0, heelV: 0, hike: 0, boom: 0, sheet: o.sheet === undefined ? 0.5 : o.sheet, rudder: 0,
             y: 0, pitch: 0, over: 0, dry: 0,
             awa: 0, aws: 0, twa: 0, tws: 0, from: 0, speed: 0, leeway: 0, drive: 0, luff: false };
  }

  var S = {}, A = {}, C = [0, 0];
  /* A moment of sailing. helm: { rudder (-1 … 1, positive turns her to
     starboard), sheet (0 … 1), fly (let it go) }. */
  function step(b, sea, helm, dt, t) {
    var k = b.spec, m = k.mass, hq = 0.5 * WATER;
    sea.surface(b.x, b.z, t, S, k.length); sea.wind(b.x, b.z, t, A);
    var sh = Math.sin(b.hdg), ch = Math.cos(b.hdg), fx = sh, fz = -ch, px = ch, pz = sh;       // forward, and to starboard
    var ax = -Math.sin(A.from) * A.speed - b.vx, az = Math.cos(A.from) * A.speed - b.vz;      // the air as she meets it
    var au = ax * fx + az * fz, av = ax * px + az * pz, va = Math.hypot(au, av), awa = Math.atan2(-av, -au);
    var rx = b.vx - S.ux, rz = b.vz - S.uz, u = rx * fx + rz * fz, v = rx * px + rz * pz, vw2 = u * u + v * v + 1e-4;
    var X = 0, Y = 0, N = 0, heelM = 0, down = b.over > 0;

    b.rudder += clamp(clamp(helm.rudder || 0, -1, 1) * k.helm - b.rudder, -2.5 * dt, 2.5 * dt);
    b.sheet = clamp(helm.sheet === undefined ? b.sheet : helm.sheet, 0, 1);
    if (b.dry > 0) b.dry -= dt;

    /* the sail: the boom goes to leeward as far as the sheet lets it, or until
       the sail lies along the wind and flaps. With the wind dead aft it stays
       the side it is on until the wind is well round the back of it: a gybe. */
    var off = Math.abs(awa), lee = awa >= 0 ? -1 : 1, limit = (5 + 83 * (helm.fly || b.dry > 0 ? 1 : b.sheet)) * DEG;
    var want = b.boom * lee < 0 && off > BY_LEE ? -lee * limit : lee * Math.min(limit, off), swing = (0.8 + 0.5 * va) * dt;
    b.boom += clamp(want - b.boom, -swing, swing);
    /* the wind as it meets the cloth: the angle between where it comes from
       and the sail's leading edge; lift square to the wind, on the side the
       sail's back faces, and drag down it */
    var cb = Math.cos(b.boom), sb = Math.sin(b.boom), wu = au / (va + 1e-9), wv = av / (va + 1e-9);
    var alpha = Math.acos(clamp(wv * sb - wu * cb, -1, 1)), back = sb * wu + cb * wv < 0 ? -1 : 1;
    var lu = -wv, lv = wu, turned = (lu * sb + lv * cb) * back < 0 ? -1 : 1;
    sailAt(alpha / DEG, C);
    var q = down ? 0 : 0.5 * AIR * k.sail * va * va * Math.cos(b.heel) * Math.cos(b.heel);
    var Fu = q * (C[0] * lu * turned + C[1] * wu), Fv = q * (C[0] * lv * turned + C[1] * wv);
    var reach = 0.4 * k.boom, wq = 0.5 * AIR * k.windage * va;
    X += Fu + wq * au; Y += Fv + wq * av; heelM += Fv * k.ce;
    N += (k.mast - reach * Math.cos(b.boom)) * Fv - (reach * Math.sin(b.boom) + k.ce * Math.sin(b.heel)) * Fu;

    /* the water: the hull drags, the board and the rudder are wings in it */
    var cl = clamp(FOIL * Math.abs(u) * v / vw2, -STALL, STALL), Yb = -hq * k.board * cl * vw2 * Math.cos(b.heel);
    var Yc = -hq * k.lateral * 1.1 * v * Math.abs(v);
    var vr = v + b.r * k.rudderX, vr2 = u * u + vr * vr + 1e-4;
    var cr = clamp(BLADE * (Math.abs(u) * vr + u * Math.abs(u) * Math.sin(b.rudder)) / vr2, -STALL, STALL), Yr = -hq * k.rudder * cr * vr2;
    var way = u < 0 ? -1 : 1;
    X -= hq * k.drag * (down ? 12 : u < 0 ? 3 : 1) * u * Math.abs(u) +     // a transom is no bow
       way * (Yb * Yb / (hq * vw2 * k.board * Math.PI * 3) + Math.abs(Yr * Math.sin(b.rudder)));
    Y += Yb + Yc + Yr;
    N += k.boardX * Yb + k.rudderX * Yr - (YAW[0] + YAW[1] * Math.abs(u)) * b.r - YAW[2] * b.r * Math.abs(b.r);

    /* a floating thing is pushed down the face of a wave */
    b.vx += ((X * fx + Y * px) / m - G * S.sx) * dt; b.vz += ((X * fz + Y * pz) / m - G * S.sz) * dt;
    b.x += b.vx * dt; b.z += b.vz * dt;
    b.r += N / k.yawI * dt; b.hdg = (b.hdg + b.r * dt + TAU) % TAU;

    /* how far over: the sail lays her down, the crew sit her up, the hull
       rights her, and level is whatever the water under her is */
    if (down) {
      b.over -= dt; b.heelV = 0;
      if (b.over <= 0) { b.heel = (b.heel < 0 ? -1 : 1) * 0.3; b.dry = 2.5; }                  // up again, and a moment to climb in
    } else {
      var lean = -Math.atan(S.sx * px + S.sz * pz), hike = clamp(-TRIM * heelM / k.hike, -1, 1);
      b.hike += clamp(hike - b.hike, -1.5 * dt, 1.5 * dt);
      b.heelV += ((heelM + b.hike * k.hike) * Math.cos(b.heel) - m * G * k.gm * Math.sin(b.heel - lean) - ROLL * b.heelV) / k.rollI * dt;
      b.heel += b.heelV * dt;
      if (Math.abs(b.heel) > OVER) { b.over = DOWN; b.heel = (b.heel < 0 ? -1 : 1) * 1.5; b.hike = 0; }
    }
    b.y = S.y; b.pitch += (Math.atan(S.sx * fx + S.sz * fz) - b.pitch) * (1 - Math.exp(-dt * 8));

    b.awa = awa; b.aws = va; b.tws = A.speed; b.from = A.from; b.twa = turn(b.hdg, A.from);
    b.speed = u; b.leeway = Math.atan2(v, Math.abs(u) + 0.05); b.drive = Fu; b.luff = !down && alpha < 2 * DEG;
    return b;
  }

  /* ── FINDING THE WAY ── */
  /* What a sailor calls the angle she makes with the wind. */
  function point(twa) {
    var a = Math.abs(twa) / DEG;
    return a < 32 ? 'in irons' : a < 55 ? 'close hauled' : a < 80 ? 'close reach' : a < 105 ? 'beam reach' : a < 150 ? 'broad reach' : 'running';
  }
  /* To a mark: how it bears, how far, how far off the bow, and how fast she is
     closing it, which is not how fast she is going. */
  function to(b, mx, mz, out) {
    var dx = mx - b.x, dz = mz - b.z, d = Math.hypot(dx, dz) || 1e-9;
    out.bearing = bearing(b.x, b.z, mx, mz); out.range = d; out.off = turn(b.hdg, out.bearing);
    out.made = (b.vx * dx + b.vz * dz) / d;
    return out;
  }
  /* A course laid to the wind, as a race is: each mark an angle off the wind's
     eye (degrees) and a distance (metres) from where the course starts. */
  function course(from, legs, x0, z0) {
    return legs.map(function (l) {
      var b = from + l[0] * DEG;
      return { x: (x0 || 0) + l[1] * Math.sin(b), z: (z0 || 0) - l[1] * Math.cos(b) };
    });
  }

  return { sea: sea, boat: boat, step: step, point: point, to: to, course: course, turn: turn, bearing: bearing,
           sailAt: sailAt, DINGHY: DINGHY, SAIL: SAIL, PX: PX, G: G, KNOT: KNOT, DEG: DEG };
});
