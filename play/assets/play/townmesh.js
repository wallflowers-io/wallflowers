/* townmesh.js — a land as shapes, not blocks.
 *
 * land.js builds its land out of blocks and also says in words what it built.
 * This draws those words: ground that rolls instead of stepping, walls that are
 * walls, pitched roofs, round trees, palms that lean, cars with wheels. It is
 * the same land — the same places, roads and lots, the same heights under your
 * feet — with no voxel in it.
 *
 * A TRIANGLE IS THREE CORNERS AND A STUFF. No normals, no winding to get right:
 * the renderer works a face's slant out from the picture it is drawing
 * (town.js), so everything is flat-faced, which is the look. A corner carries a
 * shade as well, so one house's brick is not quite the next's.
 *
 * WHAT STOPS YOU IS SAID HERE TOO, since blocks no longer do it: a rectangle
 * for a building, a hedge, a car; a circle for a trunk or a post; and the height
 * of the ground at any point, the same rolling surface that is drawn.
 *
 * NOTHING HERE TOUCHES THE DOM OR THE GPU, so node can run it.
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory(require('./skins.js'));
  else (root.Kenjin = root.Kenjin || {}).TownMesh = factory(root.Kenjin.Skins);
})(typeof self !== 'undefined' ? self : this, function (Skins) {
  'use strict';
  var G = 8, STOREY = 32, I = Skins.index;        // art pixels to a block, and to a storey

  function Mesh() { this.p = []; this.m = []; }
  Mesh.prototype.tri = function (a, b, c, mat, tint) {     // mat: a stuff's number, or a colour of its own as [r, g, b]
    this.p.push(a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2]);
    if (mat.length) { for (var k = 0; k < 3; k++) this.m.push(255, mat[0], mat[1], mat[2]); return; }
    var t = tint === undefined ? 128 : Math.max(0, Math.min(255, Math.round(tint)));
    this.m.push(mat, t, 0, 0, mat, t, 0, 0, mat, t, 0, 0);
  };
  Mesh.prototype.quad = function (a, b, c, d, mat, tint) { this.tri(a, b, c, mat, tint); this.tri(a, c, d, mat, tint); };
  Mesh.prototype.box = function (x0, y0, z0, x1, y1, z1, mat, tint) {
    this.quad([x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1], mat, tint);
    this.quad([x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0], mat, tint);
    this.quad([x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1], mat, tint);
    this.quad([x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0], mat, tint);
    this.quad([x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0], mat, tint);
  };
  Mesh.prototype.cyl = function (cx, cz, r, y0, y1, n, mat, tint, r1) {      // r1: the radius at the top, if it tapers
    var top = r1 === undefined ? r : r1;
    for (var k = 0; k < n; k++) {
      var a = k / n * 6.2832, b = (k + 1) / n * 6.2832, ca = Math.cos(a), sa = Math.sin(a), cb = Math.cos(b), sb = Math.sin(b);
      this.quad([cx + ca * r, y0, cz + sa * r], [cx + cb * r, y0, cz + sb * r], [cx + cb * top, y1, cz + sb * top], [cx + ca * top, y1, cz + sa * top], mat, tint);
      this.tri([cx, y1, cz], [cx + ca * top, y1, cz + sa * top], [cx + cb * top, y1, cz + sb * top], mat, tint);
    }
  };
  /* A ball of twenty faces: a tree's crown, a head. `rough` pushes its corners in and out. */
  var T = (1 + Math.sqrt(5)) / 2, ICO = [[-1, T, 0], [1, T, 0], [-1, -T, 0], [1, -T, 0], [0, -1, T], [0, 1, T], [0, -1, -T], [0, 1, -T], [T, 0, -1], [T, 0, 1], [-T, 0, -1], [-T, 0, 1]]
    .map(function (v) { var l = Math.hypot(v[0], v[1], v[2]); return [v[0] / l, v[1] / l, v[2] / l]; });
  var FACES = [[0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11], [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
               [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9], [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1]];
  Mesh.prototype.ball = function (cx, cy, cz, rx, ry, rz, mat, tint, rough, seed, keep) {   // keep(v): which faces, by their middle
    var v = ICO.map(function (p, i) { var k = 1 + (rough || 0) * (hash(i + (seed || 0) * 13, 77) - 0.5); return [cx + p[0] * rx * k, cy + p[1] * ry * k, cz + p[2] * rz * k]; });
    for (var f = 0; f < 20; f++) {
      var a = ICO[FACES[f][0]], b = ICO[FACES[f][1]], c = ICO[FACES[f][2]];
      if (keep && !keep([(a[0] + b[0] + c[0]) / 3, (a[1] + b[1] + c[1]) / 3, (a[2] + b[2] + c[2]) / 3])) continue;
      this.tri(v[FACES[f][0]], v[FACES[f][1]], v[FACES[f][2]], mat, (tint === undefined ? 128 : tint) + (f % 3 - 1) * 5);
    }
  };
  Mesh.prototype.done = function () { return { p: new Float32Array(this.p), m: new Uint8Array(this.m), n: this.p.length / 3 }; };

  function hash(x, y) {
    var h = (Math.imul(x | 0, 374761393) + Math.imul(y | 0, 668265263)) | 0;
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
  }
  function soft(x, z, period) {                  // a slow wander between 0 and 1: so a lawn is not one flat green
    var i = Math.floor(x / period), j = Math.floor(z / period), fx = x / period - i, fz = z / period - j;
    fx = fx * fx * (3 - 2 * fx); fz = fz * fz * (3 - 2 * fz);
    return (hash(i, j) * (1 - fx) + hash(i + 1, j) * fx) * (1 - fz) + (hash(i, j + 1) * (1 - fx) + hash(i + 1, j + 1) * fx) * fz;
  }

  function build(L) {
    var N = L.size, H = L.heights, map = L.map, M = new Mesh(), rects = [], rounds = [], lamps = [], x, z, k;
    function cls(x2, z2) { return x2 < 0 || z2 < 0 || x2 >= N || z2 >= N ? -1 : map[z2 * N + x2]; }

    /* ── THE GROUND: a height to every corner. Where a road, a pavement or a
       building touches a corner it is at their level, so they lie flat to their
       edges and the land does the sloping; elsewhere it is the mean of the four
       fields that meet there. ── */
    var Y = new Float32Array((N + 1) * (N + 1));
    for (z = 0; z <= N; z++) for (x = 0; x <= N; x++) {
      var sum = 0, n = 0, hard = -1;
      for (var dz = -1; dz <= 0; dz++) for (var dx = -1; dx <= 0; dx++) {
        var c = cls(x + dx, z + dz);
        if (c < 0) continue;
        var hh = H[(z + dz) * N + x + dx];
        sum += hh; n++;
        if (c >= 1 && c <= 3) hard = Math.max(hard, hh);
      }
      Y[z * (N + 1) + x] = (hard >= 0 ? hard : sum / n) * G;
    }
    function corner(x2, z2) { return [x2 * G, Y[z2 * (N + 1) + x2], z2 * G]; }
    function heightAt(px, pz) {
      var fx = Math.max(0, Math.min(N - 0.001, px / G)), fz = Math.max(0, Math.min(N - 0.001, pz / G)), i = Math.floor(fx), j = Math.floor(fz), u = fx - i, v = fz - j;
      return (Y[j * (N + 1) + i] * (1 - u) + Y[j * (N + 1) + i + 1] * u) * (1 - v) + (Y[(j + 1) * (N + 1) + i] * (1 - u) + Y[(j + 1) * (N + 1) + i + 1] * u) * v;
    }
    for (z = 0; z < N; z++) for (x = 0; x < N; x++) {
      var kind = map[z * N + x], wet = kind === 4, shore = !wet && (cls(x - 1, z) === 4 || cls(x + 1, z) === 4 || cls(x, z - 1) === 4 || cls(x, z + 1) === 4);
      var mat = kind === 1 ? I.asphalt : kind === 2 || kind === 3 ? I.pave : wet || (shore && kind !== 5) ? I.sand : I.grass;
      var tint = mat === I.grass ? 96 + soft(x, z, 7) * 70 : mat === I.pave ? ((x + z) & 1 ? 120 : 136) : 122 + hash(x, z) * 12;
      var a = corner(x, z), b = corner(x + 1, z), c2 = corner(x + 1, z + 1), d = corner(x, z + 1);
      if ((x + z) & 1) M.quad(a, b, c2, d, mat, tint); else M.quad(b, c2, d, a, mat, tint);
      if (wet && H[z * N + x] >= L.sea) M.quad([x * G, (H[z * N + x] + 0.6) * G, z * G], [(x + 1) * G, (H[z * N + x] + 0.6) * G, z * G],
                                               [(x + 1) * G, (H[z * N + x] + 0.6) * G, (z + 1) * G], [x * G, (H[z * N + x] + 0.6) * G, (z + 1) * G], I.water);   // a pond, up on its own level
      var lane = L.lanes[z * N + x];
      if (lane && kind === 1) {                    // the line down the middle of the road, a dash to a block
        var cy = heightAt((x + 0.5) * G, (z + 0.5) * G) + 0.35, lx = lane === 1 ? 2.6 : 0.6, lz = lane === 1 ? 0.6 : 2.6, mx = (x + 0.5) * G, mz = (z + 0.5) * G;
        M.quad([mx - lx, cy, mz - lz], [mx + lx, cy, mz - lz], [mx + lx, cy, mz + lz], [mx - lx, cy, mz + lz], I.line);
      }
    }
    for (k = 0; k < N; k++) {                      // the land's cut sides
      M.quad(corner(k, 0), corner(k + 1, 0), [(k + 1) * G, 0, 0], [k * G, 0, 0], I.dirt, 118);
      M.quad(corner(k, N), corner(k + 1, N), [(k + 1) * G, 0, N * G], [k * G, 0, N * G], I.dirt, 118);
      M.quad(corner(0, k), corner(0, k + 1), [0, 0, (k + 1) * G], [0, 0, k * G], I.dirt, 104);
      M.quad(corner(N, k), corner(N, k + 1), [N * G, 0, (k + 1) * G], [N * G, 0, k * G], I.dirt, 104);
    }
    var sea = (L.sea - 0.4) * G;
    M.quad([0, sea, 0], [N * G, sea, 0], [N * G, sea, N * G], [0, sea, N * G], I.water);

    /* ── THE BUILDINGS ── */
    var WALL = { brick: I.brick, plaster: I.plaster, timber: I.timber, ashlar: I.stone };
    L.builds.forEach(function (b, bi) {
      var x0 = b.x0 * G, x1 = (b.x0 + b.w) * G, z0 = b.z0 * G, z1 = (b.z0 + b.d) * G, yb = b.base * G, yt = b.top * G;
      var wall = b.mark ? I.mark : WALL[b.wall], tint = 110 + hash(b.x0, b.z0 + 5) * 40;
      M.box(x0, yb - G, z0, x1, yt, z1, wall, tint);
      rects.push([x0 - 1, z0 - 1, x1 + 1, z1 + 1]);
      if (b.gable) {                               // a ridge along the longer way, eaves out past the walls
        var alongX = b.w >= b.d, rise = (alongX ? b.d : b.w) * G * 0.34, o = 2.5, roof = b.mark ? I.markRoof : b.roof === 'copper' ? I.roofTile : I.roofSlate;
        if (alongX) {
          var zm = (z0 + z1) / 2;
          M.quad([x0 - o, yt - 1, z1 + o], [x1 + o, yt - 1, z1 + o], [x1 + o, yt + rise, zm], [x0 - o, yt + rise, zm], roof, tint);
          M.quad([x1 + o, yt - 1, z0 - o], [x0 - o, yt - 1, z0 - o], [x0 - o, yt + rise, zm], [x1 + o, yt + rise, zm], roof, tint - 14);
          M.tri([x0, yt, z0], [x0, yt, z1], [x0, yt + rise - 1, zm], wall, tint); M.tri([x1, yt, z1], [x1, yt, z0], [x1, yt + rise - 1, zm], wall, tint);
        } else {
          var xm = (x0 + x1) / 2;
          M.quad([x1 + o, yt - 1, z1 + o], [x1 + o, yt - 1, z0 - o], [xm, yt + rise, z0 - o], [xm, yt + rise, z1 + o], roof, tint);
          M.quad([x0 - o, yt - 1, z0 - o], [x0 - o, yt - 1, z1 + o], [xm, yt + rise, z1 + o], [xm, yt + rise, z0 - o], roof, tint - 14);
          M.tri([x0, yt, z1], [x1, yt, z1], [xm, yt + rise - 1, z1], wall, tint); M.tri([x1, yt, z0], [x0, yt, z0], [xm, yt + rise - 1, z0], wall, tint);
        }
        if (!b.mark) M.box(x0 + 5, yt + rise * 0.3, z0 + 4, x0 + 9, yt + rise + 4, z0 + 8, I.brick, 110);        // a chimney
      } else {                                     // flat, behind a parapet
        M.box(x0, yt, z0, x1, yt + 3, z0 + 1.6, I.trim); M.box(x0, yt, z1 - 1.6, x1, yt + 3, z1, I.trim);
        M.box(x0, yt, z0, x0 + 1.6, yt + 3, z1, I.trim); M.box(x1 - 1.6, yt, z0, x1, yt + 3, z1, I.trim);
        M.quad([x0, yt + 0.6, z0], [x1, yt + 0.6, z0], [x1, yt + 0.6, z1], [x0, yt + 0.6, z1], b.roof === 'copper' ? I.roofCopper : I.asphalt, tint);
      }
      /* a wall's own frame: `at` is a point along it and up it, `out` how far proud of it */
      function onWall(s, along, y, out) {
        return s === 0 ? [x0 + along, y, z1 + out] : s === 1 ? [x1 - along, y, z0 - out] : s === 2 ? [x1 + out, y, z1 - along] : [x0 - out, y, z0 + along];
      }
      function panel(s, a0, a1, y0, y1, out, mat, t) { M.quad(onWall(s, a0, y0, out), onWall(s, a1, y0, out), onWall(s, a1, y1, out), onWall(s, a0, y1, out), mat, t); }
      function along(s, kk) { return s === 0 ? kk : s === 1 ? b.w - 1 - kk : s === 2 ? b.d - 1 - kk : kk; }   // land.js counts every side from its low corner
      b.panes.forEach(function (w) {
        var c = (along(w[0], w[1]) + 0.5) * G, y0 = yb + w[2] * STOREY + (w[4] ? 7 : 13), y1 = y0 + (w[4] ? 17 : 9);
        panel(w[0], c - 3.4, c + 3.4, y0 - 0.8, y1 + 0.8, 0.25, I.trim);
        panel(w[0], c - 2.6, c + 2.6, y0, y1, 0.5, w[3] ? I.lit : I.glass);
      });
      var span = (b.face < 2 ? b.w : b.d), d0 = Math.floor((span - b.door) / 2);
      if (b.door) {
        var lo = Math.min(along(b.face, d0), along(b.face, d0 + b.door - 1)) * G + 1.5, hi = lo + b.door * G - 3;
        panel(b.face, lo - 1, hi + 1, yb, yb + 23, 0.25, I.trim); panel(b.face, lo, hi, yb, yb + 22, 0.5, I.door);
      }
      if (b.awning) {                              // stripes, out over the pavement
        var gem = I['gem' + b.awning.slice(-1)];
        for (var s2 = 0; s2 * 4 < span * G; s2++)
          M.quad(onWall(b.face, s2 * 4, yb + 30, 0.3), onWall(b.face, Math.min(span * G, s2 * 4 + 4), yb + 30, 0.3),
                 onWall(b.face, Math.min(span * G, s2 * 4 + 4), yb + 26, 8), onWall(b.face, s2 * 4, yb + 26, 8), s2 & 1 ? I.white : gem);
      }
      if (b.columns) for (k = 0; k < span; k += 2) {
        if (k >= d0 - 1 && k <= d0 + b.door) continue;
        var cp = onWall(b.face, (along(b.face, k) + 0.5) * G, yb, 5);
        M.cyl(cp[0], cp[2], 2.3, yb, yt - 1, 7, I.trim);
        rounds.push([cp[0], cp[2], 3]);
      }
    });

    /* ── WHAT GROWS, AND WHAT STANDS ON THE PAVEMENT ── */
    L.plants.forEach(function (t, i) {
      var px = (t[0] + 0.5) * G, pz = (t[1] + 0.5) * G, y = heightAt(px, pz), top = y + t[2] * G, tint = 104 + hash(t[0], t[1]) * 50;
      M.cyl(px, pz, 2, y, top + 3, 6, I.trunk, 128, 1.3);
      M.ball(px, top + 7, pz, 12, 10, 12, I.leaf, tint, 0.3, i);
      M.ball(px + 5, top + 12, pz - 3, 8, 7, 8, I.leaf2, tint, 0.3, i + 7);
      M.ball(px - 6, top + 10, pz + 4, 7, 6, 7, I.leaf, tint + 10, 0.3, i + 13);
      rounds.push([px, pz, 3]);
    });
    L.props.forEach(function (q, i) {
      var px = (q[0] + 0.5) * G, pz = (q[1] + 0.5) * G, y = heightAt(px, pz);
      rounds.push([px, pz, 2.5]);
      if (q[2] === 'lamp') {
        M.cyl(px, pz, 1, y, y + 30, 6, I.post); M.box(px - 3, y + 30, pz - 3, px + 3, y + 32.5, pz + 3, I.lamp);
        lamps.push([px, y + 30, pz]);
        return;
      }
      var tall = q[3] * G, lean = hash(q[0], q[1] + 9) * 6.28, lx = Math.cos(lean) * 4, lz = Math.sin(lean) * 4;       // a palm leans
      for (k = 0; k < 3; k++) M.cyl(px + lx * k / 3, pz + lz * k / 3, 1.7 - k * 0.2, y + tall * k / 3, y + tall * (k + 1) / 3 + 0.5, 6, I.palm, 122 + k * 6, 1.5 - k * 0.2);
      var tx = px + lx, tz = pz + lz, ty = y + tall;
      for (k = 0; k < 8; k++) {
        var a = k / 8 * 6.2832 + lean, ca = Math.cos(a), sa = Math.sin(a), w = 2.4;
        M.quad([tx - sa * w, ty, tz + ca * w], [tx + sa * w, ty, tz - ca * w], [tx + ca * 9 + sa * w, ty + 2.5, tz + sa * 9 - ca * w], [tx + ca * 9 - sa * w, ty + 2.5, tz + sa * 9 + ca * w], I.frond, 120 + (k & 1) * 16);
        M.tri([tx + ca * 9 - sa * w, ty + 2.5, tz + sa * 9 + ca * w], [tx + ca * 9 + sa * w, ty + 2.5, tz + sa * 9 - ca * w], [tx + ca * 18, ty - 6, tz + sa * 18], I.frond, 108 + (k & 1) * 16);
      }
      M.ball(tx, ty, tz, 2.2, 2.2, 2.2, I.trunk);
    });
    L.hedges.forEach(function (q) {
      var y = heightAt((q[0] + 0.5) * G, (q[1] + 0.5) * G);
      M.box(q[0] * G + 0.6, y - 2, q[1] * G + 0.6, (q[0] + 1) * G - 0.6, y + 7, (q[1] + 1) * G - 0.6, I.hedge, 112 + hash(q[0], q[1]) * 30);
      rects.push([q[0] * G, q[1] * G, (q[0] + 1) * G, (q[1] + 1) * G]);
    });
    L.parked.forEach(function (c) {
      var x0 = c.x0 * G + 1.5, x1 = c.x1 * G - 1.5, z0 = c.z0 * G + 1.5, z1 = c.z1 * G - 1.5, y = c.level * G, gem = I['gem' + c.paint];
      if (c.alongX) { z0 += 1; z1 -= 1; } else { x0 += 1; x1 -= 1; }
      var cx0 = c.alongX ? x0 + (x1 - x0) * 0.24 : x0 + 1.2, cx1 = c.alongX ? x1 - (x1 - x0) * 0.2 : x1 - 1.2, cz0 = c.alongX ? z0 + 1.2 : z0 + (z1 - z0) * 0.24, cz1 = c.alongX ? z1 - 1.2 : z1 - (z1 - z0) * 0.2;
      M.box(x0, y + 2, z0, x1, y + 7, z1, gem); M.box(cx0, y + 7, cz0, cx1, y + 11, cz1, I.glass); M.box(cx0 - 0.3, y + 11, cz0 - 0.3, cx1 + 0.3, y + 12, cz1 + 0.3, gem);
      [[0, 0], [0, 1], [1, 0], [1, 1]].forEach(function (w) {                       // a wheel at each corner
        var wx = c.alongX ? (w[0] ? x1 - 5.5 : x0 + 2.5) : (w[0] ? x1 - 1 : x0 - 0.6), wz = c.alongX ? (w[1] ? z1 - 1 : z0 - 0.6) : (w[1] ? z1 - 5.5 : z0 + 2.5);
        M.box(wx, y, wz, wx + (c.alongX ? 3.4 : 1.6), y + 3.6, wz + (c.alongX ? 1.6 : 3.4), I.tyre);
      });
      rects.push([c.x0 * G + 1, c.z0 * G + 1, c.x1 * G - 1, c.z1 * G - 1]);
    });

    /* How high each field stands, in quarter blocks, for the sun to cast by: what is built on it, or else the
       rolling ground itself — not land.js's stepped ground, which would shade every slope in patches. */
    var tops = new Uint8Array(N * N);
    for (z = 0; z < N; z++) for (x = 0; x < N; x++) {
      var f = z * N + x, built = L.tops[f] > H[f] + (map[f] === 4 ? 1 : 0);
      tops[f] = Math.max(0, Math.min(255, Math.round((built ? L.tops[f] : heightAt((x + 0.5) * G, (z + 0.5) * G) / G) * 4)));
    }
    /* Is a body of radius r, standing at x, z, in something: a wall, a hedge, a car, a trunk, or over the land's edge. */
    function inTheWay(px, pz, r) {
      if (px < r || pz < r || px > N * G - r || pz > N * G - r) return true;
      var i, q;
      for (i = 0; i < rects.length; i++) { q = rects[i]; if (px + r > q[0] && px - r < q[2] && pz + r > q[1] && pz - r < q[3]) return true; }
      for (i = 0; i < rounds.length; i++) { q = rounds[i]; if (Math.hypot(px - q[0], pz - q[1]) < r + q[2]) return true; }
      return false;
    }
    return { mesh: M.done(), rects: rects, rounds: rounds, lamps: lamps, heightAt: heightAt, inTheWay: inTheWay, wide: N * G, tops: tops };
  }

  /* A place's sign: a post, a panel in the place's own colour, a page drawn on it. Stood at 0, 0, facing south. */
  function sign(paint) {
    var M = new Mesh(), gem = I['gem' + (paint % 7)], y = 44;
    M.cyl(0, -3, 1.7, -4, y + 12, 7, I.post);
    M.box(-16, y, -1.5, 16, y + 24, 1.5, gem);
    M.quad([-13.5, y + 2.5, 1.8], [13.5, y + 2.5, 1.8], [13.5, y + 21.5, 1.8], [-13.5, y + 21.5, 1.8], I.white);
    M.quad([-13.5, y + 17.5, 2], [13.5, y + 17.5, 2], [13.5, y + 21.5, 2], [-13.5, y + 21.5, 2], I.dark);
    M.quad([-10.5, y + 9.5, 2], [10.5, y + 9.5, 2], [10.5, y + 15, 2], [-10.5, y + 15, 2], gem);
    M.quad([-10.5, y + 6.5, 2], [5, y + 6.5, 2], [5, y + 7.8, 2], [-10.5, y + 7.8, 2], I.dark);
    M.quad([-10.5, y + 4, 2], [1, y + 4, 2], [1, y + 5.3, 2], [-10.5, y + 5.3, 2], I.dark);
    return M.done();
  }
  /* The diamond over a head, or over where you are going. */
  function diamond(stuff) {
    var M = new Mesh(), r = 4.6, pts = [[r, 0, 0], [0, 0, r], [-r, 0, 0], [0, 0, -r]];
    for (var k = 0; k < 4; k++) { M.tri([0, 8, 0], pts[k], pts[(k + 1) % 4], I[stuff], 128 + (k & 1) * 30); M.tri([0, -8, 0], pts[(k + 1) % 4], pts[k], I[stuff], 96 + (k & 1) * 24); }
    return M.done();
  }

  return { build: build, sign: sign, diamond: diamond, Mesh: Mesh, G: G };
});
